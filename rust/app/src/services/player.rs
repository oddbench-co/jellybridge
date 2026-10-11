//! Serialized access to libmpv's ordinary client API.
//!
//! The render context deliberately lives elsewhere on the window's thread, inside egui paint callbacks.
//! No normal client call is made by the render notifier or mpv callbacks.

use std::ffi::CString;
use std::marker::PhantomData;
use std::path::PathBuf;
use std::ptr::NonNull;
use std::rc::Rc;
use std::sync::mpsc::{self, Receiver, RecvTimeoutError, Sender};
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant};

use libmpv2::events::{Event, PropertyData};
use libmpv2::{Format, Mpv, mpv_end_file_reason};
use libmpv2_sys as sys;

const EVENT_POLL_INTERVAL: Duration = Duration::from_millis(20);
const PROGRESS_POLL_INTERVAL: Duration = Duration::from_millis(250);

#[derive(Debug)]
pub enum PlayerCommand {
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
    Error(String),
}

pub struct PlayerWorker {
    sender: Sender<PlayerCommand>,
    thread: Option<JoinHandle<()>>,
}

/// Checked owner for the client handle used only to create the render context.
///
/// libmpv2 6.0.0's `Mpv::create_client` constructs `NonNull` with
/// `new_unchecked`, although `mpv_create_client` explicitly returns NULL on
/// error. Keeping this small wrapper at the FFI seam avoids undefined behavior
/// while leaving normal commands and events on libmpv2's safe API.
pub struct RenderClient {
    context: NonNull<sys::mpv_handle>,
    _thread_bound: PhantomData<Rc<()>>,
}

impl RenderClient {
    pub(crate) fn as_ptr(&self) -> *mut sys::mpv_handle {
        self.context.as_ptr()
    }
}

impl Drop for RenderClient {
    fn drop(&mut self) {
        unsafe { sys::mpv_destroy(self.context.as_ptr()) };
    }
}

#[derive(Debug, thiserror::Error)]
pub enum PlayerCreateError {
    #[error(transparent)]
    Mpv(#[from] libmpv2::Error),
    #[error("libmpv refused to create the dedicated render client")]
    RenderClientUnavailable,
    #[error("failed to start the serialized player worker")]
    WorkerThread(#[source] std::io::Error),
}

impl PlayerWorker {
    pub fn command_sender(&self) -> Sender<PlayerCommand> {
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

/// Creates a primary handle owned exclusively by the serialized worker and a
/// checked secondary client retained by the render thread. Observer bootstrap
/// completes before either handle is used concurrently.
pub fn create_player(
    event_sink: impl Fn(PlayerEvent) + Send + 'static,
) -> Result<(RenderClient, PlayerWorker), PlayerCreateError> {
    // `config = false` below means mpv reads no configuration file, so without
    // this there is no way to exercise the software decode path on demand --
    // which is how a fault specific to it went unnoticed. Matches the
    // `JELLYBRIDGE_GL_DEBUG` / `JELLYBRIDGE_GL_SENTINEL` convention in
    // `services/render.rs`.
    let hwdec = std::env::var("JELLYBRIDGE_HWDEC").unwrap_or_else(|_| "auto-safe".to_owned());
    tracing::info!(%hwdec, "requested libmpv hardware decoding mode");
    let core = Mpv::with_initializer(|initializer| {
        initializer.set_option("vo", "libmpv")?;
        initializer.set_option("hwdec", hwdec.as_str())?;
        initializer.set_option("config", false)?;
        initializer.set_option("terminal", false)?;
        initializer.set_option("input-default-bindings", false)?;
        initializer.set_option("osc", false)?;
        initializer.set_option("keep-open", true)?;
        // Debug builds only: automated test runs play real media, and must
        // not make sound on the machine they run on.
        #[cfg(debug_assertions)]
        if std::env::var_os("JELLYBRIDGE_MUTE").is_some() {
            initializer.set_option("ao", "null")?;
        }
        // mpv starts a Lua VM on its own thread for each of its built-in
        // scripts. They exist for mpv's own OSD and command line, none of which
        // this app has: the UI is egui, playback is driven through the command
        // API, and the URL always comes from the loopback proxy, so ytdl can
        // only ever be dead weight. Measured: seven threads and their VMs.
        // `config = false` above does not cover these -- they are built in, not
        // loaded from the config directory.
        initializer.set_option("ytdl", false)?;
        initializer.set_option("load-scripts", false)?;
        initializer.set_option("load-stats-overlay", false)?;
        initializer.set_option("load-console", false)?;
        initializer.set_option("load-select", false)?;
        initializer.set_option("load-positioning", false)?;
        initializer.set_option("load-commands", false)?;
        initializer.set_option("load-context-menu", false)?;
        initializer.set_option("load-auto-profiles", false)?;
        Ok(())
    })?;
    configure_observers(&core)?;

    let render_name = CString::new("jellybridge_render").expect("static client name");
    let render_context = unsafe { sys::mpv_create_client(core.ctx.as_ptr(), render_name.as_ptr()) };
    let render_client = RenderClient {
        context: NonNull::new(render_context).ok_or(PlayerCreateError::RenderClientUnavailable)?,
        _thread_bound: PhantomData,
    };
    let (sender, receiver) = mpsc::channel();
    let thread = thread::Builder::new()
        .name("jellybridge-mpv".to_owned())
        .spawn(move || worker_loop(core, receiver, event_sink))
        .map_err(PlayerCreateError::WorkerThread)?;

    Ok((
        render_client,
        PlayerWorker {
            sender,
            thread: Some(thread),
        },
    ))
}

struct PendingLoad {
    start_seconds: f64,
    subtitles: Vec<PlayerSubtitleFile>,
    audio_source_index: Option<i64>,
    subtitle_source_index: Option<i64>,
}

fn worker_loop(mpv: Mpv, receiver: Receiver<PlayerCommand>, event_sink: impl Fn(PlayerEvent)) {
    let mut pending_load: Option<PendingLoad> = None;
    let mut last_progress_poll = Instant::now()
        .checked_sub(PROGRESS_POLL_INTERVAL)
        .unwrap_or_else(Instant::now);
    loop {
        match receiver.recv_timeout(EVENT_POLL_INTERVAL) {
            Ok(PlayerCommand::Shutdown) | Err(RecvTimeoutError::Disconnected) => break,
            Ok(command) => {
                if let Err(error) = apply_command(&mpv, command, &mut pending_load, &event_sink) {
                    event_sink(PlayerEvent::Error(format!(
                        "player command failed: {error}"
                    )));
                }
            }
            Err(RecvTimeoutError::Timeout) => {}
        }

        while let Some(event) = mpv.wait_event(0.0) {
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
                        .get_property::<String>("hwdec-current")
                        .map(|value| classify_hwdec(&value))
                        .unwrap_or(HardwareDecodePath::Software);
                    tracing::info!(decode_path = %hardware, "classified libmpv decode path");
                    event_sink(PlayerEvent::Loaded);
                    event_sink(PlayerEvent::HardwarePath(hardware));
                    event_sink(read_tracks(&mpv));
                    event_sink(read_chapters(&mpv));
                }
                Ok(Event::PropertyChange {
                    name: "pause",
                    change: PropertyData::Flag(value),
                    ..
                }) => {
                    event_sink(PlayerEvent::Paused(value));
                }
                Ok(Event::PropertyChange {
                    name: "volume",
                    change: PropertyData::Double(value),
                    ..
                }) => {
                    event_sink(PlayerEvent::Volume(value));
                }
                Ok(Event::PropertyChange {
                    name: "mute",
                    change: PropertyData::Flag(value),
                    ..
                }) => {
                    event_sink(PlayerEvent::Muted(value));
                }
                Ok(Event::PropertyChange {
                    name: "speed",
                    change: PropertyData::Double(value),
                    ..
                }) => {
                    event_sink(PlayerEvent::Speed(value));
                }
                Ok(Event::PropertyChange {
                    name: "paused-for-cache",
                    change: PropertyData::Flag(value),
                    ..
                }) => {
                    event_sink(PlayerEvent::Buffering(value));
                }
                Ok(Event::PropertyChange {
                    name: "hwdec-current",
                    change: PropertyData::Str(value),
                    ..
                }) => {
                    event_sink(PlayerEvent::HardwarePath(classify_hwdec(value)));
                }
                // With keep-open, a file that plays to the end stays open and
                // reports `eof-reached` instead; END_FILE then only means the
                // file was stopped or replaced (leaving, or the next episode),
                // which must not read as "ended" -- that starts autoplay.
                Ok(Event::EndFile(reason)) => {
                    tracing::info!(reason, "libmpv ended media");
                    if reason == mpv_end_file_reason::Eof {
                        event_sink(PlayerEvent::Ended);
                    } else if reason == mpv_end_file_reason::Error {
                        event_sink(PlayerEvent::Error("The stream could not be played".into()));
                    }
                }
                Ok(Event::PropertyChange {
                    name: "eof-reached",
                    change: PropertyData::Flag(true),
                    ..
                }) => {
                    tracing::info!("libmpv reached the end of the media");
                    event_sink(PlayerEvent::Ended);
                }
                Ok(Event::Shutdown) => return,
                Ok(_) => {}
                Err(error) => {
                    event_sink(PlayerEvent::Error(format!("player event failed: {error}")))
                }
            }
        }

        if last_progress_poll.elapsed() >= PROGRESS_POLL_INTERVAL {
            last_progress_poll = Instant::now();
            if let (Ok(seconds), Ok(duration)) = (
                mpv.get_property::<f64>("time-pos"),
                mpv.get_property::<f64>("duration"),
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

    let _ = mpv.command("stop", &[]);
}

fn configure_observers(mpv: &Mpv) -> Result<(), libmpv2::Error> {
    mpv.disable_deprecated_events()?;
    mpv.observe_property("pause", Format::Flag, 1)?;
    mpv.observe_property("volume", Format::Double, 2)?;
    mpv.observe_property("hwdec-current", Format::String, 3)?;
    mpv.observe_property("mute", Format::Flag, 4)?;
    mpv.observe_property("speed", Format::Double, 5)?;
    mpv.observe_property("paused-for-cache", Format::Flag, 6)?;
    mpv.observe_property("eof-reached", Format::Flag, 7)?;
    Ok(())
}

fn apply_command(
    mpv: &Mpv,
    command: PlayerCommand,
    pending_load: &mut Option<PendingLoad>,
    event_sink: &impl Fn(PlayerEvent),
) -> Result<(), libmpv2::Error> {
    match command {
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
            mpv.set_property("speed", 1.0)?;
            mpv.command("loadfile", &[&uri, "replace"])
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
            mpv.command(
                "sub-add",
                &[
                    path,
                    sub_add_flag(preferred_source_index, &subtitle),
                    subtitle.title.as_deref().unwrap_or(""),
                    subtitle.language.as_deref().unwrap_or(""),
                ],
            )?;
            // The track list is the only thing the subtitle menu is built
            // from, so it has to be re-read for the new entry to appear.
            event_sink(read_tracks(mpv));
            Ok(())
        }
        PlayerCommand::TogglePaused => mpv.command("cycle", &["pause"]),
        PlayerCommand::SeekRelative(seconds) => {
            mpv.command("seek", &[&seconds.to_string(), "relative+exact"])
        }
        PlayerCommand::SeekAbsolute(seconds) => {
            mpv.command("seek", &[&seconds.max(0.0).to_string(), "absolute+exact"])
        }
        PlayerCommand::SetVolume(volume) => mpv.set_property("volume", volume.clamp(0.0, 100.0)),
        PlayerCommand::ToggleMuted => mpv.command("cycle", &["mute"]),
        PlayerCommand::SetSpeed(speed) => mpv.set_property("speed", speed.clamp(0.25, 4.0)),
        PlayerCommand::SetAspect(mode) => {
            let (keep_aspect, panscan) = mode.mpv_properties();
            mpv.set_property("keepaspect", keep_aspect)?;
            mpv.set_property("panscan", panscan)
        }
        PlayerCommand::SelectAudio(id) => mpv.set_property("aid", id),
        PlayerCommand::SelectSubtitle(Some(id)) => mpv.set_property("sid", id),
        PlayerCommand::SelectSubtitle(None) => mpv.set_property("sid", "no"),
        PlayerCommand::Stop => mpv.command("stop", &[]),
        PlayerCommand::Shutdown => Ok(()),
    }
}

/// Reads the loaded file's chapter list through the same indexed
/// sub-property mechanism as [`read_tracks`] (libmpv2 exposes no node API).
fn read_chapters(mpv: &Mpv) -> PlayerEvent {
    let count = mpv.get_property::<i64>("chapter-list/count").unwrap_or(0);
    let mut chapters = Vec::new();
    for index in 0..count.max(0) {
        let Ok(time) = mpv.get_property::<f64>(&format!("chapter-list/{index}/time")) else {
            continue;
        };
        if !time.is_finite() || time < 0.0 {
            continue;
        }
        let title = mpv
            .get_property::<String>(&format!("chapter-list/{index}/title"))
            .ok();
        chapters.push(PlayerChapter { time, title });
    }
    chapters.sort_by(|a, b| a.time.total_cmp(&b.time));
    PlayerEvent::Chapters(chapters)
}

fn read_tracks(mpv: &Mpv) -> PlayerEvent {
    let count = mpv.get_property::<i64>("track-list/count").unwrap_or(0);
    let mut audio = Vec::new();
    let mut subtitles = Vec::new();
    for index in 0..count.max(0) {
        let prefix = format!("track-list/{index}");
        let Ok(id) = mpv.get_property::<i64>(&format!("{prefix}/id")) else {
            continue;
        };
        let Ok(kind) = mpv.get_property::<String>(&format!("{prefix}/type")) else {
            continue;
        };
        let language = mpv.get_property::<String>(&format!("{prefix}/lang")).ok();
        let title = mpv.get_property::<String>(&format!("{prefix}/title")).ok();
        let source_index = mpv.get_property::<i64>(&format!("{prefix}/ff-index")).ok();
        let external_filename = mpv
            .get_property::<String>(&format!("{prefix}/external-filename"))
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
        selected_audio: mpv.get_property::<i64>("aid").ok(),
        selected_subtitle: mpv.get_property::<i64>("sid").ok(),
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

fn finish_load(mpv: &Mpv, load: PendingLoad) -> Result<(), libmpv2::Error> {
    if load.start_seconds > 0.0 {
        mpv.command("seek", &[&load.start_seconds.to_string(), "absolute+exact"])?;
    }
    for subtitle in &load.subtitles {
        if let Some(path) = subtitle.path.to_str() {
            let title = subtitle.title.as_deref().unwrap_or("");
            let language = subtitle.language.as_deref().unwrap_or("");
            let flag = sub_add_flag(load.subtitle_source_index, subtitle);
            mpv.command("sub-add", &[path, flag, title, language])?;
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
        mpv.set_property("aid", track.id)?;
    }
    match load.subtitle_source_index {
        Some(-1) => mpv.set_property("sid", "no")?,
        Some(source_index) => {
            let external_path = load
                .subtitles
                .iter()
                .find(|subtitle| subtitle.source_index == Some(source_index))
                .and_then(|subtitle| subtitle.path.to_str());
            if let Some(track) = track_for_source(&tracks.1, source_index, external_path) {
                mpv.set_property("sid", track.id)?;
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
