#!/usr/bin/env bash
# Builds JellyBridge.app and a .dmg for this Mac's CPU (arm64 or x86_64), with
# Homebrew's mpv and its whole library graph embedded, so nothing else needs
# to be installed. Needs: brew install mpv dylibbundler, and rustup.
#
# Signing and notarization (all optional; without them the app is signed
# ad hoc, which runs on this Mac but is blocked by Gatekeeper elsewhere):
#   MACOS_SIGNING_IDENTITY  "Developer ID Application: Name (TEAMID)" in the
#                           keychain: signs everything with the hardened
#                           runtime, which notarization requires.
#   APPLE_API_KEY_PATH      App Store Connect API key (.p8), with
#   APPLE_API_KEY_ID        its key id and
#   APPLE_API_ISSUER_ID     issuer id: notarizes the .dmg and staples the
#                           ticket, so it opens without a warning, offline too.
#
# Output: target/macos/dist/JellyBridge-<version>-macos-<arm64|x86_64>.dmg
set -euo pipefail
script_dir="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")" && pwd)"
project_dir="$(cd -- "$script_dir/.." && pwd)"

[[ "$(uname -s)" == Darwin ]] || { echo "builds on macOS only" >&2; exit 2; }
for tool in brew dylibbundler hdiutil install_name_tool codesign otool; do
    command -v "$tool" >/dev/null 2>&1 || { echo "$tool is required" >&2; exit 2; }
done
identity="${MACOS_SIGNING_IDENTITY:-}"
notarize=""
if [[ -n "${APPLE_API_KEY_PATH:-}${APPLE_API_KEY_ID:-}${APPLE_API_ISSUER_ID:-}" ]]; then
    [[ -n "$identity" && -f "${APPLE_API_KEY_PATH:-}" && -n "${APPLE_API_KEY_ID:-}" && -n "${APPLE_API_ISSUER_ID:-}" ]] ||
        { echo "notarization needs MACOS_SIGNING_IDENTITY and all three APPLE_API_* variables" >&2; exit 2; }
    notarize=1
fi

arch="$(uname -m)"
mpv_prefix="$(brew --prefix mpv)"
version="$(sed -n 's/^version = "\([^"]*\)"/\1/p' "$project_dir/rust/app/Cargo.toml" | head -n 1)"
# Homebrew's libraries are built for the macOS they were installed on, so
# that is the oldest macOS the bundle can run on; the app says so, instead of
# crashing at launch on an older one.
min_macos="$(sw_vers -productVersion | cut -d. -f1).0"
export MACOSX_DEPLOYMENT_TARGET="$min_macos"

cd "$project_dir"
"$script_dir/cargo-env.sh" build --locked --release -p jellybridge-app

out="$project_dir/target/macos"
app="$out/JellyBridge.app"
rm -rf -- "${app:?}"
mkdir -p "$app/Contents/MacOS" "$app/Contents/Resources" "$out/dist"
cp target/release/jellybridge "$app/Contents/MacOS/jellybridge"
cp rust/app/assets/icon.icns "$app/Contents/Resources/icon.icns"
cp LICENSE THIRD-PARTY-NOTICES.md "$app/Contents/Resources/"
cat > "$app/Contents/Info.plist" <<PLIST
<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
  <key>CFBundleName</key><string>JellyBridge</string>
  <key>CFBundleDisplayName</key><string>JellyBridge</string>
  <key>CFBundleIdentifier</key><string>co.oddbench.JellyBridge</string>
  <key>CFBundleExecutable</key><string>jellybridge</string>
  <key>CFBundleIconFile</key><string>icon</string>
  <key>CFBundlePackageType</key><string>APPL</string>
  <key>CFBundleShortVersionString</key><string>$version</string>
  <key>CFBundleVersion</key><string>$version</string>
  <key>CFBundleInfoDictionaryVersion</key><string>6.0</string>
  <key>LSMinimumSystemVersion</key><string>$min_macos</string>
  <key>LSApplicationCategoryType</key><string>public.app-category.video</string>
  <key>NSHighResolutionCapable</key><true/>
  <key>NSHumanReadableCopyright</key><string>Oddbench. GPL-3.0-or-later.</string>
</dict>
</plist>
PLIST

# The app loads libmpv at runtime (services/libmpv.rs), so its graph is
# bundled from libmpv itself, after a first pass over the binary (whose -od
# creates the empty Frameworks folder).
frameworks="$app/Contents/Frameworks"
binary="$app/Contents/MacOS/jellybridge"
dylibbundler -od -b -x "$binary" -d "$frameworks" -p @executable_path/../Frameworks/
cp "$mpv_prefix/lib/libmpv.2.dylib" "$frameworks/libmpv.2.dylib"
chmod u+w "$frameworks/libmpv.2.dylib"
install_name_tool -id @executable_path/../Frameworks/libmpv.2.dylib "$frameworks/libmpv.2.dylib"
dylibbundler -b -x "$frameworks/libmpv.2.dylib" -d "$frameworks" -p @executable_path/../Frameworks/

# dylibbundler rewrites both of Homebrew libmpv's Swift rpaths to the same
# path; macOS 26 rejects a dylib with duplicate LC_RPATH entries.
mpv_dylib="$frameworks/libmpv.2.dylib"
rpath="@executable_path/../Frameworks/"
count="$(otool -l "$mpv_dylib" | awk -v p="$rpath" '$1 == "path" && $2 == p { n++ } END { print n + 0 }')"
while ((count > 1)); do
    install_name_tool -delete_rpath "$rpath" "$mpv_dylib"
    count=$((count - 1))
done

# Nothing may still point at the build machine, and every bundled reference
# must resolve inside the bundle.
for macho in "$binary" "$frameworks"/*; do
    if otool -L "$macho" | tail -n +2 | grep -Eq '/opt/homebrew|/usr/local|/Users/'; then
        echo "build-machine dependency remains in $macho" >&2; exit 1
    fi
    while IFS= read -r dep; do
        if [[ "$dep" == @executable_path/../Frameworks/* && ! -f "$frameworks/${dep##*/}" ]]; then
            echo "missing embedded dependency: ${dep##*/}" >&2; exit 1
        fi
    done < <(otool -L "$macho" | tail -n +2 | awk '{ print $1 }')
done

# Inside out: every library, then the app. With a Developer ID the hardened
# runtime is on; all code shares one team, so library validation passes.
if [[ -n "$identity" ]]; then
    sign=(codesign --force --timestamp --options runtime --sign "$identity")
    for lib in "$frameworks"/*; do "${sign[@]}" "$lib"; done
    "${sign[@]}" --entitlements "$project_dir/packaging/macos/entitlements.plist" "$app"
else
    for lib in "$frameworks"/*; do codesign --force --sign - "$lib"; done
    codesign --force --sign - "$app"
    echo "note: signed ad hoc (no MACOS_SIGNING_IDENTITY); Gatekeeper blocks it on other Macs" >&2
fi
codesign --verify --deep --strict --verbose=2 "$app"

dmg="$out/dist/JellyBridge-$version-macos-$arch.dmg"
stage="$(mktemp -d "${TMPDIR:-/tmp}/jellybridge-dmg.XXXXXX")"
trap 'rm -rf -- "${stage:?}"' EXIT
cp -R "$app" "$stage/JellyBridge.app"
ln -s /Applications "$stage/Applications"
rm -f -- "$dmg"
hdiutil create -volname JellyBridge -srcfolder "$stage" -format UDZO -ov "$dmg"
if [[ -n "$identity" ]]; then
    codesign --force --timestamp --sign "$identity" "$dmg"
fi

if [[ -n "$notarize" ]]; then
    result="$(xcrun notarytool submit "$dmg" --key "$APPLE_API_KEY_PATH" --key-id "$APPLE_API_KEY_ID" \
        --issuer "$APPLE_API_ISSUER_ID" --wait --timeout 45m --output-format json)"
    echo "$result"
    status="$(plutil -extract status raw - <<<"$result" 2>/dev/null || true)"
    if [[ "$status" != Accepted ]]; then
        id="$(plutil -extract id raw - <<<"$result" 2>/dev/null || true)"
        if [[ -n "$id" ]]; then
            xcrun notarytool log "$id" --key "$APPLE_API_KEY_PATH" \
                --key-id "$APPLE_API_KEY_ID" --issuer "$APPLE_API_ISSUER_ID" || true
        fi
        echo "notarization failed: ${status:-no status}" >&2; exit 1
    fi
    xcrun stapler staple "$dmg"
    spctl --assess --type open --context context:primary-signature --verbose "$dmg"
fi

echo "app: $app ($(du -sh "$app" | cut -f1), macOS $min_macos+)"
echo "dmg: $dmg"
shasum -a 256 "$dmg"
