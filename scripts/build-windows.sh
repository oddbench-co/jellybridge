#!/usr/bin/env bash
# Cross-builds the Windows release (x86_64, MinGW) and its installer in a
# throwaway container, so nothing is installed on the host. Needs podman or
# docker.
#
#   scripts/build-windows.sh
#     -> target/windows/x86_64-pc-windows-gnu/release/jellybridge.exe
#     -> target/windows/dist/JellyBridge-<version>-setup.exe   (per-user installer)
#     -> target/windows/dist/JellyBridge-<version>-portable.zip
#
# libmpv-2.dll comes from a pinned shinchiro/mpv-winbuild-cmake build (cached,
# since upstream prunes old releases). It is a GPL build, which JellyBridge's
# GPL-3.0-or-later licence allows; its sources are listed in
# THIRD-PARTY-NOTICES.md, which ships beside it.
set -euo pipefail
PROJECT_DIR="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")/.." && pwd)"
ENGINE="$(command -v podman || command -v docker)" || { echo "needs podman or docker" >&2; exit 2; }
MPV_BUILD="20261014/mpv-dev-x86_64-20261014-git-ec38361c56.7z"
MPV_SHA256="${JB_WINDOWS_MPV_SHA256:-}"
# The Khronos Vulkan loader: this libmpv imports vulkan-1.dll, which GPU
# drivers install, so a PC without vendor drivers (and Windows 10 without
# them) cannot load libmpv at all. JellyBridge renders through OpenGL and
# never creates a Vulkan instance; the loader only satisfies the import.
VULKAN_RT=1.4.363.0
VERSION="$(sed -n 's/^version = "\(.*\)"/\1/p' "$PROJECT_DIR/rust/app/Cargo.toml" | head -1)"
# Crates and the libmpv archive are cached between builds; the toolchain is
# reinstalled each time.
CACHE="${XDG_CACHE_HOME:-$HOME/.cache}/jellybridge-windows-build"
mkdir -p "$CACHE/registry" "$CACHE/mpv"
MPV_ARCHIVE="$CACHE/mpv/$(basename "$MPV_BUILD")"
if [[ ! -s "$MPV_ARCHIVE" ]]; then
    curl -fL --retry 3 -sS -o "$MPV_ARCHIVE.part" \
        "https://github.com/shinchiro/mpv-winbuild-cmake/releases/download/$MPV_BUILD"
    mv "$MPV_ARCHIVE.part" "$MPV_ARCHIVE"
fi
[[ -z "$MPV_SHA256" ]] || echo "$MPV_SHA256  $MPV_ARCHIVE" | sha256sum -c --quiet
mkdir -p "$CACHE/vulkan"
VULKAN_ARCHIVE="$CACHE/vulkan/vulkan-runtime-components-$VULKAN_RT.zip"
if [[ ! -s "$VULKAN_ARCHIVE" ]]; then
    curl -fL --retry 3 -sS -o "$VULKAN_ARCHIVE.part" \
        "https://sdk.lunarg.com/sdk/download/$VULKAN_RT/windows/vulkan-runtime-components.zip?u="
    mv "$VULKAN_ARCHIVE.part" "$VULKAN_ARCHIVE"
fi
"$ENGINE" run --rm \
    -v "$PROJECT_DIR:/src" \
    -v "$CACHE/registry:/usr/local/cargo/registry" \
    -v "$CACHE/mpv:/mpv:ro" \
    -v "$CACHE/vulkan:/vulkan:ro" \
    -e VERSION="$VERSION" -e MPV_ARCHIVE="/mpv/$(basename "$MPV_BUILD")" \
    -e VULKAN_ARCHIVE="/vulkan/$(basename "$VULKAN_ARCHIVE")" \
    docker.io/library/rust:1-bookworm sh -euc '
        apt-get update -qq >/dev/null
        apt-get install -y -qq gcc-mingw-w64-x86-64 nsis p7zip-full imagemagick zip >/dev/null
        cd /src
        rustup target add x86_64-pc-windows-gnu >/dev/null
        CARGO_TARGET_DIR=/src/target/windows cargo build --release --locked \
            --target x86_64-pc-windows-gnu -p jellybridge-app
        stage=/tmp/stage; rm -rf $stage; mkdir -p $stage
        cp /src/target/windows/x86_64-pc-windows-gnu/release/jellybridge.exe $stage/
        7z e -y -o$stage "$MPV_ARCHIVE" libmpv-2.dll >/dev/null
        7z e -y -o$stage "$VULKAN_ARCHIVE" "*/x64/vulkan-1.dll" "*/VulkanRT-License.txt" -r >/dev/null
        cp /src/LICENSE $stage/LICENSE.txt
        cp /src/THIRD-PARTY-NOTICES.md $stage/THIRD-PARTY-NOTICES.txt
        convert /src/rust/app/assets/icon.png -define icon:auto-resize=256,64,48,32,16 $stage/jellybridge.ico
        dist=/src/target/windows/dist; mkdir -p $dist
        makensis -V2 -DVERSION=$VERSION -DSTAGE=$stage \
            -DOUTFILE=$dist/JellyBridge-$VERSION-setup.exe /src/packaging/windows/installer.nsi
        rm -f $dist/JellyBridge-$VERSION-portable.zip
        (cd $stage && zip -q -9 $dist/JellyBridge-$VERSION-portable.zip jellybridge.exe libmpv-2.dll vulkan-1.dll VulkanRT-License.txt LICENSE.txt THIRD-PARTY-NOTICES.txt)
    '
ls -la "$PROJECT_DIR/target/windows/dist"
