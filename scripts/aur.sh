#!/usr/bin/env bash
# Publishes the AUR packages "jellybridge" (built from the release source) and
# "jellybridge-bin" (the release's portable bundle) for a published release:
#
#   scripts/aur.sh prepare <version> <dir>   writes <dir>/jellybridge and
#                                            <dir>/jellybridge-bin: PKGBUILD with
#                                            the release checksums and .SRCINFO
#   scripts/aur.sh push <version> <dir>      commits them to the AUR repositories
#
# prepare needs makepkg (an Arch system or container; the release workflow
# uses docker.io/library/archlinux). push needs an AUR account whose SSH key
# is loaded, or AUR_SSH_KEY_FILE pointing at it; the AUR host key is pinned
# below (the fingerprints are published on https://aur.archlinux.org). The
# release must be public already: both packages download from it.
set -euo pipefail
PROJECT_DIR="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")/.." && pwd)"
CMD="${1:-}"; VERSION="${2:-}"; DIR="${3:-}"
[[ -n "$CMD" && -n "$VERSION" && -n "$DIR" ]] || { sed -n '2,15p' "$0"; exit 2; }
SOURCE_URL="https://github.com/oddbench-co/jellybridge/archive/refs/tags/v$VERSION.tar.gz"
BUNDLE_URL="https://github.com/oddbench-co/jellybridge/releases/download/v$VERSION/jellybridge-$VERSION-x86_64.tar.gz"
AUR_HOST_KEYS='# aur.archlinux.org:22 SSH-2.0-OpenSSH_10.6
aur.archlinux.org ssh-ed25519 AAAAC3NzaC1lZDI1NTE5AAAAIEuBKrPzbawxA/k2g6NcyV5jmqwJ2s+zpgZGZ7tpLIcN
# aur.archlinux.org:22 SSH-2.0-OpenSSH_10.6
aur.archlinux.org ssh-rsa AAAAB3NzaC1yc2EAAAADAQABAAABgQDKF9vAFWdgm9Bi8uc+tYRBmXASBb5cB5iZsB7LOWWFeBrLp3r14w0/9S2vozjgqY5sJLDPONWoTTaVTbhe3vwO8CBKZTEt1AcWxuXNlRnk9FliR1/eNB9uz/7y1R0+c1Md+P98AJJSJWKN12nqIDIhjl2S1vOUvm7FNY43fU2knIhEbHybhwWeg+0wxpKwcAd/JeL5i92Uv03MYftOToUijd1pqyVFdJvQFhqD4v3M157jxS5FTOBrccAEjT+zYmFyD8WvKUa9vUclRddNllmBJdy4NyLB8SvVZULUPrP3QOlmzemeKracTlVOUG1wsDbxknF1BwSCU7CmU6UFP90kpWIyz66bP0bl67QAvlIc52Yix7pKJPbw85+zykvnfl2mdROsaT8p8R9nwCdFsBc9IiD0NhPEHcyHRwB8fokXTajk2QnGhL+zP5KnkmXnyQYOCUYo3EKMXIlVOVbPDgRYYT/XqvBuzq5S9rrU70KoI/S5lDnFfx/+lPLdtcnnEPk=
# aur.archlinux.org:22 SSH-2.0-OpenSSH_10.6
aur.archlinux.org ecdsa-sha2-nistp256 AAAAE2VjZHNhLXNoYTItbmlzdHAyNTYAAAAIbmlzdHAyNTYAAABBBLMiLrP8pVi5BFX2i3vepSUnpedeiewE5XptnUnau+ZoeUOPkpoCgZZuYfpaIQfhhJJI5qgnjJmr4hyJbe/zxow=
'

fetch_sum() { # url
    curl -fsSL --retry 3 "$1" | sha256sum | cut -d' ' -f1
}

write_pkgbuild() { # template, destination dir, sha256
    local template="$1" dest="$2" sum="$3"
    mkdir -p "$dest"
    sed -e "s/^pkgver=.*/pkgver=$VERSION/" -e "s/^pkgrel=.*/pkgrel=1/" -e "s/^sha256sums=.*/sha256sums=('$sum')/" \
        "$template" > "$dest/PKGBUILD"
    (cd "$dest" && makepkg --printsrcinfo > .SRCINFO)
}

prepare() {
    command -v makepkg >/dev/null || { echo "prepare needs makepkg (run it on Arch, or in an archlinux container)" >&2; exit 2; }
    echo "checksums of the v$VERSION release"
    local source_sum bundle_sum
    source_sum="$(fetch_sum "$SOURCE_URL")"
    bundle_sum="$(fetch_sum "$BUNDLE_URL")"
    write_pkgbuild "$PROJECT_DIR/packaging/arch/PKGBUILD" "$DIR/jellybridge" "$source_sum"
    write_pkgbuild "$PROJECT_DIR/packaging/arch/PKGBUILD-bin" "$DIR/jellybridge-bin" "$bundle_sum"
    echo "written: $DIR/jellybridge $DIR/jellybridge-bin"
}

push() {
    local known="$DIR/.aur_known_hosts"
    printf '%s' "$AUR_HOST_KEYS" > "$known"
    local ssh="ssh -o UserKnownHostsFile=$known -o StrictHostKeyChecking=yes"
    [[ -z "${AUR_SSH_KEY_FILE:-}" ]] || ssh+=" -i $AUR_SSH_KEY_FILE -o IdentitiesOnly=yes"
    export GIT_SSH_COMMAND="$ssh"
    local pkg
    for pkg in jellybridge jellybridge-bin; do
        [[ -s "$DIR/$pkg/.SRCINFO" ]] || { echo "run prepare first: $DIR/$pkg/.SRCINFO is missing" >&2; exit 2; }
        rm -rf "$DIR/$pkg.git"
        # An unknown package is an empty repository; the first push creates it.
        git clone -q "ssh://aur@aur.archlinux.org/$pkg.git" "$DIR/$pkg.git"
        cp "$DIR/$pkg/PKGBUILD" "$DIR/$pkg/.SRCINFO" "$DIR/$pkg.git/"
        (
            cd "$DIR/$pkg.git"
            git add PKGBUILD .SRCINFO
            if git diff --cached --quiet; then echo "$pkg: already at $VERSION"; exit 0; fi
            git -c user.name="${AUR_GIT_NAME:-Oddbench}" -c user.email="${AUR_GIT_EMAIL:-support@oddbench.co}" \
                commit -q -m "Update to $VERSION"
            git push -q origin HEAD:master
            echo "$pkg: pushed $VERSION"
        )
    done
}

case "$CMD" in
    prepare) prepare ;;
    push) push ;;
    *) echo "unknown command: $CMD" >&2; exit 2 ;;
esac
