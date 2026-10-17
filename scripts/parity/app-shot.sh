#!/usr/bin/env bash
# Runs the release app on a headless 4K output (scale 1.5, workspace 19, muted,
# no playback reports), replays an input script, and saves the frames its
# `shot` steps ask for. Never touches the user's own windows.
# usage: app-shot.sh <input-script> <seconds-to-run>
set -u
SCRIPT=$1; RUN=${2:-10}
BIN=${BIN:-$(cd "$(dirname "$0")/../.." && pwd)/target/release/jellybridge}
if hyprctl clients -j | jq -e '.[] | select(.workspace.name=="19") | select(.class!="co.oddbench.JellyBridge")' >/dev/null; then
  echo "workspace 19 has user windows; aborting"; exit 1; fi
hyprctl output create headless JBTEST >/dev/null; sleep 0.5
# MODE/SCALE pick another output: MODE=1929x1107@60 gives a 1262x688
# window (both sides must divide by the scale, or Hyprland changes it).
hyprctl eval "hl.monitor({ output = 'JBTEST', mode = '${MODE:-3840x2160@240}', position = '10000x0', scale = ${SCALE:-1.5} })" >/dev/null; sleep 0.5
# Hyprland gives the new output a fresh, empty workspace of its own. Workspace
# 19 only exists once a window sits on it, so it is moved over after the app
# has started (below), and that fresh workspace is moved away so 19 becomes
# the one the output shows: a workspace that is not shown gets no frames.
FRESH=$(hyprctl monitors -j | jq -r '.[] | select(.name=="JBTEST") | .activeWorkspace.name')
LOG=${LOG:-/dev/null}
# The parity runs' own config (see use-session.sh), never the user's.
CONF=${JB_TEST_CONFIG:-${TMPDIR:-/tmp}/jellybridge-parity-config}
# EXTRA="VAR=value ..." adds environment (e.g. JELLYBRIDGE_MEASURE=10).
# NO_SESSION=1: a signed-out run (an empty config directory).
if [ -n "${NO_SESSION:-}" ]; then CONF=$(mktemp -d); else
[ -f "$CONF/jellybridge/session.json" ] || { echo "no test session: run use-session.sh"; exit 1; }; fi
hyprctl eval "hl.exec_cmd('env XDG_CONFIG_HOME=$CONF JELLYBRIDGE_DEVICE_ID=jellybridge-parity-test JELLYBRIDGE_MUTE=1 JELLYBRIDGE_NO_PLAYBACK_REPORTS=1 JELLYBRIDGE_INPUT_SCRIPT=$SCRIPT RUST_LOG=jellybridge=debug ${EXTRA:-} $BIN > $LOG 2>&1', { workspace = '19 silent' })" >/dev/null
for _ in $(seq 1 100); do
  hyprctl clients -j | jq -e '.[] | select(.class=="co.oddbench.JellyBridge" and .workspace.name=="19")' >/dev/null 2>&1 && break
  sleep 0.1
done
hyprctl eval "hl.dispatch(hl.dsp.workspace.move({ workspace='19', monitor='JBTEST' }))" >/dev/null 2>&1
if [ -n "$FRESH" ] && [ "$FRESH" != "19" ] && [ "$(hyprctl workspaces -j | jq -r --arg w "$FRESH" '.[] | select(.name==$w) | .windows')" = "0" ]; then
  hyprctl eval "hl.dispatch(hl.dsp.workspace.move({ workspace='$FRESH', monitor='DP-1' }))" >/dev/null 2>&1
fi
sleep "$RUN"
PID=$(hyprctl clients -j | jq -r '.[] | select(.class=="co.oddbench.JellyBridge" and .workspace.name=="19") | .pid' | head -1)
[ -n "$PID" ] && kill "$PID"; sleep 0.5
hyprctl output remove JBTEST >/dev/null
