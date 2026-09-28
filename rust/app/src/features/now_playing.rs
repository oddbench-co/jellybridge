//! What the player knows about the item it is playing beyond the stream:
//! its neighbours in the series (previous/next, Up Next, autoplay) and the
//! server's media segments (Skip Intro and friends). Pure logic; the fetch
//! lives in [`crate::features::playback`].

use crate::features::detail::DetailItemData;

/// How long before the end Up Next appears when the item has no outro
/// segment, and the share of a short item's runtime used instead.
const UP_NEXT_LEAD_SECONDS: f64 = 30.0;
const SHORT_ITEM_SECONDS: f64 = 300.0;
const SHORT_ITEM_LEAD_SHARE: f64 = 0.1;
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

pub struct NowPlaying {
    pub item: DetailItemData,
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
            previous: None,
            next: None,
            segments: Vec::new(),
            up_next_dismissed: false,
        }
    }

    /// Whether the end of this item should roll straight into the next.
    pub fn autoplays(&self) -> bool {
        self.next.is_some() && !self.up_next_dismissed
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct SkipPrompt {
    pub label: &'static str,
    pub target: f64,
}

#[derive(Clone, Debug, PartialEq)]
pub struct UpNext {
    pub title: String,
    pub subtitle: String,
    pub seconds_left: f64,
}

/// The floating player UI for one position: at most one skip button and the
/// Up Next card.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct PlayerOverlay {
    pub skip: Option<SkipPrompt>,
    pub up_next: Option<UpNext>,
}

pub fn overlay_at(now: &NowPlaying, position: f64, duration: f64) -> PlayerOverlay {
    let skip = now
        .segments
        .iter()
        .find(|segment| position >= segment.start && position < segment.end - 1.0)
        .and_then(|segment| {
            let label = match segment.kind {
                SegmentKind::Intro => "Skip Intro",
                SegmentKind::Recap => "Skip Recap",
                SegmentKind::Preview => "Skip Preview",
                SegmentKind::Commercial => "Skip Commercial",
                // With a next episode, Up Next takes the credits' place.
                SegmentKind::Outro if now.next.is_none() => "Skip Credits",
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
        .filter(|_| !now.up_next_dismissed && duration > 0.0)
        .filter(|_| up_next_start(now, duration).is_some_and(|start| position >= start))
        .map(|next| UpNext {
            title: next.title.clone(),
            subtitle: episode_number(next),
            seconds_left: (duration - position).max(0.0),
        });
    PlayerOverlay { skip, up_next }
}

/// When the Up Next card appears: at the credits if the server knows where
/// they are, otherwise a fixed lead before the end.
fn up_next_start(now: &NowPlaying, duration: f64) -> Option<f64> {
    let outro = now
        .segments
        .iter()
        .filter(|segment| segment.kind == SegmentKind::Outro && segment.start > 0.0)
        .map(|segment| segment.start)
        .reduce(f64::min);
    if let Some(start) = outro {
        return Some(start);
    }
    let lead = if duration < SHORT_ITEM_SECONDS {
        duration * SHORT_ITEM_LEAD_SHARE
    } else {
        UP_NEXT_LEAD_SECONDS
    };
    Some((duration - lead).max(0.0))
}

fn episode_number(item: &DetailItemData) -> String {
    match (item.parent_index_number, item.index_number) {
        (Some(season), Some(episode)) => format!("S{season}:E{episode}"),
        (None, Some(episode)) => format!("E{episode}"),
        _ => item.series_name.clone().unwrap_or_default(),
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
            Some("Skip Credits")
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
        assert_eq!(up_next.title, "Episode next");
        assert_eq!(up_next.subtitle, "S2:E1");
        assert_eq!(up_next.seconds_left, 100.0);

        let now = playing(Vec::new(), true);
        assert!(overlay_at(&now, 1469.0, 1500.0).up_next.is_none());
        assert!(overlay_at(&now, 1470.0, 1500.0).up_next.is_some());

        // A short item uses a share of its runtime instead.
        assert!(overlay_at(&now, 170.0, 200.0).up_next.is_none());
        assert!(overlay_at(&now, 180.0, 200.0).up_next.is_some());
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
        assert!(matches!(previous_action(&now, 2.0), PreviousAction::Restart));
        now.previous = Some(episode("previous", 1, 1));
        assert!(matches!(
            previous_action(&now, 2.0),
            PreviousAction::Play(item) if item.id == "previous"
        ));
        assert!(matches!(previous_action(&now, 60.0), PreviousAction::Restart));
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
            split_adjacent(vec![episode("a", 1, 1)], "missing").0.map(|item| item.id),
            None
        );
    }
}
