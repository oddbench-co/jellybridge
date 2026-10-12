//! The libmpv ↔ egui OpenGL seam.
//!
//! mpv renders into a framebuffer we own, on the window's own GL context,
//! from inside an egui paint callback (the one place egui guarantees that
//! context is current). Each frame that framebuffer is blitted into the
//! window at the video's place in the layout. mpv draws only when it has a new
//! frame or the size changed; every other repaint -- the pointer moving over
//! the controls, the chrome fading -- is a single GPU blit.

use std::ffi::{CStr, c_char, c_void};
use std::marker::PhantomData;
use std::ptr::{self, NonNull};
use std::rc::Rc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, mpsc};
use std::thread::{self, JoinHandle};

use eframe::glow::{self, HasContext};
use libloading::Library;
use crate::services::libmpv::{self as sys, Api};
#[cfg(target_os = "linux")]
use raw_window_handle::RawDisplayHandle;
use raw_window_handle::HasDisplayHandle;

use crate::services::player::{
    PlayerCommand, PlayerSender, RenderClient, RenderClientSlot, RenderClientState,
};

type RedrawRequest = Arc<dyn Fn() + Send + Sync + 'static>;

/// Video frames drawn by mpv since start; `JELLYBRIDGE_MEASURE` reports the
/// rate, which shows whether playback keeps up.
pub static VIDEO_FRAMES: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);

/// The window's `GL_VENDOR`, read once when the window is created.
static GL_VENDOR: std::sync::OnceLock<String> = std::sync::OnceLock::new();

pub fn note_gl_vendor(gl: &glow::Context) {
    let _ = GL_VENDOR.set(unsafe { gl.get_parameter_string(glow::VENDOR) });
}

/// The single hardware-decode interop mpv should load for this GPU, or
/// `None` to leave it on `auto`.
///
/// In the render API `auto` means "load every interop when the render
/// context is created", and that happens on the UI thread, inside a paint.
/// On NVIDIA's driver that opened the VAAPI driver (242 ms) and VDPAU
/// (~70 ms) only to then decode through CUDA, which `hwdec=auto-safe`
/// picks there: a 330 ms freeze of the loading spinner. A named interop is
/// the only one mpv will ever load, so this names one only where the choice
/// is certain; copy-back and software decoding need none.
pub fn preferred_hwdec_interop() -> Option<&'static str> {
    GL_VENDOR
        .get()
        .is_some_and(|vendor| vendor.starts_with("NVIDIA"))
        .then_some("cuda")
}

/// When playback was last asked for, until its first video frame is drawn.
static PLAY_REQUESTED: std::sync::Mutex<Option<std::time::Instant>> = std::sync::Mutex::new(None);

/// Starts the clock for `time_to_first_frame`, the playback counterpart of
/// startup's `time_to_interactive`: from pressing play to the picture.
pub fn note_play_requested() {
    if let Ok(mut requested) = PLAY_REQUESTED.lock() {
        *requested = Some(std::time::Instant::now());
    }
}

#[derive(Debug, thiserror::Error)]
pub enum RenderError {
    #[error("failed to load the platform OpenGL resolver")]
    Resolver,
    // Only constructed on Linux, where display-handle resolution can fail.
    #[cfg_attr(not(target_os = "linux"), allow(dead_code))]
    #[error("unsupported native display handle for libmpv hardware interop")]
    UnsupportedDisplay,
    #[error("libmpv render-context operation failed: {0}")]
    Mpv(String),
    #[error("OpenGL resource creation failed: {0}")]
    OpenGl(String),
    #[error("failed to start the render wake bridge")]
    WakeThread(#[source] std::io::Error),
}

// Every target constructs only a subset of these -- X11/Wayland on Linux,
// None everywhere else -- but the mpv render-param match must still cover
// them all, so the unused ones are dead code on any single target.
#[derive(Clone, Copy)]
#[allow(dead_code)]
pub enum NativeDisplay {
    None,
    X11(*const c_void),
    Wayland(*const c_void),
}

/// The window system display mpv needs for hardware-decode interop.
pub fn native_display(window: &impl HasDisplayHandle) -> Result<NativeDisplay, RenderError> {
    #[cfg(target_os = "linux")]
    {
        let handle = window
            .display_handle()
            .map_err(|_| RenderError::UnsupportedDisplay)?;
        match handle.as_raw() {
            RawDisplayHandle::Xlib(display) => display
                .display
                .map(|pointer| NativeDisplay::X11(pointer.as_ptr()))
                .ok_or(RenderError::UnsupportedDisplay),
            RawDisplayHandle::Wayland(display) => {
                Ok(NativeDisplay::Wayland(display.display.as_ptr()))
            }
            _ => Err(RenderError::UnsupportedDisplay),
        }
    }
    #[cfg(not(target_os = "linux"))]
    {
        let _ = window;
        Ok(NativeDisplay::None)
    }
}

/// Where in the window, in physical pixels with a bottom-left origin (GL's),
/// the video goes.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct VideoViewport {
    pub left: i32,
    pub bottom: i32,
    pub width: i32,
    pub height: i32,
}

pub struct RenderHost {
    mpv: Option<RawRenderContext>,
    /// Valid for the lifetime of the window. `None` until the window exists,
    /// and when it could not be resolved (mpv then renders without hardware
    /// interop).
    display: NativeDisplay,
    target: Option<FrameTarget>,
    /// Set when mpv drew the last frame, so the swap is reported for frames
    /// mpv actually produced.
    swap_pending: bool,
    /// mpv announced a frame that has not been drawn yet.
    frame_pending: bool,
    debug_checked: bool,
    debug_enabled_by_us: bool,
    /// The player was left: free the render context on the next paint.
    release_requested: bool,
    redraw: RedrawRequest,
    /// Fires once the render context exists, so playback can hold `loadfile`
    /// until mpv has a video output to choose a decode path against.
    ready: tokio::sync::watch::Sender<bool>,
    /// Where the worker leaves the render client once it has built mpv.
    slot: RenderClientSlot,
    /// To have mpv unloaded once the render context is released.
    player: PlayerSender,
    // This must be declared after RawRenderContext so Rust's field drop order
    // cannot destroy the client handle before its render context.
    core: Option<RenderClient>,
}

/// Announces that `ensure_mpv` has finished, on success and on failure alike.
struct ReadySignal<'a>(&'a tokio::sync::watch::Sender<bool>);

impl Drop for ReadySignal<'_> {
    fn drop(&mut self) {
        let _ = self.0.send(true);
    }
}

impl RenderHost {
    pub fn new(
        slot: RenderClientSlot,
        player: PlayerSender,
        display: NativeDisplay,
        redraw: RedrawRequest,
        ready: tokio::sync::watch::Sender<bool>,
    ) -> Self {
        Self {
            mpv: None,
            display,
            target: None,
            swap_pending: false,
            frame_pending: false,
            debug_checked: false,
            debug_enabled_by_us: false,
            release_requested: false,
            redraw,
            ready,
            slot,
            player,
            core: None,
        }
    }

    fn enable_debug_output(&mut self, gl: &glow::Context) {
        if std::mem::replace(&mut self.debug_checked, true) {
            return;
        }
        let version = unsafe { gl.get_parameter_string(glow::VERSION) };
        let vendor = unsafe { gl.get_parameter_string(glow::VENDOR) };
        let renderer = unsafe { gl.get_parameter_string(glow::RENDERER) };
        tracing::info!(%version, %vendor, %renderer, "window OpenGL context");
        if std::env::var_os("JELLYBRIDGE_GL_DEBUG").is_some() && gl.supports_debug() {
            let was_enabled = unsafe { gl.is_enabled(glow::DEBUG_OUTPUT) };
            unsafe {
                gl.enable(glow::DEBUG_OUTPUT);
                gl.enable(glow::DEBUG_OUTPUT_SYNCHRONOUS);
            }
            self.debug_enabled_by_us = !was_enabled;
        }
    }

    /// Builds mpv's render context the first time a frame needs it. Runs
    /// inside a paint callback, so the GL context is current, which is what
    /// `mpv_render_context_create` requires.
    ///
    /// Deliberately lazy: creating it loads the GL entry points and builds
    /// mpv's whole render pipeline, which measured 310 ms of a 539 ms
    /// time-to-interactive -- all of it spent on a video path the user has
    /// not opened.
    ///
    /// `Ok(false)` while the player worker is still loading libmpv: the
    /// worker's `RendererAvailable` event brings the frame that retries.
    fn ensure_mpv(&mut self) -> Result<bool, RenderError> {
        if self.mpv.is_some() {
            return Ok(true);
        }
        if self.core.is_none() {
            match self.slot.take() {
                RenderClientState::Pending => return Ok(false),
                RenderClientState::Ready(client) => self.core = Some(client),
                RenderClientState::Failed(message) => {
                    let _signal = ReadySignal(&self.ready);
                    return Err(RenderError::Mpv(message));
                }
            }
        }
        let started = std::time::Instant::now();
        // Released on the way out whatever happens: a machine that cannot
        // build a context must still play rather than wait forever for one.
        let _signal = ReadySignal(&self.ready);
        let core = self.core.as_ref().expect("taken above");
        let raw = RawRenderContext::create(core, self.display, self.redraw.clone())?;
        tracing::info!(
            ms = started.elapsed().as_secs_f64() * 1000.0,
            "mpv render context created"
        );
        self.mpv = Some(raw);
        Ok(true)
    }

    /// Runs mpv's render-thread work on a frame that draws no video.
    ///
    /// The advanced-control contract does not lapse because the player screen
    /// is hidden, and the render context outlives that screen. Deliberately
    /// does not `ensure_mpv`: a session that never opens the player never pays
    /// for the context.
    ///
    /// `may_release`: the user is not interacting, so the release's ~130 ms
    /// on this thread (mpv tearing down its CUDA interop and GL renderer)
    /// lands on a still screen instead of a scroll or a click.
    pub fn pump(&mut self, gl: &glow::Context, may_release: bool) {
        // Only once mpv has finished stopping. Freeing the context while its
        // video output is still shutting down deadlocks the two until mpv's
        // timeout: measured 313 ms of frozen UI on leaving the player.
        if may_release && self.release_requested && self.slot.player_idle() {
            self.release_requested = false;
            self.release(gl);
            return;
        }
        if let Some(mpv) = self.mpv.as_ref() {
            mpv.poll();
        }
    }

    /// Frees the render context and its framebuffer once the player is
    /// left. Measured after leaving a 4K film: the context held ~150 MB of
    /// NVIDIA driver memory (the CUDA interop and mpv's GPU buffers) for a
    /// screen no longer showing. The next playback builds it again (~100 ms,
    /// the same as the first), and waits for it as the first does.
    pub fn release_soon(&mut self) {
        self.release_requested = self.mpv.is_some();
    }

    /// A release is waiting for the user to go quiet.
    pub fn release_pending(&self) -> bool {
        self.release_requested
    }

    /// Back in the player before the release happened: keep the context.
    pub fn keep(&mut self) {
        self.release_requested = false;
    }

    fn release(&mut self, gl: &glow::Context) {
        let started = std::time::Instant::now();
        // After a stop mpv's video output still holds a last frame and waits
        // for it to be rendered before it can shut down; off the player
        // screen nothing renders, so freeing the context sat out mpv's
        // ~200 ms "render not being called" timeout on the UI thread.
        // Rendering it into the old target lets the output close at once.
        if let (Some(mpv), Some(target)) = (self.mpv.as_ref(), self.target.as_ref())
            && (mpv.poll() || self.frame_pending)
        {
            let _state = GlStateGuard::capture(gl);
            let (width, height) = target.size;
            if let Err(error) = mpv.render(target.framebuffer.0.get() as i32, width, height) {
                tracing::debug!(%error, "final frame before release failed");
            }
            mpv.report_swap();
        }
        self.mpv.take();
        // The client handle goes too, so the worker's is the core's last and
        // destroying it shuts mpv down entirely.
        self.core.take();
        let _ = self.player.send(PlayerCommand::Unload);
        if let Some(target) = self.target.take() {
            target.destroy(gl);
        }
        self.swap_pending = false;
        self.frame_pending = false;
        // The next `loadfile` must wait for the next context again.
        self.ready.send_replace(false);
        tracing::info!(
            ms = started.elapsed().as_secs_f64() * 1000.0,
            "mpv render context released"
        );
    }

    /// Whether a render context exists whose work must be pumped.
    pub fn is_active(&self) -> bool {
        self.mpv.is_some()
    }

    /// Draws the current video frame into the bound framebuffer at
    /// `viewport`. Must run with the window's GL context current.
    pub fn draw(&mut self, gl: &glow::Context, viewport: VideoViewport) -> Result<(), RenderError> {
        self.enable_debug_output(gl);
        if !self.ensure_mpv()? {
            return Ok(());
        }
        let mpv = self.mpv.as_ref().expect("ensured above");
        // The previous frame mpv drew has been presented by now: the window
        // swaps between our paint callbacks.
        if std::mem::take(&mut self.swap_pending) {
            mpv.report_swap();
        }
        // Unconditional, and before the branch below: `&&` would short-circuit
        // past it on a resize and on the first frame, and mpv's contract has no
        // exception for either. See `RawRenderContext::poll`.
        if mpv.poll() {
            self.frame_pending = true;
        }
        let width = viewport.width.max(1);
        let height = viewport.height.max(1);

        let state = GlStateGuard::capture(gl);
        let resized = self
            .target
            .as_ref()
            .is_none_or(|target| target.size != (width, height));
        if resized {
            if let Some(old) = self.target.take() {
                old.destroy(gl);
            }
            self.target = Some(FrameTarget::create(gl, width, height)?);
        }
        let target = self.target.as_ref().expect("created above");
        // mpv's own render call would block the UI thread until the frame's
        // display time (measured: 41 ms of every 41.7 ms frame of a 24 fps
        // film). It is told not to; instead a frame that is not due yet is
        // left pending, the previous one stays on screen, and a repaint is
        // asked for at the moment it is due.
        let due = self.frame_pending && {
            match mpv.next_frame_target_ns() {
                Some(target) => {
                    let wait = target - mpv.time_ns();
                    if wait > 1_500_000 {
                        mpv.redraw_at(
                            std::time::Instant::now() + std::time::Duration::from_nanos(wait as u64),
                        );
                        false
                    } else {
                        true
                    }
                }
                None => true,
            }
        };
        if resized || due {
            self.frame_pending = false;
            VIDEO_FRAMES.fetch_add(1, Ordering::Relaxed);
            if let Some(requested) = PLAY_REQUESTED.lock().ok().and_then(|mut slot| slot.take()) {
                tracing::info!(
                    ms = requested.elapsed().as_secs_f64() * 1000.0,
                    "time_to_first_frame"
                );
            }
            unsafe {
                gl.bind_framebuffer(glow::FRAMEBUFFER, Some(target.framebuffer));
                gl.viewport(0, 0, width, height);
                gl.disable(glow::SCISSOR_TEST);
                gl.clear_color(0.0, 0.0, 0.0, 1.0);
                gl.clear(glow::COLOR_BUFFER_BIT);
            }
            mpv.render(target.framebuffer.0.get() as i32, width, height)?;
            self.swap_pending = true;
        }

        // Copy into the window. The guard captured the framebuffer egui is
        // drawing into, and restores everything when it drops.
        unsafe {
            gl.disable(glow::SCISSOR_TEST);
            gl.bind_framebuffer(glow::READ_FRAMEBUFFER, Some(target.framebuffer));
            gl.bind_framebuffer(glow::DRAW_FRAMEBUFFER, state.draw_framebuffer);
            gl.blit_framebuffer(
                0,
                0,
                width,
                height,
                viewport.left,
                viewport.bottom,
                viewport.left + width,
                viewport.bottom + height,
                glow::COLOR_BUFFER_BIT,
                glow::NEAREST,
            );
            // mpv leaves its alpha channel unspecified, and the window's alpha
            // reaches the compositor: without this the desktop showed through
            // the video. Only alpha is written, only inside the video.
            gl.enable(glow::SCISSOR_TEST);
            gl.scissor(viewport.left, viewport.bottom, width, height);
            gl.color_mask(false, false, false, true);
            gl.clear_color(0.0, 0.0, 0.0, 1.0);
            gl.clear(glow::COLOR_BUFFER_BIT);
        }
        Ok(())
    }

    /// Frees mpv's render context and our framebuffer. Must run with the
    /// window's GL context current (eframe's `on_exit` provides it).
    pub fn teardown(&mut self, gl: Option<&glow::Context>) {
        let had_resources = self.mpv.is_some() || self.target.is_some();
        self.mpv.take(); // unregister callback and free mpv GL resources first
        // Without a context it is being destroyed, which releases the objects.
        if let (Some(target), Some(gl)) = (self.target.take(), gl) {
            target.destroy(gl);
        }
        if let Some(gl) = gl
            && self.debug_enabled_by_us
        {
            unsafe {
                gl.disable(glow::DEBUG_OUTPUT_SYNCHRONOUS);
                gl.disable(glow::DEBUG_OUTPUT);
            }
            self.debug_enabled_by_us = false;
        }
        if had_resources {
            tracing::info!("libmpv render context and OpenGL target torn down");
        }
    }
}

impl Drop for RenderHost {
    fn drop(&mut self) {
        // Only a fallback: a clean exit tears down in `on_exit` with the GL
        // context current.
        if self.mpv.is_some() || self.target.is_some() {
            tracing::error!("render host dropped without a teardown");
        }
    }
}

/// mpv's render target: one texture-backed framebuffer at the video's size.
/// One is enough -- the blit into the window happens before this callback
/// returns, so nothing samples it while mpv draws the next frame.
struct FrameTarget {
    size: (i32, i32),
    texture: glow::NativeTexture,
    framebuffer: glow::NativeFramebuffer,
}

impl FrameTarget {
    fn create(gl: &glow::Context, width: i32, height: i32) -> Result<Self, RenderError> {
        unsafe {
            let texture = gl
                .create_texture()
                .map_err(|error| RenderError::OpenGl(error.to_owned()))?;
            gl.bind_texture(glow::TEXTURE_2D, Some(texture));
            for (parameter, value) in [
                (glow::TEXTURE_MIN_FILTER, glow::LINEAR),
                (glow::TEXTURE_MAG_FILTER, glow::LINEAR),
                (glow::TEXTURE_WRAP_S, glow::CLAMP_TO_EDGE),
                (glow::TEXTURE_WRAP_T, glow::CLAMP_TO_EDGE),
            ] {
                gl.tex_parameter_i32(glow::TEXTURE_2D, parameter, value as i32);
            }
            gl.tex_image_2d(
                glow::TEXTURE_2D,
                0,
                glow::RGBA8 as i32,
                width,
                height,
                0,
                glow::RGBA,
                glow::UNSIGNED_BYTE,
                glow::PixelUnpackData::Slice(None),
            );
            let framebuffer = match gl.create_framebuffer() {
                Ok(framebuffer) => framebuffer,
                Err(error) => {
                    gl.delete_texture(texture);
                    return Err(RenderError::OpenGl(error));
                }
            };
            gl.bind_framebuffer(glow::FRAMEBUFFER, Some(framebuffer));
            gl.framebuffer_texture_2d(
                glow::FRAMEBUFFER,
                glow::COLOR_ATTACHMENT0,
                glow::TEXTURE_2D,
                Some(texture),
                0,
            );
            let status = gl.check_framebuffer_status(glow::FRAMEBUFFER);
            if status != glow::FRAMEBUFFER_COMPLETE {
                gl.delete_framebuffer(framebuffer);
                gl.delete_texture(texture);
                return Err(RenderError::OpenGl(format!(
                    "incomplete video framebuffer 0x{status:04x}"
                )));
            }
            Ok(Self {
                size: (width, height),
                texture,
                framebuffer,
            })
        }
    }

    fn destroy(self, gl: &glow::Context) {
        let _state = GlStateGuard::capture(gl);
        unsafe {
            gl.delete_framebuffer(self.framebuffer);
            gl.delete_texture(self.texture);
        }
    }
}

enum RenderWake {
    Redraw,
    /// Redraw at this moment: a frame mpv announced becomes due then.
    RedrawAt(std::time::Instant),
    Shutdown,
}

struct UpdateCallbackState {
    queued: Arc<AtomicBool>,
    sender: mpsc::Sender<RenderWake>,
}

unsafe extern "C" fn render_update_callback(context: *mut c_void) {
    if let Some(state) = NonNull::new(context.cast::<UpdateCallbackState>()) {
        // SAFETY: user data is boxed for the full registered-callback lifetime.
        let state = unsafe { state.as_ref() };
        if !state.queued.swap(true, Ordering::AcqRel) {
            let _ = state.sender.send(RenderWake::Redraw);
        }
    }
}

struct RawRenderContext {
    context: NonNull<sys::mpv_render_context>,
    /// The client handle the context was made from, for mpv's clock. Outlived
    /// by `RenderHost::core`, which owns it.
    core: *mut sys::mpv_handle,
    api: &'static Api,
    resolver: Box<GlProcResolver>,
    update_state: Box<UpdateCallbackState>,
    wake_thread: Option<JoinHandle<()>>,
    _thread_bound: PhantomData<Rc<()>>,
}

impl RawRenderContext {
    fn create(
        core: &RenderClient,
        display: NativeDisplay,
        redraw: RedrawRequest,
    ) -> Result<Self, RenderError> {
        let api = sys::api().map_err(|error| RenderError::Mpv(error.to_string()))?;
        let mut resolver = Box::new(GlProcResolver::load()?);
        let resolver_pointer = (&mut *resolver as *mut GlProcResolver).cast::<c_void>();
        let mut init = sys::mpv_opengl_init_params {
            get_proc_address: Some(resolve_gl_proc),
            get_proc_address_ctx: resolver_pointer,
        };
        let mut advanced_control = 1_i32;
        let mut parameters = vec![
            sys::mpv_render_param {
                kind: sys::RENDER_PARAM_API_TYPE,
                data: sys::RENDER_API_TYPE_OPENGL.as_ptr().cast_mut().cast(),
            },
            sys::mpv_render_param {
                kind: sys::RENDER_PARAM_OPENGL_INIT_PARAMS,
                data: (&mut init as *mut sys::mpv_opengl_init_params).cast(),
            },
            sys::mpv_render_param {
                kind: sys::RENDER_PARAM_ADVANCED_CONTROL,
                data: (&mut advanced_control as *mut i32).cast(),
            },
        ];
        match display {
            NativeDisplay::None => {}
            NativeDisplay::X11(pointer) => parameters.push(sys::mpv_render_param {
                kind: sys::RENDER_PARAM_X11_DISPLAY,
                data: pointer.cast_mut(),
            }),
            NativeDisplay::Wayland(pointer) => parameters.push(sys::mpv_render_param {
                kind: sys::RENDER_PARAM_WL_DISPLAY,
                data: pointer.cast_mut(),
            }),
        }
        parameters.push(sys::mpv_render_param {
            kind: sys::RENDER_PARAM_INVALID,
            data: ptr::null_mut(),
        });

        let mut raw = ptr::null_mut();
        let status = unsafe {
            (api.render_context_create)(&mut raw, core.as_ptr(), parameters.as_mut_ptr())
        };
        check_mpv(api, status)?;
        let context = NonNull::new(raw).ok_or_else(|| RenderError::Mpv("null context".into()))?;

        let queued = Arc::new(AtomicBool::new(false));
        let (sender, receiver) = mpsc::channel();
        let wake_queued = queued.clone();
        let wake_thread = match thread::Builder::new()
            .name("jellybridge-render-wake".to_owned())
            .spawn(move || {
                // The earliest pending "redraw at" deadline.
                let mut deadline: Option<std::time::Instant> = None;
                loop {
                    let message = match deadline {
                        Some(at) => {
                            match receiver.recv_timeout(at.saturating_duration_since(std::time::Instant::now())) {
                                Ok(message) => message,
                                Err(mpsc::RecvTimeoutError::Timeout) => {
                                    deadline = None;
                                    redraw();
                                    continue;
                                }
                                Err(mpsc::RecvTimeoutError::Disconnected) => break,
                            }
                        }
                        None => match receiver.recv() {
                            Ok(message) => message,
                            Err(_) => break,
                        },
                    };
                    match message {
                        RenderWake::RedrawAt(at) => {
                            deadline = Some(deadline.map_or(at, |current| current.min(at)));
                        }
                        RenderWake::Redraw => {
                            // Cleared before the request, not after. A callback
                            // that lands while the redraw is still queued is
                            // covered by it; one that lands after it has been
                            // requested must be able to queue another, or the
                            // `mpv_render_context_update` it needs is lost
                            // until some unrelated repaint happens. The cost of
                            // being early here is a redundant repaint, which
                            // egui coalesces.
                            wake_queued.store(false, Ordering::Release);
                            redraw();
                        }
                        RenderWake::Shutdown => break,
                    }
                }
            }) {
            Ok(thread) => thread,
            Err(error) => {
                unsafe { (api.render_context_free)(context.as_ptr()) };
                return Err(RenderError::WakeThread(error));
            }
        };
        let mut update_state = Box::new(UpdateCallbackState { queued, sender });
        unsafe {
            (api.render_context_set_update_callback)(
                context.as_ptr(),
                Some(render_update_callback),
                (&mut *update_state as *mut UpdateCallbackState).cast(),
            );
        }

        Ok(Self {
            context,
            core: core.as_ptr(),
            api,
            resolver,
            update_state,
            wake_thread: Some(wake_thread),
            _thread_bound: PhantomData,
        })
    }

    /// Runs mpv's render-thread work and reports whether a new frame is ready.
    ///
    /// `mpv_render_context_update` is not a query that can be skipped. Under
    /// `MPV_RENDER_PARAM_ADVANCED_CONTROL` libmpv documents calling it after
    /// every update callback as a hard requirement, and it is where mpv
    /// allocates the textures libavcodec decodes into (`vd-lavc-dr`). Direct
    /// rendering exists only on the software decode path -- an nvdec frame
    /// arrives as a CUDA-interop texture and needs none of it -- so gating this
    /// call behind a cached flag starved the software decoder of buffers and
    /// left the black first frame on screen while audio kept playing.
    ///
    /// The flags it returns still gate `render`: `MPV_RENDER_UPDATE_FRAME` is
    /// the one that means "call render", and drawing without it made mpv redo
    /// the full scale, colour-convert and dither pass at display refresh rate
    /// for content that produces frames far more slowly.
    fn poll(&self) -> bool {
        let flags = unsafe { (self.api.render_context_update)(self.context.as_ptr()) };
        flags & sys::RENDER_UPDATE_FRAME != 0
    }

    /// Tells mpv the rendered frame reached the display. Required by the
    /// `MPV_RENDER_PARAM_ADVANCED_CONTROL` contract this context opts into;
    /// without it mpv's frame timing has nothing to synchronise against.
    fn report_swap(&self) {
        unsafe { (self.api.render_context_report_swap)(self.context.as_ptr()) };
    }

    /// When the next frame should be shown, in `mpv_get_time_ns` time (libmpv
    /// 2.x reports target times in nanoseconds); `None` for a frame with no
    /// target (a redraw, or vsync-locked timing).
    fn next_frame_target_ns(&self) -> Option<i64> {
        let mut info = sys::mpv_render_frame_info {
            flags: 0,
            target_time: 0,
        };
        let status = unsafe {
            (self.api.render_context_get_info)(
                self.context.as_ptr(),
                sys::mpv_render_param {
                    kind: sys::RENDER_PARAM_NEXT_FRAME_INFO,
                    data: (&mut info as *mut sys::mpv_render_frame_info).cast(),
                },
            )
        };
        let redraw = info.flags & sys::RENDER_FRAME_INFO_REDRAW != 0;
        (status >= 0 && !redraw && info.target_time > 0).then_some(info.target_time)
    }

    /// Asks the wake thread for a redraw at `at`. A repaint requested from
    /// inside a paint callback is dropped by egui, so it has to come from
    /// another thread.
    fn redraw_at(&self, at: std::time::Instant) {
        let _ = self.update_state.sender.send(RenderWake::RedrawAt(at));
    }

    /// mpv's clock, the base of frame target times.
    fn time_ns(&self) -> i64 {
        unsafe { self.api.time_ns(self.core) }
    }

    fn render(&self, framebuffer: i32, width: i32, height: i32) -> Result<(), RenderError> {
        let mut fbo = sys::mpv_opengl_fbo {
            fbo: framebuffer,
            w: width,
            h: height,
            internal_format: glow::RGBA8 as i32,
        };
        let mut flip = 1_i32;
        // Never wait for the display time here: `draw` times frames itself.
        let mut block = 0_i32;
        let mut parameters = [
            sys::mpv_render_param {
                kind: sys::RENDER_PARAM_OPENGL_FBO,
                data: (&mut fbo as *mut sys::mpv_opengl_fbo).cast(),
            },
            sys::mpv_render_param {
                kind: sys::RENDER_PARAM_FLIP_Y,
                data: (&mut flip as *mut i32).cast(),
            },
            sys::mpv_render_param {
                kind: sys::RENDER_PARAM_BLOCK_FOR_TARGET_TIME,
                data: (&mut block as *mut i32).cast(),
            },
            sys::mpv_render_param {
                kind: sys::RENDER_PARAM_INVALID,
                data: ptr::null_mut(),
            },
        ];
        check_mpv(self.api, unsafe {
            (self.api.render_context_render)(self.context.as_ptr(), parameters.as_mut_ptr())
        })
    }
}

impl Drop for RawRenderContext {
    fn drop(&mut self) {
        unsafe {
            (self.api.render_context_set_update_callback)(
                self.context.as_ptr(),
                None,
                ptr::null_mut(),
            );
        }
        let _ = self.update_state.sender.send(RenderWake::Shutdown);
        if let Some(thread) = self.wake_thread.take() {
            let _ = thread.join();
        }
        unsafe { (self.api.render_context_free)(self.context.as_ptr()) };

        // Keep the resolver visibly live until after mpv frees its context.
        let _ = &self.resolver;
    }
}

unsafe extern "C" fn resolve_gl_proc(context: *mut c_void, name: *const c_char) -> *mut c_void {
    if context.is_null() || name.is_null() {
        return ptr::null_mut();
    }
    // SAFETY: the resolver box outlives the registered mpv render context.
    unsafe { (&*(context.cast::<GlProcResolver>())).resolve(CStr::from_ptr(name)) }
}

type PlatformGetProc = unsafe extern "C" fn(*const c_char) -> *const c_void;

struct GlProcResolver {
    libraries: Vec<Library>,
    fallbacks: Vec<PlatformGetProc>,
}

impl GlProcResolver {
    fn load() -> Result<Self, RenderError> {
        #[cfg(target_os = "macos")]
        let candidates = ["/System/Library/Frameworks/OpenGL.framework/OpenGL"];
        #[cfg(target_os = "linux")]
        let candidates = ["libEGL.so.1", "libGL.so.1"];
        #[cfg(not(any(target_os = "macos", target_os = "linux")))]
        let candidates: [&str; 0] = [];

        let mut libraries = Vec::new();
        for candidate in candidates {
            if let Ok(library) = unsafe { Library::new(candidate) } {
                libraries.push(library);
            }
        }
        if libraries.is_empty() {
            return Err(RenderError::Resolver);
        }
        let mut fallbacks = Vec::new();
        for library in &libraries {
            for symbol_name in [b"eglGetProcAddress\0".as_slice(), b"glXGetProcAddressARB\0"] {
                if let Ok(symbol) = unsafe { library.get::<PlatformGetProc>(symbol_name) } {
                    fallbacks.push(*symbol);
                }
            }
        }
        Ok(Self {
            libraries,
            fallbacks,
        })
    }

    unsafe fn resolve(&self, name: &CStr) -> *mut c_void {
        for library in &self.libraries {
            if let Ok(symbol) = unsafe { library.get::<*mut c_void>(name.to_bytes_with_nul()) }
                && !(*symbol).is_null()
            {
                return *symbol;
            }
        }
        for get_proc in &self.fallbacks {
            let pointer = unsafe { get_proc(name.as_ptr()) };
            if !pointer.is_null() {
                return pointer.cast_mut();
            }
        }
        ptr::null_mut()
    }
}

fn check_mpv(api: &Api, status: i32) -> Result<(), RenderError> {
    api.check(status)
        .map_err(|error| RenderError::Mpv(error.to_string()))
}

/// Complete state contract around every mpv render operation. Texture bindings
/// are captured for every combined texture unit because mpv owns its internal
/// choice of units.
struct GlStateGuard<'a> {
    gl: &'a glow::Context,
    draw_framebuffer: Option<glow::NativeFramebuffer>,
    read_framebuffer: Option<glow::NativeFramebuffer>,
    viewport: [i32; 4],
    scissor_enabled: bool,
    scissor: [i32; 4],
    program: Option<glow::NativeProgram>,
    vertex_array: Option<glow::NativeVertexArray>,
    array_buffer: Option<glow::NativeBuffer>,
    element_buffer: Option<glow::NativeBuffer>,
    active_texture: i32,
    texture_2d_bindings: Vec<Option<glow::NativeTexture>>,
    blend_enabled: bool,
    blend_equation_rgb: i32,
    blend_equation_alpha: i32,
    blend_src_rgb: i32,
    blend_dst_rgb: i32,
    blend_src_alpha: i32,
    blend_dst_alpha: i32,
    clear_color: [f32; 4],
    dither_enabled: bool,
    color_mask: [bool; 4],
    pack_alignment: i32,
    pack_row_length: i32,
    pack_skip_pixels: i32,
    pack_skip_rows: i32,
    unpack_alignment: i32,
    unpack_row_length: i32,
    unpack_skip_pixels: i32,
    unpack_skip_rows: i32,
    debug_output: bool,
    debug_output_synchronous: bool,
}

/// `MAX_COMBINED_TEXTURE_IMAGE_UNITS` for the one GL context this process
/// uses. Cached because `GlStateGuard` runs on every frame.
static TEXTURE_UNIT_COUNT: std::sync::OnceLock<usize> = std::sync::OnceLock::new();

impl<'a> GlStateGuard<'a> {
    fn capture(gl: &'a glow::Context) -> Self {
        unsafe {
            let mut viewport = [0; 4];
            gl.get_parameter_i32_slice(glow::VIEWPORT, &mut viewport);
            let mut scissor = [0; 4];
            gl.get_parameter_i32_slice(glow::SCISSOR_BOX, &mut scissor);
            let mut clear_color = [0.0; 4];
            gl.get_parameter_f32_slice(glow::COLOR_CLEAR_VALUE, &mut clear_color);
            let active_texture = gl.get_parameter_i32(glow::ACTIVE_TEXTURE);
            // A property of the context, not of the frame: querying it per
            // frame added a driver round trip to every single render.
            let unit_count = *TEXTURE_UNIT_COUNT.get_or_init(|| {
                gl.get_parameter_i32(glow::MAX_COMBINED_TEXTURE_IMAGE_UNITS)
                    .max(1) as usize
            });
            let mut texture_2d_bindings = Vec::with_capacity(unit_count);
            for unit in 0..unit_count {
                gl.active_texture(glow::TEXTURE0 + unit as u32);
                texture_2d_bindings.push(gl.get_parameter_texture(glow::TEXTURE_BINDING_2D));
            }
            gl.active_texture(active_texture as u32);
            Self {
                gl,
                draw_framebuffer: gl.get_parameter_framebuffer(glow::DRAW_FRAMEBUFFER_BINDING),
                read_framebuffer: gl.get_parameter_framebuffer(glow::READ_FRAMEBUFFER_BINDING),
                viewport,
                scissor_enabled: gl.is_enabled(glow::SCISSOR_TEST),
                scissor,
                program: gl.get_parameter_program(glow::CURRENT_PROGRAM),
                vertex_array: gl.get_parameter_vertex_array(glow::VERTEX_ARRAY_BINDING),
                array_buffer: gl.get_parameter_buffer(glow::ARRAY_BUFFER_BINDING),
                element_buffer: gl.get_parameter_buffer(glow::ELEMENT_ARRAY_BUFFER_BINDING),
                active_texture,
                texture_2d_bindings,
                blend_enabled: gl.is_enabled(glow::BLEND),
                blend_equation_rgb: gl.get_parameter_i32(glow::BLEND_EQUATION_RGB),
                blend_equation_alpha: gl.get_parameter_i32(glow::BLEND_EQUATION_ALPHA),
                blend_src_rgb: gl.get_parameter_i32(glow::BLEND_SRC_RGB),
                blend_dst_rgb: gl.get_parameter_i32(glow::BLEND_DST_RGB),
                blend_src_alpha: gl.get_parameter_i32(glow::BLEND_SRC_ALPHA),
                blend_dst_alpha: gl.get_parameter_i32(glow::BLEND_DST_ALPHA),
                clear_color,
                dither_enabled: gl.is_enabled(glow::DITHER),
                color_mask: gl.get_parameter_bool_array::<4>(glow::COLOR_WRITEMASK),
                pack_alignment: gl.get_parameter_i32(glow::PACK_ALIGNMENT),
                pack_row_length: gl.get_parameter_i32(glow::PACK_ROW_LENGTH),
                pack_skip_pixels: gl.get_parameter_i32(glow::PACK_SKIP_PIXELS),
                pack_skip_rows: gl.get_parameter_i32(glow::PACK_SKIP_ROWS),
                unpack_alignment: gl.get_parameter_i32(glow::UNPACK_ALIGNMENT),
                unpack_row_length: gl.get_parameter_i32(glow::UNPACK_ROW_LENGTH),
                unpack_skip_pixels: gl.get_parameter_i32(glow::UNPACK_SKIP_PIXELS),
                unpack_skip_rows: gl.get_parameter_i32(glow::UNPACK_SKIP_ROWS),
                debug_output: gl.supports_debug() && gl.is_enabled(glow::DEBUG_OUTPUT),
                debug_output_synchronous: gl.supports_debug()
                    && gl.is_enabled(glow::DEBUG_OUTPUT_SYNCHRONOUS),
            }
        }
    }
}

impl Drop for GlStateGuard<'_> {
    fn drop(&mut self) {
        unsafe {
            self.gl
                .bind_framebuffer(glow::DRAW_FRAMEBUFFER, self.draw_framebuffer);
            self.gl
                .bind_framebuffer(glow::READ_FRAMEBUFFER, self.read_framebuffer);
            self.gl.viewport(
                self.viewport[0],
                self.viewport[1],
                self.viewport[2],
                self.viewport[3],
            );
            set_enabled(self.gl, glow::SCISSOR_TEST, self.scissor_enabled);
            self.gl.scissor(
                self.scissor[0],
                self.scissor[1],
                self.scissor[2],
                self.scissor[3],
            );
            self.gl.use_program(self.program);
            self.gl.bind_vertex_array(self.vertex_array);
            self.gl.bind_buffer(glow::ARRAY_BUFFER, self.array_buffer);
            self.gl
                .bind_buffer(glow::ELEMENT_ARRAY_BUFFER, self.element_buffer);
            for (unit, texture) in self.texture_2d_bindings.iter().enumerate() {
                self.gl.active_texture(glow::TEXTURE0 + unit as u32);
                self.gl.bind_texture(glow::TEXTURE_2D, *texture);
            }
            self.gl.active_texture(self.active_texture as u32);
            set_enabled(self.gl, glow::BLEND, self.blend_enabled);
            self.gl.blend_equation_separate(
                self.blend_equation_rgb as u32,
                self.blend_equation_alpha as u32,
            );
            self.gl.blend_func_separate(
                self.blend_src_rgb as u32,
                self.blend_dst_rgb as u32,
                self.blend_src_alpha as u32,
                self.blend_dst_alpha as u32,
            );
            self.gl.clear_color(
                self.clear_color[0],
                self.clear_color[1],
                self.clear_color[2],
                self.clear_color[3],
            );
            set_enabled(self.gl, glow::DITHER, self.dither_enabled);
            self.gl.color_mask(
                self.color_mask[0],
                self.color_mask[1],
                self.color_mask[2],
                self.color_mask[3],
            );
            self.gl
                .pixel_store_i32(glow::PACK_ALIGNMENT, self.pack_alignment);
            self.gl
                .pixel_store_i32(glow::PACK_ROW_LENGTH, self.pack_row_length);
            self.gl
                .pixel_store_i32(glow::PACK_SKIP_PIXELS, self.pack_skip_pixels);
            self.gl
                .pixel_store_i32(glow::PACK_SKIP_ROWS, self.pack_skip_rows);
            self.gl
                .pixel_store_i32(glow::UNPACK_ALIGNMENT, self.unpack_alignment);
            self.gl
                .pixel_store_i32(glow::UNPACK_ROW_LENGTH, self.unpack_row_length);
            self.gl
                .pixel_store_i32(glow::UNPACK_SKIP_PIXELS, self.unpack_skip_pixels);
            self.gl
                .pixel_store_i32(glow::UNPACK_SKIP_ROWS, self.unpack_skip_rows);
            if self.gl.supports_debug() {
                set_enabled(self.gl, glow::DEBUG_OUTPUT, self.debug_output);
                set_enabled(
                    self.gl,
                    glow::DEBUG_OUTPUT_SYNCHRONOUS,
                    self.debug_output_synchronous,
                );
            }
        }
    }
}

unsafe fn set_enabled(gl: &glow::Context, capability: u32, enabled: bool) {
    if enabled {
        unsafe { gl.enable(capability) };
    } else {
        unsafe { gl.disable(capability) };
    }
}
