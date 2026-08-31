mod api;
mod app;
mod features;
mod services;
mod ui;

use std::cell::{Cell, RefCell};
use slint::ComponentHandle;
use std::rc::Rc;
use std::sync::atomic::AtomicU64;
use std::sync::{Arc, Mutex};

use app::{AppContext, AppState, PlayerTrackState};
use features::catalog::{CatalogDensity, CatalogSort};
use services::player::{PlayerEvent, create_player};
use services::playback::PreparedPlayback;
use services::session::{SessionStore, SettingsStore, StoredSettings, cache_directory};
use ui::{JellyBridgeWindow, UiHandle};

#[cfg(all(feature = "renderer-femtovg", feature = "renderer-skia-opengl"))]
compile_error!("select exactly one renderer feature");

#[cfg(not(any(feature = "renderer-femtovg", feature = "renderer-skia-opengl")))]
compile_error!("select renderer-femtovg or renderer-skia-opengl");

#[cfg(feature = "renderer-femtovg")]
const RENDERER_NAME: &str = "femtovg";

#[cfg(feature = "renderer-skia-opengl")]
const RENDERER_NAME: &str = "skia";

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
            StoredSettings::default()
        }
    };
    let app_state = AppState {
        catalog_sort: CatalogSort::from_key(stored_settings.library_sort_key.as_deref()),
        catalog_density: CatalogDensity::from_card_width(stored_settings.card_width_dp),
        ..AppState::default()
    };
    let image_cache = cache_directory()?;
    let window = JellyBridgeWindow::new()?;
    let ui_handle = UiHandle::new(&window);
    let playback = Arc::new(Mutex::new(None::<PreparedPlayback>));
    let player_tracks = Arc::new(Mutex::new(PlayerTrackState::default()));
    let event_playback = playback.clone();
    let event_tracks = player_tracks.clone();
    let event_runtime = runtime.handle().clone();
    let event_ui = ui_handle.clone();
    let (render_client, player_worker) = create_player(move |event| {
        let ended = matches!(event, PlayerEvent::Ended);
        let tracks = event_tracks.clone();
        event_ui.update(move |window| ui::player::apply_player_event(window, event, &tracks));
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
        ui: ui_handle,
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
    ui::connect(&window, &context);
    let player_ui = ui::player::connect(&window, &context, image_is_cleared.clone());
    ui::player::install_renderer(&window, render_client, image_is_cleared.clone(), player_ui)?;
    let close_worker = player_worker.clone();
    let close_weak = slint::ComponentHandle::as_weak(&window);
    let close_image_is_cleared = image_is_cleared.clone();
    window.window().on_close_requested(move || {
        if let Some(window) = close_weak.upgrade() {
            ui::player::shutdown_player_surface(&close_worker, &window, &close_image_is_cleared);
        } else if let Some(worker) = close_worker.borrow_mut().take() {
            worker.shutdown();
        }
        slint::CloseRequestResponse::HideWindow
    });
    features::auth::restore_session(&context);

    tracing::info!(
        renderer = RENDERER_NAME,
        "Rust JellyBridge application ready"
    );
    window.run()?;

    // Teardown order matters: invalidate in-flight work, stop the mpv worker,
    // clear the video frame while `image_is_cleared` is set (the renderer
    // teardown notifier reads it), drop the window, then shut down the proxy
    // and the runtime.
    context.next_generation();
    ui::player::shutdown_player_surface(&player_worker, &window, &image_is_cleared);
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
