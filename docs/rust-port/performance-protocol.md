# Performance Protocol

Compare release packages on the same physical machine, power mode, account,
server snapshot, window size, and network. Debug builds are invalid.

## Scenarios

1. Cold sign-in screen with no saved session.
2. Cold launch to interactive Home with an empty artwork cache.
3. Cold launch to interactive Home with a warm encoded-artwork cache.
4. Settled Home RSS after 30 seconds without interaction.
5. Peak RSS while scrolling the representative 100-card grid.

## Measurement

- Add an explicit structured `time_to_interactive` marker to both candidates.
- Run 15 paired, alternating launches per scenario.
- Fully terminate the prior process between launches.
- Report median, p95, range, peak RSS, and settled RSS.
- Record OS build, CPU, RAM, display scale, package hash, renderer, libmpv
  graph hash, server snapshot, cache mode, and power mode.
- Release requires at least 30% lower median cold-start and settled Home RSS.

## Current result

Not measured. The Kotlin DMG was built on 2026-08-30 as a 75,575,197-byte
artifact with SHA-256
`4ccac4545eb7c9849519f1deb145012c1fa4cad719c924c822eb7bf5a85355f4`.
The Rust vertical-slice package is blocked on the audited native bundle, so no
paired package measurement is valid yet. Missing paired results block the
go/no-go gate.
