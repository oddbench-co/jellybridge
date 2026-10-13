//! Keeps watched state current: re-reads whatever screen is showing from the
//! server, on a timer and whenever something pokes [`AppContext::refresh`]
//! (navigating back or home, leaving the player, a finished stop report, a
//! successful watched/favorite toggle).
//!
//! A refresh is silent. It shows no spinner or error, and it never changes
//! the screen. It takes over the navigation generation only if nothing
//! happened while it was in flight (see [`AppContext::claim_generation`]), so
//! it can never land over, or outdate, a navigation the user started.

use std::time::Duration;

use tokio::time::MissedTickBehavior;

use std::collections::HashMap;

use crate::api::{CatalogItemData, JellyfinApi};
use crate::app::{AppContext, AppState};
use crate::features::detail::DetailMessage;
use crate::features::pages::{GridSettings, LibraryTab, PageContent, PageRoute};
use crate::ui;

/// How often the visible screen is re-read when nothing else triggers it.
pub const REFRESH_INTERVAL: Duration = Duration::from_secs(60);

#[derive(Clone, Debug)]
pub enum RefreshTarget {
    /// A list page: its route, grid settings and first grid row.
    Page {
        route: Box<PageRoute>,
        settings: Box<GridSettings>,
        start: usize,
    },
    Detail(String),
}

/// What to re-read, or `None` when a refresh would get in the way: signed
/// out, something playing, or a load or toggle still in flight.
pub fn refresh_target(state: &AppState, playing: bool, loading: bool) -> Option<RefreshTarget> {
    if state.api.is_none() || playing || loading || state.detail.has_pending_mutations() {
        return None;
    }
    let detail = state.detail.snapshot();
    if detail.busy {
        return None;
    }
    if let Some(page) = detail.page {
        return Some(RefreshTarget::Detail(page.item.id.clone()));
    }
    if detail.error.is_some() {
        // A failed detail load is on screen; there is nothing to re-read.
        return None;
    }
    let page = &state.page;
    let start = match &page.content {
        PageContent::Grid(grid) => grid.start,
        PageContent::Shelves(_) => 0,
        // Search results only change when the term does, and a loading or
        // failed page has its own request (or Retry).
        PageContent::Search { .. } | PageContent::Loading | PageContent::Failed(_) => {
            return None;
        }
    };
    Some(RefreshTarget::Page {
        route: Box::new(page.route.clone()),
        settings: Box::new(state.grid_settings_for(&page.route)),
        start,
    })
}

/// Pages whose rows the server picks at random: a refresh keeps the rows and
/// only updates their watched state, so the page does not reshuffle.
fn keeps_rows(route: &PageRoute) -> bool {
    matches!(
        route,
        PageRoute::Library {
            tab: LibraryTab::Suggestions | LibraryTab::Genres,
            ..
        }
    )
}

pub async fn run_refresh_loop(context: AppContext) {
    let mut interval = tokio::time::interval(REFRESH_INTERVAL);
    interval.set_missed_tick_behavior(MissedTickBehavior::Delay);
    // The first tick fires immediately; the launch already loaded everything.
    interval.tick().await;
    loop {
        tokio::select! {
            _ = interval.tick() => {}
            () = context.refresh.notified() => interval.reset(),
        }
        refresh_visible(&context);
    }
}

pub fn refresh_visible(context: &AppContext) {
    let playing = context
        .reporter
        .lock()
        .is_ok_and(|reporter| reporter.is_active());
    let loading = context.catalog_request.lock().is_ok_and(|request| {
        request
            .as_ref()
            .is_some_and(|request| !request.is_finished())
    });
    let (target, api) = match context.state.lock() {
        Ok(state) => (refresh_target(&state, playing, loading), state.api.clone()),
        Err(_) => return,
    };
    let (Some(target), Some(api)) = (target, api) else {
        return;
    };
    let generation = context.current_generation();
    let ctx = context.clone();
    context.runtime.spawn(async move {
        tracing::debug!(?target, "refreshing visible screen");
        match target {
            RefreshTarget::Page {
                route,
                settings,
                start,
            } => match api.load_page(&route, &settings, start).await {
                Ok(content) => apply_page(&ctx, generation, &route, content),
                Err(error) => tracing::warn!(%error, "page refresh failed"),
            },
            RefreshTarget::Detail(id) => refresh_detail(&ctx, generation, api, &id).await,
        }
    });
}

fn apply_page(context: &AppContext, generation: u64, route: &PageRoute, content: PageContent) {
    let claimed = context.state.lock().ok().and_then(|mut state| {
        if state.page.route != *route || state.detail.is_active() {
            return None;
        }
        let claimed = context.claim_generation(generation)?;
        if let (PageRoute::Home { favorites: false }, PageContent::Shelves(shelves)) =
            (route, &content)
        {
            // Kept even when home is not showing, so going home later starts
            // from fresh data.
            state.home = shelves.clone();
            state.libraries = crate::features::catalog::libraries_of(shelves);
        }
        if keeps_rows(route) {
            patch_user_data(&mut state.page, content);
        } else {
            state.page.content = content;
        }
        Some(claimed)
    });
    if claimed.is_some() {
        ui::catalog::present_refresh(context);
    }
}

/// Copies watched state from `fresh` onto the rows already on the page.
fn patch_user_data(page: &mut crate::features::pages::Page, fresh: PageContent) {
    let fresh_page = crate::features::pages::Page {
        route: page.route.clone(),
        content: fresh,
    };
    let mut fresh_items: HashMap<String, CatalogItemData> = HashMap::new();
    let mut fresh_page = fresh_page;
    for item in fresh_page.items_mut() {
        fresh_items.insert(item.id.clone(), item.clone());
    }
    for item in page.items_mut() {
        if let Some(fresh) = fresh_items.get(&item.id) {
            item.watched = fresh.watched;
            item.favorite = fresh.favorite;
            item.progress = fresh.progress;
            item.unplayed_count = fresh.unplayed_count;
        }
    }
}

async fn refresh_detail(context: &AppContext, generation: u64, api: JellyfinApi, id: &str) {
    let page = match api.detail_page(id).await {
        Ok(page) => page,
        Err(error) => {
            tracing::warn!(%error, "detail refresh failed");
            return;
        }
    };
    let claimed = context.state.lock().ok().and_then(|mut state| {
        // A toggle started while this was in flight keeps the generation, so
        // check it before claiming: claiming would stop that toggle's own
        // result (or rollback) from being presented.
        if state.detail.has_pending_mutations() {
            return None;
        }
        let claimed = context.claim_generation(generation)?;
        state.detail.update(DetailMessage::Refreshed(Box::new(page)));
        Some(claimed)
    });
    if let Some(claimed) = claimed {
        ui::detail::present_refresh(context, claimed);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::features::detail::{DetailItemData, DetailKind, DetailPageData, DetailTarget};
    use crate::services::session::StoredSession;

    fn signed_in() -> AppState {
        AppState {
            api: Some(
                JellyfinApi::from_session(StoredSession {
                    server_url: "http://jellyfin.invalid/".into(),
                    token: "token".into(),
                    user_id: "user".into(),
                })
                .unwrap(),
            ),
            ..AppState::default()
        }
    }

    fn folder(id: &str) -> CatalogItemData {
        CatalogItemData {
            id: id.into(),
            title: String::new(),
            subtitle: String::new(),
            kind: "CollectionFolder".into(),
            image_item_id: id.into(),
            image_tag: None,
            watched: false,
            progress: 0.0,
            is_container: true,
            playable: false,
            ..CatalogItemData::default()
        }
    }

    fn show_detail(state: &mut AppState, id: &str) {
        state
            .detail
            .update(DetailMessage::Loaded(Box::new(DetailPageData {
                item: DetailItemData::fixture(id, DetailKind::Episode),
                seasons: Vec::new(),
                episodes: Vec::new(),
                similar: Vec::new(),
                children_error: None,
            })));
    }

    fn target(state: &AppState) -> Option<RefreshTarget> {
        refresh_target(state, false, false)
    }

    #[test]
    fn nothing_is_refreshed_while_signed_out_playing_or_loading() {
        assert!(target(&AppState::default()).is_none());
        let state = signed_in();
        assert!(refresh_target(&state, true, false).is_none());
        assert!(refresh_target(&state, false, true).is_none());
    }

    #[test]
    fn nothing_is_refreshed_while_a_detail_load_or_toggle_is_in_flight() {
        let mut state = signed_in();
        state.detail.update(DetailMessage::Open(DetailTarget::new(
            "episode",
            DetailKind::Episode,
        )));
        assert!(target(&state).is_none());

        let mut state = signed_in();
        show_detail(&mut state, "episode");
        state.detail.update(DetailMessage::SetWatched {
            id: "episode".into(),
            watched: true,
        });
        assert!(target(&state).is_none());

        let mut state = signed_in();
        state
            .detail
            .update(DetailMessage::LoadFailed("offline".into()));
        assert!(target(&state).is_none());
    }

    #[test]
    fn the_visible_page_decides_what_is_refreshed() {
        let mut state = signed_in();
        // A page still loading has its own request in flight.
        assert!(target(&state).is_none());

        state.page.content = PageContent::Shelves(Vec::new());
        assert!(matches!(
            target(&state),
            Some(RefreshTarget::Page { route, start: 0, .. }) if *route == PageRoute::home()
        ));

        state.page = crate::features::pages::Page {
            route: PageRoute::List(crate::features::pages::ListQuery::children("Library", "library")),
            content: PageContent::Grid(crate::features::pages::GridPage {
                items: vec![folder("a")],
                total: 300,
                start: 100,
            }),
        };
        assert!(matches!(
            target(&state),
            Some(RefreshTarget::Page { start: 100, .. })
        ));

        show_detail(&mut state, "episode");
        assert!(matches!(
            target(&state),
            Some(RefreshTarget::Detail(id)) if id == "episode"
        ));
    }

    #[test]
    fn random_pages_keep_their_rows_and_take_new_watched_state() {
        let route = PageRoute::Library {
            library: crate::features::pages::LibraryRef {
                id: "lib".into(),
                name: "Movies".into(),
                collection_type: "movies".into(),
            },
            tab: LibraryTab::Genres,
        };
        assert!(keeps_rows(&route));
        let shelf = |items: Vec<CatalogItemData>| crate::features::pages::Shelf {
            id: "genre".into(),
            title: "Action".into(),
            link: None,
            shape: crate::features::pages::CardShape::Portrait,
            lines: 2,
            wrap: true,
            items,
        };
        let mut page = crate::features::pages::Page {
            route,
            content: PageContent::Shelves(vec![shelf(vec![folder("a"), folder("b")])]),
        };
        let mut watched = folder("b");
        watched.watched = true;
        patch_user_data(
            &mut page,
            PageContent::Shelves(vec![shelf(vec![watched, folder("c")])]),
        );
        let ids = page.items_mut().map(|item| (item.id.clone(), item.watched)).collect::<Vec<_>>();
        assert_eq!(ids, [("a".to_owned(), false), ("b".to_owned(), true)]);
    }
}
