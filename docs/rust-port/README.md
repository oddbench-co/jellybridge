# JellyBridge Rust Port

Status: **baseline and vertical slice authorized; browser/core implementation locked**.

The Rust work runs alongside the Kotlin application. The Kotlin implementation
remains the behavioral reference and may not be removed until the physical
player, packaging, parity, and performance gates pass.

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

- `parity-matrix.md`: frozen Kotlin behavior inventory.
- `fixture-provenance.md`: fixture sources, versions, and redaction rules.
- `performance-protocol.md`: repeatable startup and RSS measurements.
- `native-dependency-policy.md`: release linkage and licensing requirements.
- `packaging.md`: audited native-bundle contract and reproducible package commands.
- `gate-report.md`: physical-machine evidence and final go/no-go decision.

Until `gate-report.md` records a passing result for physical macOS arm64,
Ubuntu 22.04 X11, and Ubuntu 22.04 Wayland, the Rust work is a feasibility
prototype and must not grow into the replacement browser.

The stress runner never manipulates the desktop by default. Resize/fullscreen
stress requires the explicit `JELLYBRIDGE_STRESS_UI=1` opt-in and should run
only on a dedicated certification machine.

## Development checks

Use the runner so the exact Rust toolchain and development libmpv lookup are
consistent:

```bash
scripts/run-rust-slice.sh clippy -p jellybridge-vertical-slice --all-targets -- -D warnings
scripts/run-rust-slice.sh test -p jellybridge-vertical-slice
scripts/run-rust-slice.sh check -p jellybridge-vertical-slice \
  --no-default-features --features renderer-skia-opengl --locked
```

macOS uses the Homebrew development mpv graph by default. Linux uses the
system `pkg-config` search path. Set `JELLYBRIDGE_DEVELOPMENT_MPV_PREFIX` on
either platform to test against a separate development prefix. This does not
make that prefix release-eligible; packaging accepts only the audited bundle
described in `packaging.md`.
