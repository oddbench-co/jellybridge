//! Player screen: the mpv event → view model mapping, presenters, hotkeys,
//! and the drawing of the video surface and its Jellyfin-style chrome.

use std::sync::{Arc, Mutex};

use egui::{Align2, Color32, CursorIcon, Id, Rect, Sense, Ui, pos2, vec2};

use super::app::UiAction;
use super::icons::{self, Icon};
use super::player_hotkeys::{
    PLAYBACK_SPEEDS, PlayerHotkeyAction, PlayerUiState, chapter_target, key_text, next_track_index,
    resolve_player_hotkey, speed_index, stepped_speed,
};
use super::{Screen, UiState, ends_at_from_now, format_playback_time, theme, widgets};
use crate::app::{AppContext, PlayerTrackState};
use crate::features::now_playing::PlayerOverlay;
use crate::features::playback::PlayerTitle;
use crate::services::player::PlayerEvent;

pub const SPEED_LABELS: [&str; 7] = ["0.5x", "0.75x", "Normal", "1.25x", "1.5x", "1.75x", "2x"];
pub const ASPECT_LABELS: [&str; 3] = ["Auto", "Cover", "Fill"];
/// How long the chrome stays up after the last activity.
const CONTROLS_IDLE_SECONDS: f64 = 3.0;
/// How long a click waits to be sure it is not the first of a double click.
const SINGLE_CLICK_SECONDS: f64 = 0.22;
const BOTTOM_HEIGHT: f32 = 150.0;

/// Which pop-up menu is open. Settings drills into speed and aspect.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum PlayerMenu {
    #[default]
    None,
    Audio,
    Subtitles,
    Settings,
    Speed,
    Aspect,
}

#[derive(Clone, Debug, PartialEq)]
pub struct ChapterMarker {
    pub time: f64,
    pub title: String,
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct UpNext {
    pub title: String,
    pub subtitle: String,
    pub countdown: i32,
}

/// What the player screen shows, plus the chrome's own state.
pub struct PlayerVm {
    pub status: String,
    pub title: String,
    pub subtitle: String,
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
            subtitle: String::new(),
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

    /// Name of the chapter at `time`, or "".
    pub fn chapter_title_at(&self, time: f64) -> &str {
        self.chapters
            .iter()
            .take_while(|chapter| chapter.time <= time)
            .last()
            .map_or("", |chapter| chapter.title.as_str())
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
        PlayerEvent::Ended => player.status = "Playback ended".to_owned(),
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
    let player = &mut state.player;
    match &overlay.skip {
        Some(skip) => {
            player.skip_label = skip.label.to_owned();
            player.skip_target = skip.target;
        }
        None => player.skip_label.clear(),
    }
    player.up_next = overlay.up_next.as_ref().map(|up_next| UpNext {
        title: up_next.title.clone(),
        subtitle: up_next.subtitle.clone(),
        countdown: up_next.seconds_left.ceil() as i32,
    });
}

/// Enters the player screen after a stream was prepared and handed to mpv.
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
        player.subtitle = title.subtitle;
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
        PlayerHotkeyAction::SeekRelative(seconds) => actions.push(UiAction::SeekRelative(seconds)),
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
            if let Some(target) = chapter_target(&chapters, player.position, forward) {
                actions.push(UiAction::SeekAbsolute(target));
            }
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
    draw_menus(ui, state, screen, actions);

    if !state.player.controls_visible() && state.player.menu == PlayerMenu::None {
        ui.ctx().set_cursor_icon(CursorIcon::None);
    }
}

/// A round, borderless icon button like those in Jellyfin's player OSD;
/// `badge` draws a number inside the icon (the 10/30 second skips).
#[allow(clippy::too_many_arguments)]
fn player_icon_button(
    ui: &mut Ui,
    center: egui::Pos2,
    icon: Icon,
    icon_size: f32,
    label: &str,
    badge: &str,
    enabled: bool,
    opacity: f32,
) -> bool {
    let side = icon_size + 18.0;
    let rect = Rect::from_center_size(center, vec2(side, side));
    let sense = if enabled {
        Sense::click()
    } else {
        Sense::hover()
    };
    let response = ui.interact(rect, Id::new(("player-button", label)), sense);
    widgets::describe_button(&response, enabled, label);
    let painter = ui.painter();
    let target = if !enabled {
        0.0
    } else if response.is_pointer_button_down_on() {
        0.22
    } else if response.hovered() {
        0.12
    } else {
        0.0
    };
    let tint = ui
        .ctx()
        .animate_value_with_time(response.id.with("tint"), target, 0.12);
    if tint > 0.0 {
        painter.circle_filled(
            rect.center(),
            side / 2.0,
            theme::with_alpha(theme::TEXT_BRIGHT, tint * opacity),
        );
    }
    if response.has_focus() {
        painter.circle_stroke(
            rect.center(),
            side / 2.0,
            egui::Stroke::new(2.0, theme::ACCENT_BRIGHT),
        );
    }
    let color = if enabled {
        theme::TEXT_BRIGHT
    } else {
        theme::TEXT_FAINT
    };
    icons::paint_centered(
        painter,
        icon,
        rect.center(),
        icon_size,
        color.gamma_multiply(opacity),
    );
    if !badge.is_empty() {
        // The replay/forward arrows circle (12, 13) of the 24-unit viewbox.
        let y = rect.center().y - icon_size / 2.0 + icon_size * 13.0 / 24.0;
        painter.text(
            pos2(rect.center().x, y),
            Align2::CENTER_CENTER,
            badge,
            theme::bold(icon_size * 0.36),
            theme::TEXT_BRIGHT.gamma_multiply(opacity),
        );
    }
    if enabled {
        widgets::pointer_on_hover(&response);
    }
    response.on_hover_text(label).clicked()
}

/// Back, title, subtitle and status over a top scrim. Returns whether the
/// pointer rests on it.
fn draw_top_chrome(
    ui: &mut Ui,
    state: &mut UiState,
    screen: Rect,
    opacity: f32,
    actions: &mut Vec<UiAction>,
) -> bool {
    let rect = Rect::from_min_size(screen.min, vec2(screen.width(), 120.0));
    // Hovering the chrome holds it up, and clicks on its empty background
    // don't fall through to the video surface.
    let backing = ui.interact(rect, Id::new("player-top-chrome"), Sense::click());
    let painter = ui.painter().clone();
    widgets::vertical_gradient(
        &painter,
        rect,
        theme::with_alpha(theme::SCRIM, 0.75 * opacity),
        theme::with_alpha(theme::SCRIM, 0.0),
    );
    let row_center = rect.top() + 42.0;
    if player_icon_button(
        ui,
        pos2(rect.left() + 16.0 + 22.0, row_center),
        Icon::ArrowBack,
        26.0,
        "Back",
        "",
        true,
        opacity,
    ) {
        state.player.activity(ui.input(|input| input.time));
        actions.push(UiAction::LeavePlayer);
    }
    let left = rect.left() + 16.0 + 44.0 + 12.0;
    let width = rect.right() - 24.0 - left;
    let player = &state.player;
    let mut lines = vec![widgets::elided(
        ui,
        &player.title,
        theme::bold(theme::FONT_HEADING),
        theme::TEXT_BRIGHT.gamma_multiply(opacity),
        width,
    )];
    if !player.subtitle.is_empty() {
        lines.push(widgets::elided(
            ui,
            &player.subtitle,
            theme::regular(theme::FONT_BODY_LARGE),
            theme::TEXT_SECONDARY.gamma_multiply(opacity),
            width,
        ));
    }
    if !player.status.is_empty() {
        lines.push(widgets::elided(
            ui,
            &player.status,
            theme::regular(theme::FONT_BODY),
            theme::TEXT_MUTED.gamma_multiply(opacity),
            width,
        ));
    }
    let total: f32 =
        lines.iter().map(|line| line.size().y).sum::<f32>() + 2.0 * (lines.len() as f32 - 1.0);
    let mut y = row_center - total / 2.0;
    for line in lines {
        let height = line.size().y;
        painter.galley(pos2(left, y), line, Color32::WHITE);
        y += height + 2.0;
    }
    backing.hovered()
}

/// Seek bar over the transport and option buttons, on a bottom scrim.
fn draw_bottom_chrome(
    ui: &mut Ui,
    state: &mut UiState,
    screen: Rect,
    opacity: f32,
    now: f64,
    actions: &mut Vec<UiAction>,
) -> bool {
    let rect = Rect::from_min_max(
        pos2(screen.left(), screen.bottom() - BOTTOM_HEIGHT),
        screen.max,
    );
    let backing = ui.interact(rect, Id::new("player-bottom-chrome"), Sense::click());
    let painter = ui.painter().clone();
    widgets::vertical_gradient(
        &painter,
        rect,
        theme::with_alpha(theme::SCRIM, 0.0),
        theme::with_alpha(theme::SCRIM, 0.85 * opacity),
    );
    let row = Rect::from_min_max(
        pos2(rect.left() + 24.0, rect.bottom() - 14.0 - 56.0),
        pos2(rect.right() - 24.0, rect.bottom() - 14.0),
    );
    let timeline = Rect::from_min_size(
        pos2(row.left(), row.top() - 4.0 - 28.0),
        vec2(row.width(), 28.0),
    );
    draw_timeline(ui, state, timeline, opacity, now, actions);

    let center_y = row.center().y;
    let mut x = row.left();
    let button = |ui: &mut Ui,
                  x: &mut f32,
                  icon: Icon,
                  icon_size: f32,
                  label: &str,
                  badge: &str,
                  enabled: bool| {
        let side = icon_size + 18.0;
        let clicked = player_icon_button(
            ui,
            pos2(*x + side / 2.0, center_y),
            icon,
            icon_size,
            label,
            badge,
            enabled,
            opacity,
        );
        *x += side + 2.0;
        clicked
    };
    let player = &state.player;
    let (paused, has_previous, has_next, muted, volume, fullscreen) = (
        player.paused,
        player.has_previous,
        player.has_next,
        player.muted,
        player.volume,
        player.ui.fullscreen,
    );
    let mut clicked = Vec::new();
    if has_previous && button(ui, &mut x, Icon::SkipPrevious, 26.0, "Previous", "", true) {
        clicked.push(UiAction::PlayPrevious(state.player.position));
    }
    if button(
        ui,
        &mut x,
        Icon::Replay,
        32.0,
        "Rewind 10 seconds",
        "10",
        true,
    ) {
        clicked.push(UiAction::SeekRelative(-10.0));
    }
    let play_label = if paused { "Play" } else { "Pause" };
    if button(
        ui,
        &mut x,
        if paused { Icon::Play } else { Icon::Pause },
        36.0,
        play_label,
        "",
        true,
    ) {
        clicked.push(UiAction::TogglePlayback);
    }
    if button(
        ui,
        &mut x,
        Icon::Forward,
        32.0,
        "Fast forward 30 seconds",
        "30",
        true,
    ) {
        clicked.push(UiAction::SeekRelative(30.0));
    }
    if has_next && button(ui, &mut x, Icon::SkipNext, 26.0, "Next", "", true) {
        clicked.push(UiAction::PlayNext);
    }
    let clock = format!(
        "{} / {}",
        state.player.position_label, state.player.duration_label
    );
    let clock = widgets::elided(
        ui,
        &clock,
        theme::regular(theme::FONT_BODY_LARGE),
        theme::TEXT_PRIMARY.gamma_multiply(opacity),
        200.0,
    );
    widgets::galley_v_centered(&painter, x + 24.0, center_y, clock);

    // Right side, laid out from the right edge.
    let mut right = row.right();
    let right_button = |ui: &mut Ui, right: &mut f32, icon: Icon, label: &str, enabled: bool| {
        let side = 26.0 + 18.0;
        let clicked = player_icon_button(
            ui,
            pos2(*right - side / 2.0, center_y),
            icon,
            26.0,
            label,
            "",
            enabled,
            opacity,
        );
        *right -= side + 2.0;
        clicked
    };
    let fullscreen_label = if fullscreen {
        "Exit fullscreen"
    } else {
        "Fullscreen"
    };
    let fullscreen_icon = if fullscreen {
        Icon::FullscreenExit
    } else {
        Icon::Fullscreen
    };
    let toggle_full = right_button(ui, &mut right, fullscreen_icon, fullscreen_label, true);
    let settings = right_button(ui, &mut right, Icon::Settings, "Settings", true);
    let subtitles_enabled = state.player.subtitle_options.len() > 1;
    let subtitles = right_button(
        ui,
        &mut right,
        Icon::Subtitles,
        "Subtitles",
        subtitles_enabled,
    );
    let audio_enabled = !state.player.audio_options.is_empty();
    let audio = right_button(ui, &mut right, Icon::Audiotrack, "Audio", audio_enabled);
    right -= 8.0;
    let volume_rect = Rect::from_min_max(pos2(right - 96.0, row.top()), pos2(right, row.bottom()));
    draw_volume(ui, state, volume_rect, opacity, now, actions);
    right = volume_rect.left() - 2.0;
    let mute_icon = if muted || volume <= 0.0 {
        Icon::VolumeOff
    } else {
        Icon::VolumeUp
    };
    let mute = right_button(
        ui,
        &mut right,
        mute_icon,
        if muted { "Unmute" } else { "Mute" },
        true,
    );
    if !state.player.ends_at_label.is_empty() {
        let galley = widgets::elided(
            ui,
            &state.player.ends_at_label,
            theme::regular(theme::FONT_BODY),
            theme::TEXT_SECONDARY.gamma_multiply(opacity),
            200.0,
        );
        let x = right - 16.0 - galley.size().x;
        widgets::galley_v_centered(&painter, x, center_y, galley);
    }

    if toggle_full {
        toggle_fullscreen(state, now);
    }
    if settings {
        state.player.menu = PlayerMenu::Settings;
    }
    if subtitles {
        state.player.menu = PlayerMenu::Subtitles;
    }
    if audio {
        state.player.menu = PlayerMenu::Audio;
    }
    if mute {
        clicked.push(UiAction::ToggleMute);
    }
    if toggle_full || settings || subtitles || audio || !clicked.is_empty() {
        state.player.activity(now);
    }
    actions.extend(clicked);
    backing.hovered() || ui.rect_contains_pointer(rect)
}

/// The seek bar: rail, hover preview, played fill, chapter marks, playhead,
/// and a tooltip naming the chapter and time under the pointer.
fn draw_timeline(
    ui: &mut Ui,
    state: &mut UiState,
    rect: Rect,
    opacity: f32,
    now: f64,
    actions: &mut Vec<UiAction>,
) {
    let response = ui.interact(rect, Id::new("player-timeline"), Sense::click_and_drag());
    widgets::describe_button(&response, true, "Seek");
    let player = &mut state.player;
    let pointer = response.hover_pos().or(response.interact_pointer_pos());
    let hover_ratio = pointer.map_or(0.0, |pointer| {
        ((pointer.x - rect.left()) / rect.width().max(1.0)).clamp(0.0, 1.0)
    });
    let hover_time = f64::from(hover_ratio) * player.duration.max(1.0);
    let pressed = response.is_pointer_button_down_on() || response.dragged();
    let active = response.hovered() || pressed;

    if response.drag_started() || (pressed && !player.scrubbing) {
        player.scrubbing = true;
        player.timeline_position = hover_time;
    }
    if response.dragged() || (pressed && player.scrubbing) {
        player.timeline_position = hover_time;
        player.activity(now);
    }
    if (response.drag_stopped() || response.clicked()) && player.scrubbing {
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

    let rail_height = ui.ctx().animate_value_with_time(
        response.id.with("rail"),
        if active { 6.0 } else { 4.0 },
        0.08,
    );
    let painter = ui.painter();
    let center_y = rect.center().y;
    let rail = Rect::from_center_size(
        pos2(rect.center().x, center_y),
        vec2(rect.width(), rail_height),
    );
    let radius = rail_height / 2.0;
    painter.rect_filled(
        rail,
        radius,
        theme::with_alpha(theme::TEXT_BRIGHT, 0.25 * opacity),
    );
    if response.hovered() {
        let hover = Rect::from_min_size(rail.min, vec2(rail.width() * hover_ratio, rail_height));
        painter.rect_filled(
            hover,
            radius,
            theme::with_alpha(theme::TEXT_BRIGHT, 0.25 * opacity),
        );
    }
    let played = player.played_ratio();
    painter.rect_filled(
        Rect::from_min_size(rail.min, vec2(rail.width() * played, rail_height)),
        radius,
        theme::ACCENT.gamma_multiply(opacity),
    );
    for chapter in &player.chapters {
        if chapter.time > 0.0 && chapter.time < player.duration {
            let x = rect.left() + (chapter.time / player.duration.max(1.0)) as f32 * rect.width();
            painter.rect_filled(
                Rect::from_center_size(pos2(x, center_y), vec2(2.0, rail_height + 4.0)),
                1.0,
                theme::with_alpha(theme::TEXT_BRIGHT, 0.6 * opacity),
            );
        }
    }
    let head = ui.ctx().animate_value_with_time(
        response.id.with("head"),
        if active { 16.0 } else { 12.0 },
        0.08,
    );
    let head_x = (rect.left() + rect.width() * played)
        .clamp(rect.left() + head / 2.0, rect.right() - head / 2.0);
    painter.circle_filled(
        pos2(head_x, center_y),
        head / 2.0,
        theme::TEXT_BRIGHT.gamma_multiply(opacity),
    );

    if active && let Some(pointer) = pointer {
        let clock = format_playback_time(hover_time);
        let chapter = player.chapter_title_at(hover_time);
        let label = if chapter.is_empty() {
            clock
        } else {
            format!("{chapter}  ·  {clock}")
        };
        let galley = widgets::elided(
            ui,
            &label,
            theme::regular(theme::FONT_BODY),
            theme::TEXT_PRIMARY,
            360.0,
        );
        let width = galley.size().x + 16.0;
        let x = (pointer.x - width / 2.0).clamp(rect.left(), rect.right() - width);
        let tip = Rect::from_min_size(pos2(x, rect.top() - 34.0), vec2(width, 26.0));
        painter.rect_filled(tip, theme::RADIUS_SMALL, theme::OVERLAY_RAISED);
        widgets::galley_centered(painter, tip, galley);
    }
    widgets::pointer_on_hover(&response);
}

/// Thin rail filled to the level, with a knob. Dragging unmutes, as in
/// Jellyfin.
fn draw_volume(
    ui: &mut Ui,
    state: &mut UiState,
    rect: Rect,
    opacity: f32,
    now: f64,
    actions: &mut Vec<UiAction>,
) {
    let response = ui.interact(rect, Id::new("player-volume"), Sense::click_and_drag());
    widgets::describe_button(&response, true, "Volume");
    let player = &mut state.player;
    if (response.is_pointer_button_down_on() || response.dragged())
        && let Some(pointer) = response.interact_pointer_pos()
    {
        player.activity(now);
        if player.muted {
            player.muted = false;
            actions.push(UiAction::ToggleMute);
        }
        let volume = (((pointer.x - rect.left()) / rect.width()).clamp(0.0, 1.0) * 100.0).round();
        if volume != player.volume {
            player.volume = volume;
            actions.push(UiAction::SetVolume(volume));
        }
    }
    let shown = if player.muted {
        0.0
    } else {
        player.volume.clamp(0.0, 100.0)
    };
    let painter = ui.painter();
    let rail = Rect::from_center_size(rect.center(), vec2(rect.width(), 4.0));
    painter.rect_filled(
        rail,
        2.0,
        theme::with_alpha(theme::TEXT_BRIGHT, 0.3 * opacity),
    );
    painter.rect_filled(
        Rect::from_min_size(rail.min, vec2(rail.width() * shown / 100.0, 4.0)),
        2.0,
        theme::TEXT_BRIGHT.gamma_multiply(opacity),
    );
    let knob = if response.hovered() || response.dragged() {
        14.0
    } else {
        10.0
    };
    let x = (rect.left() + rect.width() * shown / 100.0)
        .clamp(rect.left() + knob / 2.0, rect.right() - knob / 2.0);
    painter.circle_filled(
        pos2(x, rect.center().y),
        knob / 2.0,
        theme::TEXT_BRIGHT.gamma_multiply(opacity),
    );
    widgets::pointer_on_hover(&response);
}

/// Skip Intro / Recap / Credits and the Up Next card. Both stay up while the
/// chrome is hidden, as in Jellyfin, and slide above the chrome when it shows.
fn draw_floating(
    ui: &mut Ui,
    state: &mut UiState,
    screen: Rect,
    now: f64,
    actions: &mut Vec<UiAction>,
) {
    let lift = ui.ctx().animate_value_with_time(
        Id::new("player-floating-lift"),
        if state.player.controls_visible() {
            BOTTOM_HEIGHT
        } else {
            40.0
        },
        0.22,
    );
    let painter = ui.painter().clone();
    if let Some(up_next) = state.player.up_next.clone() {
        let width = 340.0;
        let inner = width - 32.0;
        let title = widgets::elided(
            ui,
            &up_next.title,
            theme::bold(theme::FONT_TITLE),
            theme::TEXT_BRIGHT,
            inner,
        );
        let subtitle = (!up_next.subtitle.is_empty()).then(|| {
            widgets::elided(
                ui,
                &up_next.subtitle,
                theme::regular(theme::FONT_BODY),
                theme::TEXT_SECONDARY,
                inner,
            )
        });
        let countdown = if up_next.countdown > 0 {
            format!("Starting in {} s", up_next.countdown)
        } else {
            "Starting…".to_owned()
        };
        let height = 16.0
            + 15.0
            + 6.0
            + title.size().y
            + subtitle.as_ref().map_or(0.0, |line| 6.0 + line.size().y)
            + 6.0
            + 15.0
            + 6.0
            + 6.0
            + 36.0
            + 16.0;
        let card = Rect::from_min_size(
            pos2(
                screen.right() - width - 32.0,
                screen.bottom() - height - lift,
            ),
            vec2(width, height),
        );
        ui.interact(card, Id::new("player-up-next"), Sense::click());
        widgets::drop_shadow(
            &painter,
            card,
            theme::RADIUS_LARGE,
            16.0,
            0.0,
            theme::with_alpha(theme::SCRIM, 0.5),
        );
        painter.rect_filled(card, theme::RADIUS_LARGE, theme::OVERLAY);
        let mut y = card.top() + 16.0;
        let left = card.left() + 16.0;
        painter.text(
            pos2(left, y),
            Align2::LEFT_TOP,
            "Up Next",
            theme::bold(theme::FONT_BODY),
            theme::ACCENT_BRIGHT,
        );
        y += 15.0 + 6.0;
        y = widgets::galley_at(&painter, pos2(left, y), title).bottom() + 6.0;
        if let Some(subtitle) = subtitle {
            y = widgets::galley_at(&painter, pos2(left, y), subtitle).bottom() + 6.0;
        }
        painter.text(
            pos2(left, y),
            Align2::LEFT_TOP,
            countdown,
            theme::regular(theme::FONT_BODY),
            theme::TEXT_MUTED,
        );
        y += 15.0 + 12.0;
        let half = (inner - 8.0) / 2.0;
        let play_now = Rect::from_min_size(pos2(left, y), vec2(half, 36.0));
        let hide = Rect::from_min_size(pos2(left + half + 8.0, y), vec2(half, 36.0));
        let play_response = ui.interact(play_now, Id::new("up-next-play"), Sense::click());
        widgets::describe_button(&play_response, true, "Play now");
        painter.rect_filled(
            play_now,
            theme::RADIUS_SMALL,
            if play_response.hovered() {
                theme::ACCENT_BRIGHT
            } else {
                theme::ACCENT
            },
        );
        painter.text(
            play_now.center(),
            Align2::CENTER_CENTER,
            "Play now",
            theme::bold(theme::FONT_BODY_LARGE),
            theme::TEXT_BRIGHT,
        );
        widgets::pointer_on_hover(&play_response);
        let hide_response = ui.interact(hide, Id::new("up-next-hide"), Sense::click());
        widgets::describe_button(&hide_response, true, "Hide");
        painter.rect_filled(
            hide,
            theme::RADIUS_SMALL,
            if hide_response.hovered() {
                theme::SURFACE_HOVER
            } else {
                theme::SURFACE_RAISED
            },
        );
        painter.text(
            hide.center(),
            Align2::CENTER_CENTER,
            "Hide",
            theme::regular(theme::FONT_BODY_LARGE),
            theme::TEXT_PRIMARY,
        );
        widgets::pointer_on_hover(&hide_response);
        if play_response.clicked() {
            state.player.activity(now);
            actions.push(UiAction::PlayNext);
        }
        if hide_response.clicked() {
            state.player.activity(now);
            actions.push(UiAction::DismissUpNext);
        }
    } else if !state.player.skip_label.is_empty() {
        let label = widgets::elided(
            ui,
            &state.player.skip_label,
            theme::bold(theme::FONT_SUBTITLE),
            theme::BG,
            300.0,
        );
        let width = label.size().x + 40.0;
        let button = Rect::from_min_size(
            pos2(screen.right() - width - 32.0, screen.bottom() - 44.0 - lift),
            vec2(width, 44.0),
        );
        let response = ui.interact(button, Id::new("player-skip"), Sense::click());
        widgets::describe_button(&response, true, &state.player.skip_label);
        widgets::drop_shadow(
            &painter,
            button,
            22.0,
            12.0,
            0.0,
            theme::with_alpha(theme::SCRIM, 0.5),
        );
        let fill = if response.hovered() {
            theme::TEXT_BRIGHT
        } else {
            theme::with_alpha(theme::TEXT_BRIGHT, 0.9)
        };
        painter.rect_filled(button, 22.0, fill);
        widgets::galley_centered(&painter, button, label);
        widgets::pointer_on_hover(&response);
        if response.clicked() {
            state.player.activity(now);
            actions.push(UiAction::SeekAbsolute(state.player.skip_target));
        }
    }
}

/// The open pop-up menu, over a full-window catcher that closes it.
fn draw_menus(ui: &mut Ui, state: &mut UiState, screen: Rect, actions: &mut Vec<UiAction>) {
    let player = &mut state.player;
    if player.menu == PlayerMenu::None {
        return;
    }
    let catcher = ui.interact(screen, Id::new("player-menu-catcher"), Sense::click());
    let speed_detail = SPEED_LABELS[player.speed_index.min(SPEED_LABELS.len() - 1)].to_owned();
    let aspect_detail = ASPECT_LABELS[player.aspect_index.min(ASPECT_LABELS.len() - 1)].to_owned();
    let speed_options = SPEED_LABELS.map(str::to_owned);
    let aspect_options = ASPECT_LABELS.map(str::to_owned);
    let settings_options = ["Playback speed".to_owned(), "Aspect ratio".to_owned()];
    let (heading, options, details, selected, drills, can_go_back, footer): (
        &str,
        &[String],
        Vec<String>,
        Option<usize>,
        bool,
        bool,
        &str,
    ) = match player.menu {
        PlayerMenu::Audio => (
            "Audio",
            &player.audio_options,
            Vec::new(),
            usize::try_from(player.audio_index).ok(),
            false,
            false,
            "",
        ),
        PlayerMenu::Subtitles => (
            "Subtitles",
            &player.subtitle_options,
            Vec::new(),
            usize::try_from(player.subtitle_index).ok(),
            false,
            false,
            "",
        ),
        PlayerMenu::Settings => (
            "Settings",
            &settings_options,
            vec![speed_detail, aspect_detail],
            None,
            true,
            false,
            player.decode_label.as_str(),
        ),
        PlayerMenu::Speed => (
            "Playback speed",
            &speed_options,
            Vec::new(),
            Some(player.speed_index),
            false,
            true,
            "",
        ),
        PlayerMenu::Aspect => (
            "Aspect ratio",
            &aspect_options,
            Vec::new(),
            Some(player.aspect_index),
            false,
            true,
            "",
        ),
        PlayerMenu::None => unreachable!("returned above"),
    };
    let picked = menu(
        ui,
        screen,
        heading,
        options,
        &details,
        selected,
        drills,
        can_go_back,
        footer,
    );
    let player = &mut state.player;
    match picked {
        MenuPick::Back => {
            player.menu = PlayerMenu::Settings;
        }
        MenuPick::Row(index) => match player.menu {
            PlayerMenu::Audio => {
                player.menu = PlayerMenu::None;
                player.audio_index = i32::try_from(index).unwrap_or(0);
                actions.push(UiAction::SelectAudio(player.audio_index));
            }
            PlayerMenu::Subtitles => {
                player.menu = PlayerMenu::None;
                player.subtitle_index = i32::try_from(index).unwrap_or(0);
                actions.push(UiAction::SelectSubtitle(player.subtitle_index));
            }
            PlayerMenu::Settings => {
                player.menu = if index == 0 {
                    PlayerMenu::Speed
                } else {
                    PlayerMenu::Aspect
                };
            }
            PlayerMenu::Speed => {
                player.menu = PlayerMenu::None;
                player.speed_index = index;
                actions.push(UiAction::SetSpeed(index));
            }
            PlayerMenu::Aspect => {
                player.menu = PlayerMenu::None;
                player.aspect_index = index;
                actions.push(UiAction::SetAspect(index));
            }
            PlayerMenu::None => {}
        },
        MenuPick::Nothing => {
            if catcher.clicked() || ui.input(|input| input.key_pressed(egui::Key::Escape)) {
                player.menu = PlayerMenu::None;
            }
        }
    }
}

enum MenuPick {
    Nothing,
    Back,
    Row(usize),
}

/// One of the player's pop-up lists, in the style of Jellyfin's OSD action
/// sheets: `selected` rows get a check, `details` fill a right-hand column,
/// `drills` marks rows that open a submenu.
#[allow(clippy::too_many_arguments)]
fn menu(
    ui: &mut Ui,
    screen: Rect,
    heading: &str,
    options: &[String],
    details: &[String],
    selected: Option<usize>,
    drills: bool,
    can_go_back: bool,
    footer: &str,
) -> MenuPick {
    const WIDTH: f32 = 300.0;
    const ROW: f32 = 40.0;
    let list_height = (options.len() as f32 * ROW).min(320.0);
    let footer_galley = (!footer.is_empty()).then(|| {
        widgets::wrapped(
            ui,
            footer,
            theme::regular(theme::FONT_CAPTION),
            theme::TEXT_FAINT,
            WIDTH - 24.0,
            None,
        )
    });
    let height = 6.0
        + 40.0
        + 1.0
        + list_height
        + footer_galley
            .as_ref()
            .map_or(0.0, |galley| 8.0 + galley.size().y + 4.0)
        + 6.0;
    let rect = Rect::from_min_size(
        pos2(
            screen.right() - WIDTH - 24.0,
            screen.bottom() - height - 90.0,
        ),
        vec2(WIDTH, height),
    );
    // Swallows clicks between rows so they do not close the menu.
    ui.interact(rect, Id::new(("player-menu", heading)), Sense::click());
    let painter = ui.painter().clone();
    widgets::drop_shadow(
        &painter,
        rect,
        theme::RADIUS_MEDIUM,
        16.0,
        0.0,
        theme::with_alpha(theme::SCRIM, 0.5),
    );
    painter.rect_filled(rect, theme::RADIUS_MEDIUM, theme::OVERLAY_RAISED);
    let mut pick = MenuPick::Nothing;

    let header = Rect::from_min_size(rect.min + vec2(0.0, 6.0), vec2(WIDTH, 40.0));
    let header_response = ui.interact(
        header,
        Id::new(("player-menu-back", heading)),
        if can_go_back {
            Sense::click()
        } else {
            Sense::hover()
        },
    );
    if can_go_back && header_response.hovered() {
        painter.rect_filled(header, 0.0, theme::with_alpha(theme::TEXT_BRIGHT, 0.08));
    }
    let mut x = header.left() + 12.0;
    if can_go_back {
        icons::paint_centered(
            &painter,
            Icon::ArrowBack,
            pos2(x + 10.0, header.center().y),
            20.0,
            theme::TEXT_BRIGHT,
        );
        x += 28.0;
        widgets::describe_button(&header_response, true, "Back");
        widgets::pointer_on_hover(&header_response);
        if header_response.clicked() {
            pick = MenuPick::Back;
        }
    }
    painter.text(
        pos2(x, header.center().y),
        Align2::LEFT_CENTER,
        heading,
        theme::bold(theme::FONT_SUBTITLE),
        theme::TEXT_PRIMARY,
    );
    painter.rect_filled(
        Rect::from_min_size(pos2(rect.left(), header.bottom()), vec2(WIDTH, 1.0)),
        0.0,
        theme::BORDER,
    );

    let list = Rect::from_min_size(
        pos2(rect.left(), header.bottom() + 1.0),
        vec2(WIDTH, list_height),
    );
    let mut list_ui = ui.new_child(egui::UiBuilder::new().max_rect(list));
    list_ui.set_clip_rect(list);
    egui::ScrollArea::vertical()
        .id_salt(("player-menu-list", heading))
        .max_height(list_height)
        .auto_shrink(false)
        .show(&mut list_ui, |ui| {
            ui.spacing_mut().item_spacing = egui::Vec2::ZERO;
            for (index, option) in options.iter().enumerate() {
                let (row, response) = ui.allocate_exact_size(vec2(WIDTH, ROW), Sense::click());
                widgets::describe_button(&response, true, option);
                let painter = ui.painter();
                if response.hovered() || response.has_focus() {
                    painter.rect_filled(row, 0.0, theme::with_alpha(theme::TEXT_BRIGHT, 0.08));
                }
                if selected == Some(index) {
                    icons::paint_centered(
                        painter,
                        Icon::Check,
                        pos2(row.left() + 22.0, row.center().y),
                        20.0,
                        theme::ACCENT_BRIGHT,
                    );
                }
                let mut right = row.right() - 12.0;
                if drills {
                    icons::paint_centered(
                        painter,
                        Icon::ChevronRight,
                        pos2(right - 10.0, row.center().y),
                        20.0,
                        theme::TEXT_MUTED,
                    );
                    right -= 30.0;
                }
                if let Some(detail) = details.get(index).filter(|detail| !detail.is_empty()) {
                    let galley = widgets::elided(
                        ui,
                        detail,
                        theme::regular(theme::FONT_BODY),
                        theme::TEXT_MUTED,
                        120.0,
                    );
                    right -= galley.size().x;
                    widgets::galley_v_centered(painter, right, row.center().y, galley);
                    right -= 10.0;
                }
                let label_left = row.left() + 12.0 + 20.0 + 10.0;
                let galley = widgets::elided(
                    ui,
                    option,
                    theme::regular(theme::FONT_BODY_LARGE),
                    theme::TEXT_PRIMARY,
                    right - label_left,
                );
                widgets::galley_v_centered(painter, label_left, row.center().y, galley);
                widgets::pointer_on_hover(&response);
                if response.clicked() {
                    pick = MenuPick::Row(index);
                }
            }
        });
    if let Some(galley) = footer_galley {
        painter.galley(
            pos2(rect.left() + 12.0, list.bottom() + 8.0),
            galley,
            Color32::WHITE,
        );
    }
    pick
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
    fn chapter_titles_follow_the_time() {
        let player = PlayerVm {
            chapters: vec![
                ChapterMarker {
                    time: 0.0,
                    title: "Opening".to_owned(),
                },
                ChapterMarker {
                    time: 60.0,
                    title: "Act One".to_owned(),
                },
            ],
            ..PlayerVm::default()
        };
        assert_eq!(player.chapter_title_at(30.0), "Opening");
        assert_eq!(player.chapter_title_at(61.0), "Act One");
    }
}
