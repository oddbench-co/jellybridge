//! The window: applies queued presenter updates, draws the current screen,
//! and turns what the user did ([`UiAction`]) into feature calls.
//!
//! Drawing never calls into features directly. It records actions, and the
//! window dispatches them after the frame. That keeps every draw function
//! testable without a server, a runtime or a player, and keeps feature work
//! (which locks application state) out of the middle of a frame.

use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use eframe::egui_glow;
use egui::{Rect, pos2, vec2};

use super::player_hotkeys::PLAYBACK_SPEEDS;
use super::{Screen, UiQueue, UiState, catalog, detail, images, login, player, startup, theme};
use crate::app::AppContext;
use crate::features;
use crate::features::detail::{DetailKind, DetailMessage, DetailTarget};
use crate::services::player::{AspectMode, PlayerCommand, PlayerTrack, PlayerWorker};
use crate::services::render::{RenderHost, VideoViewport};

/// How long input must be quiet before the video render context is released
/// after leaving the player (see `RenderHost::release_soon`).
const RELEASE_AFTER_QUIET: Duration = Duration::from_secs(5);

/// Something the user did, recorded while drawing and dispatched after it.
#[derive(Clone, Debug, PartialEq)]
pub enum UiAction {
    Login {
        server: String,
        username: String,
        password: String,
        remember: bool,
    },
    QuickConnect {
        server: String,
        remember: bool,
    },
    CancelQuickConnect,
    ForgotPassword {
        server: String,
        username: String,
    },
    ForgotPasswordPin {
        server: String,
        pin: String,
    },
    Logout,
    GoBack,
    GoHome,
    Retry,
    OpenItem(String),
    Search(String),
    /// A header tab.
    SwitchTab(usize),
    /// A library in the drawer.
    OpenLibrary(crate::features::pages::LibraryRef),
    /// A section title's "›".
    OpenShelfLink(String),
    /// Another page of the grid, by its first row.
    GridPage(usize),
    /// New sort, filter, view or A–Z settings for the grid.
    GridSettings(crate::features::pages::GridSettings),
    /// Opens or closes a grid dialog.
    GridDialog(Option<catalog::GridDialog>),
    /// The filter dialog wants its options.
    LoadFilterOptions,
    /// A "⋮" or right-click on an item.
    OpenMenu(Box<super::menu::MenuRequest>),
    /// An entry of the item menu.
    MenuCommand(super::menu::MenuCommand, Box<super::menu::MenuRequest>),
    /// The Add to Playlist / Collection dialog's Add or Create.
    SubmitAddTo {
        collection: bool,
        target: Option<String>,
        name: String,
        public: bool,
        item_ids: Vec<String>,
    },
    /// "New Playlist" / "New Collection".
    NewContainer {
        collection: bool,
    },
    /// A genre, studio, tag or person link.
    OpenLink(crate::features::items::LinkTarget),
    /// Play All or Shuffle.
    PlayQueue {
        source: crate::api::QueueSource,
        shuffle: bool,
    },
    /// The header's search button.
    OpenSearch,
    /// A card's play button: plays the item, or a series' next episode.
    PlayItem(String),
    /// A card's played/favorite button.
    ToggleCard {
        id: String,
        toggle: crate::features::catalog::CardToggle,
    },
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
    /// The detail item's trailer: a local one plays, an online one opens.
    PlayTrailer(String),
    OpenSettings(crate::features::settings::SettingsPage),
    ChangePassword {
        current: String,
        new: String,
    },
    AuthorizeQuickConnect(String),
    AddUserImage,
    DeleteUserImage,
    /// A short message at the bottom of the window.
    Toast(String),
    SaveSettings(Box<crate::features::settings::SettingsSave>),
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
    input_script: Option<super::input_script::InputScript>,
    measure: Option<super::measure::Measure>,
    /// When the last input event arrived; heavy deferred work waits for quiet.
    last_input: Instant,
    /// A Back key or mouse button arrived; handled on the next frame.
    back_pending: bool,
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
            input_script: super::input_script::InputScript::from_env(),
            measure: super::measure::Measure::from_env(),
            last_input: Instant::now(),
            back_pending: false,
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
    fn pump_callback(
        &self,
        ctx: &egui::Context,
        origin: egui::Pos2,
    ) -> Option<egui::PaintCallback> {
        let (active, release_pending) = self
            .video
            .lock()
            .map(|video| (video.host.is_active(), video.host.release_pending()))
            .unwrap_or_default();
        if !active {
            return None;
        }
        let quiet_for = self.last_input.elapsed();
        // egui wakes a scheduled repaint one predicted frame early.
        let may_release = quiet_for + Duration::from_millis(50) >= RELEASE_AFTER_QUIET;
        if release_pending && !may_release {
            ctx.request_repaint_after(RELEASE_AFTER_QUIET - quiet_for);
        }
        let video = self.video.clone();
        Some(egui::PaintCallback {
            rect: Rect::from_min_size(origin, vec2(1.0, 1.0)),
            callback: Arc::new(egui_glow::CallbackFn::new(move |_info, painter| {
                if let Ok(mut video) = video.lock() {
                    video.host.pump(painter.gl(), may_release);
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
                remember,
            } => features::auth::start_login(context, server, username, password, remember),
            UiAction::QuickConnect { server, remember } => {
                features::auth::start_quick_connect(context, server, remember);
            }
            UiAction::CancelQuickConnect => features::auth::cancel_quick_connect(context),
            UiAction::ForgotPassword { server, username } => {
                features::auth::forgot_password(context, server, username);
            }
            UiAction::ForgotPasswordPin { server, pin } => {
                features::auth::forgot_password_pin(context, server, pin);
            }
            UiAction::Logout => {
                self.state.login.password.clear();
                features::auth::logout(context);
            }
            UiAction::GoBack => features::catalog::go_back(context),
            UiAction::GoHome => features::catalog::go_home(context),
            UiAction::Retry => features::catalog::retry(context),
            UiAction::OpenItem(id) => features::catalog::open_item(context, &id),
            UiAction::Search(term) => features::catalog::search(context, term),
            UiAction::SwitchTab(index) => features::catalog::switch_tab(context, index),
            UiAction::OpenLibrary(library) => features::catalog::open_library(context, library),
            UiAction::OpenShelfLink(shelf) => features::catalog::open_shelf_link(context, &shelf),
            UiAction::GridPage(start) => features::catalog::show_grid_page(context, start),
            UiAction::GridSettings(settings) => {
                features::catalog::update_grid_settings(context, settings);
            }
            UiAction::GridDialog(dialog) => self.state.catalog.dialog = dialog,
            UiAction::LoadFilterOptions => features::catalog::load_filter_options(context),
            UiAction::OpenMenu(request) => {
                let commands = super::menu::menu_commands(&request);
                if !commands.is_empty() {
                    self.state.menu = Some(super::menu::OpenMenu {
                        request: *request,
                        commands,
                    });
                }
            }
            UiAction::MenuCommand(command, request) => {
                features::items::run_command(context, command, *request);
            }
            UiAction::SubmitAddTo {
                collection,
                target,
                name,
                public,
                item_ids,
            } => {
                features::items::submit_add_to(context, collection, target, name, public, item_ids)
            }
            UiAction::NewContainer { collection } => {
                features::items::open_add_to(context, collection, Vec::new());
            }
            UiAction::OpenLink(target) => features::items::open_link(context, target),
            UiAction::PlayQueue { source, shuffle } => {
                features::playback::play_queue(context, source, shuffle);
            }
            UiAction::OpenSearch => features::catalog::open_search(context),
            UiAction::PlayItem(id) => features::playback::play_item_by_id(context, id),
            UiAction::ToggleCard { id, toggle } => {
                features::catalog::toggle_card(context, &id, toggle);
            }
            UiAction::OpenDetail { id, kind } => features::catalog::open_detail(
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
            UiAction::PlayTrailer(id) => features::playback::play_trailer(context, &id),
            UiAction::OpenSettings(page) => features::catalog::navigate(
                context,
                crate::features::pages::PageRoute::Settings(page),
            ),
            UiAction::SaveSettings(save) => features::settings::save(context, *save),
            UiAction::ChangePassword { current, new } => {
                features::settings::change_password(context, current, new);
            }
            UiAction::AuthorizeQuickConnect(code) => {
                features::settings::authorize_quick_connect(context, code);
            }
            UiAction::AddUserImage => features::settings::add_user_image(context),
            UiAction::DeleteUserImage => features::settings::delete_user_image(context),
            UiAction::Toast(message) => {
                context
                    .ui
                    .update(move |state| super::show_toast(state, &message));
            }
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
        if let Ok(mut video) = self.video.lock() {
            video.host.release_soon();
        }
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
            .is_ok_and(|state| state.detail.is_active());
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
    fn ui(&mut self, ui: &mut egui::Ui, frame: &mut eframe::Frame) {
        let layout_started = std::time::Instant::now();
        self.queue.drain(&mut self.state);
        let mut actions = std::mem::take(&mut self.state.deferred);
        if let Some(script) = self.input_script.as_mut() {
            actions.extend(script.take_actions());
            for path in script.take_shots() {
                ui.ctx()
                    .send_viewport_cmd(egui::ViewportCommand::Screenshot(egui::UserData::new(
                        path,
                    )));
            }
        }
        save_screenshots(ui.ctx());
        self.report_render_error();

        let screen = ui.max_rect();
        super::note_window_width(screen.width());
        if std::mem::take(&mut self.back_pending)
            && matches!(self.state.screen, Screen::Catalog | Screen::Detail)
            && !self.state.drawer_open
            && self.state.menu.is_none()
            && self.state.item_dialog.is_none()
        {
            actions.push(UiAction::GoBack);
        }
        match self.state.screen {
            Screen::Login => login::draw(ui, &mut self.state, &mut actions),
            Screen::Catalog => catalog::draw(ui, &mut self.state, &mut actions),
            Screen::Detail => detail::draw(ui, &mut self.state, &mut actions),
            Screen::Player => {
                if let Ok(mut video) = self.video.lock() {
                    video.host.keep();
                }
                player::handle_hotkeys(ui, &mut self.state, &mut actions);
                let video = self.video_callback(screen);
                player::draw(ui, &mut self.state, Some(video), &mut actions);
            }
        }
        if self.state.screen != Screen::Player {
            super::menu::draw_menu(ui, &mut self.state.menu, &mut actions);
            super::menu::draw_dialog(ui, &mut self.state.item_dialog, &mut actions);
        }
        super::menu::draw_toast(ui, &mut self.state.toast);
        super::scrollbar::end_frame(ui.ctx());
        if self.state.screen != Screen::Player
            && let Some(pump) = self.pump_callback(ui.ctx(), pos2(screen.left(), screen.top()))
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
        if let Some(script) = &self.input_script {
            script.schedule(ui.ctx());
        }
        startup::note_frame_rendered();
        if let Some(measure) = self.measure.as_mut() {
            measure.frame(
                ui.ctx(),
                startup::content_ready(),
                layout_started.elapsed(),
                frame.info().cpu_usage,
            );
        }
    }

    fn raw_input_hook(&mut self, ctx: &egui::Context, raw_input: &mut egui::RawInput) {
        if let Some(script) = self.input_script.as_mut() {
            script.inject(raw_input);
        }
        if back_requested(ctx, raw_input) {
            self.back_pending = true;
        }
        if !raw_input.events.is_empty() {
            self.last_input = Instant::now();
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

/// Writes frames the input script asked for (see `input_script`), off the
/// UI thread.
fn save_screenshots(ctx: &egui::Context) {
    let shots = ctx.input(|input| {
        input
            .raw
            .events
            .iter()
            .filter_map(|event| match event {
                egui::Event::Screenshot {
                    image, user_data, ..
                } => user_data
                    .data
                    .as_ref()
                    .and_then(|data| data.downcast_ref::<String>())
                    .map(|path| (path.clone(), image.clone())),
                _ => None,
            })
            .collect::<Vec<_>>()
    });
    for (path, image) in shots {
        std::thread::spawn(move || {
            let [width, height] = image.size;
            let bytes = image
                .pixels
                .iter()
                .flat_map(|pixel| pixel.to_array())
                .collect::<Vec<u8>>();
            match image::RgbaImage::from_raw(width as u32, height as u32, bytes) {
                Some(frame) => {
                    if let Err(error) = frame.save(&path) {
                        tracing::warn!(%error, path, "screenshot not saved");
                    }
                }
                None => tracing::warn!(path, "screenshot had the wrong size"),
            }
        });
    }
}

/// jellyfin-web's Back besides the header button: the mouse's back button,
/// Alt+Left, and Backspace outside a text field. Read from the raw input,
/// before a focused text field can take the keys.
fn back_requested(ctx: &egui::Context, raw_input: &egui::RawInput) -> bool {
    let typing = ctx.memory(|memory| memory.focused().is_some());
    raw_input.events.iter().any(|event| match event {
        egui::Event::PointerButton {
            button: egui::PointerButton::Extra1,
            pressed: true,
            ..
        } => true,
        egui::Event::Key {
            key: egui::Key::ArrowLeft,
            pressed: true,
            modifiers,
            ..
        } => modifiers.alt,
        egui::Event::Key {
            key: egui::Key::Backspace,
            pressed: true,
            ..
        } => !typing,
        _ => false,
    })
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
