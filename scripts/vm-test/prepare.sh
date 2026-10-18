#!/usr/bin/env bash
# Stages what the test VMs download (packages, scenario, guest script, a
# test session on the demo server) into ~/.local/share/jellybridge-vmtest/www
# and starts the payload server on 127.0.0.1:8765 if it is not running.
set -euo pipefail
HERE="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")" && pwd)"
PROJECT_DIR="$(cd -- "$HERE/../.." && pwd)"
STATE="${JB_VMTEST_DIR:-$HOME/.local/share/jellybridge-vmtest}"
WWW="$STATE/www"
mkdir -p "$WWW/results"
find "$WWW" -maxdepth 1 -type f -delete
cp "$PROJECT_DIR"/target/linux/dist/* "$WWW/"
cp "$HERE/guest-test.sh" "$WWW/t.sh"
# The test items live on the tester's own server: name them, never commit them.
: "${JB_TEST_SERIES_ID:?set JB_TEST_SERIES_ID (a series cleared for testing)}"
: "${JB_TEST_EPISODE_ID:?set JB_TEST_EPISODE_ID (one of its episodes)}"
sed -e "s/@SERIES_ID@/$JB_TEST_SERIES_ID/" -e "s/@EPISODE_ID@/$JB_TEST_EPISODE_ID/" \
    -e "s/@SEARCH@/${JB_TEST_SEARCH:-a}/" "$HERE/scenario.txt" > "$WWW/scenario.txt"
# The test session: a sign-in to the public demo server (the scenario's items
# live there) for the test device id, so no personal session leaves this
# machine. The id must be the one the guests use (guest-test.sh): Jellyfin
# rewrites a token's device id on mismatch, and the dashboard's parallel
# requests racing that rewrite come back 401. JB_TEST_SERVER, JB_TEST_USER and
# JB_TEST_PASSWORD point at another server; served on loopback only.
python3 - "${JB_TEST_SERVER:-https://demo.jellyfin.org/stable}" "${JB_TEST_USER:-demo}" "${JB_TEST_PASSWORD:-}" > "$WWW/session.json" <<'PY'
import json, sys, urllib.request
server, user, password = sys.argv[1].rstrip("/"), sys.argv[2], sys.argv[3]
auth = 'MediaBrowser Client="JellyBridge", Device="Desktop", DeviceId="jellybridge-vm-test", Version="0.1.0"'
request = urllib.request.Request(server + "/Users/AuthenticateByName",
    data=json.dumps({"Username": user, "Pw": password}).encode(),
    headers={"Authorization": auth, "Content-Type": "application/json"})
reply = json.load(urllib.request.urlopen(request, timeout=30))
json.dump({"serverUrl": server, "token": reply["AccessToken"], "userId": reply["User"]["Id"]}, sys.stdout)
PY
chmod 600 "$WWW/session.json"
VERSION="$(sed -n 's/^version = "\(.*\)"/\1/p' "$PROJECT_DIR/rust/app/Cargo.toml" | head -1)"
echo "version=$VERSION" > "$WWW/manifest.txt"
if ! curl -fsS -o /dev/null http://127.0.0.1:8765/manifest.txt 2>/dev/null; then
    nohup python3 "$HERE/serve.py" "$WWW" 8765 > "$STATE/serve.log" 2>&1 &
    sleep 0.5
fi
ls "$WWW"
