#!/usr/bin/env bash
set -euo pipefail

SCRIPT_DIR="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")" && pwd)"
PROJECT_DIR="$(cd -- "$SCRIPT_DIR/.." && pwd)"
development_mpv_prefix="${JELLYBRIDGE_DEVELOPMENT_MPV_PREFIX:-}"
pkg_paths=()
case "$(uname -s)/$(uname -m)" in
    Darwin/arm64)
        if ! command -v brew >/dev/null 2>&1; then
            echo "Homebrew is required for the macOS build" >&2
            exit 2
        fi
        rustup_bin="$(brew --prefix rustup)/bin"
        if [[ -z "$development_mpv_prefix" ]]; then
            mpv_prefix="$(brew --prefix mpv)"
        else
            mpv_prefix="$development_mpv_prefix"
        fi
        pkg_paths+=("$mpv_prefix/lib/pkgconfig")
        if [[ -z "$development_mpv_prefix" ]]; then
            while IFS= read -r formula; do
                pkg_paths+=("$(brew --prefix "$formula")/lib/pkgconfig")
            done < <(brew deps --installed mpv)
            for formula in ffmpeg zlib bzip2 expat libxml2; do
                if brew --prefix "$formula" >/dev/null 2>&1; then
                    pkg_paths+=("$(brew --prefix "$formula")/lib/pkgconfig")
                fi
            done
        fi
        rust_host="aarch64-apple-darwin"
        export MACOSX_DEPLOYMENT_TARGET="13.0"
        export LIBRARY_PATH="$mpv_prefix/lib${LIBRARY_PATH:+:$LIBRARY_PATH}"
        export DYLD_FALLBACK_LIBRARY_PATH="$mpv_prefix/lib${DYLD_FALLBACK_LIBRARY_PATH:+:$DYLD_FALLBACK_LIBRARY_PATH}"
        ;;
    Linux/x86_64)
        rustup_path="$(command -v rustup || true)"
        if [[ -z "$rustup_path" ]]; then
            echo "rustup is required for the Linux build" >&2
            exit 2
        fi
        rustup_bin="$(dirname -- "$rustup_path")"
        rust_host="x86_64-unknown-linux-gnu"
        if [[ -n "$development_mpv_prefix" ]]; then
            mpv_prefix="$development_mpv_prefix"
            pkg_paths+=(
                "$mpv_prefix/lib/pkgconfig"
                "$mpv_prefix/lib64/pkgconfig"
                "$mpv_prefix/share/pkgconfig"
            )
            export LIBRARY_PATH="$mpv_prefix/lib:$mpv_prefix/lib64${LIBRARY_PATH:+:$LIBRARY_PATH}"
        fi
        ;;
    *)
        echo "unsupported host: $(uname -s)/$(uname -m)" >&2
        exit 2
        ;;
esac
export PATH="$rustup_bin:$PATH"
export RUSTUP_TOOLCHAIN="1.98.0-$rust_host"
if ((${#pkg_paths[@]} > 0)); then
    jellybridge_pkg_config_path="$(IFS=:; echo "${pkg_paths[*]}")"
    export PKG_CONFIG_PATH="$jellybridge_pkg_config_path${PKG_CONFIG_PATH:+:$PKG_CONFIG_PATH}"
fi

cd "$PROJECT_DIR"
exec cargo "$@"
