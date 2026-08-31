//! Player surface: control wiring, hotkeys, mpv event application, and the
//! OpenGL render seam.

use std::cell::{Cell, RefCell};
use std::rc::Rc;
use std::sync::{Arc, Mutex};

use slint::{ComponentHandle, Image, Model, ModelRc, RenderingState, SharedString, VecModel};

use super::player_hotkeys::{
    PlayerHotkeyAction, PlayerUiState, next_track_index, resolve_player_hotkey,
};
use super::{JellyBridgeWindow, format_playback_time};
use crate::app::{AppContext, PlayerTrackState};
use crate::features;
use crate::services::player::{PlayerCommand, PlayerEvent, RenderClient};
use crate::services::render::{RenderHost, native_display};

/// Wires the player controls and hotkeys; returns the overlay/fullscreen UI
/// state shared with the rendering notifier.
pub fn connect(
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
    let ctx = context.clone();
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
            persist_track_preference_async(&ctx, key, Some(track), None);
        }
    });
    let sender = context.player_sender.clone();
    let tracks = context.player_tracks.clone();
    let ctx = context.clone();
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
            persist_track_preference_async(&ctx, key, None, Some(track));
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
                let overlay_visible = timer_state.borrow().overlay_visible;
                window.set_controls_visible(overlay_visible);
                // The declarative mouse-cursor binding only takes effect on
                // the next pointer event, and any pointer event re-shows the
                // overlay — so the cursor must be hidden imperatively when
                // the overlay times out.
                if !overlay_visible {
                    set_cursor_visible(&window, false);
                }
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
            set_cursor_visible(&window, true);
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
            set_cursor_visible(&window, true);
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
        features::playback::stop_remote_playback(&ctx);
        if let Some(window) = weak.upgrade() {
            leave_state.borrow_mut().set_fullscreen(false);
            leave_timer.stop();
            image_is_cleared.set(true);
            window.set_video_frame(Image::default());
            window.window().set_fullscreen(false);
            window.set_fullscreen_player(false);
            window.set_controls_visible(true);
            set_cursor_visible(&window, true);
            if ctx
                .state
                .lock()
                .is_ok_and(|state| state.detail.snapshot().page.is_some())
            {
                window.set_screen(super::Screen::Detail);
            } else {
                window.set_screen(super::Screen::Catalog);
            }
        }
    });

    // The hotkey handler deliberately calls back into the window's own
    // callbacks (invoke_*) and reads window state: it runs on the UI thread
    // and must reuse the exact behavior of the on-screen controls.
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

/// Shows or hides the OS cursor. Goes through the winit window because the
/// declarative TouchArea cursor binding is only applied on pointer events.
fn set_cursor_visible(window: &JellyBridgeWindow, visible: bool) {
    use i_slint_backend_winit::WinitWindowAccessor;
    window
        .window()
        .with_winit_window(|winit_window| winit_window.set_cursor_visible(visible));
}

/// The settings store does blocking file I/O, so track preferences are
/// persisted off the UI thread.
fn persist_track_preference_async(
    context: &AppContext,
    key: Option<String>,
    audio: Option<crate::services::player::PlayerTrack>,
    subtitle: Option<Option<crate::services::player::PlayerTrack>>,
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

/// Applies one mpv worker event to the window. Runs on the UI thread.
pub fn apply_player_event(
    window: &JellyBridgeWindow,
    event: PlayerEvent,
    track_state: &Arc<Mutex<PlayerTrackState>>,
) {
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

/// Enters the player screen after a stream was prepared and handed to mpv.
pub fn present_playback_started(
    context: &AppContext,
    title: String,
    initial_position: f64,
    initial_duration: f64,
) {
    context.ui.update(move |window| {
        window.set_detail_title(title.into());
        window.set_playback_position_label(format_playback_time(initial_position).into());
        window.set_playback_duration_label(format_playback_time(initial_duration).into());
        window.set_paused(false);
        window.set_screen(super::Screen::Player);
        window.invoke_enter_player(initial_position as f32, initial_duration as f32);
        window.set_busy(false);
        window.set_message("".into());
        window.set_player_status("Loading media…".into());
        window.window().request_redraw();
    });
}

pub fn present_playback_failed(context: &AppContext, message: String) {
    context.ui.update(move |window| {
        window.set_busy(false);
        window.set_message(message.into());
    });
}

/// Installs the mpv → Slint OpenGL render seam. The notifier closure owns the
/// RenderHost and must observe `image_is_cleared` during teardown.
pub fn install_renderer(
    window: &JellyBridgeWindow,
    render_client: RenderClient,
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
                RenderingState::BeforeRendering if window.get_screen() == super::Screen::Player => {
                    let scale = window.window().scale_factor();
                    let width = (window.get_video_logical_width().max(1.0) * scale).round() as u32;
                    let height =
                        (window.get_video_logical_height().max(1.0) * scale).round() as u32;
                    render_host.render_frame(slint::PhysicalSize::new(width, height))
                }
                RenderingState::BeforeRendering => Ok(None),
                RenderingState::AfterRendering => {
                    if window.get_screen() == super::Screen::Player {
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

/// Shuts down the mpv worker and clears the video frame; shared by the
/// window-close handler and end-of-process teardown. `image_is_cleared` must
/// be set before the frame is cleared so the renderer teardown can skip the
/// GL sentinel check for an already-released surface.
pub fn shutdown_player_surface(
    worker: &Rc<RefCell<Option<crate::services::player::PlayerWorker>>>,
    window: &JellyBridgeWindow,
    image_is_cleared: &Rc<Cell<bool>>,
) {
    if let Some(worker) = worker.borrow_mut().take() {
        worker.shutdown();
    }
    image_is_cleared.set(true);
    window.set_video_frame(Image::default());
}
