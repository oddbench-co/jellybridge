#!/usr/bin/env bash
# Wraps the portable bundle from scripts/build-linux.sh into installable
# packages, each built in a throwaway container (podman or docker):
#
#   scripts/package-linux.sh [deb|rpm|appimage|tar|arch|snap ...]   (default: all but snap)
#
# Output in target/linux/dist/:
#   jellybridge_<v>_amd64.deb          Ubuntu, Debian, Mint, Pop!_OS, elementary
#   jellybridge-<v>-1.x86_64.rpm       Fedora, openSUSE
#   JellyBridge-<v>-x86_64.AppImage    any distro, no install (+ .zsync for AppImageUpdate)
#   jellybridge-<v>-x86_64.tar.gz      ~/.local install via install.sh
#   jellybridge-<v>-1-x86_64.pkg.tar.zst  Arch, Manjaro, EndeavourOS (system mpv)
#   jellybridge_<v>_amd64.snap         the Snap Store package (snap/snapcraft.yaml)
set -euo pipefail
PROJECT_DIR="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")/.." && pwd)"
ENGINE="$(command -v podman || command -v docker)" || { echo "needs podman or docker" >&2; exit 2; }
VERSION="$(sed -n 's/^version = "\(.*\)"/\1/p' "$PROJECT_DIR/rust/app/Cargo.toml" | head -1)"
BUNDLE="$PROJECT_DIR/target/linux/bundle"
DIST="$PROJECT_DIR/target/linux/dist"
CACHE="${XDG_CACHE_HOME:-$HOME/.cache}/jellybridge-linux-build"
FORMATS=("$@"); [[ ${#FORMATS[@]} -gt 0 ]] || FORMATS=(deb rpm appimage tar arch)
for f in "${FORMATS[@]}"; do
    [[ "$f" == arch || -x "$BUNDLE/usr/bin/jellybridge" ]] || { echo "run scripts/build-linux.sh first" >&2; exit 2; }
done
mkdir -p "$DIST" "$CACHE/tools"
# Rootless podman maps container root to the invoking user already; only
# docker needs files handed back.
OWNER=""; [[ "$ENGINE" == *docker ]] && OWNER="$(id -u):$(id -g)"
run() { # image, script; /src is the repo
    "$ENGINE" run --rm -v "$PROJECT_DIR:/src" -v "$CACHE:/cache" -e VERSION="$VERSION" -e OWNER="$OWNER" "$1" bash -euc "$2"
}

deb() {
    run docker.io/library/ubuntu:22.04 '
        root=/tmp/pkg; rm -rf $root; cp -a /src/target/linux/bundle $root; mkdir -p $root/DEBIAN
        size=$(du -sk $root/usr | cut -f1)
        cat > $root/DEBIAN/control <<CONTROL
Package: jellybridge
Version: $VERSION
Architecture: amd64
Maintainer: Oddbench <support@oddbench.co>
Installed-Size: $size
Section: video
Priority: optional
Homepage: https://github.com/oddbench-co/jellybridge
Depends: libc6 (>= 2.35), libgcc-s1, libgl1, libegl1, libfontconfig1, libfreetype6, libharfbuzz0b, libfribidi0, libdrm2, liblcms2-2, libasound2 | libasound2t64, libpulse0, zlib1g, libxkbcommon0, libwayland-client0, libwayland-egl1, libx11-6, libxcursor1, libxi6, libxrandr2, hicolor-icon-theme
Recommends: mesa-va-drivers | va-driver-all | va-driver
Description: Native desktop client for Jellyfin
 JellyBridge browses and plays your Jellyfin library in a fast native
 window. All common video codecs are included.
CONTROL
        dpkg-deb --root-owner-group -Zxz --build $root /src/target/linux/dist/jellybridge_${VERSION}_amd64.deb >/dev/null
        [[ -z "$OWNER" ]] || chown $OWNER /src/target/linux/dist/*.deb'
}

rpm() {
    run registry.fedoraproject.org/fedora:latest '
        dnf -q -y install rpm-build >/dev/null
        rpmbuild -bb --quiet --define "_topdir /tmp/rpm" --define "jb_version $VERSION" \
            --define "jb_bundle /src/target/linux/bundle" /src/packaging/linux/jellybridge.spec
        cp /tmp/rpm/RPMS/x86_64/*.rpm /src/target/linux/dist/
        [[ -z "$OWNER" ]] || chown $OWNER /src/target/linux/dist/*.rpm'
}

appimage() {
    run docker.io/library/ubuntu:22.04 '
        apt-get update -qq >/dev/null && apt-get install -y -qq --no-install-recommends curl ca-certificates file >/dev/null
        tool=/cache/tools/appimagetool-x86_64.AppImage
        [[ -s $tool ]] || curl -fsSL -o $tool https://github.com/AppImage/appimagetool/releases/download/continuous/appimagetool-x86_64.AppImage
        chmod +x $tool
        app=/tmp/JellyBridge.AppDir; rm -rf $app; cp -a /src/target/linux/bundle $app
        ln -s usr/bin/jellybridge $app/AppRun
        cp $app/usr/share/applications/co.oddbench.JellyBridge.desktop $app/
        cp $app/usr/share/icons/hicolor/256x256/apps/co.oddbench.JellyBridge.png $app/co.oddbench.JellyBridge.png
        ln -s co.oddbench.JellyBridge.png $app/.DirIcon
        # Update information lets AppImageUpdate (and AppImageLauncher) fetch
        # the next release as a delta; the .zsync file is published next to it.
        update="gh-releases-zsync|oddbench-co|jellybridge|latest|JellyBridge-*-x86_64.AppImage.zsync"
        cd /src/target/linux/dist
        out=JellyBridge-$VERSION-x86_64.AppImage
        ARCH=x86_64 $tool --appimage-extract-and-run --no-appstream -u "$update" $app $out >/dev/null 2>&1 \
            || ARCH=x86_64 $tool --appimage-extract-and-run --no-appstream -u "$update" $app $out
        [[ -s $out.zsync ]] || { echo "appimagetool wrote no $out.zsync" >&2; exit 1; }
        [[ -z "$OWNER" ]] || chown $OWNER $out $out.zsync'
}

tar_() {
    local name="jellybridge-$VERSION-x86_64" tmp
    tmp="$(mktemp -d)"
    mkdir -p "$tmp/$name"
    cp -a "$BUNDLE/usr" "$tmp/$name/"
    cp "$PROJECT_DIR/packaging/linux/install.sh" "$PROJECT_DIR/packaging/linux/uninstall.sh" "$tmp/$name/"
    tar -C "$tmp" -czf "$DIST/$name.tar.gz" "$name"
    rm -rf "$tmp"
}

arch() {
    # The PKGBUILD builds the release tarball; here it gets a tarball of the
    # same layout from the checked-out commit (uncommitted changes are left
    # out, as in a release), so the package is made exactly as on the AUR.
    local src="$DIST/.arch-src"
    rm -rf "$src"; mkdir -p "$src" "$CACHE/arch-cargo"
    git -C "$PROJECT_DIR" archive --format=tar.gz --prefix="jellybridge-$VERSION/" -o "$src/jellybridge-$VERSION.tar.gz" HEAD
    cp "$PROJECT_DIR/packaging/arch/PKGBUILD" "$src/"
    "$ENGINE" run --rm -v "$src:/pkg:ro" -v "$CACHE/arch-cargo:/home/builder/.cargo/registry" \
        -v "$DIST:/dist" -e OWNER="$OWNER" docker.io/library/archlinux:latest bash -euc '
        pacman -Syu --noconfirm --needed -q base-devel rust imagemagick >/dev/null 2>&1
        useradd -m builder 2>/dev/null || true; chown -R builder /home/builder
        cp -r /pkg /home/builder/pkg && chown -R builder /home/builder/pkg
        cd /home/builder/pkg
        su builder -c "PKGDEST=/tmp/out makepkg --skipchecksums --nodeps --noconfirm -f >/tmp/makepkg.log 2>&1" \
            || { tail -30 /tmp/makepkg.log; exit 1; }
        cp /tmp/out/*.pkg.tar.zst /dist/; [[ -z "$OWNER" ]] || chown $OWNER /dist/*.pkg.tar.zst'
    rm -rf "$src"
}

snap() {
    # snapcraft wants snapd and LXD, which the host may not have. Canonical's
    # snapcraft container builds in "destructive mode" instead, from a scratch
    # copy of what the snap needs (snap/, the icon, the bundle), with the GNOME
    # SDK snap and snapcraft's extension files unpacked from the store first.
    local work="$DIST/.snap-src"
    rm -rf "$work"; mkdir -p "$work/rust/app/assets" "$work/target/linux" "$CACHE/snaps"
    cp -a "$PROJECT_DIR/snap" "$work/snap"
    cp "$PROJECT_DIR/rust/app/assets/icon.png" "$work/rust/app/assets/"
    cp -a "$BUNDLE" "$work/target/linux/bundle"
    "$ENGINE" run --rm --entrypoint /bin/bash -v "$work:/project" -v "$CACHE/snaps:/cache" -v "$DIST:/dist" \
        -e OWNER="$OWNER" ghcr.io/canonical/snapcraft:8_core24 -euc '
        export DEBIAN_FRONTEND=noninteractive
        apt-get update -qq >/dev/null && apt-get install -y -qq squashfs-tools curl ca-certificates python3 >/dev/null
        store() { # name track -> download url of its stable channel
            curl -fsSL -H "Snap-Device-Series: 16" "https://api.snapcraft.io/v2/snaps/info/$1?architecture=amd64&fields=download" \
                | python3 -c "import sys,json; d=json.load(sys.stdin); print([c for c in d[\"channel-map\"] if c[\"channel\"][\"track\"]==sys.argv[1] and c[\"channel\"][\"name\"].endswith(\"stable\")][0][\"download\"][\"url\"])" "$2"
        }
        fetch() { [[ -s /cache/$1.snap ]] || { curl -fsSL -o /cache/$1.snap.part "$(store "$1" "$2")" && mv /cache/$1.snap.part /cache/$1.snap; }; }
        fetch gnome-46-2404-sdk latest
        fetch snapcraft 8.x
        rm -rf /snap/gnome-46-2404-sdk && mkdir -p /snap/gnome-46-2404-sdk
        unsquashfs -n -d /snap/gnome-46-2404-sdk/current /cache/gnome-46-2404-sdk.snap >/dev/null
        rm -rf /tmp/sc && unsquashfs -n -d /tmp/sc /cache/snapcraft.snap share/snapcraft >/dev/null
        mkdir -p /usr/share/snapcraft && cp -a /tmp/sc/share/snapcraft/. /usr/share/snapcraft/
        cd /project && snapcraft pack --destructive-mode --verbosity=brief -o /dist/
        [[ -z "$OWNER" ]] || chown $OWNER /dist/*.snap'
    rm -rf "$work"
}

for f in "${FORMATS[@]}"; do
    echo "== $f"
    case "$f" in
        deb) deb ;; rpm) rpm ;; appimage) appimage ;; tar) tar_ ;; arch) arch ;; snap) snap ;;
        *) echo "unknown format: $f" >&2; exit 2 ;;
    esac
done
ls -la "$DIST"
