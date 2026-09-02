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
use std::time::Instant;

use app::{AppContext, AppState, PlayerTrackState};
use features::catalog::{CatalogDensity, CatalogSort};
use features::progress::PlaybackReporter;
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
    // First statement: time_to_interactive is measured from here, so it
    // includes runtime, backend, and window construction.
    let process_start = Instant::now();
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                // The crate is jellybridge-app but the [[bin]] is jellybridge,
                // and tracing targets are module paths rooted at the binary's
                // name -- so a jellybridge_app filter matched nothing and the
                // app's own logging never reached the terminal.
                .unwrap_or_else(|_| "jellybridge=info".into()),
        )
        .without_time()
        .init();
    ui::startup::mark_process_start(process_start);

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

    // One worker per logical CPU is tokio's default: 32 OS threads here, for a
    // workload whose real concurrency is six dashboard requests, six artwork
    // permits and eight proxy streams, all of it I/O-bound. Four workers cover
    // that; the blocking pool, which the artwork disk probes and getaddrinfo
    // use, is capped well below its 512-thread default for the same reason.
    let runtime = tokio::runtime::Builder::new_multi_thread()
        .worker_threads(4)
        .max_blocking_threads(8)
        .enable_all()
        .build()?;
    // Before anything else touches the main thread. The saved session's
    // dashboard is the slowest thing in startup and it does not depend on the
    // window, so it runs while the window, the player and the renderer are
    // being built rather than after them. See auth::prefetch_home.
    let prefetched_home = features::auth::prefetch_home(runtime.handle());
    tracing::debug!(ms = process_start.elapsed().as_secs_f64() * 1000.0, stage = "prefetch_spawned", "startup_stage");
    let store = SessionStore::for_current_user()?;
    let settings_store = SettingsStore::for_current_user()?;
    let stored_settings = settings_store.load().unwrap_or_else(|error| {
        tracing::warn!(%error, "catalog settings could not be loaded");
        StoredSettings::default()
    });
    let app_state = AppState {
        catalog_sort: CatalogSort::from_key(stored_settings.library_sort_key.as_deref()),
        catalog_density: CatalogDensity::from_card_width(stored_settings.card_width_dp),
        ..AppState::default()
    };
    let image_cache = cache_directory()?;
    // Nothing ever removed an artwork entry, so the cache grew for the life of
    // the installation. Pruning is disk I/O over a directory of thousands of
    // files, so it runs on the blocking pool and never blocks the first frame.
    {
        let directory = image_cache.clone();
        runtime.spawn_blocking(move || {
            services::artwork::prune_image_cache(&directory, services::artwork::IMAGE_CACHE_BUDGET)
        });
    }
    let window = JellyBridgeWindow::new()?;
    tracing::debug!(ms = process_start.elapsed().as_secs_f64() * 1000.0, stage = "window_built", "startup_stage");
    let ui_handle = UiHandle::new(&window);
    tracing::debug!(ms = process_start.elapsed().as_secs_f64() * 1000.0, stage = "ui_handle", "startup_stage");
    let playback = Arc::new(Mutex::new(None::<PreparedPlayback>));
    let player_tracks = Arc::new(Mutex::new(PlayerTrackState::default()));
    let state = Arc::new(Mutex::new(app_state));
    let reporter = Arc::new(Mutex::new(PlaybackReporter::default()));
    let event_playback = playback.clone();
    let event_tracks = player_tracks.clone();
    let event_runtime = runtime.handle().clone();
    let event_ui = ui_handle.clone();
    let event_state = state.clone();
    let event_reporter = reporter.clone();
    let (render_client, player_worker) = create_player(move |event| {
        let ended = matches!(event, PlayerEvent::Ended);
        // Turn mpv's position and pause events into Jellyfin reports. The
        // reporter throttles them, so this only issues a request when the
        // server actually needs one.
        let due = match &event {
            PlayerEvent::Position { seconds, .. } => event_reporter
                .lock()
                .ok()
                .and_then(|mut reporter| reporter.note_position(*seconds, Instant::now())),
            PlayerEvent::Paused(paused) => event_reporter
                .lock()
                .ok()
                .and_then(|mut reporter| reporter.note_paused(*paused, Instant::now())),
            _ => None,
        };
        if let Some(report) = due {
            features::playback::send_progress_report(&event_state, &event_runtime, report);
        }
        let tracks = event_tracks.clone();
        event_ui.update(move |window| ui::player::apply_player_event(window, event, &tracks));
        if ended {
            // Commit the final position before the proxy goes away; Jellyfin
            // decides from this whether the item counts as watched.
            features::playback::finish_report(&event_reporter, &event_state, &event_runtime);
            if let Ok(mut slot) = event_playback.lock()
                && let Some(remote) = slot.take()
            {
                event_runtime.spawn(remote.shutdown());
            }
        }
    })?;
    tracing::debug!(ms = process_start.elapsed().as_secs_f64() * 1000.0, stage = "player_created", "startup_stage");
    let player_sender = player_worker.command_sender();
    let player_worker = Rc::new(RefCell::new(Some(player_worker)));
    let image_is_cleared = Rc::new(Cell::new(true));
    let context = AppContext {
        ui: ui_handle,
        runtime: runtime.handle().clone(),
        state,
        generation: Arc::new(AtomicU64::new(0)),
        store,
        settings_store,
        image_cache,
        image_limit: Arc::new(tokio::sync::Semaphore::new(6)),
        playback,
        reporter,
        player_sender,
        player_tracks,
        catalog_request: Arc::new(Mutex::new(None)),
    };
    ui::connect(&window, &context);
    let player_ui = ui::player::connect(&window, &context, image_is_cleared.clone());
    ui::player::install_renderer(&window, render_client, image_is_cleared.clone(), player_ui)?;
    tracing::debug!(ms = process_start.elapsed().as_secs_f64() * 1000.0, stage = "renderer_installed", "startup_stage");
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
    features::auth::restore_session(&context, prefetched_home);

    tracing::info!(
        renderer = RENDERER_NAME,
        "Rust JellyBridge application ready"
    );
    tracing::debug!(ms = process_start.elapsed().as_secs_f64() * 1000.0, stage = "event_loop_start", "startup_stage");
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
