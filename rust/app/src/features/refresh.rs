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

use crate::api::{CatalogItemData, JellyfinApi};
use crate::app::{AppContext, AppState, PageSnapshot};
use crate::features::detail::DetailMessage;
use crate::ui;

/// How often the visible screen is re-read when nothing else triggers it.
pub const REFRESH_INTERVAL: Duration = Duration::from_secs(60);

#[derive(Clone, Debug)]
pub enum RefreshTarget {
    Home,
    Browse(CatalogItemData),
    Search {
        term: String,
        parent_id: Option<String>,
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
    let current = &state.current;
    Some(if !current.search_text.is_empty() {
        RefreshTarget::Search {
            term: current.search_text.clone(),
            parent_id: current.parent.as_ref().map(|parent| parent.id.clone()),
        }
    } else if let Some(parent) = current.parent.clone() {
        RefreshTarget::Browse(parent)
    } else {
        RefreshTarget::Home
    })
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
    let (target, api, sort) = match context.state.lock() {
        Ok(state) => (
            refresh_target(&state, playing, loading),
            state.api.clone(),
            state.catalog_sort,
        ),
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
            RefreshTarget::Home => match api.home().await {
                Ok(home) => apply_home(&ctx, generation, home),
                Err(error) => tracing::warn!(%error, "home refresh failed"),
            },
            RefreshTarget::Browse(item) => match api.browse(&item, sort).await {
                Ok(page) => apply_items(&ctx, generation, page.items, page.total_record_count),
                Err(error) => tracing::warn!(%error, "browse refresh failed"),
            },
            RefreshTarget::Search { term, parent_id } => {
                match api.search(&term, parent_id.as_deref(), sort).await {
                    Ok(page) => {
                        apply_items(&ctx, generation, page.items, page.total_record_count);
                    }
                    Err(error) => tracing::warn!(%error, "search refresh failed"),
                }
            }
            RefreshTarget::Detail(id) => refresh_detail(&ctx, generation, api, &id).await,
        }
    });
}

fn apply_home(context: &AppContext, generation: u64, home: Vec<crate::api::CatalogSectionData>) {
    let claimed = context.state.lock().ok().and_then(|mut state| {
        let claimed = context.claim_generation(generation)?;
        if state.current.is_home_dashboard() {
            state.current = PageSnapshot::home(&home);
        }
        // Kept even when home is not showing, so going home later starts
        // from fresh data. `back_stack` is left alone, unlike `install_home`.
        state.home = home;
        Some(claimed)
    });
    if let Some(claimed) = claimed {
        ui::catalog::present_refresh(context, claimed);
    }
}

fn apply_items(
    context: &AppContext,
    generation: u64,
    items: Vec<CatalogItemData>,
    total_record_count: usize,
) {
    let claimed = context.state.lock().ok().and_then(|mut state| {
        let claimed = context.claim_generation(generation)?;
        state.current.items = items;
        state.current.total_record_count = total_record_count;
        Some(claimed)
    });
    if let Some(claimed) = claimed {
        ui::catalog::present_refresh(context, claimed);
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
        assert!(matches!(target(&state), Some(RefreshTarget::Home)));

        state.current.parent = Some(folder("library"));
        assert!(matches!(
            target(&state),
            Some(RefreshTarget::Browse(item)) if item.id == "library"
        ));

        state.current.search_text = "alien".into();
        assert!(matches!(
            target(&state),
            Some(RefreshTarget::Search { term, parent_id })
                if term == "alien" && parent_id.as_deref() == Some("library")
        ));

        show_detail(&mut state, "episode");
        assert!(matches!(
            target(&state),
            Some(RefreshTarget::Detail(id)) if id == "episode"
        ));
    }
}
