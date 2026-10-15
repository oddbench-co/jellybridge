#!/usr/bin/env bash
# Wraps the portable bundle from scripts/build-linux.sh into installable
# packages, each built in a throwaway container (podman or docker):
#
#   scripts/package-linux.sh [deb|rpm|appimage|tar|arch ...]   (default: all)
#
# Output in target/linux/dist/:
#   jellybridge_<v>_amd64.deb          Ubuntu, Debian, Mint, Pop!_OS, elementary
#   jellybridge-<v>-1.x86_64.rpm       Fedora, openSUSE
#   JellyBridge-<v>-x86_64.AppImage    any distro, no install
#   jellybridge-<v>-x86_64.tar.gz      ~/.local install via install.sh
#   jellybridge-<v>-1-x86_64.pkg.tar.zst  Arch, Manjaro, EndeavourOS (system mpv)
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
        out=/src/target/linux/dist/JellyBridge-$VERSION-x86_64.AppImage
        ARCH=x86_64 $tool --appimage-extract-and-run --no-appstream $app $out >/dev/null 2>&1 \
            || ARCH=x86_64 $tool --appimage-extract-and-run --no-appstream $app $out
        [[ -z "$OWNER" ]] || chown $OWNER $out'
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
    mkdir -p "$CACHE/arch-cargo"
    "$ENGINE" run --rm -v "$PROJECT_DIR:/src:ro" -v "$CACHE/arch-cargo:/home/builder/.cargo/registry" \
        -v "$DIST:/dist" -e OWNER="$OWNER" docker.io/library/archlinux:latest bash -euc '
        pacman -Syu --noconfirm --needed -q base-devel rust imagemagick >/dev/null 2>&1
        useradd -m builder 2>/dev/null || true; chown -R builder /home/builder
        cp -r /src/packaging/arch /home/builder/pkg && chown -R builder /home/builder/pkg
        cd /home/builder/pkg
        su builder -c "JELLYBRIDGE_SRC=/src PKGDEST=/tmp/out makepkg --nodeps --noconfirm -f >/tmp/makepkg.log 2>&1" \
            || { tail -30 /tmp/makepkg.log; exit 1; }
        cp /tmp/out/*.pkg.tar.zst /dist/; [[ -z "$OWNER" ]] || chown $OWNER /dist/*.pkg.tar.zst'
}

for f in "${FORMATS[@]}"; do
    echo "== $f"
    case "$f" in
        deb) deb ;; rpm) rpm ;; appimage) appimage ;; tar) tar_ ;; arch) arch ;;
        *) echo "unknown format: $f" >&2; exit 2 ;;
    esac
done
ls -la "$DIST"
