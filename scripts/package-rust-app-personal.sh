#!/usr/bin/env bash
set -euo pipefail

# Creates a self-contained macOS package from the complete Homebrew mpv graph.
# This intentionally includes GPL-enabled FFmpeg dependencies and is for the
# owner's personal use only. Public release packages must continue to use the
# audited native-bundle workflow in package-rust-slice.sh.

script_dir="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")" && pwd)"
project_dir="$(cd -- "$script_dir/.." && pwd)"

if [[ "$(uname -s)/$(uname -m)" != "Darwin/arm64" ]]; then
    echo "the personal package currently supports only macOS arm64" >&2
    exit 2
fi

for tool in brew dylibbundler hdiutil install_name_tool codesign otool; do
    if ! command -v "$tool" >/dev/null 2>&1; then
        echo "$tool is required" >&2
        exit 2
    fi
done

packager_bin="${CARGO_PACKAGER_BIN:-}"
if [[ -z "$packager_bin" ]]; then
    packager_bin="$(command -v cargo-packager || true)"
fi
if [[ -z "$packager_bin" && -x "$HOME/.cargo/bin/cargo-packager" ]]; then
    packager_bin="$HOME/.cargo/bin/cargo-packager"
fi
if [[ -z "$packager_bin" || ! -x "$packager_bin" ]]; then
    echo "cargo-packager 0.11.8 is required" >&2
    exit 2
fi
if [[ "$($packager_bin --version)" != "cargo-packager 0.11.8" ]]; then
    echo "cargo-packager 0.11.8 is required" >&2
    exit 2
fi

rustup_bin="$(brew --prefix rustup)/bin"
mpv_prefix="$(brew --prefix mpv)"
export PATH="$rustup_bin:$PATH"
export RUSTUP_TOOLCHAIN="1.98.0-aarch64-apple-darwin"
export MACOSX_DEPLOYMENT_TARGET="13.0"
export PKG_CONFIG_PATH="$mpv_prefix/lib/pkgconfig${PKG_CONFIG_PATH:+:$PKG_CONFIG_PATH}"
export LIBRARY_PATH="$mpv_prefix/lib${LIBRARY_PATH:+:$LIBRARY_PATH}"
export DYLD_FALLBACK_LIBRARY_PATH="$mpv_prefix/lib${DYLD_FALLBACK_LIBRARY_PATH:+:$DYLD_FALLBACK_LIBRARY_PATH}"

cd "$project_dir"
"$script_dir/run-rust-slice.sh" build --locked --release -p jellybridge-app
"$packager_bin" \
    --manifest-path Cargo.toml \
    --packages jellybridge-app \
    --release \
    --formats app

app_path="$project_dir/target/release/JellyBridge.app"
binary_path="$app_path/Contents/MacOS/jellybridge"
frameworks_path="$app_path/Contents/Frameworks"

dylibbundler \
    -od \
    -b \
    -x "$binary_path" \
    -d "$frameworks_path" \
    -p @executable_path/../Frameworks/

# dylibbundler rewrites both of Homebrew libmpv's Swift rpaths to the same
# destination. macOS 26 rejects the dylib when duplicate LC_RPATH commands
# remain, so retain exactly one copy.
mpv_dylib="$frameworks_path/libmpv.2.dylib"
framework_rpath="@executable_path/../Frameworks/"
rpath_count="$(
    otool -l "$mpv_dylib" |
        awk -v expected="$framework_rpath" '$1 == "path" && $2 == expected { count++ } END { print count + 0 }'
)"
while ((rpath_count > 1)); do
    install_name_tool -delete_rpath "$framework_rpath" "$mpv_dylib"
    rpath_count=$((rpath_count - 1))
done

codesign --force --sign - "$mpv_dylib"
# Personal ad-hoc dependencies do not share a Developer ID team. Do not add
# hardened runtime here; dyld otherwise rejects the embedded libraries.
codesign --force --deep --sign - "$app_path"
codesign --verify --deep --strict --verbose=2 "$app_path"

mach_o_files=("$binary_path")
while IFS= read -r -d '' dylib; do
    mach_o_files+=("$dylib")
done < <(find "$frameworks_path" -maxdepth 1 -type f -print0)

for mach_o in "${mach_o_files[@]}"; do
    if otool -L "$mach_o" | tail -n +2 | grep -Eq '/opt/homebrew|/usr/local|/Users/'; then
        echo "build-machine dependency remains in $mach_o" >&2
        exit 1
    fi
    while IFS= read -r dependency; do
        case "$dependency" in
            @executable_path/../Frameworks/*)
                dependency_name="${dependency##*/}"
                if [[ ! -f "$frameworks_path/$dependency_name" ]]; then
                    echo "missing embedded dependency: $dependency_name" >&2
                    exit 1
                fi
                ;;
        esac
    done < <(otool -L "$mach_o" | tail -n +2 | awk '{ print $1 }')
done

app_version="$(sed -n 's/^version = "\([^"]*\)"/\1/p' rust/app/Cargo.toml | head -n 1)"
dmg_path="$project_dir/target/release/JellyBridge_${app_version}_aarch64_self-contained.dmg"
stage_dir="$(mktemp -d "${TMPDIR:-/tmp}/jellybridge-personal-dmg.XXXXXX")"
case "$stage_dir" in
    "${TMPDIR:-/tmp}"/jellybridge-personal-dmg.*) ;;
    *)
        echo "unexpected temporary directory: $stage_dir" >&2
        exit 1
        ;;
esac
cleanup() {
    rm -rf -- "$stage_dir"
}
trap cleanup EXIT

cp -R "$app_path" "$stage_dir/JellyBridge.app"
ln -s /Applications "$stage_dir/Applications"
hdiutil create \
    -volname JellyBridge \
    -srcfolder "$stage_dir" \
    -format UDZO \
    -ov \
    "$dmg_path"
hdiutil verify "$dmg_path"

echo "personal self-contained app: $app_path"
du -sh "$app_path"
echo "personal self-contained DMG: $dmg_path"
du -sh "$dmg_path"
shasum -a 256 "$dmg_path"
