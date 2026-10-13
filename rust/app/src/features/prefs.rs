//! The user's jellyfin-web settings, which live on the server so every
//! client shares them: display preferences (`CustomPrefs` of the "emby"
//! client: home sections, library sorts, player options) and the user's
//! configuration (what Home hides).

use std::collections::HashMap;

/// One slot of jellyfin-web's Home screen.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum HomeSection {
    LibraryTiles,
    LibraryButtons,
    Resume,
    ResumeAudio,
    ResumeBook,
    ActiveRecordings,
    NextUp,
    LatestMedia,
    LiveTv,
    None,
}

impl HomeSection {
    fn from_key(key: &str) -> Option<Self> {
        Some(match key {
            "smalllibrarytiles" => Self::LibraryTiles,
            "librarybuttons" => Self::LibraryButtons,
            "resume" => Self::Resume,
            "resumeaudio" => Self::ResumeAudio,
            "resumebook" => Self::ResumeBook,
            "activerecordings" => Self::ActiveRecordings,
            "nextup" => Self::NextUp,
            "latestmedia" => Self::LatestMedia,
            "livetv" => Self::LiveTv,
            "none" => Self::None,
            _ => return Option::None,
        })
    }
}

/// jellyfin-web's default Home, slot by slot.
const DEFAULT_HOME: [HomeSection; 10] = [
    HomeSection::LibraryTiles,
    HomeSection::Resume,
    HomeSection::ResumeAudio,
    HomeSection::ResumeBook,
    HomeSection::LiveTv,
    HomeSection::NextUp,
    HomeSection::LatestMedia,
    HomeSection::None,
    HomeSection::None,
    HomeSection::None,
];

#[derive(Clone, Debug, Default, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct UserPrefs {
    /// The "emby" client's `CustomPrefs`.
    pub custom: HashMap<String, String>,
    pub hide_played_in_latest: bool,
    pub my_media_excludes: Vec<String>,
    pub latest_excludes: Vec<String>,
    /// `EnableNextEpisodeAutoPlay` of the user's configuration.
    pub next_episode_autoplay: bool,
}

impl UserPrefs {
    /// The Home sections in order, as the user arranged them.
    pub fn home_sections(&self) -> Vec<HomeSection> {
        (0..DEFAULT_HOME.len())
            .map(|slot| {
                self.custom
                    .get(&format!("homesection{slot}"))
                    .and_then(|key| HomeSection::from_key(key))
                    .unwrap_or(DEFAULT_HOME[slot])
            })
            .filter(|section| *section != HomeSection::None)
            .collect()
    }

    /// Days a show stays in Next Up after it was last watched.
    pub fn next_up_days(&self) -> u32 {
        self.custom
            .get("maxDaysForNextUp")
            .and_then(|days| days.parse().ok())
            .unwrap_or(365)
    }

    /// jellyfin-web counts this on only when it is exactly "true".
    pub fn rewatching_in_next_up(&self) -> bool {
        self.custom
            .get("enableRewatchingInNextUp")
            .is_some_and(|value| value == "true")
    }

    /// The Up Next card at the end of a video; on unless exactly "false".
    pub fn next_video_overlay(&self) -> bool {
        self.custom
            .get("enableNextVideoInfoOverlay")
            .is_none_or(|value| value != "false")
    }

    fn millis(&self, key: &str, default: f64) -> f64 {
        self.custom
            .get(key)
            .and_then(|value| value.parse::<f64>().ok())
            .filter(|value| *value > 0.0)
            .map_or(default, |value| value / 1000.0)
    }

    /// Episode images in Next Up and Continue Watching; on only when
    /// exactly "true", as jellyfin-web reads it.
    pub fn use_episode_images(&self) -> bool {
        self.custom
            .get("useEpisodeImagesInNextUpAndResume")
            .is_some_and(|value| value == "true")
    }

    /// Skip back and forward lengths, in seconds.
    pub fn skip_lengths(&self) -> (f64, f64) {
        (
            self.millis("skipBackLength", 10.0),
            self.millis("skipForwardLength", 30.0),
        )
    }

    /// Items per library page; 0 turns paging off.
    pub fn library_page_size(&self) -> usize {
        self.custom
            .get("libraryPageSize")
            .and_then(|value| value.parse().ok())
            .unwrap_or(100)
    }

    /// A library tab's saved sort: `SortBy` and whether it descends.
    pub fn grid_sort(&self, key: &str) -> Option<(String, bool)> {
        let stored: serde_json::Value = serde_json::from_str(self.custom.get(key)?).ok()?;
        let sort = stored["SortBy"].as_str()?.to_owned();
        let descending = stored["SortOrder"].as_str() == Some("Descending");
        Some((sort, descending))
    }

    /// The `CustomPrefs` value jellyfin-web stores for a sort.
    pub fn grid_sort_value(sort: &str, descending: bool) -> String {
        serde_json::json!({
            "SortBy": sort,
            "SortOrder": if descending { "Descending" } else { "Ascending" },
        })
        .to_string()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn home_follows_jellyfin_defaults_and_the_users_slots() {
        let mut prefs = UserPrefs::default();
        assert_eq!(
            prefs.home_sections(),
            vec![
                HomeSection::LibraryTiles,
                HomeSection::Resume,
                HomeSection::ResumeAudio,
                HomeSection::ResumeBook,
                HomeSection::LiveTv,
                HomeSection::NextUp,
                HomeSection::LatestMedia,
            ]
        );
        prefs.custom.insert("homesection1".into(), "nextup".into());
        prefs.custom.insert("homesection5".into(), "none".into());
        assert_eq!(prefs.home_sections()[1], HomeSection::NextUp);
        assert!(!prefs.home_sections().contains(&HomeSection::Resume));
    }

    #[test]
    fn grid_sorts_round_trip_jellyfin_json() {
        let mut prefs = UserPrefs::default();
        prefs.custom.insert(
            "lib-movies".into(),
            UserPrefs::grid_sort_value("DateCreated,SortName,ProductionYear", true),
        );
        assert_eq!(
            prefs.grid_sort("lib-movies"),
            Some(("DateCreated,SortName,ProductionYear".to_owned(), true))
        );
        assert_eq!(prefs.next_up_days(), 365);
    }
}
