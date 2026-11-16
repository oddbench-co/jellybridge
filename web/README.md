# JellyBridge website

The site at **https://jellybridge.oddbench.co**: Home, Features, Screenshots, Download, Docs, FAQ, Privacy
and About. The visual reference is `design.png`.

Rust (axum + askama), server-rendered. All content is compiled in, so every page is rendered and
brotli/gzip-compressed once at startup (~150 ms); a request only picks prepared bytes (~0.2 ms, ~25 MB
RSS). Static files are embedded by `build.rs` with content-hashed URLs and served as immutable. One
small same-origin script (`static/js/site.js`, 1.4 KB) adds the "Download for <your system>" button, the
FAQ filter and the carousel arrows; every page works without it. The CSP allows no inline code and
nothing from other origins.

This is its own cargo workspace: the app's workspace at the repository root does not build it.

```bash
cd web
cargo run                         # http://localhost:8080  (PORT, BIND, SITE_URL env vars)
cargo test                        # pages render, 404s, links resolve, SEO metadata, caching, CSP
cargo clippy --all-targets -- -D warnings
cargo build --release             # one ~3.7 MB binary with fonts, images and pages inside
e2e/run.sh                        # end-to-end and visual tests (below)
```

| Path | What |
| --- | --- |
| `src/content.rs` | Every word on the site: features, downloads, FAQ, docs pages. Facts come from the repository's README and docs |
| `src/pages.rs` | Template structs, SEO metadata and JSON-LD; renders the whole site at startup |
| `src/assets.rs`, `build.rs` | Embedded assets, precompressed responses, ETag/304 |
| `templates/` | askama templates (`base.html` layout, one per page) |
| `static/css/site.css` | All styles; design tokens on `:root` |
| `static/img/icons.svg` | Icon sprite, used through `<use href>` |
| `static/img/screens/` | The app screenshots as WebP at 640 and 1280 px (`tools/images.sh` makes them from `../docs/screenshots`) |
| `static/img/mountains.svg` | About page landscape (`tools/landscape.py`) |
| `static/img/og.jpg` | Social preview card, 1200x630 (`node tools/og.mjs` with the site running). Retake it if the hero changes |

**Releasing a version of the app:** set `VERSION` in `src/content.rs` and, once the GitHub release is
published, `RELEASED = true`. Until then the Download page says the release is being prepared and links
to the Releases page; afterwards every package links straight to its file.

**Adding a FAQ entry or a doc page:** append to `FAQS` or `DOCS` in `src/content.rs`. Doc pages are
grouped in the sidebar by `Group`, appear in the sitemap and get previous/next links automatically. The
tests check title (≤ 60 characters) and description (110–165) lengths, so search results show them whole.

## End-to-end and visual tests

`e2e/` is a Playwright suite run in Chromium, Firefox and WebKit at phone (375), tablet (820) and desktop
(1440) widths. It checks that every page renders without console errors, failed requests, broken images,
missing icons or sideways scrolling; that it passes axe accessibility checks (WCAG 2.2 AA) and has touch
targets of at least 24 px; that it matches the approved screenshot (`e2e/snapshots/`); and that the
scripted parts work, with and without JavaScript.

`e2e/run.sh` builds the site, serves it on :8090 and runs the suite in the official Playwright container
(podman or docker), so browsers, fonts and anti-aliasing are identical on every machine and in CI, and
screenshots compare pixel for pixel. Arguments go to `playwright test`:

```bash
e2e/run.sh                              # everything (~1.5 min)
e2e/run.sh --project=webkit-phone       # one browser and size
e2e/run.sh --update-snapshots           # accept a deliberate visual change, then review the PNG diff in git
```

The HTML report is written to `e2e/report/`.

## Deploy

Runs on the Oddbench production server (one k3s node at IONOS, 212.132.117.42), app `jellybridge-site` in
the namespace `jellybridge`. Cloudflare proxies `jellybridge.oddbench.co`. This repository holds no
Kubernetes files: the whole configuration (image, env, probes, resources, ingress host) lives in the private
infra repository `oddbench-co/infra` under `apps/jellybridge-site/values.yaml`, with `docs/BOOTSTRAP.md`
there as the reference, and Argo CD applies it by itself.

- **Release:** push to `main` (a change under `web/`; documentation and `web/tools/` do not start a
  build). The "Website" workflow (`.github/workflows/site.yml`) runs the tests and the e2e suite and builds
  `ghcr.io/oddbench-co/jellybridge-site:<sha>`; its last job, `deploy`, runs
  `tools/bump-image.sh jellybridge-site <sha>` in the infra repository, which commits
  `deploy: jellybridge-site <sha>` there, and Argo CD rolls it out. There is no pull request to merge and
  no sync to start. The site is stateless, so it rolls over without downtime.
- **Rollback:** revert that `deploy: jellybridge-site <sha>` commit in the infra repository.
- **Env:** `SITE_URL`, `PORT`/`BIND`, and the public ownership tokens `GOOGLE_SITE_VERIFICATION` /
  `AHREFS_SITE_VERIFICATION`, plus `AHREFS_ANALYTICS_KEY` (unset renders nothing; when set, an async
  Ahrefs script is admitted by the CSP and disclosed in the footer and on the Privacy page). Production
  sets them in the values file.
