//! Playback session reporting for Jellyfin's `Sessions/Playing` API.
//!
//! The server owns resume points, "currently playing" state and the watched
//! flag, so it only knows where the user got to if this client tells it. mpv
//! drives position events far faster than the server needs them, so the
//! throttle here decides which of those become requests.
//!
//! Everything in this module is pure decision logic, so it is testable without
//! a window or a server: the caller performs the request the decisions ask for.

use std::time::{Duration, Instant};

/// Jellyfin's own web client heartbeats every ten seconds. Matching it keeps
/// the stored resume point within one interval of the player.
pub const PROGRESS_INTERVAL: Duration = Duration::from_secs(10);

/// Jellyfin counts time in 100-nanosecond ticks.
const TICKS_PER_SECOND: f64 = 10_000_000.0;

pub fn position_ticks(seconds: f64) -> i64 {
    if !seconds.is_finite() || seconds <= 0.0 {
        return 0;
    }
    (seconds * TICKS_PER_SECOND).round() as i64
}

/// What the server needs to attribute a report to the right item and session.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ReportTarget {
    pub item_id: String,
    pub media_source_id: String,
    pub play_session_id: Option<String>,
}

/// One report the caller should send.
#[derive(Clone, Debug, PartialEq)]
pub struct ProgressReport {
    pub target: ReportTarget,
    pub position_seconds: f64,
    pub paused: bool,
}

/// Decides which mpv position events are worth a request.
#[derive(Default)]
struct ProgressThrottle {
    last_sent: Option<Instant>,
    last_paused: Option<bool>,
}

impl ProgressThrottle {
    /// Reports on the interval, and immediately whenever the pause state
    /// flips: a pause changes what other clients see, so deferring it for up
    /// to a full interval would leave the server showing the wrong state.
    fn should_send(&mut self, now: Instant, paused: bool) -> bool {
        let paused_changed = self.last_paused != Some(paused);
        let due = self
            .last_sent
            .is_none_or(|last| now.duration_since(last) >= PROGRESS_INTERVAL);
        if !paused_changed && !due {
            return false;
        }
        self.last_sent = Some(now);
        self.last_paused = Some(paused);
        true
    }

    /// Clears the cadence so the next item reports its first position at once.
    fn reset(&mut self) {
        self.last_sent = None;
        self.last_paused = None;
    }
}

/// Tracks what is playing and decides when to tell the server about it.
///
/// The mpv worker reports position and pause state on its own schedule and
/// knows nothing about Jellyfin, so this sits between the two: it remembers
/// the last position seen, and each `note_*` call returns the report to send,
/// or `None` when the event does not warrant a request.
#[derive(Default)]
pub struct PlaybackReporter {
    target: Option<ReportTarget>,
    throttle: ProgressThrottle,
    paused: bool,
    position_seconds: f64,
}

impl PlaybackReporter {
    /// A new item is playing. Returns the start report to send.
    pub fn begin(&mut self, target: ReportTarget, position_seconds: f64) -> ProgressReport {
        self.target = Some(target.clone());
        self.throttle.reset();
        self.paused = false;
        self.position_seconds = position_seconds;
        ProgressReport {
            target,
            position_seconds,
            paused: false,
        }
    }

    /// mpv moved the playhead. Reports on the heartbeat interval only.
    pub fn note_position(&mut self, seconds: f64, now: Instant) -> Option<ProgressReport> {
        if seconds.is_finite() {
            self.position_seconds = seconds;
        }
        // Decide first, then clone: this runs on every mpv position event
        // (4x a second) and the throttle discards all but one report per
        // interval, so cloning the target up front was thrown away 39 times
        // out of 40.
        if !self.throttle.should_send(now, self.paused) {
            return None;
        }
        Some(ProgressReport {
            target: self.target.clone()?,
            position_seconds: self.position_seconds,
            paused: self.paused,
        })
    }

    /// mpv paused or resumed. Always worth a report so other clients and the
    /// dashboard do not sit on a stale state for up to a full interval.
    pub fn note_paused(&mut self, paused: bool, now: Instant) -> Option<ProgressReport> {
        self.paused = paused;
        if !self.throttle.should_send(now, paused) {
            return None;
        }
        Some(ProgressReport {
            target: self.target.clone()?,
            position_seconds: self.position_seconds,
            paused,
        })
    }

    /// Whether an item is playing and has not been reported stopped yet.
    pub fn is_active(&self) -> bool {
        self.target.is_some()
    }

    /// Playback ended or the user left the player. Clears the target so a
    /// stop is reported exactly once, and returns the final position -- which
    /// is what Jellyfin uses to decide whether the item counts as watched.
    pub fn finish(&mut self) -> Option<(ReportTarget, f64)> {
        let target = self.target.take()?;
        let position = self.position_seconds;
        self.throttle.reset();
        self.paused = false;
        self.position_seconds = 0.0;
        Some((target, position))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn target() -> ReportTarget {
        ReportTarget {
            item_id: "item".into(),
            media_source_id: "source".into(),
            play_session_id: Some("session".into()),
        }
    }

    #[test]
    fn converts_seconds_to_ticks_and_clamps_nonsense_positions() {
        assert_eq!(position_ticks(1.0), 10_000_000);
        assert_eq!(position_ticks(14.5), 145_000_000);
        assert_eq!(position_ticks(0.0), 0);
        assert_eq!(position_ticks(-3.0), 0);
        assert_eq!(position_ticks(f64::NAN), 0);
        assert_eq!(position_ticks(f64::INFINITY), 0);
    }

    #[test]
    fn reports_the_first_position_then_throttles_within_the_interval() {
        let mut reporter = PlaybackReporter::default();
        let start = Instant::now();
        reporter.begin(target(), 0.0);
        assert!(reporter.note_position(1.0, start).is_some());
        assert!(
            reporter
                .note_position(2.0, start + Duration::from_secs(1))
                .is_none()
        );
        assert!(
            reporter
                .note_position(9.0, start + Duration::from_secs(9))
                .is_none()
        );
        assert!(
            reporter
                .note_position(10.0, start + PROGRESS_INTERVAL)
                .is_some()
        );
    }

    #[test]
    fn reports_a_pause_flip_without_waiting_for_the_interval() {
        let mut reporter = PlaybackReporter::default();
        let start = Instant::now();
        reporter.begin(target(), 0.0);
        assert!(reporter.note_position(1.0, start).is_some());

        let paused = reporter
            .note_paused(true, start + Duration::from_secs(1))
            .expect("a pause must report at once");
        assert!(paused.paused);
        // Still paused a second later: nothing new to say.
        assert!(
            reporter
                .note_position(1.0, start + Duration::from_secs(2))
                .is_none()
        );
        assert!(
            reporter
                .note_paused(false, start + Duration::from_secs(3))
                .is_some()
        );
    }

    #[test]
    fn nothing_is_reported_before_playback_begins_or_after_it_finishes() {
        let mut reporter = PlaybackReporter::default();
        let start = Instant::now();
        assert!(reporter.note_position(5.0, start).is_none());
        assert!(reporter.note_paused(true, start).is_none());
        assert!(reporter.finish().is_none());

        reporter.begin(target(), 0.0);
        reporter.note_position(30.0, start);
        let (finished, position) = reporter.finish().expect("a stop must be reported");
        assert_eq!(finished, target());
        assert_eq!(position, 30.0);
        // Stopping twice must not report twice.
        assert!(reporter.finish().is_none());
        assert!(
            reporter
                .note_position(31.0, start + PROGRESS_INTERVAL)
                .is_none()
        );
    }

    #[test]
    fn the_stop_report_carries_the_last_position_seen() {
        let mut reporter = PlaybackReporter::default();
        let start = Instant::now();
        reporter.begin(target(), 12.0);
        // Positions between heartbeats are still remembered, so leaving the
        // player mid-interval commits where the user actually got to.
        reporter.note_position(41.5, start + Duration::from_secs(1));
        assert_eq!(reporter.finish().expect("stop").1, 41.5);
    }

    #[test]
    fn a_new_item_reports_its_first_position_immediately() {
        let mut reporter = PlaybackReporter::default();
        let start = Instant::now();
        reporter.begin(target(), 0.0);
        assert!(reporter.note_position(1.0, start).is_some());
        reporter.finish();

        reporter.begin(target(), 0.0);
        assert!(
            reporter
                .note_position(1.0, start + Duration::from_secs(1))
                .is_some(),
            "the next item must not inherit the previous item's cadence"
        );
    }
}
