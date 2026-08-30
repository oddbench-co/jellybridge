# Embedded Player Gate Report

Decision: **NO-GO for browser/core implementation — certification is incomplete.**

## Required physical certifications

| Target | Package | Renderer | DirectPlay | HW path | Stress | Result |
|---|---|---|---|---|---|---|
| macOS 13+ arm64 | DMG | — | — | — | — | Not run |
| Ubuntu 22.04 X11 x86_64 | DEB/AppImage | — | — | — | — | Not run |
| Ubuntu 22.04 Wayland x86_64 | DEB/AppImage | — | — | — | — | Not run |

## Development evidence captured on 2026-08-30

This evidence is useful for the feasibility decision but does not satisfy a
packaged clean-machine certification row.

- Physical Apple M5 Max arm64 machine running macOS 26.6.1.
- Rust 1.98.0 release binary with explicitly selected FemtoVG/OpenGL renderer.
- Slint supplied OpenGL 4.1; libmpv rendered into a same-context 2128×1212
  borrowed texture and the post-mpv magenta sentinel reported no failure.
- A generated local clip loaded, rendered, and automatically closed with clean
  normal-path teardown. Its observed decode path was **software**, so this is
  not hardware-decode evidence.
- Three short release runs recorded peak RSS values of 134,414,336,
  134,430,720, and 134,807,552 bytes (median 134,430,720 bytes).
- A pre-fix stress run reproduced a SIGSEGV in `mpv_request_event` at cycle
  183. A minimized run reproduced it at cycle 197, and moving observer setup
  before render creation reproduced it at cycle 67 on the main thread. The
  crash reports identify an invalid mutex access at address `0x88`, consistent
  with dereferencing a NULL client handle.
- Root cause: libmpv2 6.0.0's safe `Mpv::create_client` wrapper constructs a
  `NonNull` with `new_unchecked`, although libmpv documents that
  `mpv_create_client` returns NULL on error. The slice now keeps normal
  commands/events on the safe primary handle and owns a small checked raw
  render-client handle whose destruction follows render-context destruction.
- The minimized post-fix regression passed 250/250 cycles. The original
  physical macOS trigger then passed 500/500 cycles; each cycle performed
  repeated physical-pixel resizes, two fullscreen transitions, seeks, a GL
  sentinel check, and an asserted worker-before-render teardown. Persistent
  per-cycle RSS was 132,022,272–138,133,504 bytes (median 135,757,824; p95
  136,511,488 bytes). Monitor moves and sleep/wake were not covered.
- Rust formatting, strict Clippy, and twelve unit/integration tests pass. The
  proxy tests cover GET/range and HEAD forwarding, credential isolation,
  same-origin redirects, cross-origin rejection, HTTPS-downgrade rejection,
  and bounded shutdown while an upstream is stalled before sending headers.
  A mock Jellyfin contract test covers header authentication, strict source
  selection, mode-0600 external-subtitle materialization, range playback, and
  cleanup.
- The separate Skia/OpenGL renderer feature compiles successfully. It has not
  received physical rendering or resource-budget certification.
- cargo-packager 0.11.8 successfully discovers the slice configuration through
  its member manifest. Packaging then stops at the intended audited-native-
  bundle precondition; the available Homebrew mpv graph is development-only.

## Open blockers

- Slint 1.17.1 panics if the borrowed-image property is cleared from inside its
  `RenderingTeardown` notification because the winit adapter is already
  mutably borrowed. The slice tracks whether normal close cleared the image;
  on unexpected context loss it frees the libmpv render context but leaves
  texture/FBO reclamation to the dying GL context, then replaces the stale
  image before the next context draws. Sleep/wake must still certify this
  mitigation on both renderers before the lifetime blocker is closed.
- No audited dynamic LGPL libmpv/FFmpeg bundle, corresponding-source package,
  signed/notarized Rust DMG, DEB, or AppImage exists yet.
- Authenticated playback against a real Jellyfin server, private-CA playback,
  authenticated subtitle materialization, and actual hardware decoding remain
  uncertified.
- Monitor-move and sleep/wake tests, a clean macOS 13 package test, and both
  physical Ubuntu display-session tests remain open.
- Jellyfin 10.9, 10.11.11, and 12 canary fixtures are not yet captured, and the
  full paired Kotlin-versus-Rust performance protocol cannot run before a Rust
  package exists.

The gate passes only after deterministic renderer/context checks, authenticated
loopback-proxy raw-source playback, mpv-decided format support, subtitle materialization,
hardware classification, 500 open/close cycles, resize/fullscreen/sleep stress,
native dependency audit, clean-machine packaging, and early resource-budget
measurements are attached here with package and source hashes.
