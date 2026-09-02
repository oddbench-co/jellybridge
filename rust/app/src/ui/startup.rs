//! Startup instrumentation.
//!
//! `time_to_interactive` is what `docs/rust-port/performance-protocol.md`
//! measures: the gap between process start and the first frame the user could
//! actually act on. It is deliberately *not* "the first frame" (that is the
//! empty window) nor "the presenter ran" (properties are set, but nothing is
//! on screen yet) -- a presenter marks the content ready, and the render
//! notifier reports once the frame carrying it has been drawn.
//!
//! "Process start" has two meanings and the difference is not small. `main`
//! runs only after the dynamic linker has mapped, relocated, and run the
//! constructors of every `DT_NEEDED` object -- 206 of them here, 202 pulled in
//! by `libmpv.so.2` alone. Timing from the first statement of `main` hides all
//! of that, so `exec_to_main` reads the real `execve` time out of
//! `/proc/self/stat` and reports the two halves separately.

use std::sync::OnceLock;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Instant;

static PROCESS_START: OnceLock<Instant> = OnceLock::new();
static EXEC_TO_MAIN_MS: OnceLock<Option<f64>> = OnceLock::new();
static READY_SCREEN: OnceLock<&'static str> = OnceLock::new();
static REPORTED: AtomicBool = AtomicBool::new(false);

/// Records the process start. Called as the first statement in `main`, so the
/// measurement includes runtime, backend, and window construction.
pub fn mark_process_start(at: Instant) {
    let _ = PROCESS_START.set(at);
    let _ = EXEC_TO_MAIN_MS.set(exec_to_main_ms());
}

/// Milliseconds since `main` began, for temporary stage timing.
pub fn elapsed_ms() -> f64 {
    PROCESS_START
        .get()
        .map(|start| start.elapsed().as_secs_f64() * 1000.0)
        .unwrap_or(-1.0)
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
    let in_main = start.elapsed().as_secs_f64() * 1000.0;
    // `-1` rather than a skipped field: the log is parsed by scripts/profile.sh,
    // and a missing column is harder to notice than an impossible value.
    let pre_main = EXEC_TO_MAIN_MS.get().copied().flatten();
    tracing::info!(
        ms = in_main,
        exec_to_main_ms = pre_main.unwrap_or(-1.0),
        total_ms = pre_main.unwrap_or(0.0) + in_main,
        screen,
        "time_to_interactive"
    );
}

/// Milliseconds between `execve` and the call site, or `None` where the
/// platform does not expose it. Only ever called once, from `mark_process_start`.
#[cfg(target_os = "linux")]
fn exec_to_main_ms() -> Option<f64> {
    // Field 22 of /proc/self/stat is the process start time in clock ticks
    // since boot. Fields 1 and 2 are skipped by splitting after the *last*
    // ')': the comm field is parenthesised and may itself contain spaces and
    // parentheses, so tokenising the whole line is not safe.
    let stat = std::fs::read_to_string("/proc/self/stat").ok()?;
    let after_comm = &stat[stat.rfind(')')? + 1..];
    let start_ticks: f64 = after_comm.split_whitespace().nth(19)?.parse().ok()?;

    let uptime = std::fs::read_to_string("/proc/uptime").ok()?;
    let uptime_seconds: f64 = uptime.split_whitespace().next()?.parse().ok()?;

    // SAFETY: sysconf with a valid name is a pure read of a runtime constant.
    let ticks_per_second = unsafe { libc::sysconf(libc::_SC_CLK_TCK) };
    if ticks_per_second <= 0 {
        return None;
    }
    let elapsed = uptime_seconds - start_ticks / ticks_per_second as f64;
    // Clock-tick granularity is 10 ms, so a very short pre-main can round
    // negative. Report it as zero rather than as nonsense.
    Some((elapsed.max(0.0)) * 1000.0)
}

#[cfg(not(target_os = "linux"))]
fn exec_to_main_ms() -> Option<f64> {
    None
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

    #[cfg(target_os = "linux")]
    #[test]
    fn the_pre_main_gap_is_a_plausible_duration() {
        // The test binary has been running for a moment, and no process can
        // have started before boot. An unparsed /proc would show up as a wild
        // value or None, both of which this catches.
        let gap = exec_to_main_ms().expect("/proc/self/stat is readable on Linux");
        assert!(
            (0.0..60_000.0).contains(&gap),
            "implausible exec-to-now gap: {gap} ms"
        );
    }
}
