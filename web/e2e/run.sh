#!/bin/bash
# End-to-end and visual tests: builds the site, serves it on :8090 and runs Playwright in the
# official container (same browsers, fonts and rendering everywhere, so screenshots compare exactly).
#
#   e2e/run.sh                      # all browsers and sizes
#   e2e/run.sh --project=chromium-phone
#   e2e/run.sh --update-snapshots   # accept the current look as the new baseline (review the diff first)
set -euo pipefail
cd "$(dirname "$0")/.."
PORT="${PORT:-8090}"
IMAGE="mcr.microsoft.com/playwright:v$(node -p 'require("./e2e/node_modules/@playwright/test/package.json").version' 2>/dev/null || echo 1.64.1)-noble"
ENGINE="$(command -v podman || command -v docker)"

[ -d e2e/node_modules ] || (cd e2e && npm ci)
cargo build --release --locked
PORT="$PORT" BIND=127.0.0.1 target/release/jellybridge-site >/dev/null 2>&1 &
SERVER=$!
trap 'kill $SERVER 2>/dev/null' EXIT
for _ in $(seq 50); do curl -sf "http://127.0.0.1:$PORT/healthz" >/dev/null && break; sleep 0.1; done

"$ENGINE" run --rm --network host --ipc host --init \
  -e CI -e BASE_URL="http://127.0.0.1:$PORT" \
  -v "$PWD/e2e:/e2e:Z" -w /e2e "$IMAGE" \
  npx playwright test "$@"
