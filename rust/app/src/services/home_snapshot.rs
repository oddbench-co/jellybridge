//! The last Home the server returned, kept on disk so a launch can paint it
//! at once instead of waiting ~250 ms on the server's slowest request. The
//! fresh Home replaces it as soon as it arrives.
//!
//! The file sits beside the artwork cache (not in it: that one is pruned)
//! and belongs to one server and user; anything else, an older format or a
//! week-old snapshot is ignored, and launch falls back to waiting.

use std::path::PathBuf;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use serde::{Deserialize, Serialize};

use crate::api::HomeData;

/// Bumped whenever `HomeData`'s shape changes, so an old file is not
/// misread.
const VERSION: u32 = 1;
/// Older than this, a snapshot shows too much that has changed.
const MAX_AGE: Duration = Duration::from_secs(7 * 24 * 3600);

#[derive(Serialize, Deserialize)]
struct Snapshot {
    version: u32,
    server_url: String,
    user_id: String,
    saved_at: u64,
    home: HomeData,
}

fn path() -> Option<PathBuf> {
    if cfg!(test) {
        return None;
    }
    crate::services::session::cache_directory()
        .ok()
        .and_then(|images| images.parent().map(|cache| cache.join("home.json")))
}

fn now_seconds() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |since| since.as_secs())
}

/// The saved Home for this server and user, if there is a recent one.
/// Blocking file I/O: call it off the UI thread.
pub fn load(server_url: &str, user_id: &str) -> Option<HomeData> {
    let bytes = std::fs::read(path()?).ok()?;
    let snapshot: Snapshot = serde_json::from_slice(&bytes)
        .map_err(|error| tracing::debug!(%error, "home snapshot unreadable"))
        .ok()?;
    let fresh = now_seconds().saturating_sub(snapshot.saved_at) < MAX_AGE.as_secs();
    (snapshot.version == VERSION
        && snapshot.server_url == server_url
        && snapshot.user_id == user_id
        && fresh)
        .then_some(snapshot.home)
}

/// Saves `home` for the next launch, replacing the file atomically so a
/// crash mid-write leaves the previous one. Blocking: call it off the UI
/// thread.
pub fn save(server_url: &str, user_id: &str, home: &HomeData) {
    let Some(path) = path() else { return };
    let snapshot = Snapshot {
        version: VERSION,
        server_url: server_url.to_owned(),
        user_id: user_id.to_owned(),
        saved_at: now_seconds(),
        home: home.clone(),
    };
    let result = serde_json::to_vec(&snapshot)
        .map_err(std::io::Error::other)
        .and_then(|bytes| {
            let temporary = path.with_extension("json.tmp");
            std::fs::write(&temporary, bytes)?;
            std::fs::rename(&temporary, &path)
        });
    if let Err(error) = result {
        tracing::warn!(%error, "home snapshot not saved");
    }
}

/// Forgets the saved Home (sign-out): the next user must not see it.
pub fn clear() {
    if let Some(path) = path() {
        let _ = std::fs::remove_file(path);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::api::{CatalogItemData, CatalogSectionData};
    use crate::features::prefs::UserPrefs;

    #[test]
    fn a_saved_home_reads_back_equal() {
        // The unchanged-Home check compares a read-back snapshot with the
        // server's: every field must survive the round trip exactly.
        let mut prefs = UserPrefs::default();
        prefs.custom.insert("homesection0".into(), "resume".into());
        let home = HomeData {
            sections: vec![CatalogSectionData {
                id: "resume".into(),
                title: "Continue Watching".into(),
                items: vec![CatalogItemData {
                    id: "item".into(),
                    title: "Pilot".into(),
                    progress: 0.371_234_5,
                    ..CatalogItemData::default()
                }],
            }],
            prefs,
        };
        let snapshot = Snapshot {
            version: VERSION,
            server_url: "http://jellyfin.local".into(),
            user_id: "user".into(),
            saved_at: 1,
            home: home.clone(),
        };
        let bytes = serde_json::to_vec(&snapshot).unwrap();
        let read: Snapshot = serde_json::from_slice(&bytes).unwrap();
        assert_eq!(read.home, home);
    }
}
