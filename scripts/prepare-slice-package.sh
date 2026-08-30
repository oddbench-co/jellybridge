#!/usr/bin/env bash
set -euo pipefail

script_dir="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")" && pwd)"
project_dir="$(cd -- "$script_dir/.." && pwd)"
stage_dir="$project_dir/target/package-native"
native_bundle="${JELLYBRIDGE_RELEASE_NATIVE_BUNDLE:-}"

if [[ -z "$native_bundle" ]]; then
    echo "JELLYBRIDGE_RELEASE_NATIVE_BUNDLE must name the audited native bundle" >&2
    echo "Homebrew/system libmpv is development-only and will not be packaged" >&2
    exit 2
fi
if [[ ! -d "$native_bundle" ]]; then
    echo "native bundle does not exist: $native_bundle" >&2
    exit 2
fi

required_files=(
    "SBOM.spdx.json"
    "build-manifest.json"
    "corresponding-source.txt"
    "lib/pkgconfig/mpv.pc"
)
for required_file in "${required_files[@]}"; do
    if [[ ! -s "$native_bundle/$required_file" ]]; then
        echo "native bundle is missing required evidence: $required_file" >&2
        exit 2
    fi
done
if ! grep -Eq '"releaseEligible"[[:space:]]*:[[:space:]]*true' "$native_bundle/build-manifest.json"; then
    echo "native bundle is not attested as release eligible" >&2
    exit 2
fi
if ! grep -Eq '"mpvVersion"[[:space:]]*:[[:space:]]*"0\.41\.0"' "$native_bundle/build-manifest.json"; then
    echo "native bundle does not attest the required mpv 0.41.0 build" >&2
    exit 2
fi

mkdir -p "$stage_dir"
find "$stage_dir" -mindepth 1 -maxdepth 1 -exec rm -rf -- {} +
cp -R "$native_bundle"/. "$stage_dir"/

if find "$stage_dir" -type l -exec sh -c 'for path do target=$(readlink "$path"); case "$target" in /*) exit 1;; esac; done' sh {} +; then
    :
else
    echo "native bundle contains an absolute symlink" >&2
    exit 2
fi

if find "$stage_dir/lib" -type f \( \
    -name 'libGL.so*' -o -name 'libEGL.so*' -o -name 'libvulkan.so*' -o \
    -name 'libva.so*' -o -name 'libvdpau.so*' -o -name '*x264*' -o \
    -name '*x265*' -o -name '*fdk*aac*' \
\) | grep -q .; then
    echo "native bundle contains a prohibited graphics/GPL/nonfree dependency" >&2
    exit 2
fi

echo "staged audited native bundle from $native_bundle"
