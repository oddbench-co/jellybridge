#!/usr/bin/env bash
# Stages what the test VMs download (packages, scenario, guest script, the
# test session) into ~/.local/share/jellybridge-vmtest/www and starts the
# payload server on 127.0.0.1:8765 if it is not running.
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
# The developer's own session, so the VM is signed in; served on loopback only.
cp "${XDG_CONFIG_HOME:-$HOME/.config}/jellybridge/session.json" "$WWW/session.json"
VERSION="$(sed -n 's/^version = "\(.*\)"/\1/p' "$PROJECT_DIR/rust/app/Cargo.toml" | head -1)"
echo "version=$VERSION" > "$WWW/manifest.txt"
if ! curl -fsS -o /dev/null http://127.0.0.1:8765/manifest.txt 2>/dev/null; then
    nohup python3 "$HERE/serve.py" "$WWW" 8765 > "$STATE/serve.log" 2>&1 &
    sleep 0.5
fi
ls "$WWW"
