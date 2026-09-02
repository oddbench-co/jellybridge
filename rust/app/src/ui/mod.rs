//! The only Rust ↔ Slint boundary. Everything that names a generated Slint
//! type — callback wiring, presenters, model mapping, the render seam — lives
//! under this module; features reach the window solely through [`UiHandle`].

pub mod catalog;
pub mod detail;
pub mod player;
pub mod player_hotkeys;
pub mod startup;

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
    startup::mark_content_ready("login");
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
    static IMAGE_CACHE: RefCell<ImageCache> = RefCell::new(ImageCache::default());
}

/// How many bytes of decoded artwork stay resident.
///
/// A 320 px poster decodes to ~590 KB and a 1600 px backdrop to ~5.7 MB, so
/// this holds a full 120-card grid page (~71 MB) with room to spare, which is
/// what keeps scrolling back from re-decoding. Measured before the budget
/// existed: one library browse plus six detail pages reached 80 MB and was
/// still climbing, because nothing was ever evicted.
const IMAGE_CACHE_BUDGET: usize = 96 * 1024 * 1024;

/// Decoded artwork, bounded by the bytes it holds.
///
/// Entry counts do not describe this cache -- entries differ by an order of
/// magnitude in size -- and the previous policy, `clear()` at 1024 entries, was
/// a cliff rather than a gradient: it dropped every image at once and the next
/// present re-decoded all of them.
#[derive(Default)]
struct ImageCache {
    entries: HashMap<String, Entry>,
    bytes: usize,
    /// Monotonic use counter. Cheaper than maintaining a linked list for a
    /// cache that only evicts when it is over budget.
    clock: u64,
    /// Highest `bytes` ever reached, reported when it grows, so a profiling
    /// run records the peak without sampling.
    reported_high_water: usize,
}

struct Entry {
    image: Image,
    bytes: usize,
    used: u64,
}

impl ImageCache {
    fn get(&mut self, key: &str) -> Option<Image> {
        self.clock += 1;
        let clock = self.clock;
        let entry = self.entries.get_mut(key)?;
        entry.used = clock;
        Some(entry.image.clone())
    }

    fn insert(&mut self, key: String, image: Image, bytes: usize) {
        self.clock += 1;
        let entry = Entry {
            image,
            bytes,
            used: self.clock,
        };
        if let Some(previous) = self.entries.insert(key, entry) {
            self.bytes = self.bytes.saturating_sub(previous.bytes);
        }
        self.bytes += bytes;
        self.report_growth();
        self.evict_to_budget();
    }

    /// Drops least-recently-used entries until the cache fits. Pruning to 80%
    /// of the budget rather than exactly to it keeps a cache sitting on the
    /// limit from evicting on every single insert.
    fn evict_to_budget(&mut self) {
        if self.bytes <= IMAGE_CACHE_BUDGET {
            return;
        }
        let target = IMAGE_CACHE_BUDGET / 5 * 4;
        let mut by_age: Vec<(u64, String)> = self
            .entries
            .iter()
            .map(|(key, entry)| (entry.used, key.clone()))
            .collect();
        by_age.sort_unstable_by_key(|(used, _)| *used);
        for (_, key) in by_age {
            if self.bytes <= target {
                break;
            }
            if let Some(entry) = self.entries.remove(&key) {
                self.bytes = self.bytes.saturating_sub(entry.bytes);
            }
        }
        self.reported_high_water = self.bytes;
    }

    fn clear(&mut self) {
        self.entries.clear();
        self.bytes = 0;
    }

    /// Logs each new 8 MiB high-water mark. Reporting every insert would spam
    /// a profiling log with hundreds of lines; reporting nothing would leave
    /// the peak unmeasurable.
    fn report_growth(&mut self) {
        const STEP: usize = 8 * 1024 * 1024;
        if self.bytes < self.reported_high_water + STEP {
            return;
        }
        self.reported_high_water = self.bytes - self.bytes % STEP;
        tracing::info!(
            mib = self.bytes as f64 / (1024.0 * 1024.0),
            entries = self.entries.len(),
            "image_cache_high_water"
        );
    }
}

fn cached_image(key: &str) -> Option<Image> {
    IMAGE_CACHE.with(|cache| cache.borrow_mut().get(key))
}

fn store_cached_image(key: String, image: Image) {
    let bytes = decoded_bytes(&image);
    IMAGE_CACHE.with(|cache| cache.borrow_mut().insert(key, image, bytes));
}

/// Upper bound on what a decoded image costs.
///
/// Slint stores artwork without an alpha channel as RGB8, so this over-counts
/// a JPEG poster by a third. That is the safe direction for a budget: it
/// evicts slightly sooner than strictly necessary, and there is no API that
/// reports the stored pixel format.
fn decoded_bytes(image: &Image) -> usize {
    let size = image.size();
    size.width as usize * size.height as usize * 4
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

/// The width is part of the key: changing density changes the size we ask the
/// server for, and without it the cache would keep serving the image decoded
/// for the old size.
fn catalog_image_key(image_item_id: &str, tag: &str, max_width: u32) -> String {
    format!("{image_item_id}:Primary:0:{tag}@{max_width}")
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

    use super::{IMAGE_CACHE_BUDGET, ImageCache, JellyBridgeWindow, Screen};
    use slint::{Image, Rgba8Pixel, SharedPixelBuffer};

    /// An image of a known decoded size, for the cache tests.
    fn image_of(mib: usize) -> (Image, usize) {
        let bytes = mib * 1024 * 1024;
        let pixels = bytes / 4;
        (
            Image::from_rgba8(SharedPixelBuffer::<Rgba8Pixel>::new(pixels as u32, 1)),
            bytes,
        )
    }

    #[test]
    fn the_image_cache_evicts_the_least_recently_used_entry_first() {
        let mut cache = ImageCache::default();
        for index in 0..12 {
            let (image, bytes) = image_of(8);
        cache.insert(format!("cold-{index}"), image, bytes);
        }
        // 96 MB is exactly the budget, so nothing has been evicted yet.
        assert_eq!(cache.entries.len(), 12);

        // Touch the first one so it is no longer the oldest, then overflow.
        assert!(cache.get("cold-0").is_some());
        let (image, bytes) = image_of(8);
        cache.insert("hot".to_owned(), image, bytes);

        assert!(
            cache.bytes <= IMAGE_CACHE_BUDGET,
            "cache is {} bytes, over its budget",
            cache.bytes
        );
        assert!(cache.entries.contains_key("hot"), "the new entry was evicted");
        assert!(
            cache.entries.contains_key("cold-0"),
            "a recently used entry was evicted before older ones"
        );
        assert!(
            !cache.entries.contains_key("cold-1"),
            "the least recently used entry survived"
        );
    }

    #[test]
    fn the_image_cache_accounts_bytes_across_replacement_and_clear() {
        let mut cache = ImageCache::default();
        let (image, bytes) = image_of(8);
        cache.insert("one".to_owned(), image, bytes);
        let (image, bytes) = image_of(8);
        cache.insert("two".to_owned(), image, bytes);
        assert_eq!(cache.bytes, 16 * 1024 * 1024);

        // Replacing an entry must not double-count the one it displaced.
        let (image, bytes) = image_of(4);
        cache.insert("one".to_owned(), image, bytes);
        assert_eq!(cache.bytes, 12 * 1024 * 1024);
        assert_eq!(cache.entries.len(), 2);

        cache.clear();
        assert_eq!(cache.bytes, 0);
        assert!(cache.entries.is_empty());
    }

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


    /// Opening a second item used to show the first one's poster, tagline,
    /// genres and cast for a few hundred milliseconds: `DetailMessage::Open`
    /// dropped the outgoing page from the state machine, but nothing blanked
    /// the window, and the reset that did exist skipped the loudest fields.
    #[test]
    fn resetting_the_detail_page_leaves_nothing_of_the_previous_item() {
        use slint::{Image, Model, ModelRc, VecModel};

        let _window = setup_test_window();
        let ui = JellyBridgeWindow::new().expect("create detail UI");
        ui.set_detail_title("Movie A".into());
        ui.set_detail_overview("A's synopsis".into());
        ui.set_detail_tagline("A's tagline".into());
        ui.set_detail_genres("Action • Crime".into());
        ui.set_detail_studios("A Studio".into());
        ui.set_detail_media_info("2160p • HEVC • EAC3".into());
        ui.set_detail_director("A Director".into());
        ui.set_detail_subtitle("2005 • 47m".into());
        ui.set_detail_playable(true);
        ui.set_detail_watched(true);
        ui.set_detail_people(ModelRc::new(VecModel::from(vec![super::DetailPerson {
            id: "p1".into(),
            name: "Someone".into(),
            role: "Lead".into(),
            artwork: Image::default(),
        }])));

        super::detail::clear_detail_page(&ui);

        for (field, value) in [
            ("title", ui.get_detail_title()),
            ("overview", ui.get_detail_overview()),
            ("tagline", ui.get_detail_tagline()),
            ("genres", ui.get_detail_genres()),
            ("studios", ui.get_detail_studios()),
            ("media info", ui.get_detail_media_info()),
            ("director", ui.get_detail_director()),
            ("subtitle", ui.get_detail_subtitle()),
        ] {
            assert_eq!(value, "", "{field} survived the reset");
        }
        assert_eq!(ui.get_detail_people().row_count(), 0, "cast survived");
        assert!(!ui.get_detail_playable());
        assert!(!ui.get_detail_watched());
    }

    #[test]
    fn season_page_scrolls_with_the_wheel() {
        use slint::{ModelRc, VecModel};
        let platform_window = setup_test_window();
        let ui = JellyBridgeWindow::new().expect("create window");
        ui.set_screen(Screen::Detail);
        ui.set_detail_kind(super::DetailKind::Season);
        let episodes = (0..30)
            .map(|i| super::DetailCard {
                id: format!("e{i}").into(),
                title: format!("Episode {i}").into(),
                subtitle: "40 min".into(),
                overview: "An overview line that wraps a little bit.".into(),
                kind: "Episode".into(),
                artwork: Default::default(),
                watched: false,
                favorite: false,
                progress: 0.4,
                unplayed_count: 0,
                rating: 6.4,
                ends_at: "Ends at 4:00 PM".into(),
                index_label: format!("{i}.").into(),
            })
            .collect::<Vec<_>>();
        ui.set_detail_episodes(ModelRc::new(VecModel::from(episodes)));
        ui.show().expect("show");
        platform_window.draw_if_needed(|_| {});
        assert!(
            ui.get_detail_page_height() > 2000.0,
            "the episode rows must grow the page beyond the viewport"
        );
        ui.window().dispatch_event(WindowEvent::PointerMoved {
            position: LogicalPosition::new(640.0, 400.0),
        });
        platform_window.draw_if_needed(|_| {});
        ui.window().dispatch_event(WindowEvent::PointerScrolled {
            position: LogicalPosition::new(640.0, 400.0),
            delta_x: 0.0,
            delta_y: -240.0,
        });
        for _ in 0..20 {
            std::thread::sleep(std::time::Duration::from_millis(16));
            slint::platform::update_timers_and_animations();
            platform_window.draw_if_needed(|_| {});
        }
        assert!(
            ui.get_detail_page_scroll_y() < -1.0,
            "wheel scrolling over the episode list must scroll the page; viewport stayed at {}",
            ui.get_detail_page_scroll_y()
        );
    }

    /// Drives the window through the event loop so scroll animations settle.
    fn settle(platform_window: &Rc<MinimalSoftwareWindow>) {
        for _ in 0..20 {
            std::thread::sleep(std::time::Duration::from_millis(16));
            slint::platform::update_timers_and_animations();
            platform_window.draw_if_needed(|_| {});
        }
    }

    fn season_episodes(count: usize) -> Vec<super::DetailCard> {
        (0..count)
            .map(|i| super::DetailCard {
                id: format!("e{i}").into(),
                title: format!("Episode {i}").into(),
                subtitle: "40 min".into(),
                overview: "An overview line that wraps a little bit.".into(),
                kind: "Episode".into(),
                artwork: Default::default(),
                watched: false,
                favorite: false,
                progress: 0.4,
                unplayed_count: 0,
                rating: 6.4,
                ends_at: "Ends at 4:00 PM".into(),
                index_label: format!("{i}.").into(),
            })
            .collect()
    }

    /// A scroll offset is only clamped where it is written, against the bounds
    /// in force at that moment. Growing the window shrinks the scrollable
    /// range, so an offset captured while the window was small is left out of
    /// range -- the page renders scrolled past its own end and looks stuck.
    #[test]
    fn detail_page_scroll_is_reclamped_when_the_window_grows() {
        use slint::{ModelRc, VecModel};
        let platform_window = setup_test_window();
        let ui = JellyBridgeWindow::new().expect("create window");
        ui.set_screen(Screen::Detail);
        ui.set_detail_kind(super::DetailKind::Season);
        ui.set_detail_episodes(ModelRc::new(VecModel::from(season_episodes(30))));
        ui.show().expect("show");
        platform_window.draw_if_needed(|_| {});

        // Scroll to the bottom of a page that is far taller than the viewport.
        ui.window().dispatch_event(WindowEvent::PointerMoved {
            position: LogicalPosition::new(640.0, 400.0),
        });
        platform_window.draw_if_needed(|_| {});
        for _ in 0..12 {
            ui.window().dispatch_event(WindowEvent::PointerScrolled {
                position: LogicalPosition::new(640.0, 400.0),
                delta_x: 0.0,
                delta_y: -480.0,
            });
            settle(&platform_window);
        }
        let scrolled = ui.get_detail_page_scroll_y();
        assert!(
            scrolled < -1.0,
            "precondition: the page must be scrolled before the resize, was {scrolled}"
        );

        // Grow the window past the full page height: nothing is scrollable any
        // more, so the only valid offset is 0.
        let page_height = ui.get_detail_page_height();
        platform_window.set_size(PhysicalSize::new(1280, page_height as u32 + 400));
        settle(&platform_window);

        assert!(
            ui.get_detail_page_scroll_y().abs() < 1.0,
            "a viewport taller than the page must reset the scroll offset; it stayed at {} \
             (page height {page_height})",
            ui.get_detail_page_scroll_y()
        );
    }

    /// Same defect on the horizontal axis: the shared ShelfScrollArea clamps
    /// offset-x only inside its own scroll-event, so widening the window leaves
    /// the shelf scrolled past its new end, showing blank space after the last
    /// card until the next wheel tick snaps it back.
    #[test]
    fn shelf_offset_is_reclamped_when_the_window_grows() {
        use slint::{ModelRc, VecModel};
        let platform_window = setup_test_window();
        platform_window.set_size(PhysicalSize::new(700, 800));
        let ui = JellyBridgeWindow::new().expect("create window");
        ui.set_screen(Screen::Detail);
        ui.set_detail_kind(super::DetailKind::Movie);
        // Cards are 112px + 14px of spacing and the shelf viewport is capped at
        // 1180px (people-shelf.slint), so keep the cast narrow enough to fit
        // inside that cap once the window is wide -- otherwise the shelf stays
        // legitimately scrollable and there is nothing to reset to zero.
        let people = (0..8)
            .map(|i| super::DetailPerson {
                id: format!("p{i}").into(),
                name: format!("Performer {i}").into(),
                role: format!("Role {i}").into(),
                artwork: Default::default(),
            })
            .collect::<Vec<_>>();
        ui.set_detail_people(ModelRc::new(VecModel::from(people)));
        ui.show().expect("show");
        settle(&platform_window);

        // Scroll the cast shelf to its right-hand end. The shelf sits below the
        // hero, so aim at the lower half of the page.
        let cast_y = 700.0;
        ui.window().dispatch_event(WindowEvent::PointerMoved {
            position: LogicalPosition::new(350.0, cast_y),
        });
        platform_window.draw_if_needed(|_| {});
        for _ in 0..12 {
            ui.window().dispatch_event(WindowEvent::PointerScrolled {
                position: LogicalPosition::new(350.0, cast_y),
                delta_x: -400.0,
                delta_y: 0.0,
            });
            settle(&platform_window);
        }
        let scrolled = ui.get_detail_people_scroll_x();
        assert!(
            scrolled < -1.0,
            "precondition: the cast shelf must be scrolled before the resize, was {scrolled}"
        );

        // Widen past the full content width: the cast now fits inside the
        // shelf's 1180px viewport cap, so the only valid offset is 0.
        platform_window.set_size(PhysicalSize::new(4000, 800));
        settle(&platform_window);

        assert!(
            ui.get_detail_people_scroll_x().abs() < 1.0,
            "a viewport wider than the shelf must reset its offset; it stayed at {}",
            ui.get_detail_people_scroll_x()
        );
    }

    /// Reproduces the reported failure: the app starts on half a screen, is
    /// moved to a monitor where it fills the workspace, and the wheel stops
    /// scrolling the home page.
    #[test]
    fn home_still_scrolls_after_the_window_is_enlarged() {
        let platform_window = setup_test_window();
        // Half of a 2560x1440 workspace, roughly what the app starts on.
        platform_window.set_size(PhysicalSize::new(1280, 1400));
        let ui = JellyBridgeWindow::new().expect("create window");
        show_home(&ui, &platform_window);

        // Scrolling works at the starting size.
        let scrolled_small = scroll_home(&ui, &platform_window);
        assert!(
            scrolled_small < -1.0,
            "precondition: the home page must scroll at the starting size, was {scrolled_small}"
        );

        // Move to the big monitor: the window now fills a 2560x1440 workspace.
        platform_window.set_size(PhysicalSize::new(2560, 1440));
        settle(&platform_window);

        let before = ui.get_catalog_home_scroll_y();
        let scrolled_large = scroll_home(&ui, &platform_window);
        assert!(
            scrolled_large < before - 1.0,
            "the home page must still scroll after the window is enlarged; \
             offset went {before} -> {scrolled_large}"
        );
    }

    /// Builds a home page of ten six-card shelves and shows it.
    fn show_home(ui: &JellyBridgeWindow, platform_window: &Rc<MinimalSoftwareWindow>) {
        use slint::{ModelRc, VecModel};
        ui.set_screen(Screen::Catalog);
        ui.set_catalog_home(true);
        ui.set_tile_target_width(176.0);
        let items = (0..100)
            .map(|i| super::CatalogItem {
                id: format!("i{i}").into(),
                title: format!("Title {i}").into(),
                subtitle: "2024".into(),
                kind_label: "Movie".into(),
                kind: "Movie".into(),
                placeholder: "T".into(),
                artwork: Default::default(),
                watched: false,
                progress: 0.0,
                is_container: false,
                playable: true,
            })
            .collect::<Vec<_>>();
        let sections = (0..10)
            .map(|s| super::CatalogSection {
                id: format!("s{s}").into(),
                title: format!("Section {s}").into(),
                start_index: s * 10,
                count: 10,
                wide: false,
            })
            .collect::<Vec<_>>();
        ui.set_catalog_items(ModelRc::new(VecModel::from(items)));
        ui.set_sections(ModelRc::new(VecModel::from(sections)));
        ui.show().expect("show");
        settle(platform_window);
    }

    /// The reported failure. A shelf locks the gesture axis to horizontal while
    /// the window is narrow enough for its cards to overflow. Moving the window
    /// to an empty workspace retiles it full-width, so the shelf now fits and
    /// can no longer scroll horizontally -- but the stale lock keeps swallowing
    /// vertical wheel events, and because every event restarts the 200ms
    /// release timer, scrolling without pausing keeps the lock alive forever.
    /// The page comes back only once the user stops scrolling long enough for
    /// the timer to fire, which is why it "fixed itself after a while".
    ///
    /// No sleeping here: the wheel ticks arrive back to back exactly as they do
    /// from a real wheel, so the release timer never gets its 200ms.
    #[test]
    fn home_still_scrolls_when_a_stale_shelf_lock_survives_widening() {
        let platform_window = setup_test_window();
        // Tiled to half of the 3840x2160 workspace (scale 1.5 => 2560x1440
        // logical), so a six-card shelf overflows its viewport.
        platform_window.set_size(PhysicalSize::new(1280, 1440));
        let ui = JellyBridgeWindow::new().expect("create window");
        show_home(&ui, &platform_window);

        // Scroll a shelf sideways: the axis locks to horizontal.
        let position = LogicalPosition::new(640.0, 400.0);
        ui.window()
            .dispatch_event(WindowEvent::PointerMoved { position });
        platform_window.draw_if_needed(|_| {});
        ui.window().dispatch_event(WindowEvent::PointerScrolled {
            position,
            delta_x: -120.0,
            delta_y: -8.0,
        });
        platform_window.draw_if_needed(|_| {});

        // Moved to the empty workspace: full width, every shelf now fits.
        platform_window.set_size(PhysicalSize::new(2560, 1440));
        platform_window.draw_if_needed(|_| {});

        // Keep scrolling vertically, back to back, as a frustrated user would.
        let before = ui.get_catalog_home_scroll_y();
        let position = LogicalPosition::new(1280.0, 400.0);
        ui.window()
            .dispatch_event(WindowEvent::PointerMoved { position });
        platform_window.draw_if_needed(|_| {});
        for _ in 0..8 {
            ui.window().dispatch_event(WindowEvent::PointerScrolled {
                position,
                delta_x: 0.0,
                delta_y: -240.0,
            });
            platform_window.draw_if_needed(|_| {});
        }

        assert!(
            ui.get_catalog_home_scroll_y() < before - 1.0,
            "a shelf that no longer overflows must not swallow the page's \
             vertical scrolling; offset stayed at {}",
            ui.get_catalog_home_scroll_y()
        );
    }

    /// Every point of a full-width home page must scroll it. The wheel used to
    /// be dropped wherever the enclosing ListView did not handle it -- section
    /// titles, gaps, side margins, and the empty space right of the last card,
    /// which is ~2000px wide on a two-card row once the window fills the
    /// workspace. That is why the page went unscrollable only at full size.
    #[test]
    fn home_scrolls_from_every_point_at_full_width() {
        use slint::{ModelRc, VecModel};
        let platform_window = setup_test_window();
        platform_window.set_size(PhysicalSize::new(2560, 1440));
        let ui = JellyBridgeWindow::new().expect("create window");
        ui.set_screen(Screen::Catalog);
        ui.set_catalog_home(true);
        ui.set_tile_target_width(176.0);
        // Realistic home: a 2-card "Libraries" row first, then full rows.
        let items = (0..62)
            .map(|i| super::CatalogItem {
                id: format!("i{i}").into(),
                title: format!("Title {i}").into(),
                subtitle: "2024".into(),
                kind_label: "Movie".into(),
                kind: "Movie".into(),
                placeholder: "T".into(),
                artwork: Default::default(),
                watched: false,
                progress: 0.0,
                is_container: false,
                playable: true,
            })
            .collect::<Vec<_>>();
        let mut sections = vec![super::CatalogSection {
            id: "lib".into(),
            title: "Libraries".into(),
            start_index: 0,
            count: 2,
            wide: false,
        }];
        for s in 0..10 {
            sections.push(super::CatalogSection {
                id: format!("s{s}").into(),
                title: format!("Section {s}").into(),
                start_index: 2 + s * 6,
                count: 6,
                wide: false,
            });
        }
        ui.set_catalog_items(ModelRc::new(VecModel::from(items)));
        ui.set_sections(ModelRc::new(VecModel::from(sections)));
        ui.show().expect("show");
        settle(&platform_window);

        // Park the page mid-scroll so both directions are available.
        let start = LogicalPosition::new(1280.0, 180.0);
        ui.window()
            .dispatch_event(WindowEvent::PointerMoved { position: start });
        platform_window.draw_if_needed(|_| {});
        for _ in 0..4 {
            ui.window().dispatch_event(WindowEvent::PointerScrolled {
                position: start,
                delta_x: 0.0,
                delta_y: -240.0,
            });
            settle(&platform_window);
        }
        assert!(
            ui.get_catalog_home_scroll_y() < -1.0,
            "precondition: the home page must be parked mid-scroll"
        );

        let mut dead = Vec::new();
        // Sample the title strip, the card row, the gap below it, and the
        // margins/empty space either side of a short row.
        for y in [170.0, 230.0, 400.0, 560.0, 700.0] {
            for x in [12.0, 600.0, 2200.0, 2550.0] {
                let position = LogicalPosition::new(x, y);
                ui.window()
                    .dispatch_event(WindowEvent::PointerMoved { position });
                platform_window.draw_if_needed(|_| {});
                let before = ui.get_catalog_home_scroll_y();
                ui.window().dispatch_event(WindowEvent::PointerScrolled {
                    position,
                    delta_x: 0.0,
                    delta_y: -240.0,
                });
                settle(&platform_window);
                let moved = ui.get_catalog_home_scroll_y() < before - 0.5;
                // Undo, so the page cannot creep to its end and read DEAD
                // simply because there is nothing left to scroll.
                ui.window().dispatch_event(WindowEvent::PointerScrolled {
                    position,
                    delta_x: 0.0,
                    delta_y: 240.0,
                });
                settle(&platform_window);
                if !moved {
                    dead.push(format!("({x}, {y})"));
                }
            }
        }
        assert!(
            dead.is_empty(),
            "the wheel must scroll the home page everywhere; dead at: {}",
            dead.join(", ")
        );
    }

    /// The detail page hand-rolls its wheel handling, so it gets the same
    /// enlarge-the-window check as home.
    #[test]
    fn detail_still_scrolls_after_the_window_is_enlarged() {
        use slint::{ModelRc, VecModel};
        let platform_window = setup_test_window();
        platform_window.set_size(PhysicalSize::new(1280, 1400));
        let ui = JellyBridgeWindow::new().expect("create window");
        ui.set_screen(Screen::Detail);
        ui.set_detail_kind(super::DetailKind::Season);
        ui.set_detail_episodes(ModelRc::new(VecModel::from(season_episodes(40))));
        ui.show().expect("show");
        settle(&platform_window);

        let position = LogicalPosition::new(640.0, 700.0);
        ui.window()
            .dispatch_event(WindowEvent::PointerMoved { position });
        platform_window.draw_if_needed(|_| {});
        ui.window().dispatch_event(WindowEvent::PointerScrolled {
            position,
            delta_x: 0.0,
            delta_y: -240.0,
        });
        settle(&platform_window);
        assert!(
            ui.get_detail_page_scroll_y() < -1.0,
            "precondition: the detail page must scroll at the starting size"
        );

        // Fill a 2560x1440 workspace. 40 episodes still overflow it.
        platform_window.set_size(PhysicalSize::new(2560, 1440));
        settle(&platform_window);
        assert!(
            ui.get_detail_page_height() > 1440.0,
            "precondition: the page must still overflow the enlarged viewport, was {}",
            ui.get_detail_page_height()
        );

        let before = ui.get_detail_page_scroll_y();
        let position = LogicalPosition::new(1280.0, 700.0);
        ui.window()
            .dispatch_event(WindowEvent::PointerMoved { position });
        platform_window.draw_if_needed(|_| {});
        for _ in 0..3 {
            ui.window().dispatch_event(WindowEvent::PointerScrolled {
                position,
                delta_x: 0.0,
                delta_y: -240.0,
            });
            settle(&platform_window);
        }
        assert!(
            ui.get_detail_page_scroll_y() < before - 1.0,
            "the detail page must still scroll after the window is enlarged; \
             offset went {before} -> {}",
            ui.get_detail_page_scroll_y()
        );
    }

    /// Sends a few wheel ticks over the middle of the home page and returns the
    /// resulting scroll offset.
    fn scroll_home(ui: &JellyBridgeWindow, platform_window: &Rc<MinimalSoftwareWindow>) -> f32 {
        let width = ui.get_video_logical_width();
        let position = LogicalPosition::new(width / 2.0, 500.0);
        ui.window()
            .dispatch_event(WindowEvent::PointerMoved { position });
        platform_window.draw_if_needed(|_| {});
        for _ in 0..3 {
            ui.window().dispatch_event(WindowEvent::PointerScrolled {
                position,
                delta_x: 0.0,
                delta_y: -240.0,
            });
            settle(platform_window);
        }
        ui.get_catalog_home_scroll_y()
    }

    /// What the timeline draws must be exactly played / total. The stock
    /// Slider that used to draw it mapped its value over a thumb-inset rail
    /// while the chapter marks and the click target mapped over the full
    /// width, so the three disagreed with each other.
    #[test]
    fn the_timeline_draws_the_played_fraction_of_the_duration() {
        let platform_window = setup_test_window();
        let ui = JellyBridgeWindow::new().expect("create player UI");
        ui.set_screen(Screen::Player);
        ui.invoke_enter_player(0.0, 100.0);
        ui.show().expect("show");
        platform_window.draw_if_needed(|_| {});

        for (position, expected) in [(0.0, 0.0), (25.0, 0.25), (50.0, 0.5), (100.0, 1.0)] {
            ui.set_playback_position(position);
            slint::platform::update_timers_and_animations();
            platform_window.draw_if_needed(|_| {});
            let drawn = ui.get_timeline_played_ratio();
            assert!(
                (drawn - expected).abs() < 0.001,
                "at {position}s of 100s the timeline must be {expected} full, was {drawn}"
            );
        }
    }

    /// A duration of zero means "not known yet", not "already finished". The
    /// resume position used to be substituted for a missing runtime, which
    /// drew a completely full bar until mpv reported the real duration.
    #[test]
    fn an_unknown_duration_draws_an_empty_timeline_not_a_full_one() {
        let platform_window = setup_test_window();
        let ui = JellyBridgeWindow::new().expect("create player UI");
        ui.set_screen(Screen::Player);
        ui.invoke_enter_player(0.0, 0.0);
        ui.show().expect("show");
        platform_window.draw_if_needed(|_| {});

        ui.set_playback_position(90.0);
        slint::platform::update_timers_and_animations();
        platform_window.draw_if_needed(|_| {});
        assert_eq!(
            ui.get_timeline_played_ratio(),
            0.0,
            "an unknown duration must not draw a full bar"
        );

        // Once mpv reports the real duration the bar catches up.
        ui.set_playback_duration(180.0);
        slint::platform::update_timers_and_animations();
        platform_window.draw_if_needed(|_| {});
        assert!((ui.get_timeline_played_ratio() - 0.5).abs() < 0.001);
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
        // Pointer entry synthesizes a hover move, which also (deliberately)
        // counts as activity now, so the click yields at least one report.
        assert!(
            activity_count.get() >= 1,
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
            include_str!("../../ui/features/detail/episode-shelf.slint"),
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

        assert_eq!(progress_fill_count, 3, "update this contract for new fills");
    }
}
