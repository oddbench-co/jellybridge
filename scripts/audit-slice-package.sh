#!/usr/bin/env bash
set -euo pipefail

if [[ $# -lt 1 || $# -gt 2 ]]; then
    echo "usage: $0 APP_OR_EXTRACTED_PACKAGE [REPORT_DIRECTORY]" >&2
    exit 2
fi

script_dir="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")" && pwd)"
project_dir="$(cd -- "$script_dir/.." && pwd)"
artifact="$1"
report_dir="${2:-$project_dir/target/package-audit/$(basename -- "$artifact")}"

if [[ ! -e "$artifact" ]]; then
    echo "artifact does not exist: $artifact" >&2
    exit 2
fi
if [[ -e "$report_dir" ]] && find "$report_dir" -mindepth 1 -print -quit | grep -q .; then
    echo "report directory is not empty: $report_dir" >&2
    exit 2
fi
mkdir -p "$report_dir"

sha256_file() {
    if command -v sha256sum >/dev/null 2>&1; then
        sha256sum "$1"
    else
        shasum -a 256 "$1"
    fi
}

if [[ -f "$artifact" ]]; then
    sha256_file "$artifact" > "$report_dir/artifact.sha256"
else
    find "$artifact" -type f -exec sh -c '
        for path do
            if command -v sha256sum >/dev/null 2>&1; then
                sha256sum "$path"
            else
                shasum -a 256 "$path"
            fi
        done
    ' sh {} + | LC_ALL=C sort > "$report_dir/files.sha256"
fi

find "$artifact" -type f -print > "$report_dir/files.txt"
candidate_number=0
found_native_mpv=false

if [[ "$(uname -s)" == "Darwin" ]]; then
    while IFS= read -r binary; do
        if ! file "$binary" | grep -q 'Mach-O'; then
            continue
        fi
        candidate_number=$((candidate_number + 1))
        report_prefix="$report_dir/macho-$(printf '%04d' "$candidate_number")"
        printf '%s\n' "$binary" > "$report_prefix.path.txt"
        otool -L "$binary" > "$report_prefix.otool.txt"
        otool -l "$binary" > "$report_prefix.load-commands.txt"
        codesign -dvvv "$binary" > "$report_prefix.codesign.txt" 2>&1 || true

        if otool -L "$binary" | awk 'NR > 1 { print $1 }' | grep -E '^/' \
            | grep -Ev '^/System/Library/|^/usr/lib/' >/dev/null; then
            echo "non-system absolute dependency in $binary" >&2
            exit 1
        fi
        if [[ "$(basename -- "$binary")" == libmpv.*.dylib ]]; then
            found_native_mpv=true
        fi
    done < "$report_dir/files.txt"
    codesign --verify --deep --strict --verbose=2 "$artifact" \
        > "$report_dir/codesign-verification.txt" 2>&1
else
    while IFS= read -r binary; do
        if ! file "$binary" | grep -q 'ELF'; then
            continue
        fi
        candidate_number=$((candidate_number + 1))
        report_prefix="$report_dir/elf-$(printf '%04d' "$candidate_number")"
        printf '%s\n' "$binary" > "$report_prefix.path.txt"
        readelf -d "$binary" > "$report_prefix.dynamic.txt"
        readelf --version-info "$binary" > "$report_prefix.versions.txt"
        ldd "$binary" > "$report_prefix.ldd.txt"
        if command -v lddtree >/dev/null 2>&1; then
            lddtree "$binary" > "$report_prefix.lddtree.txt"
        fi
        if readelf -d "$binary" | grep -E '/home/|/Users/|/opt/homebrew|/usr/local|/tmp/' >/dev/null; then
            echo "build-machine RPATH/RUNPATH in $binary" >&2
            exit 1
        fi
        if grep -F 'not found' "$report_prefix.ldd.txt" >/dev/null; then
            echo "unresolved dynamic dependency in $binary" >&2
            exit 1
        fi
        if [[ "$(basename -- "$binary")" == libmpv.so.* ]]; then
            found_native_mpv=true
        fi
    done < "$report_dir/files.txt"
fi

if [[ "$found_native_mpv" != true ]]; then
    echo "package does not contain its own libmpv" >&2
    exit 1
fi
if ! find "$artifact" -type f -name 'SBOM.spdx.json' -print -quit | grep -q .; then
    echo "package has no SBOM.spdx.json" >&2
    exit 1
fi
if ! find "$artifact" -type f -name 'build-manifest.json' -print -quit | grep -q .; then
    echo "package has no native build manifest" >&2
    exit 1
fi
if ! find "$artifact" -type f -name 'corresponding-source.txt' -print -quit | grep -q .; then
    echo "package has no corresponding-source notice" >&2
    exit 1
fi
if find "$artifact" -type f \( \
    -name 'libGL.so*' -o -name 'libEGL.so*' -o -name 'libvulkan.so*' -o \
    -name 'libva.so*' -o -name 'libvdpau.so*' -o -name '*x264*' -o \
    -name '*x265*' -o -name '*fdk*aac*' \
\) | grep -q .; then
    echo "package contains a prohibited graphics/GPL/nonfree dependency" >&2
    exit 1
fi

printf 'PASS\n' > "$report_dir/verdict.txt"
echo "native package audit passed; persistent reports: $report_dir"
