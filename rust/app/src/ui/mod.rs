//! The only Rust ↔ Slint boundary. Everything that names a generated Slint
//! type — callback wiring, presenters, model mapping, the render seam — lives
//! under this module; features reach the window solely through [`UiHandle`].

pub mod catalog;
pub mod detail;
pub mod player;
pub mod player_hotkeys;

use std::cell::RefCell;
use std::collections::HashMap;

use slint::{ComponentHandle, Image, Model, ModelRc, SharedString, VecModel};

use crate::app::AppContext;
use crate::features;
use crate::features::detail::{DetailItemData, DetailKind as ModelDetailKind, ImageRef};

slint::include_modules!();

/// Cloneable, thread-safe handle to the window: the single funnel through
/// which any thread schedules UI updates onto the Slint event loop.
#[derive(Clone)]
pub struct UiHandle {
    window: slint::Weak<JellyBridgeWindow>,
}

impl UiHandle {
    pub fn new(window: &JellyBridgeWindow) -> Self {
        Self {
            window: window.as_weak(),
        }
    }

    /// Runs `apply` on the UI thread with the window, if it is still alive.
    pub fn update(&self, apply: impl FnOnce(&JellyBridgeWindow) + Send + 'static) {
        let weak = self.window.clone();
        let _ = slint::invoke_from_event_loop(move || {
            if let Some(window) = weak.upgrade() {
                apply(&window);
            }
        });
    }
}

/// Wires every window callback that is not part of the player surface.
pub fn connect(window: &JellyBridgeWindow, context: &AppContext) {
    let ctx = context.clone();
    window.on_login(move |server, username, password| {
        features::auth::start_login(
            &ctx,
            server.as_str().to_owned(),
            username.as_str().to_owned(),
            password.as_str().to_owned(),
        );
    });

    let ctx = context.clone();
    window.on_logout(move || features::auth::logout(&ctx));

    catalog::connect(window, context);
    detail::connect(window, context);
}

pub fn set_screen_busy(context: &AppContext, screen: Screen, busy: bool, message: &str) {
    let message: SharedString = message.into();
    context.ui.update(move |window| {
        window.set_screen(screen);
        window.set_busy(busy);
        window.set_message(message);
    });
}

pub fn set_message(context: &AppContext, message: &str) {
    let message: SharedString = message.into();
    context.ui.update(move |window| {
        window.set_message(message);
    });
}

pub fn show_login(context: &AppContext, message: &str) {
    set_screen_busy(context, Screen::Login, false, message);
}

pub fn show_catalog_error(context: &AppContext, message: String) {
    context.ui.update(move |window| {
        window.set_screen(Screen::Catalog);
        window.set_busy(false);
        window.set_message(message.into());
        window.set_catalog_items(ModelRc::new(VecModel::<CatalogItem>::default()));
    });
}

// UI-thread cache of loaded artwork, keyed by the image identity. Presenters
// populate rows from it at construction time, so replacing a model never
// loses already-loaded images; fetch tasks insert into it even when their
// navigation generation has gone stale, because a fetched image stays valid.
thread_local! {
    static IMAGE_CACHE: RefCell<HashMap<String, Image>> = RefCell::new(HashMap::new());
}

const IMAGE_CACHE_CAP: usize = 1024;

fn cached_image(key: &str) -> Option<Image> {
    IMAGE_CACHE.with(|cache| cache.borrow().get(key).cloned())
}

fn store_cached_image(key: String, image: Image) {
    IMAGE_CACHE.with(|cache| {
        let mut cache = cache.borrow_mut();
        if cache.len() >= IMAGE_CACHE_CAP {
            cache.clear();
        }
        cache.insert(key, image);
    });
}

fn clear_image_cache() {
    IMAGE_CACHE.with(|cache| cache.borrow_mut().clear());
}

/// Clears the artwork cache from any thread (used on logout).
pub fn clear_image_cache_async() {
    let _ = slint::invoke_from_event_loop(clear_image_cache);
}

fn image_key(image: &ImageRef) -> String {
    format!(
        "{}:{}:{}:{}",
        image.item_id,
        image.image_type.jellyfin_name(),
        image.index.unwrap_or(0),
        image.tag
    )
}

fn catalog_image_key(image_item_id: &str, tag: &str) -> String {
    format!("{image_item_id}:Primary:0:{tag}")
}

/// The image shown on a card (and the detail hero): episodes prefer their
/// widescreen thumbnail. Fetchers and presenters must agree on this choice so
/// cache lookups hit.
fn primary_card_image(item: &DetailItemData) -> Option<&ImageRef> {
    if item.kind == ModelDetailKind::Episode {
        item.thumbnail.as_ref().or(item.poster.as_ref())
    } else {
        item.poster.as_ref()
    }
}

/// Updates only the rows that changed when the new rows describe the same
/// item sequence as the current model; returns false when the caller should
/// replace the model wholesale (different items, order, or length).
fn patch_rows<T: Clone + PartialEq + 'static>(
    model: &ModelRc<T>,
    rows: &[T],
    same_identity: impl Fn(&T, &T) -> bool,
) -> bool {
    if model.row_count() != rows.len() {
        return false;
    }
    let aligned = rows.iter().enumerate().all(|(index, row)| {
        model
            .row_data(index)
            .is_some_and(|previous| same_identity(&previous, row))
    });
    if !aligned {
        return false;
    }
    for (index, row) in rows.iter().enumerate() {
        if model.row_data(index).as_ref() != Some(row) {
            model.set_row_data(index, row.clone());
        }
    }
    true
}

pub(crate) fn format_playback_time(seconds: f64) -> String {
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

#[cfg(test)]
mod tests {
    use std::cell::Cell;
    use std::rc::Rc;

    use slint::platform::software_renderer::{MinimalSoftwareWindow, RepaintBufferType};
    use slint::platform::{
        Platform, PlatformError, PointerEventButton, WindowAdapter, WindowEvent,
    };
    use slint::{ComponentHandle, LogicalPosition, PhysicalSize};

    use super::{JellyBridgeWindow, Screen};

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
        ui.set_screen(Screen::Detail);
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
        ui.set_screen(Screen::Player);
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
        ui.set_screen(Screen::Player);
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

        ui.set_screen(Screen::Player);
        ui.invoke_enter_player(5.0, 200.0);
        assert_eq!(ui.get_playback_position(), 5.0);
        assert_eq!(ui.get_playback_duration(), 200.0);
        assert_eq!(ui.get_timeline_display_position(), 5.0);
    }

    #[test]
    fn single_video_click_toggles_playback_after_double_click_window() {
        let platform_window = setup_test_window();
        let ui = JellyBridgeWindow::new().expect("create player UI");
        ui.set_screen(Screen::Player);
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
        ui.set_screen(Screen::Player);
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
        let source = concat!(
            include_str!("../../ui/app-window.slint"),
            include_str!("../../ui/features/catalog/media-tile.slint"),
            include_str!("../../ui/features/detail/detail-headers.slint"),
            include_str!("../../ui/features/detail/episode-row.slint"),
        );
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
}
