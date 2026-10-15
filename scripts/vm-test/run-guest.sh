#!/usr/bin/env bash
# Types the bootstrap into the focused terminal of a test VM, waits for the
# guest test to finish and prints a summary.   run-guest.sh NAME [MODE]
set -uo pipefail
HERE="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")" && pwd)"
NAME="$1"; MODE="${2:-package}"
R="${JB_VMTEST_DIR:-$HOME/.local/share/jellybridge-vmtest}/www/results/$NAME-$MODE"
rm -rf "$R"
"$HERE/vm.py" type "$NAME" "clear; (curl -fsS 10.0.2.2:8765/t.sh || wget -qO- 10.0.2.2:8765/t.sh) | bash -s $NAME $MODE\n"
timeout 120 bash -c "until [[ -s '$R/status' ]]; do sleep 2; done" || { echo "guest never reported: is the terminal focused?"; exit 1; }
# The whole desktop while the app runs: window decorations, dock icon.
if timeout 600 bash -c "until grep -qsE 'running|done|error' '$R/status'; do sleep 2; done" && grep -qs running "$R/status"; then
    sleep 22
    "$HERE/vm.py" shot "$NAME" "$R/00-desktop.png" >/dev/null
fi
timeout 900 bash -c "until grep -qsE 'done|error' '$R/status'; do sleep 3; done"
echo "status: $(cat "$R/status")"
ls "$R"
grep -E '^(PRETTY_NAME|session)' "$R/env.txt" 2>/dev/null
grep -E 'glibc' "$R/env.txt" 2>/dev/null
cat "$R/install-check.txt" 2>/dev/null | grep -vE '^/usr/share/icons|\.png$'
grep -iE 'steps=|libmpv loaded|hwdec-current|hardware|ERROR|WARN|could not|failed|panick' "$R/app.log" 2>/dev/null | grep -v 'expect bad rendering' | cut -c1-220 | head -25
tail -1 "$R/app.log" 2>/dev/null
