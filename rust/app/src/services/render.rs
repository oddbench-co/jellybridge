//! Audited Slint/OpenGL/libmpv seam for the feasibility slice.

use std::ffi::{CStr, c_char, c_void};
use std::marker::PhantomData;
use std::ptr::{self, NonNull};
use std::rc::Rc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, mpsc};
use std::thread::{self, JoinHandle};

use glow::HasContext;
use libloading::Library;
use libmpv2_sys as sys;
#[cfg(target_os = "linux")]
use raw_window_handle::{HasDisplayHandle, RawDisplayHandle};
use slint::{
    BorrowedOpenGLTextureBuilder, BorrowedOpenGLTextureOrigin, GraphicsAPI, Image, PhysicalSize,
};

use crate::services::player::RenderClient;

type RedrawRequest = Arc<dyn Fn() + Send + Sync + 'static>;

#[derive(Debug, thiserror::Error)]
pub enum RenderError {
    #[error("Slint did not provide its native OpenGL context")]
    NotNativeOpenGl,
    #[error("failed to load the platform OpenGL resolver")]
    Resolver,
    #[error("unsupported native display handle for libmpv hardware interop")]
    UnsupportedDisplay,
    #[error("libmpv render-context operation failed: {0}")]
    Mpv(String),
    #[error("OpenGL resource creation failed: {0}")]
    OpenGl(String),
    #[error("failed to start the render wake bridge")]
    WakeThread(#[source] std::io::Error),
}

#[derive(Clone, Copy)]
pub enum NativeDisplay {
    None,
    X11(*const c_void),
    Wayland(*const c_void),
}

pub fn native_display(window: &slint::Window) -> Result<NativeDisplay, RenderError> {
    #[cfg(target_os = "linux")]
    {
        let handle = window
            .window_handle()
            .display_handle()
            .map_err(|_| RenderError::UnsupportedDisplay)?;
        return match handle.as_raw() {
            RawDisplayHandle::Xlib(display) => display
                .display
                .map(|pointer| NativeDisplay::X11(pointer.as_ptr()))
                .ok_or(RenderError::UnsupportedDisplay),
            RawDisplayHandle::Wayland(display) => {
                Ok(NativeDisplay::Wayland(display.display.as_ptr()))
            }
            _ => Err(RenderError::UnsupportedDisplay),
        };
    }
    #[cfg(not(target_os = "linux"))]
    {
        let _ = window;
        Ok(NativeDisplay::None)
    }
}

pub struct RenderHost {
    mpv: Option<RawRenderContext>,
    gl: Option<glow::Context>,
    targets: Option<FrameTargets>,
    retired: Vec<(u64, FrameTargets)>,
    front: usize,
    completed_frames: u64,
    debug_enabled_by_us: bool,
    redraw: RedrawRequest,
    // This must be declared after RawRenderContext so Rust's field drop order
    // cannot destroy the client handle before its render context.
    core: RenderClient,
}

impl RenderHost {
    pub fn new(core: RenderClient, redraw: RedrawRequest) -> Self {
        Self {
            mpv: None,
            gl: None,
            targets: None,
            retired: Vec::new(),
            front: 0,
            completed_frames: 0,
            debug_enabled_by_us: false,
            redraw,
            core,
        }
    }

    pub fn setup(
        &mut self,
        graphics_api: &GraphicsAPI,
        display: NativeDisplay,
    ) -> Result<(), RenderError> {
        self.teardown(false);
        let GraphicsAPI::NativeOpenGL { get_proc_address } = graphics_api else {
            return Err(RenderError::NotNativeOpenGl);
        };
        // SAFETY: Slint documents that its GL context is current throughout the
        // rendering notifier. glow copies the resolved function pointers.
        let mut gl =
            unsafe { glow::Context::from_loader_function_cstr(|name| get_proc_address(name)) };
        let version = unsafe { gl.get_parameter_string(glow::VERSION) };
        let vendor = unsafe { gl.get_parameter_string(glow::VENDOR) };
        let renderer = unsafe { gl.get_parameter_string(glow::RENDERER) };
        tracing::info!(%version, %vendor, %renderer, "Slint OpenGL context acquired");

        let debug_requested = std::env::var_os("JELLYBRIDGE_GL_DEBUG").is_some();
        if debug_requested && gl.supports_debug() {
            let was_enabled = unsafe { gl.is_enabled(glow::DEBUG_OUTPUT) };
            unsafe {
                gl.enable(glow::DEBUG_OUTPUT);
                gl.enable(glow::DEBUG_OUTPUT_SYNCHRONOUS);
                gl.debug_message_callback(|source, kind, id, severity, message| {
                    tracing::warn!(source, kind, id, severity, message, "OpenGL debug message");
                });
            }
            self.debug_enabled_by_us = !was_enabled;
        }

        let raw = RawRenderContext::create(&self.core, display, self.redraw.clone())?;
        self.gl = Some(gl);
        self.mpv = Some(raw);
        Ok(())
    }

    pub fn render_frame(&mut self, size: PhysicalSize) -> Result<Option<Image>, RenderError> {
        let width = i32::try_from(size.width.max(1)).unwrap_or(i32::MAX);
        let height = i32::try_from(size.height.max(1)).unwrap_or(i32::MAX);
        let gl = self.gl.as_ref().ok_or(RenderError::NotNativeOpenGl)?;
        let mpv = self.mpv.as_ref().ok_or(RenderError::NotNativeOpenGl)?;

        if self
            .targets
            .as_ref()
            .is_none_or(|targets| targets.size != (width, height))
        {
            let new_targets = FrameTargets::create(gl, width, height)?;
            if let Some(old_targets) = self.targets.replace(new_targets) {
                // The old image can remain cached through this render. Release it
                // only after Slint has completed a cycle with the replacement.
                self.retired.push((self.completed_frames + 1, old_targets));
            }
            self.front = 0;
        } else {
            self.front ^= 1;
        }

        let target = &self.targets.as_ref().expect("targets initialized").frames[self.front];
        let _state = GlStateGuard::capture(gl);
        unsafe {
            gl.bind_framebuffer(glow::FRAMEBUFFER, Some(target.framebuffer));
            gl.viewport(0, 0, width, height);
            gl.disable(glow::SCISSOR_TEST);
            gl.clear_color(0.0, 0.0, 0.0, 1.0);
            gl.clear(glow::COLOR_BUFFER_BIT);
        }
        mpv.update()?;
        mpv.render(target.framebuffer.0.get() as i32, width, height)?;

        let image = unsafe {
            BorrowedOpenGLTextureBuilder::new_gl_2d_rgba_texture(
                target.texture.0,
                [width as u32, height as u32].into(),
            )
        }
        .origin(BorrowedOpenGLTextureOrigin::BottomLeft)
        .build();
        Ok(Some(image))
    }

    pub fn finish_frame(&mut self, window_size: PhysicalSize) {
        self.completed_frames = self.completed_frames.saturating_add(1);
        if self.completed_frames == 1
            && let Some(targets) = self.targets.as_ref()
        {
            tracing::info!(
                width = targets.size.0,
                height = targets.size.1,
                "libmpv rendered into a same-context borrowed texture"
            );
        }
        if let Some(gl) = self.gl.as_ref() {
            let mut retained = Vec::new();
            for (release_after, targets) in self.retired.drain(..) {
                if release_after <= self.completed_frames {
                    targets.destroy(gl);
                } else {
                    retained.push((release_after, targets));
                }
            }
            self.retired = retained;
            if std::env::var_os("JELLYBRIDGE_GL_SENTINEL").is_some()
                && (self.completed_frames == 1 || self.completed_frames.is_multiple_of(120))
            {
                verify_magenta_sentinel(gl, window_size);
            }
        }
    }

    /// Must be called from RenderingTeardown while Slint's GL context is current.
    /// If Slint still owns a borrowed image during an unexpected context loss,
    /// the dying context reclaims texture/FBO storage instead of us deleting
    /// objects that the scene can still name.
    pub fn teardown(&mut self, image_cleared: bool) {
        let had_resources = self.mpv.is_some() || self.gl.is_some();
        self.mpv.take(); // unregister callback and free mpv GL resources first
        if let Some(gl) = self.gl.take() {
            if image_cleared {
                if let Some(targets) = self.targets.take() {
                    targets.destroy(&gl);
                }
                for (_, targets) in self.retired.drain(..) {
                    targets.destroy(&gl);
                }
            } else {
                // FrameTargets intentionally has no Drop implementation. Slint
                // is tearing down this exact context, which releases its GL
                // objects after the notifier returns. A new context replaces
                // the stale component image in BeforeRendering before drawing.
                self.targets.take();
                self.retired.clear();
                tracing::warn!(
                    "borrowed image was still set during context loss; deferred texture reclamation to the GL context"
                );
            }
            if self.debug_enabled_by_us {
                unsafe {
                    gl.disable(glow::DEBUG_OUTPUT_SYNCHRONOUS);
                    gl.disable(glow::DEBUG_OUTPUT);
                }
                self.debug_enabled_by_us = false;
            }
            // glow's Drop implementation unregisters its debug callback.
            drop(gl);
        }
        if had_resources {
            tracing::info!("libmpv render context and OpenGL targets torn down");
        }
    }
}

impl Drop for RenderHost {
    fn drop(&mut self) {
        // This is only a fallback. A passing gate always reaches RenderingTeardown
        // while the GL context is current.
        if self.mpv.is_some() || self.targets.is_some() || !self.retired.is_empty() {
            tracing::error!("render host dropped without a complete RenderingTeardown");
        }
    }
}

struct FrameTarget {
    texture: glow::NativeTexture,
    framebuffer: glow::NativeFramebuffer,
}

struct FrameTargets {
    size: (i32, i32),
    frames: [FrameTarget; 2],
}

impl FrameTargets {
    fn create(gl: &glow::Context, width: i32, height: i32) -> Result<Self, RenderError> {
        let _state = GlStateGuard::capture(gl);
        let first = create_frame_target(gl, width, height)?;
        let second = match create_frame_target(gl, width, height) {
            Ok(target) => target,
            Err(error) => {
                destroy_frame_target(gl, first);
                return Err(error);
            }
        };
        Ok(Self {
            size: (width, height),
            frames: [first, second],
        })
    }

    fn destroy(self, gl: &glow::Context) {
        let _state = GlStateGuard::capture(gl);
        for target in self.frames {
            destroy_frame_target(gl, target);
        }
    }
}

fn create_frame_target(
    gl: &glow::Context,
    width: i32,
    height: i32,
) -> Result<FrameTarget, RenderError> {
    unsafe {
        let texture = gl
            .create_texture()
            .map_err(|error| RenderError::OpenGl(error.to_owned()))?;
        gl.bind_texture(glow::TEXTURE_2D, Some(texture));
        gl.tex_parameter_i32(
            glow::TEXTURE_2D,
            glow::TEXTURE_MIN_FILTER,
            glow::LINEAR as i32,
        );
        gl.tex_parameter_i32(
            glow::TEXTURE_2D,
            glow::TEXTURE_MAG_FILTER,
            glow::LINEAR as i32,
        );
        gl.tex_parameter_i32(
            glow::TEXTURE_2D,
            glow::TEXTURE_WRAP_S,
            glow::CLAMP_TO_EDGE as i32,
        );
        gl.tex_parameter_i32(
            glow::TEXTURE_2D,
            glow::TEXTURE_WRAP_T,
            glow::CLAMP_TO_EDGE as i32,
        );
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
        Ok(FrameTarget {
            texture,
            framebuffer,
        })
    }
}

fn destroy_frame_target(gl: &glow::Context, target: FrameTarget) {
    unsafe {
        gl.delete_framebuffer(target.framebuffer);
        gl.delete_texture(target.texture);
    }
}

enum RenderWake {
    Redraw,
    Shutdown,
}

struct UpdateCallbackState {
    pending: Arc<AtomicBool>,
    queued: Arc<AtomicBool>,
    sender: mpsc::Sender<RenderWake>,
}

unsafe extern "C" fn render_update_callback(context: *mut c_void) {
    if let Some(state) = NonNull::new(context.cast::<UpdateCallbackState>()) {
        // SAFETY: user data is boxed for the full registered-callback lifetime.
        let state = unsafe { state.as_ref() };
        state.pending.store(true, Ordering::Release);
        if !state.queued.swap(true, Ordering::AcqRel) {
            let _ = state.sender.send(RenderWake::Redraw);
        }
    }
}

struct RawRenderContext {
    context: NonNull<sys::mpv_render_context>,
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
        let mut resolver = Box::new(GlProcResolver::load()?);
        let resolver_pointer = (&mut *resolver as *mut GlProcResolver).cast::<c_void>();
        let mut init = sys::mpv_opengl_init_params {
            get_proc_address: Some(resolve_gl_proc),
            get_proc_address_ctx: resolver_pointer,
        };
        let mut advanced_control = 1_i32;
        let mut parameters = vec![
            sys::mpv_render_param {
                type_: sys::mpv_render_param_type_MPV_RENDER_PARAM_API_TYPE,
                data: sys::MPV_RENDER_API_TYPE_OPENGL.as_ptr().cast_mut().cast(),
            },
            sys::mpv_render_param {
                type_: sys::mpv_render_param_type_MPV_RENDER_PARAM_OPENGL_INIT_PARAMS,
                data: (&mut init as *mut sys::mpv_opengl_init_params).cast(),
            },
            sys::mpv_render_param {
                type_: sys::mpv_render_param_type_MPV_RENDER_PARAM_ADVANCED_CONTROL,
                data: (&mut advanced_control as *mut i32).cast(),
            },
        ];
        match display {
            NativeDisplay::None => {}
            NativeDisplay::X11(pointer) => parameters.push(sys::mpv_render_param {
                type_: sys::mpv_render_param_type_MPV_RENDER_PARAM_X11_DISPLAY,
                data: pointer.cast_mut(),
            }),
            NativeDisplay::Wayland(pointer) => parameters.push(sys::mpv_render_param {
                type_: sys::mpv_render_param_type_MPV_RENDER_PARAM_WL_DISPLAY,
                data: pointer.cast_mut(),
            }),
        }
        parameters.push(sys::mpv_render_param {
            type_: sys::mpv_render_param_type_MPV_RENDER_PARAM_INVALID,
            data: ptr::null_mut(),
        });

        let mut raw = ptr::null_mut();
        let status = unsafe {
            sys::mpv_render_context_create(&mut raw, core.as_ptr(), parameters.as_mut_ptr())
        };
        check_mpv(status)?;
        let context = NonNull::new(raw).ok_or_else(|| RenderError::Mpv("null context".into()))?;

        let pending = Arc::new(AtomicBool::new(true));
        let queued = Arc::new(AtomicBool::new(false));
        let (sender, receiver) = mpsc::channel();
        let wake_queued = queued.clone();
        let wake_thread = match thread::Builder::new()
            .name("jellybridge-render-wake".to_owned())
            .spawn(move || {
                while let Ok(message) = receiver.recv() {
                    match message {
                        RenderWake::Redraw => {
                            redraw();
                            wake_queued.store(false, Ordering::Release);
                        }
                        RenderWake::Shutdown => break,
                    }
                }
            }) {
            Ok(thread) => thread,
            Err(error) => {
                unsafe { sys::mpv_render_context_free(context.as_ptr()) };
                return Err(RenderError::WakeThread(error));
            }
        };
        let mut update_state = Box::new(UpdateCallbackState {
            pending,
            queued,
            sender,
        });
        unsafe {
            sys::mpv_render_context_set_update_callback(
                context.as_ptr(),
                Some(render_update_callback),
                (&mut *update_state as *mut UpdateCallbackState).cast(),
            );
        }

        Ok(Self {
            context,
            resolver,
            update_state,
            wake_thread: Some(wake_thread),
            _thread_bound: PhantomData,
        })
    }

    fn update(&self) -> Result<(), RenderError> {
        if self.update_state.pending.swap(false, Ordering::AcqRel) {
            unsafe {
                let _ = sys::mpv_render_context_update(self.context.as_ptr());
            }
        }
        Ok(())
    }

    fn render(&self, framebuffer: i32, width: i32, height: i32) -> Result<(), RenderError> {
        let mut fbo = sys::mpv_opengl_fbo {
            fbo: framebuffer,
            w: width,
            h: height,
            internal_format: glow::RGBA8 as i32,
        };
        let mut flip = 1_i32;
        let mut parameters = [
            sys::mpv_render_param {
                type_: sys::mpv_render_param_type_MPV_RENDER_PARAM_OPENGL_FBO,
                data: (&mut fbo as *mut sys::mpv_opengl_fbo).cast(),
            },
            sys::mpv_render_param {
                type_: sys::mpv_render_param_type_MPV_RENDER_PARAM_FLIP_Y,
                data: (&mut flip as *mut i32).cast(),
            },
            sys::mpv_render_param {
                type_: sys::mpv_render_param_type_MPV_RENDER_PARAM_INVALID,
                data: ptr::null_mut(),
            },
        ];
        check_mpv(unsafe {
            sys::mpv_render_context_render(self.context.as_ptr(), parameters.as_mut_ptr())
        })
    }
}

impl Drop for RawRenderContext {
    fn drop(&mut self) {
        unsafe {
            sys::mpv_render_context_set_update_callback(
                self.context.as_ptr(),
                None,
                ptr::null_mut(),
            );
        }
        let _ = self.update_state.sender.send(RenderWake::Shutdown);
        if let Some(thread) = self.wake_thread.take() {
            let _ = thread.join();
        }
        unsafe { sys::mpv_render_context_free(self.context.as_ptr()) };
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

fn check_mpv(status: i32) -> Result<(), RenderError> {
    if status >= 0 {
        return Ok(());
    }
    let message = unsafe {
        let pointer = sys::mpv_error_string(status);
        if pointer.is_null() {
            format!("error {status}")
        } else {
            CStr::from_ptr(pointer).to_string_lossy().into_owned()
        }
    };
    Err(RenderError::Mpv(message))
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
            let unit_count = gl
                .get_parameter_i32(glow::MAX_COMBINED_TEXTURE_IMAGE_UNITS)
                .max(1) as usize;
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

fn verify_magenta_sentinel(gl: &glow::Context, size: PhysicalSize) {
    const SAMPLE_EDGE: i32 = 32;
    if size.width < SAMPLE_EDGE as u32 || size.height < SAMPLE_EDGE as u32 {
        return;
    }
    let _state = GlStateGuard::capture(gl);
    let mut top = vec![0_u8; SAMPLE_EDGE as usize * SAMPLE_EDGE as usize * 4];
    let mut bottom = vec![0_u8; SAMPLE_EDGE as usize * SAMPLE_EDGE as usize * 4];
    unsafe {
        gl.bind_framebuffer(glow::READ_FRAMEBUFFER, None);
        gl.pixel_store_i32(glow::PACK_ALIGNMENT, 1);
        gl.read_pixels(
            0,
            size.height as i32 - SAMPLE_EDGE,
            SAMPLE_EDGE,
            SAMPLE_EDGE,
            glow::RGBA,
            glow::UNSIGNED_BYTE,
            glow::PixelPackData::Slice(Some(&mut top)),
        );
        gl.read_pixels(
            0,
            0,
            SAMPLE_EDGE,
            SAMPLE_EDGE,
            glow::RGBA,
            glow::UNSIGNED_BYTE,
            glow::PixelPackData::Slice(Some(&mut bottom)),
        );
    }
    if !contains_magenta(&top) && !contains_magenta(&bottom) {
        tracing::error!("post-mpv Slint sentinel did not survive GL state restoration");
    }
}

fn contains_magenta(bytes: &[u8]) -> bool {
    bytes
        .as_chunks::<4>()
        .0
        .iter()
        .any(|pixel| pixel[0] > 180 && pixel[1] < 100 && pixel[2] > 180 && pixel[3] > 180)
}
