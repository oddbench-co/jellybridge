//! What the item menu and its dialogs do: play queues, playlists and
//! collections, media info, the stream URL, downloads, and the links on a
//! detail page.

use std::collections::HashMap;
use std::sync::{Mutex, OnceLock};

use futures_util::StreamExt;
use tokio::io::AsyncWriteExt;

use crate::api::QueueSource;
use crate::app::AppContext;
use crate::features::detail::{DetailKind, DetailTarget};
use crate::features::pages::{ListFilter, ListQuery, PageRoute};
use crate::ui;
use crate::ui::menu::{ItemDialog, MenuCommand, MenuOrigin, MenuRequest};

/// Where a link on a detail page leads.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum LinkTarget {
    Genre {
        id: String,
        name: String,
        item_types: String,
    },
    Studio {
        id: String,
        name: String,
        item_types: String,
    },
    Tag {
        name: String,
        item_types: String,
    },
    Person(String),
    /// An external page (IMDb, TMDB), opened in the browser.
    Url(String),
}

pub fn open_link(context: &AppContext, target: LinkTarget) {
    let list = |title: String, filter: ListFilter, item_types: String| {
        PageRoute::List(ListQuery {
            title,
            parent_id: None,
            filter,
            item_types,
        })
    };
    match target {
        LinkTarget::Genre {
            id,
            name,
            item_types,
        } => crate::features::catalog::navigate(
            context,
            list(name, ListFilter::Genre(id), item_types),
        ),
        LinkTarget::Studio {
            id,
            name,
            item_types,
        } => crate::features::catalog::navigate(
            context,
            list(name, ListFilter::Studio(id), item_types),
        ),
        LinkTarget::Tag { name, item_types } => crate::features::catalog::navigate(
            context,
            list(name.clone(), ListFilter::Tag(name), item_types),
        ),
        LinkTarget::Person(id) => crate::features::catalog::open_detail(
            context,
            DetailTarget::new(id, DetailKind::Person),
        ),
        LinkTarget::Url(url) => crate::ui::detail::open_external_url(url),
    }
}

/// The queue a container plays from.
pub fn queue_source(kind: &str, id: &str) -> QueueSource {
    match kind {
        "Series" | "Season" => QueueSource::Episodes(id.to_owned()),
        "Playlist" => QueueSource::Playlist(id.to_owned()),
        _ => QueueSource::Children(id.to_owned()),
    }
}

pub fn run_command(context: &AppContext, command: MenuCommand, request: MenuRequest) {
    let id = request.item_id.clone();
    let video = matches!(
        request.kind.as_str(),
        "Movie" | "Episode" | "Video" | "MusicVideo"
    );
    match command {
        MenuCommand::Play if video => crate::features::playback::play_item_by_id(context, id),
        MenuCommand::Play => {
            let source = queue_source(&request.kind, &id);
            crate::features::playback::play_queue(context, source, false);
        }
        MenuCommand::PlayAllFromHere => {
            if let MenuOrigin::Card { queue } = request.origin {
                crate::features::playback::play_ids(context, queue, 0);
            }
        }
        MenuCommand::Shuffle => {
            let source = queue_source(&request.kind, &id);
            crate::features::playback::play_queue(context, source, true);
        }
        MenuCommand::AddToPlaylist => open_add_to(context, false, vec![id]),
        MenuCommand::AddToCollection => open_add_to(context, true, vec![id]),
        MenuCommand::RemoveFromPlaylist => {
            if let MenuOrigin::PlaylistEntry {
                playlist_id,
                entry_id,
            } = request.origin
            {
                remove_from_playlist(context, playlist_id, entry_id);
            }
        }
        MenuCommand::Download => download(context, id, request.name),
        MenuCommand::CopyStreamUrl => {
            if let Some(api) = context.api() {
                let url = api.stream_url(&id);
                context.ui.update(move |state| {
                    state.ctx.copy_text(url);
                    ui::show_toast(state, "Copied to clipboard");
                });
            }
        }
        MenuCommand::MediaInfo => media_info(context, id, request.name),
        MenuCommand::GoToSeason => {
            if let Some(season) = request.season_id {
                crate::features::catalog::open_detail(
                    context,
                    DetailTarget::new(season, DetailKind::Season),
                );
            }
        }
        MenuCommand::GoToSeries => {
            if let Some(series) = request.series_id {
                crate::features::catalog::open_detail(
                    context,
                    DetailTarget::new(series, DetailKind::Series),
                );
            }
        }
    }
}

/// Opens the Add to Playlist / Collection dialog (or, with no items, New
/// Playlist / Collection) and fills its list.
pub fn open_add_to(context: &AppContext, collection: bool, item_ids: Vec<String>) {
    let Some(api) = context.api() else { return };
    let listing = !item_ids.is_empty();
    context.ui.update(move |state| {
        state.item_dialog = Some(ItemDialog::AddTo {
            collection,
            item_ids,
            containers: if listing { None } else { Some(Vec::new()) },
            selected: None,
            name: String::new(),
            public: false,
            busy: false,
        });
    });
    if !listing {
        return;
    }
    let ctx = context.clone();
    context.runtime.spawn(async move {
        let kind = if collection { "BoxSet" } else { "Playlist" };
        let containers = api.containers(kind).await.unwrap_or_else(|error| {
            tracing::warn!(%error, "could not list playlists or collections");
            Vec::new()
        });
        ctx.ui.update(move |state| {
            if let Some(ItemDialog::AddTo {
                containers: shown,
                collection: open_kind,
                ..
            }) = state.item_dialog.as_mut()
                && *open_kind == collection
            {
                *shown = Some(containers);
            }
        });
    });
}

/// Creates the playlist or collection, or adds to the chosen one.
pub fn submit_add_to(
    context: &AppContext,
    collection: bool,
    target: Option<String>,
    name: String,
    public: bool,
    item_ids: Vec<String>,
) {
    let Some(api) = context.api() else { return };
    let ctx = context.clone();
    context.runtime.spawn(async move {
        let result = match (&target, collection) {
            (Some(id), false) => api
                .add_to_playlist(id, &item_ids)
                .await
                .map(|()| id.clone()),
            (Some(id), true) => api
                .add_to_collection(id, &item_ids)
                .await
                .map(|()| id.clone()),
            (None, false) => api.create_playlist(&name, &item_ids, public).await,
            (None, true) => api.create_collection(&name, &item_ids).await,
        };
        let message = match &result {
            Ok(_) if target.is_none() && item_ids.is_empty() => format!("Created {name}"),
            Ok(_) if collection => "Added to collection".to_owned(),
            Ok(_) => "Added to playlist".to_owned(),
            Err(error) => error.to_string(),
        };
        let succeeded = result.is_ok();
        ctx.ui.update(move |state| {
            if succeeded {
                state.item_dialog = None;
            } else if let Some(ItemDialog::AddTo { busy, .. }) = state.item_dialog.as_mut() {
                *busy = false;
            }
            ui::show_toast(state, &message);
        });
        if succeeded {
            ctx.request_refresh();
        }
    });
}

fn remove_from_playlist(context: &AppContext, playlist_id: String, entry_id: String) {
    let Some(api) = context.api() else { return };
    let ctx = context.clone();
    context.runtime.spawn(async move {
        let result = api.remove_from_playlist(&playlist_id, &[entry_id]).await;
        let message = match &result {
            Ok(()) => "Removed from playlist".to_owned(),
            Err(error) => error.to_string(),
        };
        ctx.ui.update(move |state| ui::show_toast(state, &message));
        ctx.request_refresh();
    });
}

fn media_info(context: &AppContext, id: String, name: String) {
    let Some(api) = context.api() else { return };
    context.ui.update({
        let name = name.clone();
        move |state| {
            state.item_dialog = Some(ItemDialog::MediaInfo {
                title: name,
                blocks: None,
            });
        }
    });
    let ctx = context.clone();
    context.runtime.spawn(async move {
        let result = api.media_info(&id).await;
        ctx.ui.update(move |state| match result {
            Ok((title, blocks)) => {
                if let Some(ItemDialog::MediaInfo {
                    title: shown,
                    blocks: shown_blocks,
                }) = state.item_dialog.as_mut()
                {
                    *shown = title;
                    *shown_blocks = Some(blocks);
                }
            }
            Err(error) => {
                state.item_dialog = None;
                ui::show_toast(state, &error.to_string());
            }
        });
    });
}

/// Downloads in flight, so a second click on the same item does not start
/// another copy.
fn downloads() -> &'static Mutex<HashMap<String, tokio::task::AbortHandle>> {
    static DOWNLOADS: OnceLock<Mutex<HashMap<String, tokio::task::AbortHandle>>> = OnceLock::new();
    DOWNLOADS.get_or_init(Default::default)
}

/// The folder downloads land in: `$XDG_DOWNLOAD_DIR`, else `~/Downloads`
/// (on Windows the profile's Downloads).
fn download_directory() -> Option<std::path::PathBuf> {
    if let Some(dir) = std::env::var_os("XDG_DOWNLOAD_DIR") {
        return Some(dir.into());
    }
    let home = std::env::var_os(if cfg!(windows) { "USERPROFILE" } else { "HOME" })?;
    Some(std::path::PathBuf::from(home).join("Downloads"))
}

/// The server's file name from `Content-Disposition`, made safe to use as a
/// file name; else the item's name.
fn file_name(disposition: Option<&str>, fallback: &str) -> String {
    let from_header = disposition.and_then(|value| {
        let start = value.find("filename=")? + "filename=".len();
        let name = value[start..].split(';').next()?.trim().trim_matches('"');
        (!name.is_empty()).then(|| name.to_owned())
    });
    let name = from_header.unwrap_or_else(|| fallback.to_owned());
    let cleaned = name
        .chars()
        .map(|character| match character {
            '/' | '\\' | '\0' => '_',
            other => other,
        })
        .collect::<String>();
    let cleaned = cleaned.trim_start_matches('.').trim().to_owned();
    if cleaned.is_empty() {
        "download".to_owned()
    } else {
        cleaned
    }
}

/// Streams the original file to the downloads folder, a chunk at a time,
/// with progress in the toast.
fn download(context: &AppContext, id: String, name: String) {
    let Some(api) = context.api() else { return };
    if downloads()
        .lock()
        .is_ok_and(|active| active.contains_key(&id))
    {
        context
            .ui
            .update(|state| ui::show_toast(state, "Already downloading"));
        return;
    }
    let ctx = context.clone();
    let key = id.clone();
    let task = context.runtime.spawn(async move {
        let result = async {
            let response = api.download(&id).await.map_err(|error| error.to_string())?;
            let total = response.content_length();
            let disposition = response
                .headers()
                .get(reqwest::header::CONTENT_DISPOSITION)
                .and_then(|value| value.to_str().ok())
                .map(str::to_owned);
            let directory = download_directory().ok_or("No downloads folder")?;
            tokio::fs::create_dir_all(&directory)
                .await
                .map_err(|error| error.to_string())?;
            let file_name = file_name(disposition.as_deref(), &name);
            let path = directory.join(&file_name);
            let partial = directory.join(format!(".{file_name}.part"));
            let mut file = tokio::fs::File::create(&partial)
                .await
                .map_err(|error| error.to_string())?;
            let mut stream = response.bytes_stream();
            let mut written: u64 = 0;
            let mut shown_percent = u64::MAX;
            while let Some(chunk) = stream.next().await {
                let chunk = chunk.map_err(|error| error.to_string())?;
                file.write_all(&chunk)
                    .await
                    .map_err(|error| error.to_string())?;
                written += chunk.len() as u64;
                if let Some(total) = total.filter(|total| *total > 0) {
                    let percent = written * 100 / total;
                    if percent != shown_percent {
                        shown_percent = percent;
                        let text = format!("Downloading {name}… {percent}%");
                        ctx.ui.update(move |state| ui::show_toast(state, &text));
                    }
                }
            }
            file.flush().await.map_err(|error| error.to_string())?;
            drop(file);
            tokio::fs::rename(&partial, &path)
                .await
                .map_err(|error| error.to_string())?;
            Ok::<_, String>(file_name)
        }
        .await;
        if let Ok(mut active) = downloads().lock() {
            active.remove(&id);
        }
        let message = match result {
            Ok(file_name) => format!("Downloaded {file_name}"),
            Err(error) => format!("Download failed: {error}"),
        };
        ctx.ui.update(move |state| ui::show_toast(state, &message));
    });
    if let Ok(mut active) = downloads().lock() {
        active.insert(key, task.abort_handle());
    }
    context
        .ui
        .update(|state| ui::show_toast(state, "Download started"));
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn download_names_come_from_the_server_and_stay_in_the_folder() {
        assert_eq!(
            file_name(Some("attachment; filename=\"Starsky & Hutch.mkv\""), "x"),
            "Starsky & Hutch.mkv"
        );
        assert_eq!(
            file_name(Some("attachment; filename=../../etc/passwd"), "x"),
            "_.._etc_passwd"
        );
        assert_eq!(file_name(None, "Bookish"), "Bookish");
        assert_eq!(file_name(Some("attachment"), ""), "download");
    }

    #[test]
    fn containers_queue_their_contents() {
        assert!(matches!(
            queue_source("Series", "s"),
            QueueSource::Episodes(_)
        ));
        assert!(matches!(
            queue_source("Playlist", "p"),
            QueueSource::Playlist(_)
        ));
        assert!(matches!(
            queue_source("BoxSet", "b"),
            QueueSource::Children(_)
        ));
    }
}
