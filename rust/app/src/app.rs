//! Application state and shared orchestration context. This module must not
//! know any Slint type; the UI is reached only through [`crate::ui::UiHandle`].

use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};

use crate::api::{CatalogItemData, CatalogSectionData, JellyfinApi};
use crate::features::catalog::{CatalogDensity, CatalogSort, CatalogSectionRange, flatten_sections};
use crate::features::detail::{DetailModel, DetailTarget};
use crate::features::now_playing::NowPlaying;
use crate::features::progress::PlaybackReporter;
use crate::services::playback::PreparedPlayback;
use crate::services::player::{PlayerCommand, PlayerTrack};
use crate::services::session::{SessionStore, SettingsStore};
use crate::ui::UiHandle;

#[derive(Clone)]
pub struct PageSnapshot {
    pub title: String,
    pub items: Vec<CatalogItemData>,
    pub sections: Vec<CatalogSectionRange>,
    pub parent: Option<CatalogItemData>,
    pub search_text: String,
    pub total_record_count: usize,
}

impl PageSnapshot {
    pub fn empty_home() -> Self {
        Self {
            title: "Home".into(),
            items: Vec::new(),
            sections: Vec::new(),
            parent: None,
            search_text: String::new(),
            total_record_count: 0,
        }
    }

    pub fn home(sections: &[CatalogSectionData]) -> Self {
        let flattened = flatten_sections(sections);
        let total_record_count = flattened.items.len();
        Self {
            title: "Home".into(),
            items: flattened.items,
            sections: flattened.sections,
            parent: None,
            search_text: String::new(),
            total_record_count,
        }
    }

    pub fn is_home_dashboard(&self) -> bool {
        self.parent.is_none() && self.search_text.is_empty()
    }
}

#[derive(Clone)]
pub enum RetryAction {
    Home,
    Browse(CatalogItemData, bool),
    Search(String),
    Detail(DetailTarget),
}

pub struct AppState {
    pub api: Option<JellyfinApi>,
    pub account_name: String,
    pub home: Vec<CatalogSectionData>,
    pub current: PageSnapshot,
    pub back_stack: Vec<PageSnapshot>,
    pub detail: DetailModel,
    pub detail_tracks: DetailTrackSelection,
    pub retry: RetryAction,
    pub catalog_sort: CatalogSort,
    pub catalog_density: CatalogDensity,
    /// The item in the player and its neighbours/segments, while playing.
    pub now_playing: Option<NowPlaying>,
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
            current: PageSnapshot::empty_home(),
            back_stack: Vec::new(),
            detail: DetailModel::default(),
            detail_tracks: DetailTrackSelection::default(),
            retry: RetryAction::Home,
            catalog_sort: CatalogSort::default(),
            catalog_density: CatalogDensity::default(),
            now_playing: None,
        }
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
    pub player_sender: std::sync::mpsc::Sender<PlayerCommand>,
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
