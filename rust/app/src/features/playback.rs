//! Playback orchestration: preparing a Jellyfin stream for the mpv worker and
//! remembering per-title track preferences.

use std::sync::{Arc, Mutex};

use crate::api::JellyfinApi;
use crate::app::{AppContext, AppState};
use crate::features::detail::{
    DetailEffect, DetailItemData, DetailKind, DetailMessage, StreamData,
};
use crate::features::now_playing::{NowPlaying, PlayerOverlay, PreviousAction, previous_action};
use crate::features::progress::{PlaybackReporter, ProgressReport, ReportTarget};
use crate::services::playback::{PlaybackConfig, SubtitleFetcher, prepare_playback};
use crate::services::player::{PlayerCommand, PlayerSubtitleFile, PlayerTrack};
use crate::services::session::{SettingsStore, StoredSettings};
use crate::ui;

pub fn start_playback(context: &AppContext, from_start: bool) {
    let detail = context.state.lock().ok().and_then(|mut state| {
        let id = state
            .detail
            .update(DetailMessage::Play)
            .into_iter()
            .find_map(|effect| match effect {
                DetailEffect::Play(id) => Some(id),
                _ => None,
            })?;
        state.detail.item_by_id(&id)
    });
    let Some(detail) = detail else { return };
    start_playback_item(context, detail, from_start);
}

pub fn start_playback_item(context: &AppContext, detail: DetailItemData, from_start: bool) {
    begin_playback(context, detail, from_start, Origin::Detail);
}

/// Starts loading libmpv now, so it is ready by the time the stream is: the
/// player is built on first use, and this overlaps that with the server
/// round trips that come before `loadfile`.
fn prepare_player(context: &AppContext) {
    crate::services::render::note_play_requested();
    let _ = context.player_sender.send(PlayerCommand::Prepare);
}

/// Plays an item straight from a home or library card, as jellyfin-web's
/// card play button does: a movie or episode itself, a series or season its
/// next episode. The player opens at once and spins while the item and its
/// stream are prepared.
pub fn play_item_by_id(context: &AppContext, id: String) {
    let Some(api) = context.api() else { return };
    prepare_player(context);
    let ctx = context.clone();
    context.runtime.spawn(async move {
        match api.detail_page(&id).await {
            Ok(page) => {
                let target = if page.item.playable {
                    Some(page.item.clone())
                } else {
                    page.play_target().cloned()
                };
                match target {
                    Some(item) => begin_playback(&ctx, item, false, Origin::Card),
                    None => ui::set_message(&ctx, "Nothing to play"),
                }
            }
            Err(error) => ui::set_message(&ctx, &error.to_string()),
        }
    });
}

/// Where playback was started from, which decides what is shown while the
/// stream is prepared.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Origin {
    /// The detail page, which shows its busy overlay.
    Detail,
    /// The player itself (next/previous), which keeps spinning.
    Player,
    /// A card, which opens the player straight away.
    Card,
    /// The first item of a play queue; like a card, but keeps the queue.
    Queue,
}

/// Plays a queue built on the server from `source` (Play All, Shuffle).
pub fn play_queue(context: &AppContext, source: crate::api::QueueSource, shuffle: bool) {
    let Some(api) = context.api() else { return };
    prepare_player(context);
    let ctx = context.clone();
    context.runtime.spawn(async move {
        match api.queue_ids(&source, shuffle).await {
            Ok(ids) if !ids.is_empty() => start_queue(&ctx, &api, ids, 0).await,
            Ok(_) => ui::set_message(&ctx, "Nothing to play"),
            Err(error) => ui::set_message(&ctx, &error.to_string()),
        }
    });
}

/// Plays `ids` in order from `start` ("Play all from here").
/// The Trailer button: the item's first local trailer plays here; without
/// one, its first online trailer opens in the browser (jellyfin-web plays
/// those in its own YouTube overlay).
pub fn play_trailer(context: &AppContext, id: &str) {
    let item = context
        .state
        .lock()
        .ok()
        .and_then(|state| state.detail.item_by_id(id));
    let Some(item) = item else { return };
    if item.local_trailers == 0 {
        if let Some(url) = item.trailer_urls.first() {
            crate::ui::detail::open_external_url(url.clone());
        }
        return;
    }
    let Some(api) = context.api() else { return };
    let ctx = context.clone();
    let id = id.to_owned();
    context.runtime.spawn(async move {
        match api.local_trailers(&id).await {
            Ok(trailers) if !trailers.is_empty() => play_ids(&ctx, trailers, 0),
            Ok(_) => {}
            Err(error) => tracing::warn!(%error, "trailer unavailable"),
        }
    });
}

pub fn play_ids(context: &AppContext, ids: Vec<String>, start: usize) {
    let Some(api) = context.api() else { return };
    if start >= ids.len() {
        return;
    }
    prepare_player(context);
    let ctx = context.clone();
    context
        .runtime
        .spawn(async move { start_queue(&ctx, &api, ids, start).await });
}

async fn start_queue(context: &AppContext, api: &JellyfinApi, ids: Vec<String>, start: usize) {
    let first = ids[start].clone();
    match api.detail_items(std::slice::from_ref(&first)).await {
        Ok(items) => match items.into_iter().next() {
            Some(item) => {
                if let Ok(mut state) = context.state.lock() {
                    state.queue = Some(ids);
                }
                begin_playback(context, item, false, Origin::Queue);
            }
            None => ui::set_message(context, "Nothing to play"),
        },
        Err(error) => ui::set_message(context, &error.to_string()),
    }
}

/// The queue's neighbours of `id`, if it is playing from a queue.
fn queue_neighbours(
    queue: Option<&Vec<String>>,
    id: &str,
) -> Option<(Option<String>, Option<String>)> {
    let queue = queue?;
    let index = queue.iter().position(|queued| queued == id)?;
    Some((
        index
            .checked_sub(1)
            .and_then(|previous| queue.get(previous).cloned()),
        queue.get(index + 1).cloned(),
    ))
}

/// Moves to the next (or previous) episode from inside the player.
pub fn play_adjacent(context: &AppContext, forward: bool) {
    let item = context.state.lock().ok().and_then(|state| {
        let now = state.now_playing.as_ref()?;
        if forward {
            now.next.clone()
        } else {
            now.previous.clone()
        }
    });
    if let Some(item) = item {
        begin_playback(context, item, false, Origin::Player);
    }
}

/// Fills Playback Info's "Original Media Info" from the server.
pub fn load_playback_info(context: &AppContext, item_id: String, audio_index: Option<i64>) {
    let Some(api) = context.api() else { return };
    let ui = context.ui.clone();
    context.runtime.spawn(async move {
        match api.original_media_info(&item_id, audio_index).await {
            Ok(rows) => ui.update(move |state| {
                if let Some(stats) = &mut state.player.stats {
                    stats.original = rows;
                }
            }),
            Err(error) => tracing::warn!(%error, "playback info failed"),
        }
    });
}

/// Starts the playing item again from the beginning.
pub fn replay_current(context: &AppContext) {
    let item = context
        .state
        .lock()
        .ok()
        .and_then(|state| state.now_playing.as_ref().map(|now| now.item.clone()));
    if let Some(item) = item {
        begin_playback(context, item, true, Origin::Player);
    }
}

/// Jellyfin's previous button: restart once a few seconds in, otherwise go
/// back an episode.
pub fn play_previous(context: &AppContext, position: f64) {
    let action = context.state.lock().ok().and_then(|state| {
        state
            .now_playing
            .as_ref()
            .map(|now| previous_action(now, position))
    });
    match action {
        Some(PreviousAction::Play(item)) => begin_playback(context, *item, false, Origin::Player),
        Some(PreviousAction::Restart) | None => {
            let _ = context.player_sender.send(PlayerCommand::SeekAbsolute(0.0));
        }
    }
}

/// Hides the Up Next card for this item, which also cancels autoplay.
pub fn dismiss_up_next(context: &AppContext) {
    if let Ok(mut state) = context.state.lock()
        && let Some(now) = state.now_playing.as_mut()
    {
        now.up_next_dismissed = true;
    }
    ui::player::present_overlay(context, PlayerOverlay::default());
}

fn begin_playback(context: &AppContext, detail: DetailItemData, from_start: bool, origin: Origin) {
    let Some(session) = context
        .state
        .lock()
        .ok()
        .and_then(|state| state.api.as_ref().map(|api| api.session().clone()))
    else {
        return;
    };
    if !detail.playable {
        return;
    }
    let initial_position = if from_start {
        0.0
    } else {
        detail.resume_seconds.max(0.0)
    };
    // An unknown runtime means "no duration yet", not "duration equals the
    // resume position" -- the latter renders a full bar until mpv reports the
    // real duration a moment later.
    let initial_duration = detail
        .runtime_seconds
        .filter(|runtime| *runtime > 0.0)
        .map(|runtime| runtime.max(initial_position))
        .unwrap_or(0.0);
    let preference_key = playback_preference_key(&detail);
    let config = match PlaybackConfig::from_session(&session, detail.id.clone()) {
        Ok(config) => config,
        Err(error) => {
            ui::set_message(context, &error.to_string());
            return;
        }
    };
    // A card's play button prepared the player before looking the item up.
    if !matches!(origin, Origin::Card | Origin::Queue) {
        prepare_player(context);
    }
    // Playing one item ends any queue; next/previous move within it.
    if matches!(origin, Origin::Card | Origin::Detail)
        && let Ok(mut state) = context.state.lock()
    {
        state.queue = None;
    }
    let generation = context.next_generation();
    match origin {
        Origin::Player => ui::player::present_player_loading(context),
        Origin::Card | Origin::Queue => ui::player::present_playback_started(
            context,
            player_title(&detail),
            initial_position,
            initial_duration,
        ),
        Origin::Detail => ui::set_screen_busy(
            context,
            ui::Screen::Detail,
            true,
            "Preparing authenticated playback…",
        ),
    }
    let ctx = context.clone();
    context.runtime.spawn(async move {
        // Settings are read here, off the UI thread, because the store does
        // blocking file I/O.
        let settings = ctx.settings_store.load().unwrap_or_default();
        let (audio_source_index, subtitle_source_index) =
            resolved_playback_sources(&detail, &settings);
        let old = ctx.playback.lock().ok().and_then(|mut slot| slot.take());
        if let Some(old) = old {
            old.shutdown().await;
        }
        let prepared = prepare_playback(config).await;
        if !ctx.is_current(generation) {
            if let Ok((prepared, _)) = prepared {
                prepared.shutdown().await;
            }
            // The overlay was raised before this task started and nothing else
            // lowers it, so abandoning the playback silently left the detail
            // screen on "Loading…" for the rest of the session.
            ui::player::present_playback_failed(&ctx, String::new());
            return;
        }
        match prepared {
            Ok((prepared, fetcher)) => {
                // Doubles as this playback's identity: the proxy's secret path
                // is fresh for every prepared stream.
                let local_url = prepared.local_url.clone();
                let uri = local_url.clone();
                if let Ok(mut tracks) = ctx.player_tracks.lock() {
                    tracks.preference_key = Some(preference_key);
                }
                let mut options = Vec::new();
                if let Ok(mut state) = ctx.state.lock() {
                    let mut now = NowPlaying::new(detail.clone());
                    now.behaviour = crate::features::now_playing::Behaviour::from_settings(
                        &state.prefs,
                        &state.device,
                    );
                    state.now_playing = Some(now);
                    options = crate::features::settings::player_options(&state.device);
                }
                // The user's audio channels and subtitle look, before the
                // file loads.
                let _ = ctx.player_sender.send(PlayerCommand::Configure(options));
                load_now_playing_context(&ctx, &detail);
                let target = ReportTarget {
                    item_id: detail.id.clone(),
                    media_source_id: prepared.media_source_id.clone(),
                    play_session_id: prepared.play_session_id.clone(),
                };
                // Enter the player screen before loading, not after: the
                // rendering notifier only builds mpv's render context on a
                // frame drawn with this screen showing.
                ui::player::present_playback_started(
                    &ctx,
                    player_title(&detail),
                    initial_position,
                    initial_duration,
                );
                wait_for_render_context(&ctx).await;
                if ctx
                    .player_sender
                    .send(PlayerCommand::Load {
                        uri,
                        start_seconds: initial_position,
                        // External text subtitles are not here: they are
                        // fetched below, after playback has started, and
                        // attached as each one arrives.
                        subtitles: Vec::new(),
                        audio_source_index,
                        subtitle_source_index,
                    })
                    .is_err()
                {
                    prepared.shutdown().await;
                    ui::player::present_playback_failed(
                        &ctx,
                        "The MPV player worker is unavailable".to_owned(),
                    );
                    return;
                }
                if let Ok(mut slot) = ctx.playback.lock() {
                    *slot = Some(prepared);
                }
                // Tell the server the previous item stopped before claiming
                // the session for this one.
                finish_report(&ctx.reporter, &ctx.state, &ctx.runtime, &ctx.refresh);
                let start = ctx
                    .reporter
                    .lock()
                    .ok()
                    .map(|mut reporter| reporter.begin(target, initial_position));
                if let Some(start) = start {
                    send_start_report(&ctx.state, &ctx.runtime, start);
                }
                fetch_subtitles(&ctx, fetcher, local_url, subtitle_source_index);
            }
            Err(error) => {
                ui::player::present_playback_failed(&ctx, error.to_string());
            }
        }
    });
}

/// Fetches what the player needs beyond the stream -- the neighbouring
/// episodes and the media segments -- and installs them if the same item is
/// still playing. Both are optional extras, so failures are only logged.
fn load_now_playing_context(context: &AppContext, detail: &DetailItemData) {
    let Some(api) = context.api() else { return };
    let item_id = detail.id.clone();
    let series_id = (detail.kind == DetailKind::Episode)
        .then(|| detail.series_id.clone())
        .flatten();
    let queued = context
        .state
        .lock()
        .ok()
        .and_then(|state| queue_neighbours(state.queue.as_ref(), &item_id));
    let ctx = context.clone();
    context.runtime.spawn(async move {
        let adjacent = async {
            if let Some((previous, next)) = queued {
                let ids = previous
                    .iter()
                    .chain(next.iter())
                    .cloned()
                    .collect::<Vec<_>>();
                let items = api.detail_items(&ids).await?;
                let find = |id: &Option<String>| {
                    id.as_ref()
                        .and_then(|id| items.iter().find(|item| item.id == *id).cloned())
                };
                return Ok((find(&previous), find(&next)));
            }
            match &series_id {
                Some(series_id) => api.adjacent_episodes(series_id, &item_id).await,
                None => Ok((None, None)),
            }
        };
        let (adjacent, segments) = tokio::join!(adjacent, api.media_segments(&item_id));
        let (previous, next) = adjacent.unwrap_or_else(|error| {
            tracing::warn!(%error, "could not load adjacent episodes");
            (None, None)
        });
        let segments = segments.unwrap_or_else(|error| {
            tracing::warn!(%error, "could not load media segments");
            Vec::new()
        });
        tracing::debug!(
            segments = segments.len(),
            previous = previous.is_some(),
            next = next.is_some(),
            "loaded now-playing context"
        );
        let neighbours = ctx.state.lock().ok().and_then(|mut state| {
            let now = state
                .now_playing
                .as_mut()
                .filter(|now| now.item.id == item_id)?;
            now.previous = previous;
            now.next = next;
            now.segments = segments;
            Some((now.previous.is_some(), now.next.is_some()))
        });
        if let Some((has_previous, has_next)) = neighbours {
            ui::player::present_neighbours(&ctx, has_previous, has_next);
        }
    });
}

/// How long playback waits for mpv's render context before loading regardless.
/// Creation measured ~320 ms on this machine; five seconds is "something is
/// badly wrong", not a tuning knob.
const RENDER_CONTEXT_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(5);

/// Holds `loadfile` until mpv has a video output to choose a decode path with.
///
/// `vo=libmpv` has no output until `mpv_render_context_create` has run, and the
/// context is built lazily, on the first frame drawn with the player screen
/// showing -- roughly 320 ms of work. Loading first meant mpv reached decoder
/// init with no GL interop to probe: it fell back to software against a VO with
/// no surface and rendered a black picture while audio played on. Which side
/// won was a race against how fast the file opened, so the same item played
/// correctly on one launch and not the next.
///
/// Measured on a losing run: `decode_path=software` was logged 196 ms before
/// `mpv render context created`.
async fn wait_for_render_context(context: &AppContext) {
    let mut ready = context.render_ready.clone();
    if tokio::time::timeout(RENDER_CONTEXT_TIMEOUT, ready.wait_for(|ready| *ready))
        .await
        .is_err()
    {
        // Deliberately not fatal: a machine that cannot build a context should
        // still play. This line is what names the fault if it ever returns.
        tracing::warn!("mpv render context was not ready in time; loading anyway");
    }
}

/// Downloads the external text subtitles for a playback that is already on
/// screen, attaching each to mpv as it lands.
///
/// Jellyfin delivers an embedded text track through the same route as a sidecar
/// file and extracts it with ffmpeg on demand, which on a large remux takes
/// minutes. Waiting for that before handing mpv a URL is what made the first
/// play of a title sit on the loading overlay, so it happens here instead, and
/// a subtitle that never arrives costs nothing but its own absence.
fn fetch_subtitles(
    context: &AppContext,
    fetcher: SubtitleFetcher,
    local_url: String,
    subtitle_source_index: Option<i64>,
) {
    if fetcher.pending.is_empty() {
        return;
    }
    let ctx = context.clone();
    context.runtime.spawn(async move {
        // Sequentially: every request starts a server-side ffmpeg pass over the
        // same source file, so running them at once only makes both slower.
        for pending in &fetcher.pending {
            // Generation is not the right guard here -- an unrelated catalog
            // refresh bumps it -- so this asks the only question that matters:
            // is this still the stream that is playing?
            let still_current = ctx
                .playback
                .lock()
                .ok()
                .and_then(|slot| {
                    slot.as_ref()
                        .map(|playback| playback.local_url == local_url)
                })
                .unwrap_or(false);
            if !still_current {
                tracing::info!("playback moved on; stopped materializing subtitles");
                return;
            }
            let subtitle = match fetcher.materialize(pending).await {
                Ok(subtitle) => subtitle,
                Err(error) => {
                    // Never fatal: the video is already playing.
                    tracing::warn!(
                        %error,
                        stream_index = pending.stream_index,
                        "could not materialize subtitle"
                    );
                    continue;
                }
            };
            if ctx
                .player_sender
                .send(PlayerCommand::AddSubtitle {
                    subtitle: PlayerSubtitleFile {
                        path: subtitle.path,
                        source_index: Some(i64::from(subtitle.stream_index)),
                        language: subtitle.language,
                        title: subtitle.title,
                    },
                    preferred_source_index: subtitle_source_index,
                })
                .is_err()
            {
                tracing::warn!("the MPV player worker is gone; subtitle not attached");
                return;
            }
        }
    });
}

/// Reporting is best-effort: a failed report must never interrupt playback, so
/// each one is spawned and its error only logged.
fn api_of(state: &Arc<Mutex<AppState>>) -> Option<JellyfinApi> {
    let api = state.lock().ok()?.api.clone();
    if api.is_none() {
        // Reporting silently doing nothing is exactly the failure that is hard
        // to notice, so say so rather than just returning.
        tracing::warn!("no Jellyfin client available; playback was not reported");
    }
    api
}

pub fn send_start_report(
    state: &Arc<Mutex<AppState>>,
    runtime: &tokio::runtime::Handle,
    report: ProgressReport,
) {
    let Some(api) = api_of(state) else { return };
    runtime.spawn(async move {
        match api
            .report_playback_start(&report.target, report.position_seconds)
            .await
        {
            Ok(()) => tracing::info!(
                item = %report.target.item_id,
                position = report.position_seconds,
                "reported playback start"
            ),
            Err(error) => tracing::warn!(%error, "could not report playback start"),
        }
    });
}

pub fn send_progress_report(
    state: &Arc<Mutex<AppState>>,
    runtime: &tokio::runtime::Handle,
    report: ProgressReport,
) {
    let Some(api) = api_of(state) else { return };
    runtime.spawn(async move {
        match api
            .report_playback_progress(&report.target, report.position_seconds, report.paused)
            .await
        {
            Ok(()) => tracing::debug!(
                item = %report.target.item_id,
                position = report.position_seconds,
                paused = report.paused,
                "reported playback progress"
            ),
            Err(error) => tracing::warn!(%error, "could not report playback progress"),
        }
    });
}

/// Reports the stop, then wakes `refresh`: the server decides from this
/// report whether the item counts as watched, so re-reading before it lands
/// would show the old state.
pub fn send_stop_report(
    state: &Arc<Mutex<AppState>>,
    runtime: &tokio::runtime::Handle,
    refresh: &Arc<tokio::sync::Notify>,
    target: ReportTarget,
    position_seconds: f64,
) {
    let Some(api) = api_of(state) else { return };
    let refresh = refresh.clone();
    runtime.spawn(async move {
        match api.report_playback_stopped(&target, position_seconds).await {
            Ok(()) => tracing::info!(
                item = %target.item_id,
                position = position_seconds,
                "reported playback stop"
            ),
            Err(error) => tracing::warn!(%error, "could not report playback stop"),
        }
        refresh.notify_one();
    });
}

/// Ends the reported session, if one is open. Safe to call more than once --
/// `finish` clears the target, so the stop reaches the server exactly once.
pub fn finish_report(
    reporter: &Arc<Mutex<PlaybackReporter>>,
    state: &Arc<Mutex<AppState>>,
    runtime: &tokio::runtime::Handle,
    refresh: &Arc<tokio::sync::Notify>,
) {
    let finished = reporter.lock().ok().and_then(|mut r| r.finish());
    if let Some((target, position)) = finished {
        send_stop_report(state, runtime, refresh, target, position);
    }
}

pub fn resolved_playback_sources(
    item: &DetailItemData,
    settings: &StoredSettings,
) -> (Option<i64>, Option<i64>) {
    let audio = item
        .streams
        .iter()
        .filter(|stream| stream.kind.eq_ignore_ascii_case("audio"))
        .collect::<Vec<_>>();
    let subtitles = item
        .streams
        .iter()
        .filter(|stream| stream.kind.eq_ignore_ascii_case("subtitle"))
        .collect::<Vec<_>>();
    let preference = settings
        .playback_preferences
        .get(&playback_preference_key(item));
    let selected_audio = preference
        .and_then(|preference| {
            best_preference_match(&audio, &preference.audio_language, &preference.audio_title)
        })
        .or_else(|| audio.iter().position(|stream| stream.is_default))
        .or_else(|| (!audio.is_empty()).then_some(0))
        .and_then(|index| audio.get(index))
        .map(|stream| i64::from(stream.index));
    let selected_subtitle = if preference.is_some_and(|preference| preference.subtitles_disabled) {
        Some(-1)
    } else {
        preference
            .and_then(|preference| {
                best_preference_match(
                    &subtitles,
                    &preference.subtitle_language,
                    &preference.subtitle_title,
                )
            })
            .or_else(|| subtitles.iter().position(|stream| stream.is_default))
            .and_then(|index| subtitles.get(index))
            .map(|stream| i64::from(stream.index))
    };
    (selected_audio, selected_subtitle)
}

pub fn stop_remote_playback(context: &AppContext) {
    if let Ok(mut state) = context.state.lock() {
        state.queue = None;
    }
    if let Ok(mut state) = context.state.lock() {
        state.now_playing = None;
    }
    finish_report(
        &context.reporter,
        &context.state,
        &context.runtime,
        &context.refresh,
    );
    let remote = context
        .playback
        .lock()
        .ok()
        .and_then(|mut slot| slot.take());
    if let Some(remote) = remote {
        context.runtime.spawn(remote.shutdown());
    }
}

pub fn playback_preference_key(item: &DetailItemData) -> String {
    match item.kind {
        DetailKind::Episode => {
            if let Some(id) = item.series_id.as_ref() {
                format!("series:{id}")
            } else if let Some(id) = item.season_id.as_ref() {
                format!("season:{id}")
            } else if let Some(id) = item.parent_id.as_ref() {
                format!("item:{id}")
            } else {
                format!("item:{}", item.id)
            }
        }
        DetailKind::Movie => format!("movie:{}", item.id),
        _ => format!("item:{}", item.id),
    }
}

pub fn persist_player_track_preference(
    store: &SettingsStore,
    key: Option<&str>,
    audio: Option<&PlayerTrack>,
    subtitle: Option<Option<&PlayerTrack>>,
) {
    let Some(key) = key else { return };
    let settings = match store.load() {
        Ok(settings) => settings,
        Err(error) => {
            tracing::warn!(%error, "playback preference could not be loaded");
            return;
        }
    };
    let mut preference = settings
        .playback_preferences
        .get(key)
        .cloned()
        .unwrap_or_default();
    if let Some(track) = audio {
        preference.audio_language = clean_preference(&track.language);
        preference.audio_title = clean_preference(&track.title);
    }
    if let Some(track) = subtitle {
        if let Some(track) = track {
            preference.subtitle_language = clean_preference(&track.language);
            preference.subtitle_title = clean_preference(&track.title);
            preference.subtitles_disabled = false;
        } else {
            preference.subtitles_disabled = true;
        }
    }
    if let Err(error) = store.save_playback_preference(key, preference) {
        tracing::warn!(%error, "playback preference could not be saved");
    }
}

pub fn best_preference_match<T>(
    streams: &[T],
    language: &Option<String>,
    title: &Option<String>,
) -> Option<usize>
where
    T: std::borrow::Borrow<StreamData>,
{
    let language = normalized_preference(language);
    let title = normalized_preference(title);
    if language.is_none() && title.is_none() {
        return None;
    }
    streams
        .iter()
        .enumerate()
        .map(|(index, stream)| {
            let stream = stream.borrow();
            let mut score = 0;
            if language.is_some() && normalized_preference(&stream.language) == language {
                score += 4;
            }
            if title.is_some() && normalized_preference(&stream.title) == title {
                score += 2;
            }
            (index, score)
        })
        .filter(|(_, score)| *score > 0)
        .max_by_key(|(_, score)| *score)
        .map(|(index, _)| index)
}

fn normalized_preference(value: &Option<String>) -> Option<String> {
    clean_preference(value).map(|value| value.to_lowercase())
}

/// The player's title lines, as Jellyfin shows them: an episode leads with
/// its series and puts `S1:E3 - Title` underneath; anything else shows its
/// own title over its year.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PlayerTitle {
    /// jellyfin-web's OSD title, one line.
    pub title: String,
    pub item_id: String,
    pub favorite: bool,
    /// Jellyfin's audio and subtitle streams, which the player's menus list.
    pub streams: Vec<crate::features::detail::StreamData>,
}

/// jellyfin-web's video OSD title: for an episode "Series - S1:E3 - Name",
/// then the year in brackets (a movie's production year, anything else's
/// premiere year).
pub fn player_title(item: &DetailItemData) -> PlayerTitle {
    let mut title = item.title.clone();
    if item.kind == DetailKind::Episode {
        let number = match (item.parent_index_number, item.index_number) {
            (Some(season), Some(episode)) => format!("S{season}:E{episode} - "),
            (None, Some(episode)) => format!("E{episode} - "),
            _ => String::new(),
        };
        title = format!("{number}{title}");
        if let Some(series) = item.series_name.as_deref().filter(|name| !name.is_empty()) {
            title = format!("{series} - {title}");
        }
    }
    let year = if item.kind == DetailKind::Movie {
        item.production_year
    } else {
        item.premiere_date.map(|(year, _, _)| year)
    };
    if let Some(year) = year {
        title.push_str(&format!(" ({year})"));
    }
    PlayerTitle {
        title,
        item_id: item.id.clone(),
        favorite: item.favorite,
        streams: item.streams.clone(),
    }
}

pub fn clean_preference(value: &Option<String>) -> Option<String> {
    value
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_owned)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_queue_gives_the_player_its_neighbours() {
        let queue = vec!["a".to_owned(), "b".to_owned(), "c".to_owned()];
        assert_eq!(
            queue_neighbours(Some(&queue), "b"),
            Some((Some("a".to_owned()), Some("c".to_owned())))
        );
        assert_eq!(
            queue_neighbours(Some(&queue), "a"),
            Some((None, Some("b".to_owned())))
        );
        assert_eq!(queue_neighbours(Some(&queue), "z"), None);
        assert_eq!(queue_neighbours(None, "a"), None);
    }
    use crate::services::session::StoredPlaybackPreference;

    #[test]
    fn episodes_title_the_player_with_series_and_episode_number() {
        let mut episode = DetailItemData::fixture("episode-1", DetailKind::Episode);
        episode.title = "Pilot".into();
        episode.series_name = Some("The Show".into());
        episode.parent_index_number = Some(1);
        episode.index_number = Some(3);
        episode.premiere_date = Some((2025, 3, 1));
        assert_eq!(player_title(&episode).title, "The Show - S1:E3 - Pilot (2025)");

        let mut movie = DetailItemData::fixture("movie-1", DetailKind::Movie);
        movie.title = "Alien".into();
        movie.production_year = Some(1979);
        movie.favorite = true;
        let title = player_title(&movie);
        assert_eq!(title.title, "Alien (1979)");
        assert_eq!(title.item_id, "movie-1");
        assert!(title.favorite);
    }

    #[test]
    fn episode_preferences_are_scoped_to_the_series() {
        let mut episode = DetailItemData::fixture("episode-1", DetailKind::Episode);
        episode.series_id = Some("series-1".into());

        assert_eq!(playback_preference_key(&episode), "series:series-1");
    }

    #[test]
    fn stored_language_preferences_resolve_to_jellyfin_source_indexes() {
        let mut episode = DetailItemData::fixture("episode-1", DetailKind::Episode);
        episode.series_id = Some("series-1".into());
        episode.streams = vec![
            StreamData {
                index: 4,
                kind: "Audio".into(),
                label: "ENG".into(),
                language: Some("eng".into()),
                title: Some("Stereo".into()),
                codec: Some("aac".into()),
                is_default: true,
            },
            StreamData {
                index: 9,
                kind: "Audio".into(),
                label: "JPN".into(),
                language: Some("jpn".into()),
                title: Some("5.1".into()),
                codec: Some("aac".into()),
                is_default: false,
            },
            StreamData {
                index: 12,
                kind: "Subtitle".into(),
                label: "ENG Signs".into(),
                language: Some("eng".into()),
                title: Some("Signs".into()),
                codec: Some("srt".into()),
                is_default: false,
            },
        ];
        let mut settings = StoredSettings::default();
        settings.playback_preferences.insert(
            "series:series-1".into(),
            StoredPlaybackPreference {
                audio_language: Some("JPN".into()),
                audio_title: None,
                subtitle_language: Some("eng".into()),
                subtitle_title: Some("Signs".into()),
                subtitles_disabled: false,
            },
        );

        assert_eq!(
            resolved_playback_sources(&episode, &settings),
            (Some(9), Some(12))
        );
    }

    #[test]
    fn stored_subtitle_off_choice_is_applied_explicitly() {
        let mut movie = DetailItemData::fixture("movie-1", DetailKind::Movie);
        movie.streams.push(StreamData {
            index: 5,
            kind: "Subtitle".into(),
            label: "English".into(),
            language: Some("eng".into()),
            title: None,
            codec: Some("srt".into()),
            is_default: true,
        });
        let mut settings = StoredSettings::default();
        settings.playback_preferences.insert(
            "movie:movie-1".into(),
            StoredPlaybackPreference {
                subtitles_disabled: true,
                ..StoredPlaybackPreference::default()
            },
        );

        assert_eq!(resolved_playback_sources(&movie, &settings).1, Some(-1));
    }
}
