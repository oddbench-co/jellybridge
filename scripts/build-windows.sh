#!/usr/bin/env bash
# Cross-builds the Windows release (x86_64, MinGW) in a throwaway container,
# so nothing is installed on the host. Needs podman or docker.
#
#   scripts/build-windows.sh            -> target/windows/x86_64-pc-windows-gnu/release/jellybridge.exe
#
# The app loads libmpv at runtime: put libmpv-2.dll (e.g. from
# shinchiro/mpv-winbuild-cmake's mpv-dev-x86_64 build) beside the exe.
set -euo pipefail
PROJECT_DIR="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")/.." && pwd)"
ENGINE="$(command -v podman || command -v docker)" || { echo "needs podman or docker" >&2; exit 2; }
# Crates are cached between builds; the toolchain is reinstalled each time.
CACHE="${XDG_CACHE_HOME:-$HOME/.cache}/jellybridge-windows-build"
mkdir -p "$CACHE/registry"
"$ENGINE" run --rm \
    -v "$PROJECT_DIR:/src" \
    -v "$CACHE/registry:/usr/local/cargo/registry" \
    docker.io/library/rust:1-bookworm sh -euc '
        apt-get update -qq >/dev/null
        apt-get install -y -qq gcc-mingw-w64-x86-64 >/dev/null
        cd /src
        rustup target add x86_64-pc-windows-gnu >/dev/null
        CARGO_TARGET_DIR=/src/target/windows cargo build --release --locked \
            --target x86_64-pc-windows-gnu -p jellybridge-app
    '
echo "$PROJECT_DIR/target/windows/x86_64-pc-windows-gnu/release/jellybridge.exe"
