#!/usr/bin/env bash
# Shoots one page in jellyfin-web (headless Chromium) and in the app at the
# same size, and diffs them.
# usage: compare-page.sh <workdir> <name> <web hash, e.g. '#/movies?topParentId=..'> <app steps file> [web-js]
#   The app steps run from launch (times in seconds); a `shot` step is added
#   at the end. web-js, if given, runs on the web page before its shot.
set -e
P=$1; NAME=$2; HASH=$3; STEPS=$4; JS=${5:-}
HERE=$(cd "$(dirname "$0")" && pwd)
SESSION=${SESSION:-${JB_TEST_CONFIG:-${TMPDIR:-/tmp}/jellybridge-parity-config}/jellybridge/session.json}
{
  echo "js location.hash = '$HASH'; 1"
  echo "wait 4000"
  [ -n "$JS" ] && { echo "js $JS"; echo "wait 1200"; }
  echo "move 2535 1389"
  echo "wait 600"
  echo "shot $P/web-$NAME.png"
} > "$P/w-$NAME.txt"
node "$HERE/web.mjs" "$SESSION" "$P/w-$NAME.txt" 2536 1390 >/dev/null
LAST=$(awk '{t=$1} END {print t+0}' "$STEPS")
{ cat "$STEPS"; awk -v t="$LAST" -v f="$P/app-$NAME.png" 'BEGIN { print t + 3 " move 2535 1389"; print t + 4 " shot " f }'; } > "$P/a-$NAME.txt"
"$HERE/app-shot.sh" "$P/a-$NAME.txt" "$(awk -v t="$LAST" 'BEGIN { print int(t + 6) }')"
python3 "$HERE/compare.py" "$P/web-$NAME.png" "$P/app-$NAME.png" "$P/cmp-$NAME.png"
