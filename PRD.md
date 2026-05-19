# JellyBridge — Product Requirements Document

## Overview

A native desktop Jellyfin client built with Kotlin and Compose Multiplatform, using MPV as the video playback engine. The goal is to replace the existing Electron-based Jellyfin Media Player with a lightweight, native alternative that avoids transcoding entirely by relying on MPV's broad codec support.

---

## Problem Statement

The official Jellyfin desktop client (Jellyfin Media Player) is an Electron application — it ships a full Chromium instance, is resource-heavy, and relies on the browser's limited codec support which often forces server-side transcoding. Transcoding wastes server CPU, introduces latency, and reduces quality.

---

## Goals

- Native desktop application (no Electron, no browser engine)
- Zero transcoding — direct play only via MPV
- Lightweight resource footprint
- Full Jellyfin library browsing and playback experience
- Cross-platform: Linux first, Windows/macOS later

---

## Non-Goals (v1)

- Transcoding support
- Live TV / DVR
- Mobile clients
- Offline/download support
- Multi-server management

---

## Architecture

```
Kotlin/Compose Multiplatform (UI layer)
├── Auth module
├── Library browser
├── Playback controller
│   ├── Direct play URL builder
│   ├── Resume point manager
│   └── Subtitle manager
└── MPV process manager
    ├── Process lifecycle (spawn/kill)
    ├── IPC socket client (JSON-RPC)
    └── Position polling loop
```

### Key design decision: no transcoding

MPV supports virtually every codec and container natively (H.264, HEVC, AV1, VP9, MKV, MP4, TS, FLAC, DTS, TrueHD, etc.). The client always requests direct play from Jellyfin using `?static=true`. If a file cannot be direct played, the user is shown an error — no transcoding fallback.

---

## Tech Stack

| Component | Technology |
|---|---|
| UI framework | Kotlin + Compose Multiplatform |
| Video playback | MPV (subprocess, IPC control) |
| Jellyfin API | REST + WebSocket (Kotlin HTTP client) |
| IPC transport | Unix socket (Linux) / Named pipe (Windows) |

---

## Functional Requirements

### Authentication

- `POST /Users/AuthenticateByName` with username + password
- Store token and `UserId` securely in local config
- Support manual server URL entry
- Auto-connect on launch if credentials are saved

### Library Browsing

- `GET /Users/{userId}/Items` — list libraries (Movies, TV Shows, Music, etc.)
- `GET /Users/{userId}/Items?ParentId={id}` — browse library contents
- `GET /Users/{userId}/Items?SearchTerm={q}` — search
- Display: poster, title, year, rating, watched state
- Pagination support

### Playback

- Construct direct play URL:
  ```
  /Videos/{itemId}/stream?static=true&mediaSourceId={id}&api_key={token}
  ```
- Pass URL to MPV subprocess on launch
- Report playback start to Jellyfin:
  ```
  POST /Sessions/Playing
  ```
- Report playback stopped:
  ```
  POST /Sessions/Playing/Stopped
  ```

### Resume Points

- On item open: read `UserData.PlaybackPositionTicks` from item metadata
- Convert ticks to seconds: `ticks / 10,000,000`
- Pass to MPV: `--start={seconds}`
- Poll MPV position every 10 seconds via IPC:
  ```json
  { "command": ["get_property", "time-pos"] }
  ```
- Report progress to Jellyfin:
  ```
  POST /Sessions/Playing/Progress
  Body: { PositionTicks, IsPaused, PlayMethod: "DirectPlay" }
  ```
- Mark as watched when >90% played

### Subtitle Track Selection

- Parse `MediaStreams[]` from `POST /Items/{itemId}/PlaybackInfo`
- List all subtitle streams to the user (language, format, external/embedded)
- For text subtitles (SRT, ASS): pass delivery URL to MPV:
  ```
  --sub-file=http://jellyfin:8096/Videos/{id}/Subtitles/{index}/Stream.srt
  ```
- For image subtitles (PGS, DVD): pass embedded stream index to MPV:
  ```
  --sid={streamIndex}
  ```
- Allow subtitle switching mid-playback via MPV IPC:
  ```json
  { "command": ["sub-add", "<url>", "select"] }
  ```
- Report active subtitle index in progress updates

### MPV Process Manager

- Spawn MPV with IPC socket flag: `--input-ipc-server=/tmp/mpv-jellyfin.sock`
- Connect to socket after spawn
- Send/receive JSON-RPC commands
- Handle process exit (user closed MPV window)
- Emit events: `onPositionChange`, `onPaused`, `onStopped`

---

## API Reference (Jellyfin endpoints used)

| Endpoint | Method | Purpose |
|---|---|---|
| `/Users/AuthenticateByName` | POST | Login |
| `/Users/{userId}/Items` | GET | Browse library |
| `/Items/{itemId}/PlaybackInfo` | POST | Get media streams + subtitle info |
| `/Videos/{itemId}/stream` | GET | Direct play stream URL |
| `/Sessions/Playing` | POST | Report playback start |
| `/Sessions/Playing/Progress` | POST | Report position (every 10s) |
| `/Sessions/Playing/Stopped` | POST | Report playback end |

---

## MVP Milestone

The v1 milestone is considered complete when:

1. User can log in to a Jellyfin server
2. User can browse their movie/show library
3. User can start playback of any item via MPV (direct play only)
4. Resume position is saved and restored
5. Subtitle tracks can be selected before and during playback

---

## Out of Scope for v1 — Future Considerations

- Transcoding fallback
- Hardware decode capability detection
- Audio track switching UI
- Custom OSD / UI overlay on the video surface (libmpv render API)
- Watched/unwatched toggle from UI
- Collections, playlists, favorites
- Keyboard shortcut customization
- Live TV
- Windows / macOS support
