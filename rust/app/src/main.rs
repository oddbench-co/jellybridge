mod api;
mod catalog;
mod detail;
mod player_ui;
mod session;

use std::cell::{Cell, RefCell};
use std::collections::hash_map::DefaultHasher;
use std::fs::{self, File, OpenOptions};
use std::hash::{Hash, Hasher};
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::rc::Rc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};

use api::{ApiError, CatalogItemData, CatalogSectionData, JellyfinApi};
use catalog::{CatalogDensity, CatalogSectionRange, CatalogSort, flatten_sections};
use detail::{
    DetailEffect, DetailItemData, DetailKind, DetailMessage, DetailModel, DetailMutationKind,
    DetailPageData, DetailTarget, ImageRef,
};
use jellybridge_vertical_slice::jellyfin::{
    JellyfinSliceConfig, PreparedJellyfinPlayback, prepare_jellyfin_playback,
};
use jellybridge_vertical_slice::player::{
    PlayerCommand, PlayerEvent, PlayerSubtitleFile, PlayerTrack, create_player,
};
use jellybridge_vertical_slice::render::{RenderHost, native_display};
use player_ui::{PlayerHotkeyAction, PlayerUiState, next_track_index, resolve_player_hotkey};
use session::{
    SessionStore, SettingsStore, StoredPlaybackPreference, StoredSettings, cache_directory,
};
use slint::{ComponentHandle, Image, Model, ModelRc, RenderingState, SharedString, VecModel};

#[cfg(all(feature = "renderer-femtovg", feature = "renderer-skia-opengl"))]
compile_error!("select exactly one renderer feature");

#[cfg(not(any(feature = "renderer-femtovg", feature = "renderer-skia-opengl")))]
compile_error!("select renderer-femtovg or renderer-skia-opengl");

slint::include_modules!();

#[cfg(feature = "renderer-femtovg")]
const RENDERER_NAME: &str = "femtovg";

#[cfg(feature = "renderer-skia-opengl")]
const RENDERER_NAME: &str = "skia";

#[derive(Clone)]
struct PageSnapshot {
    title: String,
    items: Vec<CatalogItemData>,
    sections: Vec<CatalogSectionRange>,
    parent: Option<CatalogItemData>,
    search_text: String,
    total_record_count: usize,
}

impl PageSnapshot {
    fn empty_home() -> Self {
        Self {
            title: "Home".into(),
            items: Vec::new(),
            sections: Vec::new(),
            parent: None,
            search_text: String::new(),
            total_record_count: 0,
        }
    }

    fn home(sections: &[CatalogSectionData]) -> Self {
        let flattened = flatten_sections(sections);
        let total_record_count = flattened.items.len();
        Self {
            title: "Home".into(),
            items: flattened.items,
            sections: flattened.sections,
            parent: None,
            search_text: String::new(),
            total_record_count,
        }
    }

    fn is_home_dashboard(&self) -> bool {
        self.parent.is_none() && self.search_text.is_empty()
    }
}

#[derive(Clone)]
enum RetryAction {
    Home,
    Browse(CatalogItemData, bool),
    Search(String),
    Detail(DetailTarget),
}

enum EmptySearchDestination {
    Ignore,
    Home,
    Parent(CatalogItemData),
}

fn empty_search_destination(current: &PageSnapshot) -> EmptySearchDestination {
    if current.search_text.is_empty() {
        EmptySearchDestination::Ignore
    } else if let Some(parent) = current.parent.clone() {
        EmptySearchDestination::Parent(parent)
    } else {
        EmptySearchDestination::Home
    }
}

struct AppState {
    api: Option<JellyfinApi>,
    account_name: String,
    home: Vec<CatalogSectionData>,
    current: PageSnapshot,
    back_stack: Vec<PageSnapshot>,
    detail: DetailModel,
    detail_tracks: DetailTrackSelection,
    retry: RetryAction,
    catalog_sort: CatalogSort,
    catalog_density: CatalogDensity,
}

#[derive(Clone, Default)]
struct DetailTrackSelection {
    audio_ids: Vec<i32>,
    subtitle_ids: Vec<Option<i32>>,
    audio_index: i32,
    subtitle_index: i32,
}

#[derive(Default)]
struct PlayerTrackState {
    audio: Vec<PlayerTrack>,
    subtitles: Vec<PlayerTrack>,
    preference_key: Option<String>,
}

impl Default for AppState {
    fn default() -> Self {
        Self {
            api: None,
            account_name: String::new(),
            home: Vec::new(),
            current: PageSnapshot::empty_home(),
            back_stack: Vec::new(),
            detail: DetailModel::default(),
            detail_tracks: DetailTrackSelection::default(),
            retry: RetryAction::Home,
            catalog_sort: CatalogSort::default(),
            catalog_density: CatalogDensity::default(),
        }
    }
}

#[derive(Clone)]
struct AppContext {
    window: slint::Weak<JellyBridgeWindow>,
    runtime: tokio::runtime::Handle,
    state: Arc<Mutex<AppState>>,
    generation: Arc<AtomicU64>,
    store: SessionStore,
    settings_store: SettingsStore,
    image_cache: PathBuf,
    image_limit: Arc<tokio::sync::Semaphore>,
    playback: Arc<Mutex<Option<PreparedJellyfinPlayback>>>,
    player_sender: std::sync::mpsc::Sender<PlayerCommand>,
    player_tracks: Arc<Mutex<PlayerTrackState>>,
    catalog_request: Arc<Mutex<Option<tokio::task::AbortHandle>>>,
}

impl AppContext {
    fn next_generation(&self) -> u64 {
        self.generation.fetch_add(1, Ordering::AcqRel) + 1
    }

    fn is_current(&self, generation: u64) -> bool {
        self.generation.load(Ordering::Acquire) == generation
    }

    fn api(&self) -> Option<JellyfinApi> {
        self.state.lock().ok()?.api.clone()
    }
}

fn replace_catalog_request(context: &AppContext, request: tokio::task::AbortHandle) {
    if let Ok(mut active) = context.catalog_request.lock()
        && let Some(previous) = active.replace(request)
    {
        previous.abort();
    }
}

fn cancel_catalog_request(context: &AppContext) {
    if let Ok(mut active) = context.catalog_request.lock()
        && let Some(request) = active.take()
    {
        request.abort();
    }
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| "jellybridge_app=info".into()),
        )
        .without_time()
        .init();

    slint::BackendSelector::new()
        .backend_name("winit".into())
        .renderer_name(RENDERER_NAME.into())
        .require_opengl()
        .select()
        .map_err(|error| {
            format!(
                "JellyBridge requires the explicitly configured {RENDERER_NAME} OpenGL renderer: {error}"
            )
        })?;

    let runtime = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()?;
    let store = SessionStore::for_current_user()?;
    let settings_store = SettingsStore::for_current_user()?;
    let stored_settings = match settings_store.load() {
        Ok(settings) => settings,
        Err(error) => {
            tracing::warn!(%error, "catalog settings could not be loaded");
            session::StoredSettings::default()
        }
    };
    let app_state = AppState {
        catalog_sort: CatalogSort::from_key(stored_settings.library_sort_key.as_deref()),
        catalog_density: CatalogDensity::from_card_width(stored_settings.card_width_dp),
        ..AppState::default()
    };
    let image_cache = cache_directory()?;
    let window = JellyBridgeWindow::new()?;
    let playback = Arc::new(Mutex::new(None::<PreparedJellyfinPlayback>));
    let player_tracks = Arc::new(Mutex::new(PlayerTrackState::default()));
    let event_playback = playback.clone();
    let event_tracks = player_tracks.clone();
    let event_runtime = runtime.handle().clone();
    let event_window = window.as_weak();
    let (render_client, player_worker) = create_player(move |event| {
        let ended = matches!(event, PlayerEvent::Ended);
        let weak = event_window.clone();
        let tracks = event_tracks.clone();
        let _ =
            slint::invoke_from_event_loop(move || update_window_from_player(&weak, event, &tracks));
        if ended
            && let Ok(mut slot) = event_playback.lock()
            && let Some(remote) = slot.take()
        {
            event_runtime.spawn(remote.shutdown());
        }
    })?;
    let player_sender = player_worker.command_sender();
    let player_worker = Rc::new(RefCell::new(Some(player_worker)));
    let image_is_cleared = Rc::new(Cell::new(true));
    let context = AppContext {
        window: window.as_weak(),
        runtime: runtime.handle().clone(),
        state: Arc::new(Mutex::new(app_state)),
        generation: Arc::new(AtomicU64::new(0)),
        store,
        settings_store,
        image_cache,
        image_limit: Arc::new(tokio::sync::Semaphore::new(6)),
        playback,
        player_sender,
        player_tracks,
        catalog_request: Arc::new(Mutex::new(None)),
    };
    wire_callbacks(&window, &context);
    let player_ui = wire_player_controls(&window, &context, image_is_cleared.clone());
    install_renderer(&window, render_client, image_is_cleared.clone(), player_ui)?;
    let close_worker = player_worker.clone();
    let close_weak = window.as_weak();
    let close_image_is_cleared = image_is_cleared.clone();
    window.window().on_close_requested(move || {
        if let Some(worker) = close_worker.borrow_mut().take() {
            worker.shutdown();
        }
        if let Some(window) = close_weak.upgrade() {
            close_image_is_cleared.set(true);
            window.set_video_frame(Image::default());
        }
        slint::CloseRequestResponse::HideWindow
    });
    restore_session(&context);

    tracing::info!(
        renderer = RENDERER_NAME,
        "Rust JellyBridge application ready"
    );
    window.run()?;
    context.next_generation();
    if let Some(worker) = player_worker.borrow_mut().take() {
        worker.shutdown();
    }
    image_is_cleared.set(true);
    window.set_video_frame(Image::default());
    drop(window);
    let remote = context
        .playback
        .lock()
        .ok()
        .and_then(|mut slot| slot.take());
    if let Some(remote) = remote {
        runtime.block_on(remote.shutdown());
    }
    runtime.shutdown_timeout(std::time::Duration::from_secs(2));
    Ok(())
}

fn wire_callbacks(window: &JellyBridgeWindow, context: &AppContext) {
    let ctx = context.clone();
    window.on_login(move |server, username, password| {
        start_login(
            &ctx,
            server.as_str().to_owned(),
            username.as_str().to_owned(),
            password.as_str().to_owned(),
        );
    });

    let ctx = context.clone();
    window.on_logout(move || logout(&ctx));

    let ctx = context.clone();
    window.on_open_item(move |id, _kind, _container, _playable| {
        open_item(&ctx, id.as_str());
    });

    let ctx = context.clone();
    window.on_search(move |term| start_search(&ctx, term.as_str().to_owned()));

    let ctx = context.clone();
    window.on_sort_changed(move |index| change_catalog_sort(&ctx, index));

    let ctx = context.clone();
    window.on_density_changed(move |index| change_catalog_density(&ctx, index));

    let ctx = context.clone();
    window.on_go_home(move || show_home(&ctx, true));

    let ctx = context.clone();
    window.on_go_back(move || go_back(&ctx));

    let ctx = context.clone();
    window.on_retry(move || retry(&ctx));

    let ctx = context.clone();
    window.on_play_current(move || start_playback(&ctx));

    let ctx = context.clone();
    window.on_open_detail(move |id, kind| {
        start_detail(
            &ctx,
            DetailTarget::new(id.as_str(), DetailKind::from_jellyfin(kind.as_str())),
        );
    });

    let ctx = context.clone();
    window.on_play_detail(move |id| {
        let item = ctx
            .state
            .lock()
            .ok()
            .and_then(|state| state.detail.item_by_id(id.as_str()));
        if let Some(item) = item {
            start_playback_item(&ctx, item);
        }
    });

    let ctx = context.clone();
    window.on_set_detail_watched(move |id, watched| {
        update_detail_mutation(
            &ctx,
            DetailMessage::SetWatched {
                id: id.as_str().to_owned(),
                watched,
            },
        );
    });

    let ctx = context.clone();
    window.on_set_detail_favorite(move |id, favorite| {
        update_detail_mutation(
            &ctx,
            DetailMessage::SetFavorite {
                id: id.as_str().to_owned(),
                favorite,
            },
        );
    });

    let ctx = context.clone();
    window.on_detail_audio_selected(move |index| {
        select_detail_track(&ctx, index, true);
    });

    let ctx = context.clone();
    window.on_detail_subtitle_selected(move |index| {
        select_detail_track(&ctx, index, false);
    });
}

fn wire_player_controls(
    window: &JellyBridgeWindow,
    context: &AppContext,
    image_is_cleared: Rc<Cell<bool>>,
) -> Rc<RefCell<PlayerUiState>> {
    let sender = context.player_sender.clone();
    window.on_toggle_playback(move || {
        let _ = sender.send(PlayerCommand::TogglePaused);
    });
    let sender = context.player_sender.clone();
    window.on_seek_relative(move |seconds| {
        let _ = sender.send(PlayerCommand::SeekRelative(seconds as f64));
    });
    let sender = context.player_sender.clone();
    window.on_seek_absolute(move |seconds| {
        let _ = sender.send(PlayerCommand::SeekAbsolute(seconds as f64));
    });
    let sender = context.player_sender.clone();
    window.on_volume_changed(move |volume| {
        let _ = sender.send(PlayerCommand::SetVolume(volume as f64));
    });
    let sender = context.player_sender.clone();
    window.on_toggle_mute(move || {
        let _ = sender.send(PlayerCommand::ToggleMuted);
    });

    let sender = context.player_sender.clone();
    let tracks = context.player_tracks.clone();
    let settings = context.settings_store.clone();
    window.on_select_audio(move |index| {
        let selection = usize::try_from(index).ok().and_then(|index| {
            let tracks = tracks.lock().ok()?;
            Some((
                tracks.audio.get(index)?.clone(),
                tracks.preference_key.clone(),
            ))
        });
        if let Some((track, key)) = selection {
            let _ = sender.send(PlayerCommand::SelectAudio(track.id));
            persist_player_track_preference(&settings, key.as_deref(), Some(&track), None);
        }
    });
    let sender = context.player_sender.clone();
    let tracks = context.player_tracks.clone();
    let settings = context.settings_store.clone();
    window.on_select_subtitle(move |index| {
        let selection = usize::try_from(index).ok().and_then(|index| {
            let tracks = tracks.lock().ok()?;
            let track = if index == 0 {
                None
            } else {
                Some(tracks.subtitles.get(index - 1)?.clone())
            };
            Some((track, tracks.preference_key.clone()))
        });
        if let Some((track, key)) = selection {
            let _ = sender.send(PlayerCommand::SelectSubtitle(
                track.as_ref().map(|track| track.id),
            ));
            persist_player_track_preference(&settings, key.as_deref(), None, Some(track.as_ref()));
        }
    });

    let player_ui = Rc::new(RefCell::new(PlayerUiState::default()));
    let controls_timer = Rc::new(slint::Timer::default());
    let timer_weak = window.as_weak();
    let timer_state = player_ui.clone();
    controls_timer.start(
        slint::TimerMode::SingleShot,
        std::time::Duration::from_secs(3),
        move || {
            timer_state.borrow_mut().auto_hide_overlay();
            if let Some(window) = timer_weak.upgrade() {
                window.set_controls_visible(timer_state.borrow().overlay_visible);
            }
        },
    );
    controls_timer.stop();

    let weak = window.as_weak();
    let fullscreen_state = player_ui.clone();
    let fullscreen_timer = controls_timer.clone();
    window.on_toggle_fullscreen(move || {
        if let Some(window) = weak.upgrade() {
            let fullscreen = !window.window().is_fullscreen();
            fullscreen_state.borrow_mut().set_fullscreen(fullscreen);
            window.set_fullscreen_player(fullscreen);
            window.set_controls_visible(true);
            window.window().set_fullscreen(fullscreen);
            if fullscreen {
                fullscreen_timer.restart();
            } else {
                fullscreen_timer.stop();
            }
        }
    });
    let weak = window.as_weak();
    let activity_state = player_ui.clone();
    let activity_timer = controls_timer.clone();
    window.on_player_activity(move || {
        activity_state.borrow_mut().pointer_activity();
        if let Some(window) = weak.upgrade() {
            window.set_controls_visible(true);
            if activity_state.borrow().fullscreen {
                activity_timer.restart();
            }
        }
    });

    let sender = context.player_sender.clone();
    let weak = window.as_weak();
    let ctx = context.clone();
    let leave_state = player_ui.clone();
    let leave_timer = controls_timer;
    window.on_leave_player(move || {
        let _ = sender.send(PlayerCommand::Stop);
        stop_remote_playback(&ctx);
        if let Some(window) = weak.upgrade() {
            leave_state.borrow_mut().set_fullscreen(false);
            leave_timer.stop();
            image_is_cleared.set(true);
            window.set_video_frame(Image::default());
            window.window().set_fullscreen(false);
            window.set_fullscreen_player(false);
            window.set_controls_visible(true);
            if ctx
                .state
                .lock()
                .is_ok_and(|state| state.detail.snapshot().page.is_some())
            {
                window.set_screen("detail".into());
            } else {
                window.set_screen("catalog".into());
            }
        }
    });

    let weak = window.as_weak();
    window.on_player_key_pressed(move |text, repeat, shift, command_modifier| {
        let Some(window) = weak.upgrade() else {
            return false;
        };
        let decision = resolve_player_hotkey(
            text.as_str(),
            repeat,
            shift,
            command_modifier,
            window.window().is_fullscreen(),
        );
        let Some(action) = decision.action else {
            return decision.consumed;
        };

        window.invoke_player_activity();
        match action {
            PlayerHotkeyAction::TogglePlayback => window.invoke_toggle_playback(),
            PlayerHotkeyAction::SeekRelative(seconds) => {
                window.invoke_seek_relative(seconds as f32);
            }
            PlayerHotkeyAction::SeekToStart => window.invoke_seek_absolute(0.0),
            PlayerHotkeyAction::SeekToEnd => {
                let duration = window.get_playback_duration();
                if duration > 0.0 {
                    window.invoke_seek_absolute(duration);
                }
            }
            PlayerHotkeyAction::AdjustVolume(delta) => {
                let volume = (window.get_volume() + delta).clamp(0.0, 100.0);
                window.set_volume(volume);
                window.invoke_volume_changed(volume);
            }
            PlayerHotkeyAction::ToggleMute => window.invoke_toggle_mute(),
            PlayerHotkeyAction::ToggleFullscreen | PlayerHotkeyAction::ExitFullscreen => {
                window.invoke_toggle_fullscreen();
            }
            PlayerHotkeyAction::CycleAudio => {
                if let Some(index) = next_track_index(
                    window.get_audio_index(),
                    window.get_audio_options().row_count(),
                ) {
                    window.set_audio_index(index);
                    window.invoke_select_audio(index);
                }
            }
            PlayerHotkeyAction::CycleSubtitle => {
                if let Some(index) = next_track_index(
                    window.get_subtitle_index(),
                    window.get_subtitle_options().row_count(),
                ) {
                    window.set_subtitle_index(index);
                    window.invoke_select_subtitle(index);
                }
            }
            PlayerHotkeyAction::LeavePlayer => window.invoke_leave_player(),
        }
        true
    });
    player_ui
}

fn install_renderer(
    window: &JellyBridgeWindow,
    render_client: jellybridge_vertical_slice::player::RenderClient,
    image_is_cleared: Rc<Cell<bool>>,
    player_ui: Rc<RefCell<PlayerUiState>>,
) -> Result<(), Box<dyn std::error::Error>> {
    let redraw_weak = window.as_weak();
    let redraw: Arc<dyn Fn() + Send + Sync> = Arc::new(move || {
        let weak = redraw_weak.clone();
        let _ = slint::invoke_from_event_loop(move || {
            if let Some(window) = weak.upgrade() {
                window.window().request_redraw();
            }
        });
    });
    let mut render_host = RenderHost::new(render_client, redraw);
    let render_weak = window.as_weak();
    let render_image_is_cleared = image_is_cleared.clone();
    window
        .window()
        .set_rendering_notifier(move |state, graphics_api| {
            let Some(window) = render_weak.upgrade() else {
                return;
            };
            let result = match state {
                RenderingState::RenderingSetup => native_display(window.window())
                    .and_then(|display| render_host.setup(graphics_api, display))
                    .map(|()| None),
                RenderingState::BeforeRendering if window.get_screen().as_str() == "player" => {
                    let scale = window.window().scale_factor();
                    let width = (window.get_video_logical_width().max(1.0) * scale).round() as u32;
                    let height =
                        (window.get_video_logical_height().max(1.0) * scale).round() as u32;
                    render_host.render_frame(slint::PhysicalSize::new(width, height))
                }
                RenderingState::BeforeRendering => Ok(None),
                RenderingState::AfterRendering => {
                    if window.get_screen().as_str() == "player" {
                        let actual_fullscreen = window.window().is_fullscreen();
                        if player_ui.borrow().fullscreen != actual_fullscreen {
                            player_ui.borrow_mut().set_fullscreen(actual_fullscreen);
                            window.set_fullscreen_player(actual_fullscreen);
                            window.set_controls_visible(true);
                        }
                        render_host.finish_frame(window.window().size());
                    }
                    Ok(None)
                }
                RenderingState::RenderingTeardown => {
                    render_host.teardown(render_image_is_cleared.get());
                    Ok(None)
                }
                _ => Ok(None),
            };
            match result {
                Ok(Some(image)) => {
                    render_image_is_cleared.set(false);
                    window.set_video_frame(image);
                }
                Ok(None) => {}
                Err(error) => {
                    tracing::error!(%error, "application render seam failed");
                    window.set_player_status(format!("Rendering failed: {error}").into());
                }
            }
        })?;
    Ok(())
}

fn start_playback(context: &AppContext) {
    let detail = context.state.lock().ok().and_then(|mut state| {
        let id = state
            .detail
            .update(DetailMessage::Play)
            .into_iter()
            .find_map(|effect| match effect {
                DetailEffect::Play(id) => Some(id),
                _ => None,
            })?;
        state.detail.item_by_id(&id)
    });
    let Some(detail) = detail else { return };
    start_playback_item(context, detail);
}

fn start_playback_item(context: &AppContext, detail: DetailItemData) {
    let Some(session) = context
        .state
        .lock()
        .ok()
        .and_then(|state| state.api.as_ref().map(|api| api.session().clone()))
    else {
        return;
    };
    if !detail.playable {
        return;
    }
    let initial_position = detail.resume_seconds.max(0.0);
    let initial_duration = detail
        .runtime_seconds
        .unwrap_or_default()
        .max(initial_position);
    let settings = context.settings_store.load().unwrap_or_default();
    let (audio_source_index, subtitle_source_index) = resolved_playback_sources(&detail, &settings);
    let preference_key = playback_preference_key(&detail);
    let config = match JellyfinSliceConfig::from_session(
        &session.server_url,
        detail.id.clone(),
        session.user_id,
        session.token,
        None,
    ) {
        Ok(config) => config,
        Err(error) => {
            set_message(context, &error.to_string());
            return;
        }
    };
    let generation = context.next_generation();
    set_screen_busy(context, "detail", true, "Preparing authenticated playback…");
    let ctx = context.clone();
    context.runtime.spawn(async move {
        let old = ctx.playback.lock().ok().and_then(|mut slot| slot.take());
        if let Some(old) = old {
            old.shutdown().await;
        }
        let prepared = prepare_jellyfin_playback(config).await;
        if !ctx.is_current(generation) {
            if let Ok(prepared) = prepared {
                prepared.shutdown().await;
            }
            return;
        }
        match prepared {
            Ok(prepared) => {
                let uri = prepared.local_url.clone();
                let subtitles = prepared
                    .subtitle_streams
                    .iter()
                    .map(|subtitle| PlayerSubtitleFile {
                        path: subtitle.path.clone(),
                        source_index: Some(i64::from(subtitle.stream_index)),
                        language: subtitle.language.clone(),
                        title: subtitle.title.clone(),
                    })
                    .collect();
                if let Ok(mut tracks) = ctx.player_tracks.lock() {
                    tracks.preference_key = Some(preference_key);
                }
                if ctx
                    .player_sender
                    .send(PlayerCommand::Load {
                        uri,
                        start_seconds: initial_position,
                        subtitles,
                        audio_source_index,
                        subtitle_source_index,
                    })
                    .is_err()
                {
                    prepared.shutdown().await;
                    set_message(&ctx, "The MPV player worker is unavailable");
                    return;
                }
                if let Ok(mut slot) = ctx.playback.lock() {
                    *slot = Some(prepared);
                }
                let weak = ctx.window.clone();
                let title = detail.title;
                let _ = slint::invoke_from_event_loop(move || {
                    if let Some(window) = weak.upgrade() {
                        window.set_detail_title(title.into());
                        window.set_playback_position_label(
                            format_playback_time(initial_position).into(),
                        );
                        window.set_playback_duration_label(
                            format_playback_time(initial_duration).into(),
                        );
                        window.set_paused(false);
                        window
                            .invoke_enter_player(initial_position as f32, initial_duration as f32);
                        window.set_busy(false);
                        window.set_message("".into());
                        window.set_player_status("Loading media…".into());
                        window.window().request_redraw();
                    }
                });
            }
            Err(error) => {
                let weak = ctx.window.clone();
                let message = error.to_string();
                let _ = slint::invoke_from_event_loop(move || {
                    if let Some(window) = weak.upgrade() {
                        window.set_busy(false);
                        window.set_message(message.into());
                    }
                });
            }
        }
    });
}

fn resolved_playback_sources(
    item: &DetailItemData,
    settings: &StoredSettings,
) -> (Option<i64>, Option<i64>) {
    let audio = item
        .streams
        .iter()
        .filter(|stream| stream.kind.eq_ignore_ascii_case("audio"))
        .collect::<Vec<_>>();
    let subtitles = item
        .streams
        .iter()
        .filter(|stream| stream.kind.eq_ignore_ascii_case("subtitle"))
        .collect::<Vec<_>>();
    let preference = settings
        .playback_preferences
        .get(&playback_preference_key(item));
    let selected_audio = preference
        .and_then(|preference| {
            best_preference_match(&audio, &preference.audio_language, &preference.audio_title)
        })
        .or_else(|| audio.iter().position(|stream| stream.is_default))
        .or_else(|| (!audio.is_empty()).then_some(0))
        .and_then(|index| audio.get(index))
        .map(|stream| i64::from(stream.index));
    let selected_subtitle = if preference.is_some_and(|preference| preference.subtitles_disabled) {
        Some(-1)
    } else {
        preference
            .and_then(|preference| {
                best_preference_match(
                    &subtitles,
                    &preference.subtitle_language,
                    &preference.subtitle_title,
                )
            })
            .or_else(|| subtitles.iter().position(|stream| stream.is_default))
            .and_then(|index| subtitles.get(index))
            .map(|stream| i64::from(stream.index))
    };
    (selected_audio, selected_subtitle)
}

fn stop_remote_playback(context: &AppContext) {
    let remote = context
        .playback
        .lock()
        .ok()
        .and_then(|mut slot| slot.take());
    if let Some(remote) = remote {
        context.runtime.spawn(remote.shutdown());
    }
}

fn update_window_from_player(
    weak: &slint::Weak<JellyBridgeWindow>,
    event: PlayerEvent,
    track_state: &Arc<Mutex<PlayerTrackState>>,
) {
    let Some(window) = weak.upgrade() else { return };
    match event {
        PlayerEvent::Loaded => window.set_player_status("Playing; decode path pending".into()),
        PlayerEvent::Paused(paused) => {
            window.set_paused(paused);
            window.set_player_status(if paused { "Paused" } else { "Playing" }.into());
        }
        PlayerEvent::Volume(volume) => window.set_volume(volume as f32),
        PlayerEvent::Position { seconds, duration } => {
            window.set_playback_position(seconds as f32);
            window.set_playback_duration(duration as f32);
            window.set_playback_position_label(format_playback_time(seconds).into());
            window.set_playback_duration_label(format_playback_time(duration).into());
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
            let audio_labels = audio
                .iter()
                .map(|track| SharedString::from(track.label.as_str()))
                .collect::<Vec<_>>();
            let mut subtitle_labels = vec![SharedString::from("Off")];
            subtitle_labels.extend(
                subtitles
                    .iter()
                    .map(|track| SharedString::from(track.label.as_str())),
            );
            if let Ok(mut state) = track_state.lock() {
                state.audio.clone_from(&audio);
                state.subtitles.clone_from(&subtitles);
            }
            window.set_audio_options(ModelRc::new(VecModel::from(audio_labels)));
            window.set_subtitle_options(ModelRc::new(VecModel::from(subtitle_labels)));
            window.set_audio_index(i32::try_from(selected_audio_index).unwrap_or(i32::MAX));
            window.set_subtitle_index(i32::try_from(selected_subtitle_index).unwrap_or(i32::MAX));
        }
        PlayerEvent::HardwarePath(path) => {
            tracing::info!(decode_path = %path, "application playback decode path");
            window.set_player_status(format!("Decode path: {path}").into());
        }
        PlayerEvent::Ended => window.set_player_status("Playback ended".into()),
        PlayerEvent::Error(message) => {
            tracing::error!(%message, "application player failed");
            window.set_player_status(message.into());
        }
    }
}

fn format_playback_time(seconds: f64) -> String {
    let total = seconds.max(0.0).round() as u64;
    let hours = total / 3600;
    let minutes = (total % 3600) / 60;
    let seconds = total % 60;
    if hours > 0 {
        format!("{hours}:{minutes:02}:{seconds:02}")
    } else {
        format!("{minutes}:{seconds:02}")
    }
}

fn restore_session(context: &AppContext) {
    match context.store.load() {
        Ok(Some(session)) => match JellyfinApi::from_session(session.clone()) {
            Ok(api) => {
                if let Ok(mut state) = context.state.lock() {
                    state.account_name = api
                        .session()
                        .server_url
                        .split("//")
                        .nth(1)
                        .unwrap_or("Saved session")
                        .trim_end_matches('/')
                        .to_owned();
                    state.api = Some(api);
                }
                start_home_load(context);
            }
            Err(error) => show_login(context, &error.to_string()),
        },
        Ok(None) => show_login(context, ""),
        Err(error) => show_login(context, &error.to_string()),
    }
}

fn start_login(context: &AppContext, server: String, username: String, password: String) {
    let generation = context.next_generation();
    set_screen_busy(context, "login", true, "");
    let ctx = context.clone();
    context.runtime.spawn(async move {
        let result = async {
            let (api, account_name) = JellyfinApi::authenticate(&server, &username, &password)
                .await
                .map_err(|error| error.to_string())?;
            ctx.store
                .save(api.session())
                .map_err(|error| error.to_string())?;
            let home = api.home().await.map_err(|error| error.to_string())?;
            Ok::<_, String>((api, account_name, home))
        }
        .await;
        if !ctx.is_current(generation) {
            return;
        }
        match result {
            Ok((api, account_name, home)) => {
                install_home(&ctx, api, account_name, home);
            }
            Err(error) => show_login(&ctx, &error),
        }
    });
}

fn start_home_load(context: &AppContext) {
    let Some(api) = context.api() else {
        show_login(context, "Please sign in to continue");
        return;
    };
    let generation = context.next_generation();
    set_screen_busy(context, "catalog", true, "");
    if let Ok(mut state) = context.state.lock() {
        state.retry = RetryAction::Home;
    }
    let ctx = context.clone();
    let task = context.runtime.spawn(async move {
        let result = api.home().await;
        if !ctx.is_current(generation) {
            return;
        }
        match result {
            Ok(home) => {
                let account = ctx
                    .state
                    .lock()
                    .map(|state| state.account_name.clone())
                    .unwrap_or_default();
                install_home(&ctx, api, account, home);
            }
            Err(ApiError::Unauthorized) => {
                let _ = ctx.store.clear();
                if let Ok(mut state) = ctx.state.lock() {
                    reset_session_state(&mut state);
                }
                show_login(
                    &ctx,
                    "Your saved session is no longer valid. Please sign in again.",
                );
            }
            Err(error) => show_catalog_error(&ctx, error.to_string()),
        }
    });
    replace_catalog_request(context, task.abort_handle());
}

fn install_home(
    context: &AppContext,
    api: JellyfinApi,
    account_name: String,
    home: Vec<CatalogSectionData>,
) {
    let current = PageSnapshot::home(&home);
    if let Ok(mut state) = context.state.lock() {
        state.api = Some(api);
        state.account_name = account_name;
        state.home = home;
        state.current = current;
        state.back_stack.clear();
        state.detail.update(DetailMessage::Reset);
        state.retry = RetryAction::Home;
    }
    show_current_catalog(context, "");
}

fn show_home(context: &AppContext, clear_history: bool) {
    context.next_generation();
    cancel_catalog_request(context);
    if let Ok(mut state) = context.state.lock() {
        state.current = PageSnapshot::home(&state.home);
        if clear_history {
            state.back_stack.clear();
        }
        state.detail.update(DetailMessage::Reset);
    }
    show_current_catalog(context, "");
}

fn open_item(context: &AppContext, id: &str) {
    let item = context.state.lock().ok().and_then(|state| {
        state
            .current
            .items
            .iter()
            .find(|item| item.id == id)
            .cloned()
    });
    let Some(item) = item else { return };
    let detail_kind = DetailKind::from_jellyfin(&item.kind);
    if !matches!(detail_kind, DetailKind::Other) {
        start_detail(context, DetailTarget::new(item.id, detail_kind));
    } else if item.is_container {
        start_browse(context, item);
    }
}

fn start_browse(context: &AppContext, item: CatalogItemData) {
    start_browse_request(context, item, true);
}

fn start_browse_request(context: &AppContext, item: CatalogItemData, push_history: bool) {
    let Some(api) = context.api() else { return };
    let generation = context.next_generation();
    let sort = context
        .state
        .lock()
        .map(|state| state.catalog_sort)
        .unwrap_or_default();
    if let Ok(mut state) = context.state.lock() {
        state.retry = RetryAction::Browse(item.clone(), push_history);
    }
    set_screen_busy(context, "catalog", true, "");
    let ctx = context.clone();
    let task = context.runtime.spawn(async move {
        let result = api.browse(&item, sort).await;
        if !ctx.is_current(generation) {
            return;
        }
        match result {
            Ok(page) => {
                if let Ok(mut state) = ctx.state.lock() {
                    if push_history {
                        let previous = state.current.clone();
                        state.back_stack.push(previous);
                    }
                    state.current = PageSnapshot {
                        title: item.title.clone(),
                        items: page.items,
                        sections: Vec::new(),
                        parent: Some(item.clone()),
                        search_text: String::new(),
                        total_record_count: page.total_record_count,
                    };
                    state.detail.update(DetailMessage::Reset);
                }
                show_current_catalog(&ctx, "");
            }
            Err(error) => show_catalog_error(&ctx, error.to_string()),
        }
    });
    replace_catalog_request(context, task.abort_handle());
}

fn start_search(context: &AppContext, term: String) {
    let term = term.trim().to_owned();
    if term.is_empty() {
        let destination = context
            .state
            .lock()
            .map(|state| empty_search_destination(&state.current))
            .unwrap_or(EmptySearchDestination::Ignore);
        match destination {
            EmptySearchDestination::Ignore => {}
            EmptySearchDestination::Home => show_home(context, false),
            EmptySearchDestination::Parent(parent) => {
                start_browse_request(context, parent, false);
            }
        }
        return;
    }
    let Some(api) = context.api() else { return };
    let generation = context.next_generation();
    let (parent, sort) = context
        .state
        .lock()
        .map(|state| (state.current.parent.clone(), state.catalog_sort))
        .unwrap_or((None, CatalogSort::default()));
    if let Ok(mut state) = context.state.lock() {
        state.retry = RetryAction::Search(term.clone());
        state.current.search_text.clone_from(&term);
    }
    set_screen_busy(context, "catalog", true, "");
    let ctx = context.clone();
    let task = context.runtime.spawn(async move {
        let result = api
            .search(&term, parent.as_ref().map(|item| item.id.as_str()), sort)
            .await;
        if !ctx.is_current(generation) {
            return;
        }
        match result {
            Ok(page) => {
                if let Ok(mut state) = ctx.state.lock() {
                    state.current.items = page.items;
                    state.current.sections.clear();
                    state.current.search_text.clone_from(&term);
                    state.current.total_record_count = page.total_record_count;
                    state.detail.update(DetailMessage::Reset);
                }
                show_current_catalog(&ctx, "");
            }
            Err(error) => show_catalog_error(&ctx, error.to_string()),
        }
    });
    replace_catalog_request(context, task.abort_handle());
}

fn change_catalog_sort(context: &AppContext, index: i32) {
    let Some(sort) = CatalogSort::from_index(index) else {
        return;
    };
    let (search_text, parent, is_home_dashboard) = match context.state.lock() {
        Ok(mut state) => {
            state.catalog_sort = sort;
            (
                state.current.search_text.clone(),
                state.current.parent.clone(),
                state.current.is_home_dashboard(),
            )
        }
        Err(_) => return,
    };
    persist_catalog_preferences(context);
    if is_home_dashboard {
        set_catalog_preference_properties(context);
    } else if !search_text.is_empty() {
        start_search(context, search_text);
    } else if let Some(parent) = parent {
        start_browse_request(context, parent, false);
    }
}

fn change_catalog_density(context: &AppContext, index: i32) {
    let Some(density) = CatalogDensity::from_index(index) else {
        return;
    };
    if let Ok(mut state) = context.state.lock() {
        state.catalog_density = density;
    }
    persist_catalog_preferences(context);
    set_catalog_preference_properties(context);
}

fn persist_catalog_preferences(context: &AppContext) {
    let (density, sort) = match context.state.lock() {
        Ok(state) => (state.catalog_density, state.catalog_sort),
        Err(_) => return,
    };
    if let Err(error) = context
        .settings_store
        .save_catalog_preferences(density.card_width(), sort.key())
    {
        tracing::warn!(%error, "catalog preferences could not be saved");
        set_message(context, "Catalog preferences could not be saved");
    }
}

fn set_catalog_preference_properties(context: &AppContext) {
    let (density, sort) = match context.state.lock() {
        Ok(state) => (state.catalog_density, state.catalog_sort),
        Err(_) => return,
    };
    let weak = context.window.clone();
    let _ = slint::invoke_from_event_loop(move || {
        if let Some(window) = weak.upgrade() {
            window.set_density_index(density.index());
            window.set_tile_target_width(density.card_width() as f32);
            window.set_sort_index(sort.index());
        }
    });
}

fn start_detail(context: &AppContext, target: DetailTarget) {
    let Some(api) = context.api() else { return };
    let generation = context.next_generation();
    cancel_catalog_request(context);
    if let Ok(mut state) = context.state.lock() {
        state.retry = RetryAction::Detail(target.clone());
        state.detail.update(DetailMessage::Open(target.clone()));
    }
    set_screen_busy(context, "detail", true, "");
    let ctx = context.clone();
    context.runtime.spawn(async move {
        let result = api.detail_page(&target.id).await;
        if !ctx.is_current(generation) {
            return;
        }
        match result {
            Ok(page) => {
                if let Ok(mut state) = ctx.state.lock() {
                    state.detail.update(DetailMessage::Loaded(Box::new(page)));
                    let settings = ctx.settings_store.load().unwrap_or_default();
                    state.detail_tracks = detail_track_defaults(&state.detail, &settings);
                }
                show_current_detail(&ctx, generation);
            }
            Err(error) => {
                let message = error.to_string();
                if let Ok(mut state) = ctx.state.lock() {
                    state
                        .detail
                        .update(DetailMessage::LoadFailed(message.clone()));
                }
                show_current_detail(&ctx, generation);
            }
        }
    });
}

fn update_detail_mutation(context: &AppContext, message: DetailMessage) {
    let effects = context
        .state
        .lock()
        .map(|mut state| state.detail.update(message))
        .unwrap_or_default();
    let generation = context.generation.load(Ordering::Acquire);
    show_current_detail(context, generation);
    for effect in effects {
        let (id, kind, future) = match effect {
            DetailEffect::SetWatched { id, watched } => {
                let Some(api) = context.api() else { continue };
                let call_id = id.clone();
                (
                    id,
                    DetailMutationKind::Watched,
                    context.runtime.spawn(async move {
                        api.set_watched(&call_id, watched)
                            .await
                            .map_err(|error| error.to_string())
                    }),
                )
            }
            DetailEffect::SetFavorite { id, favorite } => {
                let Some(api) = context.api() else { continue };
                let call_id = id.clone();
                (
                    id,
                    DetailMutationKind::Favorite,
                    context.runtime.spawn(async move {
                        api.set_favorite(&call_id, favorite)
                            .await
                            .map_err(|error| error.to_string())
                    }),
                )
            }
            _ => continue,
        };
        let ctx = context.clone();
        context.runtime.spawn(async move {
            let error = match future.await {
                Ok(Ok(())) => None,
                Ok(Err(error)) => Some(error),
                Err(error) => Some(format!("detail update task failed: {error}")),
            };
            if let Ok(mut state) = ctx.state.lock() {
                state
                    .detail
                    .update(DetailMessage::MutationFinished { id, kind, error });
            }
            if ctx.is_current(generation) {
                show_current_detail(&ctx, generation);
            }
        });
    }
}

fn select_detail_track(context: &AppContext, index: i32, audio: bool) {
    let preference = context.state.lock().ok().and_then(|mut state| {
        if audio {
            if usize::try_from(index)
                .ok()
                .is_none_or(|index| index >= state.detail_tracks.audio_ids.len())
            {
                return None;
            }
            state.detail_tracks.audio_index = index;
        } else {
            if usize::try_from(index)
                .ok()
                .is_none_or(|index| index >= state.detail_tracks.subtitle_ids.len())
            {
                return None;
            }
            state.detail_tracks.subtitle_index = index;
        }
        selected_detail_preference(&state)
    });
    let Some((key, preference)) = preference else {
        return;
    };
    if let Err(error) = context
        .settings_store
        .save_playback_preference(&key, preference)
    {
        tracing::warn!(%error, "playback preference could not be saved");
        set_message(context, "Playback preference could not be saved");
    }
}

fn selected_detail_preference(state: &AppState) -> Option<(String, StoredPlaybackPreference)> {
    let item = &state.detail.snapshot().page?.item;
    let settings = state.detail_tracks.clone();
    let audio_stream = usize::try_from(settings.audio_index)
        .ok()
        .and_then(|index| settings.audio_ids.get(index))
        .and_then(|selected| {
            item.streams.iter().find(|stream| {
                stream.kind.eq_ignore_ascii_case("audio") && stream.index == *selected
            })
        });
    let subtitle_selection = usize::try_from(settings.subtitle_index)
        .ok()
        .and_then(|index| settings.subtitle_ids.get(index))
        .copied()
        .flatten();
    let subtitle_stream = subtitle_selection.and_then(|selected| {
        item.streams
            .iter()
            .find(|stream| stream.kind.eq_ignore_ascii_case("subtitle") && stream.index == selected)
    });
    Some((
        playback_preference_key(item),
        StoredPlaybackPreference {
            audio_language: audio_stream.and_then(|stream| clean_preference(&stream.language)),
            audio_title: audio_stream.and_then(|stream| clean_preference(&stream.title)),
            subtitle_language: subtitle_stream
                .and_then(|stream| clean_preference(&stream.language)),
            subtitle_title: subtitle_stream.and_then(|stream| clean_preference(&stream.title)),
            subtitles_disabled: subtitle_selection.is_none() && settings.subtitle_ids.len() > 1,
        },
    ))
}

fn detail_track_defaults(model: &DetailModel, settings: &StoredSettings) -> DetailTrackSelection {
    let Some(page) = model.snapshot().page else {
        return DetailTrackSelection::default();
    };
    let audio = page
        .item
        .streams
        .iter()
        .filter(|stream| stream.kind.eq_ignore_ascii_case("audio"))
        .collect::<Vec<_>>();
    let subtitles = page
        .item
        .streams
        .iter()
        .filter(|stream| stream.kind.eq_ignore_ascii_case("subtitle"))
        .collect::<Vec<_>>();
    let preference = settings
        .playback_preferences
        .get(&playback_preference_key(&page.item));
    let audio_index = preference
        .and_then(|preference| {
            best_preference_match(&audio, &preference.audio_language, &preference.audio_title)
        })
        .or_else(|| audio.iter().position(|stream| stream.is_default))
        .and_then(|index| i32::try_from(index).ok())
        .unwrap_or(0);
    let subtitle_default = if preference.is_some_and(|preference| preference.subtitles_disabled) {
        0
    } else {
        preference
            .and_then(|preference| {
                best_preference_match(
                    &subtitles,
                    &preference.subtitle_language,
                    &preference.subtitle_title,
                )
            })
            .or_else(|| subtitles.iter().position(|stream| stream.is_default))
            .and_then(|index| i32::try_from(index + 1).ok())
            .unwrap_or(0)
    };
    DetailTrackSelection {
        audio_ids: audio.iter().map(|stream| stream.index).collect(),
        subtitle_ids: std::iter::once(None)
            .chain(subtitles.iter().map(|stream| Some(stream.index)))
            .collect(),
        audio_index,
        subtitle_index: subtitle_default,
    }
}

fn playback_preference_key(item: &DetailItemData) -> String {
    match item.kind {
        DetailKind::Episode => {
            if let Some(id) = item.series_id.as_ref() {
                format!("series:{id}")
            } else if let Some(id) = item.season_id.as_ref() {
                format!("season:{id}")
            } else if let Some(id) = item.parent_id.as_ref() {
                format!("item:{id}")
            } else {
                format!("item:{}", item.id)
            }
        }
        DetailKind::Movie => format!("movie:{}", item.id),
        _ => format!("item:{}", item.id),
    }
}

fn persist_player_track_preference(
    store: &SettingsStore,
    key: Option<&str>,
    audio: Option<&PlayerTrack>,
    subtitle: Option<Option<&PlayerTrack>>,
) {
    let Some(key) = key else { return };
    let settings = match store.load() {
        Ok(settings) => settings,
        Err(error) => {
            tracing::warn!(%error, "playback preference could not be loaded");
            return;
        }
    };
    let mut preference = settings
        .playback_preferences
        .get(key)
        .cloned()
        .unwrap_or_default();
    if let Some(track) = audio {
        preference.audio_language = clean_preference(&track.language);
        preference.audio_title = clean_preference(&track.title);
    }
    if let Some(track) = subtitle {
        if let Some(track) = track {
            preference.subtitle_language = clean_preference(&track.language);
            preference.subtitle_title = clean_preference(&track.title);
            preference.subtitles_disabled = false;
        } else {
            preference.subtitles_disabled = true;
        }
    }
    if let Err(error) = store.save_playback_preference(key, preference) {
        tracing::warn!(%error, "playback preference could not be saved");
    }
}

fn best_preference_match<T>(
    streams: &[T],
    language: &Option<String>,
    title: &Option<String>,
) -> Option<usize>
where
    T: std::borrow::Borrow<detail::StreamData>,
{
    let language = normalized_preference(language);
    let title = normalized_preference(title);
    if language.is_none() && title.is_none() {
        return None;
    }
    streams
        .iter()
        .enumerate()
        .map(|(index, stream)| {
            let stream = stream.borrow();
            let mut score = 0;
            if language.is_some() && normalized_preference(&stream.language) == language {
                score += 4;
            }
            if title.is_some() && normalized_preference(&stream.title) == title {
                score += 2;
            }
            (index, score)
        })
        .filter(|(_, score)| *score > 0)
        .max_by_key(|(_, score)| *score)
        .map(|(index, _)| index)
}

fn normalized_preference(value: &Option<String>) -> Option<String> {
    clean_preference(value).map(|value| value.to_lowercase())
}

fn clean_preference(value: &Option<String>) -> Option<String> {
    value
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_owned)
}

fn go_back(context: &AppContext) {
    context.next_generation();
    cancel_catalog_request(context);
    let detail_effects = context.state.lock().ok().and_then(|mut state| {
        let snapshot = state.detail.snapshot();
        (snapshot.page.is_some() || snapshot.busy || snapshot.error.is_some())
            .then(|| state.detail.update(DetailMessage::Back))
    });
    if let Some(effects) = detail_effects {
        if effects
            .iter()
            .any(|effect| matches!(effect, DetailEffect::ShowCatalog))
        {
            show_current_catalog(context, "");
        } else {
            let generation = context.generation.load(Ordering::Acquire);
            show_current_detail(context, generation);
        }
        return;
    }
    let mut show_catalog = false;
    if let Ok(mut state) = context.state.lock()
        && let Some(previous) = state.back_stack.pop()
    {
        state.current = previous;
        show_catalog = true;
    }
    if show_catalog {
        show_current_catalog(context, "");
    }
}

fn retry(context: &AppContext) {
    let action = context
        .state
        .lock()
        .map(|state| state.retry.clone())
        .unwrap_or(RetryAction::Home);
    match action {
        RetryAction::Home => start_home_load(context),
        RetryAction::Browse(item, push_history) => {
            start_browse_request(context, item, push_history);
        }
        RetryAction::Search(term) => start_search(context, term),
        RetryAction::Detail(target) => start_detail(context, target),
    }
}

fn logout(context: &AppContext) {
    context.next_generation();
    cancel_catalog_request(context);
    let _ = context.player_sender.send(PlayerCommand::Stop);
    stop_remote_playback(context);
    let message = context.store.clear().err().map(|error| error.to_string());
    if let Ok(mut state) = context.state.lock() {
        reset_session_state(&mut state);
    }
    if let Ok(mut tracks) = context.player_tracks.lock() {
        *tracks = PlayerTrackState::default();
    }
    show_login(context, message.as_deref().unwrap_or(""));
}

fn reset_session_state(state: &mut AppState) {
    let sort = state.catalog_sort;
    let density = state.catalog_density;
    *state = AppState::default();
    state.catalog_sort = sort;
    state.catalog_density = density;
}

fn show_current_catalog(context: &AppContext, message: &str) {
    let (current, account, can_go_back, sort, density) = match context.state.lock() {
        Ok(state) => (
            state.current.clone(),
            state.account_name.clone(),
            !state.back_stack.is_empty(),
            state.catalog_sort,
            state.catalog_density,
        ),
        Err(_) => return,
    };
    let ui_sections: Vec<CatalogSection> = current
        .sections
        .iter()
        .map(|section| CatalogSection {
            id: section.id.as_str().into(),
            title: section.title.as_str().into(),
            start_index: i32::try_from(section.start_index).unwrap_or(i32::MAX),
            count: i32::try_from(section.count).unwrap_or(i32::MAX),
        })
        .collect();
    let ui_item_data = current.items.clone();
    let artwork_items = current.items.clone();
    let generation = context.next_generation();
    let weak = context.window.clone();
    let home_dashboard = current.is_home_dashboard();
    let title = if current.parent.is_none() && !current.search_text.is_empty() {
        "Search results".to_owned()
    } else {
        current.title.clone()
    };
    let summary = if home_dashboard {
        format!(
            "{} sections • {} items",
            current.sections.len(),
            current.items.len()
        )
    } else if current.total_record_count > current.items.len() {
        format!(
            "Showing first {} of {}",
            current.items.len(),
            current.total_record_count
        )
    } else {
        let count = current.total_record_count;
        format!("{count} {}", if count == 1 { "item" } else { "items" })
    };
    let search_text = current.search_text.clone();
    let account_initial = account
        .trim()
        .chars()
        .next()
        .map(|character| character.to_uppercase().collect::<String>())
        .unwrap_or_else(|| "?".to_owned());
    let sort_options = CatalogSort::ALL
        .into_iter()
        .map(|sort| SharedString::from(sort.label()))
        .collect::<Vec<_>>();
    let message = message.to_owned();
    let _ = slint::invoke_from_event_loop(move || {
        if let Some(window) = weak.upgrade() {
            window.set_screen("catalog".into());
            window.set_busy(false);
            window.set_message(message.into());
            window.set_page_title(title.into());
            window.set_catalog_summary(summary.into());
            window.set_catalog_home(home_dashboard);
            window.set_search_text(search_text.into());
            window.set_sort_options(ModelRc::new(VecModel::from(sort_options)));
            window.set_sort_index(sort.index());
            window.set_density_index(density.index());
            window.set_tile_target_width(density.card_width() as f32);
            window.set_account_name(account.into());
            window.set_account_initial(account_initial.into());
            window.set_can_go_back(can_go_back);
            window.set_sections(ModelRc::new(VecModel::from(ui_sections)));
            window.set_catalog_items(ModelRc::new(VecModel::from(
                ui_item_data.into_iter().map(to_ui_item).collect::<Vec<_>>(),
            )));
        }
    });
    load_artwork(context, generation, artwork_items);
}

fn show_current_detail(context: &AppContext, generation: u64) {
    let (page, error, can_go_back, tracks) = match context.state.lock() {
        Ok(state) => {
            let snapshot = state.detail.snapshot();
            (
                snapshot.page.cloned(),
                snapshot.error.unwrap_or_default().to_owned(),
                snapshot.can_go_back,
                state.detail_tracks.clone(),
            )
        }
        Err(_) => return,
    };
    let weak = context.window.clone();
    let Some(page) = page else {
        let _ = slint::invoke_from_event_loop(move || {
            if let Some(window) = weak.upgrade() {
                window.set_screen("detail".into());
                window.set_busy(false);
                window.set_message(error.into());
                window.set_can_go_back(can_go_back);
                window.set_detail_id("".into());
                window.set_detail_title("".into());
                window.set_detail_series_id("".into());
                window.set_detail_series_name("".into());
                window.set_detail_season_id("".into());
                window.set_detail_season_name("".into());
                window.set_detail_series_summary("".into());
                window.set_detail_play_context("".into());
                window.set_detail_season_summary("".into());
                window.set_detail_play_label("".into());
                window.set_detail_progress(0.0);
                window.set_detail_progress_label("".into());
                window.set_detail_seasons(ModelRc::new(VecModel::from(Vec::new())));
                window.set_detail_episodes(ModelRc::new(VecModel::from(Vec::new())));
                window.set_detail_people(ModelRc::new(VecModel::from(Vec::new())));
            }
        });
        return;
    };
    let detail_id = page.item.id.clone();
    let title = page.item.title.clone();
    let subtitle = detail_metadata_line(&page.item);
    let overview = page.item.overview.clone();
    let kind = detail_kind_name(page.item.kind).to_owned();
    let tagline = page.item.tagline.clone();
    let genres = page.item.genres.join(" • ");
    let studios = page.item.studios.join(" • ");
    let media_info = page.item.media_summary.clone();
    let series_id = page.item.series_id.clone().unwrap_or_default();
    let series_name = page.item.series_name.clone().unwrap_or_default();
    let season_id = (page.item.kind == DetailKind::Episode)
        .then(|| page.item.season_id.clone())
        .flatten()
        .unwrap_or_default();
    let season_name = (page.item.kind == DetailKind::Episode)
        .then(|| page.item.season_name.clone())
        .flatten()
        .unwrap_or_default();
    let playable = page.item.playable || !page.episodes.is_empty();
    let watched = page.item.watched;
    let favorite = page.item.favorite;
    let wide_art = page.item.kind == DetailKind::Episode;
    let series_summary = detail_series_summary(&page);
    let season_summary = detail_season_summary(&page);
    let play_label = page.primary_play_label().unwrap_or_default().to_owned();
    let play_context = detail_series_play_context(&page);
    let progress_item = if page.item.kind == DetailKind::Series {
        page.play_target().unwrap_or(&page.item)
    } else {
        &page.item
    };
    let progress = progress_item.progress;
    let progress_label = detail_progress_label(progress_item);
    let message = page.children_error.clone().unwrap_or(error);
    let seasons = page.seasons.clone();
    let episodes = page.episodes.clone();
    let people = page.item.people.clone();
    let audio_options = page
        .item
        .streams
        .iter()
        .filter(|stream| stream.kind.eq_ignore_ascii_case("audio"))
        .enumerate()
        .map(|(index, stream)| detail_stream_label(stream, "Audio", index))
        .collect::<Vec<_>>();
    let subtitle_options = std::iter::once("Off".to_owned())
        .chain(
            page.item
                .streams
                .iter()
                .filter(|stream| stream.kind.eq_ignore_ascii_case("subtitle"))
                .enumerate()
                .map(|(index, stream)| detail_stream_label(stream, "Subtitle", index)),
        )
        .collect::<Vec<_>>();
    let audio_index = tracks.audio_index;
    let subtitle_index = tracks.subtitle_index;
    let _ = slint::invoke_from_event_loop(move || {
        if let Some(window) = weak.upgrade() {
            let same_item = window.get_detail_id().as_str() == detail_id;
            let poster = same_item.then(|| window.get_detail_artwork());
            let backdrop = same_item.then(|| window.get_detail_backdrop());
            let logo = same_item.then(|| window.get_detail_logo());
            let mut seasons = seasons.iter().map(to_detail_card).collect::<Vec<_>>();
            let mut episodes = episodes.iter().map(to_detail_card).collect::<Vec<_>>();
            let mut people = people
                .iter()
                .map(|person| DetailPerson {
                    id: person.id.as_deref().unwrap_or_default().into(),
                    name: person.name.as_str().into(),
                    role: person.role.as_str().into(),
                    artwork: Image::default(),
                })
                .collect::<Vec<_>>();
            preserve_detail_card_artwork(&window.get_detail_seasons(), &mut seasons);
            preserve_detail_card_artwork(&window.get_detail_episodes(), &mut episodes);
            preserve_detail_people_artwork(&window.get_detail_people(), &mut people);
            window.set_screen("detail".into());
            window.set_busy(false);
            window.set_message(message.into());
            window.set_detail_id(detail_id.into());
            window.set_detail_title(title.into());
            window.set_detail_subtitle(subtitle.into());
            window.set_detail_overview(overview.into());
            window.set_detail_kind(kind.into());
            window.set_detail_tagline(tagline.into());
            window.set_detail_genres(genres.into());
            window.set_detail_studios(studios.into());
            window.set_detail_media_info(media_info.into());
            window.set_detail_series_id(series_id.into());
            window.set_detail_series_name(series_name.into());
            window.set_detail_season_id(season_id.into());
            window.set_detail_season_name(season_name.into());
            window.set_detail_series_summary(series_summary.into());
            window.set_detail_play_context(play_context.into());
            window.set_detail_season_summary(season_summary.into());
            window.set_detail_play_label(play_label.into());
            window.set_detail_progress(progress);
            window.set_detail_progress_label(progress_label.into());
            window.set_detail_playable(playable);
            window.set_detail_watched(watched);
            window.set_detail_favorite(favorite);
            window.set_detail_wide_art(wide_art);
            window.set_detail_artwork(poster.unwrap_or_default());
            window.set_detail_backdrop(backdrop.unwrap_or_default());
            window.set_detail_logo(logo.unwrap_or_default());
            window.set_detail_seasons(ModelRc::new(VecModel::from(seasons)));
            window.set_detail_episodes(ModelRc::new(VecModel::from(episodes)));
            window.set_detail_people(ModelRc::new(VecModel::from(people)));
            window.set_detail_audio_options(ModelRc::new(VecModel::from(
                audio_options
                    .into_iter()
                    .map(SharedString::from)
                    .collect::<Vec<_>>(),
            )));
            window.set_detail_subtitle_options(ModelRc::new(VecModel::from(
                subtitle_options
                    .into_iter()
                    .map(SharedString::from)
                    .collect::<Vec<_>>(),
            )));
            window.set_detail_audio_index(audio_index);
            window.set_detail_subtitle_index(subtitle_index);
            window.set_can_go_back(can_go_back);
        }
    });
    load_detail_artwork(context, generation, &page);
}

fn preserve_detail_card_artwork(model: &ModelRc<DetailCard>, rows: &mut [DetailCard]) {
    for row in rows {
        if let Some(previous) = (0..model.row_count())
            .filter_map(|index| model.row_data(index))
            .find(|previous| previous.id == row.id)
        {
            row.artwork = previous.artwork;
        }
    }
}

fn preserve_detail_people_artwork(model: &ModelRc<DetailPerson>, rows: &mut [DetailPerson]) {
    for row in rows {
        if let Some(previous) = (0..model.row_count())
            .filter_map(|index| model.row_data(index))
            .find(|previous| previous.id == row.id && previous.name == row.name)
        {
            row.artwork = previous.artwork;
        }
    }
}

fn to_detail_card(item: &DetailItemData) -> DetailCard {
    DetailCard {
        id: item.id.as_str().into(),
        title: item.title.as_str().into(),
        subtitle: detail_card_subtitle(item).into(),
        overview: item.overview.as_str().into(),
        kind: detail_kind_name(item.kind).into(),
        artwork: Image::default(),
        watched: item.watched,
        favorite: item.favorite,
        progress: item.progress,
        unplayed_count: item.unplayed_count.unwrap_or(0),
    }
}

fn detail_card_subtitle(item: &DetailItemData) -> String {
    let episode_number = detail_episode_number(item);
    let runtime = item.runtime_seconds.map(|seconds| {
        let minutes = (seconds / 60.0).round() as u64;
        format!("{minutes} min")
    });
    let unplayed = item
        .unplayed_count
        .filter(|count| *count > 0)
        .map(|count| format!("{count} unwatched"));
    [
        episode_number,
        item.production_year.map(|year| year.to_string()),
        runtime,
        unplayed,
    ]
    .into_iter()
    .flatten()
    .collect::<Vec<_>>()
    .join(" • ")
}

fn detail_episode_number(item: &DetailItemData) -> Option<String> {
    match (item.parent_index_number, item.index_number) {
        (Some(season), Some(episode)) if item.kind == DetailKind::Episode => {
            Some(format!("S{season:02}E{episode:02}"))
        }
        (_, Some(episode)) if item.kind == DetailKind::Episode => {
            Some(format!("Episode {episode}"))
        }
        _ => None,
    }
}

fn detail_stream_label(stream: &detail::StreamData, fallback: &str, index: usize) -> String {
    if !stream.label.is_empty() {
        stream.label.clone()
    } else if let Some(codec) = clean_preference(&stream.codec) {
        codec.to_uppercase()
    } else {
        format!("{fallback} {}", index + 1)
    }
}

fn detail_kind_name(kind: DetailKind) -> &'static str {
    match kind {
        DetailKind::Movie => "Movie",
        DetailKind::Series => "Series",
        DetailKind::Season => "Season",
        DetailKind::Episode => "Episode",
        DetailKind::Other => "Media",
    }
}

fn detail_metadata_line(item: &DetailItemData) -> String {
    let runtime = item.runtime_seconds.map(|seconds| {
        let minutes = (seconds / 60.0).round() as u64;
        if minutes >= 60 {
            format!("{}h {}m", minutes / 60, minutes % 60)
        } else {
            format!("{minutes}m")
        }
    });
    let leading_label = match item.kind {
        DetailKind::Movie => None,
        DetailKind::Episode => detail_episode_number(item),
        _ => Some(detail_kind_name(item.kind).to_owned()),
    };
    [
        leading_label,
        item.production_year.map(|year| year.to_string()),
        item.official_rating.clone(),
        runtime,
        item.community_rating.map(|rating| format!("★ {rating:.1}")),
        item.critic_rating
            .map(|rating| format!("Critics {rating:.0}%")),
    ]
    .into_iter()
    .flatten()
    .collect::<Vec<_>>()
    .join(" • ")
}

fn detail_progress_label(item: &DetailItemData) -> String {
    if item.watched || item.resume_seconds <= 0.0 {
        String::new()
    } else {
        format!("Resume at {}", format_playback_time(item.resume_seconds))
    }
}

fn detail_series_summary(page: &DetailPageData) -> String {
    if page.item.kind != DetailKind::Series {
        return String::new();
    }
    if page.seasons.is_empty() && page.episodes.is_empty() {
        return if page.children_error.is_some() {
            "Series contents unavailable".to_owned()
        } else {
            "No seasons available".to_owned()
        };
    }
    let mut parts = Vec::new();
    if !page.seasons.is_empty() {
        parts.push(if page.seasons.len() == 1 {
            "1 season".to_owned()
        } else {
            format!("{} seasons", page.seasons.len())
        });
    }
    if !page.episodes.is_empty() {
        parts.push(if page.episodes.len() == 1 {
            "1 episode".to_owned()
        } else {
            format!("{} episodes", page.episodes.len())
        });
        let unwatched = page
            .episodes
            .iter()
            .filter(|episode| !episode.watched)
            .count();
        parts.push(if unwatched == 0 {
            "All watched".to_owned()
        } else {
            format!("{unwatched} unwatched")
        });
    }
    parts.join(" • ")
}

fn detail_series_play_context(page: &DetailPageData) -> String {
    if page.item.kind != DetailKind::Series {
        return String::new();
    }
    let Some(target) = page.play_target() else {
        return String::new();
    };
    let episode = detail_episode_number(target).unwrap_or_else(|| target.title.clone());
    if target.resume_seconds > 0.0 && !target.watched {
        format!(
            "{episode} • Resume at {}",
            format_playback_time(target.resume_seconds)
        )
    } else if !target.watched {
        format!("{episode} • Up next")
    } else {
        format!("{episode} • Start again")
    }
}

fn detail_season_summary(page: &DetailPageData) -> String {
    if page.item.kind != DetailKind::Season {
        return String::new();
    }
    let episode_count = page.episodes.len();
    if episode_count == 0 {
        return if page.children_error.is_some() {
            "Episodes unavailable".to_owned()
        } else {
            "No episodes available".to_owned()
        };
    }
    let unwatched_count = page
        .episodes
        .iter()
        .filter(|episode| !episode.watched)
        .count();
    let episode_label = if episode_count == 1 {
        "1 episode".to_owned()
    } else {
        format!("{episode_count} episodes")
    };
    if unwatched_count == 0 {
        format!("{episode_label} • All watched")
    } else {
        format!("{episode_label} • {unwatched_count} unwatched")
    }
}

#[derive(Clone)]
enum DetailArtworkTarget {
    HeroPoster(String),
    HeroBackdrop(String),
    HeroLogo(String),
    Season {
        index: usize,
        id: String,
    },
    Episode {
        index: usize,
        id: String,
    },
    Person {
        index: usize,
        id: String,
        name: String,
    },
}

fn load_detail_artwork(context: &AppContext, generation: u64, page: &DetailPageData) {
    let item_id = page.item.id.clone();
    let mut requests = Vec::new();
    let hero_poster = if page.item.kind == DetailKind::Episode {
        page.item
            .thumbnail
            .clone()
            .or_else(|| page.item.poster.clone())
    } else {
        page.item.poster.clone()
    };
    if let Some(image) = hero_poster {
        requests.push((DetailArtworkTarget::HeroPoster(item_id.clone()), image));
    }
    if let Some(image) = page.item.backdrop.clone() {
        requests.push((DetailArtworkTarget::HeroBackdrop(item_id.clone()), image));
    }
    if let Some(image) = page.item.logo.clone() {
        requests.push((DetailArtworkTarget::HeroLogo(item_id), image));
    }
    requests.extend(page.seasons.iter().enumerate().filter_map(|(index, item)| {
        item.poster.clone().map(|image| {
            (
                DetailArtworkTarget::Season {
                    index,
                    id: item.id.clone(),
                },
                image,
            )
        })
    }));
    requests.extend(
        page.episodes
            .iter()
            .enumerate()
            .filter_map(|(index, item)| {
                item.thumbnail
                    .clone()
                    .or_else(|| item.poster.clone())
                    .map(|image| {
                        (
                            DetailArtworkTarget::Episode {
                                index,
                                id: item.id.clone(),
                            },
                            image,
                        )
                    })
            }),
    );
    requests.extend(
        page.item
            .people
            .iter()
            .enumerate()
            .filter_map(|(index, person)| {
                person.image.clone().map(|image| {
                    (
                        DetailArtworkTarget::Person {
                            index,
                            id: person.id.clone().unwrap_or_default(),
                            name: person.name.clone(),
                        },
                        image,
                    )
                })
            }),
    );
    for (target, image) in requests {
        load_detail_image(context, generation, target, image);
    }
}

fn load_detail_image(
    context: &AppContext,
    generation: u64,
    target: DetailArtworkTarget,
    image: ImageRef,
) {
    let Some(api) = context.api() else { return };
    let ctx = context.clone();
    context.runtime.spawn(async move {
        let Ok(_permit) = ctx.image_limit.clone().acquire_owned().await else {
            return;
        };
        let cache_id = format!(
            "{}-{}-{}",
            image.item_id,
            image.image_type.jellyfin_name(),
            image.index.unwrap_or(0)
        );
        let path = image_cache_path(
            &ctx.image_cache,
            api.session().server_url.as_str(),
            &cache_id,
            &image.tag,
        );
        let result = if let Some(path) = existing_cached_image(&path) {
            Ok(path)
        } else {
            let max_width = match &target {
                DetailArtworkTarget::HeroBackdrop(_) => 1600,
                DetailArtworkTarget::Episode { .. } | DetailArtworkTarget::HeroPoster(_)
                    if image.image_type == detail::ImageType::Thumb =>
                {
                    900
                }
                DetailArtworkTarget::Person { .. } => 320,
                _ => 600,
            };
            match api.fetch_detail_image(&image, max_width).await {
                Ok(bytes) if bytes.len() <= 20 * 1024 * 1024 => write_cached_image(&path, &bytes),
                Ok(_) => Err("detail artwork exceeded 20 MiB".to_owned()),
                Err(error) => Err(error.to_string()),
            }
        };
        if !ctx.is_current(generation) {
            return;
        }
        if let Ok(path) = result {
            let weak = ctx.window.clone();
            let _ = slint::invoke_from_event_loop(move || {
                let Some(window) = weak.upgrade() else { return };
                let Ok(image) = Image::load_from_path(&path) else {
                    return;
                };
                match target {
                    DetailArtworkTarget::HeroPoster(id)
                        if window.get_detail_id().as_str() == id =>
                    {
                        window.set_detail_artwork(image);
                    }
                    DetailArtworkTarget::HeroBackdrop(id)
                        if window.get_detail_id().as_str() == id =>
                    {
                        window.set_detail_backdrop(image);
                    }
                    DetailArtworkTarget::HeroLogo(id) if window.get_detail_id().as_str() == id => {
                        window.set_detail_logo(image);
                    }
                    DetailArtworkTarget::Season { index, id } => {
                        let model = window.get_detail_seasons();
                        if let Some(mut row) = model.row_data(index)
                            && row.id.as_str() == id
                        {
                            row.artwork = image;
                            model.set_row_data(index, row);
                        }
                    }
                    DetailArtworkTarget::Episode { index, id } => {
                        let model = window.get_detail_episodes();
                        if let Some(mut row) = model.row_data(index)
                            && row.id.as_str() == id
                        {
                            row.artwork = image;
                            model.set_row_data(index, row);
                        }
                    }
                    DetailArtworkTarget::Person { index, id, name } => {
                        let model = window.get_detail_people();
                        if let Some(mut row) = model.row_data(index)
                            && row.id.as_str() == id
                            && row.name.as_str() == name
                        {
                            row.artwork = image;
                            model.set_row_data(index, row);
                        }
                    }
                    _ => {}
                }
            });
        }
    });
}

fn load_artwork(context: &AppContext, generation: u64, items: Vec<CatalogItemData>) {
    let Some(api) = context.api() else { return };
    for (index, item) in items.into_iter().enumerate() {
        let Some(tag) = item.image_tag.clone() else {
            continue;
        };
        let ctx = context.clone();
        let api = api.clone();
        context.runtime.spawn(async move {
            let Ok(_permit) = ctx.image_limit.clone().acquire_owned().await else {
                return;
            };
            let path = image_cache_path(
                &ctx.image_cache,
                api.session().server_url.as_str(),
                &item.image_item_id,
                &tag,
            );
            let result = if let Some(path) = existing_cached_image(&path) {
                Ok(path)
            } else {
                match api.fetch_primary_image(&item.image_item_id).await {
                    Ok(bytes) if bytes.len() <= 20 * 1024 * 1024 => {
                        write_cached_image(&path, &bytes)
                    }
                    Ok(_) => Err("artwork exceeded 20 MiB".to_owned()),
                    Err(error) => Err(error.to_string()),
                }
            };
            if !ctx.is_current(generation) {
                return;
            }
            if let Ok(path) = result {
                let weak = ctx.window.clone();
                let item_id = item.id;
                let _ = slint::invoke_from_event_loop(move || {
                    let Some(window) = weak.upgrade() else { return };
                    let Ok(image) = Image::load_from_path(&path) else {
                        return;
                    };
                    let model = window.get_catalog_items();
                    if let Some(mut row) = model.row_data(index)
                        && row.id.as_str() == item_id
                    {
                        row.artwork = image;
                        model.set_row_data(index, row);
                    }
                });
            }
        });
    }
}

fn image_cache_path(directory: &Path, server: &str, item_id: &str, tag: &str) -> PathBuf {
    let mut hasher = DefaultHasher::new();
    server.hash(&mut hasher);
    item_id.hash(&mut hasher);
    tag.hash(&mut hasher);
    directory.join(format!("{:016x}.jpg", hasher.finish()))
}

fn existing_cached_image(path: &Path) -> Option<PathBuf> {
    for extension in ["jpg", "png", "webp", "gif"] {
        let candidate = path.with_extension(extension);
        if !candidate.is_file() {
            continue;
        }
        let mut header = [0_u8; 12];
        let read = File::open(&candidate)
            .and_then(|mut file| file.read(&mut header))
            .ok()?;
        let corrected = path.with_extension(image_extension(&header[..read]));
        if corrected == candidate {
            return Some(candidate);
        }
        if corrected.is_file() || fs::rename(&candidate, &corrected).is_ok() {
            return Some(corrected);
        }
    }
    None
}

fn image_extension(data: &[u8]) -> &'static str {
    if data.starts_with(b"\x89PNG\r\n\x1a\n") {
        "png"
    } else if data.starts_with(b"GIF87a") || data.starts_with(b"GIF89a") {
        "gif"
    } else if data.len() >= 12 && data.starts_with(b"RIFF") && &data[8..12] == b"WEBP" {
        "webp"
    } else {
        "jpg"
    }
}

fn write_cached_image(path: &Path, data: &[u8]) -> Result<PathBuf, String> {
    let path = path.with_extension(image_extension(data));
    let parent = path.parent().ok_or("image cache has no parent")?;
    fs::create_dir_all(parent).map_err(|error| error.to_string())?;
    let temporary = parent.join(format!(
        ".image-{}-{}.tmp",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_nanos()
    ));
    let result = (|| -> Result<(), String> {
        let mut options = OpenOptions::new();
        options.write(true).create_new(true);
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt;
            options.mode(0o600);
        }
        let mut file = options
            .open(&temporary)
            .map_err(|error| error.to_string())?;
        file.write_all(data).map_err(|error| error.to_string())?;
        file.sync_all().map_err(|error| error.to_string())?;
        fs::rename(&temporary, &path).map_err(|error| error.to_string())?;
        File::open(parent)
            .and_then(|directory| directory.sync_all())
            .map_err(|error| error.to_string())?;
        Ok(())
    })();
    if result.is_err() {
        let _ = fs::remove_file(&temporary);
    }
    result.map(|()| path)
}

fn to_ui_item(item: CatalogItemData) -> CatalogItem {
    let placeholder = item
        .title
        .split_whitespace()
        .filter_map(|word| word.chars().next())
        .take(2)
        .flat_map(char::to_uppercase)
        .collect::<String>();
    CatalogItem {
        id: item.id.into(),
        title: item.title.into(),
        subtitle: item.subtitle.into(),
        kind_label: catalog_kind_label(&item.kind).into(),
        kind: item.kind.into(),
        placeholder: placeholder.into(),
        artwork: Image::default(),
        watched: item.watched,
        progress: item.progress,
        is_container: item.is_container,
        playable: item.playable,
    }
}

fn catalog_kind_label(kind: &str) -> &str {
    match kind {
        "CollectionFolder" | "UserView" => "Library",
        "ManualPlaylistsFolder" => "Playlists",
        "BoxSet" => "Collection",
        other => other,
    }
}

fn set_screen_busy(context: &AppContext, screen: &str, busy: bool, message: &str) {
    let weak = context.window.clone();
    let screen: SharedString = screen.into();
    let message: SharedString = message.into();
    let _ = slint::invoke_from_event_loop(move || {
        if let Some(window) = weak.upgrade() {
            window.set_screen(screen);
            window.set_busy(busy);
            window.set_message(message);
        }
    });
}

fn set_message(context: &AppContext, message: &str) {
    let weak = context.window.clone();
    let message: SharedString = message.into();
    let _ = slint::invoke_from_event_loop(move || {
        if let Some(window) = weak.upgrade() {
            window.set_message(message);
        }
    });
}

fn show_login(context: &AppContext, message: &str) {
    set_screen_busy(context, "login", false, message);
}

fn show_catalog_error(context: &AppContext, message: String) {
    let weak = context.window.clone();
    let _ = slint::invoke_from_event_loop(move || {
        if let Some(window) = weak.upgrade() {
            window.set_screen("catalog".into());
            window.set_busy(false);
            window.set_message(message.into());
            window.set_catalog_items(ModelRc::new(VecModel::<CatalogItem>::default()));
        }
    });
}

#[cfg(test)]
mod tests {
    use std::cell::Cell;
    use std::rc::Rc;

    use slint::platform::software_renderer::{MinimalSoftwareWindow, RepaintBufferType};
    use slint::platform::{
        Platform, PlatformError, PointerEventButton, WindowAdapter, WindowEvent,
    };
    use slint::{ComponentHandle, LogicalPosition, PhysicalSize};

    use super::{
        EmptySearchDestination, JellyBridgeWindow, PageSnapshot, StoredPlaybackPreference,
        StoredSettings, detail::DetailItemData, detail::DetailKind, detail::DetailPageData,
        detail::StreamData, detail_metadata_line, detail_progress_label, detail_season_summary,
        detail_series_play_context, detail_series_summary, empty_search_destination,
        image_extension, playback_preference_key, resolved_playback_sources,
    };

    thread_local! {
        static TEST_WINDOW: Rc<MinimalSoftwareWindow> =
            MinimalSoftwareWindow::new(RepaintBufferType::ReusedBuffer);
    }

    struct TestPlatform;

    impl Platform for TestPlatform {
        fn create_window_adapter(&self) -> Result<Rc<dyn WindowAdapter>, PlatformError> {
            Ok(TEST_WINDOW.with(Clone::clone))
        }
    }

    fn setup_test_window() -> Rc<MinimalSoftwareWindow> {
        let _ = slint::platform::set_platform(Box::new(TestPlatform));
        let window = TEST_WINDOW.with(Clone::clone);
        window.set_size(PhysicalSize::new(1280, 800));
        window
    }

    #[test]
    fn player_entry_restores_hotkeys_after_detail_focus_is_removed() {
        let platform_window = setup_test_window();
        let ui = JellyBridgeWindow::new().expect("create player UI");
        ui.set_screen("detail".into());
        ui.show().expect("show player UI");
        platform_window.draw_if_needed(|_| {});

        // Move focus from the root scope into a detail-page control, matching a user
        // activating Play before that control disappears during the screen change.
        let tab = char::from(slint::platform::Key::Tab).to_string().into();
        ui.window()
            .dispatch_event(WindowEvent::KeyPressed { text: tab });

        let hotkey_count = Rc::new(Cell::new(0));
        let observed_hotkeys = hotkey_count.clone();
        ui.on_player_key_pressed(move |_, _, _, _| {
            observed_hotkeys.set(observed_hotkeys.get() + 1);
            true
        });
        ui.invoke_enter_player(0.0, 100.0);
        platform_window.draw_if_needed(|_| {});
        let player_keys = [
            " ".to_owned(),
            char::from(slint::platform::Key::LeftArrow).to_string(),
            char::from(slint::platform::Key::RightArrow).to_string(),
            char::from(slint::platform::Key::UpArrow).to_string(),
            char::from(slint::platform::Key::DownArrow).to_string(),
            char::from(slint::platform::Key::Home).to_string(),
            char::from(slint::platform::Key::End).to_string(),
            "m".to_owned(),
            "f".to_owned(),
            "a".to_owned(),
            "j".to_owned(),
        ];
        for text in &player_keys {
            ui.window().dispatch_event(WindowEvent::KeyPressed {
                text: text.as_str().into(),
            });
        }
        assert_eq!(
            hotkey_count.get(),
            player_keys.len(),
            "player entry must restore keyboard focus for every player shortcut"
        );
    }

    #[test]
    fn timeline_keeps_following_player_events_after_scrubbing() {
        let platform_window = setup_test_window();
        let ui = JellyBridgeWindow::new().expect("create player UI");
        ui.invoke_enter_player(10.0, 100.0);
        ui.show().expect("show player UI");
        platform_window.draw_if_needed(|_| {});

        let seek_value = Rc::new(Cell::new(None));
        let observed_seek = seek_value.clone();
        ui.on_seek_absolute(move |value| observed_seek.set(Some(value)));
        let timeline_point = LogicalPosition::new(640.0, 666.0);
        ui.window().dispatch_event(WindowEvent::PointerPressed {
            position: timeline_point,
            button: PointerEventButton::Left,
        });
        ui.window().dispatch_event(WindowEvent::PointerReleased {
            position: timeline_point,
            button: PointerEventButton::Left,
        });
        let sought = seek_value.get().expect("timeline release must seek");
        assert!(
            (40.0..=60.0).contains(&sought),
            "unexpected seek value: {sought}"
        );

        ui.set_playback_position(25.0);
        slint::platform::update_timers_and_animations();
        platform_window.draw_if_needed(|_| {});
        let displayed_position = ui.get_timeline_display_position();
        assert!(
            (displayed_position - 25.0).abs() < 0.5,
            "timeline must resume following mpv position events after scrubbing; displayed {displayed_position}"
        );

        ui.set_playback_position(30.0);
        slint::platform::update_timers_and_animations();
        platform_window.draw_if_needed(|_| {});
        assert!(
            (ui.get_timeline_display_position() - 30.0).abs() < 0.5,
            "programmatic timeline updates must not re-enter scrub mode"
        );

        ui.invoke_enter_player(5.0, 200.0);
        assert_eq!(ui.get_playback_position(), 5.0);
        assert_eq!(ui.get_playback_duration(), 200.0);
        assert_eq!(ui.get_timeline_display_position(), 5.0);
    }

    #[test]
    fn single_video_click_toggles_playback_after_double_click_window() {
        let platform_window = setup_test_window();
        let ui = JellyBridgeWindow::new().expect("create player UI");
        ui.invoke_enter_player(10.0, 100.0);
        ui.show().expect("show player UI");
        platform_window.draw_if_needed(|_| {});

        let toggle_count = Rc::new(Cell::new(0));
        let observed_toggles = toggle_count.clone();
        ui.on_toggle_playback(move || observed_toggles.set(observed_toggles.get() + 1));
        let activity_count = Rc::new(Cell::new(0));
        let observed_activity = activity_count.clone();
        ui.on_player_activity(move || observed_activity.set(observed_activity.get() + 1));

        let video_center = LogicalPosition::new(640.0, 400.0);
        ui.window().dispatch_event(WindowEvent::PointerPressed {
            position: video_center,
            button: PointerEventButton::Left,
        });
        ui.window().dispatch_event(WindowEvent::PointerReleased {
            position: video_center,
            button: PointerEventButton::Left,
        });
        assert_eq!(
            activity_count.get(),
            1,
            "video surface must receive the click"
        );
        assert_eq!(
            toggle_count.get(),
            0,
            "single click must wait for a possible double click"
        );

        std::thread::sleep(std::time::Duration::from_millis(250));
        slint::platform::update_timers_and_animations();
        assert_eq!(
            toggle_count.get(),
            1,
            "single video click must toggle playback exactly once"
        );
    }

    #[test]
    fn double_video_click_cancels_pause_and_requests_fullscreen() {
        let platform_window = setup_test_window();
        let ui = JellyBridgeWindow::new().expect("create player UI");
        ui.invoke_enter_player(10.0, 100.0);
        ui.show().expect("show player UI");
        platform_window.draw_if_needed(|_| {});

        let toggle_count = Rc::new(Cell::new(0));
        let observed_toggles = toggle_count.clone();
        ui.on_toggle_playback(move || observed_toggles.set(observed_toggles.get() + 1));
        let fullscreen_count = Rc::new(Cell::new(0));
        let observed_fullscreen = fullscreen_count.clone();
        ui.on_toggle_fullscreen(move || observed_fullscreen.set(observed_fullscreen.get() + 1));

        let video_center = LogicalPosition::new(640.0, 400.0);
        for _ in 0..2 {
            ui.window().dispatch_event(WindowEvent::PointerPressed {
                position: video_center,
                button: PointerEventButton::Left,
            });
            ui.window().dispatch_event(WindowEvent::PointerReleased {
                position: video_center,
                button: PointerEventButton::Left,
            });
        }

        std::thread::sleep(std::time::Duration::from_millis(250));
        slint::platform::update_timers_and_animations();
        assert_eq!(
            fullscreen_count.get(),
            1,
            "double click must request fullscreen once"
        );
        assert_eq!(
            toggle_count.get(),
            0,
            "double click must not also toggle playback"
        );
    }

    #[test]
    fn custom_progress_fills_are_anchored_to_the_left_edge() {
        let source = include_str!("../ui/app.slint");
        let mut progress_fill_count = 0;

        for (width_offset, _) in source.match_indices("width: parent.width *") {
            let width_line = &source[width_offset..];
            let line = &width_line[..width_line.find('\n').unwrap_or(width_line.len())];
            if !line.contains(".progress") {
                continue;
            }

            progress_fill_count += 1;
            let source_before_width = &source[..width_offset];
            let fill_start = source_before_width
                .rfind("Rectangle {")
                .expect("a progress width must belong to a Rectangle");
            let fill_declaration = &source_before_width[fill_start..];
            assert!(
                fill_declaration.contains("x: 0px;"),
                "custom progress fill must be explicitly anchored to the left: {line}"
            );
        }

        assert_eq!(progress_fill_count, 4, "update this contract for new fills");
    }

    #[test]
    fn programmatic_empty_search_does_not_interrupt_initial_home_load() {
        let initial_home = PageSnapshot::empty_home();

        assert!(matches!(
            empty_search_destination(&initial_home),
            EmptySearchDestination::Ignore
        ));
    }

    #[test]
    fn clearing_an_active_search_returns_to_home() {
        let mut search_results = PageSnapshot::empty_home();
        search_results.search_text = "movie".into();

        assert!(matches!(
            empty_search_destination(&search_results),
            EmptySearchDestination::Home
        ));
    }

    #[test]
    fn episode_preferences_are_scoped_to_the_series() {
        let mut episode = DetailItemData::fixture("episode-1", DetailKind::Episode);
        episode.series_id = Some("series-1".into());

        assert_eq!(playback_preference_key(&episode), "series:series-1");
    }

    #[test]
    fn stored_language_preferences_resolve_to_jellyfin_source_indexes() {
        let mut episode = DetailItemData::fixture("episode-1", DetailKind::Episode);
        episode.series_id = Some("series-1".into());
        episode.streams = vec![
            StreamData {
                index: 4,
                kind: "Audio".into(),
                label: "ENG".into(),
                language: Some("eng".into()),
                title: Some("Stereo".into()),
                codec: Some("aac".into()),
                is_default: true,
            },
            StreamData {
                index: 9,
                kind: "Audio".into(),
                label: "JPN".into(),
                language: Some("jpn".into()),
                title: Some("5.1".into()),
                codec: Some("aac".into()),
                is_default: false,
            },
            StreamData {
                index: 12,
                kind: "Subtitle".into(),
                label: "ENG Signs".into(),
                language: Some("eng".into()),
                title: Some("Signs".into()),
                codec: Some("srt".into()),
                is_default: false,
            },
        ];
        let mut settings = StoredSettings::default();
        settings.playback_preferences.insert(
            "series:series-1".into(),
            StoredPlaybackPreference {
                audio_language: Some("JPN".into()),
                audio_title: None,
                subtitle_language: Some("eng".into()),
                subtitle_title: Some("Signs".into()),
                subtitles_disabled: false,
            },
        );

        assert_eq!(
            resolved_playback_sources(&episode, &settings),
            (Some(9), Some(12))
        );
    }

    #[test]
    fn stored_subtitle_off_choice_is_applied_explicitly() {
        let mut movie = DetailItemData::fixture("movie-1", DetailKind::Movie);
        movie.streams.push(StreamData {
            index: 5,
            kind: "Subtitle".into(),
            label: "English".into(),
            language: Some("eng".into()),
            title: None,
            codec: Some("srt".into()),
            is_default: true,
        });
        let mut settings = StoredSettings::default();
        settings.playback_preferences.insert(
            "movie:movie-1".into(),
            StoredPlaybackPreference {
                subtitles_disabled: true,
                ..StoredPlaybackPreference::default()
            },
        );

        assert_eq!(resolved_playback_sources(&movie, &settings).1, Some(-1));
    }

    #[test]
    fn cached_images_keep_their_encoded_format() {
        assert_eq!(image_extension(b"\x89PNG\r\n\x1a\nrest"), "png");
        assert_eq!(image_extension(b"\xff\xd8\xffrest"), "jpg");
        assert_eq!(image_extension(b"RIFF1234WEBPrest"), "webp");
    }

    #[test]
    fn season_summary_reports_episode_and_unwatched_counts() {
        let mut watched = DetailItemData::fixture("watched", DetailKind::Episode);
        watched.watched = true;
        let unwatched = DetailItemData::fixture("unwatched", DetailKind::Episode);
        let page = DetailPageData {
            item: DetailItemData::fixture("season", DetailKind::Season),
            seasons: Vec::new(),
            episodes: vec![watched, unwatched],
            children_error: None,
        };

        assert_eq!(detail_season_summary(&page), "2 episodes • 1 unwatched");
    }

    #[test]
    fn season_summary_distinguishes_empty_from_unavailable() {
        let mut page = DetailPageData {
            item: DetailItemData::fixture("season", DetailKind::Season),
            seasons: Vec::new(),
            episodes: Vec::new(),
            children_error: None,
        };
        assert_eq!(detail_season_summary(&page), "No episodes available");

        page.children_error = Some("offline".into());
        assert_eq!(detail_season_summary(&page), "Episodes unavailable");
    }

    #[test]
    fn episode_metadata_uses_number_without_repeating_its_parents() {
        let mut episode = DetailItemData::fixture("episode", DetailKind::Episode);
        episode.series_name = Some("Example Show".into());
        episode.season_name = Some("Season 2".into());
        episode.parent_index_number = Some(2);
        episode.index_number = Some(3);
        episode.production_year = Some(2026);
        episode.runtime_seconds = Some(2_700.0);

        let metadata = detail_metadata_line(&episode);
        assert_eq!(metadata, "S02E03 • 2026 • 45m");
        assert!(!metadata.contains("Example Show"));
        assert!(!metadata.contains("Season 2"));
    }

    #[test]
    fn detail_progress_label_formats_resume_position() {
        let mut movie = DetailItemData::fixture("movie", DetailKind::Movie);
        movie.resume_seconds = 3_661.0;
        assert_eq!(detail_progress_label(&movie), "Resume at 1:01:01");

        movie.watched = true;
        assert_eq!(detail_progress_label(&movie), "");
    }

    #[test]
    fn series_summary_reports_loaded_seasons_and_episode_state() {
        let mut watched = DetailItemData::fixture("watched", DetailKind::Episode);
        watched.watched = true;
        let unwatched = DetailItemData::fixture("unwatched", DetailKind::Episode);
        let page = DetailPageData {
            item: DetailItemData::fixture("series", DetailKind::Series),
            seasons: vec![
                DetailItemData::fixture("season-1", DetailKind::Season),
                DetailItemData::fixture("season-2", DetailKind::Season),
            ],
            episodes: vec![watched, unwatched],
            children_error: None,
        };

        assert_eq!(
            detail_series_summary(&page),
            "2 seasons • 2 episodes • 1 unwatched"
        );
    }

    #[test]
    fn series_play_context_describes_the_selected_episode() {
        let mut episode = DetailItemData::fixture("episode", DetailKind::Episode);
        episode.parent_index_number = Some(1);
        episode.index_number = Some(4);
        episode.resume_seconds = 1_227.0;
        let mut page = DetailPageData {
            item: DetailItemData::fixture("series", DetailKind::Series),
            seasons: Vec::new(),
            episodes: vec![episode],
            children_error: None,
        };

        assert_eq!(
            detail_series_play_context(&page),
            "S01E04 • Resume at 20:27"
        );

        page.episodes[0].resume_seconds = 0.0;
        assert_eq!(detail_series_play_context(&page), "S01E04 • Up next");

        page.episodes[0].watched = true;
        assert_eq!(detail_series_play_context(&page), "S01E04 • Start again");
    }
}
