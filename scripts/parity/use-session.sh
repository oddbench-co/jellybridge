#!/usr/bin/env bash
# Copies your signed-in JellyBridge session into a separate config directory
# that the parity runs use, so test runs and your own app never share (or
# sign out of) one session.
# usage: use-session.sh   (then export JB_TEST_CONFIG as printed, or rely on the default)
set -e
DEST=${JB_TEST_CONFIG:-${TMPDIR:-/tmp}/jellybridge-parity-config}
SRC=${XDG_CONFIG_HOME:-$HOME/.config}/jellybridge
[ -f "$SRC/session.json" ] || { echo "no session at $SRC: sign in to JellyBridge first"; exit 1; }
mkdir -p "$DEST/jellybridge"
chmod 700 "$DEST" "$DEST/jellybridge"
cp "$SRC/session.json" "$DEST/jellybridge/"
[ -f "$SRC/settings.json" ] && cp "$SRC/settings.json" "$DEST/jellybridge/"
chmod 600 "$DEST/jellybridge/"*.json
echo "test config ready: $DEST"
