//! Navigation between jellyfin-web's pages, and what the cards on them do.
//!
//! There is one history for every page: list pages ([`Page`]) and item
//! detail pages alike. Opening anything pushes the place being left, Back
//! pops it, and Home clears it. A page is kept with what it showed, so Back
//! is instant; the background refresh then re-reads it.

use crate::api::{ApiError, CatalogItemData, HomeData, JellyfinApi, library_route};
use crate::app::{
    AppContext, AppState, HistoryEntry, RetryAction, cancel_catalog_request,
    replace_catalog_request,
};
use crate::features::detail::{DetailKind, DetailMessage, DetailTarget};
use crate::features::pages::{
    GridSettings, LibraryRef, ListFilter, ListQuery, Page, PageContent, PageRoute, Shelf,
};
use crate::ui;

/// Installs a finished dashboard request, or reports why it failed.
///
/// Shared by the launch prefetch in [`crate::features::auth`] and by every
/// later reload, so a session that expired between launches is handled the
/// same way whichever path fetched it. The caller has already checked the
/// navigation generation.
pub fn apply_home_result(
    context: &AppContext,
    api: JellyfinApi,
    result: Result<HomeData, ApiError>,
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
        // One refused request does not prove the sign-in is gone (a server
        // restart or a lock can answer 401 once): the session is dropped only
        // when the server also refuses to say who it belongs to.
        Err(ApiError::Unauthorized) => {
            let ctx = context.clone();
            context.runtime.spawn(async move {
                match api.session_is_valid().await {
                    Ok(false) => {
                        let _ = ctx.store.clear();
                        if let Ok(mut state) = ctx.state.lock() {
                            crate::features::auth::reset_session_state(&mut state);
                        }
                        ui::show_login(
                            &ctx,
                            "Your saved session is no longer valid. Please sign in again.",
                        );
                    }
                    _ => apply_home_result(
                        &ctx,
                        api,
                        Err(ApiError::Status {
                            status: reqwest::StatusCode::UNAUTHORIZED,
                            operation: "loading the page",
                        }),
                    ),
                }
            });
        }
        Err(error) => {
            if let Ok(mut state) = context.state.lock() {
                state.api = Some(api);
                state.page = Page {
                    route: PageRoute::home(),
                    content: PageContent::Failed(error.to_string()),
                };
            }
            ui::catalog::present(context);
        }
    }
}

/// Shows a freshly signed-in or restored session's home page.
pub fn install_home(context: &AppContext, api: JellyfinApi, account_name: String, home: HomeData) {
    let shelves = crate::api::home_shelves(home.sections);
    if let Ok(mut state) = context.state.lock() {
        state.prefs = home.prefs;
        state.api = Some(api);
        state.account_name = account_name;
        state.libraries = libraries_of(&shelves);
        state.home = shelves.clone();
        state.page = Page {
            route: PageRoute::home(),
            content: PageContent::Shelves(shelves),
        };
        state.history.clear();
        state.detail.update(DetailMessage::Reset);
        state.retry = RetryAction::Page;
    }
    ui::catalog::present(context);
    load_deferred_home(context);
}

/// Fetches the home shelves left out of the first screen and slots them in.
fn load_deferred_home(context: &AppContext) {
    let Some(api) = context.api() else { return };
    let libraries = context
        .state
        .lock()
        .ok()
        .and_then(|state| {
            state
                .home
                .iter()
                .find(|shelf| shelf.id == "libraries")
                .map(|shelf| shelf.items.clone())
        })
        .unwrap_or_default();
    if !libraries
        .iter()
        .any(|library| library.collection_type == "music")
    {
        return;
    }
    let ctx = context.clone();
    context.runtime.spawn(async move {
        let sections = api.deferred_home_sections(&libraries).await;
        if sections.is_empty() {
            return;
        }
        if let Ok(mut state) = ctx.state.lock() {
            crate::api::merge_home_shelves(&mut state.home, sections.clone());
            if state.page.route == PageRoute::home()
                && let PageContent::Shelves(shelves) = &mut state.page.content
            {
                crate::api::merge_home_shelves(shelves, sections);
            }
        }
        ui::catalog::present_refresh(&ctx);
    });
}

/// The libraries listed in My Media, for the drawer.
pub fn libraries_of(home: &[Shelf]) -> Vec<LibraryRef> {
    home.iter()
        .find(|shelf| shelf.id == "libraries")
        .map(|shelf| {
            shelf
                .items
                .iter()
                .map(|item| LibraryRef {
                    id: item.id.clone(),
                    name: item.title.clone(),
                    collection_type: item.collection_type.clone(),
                })
                .collect()
        })
        .unwrap_or_default()
}

/// Moves whatever is on screen into the history.
fn push_location(state: &mut AppState) {
    if state.detail.is_active() {
        if let Some(page) = state.detail.take_page() {
            state.history.push(HistoryEntry::Detail(Box::new(page)));
        }
    } else {
        state.history.push(HistoryEntry::Page(state.page.clone()));
    }
}

/// Opens a list page, keeping the current place in the history.
pub fn navigate(context: &AppContext, route: PageRoute) {
    if let Ok(mut state) = context.state.lock() {
        let already_here = !state.detail.is_active() && state.page.route == route;
        if already_here {
            return;
        }
        push_location(&mut state);
    }
    load_route(context, route, 0);
}

/// Switches the header tab: the same page on another tab, without a new
/// history entry, as jellyfin-web's tabs behave.
pub fn switch_tab(context: &AppContext, index: usize) {
    let route = context
        .state
        .lock()
        .ok()
        .and_then(|state| state.page.route.with_tab(index));
    if let Some(route) = route {
        load_route(context, route, 0);
    }
}

/// Opens an item's detail page.
pub fn open_detail(context: &AppContext, target: DetailTarget) {
    if let Ok(mut state) = context.state.lock() {
        push_location(&mut state);
    }
    crate::features::detail::start_detail(context, target);
}

/// Loads `route` into the page on screen. Home shows its last shelves at
/// once and refreshes behind them.
fn load_route(context: &AppContext, route: PageRoute, start: usize) {
    let Some(api) = context.api() else {
        ui::show_login(context, "Please sign in to continue");
        return;
    };
    let generation = context.next_generation();
    cancel_catalog_request(context);
    let settings = {
        let Ok(mut state) = context.state.lock() else {
            return;
        };
        state.detail.update(DetailMessage::Reset);
        state.retry = RetryAction::Page;
        if route == PageRoute::home() && !state.home.is_empty() {
            state.page = Page {
                route,
                content: PageContent::Shelves(state.home.clone()),
            };
            drop(state);
            ui::catalog::present(context);
            context.request_refresh();
            return;
        }
        // A grid turning its page keeps its rows until the next arrive.
        let same_grid =
            state.page.route == route && matches!(state.page.content, PageContent::Grid(_));
        if !same_grid {
            state.page = Page::loading(route.clone());
        }
        state.grid_settings_for(&route)
    };
    ui::catalog::present(context);
    let ctx = context.clone();
    let task = context.runtime.spawn(async move {
        let result = api.load_page(&route, &settings, start).await;
        if !ctx.is_current(generation) {
            return;
        }
        let content = match result {
            Ok(content) => content,
            Err(ApiError::Unauthorized) => {
                apply_home_result(&ctx, api, Err(ApiError::Unauthorized));
                return;
            }
            Err(error) => PageContent::Failed(error.to_string()),
        };
        if let Ok(mut state) = ctx.state.lock() {
            if state.page.route != route {
                return;
            }
            if let (PageRoute::Home { favorites: false }, PageContent::Shelves(shelves)) =
                (&route, &content)
            {
                state.home = shelves.clone();
                state.libraries = libraries_of(shelves);
            }
            state.page.content = content;
        }
        ui::catalog::present(&ctx);
    });
    replace_catalog_request(context, task.abort_handle());
}

pub fn go_back(context: &AppContext) {
    context.next_generation();
    cancel_catalog_request(context);
    let entry = context.state.lock().ok().and_then(|mut state| {
        state.detail.update(DetailMessage::Reset);
        state.history.pop()
    });
    match entry {
        Some(HistoryEntry::Page(page)) => {
            if let Ok(mut state) = context.state.lock() {
                state.page = page;
                state.retry = RetryAction::Page;
            }
            ui::catalog::present(context);
        }
        Some(HistoryEntry::Detail(page)) => {
            if let Ok(mut state) = context.state.lock() {
                state.retry =
                    RetryAction::Detail(DetailTarget::new(page.item.id.clone(), page.item.kind));
                state.detail.update(DetailMessage::Restore(page));
            }
            let generation = context.current_generation();
            ui::detail::present(context, generation);
        }
        // Nothing to return to: the page under a closed detail, or home.
        None => {
            let on_home = context
                .state
                .lock()
                .is_ok_and(|state| state.page.route.is_home());
            if on_home {
                ui::catalog::present(context);
            } else {
                go_home(context);
                return;
            }
        }
    }
    // Restored pages are snapshots from when they were left.
    context.request_refresh();
}

pub fn go_home(context: &AppContext) {
    if let Ok(mut state) = context.state.lock() {
        state.history.clear();
    }
    load_route(context, PageRoute::home(), 0);
}

/// Shows another page of the grid on screen.
pub fn show_grid_page(context: &AppContext, start: usize) {
    let route = context
        .state
        .lock()
        .ok()
        .map(|state| state.page.route.clone());
    if let Some(route) = route {
        load_route(context, route, start);
    }
}

/// Applies new sort, filter or view settings to the grid on screen,
/// remembering them for that grid.
pub fn update_grid_settings(context: &AppContext, settings: GridSettings) {
    let Some((route, reload)) = context.state.lock().ok().map(
        |mut state| -> (PageRoute, (bool, Option<(String, String)>)) {
            let route = state.page.route.clone();
            let previous = state.grid_settings_for(&route);
            // The view only changes how rows are drawn; everything else changes
            // which rows there are.
            let sort_changed =
                previous.sort != settings.sort || previous.descending != settings.descending;
            let reload = sort_changed || previous.filters != settings.filters;
            // A library tab's sort is shared with jellyfin-web through the
            // server; save it there (and locally for the next read).
            let server_sort = sort_changed
                .then(|| route.server_sort_key())
                .flatten()
                .map(|key| {
                    let value = crate::features::prefs::UserPrefs::grid_sort_value(
                        &settings.sort,
                        settings.descending,
                    );
                    state.prefs.custom.insert(key.clone(), value.clone());
                    (key, value)
                });
            state.grid_settings.insert(route.settings_key(), settings);
            (route, (reload, server_sort))
        },
    ) else {
        return;
    };
    let (route, reload, server_sort) = (route, reload.0, reload.1);
    if let (Some((key, value)), Some(api)) = (server_sort, context.api()) {
        context.runtime.spawn(async move {
            if let Err(error) = api.save_custom_pref(&key, &value).await {
                tracing::warn!(%error, "sort could not be saved to the server");
            }
        });
    }
    persist_grid_settings(context);
    if reload {
        load_route(context, route, 0);
    } else {
        ui::catalog::present(context);
    }
}

/// Fetches what the filter dialog can offer for the grid on screen.
pub fn load_filter_options(context: &AppContext) {
    let Some(api) = context.api() else { return };
    let Some(route) = context
        .state
        .lock()
        .ok()
        .map(|state| state.page.route.clone())
    else {
        return;
    };
    let key = route.settings_key();
    let cached = context.state.lock().is_ok_and(|state| {
        state
            .filter_options
            .as_ref()
            .is_some_and(|(cached, _)| *cached == key)
    });
    if cached {
        return;
    }
    let ctx = context.clone();
    context.runtime.spawn(async move {
        match api.filter_options(&route).await {
            Ok(options) => {
                if let Ok(mut state) = ctx.state.lock() {
                    state.filter_options = Some((key, options));
                }
                ui::catalog::present_filter_options(&ctx);
            }
            Err(error) => tracing::warn!(%error, "filter options failed"),
        }
    });
}

fn persist_grid_settings(context: &AppContext) {
    let Ok(settings) = context.state.lock().map(|state| {
        state
            .grid_settings
            .iter()
            .map(|(key, settings)| (key.clone(), settings.to_stored()))
            .collect()
    }) else {
        return;
    };
    // The settings store does blocking file I/O, so persist off the UI thread.
    let store = context.settings_store.clone();
    context.runtime.spawn_blocking(move || {
        if let Err(error) = store.save_grid_settings(settings) {
            tracing::warn!(%error, "grid settings could not be saved");
        }
    });
}

/// Opens the search page, as the header's search button does.
pub fn open_search(context: &AppContext) {
    navigate(context, PageRoute::Search);
}

/// Runs a search on the search page; an empty term shows the suggestions.
pub fn search(context: &AppContext, term: String) {
    let Some(api) = context.api() else { return };
    let term = term.trim().to_owned();
    let generation = context.next_generation();
    let on_search = context.state.lock().is_ok_and(|mut state| {
        let PageContent::Search {
            term: shown,
            results,
            ..
        } = &mut state.page.content
        else {
            return false;
        };
        shown.clone_from(&term);
        if term.is_empty() {
            results.clear();
        }
        true
    });
    if !on_search {
        return;
    }
    ui::catalog::present(context);
    if term.is_empty() {
        return;
    }
    let ctx = context.clone();
    let task = context.runtime.spawn(async move {
        let result = api.search_results(&term).await;
        if !ctx.is_current(generation) {
            return;
        }
        let results = match result {
            Ok(results) => results,
            Err(error) => {
                tracing::warn!(%error, "search failed");
                ui::set_message(&ctx, &error.to_string());
                Vec::new()
            }
        };
        if let Ok(mut state) = ctx.state.lock()
            && let PageContent::Search {
                term: shown,
                results: shown_results,
                ..
            } = &mut state.page.content
            && *shown == term
        {
            *shown_results = results;
        }
        ui::catalog::present(&ctx);
    });
    replace_catalog_request(context, task.abort_handle());
}

/// A played/favorite toggle on a card.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CardToggle {
    Played(bool),
    Favorite(bool),
}

/// Flips a card's played or favorite state, as the buttons on a jellyfin-web
/// card do. The card changes at once; the server is told in the background
/// and the visible screen is refreshed from it afterwards, which also undoes
/// the change if the server refused it.
pub fn toggle_card(context: &AppContext, id: &str, toggle: CardToggle) {
    let Some(api) = context.api() else { return };
    if let Ok(mut state) = context.state.lock() {
        let apply = |item: &mut CatalogItemData| {
            if item.id == id {
                match toggle {
                    CardToggle::Played(played) => {
                        item.watched = played;
                        if played {
                            item.progress = 0.0;
                            item.unplayed_count = 0;
                        }
                    }
                    CardToggle::Favorite(favorite) => item.favorite = favorite,
                }
            }
        };
        state.page.items_mut().for_each(apply);
        state
            .home
            .iter_mut()
            .flat_map(|shelf| shelf.items.iter_mut())
            .for_each(apply);
    }
    ui::catalog::present(context);
    let ctx = context.clone();
    let id = id.to_owned();
    context.runtime.spawn(async move {
        let result = match toggle {
            CardToggle::Played(played) => api.set_watched(&id, played).await,
            CardToggle::Favorite(favorite) => api.set_favorite(&id, favorite).await,
        };
        if let Err(error) = result {
            tracing::warn!(%error, "card state change failed");
            ui::set_message(&ctx, "The change could not be saved");
        }
        ctx.request_refresh();
    });
}

/// What opening a card leads to, by the item's kind.
pub fn destination(item: &CatalogItemData) -> Destination {
    match item.kind.as_str() {
        "CollectionFolder" | "UserView" => Destination::Page(library_route(item)),
        "Movie" | "Series" | "Season" | "Episode" | "Person" | "BoxSet" | "Playlist" | "Video"
        | "MusicVideo" => Destination::Detail(DetailTarget::new(
            item.id.clone(),
            DetailKind::from_jellyfin(&item.kind),
        )),
        "Genre" => Destination::Page(PageRoute::List(ListQuery {
            title: item.title.clone(),
            parent_id: None,
            filter: ListFilter::Genre(item.id.clone()),
            item_types: String::new(),
        })),
        "Studio" => Destination::Page(PageRoute::List(ListQuery {
            title: item.title.clone(),
            parent_id: None,
            filter: ListFilter::Studio(item.id.clone()),
            item_types: "Series".to_owned(),
        })),
        _ => Destination::Page(PageRoute::List(ListQuery::children(
            item.title.clone(),
            item.id.clone(),
        ))),
    }
}

#[derive(Clone, Debug, PartialEq)]
pub enum Destination {
    Page(PageRoute),
    Detail(DetailTarget),
}

pub fn open_item(context: &AppContext, id: &str) {
    let item = context
        .state
        .lock()
        .ok()
        .and_then(|state| state.page.find(id).cloned());
    let Some(item) = item else { return };
    match destination(&item) {
        Destination::Page(route) => navigate(context, route),
        Destination::Detail(target) => open_detail(context, target),
    }
}

/// Opens the page a section title's "›" leads to.
pub fn open_shelf_link(context: &AppContext, shelf_id: &str) {
    let link = context
        .state
        .lock()
        .ok()
        .and_then(|state| match &state.page.content {
            PageContent::Shelves(shelves)
            | PageContent::Search {
                results: shelves, ..
            } => shelves
                .iter()
                .find(|shelf| shelf.id == shelf_id)
                .and_then(|shelf| shelf.link.clone()),
            _ => None,
        });
    if let Some(route) = link {
        navigate(context, route);
    }
}

/// Opens a library from the drawer.
pub fn open_library(context: &AppContext, library: LibraryRef) {
    let tab = library.tabs()[0];
    // Like jellyfin-web, a library opened from the drawer goes back home.
    if let Ok(mut state) = context.state.lock() {
        let home = Page {
            route: PageRoute::home(),
            content: PageContent::Shelves(state.home.clone()),
        };
        state.history.clear();
        state.history.push(HistoryEntry::Page(home));
        state.detail.update(DetailMessage::Reset);
    }
    load_route(context, PageRoute::Library { library, tab }, 0);
}

pub fn retry(context: &AppContext) {
    let action = context
        .state
        .lock()
        .map(|state| state.retry.clone())
        .unwrap_or(RetryAction::Page);
    match action {
        RetryAction::Page => {
            let route = context
                .state
                .lock()
                .ok()
                .map(|state| state.page.route.clone())
                .unwrap_or_else(PageRoute::home);
            if route == PageRoute::home()
                && let Ok(mut state) = context.state.lock()
            {
                state.home.clear();
            }
            load_route(context, route, 0);
        }
        RetryAction::Detail(target) => crate::features::detail::start_detail(context, target),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::features::pages::LibraryTab;

    fn item(kind: &str) -> CatalogItemData {
        CatalogItemData {
            id: "id".into(),
            title: "Title".into(),
            kind: kind.into(),
            ..CatalogItemData::default()
        }
    }

    #[test]
    fn cards_open_what_jellyfin_opens() {
        let mut library = item("CollectionFolder");
        library.collection_type = "tvshows".into();
        assert_eq!(
            destination(&library),
            Destination::Page(PageRoute::Library {
                library: LibraryRef {
                    id: "id".into(),
                    name: "Title".into(),
                    collection_type: "tvshows".into(),
                },
                tab: LibraryTab::Shows,
            })
        );
        assert!(matches!(
            destination(&item("Episode")),
            Destination::Detail(_)
        ));
        assert!(matches!(
            destination(&item("Person")),
            Destination::Detail(_)
        ));
        assert!(matches!(
            destination(&item("Playlist")),
            Destination::Detail(_)
        ));
        assert!(matches!(
            destination(&item("Studio")),
            Destination::Page(PageRoute::List(ListQuery {
                filter: ListFilter::Studio(_),
                ..
            }))
        ));
        assert!(matches!(
            destination(&item("Folder")),
            Destination::Page(PageRoute::List(ListQuery {
                filter: ListFilter::Children,
                ..
            }))
        ));
    }

    #[test]
    fn drawer_libraries_come_from_my_media() {
        let mut movies = item("CollectionFolder");
        movies.collection_type = "movies".into();
        let shelves = vec![Shelf {
            id: "libraries".into(),
            title: "My Media".into(),
            link: None,
            shape: crate::features::pages::CardShape::Backdrop,
            lines: 1,
            wrap: false,
            title_style: crate::features::pages::TitleStyle::Auto,
            items: vec![movies],
        }];
        assert_eq!(
            libraries_of(&shelves),
            vec![LibraryRef {
                id: "id".into(),
                name: "Title".into(),
                collection_type: "movies".into(),
            }]
        );
    }
}
