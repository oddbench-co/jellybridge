//! The only module that knows egui. Everything that draws, every view model,
//! the presenters that fill them, and the window itself live under here;
//! features reach the window solely through [`UiHandle`].
//!
//! The UI is immediate mode: each frame is drawn afresh from [`UiState`],
//! which Rust owns outright. Presenters never touch the window -- they queue a
//! closure that edits the state, and the window applies queued edits at the
//! start of its next frame, on the UI thread.

pub mod app;
pub mod catalog;
pub mod detail;
pub mod icons;
pub mod images;
pub mod input_script;
pub mod jf;
pub mod login;
pub mod measure;
pub mod player;
pub mod player_hotkeys;
pub mod startup;
pub mod theme;
pub mod top_bar;
pub mod widgets;

use std::sync::{Arc, OnceLock, mpsc};

use crate::app::AppContext;
use crate::features::detail::{DetailItemData, DetailKind, ImageRef};

/// Which screen the window shows.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Screen {
    #[default]
    Login,
    Catalog,
    Detail,
    Player,
}

/// Everything the window draws. Owned by the UI thread; other threads edit it
/// only through [`UiHandle::update`].
pub struct UiState {
    pub ctx: egui::Context,
    pub screen: Screen,
    pub busy: bool,
    pub message: String,
    pub page_title: String,
    pub account_name: String,
    pub account_initial: String,
    pub can_go_back: bool,
    pub catalog: catalog::CatalogVm,
    pub detail: detail::DetailVm,
    pub player: player::PlayerVm,
    pub images: images::ImageCache,
    pub login: login::LoginForm,
    /// The app icon, drawn in the brand marks.
    pub brand_icon: Option<egui::TextureHandle>,
    /// Actions raised outside a frame (mpv ending an item), run on the next.
    pub deferred: Vec<app::UiAction>,
}

impl UiState {
    pub fn new(ctx: egui::Context) -> Self {
        Self {
            ctx,
            screen: Screen::Login,
            busy: false,
            message: String::new(),
            page_title: "Home".to_owned(),
            account_name: String::new(),
            account_initial: "?".to_owned(),
            can_go_back: false,
            catalog: catalog::CatalogVm::default(),
            detail: detail::DetailVm::default(),
            player: player::PlayerVm::default(),
            images: images::ImageCache::default(),
            login: login::LoginForm::default(),
            brand_icon: None,
            deferred: Vec::new(),
        }
    }
}

type Update = Box<dyn FnOnce(&mut UiState) + Send>;

/// Cloneable, thread-safe handle to the window: the single funnel through
/// which any thread schedules UI updates onto the UI thread.
#[derive(Clone)]
pub struct UiHandle {
    sender: mpsc::Sender<Update>,
    /// Set once the window exists; updates queued before that are applied
    /// on its first frame.
    ctx: Arc<OnceLock<egui::Context>>,
}

/// The receiving end of [`UiHandle`], drained by the window every frame.
pub struct UiQueue {
    receiver: mpsc::Receiver<Update>,
    ctx: Arc<OnceLock<egui::Context>>,
}

impl UiHandle {
    pub fn new() -> (Self, UiQueue) {
        let (sender, receiver) = mpsc::channel();
        let ctx = Arc::new(OnceLock::new());
        (
            Self {
                sender,
                ctx: ctx.clone(),
            },
            UiQueue { receiver, ctx },
        )
    }

    /// Runs `apply` on the UI thread with the window state, before the next
    /// frame is drawn. Wakes the window, so the change shows immediately.
    pub fn update(&self, apply: impl FnOnce(&mut UiState) + Send + 'static) {
        if self.sender.send(Box::new(apply)).is_ok()
            && let Some(ctx) = self.ctx.get()
        {
            ctx.request_repaint();
        }
    }
}

impl UiQueue {
    /// Connects the queue to the window, which wakes it for every update.
    pub fn attach(&self, ctx: &egui::Context) {
        let _ = self.ctx.set(ctx.clone());
    }

    /// Applies every queued update, in order.
    pub fn drain(&self, state: &mut UiState) {
        while let Ok(apply) = self.receiver.try_recv() {
            apply(state);
        }
    }
}

pub fn set_screen_busy(context: &AppContext, screen: Screen, busy: bool, message: &str) {
    let message = message.to_owned();
    context.ui.update(move |state| {
        state.screen = screen;
        state.busy = busy;
        state.message = message;
    });
}

pub fn set_message(context: &AppContext, message: &str) {
    let message = message.to_owned();
    context.ui.update(move |state| state.message = message);
}

pub fn show_login(context: &AppContext, message: &str) {
    set_screen_busy(context, Screen::Login, false, message);
    startup::mark_content_ready("login");
}

pub fn show_catalog_error(context: &AppContext, message: String) {
    context.ui.update(move |state| {
        state.screen = Screen::Catalog;
        state.busy = false;
        state.message = message;
        state.catalog.items = Arc::new(Vec::new());
        state.catalog.sections.clear();
    });
}

/// Clears the artwork cache from any thread (used on logout).
pub fn clear_image_cache_async(context: &AppContext) {
    context.ui.update(|state| state.images.clear());
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

/// The width is part of the key: changing density changes the size we ask the
/// server for, and without it the cache would keep serving the image decoded
/// for the old size.
fn catalog_image_key(image_item_id: &str, tag: &str, max_width: u32) -> String {
    format!("{image_item_id}:Primary:0:{tag}@{max_width}")
}

/// The image shown on a card (and the detail hero): episodes prefer their
/// widescreen thumbnail.
fn primary_card_image(item: &DetailItemData) -> Option<&ImageRef> {
    if item.kind == DetailKind::Episode {
        item.thumbnail.as_ref().or(item.poster.as_ref())
    } else {
        item.poster.as_ref()
    }
}

/// Detail-page artwork at `max_width` pixels.
fn detail_art(image: &ImageRef, max_width: u32) -> images::Art {
    images::Art {
        key: image_key(image),
        source: images::ArtSource::Detail {
            image: image.clone(),
            max_width,
        },
    }
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

/// "Ends at 4:47 PM" for `seconds_left` from now, in local time.
pub(crate) fn ends_at_from_now(seconds_left: f64) -> String {
    let Ok(now) = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH) else {
        return String::new();
    };
    let end_epoch = now.as_secs() as i64 + seconds_left.max(0.0).round() as i64;
    // libc::localtime_r converts the epoch using the local timezone; the
    // struct tm is fully written by the call.
    let mut tm: libc::tm = unsafe { std::mem::zeroed() };
    let time: libc::time_t = end_epoch;
    if unsafe { libc::localtime_r(&time, &mut tm) }.is_null() {
        return String::new();
    }
    let (hour, meridiem) = match tm.tm_hour {
        0 => (12, "AM"),
        1..=11 => (tm.tm_hour, "AM"),
        12 => (12, "PM"),
        hour => (hour - 12, "PM"),
    };
    format!("Ends at {}:{:02} {}", hour, tm.tm_min, meridiem)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn updates_queued_before_the_window_exists_apply_in_order_on_its_first_frame() {
        let (handle, queue) = UiHandle::new();
        handle.update(|state| state.message = "first".to_owned());
        handle.update(|state| state.message.push_str(" then second"));
        let ctx = egui::Context::default();
        queue.attach(&ctx);
        let mut state = UiState::new(ctx);
        queue.drain(&mut state);
        assert_eq!(state.message, "first then second");
    }

    #[test]
    fn playback_times_read_like_a_clock() {
        assert_eq!(format_playback_time(0.0), "0:00");
        assert_eq!(format_playback_time(65.4), "1:05");
        assert_eq!(format_playback_time(3725.0), "1:02:05");
        assert_eq!(format_playback_time(-3.0), "0:00");
    }
}
