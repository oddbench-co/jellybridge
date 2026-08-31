# Repository Guidelines

## Project Structure & Module Organization

JellyBridge is a Rust + Slint desktop Jellyfin client with embedded libmpv
playback. The single production crate lives in `rust/app`:

- `src/main.rs` — composition root only: startup, dependency construction,
  `ui::connect`, `window.run()`, and the ordered teardown.
- `src/app.rs` — `AppState`/`AppContext`. Must not name any Slint type; the
  window is reached through the opaque `ui::UiHandle`.
- `src/api.rs` — the one Jellyfin HTTP client (browsing, auth, images).
- `src/features/` — use-cases by feature: `auth`, `catalog`, `detail`
  (message/effect state machine + orchestration), `playback`.
- `src/services/` — infrastructure: `artwork` (disk cache), `session`
  (hardened session/settings stores), `playback` (PlaybackInfo + subtitle
  materialization), `player` (mpv worker thread), `proxy` (token-hiding
  loopback proxy), `render` (mpv↔Slint OpenGL bridge).
- `src/ui/` — the only module that imports `slint`: `UiHandle` (the single
  event-loop funnel), callback wiring, presenters, formatters, the in-memory
  image cache, and the window tests.
- `ui/` — Slint markup: `app-window.slint` (composition root and the
  Rust-facing property/callback boundary), `theme.slint` (design tokens),
  `types.slint` (boundary structs/enums), `components/` (genuinely shared),
  `features/<feature>/` (screens and feature-local components).

Architecture rules: Rust owns business/application state; Slint owns
presentation and ephemeral UI state; use `Theme` tokens instead of literal
colors; use precise `in`/`out`/`in-out` property directions; keep blocking
work off the UI thread; update Slint models incrementally (`patch_rows`)
rather than replacing them when identity is unchanged.

## Build, Test, and Development Commands

All cargo commands go through `scripts/cargo-env.sh`, which points pkg-config
at the local libmpv (Homebrew on macOS):

- `scripts/cargo-env.sh build --locked` — build.
- `scripts/cargo-env.sh test --locked` — run the test suite.
- `scripts/cargo-env.sh clippy --all-targets --locked -- -D warnings` — lint.
- `scripts/cargo-env.sh run` — launch the app.
- `scripts/package-rust-app-personal.sh` — self-contained personal macOS
  package (see `docs/rust-port/packaging.md`).

## Coding Style & Naming Conventions

Standard rustfmt style, edition 2024, toolchain pinned in
`rust-toolchain.toml`. Slint files use kebab-case file and property names.
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
