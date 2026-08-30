#!/usr/bin/env bash
set -euo pipefail

if [[ $# -lt 1 || $# -gt 2 ]]; then
    echo "usage: $0 MEDIA_FILE [CYCLES]" >&2
    exit 2
fi

script_dir="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")" && pwd)"
project_dir="$(cd -- "$script_dir/.." && pwd)"
media_file="$(cd -- "$(dirname -- "$1")" && pwd)/$(basename -- "$1")"
cycles="${2:-500}"

if [[ ! -f "$media_file" || ! "$cycles" =~ ^[1-9][0-9]*$ ]]; then
    echo "MEDIA_FILE must exist and CYCLES must be a positive integer" >&2
    exit 2
fi

"$script_dir/run-rust-slice.sh" build --release -p jellybridge-vertical-slice
binary="$project_dir/target/release/jellybridge-player-slice"
log_dir="${JELLYBRIDGE_STRESS_LOG_DIR:-$project_dir/target/stress/$(date -u +%Y%m%dT%H%M%SZ)}"
auto_close_ms="${JELLYBRIDGE_STRESS_AUTO_CLOSE_MS:-1200}"
# Desktop manipulation is deliberately opt-in. Set this to 1 only on a
# dedicated certification machine where resize/fullscreen churn is expected.
stress_ui="${JELLYBRIDGE_STRESS_UI:-0}"

if [[ ! "$auto_close_ms" =~ ^[1-9][0-9]*$ ]]; then
    echo "JELLYBRIDGE_STRESS_AUTO_CLOSE_MS must be a positive integer" >&2
    exit 2
fi
if [[ "$stress_ui" != 0 && "$stress_ui" != 1 ]]; then
    echo "JELLYBRIDGE_STRESS_UI must be 0 or 1" >&2
    exit 2
fi
if [[ -e "$log_dir" ]] && find "$log_dir" -mindepth 1 -print -quit | grep -q .; then
    echo "stress log directory is not empty: $log_dir" >&2
    exit 2
fi
mkdir -p "$log_dir"

{
    echo "media=$media_file"
    echo "cycles=$cycles"
    echo "auto_close_ms=$auto_close_ms"
    echo "stress_ui=$stress_ui"
    echo "host=$(uname -s)/$(uname -m)"
    echo "started_utc=$(date -u +%Y-%m-%dT%H:%M:%SZ)"
} > "$log_dir/summary.txt"

for ((cycle = 1; cycle <= cycles; cycle++)); do
    stress_args=()
    if [[ "$stress_ui" == 1 ]]; then
        stress_args+=(--stress-ui)
    fi
    if [[ "$(uname -s)" == "Darwin" ]]; then
        /usr/bin/time -l env JELLYBRIDGE_GL_DEBUG=1 JELLYBRIDGE_GL_SENTINEL=1 \
            "$binary" --file "$media_file" --auto-close-ms "$auto_close_ms" "${stress_args[@]}" \
            >"$log_dir/cycle-$cycle.log" 2>&1
    else
        /usr/bin/time -v env JELLYBRIDGE_GL_DEBUG=1 JELLYBRIDGE_GL_SENTINEL=1 \
            "$binary" --file "$media_file" --auto-close-ms "$auto_close_ms" "${stress_args[@]}" \
            >"$log_dir/cycle-$cycle.log" 2>&1
    fi
    if grep -E 'ERROR|panic|render host dropped' "$log_dir/cycle-$cycle.log" >/dev/null; then
        echo "stress cycle $cycle failed; logs retained at $log_dir" >&2
        exit 1
    fi
    worker_line="$(grep -n 'serialized mpv worker stopped' "$log_dir/cycle-$cycle.log" | tail -1 | cut -d: -f1 || true)"
    render_line="$(grep -n 'libmpv render context and OpenGL targets torn down' "$log_dir/cycle-$cycle.log" | tail -1 | cut -d: -f1 || true)"
    if [[ -z "$worker_line" || -z "$render_line" || "$worker_line" -ge "$render_line" ]]; then
        echo "stress cycle $cycle did not prove worker-before-render teardown; logs retained at $log_dir" >&2
        exit 1
    fi
    if ((cycle % 10 == 0)); then
        echo "completed $cycle/$cycles cycles"
    fi
done

if [[ "$(uname -s)" == "Darwin" ]]; then
    grep 'maximum resident set size' "$log_dir"/cycle-*.log \
        | awk '{ print $(NF-4) }' > "$log_dir/peak-rss-bytes.txt" || true
else
    grep 'Maximum resident set size' "$log_dir"/cycle-*.log \
        | awk -F: '{ gsub(/^[[:space:]]+/, "", $2); print $2 * 1024 }' \
        > "$log_dir/peak-rss-bytes.txt" || true
fi
echo "completed_utc=$(date -u +%Y-%m-%dT%H:%M:%SZ)" >> "$log_dir/summary.txt"
echo "result=PASS" >> "$log_dir/summary.txt"
echo "completed $cycles clean open/render/close cycles; logs retained at $log_dir"
