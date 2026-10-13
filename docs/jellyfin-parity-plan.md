# Jellyfin web parity plan

Goal (2026-10-13): JellyBridge looks and works like jellyfin-web 10.11 for
everything a viewer does. Out of scope: the admin Dashboard and Metadata
Manager, admin item actions (edit metadata/images/subtitles, identify, refresh,
delete), Music, Live TV, Books and Photos. Those libraries still open as a
plain folder grid. Measurements live in `jellyfin-web-reference.md`.

Performance rules from `PERFORMANCE.md` hold for every phase. Pages load off
the UI thread, grids are virtualized and paged, visited pages are kept in the
history so Back is instant, and no page polls.

## Phases

Status (2026-10-13): all seven phases are done and compared against
jellyfin-web screen by screen (`scripts/parity/`). Pages differ by less than
2% of pixels apart from artwork resampling (Home, library grids: 3–4%).

Deliberate differences, each because the app is a native viewer client:

- No SyncPlay or Cast buttons in the header, and no admin entries in the
  drawer or item menus (out of scope).
- Sign-in keeps **Change Server**, and a first-run "Connect to Server" has
  no back arrow or Cancel: jellyfin-web runs on its server and never needs
  to pick one, while there is nothing to go back to here.
- The player has no picture-in-picture button (no reliable always-on-top
  mini window on Wayland compositors) and its settings menu no **Quality**
  entry (the app always plays the original stream, so there is nothing to
  choose).
- Selects open jellyfin-web's action sheet rather than the browser's
  native list.
- The Up Next card follows jellyfin-web's CSS and timing exactly but was not
  compared live: neither test item has a next episode.

1. **Navigation shell**
   - One history for every page, catalog and detail alike. Back restores the
     page with its scroll position, and the mouse back button and Alt+Left
     also go back.
   - A header like jellyfin-web's: back, home, menu, title, search and user.
   - The side drawer: Home, the libraries, Settings and Sign Out.
   - The header tab strip: Home / Favorites, and each library's tabs.
2. **Library pages**
   - Movies tabs: Movies, Suggestions, Favorites, Collections, Genres.
   - Shows tabs: Shows, Suggestions, Upcoming, Genres, TV Networks, Episodes.
   - A paged grid with Play All, Shuffle, View, Sort, Filter and A–Z.
   - The Home Favorites tab.
   - A generic list page for genres, studios, tags and other folders.
3. **Search**: suggestions while the box is empty, and results grouped by
   type (Movies, Shows, Episodes, People, Collections, Playlists).
4. **Item pages and menus**
   - Person, collection (BoxSet) and playlist pages.
   - Detail links for genres, studios, people and tags.
   - The detail "More" menu and the card "More" menu or right-click: play,
     shuffle, add to playlist or collection, played, favorite, go to
     series/season, media info, copy stream URL.
5. **Playlists and collections**: create, add to, remove from, and play all
   or shuffle.
6. **User settings**
   - The preferences menu, Profile, Display, Home, Playback, Subtitles and
     Quick Connect.
   - These are stored on the server (user configuration and display
     preferences), so they stay shared with jellyfin-web.
7. **Player OSD**
   - jellyfin-web's control bar: trickplay thumbnails on the seek bar,
     chapter marks and the settings menu (quality, speed, aspect, repeat,
     playback info).
   - The stats overlay.

Each phase ends with:
- tests and clippy passing;
- screenshots compared against jellyfin-web;
- a measurement against `PERFORMANCE.md`;
- a review with the user before committing.
