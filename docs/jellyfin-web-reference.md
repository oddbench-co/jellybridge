# Jellyfin web reference measurements

Measured from jellyfin-web 10.11 (dark theme, desktop layout) in Chrome at a
1261 CSS px wide window, device pixel ratio 1.5. One CSS px maps to one egui
point at the same scale factor, so these numbers are used directly. `vw` is a
percentage of the window width, `vh` of its height. Media-query breakpoints are
in 16 px ems.

## Global

| Token | Value |
|---|---|
| Font | Noto Sans; root size 93% = 14.88 px |
| Page background | `#101010` |
| Header bar | `#202020`; over detail backdrops `rgba(0,0,0,0.4)` |
| Primary text | `rgba(255,255,255,0.8)` |
| Secondary text | `rgba(255,255,255,0.5)`, 12.8 px (0.86 em) |
| Accent | `#00a4dc` |
| Played check (list rows) | `#cc3333` |
| Raised button | `#303030`, text `rgba(255,255,255,0.87)`, 600 weight |
| Select / input | `#292929`, radius 3 px |
| Page side padding | 3.3vw (41.3 px) |

## Header

Row of 41 px round icon buttons at y 8. Home page: logo at x 59. Other pages:
back (x 4), home (x 54), menu (x 104), page title 17.4 px at x 159. Right side:
SyncPlay, Cast, Search, user (each 41 px, 50 px apart, last at right − 45).
Tab strip below (Home/Favorites; library: Movies, Suggestions, …): 53 px
tall, 12.4 px semibold, active white, inactive `#999`. Header + tabs = 110 px.

## Cards

Card = image + text lines, `cardBox` margin 9 px top, 18 px right, 27 px
bottom. Image radius 3 px. Text centered: title 14.88 px (line 26 px tall),
secondary 12.8 px at 50% white (line 20 px).

Overflow (shelf) card widths, including the 18 px gap:

| Breakpoint (px) | Backdrop 16:9 | Portrait 2:3 / Square |
|---|---|---|
| < 400 | 72vw | 40vw |
| ≥ 400 | 72vw | 31.2vw (portrait) |
| ≥ 560 | 45.5vw | 31.2vw |
| ≥ 700 | 45.5vw | 23.1vw |
| ≥ 770 | 30vw | 23.1vw |
| ≥ 800 | 30vw | 18.5vw |
| ≥ 1200 | 23.1vw | 15.5vw |
| ≥ 1400 | 23.1vw | 13.3vw |
| ≥ 1600 | 18.7vw | 11.6vw |
| ≥ 1920 | 18.7vw | 10.41vw |
| ≥ 2100 | 18.7vw | 9.3vw |
| ≥ 2500 | 15.6vw | 9.3vw |

Grid (library) widths are a share of the grid width (window − 41 px left −
94 px right, the right side holding the A–Z picker): portrait/square 50% <
500, 33.3% ≥ 500, 25% ≥ 700, 20% ≥ 800, 16.67% ≥ 1200, 14.29% ≥ 1400,
12.5% ≥ 1600, 11.1% ≥ 1920, 10% ≥ 2100.

Indicators: played = 24 px accent circle with a check, top-right inset 7 px;
unplayed count = same circle with the number (11.9 px). Progress bar: 4 px at
the bottom of the image, track `rgba(51,51,51,0.8)`, fill accent.

Hover overlay: dark wash over the image, 58 px round play button centred,
39 px Played / Favorite / More buttons bottom-right.

Section title: 22.3 px, padding-left 3.3vw, row 46 px; paging ‹ › 41 px
round buttons at the right end of the title row.

Home sections, in order: My Media (libraries, backdrop), Continue Watching
(thumb), Next Up (thumb), Recently Added in <library> (one per library;
portrait for movies/shows, square for music). Thumb images prefer the item's
Thumb, then the series' Thumb, then a backdrop. Episode cards read
"<series>" / "S1:E16 - <episode>"; movies "<title>" / "<year>".

## Library page

Toolbar row centred at y 126: "1-34 of 34" (14.88 px), then 41 px icon
buttons (Play All, Shuffle, View, Sort, Filter). A–Z picker: column at
right − 37, letters 14.88 px at 50% white, 20.7 px apart.

## Detail page

- Backdrop 40vh, cover, centred; logo centred over it (max 315×200 here).
- Ribbon: `rgba(32,32,32,0.8)`, 107 px tall, top at backdrop bottom − 107.
- Poster: x 41, width 25vw (16:9 for episodes), top 306 (backdrop − 193).
- Content column starts at 32.45vw, right padding 3.3vw.
- Title 26.8 px semibold; episode subtitle ("Season 1 - 16. Title") 17.4 px
  semibold. Misc row 14.88 px, items 15 px apart: year/date, runtime,
  official rating badge (`rgba(170,170,190,0.2)`, 13.7 px, padding 3×7,
  radius 3), ★ rating (gold star), critic (tomato icon + %), "Ends at …".
- Buttons: 49×45 transparent icon buttons, 24.8 px icons at 80% white, at the
  right end of the ribbon: Resume, Play/Restart, Trailer, Played, Favorite,
  More. Played/favorite turn red when active.
- Media rows: label 93 px, values 14.88 px; selects 559 px wide, 26 px tall.
- Tagline 17.4 px; overview 14.88 px, line height 20 px; tags "Tags:" then
  bold comma-separated links 13.7 px; external links bold 13.7 px.
- Label/value rows (Genres, Director, Writers, Studios): label 93 px; values
  are links, 600 weight, comma separated.
- Shelves under the content column: More from Season N (backdrop cards,
  "1. Title"), Next Up (backdrop "S1:E16 - Title"), Seasons (portrait,
  unplayed badge), Cast & Crew (portrait photos, name / "as Role"), More Like
  This (portrait).

Season episode rows: 181 px apart; image 246×164 at +8,+9 with an 82 px play
overlay; title "1. Name" 14.88 px; "25m ★ 9.6 Ends at …" and overview at 50%
white, overview line height 17.9 px; right: 41 px Info, Played (`#cc3333`
when played), Favorite, More.

## Login

"Please sign in" 26.8 px regular, centred. Fields 804 px wide (≈ 64vw),
39 px tall, `#292929`, 2 px border (accent when focused), 16.4 px text; labels
14.88 px above at 70% white (accent when focused). Sign In: 47 px, accent,
semibold white. Secondary buttons `#303030`. Server entry is a separate
"Connect to Server" step with a Host field and Connect.

## Drawer

320 px wide, `#101010`, shadow `rgba(0,0,0,0.4) 2px 0 12px`, slides in with
`transform 0.24s ease-out` over the page dimmed to 50%. Rows 47 px tall:
20 px icon at x 36, label 14.88 px at x 76. Section headers ("Media",
"Administration", "User") 14.88 px at x 17.9, 14.9 px above and 7.4 px
below. Rows: Home; each library (icons: movie, tv, music_note, book, queue);
Settings and Sign Out under User.

## Header tabs

Centred in a 53 px strip under the 57 px header, 12.4 px semibold; each tab
is its label plus 18 px either side. Active white, inactive `#999`, hovered
the accent. No underline.

## Library tabs and dialogs

- Movies: Movies, Suggestions, Favorites, Collections, Genres. Shows: Shows,
  Suggestions, Upcoming, Genres, TV Networks, Episodes.
- Grid toolbar: "1-35 of 35", then (when paged by 100) ← →, Play All,
  Shuffle, View, Sort, Filter, 41 px buttons ~50 px apart.
- Sort: a centred dialog (black 50% backdrop, `#101010`, radius 3, content
  width 199) with "Sort By" and "Sort Order" (22.3 px headings) and radio
  rows 31.5 px apart; 16 px accent rings.
- View: an action sheet opened on the button, its checked row level with it;
  rows 40 px, check at x 17, label at x 63. Banner, List, Poster, Poster
  Card, Thumb, Thumb Card.
- Filter: a 300 px dialog of collapsible 52 px sections with hairlines:
  Filters (Played, Unplayed, Resumable, Favorites), Status (shows), Features,
  Genres, Parental Ratings, Tags, Video Types (BD, DVD, HD, 4K, SD, 3D),
  Years. Options come from `Items/Filters`.
- Genres tab: per genre a titled section with "›" to the full list and up
  to 9 cards wrapping into rows. Upcoming: one section per day ("Tuesday,
  Oct 13"), thumb cards. TV Networks: studio thumbs; a studio without one is
  a coloured tile with its name (`#00a4dc`, `#44bae1`, `#00455c`,
  `#1c4c5c`, `#007ea8`, picked from the name).
