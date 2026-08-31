# JellyBridge TODO

Last compared with the current app state: 2026-06-30 (Kotlin implementation;
the app has since been rewritten in Rust/Slint — progress reporting and
filters from the list below are not yet ported, see
docs/rust-port/parity-matrix.md).

## Current Functionality

- [x] Manual Jellyfin server login and saved session restore.
- [x] Top-level library browsing with folder navigation.
- [x] Global and recursive media search.
- [x] Basic type, watched, and favorite filters.
- [x] Poster grid with watched, progress, resolution, and unwatched-count indicators.
- [x] Home dashboard rows for Libraries, Continue Watching, Next Up, Latest Media, Favorites, and Recently Added.
- [x] Item detail pages with overview, ratings, genres, cast, studios, and media info.
- [x] Jellyfin-style detail pages with backdrop, poster, hierarchy breadcrumbs, and action buttons.
- [x] Stable hierarchical navigation between series, seasons, episodes, folders, and Home.
- [x] Season detail pages list all episodes with watched/unwatched state and resume progress.
- [x] Series and season unwatched episode badges, hidden when fully watched.
- [x] Server-backed mark watched/unwatched actions for movies, episodes, seasons, and series.
- [x] Direct MPV playback for movies and episodes.
- [x] Resume position restore and Jellyfin playback start/progress/stop reporting.
- [x] Audio and subtitle stream indexes included in playback progress reports.
- [x] Remembered audio and subtitle language preferences for movie replays and TV episodes.
- [x] Movie and episode detail selectors for audio and subtitle tracks.
- [x] Local card size persistence.
- [x] Header JellyBridge brand click returns to Home.
- [x] Home dashboard episode cards show series context.

## Missing Versus Official Jellyfin UI

- [x] Home dashboard backed by libraries, resumable, next-up, latest, favorites, and recently added queries.
- [x] Basic search and filters for media type, watched status, and favorites.
- [x] Audio/subtitle selection before playback from movie and episode detail pages.
- [x] Next Up is present on Home and episode cards include show context.
- [x] Card size preference persists locally.
- [ ] User-selectable library sorting and pagination.
- [ ] Richer filters, saved views, and list/grid view persistence.
- [ ] Favorite/unfavorite, like/dislike, playlist, and collection management actions.
- [ ] Chapter navigation, pause/seek controls from the app, and next-video flow.
- [ ] Auto-play next episode.
- [ ] App settings UI for MPV path, display preferences, and playback behavior.

## Useful Jellyfin API Features To Add

- [x] Home dashboard using latest, resumable, next-up, and favorite item queries.
- [x] Rich item details using expanded fields for genres, people, studios, ratings, media streams, and artwork.
- [x] Basic item queries with filters, fixed sorting, and dashboard limits.
- [x] Watched-state user-data actions.
- [x] Playback progress reporting, including audio and subtitle stream indexes.
- [ ] Chapters, trailers, specials, and richer related-media sections on detail pages.
- [ ] Advanced item queries with `StartIndex`, user-chosen `Limit`, richer filters, and sort options.
- [ ] Favorite, rating, playlist, and collection APIs.
- [ ] Session command handling so JellyBridge can participate more fully as a Jellyfin client.

## Competitor-Inspired Ideas

- [ ] Plex: downloads, skip intro/credits, watchlist-style queues, and synchronized watching.
- [ ] Emby: Live TV/DVR, offline sync, Chromecast-style casting, and parental/session controls.
- [ ] Kodi: remote-control-friendly navigation, smart playlists, skins, add-ons, and PVR-style browsing.
- [ ] Infuse: polished metadata, strong direct-play UX, rich filters, subtitle downloads, collections, Trakt-style sync, and skip intro/credits.

## Recommended Priority

1. [x] Build a home dashboard.
2. [x] Add item detail pages.
3. [x] Add global search and basic filters.
4. [ ] Add user-selectable sorting and pagination.
5. [x] Add watched/unwatched user actions.
6. [ ] Add favorite user actions.
7. [x] Add audio and subtitle selection before playback.
8. [ ] Add chapter navigation, pause/seek controls, and next-video playback flow.
9. [x] Add Next Up and show context for TV episodes.
10. [ ] Add auto-play next episode.
11. [x] Persist card size and playback stream preferences locally.
12. [ ] Add app settings for MPV path, display preferences, and playback behavior.
