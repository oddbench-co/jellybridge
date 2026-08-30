# JellyBridge

JellyBridge is a native Kotlin/JVM Compose Desktop client for Jellyfin that uses
MPV for video playback. It is built as a lightweight desktop alternative to
browser or Electron-based Jellyfin clients, with a strong preference for direct
play instead of server-side transcoding.

An isolated Rust/Slint/libmpv feasibility slice is under active development.
It is not yet the production client and remains blocked on physical playback,
packaging, and licensing certification. See the
[Rust port gate](docs/rust-port/README.md) for its exact status and commands.

The app handles Jellyfin login, library browsing, metadata display, MPV launch
and control, resume reporting, watched-state updates, and audio/subtitle stream
selection.

## What It Does

- Logs in to a Jellyfin server with username and password.
- Restores the saved Jellyfin session on startup.
- Shows a Home dashboard with libraries, Continue Watching, Next Up, latest
  media, favorites, and recently added media.
- Browses Jellyfin libraries and nested folders.
- Searches globally and recursively inside folders.
- Filters by media type, watched status, and favorites.
- Sorts and paginates browse/search grids.
- Shows movie, series, season, and episode detail pages.
- Displays posters, backdrops, metadata, ratings, genres, cast, studios,
  runtime, watched status, resume progress, resolution, and unwatched episode
  counts.
- Plays movies and episodes in MPV using direct play URLs.
- Restores resume position when playback starts.
- Reports playback start, progress, and stop events back to Jellyfin.
- Lets users choose audio and subtitle tracks before playback.
- Remembers card size and playback stream preferences locally.
- Supports marking items watched or unwatched from the UI.

## What It Does Not Do Yet

- No transcoding fallback. JellyBridge expects MPV to direct play the media.
- No Live TV or DVR support.
- No offline downloads.
- No multi-server account switcher.
- No app settings screen yet.
- No playlist or collection management yet.
- No remote Jellyfin session command handling yet, such as receiving
  play/pause/seek commands from another Jellyfin client.

## Requirements

- A Jellyfin server reachable from the desktop machine.
- A Jellyfin user account.
- JDK 21.
- MPV installed and executable.
- macOS, Linux, or Windows for development runs. The install helper is currently
  Linux-oriented.

The app looks for MPV in this order:

1. `JELLYBRIDGE_MPV_PATH`
2. `/opt/homebrew/bin/mpv`
3. `/usr/local/bin/mpv`
4. `/opt/local/bin/mpv`
5. `mpv` found on `PATH`

Examples:

```bash
# macOS with Homebrew
brew install mpv

# Debian/Ubuntu
sudo apt install mpv

# Custom MPV path
export JELLYBRIDGE_MPV_PATH=/path/to/mpv
```

## How To Run

Use the checked-in Gradle wrapper.

```bash
./gradlew run
```

On Windows:

```powershell
.\gradlew.bat run
```

The first launch shows the login screen. Enter:

- Jellyfin server URL, for example `http://localhost:8096` or
  `https://jellyfin.example.com`
- Jellyfin username
- Jellyfin password

After login, JellyBridge saves the server URL, access token, and user ID under:

```text
~/.config/jellybridge/session.json
```

Use Logout from the app to clear the saved session.

## How To Use The App

### Home

The Home screen groups commonly used Jellyfin views:

- Libraries
- Continue Watching
- Next Up
- Latest Media
- Favorites
- Recently Added

Click a library, folder, series, season, movie, or episode to navigate deeper.
Click the JellyBridge title in the top bar to return Home.

### Browsing, Search, Sorting, And Pagination

Use the search field in the header to search media. In folder views, search is
recursive within that folder. From Home, search runs globally across movies,
series, and episodes.

Available filters:

- Type: all media, movies, shows, episodes
- Status: all, unplayed, played, favorites

Browse/search grids support sort and page-size controls. Page navigation is
shown at the bottom of the grid.

The Rust client presents Home as horizontally scrollable Jellyfin-style media
shelves and library, folder, and search results as adaptive 2:3 poster grids.
Its search updates after a 300 ms typing pause and stays scoped to the current
folder. Sort choices cover name, date added, release date, and rating; Compact,
Normal, and Large density settings use 220, 260, and 320 px target card widths
and persist in the existing `settings.json`. Rust search results are currently
capped at 120 items and show the server total when more results exist.

### Detail Pages

Movies, episodes, series, and seasons open detail pages. Detail pages include
metadata, artwork, playback actions, watched-state controls, and related
hierarchy links.

Series detail pages show seasons. Season detail pages show episodes.

### Playback

Click Play on a movie or episode. JellyBridge asks Jellyfin for playback info,
builds a direct-play URL, launches MPV, and connects to MPV through a local IPC
socket:

```text
/tmp/mpv-jellyfin.sock
```

Playback progress is periodically reported to Jellyfin so resume positions stay
in sync.

Player hotkeys:

- `Space`, `K`, or `P`: play/pause.
- `Left`/`Right`: seek 5 seconds; hold `Shift` to seek 30 seconds.
- `Up`/`Down`: change volume by 5; `9`/`0` use mpv's 2-point steps.
- `M`: mute, `F`: toggle fullscreen, `Esc`: leave fullscreen.
- `A`: cycle audio tracks, `J`: cycle subtitles.
- `Home`/`End`: jump to the start/end; `Q`: stop playback and return.

### Audio And Subtitle Tracks

Movie and episode detail pages expose audio and subtitle selectors when playback
info is available. JellyBridge stores language/title preferences locally and
reuses them for later playback where possible.

Local preferences are saved under:

```text
~/.config/jellybridge/settings.json
```

## Development Commands

```bash
# Run the desktop app
./gradlew run

# Compile and run verification tasks
./gradlew build

# Run tests
./gradlew test

# Create a native package for the current OS
./gradlew packageDistributionForCurrentOS

# Remove generated build output
./gradlew clean
```

## Packaging And Installation

Compose Desktop can create native packages for the current OS:

```bash
./gradlew packageDistributionForCurrentOS
```

The repository also includes `install.sh`, a Linux-oriented helper that builds
and installs JellyBridge under `/opt/jellybridge` and links the executable into
`/usr/local/bin`.

Review it before running because it uses `sudo`:

```bash
./install.sh
```

## Project Structure

```text
src/main/kotlin/dev/nk/jellybridge
├── api        Jellyfin HTTP client and DTOs
├── auth       Saved Jellyfin session storage
├── mpv        MPV process and JSON IPC integration
├── playback   Playback orchestration, resume, subtitles, stream preferences
├── settings   Local app preference storage
└── ui         Compose screens, components, theme, and paging helpers

src/main/resources
└── icon.png   Desktop package icon

src/test/kotlin
└── Unit tests
```

## Configuration Files

JellyBridge stores local runtime data in:

```text
~/.config/jellybridge/session.json
~/.config/jellybridge/settings.json
```

These files can contain Jellyfin server URLs, access tokens, user IDs, and local
preferences. Do not commit real local configuration or tokens.

## Troubleshooting

### MPV does not start

Install MPV or set `JELLYBRIDGE_MPV_PATH`:

```bash
export JELLYBRIDGE_MPV_PATH=/path/to/mpv
./gradlew run
```

### Playback starts from the wrong place

JellyBridge uses Jellyfin `UserData.PlaybackPositionTicks` for resume and then
reports progress during playback. Check that playback progress reporting is
working and that the Jellyfin user has normal write access to watched state.

### Direct play fails

JellyBridge intentionally does not request transcoding. Confirm that:

- MPV can play the file format.
- The Jellyfin server can serve the media file.
- The desktop machine can reach the Jellyfin server URL used at login.
- The selected user has permission to access the item.

### Login keeps returning to the auth screen

The saved session may be missing, invalid, or revoked. Log in again. To reset
manually:

```bash
rm ~/.config/jellybridge/session.json
```

## Notes For Contributors

- Keep Jellyfin API and MPV integration out of composables when practical.
- Keep UI code under `ui/screens`, `ui/components`, or `ui/theme`.
- Use the checked-in Gradle wrapper.
- Add tests under `src/test/kotlin` for behavior that can be isolated from the
  desktop UI.
- Run `./gradlew test` or `./gradlew build` before submitting changes.
