//! The only module that knows egui. Everything that draws, every view model,
//! the presenters that fill them, and the window itself live under here;
//! features reach the window solely through [`UiHandle`].
//!
//! The UI is immediate mode: each frame is drawn afresh from [`UiState`],
//! which Rust owns outright. Presenters never touch the window -- they queue a
//! closure that edits the state, and the window applies queued edits at the
//! start of its next frame, on the UI thread.

pub mod app;

/// The window's width in points, as of the last frame (0 before the
/// first). Loaders that size requests by it, as jellyfin-web sizes some by
/// its window, read it from any thread.
pub fn window_width() -> f32 {
    f32::from_bits(WINDOW_WIDTH.load(std::sync::atomic::Ordering::Relaxed))
}

pub(crate) fn note_window_width(width: f32) {
    WINDOW_WIDTH.store(width.to_bits(), std::sync::atomic::Ordering::Relaxed);
}

static WINDOW_WIDTH: std::sync::atomic::AtomicU32 = std::sync::atomic::AtomicU32::new(0);
pub mod catalog;
pub mod detail;
pub mod dialogs;
pub mod icons;
pub mod images;
pub mod input_script;
pub mod jf;
pub mod login;
pub mod measure;
pub mod menu;
pub mod player;
pub mod player_hotkeys;
pub mod scrollbar;
pub mod settings;
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
    pub can_go_back: bool,
    pub catalog: catalog::CatalogVm,
    pub detail: detail::DetailVm,
    pub player: player::PlayerVm,
    pub images: images::ImageCache,
    pub login: login::LoginForm,
    /// Whether the side drawer is open.
    pub drawer_open: bool,
    /// The open item menu ("⋮"), its dialog, and the confirmation toast.
    pub menu: Option<menu::OpenMenu>,
    pub item_dialog: Option<menu::ItemDialog>,
    pub toast: Option<menu::Toast>,
    /// The user's libraries, listed in the drawer.
    pub libraries: Vec<crate::features::pages::LibraryRef>,
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
            can_go_back: false,
            catalog: catalog::CatalogVm::default(),
            detail: detail::DetailVm::default(),
            player: player::PlayerVm::default(),
            images: images::ImageCache::default(),
            login: login::LoginForm::default(),
            drawer_open: false,
            menu: None,
            item_dialog: None,
            toast: None,
            libraries: Vec::new(),
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

/// Shows a short confirmation at the bottom of the window.
pub fn show_toast(state: &mut UiState, text: &str) {
    let now = state.ctx.input(|input| input.time);
    state.toast = Some(menu::Toast {
        text: text.to_owned(),
        until: now + menu::TOAST_SECONDS,
    });
    state.ctx.request_repaint();
}

pub fn set_message(context: &AppContext, message: &str) {
    let message = message.to_owned();
    context.ui.update(move |state| state.message = message);
}

pub fn show_login(context: &AppContext, message: &str) {
    set_screen_busy(context, Screen::Login, false, message);
    startup::mark_content_ready("login");
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
/// Artwork fetched with jellyfin-web's own parameters (see
/// [`images::ArtSource::Web`]).
fn web_art(image: &ImageRef, max_width: Option<u32>, quality: u8) -> images::Art {
    images::Art {
        key: format!(
            "{}@web{}q{quality}",
            image_key(image),
            max_width.unwrap_or(0)
        ),
        source: images::ArtSource::Web {
            image: image.clone(),
            max_width,
            quality,
        },
    }
}

fn detail_art(image: &ImageRef, max_width: u32) -> images::Art {
    images::Art {
        key: image_key(image),
        source: images::ArtSource::Detail {
            image: image.clone(),
            max_width,
        },
    }
}

/// A running time as jellyfin-web writes it ("42:37", "1:02:03"), whole
/// seconds floored like its getDisplayRunningTime.
pub(crate) fn format_playback_time(seconds: f64) -> String {
    let total = seconds.max(0.0).floor() as u64;
    let hours = total / 3600;
    let minutes = (total % 3600) / 60;
    let seconds = total % 60;
    if hours > 0 {
        format!("{hours}:{minutes:02}:{seconds:02}")
    } else {
        format!("{minutes}:{seconds:02}")
    }
}

/// `epoch` seconds as a calendar time in the local timezone.
pub(crate) fn local_time(epoch: i64) -> Option<libc::tm> {
    // SAFETY: tm is plain data, fully written by a successful call.
    let mut tm: libc::tm = unsafe { std::mem::zeroed() };
    let time: libc::time_t = epoch as libc::time_t;
    #[cfg(unix)]
    let ok = !unsafe { libc::localtime_r(&time, &mut tm) }.is_null();
    #[cfg(windows)]
    let ok = unsafe { libc::localtime_s(&mut tm, &time) } == 0;
    ok.then_some(tm)
}

/// "Ends at 4:47 PM" for `seconds_left` from now, in local time.
pub(crate) fn ends_at_from_now(seconds_left: f64) -> String {
    let Ok(now) = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH) else {
        return String::new();
    };
    let end_epoch = now.as_secs() as i64 + seconds_left.max(0.0).round() as i64;
    let Some(tm) = local_time(end_epoch) else {
        return String::new();
    };
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
