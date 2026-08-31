# JellyBridge Rust Port

Status: **the Rust/Slint application is the production client.** The original
Kotlin/Compose implementation was removed from the working tree in August 2026
(it remains in git history) after the Rust rewrite reached feature parity for
browsing and playback. The former "vertical slice" feasibility crate was
absorbed into `rust/app/src/services/`.

## Known parity gaps

Tracked in `parity-matrix.md`. The most significant one: **playback progress
reporting to Jellyfin** (`/Sessions/Playing`, `/Sessions/Playing/Progress`,
`/Sessions/Playing/Stopped`) is not yet ported. The reference implementation
lived in the Kotlin tree at
`src/main/kotlin/dev/nk/jellybridge/api/JellyfinClient.kt` and
`playback/PlaybackController.kt` (see git history before the Kotlin removal).
Also missing relative to the matrix: media-type/status filters, pagination,
chapter navigation, and auto-play-next.

## Fixed targets

- Rust 1.98.0, edition 2024.
- Slint 1.17.1 with an explicitly selected native OpenGL renderer.
- mpv 0.41.0 and libmpv2 6.0.0.
- macOS arm64 (macOS 13+) and Linux x86_64 (glibc 2.35+).
- Jellyfin 10.9 minimum, 10.11.11 stable fixture, and a Jellyfin 12 canary.
- Raw static-source playback through mpv. Jellyfin's DirectPlay recommendation
  is preferred but never used as a codec/container gate; server transcoding
  URLs are never followed.

## Gate artifacts

- `parity-matrix.md`: frozen Kotlin behavior inventory (historical reference).
- `fixture-provenance.md`: fixture sources, versions, and redaction rules.
- `performance-protocol.md`: repeatable startup and RSS measurements.
- `native-dependency-policy.md`: release linkage and licensing requirements.
- `packaging.md`: packaging commands and the audited native-bundle contract.
- `gate-report.md`: physical-machine evidence from the feasibility gate.

## Development checks

Use the runner so the exact Rust toolchain and development libmpv lookup are
consistent:

```bash
scripts/cargo-env.sh clippy --all-targets --locked -- -D warnings
scripts/cargo-env.sh test --locked
scripts/cargo-env.sh check --no-default-features --features renderer-skia-opengl --locked
```

macOS uses the Homebrew development mpv graph by default. Linux uses the
system `pkg-config` search path. Set `JELLYBRIDGE_DEVELOPMENT_MPV_PREFIX` on
either platform to test against a separate development prefix. This does not
make that prefix release-eligible; packaging accepts only the audited bundle
described in `packaging.md`.
