#!/usr/bin/env bash
set -euo pipefail

if [[ $# -ne 1 ]]; then
    echo "usage: $0 app|dmg|deb|appimage" >&2
    exit 2
fi

format="$1"
case "$format" in
    app|dmg|deb|appimage) ;;
    *)
        echo "unsupported package format: $format" >&2
        exit 2
        ;;
esac

script_dir="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")" && pwd)"
project_dir="$(cd -- "$script_dir/.." && pwd)"
native_bundle="${JELLYBRIDGE_RELEASE_NATIVE_BUNDLE:-}"
packager_bin="${CARGO_PACKAGER_BIN:-}"

if [[ -z "$native_bundle" || ! -d "$native_bundle" ]]; then
    echo "JELLYBRIDGE_RELEASE_NATIVE_BUNDLE must name the audited native bundle" >&2
    exit 2
fi
native_bundle="$(cd -- "$native_bundle" && pwd)"

if [[ -z "$packager_bin" ]]; then
    packager_bin="$(command -v cargo-packager || true)"
fi
if [[ -z "$packager_bin" || ! -x "$packager_bin" ]]; then
    echo "cargo-packager 0.11.8 is required; set CARGO_PACKAGER_BIN if it is not on PATH" >&2
    exit 2
fi
if [[ "$($packager_bin --version)" != "cargo-packager 0.11.8" ]]; then
    echo "cargo-packager 0.11.8 is required" >&2
    exit 2
fi

case "$(uname -s)/$(uname -m)" in
    Darwin/arm64)
        rustup_bin="$(brew --prefix rustup)/bin"
        rust_host="aarch64-apple-darwin"
        export MACOSX_DEPLOYMENT_TARGET="13.0"
        export DYLD_FALLBACK_LIBRARY_PATH="$native_bundle/lib${DYLD_FALLBACK_LIBRARY_PATH:+:$DYLD_FALLBACK_LIBRARY_PATH}"
        ;;
    Linux/x86_64)
        rustup_path="$(command -v rustup || true)"
        if [[ -z "$rustup_path" ]]; then
            echo "rustup is required" >&2
            exit 2
        fi
        rustup_bin="$(dirname -- "$rustup_path")"
        rust_host="x86_64-unknown-linux-gnu"
        ;;
    *)
        echo "unsupported packaging host: $(uname -s)/$(uname -m)" >&2
        exit 2
        ;;
esac

export PATH="$rustup_bin:$PATH"
export RUSTUP_TOOLCHAIN="1.98.0-$rust_host"
export PKG_CONFIG_PATH="$native_bundle/lib/pkgconfig:$native_bundle/share/pkgconfig${PKG_CONFIG_PATH:+:$PKG_CONFIG_PATH}"
export LIBRARY_PATH="$native_bundle/lib${LIBRARY_PATH:+:$LIBRARY_PATH}"

cd "$project_dir"
if [[ "$(rustc --version)" != rustc\ 1.98.0* ]]; then
    echo "Rust 1.98.0 is required" >&2
    exit 2
fi

"$script_dir/prepare-slice-package.sh"
cargo build --locked --release -p jellybridge-vertical-slice

binary="$project_dir/target/release/jellybridge-player-slice"
if [[ "$(uname -s)" == "Darwin" ]]; then
    if otool -L "$binary" | grep -E '/Users/|/opt/homebrew|/usr/local' >/dev/null; then
        echo "release binary still links to a development/build-machine path" >&2
        exit 1
    fi
else
    if ! readelf -d "$binary" | grep -F 'libmpv.so' >/dev/null; then
        echo "release binary does not dynamically link libmpv" >&2
        exit 1
    fi
fi

"$packager_bin" \
    --manifest-path rust/vertical-slice/Cargo.toml \
    --release \
    --formats "$format"
