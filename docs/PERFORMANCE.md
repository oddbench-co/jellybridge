# Performance: what was slow, how it was found, what fixed it

The performance work on the egui app, written to be learned from. Each item
names the symptom, the measurement that pinned it down, the fix and why it
works. `docs/rust-port/performance-protocol.md` is the Slint-era protocol and
is kept for history.

All numbers are from one machine and one setup: Ryzen 9 7950X, RTX 3090
(NVIDIA 610 driver), Linux with Hyprland, the window on a 3840×2160 240 Hz
output at scale 1.5 (2536×1390 points), libmpv 2.5 (mpv 0.41), and a Jellyfin
server on the LAN. Playback numbers use one 4K HEVC 10-bit episode
(Bookish S1:E1) unless stated otherwise.

## The method

1. **Measure before changing anything, with a reproducible scenario.** Every
   change below started from a `JELLYBRIDGE_MEASURE` run, not a guess. Several
   "obvious" fixes measured as no gain and were dropped (see *Measured and
   rejected*).
2. **Look per thread and per mapping, not at the process.** The report lists
   the busiest threads; `/proc/<pid>/smaps` grouped by mapping explained every
   memory finding (driver mappings, heap, arenas, libraries).
3. **Measure on the screen people use.** At 240 Hz a frame has 4.2 ms. A 7 ms
   frame is invisible at 60 Hz and a dropped frame at 240 Hz, so the test
   output matches the real one.
4. **Never trade UX for a number.** A memory saving that froze the UI for
   300 ms was reworked until the freeze was gone, not accepted.
5. **Leave the measurement in the tooling.** Each change left a log line or a
   scenario behind, so a regression shows up in a script.

## The numbers

| | before | now |
|---|---|---|
| Launch to content (`total_ms`, median) | 302 ms | 102 ms (the last Home from disk); the server's Home lands at ~285 ms |
| Before `main` (dynamic linking) | 70–80 ms | 0 ms |
| Memory while browsing (RSS) | 166–171 MB | 117–118 MB |
| Frames drawn while idle on Home | 588 per 10 s | ~20 per 10 s (launch only) |
| Wake-ups while idle, whole app | 624 per 10 s | 44 per 10 s |
| Scrolling at 240 Hz: frames over budget | 0 | 0 (UI 0.33 ms per frame, p95) |
| Playback, UI thread per frame (p95) | 41 ms | 0.5 ms |
| Play → first video frame | 448–471 ms | 205–215 ms |
| UI freeze while the player starts | ~335 ms | ~100 ms |
| Memory during 4K playback | ~711 MB | 635 MB |
| CPU during 4K playback | ~8.5 % | ~8.7 % |
| Memory after leaving the player | 542 MB | 194 MB |
| Second playback: building the player | — | 2.6 ms |

The jellyfin-web parity work (October 2026) was measured against the build
before it (`aefed8f`), interleaved run for run: launch 262 → 102 ms, scrolling
UI per frame 0.23 → 0.33 ms p95 (jellyfin's layout draws more, still a tenth
of the 240 Hz budget), browsing memory 115 → 118 MB (jellyfin's larger cards
and its Continue Listening/Reading shelves hold more artwork), playback and
after-player memory unchanged.

## What changed, and why it works

### libmpv is loaded at runtime, on first play (`services/libmpv.rs`)

*Symptom.* 70–80 ms passed before `main` on every launch.
*Measurement.* An empty C program linked against libmpv starts in 74.5 ms;
unlinked, in 0.3 ms. `perf` put 74.5 % of that inside `ld-linux`: libmpv pulls
in 206 libraries (ffmpeg, libplacebo, codecs), Arch builds them with immediate
binding, and the loader resolves every symbol of all of them. `dlopen` of
libmpv alone: 71 ms and 86 MB resident.
*Fix.* The app no longer links libmpv (the binary needs 5 libraries instead
of 206). A small FFI layer declares the ~25 functions used and resolves them
with `dlopen` the first time playback is asked for. Pressing Play sends
`PlayerCommand::Prepare` before any server request, so the load overlaps the
PlaybackInfo round trip.
*Result.* Browsing RSS 170 → 119 MB, launch ~70 ms faster, and the first
play is not slower: 445–467 ms to the first frame against 456–465 ms with
the library loaded at startup.

### One hardware-decode interop instead of all (`render::preferred_hwdec_interop`)

*Symptom.* Building mpv's render context took ~330 ms, on the UI thread
inside a paint, so the loading spinner froze.
*Measurement.* mpv's debug log: 242 ms opening the VAAPI driver
(`nvidia_drv_video.so`) and ~70 ms for VDPAU, on a machine that then decodes
through CUDA. In the render API, `gpu-hwdec-interop=auto` means "load every
interop now" (`ra_hwdec_ctx_init(..., load_all_by_default = true)`).
*Fix.* On NVIDIA's driver (`GL_VENDOR` starts with "NVIDIA") mpv is told to
load only `cuda`, the one `hwdec=auto-safe` uses there. A named interop is the
only one mpv will ever load, so this is not done where the right choice is
uncertain; copy-back and software decoding need none.
*Result.* Render context 330 → 100 ms, first frame 455 → 210 ms, playback
RSS down ~70 MB, CPU 8.5 → 7.1 %. Still `nvdec`, 0 dropped frames.

### After the player is left, mpv is released (`RenderHost::release_soon`)

*Symptom.* 542 MB stayed resident after leaving a 4K film, against 119 MB
before it.
*Measurement.* smaps: 180 MB of freed heap glibc kept, 149 MB of NVIDIA
driver mappings (the render context's CUDA interop and buffers), and later
190 MB of live allocations in glibc's second arena (decoder pools, the CUDA
device) held by the mpv core.
*Fix, in three parts.*
1. When a file ends with nothing queued, the worker calls `malloc_trim`
   (heap 180 → 10 MB).
2. The render context and its framebuffer are freed once the player is left.
   The next playback builds them again (~100 ms, as the first does) and waits
   for them as the first does.
3. Then the worker destroys the mpv core (`PlayerCommand::Unload`) and goes
   back to having no mpv. libmpv stays loaded, so rebuilding is ~2.6 ms.
*Two UX traps found on the way.* Freeing the context right after `stop` froze
the UI for 313 ms: mpv's video output still held a last frame and waited for
it to be rendered (log: `mpv_render_context_render() not being called or
stuck`). The release now waits for the worker's idle signal and renders that
last frame first, which cut it to ~130 ms. That remaining 130 ms is mpv
tearing down CUDA on the GL thread, so the release also waits for 5 seconds
without input: it lands on a still screen, never on a scroll or a click.
*Result.* 542 → 255 MB after leaving; repeated plays settle at the same level.

### The mpv worker sleeps (`services/player.rs`)

*Symptom.* The worker thread woke every 20 ms for the life of the app.
*Measurement.* Voluntary context switches: 52 per second on the mpv thread
alone, 624 per 10 s for the whole app while idle.
*Fix.* The worker blocks inside `mpv_wait_event`; commands are queued and
interrupt it with `mpv_wakeup`. The position is polled (4 Hz) only while a
file plays. A first version used mpv's wakeup callback and intermittently
missed `FILE_LOADED`; the wait-plus-`mpv_wakeup` pattern is mpv's documented
one and resets its own state on every wait.
*Result.* 44 wake-ups per 10 s in total, most of them accessibility (AT-SPI)
and the Wayland clipboard thread, which are not ours.

### mpv never blocks the UI thread (`RenderHost::draw`)

*Symptom.* During playback the UI thread spent 41 ms of every frame.
*Measurement.* `frameCpuMs` p95 41 ms: `mpv_render_context_render` blocked
until each frame's display time.
*Fix.* `BLOCK_FOR_TARGET_TIME = 0`. A frame that is not due yet stays
pending; the wake thread asks for a repaint at its target time
(`NEXT_FRAME_INFO`, which libmpv 2.x reports in nanoseconds).
*Result.* p95 0.6 ms per frame while playing.

### glibc arenas (`main.rs::tune_allocator`)

`M_ARENA_MAX=2` and a fixed `M_MMAP_THRESHOLD` of 256 KiB: ~199 → ~170 MB on
the home screen at the time. mimalloc was measured as an alternative (see
below).

### A known address is never waited on (`services/dns.rs`)

*Symptom.* Some launches stalled 1–5 s on their first requests.
*Measurement.* The `dns lookup` log line caught it: `jellyfin.local` took 5.3 s
and 1.1 s in two of 38 launches (the resolver's retry after a lost UDP
reply), and the whole first screen waited on it. A normal lookup is still
25–40 ms on the critical path, and an expired entry made the next artwork
request wait for a lookup mid-browse.
*Fix.* Stale-while-revalidate: a known address is returned at once and
refreshed in the background, and the addresses are kept in
`~/.cache/jellybridge/dns.json` (trusted for a week), so launch connects
without a lookup.
*Result.* Launch median 290 → 254 ms (8 launches), and the 5 s stall cannot
hit a launch or a page once the server has been seen.

### Home's music shelf loads after the first screen (`JellyfinApi::deferred_home_sections`)

*Symptom.* After My Media started listing every library, as jellyfin-web
does, launch was ~15 ms slower (median of 16 interleaved A/B launches).
*Measurement.* The client's own share (launch minus Next Up) was unchanged at
10–11 ms; Next Up, the slowest request, got slower on the server while the
new "Recently Added in Music" query ran beside it.
*Fix.* Music libraries' Latest shelves are fetched after the home is shown
and slotted into their place; they sit below the first screen.

### The last Home is painted at launch (`services/home_snapshot.rs`)

*Symptom.* Launch waited on the server: content appeared when the slowest
dashboard request (Next Up) ended, 250–300 ms after start. jellyfin-web's
Home adds Continue Listening, Continue Reading and the user's display
preferences, three more requests that made every one of them slower
(median 252 → 302 ms against the build before).
*Measurement.* `startup_stage` lines: the window was ready at ~92 ms and then
waited ~200 ms for `home_arrived`. The requests already ran in parallel, so
the cost was the server's, not the client's.
*Fix.* Every Home the server returns is saved beside the artwork cache
(`home.json`, ~64 KB, per server and user, trusted for a week, removed at
sign-out). At launch it is read while the window is built and painted as soon
as the window exists; the request still runs, and its Home replaces the saved
one in place, without moving the page or pulling a user back who already left
Home. When nothing changed, the usual case, the two compare equal and nothing
is redrawn or saved. A failed refresh leaves the saved Home showing, so a
server that is down still shows the library. The library tiles' item counts
are dropped: the server's value changes from one request to the next and no
tile shows it, and it made every saved Home look stale.
*Result.* Launch to content 262 → 102 ms (median of six interleaved launches);
the fresh Home is confirmed ~180 ms later with no visible change.

### Artwork reaches the GPU a megabyte a frame (`ImageCache::upload_ready`)

*Symptom.* With the larger Home, frames over the 240 Hz budget (4.2 ms)
appeared: 5–8 ms when a shelf scrolled into view or the music shelf arrived,
20 ms at launch.
*Measurement.* The slow frames lined up with decoded images arriving in
bursts; an upload with its mipmap chain costs about 2 ms per megabyte, and
egui uploads everything registered in a frame at the end of it.
*Fix.* Decoded images wait in a queue, still marked pending so they are not
requested twice; each frame uploads up to 1 MB of them (always at least
one), and asks for another frame while any wait. A shelf's artwork appears
over a few frames instead of in one long one.
*Result.* No frame over budget while scrolling or when shelves arrive; the
launch's worst frame 20 → 10 ms (the first frame's font atlas and layout).

### Idle redraws

The build before the parity work redrew ~60 times a second while idle (588
frames in 10 s, egui's scroll-area animation asking for repaints). The page
scrollbar is now drawn by the app (`ui/scrollbar.rs`, Chrome's look), and an
idle Home draws nothing after launch.

## Measured and rejected

Each was measured and lost; the reason is kept so it is not tried again
blindly.

- **mimalloc** (mytele uses it): ~7 % less CPU while scrolling, but 192–292
  MB resident against ~170 MB.
- **Smaller mpv demuxer cache** (64 or 96 MiB ahead): within run-to-run noise
  on these files; mpv's defaults stay.
- **`hwdec-extra-frames=2`**: ~50 MB less in 4K playback, but mpv documents
  that too few frames can corrupt or drop video on other content.
- **A texture-upload budget per frame**: scrolling at 240 Hz has no frame over
  budget; the only slow frames are the first two after launch.
- **Removing the second repaint per video frame**: `ADVANCED_CONTROL` requires
  `mpv_render_context_update` on the GL thread after every update callback,
  and the cost is ~2.5 % of one core.
- **jellyfin-web's Next Up parameters**: no faster on the server
  (~120–135 ms either way).
- **Tooltip repaints**: they looked like a loop in the counts, but a resting
  pointer costs ~0.5 % CPU and no extra frames. The counts were per-widget
  causes inside frames that happen anyway.

## Still open

- The render context's ~100 ms creation still runs on the UI thread at the
  start of playback, while a spinner shows.
- After playback ~75 MB more than a fresh launch stays mapped: mpv's and
  CUDA's libraries, which are not unloaded once loaded.
- The first two frames after launch take 9–10 ms (font atlas and the first
  layout), before anything can be clicked.
- Shelves load one page of artwork beyond each edge so paging shows images
  at once; on Home that is ~40 MiB of textures, most of it GPU memory.

## Tooling

All of this works in release builds.

| Variable | What it does |
|---|---|
| `JELLYBRIDGE_MEASURE=<seconds>` | Measures from the first frame with content for that long, prints a JSON report and closes: frame intervals, UI-thread CPU per frame and frames over a 240/120 Hz refresh (with their times), layout time, repaint causes, video fps, process CPU, busiest threads, RSS and peak. |
| `JELLYBRIDGE_INPUT_SCRIPT=<file>` | Replays timed input (`move`, `click`, `scroll`, `key`, `text`), `play <item id>` and page steps (`open`, `detail`, `settings`, `search`, `tab`, `back`), and `shot <file>` saves the frame, so a scenario plays exactly the item it names. |
| `JELLYBRIDGE_MUTE=1` | mpv outputs no audio (`ao=null`). |
| `JELLYBRIDGE_NO_PLAYBACK_REPORTS=1` | Never reports playback to the server, so test runs cannot move anyone's resume points. |
| `JELLYBRIDGE_MPV_OPTIONS=a=b,c=d` | Any extra mpv option, applied last (cache sizes, `log-file=…`, `msg-level=all=debug`). |
| `JELLYBRIDGE_HWDEC=no` | Forces software decoding. |

Log lines (`RUST_LOG=jellybridge=debug`): `time_to_interactive` (launch, with
`exec_to_main_ms` for the time before `main`), `startup_stage`,
`dashboard_request`, `home_snapshot_shown`, `home_unchanged` / `home_refreshed`, `dns lookup`, `libmpv loaded`, `libmpv player built`,
`mpv render context created`, `time_to_first_frame`, `libmpv dropped frames`
(at every stop), `mpv render context released`, `libmpv player unloaded`.

Run tests on a display nobody is using, muted and without playback reports.
For representative numbers, give it the real screen's mode, for example a
Hyprland headless output:

    hyprctl output create headless JBTEST
    hyprctl eval 'hl.monitor({output="JBTEST", mode="3840x2160@240", position="10000x0", scale=1.5})'

`scripts/parity/app-shot.sh` does all of this for one run (headless output
on workspace 19, muted, no reports, the parity test session); pass
`EXTRA="JELLYBRIDGE_MEASURE=10"` for a report and `BIN=` for another build,
so two builds can be measured interleaved.
