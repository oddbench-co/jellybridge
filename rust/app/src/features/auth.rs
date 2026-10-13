//! Session lifecycle: sign-in, session restore, and logout.

use tokio::task::JoinHandle;

use crate::api::{ApiError, HomeData, JellyfinApi};
use crate::app::{
    AppContext, AppState, RetryAction, cancel_catalog_request, replace_catalog_request,
};
use crate::features::{catalog, playback};
use crate::services::player::PlayerCommand;
use crate::services::home_snapshot;
use crate::services::session::SessionStore;
use crate::ui;

/// The saved session's last Home, read from disk while the window is built.
pub type HomeSnapshot = tokio::sync::oneshot::Receiver<Option<(JellyfinApi, HomeData)>>;

/// What the launch prefetch found, handed to [`restore_session`] once the
/// window exists.
pub enum PrefetchedHome {
    /// No saved session on disk: sign in.
    NoSession,
    /// The saved session could not be read or used at all.
    Unusable(String),
    /// A usable session, and whatever its dashboard request returned.
    Fetched(JellyfinApi, Box<Result<HomeData, ApiError>>),
}

/// Starts the saved session's dashboard request before the window exists.
///
/// The first HTTP byte used to leave from `restore_session`, at the very end of
/// `main`: the tokio runtime, the window and its GL context, mpv core
/// init and the renderer install had all completed on the main thread first, so
/// the server round trip was *appended to* local startup rather than overlapping
/// it. Everything here -- the blocking `session.json` read and the reqwest
/// client build, which loads and parses the system CA store -- runs on the
/// runtime, so the main thread goes straight on to building the window.
///
/// Alongside, the last Home saved for this session is read from disk and
/// handed over at once (see [`HomeSnapshot`]); the request's result then
/// replaces it.
pub fn prefetch_home(runtime: &tokio::runtime::Handle) -> (JoinHandle<PrefetchedHome>, HomeSnapshot) {
    let (snapshot_sender, snapshot) = tokio::sync::oneshot::channel();
    let task = runtime.spawn(async move {
        let loaded = tokio::task::spawn_blocking(|| {
            SessionStore::for_current_user().and_then(|store| store.load())
        })
        .await;
        let session = match loaded {
            Ok(Ok(Some(session))) => session,
            Ok(Ok(None)) => {
                let _ = snapshot_sender.send(None);
                return PrefetchedHome::NoSession;
            }
            Ok(Err(error)) => return PrefetchedHome::Unusable(error.to_string()),
            Err(error) => return PrefetchedHome::Unusable(error.to_string()),
        };
        match JellyfinApi::from_session(session) {
            Ok(api) => {
                // The disk read races the request and always wins; it never
                // delays it.
                let disk_api = api.clone();
                tokio::task::spawn_blocking(move || {
                    let session = disk_api.session();
                    let home = home_snapshot::load(&session.server_url, &session.user_id);
                    let _ = snapshot_sender.send(home.map(|home| (disk_api, home)));
                });
                let home = api.home().await;
                tracing::debug!(
                    ms = crate::ui::startup::elapsed_ms(),
                    stage = "home_arrived",
                    "startup_stage"
                );
                PrefetchedHome::Fetched(api, Box::new(home))
            }
            Err(error) => PrefetchedHome::Unusable(error.to_string()),
        }
    });
    (task, snapshot)
}

/// Shows whatever the prefetch found. Called once, from `main`, after the
/// window is up: the saved Home first, if there is one, then the server's.
pub fn restore_session(context: &AppContext, prefetch: JoinHandle<PrefetchedHome>, snapshot: HomeSnapshot) {
    let generation = context.next_generation();
    ui::set_screen_busy(context, ui::Screen::Catalog, true, "");
    if let Ok(mut state) = context.state.lock() {
        state.retry = RetryAction::Page;
    }
    let ctx = context.clone();
    let task = context.runtime.spawn(async move {
        // The saved Home, kept to tell whether the server's differs at all.
        let shown = match snapshot.await {
            Ok(Some((api, home))) if ctx.is_current(generation) => {
                tracing::debug!(
                    ms = crate::ui::startup::elapsed_ms(),
                    stage = "home_snapshot_shown",
                    "startup_stage"
                );
                let saved = home.clone();
                set_account(&ctx, &api);
                catalog::install_saved_home(&ctx, api, account_of(&ctx), home);
                Some(saved)
            }
            _ => None,
        };
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
            // The saved Home is up: the fresh one replaces it where it is,
            // and a failed refresh leaves it showing.
            PrefetchedHome::Fetched(api, home) if shown.is_some() => match *home {
                // Usually nothing changed since the last launch: then there
                // is nothing to redraw or save, only the deferred shelves.
                Ok(home) if shown.as_ref() == Some(&home) => {
                    tracing::debug!(ms = crate::ui::startup::elapsed_ms(), stage = "home_unchanged", "startup_stage");
                    catalog::load_deferred_home(&ctx);
                }
                Ok(home) => catalog::refresh_home(&ctx, home),
                Err(crate::api::ApiError::Unauthorized) => {
                    catalog::apply_home_result(&ctx, api, Err(crate::api::ApiError::Unauthorized));
                }
                Err(error) => tracing::warn!(%error, "home refresh failed; showing the saved home"),
            },
            PrefetchedHome::Fetched(api, home) => {
                set_account(&ctx, &api);
                catalog::apply_home_result(&ctx, api, *home);
            }
        }
    });
    replace_catalog_request(context, task.abort_handle());
}

/// The account line: the server's host.
fn set_account(context: &AppContext, api: &JellyfinApi) {
    let account = api
        .session()
        .server_url
        .split("//")
        .nth(1)
        .unwrap_or("Saved session")
        .trim_end_matches('/')
        .to_owned();
    if let Ok(mut state) = context.state.lock() {
        state.account_name = account;
    }
}

fn account_of(context: &AppContext) -> String {
    context
        .state
        .lock()
        .map(|state| state.account_name.clone())
        .unwrap_or_default()
}

pub fn start_login(
    context: &AppContext,
    server: String,
    username: String,
    password: String,
    remember: bool,
) {
    let generation = context.next_generation();
    ui::set_screen_busy(context, ui::Screen::Login, true, "");
    let ctx = context.clone();
    context.runtime.spawn(async move {
        match JellyfinApi::authenticate(&server, &username, &password).await {
            Ok((api, account_name)) => {
                finish_sign_in(&ctx, generation, api, account_name, remember).await;
            }
            Err(error) => {
                if ctx.is_current(generation) {
                    sign_in_failed(&ctx, &error);
                }
            }
        }
    });
}

/// After a sign-in: keeps the session (unless "Remember Me" was off, when
/// it lives only until the app closes) and shows Home.
async fn finish_sign_in(
    ctx: &AppContext,
    generation: u64,
    api: JellyfinApi,
    account_name: String,
    remember: bool,
) {
    let result = async {
        if remember {
            ctx.store.save(api.session()).map_err(|error| error.to_string())?;
        } else {
            ctx.store.clear().map_err(|error| error.to_string())?;
        }
        api.home().await.map_err(|error| error.to_string())
    }
    .await;
    if !ctx.is_current(generation) {
        return;
    }
    match result {
        Ok(home) => catalog::install_home(ctx, api, account_name, home),
        Err(error) => ui::show_login(ctx, &error),
    }
}

/// jellyfin-web's sign-in failure: a toast, the form as it was.
fn sign_in_failed(ctx: &AppContext, error: &ApiError) {
    let message = match error {
        ApiError::Unauthorized => "Invalid username or password. Please try again.".to_owned(),
        ApiError::Status { status, .. } if status.as_u16() == 401 => {
            "Invalid username or password. Please try again.".to_owned()
        }
        ApiError::Http(_) => {
            "We're unable to connect to the selected server right now. Please ensure it is running and try again."
                .to_owned()
        }
        other => other.to_string(),
    };
    ui::show_login(ctx, "");
    ctx.ui.update(move |state| ui::show_toast(state, &message));
}

/// Quick Connect: shows a code to enter on a signed-in device, then signs
/// in once it is approved (checked every 5 seconds, as jellyfin-web does).
pub fn start_quick_connect(context: &AppContext, server: String, remember: bool) {
    let generation = context.next_generation();
    let ctx = context.clone();
    context.runtime.spawn(async move {
        let alert = |title: &'static str, body: String| {
            ctx.ui.update(move |state| {
                state.login.quick_code = None;
                state.login.alert = Some((title.to_owned(), body));
            });
        };
        match JellyfinApi::quick_connect_enabled(&server).await {
            Ok(true) => {}
            Ok(false) => {
                return alert(
                    "Quick Connect",
                    "Quick Connect is not active on this server".to_owned(),
                );
            }
            Err(error) => return alert("Quick Connect", error.to_string()),
        }
        let (secret, code) = match JellyfinApi::quick_connect_initiate(&server).await {
            Ok(started) => started,
            Err(error) => return alert("Quick Connect", error.to_string()),
        };
        ctx.ui.update(move |state| state.login.quick_code = Some(code));
        loop {
            tokio::time::sleep(std::time::Duration::from_secs(5)).await;
            if !ctx.is_current(generation) {
                return;
            }
            match JellyfinApi::quick_connect_approved(&server, &secret).await {
                Ok(true) => break,
                Ok(false) => {}
                Err(error) => {
                    return alert(
                        "Quick Connect",
                        if matches!(error, ApiError::Unauthorized) {
                            "Quick Connect was deactivated before the login request could be approved"
                                .to_owned()
                        } else {
                            error.to_string()
                        },
                    );
                }
            }
        }
        ctx.ui.update(|state| state.login.quick_code = None);
        ui::set_screen_busy(&ctx, ui::Screen::Login, true, "");
        match JellyfinApi::authenticate_with_quick_connect(&server, &secret).await {
            Ok((api, account_name)) => {
                finish_sign_in(&ctx, generation, api, account_name, remember).await;
            }
            Err(error) => sign_in_failed(&ctx, &error),
        }
    });
}

/// Stops waiting for a Quick Connect approval.
pub fn cancel_quick_connect(context: &AppContext) {
    context.next_generation();
    context.ui.update(|state| state.login.quick_code = None);
}

/// "Forgot Password": asks the server to start a reset and says what to do.
pub fn forgot_password(context: &AppContext, server: String, username: String) {
    let ctx = context.clone();
    context.runtime.spawn(async move {
        let (body, pin) = match JellyfinApi::forgot_password(&server, &username).await {
            Ok(answer) => match answer["Action"].as_str() {
                Some("ContactAdmin") => (
                    "Please contact your system administrator to reset your password.".to_owned(),
                    false,
                ),
                Some("InNetworkRequired") => (
                    "Please try again within your home network to initiate the password reset process."
                        .to_owned(),
                    false,
                ),
                Some("PinCode") => (
                    format!(
                        "The following file has been created on your server and contains instructions on how to proceed\n\n{}",
                        answer["PinFile"].as_str().unwrap_or_default()
                    ),
                    true,
                ),
                _ => ("There was an error processing the request. Please try again later.".to_owned(), false),
            },
            Err(error) => (error.to_string(), false),
        };
        ctx.ui.update(move |state| {
            state.login.alert = Some(("Forgot Password".to_owned(), body));
            if pin {
                state.login.step = ui::login::LoginStep::ForgotPin;
            }
        });
    });
}

/// Finishes a password reset with the PIN.
pub fn forgot_password_pin(context: &AppContext, server: String, pin: String) {
    let ctx = context.clone();
    context.runtime.spawn(async move {
        let (body, done) = match JellyfinApi::forgot_password_pin(&server, pin.trim()).await {
            Ok(answer) if answer["Success"].as_bool() == Some(true) => {
                let users = answer["UsersReset"]
                    .as_array()
                    .into_iter()
                    .flatten()
                    .filter_map(|user| user.as_str())
                    .collect::<Vec<_>>()
                    .join("\n");
                (
                    format!(
                        "The following users have had their passwords reset. They can now sign in with the PIN codes that were used to do the reset.\n\n{users}"
                    ),
                    true,
                )
            }
            Ok(_) | Err(_) => (
                "An invalid or expired PIN code was entered. Please try again.".to_owned(),
                false,
            ),
        };
        ctx.ui.update(move |state| {
            state.login.alert = Some(("Password Reset".to_owned(), body));
            if done {
                state.login.step = ui::login::LoginStep::SignIn;
            }
        });
    });
}

pub fn logout(context: &AppContext) {
    // Like jellyfin-web, signing out lands on "Please sign in" for the
    // same server.
    let server = context
        .state
        .lock()
        .ok()
        .and_then(|state| state.api.as_ref().map(|api| api.session().server_url.clone()));
    if let Some(server) = server {
        context.ui.update(move |state| {
            state.login.server = server;
            state.login.step = ui::login::LoginStep::SignIn;
            state.login.password.clear();
        });
    }
    context.next_generation();
    cancel_catalog_request(context);
    let _ = context.player_sender.send(PlayerCommand::Stop);
    playback::stop_remote_playback(context);
    let message = context.store.clear().err().map(|error| error.to_string());
    // The next user must not see this one's Home.
    context.runtime.spawn_blocking(home_snapshot::clear);
    if let Ok(mut state) = context.state.lock() {
        reset_session_state(&mut state);
    }
    if let Ok(mut tracks) = context.player_tracks.lock() {
        *tracks = crate::app::PlayerTrackState::default();
    }
    ui::clear_image_cache_async(context);
    ui::show_login(context, message.as_deref().unwrap_or(""));
}

pub fn reset_session_state(state: &mut AppState) {
    let grid_settings = std::mem::take(&mut state.grid_settings);
    *state = AppState::default();
    state.grid_settings = grid_settings;
}
