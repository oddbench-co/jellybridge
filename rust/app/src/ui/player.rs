//! Player screen: the mpv event → view model mapping, presenters, hotkeys,
//! and the drawing of the video surface and its Jellyfin-style chrome.

use std::sync::{Arc, Mutex};

use egui::{Color32, CursorIcon, Id, Rect, Sense, Ui, pos2, vec2};

use super::app::UiAction;
use super::icons::{self, Icon};
use super::jf;
use super::player_hotkeys::{
    PLAYBACK_SPEEDS, PlayerHotkeyAction, PlayerUiState, chapter_target, key_text, next_track_index,
    resolve_player_hotkey, speed_index, stepped_speed,
};
use super::{Screen, UiState, ends_at_from_now, format_playback_time, theme, widgets};
use crate::app::{AppContext, PlayerTrackState};
use crate::features::now_playing::PlayerOverlay;
use crate::features::playback::PlayerTitle;
use crate::services::player::{PlayerEvent, PlayerTrack};

pub const SPEED_LABELS: [&str; 11] = [
    "0.5x", "0.75x", "1x", "1.25x", "1.5x", "1.75x", "2x", "2.5x", "3x", "3.5x", "4.0x",
];
pub const ASPECT_LABELS: [&str; 3] = ["Auto", "Cover", "Fill"];
/// How long the chrome stays up after the last activity.
const CONTROLS_IDLE_SECONDS: f64 = 3.0;
/// How long a click waits to be sure it is not the first of a double click.
const SINGLE_CLICK_SECONDS: f64 = 0.22;

/// Which pop-up menu is open. Settings drills into speed, aspect and
/// repeat.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum PlayerMenu {
    #[default]
    None,
    Audio,
    Subtitles,
    Settings,
    Speed,
    Aspect,
    Repeat,
}

/// jellyfin-web's Playback Info panel, while open.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct PlaybackStats {
    pub video_size: (i64, i64),
    pub dropped_frames: i64,
    /// "Original Media Info" from the server; empty while it loads.
    pub original: Vec<(String, String)>,
}

/// jellyfin-web's repeat modes.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum RepeatMode {
    All,
    One,
    #[default]
    None,
}

impl RepeatMode {
    pub const ALL: [Self; 3] = [Self::All, Self::One, Self::None];

    pub fn label(self) -> &'static str {
        match self {
            Self::All => "Repeat all",
            Self::One => "Repeat one",
            Self::None => "None",
        }
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct ChapterMarker {
    pub time: f64,
    pub title: String,
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct UpNext {
    pub title: String,
    pub info: Vec<super::detail::MiscItem>,
    pub countdown: i32,
    pub episode: bool,
    pub autoplay: bool,
}

/// What the player screen shows, plus the chrome's own state.
pub struct PlayerVm {
    pub status: String,
    pub title: String,
    /// The playing item, for the favourite button.
    pub item_id: String,
    pub favorite: bool,
    /// Jellyfin's streams for the playing item: (index, kind, label).
    pub streams: Vec<(i64, String, String)>,
    /// jellyfin-web's "remaining time" toggle on the duration label.
    pub show_remaining: bool,
    pub decode_label: String,
    pub paused: bool,
    pub muted: bool,
    pub loading: bool,
    pub buffering: bool,
    pub ends_at_label: String,
    pub has_previous: bool,
    pub has_next: bool,
    pub skip_label: String,
    pub skip_target: f64,
    /// When the current skip prompt appeared: jellyfin-web hides it after
    /// 8 s unless the controls are up.
    pub skip_since: Option<f64>,
    /// The user's skip lengths (Playback settings), in seconds.
    pub skip_back: f64,
    pub skip_forward: f64,
    pub up_next: Option<UpNext>,
    pub speed_index: usize,
    pub aspect_index: usize,
    pub volume: f32,
    pub position: f64,
    pub duration: f64,
    pub position_label: String,
    pub duration_label: String,
    pub chapters: Vec<ChapterMarker>,
    pub audio_options: Vec<String>,
    pub subtitle_options: Vec<String>,
    pub audio_index: i32,
    pub subtitle_index: i32,
    /// Fullscreen and overlay visibility.
    pub ui: PlayerUiState,
    pub menu: PlayerMenu,
    /// The button the open menu came from; its sheet is centred on it.
    pub menu_anchor: Rect,
    pub repeat: RepeatMode,
    pub stats: Option<PlaybackStats>,
    /// When the chrome hides unless something holds it up.
    pub hide_at: Option<f64>,
    /// When a single click on the video arrived, pending a double click.
    pub pending_click: Option<f64>,
    /// The timeline's playhead: follows `position` except while scrubbing.
    pub timeline_position: f64,
    pub scrubbing: bool,
    /// Whether the pointer rests on the chrome, which holds it up.
    pub chrome_hovered: bool,
}

impl Default for PlayerVm {
    fn default() -> Self {
        Self {
            status: String::new(),
            title: String::new(),
            item_id: String::new(),
            favorite: false,
            streams: Vec::new(),
            show_remaining: true,
            decode_label: String::new(),
            paused: false,
            muted: false,
            loading: false,
            buffering: false,
            ends_at_label: String::new(),
            has_previous: false,
            has_next: false,
            skip_label: String::new(),
            skip_target: 0.0,
            skip_since: None,
            skip_back: 10.0,
            skip_forward: 30.0,
            up_next: None,
            speed_index: 2,
            aspect_index: 0,
            volume: 100.0,
            position: 0.0,
            duration: 0.0,
            position_label: "0:00".to_owned(),
            duration_label: "0:00".to_owned(),
            chapters: Vec::new(),
            audio_options: Vec::new(),
            subtitle_options: Vec::new(),
            audio_index: 0,
            subtitle_index: 0,
            ui: PlayerUiState::default(),
            menu: PlayerMenu::None,
            menu_anchor: Rect::NOTHING,
            repeat: RepeatMode::None,
            stats: None,
            hide_at: None,
            pending_click: None,
            timeline_position: 0.0,
            scrubbing: false,
            chrome_hovered: false,
        }
    }
}

impl PlayerVm {
    /// The fraction of the timeline drawn as played.
    pub fn played_ratio(&self) -> f32 {
        if self.duration > 0.0 {
            (self.timeline_position / self.duration).clamp(0.0, 1.0) as f32
        } else {
            0.0
        }
    }

    fn sync_timeline(&mut self) {
        if !self.scrubbing {
            self.timeline_position = self.position.clamp(0.0, self.duration.max(1.0));
        }
    }

    /// Shows the chrome and cursor and restarts the idle countdown.
    pub fn activity(&mut self, now: f64) {
        self.ui.pointer_activity();
        self.hide_at = Some(now + CONTROLS_IDLE_SECONDS);
    }

    fn controls_visible(&self) -> bool {
        self.ui.overlay_visible
    }
}

/// mpv's tracks as Jellyfin lists them: for each of Jellyfin's `kind`
/// streams, the embedded track with its index, else the downloaded copy
/// (titled with Jellyfin's label), relabelled with Jellyfin's display title.
/// Without any match (no stream list yet) mpv's tracks are kept as they are.
fn jellyfin_tracks(
    streams: &[(i64, String, String)],
    kind: &str,
    tracks: Vec<PlayerTrack>,
) -> Vec<PlayerTrack> {
    let mapped: Vec<PlayerTrack> = streams
        .iter()
        .filter(|(_, stream_kind, _)| stream_kind == kind)
        .filter_map(|(index, _, label)| {
            let embedded = tracks.iter().find(|track| {
                track.external_filename.is_none() && track.source_index == Some(*index)
            });
            let external = || {
                tracks.iter().find(|track| {
                    track.external_filename.is_some()
                        && track.title.as_deref() == Some(label.as_str())
                })
            };
            let mut track = embedded.or_else(external)?.clone();
            track.label.clone_from(label);
            Some(track)
        })
        .collect();
    if mapped.is_empty() { tracks } else { mapped }
}

/// Applies one mpv worker event. Runs on the UI thread.
pub fn apply_player_event(
    state: &mut UiState,
    event: PlayerEvent,
    track_state: &Arc<Mutex<PlayerTrackState>>,
) {
    let player = &mut state.player;
    match event {
        PlayerEvent::Loaded => player.status.clear(),
        PlayerEvent::Paused(paused) => player.paused = paused,
        PlayerEvent::Volume(volume) => player.volume = volume as f32,
        PlayerEvent::Muted(muted) => player.muted = muted,
        PlayerEvent::Speed(speed) => player.speed_index = speed_index(speed),
        PlayerEvent::Buffering(buffering) => player.buffering = buffering,
        PlayerEvent::Position { seconds, duration } => {
            player.loading = false;
            player.position = seconds;
            player.duration = duration;
            player.position_label = format_playback_time(seconds);
            player.duration_label = format_playback_time(duration);
            let speed = PLAYBACK_SPEEDS
                .get(player.speed_index)
                .copied()
                .unwrap_or(1.0);
            player.ends_at_label = ends_at_from_now((duration - seconds) / speed);
            player.sync_timeline();
        }
        PlayerEvent::Tracks {
            audio,
            subtitles,
            selected_audio,
            selected_subtitle,
        } => {
            // The menus list Jellyfin's streams under its labels, each
            // played by the mpv track carrying it; mpv's own extras (tracks
            // Jellyfin skips, downloaded copies of embedded ones) stay out.
            let audio = jellyfin_tracks(&player.streams, "Audio", audio);
            let subtitles = jellyfin_tracks(&player.streams, "Subtitle", subtitles);
            let selected_audio_index = audio
                .iter()
                .position(|track| Some(track.id) == selected_audio)
                .unwrap_or(0);
            let selected_subtitle_index = subtitles
                .iter()
                .position(|track| Some(track.id) == selected_subtitle)
                .map(|index| index + 1)
                .unwrap_or(0);
            player.audio_options = audio.iter().map(|track| track.label.clone()).collect();
            player.subtitle_options = std::iter::once("Off".to_owned())
                .chain(subtitles.iter().map(|track| track.label.clone()))
                .collect();
            if let Ok(mut tracks) = track_state.lock() {
                tracks.audio.clone_from(&audio);
                tracks.subtitles.clone_from(&subtitles);
            }
            player.audio_index = i32::try_from(selected_audio_index).unwrap_or(i32::MAX);
            player.subtitle_index = i32::try_from(selected_subtitle_index).unwrap_or(i32::MAX);
        }
        PlayerEvent::Chapters(chapters) => {
            player.chapters = chapters
                .into_iter()
                .map(|chapter| ChapterMarker {
                    time: chapter.time,
                    title: chapter.title.unwrap_or_default(),
                })
                .collect();
        }
        PlayerEvent::HardwarePath(path) => {
            tracing::info!(decode_path = %path, "application playback decode path");
            player.decode_label = format!("Decode path: {path}");
        }
        PlayerEvent::Stats {
            width,
            height,
            dropped_frames,
        } => {
            if let Some(stats) = &mut player.stats {
                stats.video_size = (width, height);
                stats.dropped_frames = dropped_frames;
            }
        }
        PlayerEvent::Ended => player.status = "Playback ended".to_owned(),
        // Only here for the repaint every player event brings: that frame
        // builds the render context, or releases it after the player is left.
        PlayerEvent::RendererAvailable | PlayerEvent::Idle => {}
        PlayerEvent::Error(message) => {
            tracing::error!(%message, "application player failed");
            player.status = message;
        }
    }
}

/// [`apply_player_overlay`] from any thread.
pub fn present_overlay(context: &AppContext, overlay: PlayerOverlay) {
    context
        .ui
        .update(move |state| apply_player_overlay(state, &overlay));
}

/// Shows the previous/next buttons once the neighbouring episodes are known.
pub fn present_neighbours(context: &AppContext, has_previous: bool, has_next: bool) {
    context.ui.update(move |state| {
        state.player.has_previous = has_previous;
        state.player.has_next = has_next;
    });
}

/// The player stays up, spinning, while the next item's stream is prepared.
pub fn present_player_loading(context: &AppContext) {
    context.ui.update(|state| {
        state.player.loading = true;
        state.player.status.clear();
        apply_player_overlay(state, &PlayerOverlay::default());
    });
}

/// Shows or clears the skip-segment button and the Up Next card.
pub fn apply_player_overlay(state: &mut UiState, overlay: &PlayerOverlay) {
    let now = state.ctx.input(|input| input.time);
    let player = &mut state.player;
    match &overlay.skip {
        Some(skip) => {
            if player.skip_label != skip.label || player.skip_since.is_none() {
                player.skip_since = Some(now);
            }
            player.skip_label = skip.label.to_owned();
            player.skip_target = skip.target;
        }
        None => {
            player.skip_label.clear();
            player.skip_since = None;
        }
    }
    player.up_next = overlay.up_next.as_ref().map(|up_next| UpNext {
        title: up_next.title.clone(),
        info: up_next.info.clone(),
        countdown: up_next.seconds_left.round() as i32,
        episode: up_next.episode,
        autoplay: up_next.autoplay,
    });
}

/// Enters the player screen after a stream was prepared and handed to mpv.
/// The user's skip lengths, from their Playback settings.
pub fn present_skip_lengths(context: &AppContext, back: f64, forward: f64) {
    context.ui.update(move |state| {
        state.player.skip_back = back;
        state.player.skip_forward = forward;
    });
}

pub fn present_playback_started(
    context: &AppContext,
    title: PlayerTitle,
    initial_position: f64,
    initial_duration: f64,
) {
    context.ui.update(move |state| {
        let now = state.ctx.input(|input| input.time);
        let player = &mut state.player;
        player.title = title.title;
        player.item_id = title.item_id;
        player.favorite = title.favorite;
        player.streams = title
            .streams
            .into_iter()
            .map(|stream| (i64::from(stream.index), stream.kind, stream.label))
            .collect();
        player.loading = true;
        player.buffering = false;
        player.has_previous = false;
        player.has_next = false;
        player.speed_index = 2;
        player.ends_at_label.clear();
        player.decode_label.clear();
        player.position_label = format_playback_time(initial_position);
        player.duration_label = format_playback_time(initial_duration);
        // The previous item's chapter marks must not linger while the next
        // stream loads; mpv reports the new list at FileLoaded.
        player.chapters.clear();
        player.paused = false;
        player.duration = initial_duration.max(0.0);
        player.position = initial_position.clamp(0.0, player.duration.max(1.0));
        player.scrubbing = false;
        player.timeline_position = player.position;
        player.menu = PlayerMenu::None;
        player.stats = None;
        player.pending_click = None;
        player.status.clear();
        // Shows the chrome and starts its idle timer, as any activity does.
        player.activity(now);
        apply_player_overlay(state, &PlayerOverlay::default());
        state.screen = Screen::Player;
        state.busy = false;
        state.message.clear();
        // Focus belongs to the player's hotkeys, not a field on the page
        // the user came from.
        state.ctx.memory_mut(|memory| {
            if let Some(focused) = memory.focused() {
                memory.surrender_focus(focused);
            }
        });
    });
}

pub fn present_playback_failed(context: &AppContext, message: String) {
    context.ui.update(move |state| {
        state.busy = false;
        // Also on the player, for a failure while moving to the next item.
        state.player.loading = false;
        state.player.status.clone_from(&message);
        state.message = message;
    });
}

/// Turns or leaves fullscreen and shows the chrome, which the change moves.
pub fn toggle_fullscreen(state: &mut UiState, now: f64) {
    let fullscreen = !state.player.ui.fullscreen;
    set_fullscreen(state, fullscreen, now);
}

pub fn set_fullscreen(state: &mut UiState, fullscreen: bool, now: f64) {
    state.player.ui.set_fullscreen(fullscreen);
    state.player.activity(now);
    state
        .ctx
        .send_viewport_cmd(egui::ViewportCommand::Fullscreen(fullscreen));
}

/// Handles the player's hotkeys for this frame, consuming the key events it
/// acts on so no widget also reacts to them.
pub fn handle_hotkeys(ui: &Ui, state: &mut UiState, actions: &mut Vec<UiAction>) {
    let now = ui.input(|input| input.time);
    // Escape closes an open menu before it means anything else.
    if state.player.menu != PlayerMenu::None
        && ui
            .ctx()
            .input_mut(|input| input.consume_key(egui::Modifiers::NONE, egui::Key::Escape))
    {
        state.player.menu = PlayerMenu::None;
        state.player.activity(now);
    }
    let mut presses = Vec::new();
    ui.ctx().input_mut(|input| {
        input.events.retain(|event| {
            let (text, repeat, modifiers) = match event {
                egui::Event::Key {
                    key,
                    pressed: true,
                    repeat,
                    modifiers,
                    ..
                } => match key_text(*key, modifiers.shift) {
                    Some(text) => (text, *repeat, *modifiers),
                    None => return true,
                },
                egui::Event::Text(text) if text == ">" || text == "<" => {
                    (text.clone(), false, input.modifiers)
                }
                _ => return true,
            };
            let command = modifiers.ctrl || modifiers.alt || modifiers.mac_cmd || modifiers.command;
            let decision = resolve_player_hotkey(
                &text,
                repeat,
                modifiers.shift,
                command,
                state.player.ui.fullscreen,
            );
            if let Some(action) = decision.action {
                presses.push(action);
            }
            !decision.consumed
        });
    });
    for action in presses {
        apply_hotkey(state, action, now, actions);
    }
}

fn apply_hotkey(
    state: &mut UiState,
    action: PlayerHotkeyAction,
    now: f64,
    actions: &mut Vec<UiAction>,
) {
    state.player.activity(now);
    let player = &mut state.player;
    match action {
        PlayerHotkeyAction::TogglePlayback => actions.push(UiAction::TogglePlayback),
        // The plain skip keys follow the user's skip lengths.
        PlayerHotkeyAction::SeekRelative(seconds) => {
            let seconds = if seconds == -super::player_hotkeys::REWIND_SECONDS {
                -state.player.skip_back
            } else if seconds == super::player_hotkeys::FAST_FORWARD_SECONDS {
                state.player.skip_forward
            } else {
                seconds
            };
            actions.push(UiAction::SeekRelative(seconds));
        }
        PlayerHotkeyAction::SeekToStart => actions.push(UiAction::SeekAbsolute(0.0)),
        PlayerHotkeyAction::SeekToFraction(fraction) => {
            if player.duration > 0.0 {
                actions.push(UiAction::SeekAbsolute(player.duration * fraction));
            }
        }
        PlayerHotkeyAction::StepSpeed(steps) => {
            let current = PLAYBACK_SPEEDS
                .get(player.speed_index)
                .copied()
                .unwrap_or(1.0);
            let index = speed_index(stepped_speed(current, steps));
            player.speed_index = index;
            actions.push(UiAction::SetSpeed(index));
        }
        PlayerHotkeyAction::NextChapter | PlayerHotkeyAction::PreviousChapter => {
            let chapters = player
                .chapters
                .iter()
                .map(|chapter| chapter.time)
                .collect::<Vec<_>>();
            let forward = action == PlayerHotkeyAction::NextChapter;
            actions.push(chapter_action(&chapters, player, forward));
        }
        PlayerHotkeyAction::NextItem => {
            if player.has_next {
                actions.push(UiAction::PlayNext);
            }
        }
        PlayerHotkeyAction::PreviousItem => actions.push(UiAction::PlayPrevious(player.position)),
        PlayerHotkeyAction::SeekToEnd => {
            if player.duration > 0.0 {
                actions.push(UiAction::SeekAbsolute(player.duration));
            }
        }
        PlayerHotkeyAction::AdjustVolume(delta) => {
            player.volume = (player.volume + delta).clamp(0.0, 100.0);
            actions.push(UiAction::SetVolume(player.volume));
        }
        PlayerHotkeyAction::ToggleMute => actions.push(UiAction::ToggleMute),
        PlayerHotkeyAction::ToggleFullscreen | PlayerHotkeyAction::ExitFullscreen => {
            toggle_fullscreen(state, now);
        }
        PlayerHotkeyAction::CycleAudio => {
            if let Some(index) = next_track_index(player.audio_index, player.audio_options.len()) {
                player.audio_index = index;
                actions.push(UiAction::SelectAudio(index));
            }
        }
        PlayerHotkeyAction::CycleSubtitle => {
            if let Some(index) =
                next_track_index(player.subtitle_index, player.subtitle_options.len())
            {
                player.subtitle_index = index;
                actions.push(UiAction::SelectSubtitle(index));
            }
        }
        PlayerHotkeyAction::LeavePlayer => actions.push(UiAction::LeavePlayer),
    }
}

/// Runs the chrome's timers: the idle auto-hide and the pending single click.
fn run_timers(ui: &Ui, state: &mut UiState, now: f64, actions: &mut Vec<UiAction>) {
    let player = &mut state.player;
    if let Some(clicked) = player.pending_click {
        let waited = now - clicked;
        if waited >= SINGLE_CLICK_SECONDS {
            player.pending_click = None;
            actions.push(UiAction::TogglePlayback);
        } else {
            ui.ctx()
                .request_repaint_after(std::time::Duration::from_secs_f64(
                    SINGLE_CLICK_SECONDS - waited,
                ));
        }
    }
    if let Some(hide_at) = player.hide_at {
        if now >= hide_at {
            player.hide_at = None;
            let hold = player.paused || player.menu != PlayerMenu::None || player.chrome_hovered;
            player.ui.auto_hide_overlay(hold);
        } else {
            ui.ctx()
                .request_repaint_after(std::time::Duration::from_secs_f64(hide_at - now));
        }
    }
}

/// Draws the player. `video` paints the frame under the chrome.
pub fn draw(
    ui: &mut Ui,
    state: &mut UiState,
    video: Option<egui::PaintCallback>,
    actions: &mut Vec<UiAction>,
) {
    let screen = ui.max_rect();
    let now = ui.input(|input| input.time);

    // The window manager can change fullscreen behind our back (a keybind,
    // the title bar); follow what the window actually is.
    let actual_fullscreen = ui.input(|input| input.viewport().fullscreen.unwrap_or(false));
    if actual_fullscreen != state.player.ui.fullscreen {
        state.player.ui.set_fullscreen(actual_fullscreen);
        state.player.activity(now);
    }
    if ui.input(|input| input.pointer.delta() != egui::Vec2::ZERO)
        && ui.rect_contains_pointer(screen)
    {
        state.player.activity(now);
    }
    run_timers(ui, state, now, actions);

    let painter = ui.painter().clone();
    painter.rect_filled(screen, 0.0, theme::VIDEO_BG);
    if let Some(video) = video {
        painter.add(video);
    }

    // The video surface: single click toggles playback after the double-click
    // window, double click toggles fullscreen. Created first, so the chrome
    // drawn later takes priority under the pointer.
    let surface = ui.interact(screen, Id::new("player-surface"), Sense::click());
    if surface.double_clicked() {
        state.player.pending_click = None;
        toggle_fullscreen(state, now);
    } else if surface.clicked() {
        state.player.activity(now);
        state.player.pending_click = Some(now);
        ui.ctx()
            .request_repaint_after(std::time::Duration::from_secs_f64(SINGLE_CLICK_SECONDS));
    }

    if state.player.loading || state.player.buffering {
        let spinner = Rect::from_center_size(screen.center(), vec2(64.0, 64.0));
        ui.put(
            spinner,
            egui::Spinner::new().size(64.0).color(theme::TEXT_BRIGHT),
        );
    }

    let shown = state.player.controls_visible() || state.player.menu != PlayerMenu::None;
    let opacity = ui.ctx().animate_value_with_time(
        Id::new("player-chrome"),
        if shown { 1.0 } else { 0.0 },
        0.22,
    );
    let mut chrome_hovered = false;
    if opacity > 0.01 {
        chrome_hovered |= draw_top_chrome(ui, state, screen, opacity, actions);
        chrome_hovered |= draw_bottom_chrome(ui, state, screen, opacity, now, actions);
    }
    state.player.chrome_hovered = chrome_hovered;
    draw_floating(ui, state, screen, now, actions);
    draw_playback_info(ui, state, screen, actions);
    draw_menus(ui, state, screen, actions);

    if !state.player.controls_visible() && state.player.menu == PlayerMenu::None {
        ui.ctx().set_cursor_icon(CursorIcon::None);
    }
}

/// A chapter button or key: the chapter jellyfin-web would seek to, else
/// the next or previous item (the previous one restarts the first).
fn chapter_action(chapters: &[f64], player: &PlayerVm, forward: bool) -> UiAction {
    match chapter_target(chapters, player.position, forward, !player.has_previous) {
        Some(time) => UiAction::SeekAbsolute(time),
        None if forward => UiAction::PlayNext,
        None => UiAction::PlayPrevious(player.position),
    }
}

// jellyfin-web's video OSD, at 1em = 14.88 px.
const OSD_HEADER: f32 = 111.59;
const OSD_BOTTOM: f32 = 222.88;
/// From the bottom gradient's top to the controls (its top padding).
const OSD_CONTROLS_TOP: f32 = 111.6;
const OSD_BUTTON: f32 = 41.36;
const OSD_ICON: f32 = 24.84;
const OSD_BUTTON_MARGIN: f32 = 4.3152;
/// The OSD's side padding (0.8em).
const OSD_PADDING: f32 = 11.904;
/// Inline buttons are parted by a space as well as their margins.
const OSD_SPACE: f32 = 3.87;
const OSD_FONT: f32 = 14.88;
const OSD_LINE: f32 = 20.08;
/// A slider's container (1.25em) and its thumb (1.08em).
const SLIDER_HEIGHT: f32 = 18.59;
const SLIDER_THUMB: f32 = 16.07;
const SLIDER_TRACK: f32 = 2.976;
const SLIDER_BUBBLE_FONT: f32 = 1.8 * OSD_FONT;
const OSD_SCRIM: Color32 = Color32::from_rgb(15, 15, 15);

/// One of the OSD's icon buttons at `left` on the row starting at `top`.
fn osd_button(ui: &mut Ui, left: f32, top: f32, icon: Icon, label: &str, color: Color32) -> bool {
    jf::round_icon_button(
        ui,
        Rect::from_min_size(pos2(left, top), vec2(OSD_BUTTON, OSD_BUTTON)),
        icon,
        color,
        OSD_ICON,
        label,
        true,
        Id::new(("player-button", label)),
    )
    .clicked()
}

/// One line of OSD text whose 20 px line box starts at `top`.
fn osd_text(ui: &Ui, left: f32, top: f32, text: &str, color: Color32) -> Rect {
    let galley = ui
        .painter()
        .layout_no_wrap(text.to_owned(), theme::regular(OSD_FONT), color);
    widgets::galley_on_baseline(
        ui.painter(),
        left,
        jf::baseline(top, OSD_FONT, OSD_LINE),
        galley,
    )
}

fn text_width(ui: &Ui, text: &str, size: f32) -> f32 {
    ui.painter()
        .layout_no_wrap(text.to_owned(), theme::regular(size), Color32::WHITE)
        .size()
        .x
}

/// jellyfin-web's OSD header: the back arrow and the title over a fading
/// top gradient. Returns whether the pointer rests on it.
fn draw_top_chrome(
    ui: &mut Ui,
    state: &mut UiState,
    screen: Rect,
    opacity: f32,
    actions: &mut Vec<UiAction>,
) -> bool {
    let rect = Rect::from_min_size(screen.min, vec2(screen.width(), OSD_HEADER));
    // Hovering the chrome holds it up, and clicks on its empty background
    // don't fall through to the video surface.
    let backing = ui.interact(rect, Id::new("player-top-chrome"), Sense::click());
    let mut ui = ui.new_child(egui::UiBuilder::new().max_rect(screen));
    ui.multiply_opacity(opacity);
    widgets::vertical_gradient(
        ui.painter(),
        rect,
        theme::with_alpha(OSD_SCRIM, 0.75),
        theme::with_alpha(OSD_SCRIM, 0.0),
    );
    let header_text = Color32::from_gray(238);
    if osd_button(
        &mut ui,
        rect.left() + 16.2,
        rect.top() + 17.23,
        Icon::ArrowBack,
        "Back",
        header_text,
    ) {
        state.player.activity(ui.input(|input| input.time));
        actions.push(UiAction::LeavePlayer);
    }
    let size = 1.17 * OSD_FONT;
    let left = rect.left() + 70.58;
    let galley = widgets::elided(
        &ui,
        &state.player.title,
        theme::regular(size),
        header_text,
        (rect.right() - OSD_PADDING - left).max(0.0),
    );
    widgets::galley_on_baseline(
        ui.painter(),
        left,
        jf::baseline(rect.top() + 23.13, size, 1.7 * size),
        galley,
    );
    backing.hovered()
}

/// jellyfin-web's OSD controls over a bottom gradient: the time row (time,
/// position slider, remaining time) above the buttons. Returns whether the
/// pointer rests on them.
fn draw_bottom_chrome(
    ui: &mut Ui,
    state: &mut UiState,
    screen: Rect,
    opacity: f32,
    now: f64,
    actions: &mut Vec<UiAction>,
) -> bool {
    let rect = Rect::from_min_max(
        pos2(screen.left(), screen.bottom() - OSD_BOTTOM),
        screen.max,
    );
    let backing = ui.interact(rect, Id::new("player-bottom-chrome"), Sense::click());
    let mut ui = ui.new_child(egui::UiBuilder::new().max_rect(screen));
    ui.multiply_opacity(opacity);
    let ui = &mut ui;
    widgets::vertical_gradient(
        ui.painter(),
        rect,
        theme::with_alpha(OSD_SCRIM, 0.0),
        theme::with_alpha(OSD_SCRIM, 0.75),
    );
    let white = Color32::WHITE;
    let controls = rect.top() + OSD_CONTROLS_TOP;
    let row = controls + 10.416;

    // The time row.
    let player = &state.player;
    let shown_position = if player.scrubbing {
        player.timeline_position
    } else {
        player.position
    };
    let position_text = format_playback_time(shown_position);
    let end_text = if player.show_remaining {
        format!(
            "-{}",
            format_playback_time(player.duration - shown_position)
        )
    } else {
        format_playback_time(player.duration)
    };
    let text_top = row + 4.82;
    let start = osd_text(
        ui,
        rect.left() + OSD_PADDING + 7.44,
        text_top,
        &position_text,
        white,
    );
    let end_width = text_width(ui, &end_text, OSD_FONT);
    let end_left = rect.right() - OSD_PADDING - end_width;
    osd_text(ui, end_left, text_top, &end_text, white);
    let end_box = Rect::from_min_size(
        pos2(end_left - 7.44, text_top),
        vec2(end_width + 7.44, OSD_LINE),
    );
    let end_response = ui.interact(end_box, Id::new("player-duration"), Sense::click());
    widgets::pointer_on_hover(&end_response);
    if end_response.clicked() {
        state.player.show_remaining = !state.player.show_remaining;
    }
    let slider = Rect::from_min_max(
        pos2(start.right() + 3.72, row + 7.43),
        pos2(end_box.left() - 3.72, row + 7.43 + SLIDER_HEIGHT),
    );
    draw_timeline(ui, state, slider, now, actions);

    // The buttons.
    let top = row + 29.75 + 3.72;
    let player = &state.player;
    let chapters = player.chapters.len() > 1;
    let mut clicked = Vec::new();
    let mut x = rect.left() + OSD_PADDING + OSD_BUTTON_MARGIN;
    let pitch = OSD_BUTTON + 2.0 * OSD_BUTTON_MARGIN + OSD_SPACE;
    let mut last_right = x;
    let mut button = |ui: &mut Ui, icon: Icon, label: &str| {
        let pressed = osd_button(ui, x, top, icon, label, white);
        last_right = x + OSD_BUTTON;
        x += pitch;
        pressed
    };
    if player.has_previous && button(ui, Icon::SkipPrevious, "Previous track") {
        clicked.push(UiAction::PlayPrevious(player.position));
    }
    if chapters && button(ui, Icon::Undo, "Previous chapter") {
        let times = player
            .chapters
            .iter()
            .map(|chapter| chapter.time)
            .collect::<Vec<_>>();
        clicked.push(chapter_action(&times, player, false));
    }
    if button(ui, Icon::FastRewind, "Rewind") {
        clicked.push(UiAction::SeekRelative(-player.skip_back));
    }
    let (icon, label) = if player.paused {
        (Icon::Play, "Play")
    } else {
        (Icon::Pause, "Pause")
    };
    if button(ui, icon, label) {
        clicked.push(UiAction::TogglePlayback);
    }
    if button(ui, Icon::FastForward, "Fast Forward") {
        clicked.push(UiAction::SeekRelative(player.skip_forward));
    }
    if chapters && button(ui, Icon::Redo, "Next chapter") {
        let times = player
            .chapters
            .iter()
            .map(|chapter| chapter.time)
            .collect::<Vec<_>>();
        clicked.push(chapter_action(&times, player, true));
    }
    if player.has_next && button(ui, Icon::SkipNext, "Next track") {
        clicked.push(UiAction::PlayNext);
    }
    if !player.ends_at_label.is_empty() {
        // jellyfin-web puts four no-break spaces before "Ends at".
        let lead = text_width(ui, "\u{a0}\u{a0}\u{a0}\u{a0}", OSD_FONT);
        osd_text(
            ui,
            last_right + OSD_BUTTON_MARGIN + OSD_FONT + lead,
            top + 10.64,
            &player.ends_at_label,
            white,
        );
    }

    // Right side, from the right edge: fullscreen, settings, then the
    // volume group, audio, subtitles and the favourite.
    let (fullscreen, muted) = (player.ui.fullscreen, player.muted);
    let step = OSD_BUTTON + 2.0 * OSD_BUTTON_MARGIN;
    let full_left = rect.right() - OSD_PADDING - OSD_BUTTON_MARGIN - OSD_BUTTON;
    let (full_icon, full_label) = if fullscreen {
        (Icon::FullscreenExit, "Exit Fullscreen")
    } else {
        (Icon::Fullscreen, "Fullscreen")
    };
    let toggle_full = osd_button(ui, full_left, top, full_icon, full_label, white);
    let settings_left = full_left - step;
    let settings = osd_button(ui, settings_left, top, Icon::Settings, "Settings", white);
    let volume_right = settings_left - OSD_BUTTON_MARGIN - OSD_FONT;
    let volume_rect = Rect::from_min_size(
        pos2(volume_right - 133.91, top + 11.39),
        vec2(133.91, SLIDER_HEIGHT),
    );
    draw_volume(ui, state, volume_rect, now, actions);
    let mute_left = volume_rect.left() - OSD_BUTTON_MARGIN - OSD_BUTTON;
    let mute_icon = if muted {
        Icon::VolumeOff
    } else {
        Icon::VolumeUp
    };
    let mute = osd_button(
        ui,
        mute_left,
        top,
        mute_icon,
        if muted { "Unmute" } else { "Mute" },
        white,
    );
    let mut right = mute_left - 3.0 * OSD_BUTTON_MARGIN;
    // Each returns its rect when pressed, for the menu it opens.
    let mut next = |ui: &mut Ui, icon: Icon, label: &str, color: Color32| {
        let left = right - OSD_BUTTON;
        right = left - 2.0 * OSD_BUTTON_MARGIN;
        osd_button(ui, left, top, icon, label, color)
            .then(|| Rect::from_min_size(pos2(left, top), vec2(OSD_BUTTON, OSD_BUTTON)))
    };
    let player = &state.player;
    let audio = if player.audio_options.len() > 1 {
        next(ui, Icon::Audiotrack, "Audio", white)
    } else {
        None
    };
    let subtitles = if player.subtitle_options.len() > 1 {
        next(ui, Icon::ClosedCaption, "Subtitles", white)
    } else {
        None
    };
    let favorite_color = if player.favorite {
        theme::JF_PLAYED_RED
    } else {
        white
    };
    let favorite = !player.item_id.is_empty()
        && next(ui, Icon::Favorite, "Favorite", favorite_color).is_some();

    if toggle_full {
        toggle_fullscreen(state, now);
    }
    if settings {
        state.player.menu = PlayerMenu::Settings;
        state.player.menu_anchor =
            Rect::from_min_size(pos2(settings_left, top), vec2(OSD_BUTTON, OSD_BUTTON));
    }
    if let Some(anchor) = subtitles {
        state.player.menu = PlayerMenu::Subtitles;
        state.player.menu_anchor = anchor;
    }
    if let Some(anchor) = audio {
        state.player.menu = PlayerMenu::Audio;
        state.player.menu_anchor = anchor;
    }
    if mute {
        clicked.push(UiAction::ToggleMute);
    }
    if favorite {
        let player = &mut state.player;
        player.favorite = !player.favorite;
        clicked.push(UiAction::SetFavorite {
            id: player.item_id.clone(),
            favorite: player.favorite,
        });
    }
    if toggle_full
        || settings
        || subtitles.is_some()
        || audio.is_some()
        || favorite
        || !clicked.is_empty()
    {
        state.player.activity(now);
    }
    actions.extend(clicked);
    backing.hovered() || ui.rect_contains_pointer(rect)
}

/// A jellyfin-web slider in its 1.25em container: the 0.2em track inset by
/// half a thumb, filled to `ratio` in the accent, and the thumb. Returns the
/// track's ends.
fn paint_slider(
    painter: &egui::Painter,
    rect: Rect,
    ratio: f32,
    buffered: Option<(f32, f32)>,
    thumb_scale: f32,
) -> (f32, f32) {
    let x0 = rect.left() + SLIDER_THUMB / 2.0;
    let x1 = rect.right() - SLIDER_THUMB / 2.0;
    let center = rect.top() + SLIDER_HEIGHT / 2.0;
    let track = Rect::from_min_max(
        pos2(x0, center - SLIDER_TRACK / 2.0),
        pos2(x1, center + SLIDER_TRACK / 2.0),
    );
    painter.rect_filled(track, 0.0, theme::with_alpha(Color32::WHITE, 0.3));
    if let Some((from, to)) = buffered {
        let from = x0 + (x1 - x0) * from.clamp(0.0, 1.0);
        let to = x0 + (x1 - x0) * to.clamp(0.0, 1.0);
        painter.rect_filled(
            Rect::from_min_max(pos2(from, track.top()), pos2(to, track.bottom())),
            0.0,
            theme::with_alpha(Color32::WHITE, 0.4),
        );
    }
    let head = x0 + (x1 - x0) * ratio.clamp(0.0, 1.0);
    painter.rect_filled(
        Rect::from_min_max(track.min, pos2(head, track.bottom())),
        0.0,
        theme::JF_ACCENT,
    );
    painter.circle_filled(
        pos2(head, center),
        SLIDER_THUMB * thumb_scale / 2.0,
        theme::JF_ACCENT,
    );
    (x0, x1)
}

/// The position slider: track, chapter markers, thumb, and the time
/// bubble over the pointer while hovered.
fn draw_timeline(
    ui: &mut Ui,
    state: &mut UiState,
    rect: Rect,
    now: f64,
    actions: &mut Vec<UiAction>,
) {
    let response = ui.interact(rect, Id::new("player-timeline"), Sense::click_and_drag());
    widgets::describe_button(&response, true, "Seek");
    let x0 = rect.left() + SLIDER_THUMB / 2.0;
    let x1 = rect.right() - SLIDER_THUMB / 2.0;
    let player = &mut state.player;
    let pointer = response.hover_pos().or(response.interact_pointer_pos());
    let hover_ratio = pointer.map_or(0.0, |pointer| {
        ((pointer.x - x0) / (x1 - x0).max(1.0)).clamp(0.0, 1.0)
    });
    let hover_time = f64::from(hover_ratio) * player.duration.max(1.0);
    let pressed = response.is_pointer_button_down_on() || response.dragged();

    if response.drag_started() || (pressed && !player.scrubbing) {
        player.scrubbing = true;
        player.timeline_position = hover_time;
    }
    if response.dragged() || (pressed && player.scrubbing) {
        player.timeline_position = hover_time;
        player.activity(now);
    }
    // A click seeks even when its press and release came in one frame, so
    // no scrub began.
    if response.clicked() || (response.drag_stopped() && player.scrubbing) {
        player.activity(now);
        if response.clicked() {
            player.timeline_position = hover_time;
        }
        // The scrub lands where it was released; mpv's next position event
        // takes over from there.
        player.position = player.timeline_position;
        player.scrubbing = false;
        actions.push(UiAction::SeekAbsolute(player.timeline_position));
    }

    let painter = ui.painter().clone();
    let played = player.played_ratio();
    let scale = if response.hovered() || pressed {
        1.3
    } else {
        1.0
    };
    // Chapter markers sit under the thumb: 2×12 px, accent once passed.
    let duration = player.duration.max(1.0);
    for chapter in &player.chapters {
        let ratio = (chapter.time / duration) as f32;
        if !(0.0..=1.0).contains(&ratio) {
            continue;
        }
        let color = if ratio <= played {
            theme::JF_ACCENT
        } else {
            theme::with_alpha(Color32::WHITE, 0.3)
        };
        painter.rect_filled(
            Rect::from_min_size(
                pos2(x0 + (x1 - x0) * ratio - 1.0, rect.top() + 3.0),
                vec2(2.0, 12.0),
            ),
            0.0,
            color,
        );
    }
    paint_slider(&painter, rect, played, None, scale);

    if response.hovered() || pressed {
        let galley = painter.layout_no_wrap(
            format_playback_time(hover_time),
            theme::regular(SLIDER_BUBBLE_FONT),
            Color32::WHITE,
        );
        let line = 36.14;
        let size = vec2(
            galley.size().x + 1.5 * SLIDER_BUBBLE_FONT,
            line + SLIDER_BUBBLE_FONT,
        );
        let center = x0 + (x1 - x0) * hover_ratio;
        let bubble =
            Rect::from_min_size(pos2(center - size.x / 2.0, rect.top() - 1.2 * size.y), size);
        let layer = egui::LayerId::new(egui::Order::Tooltip, Id::new("player-bubble"));
        let top_painter = ui.ctx().layer_painter(layer);
        top_painter.rect_filled(bubble, 0.0, Color32::from_gray(40));
        widgets::galley_on_baseline(
            &top_painter,
            bubble.left() + 0.75 * SLIDER_BUBBLE_FONT,
            jf::baseline(
                bubble.top() + 0.5 * SLIDER_BUBBLE_FONT,
                SLIDER_BUBBLE_FONT,
                line,
            ),
            galley,
        );
    }
    widgets::pointer_on_hover(&response);
}

/// The volume slider, jellyfin's: dragging it also unmutes.
fn draw_volume(
    ui: &mut Ui,
    state: &mut UiState,
    rect: Rect,
    now: f64,
    actions: &mut Vec<UiAction>,
) {
    let response = ui.interact(rect, Id::new("player-volume"), Sense::click_and_drag());
    widgets::describe_button(&response, true, "Volume");
    let x0 = rect.left() + SLIDER_THUMB / 2.0;
    let x1 = rect.right() - SLIDER_THUMB / 2.0;
    let player = &mut state.player;
    if (response.is_pointer_button_down_on() || response.dragged())
        && let Some(pointer) = response.interact_pointer_pos()
    {
        player.activity(now);
        if player.muted {
            player.muted = false;
            actions.push(UiAction::ToggleMute);
        }
        let volume = (((pointer.x - x0) / (x1 - x0)).clamp(0.0, 1.0) * 100.0).round();
        if volume != player.volume {
            player.volume = volume;
            actions.push(UiAction::SetVolume(volume));
        }
    }
    let shown = if player.muted {
        0.0
    } else {
        player.volume.clamp(0.0, 100.0) / 100.0
    };
    let scale = if response.hovered() || response.dragged() {
        1.3
    } else {
        1.0
    };
    paint_slider(ui.painter(), rect, shown, None, scale);
    widgets::pointer_on_hover(&response);
}

/// A Playback Info section: its heading (if any) and label/value rows.
type StatsSection<'a> = (Option<&'a str>, Vec<(String, String)>);

/// jellyfin-web's Playback Info (playerstats): a translucent panel at the
/// top left listing the player, the play method, the video's live numbers
/// and the original media, with a close button.
fn draw_playback_info(ui: &mut Ui, state: &mut UiState, screen: Rect, actions: &mut Vec<UiAction>) {
    let Some(stats) = &state.player.stats else {
        return;
    };
    // 84% type; sizes in its em.
    let font = 0.84 * OSD_FONT;
    let line = 17.0;
    let pixels = ui.ctx().pixels_per_point();
    let mut sections: Vec<StatsSection<'_>> = vec![
        (
            Some("Playback Info"),
            vec![
                ("Player".to_owned(), "mpv".to_owned()),
                ("Play method".to_owned(), "Direct playing".to_owned()),
            ],
        ),
        (
            None,
            vec![
                ("Protocol".to_owned(), "http".to_owned()),
                ("Stream type".to_owned(), "Video".to_owned()),
            ],
        ),
    ];
    let mut video = vec![(
        "Player dimensions".to_owned(),
        format!(
            "{}x{}",
            (screen.width() * pixels).round(),
            (screen.height() * pixels).round()
        ),
    )];
    if stats.video_size.0 > 0 && stats.video_size.1 > 0 {
        video.push((
            "Video resolution".to_owned(),
            format!("{}x{}", stats.video_size.0, stats.video_size.1),
        ));
    }
    video.push((
        "Dropped frames".to_owned(),
        stats.dropped_frames.to_string(),
    ));
    sections.push((Some("Video Info"), video));
    if !stats.original.is_empty() {
        sections.push((Some("Original Media Info"), stats.original.clone()));
    }
    let label_font = theme::regular(font);
    let value_color = Color32::from_gray(0xdd);
    let width_of = |text: &str| {
        ui.painter()
            .layout_no_wrap(text.to_owned(), label_font.clone(), Color32::WHITE)
            .size()
            .x
    };
    let row_width = |indent: f32, (label, value): &(String, String)| {
        indent + width_of(label) + 0.5 * font + width_of(value)
    };
    let content = sections
        .iter()
        .flat_map(|(name, rows)| {
            name.map(width_of)
                .into_iter()
                .chain(rows.iter().map(|row| row_width(font, row)))
        })
        .fold(0.0, f32::max)
        .min(50.0 * font);
    let height: f32 = sections
        .iter()
        .map(|(name, rows)| if name.is_some() { font + line } else { 0.0 } + rows.len() as f32 * line)
        .sum::<f32>()
        + font;
    let rect = Rect::from_min_size(
        pos2(
            screen.left() + 1.5 * OSD_FONT,
            screen.top() + 5.0 * OSD_FONT,
        ),
        vec2(font + content + 3.0 * font, height),
    );
    let layer = egui::LayerId::new(egui::Order::Foreground, Id::new("player-stats"));
    let painter = ui.ctx().layer_painter(layer);
    painter.rect_filled(
        rect,
        0.3 * OSD_FONT,
        Color32::from_rgba_unmultiplied(28, 28, 28, 204),
    );
    let mut y = rect.top();
    let left = rect.left() + font;
    for (name, rows) in &sections {
        if let Some(name) = name {
            y += font;
            let galley =
                painter.layout_no_wrap((*name).to_owned(), label_font.clone(), Color32::WHITE);
            widgets::galley_on_baseline(&painter, left, jf::baseline(y, font, line), galley);
            y += line;
        }
        for (label, value) in rows {
            let baseline = jf::baseline(y, font, line);
            let label_galley =
                painter.layout_no_wrap(label.clone(), label_font.clone(), Color32::WHITE);
            let label_width = label_galley.size().x;
            widgets::galley_on_baseline(&painter, left + font, baseline, label_galley);
            let value_galley =
                painter.layout_no_wrap(value.clone(), label_font.clone(), value_color);
            widgets::galley_on_baseline(
                &painter,
                left + font + label_width + 0.5 * font,
                baseline,
                value_galley,
            );
            y += line;
        }
    }
    let close = Rect::from_min_size(
        pos2(rect.right() - 6.72 - 34.73, rect.top() + 3.11),
        vec2(34.73, 34.73),
    );
    let mut child = ui.new_child(egui::UiBuilder::new().layer_id(layer).max_rect(screen));
    if jf::round_icon_button(
        &mut child,
        close,
        Icon::Close,
        Color32::from_gray(0xcc),
        20.87,
        "Close",
        true,
        Id::new("player-stats-close"),
    )
    .clicked()
    {
        actions.push(UiAction::TogglePlaybackInfo);
    }
}

/// jellyfin-web's skip-segment button and Up Next card.
fn draw_floating(
    ui: &mut Ui,
    state: &mut UiState,
    screen: Rect,
    now: f64,
    actions: &mut Vec<UiAction>,
) {
    let painter = ui.painter().clone();
    // The skip button: shown as its segment starts, for 8 s or while the
    // controls are up, fading in and out over 0.2 s.
    let skip_visible = !state.player.skip_label.is_empty()
        && (state.player.controls_visible()
            || state
                .player
                .skip_since
                .is_some_and(|since| now - since < 8.0));
    if let Some(since) = state.player.skip_since
        && !state.player.controls_visible()
        && now - since < 8.0
    {
        ui.ctx()
            .request_repaint_after(std::time::Duration::from_secs_f64(8.0 - (now - since)));
    }
    let skip_opacity =
        ui.ctx()
            .animate_bool_with_time(Id::new("player-skip-fade"), skip_visible, 0.2);
    if skip_opacity > 0.0 && !state.player.skip_label.is_empty() {
        let font = 1.2 * OSD_FONT;
        let color = theme::with_alpha(Color32::WHITE, 0.87 * skip_opacity);
        let galley =
            painter.layout_no_wrap(state.player.skip_label.clone(), theme::bold(font), color);
        let size = vec2(
            20.0 + galley.size().x + 3.0 + 24.0 + 20.0,
            24.0 + 1.35 * font,
        );
        let button = Rect::from_min_size(
            pos2(
                screen.right() - 96.0 - size.x,
                screen.bottom() - 128.0 - 0.3 * font - size.y,
            ),
            size,
        );
        let response = ui.interact(button, Id::new("player-skip"), Sense::click());
        widgets::describe_button(&response, true, &state.player.skip_label);
        painter.rect_filled(
            button,
            0.2 * font,
            Color32::from_gray(48).gamma_multiply(skip_opacity),
        );
        let text_width = galley.size().x;
        widgets::galley_on_baseline(
            &painter,
            button.left() + 20.0,
            jf::baseline(button.top() + 12.0, font, 1.35 * font),
            galley,
        );
        icons::paint_centered(
            &painter,
            Icon::SkipNext,
            pos2(
                button.left() + 20.0 + text_width + 3.0 + 12.0,
                button.center().y,
            ),
            24.0,
            color,
        );
        widgets::pointer_on_hover(&response);
        if response.clicked() && skip_visible {
            state.player.activity(now);
            actions.push(UiAction::SeekAbsolute(state.player.skip_target));
        }
    }

    // Up Next: a translucent card at the bottom right, fading over 0.3 s.
    let up_next = state.player.up_next.clone();
    let opacity =
        ui.ctx()
            .animate_bool_with_time(Id::new("player-up-next-fade"), up_next.is_some(), 0.3);
    let Some(up_next) = up_next else { return };
    let em = OSD_FONT;
    let content = 30.0 * em;
    let h2 = 1.5 * em;
    let h3 = 1.17 * em;
    let misc = super::detail::misc_layout(ui, &up_next.info);
    let misc_height = misc.as_ref().map_or(0.0, |(_, height)| *height);
    let button_height = 46.87;
    let height = em
        + 0.25 * h2
        + 30.13
        + 0.25 * h2
        + 0.25 * h3
        + 23.5
        + 0.5 * h3
        + misc_height
        + em
        + 0.3 * em
        + button_height
        + 0.3 * em
        + em;
    let card = Rect::from_min_size(
        pos2(
            screen.right() - 2.0 * em - content - 2.0 * em,
            screen.bottom() - 2.0 * em - height,
        ),
        vec2(content + 2.0 * em, height),
    );
    let mut ui = ui.new_child(egui::UiBuilder::new().max_rect(screen));
    ui.multiply_opacity(opacity);
    let painter = ui.painter().clone();
    ui.interact(card, Id::new("player-up-next"), Sense::click());
    painter.rect_filled(card, 0.0, Color32::from_black_alpha(179));
    let left = card.left() + em;
    let mut y = card.top() + em + 0.25 * h2;
    // "Next Episode Playing in 12 Seconds", the count in the accent; a
    // static "Next Episode" when it will not play by itself.
    let noun = if up_next.episode { "Episode" } else { "Video" };
    let mut job = egui::text::LayoutJob::default();
    let format = |color| egui::TextFormat::simple(theme::regular(h2), color);
    if up_next.autoplay {
        job.append(
            &format!("Next {noun} Playing in "),
            0.0,
            format(Color32::WHITE),
        );
        job.append(
            &format!("{} Seconds", up_next.countdown.max(0)),
            0.0,
            format(theme::JF_ACCENT),
        );
    } else {
        job.append(&format!("Next {noun}"), 0.0, format(Color32::WHITE));
    }
    let heading = painter.layout_job(job);
    widgets::galley_on_baseline(&painter, left, jf::baseline(y, h2, 30.13), heading);
    y += 30.13 + 0.25 * h2 + 0.25 * h3;
    let title = widgets::elided(
        &ui,
        &up_next.title,
        theme::regular(h3),
        Color32::WHITE,
        25.5 * em,
    );
    widgets::galley_on_baseline(&painter, left, jf::baseline(y, h3, 23.5), title);
    y += 23.5 + 0.5 * h3;
    if let Some((pieces, misc_height)) = misc {
        super::detail::draw_misc(&painter, pieces, pos2(left, y), misc_height, left + content);
        y += misc_height;
    }
    y += em + 0.3 * em;
    // The buttons sit at the right of a 29.75em row.
    let mut right = left + 29.75 * em - 0.3 * em;
    let mut clicked = [false; 2];
    for (index, label) in ["Hide", "Start Now"].into_iter().enumerate() {
        let galley = painter.layout_no_wrap(label.to_owned(), theme::bold(em), Color32::WHITE);
        let width = galley.size().x + 2.0 * em;
        let rect = Rect::from_min_size(pos2(right - width, y), vec2(width, button_height));
        right = rect.left() - 0.6 * em;
        let response = ui.interact(rect, Id::new(("up-next-button", label)), Sense::click());
        widgets::describe_button(&response, true, label);
        let fill = if response.hovered() {
            Color32::from_gray(0x50)
        } else {
            Color32::from_gray(0x40)
        };
        painter.rect_filled(rect, 0.2 * em, fill);
        widgets::galley_on_baseline(
            &painter,
            rect.left() + em,
            jf::baseline(rect.top() + 0.9 * em, em, 1.35 * em),
            galley,
        );
        widgets::pointer_on_hover(&response);
        clicked[index] = response.clicked();
    }
    if clicked[1] {
        state.player.activity(now);
        actions.push(UiAction::PlayNext);
    }
    if clicked[0] {
        state.player.activity(now);
        actions.push(UiAction::DismissUpNext);
    }
}

/// The OSD's menus, as jellyfin-web's action sheets on the button that
/// opened them: Settings (with each setting's value), its speed, aspect and
/// repeat lists, and the titled audio and subtitle lists.
fn draw_menus(ui: &mut Ui, state: &mut UiState, _screen: Rect, actions: &mut Vec<UiAction>) {
    use super::dialogs::{self, SheetItem};
    let player = &mut state.player;
    if player.menu == PlayerMenu::None {
        return;
    }
    let speed = SPEED_LABELS[player.speed_index.min(SPEED_LABELS.len() - 1)];
    let aspect = ASPECT_LABELS[player.aspect_index.min(ASPECT_LABELS.len() - 1)];
    let item = |label, checked: bool| SheetItem {
        label,
        icon: None,
        checked,
        aside: None,
    };
    let (title, items): (Option<&str>, Vec<SheetItem<'_>>) = match player.menu {
        PlayerMenu::Settings => (
            None,
            vec![
                SheetItem {
                    label: "Aspect Ratio",
                    icon: None,
                    checked: false,
                    aside: Some(aspect),
                },
                SheetItem {
                    label: "Playback Speed",
                    icon: None,
                    checked: false,
                    aside: Some(speed),
                },
                SheetItem {
                    label: "Repeat Mode",
                    icon: None,
                    checked: false,
                    aside: Some(player.repeat.label()),
                },
                item("Playback Info", false),
            ],
        ),
        PlayerMenu::Speed => (
            None,
            SPEED_LABELS
                .iter()
                .enumerate()
                .map(|(index, label)| item(label, index == player.speed_index))
                .collect(),
        ),
        PlayerMenu::Aspect => (
            None,
            ASPECT_LABELS
                .iter()
                .enumerate()
                .map(|(index, label)| item(label, index == player.aspect_index))
                .collect(),
        ),
        PlayerMenu::Repeat => (
            None,
            RepeatMode::ALL
                .iter()
                .map(|mode| item(mode.label(), *mode == player.repeat))
                .collect(),
        ),
        PlayerMenu::Audio => (
            Some("Audio"),
            player
                .audio_options
                .iter()
                .enumerate()
                .map(|(index, label)| {
                    item(label, i32::try_from(index).ok() == Some(player.audio_index))
                })
                .collect(),
        ),
        PlayerMenu::Subtitles => (
            Some("Subtitles"),
            player
                .subtitle_options
                .iter()
                .enumerate()
                .map(|(index, label)| {
                    item(
                        label,
                        i32::try_from(index).ok() == Some(player.subtitle_index),
                    )
                })
                .collect(),
        ),
        PlayerMenu::None => unreachable!("returned above"),
    };
    let id = Id::new(("player-menu", player.menu as u8));
    let (picked, close) =
        dialogs::action_sheet(ui.ctx(), id, player.menu_anchor, title, &items, None);
    drop(items);
    let player = &mut state.player;
    let menu = player.menu;
    if close {
        player.menu = PlayerMenu::None;
    }
    let Some(index) = picked else { return };
    match menu {
        PlayerMenu::Settings if index == 3 => actions.push(UiAction::TogglePlaybackInfo),
        PlayerMenu::Settings => {
            player.menu = [PlayerMenu::Aspect, PlayerMenu::Speed, PlayerMenu::Repeat][index.min(2)];
        }
        PlayerMenu::Speed => {
            player.speed_index = index;
            actions.push(UiAction::SetSpeed(index));
        }
        PlayerMenu::Aspect => {
            player.aspect_index = index;
            actions.push(UiAction::SetAspect(index));
        }
        PlayerMenu::Repeat => {
            player.repeat = RepeatMode::ALL[index.min(2)];
            actions.push(UiAction::SetRepeat(player.repeat));
        }
        PlayerMenu::Audio => {
            player.audio_index = i32::try_from(index).unwrap_or(0);
            actions.push(UiAction::SelectAudio(player.audio_index));
        }
        PlayerMenu::Subtitles => {
            player.subtitle_index = i32::try_from(index).unwrap_or(0);
            actions.push(UiAction::SelectSubtitle(player.subtitle_index));
        }
        PlayerMenu::None => {}
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_timeline_draws_the_played_fraction_and_nothing_without_a_duration() {
        let mut player = PlayerVm {
            duration: 200.0,
            position: 50.0,
            ..PlayerVm::default()
        };
        player.sync_timeline();
        assert_eq!(player.played_ratio(), 0.25);

        let unknown = PlayerVm {
            timeline_position: 30.0,
            ..PlayerVm::default()
        };
        assert_eq!(unknown.played_ratio(), 0.0);
    }

    #[test]
    fn position_events_do_not_move_the_playhead_while_scrubbing() {
        let mut player = PlayerVm {
            duration: 100.0,
            scrubbing: true,
            timeline_position: 80.0,
            ..PlayerVm::default()
        };
        player.position = 10.0;
        player.sync_timeline();
        assert_eq!(player.timeline_position, 80.0);
        player.scrubbing = false;
        player.sync_timeline();
        assert_eq!(player.timeline_position, 10.0);
    }

    #[test]
    fn menus_list_jellyfin_streams_played_by_their_mpv_tracks() {
        let track = |id: i64, source: Option<i64>, title: &str, external: bool| PlayerTrack {
            id,
            label: format!("mpv {id}"),
            source_index: source,
            language: Some("fra".to_owned()),
            title: Some(title.to_owned()),
            external_filename: external.then(|| format!("/tmp/{id}.srt")),
        };
        let streams = vec![
            (3, "Subtitle".to_owned(), "French SDH - SubRip".to_owned()),
            (4, "Subtitle".to_owned(), "French Full - SubRip".to_owned()),
            (1, "Audio".to_owned(), "French - E-AC-3".to_owned()),
        ];
        let tracks = vec![
            // Embedded: Jellyfin's stream 3, and a WebVTT track it skips.
            track(1, Some(3), "French SDH", false),
            track(2, Some(5), "French SDH - WebVTT", false),
            // Downloaded copies of streams 3 and 4.
            track(3, Some(0), "French SDH - SubRip", true),
            track(4, Some(0), "French Full - SubRip", true),
        ];
        let listed = jellyfin_tracks(&streams, "Subtitle", tracks);
        // Stream 3 plays from the file itself, stream 4 from its copy.
        assert_eq!(
            listed.iter().map(|track| track.id).collect::<Vec<_>>(),
            [1, 4]
        );
        assert_eq!(listed[0].label, "French SDH - SubRip");
        // No stream list: mpv's tracks as they are.
        assert_eq!(
            jellyfin_tracks(&[], "Audio", vec![track(7, Some(1), "x", false)]).len(),
            1
        );
    }
}
