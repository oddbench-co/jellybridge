mod api;
mod app;
mod features;
mod services;
mod ui;

use std::sync::atomic::AtomicU64;
use std::sync::{Arc, Mutex};
use std::time::Instant;

use app::{AppContext, AppState, PlayerTrackState};
use features::progress::PlaybackReporter;
use services::playback::PreparedPlayback;
use services::player::{PlayerEvent, create_player};
use services::render::{NativeDisplay, RenderHost, native_display};
use services::session::{SessionStore, SettingsStore, StoredSettings, cache_directory};
use ui::UiHandle;
use ui::app::{JellyBridgeApp, UiAction};

/// Identifies our windows to the window manager. Matches the packager
/// identifier in `Cargo.toml` so the desktop entry, the window class and the
/// installed package all agree. Without it the Wayland surface carries no app
/// id at all, and anything that identifies windows by class -- window rules,
/// taskbars, launch-or-focus keybinds -- can only fall back to the title.
const APP_ID: &str = "dev.nk.jellybridge";

const ICON_PNG: &[u8] = include_bytes!("../assets/icon.png");
/// jellyfin-web's header logo (CC BY-SA 4.0, see assets/LICENSE-jellyfin-logo.txt).
const LOGO_PNG: &[u8] = include_bytes!("../assets/jellyfin-banner-light.png");

/// The window icon and the brand mark drawn in the UI, decoded once.
struct DecodedIcon {
    window: egui::IconData,
    /// The header logo, downscaled to twice its drawn size: drawing the
    /// 1302 px original at ~100 px would alias.
    brand: egui::ColorImage,
}

fn decode_icon() -> Option<DecodedIcon> {
    let image = image::load_from_memory(ICON_PNG).ok()?.to_rgba8();
    let window = egui::IconData {
        width: image.width(),
        height: image.height(),
        rgba: image.as_raw().clone(),
    };
    let logo = image::load_from_memory(LOGO_PNG).ok()?.to_rgba8();
    // Drawn 29.6 points tall at up to 2x: 90 px covers it.
    let height = 90;
    let width = logo.width() * height / logo.height();
    let small =
        image::imageops::resize(&logo, width, height, image::imageops::FilterType::Triangle);
    let brand =
        egui::ColorImage::from_rgba_unmultiplied([width as usize, height as usize], small.as_raw());
    Some(DecodedIcon { window, brand })
}

/// Keeps freed artwork buffers from inflating resident memory.
///
/// Artwork is decoded on the blocking pool, so the multi-megabyte pixel
/// buffers are allocated on up to eight threads. glibc gives each thread its
/// own arena and keeps what is freed there, and after the first large free it
/// raises its mmap threshold so later buffers come from those arenas too.
/// Measured on the home screen: ~199 MB resident by default, ~170–183 MB
/// with these two settings, for the same pixels.
/// mimalloc, as mytele uses, was measured too: ~7% less CPU while scrolling
/// but 192–292 MB resident against ~170 MB, so glibc stays.
#[cfg(all(target_os = "linux", target_env = "gnu"))]
fn tune_allocator() {
    // SAFETY: mallopt only adjusts allocator parameters; called before any
    // other thread exists.
    unsafe {
        libc::mallopt(libc::M_ARENA_MAX, 2);
        // A fixed threshold: buffers this size and up are always mmap'd and
        // go straight back to the system when freed.
        libc::mallopt(libc::M_MMAP_THRESHOLD, 256 * 1024);
    }
}

#[cfg(not(all(target_os = "linux", target_env = "gnu")))]
fn tune_allocator() {}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    // First statement: time_to_interactive is measured from here, so it
    // includes runtime, backend, and window construction.
    let process_start = Instant::now();
    tune_allocator();
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

    // One worker per logical CPU is tokio's default: 32 OS threads here, for a
    // workload whose real concurrency is six dashboard requests, six artwork
    // permits and eight proxy streams, all of it I/O-bound. Four workers cover
    // that; the blocking pool, which the artwork disk probes, decodes and
    // getaddrinfo use, is capped well below its 512-thread default for the
    // same reason.
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
    // The icon is needed when the window is created; decoding it meanwhile
    // keeps it off the critical path.
    let icon = runtime.spawn_blocking(decode_icon);
    tracing::debug!(
        ms = process_start.elapsed().as_secs_f64() * 1000.0,
        stage = "prefetch_spawned",
        "startup_stage"
    );
    let store = SessionStore::for_current_user()?;
    let settings_store = SettingsStore::for_current_user()?;
    let stored_settings = settings_store.load().unwrap_or_else(|error| {
        tracing::warn!(%error, "catalog settings could not be loaded");
        StoredSettings::default()
    });
    let app_state = AppState {
        grid_settings: stored_settings
            .grid_settings
            .iter()
            .filter_map(|(key, stored)| {
                Some((
                    key.clone(),
                    features::pages::GridSettings::from_stored(stored)?,
                ))
            })
            .collect(),
        device: stored_settings.device.clone(),
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
    let (ui_handle, ui_queue) = UiHandle::new();
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
    let refresh = Arc::new(tokio::sync::Notify::new());
    let event_refresh = refresh.clone();
    let (render_client_slot, player_worker) = create_player(move |event| {
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
        // Skip Intro and Up Next follow the playhead; the now-playing context
        // is small, so this is cheap at mpv's 4 Hz position rate.
        let overlay = match &event {
            PlayerEvent::Position { seconds, duration } => {
                event_state.lock().ok().and_then(|state| {
                    state
                        .now_playing
                        .as_ref()
                        .map(|now| features::now_playing::overlay_at(now, *seconds, *duration))
                })
            }
            _ => None,
        };
        let autoplay = ended
            && event_state.lock().is_ok_and(|state| {
                state
                    .now_playing
                    .as_ref()
                    .is_some_and(features::now_playing::NowPlaying::autoplays)
            });
        let tracks = event_tracks.clone();
        event_ui.update(move |ui_state| {
            ui::player::apply_player_event(ui_state, event, &tracks);
            if let Some(overlay) = overlay {
                ui::player::apply_player_overlay(ui_state, &overlay);
                // A segment the user chose to skip is jumped over at once.
                if let Some(target) = overlay.auto_skip {
                    ui_state.deferred.push(UiAction::SeekAbsolute(target));
                }
            }
            // At the natural end, roll into the next episode or, like
            // Jellyfin, return to where playback started.
            if ended && ui_state.screen == ui::Screen::Player {
                ui_state.deferred.push(if autoplay {
                    UiAction::PlayNext
                } else if ui_state.player.repeat == ui::player::RepeatMode::All {
                    UiAction::ReplayCurrent
                } else {
                    UiAction::LeavePlayer
                });
            }
        });
        if ended {
            // Commit the final position before the proxy goes away; Jellyfin
            // decides from this whether the item counts as watched.
            features::playback::finish_report(
                &event_reporter,
                &event_state,
                &event_runtime,
                &event_refresh,
            );
            if let Ok(mut slot) = event_playback.lock()
                && let Some(remote) = slot.take()
            {
                event_runtime.spawn(remote.shutdown());
            }
        }
    })?;
    tracing::debug!(
        ms = process_start.elapsed().as_secs_f64() * 1000.0,
        stage = "player_created",
        "startup_stage"
    );
    let player_sender = player_worker.command_sender();
    let render_player = player_sender.clone();
    let (render_ready, render_ready_rx) = tokio::sync::watch::channel(false);
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
        render_ready: render_ready_rx,
        player_tracks,
        catalog_request: Arc::new(Mutex::new(None)),
        refresh,
    };
    runtime.spawn(features::refresh::run_refresh_loop(context.clone()));
    // Its updates queue up and land on the window's first frame.
    features::auth::restore_session(&context, prefetched_home);

    let icon = runtime.block_on(icon).ok().flatten();
    let mut viewport = egui::ViewportBuilder::default()
        .with_title("JellyBridge")
        .with_app_id(APP_ID)
        .with_inner_size([1280.0, 800.0])
        .with_min_inner_size([760.0, 520.0]);
    let mut brand_icon = None;
    if let Some(icon) = icon {
        viewport = viewport.with_icon(icon.window);
        brand_icon = Some(icon.brand);
    }
    let mut glow_options = eframe::egui_glow::GlowConfiguration::default();
    // On Wayland, Mesa implements vsync by blocking in swap_buffers until the
    // compositor's frame callback for the previous frame arrives. A window on
    // a hidden workspace never gets one, so the first repaint after switching
    // away would park the UI thread for good and the compositor would report
    // the app as not responding. winit already paces redraws by frame
    // callback on Wayland, so turning vsync off there neither tears nor spins.
    if std::env::var_os("WAYLAND_DISPLAY").is_some() {
        glow_options.vsync = false;
    }
    let options = eframe::NativeOptions {
        viewport,
        // mpv renders through OpenGL on the window's own context, so the
        // window must be drawn with OpenGL too.
        renderer: eframe::Renderer::Glow,
        glow_options,
        ..Default::default()
    };
    let app_context = context.clone();
    tracing::info!("Rust JellyBridge application ready");
    tracing::debug!(
        ms = process_start.elapsed().as_secs_f64() * 1000.0,
        stage = "event_loop_start",
        "startup_stage"
    );
    eframe::run_native(
        "JellyBridge",
        options,
        Box::new(move |creation| {
            let display = native_display(creation).unwrap_or_else(|error| {
                tracing::warn!(%error, "no native display for hardware-decode interop");
                NativeDisplay::None
            });
            if let Some(gl) = creation.gl.as_ref() {
                services::render::note_gl_vendor(gl);
            }
            let wake = creation.egui_ctx.clone();
            let render_host = RenderHost::new(
                render_client_slot,
                render_player,
                display,
                Arc::new(move || wake.request_repaint()),
                render_ready,
            );
            tracing::debug!(
                ms = process_start.elapsed().as_secs_f64() * 1000.0,
                stage = "window_built",
                "startup_stage"
            );
            Ok(Box::new(JellyBridgeApp::new(
                &creation.egui_ctx,
                ui_queue,
                app_context,
                render_host,
                player_worker,
                brand_icon,
            )))
        }),
    )?;

    // The window has torn down mpv and its render context (`on_exit`); shut
    // down the proxy and the runtime.
    context.next_generation();
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
