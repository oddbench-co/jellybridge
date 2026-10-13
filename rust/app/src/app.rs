//! Application state and shared orchestration context. This module must not
//! know any UI type; the UI is reached only through [`crate::ui::UiHandle`].

use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};

use std::collections::HashMap;

use crate::api::JellyfinApi;
use crate::features::detail::{DetailModel, DetailPageData, DetailTarget};
use crate::features::now_playing::NowPlaying;
use crate::features::pages::{FilterOptions, GridSettings, LibraryRef, Page, PageRoute, Shelf};
use crate::features::progress::PlaybackReporter;
use crate::services::playback::PreparedPlayback;
use crate::services::player::{PlayerSender, PlayerTrack};
use crate::services::session::{SessionStore, SettingsStore};
use crate::ui::UiHandle;

/// A place the Back button returns to.
#[derive(Clone, Debug)]
pub enum HistoryEntry {
    /// A list page, with what it showed when it was left.
    Page(Page),
    /// An item's detail page.
    Detail(Box<DetailPageData>),
}

/// What Retry reloads.
#[derive(Clone)]
pub enum RetryAction {
    /// The current list page.
    Page,
    Detail(DetailTarget),
}

pub struct AppState {
    pub api: Option<JellyfinApi>,
    pub account_name: String,
    /// The home page's shelves as last loaded, so Home shows at once.
    pub home: Vec<Shelf>,
    /// The user's libraries, for the drawer.
    pub libraries: Vec<LibraryRef>,
    /// The list page on screen, or under the detail page when one is open.
    pub page: Page,
    pub history: Vec<HistoryEntry>,
    /// Each grid's sort, filters and view, by `PageRoute::settings_key`.
    pub grid_settings: HashMap<String, GridSettings>,
    /// The filter dialog's choices for the grid on screen, by settings key.
    pub filter_options: Option<(String, FilterOptions)>,
    pub detail: DetailModel,
    pub detail_tracks: DetailTrackSelection,
    pub retry: RetryAction,
    /// The item in the player and its neighbours/segments, while playing.
    pub now_playing: Option<NowPlaying>,
    /// What Play all / Shuffle queued, in play order.
    pub queue: Option<Vec<String>>,
    /// The user's jellyfin-web settings, from the server.
    pub prefs: crate::features::prefs::UserPrefs,
    /// This device's own settings (jellyfin-web's browser-only ones).
    pub device: HashMap<String, String>,
}

#[derive(Clone, Default)]
pub struct DetailTrackSelection {
    pub audio_ids: Vec<i32>,
    pub subtitle_ids: Vec<Option<i32>>,
    pub audio_index: i32,
    pub subtitle_index: i32,
}

#[derive(Default)]
pub struct PlayerTrackState {
    pub audio: Vec<PlayerTrack>,
    pub subtitles: Vec<PlayerTrack>,
    pub preference_key: Option<String>,
}

impl Default for AppState {
    fn default() -> Self {
        Self {
            api: None,
            account_name: String::new(),
            home: Vec::new(),
            libraries: Vec::new(),
            page: Page::loading(PageRoute::home()),
            history: Vec::new(),
            grid_settings: HashMap::new(),
            filter_options: None,
            detail: DetailModel::default(),
            detail_tracks: DetailTrackSelection::default(),
            retry: RetryAction::Page,
            now_playing: None,
            queue: None,
            prefs: Default::default(),
            device: HashMap::new(),
        }
    }
}

impl AppState {
    /// The grid settings for `route`, or the defaults for its kind.
    /// The grid settings for `route`: its view and filters from this
    /// app, its sort from the server where jellyfin-web keeps it.
    pub fn grid_settings_for(&self, route: &PageRoute) -> GridSettings {
        let mut settings = self
            .grid_settings
            .get(&route.settings_key())
            .cloned()
            .or_else(|| route.grid().map(GridSettings::default_for))
            .unwrap_or_else(|| GridSettings::default_for(crate::features::pages::GridKind::Folder));
        if let Some((sort, descending)) = route
            .server_sort_key()
            .and_then(|key| self.prefs.grid_sort(&key))
        {
            settings.sort = sort;
            settings.descending = descending;
        }
        settings
    }
}

#[derive(Clone)]
pub struct AppContext {
    pub ui: UiHandle,
    pub runtime: tokio::runtime::Handle,
    pub state: Arc<Mutex<AppState>>,
    pub generation: Arc<AtomicU64>,
    pub store: SessionStore,
    pub settings_store: SettingsStore,
    pub image_cache: PathBuf,
    pub image_limit: Arc<tokio::sync::Semaphore>,
    pub playback: Arc<Mutex<Option<PreparedPlayback>>>,
    /// Tracks what is playing so progress can be reported to the server.
    pub reporter: Arc<Mutex<PlaybackReporter>>,
    pub player_sender: PlayerSender,
    /// Goes true once mpv's render context exists. Playback waits on it before
    /// `loadfile`, so mpv has a video output when it picks a decode path.
    pub render_ready: tokio::sync::watch::Receiver<bool>,
    pub player_tracks: Arc<Mutex<PlayerTrackState>>,
    pub catalog_request: Arc<Mutex<Option<tokio::task::AbortHandle>>>,
    /// Wakes the background refresh loop so the visible screen re-reads
    /// watched state from the server. See [`crate::features::refresh`].
    pub refresh: Arc<tokio::sync::Notify>,
}

impl AppContext {
    pub fn next_generation(&self) -> u64 {
        self.generation.fetch_add(1, Ordering::AcqRel) + 1
    }

    pub fn current_generation(&self) -> u64 {
        self.generation.load(Ordering::Acquire)
    }

    pub fn is_current(&self, generation: u64) -> bool {
        self.generation.load(Ordering::Acquire) == generation
    }

    /// Advances the navigation generation only if it is still `expected`.
    /// A silent refresh uses this so it never invalidates a navigation the
    /// user started while the refresh was in flight.
    pub fn claim_generation(&self, expected: u64) -> Option<u64> {
        claim_generation(&self.generation, expected)
    }

    /// Asks the refresh loop to re-read the visible screen. Bursts coalesce.
    pub fn request_refresh(&self) {
        self.refresh.notify_one();
    }

    pub fn api(&self) -> Option<JellyfinApi> {
        self.state.lock().ok()?.api.clone()
    }
}

fn claim_generation(generation: &AtomicU64, expected: u64) -> Option<u64> {
    generation
        .compare_exchange(expected, expected + 1, Ordering::AcqRel, Ordering::Acquire)
        .ok()
        .map(|previous| previous + 1)
}

pub fn replace_catalog_request(context: &AppContext, request: tokio::task::AbortHandle) {
    if let Ok(mut active) = context.catalog_request.lock()
        && let Some(previous) = active.replace(request)
    {
        previous.abort();
    }
}

pub fn cancel_catalog_request(context: &AppContext) {
    if let Ok(mut active) = context.catalog_request.lock()
        && let Some(request) = active.take()
    {
        request.abort();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn claim_generation_only_advances_an_unchanged_generation() {
        let generation = AtomicU64::new(4);
        assert_eq!(claim_generation(&generation, 3), None);
        assert_eq!(generation.load(Ordering::Acquire), 4);
        assert_eq!(claim_generation(&generation, 4), Some(5));
        assert_eq!(generation.load(Ordering::Acquire), 5);
    }
}
