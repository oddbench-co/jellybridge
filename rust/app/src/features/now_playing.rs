//! What the player knows about the item it is playing beyond the stream:
//! its neighbours in the series (previous/next, Up Next, autoplay) and the
//! server's media segments (Skip Intro and friends). Pure logic; the fetch
//! lives in [`crate::features::playback`].

use crate::features::detail::DetailItemData;

/// How long before the end Up Next appears when the item has no outro
/// segment, and the share of a short item's runtime used instead.
/// jellyfin-web shows Up Next this long before the end: 30 s, 35 s from
/// 40 minutes, 40 s from 50 minutes; never for items under 10 minutes
/// (unless their credits are marked).
fn up_next_lead(duration: f64) -> f64 {
    if duration >= 50.0 * 60.0 {
        40.0
    } else if duration >= 40.0 * 60.0 {
        35.0
    } else {
        30.0
    }
}
const UP_NEXT_MIN_RUNTIME: f64 = 10.0 * 60.0;
/// Past this point "previous" restarts the item instead of going back, as
/// in Jellyfin's player.
const RESTART_THRESHOLD_SECONDS: f64 = 5.0;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SegmentKind {
    Intro,
    Outro,
    Recap,
    Preview,
    Commercial,
    Unknown,
}

impl SegmentKind {
    pub fn from_jellyfin(value: &str) -> Self {
        match value {
            "Intro" => Self::Intro,
            "Outro" => Self::Outro,
            "Recap" => Self::Recap,
            "Preview" => Self::Preview,
            "Commercial" => Self::Commercial,
            _ => Self::Unknown,
        }
    }
}

/// A span of the item the server has classified, in seconds.
#[derive(Clone, Debug, PartialEq)]
pub struct MediaSegment {
    pub kind: SegmentKind,
    pub start: f64,
    pub end: f64,
}

/// What the player does at a kind of segment (Playback settings, "Media
/// Segment Actions").
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SegmentAction {
    None,
    Skip,
    AskToSkip,
}

impl SegmentAction {
    fn from_setting(value: &str) -> Self {
        match value {
            "Skip" => Self::Skip,
            "AskToSkip" => Self::AskToSkip,
            _ => Self::None,
        }
    }
}

/// The user's playback choices the player follows at segments and at the
/// end of an item.
#[derive(Clone, Debug, PartialEq)]
pub struct Behaviour {
    pub intro: SegmentAction,
    pub outro: SegmentAction,
    pub recap: SegmentAction,
    pub preview: SegmentAction,
    pub commercial: SegmentAction,
    /// "Show next video info during playback".
    pub up_next: bool,
    /// "Play next episode automatically".
    pub autoplay: bool,
}

impl Default for Behaviour {
    /// jellyfin-web's defaults: ask to skip intros and credits.
    fn default() -> Self {
        Self {
            intro: SegmentAction::AskToSkip,
            outro: SegmentAction::AskToSkip,
            recap: SegmentAction::None,
            preview: SegmentAction::None,
            commercial: SegmentAction::None,
            up_next: true,
            autoplay: true,
        }
    }
}

impl Behaviour {
    pub fn from_settings(
        prefs: &crate::features::prefs::UserPrefs,
        device: &std::collections::HashMap<String, String>,
    ) -> Self {
        let defaults = Self::default();
        let action = |kind: &str, default: SegmentAction| {
            device
                .get(crate::features::settings::segment_action_key(kind))
                .map_or(default, |value| SegmentAction::from_setting(value))
        };
        Self {
            intro: action("Intro", defaults.intro),
            outro: action("Outro", defaults.outro),
            recap: action("Recap", defaults.recap),
            preview: action("Preview", defaults.preview),
            commercial: action("Commercial", defaults.commercial),
            up_next: prefs.next_video_overlay(),
            autoplay: prefs.next_episode_autoplay,
        }
    }

    fn action(&self, kind: SegmentKind) -> SegmentAction {
        match kind {
            SegmentKind::Intro => self.intro,
            SegmentKind::Outro => self.outro,
            SegmentKind::Recap => self.recap,
            SegmentKind::Preview => self.preview,
            SegmentKind::Commercial => self.commercial,
            SegmentKind::Unknown => SegmentAction::None,
        }
    }
}

pub struct NowPlaying {
    pub item: DetailItemData,
    pub behaviour: Behaviour,
    pub previous: Option<DetailItemData>,
    pub next: Option<DetailItemData>,
    pub segments: Vec<MediaSegment>,
    /// The user hid the Up Next card, which also cancels autoplay.
    pub up_next_dismissed: bool,
}

impl NowPlaying {
    pub fn new(item: DetailItemData) -> Self {
        Self {
            item,
            behaviour: Behaviour::default(),
            previous: None,
            next: None,
            segments: Vec::new(),
            up_next_dismissed: false,
        }
    }

    /// Whether the end of this item should roll straight into the next.
    pub fn autoplays(&self) -> bool {
        self.behaviour.autoplay && self.next.is_some() && !self.up_next_dismissed
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct SkipPrompt {
    pub label: &'static str,
    pub target: f64,
}

#[derive(Clone, Debug, PartialEq)]
pub struct UpNext {
    /// jellyfin-web's "Series - S1:E2 - Name".
    pub title: String,
    /// The next item's media info row.
    pub info: Vec<crate::ui::detail::MiscItem>,
    pub seconds_left: f64,
    pub episode: bool,
    /// Whether it will play by itself (else the card is static).
    pub autoplay: bool,
}

/// The floating player UI for one position: at most one skip button and the
/// Up Next card.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct PlayerOverlay {
    pub skip: Option<SkipPrompt>,
    pub up_next: Option<UpNext>,
    /// A segment set to "Skip" is under the playhead: seek here.
    pub auto_skip: Option<f64>,
}

pub fn overlay_at(now: &NowPlaying, position: f64, duration: f64) -> PlayerOverlay {
    let current = now
        .segments
        .iter()
        .find(|segment| position >= segment.start && position < segment.end - 1.0);
    let auto_skip = current
        .filter(|segment| now.behaviour.action(segment.kind) == SegmentAction::Skip)
        .map(|segment| segment.end);
    // An outro running to the end gives way to Up Next when there is one.
    let ends_into_next = |segment: &MediaSegment| {
        segment.end >= duration - 1.0 && now.next.is_some() && now.behaviour.up_next
    };
    let skip = current
        .filter(|segment| now.behaviour.action(segment.kind) == SegmentAction::AskToSkip)
        .and_then(|segment| {
            let label = match segment.kind {
                SegmentKind::Intro => "Skip Intro",
                SegmentKind::Recap => "Skip Recap",
                SegmentKind::Preview => "Skip Preview",
                SegmentKind::Commercial => "Skip Commercial",
                SegmentKind::Outro if !ends_into_next(segment) => "Skip Outro",
                SegmentKind::Outro | SegmentKind::Unknown => return None,
            };
            Some(SkipPrompt {
                label,
                target: segment.end,
            })
        });
    let up_next = now
        .next
        .as_ref()
        .filter(|_| now.behaviour.up_next && !now.up_next_dismissed && duration > 0.0)
        .filter(|_| up_next_start(now, duration).is_some_and(|start| position >= start))
        .map(|next| UpNext {
            title: display_name(next),
            info: crate::ui::detail::misc_items(next, false),
            seconds_left: (duration - position).max(0.0),
            episode: next.kind == crate::features::detail::DetailKind::Episode,
            autoplay: now.behaviour.autoplay,
        });
    PlayerOverlay {
        skip,
        up_next,
        auto_skip,
    }
}

/// When the Up Next card appears: at credits that run to the end if the
/// server knows them, otherwise jellyfin-web's lead before the end.
fn up_next_start(now: &NowPlaying, duration: f64) -> Option<f64> {
    let outro = now
        .segments
        .iter()
        .filter(|segment| segment.kind == SegmentKind::Outro && segment.end >= duration - 1.0)
        .map(|segment| segment.start)
        .reduce(f64::min);
    if let Some(start) = outro {
        return Some(start);
    }
    (duration >= UP_NEXT_MIN_RUNTIME).then(|| duration - up_next_lead(duration))
}

/// jellyfin-web's name for an item with its series: "Series - S1:E2 - Name".
fn display_name(item: &DetailItemData) -> String {
    let number = match (item.parent_index_number, item.index_number) {
        _ if item.kind != crate::features::detail::DetailKind::Episode => String::new(),
        (Some(season), Some(episode)) => format!("S{season}:E{episode} - "),
        (None, Some(episode)) => format!("E{episode} - "),
        _ => String::new(),
    };
    let name = format!("{number}{}", item.title);
    match item.series_name.as_deref().filter(|series| !series.is_empty()) {
        Some(series) => format!("{series} - {name}"),
        None => name,
    }
}

#[derive(Debug)]
pub enum PreviousAction {
    Restart,
    Play(Box<DetailItemData>),
}

impl std::fmt::Debug for NowPlaying {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("NowPlaying")
            .field("item", &self.item.id)
            .field("previous", &self.previous.as_ref().map(|item| &item.id))
            .field("next", &self.next.as_ref().map(|item| &item.id))
            .field("segments", &self.segments.len())
            .finish()
    }
}

/// Jellyfin's "previous" button: back to the start once a few seconds in,
/// otherwise the previous episode (or the start, when there is none).
pub fn previous_action(now: &NowPlaying, position: f64) -> PreviousAction {
    match &now.previous {
        Some(previous) if position <= RESTART_THRESHOLD_SECONDS => {
            PreviousAction::Play(Box::new(previous.clone()))
        }
        _ => PreviousAction::Restart,
    }
}

/// Splits a `Shows/{id}/Episodes?AdjacentTo=` response, which lists up to
/// the episode before, the episode itself and the episode after, in order.
pub fn split_adjacent(
    episodes: Vec<DetailItemData>,
    current_id: &str,
) -> (Option<DetailItemData>, Option<DetailItemData>) {
    let Some(index) = episodes.iter().position(|item| item.id == current_id) else {
        return (None, None);
    };
    let previous = index
        .checked_sub(1)
        .and_then(|index| episodes.get(index))
        .filter(|item| item.playable)
        .cloned();
    let next = episodes
        .get(index + 1)
        .filter(|item| item.playable)
        .cloned();
    (previous, next)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::features::detail::DetailKind;

    fn episode(id: &str, season: i32, number: i32) -> DetailItemData {
        let mut item = DetailItemData::fixture(id, DetailKind::Episode);
        item.title = format!("Episode {id}");
        item.parent_index_number = Some(season);
        item.index_number = Some(number);
        item
    }

    fn playing(segments: Vec<MediaSegment>, next: bool) -> NowPlaying {
        let mut now = NowPlaying::new(episode("current", 1, 2));
        now.segments = segments;
        now.next = next.then(|| episode("next", 2, 1));
        now
    }

    fn segment(kind: SegmentKind, start: f64, end: f64) -> MediaSegment {
        MediaSegment { kind, start, end }
    }

    #[test]
    fn segments_offer_a_skip_to_their_end_while_inside_them() {
        let now = playing(vec![segment(SegmentKind::Intro, 30.0, 90.0)], false);
        assert_eq!(overlay_at(&now, 10.0, 1500.0).skip, None);
        assert_eq!(
            overlay_at(&now, 30.0, 1500.0).skip,
            Some(SkipPrompt {
                label: "Skip Intro",
                target: 90.0,
            })
        );
        // Not in the last second, where skipping would do nothing useful.
        assert_eq!(overlay_at(&now, 89.5, 1500.0).skip, None);
    }

    #[test]
    fn credits_skip_only_when_there_is_no_next_episode() {
        let outro = vec![segment(SegmentKind::Outro, 1400.0, 1500.0)];
        assert_eq!(
            overlay_at(&playing(outro.clone(), false), 1410.0, 1500.0)
                .skip
                .map(|skip| skip.label),
            Some("Skip Outro")
        );
        // Credits that stop before the end are skippable either way.
        let early = vec![segment(SegmentKind::Outro, 1300.0, 1400.0)];
        assert_eq!(
            overlay_at(&playing(early, true), 1310.0, 1500.0).skip.map(|skip| skip.label),
            Some("Skip Outro")
        );
        let with_next = overlay_at(&playing(outro, true), 1410.0, 1500.0);
        assert_eq!(with_next.skip, None);
        assert!(with_next.up_next.is_some());
    }

    #[test]
    fn up_next_starts_at_the_credits_or_near_the_end() {
        let now = playing(vec![segment(SegmentKind::Outro, 1400.0, 1500.0)], true);
        assert!(overlay_at(&now, 1399.0, 1500.0).up_next.is_none());
        let up_next = overlay_at(&now, 1400.0, 1500.0).up_next.unwrap();
        assert_eq!(up_next.title, "S2:E1 - Episode next");
        assert!(up_next.episode);
        assert_eq!(up_next.seconds_left, 100.0);

        // jellyfin-web's lead: 30 s, 35 s from 40 minutes, 40 s from 50.
        let now = playing(Vec::new(), true);
        assert!(overlay_at(&now, 1469.0, 1500.0).up_next.is_none());
        assert!(overlay_at(&now, 1470.0, 1500.0).up_next.is_some());
        assert!(overlay_at(&now, 2962.5, 3003.0).up_next.is_none());
        assert!(overlay_at(&now, 2963.5, 3003.0).up_next.is_some());

        // Never for items under ten minutes.
        assert!(overlay_at(&now, 195.0, 200.0).up_next.is_none());
    }

    #[test]
    fn hiding_up_next_keeps_it_hidden_and_cancels_autoplay() {
        let mut now = playing(Vec::new(), true);
        assert!(now.autoplays());
        now.up_next_dismissed = true;
        assert!(overlay_at(&now, 1490.0, 1500.0).up_next.is_none());
        assert!(!now.autoplays());
        assert!(!playing(Vec::new(), false).autoplays());
    }

    #[test]
    fn previous_restarts_after_a_few_seconds() {
        let mut now = playing(Vec::new(), false);
        assert!(matches!(
            previous_action(&now, 2.0),
            PreviousAction::Restart
        ));
        now.previous = Some(episode("previous", 1, 1));
        assert!(matches!(
            previous_action(&now, 2.0),
            PreviousAction::Play(item) if item.id == "previous"
        ));
        assert!(matches!(
            previous_action(&now, 60.0),
            PreviousAction::Restart
        ));
    }

    #[test]
    fn adjacent_episodes_split_around_the_current_one() {
        let (previous, next) = split_adjacent(
            vec![episode("a", 1, 9), episode("b", 1, 10), episode("c", 2, 1)],
            "b",
        );
        assert_eq!(previous.map(|item| item.id), Some("a".into()));
        assert_eq!(next.map(|item| item.id), Some("c".into()));

        let (previous, next) = split_adjacent(vec![episode("a", 1, 1), episode("b", 1, 2)], "a");
        assert!(previous.is_none());
        assert_eq!(next.map(|item| item.id), Some("b".into()));

        let (previous, next) = split_adjacent(vec![episode("a", 1, 1), episode("b", 1, 2)], "b");
        assert_eq!(previous.map(|item| item.id), Some("a".into()));
        assert!(next.is_none());

        assert_eq!(
            split_adjacent(vec![episode("a", 1, 1)], "missing")
                .0
                .map(|item| item.id),
            None
        );
    }
}
