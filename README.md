# JellyBridge

JellyBridge is a native Rust/Slint desktop client for Jellyfin with embedded
libmpv video playback. It is a lightweight alternative to browser or
Electron-based Jellyfin clients, with a strong preference for direct play
instead of server-side transcoding.

## What It Does

- Logs in to a Jellyfin server with username and password and restores the
  saved session on startup.
- Shows a Home dashboard with libraries, Continue Watching, Next Up, latest
  media, favorites, and recently added media as horizontally scrollable
  shelves.
- Browses Jellyfin libraries and nested folders as adaptive 2:3 poster grids.
- Searches globally from Home and recursively inside folders, updating after a
  300 ms typing pause. Results are capped at 120 items and show the server
  total when more exist.
- Sorts by name, date added, release date, and rating; Compact/Normal/Large
  density settings persist locally.
- Shows movie, series, season, and episode detail pages with posters,
  backdrops, metadata, ratings, genres, cast, studios, runtime, watched state,
  resume progress, and unwatched counts.
- Plays media in an embedded libmpv surface through a token-hiding local
  loopback proxy, so mpv never sees Jellyfin credentials.
- Restores resume positions, downloads external text subtitles with
  authentication, and remembers per-title audio/subtitle track preferences.
- Marks items watched/unwatched and favorite/unfavorite from the UI.

## What It Does Not Do Yet

- No transcoding fallback. JellyBridge expects mpv to direct play the media.
- No playback progress reporting back to Jellyfin (the retired Kotlin client
  had it; see `docs/rust-port/parity-matrix.md`).
- No Live TV, DVR, offline downloads, or multi-server account switcher.
- No app settings screen, playlist, or collection management yet.

## Requirements

- A Jellyfin server reachable from the desktop machine and a user account.
- Rust (pinned by `rust-toolchain.toml`) via rustup.
- libmpv (`brew install mpv` on macOS; distribution packages on Linux).

## How To Run

All cargo invocations go through the wrapper script, which points pkg-config
at the local libmpv installation:

```bash
scripts/cargo-env.sh run --release
```

The first launch shows the login screen. Enter the Jellyfin server URL (for
example `http://localhost:8096`), username, and password.

After login, JellyBridge saves the server URL, access token, and user ID under
`~/.config/jellybridge/session.json`. Use Sign out in the app to clear it.

## Player Hotkeys

- `Space`, `K`, or `P`: play/pause.
- `Left`/`Right`: seek 5 seconds; hold `Shift` to seek 30 seconds.
- `Up`/`Down`: change volume by 5; `9`/`0` use 2-point steps.
- `M`: mute, `F`: toggle fullscreen, `Esc`: leave fullscreen.
- `A`: cycle audio tracks, `J`: cycle subtitles.
- `Home`/`End`: jump to the start/end; `Q`: stop playback and return.

## Development Commands

```bash
# Build
scripts/cargo-env.sh build --locked

# Run the test suite
scripts/cargo-env.sh test --locked

# Lint
scripts/cargo-env.sh clippy --all-targets --locked -- -D warnings

# Run the app
scripts/cargo-env.sh run
```

## Packaging

For a self-contained personal macOS arm64 build with the complete installed
Homebrew mpv/FFmpeg codec graph embedded:

```bash
brew install mpv dylibbundler
scripts/package-rust-app-personal.sh
```

The script creates `target/release/JellyBridge.app` and a compressed
`JellyBridge_<version>_aarch64_self-contained.dmg`. It verifies code
signatures, dependency paths, embedded-library completeness, and the final DMG
checksum. This personal package is ad-hoc signed, not notarized, and includes
GPL-enabled Homebrew components; do not use it as the public release artifact.

## Project Structure

```text
rust/app
├── build.rs                 rpath link arguments + Slint compilation
├── assets/                  application icon
├── src/
│   ├── main.rs              composition root: startup, wiring, teardown
│   ├── app.rs               AppState/AppContext (no Slint types)
│   ├── api.rs               Jellyfin HTTP client
│   ├── features/            use-cases by feature: auth, catalog, detail, playback
│   ├── services/            infrastructure: artwork cache, session storage,
│   │                        playback preparation, mpv worker, loopback proxy,
│   │                        GL render bridge
│   └── ui/                  the only Rust↔Slint boundary: UiHandle event-loop
│                            funnel, callback wiring, presenters, hotkeys
└── ui/
    ├── app-window.slint     composition root: boundary properties, screen switch
    ├── theme.slint          design tokens (palette, spacing, radii, type)
    ├── types.slint          boundary structs and enums shared with Rust
    ├── components/          genuinely shared components (top bar)
    └── features/            per-feature screens and their local components
```

Dependency direction: `.slint` ↔ `src/ui` ↔ `app.rs` + `features/` →
`services/`. Only `src/ui` may import `slint`.

## Configuration Files

```text
~/.config/jellybridge/session.json    server URL, access token, user ID
~/.config/jellybridge/settings.json   card density, sort, track preferences
~/.cache/jellybridge/                 artwork disk cache
```

These files can contain Jellyfin server URLs, access tokens, and user IDs. Do
not commit real local configuration or tokens.

## Troubleshooting

### Build fails to find libmpv

Install mpv (`brew install mpv`) and always build through
`scripts/cargo-env.sh`, which exports the right `PKG_CONFIG_PATH`.

### Direct play fails

JellyBridge intentionally does not request transcoding. Confirm that mpv can
play the file format, the server can serve the file, the machine can reach the
server URL used at login, and the user has access to the item.

### Video is black, or a specific item will not decode

JellyBridge direct-plays the original file, so mpv falls back to software
decoding for anything the GPU will not take -- 10-bit H.264, 4:2:2 or 4:4:4
chroma, and similar. `JELLYBRIDGE_HWDEC` overrides mpv's `hwdec` setting for a
run; it defaults to `auto-safe`. Set it to `no` to force software decoding, or
to `auto` to allow the copy-back modes `auto-safe` excludes:

    JELLYBRIDGE_HWDEC=no jellybridge

The chosen mode is logged at startup, and the path mpv actually took is logged
when each item loads and shown in the player status line.

### Login keeps returning to the auth screen

The saved session may be missing, invalid, or revoked. Log in again, or reset
manually with `rm ~/.config/jellybridge/session.json`.
