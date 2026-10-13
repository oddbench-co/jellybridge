#!/usr/bin/env bash
# Shoots the app's Home and compares it with jellyfin-web's at the same size.
# usage: check-home.sh <workdir>   (needs <workdir>/web-home.png)
set -e
P=$1; HERE=$(cd "$(dirname "$0")" && pwd)
printf "4.0 move 2535 1389\n5.0 shot $P/app-home.png\n" > "$P/a-home.txt"
"$HERE/app-shot.sh" "$P/a-home.txt" 7
python3 "$HERE/compare.py" "$P/web-home.png" "$P/app-home.png" "$P/cmp-home.png"
