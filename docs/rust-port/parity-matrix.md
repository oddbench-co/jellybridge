# Frozen Kotlin Behavior Inventory

Captured from revision `b81ea06453fa` plus the user-owned working-tree package
and icon changes present on 2026-08-30. This inventory freezes observable
behavior; the Rust UI may change layout but not silently lose these behaviors.

Legend: `K` verified in Kotlin source, `T` covered by an automated Kotlin test,
`M` requires manual characterization, `R` required of Rust before cutover.

| Area | Observable behavior | Baseline | Rust |
|---|---|---:|---:|
| Startup | 1280×800 window with JellyBridge branding and theme | K | R |
| Session | Load saved server URL, token, and user ID at startup | K | R |
| Session | Missing/malformed saved session opens sign-in | K | R |
| Auth | Require `http://` or `https://` and non-empty username | K | R |
| Auth | Enter in the password field submits; loading disables duplicate submit | K | R |
| Auth | Authentication failure remains on sign-in with an error | K | R |
| Logout | Delete saved session and return to sign-in; settings survive | K | R |
| Home | Load Libraries, Continue Watching, Next Up, Latest, Favorites, Recently Added | K | R |
| Home | A failed secondary row becomes empty without failing successful rows | K | R |
| Browse | Navigate folders with Home, back, and breadcrumb actions | K | R |
| Browse | Preserve hierarchy while opening series, season, episode, and parent links | K | R |
| Search | Home search is global across Movie, Series, and Episode | K | R |
| Search | Folder search is recursive beneath the current folder | K | R |
| Filter | All, Movie, Series, and Episode media filters | K | R |
| Filter | All, Unplayed, Played, and Favorite status filters | K | R |
| Sort | Name, added date, release date, runtime, rating, watched/unwatched orders | K | R |
| Paging | Page sizes 30/60/120; range label and first/previous/next/last controls | K,T | R |
| Paging | Clamp an invalid page after the server total changes | K,T | R |
| Grid | Adjustable 220–360dp card width persists across launches | K | R |
| Artwork | Primary, backdrop, thumb, and logo placeholders survive image failure | K | R |
| Cards | Title/context, watched, resume progress, resolution, and unwatched badges | K | R |
| Detail | Movie, series, season, and episode metadata and artwork | K | R |
| Detail | Overview, runtime, ratings, genres, people, studios, and hierarchy links | K | R |
| Detail | Series loads seasons; season loads ordered episodes | K | R |
| Watched | Mark movie/episode/season/series watched or unwatched and reload server state | K | R |
| Watched | Duplicate mutation is suppressed; failure produces a visible message | K | R |
| Playback | Only playable Movie/Episode items expose playback | K | R |
| Playback | Fetch playback info, use first media source, and direct-play through MPV | K | R |
| Resume | Convert Jellyfin ticks to seconds and start MPV at saved position | K | R |
| Reporting | Send start, immediate progress, 10-second progress, final progress, and stop | K | R |
| Reporting | If MPV closes first, use the last known position rather than zero | K | R |
| Tracks | Preselect audio/subtitle streams and report selected indexes | K | R |
| Tracks | Load external text subtitles and match MPV tracks by URL/index/language/title | K | R |
| Preferences | Remember movie-specific and series-wide audio/subtitle language/title choices | K | R |
| Preferences | Remember explicit subtitles-disabled preference | K | R |
| Shutdown | Stop playback and disconnect IPC when the library screen is disposed | K | R |
| Errors | Browse/detail failures expose retry; playback failures use a snackbar | K | R |

## Manual characterization required before the gate can pass

- Loading, empty, partial-error, offline, and expired-auth visuals.
- Keyboard focus order, back navigation, focus restoration, and scroll retention.
- Watched-state timing and rollback expectations.
- Window restoration after fullscreen playback.
- Shutdown while HTTP, image loading, or playback is active.
- Accessibility names, actions, and focus order.
- Exact appearance at compact and desktop breakpoints.

The Kotlin suite currently contains only the paging tests. All other rows need
Rust tests or explicit physical/manual evidence before cutover.
