//! `JELLYBRIDGE_MEASURE=SECONDS`: measures the running window for that long,
//! starting once the first screen with content is on screen, then logs a
//! JSON report and closes. Paired with `JELLYBRIDGE_INPUT_SCRIPT` it measures
//! a reproducible scenario: idle home, scrolling, a detail page, playback.
//!
//! Reported: `frames` (time between frames), `layoutMs` (the app's own work
//! per frame), `frameCpuMs` (eframe's whole UI-thread time per frame,
//! tessellation and painting included), the redraw causes, process CPU time
//! (all threads) and the busiest threads, and resident / peak memory. A frame
//! that keeps the UI thread busy longer than a display refresh is a visible
//! stutter; CPU per second is what an idle or playing window costs.

use std::collections::BTreeMap;
use std::time::{Duration, Instant};

pub struct Measure {
    seconds: f64,
    started: Option<Instant>,
    last: Option<Instant>,
    cpu_start: f64,
    video_start: u64,
    intervals: Vec<f64>,
    layout: Vec<f64>,
    frame_cpu: Vec<f64>,
    causes: BTreeMap<String, u64>,
    /// Frames over a 240 Hz refresh: seconds into the run and UI-thread ms.
    slow: Vec<(f64, f64)>,
    pub done: bool,
}

impl Measure {
    pub fn from_env() -> Option<Self> {
        let seconds = std::env::var("JELLYBRIDGE_MEASURE")
            .ok()?
            .parse::<f64>()
            .ok()?;
        Some(Self {
            seconds,
            started: None,
            last: None,
            cpu_start: 0.0,
            video_start: 0,
            intervals: Vec::new(),
            layout: Vec::new(),
            frame_cpu: Vec::new(),
            causes: BTreeMap::new(),
            slow: Vec::new(),
            done: false,
        })
    }

    /// Records one frame. `ready` is whether content is on screen yet.
    pub fn frame(
        &mut self,
        ctx: &egui::Context,
        ready: bool,
        layout: Duration,
        frame_cpu: Option<f32>,
    ) {
        if self.done {
            return;
        }
        let now = Instant::now();
        let Some(started) = self.started else {
            if ready {
                self.started = Some(now);
                self.cpu_start = process_cpu_seconds();
                self.video_start = video_frames();
                self.last = Some(now);
                // One wake-up for the end of the run; nothing else is added,
                // so an idle window's own redraw rate is what is measured.
                ctx.request_repaint_after(Duration::from_secs_f64(self.seconds));
            }
            return;
        };
        let previous = self.last.replace(now);
        if let Some(last) = previous {
            self.intervals.push((now - last).as_secs_f64() * 1000.0);
        }
        self.layout.push(layout.as_secs_f64() * 1000.0);
        // eframe reports a frame's CPU time in the next frame's info, so a
        // slow frame is dated by the frame before this one.
        if let Some(cpu) = frame_cpu {
            let ms = f64::from(cpu) * 1000.0;
            self.frame_cpu.push(ms);
            if ms > 1000.0 / 240.0 && self.slow.len() < 32 {
                let at = previous.unwrap_or(now).saturating_duration_since(started);
                self.slow.push((at.as_secs_f64(), ms));
            }
        }
        for cause in ctx.repaint_causes() {
            *self.causes.entry(cause.to_string()).or_default() += 1;
        }
        let elapsed = (now - started).as_secs_f64();
        if elapsed < self.seconds {
            ctx.request_repaint_after(Duration::from_secs_f64(self.seconds - elapsed));
            return;
        }
        self.done = true;
        let cpu = process_cpu_seconds() - self.cpu_start;
        let long = self.layout.iter().filter(|&&ms| ms > 1000.0 / 60.0).count();
        // UI-thread time over one refresh at 240 Hz and 120 Hz: on such a
        // display, a frame that busy is a skipped refresh.
        let over = |budget: f64| self.frame_cpu.iter().filter(|&&ms| ms > budget).count();
        let over_240hz = over(1000.0 / 240.0);
        let over_120hz = over(1000.0 / 120.0);
        let report = serde_json::json!({
            "seconds": elapsed,
            "uiFps": self.intervals.len() as f64 / elapsed,
            "frames": stats(&mut self.intervals),
            "layoutMs": stats(&mut self.layout),
            "frameCpuMs": stats(&mut self.frame_cpu),
            "framesOver16ms": long,
            "frameCpuOver240Hz": over_240hz,
            "frameCpuOver120Hz": over_120hz,
            "slowFrames": self.slow,
            "processCpuPercent": cpu / elapsed * 100.0,
            "videoFps": (video_frames() - self.video_start) as f64 / elapsed,
            "threads": busiest_threads(),
            "rssMb": status_kb("VmRSS:") / 1024.0,
            "peakRssMb": status_kb("VmHWM:") / 1024.0,
            "pixelsPerPoint": ctx.pixels_per_point(),
            "window": [ctx.content_rect().width(), ctx.content_rect().height()],
            "repaintCauses": self.causes,
        });
        tracing::info!(report = %report, "measure");
        println!("{report}");
        ctx.send_viewport_cmd(egui::ViewportCommand::Close);
    }
}

fn video_frames() -> u64 {
    crate::services::render::VIDEO_FRAMES.load(std::sync::atomic::Ordering::Relaxed)
}

fn stats(values: &mut [f64]) -> serde_json::Value {
    values.sort_by(f64::total_cmp);
    let mean = values.iter().sum::<f64>() / values.len().max(1) as f64;
    let at = |q: f64| {
        values
            .get(((values.len() as f64 * q) as usize).min(values.len().saturating_sub(1)))
            .copied()
            .unwrap_or(0.0)
    };
    serde_json::json!({
        "count": values.len(),
        "meanMs": mean,
        "p50Ms": at(0.5),
        "p95Ms": at(0.95),
        "p99Ms": at(0.99),
        "maxMs": values.last().copied().unwrap_or(0.0),
    })
}

fn ticks_per_second() -> f64 {
    // SAFETY: sysconf with a valid name is a pure read.
    #[cfg(unix)]
    let ticks = unsafe { libc::sysconf(libc::_SC_CLK_TCK) };
    // No /proc to read elsewhere; the CPU figures stay zero.
    #[cfg(not(unix))]
    let ticks = 100;
    if ticks > 0 { ticks as f64 } else { 100.0 }
}

/// utime + stime of a `/proc/.../stat` line, in seconds.
fn stat_cpu_seconds(stat: &str) -> Option<f64> {
    let after_comm = &stat[stat.rfind(')')? + 1..];
    let mut fields = after_comm.split_whitespace().skip(11);
    let user: f64 = fields.next()?.parse().ok()?;
    let system: f64 = fields.next()?.parse().ok()?;
    Some((user + system) / ticks_per_second())
}

fn process_cpu_seconds() -> f64 {
    std::fs::read_to_string("/proc/self/stat")
        .ok()
        .and_then(|stat| stat_cpu_seconds(&stat))
        .unwrap_or(0.0)
}

/// CPU seconds per thread since it started, busiest first (top 8).
fn busiest_threads() -> Vec<(String, f64)> {
    let Ok(tasks) = std::fs::read_dir("/proc/self/task") else {
        return Vec::new();
    };
    let mut threads = tasks
        .flatten()
        .filter_map(|task| {
            let stat = std::fs::read_to_string(task.path().join("stat")).ok()?;
            let name = stat[stat.find('(')? + 1..stat.rfind(')')?].to_owned();
            Some((name, stat_cpu_seconds(&stat)?))
        })
        .collect::<Vec<_>>();
    threads.sort_by(|a, b| b.1.total_cmp(&a.1));
    threads.truncate(8);
    threads
}

fn status_kb(key: &str) -> f64 {
    std::fs::read_to_string("/proc/self/status")
        .ok()
        .and_then(|status| {
            status
                .lines()
                .find(|line| line.starts_with(key))?
                .split_whitespace()
                .nth(1)?
                .parse()
                .ok()
        })
        .unwrap_or(0.0)
}
