#!/usr/bin/env bash
# Profiling harness for the scenarios in docs/rust-port/performance-protocol.md.
#
#   scripts/profile.sh build              build the profiling binary
#   scripts/profile.sh run   [label]      run with frame metrics + RSS sampling
#   scripts/profile.sh cpu   [label]      same, under samply (CPU flamegraph)
#
# Options (before the mode):
#   --cold-artwork    move ~/.cache/jellybridge aside for the run, then restore
#   --duration N      kill the app after N seconds (unattended scenarios)
#
# Artifacts land in target/profiling-runs/<label>-<stamp>/:
#   app.log     the app's tracing output, including the time_to_interactive marker
#   rss.csv     elapsed_ms,rss_kib,pss_kib sampled every 200 ms
#   maps.txt    per-library resident/proportional breakdown, taken once settled
#   profile.json.gz  (cpu mode) open with: samply load profile.json.gz
set -euo pipefail

SCRIPT_DIR="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")" && pwd)"
PROJECT_DIR="$(cd -- "$SCRIPT_DIR/.." && pwd)"
cd "$PROJECT_DIR"

SAMPLY="${SAMPLY:-$HOME/.cargo/bin/samply}"
# A separate target dir: the RUSTFLAGS below change the fingerprint, and
# sharing one with ordinary builds would make every switch a full rebuild.
export CARGO_TARGET_DIR="$PROJECT_DIR/target/profiling"
# Frame pointers are what a sampling profiler walks; without them the stacks
# are truncated at the first non-leaf frame.
export RUSTFLAGS="${RUSTFLAGS:-} -C force-frame-pointers=yes"

BINARY="$CARGO_TARGET_DIR/profiling/jellybridge"
cold_artwork=0
duration=""
while [[ $# -gt 0 && "$1" == --* ]]; do
    case "$1" in
        --cold-artwork) cold_artwork=1; shift ;;
        --duration) duration="$2"; shift 2 ;;
        *) echo "unknown option: $1" >&2; exit 2 ;;
    esac
done
mode="${1:-run}"
label="${2:-${mode}}"

build() {
    echo "building the profiling binary (separate target dir, frame pointers on)" >&2
    "$SCRIPT_DIR/cargo-env.sh" build --locked --profile profiling
}

case "$mode" in
    build) build; echo "$BINARY"; exit 0 ;;
    run|cpu) ;;
    *) echo "usage: $0 [--cold-artwork] [--duration N] {build|run|cpu} [label]" >&2; exit 2 ;;
esac

build

stamp="$(date +%Y%m%d-%H%M%S)"
out="$PROJECT_DIR/target/profiling-runs/${label}-${stamp}"
mkdir -p "$out"

artwork_cache="${XDG_CACHE_HOME:-$HOME/.cache}/jellybridge"
stashed=""
restore_artwork() {
    if [[ -n "$stashed" && -d "$stashed" ]]; then
        rm -rf "$artwork_cache"
        mv "$stashed" "$artwork_cache"
        echo "restored the artwork cache" >&2
    fi
}
if ((cold_artwork)) && [[ -d "$artwork_cache" ]]; then
    stashed="${artwork_cache}.profiling-stash-$$"
    mv "$artwork_cache" "$stashed"
    trap restore_artwork EXIT
    echo "artwork cache moved aside for a cold run" >&2
fi

# refresh_lazy measures only frames that were actually requested, which is what
# an interactive app does; refresh_full_speed would measure a benchmark loop
# the app never runs.
export SLINT_DEBUG_PERFORMANCE="refresh_lazy,console"
export RUST_LOG="${RUST_LOG:-jellybridge=debug}"

if [[ "$mode" == cpu ]]; then
    [[ -x "$SAMPLY" ]] || { echo "samply not found at $SAMPLY (cargo install samply)" >&2; exit 3; }
    # No --duration here: samply writes its profile when the process it
    # launched exits, so the run is bounded by stopping the app instead.
    set -- "$SAMPLY" record --save-only --no-open --output "$out/profile.json.gz" "$BINARY"
else
    set -- "$BINARY"
fi

"$@" >"$out/app.log" 2>&1 &
app_pid=$!

# Sample RSS of the app itself. Under samply the direct child is samply, so
# find the jellybridge process rather than assuming $app_pid is it.
# RSS counts every resident page of a shared mapping against us; PSS divides
# each by the number of processes sharing it. The GPU driver mappings are the
# reason the two differ here, so both are recorded.
(
    started=$(date +%s%3N)
    target=""
    mapped=0
    while kill -0 "$app_pid" 2>/dev/null; do
        if [[ -z "$target" || ! -r "/proc/$target/status" ]]; then
            target="$(pgrep -n -u "$UID" -x jellybridge 2>/dev/null || true)"
        fi
        if [[ -n "$target" && -r "/proc/$target/status" ]]; then
            rss="$(awk '/^VmRSS:/{print $2}' "/proc/$target/status" 2>/dev/null || true)"
            pss="$(awk '/^Pss:/{print $2}' "/proc/$target/smaps_rollup" 2>/dev/null || true)"
            [[ -n "$rss" ]] && echo "$(( $(date +%s%3N) - started )),$rss,${pss:-}"
            # One per-library breakdown, once the app has settled, so the share
            # belonging to the graphics driver is attributable rather than
            # guessed at.
            if ((!mapped)) && (( $(date +%s%3N) - started > 8000 )); then
                mapped=1
                awk '
                    /^[0-9a-f]+-[0-9a-f]+ / { path = ($6 == "" ? "[anon]" : $6); next }
                    /^Rss:/  { rss[path]  += $2 }
                    /^Pss:/  { pss[path]  += $2 }
                    END {
                        printf "%10s %10s  %s\n", "RSS_KiB", "PSS_KiB", "MAPPING"
                        for (p in rss) printf "%10d %10d  %s\n", rss[p], pss[p], p
                    }
                ' "/proc/$target/smaps" 2>/dev/null | sort -rn >"$out/maps.txt" || true
            fi
        fi
        sleep 0.2
    done
) >"$out/rss.csv" &
sampler_pid=$!

if [[ -n "$duration" ]]; then
    # Stop the app itself, never the profiler wrapping it. SIGINT is not
    # reliably honoured by the winit event loop, so go straight to SIGTERM.
    (
        sleep "$duration"
        pkill -TERM -u "$UID" -x jellybridge 2>/dev/null || true
        sleep 5
        pkill -KILL -u "$UID" -x jellybridge 2>/dev/null || true
    ) &
fi

wait "$app_pid" || true
wait "$sampler_pid" 2>/dev/null || true

echo
echo "run: $out"
grep -h "time_to_interactive" "$out/app.log" || echo "  (no time_to_interactive marker in the log)"
grep -h "image_cache_high_water" "$out/app.log" | tail -1 || true
if [[ -s "$out/rss.csv" ]]; then
    awk -F, 'NR>1{if($2>peak)peak=$2; last=$2; if($3!=""){plast=$3; if($3>ppeak)ppeak=$3}} END{
        printf "  peak RSS   %.1f MiB   peak PSS  %.1f MiB\n", peak/1024, ppeak/1024
        printf "  final RSS  %.1f MiB   final PSS %.1f MiB\n", last/1024, plast/1024}' "$out/rss.csv"
fi
if [[ -s "$out/maps.txt" ]]; then
    echo "  top mappings:"
    sed -n '2,6p' "$out/maps.txt" | sed 's/^/    /'
fi
grep -hE "^Slint: |frames? per second|FPS" "$out/app.log" | tail -5 || true
