# Repository Guidelines

## Project Structure & Module Organization

JellyBridge is a Rust + egui (eframe) desktop Jellyfin client with embedded
libmpv playback. The single production crate lives in `rust/app`:

- `src/main.rs` — composition root only: startup, dependency construction,
  `eframe::run_native` with the glow renderer, and the ordered teardown.
- `src/app.rs` — `AppState`/`AppContext`. Must not name any UI type; the
  window is reached through the opaque `ui::UiHandle`.
- `src/api.rs` — the one Jellyfin HTTP client (browsing, auth, images).
- `src/features/` — use-cases by feature: `auth`, `catalog`, `detail`
  (message/effect state machine + orchestration), `playback`.
- `src/services/` — infrastructure: `artwork` (disk cache), `session`
  (hardened session/settings stores), `playback` (PlaybackInfo + subtitle
  materialization), `libmpv` (libmpv loaded with `dlopen` on first play, not
  linked: see `docs/PERFORMANCE.md`), `player` (mpv worker thread), `proxy` (token-hiding
  loopback proxy), `render` (mpv renders into its own framebuffer inside an
  egui paint callback and is blitted into the window).
- `src/ui/` — the only module that knows egui: `UiHandle` (queues closures
  over `UiState` and wakes the window), the view models and presenters per
  screen, `app.rs` (the `eframe::App`: drains updates, draws, dispatches
  `UiAction`s to features), `theme.rs` (design tokens and fonts),
  `widgets.rs` (shared drawing), `images.rs` (artwork pipeline and texture
  LRU), `icons.rs` (Material icons rasterized per size), and
  `input_script.rs` (debug builds: scripted input for driving the real app).

Architecture rules: Rust owns all state, as plain view-model structs in
`UiState`; drawing reads them each frame and records `UiAction`s instead of
calling features directly, so draw code is testable without a server; use the
`theme` tokens instead of literal colors; keep blocking work (I/O, image
decode) off the UI thread; request repaints only when something changes
(`UiHandle::update`, timers via `request_repaint_after`) so an idle window
costs no CPU; draw only what is visible (`show_rows`, shelf culling) and
request artwork only for what is drawn.

## Build, Test, and Development Commands

All cargo commands go through `scripts/cargo-env.sh`, which pins the toolchain
and points pkg-config at the local libmpv (Homebrew on macOS). libmpv is not
linked; it is loaded at runtime the first time something plays, so it must be
installed to play, not to build. Performance work follows
`docs/PERFORMANCE.md` (measure first; `JELLYBRIDGE_MEASURE` and friends):

- `scripts/cargo-env.sh build --locked` — build.
- `scripts/cargo-env.sh test --locked` — run the test suite.
- `scripts/cargo-env.sh clippy --all-targets --locked -- -D warnings` — lint.
- `scripts/cargo-env.sh run` — launch the app.
- `scripts/build-windows.sh` — cross-builds the Windows exe, a per-user NSIS
  installer and a portable zip in a container (podman or docker), with
  `libmpv-2.dll` and the Vulkan loader beside the exe.
- `scripts/build-linux.sh` then `scripts/package-linux.sh` — the portable
  Linux bundle (Ubuntu 22.04 base, LGPL libmpv + libva bundled) and the
  .deb/.rpm/AppImage/tarball/Arch packages, all in containers.
- `scripts/vm-test/` — headless QEMU VMs that install each package on a
  distro's live ISO and run a scripted scenario; see
  `docs/rust-port/platform-matrix.md`.
- `scripts/package-rust-app-personal.sh` — self-contained personal macOS
  package (see `docs/rust-port/packaging.md`).

## Coding Style & Naming Conventions

Standard rustfmt style, edition 2024, toolchain pinned in
`rust-toolchain.toml`.
Errors are `thiserror` enums; avoid silently swallowing errors in new code.
Tests live in `#[cfg(test)]` modules next to the code they cover; pure logic
belongs in `features/`/`services/` where it is testable without a window.

## Commit & Pull Request Guidelines

Concise, sentence-case commit subjects describing the change. Pull requests
should include a short summary, testing performed, and screenshots for UI
changes. Note any Jellyfin server or mpv setup assumptions needed to verify.

## Security & Configuration Tips

Do not commit real Jellyfin tokens, server URLs, or user IDs. Session and
settings live under `~/.config/jellybridge/`. The playback proxy exists so
mpv never sees the Jellyfin token — do not route authenticated URLs around
it.
