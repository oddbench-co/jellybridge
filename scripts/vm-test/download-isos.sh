#!/usr/bin/env bash
# Downloads the ISOs in matrix.tsv (or only the names given) into
# ~/.local/share/jellybridge-vmtest/iso/, resuming partial downloads.
set -uo pipefail
HERE="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")" && pwd)"
ISO="${JB_VMTEST_DIR:-$HOME/.local/share/jellybridge-vmtest}/iso"
mkdir -p "$ISO"
grep -v '^#' "$HERE/matrix.tsv" | while IFS=$'\t' read -r name modes url; do
    [[ $# -gt 0 && ! " $* " =~ " $name " ]] && continue
    [[ -s "$ISO/$name.iso" ]] && { echo "have $name"; continue; }
    echo "downloading $name"
    if curl -fL --retry 5 --retry-delay 5 -C - -o "$ISO/$name.iso.part" "$url" -sS; then
        mv "$ISO/$name.iso.part" "$ISO/$name.iso"
        echo "$(sha256sum "$ISO/$name.iso" | cut -d' ' -f1)  $name.iso  $url" >> "$ISO/SHA256SUMS.downloaded"
        echo "done $name"
    else
        echo "FAILED $name"
    fi
done
