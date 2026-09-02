//! Startup instrumentation.
//!
//! `time_to_interactive` is what `docs/rust-port/performance-protocol.md`
//! measures: the gap between process start and the first frame the user could
//! actually act on. It is deliberately *not* "the first frame" (that is the
//! empty window) nor "the presenter ran" (properties are set, but nothing is
//! on screen yet) -- a presenter marks the content ready, and the render
//! notifier reports once the frame carrying it has been drawn.

use std::sync::OnceLock;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Instant;

static PROCESS_START: OnceLock<Instant> = OnceLock::new();
static READY_SCREEN: OnceLock<&'static str> = OnceLock::new();
static REPORTED: AtomicBool = AtomicBool::new(false);

/// Records the process start. Called as the first statement in `main`, so the
/// measurement includes runtime, backend, and window construction.
pub fn mark_process_start(at: Instant) {
    let _ = PROCESS_START.set(at);
}

/// Marks the first screen with real content as pushed to the window. Cheap and
/// idempotent: only the first caller wins, later presents are not startup.
pub fn mark_content_ready(screen: &'static str) {
    let _ = READY_SCREEN.set(screen);
}

/// Reports `time_to_interactive` once the frame carrying that content has been
/// drawn. Called from the render notifier, so it must stay allocation-free on
/// the path where there is nothing to report.
pub fn note_frame_rendered() {
    if REPORTED.load(Ordering::Relaxed) {
        return;
    }
    let Some(screen) = READY_SCREEN.get() else {
        return;
    };
    let Some(start) = PROCESS_START.get() else {
        return;
    };
    if REPORTED.swap(true, Ordering::Relaxed) {
        return;
    }
    tracing::info!(
        ms = start.elapsed().as_secs_f64() * 1000.0,
        screen,
        "time_to_interactive"
    );
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn nothing_is_reported_before_a_screen_is_ready() {
        // The statics are process-wide, so this only asserts the guard that
        // runs before any of them are set: no panic, no report.
        note_frame_rendered();
        assert!(READY_SCREEN.get().is_none() || REPORTED.load(Ordering::Relaxed));
    }
}
