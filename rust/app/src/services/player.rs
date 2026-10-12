//! Serialized access to libmpv's ordinary client API.
//!
//! The render context deliberately lives elsewhere on the window's thread, inside egui paint callbacks.
//! No normal client call is made by the render notifier or mpv callbacks.
//!
//! mpv does not exist until playback is first asked for: the worker thread
//! starts empty, and the first [`PlayerCommand::Prepare`] or `Load` loads
//! libmpv (see `services::libmpv`) and builds the core. A session that only
//! browses never maps mpv's ~200 libraries.

use std::marker::PhantomData;
use std::path::PathBuf;
use std::ptr::NonNull;
use std::rc::Rc;
use std::sync::mpsc::{self, Receiver, SendError, Sender, TryRecvError};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant};

use super::libmpv::{self, END_FILE_REASON_EOF, END_FILE_REASON_ERROR, Event, Mpv, Value, mpv_handle};

const PROGRESS_POLL_INTERVAL: Duration = Duration::from_millis(250);

#[derive(Debug)]
pub enum PlayerCommand {
    /// Load libmpv and build the player now, ahead of a `Load`: sent the
    /// moment playback is asked for, so the ~70 ms library load overlaps the
    /// server round trips instead of following them. A no-op once built.
    Prepare,
    Load {
        uri: String,
        start_seconds: f64,
        subtitles: Vec<PlayerSubtitleFile>,
        audio_source_index: Option<i64>,
        subtitle_source_index: Option<i64>,
    },
    /// Attaches a subtitle that finished downloading after playback began.
    AddSubtitle {
        subtitle: PlayerSubtitleFile,
        /// The subtitle preference already resolved for this playback, so the
        /// worker picks the same `sub-add` flag [`finish_load`] would have.
        preferred_source_index: Option<i64>,
    },
    TogglePaused,
    SeekRelative(f64),
    SeekAbsolute(f64),
    SetVolume(f64),
    ToggleMuted,
    SetSpeed(f64),
    SetAspect(AspectMode),
    SelectAudio(i64),
    SelectSubtitle(Option<i64>),
    Stop,
    /// Destroy mpv until playback is asked for again, once the window has
    /// released the render context. Ignored if a new file is loading.
    Unload,
    Shutdown,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PlayerTrack {
    pub id: i64,
    pub label: String,
    pub source_index: Option<i64>,
    pub language: Option<String>,
    pub title: Option<String>,
    pub external_filename: Option<String>,
}

#[derive(Clone, Debug)]
pub struct PlayerSubtitleFile {
    pub path: PathBuf,
    pub source_index: Option<i64>,
    pub language: Option<String>,
    pub title: Option<String>,
}

/// How the video fills the window, as in Jellyfin's aspect ratio menu.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum AspectMode {
    /// Letterboxed at the source aspect.
    #[default]
    Auto,
    /// Zoomed to fill, cropping the overflow.
    Cover,
    /// Stretched to the window, ignoring the source aspect.
    Fill,
}

impl AspectMode {
    pub const ALL: [Self; 3] = [Self::Auto, Self::Cover, Self::Fill];

    /// mpv's `keepaspect` and `panscan` for this mode. They are global
    /// options, so a choice carries over to the next file.
    fn mpv_properties(self) -> (bool, f64) {
        match self {
            Self::Auto => (true, 0.0),
            Self::Cover => (true, 1.0),
            Self::Fill => (false, 0.0),
        }
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct PlayerChapter {
    pub time: f64,
    pub title: Option<String>,
}

#[derive(Clone, Debug)]
pub enum HardwareDecodePath {
    Direct(String),
    CopyBack(String),
    Software,
}

impl std::fmt::Display for HardwareDecodePath {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Direct(name) => write!(formatter, "hardware/direct ({name})"),
            Self::CopyBack(name) => write!(formatter, "hardware/copy-back ({name})"),
            Self::Software => formatter.write_str("software"),
        }
    }
}

#[derive(Clone, Debug)]
pub enum PlayerEvent {
    Loaded,
    Paused(bool),
    Volume(f64),
    Muted(bool),
    Speed(f64),
    /// mpv stalled waiting for the network cache.
    Buffering(bool),
    Position {
        seconds: f64,
        duration: f64,
    },
    Tracks {
        audio: Vec<PlayerTrack>,
        subtitles: Vec<PlayerTrack>,
        selected_audio: Option<i64>,
        selected_subtitle: Option<i64>,
    },
    Chapters(Vec<PlayerChapter>),
    HardwarePath(HardwareDecodePath),
    /// The media played to its end (not stopped or replaced).
    Ended,
    /// mpv was built and its render client is waiting in the
    /// [`RenderClientSlot`]: the window should draw so the render context
    /// gets created.
    RendererAvailable,
    /// mpv stopped with nothing queued; its video output is gone, so the
    /// render context can be released without waiting on it.
    Idle,
    Error(String),
}

pub struct PlayerWorker {
    sender: PlayerSender,
    thread: Option<JoinHandle<()>>,
}

/// Owner of the client handle used only to create the render context.
/// Bound to the window's thread, which created the context from it.
pub struct RenderClient {
    context: NonNull<mpv_handle>,
    _thread_bound: PhantomData<Rc<()>>,
}

impl RenderClient {
    pub(crate) fn as_ptr(&self) -> *mut mpv_handle {
        self.context.as_ptr()
    }
}

impl Drop for RenderClient {
    fn drop(&mut self) {
        if let Some(api) = libmpv::loaded() {
            unsafe { api.destroy(self.context.as_ptr()) };
        }
    }
}

/// Hands the render client from the worker, which builds mpv, to the
/// window, which builds the render context from it.
#[derive(Clone, Default)]
pub struct RenderClientSlot(Arc<SlotInner>);

#[derive(Default)]
struct SlotInner {
    state: Mutex<ClientState>,
    /// Set by the worker when mpv has stopped with nothing queued, cleared
    /// when a load starts.
    idle: AtomicBool,
}

#[derive(Default)]
enum ClientState {
    #[default]
    Pending,
    Ready(SendHandle),
    Failed(String),
    Taken,
}

/// SAFETY: a client handle may be used from any thread; it is moved, never
/// shared, from the worker to the window.
struct SendHandle(NonNull<mpv_handle>);
unsafe impl Send for SendHandle {}

/// What [`RenderClientSlot::take`] found.
pub enum RenderClientState {
    /// mpv is not built yet (or never was asked for).
    Pending,
    Ready(RenderClient),
    Failed(String),
}

impl RenderClientSlot {
    /// Takes the client once it exists. Call on the window's thread.
    pub fn take(&self) -> RenderClientState {
        let Ok(mut state) = self.0.state.lock() else {
            return RenderClientState::Failed("render client slot poisoned".into());
        };
        match std::mem::replace(&mut *state, ClientState::Taken) {
            ClientState::Ready(handle) => RenderClientState::Ready(RenderClient {
                context: handle.0,
                _thread_bound: PhantomData,
            }),
            ClientState::Failed(message) => {
                *state = ClientState::Failed(message.clone());
                RenderClientState::Failed(message)
            }
            other => {
                *state = other;
                RenderClientState::Pending
            }
        }
    }

    fn publish(&self, state: ClientState) {
        if let Ok(mut slot) = self.0.state.lock() {
            *slot = state;
        }
    }

    /// Whether mpv has finished stopping and nothing is queued to play.
    pub fn player_idle(&self) -> bool {
        self.0.idle.load(Ordering::Acquire)
    }

    fn set_idle(&self, idle: bool) {
        self.0.idle.store(idle, Ordering::Release);
    }
}

#[derive(Debug, thiserror::Error)]
pub enum PlayerCreateError {
    #[error(transparent)]
    Mpv(#[from] libmpv::Error),
    #[error("libmpv refused to create the dedicated render client")]
    RenderClientUnavailable,
    #[error("failed to start the serialized player worker")]
    WorkerThread(#[source] std::io::Error),
}

/// Queues a command for the mpv worker and wakes it. The worker sleeps
/// inside `mpv_wait_event`, so it costs nothing while idle and reacts to a
/// command or an mpv event at once; `mpv_wakeup` is what interrupts that wait
/// for a command.
#[derive(Clone)]
pub struct PlayerSender {
    sender: Sender<PlayerCommand>,
    wake: Arc<WakeTarget>,
}

/// The worker's mpv handle, for `mpv_wakeup` only. The worker clears it
/// before the handle is destroyed, so a sender that outlives the player
/// never touches freed memory.
struct WakeTarget(Mutex<Option<NonNull<mpv_handle>>>);

// SAFETY: the pointer is only passed to `mpv_wakeup`, which libmpv documents
// as callable from any thread, and only while the mutex says it is alive.
unsafe impl Send for WakeTarget {}
unsafe impl Sync for WakeTarget {}

impl WakeTarget {
    fn wake(&self) {
        if let Ok(handle) = self.0.lock()
            && let Some(handle) = *handle
            && let Some(api) = libmpv::loaded()
        {
            unsafe { api.wakeup(handle.as_ptr()) };
        }
    }

    fn set(&self, handle: NonNull<mpv_handle>) {
        if let Ok(mut slot) = self.0.lock() {
            *slot = Some(handle);
        }
    }

    fn clear(&self) {
        if let Ok(mut handle) = self.0.lock() {
            *handle = None;
        }
    }
}

impl PlayerSender {
    pub fn send(&self, command: PlayerCommand) -> Result<(), SendError<PlayerCommand>> {
        self.sender.send(command)?;
        self.wake.wake();
        Ok(())
    }
}

impl PlayerWorker {
    pub fn command_sender(&self) -> PlayerSender {
        self.sender.clone()
    }

    pub fn shutdown(mut self) {
        let _ = self.sender.send(PlayerCommand::Shutdown);
        if let Some(thread) = self.thread.take() {
            if thread.join().is_ok() {
                tracing::info!("serialized mpv worker stopped");
            } else {
                tracing::error!("serialized mpv worker panicked during shutdown");
            }
        }
    }
}

impl Drop for PlayerWorker {
    fn drop(&mut self) {
        let _ = self.sender.send(PlayerCommand::Shutdown);
        if let Some(thread) = self.thread.take() {
            let _ = thread.join();
        }
    }
}

/// Starts the player's worker thread. It holds no mpv until playback is
/// first asked for; the render client appears in the returned slot then.
pub fn create_player(
    event_sink: impl Fn(PlayerEvent) + Send + 'static,
) -> Result<(RenderClientSlot, PlayerWorker), PlayerCreateError> {
    let slot = RenderClientSlot::default();
    let (sender, receiver) = mpsc::channel();
    let wake = Arc::new(WakeTarget(Mutex::new(None)));
    let worker_wake = Arc::clone(&wake);
    let worker_slot = slot.clone();
    let thread = thread::Builder::new()
        .name("jellybridge-mpv".to_owned())
        .spawn(move || worker_main(receiver, &worker_wake, &worker_slot, event_sink))
        .map_err(PlayerCreateError::WorkerThread)?;
    Ok((
        slot,
        PlayerWorker {
            sender: PlayerSender { sender, wake },
            thread: Some(thread),
        },
    ))
}

/// Loads libmpv and builds the core, and the render client the window needs.
fn build_mpv() -> Result<(Mpv, NonNull<mpv_handle>), PlayerCreateError> {
    let started = Instant::now();
    // `config = false` below means mpv reads no configuration file, so without
    // this there is no way to exercise the software decode path on demand --
    // which is how a fault specific to it went unnoticed. Matches the
    // `JELLYBRIDGE_GL_DEBUG` / `JELLYBRIDGE_GL_SENTINEL` convention in
    // `services/render.rs`.
    let hwdec = std::env::var("JELLYBRIDGE_HWDEC").unwrap_or_else(|_| "auto-safe".to_owned());
    tracing::info!(%hwdec, "requested libmpv hardware decoding mode");
    let core = Mpv::create()?;
    core.set_option("vo", "libmpv")?;
    core.set_option("hwdec", &hwdec)?;
    core.set_option("config", "no")?;
    core.set_option("terminal", "no")?;
    core.set_option("input-default-bindings", "no")?;
    core.set_option("osc", "no")?;
    core.set_option("keep-open", "yes")?;
    // Automated test and measurement runs play real media, and must not
    // make sound on the machine they run on.
    if std::env::var_os("JELLYBRIDGE_MUTE").is_some() {
        core.set_option("ao", "null")?;
    }
    // mpv starts a Lua VM on its own thread for each of its built-in
    // scripts. They exist for mpv's own OSD and command line, none of which
    // this app has: the UI is egui, playback is driven through the command
    // API, and the URL always comes from the loopback proxy, so ytdl can
    // only ever be dead weight. Measured: seven threads and their VMs.
    // `config = no` above does not cover these -- they are built in, not
    // loaded from the config directory.
    for option in [
        "ytdl",
        "load-scripts",
        "load-stats-overlay",
        "load-console",
        "load-select",
        "load-positioning",
        "load-commands",
        "load-context-menu",
        "load-auto-profiles",
    ] {
        core.set_option(option, "no")?;
    }
    if let Some(interop) = super::render::preferred_hwdec_interop() {
        tracing::info!(interop, "libmpv hardware-decode interop");
        core.set_option("gpu-hwdec-interop", interop)?;
    }
    // `JELLYBRIDGE_MPV_OPTIONS=name=value,name=value`: any further mpv
    // option, for measuring a setting (cache sizes, decoder frames)
    // without a rebuild. Applied last, so it overrides the above.
    if let Ok(options) = std::env::var("JELLYBRIDGE_MPV_OPTIONS") {
        for option in options.split(',').filter(|option| !option.is_empty()) {
            let (name, value) = option.split_once('=').unwrap_or((option, "yes"));
            tracing::info!(name, value, "extra libmpv option");
            core.set_option(name, value)?;
        }
    }
    core.initialize()?;
    configure_observers(&core)?;
    let render_client = core
        .create_client(c"jellybridge_render")
        .ok_or(PlayerCreateError::RenderClientUnavailable)?;
    tracing::info!(ms = started.elapsed().as_secs_f64() * 1000.0, "libmpv player built");
    Ok((core, render_client))
}

/// Before playback is first asked for there is no mpv to wait on, so the
/// worker sleeps on its channel; a command that would only act on a playing
/// file (a stop at sign-out, say) is dropped rather than loading mpv for it.
fn worker_main(
    receiver: Receiver<PlayerCommand>,
    wake: &WakeTarget,
    slot: &RenderClientSlot,
    event_sink: impl Fn(PlayerEvent),
) {
    loop {
        let command = match receiver.recv() {
            Ok(PlayerCommand::Shutdown) | Err(_) => return,
            Ok(command @ (PlayerCommand::Prepare | PlayerCommand::Load { .. })) => command,
            Ok(_) => continue,
        };
        match build_mpv() {
            Ok((mpv, render_client)) => {
                wake.set(mpv_handle_of(&mpv));
                slot.publish(ClientState::Ready(SendHandle(render_client)));
                event_sink(PlayerEvent::RendererAvailable);
                match worker_loop(mpv, command, &receiver, wake, slot, &event_sink) {
                    WorkerExit::Shutdown => return,
                    // Back to no mpv at all: the decoder's pools, its CUDA
                    // device and mpv's threads went with the core (measured
                    // ~190 MB still resident after a 4K film without this).
                    // libmpv stays loaded, so rebuilding is ~2 ms.
                    WorkerExit::Unloaded => {
                        slot.publish(ClientState::Pending);
                        trim_heap();
                        tracing::info!("libmpv player unloaded");
                    }
                }
            }
            Err(error) => {
                tracing::error!(%error, "could not start libmpv");
                slot.publish(ClientState::Failed(error.to_string()));
                event_sink(PlayerEvent::Error(format!("The player could not start: {error}")));
            }
        }
    }
}

enum WorkerExit {
    Shutdown,
    Unloaded,
}

fn mpv_handle_of(mpv: &Mpv) -> NonNull<mpv_handle> {
    NonNull::new(mpv.handle()).expect("a created mpv handle is never null")
}

struct PendingLoad {
    start_seconds: f64,
    subtitles: Vec<PlayerSubtitleFile>,
    audio_source_index: Option<i64>,
    subtitle_source_index: Option<i64>,
}

fn worker_loop(
    mpv: Mpv,
    first: PlayerCommand,
    receiver: &Receiver<PlayerCommand>,
    wake: &WakeTarget,
    slot: &RenderClientSlot,
    event_sink: &impl Fn(PlayerEvent),
) -> WorkerExit {
    let mut pending_load: Option<PendingLoad> = None;
    let mut exit = WorkerExit::Shutdown;
    if matches!(first, PlayerCommand::Load { .. }) {
        slot.set_idle(false);
    }
    if let Err(error) = apply_command(&mpv, first, &mut pending_load, event_sink) {
        event_sink(PlayerEvent::Error(format!("player command failed: {error}")));
    }
    // The position only moves while a file is loaded and not paused; only
    // then is it polled. Otherwise the thread sleeps in `mpv_wait_event` until
    // mpv has an event or `PlayerSender` wakes it for a command.
    let mut loaded = false;
    let mut paused = false;
    let mut poll_now = false;
    let mut next_progress_poll = Instant::now();
    'worker: loop {
        loop {
            match receiver.try_recv() {
                Ok(PlayerCommand::Shutdown) | Err(TryRecvError::Disconnected) => break 'worker,
                Ok(PlayerCommand::Unload) => {
                    if !loaded && pending_load.is_none() {
                        exit = WorkerExit::Unloaded;
                        break 'worker;
                    }
                }
                Ok(command) => {
                    if matches!(command, PlayerCommand::Load { .. }) {
                        slot.set_idle(false);
                    }
                    if let Err(error) =
                        apply_command(&mpv, command, &mut pending_load, event_sink)
                    {
                        event_sink(PlayerEvent::Error(format!(
                            "player command failed: {error}"
                        )));
                    }
                }
                Err(TryRecvError::Empty) => break,
            }
        }

        // Negative waits forever. After the first event the rest of the queue
        // is drained without waiting.
        let mut timeout = if loaded && !paused {
            next_progress_poll
                .saturating_duration_since(Instant::now())
                .as_secs_f64()
        } else {
            -1.0
        };
        while let Some(event) = mpv.wait_event(timeout) {
            timeout = 0.0;
            match event {
                Ok(Event::FileLoaded) => {
                    if let Some(load) = pending_load.take()
                        && let Err(error) = finish_load(&mpv, load)
                    {
                        event_sink(PlayerEvent::Error(format!(
                            "post-load setup failed: {error}"
                        )));
                    }
                    tracing::info!("libmpv loaded media");
                    let hardware = mpv
                        .get_string("hwdec-current")
                        .map(|value| classify_hwdec(&value))
                        .unwrap_or(HardwareDecodePath::Software);
                    tracing::info!(decode_path = %hardware, "classified libmpv decode path");
                    loaded = true;
                    poll_now = true;
                    event_sink(PlayerEvent::Loaded);
                    event_sink(PlayerEvent::HardwarePath(hardware));
                    event_sink(read_tracks(&mpv));
                    event_sink(read_chapters(&mpv));
                }
                Ok(Event::PropertyChange {
                    name: "pause",
                    value: Some(Value::Flag(value)),
                }) => {
                    paused = value;
                    poll_now = true;
                    event_sink(PlayerEvent::Paused(value));
                }
                Ok(Event::PropertyChange {
                    name: "volume",
                    value: Some(Value::Double(value)),
                }) => {
                    event_sink(PlayerEvent::Volume(value));
                }
                Ok(Event::PropertyChange {
                    name: "mute",
                    value: Some(Value::Flag(value)),
                }) => {
                    event_sink(PlayerEvent::Muted(value));
                }
                Ok(Event::PropertyChange {
                    name: "speed",
                    value: Some(Value::Double(value)),
                }) => {
                    event_sink(PlayerEvent::Speed(value));
                }
                Ok(Event::PropertyChange {
                    name: "paused-for-cache",
                    value: Some(Value::Flag(value)),
                }) => {
                    event_sink(PlayerEvent::Buffering(value));
                }
                Ok(Event::PropertyChange {
                    name: "hwdec-current",
                    value: Some(Value::Str(value)),
                }) => {
                    event_sink(PlayerEvent::HardwarePath(classify_hwdec(value)));
                }
                // With keep-open, a file that plays to the end stays open and
                // reports `eof-reached` instead; END_FILE then only means the
                // file was stopped or replaced (leaving, or the next episode),
                // which must not read as "ended" -- that starts autoplay.
                Ok(Event::EndFile(reason)) => {
                    tracing::info!(reason, "libmpv ended media");
                    loaded = false;
                    // Stopped with nothing queued: the demuxer cache and the
                    // decoder's buffers were just freed (~180 MB after a 4K
                    // film), but glibc keeps freed pages unless asked.
                    if pending_load.is_none() {
                        trim_heap();
                        slot.set_idle(true);
                        event_sink(PlayerEvent::Idle);
                    }
                    if reason == END_FILE_REASON_EOF {
                        event_sink(PlayerEvent::Ended);
                    } else if reason == END_FILE_REASON_ERROR {
                        event_sink(PlayerEvent::Error("The stream could not be played".into()));
                    }
                }
                Ok(Event::PropertyChange {
                    name: "eof-reached",
                    value: Some(Value::Flag(true)),
                }) => {
                    tracing::info!("libmpv reached the end of the media");
                    event_sink(PlayerEvent::Ended);
                }
                // A seek while paused moves the position without the clock
                // running: report where it landed.
                Ok(Event::PlaybackRestart) => poll_now = true,
                Ok(Event::Shutdown) => break 'worker,
                Ok(_) => {}
                Err(error) => {
                    event_sink(PlayerEvent::Error(format!("player event failed: {error}")))
                }
            }
        }
        if poll_now || (loaded && !paused && Instant::now() >= next_progress_poll) {
            poll_now = false;
            next_progress_poll = Instant::now() + PROGRESS_POLL_INTERVAL;
            if let (Ok(seconds), Ok(duration)) = (
                mpv.get_f64("time-pos"),
                mpv.get_f64("duration"),
            ) && seconds.is_finite()
                && duration.is_finite()
                && duration > 0.0
            {
                event_sink(PlayerEvent::Position {
                    seconds: seconds.clamp(0.0, duration),
                    duration,
                });
            }
        }
    }

    wake.clear();
    if matches!(exit, WorkerExit::Shutdown) {
        log_frame_drops(&mpv);
        let _ = mpv.command(&["stop"]);
    }
    // Dropping `mpv` destroys the last handle (the window destroyed the render
    // client first), which shuts the core down.
    exit
}

fn configure_observers(mpv: &Mpv) -> Result<(), libmpv::Error> {
    mpv.disable_deprecated_events()?;
    mpv.observe_flag("pause", 1)?;
    mpv.observe_double("volume", 2)?;
    mpv.observe_string("hwdec-current", 3)?;
    mpv.observe_flag("mute", 4)?;
    mpv.observe_double("speed", 5)?;
    mpv.observe_flag("paused-for-cache", 6)?;
    mpv.observe_flag("eof-reached", 7)?;
    Ok(())
}

fn apply_command(
    mpv: &Mpv,
    command: PlayerCommand,
    pending_load: &mut Option<PendingLoad>,
    event_sink: &impl Fn(PlayerEvent),
) -> Result<(), libmpv::Error> {
    match command {
        PlayerCommand::Prepare => Ok(()),
        PlayerCommand::Load {
            uri,
            start_seconds,
            subtitles,
            audio_source_index,
            subtitle_source_index,
        } => {
            *pending_load = Some(PendingLoad {
                start_seconds,
                subtitles,
                audio_source_index,
                subtitle_source_index,
            });
            // Speed is a global option; like Jellyfin, each item starts at 1x.
            mpv.set_property("speed", Value::Double(1.0))?;
            mpv.command(&["loadfile", &uri, "replace"])
        }
        PlayerCommand::AddSubtitle {
            subtitle,
            preferred_source_index,
        } => {
            // The file is still opening, so `finish_load` has not run its own
            // sub-add loop yet: hand the subtitle over rather than racing it.
            // Opening a large remux takes long enough that this is the common
            // case for the first subtitle to arrive.
            if let Some(load) = pending_load.as_mut() {
                load.subtitles.push(subtitle);
                return Ok(());
            }
            let Some(path) = subtitle.path.to_str() else {
                return Ok(());
            };
            mpv.command(&[
                    "sub-add",
                    path,
                    sub_add_flag(preferred_source_index, &subtitle),
                    subtitle.title.as_deref().unwrap_or(""),
                    subtitle.language.as_deref().unwrap_or(""),
            ])?;
            // The track list is the only thing the subtitle menu is built
            // from, so it has to be re-read for the new entry to appear.
            event_sink(read_tracks(mpv));
            Ok(())
        }
        PlayerCommand::TogglePaused => mpv.command(&["cycle", "pause"]),
        PlayerCommand::SeekRelative(seconds) => {
            mpv.command(&["seek", &seconds.to_string(), "relative+exact"])
        }
        PlayerCommand::SeekAbsolute(seconds) => {
            mpv.command(&["seek", &seconds.max(0.0).to_string(), "absolute+exact"])
        }
        PlayerCommand::SetVolume(volume) => {
            mpv.set_property("volume", Value::Double(volume.clamp(0.0, 100.0)))
        }
        PlayerCommand::ToggleMuted => mpv.command(&["cycle", "mute"]),
        PlayerCommand::SetSpeed(speed) => {
            mpv.set_property("speed", Value::Double(speed.clamp(0.25, 4.0)))
        }
        PlayerCommand::SetAspect(mode) => {
            let (keep_aspect, panscan) = mode.mpv_properties();
            mpv.set_property("keepaspect", Value::Flag(keep_aspect))?;
            mpv.set_property("panscan", Value::Double(panscan))
        }
        PlayerCommand::SelectAudio(id) => mpv.set_property("aid", Value::Int(id)),
        PlayerCommand::SelectSubtitle(Some(id)) => mpv.set_property("sid", Value::Int(id)),
        PlayerCommand::SelectSubtitle(None) => mpv.set_property("sid", Value::Str("no")),
        PlayerCommand::Stop => {
            log_frame_drops(mpv);
            mpv.command(&["stop"])
        }
        PlayerCommand::Shutdown | PlayerCommand::Unload => Ok(()),
    }
}

/// Returns freed heap pages to the system. glibc keeps them for reuse, which
/// after playback means holding the memory of a cache that no longer exists.
pub fn trim_heap() {
    #[cfg(all(target_os = "linux", target_env = "gnu"))]
    {
        let started = Instant::now();
        // SAFETY: malloc_trim only releases memory the allocator holds free.
        unsafe { libc::malloc_trim(0) };
        tracing::debug!(ms = started.elapsed().as_secs_f64() * 1000.0, "heap trimmed");
    }
}

/// Frames the decoder skipped and frames the output dropped for being late,
/// for the file that is about to stop. Zero on both is smooth playback; it is
/// what any change to caching or decoding has to keep.
fn log_frame_drops(mpv: &Mpv) {
    if let (Ok(output), Ok(decoder)) = (
        mpv.get_i64("frame-drop-count"),
        mpv.get_i64("decoder-frame-drop-count"),
    ) {
        tracing::info!(output, decoder, "libmpv dropped frames");
    }
}

/// Reads the loaded file's chapter list through the same indexed
/// sub-property mechanism as [`read_tracks`], which needs no node API.
fn read_chapters(mpv: &Mpv) -> PlayerEvent {
    let count = mpv.get_i64("chapter-list/count").unwrap_or(0);
    let mut chapters = Vec::new();
    for index in 0..count.max(0) {
        let Ok(time) = mpv.get_f64(&format!("chapter-list/{index}/time")) else {
            continue;
        };
        if !time.is_finite() || time < 0.0 {
            continue;
        }
        let title = mpv
            .get_string(&format!("chapter-list/{index}/title"))
            .ok();
        chapters.push(PlayerChapter { time, title });
    }
    chapters.sort_by(|a, b| a.time.total_cmp(&b.time));
    PlayerEvent::Chapters(chapters)
}

fn read_tracks(mpv: &Mpv) -> PlayerEvent {
    let count = mpv.get_i64("track-list/count").unwrap_or(0);
    let mut audio = Vec::new();
    let mut subtitles = Vec::new();
    for index in 0..count.max(0) {
        let prefix = format!("track-list/{index}");
        let Ok(id) = mpv.get_i64(&format!("{prefix}/id")) else {
            continue;
        };
        let Ok(kind) = mpv.get_string(&format!("{prefix}/type")) else {
            continue;
        };
        let language = mpv.get_string(&format!("{prefix}/lang")).ok();
        let title = mpv.get_string(&format!("{prefix}/title")).ok();
        let source_index = mpv.get_i64(&format!("{prefix}/ff-index")).ok();
        let external_filename = mpv
            .get_string(&format!("{prefix}/external-filename"))
            .ok();
        let label = track_label(id, language.as_deref(), title.as_deref());
        let track = PlayerTrack {
            id,
            label,
            source_index,
            language,
            title,
            external_filename,
        };
        match kind.as_str() {
            "audio" => audio.push(track),
            "sub" => subtitles.push(track),
            _ => {}
        }
    }
    PlayerEvent::Tracks {
        audio,
        subtitles,
        selected_audio: mpv.get_i64("aid").ok(),
        selected_subtitle: mpv.get_i64("sid").ok(),
    }
}

fn track_label(id: i64, language: Option<&str>, title: Option<&str>) -> String {
    let language = language.map(str::trim).filter(|value| !value.is_empty());
    let title = title.map(str::trim).filter(|value| !value.is_empty());
    match (language, title) {
        (Some(language), Some(title)) if !title.eq_ignore_ascii_case(language) => {
            format!("{language} — {title}")
        }
        (Some(language), _) => language.to_owned(),
        (_, Some(title)) => title.to_owned(),
        _ => format!("Track {id}"),
    }
}

fn finish_load(mpv: &Mpv, load: PendingLoad) -> Result<(), libmpv::Error> {
    if load.start_seconds > 0.0 {
        mpv.command(&["seek", &load.start_seconds.to_string(), "absolute+exact"])?;
    }
    for subtitle in &load.subtitles {
        if let Some(path) = subtitle.path.to_str() {
            let title = subtitle.title.as_deref().unwrap_or("");
            let language = subtitle.language.as_deref().unwrap_or("");
            let flag = sub_add_flag(load.subtitle_source_index, subtitle);
            mpv.command(&["sub-add", path, flag, title, language])?;
        }
    }
    let tracks = match read_tracks(mpv) {
        PlayerEvent::Tracks {
            audio, subtitles, ..
        } => (audio, subtitles),
        _ => unreachable!("read_tracks always returns a track event"),
    };
    if let Some(source_index) = load.audio_source_index
        && let Some(track) = tracks
            .0
            .iter()
            .find(|track| track.source_index == Some(source_index))
    {
        mpv.set_property("aid", Value::Int(track.id))?;
    }
    match load.subtitle_source_index {
        Some(-1) => mpv.set_property("sid", Value::Str("no"))?,
        Some(source_index) => {
            let external_path = load
                .subtitles
                .iter()
                .find(|subtitle| subtitle.source_index == Some(source_index))
                .and_then(|subtitle| subtitle.path.to_str());
            if let Some(track) = track_for_source(&tracks.1, source_index, external_path) {
                mpv.set_property("sid", Value::Int(track.id))?;
            }
        }
        None => {}
    }
    Ok(())
}

/// mpv's `sub-add` selection flag for a materialized subtitle.
///
/// This used to be an unconditional `auto`, which mpv defines as "select unless
/// a subtitle is already selected". That was harmless while every subtitle was
/// added before playback began, but subtitles now arrive during playback, and
/// `auto` would turn them on for someone who has them off -- mpv counts
/// `sid=no` as nothing being selected. So the decision is made here, from the
/// preference the caller already resolved, and both add paths share it.
fn sub_add_flag(
    preferred_source_index: Option<i64>,
    subtitle: &PlayerSubtitleFile,
) -> &'static str {
    match preferred_source_index {
        Some(index) if subtitle.source_index == Some(index) => "select",
        // No stored preference and no default stream: leave the pick to mpv,
        // which is what this path did before subtitles arrived late.
        None => "auto",
        _ => "cached",
    }
}

fn track_for_source<'a>(
    tracks: &'a [PlayerTrack],
    source_index: i64,
    external_path: Option<&str>,
) -> Option<&'a PlayerTrack> {
    tracks.iter().find(|track| {
        external_path.is_some_and(|path| track.external_filename.as_deref() == Some(path))
            || (external_path.is_none() && track.source_index == Some(source_index))
    })
}

fn classify_hwdec(value: &str) -> HardwareDecodePath {
    let normalized = value.trim().to_ascii_lowercase();
    if normalized.is_empty() || normalized == "no" {
        HardwareDecodePath::Software
    } else if normalized.contains("copy") {
        HardwareDecodePath::CopyBack(normalized)
    } else {
        HardwareDecodePath::Direct(normalized)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn classifies_hardware_decode_paths_without_hiding_software() {
        assert!(matches!(classify_hwdec("no"), HardwareDecodePath::Software));
        assert!(matches!(
            classify_hwdec("videotoolbox-copy"),
            HardwareDecodePath::CopyBack(_)
        ));
        assert!(matches!(
            classify_hwdec("vaapi"),
            HardwareDecodePath::Direct(_)
        ));
    }

    #[test]
    fn labels_tracks_without_assuming_language_or_title_is_present() {
        assert_eq!(track_label(2, Some("eng"), Some("Stereo")), "eng — Stereo");
        assert_eq!(track_label(3, Some("jpn"), None), "jpn");
        assert_eq!(track_label(4, None, Some("Commentary")), "Commentary");
        assert_eq!(track_label(5, None, None), "Track 5");
    }

    #[test]
    fn resolves_jellyfin_streams_without_treating_source_indexes_as_mpv_ids() {
        let tracks = vec![PlayerTrack {
            id: 42,
            label: "eng".into(),
            source_index: Some(3),
            language: Some("eng".into()),
            title: None,
            external_filename: None,
        }];

        assert_eq!(track_for_source(&tracks, 3, None).unwrap().id, 42);
        assert!(track_for_source(&tracks, 42, None).is_none());
    }

    #[test]
    fn resolves_materialized_subtitles_by_their_local_filename() {
        let tracks = vec![PlayerTrack {
            id: 17,
            label: "Signs".into(),
            source_index: Some(0),
            language: Some("eng".into()),
            title: Some("Signs".into()),
            external_filename: Some("/tmp/jellybridge/signs.srt".into()),
        }];

        assert_eq!(
            track_for_source(&tracks, 8, Some("/tmp/jellybridge/signs.srt"))
                .unwrap()
                .id,
            17
        );
    }

    fn subtitle(source_index: Option<i64>) -> PlayerSubtitleFile {
        PlayerSubtitleFile {
            path: PathBuf::from("/tmp/subtitle-0.srt"),
            source_index,
            language: None,
            title: None,
        }
    }

    #[test]
    fn a_late_subtitle_is_only_selected_when_it_is_the_one_that_was_asked_for() {
        // The subtitle the resolved preference names.
        assert_eq!(sub_add_flag(Some(2), &subtitle(Some(2))), "select");
        // A different track: added so it appears in the menu, not selected.
        assert_eq!(sub_add_flag(Some(2), &subtitle(Some(3))), "cached");
        // Subtitles off. `auto` here would turn them on mid-playback, because
        // mpv reads `sid=no` as nothing being selected.
        assert_eq!(sub_add_flag(Some(-1), &subtitle(Some(2))), "cached");
        // No preference and no default stream: unchanged from before, mpv picks.
        assert_eq!(sub_add_flag(None, &subtitle(Some(2))), "auto");
    }

    #[test]
    fn aspect_modes_map_to_mpv_geometry_options() {
        assert_eq!(AspectMode::Auto.mpv_properties(), (true, 0.0));
        assert_eq!(AspectMode::Cover.mpv_properties(), (true, 1.0));
        assert_eq!(AspectMode::Fill.mpv_properties(), (false, 0.0));
    }
}
