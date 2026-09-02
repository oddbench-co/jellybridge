use crate::api::{ApiError, CatalogItemData, CatalogSectionData, JellyfinApi};
use crate::app::{
    AppContext, PageSnapshot, RetryAction, cancel_catalog_request, replace_catalog_request,
};
use crate::features::detail::{DetailEffect, DetailKind, DetailMessage, DetailTarget};
use crate::ui;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum CatalogSort {
    #[default]
    NameAscending,
    NameDescending,
    DateAddedNewest,
    DateAddedOldest,
    ReleaseDateNewest,
    ReleaseDateOldest,
    RatingHighest,
}

impl CatalogSort {
    pub const ALL: [Self; 7] = [
        Self::NameAscending,
        Self::NameDescending,
        Self::DateAddedNewest,
        Self::DateAddedOldest,
        Self::ReleaseDateNewest,
        Self::ReleaseDateOldest,
        Self::RatingHighest,
    ];

    pub fn key(self) -> &'static str {
        match self {
            Self::NameAscending => "name_asc",
            Self::NameDescending => "name_desc",
            Self::DateAddedNewest => "date_added_desc",
            Self::DateAddedOldest => "date_added_asc",
            Self::ReleaseDateNewest => "release_date_desc",
            Self::ReleaseDateOldest => "release_date_asc",
            Self::RatingHighest => "rating_desc",
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            Self::NameAscending => "Name A-Z",
            Self::NameDescending => "Name Z-A",
            Self::DateAddedNewest => "Date added newest",
            Self::DateAddedOldest => "Date added oldest",
            Self::ReleaseDateNewest => "Release date newest",
            Self::ReleaseDateOldest => "Release date oldest",
            Self::RatingHighest => "Rating highest",
        }
    }

    pub fn query(self) -> (&'static str, &'static str) {
        match self {
            Self::NameAscending => ("SortName", "Ascending"),
            Self::NameDescending => ("SortName", "Descending"),
            Self::DateAddedNewest => ("DateCreated", "Descending"),
            Self::DateAddedOldest => ("DateCreated", "Ascending"),
            Self::ReleaseDateNewest => ("PremiereDate", "Descending"),
            Self::ReleaseDateOldest => ("PremiereDate", "Ascending"),
            Self::RatingHighest => ("CommunityRating,SortName", "Descending,Ascending"),
        }
    }

    pub fn from_key(value: Option<&str>) -> Self {
        value
            .and_then(|value| Self::ALL.into_iter().find(|sort| sort.key() == value))
            .unwrap_or_default()
    }

    pub fn from_index(index: i32) -> Option<Self> {
        usize::try_from(index)
            .ok()
            .and_then(|index| Self::ALL.get(index).copied())
    }

    pub fn index(self) -> i32 {
        Self::ALL
            .iter()
            .position(|candidate| *candidate == self)
            .and_then(|index| i32::try_from(index).ok())
            .unwrap_or(0)
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum CatalogDensity {
    Compact,
    #[default]
    Normal,
    Large,
}

impl CatalogDensity {
    pub const ALL: [Self; 3] = [Self::Compact, Self::Normal, Self::Large];

    pub fn card_width(self) -> i32 {
        match self {
            Self::Compact => 220,
            Self::Normal => 260,
            Self::Large => 320,
        }
    }

    /// The pixel width to request artwork at for this density.
    ///
    /// Cards were always fetched at a hardcoded 320 px while being drawn at
    /// `card_width * scale_factor` physical pixels -- 390 px for Normal
    /// density on a 1.5x display, so every poster was upscaled 22% past its
    /// own resolution. Rounding up to a multiple of 40 keeps the number of
    /// distinct sizes in the disk cache small without wasting much.
    pub fn artwork_width(self, scale_factor: f32) -> u32 {
        let needed = (self.card_width() as f32 * scale_factor.max(1.0)).ceil() as u32;
        let rounded = needed.div_ceil(40) * 40;
        rounded.clamp(160, 1200)
    }

    pub fn from_card_width(value: Option<i32>) -> Self {
        let value = value.unwrap_or(Self::Normal.card_width());
        Self::ALL
            .into_iter()
            .min_by_key(|density| (density.card_width() - value).abs())
            .unwrap_or_default()
    }

    pub fn from_index(index: i32) -> Option<Self> {
        usize::try_from(index)
            .ok()
            .and_then(|index| Self::ALL.get(index).copied())
    }

    pub fn index(self) -> i32 {
        Self::ALL
            .iter()
            .position(|candidate| *candidate == self)
            .and_then(|index| i32::try_from(index).ok())
            .unwrap_or(1)
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CatalogSectionRange {
    pub id: String,
    pub title: String,
    pub start_index: usize,
    pub count: usize,
}

#[derive(Clone, Debug)]
pub struct FlattenedCatalog {
    pub items: Vec<CatalogItemData>,
    pub sections: Vec<CatalogSectionRange>,
}

pub fn flatten_sections(sections: &[CatalogSectionData]) -> FlattenedCatalog {
    let mut items = Vec::new();
    let mut ranges = Vec::with_capacity(sections.len());
    for section in sections {
        if section.items.is_empty() {
            continue;
        }
        let start_index = items.len();
        items.extend(section.items.iter().cloned());
        ranges.push(CatalogSectionRange {
            id: section.id.clone(),
            title: section.title.clone(),
            start_index,
            count: section.items.len(),
        });
    }
    FlattenedCatalog {
        items,
        sections: ranges,
    }
}

pub enum EmptySearchDestination {
    Ignore,
    Home,
    Parent(CatalogItemData),
}

pub fn empty_search_destination(current: &PageSnapshot) -> EmptySearchDestination {
    if current.search_text.is_empty() {
        EmptySearchDestination::Ignore
    } else if let Some(parent) = current.parent.clone() {
        EmptySearchDestination::Parent(parent)
    } else {
        EmptySearchDestination::Home
    }
}

pub fn start_home_load(context: &AppContext) {
    let Some(api) = context.api() else {
        ui::show_login(context, "Please sign in to continue");
        return;
    };
    let generation = context.next_generation();
    ui::set_screen_busy(context, ui::Screen::Catalog, true, "");
    if let Ok(mut state) = context.state.lock() {
        state.retry = RetryAction::Home;
    }
    let ctx = context.clone();
    let task = context.runtime.spawn(async move {
        let result = api.home().await;
        if !ctx.is_current(generation) {
            return;
        }
        apply_home_result(&ctx, api, result);
    });
    replace_catalog_request(context, task.abort_handle());
}

/// Installs a finished dashboard request, or reports why it failed.
///
/// Shared by the launch prefetch in [`crate::features::auth`] and by every
/// later reload, so a session that expired between launches is handled the
/// same way whichever path fetched it. The caller has already checked the
/// navigation generation.
pub fn apply_home_result(
    context: &AppContext,
    api: JellyfinApi,
    result: Result<Vec<CatalogSectionData>, ApiError>,
) {
    match result {
        Ok(home) => {
            let account = context
                .state
                .lock()
                .map(|state| state.account_name.clone())
                .unwrap_or_default();
            install_home(context, api, account, home);
        }
        Err(ApiError::Unauthorized) => {
            let _ = context.store.clear();
            if let Ok(mut state) = context.state.lock() {
                crate::features::auth::reset_session_state(&mut state);
            }
            ui::show_login(
                context,
                "Your saved session is no longer valid. Please sign in again.",
            );
        }
        Err(error) => ui::show_catalog_error(context, error.to_string()),
    }
}

pub fn install_home(
    context: &AppContext,
    api: JellyfinApi,
    account_name: String,
    home: Vec<CatalogSectionData>,
) {
    let current = PageSnapshot::home(&home);
    if let Ok(mut state) = context.state.lock() {
        state.api = Some(api);
        state.account_name = account_name;
        state.home = home;
        state.current = current;
        state.back_stack.clear();
        state.detail.update(DetailMessage::Reset);
        state.retry = RetryAction::Home;
    }
    ui::catalog::present(context, "");
}

pub fn show_home(context: &AppContext, clear_history: bool) {
    context.next_generation();
    cancel_catalog_request(context);
    if let Ok(mut state) = context.state.lock() {
        state.current = PageSnapshot::home(&state.home);
        if clear_history {
            state.back_stack.clear();
        }
        state.detail.update(DetailMessage::Reset);
    }
    ui::catalog::present(context, "");
}

pub fn open_item(context: &AppContext, id: &str) {
    let item = context.state.lock().ok().and_then(|state| {
        state
            .current
            .items
            .iter()
            .find(|item| item.id == id)
            .cloned()
    });
    let Some(item) = item else { return };
    let detail_kind = DetailKind::from_jellyfin(&item.kind);
    if !matches!(detail_kind, DetailKind::Other) {
        crate::features::detail::start_detail(context, DetailTarget::new(item.id, detail_kind));
    } else if item.is_container {
        start_browse(context, item);
    }
}

pub fn start_browse(context: &AppContext, item: CatalogItemData) {
    start_browse_request(context, item, true);
}

pub fn start_browse_request(context: &AppContext, item: CatalogItemData, push_history: bool) {
    let Some(api) = context.api() else { return };
    let generation = context.next_generation();
    let sort = context
        .state
        .lock()
        .map(|state| state.catalog_sort)
        .unwrap_or_default();
    if let Ok(mut state) = context.state.lock() {
        state.retry = RetryAction::Browse(item.clone(), push_history);
    }
    ui::set_screen_busy(context, ui::Screen::Catalog, true, "");
    let ctx = context.clone();
    let task = context.runtime.spawn(async move {
        let result = api.browse(&item, sort).await;
        if !ctx.is_current(generation) {
            return;
        }
        match result {
            Ok(page) => {
                if let Ok(mut state) = ctx.state.lock() {
                    if push_history {
                        let previous = state.current.clone();
                        state.back_stack.push(previous);
                    }
                    state.current = PageSnapshot {
                        title: item.title.clone(),
                        items: page.items,
                        sections: Vec::new(),
                        parent: Some(item.clone()),
                        search_text: String::new(),
                        total_record_count: page.total_record_count,
                    };
                    state.detail.update(DetailMessage::Reset);
                }
                ui::catalog::present(&ctx, "");
            }
            Err(error) => ui::show_catalog_error(&ctx, error.to_string()),
        }
    });
    replace_catalog_request(context, task.abort_handle());
}

pub fn start_search(context: &AppContext, term: String) {
    let term = term.trim().to_owned();
    if term.is_empty() {
        let destination = context
            .state
            .lock()
            .map(|state| empty_search_destination(&state.current))
            .unwrap_or(EmptySearchDestination::Ignore);
        match destination {
            EmptySearchDestination::Ignore => {}
            EmptySearchDestination::Home => show_home(context, false),
            EmptySearchDestination::Parent(parent) => {
                start_browse_request(context, parent, false);
            }
        }
        return;
    }
    let Some(api) = context.api() else { return };
    let generation = context.next_generation();
    let (parent, sort) = context
        .state
        .lock()
        .map(|state| (state.current.parent.clone(), state.catalog_sort))
        .unwrap_or((None, CatalogSort::default()));
    if let Ok(mut state) = context.state.lock() {
        state.retry = RetryAction::Search(term.clone());
        state.current.search_text.clone_from(&term);
    }
    ui::set_screen_busy(context, ui::Screen::Catalog, true, "");
    let ctx = context.clone();
    let task = context.runtime.spawn(async move {
        let result = api
            .search(&term, parent.as_ref().map(|item| item.id.as_str()), sort)
            .await;
        if !ctx.is_current(generation) {
            return;
        }
        match result {
            Ok(page) => {
                if let Ok(mut state) = ctx.state.lock() {
                    state.current.items = page.items;
                    state.current.sections.clear();
                    state.current.search_text.clone_from(&term);
                    state.current.total_record_count = page.total_record_count;
                    state.detail.update(DetailMessage::Reset);
                }
                ui::catalog::present(&ctx, "");
            }
            Err(error) => ui::show_catalog_error(&ctx, error.to_string()),
        }
    });
    replace_catalog_request(context, task.abort_handle());
}

pub fn change_catalog_sort(context: &AppContext, index: i32) {
    let Some(sort) = CatalogSort::from_index(index) else {
        return;
    };
    let (search_text, parent, is_home_dashboard) = match context.state.lock() {
        Ok(mut state) => {
            state.catalog_sort = sort;
            (
                state.current.search_text.clone(),
                state.current.parent.clone(),
                state.current.is_home_dashboard(),
            )
        }
        Err(_) => return,
    };
    persist_catalog_preferences(context);
    if is_home_dashboard {
        ui::catalog::present_preferences(context);
    } else if !search_text.is_empty() {
        start_search(context, search_text);
    } else if let Some(parent) = parent {
        start_browse_request(context, parent, false);
    }
}

pub fn change_catalog_density(context: &AppContext, index: i32) {
    let Some(density) = CatalogDensity::from_index(index) else {
        return;
    };
    if let Ok(mut state) = context.state.lock() {
        state.catalog_density = density;
    }
    persist_catalog_preferences(context);
    ui::catalog::present_preferences(context);
}

fn persist_catalog_preferences(context: &AppContext) {
    let (density, sort) = match context.state.lock() {
        Ok(state) => (state.catalog_density, state.catalog_sort),
        Err(_) => return,
    };
    // The settings store does blocking file I/O, so persist off the UI thread.
    let ctx = context.clone();
    context.runtime.spawn(async move {
        let store = ctx.settings_store.clone();
        let result = tokio::task::spawn_blocking(move || {
            store.save_catalog_preferences(density.card_width(), sort.key())
        })
        .await;
        if !matches!(result, Ok(Ok(()))) {
            tracing::warn!("catalog preferences could not be saved");
            ui::set_message(&ctx, "Catalog preferences could not be saved");
        }
    });
}

pub fn go_back(context: &AppContext) {
    context.next_generation();
    cancel_catalog_request(context);
    let detail_effects = context.state.lock().ok().and_then(|mut state| {
        let snapshot = state.detail.snapshot();
        (snapshot.page.is_some() || snapshot.busy || snapshot.error.is_some())
            .then(|| state.detail.update(DetailMessage::Back))
    });
    if let Some(effects) = detail_effects {
        if effects
            .iter()
            .any(|effect| matches!(effect, DetailEffect::ShowCatalog))
        {
            ui::catalog::present(context, "");
        } else {
            let generation = context.current_generation();
            ui::detail::present(context, generation);
        }
        return;
    }
    let mut show_catalog = false;
    if let Ok(mut state) = context.state.lock()
        && let Some(previous) = state.back_stack.pop()
    {
        state.current = previous;
        show_catalog = true;
    }
    if show_catalog {
        ui::catalog::present(context, "");
    }
}

pub fn retry(context: &AppContext) {
    let action = context
        .state
        .lock()
        .map(|state| state.retry.clone())
        .unwrap_or(RetryAction::Home);
    match action {
        RetryAction::Home => start_home_load(context),
        RetryAction::Browse(item, push_history) => {
            start_browse_request(context, item, push_history);
        }
        RetryAction::Search(term) => start_search(context, term),
        RetryAction::Detail(target) => crate::features::detail::start_detail(context, target),
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn artwork_is_requested_at_the_size_it_will_be_drawn() {
        // A card is card_width * scale_factor physical pixels. Asking for 320
        // regardless, as this used to, upscaled every poster on any display
        // above 1x.
        assert!(
            CatalogDensity::Normal.artwork_width(1.5) >= (260.0 * 1.5) as u32,
            "Normal density on a 1.5x display still under-requests"
        );
        assert!(
            CatalogDensity::Large.artwork_width(2.0) >= (320.0 * 2.0) as u32,
            "Large density on a 2x display still under-requests"
        );

        // Rounded, so a handful of sizes cover every density and scale rather
        // than one cache entry per pixel width.
        for density in CatalogDensity::ALL {
            for scale in [1.0, 1.25, 1.5, 2.0] {
                let width = density.artwork_width(scale);
                assert_eq!(width % 40, 0, "{width} is not a rounded size");
                assert!((160..=1200).contains(&width), "{width} is out of range");
            }
        }

        // A scale factor below 1 must never shrink the request below the card.
        assert!(CatalogDensity::Normal.artwork_width(0.5) >= 260);
    }

    use super::*;

    fn item(id: &str) -> CatalogItemData {
        CatalogItemData {
            id: id.into(),
            title: id.into(),
            subtitle: String::new(),
            kind: "Movie".into(),
            image_item_id: id.into(),
            image_tag: None,
            watched: false,
            progress: 0.0,
            is_container: false,
            playable: true,
        }
    }

    #[test]
    fn flattening_preserves_section_ranges_and_duplicate_items() {
        let sections = vec![
            CatalogSectionData {
                id: "one".into(),
                title: "One".into(),
                items: vec![item("a"), item("shared")],
            },
            CatalogSectionData {
                id: "empty".into(),
                title: "Empty".into(),
                items: Vec::new(),
            },
            CatalogSectionData {
                id: "two".into(),
                title: "Two".into(),
                items: vec![item("shared"), item("b")],
            },
        ];

        let flattened = flatten_sections(&sections);
        assert_eq!(
            flattened
                .items
                .iter()
                .map(|item| item.id.as_str())
                .collect::<Vec<_>>(),
            ["a", "shared", "shared", "b"]
        );
        assert_eq!(
            flattened.sections,
            [
                CatalogSectionRange {
                    id: "one".into(),
                    title: "One".into(),
                    start_index: 0,
                    count: 2,
                },
                CatalogSectionRange {
                    id: "two".into(),
                    title: "Two".into(),
                    start_index: 2,
                    count: 2,
                },
            ]
        );
    }

    #[test]
    fn catalog_preferences_use_kotlin_compatible_values() {
        assert_eq!(
            CatalogDensity::from_card_width(Some(220)),
            CatalogDensity::Compact
        );
        assert_eq!(
            CatalogDensity::from_card_width(Some(360)),
            CatalogDensity::Large
        );
        assert_eq!(CatalogDensity::default().card_width(), 260);
        assert_eq!(
            CatalogSort::from_key(Some("rating_desc")),
            CatalogSort::RatingHighest
        );
        assert_eq!(
            CatalogSort::from_key(Some("runtime_asc")),
            CatalogSort::NameAscending
        );
        assert_eq!(
            CatalogSort::RatingHighest.query(),
            ("CommunityRating,SortName", "Descending,Ascending")
        );
    }

    #[test]
    fn programmatic_empty_search_does_not_interrupt_initial_home_load() {
        let initial_home = PageSnapshot::empty_home();

        assert!(matches!(
            empty_search_destination(&initial_home),
            EmptySearchDestination::Ignore
        ));
    }

    #[test]
    fn clearing_an_active_search_returns_to_home() {
        let mut search_results = PageSnapshot::empty_home();
        search_results.search_text = "movie".into();

        assert!(matches!(
            empty_search_destination(&search_results),
            EmptySearchDestination::Home
        ));
    }
}
