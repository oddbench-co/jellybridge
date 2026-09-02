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

    scripts/profile.sh run  [label]              # frame metrics + RSS
    scripts/profile.sh cpu  [label]              # the same, under samply
    scripts/profile.sh --cold-artwork --duration 20 run cold

The `time_to_interactive` marker (`src/ui/startup.rs`) is reported once, when
the first frame carrying real content has actually been drawn -- not when the
presenter ran. Artifacts land in `target/profiling-runs/<label>-<stamp>/`.

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

### Package comparison gate

The paired Kotlin-versus-Rust package comparison described above is still not
run. The Kotlin DMG was built on 2026-08-30 as a 75,575,197-byte artifact with
SHA-256
`4ccac4545eb7c9849519f1deb145012c1fa4cad719c924c822eb7bf5a85355f4`.
The Rust package remains blocked on the audited native bundle, so the 30%
go/no-go gate is still unmet. The numbers above are single-binary measurements
on the development machine, not the packaged comparison that gate requires.
