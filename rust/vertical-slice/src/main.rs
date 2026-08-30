//! PROTOTYPE: packaged Slint ↔ libmpv feasibility slice.
//!
//! This binary must not grow into the production browser. It exists to settle
//! renderer, ownership, proxy, native dependency, and package questions.

use std::cell::{Cell, RefCell};
use std::path::PathBuf;
use std::rc::Rc;
use std::sync::Arc;
use std::time::Duration;

use jellybridge_vertical_slice::jellyfin::{
    JellyfinSliceConfig, PreparedJellyfinPlayback, prepare_jellyfin_playback,
};
use jellybridge_vertical_slice::player::{
    PlayerCommand, PlayerEvent, PlayerSubtitleFile, PlayerWorker, create_player,
};
use jellybridge_vertical_slice::render::{RenderHost, native_display};
use slint::{ComponentHandle, RenderingState};

#[cfg(all(feature = "renderer-femtovg", feature = "renderer-skia-opengl"))]
compile_error!("select exactly one renderer feature");

#[cfg(not(any(feature = "renderer-femtovg", feature = "renderer-skia-opengl")))]
compile_error!("select renderer-femtovg or renderer-skia-opengl");

slint::include_modules!();

#[cfg(feature = "renderer-femtovg")]
const RENDERER_NAME: &str = "femtovg";

#[cfg(feature = "renderer-skia-opengl")]
const RENDERER_NAME: &str = "skia";

fn main() -> Result<(), Box<dyn std::error::Error>> {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| "jellybridge_vertical_slice=info".into()),
        )
        .without_time()
        .init();
    let options = match Options::parse()? {
        ParseResult::Run(options) => options,
        ParseResult::Help => {
            print_help();
            return Ok(());
        }
    };

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
    let (load_uri, subtitle_paths, remote) = prepare_input(&runtime, options.input)?;

    let window = PlayerSliceWindow::new()?;
    window.set_status(format!("{RENDERER_NAME}/OpenGL selected; initializing libmpv").into());

    let event_weak = window.as_weak();
    let (mpv_core, worker) = create_player(move |event| {
        let weak = event_weak.clone();
        let _ = slint::invoke_from_event_loop(move || update_window_from_player(&weak, event));
    })?;
    let image_is_cleared = Rc::new(Cell::new(true));
    wire_controls(&window, &worker, image_is_cleared.clone());
    let load_sender = worker.command_sender();
    let stress_sender = worker.command_sender();
    let worker = Rc::new(RefCell::new(Some(worker)));

    let redraw_weak = window.as_weak();
    let redraw: Arc<dyn Fn() + Send + Sync> = Arc::new(move || {
        let weak = redraw_weak.clone();
        let _ = slint::invoke_from_event_loop(move || {
            if let Some(window) = weak.upgrade() {
                window.window().request_redraw();
            }
        });
    });
    let mut render_host = RenderHost::new(mpv_core, redraw);
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
                RenderingState::BeforeRendering => {
                    let scale = window.window().scale_factor();
                    let width = (window.get_video_logical_width().max(1.0) * scale).round() as u32;
                    let height =
                        (window.get_video_logical_height().max(1.0) * scale).round() as u32;
                    render_host.render_frame(slint::PhysicalSize::new(width, height))
                }
                RenderingState::AfterRendering => {
                    render_host.finish_frame(window.window().size());
                    Ok(None)
                }
                RenderingState::RenderingTeardown => {
                    // Normal close paths clear this property before hiding the
                    // window. Mutating it here re-enters Slint's winit adapter while
                    // the adapter is suspended and panics in Slint 1.17.1.
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
                    tracing::error!(%error, "render seam failed");
                    window.set_status(format!("Rendering failed: {error}").into());
                }
            }
        })?;

    let close_worker = worker.clone();
    let close_weak = window.as_weak();
    let close_image_is_cleared = image_is_cleared.clone();
    window.window().on_close_requested(move || {
        if let Some(worker) = close_worker.borrow_mut().take() {
            worker.shutdown();
        }
        if let Some(window) = close_weak.upgrade() {
            close_image_is_cleared.set(true);
            window.set_video_frame(slint::Image::default());
        }
        slint::CloseRequestResponse::HideWindow
    });

    if let Some(uri) = load_uri {
        slint::Timer::single_shot(Duration::ZERO, move || {
            let _ = load_sender.send(PlayerCommand::Load {
                uri,
                start_seconds: options.start_seconds,
                subtitles: subtitle_paths
                    .into_iter()
                    .map(|path| PlayerSubtitleFile {
                        path,
                        source_index: None,
                        language: None,
                        title: None,
                    })
                    .collect(),
                audio_source_index: None,
                subtitle_source_index: None,
            });
        });
    } else {
        window.set_status(
            format!("{RENDERER_NAME}/OpenGL selected; pass --file or --jellyfin-server").into(),
        );
    }
    if let Some(milliseconds) = options.auto_close_ms {
        let auto_close_worker = worker.clone();
        let weak = window.as_weak();
        let auto_close_image_is_cleared = image_is_cleared.clone();
        slint::Timer::single_shot(Duration::from_millis(milliseconds), move || {
            if let Some(worker) = auto_close_worker.borrow_mut().take() {
                worker.shutdown();
            }
            if let Some(window) = weak.upgrade() {
                auto_close_image_is_cleared.set(true);
                window.set_video_frame(slint::Image::default());
                let _ = window.hide();
            }
            let _ = slint::quit_event_loop();
        });
    }
    let stress_timer = slint::Timer::default();
    if options.stress_ui {
        let weak = window.as_weak();
        let tick = Rc::new(Cell::new(0_u32));
        stress_timer.start(
            slint::TimerMode::Repeated,
            Duration::from_millis(40),
            move || {
                let Some(window) = weak.upgrade() else { return };
                let current = tick.get().wrapping_add(1);
                tick.set(current);
                let width = 800 + (current % 7) * 53;
                let height = 500 + (current % 5) * 47;
                window
                    .window()
                    .set_size(slint::PhysicalSize::new(width, height));
                if current.is_multiple_of(6) {
                    window
                        .window()
                        .set_fullscreen(!window.window().is_fullscreen());
                }
                if current.is_multiple_of(5) {
                    let seconds = if (current / 5).is_multiple_of(2) {
                        0.25
                    } else {
                        -0.25
                    };
                    let _ = stress_sender.send(PlayerCommand::SeekRelative(seconds));
                }
            },
        );
    }

    tracing::info!(renderer = RENDERER_NAME, "vertical slice ready");
    window.run()?;

    if let Some(worker) = worker.borrow_mut().take() {
        worker.shutdown();
    }
    drop(window);
    if let Some(remote) = remote {
        runtime.block_on(remote.shutdown());
    }
    Ok(())
}

fn wire_controls(
    window: &PlayerSliceWindow,
    worker: &PlayerWorker,
    image_is_cleared: Rc<Cell<bool>>,
) {
    let sender = worker.command_sender();
    window.on_toggle_playback(move || {
        let _ = sender.send(PlayerCommand::TogglePaused);
    });
    let sender = worker.command_sender();
    window.on_seek_relative(move |seconds| {
        let _ = sender.send(PlayerCommand::SeekRelative(seconds as f64));
    });
    let sender = worker.command_sender();
    window.on_volume_changed(move |volume| {
        let _ = sender.send(PlayerCommand::SetVolume(volume as f64));
    });
    let weak = window.as_weak();
    window.on_toggle_fullscreen(move || {
        if let Some(window) = weak.upgrade() {
            window
                .window()
                .set_fullscreen(!window.window().is_fullscreen());
        }
    });
    let sender = worker.command_sender();
    let weak = window.as_weak();
    window.on_stop_playback(move || {
        let _ = sender.send(PlayerCommand::Stop);
        if let Some(window) = weak.upgrade() {
            window.set_status("Stopped".into());
            image_is_cleared.set(true);
            window.set_video_frame(slint::Image::default());
        }
    });
}

fn update_window_from_player(weak: &slint::Weak<PlayerSliceWindow>, event: PlayerEvent) {
    let Some(window) = weak.upgrade() else { return };
    match event {
        PlayerEvent::Loaded => window.set_status("Playing; decode path pending".into()),
        PlayerEvent::Paused(paused) => {
            window.set_paused(paused);
            window.set_status(if paused { "Paused" } else { "Playing" }.into());
        }
        PlayerEvent::Volume(volume) => window.set_volume(volume as f32),
        PlayerEvent::Position { .. } | PlayerEvent::Tracks { .. } => {}
        PlayerEvent::HardwarePath(path) => {
            tracing::info!(decode_path = %path, "hardware decode classification");
            window.set_status(format!("Decode path: {path}").into());
        }
        PlayerEvent::Ended => window.set_status("Playback ended".into()),
        PlayerEvent::Error(message) => {
            tracing::error!(%message, "player failed");
            window.set_status(message.into());
        }
    }
}

type PreparedInput = (
    Option<String>,
    Vec<PathBuf>,
    Option<PreparedJellyfinPlayback>,
);

fn prepare_input(
    runtime: &tokio::runtime::Runtime,
    input: Option<Input>,
) -> Result<PreparedInput, Box<dyn std::error::Error>> {
    match input {
        None => Ok((None, Vec::new(), None)),
        Some(Input::File { path, subtitles }) => {
            let path = path.canonicalize()?;
            if !path.is_file() {
                return Err("--file must name a regular media file".into());
            }
            let uri = path
                .to_str()
                .ok_or("media path is not valid UTF-8")?
                .to_owned();
            Ok((Some(uri), subtitles, None))
        }
        Some(Input::Jellyfin {
            server,
            item_id,
            user_id,
            token_file,
            ca_certificate,
        }) => {
            let config = JellyfinSliceConfig::from_token_file(
                &server,
                item_id,
                user_id,
                &token_file,
                ca_certificate.as_deref(),
            )?;
            let remote = runtime.block_on(prepare_jellyfin_playback(config))?;
            Ok((
                Some(remote.local_url.clone()),
                remote.subtitles.clone(),
                Some(remote),
            ))
        }
    }
}

enum Input {
    File {
        path: PathBuf,
        subtitles: Vec<PathBuf>,
    },
    Jellyfin {
        server: String,
        item_id: String,
        user_id: String,
        token_file: PathBuf,
        ca_certificate: Option<PathBuf>,
    },
}

struct Options {
    input: Option<Input>,
    start_seconds: f64,
    auto_close_ms: Option<u64>,
    stress_ui: bool,
}

enum ParseResult {
    Run(Options),
    Help,
}

impl Options {
    fn parse() -> Result<ParseResult, String> {
        let mut args = std::env::args_os().skip(1);
        let mut file = None;
        let mut subtitles = Vec::new();
        let mut server = None;
        let mut item_id = None;
        let mut user_id = None;
        let mut token_file = None;
        let mut ca_certificate = None;
        let mut start_seconds = 0.0;
        let mut auto_close_ms = None;
        let mut stress_ui = false;
        while let Some(argument) = args.next() {
            let argument = argument.to_string_lossy();
            match argument.as_ref() {
                "-h" | "--help" => return Ok(ParseResult::Help),
                "--file" => file = Some(next_path(&mut args, "--file")?),
                "--subtitle" => subtitles.push(next_path(&mut args, "--subtitle")?),
                "--jellyfin-server" => server = Some(next_string(&mut args, "--jellyfin-server")?),
                "--item-id" => item_id = Some(next_string(&mut args, "--item-id")?),
                "--user-id" => user_id = Some(next_string(&mut args, "--user-id")?),
                "--token-file" => token_file = Some(next_path(&mut args, "--token-file")?),
                "--ca-certificate" => {
                    ca_certificate = Some(next_path(&mut args, "--ca-certificate")?)
                }
                "--start" => {
                    start_seconds = next_string(&mut args, "--start")?
                        .parse::<f64>()
                        .map_err(|_| "--start must be a number of seconds".to_owned())?;
                    if !start_seconds.is_finite() || start_seconds < 0.0 {
                        return Err("--start must be a finite non-negative number".into());
                    }
                }
                "--auto-close-ms" => {
                    let value = next_string(&mut args, "--auto-close-ms")?
                        .parse::<u64>()
                        .map_err(|_| "--auto-close-ms must be a positive integer".to_owned())?;
                    if value == 0 {
                        return Err("--auto-close-ms must be a positive integer".into());
                    }
                    auto_close_ms = Some(value);
                }
                "--stress-ui" => stress_ui = true,
                other => return Err(format!("unknown argument: {other}")),
            }
        }

        let jellyfin_used = server.is_some()
            || item_id.is_some()
            || user_id.is_some()
            || token_file.is_some()
            || ca_certificate.is_some();
        let input = if let Some(path) = file {
            if jellyfin_used {
                return Err("--file cannot be combined with Jellyfin input options".into());
            }
            Some(Input::File { path, subtitles })
        } else if jellyfin_used {
            if !subtitles.is_empty() {
                return Err("--subtitle is only accepted with --file; Jellyfin subtitles are materialized from playback info".into());
            }
            Some(Input::Jellyfin {
                server: server.ok_or("missing --jellyfin-server")?,
                item_id: item_id.ok_or("missing --item-id")?,
                user_id: user_id.ok_or("missing --user-id")?,
                token_file: token_file.ok_or("missing --token-file")?,
                ca_certificate,
            })
        } else {
            if !subtitles.is_empty() {
                return Err("--subtitle requires --file".into());
            }
            None
        };
        Ok(ParseResult::Run(Self {
            input,
            start_seconds,
            auto_close_ms,
            stress_ui,
        }))
    }
}

fn next_path(
    args: &mut impl Iterator<Item = std::ffi::OsString>,
    name: &str,
) -> Result<PathBuf, String> {
    args.next()
        .map(PathBuf::from)
        .ok_or_else(|| format!("missing value after {name}"))
}

fn next_string(
    args: &mut impl Iterator<Item = std::ffi::OsString>,
    name: &str,
) -> Result<String, String> {
    args.next()
        .and_then(|value| value.into_string().ok())
        .ok_or_else(|| format!("missing or non-UTF-8 value after {name}"))
}

fn print_help() {
    println!(
        "JellyBridge embedded-player feasibility slice\n\n\
         Local media:\n  jellybridge-player-slice --file FILE [--subtitle FILE] [--start SECONDS] [--auto-close-ms MS] [--stress-ui]\n\n\
         Authenticated Jellyfin DirectPlay:\n  jellybridge-player-slice --jellyfin-server URL --item-id ID --user-id ID --token-file FILE [--start SECONDS]\n\n\
         Add --ca-certificate PEM for a per-playback, per-origin private CA. The token file must be mode 0600. Tokens are sent only in authenticated headers; mpv receives an unguessable loopback URL.\n\
         Set JELLYBRIDGE_GL_DEBUG=1 and JELLYBRIDGE_GL_SENTINEL=1 for certification diagnostics."
    );
}
