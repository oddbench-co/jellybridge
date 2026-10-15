#!/usr/bin/env bash
# Builds the portable Linux bundle in a throwaway Ubuntu 22.04 container, so
# the binary runs on any distro with glibc 2.35 or newer and nothing is
# installed on the host. Needs podman or docker.
#
#   scripts/build-linux.sh      -> target/linux/bundle/  (usr/bin, usr/lib/jellybridge/lib, usr/share)
#
# libmpv 0.41 is built LGPL (-Dgpl=false) with FFmpeg, dav1d, libass and
# libplacebo linked into it statically, so the bundle carries exactly one
# native library. GL/EGL, VA-API, Wayland/X11, audio and fonts stay the
# host's own: those talk to drivers and must match the installed system.
# See docs/rust-port/native-dependency-policy.md.
set -euo pipefail
PROJECT_DIR="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")/.." && pwd)"
ENGINE="$(command -v podman || command -v docker)" || { echo "needs podman or docker" >&2; exit 2; }
# Sources, the built mpv prefix and crates are cached between builds; only
# the app is rebuilt when the native versions in the container script stay.
CACHE="${XDG_CACHE_HOME:-$HOME/.cache}/jellybridge-linux-build"
mkdir -p "$CACHE/registry" "$CACHE/native" "$CACHE/rustup"
"$ENGINE" run --rm \
    -v "$PROJECT_DIR:/src" \
    -v "$CACHE/registry:/root/.cargo/registry" \
    -v "$CACHE/rustup:/root/.rustup" \
    -v "$CACHE/native:/native" \
    docker.io/library/ubuntu:22.04 bash /src/scripts/linux/build-in-container.sh
# Docker runs as real root: hand the output back. Rootless podman needs nothing.
[[ "$ENGINE" == *docker ]] && "$ENGINE" run --rm -v "$PROJECT_DIR:/src" docker.io/library/ubuntu:22.04 \
    chown -R "$(id -u):$(id -g)" /src/target/linux
echo "$PROJECT_DIR/target/linux/bundle"
