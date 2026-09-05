# Performance Protocol

Compare release packages on the same physical machine, power mode, account,
server snapshot, window size, and network. Debug builds are invalid.

## Scenarios

1. Cold sign-in screen with no saved session.
2. Cold launch to interactive Home with an empty artwork cache.
3. Cold launch to interactive Home with a warm encoded-artwork cache.
4. Settled Home RSS after 30 seconds without interaction.
5. Peak RSS while scrolling the representative 100-card grid.
6. Repeated Home to detail page navigation, measuring CPU per round trip.
7. Steady-state playback of a 2160p HEVC item, playing and paused.
8. Browse into the largest library and scroll its grid end to end.

## Measurement

- Add an explicit structured `time_to_interactive` marker to both candidates.
- Run 15 paired, alternating launches per scenario.
- Fully terminate the prior process between launches.
- Report median, p95, range, peak RSS, and settled RSS.
- Record OS build, CPU, RAM, display scale, package hash, renderer, libmpv
  graph hash, server snapshot, cache mode, and power mode.
- Release requires at least 30% lower median cold-start and settled Home RSS.

## Harness

`scripts/profile.sh` builds the `profiling` cargo profile (release codegen with
symbols, in its own target directory) and runs the app with Slint's frame
metrics on, sampling `VmRSS` into a CSV:

    scripts/profile.sh run  [label]              # frame metrics + RSS/PSS
    scripts/profile.sh cpu  [label]              # the same, under samply
    scripts/profile.sh --cold-artwork --duration 20 run cold

The `time_to_interactive` marker (`src/ui/startup.rs`) is reported once, when
the first frame carrying real content has actually been drawn -- not when the
presenter ran. It reports two halves: `exec_to_main_ms`, read from
`/proc/self/stat`, covers the dynamic linker, and `ms` covers everything after
`main` begins. Timing only the second half understated the launch by a fifth.

Memory is sampled as both RSS and PSS, and one settled per-library breakdown
lands in `maps.txt`. RSS charges every resident page of a shared mapping to
us, and most of this process is the GPU driver, so RSS alone is not a measure
of anything this code controls.

Artifacts land in `target/profiling-runs/<label>-<stamp>/`. Run with
`RUST_LOG=jellybridge=debug` (the harness default) for per-stage startup
timings and per-request dashboard timings.

samply needs `kernel.perf_event_paranoid <= 1` and a
`kernel.perf_event_mlock_kb` above the 516 KiB default; both are runtime
sysctls and neither persists across a reboot.

## Current result

Measured 2026-09-02 on Arch Linux (kernel 7.1.9), Ryzen 9 7950X, 93 GiB RAM,
RTX 3090 (nvidia 610.57.04) under Hyprland/Wayland at 3840x2160 with a 1.5
scale, femtovg renderer, libmpv 2.5.0, against a Jellyfin 10.11.11 server on
the LAN (55 ms to `/System/Info/Public`). Library: 30 movies, 91 series / 885
episodes. Medians of three runs.

| Scenario | Before | After |
|---|---|---|
| `time_to_interactive`, warm artwork cache | 576 ms | 300 ms |
| `time_to_interactive`, cold artwork cache | 528 ms | 300 ms |
| Settled Home RSS | 540 MiB | 352 MiB |
| CPU per Home-to-detail round trip | 125 ms | 82-96 ms |
| mpv framebuffer height (1261x1390 logical window) | 1917 px | 2085 px |
| Playback CPU, 2160p HEVC | 4.5% | 4.5% |
| Playback CPU, paused | 0.0% | 0.0% |

What produced the changes:

- **`time_to_interactive`, -43%.** `home()` awaits all six dashboard requests
  before anything is presented, so the slowest one gates the first frame. The
  list field set carried `Overview` and `MediaStreams`, which nothing on a
  catalog row reads -- `CatalogItemData` has no overview, and the media summary
  is built only for detail pages, which fetch `DETAIL_FIELDS` separately.
  Dropping both took the six responses from ~900 KiB to ~75 KiB and roughly
  halved the slowest request:

  | request | before | after |
  |---|---|---|
  | recent | 283 ms / 297 KiB | 165 ms / 20 KiB |
  | latest | 173 ms / 323 KiB | 64 ms / 21 KiB |
  | next_up | 196 ms / 153 KiB | 142 ms / 23 KiB |
  | resume | 130 ms / 124 KiB | 82 ms / 10 KiB |

  Cold and warm artwork caches now measure the same, because startup was never
  artwork-bound.
- **Settled RSS, -35%.** `RenderHost::setup` used to build mpv's GL render
  context during `RenderingSetup`, at startup, for a screen the user had not
  opened. It cost 310 ms of CPU and ~187 MiB of RSS. `RenderHost::ensure_mpv`
  now builds it on the first frame that needs it. It does not move
  `time_to_interactive`, because that 310 ms overlapped the Home network
  fan-out rather than blocking the first frame.
- **CPU per navigation, -34%.** The artwork loaders spawned a fetch task per
  row on every present without consulting the in-memory image cache that the
  presenters read from, so every re-present re-ran probe-fetch-decode for
  images already on screen.
- **Video resolution.** `video-logical-height` subtracted 112 px for bars that
  are overlays drawn on top of a full-height surface, so `image-fit: contain`
  letterboxed away that much real resolution.

The Home-to-detail figure is noisy run to run (82, 82, 93, 96 ms across
repeats of the same six-cycle script), so treat anything under about 15% on
that metric as unresolvable.

Changes made for correctness or removed work, which did **not** move any
measured number and are not claimed as wins:

- **Section model patched instead of replaced** (`ui/catalog.rs`).
  `catalog-screen.slint` iterates `sections` to build every CatalogShelf and
  its MediaTiles, so replacing the model destroyed and rebuilt all of them and
  threw away the `patch_rows` applied to `catalog_items` immediately after.
  Also removes two of the three deep clones of the row vector per present.
  The navigation metric did not change.
- **Series pages fetch one episode image instead of all of them**
  (`ui/detail.rs`). `detail-screen.slint` renders `detail-episodes` only for
  season and episode pages; a series page shows a single Next Up card, which
  reads the same image cache. Fetching every episode's still was up to 47
  wasted round trips and decodes on a 48-episode show. Verified on The Office:
  the Next Up card still shows its artwork.
- **Artwork disk I/O moved to `spawn_blocking`** (`ui/catalog.rs`,
  `ui/detail.rs`). `existing_cached_image` and `write_cached_image` are
  blocking `std::fs` and the write fsyncs; they were being called directly on
  tokio runtime workers.
- **One fsync per cached image instead of two** (`services/artwork.rs`). The
  file sync stays, because it is what prevents a truncated entry being served
  forever; the parent-directory sync is gone, because losing a rename loses one
  disposable cache entry that the next fetch replaces.
- **mpv rendering gated on `MPV_RENDER_UPDATE_FRAME`, and
  `mpv_render_context_report_swap` now called** (`services/render.rs`). The
  update flags were being discarded, so mpv redid its full scale, convert and
  dither pass at display refresh rate regardless of whether it had a new frame;
  and the context opts into `MPV_RENDER_PARAM_ADVANCED_CONTROL`, whose contract
  requires reporting the swap. Playing CPU measured 4.5% before and 4.2% after
  -- within noise on this GPU. Kept as contract correctness and as work that
  will matter on weaker hardware, not as a measured win. Verified against
  playing, paused, seek-while-paused (the frame does update), and the
  fullscreen resize path.

Smaller changes, each below the noise floor individually: `RecursiveItemCount`
dropped from list queries; a page limit on `browse`; the parsed server URL and
authorization header built once per session instead of per request; the GL
texture-unit count and the GL sentinel environment lookup hoisted out of the
per-frame path; the cursor-visibility winit round trip skipped when the state
already matches; the progress `ReportTarget` cloned after the throttle
decision rather than before.

Also patched rather than replaced: `detail_links`, `detail_audio_options` and
`detail_subtitle_options` (`ui/detail.rs`). The last two back ComboBoxes, and
`update_detail_mutation` re-presents the whole page twice for a single watched
toggle with identical track lists. `sort_detail_items` no longer lowercases
into two fresh Strings per comparison.

### Rejected after measurement

- **A byte-budgeted LRU for the decoded-image cache.** Of 352 MiB settled RSS,
  the app heap is 34 MiB. 104 MiB is Mesa's `libgallium` + `libLLVM`, pulled in
  by `libEGL_mesa` when glvnd enumerates EGL vendors on this dual-GPU machine
  and never used for rendering; ~135 MiB more is the NVIDIA driver. The image
  cache is not where the memory is.
- **Suppressing the per-frame redraw while the player is up.** The alternating
  double-buffered texture id was expected to dirty `video-frame` every frame
  and repaint the UI at vsync. Measured CPU while paused is 0.0%, so there was
  nothing to fix and the change would have risked video correctness for
  nothing.
- **Replacing the mpv worker's 20 ms poll with a wakeup callback.** Idle CPU is
  already 0.0%; the poll is not costing anything worth the risk.
- **Progressive Home rendering.** Presenting each dashboard section as it
  arrives would cut the remaining wait to the fastest request (`libraries`, ~90
  ms) rather than the slowest (~165 ms). After the field trim that is worth
  roughly 100 ms, against streaming six independently-failing sections into a
  models-and-ordering path that currently installs them atomically. Not worth
  the risk at 300 ms; revisit if the dashboard grows more sections.
- **Making the catalog screen persistent.** `app-window.slint:204` builds
  `CatalogScreen` behind an `if`, so leaving it destroys every tile and femtovg
  frees their GL textures -- `ImageStore::remove` is 24.8% of navigation CPU,
  about 33 ms of an 82-96 ms cycle. Keeping the tree alive behind `visible`
  would retain the textures, but it changes layout and hit-testing for a
  transition that is already imperceptible on this hardware.
- **Moving JPEG decode off the UI thread.** `zune_jpeg` accounts for ~1.5% of
  a 660 ms CPU budget across a 22 s warm-cache session, and cold-cache startup
  is within noise of warm. The decode is real but it is not a bottleneck here;
  it would need a library far larger than this one to become one.

### Guarding the field sets

The two field-set regressions are the easy ones to reintroduce -- both are one
careless edit to a string constant, and neither shows up in a code review as
anything but a longer list. `api::tests` pins them:
`list_queries_ask_for_nothing_a_catalog_row_cannot_use`,
`library_rows_keep_the_recursive_count_their_subtitle_needs`,
`detail_queries_still_ask_for_what_the_detail_page_renders`, and
`browsing_a_folder_is_a_bounded_page`. The first was checked by putting all
three fields back and confirming it fails.

## Round two, 2026-09-02

Same machine and server as above. Round one's numbers were measured from the
first statement of `main`; the figures below separate the dynamic linker out,
so the "before" column is re-measured rather than copied.

| Scenario | Before | After |
|---|---|---|
| `exec_to_main` (dynamic linker, previously unmeasured) | 70 ms | 70 ms |
| `time_to_interactive` from `main` | 303-339 ms | 288-343 ms |
| Settled Home RSS / PSS | 349 MiB / 151 MiB | 345 MiB / 147 MiB |
| Settled thread count | 55 | 13 |
| Release binary | 25,888,064 B | 24,317,048 B |
| Decoded-image cache after a browse | unbounded (80 MiB and climbing) | bounded at 96 MiB, LRU |
| On-disk artwork cache | unbounded, never pruned | bounded at 256 MiB, oldest first |

### Where launch time actually goes

Round one assumed the fan-out was gated on things that could be moved. Stage
timings say otherwise:

| stage | at |
|---|---|
| tracing + backend selection + tokio runtime | 25 ms |
| window built (the whole Slint tree) | 26 ms |
| mpv core init and renderer installed | 30 ms |
| HTTP client built | 31 ms |
| **dashboard response** | **285 ms** |
| catalog presented | 286 ms |
| first frame carrying it | 298 ms |

**Local init is 31 ms.** Building the window costs 1 ms and mpv core init 4 ms,
so moving either off the critical path cannot help. The launch is the
dashboard, and the dashboard is the server:

| request | arrives at |
|---|---|
| libraries | 123 ms |
| resume | 183 ms |
| latest | 184 ms |
| recent | 207 ms |
| favorites | 236 ms |
| **`Shows/NextUp`** | **285 ms** |

`home()` installs nothing until all six finish, so `Shows/NextUp` alone sets
the first frame, and `time_to_interactive` tracks it one-for-one across runs
(313/273, 326/286, 298/258, 349/308 -- a flat ~40 ms of client-side work on
top). Its cost scales with `Limit`: measured against this server at 20 items it
is 111 ms, at 12 items 77 ms, at 8 items 53 ms.

So there are exactly two client-side levers left on launch, and both are
recorded here rather than taken:

- **Ask `Shows/NextUp` for fewer items.** Worth roughly 35 ms solo and more
  under the six-way contention. It shortens a user-visible shelf, which is a
  product decision, not a performance one.
- **Present each dashboard section as it arrives.** First content would land
  with `libraries` at ~123 ms instead of ~298 ms. Round one estimated this at
  "roughly 100 ms" and rejected it; the measured figure is 162 ms, and it stays
  fully network-authoritative -- each section is still painted only after the
  server confirms it. It remains a real restructure of the models-and-ordering
  path, which currently installs all six atomically.

### What moved, and why

- **DNS.** hyper resolves once per connection with no in-flight
  deduplication, so the six parallel dashboard requests each ran the same
  `getaddrinfo`: one lookup of `jellyfin.local` measures 22-40 ms and six contending
  measure ~80 ms of wall time. `services::dns` caches with a 60 s TTL -- a TTL
  rather than a pinned address, so a server that moves recovers without a
  restart. The log now shows one lookup and five hits. It does not move
  `time_to_interactive`, because the lookups overlapped connection setup; it is
  kept for the artwork connections that follow and for resolvers slower than
  this one.
- **Threads, 55 to 13.** tokio defaulted to one worker per logical CPU, 32
  here, plus a blocking pool ceiling of 512. Eight more were mpv's: it starts a
  Lua VM on its own thread for each built-in script -- `ytdl_hook`, `stats`,
  `console`, `select`, `positioning`, `commands`, `context_menu`, `osc` -- all
  of which serve mpv's own OSD and command line, which this app does not have.
  `config = false` does not cover them. Worth ~5 MiB; thread stacks are
  virtual, so the rest of the win is scheduler pressure, not resident memory.
- **The decoded-image cache had no byte budget.** It counted entries and, at
  1024, called `clear()`. Entries range from ~590 KB to ~5.7 MB, and nothing
  was evicted before the cliff: one library browse plus six detail pages
  reached 80 MiB across 123 entries and was still climbing. Now 96 MiB with LRU
  eviction to 80% -- enough for a full 120-card grid page, so ordinary
  scrolling never evicts.
- **The on-disk artwork cache was never pruned**, growing for the life of the
  installation. Now bounded at 256 MiB, oldest first, on the blocking pool at
  launch.
- **The artwork cache filename came from `DefaultHasher`**, whose output is
  unspecified between Rust releases: a toolchain upgrade silently renamed every
  entry, leaked the old ones, and re-fetched everything. FNV-1a, with a test
  pinning the exact filename.
- **The download permit was taken ahead of the disk probe**, so every cache hit
  queued behind six in-flight fetches for a permit it did not use.

### Rejected after measurement, round two

- **Moving JPEG decode off the UI thread.** Round one rejected this at 1.5% of
  CPU on Home; a library browse with scrolling put it at 8.6% of the UI thread,
  which justified revisiting. Implemented -- decoding on the blocking pool into
  a `SharedPixelBuffer`, straight from the decoder, preserving the source pixel
  format -- it removes the decode from the UI thread completely (8.6% to
  absent) and costs **17 MiB of settled RSS and PSS**, reproducibly: 345-348
  MiB RSS / 147-149 PSS with `Image::load_from_path` on the event loop, 362-364
  / 164-165 with the identical code decoding off it. Slint's own loader stores
  the pixels more compactly than `Image::from_rgb8` does. Forcing RGBA8 rather
  than preserving RGB8 costs a further 17 MiB on top. 8.6% of a UI thread that
  is not dropping frames is not worth 11% of the memory this app actually owns.
  Reverted; the byte budget it was developed alongside was kept.
- **Reusing the per-frame `Vec` in `GlStateGuard::capture`.** It is
  `MAX_COMBINED_TEXTURE_IMAGE_UNITS` entries, 1.5 KiB, so roughly 50 ns against
  a 16.6 ms frame budget -- 0.0003%, and only during playback. Threading a
  scratch buffer through the most delicate code in the app is the wrong trade
  at that size. The 192 `glGetIntegerv` calls in the same loop dominate it by
  orders of magnitude and are the thing worth attacking, at a risk level this
  round deliberately avoided.
- **Skipping the `chmod` in `secure_directory` when the mode already matches.**
  One syscall on an existing directory, per launch. Adding a branch to
  permission-enforcement code cannot be justified by an unmeasurable saving.
- **Starting the dashboard request before the window is built.** Implemented
  and kept, because it is strictly better ordering and takes the blocking
  `session.json` read and the client build off the main thread -- but it moved
  nothing, because the local init it now overlaps is 31 ms, not the ~200 ms the
  plan assumed.

### Software decoding rendered a black picture -- two causes, both now fixed

This had **two** independent causes with one symptom, and the entry below
originally closed the fault after finding the first. It did not hold: the
black picture came back on 2026-09-05. Both halves are recorded here, in the
order they were found.

#### Cause one: starved direct-rendering buffers -- fixed 2026-09-03

Round two described it as reproducible but not isolated, and named round one's
frame gating as a hypothesis. The cause was in that gating, but the broken half
was not the one suspected.

`render.h`, on `mpv_render_context_update`:

> This is optional if MPV_RENDER_PARAM_ADVANCED_CONTROL was not set (default).
> Otherwise, it's a hard requirement that this is called after each update
> callback. [...] If MPV_RENDER_PARAM_ADVANCED_CONTROL was set, this will do
> additional work **such as allocating textures for the video decoder**.

The context sets `ADVANCED_CONTROL = 1`, which enables mpv's direct rendering
(`vd-lavc-dr`, default `auto`): libavcodec decodes straight into textures mpv
allocates inside `mpv_render_context_update`. **Direct rendering exists only on
the software path** -- an nvdec frame arrives as a CUDA-interop texture and
needs none of it. That is the asymmetry the luma table measured, and why three
rounds of verification against hardware decoding never saw it.

`take_frame_request` returned early without calling `mpv_render_context_update`
at all whenever its cached `pending` flag was clear, and its caller
short-circuited past it on a resize and on the first frame; `BeforeRendering`
for any screen other than the player never called it either, though the render
context outlives that screen. Separately, the wake bridge cleared its `queued`
coalescing flag *after* the redraw had already been posted, so a callback
landing in that window sent nothing and its `update()` was lost until an
unrelated repaint. The software decoder was starved of buffers and the black
first frame stayed on screen while audio and `time-pos` carried on.

The fix calls `mpv_render_context_update` unconditionally, on every frame and
on every screen, and keeps `MPV_RENDER_UPDATE_FRAME` gating only
`mpv_render_context_render` -- which is where round one's saving actually was,
and that saving was 4.5% to 4.2% playing CPU, within noise. The `pending` flag
is gone; `queued` is now cleared before the redraw is requested, so the cost of
being early is a redundant repaint that Slint coalesces.

Measured the same way as the fault, mean and max luma over the video area, on
the same machine with `JELLYBRIDGE_HWDEC=no` forcing the path:

  | decode path | video area | verdict |
  |---|---|---|
  | `software`, before | mean 0.3, max 3 | black |
  | `software`, before | mean 1.0, max 3 | black |
  | `software`, before | mean 1.0, max 3 | black |
  | `software`, after | mean 21.7, max 251 | picture |
  | `software`, after | mean 47.1, max 254 | picture |
  | `software`, after | mean 21.3, max 246 | picture |

The mean varies between captures, which is the part that says frames are
advancing rather than one frame being handed back forever.

`hwdec` was hard-coded and `config = false` leaves mpv no configuration file,
so there was no way to exercise this path on demand -- which is how a fault
specific to it survived a smoke test. `JELLYBRIDGE_HWDEC` now overrides it,
defaulting to `auto-safe`.

Not chased, ruled out by inspection first: femtovg restores `UNPACK_*`
pixel-store state itself and uses no PBOs, so stale unpack state -- the other
thing only a software upload would depend on -- was never a candidate, and
`GlStateGuard` saves and restores that state regardless.

**What this measurement missed.** Every capture above was taken with
`JELLYBRIDGE_HWDEC=no`, so it only ever exercised software chosen *by
configuration*. Software chosen as a *fallback* is a different path, and it was
still black. That is cause two.

#### Cause two: `loadfile` ran before the render context existed -- fixed 2026-09-05

`vo=libmpv` has no video output at all until `mpv_render_context_create` has
run, and since the RSS work above the context is built lazily, on the first
frame drawn with the player screen showing -- ~320 ms of work. But
`PlayerCommand::Load` was sent *before* the screen switched, so mpv reached
decoder init with no GL interop to probe: `hwdec=auto-safe` had nothing to
select, fell back to software, and configured its video chain against a VO with
no surface. The picture stayed black while audio and `time-pos` ran on.

Which side won was a race against how fast the file opened. Captured on a
losing run, timestamps relative to process start:

```
184787.6ms  libmpv loaded media
184787.6ms  classified libmpv decode path decode_path=software
184983.6ms  mpv render context created ms=343.031272
```

The decode path was chosen **196 ms before the context existed**. Video-area
luma on that run: mean 0.43, max 3 -- the same black as cause one. A winning run
has the two lines the other way round and reports `hardware/direct (nvdec)`.

This is also what the previous round recorded but could not explain:

> **Which path mpv picks for the same item is itself not deterministic** -- the
> same episode reported `nvdec` on one launch and `software` on the next

That was never a separate mystery. It was this race, and it is why a fault that
only ever appeared on the software path survived three rounds of verification
against hardware decoding.

The fix keeps the context lazy -- eager creation is what cost 187 MiB of settled
RSS -- and makes it *ordered* instead. `features/playback.rs` now enters the
player screen, awaits a `watch` channel that `RenderHost::ensure_mpv` fires on
success and on failure alike, and only then sends `Load`. The wait is the same
~320 ms that already elapsed; it just no longer overlaps `loadfile`. A five
second timeout logs `mpv render context was not ready in time` and loads anyway,
so a machine that cannot build a context still plays.

Measured after the fix: a session that never opens the player builds no context,
`time_to_interactive` 298 ms and settled RSS 365 MiB, both unchanged -- the
laziness the RSS work bought is preserved.

### Still open

- The **`ImageStore::remove`** cost of tearing down a screen is 9% of the UI
  thread across a browse-and-navigate session. Keeping the catalog and detail
  screens alive behind `visible:` instead of `if` is what would remove it; it
  changes layout and hit-testing and was out of scope this round.
- **libmpv is a direct `DT_NEEDED`**, and 202 of the 206 objects the process
  loads at exec (~213 MB of `.so`) come from that one tree. `exec_to_main` is
  70 ms, a fifth of the launch. `dlopen`ing libmpv at first playback would need
  the `libmpv2` crate replaced by a hand-rolled symbol table; out of scope this
  round.

### Package comparison gate

The paired Kotlin-versus-Rust package comparison described above is still not
run. The Kotlin DMG was built on 2026-08-30 as a 75,575,197-byte artifact with
SHA-256
`4ccac4545eb7c9849519f1deb145012c1fa4cad719c924c822eb7bf5a85355f4`.
The Rust package remains blocked on the audited native bundle, so the 30%
go/no-go gate is still unmet. The numbers above are single-binary measurements
on the development machine, not the packaged comparison that gate requires.
