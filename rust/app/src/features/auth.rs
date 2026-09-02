//! Session lifecycle: sign-in, session restore, and logout.

use tokio::task::JoinHandle;

use crate::api::{ApiError, CatalogSectionData, JellyfinApi};
use crate::app::{AppContext, AppState, RetryAction, cancel_catalog_request, replace_catalog_request};
use crate::features::{catalog, playback};
use crate::services::player::PlayerCommand;
use crate::services::session::SessionStore;
use crate::ui;

/// What the launch prefetch found, handed to [`restore_session`] once the
/// window exists.
pub enum PrefetchedHome {
    /// No saved session on disk: sign in.
    NoSession,
    /// The saved session could not be read or used at all.
    Unusable(String),
    /// A usable session, and whatever its dashboard request returned.
    Fetched(JellyfinApi, Box<Result<Vec<CatalogSectionData>, ApiError>>),
}

/// Starts the saved session's dashboard request before the window exists.
///
/// The first HTTP byte used to leave from `restore_session`, at the very end of
/// `main`: backend selection, the tokio runtime, the whole Slint tree, mpv core
/// init and the renderer install had all completed on the main thread first, so
/// the server round trip was *appended to* local startup rather than overlapping
/// it. Everything here -- the blocking `session.json` read and the reqwest
/// client build, which loads and parses the system CA store -- runs on the
/// runtime, so the main thread goes straight on to building the window.
///
/// The result is authoritative: nothing is painted from disk, and the request
/// is the same one `start_home_load` would have made.
pub fn prefetch_home(runtime: &tokio::runtime::Handle) -> JoinHandle<PrefetchedHome> {
    runtime.spawn(async move {
        let loaded = tokio::task::spawn_blocking(|| {
            SessionStore::for_current_user().and_then(|store| store.load())
        })
        .await;
        let session = match loaded {
            Ok(Ok(Some(session))) => session,
            Ok(Ok(None)) => return PrefetchedHome::NoSession,
            Ok(Err(error)) => return PrefetchedHome::Unusable(error.to_string()),
            Err(error) => return PrefetchedHome::Unusable(error.to_string()),
        };
        match JellyfinApi::from_session(session) {
            Ok(api) => {
                let home = api.home().await;
                tracing::debug!(ms = crate::ui::startup::elapsed_ms(), stage = "home_arrived", "startup_stage");
                PrefetchedHome::Fetched(api, Box::new(home))
            }
            Err(error) => PrefetchedHome::Unusable(error.to_string()),
        }
    })
}

/// Shows whatever the prefetch found. Called once, from `main`, after the
/// window is up.
pub fn restore_session(context: &AppContext, prefetch: JoinHandle<PrefetchedHome>) {
    let generation = context.next_generation();
    ui::set_screen_busy(context, ui::Screen::Catalog, true, "");
    if let Ok(mut state) = context.state.lock() {
        state.retry = RetryAction::Home;
    }
    let ctx = context.clone();
    let task = context.runtime.spawn(async move {
        let prefetched = match prefetch.await {
            Ok(prefetched) => prefetched,
            // The prefetch task itself failed; there is nothing to show but
            // the login screen, and the user can retry from there.
            Err(error) => PrefetchedHome::Unusable(error.to_string()),
        };
        if !ctx.is_current(generation) {
            return;
        }
        match prefetched {
            PrefetchedHome::NoSession => ui::show_login(&ctx, ""),
            PrefetchedHome::Unusable(error) => ui::show_login(&ctx, &error),
            PrefetchedHome::Fetched(api, home) => {
                let account = api
                    .session()
                    .server_url
                    .split("//")
                    .nth(1)
                    .unwrap_or("Saved session")
                    .trim_end_matches('/')
                    .to_owned();
                if let Ok(mut state) = ctx.state.lock() {
                    state.account_name = account;
                }
                catalog::apply_home_result(&ctx, api, *home);
            }
        }
    });
    replace_catalog_request(context, task.abort_handle());
}

pub fn start_login(context: &AppContext, server: String, username: String, password: String) {
    let generation = context.next_generation();
    ui::set_screen_busy(context, ui::Screen::Login, true, "");
    let ctx = context.clone();
    context.runtime.spawn(async move {
        let result = async {
            let (api, account_name) = JellyfinApi::authenticate(&server, &username, &password)
                .await
                .map_err(|error| error.to_string())?;
            ctx.store
                .save(api.session())
                .map_err(|error| error.to_string())?;
            let home = api.home().await.map_err(|error| error.to_string())?;
            Ok::<_, String>((api, account_name, home))
        }
        .await;
        if !ctx.is_current(generation) {
            return;
        }
        match result {
            Ok((api, account_name, home)) => {
                catalog::install_home(&ctx, api, account_name, home);
            }
            Err(error) => ui::show_login(&ctx, &error),
        }
    });
}

pub fn logout(context: &AppContext) {
    context.next_generation();
    cancel_catalog_request(context);
    let _ = context.player_sender.send(PlayerCommand::Stop);
    playback::stop_remote_playback(context);
    let message = context.store.clear().err().map(|error| error.to_string());
    if let Ok(mut state) = context.state.lock() {
        reset_session_state(&mut state);
    }
    if let Ok(mut tracks) = context.player_tracks.lock() {
        *tracks = crate::app::PlayerTrackState::default();
    }
    ui::clear_image_cache_async();
    ui::show_login(context, message.as_deref().unwrap_or(""));
}

pub fn reset_session_state(state: &mut AppState) {
    let sort = state.catalog_sort;
    let density = state.catalog_density;
    *state = AppState::default();
    state.catalog_sort = sort;
    state.catalog_density = density;
}
