//! Session lifecycle: sign-in, session restore, and logout.

use crate::api::JellyfinApi;
use crate::app::{AppContext, AppState, cancel_catalog_request};
use crate::features::{catalog, playback};
use crate::services::player::PlayerCommand;
use crate::ui;

pub fn restore_session(context: &AppContext) {
    match context.store.load() {
        Ok(Some(session)) => match JellyfinApi::from_session(session.clone()) {
            Ok(api) => {
                if let Ok(mut state) = context.state.lock() {
                    state.account_name = api
                        .session()
                        .server_url
                        .split("//")
                        .nth(1)
                        .unwrap_or("Saved session")
                        .trim_end_matches('/')
                        .to_owned();
                    state.api = Some(api);
                }
                catalog::start_home_load(context);
            }
            Err(error) => ui::show_login(context, &error.to_string()),
        },
        Ok(None) => ui::show_login(context, ""),
        Err(error) => ui::show_login(context, &error.to_string()),
    }
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
