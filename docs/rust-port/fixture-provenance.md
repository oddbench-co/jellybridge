# Jellyfin Fixture Provenance

No real Jellyfin credentials, URLs, tokens, user IDs, or media paths may be
committed. Captured fixtures must replace identifiers consistently so relations
between series, seasons, episodes, media sources, and sessions remain testable.

## Required fixture sets

| Set | Status | Required source |
|---|---|---|
| Jellyfin 10.9 | Missing | Disposable 10.9 server populated with synthetic media |
| Jellyfin 10.11.11 | Missing | Disposable 10.11.11 server populated with the same media |
| Jellyfin 12 canary | Missing | Latest available 12 pre-release/stable image |

Each set must capture authentication, system information, home rows, paged item
queries, item details, seasons, next-up, playback information with multiple
media sources, watched mutations, and playback reports.

## Redaction contract

- Replace access tokens and passwords with fixed sentinel values.
- Replace hostnames with `jellyfin.invalid`.
- Replace IDs deterministically, preserving equality and parent relationships.
- Replace filesystem paths and personal names.
- Retain missing/null fields, unknown fields, and unknown enum values.
- Record server image tag/digest, capture date, request method/path/query, and
  response status next to every fixture.

Fixture absence blocks the browser/core phase. The vertical slice may use a
locally supplied server because it does not assert full contract compatibility.
