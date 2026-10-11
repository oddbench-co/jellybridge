//! The window: applies queued presenter updates, draws the current screen,
//! and turns what the user did ([`UiAction`]) into feature calls.
//!
//! Drawing never calls into features directly. It records actions, and the
//! window dispatches them after the frame. That keeps every draw function
//! testable without a server, a runtime or a player, and keeps feature work
//! (which locks application state) out of the middle of a frame.

use std::sync::{Arc, Mutex};

use eframe::egui_glow;
use egui::{Rect, pos2, vec2};

use super::player_hotkeys::PLAYBACK_SPEEDS;
use super::{Screen, UiQueue, UiState, catalog, detail, images, login, player, startup, theme};
use crate::app::AppContext;
use crate::features;
use crate::features::detail::{DetailKind, DetailMessage, DetailTarget};
use crate::services::player::{AspectMode, PlayerCommand, PlayerTrack, PlayerWorker};
use crate::services::render::{RenderHost, VideoViewport};

/// Something the user did, recorded while drawing and dispatched after it.
#[derive(Clone, Debug, PartialEq)]
pub enum UiAction {
    Login {
        server: String,
        username: String,
        password: String,
    },
    Logout,
    GoBack,
    GoHome,
    Retry,
    OpenItem(String),
    Search(String),
    SortChanged(usize),
    DensityChanged(usize),
    OpenDetail {
        id: String,
        kind: String,
    },
    PlayCurrent {
        from_start: bool,
    },
    PlayDetail(String),
    SetWatched {
        id: String,
        watched: bool,
    },
    SetFavorite {
        id: String,
        favorite: bool,
    },
    DetailTrack {
        index: i32,
        audio: bool,
    },
    OpenExternalUrl(String),
    TogglePlayback,
    SeekRelative(f64),
    SeekAbsolute(f64),
    SetVolume(f32),
    ToggleMute,
    SetSpeed(usize),
    SetAspect(usize),
    SelectAudio(i32),
    SelectSubtitle(i32),
    PlayNext,
    /// Carries the playhead: early in an item "previous" restarts it.
    PlayPrevious(f64),
    DismissUpNext,
    LeavePlayer,
}

/// The mpv render host, reachable from paint callbacks.
///
/// Paint callbacks must be `Send + Sync`, but the host holds mpv's render
/// client and GL objects, which belong to the window's thread. eframe runs
/// every paint callback, and `on_exit`, on that one thread, so the host is
/// never actually touched anywhere else.
struct VideoSurface {
    host: RenderHost,
    /// The last render failure, reported once to the player status line.
    error: Option<String>,
    error_reported: bool,
}

// SAFETY: see the type's documentation -- only the window thread uses it.
unsafe impl Send for VideoSurface {}

pub struct JellyBridgeApp {
    state: UiState,
    queue: UiQueue,
    context: AppContext,
    video: Arc<Mutex<VideoSurface>>,
    player_worker: Option<PlayerWorker>,
    #[cfg(debug_assertions)]
    input_script: Option<super::input_script::InputScript>,
}

impl JellyBridgeApp {
    pub fn new(
        ctx: &egui::Context,
        queue: UiQueue,
        context: AppContext,
        render_host: RenderHost,
        player_worker: PlayerWorker,
        brand_icon: Option<egui::ColorImage>,
    ) -> Self {
        theme::install(ctx);
        queue.attach(ctx);
        let mut state = UiState::new(ctx.clone());
        state.brand_icon = brand_icon
            .map(|image| ctx.load_texture("brand-icon", image, egui::TextureOptions::LINEAR));
        Self {
            state,
            queue,
            context,
            video: Arc::new(Mutex::new(VideoSurface {
                host: render_host,
                error: None,
                error_reported: false,
            })),
            player_worker: Some(player_worker),
            #[cfg(debug_assertions)]
            input_script: super::input_script::InputScript::from_env(),
        }
    }

    /// The paint callback that draws the video into `rect`.
    fn video_callback(&self, rect: Rect) -> egui::PaintCallback {
        let video = self.video.clone();
        egui::PaintCallback {
            rect,
            callback: Arc::new(egui_glow::CallbackFn::new(move |info, painter| {
                let pixels = info.viewport_in_pixels();
                let viewport = VideoViewport {
                    left: pixels.left_px,
                    bottom: pixels.from_bottom_px,
                    width: pixels.width_px,
                    height: pixels.height_px,
                };
                let Ok(mut video) = video.lock() else { return };
                match video.host.draw(painter.gl(), viewport) {
                    Ok(()) => {
                        video.error = None;
                        video.error_reported = false;
                    }
                    Err(error) => {
                        let message = error.to_string();
                        if video.error.as_deref() != Some(message.as_str()) {
                            tracing::error!(%error, "video render failed");
                            video.error = Some(message);
                            video.error_reported = false;
                        }
                    }
                }
            })),
        }
    }

    /// A paint callback that only runs mpv's render-thread work, for frames
    /// that show no video. Needs a real (1×1) rect: egui skips empty ones.
    fn pump_callback(&self, origin: egui::Pos2) -> Option<egui::PaintCallback> {
        let active = self.video.lock().is_ok_and(|video| video.host.is_active());
        if !active {
            return None;
        }
        let video = self.video.clone();
        Some(egui::PaintCallback {
            rect: Rect::from_min_size(origin, vec2(1.0, 1.0)),
            callback: Arc::new(egui_glow::CallbackFn::new(move |_info, _painter| {
                if let Ok(mut video) = video.lock() {
                    video.host.pump();
                }
            })),
        })
    }

    fn report_render_error(&mut self) {
        let Ok(mut video) = self.video.lock() else {
            return;
        };
        if let Some(error) = &video.error
            && !video.error_reported
        {
            self.state.player.status = format!("Rendering failed: {error}");
            video.error_reported = true;
        }
    }

    fn dispatch(&mut self, action: UiAction) {
        let context = &self.context;
        let send = |command: PlayerCommand| {
            let _ = context.player_sender.send(command);
        };
        match action {
            UiAction::Login {
                server,
                username,
                password,
            } => features::auth::start_login(context, server, username, password),
            UiAction::Logout => {
                self.state.login.password.clear();
                features::auth::logout(context);
            }
            UiAction::GoBack => features::catalog::go_back(context),
            UiAction::GoHome => features::catalog::show_home(context, true),
            UiAction::Retry => features::catalog::retry(context),
            UiAction::OpenItem(id) => features::catalog::open_item(context, &id),
            UiAction::Search(term) => features::catalog::start_search(context, term),
            UiAction::SortChanged(index) => {
                features::catalog::change_catalog_sort(context, i32::try_from(index).unwrap_or(0));
            }
            UiAction::DensityChanged(index) => {
                features::catalog::change_catalog_density(
                    context,
                    i32::try_from(index).unwrap_or(1),
                );
            }
            UiAction::OpenDetail { id, kind } => features::detail::start_detail(
                context,
                DetailTarget::new(&id, DetailKind::from_jellyfin(&kind)),
            ),
            UiAction::PlayCurrent { from_start } => {
                features::playback::start_playback(context, from_start)
            }
            UiAction::PlayDetail(id) => {
                let item = context
                    .state
                    .lock()
                    .ok()
                    .and_then(|state| state.detail.item_by_id(&id));
                if let Some(item) = item {
                    features::playback::start_playback_item(context, item, false);
                }
            }
            UiAction::SetWatched { id, watched } => features::detail::update_detail_mutation(
                context,
                DetailMessage::SetWatched { id, watched },
            ),
            UiAction::SetFavorite { id, favorite } => features::detail::update_detail_mutation(
                context,
                DetailMessage::SetFavorite { id, favorite },
            ),
            UiAction::DetailTrack { index, audio } => {
                features::detail::select_detail_track(context, index, audio);
            }
            UiAction::OpenExternalUrl(url) => detail::open_external_url(url),
            UiAction::TogglePlayback => send(PlayerCommand::TogglePaused),
            UiAction::SeekRelative(seconds) => send(PlayerCommand::SeekRelative(seconds)),
            UiAction::SeekAbsolute(seconds) => send(PlayerCommand::SeekAbsolute(seconds)),
            UiAction::SetVolume(volume) => send(PlayerCommand::SetVolume(f64::from(volume))),
            UiAction::ToggleMute => send(PlayerCommand::ToggleMuted),
            UiAction::SetSpeed(index) => {
                if let Some(speed) = PLAYBACK_SPEEDS.get(index) {
                    send(PlayerCommand::SetSpeed(*speed));
                }
            }
            UiAction::SetAspect(index) => {
                if let Some(mode) = AspectMode::ALL.get(index) {
                    send(PlayerCommand::SetAspect(*mode));
                }
            }
            UiAction::SelectAudio(index) => {
                let selection = usize::try_from(index).ok().and_then(|index| {
                    let tracks = context.player_tracks.lock().ok()?;
                    Some((
                        tracks.audio.get(index)?.clone(),
                        tracks.preference_key.clone(),
                    ))
                });
                if let Some((track, key)) = selection {
                    send(PlayerCommand::SelectAudio(track.id));
                    persist_track_preference_async(context, key, Some(track), None);
                }
            }
            UiAction::SelectSubtitle(index) => {
                let selection = usize::try_from(index).ok().and_then(|index| {
                    let tracks = context.player_tracks.lock().ok()?;
                    let track = if index == 0 {
                        None
                    } else {
                        Some(tracks.subtitles.get(index - 1)?.clone())
                    };
                    Some((track, tracks.preference_key.clone()))
                });
                if let Some((track, key)) = selection {
                    send(PlayerCommand::SelectSubtitle(
                        track.as_ref().map(|track| track.id),
                    ));
                    persist_track_preference_async(context, key, None, Some(track));
                }
            }
            UiAction::PlayNext => features::playback::play_adjacent(context, true),
            UiAction::PlayPrevious(position) => {
                features::playback::play_previous(context, position)
            }
            UiAction::DismissUpNext => features::playback::dismiss_up_next(context),
            UiAction::LeavePlayer => self.leave_player(),
        }
    }

    fn leave_player(&mut self) {
        let context = &self.context;
        let _ = context.player_sender.send(PlayerCommand::Stop);
        features::playback::stop_remote_playback(context);
        let now = self.state.ctx.input(|input| input.time);
        if self.state.player.ui.fullscreen {
            player::set_fullscreen(&mut self.state, false, now);
        }
        self.state.player.menu = player::PlayerMenu::None;
        self.state.player.hide_at = None;
        self.state.player.ui.pointer_activity();
        let has_detail = context
            .state
            .lock()
            .is_ok_and(|state| state.detail.snapshot().page.is_some());
        self.state.screen = if has_detail {
            Screen::Detail
        } else {
            Screen::Catalog
        };
        // The stop report refreshes once it lands; this covers leaving after
        // mpv already ended the item and the stop was reported then.
        context.request_refresh();
    }
}

impl eframe::App for JellyBridgeApp {
    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        self.queue.drain(&mut self.state);
        let mut actions = std::mem::take(&mut self.state.deferred);
        self.report_render_error();

        let screen = ui.max_rect();
        match self.state.screen {
            Screen::Login => login::draw(ui, &mut self.state, &mut actions),
            Screen::Catalog => catalog::draw(ui, &mut self.state, &mut actions),
            Screen::Detail => detail::draw(ui, &mut self.state, &mut actions),
            Screen::Player => {
                player::handle_hotkeys(ui, &mut self.state, &mut actions);
                let video = self.video_callback(screen);
                player::draw(ui, &mut self.state, Some(video), &mut actions);
            }
        }
        if self.state.screen != Screen::Player
            && let Some(pump) = self.pump_callback(pos2(screen.left(), screen.top()))
        {
            ui.painter().add(pump);
        }

        for art in self.state.images.take_wanted() {
            images::fetch(&self.context, art);
        }
        if !actions.is_empty() {
            for action in actions {
                self.dispatch(action);
            }
            ui.ctx().request_repaint();
        }
        #[cfg(debug_assertions)]
        if let Some(script) = &self.input_script {
            script.schedule(ui.ctx());
        }
        startup::note_frame_rendered();
    }

    #[cfg(debug_assertions)]
    fn raw_input_hook(&mut self, _ctx: &egui::Context, raw_input: &mut egui::RawInput) {
        if let Some(script) = self.input_script.as_mut() {
            script.inject(raw_input);
        }
    }

    fn clear_color(&self, _visuals: &egui::Visuals) -> [f32; 4] {
        theme::BG.to_normalized_gamma_f32()
    }

    fn on_exit(&mut self, gl: Option<&eframe::glow::Context>) {
        // Teardown order matters: invalidate in-flight work, stop the mpv
        // worker, then free the render context while the GL context is
        // still current.
        self.context.next_generation();
        if let Some(worker) = self.player_worker.take() {
            worker.shutdown();
        }
        if let Ok(mut video) = self.video.lock() {
            video.host.teardown(gl);
        }
    }
}

/// The settings store does blocking file I/O, so track preferences are
/// persisted off the UI thread.
fn persist_track_preference_async(
    context: &AppContext,
    key: Option<String>,
    audio: Option<PlayerTrack>,
    subtitle: Option<Option<PlayerTrack>>,
) {
    let store = context.settings_store.clone();
    context.runtime.spawn(async move {
        let _ = tokio::task::spawn_blocking(move || {
            features::playback::persist_player_track_preference(
                &store,
                key.as_deref(),
                audio.as_ref(),
                subtitle.as_ref().map(|track| track.as_ref()),
            );
        })
        .await;
    });
}
