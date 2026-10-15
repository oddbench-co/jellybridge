#!/usr/bin/env bash
# Runs inside a test VM's live session. Fetched and started by typing:
#   curl -fsS 10.0.2.2:8765/t.sh | bash -s NAME [package|appimage|tar]
# Installs JellyBridge the way a user would, runs the scripted scenario, and
# uploads logs and screenshots to the host. Test-safe: muted, and no playback
# reports reach the Jellyfin server.
set -u
NAME="${1:?vm name}"; MODE="${2:-package}"
HOST=http://10.0.2.2:8765
W=/tmp/jb; rm -rf "$W"; mkdir -p "$W/shots" "$W/out"
exec > >(tee "$W/out/guest.log") 2>&1

get() { curl -fsS -o "$2" "$HOST/$1" 2>/dev/null || wget -q -O "$2" "$HOST/$1"; }
put() { # file, remote name
    curl -fsS -T "$1" "$HOST/results/$NAME-$MODE/$2" 2>/dev/null \
        || wget -q --method=PUT --body-file="$1" -O /dev/null "$HOST/results/$NAME-$MODE/$2" \
        || python3 -c "import sys,urllib.request as u;u.urlopen(u.Request(sys.argv[2],data=open(sys.argv[1],'rb').read(),method='PUT'))" "$1" "$HOST/results/$NAME-$MODE/$2"
}
status() { echo "$1" > "$W/out/status"; put "$W/out/status" status; echo "== $1"; }
SUDO="sudo -n"; [[ $(id -u) == 0 ]] && SUDO=""

status "starting"
# A blanked or locked screen would swallow the host's input and screenshots.
if command -v gsettings >/dev/null; then
    gsettings set org.gnome.desktop.session idle-delay 0 2>/dev/null
    gsettings set org.gnome.desktop.screensaver lock-enabled false 2>/dev/null
    gsettings set org.cinnamon.desktop.session idle-delay 0 2>/dev/null
fi
command -v kwriteconfig6 >/dev/null && kwriteconfig6 --file kscreenlockerrc --group Daemon --key Autolock false
{
    cat /etc/os-release
    echo "session=${XDG_SESSION_TYPE:-?} desktop=${XDG_CURRENT_DESKTOP:-?} wayland=${WAYLAND_DISPLAY:-} display=${DISPLAY:-}"
    echo "glibc=$(ldd --version | head -1)"
    uname -a
} > "$W/out/env.txt"
put "$W/out/env.txt" env.txt
$SUDO true || { status "error: sudo needs a password; run 'sudo -v' in this terminal and retry"; exit 1; }

get manifest.txt "$W/manifest.txt"
VERSION="$(sed -n 's/^version=//p' "$W/manifest.txt")"
BIN=jellybridge
case "$MODE" in
package)
    if command -v apt-get >/dev/null; then
        get "jellybridge_${VERSION}_amd64.deb" "$W/jb.deb"
        $SUDO apt-get update -qq; $SUDO env DEBIAN_FRONTEND=noninteractive apt-get install -y "$W/jb.deb"
    elif command -v dnf >/dev/null; then
        get "jellybridge-${VERSION}-1.x86_64.rpm" "$W/jb.rpm"; $SUDO dnf install -y "$W/jb.rpm"
    elif command -v zypper >/dev/null; then
        get "jellybridge-${VERSION}-1.x86_64.rpm" "$W/jb.rpm"
        $SUDO zypper --non-interactive --no-gpg-checks install --allow-unsigned-rpm "$W/jb.rpm"
    elif command -v pacman >/dev/null; then
        get "jellybridge-${VERSION}-1-x86_64.pkg.tar.zst" "$W/jb.pkg.tar.zst"
        # Arch does not support partial upgrades (-Sy then install), so try
        # the system as it is first, then a full upgrade.
        $SUDO pacman -U --noconfirm "$W/jb.pkg.tar.zst" \
            || { $SUDO pacman -Syu --noconfirm && $SUDO pacman -U --noconfirm "$W/jb.pkg.tar.zst"; }
    else
        status "error: no known package manager"; exit 1
    fi
    BIN=/usr/bin/jellybridge ;;
appimage)
    get "JellyBridge-${VERSION}-x86_64.AppImage" "$W/JellyBridge.AppImage"; chmod +x "$W/JellyBridge.AppImage"
    BIN="$W/JellyBridge.AppImage" ;;
tar)
    get "jellybridge-${VERSION}-x86_64.tar.gz" "$W/jb.tar.gz"; tar -C "$W" -xzf "$W/jb.tar.gz"
    (cd "$W/jellybridge-${VERSION}-x86_64" && ./install.sh)
    BIN="$HOME/.local/bin/jellybridge" ;;
esac
[[ -x "$BIN" || "$MODE" == package && -x /usr/bin/jellybridge ]] || { status "error: install failed"; put "$W/out/guest.log" guest.log; exit 1; }
status "installed"

{
    echo "# desktop entry"
    for d in /usr/share/applications "$HOME/.local/share/applications"; do ls -l "$d/co.oddbench.JellyBridge.desktop" 2>/dev/null; done
    command -v desktop-file-validate >/dev/null && desktop-file-validate /usr/share/applications/co.oddbench.JellyBridge.desktop 2>&1
    echo "# icons"; ls /usr/share/icons/hicolor/*/apps/co.oddbench.JellyBridge.png "$HOME"/.local/share/icons/hicolor/*/apps/co.oddbench.JellyBridge.png 2>/dev/null
    if [[ "$MODE" != appimage ]]; then
        real="$(readlink -f "$BIN")"
        echo "# ldd $real"; ldd "$real" | grep -E 'not found' || echo "all resolved"
        lib="$(dirname "$real")/../lib/jellybridge/lib/libmpv.so.2"
        [[ -e "$lib" ]] && { echo "# ldd bundled libmpv"; ldd "$lib" | grep -E 'not found' || echo "all resolved"; }
    fi
} > "$W/out/install-check.txt" 2>&1
put "$W/out/install-check.txt" install-check.txt

mkdir -p "$HOME/.config/jellybridge"
get session.json "$HOME/.config/jellybridge/session.json"
get scenario.txt "$W/scenario.txt"
sed -i "s#@SHOTS@#$W/shots#g" "$W/scenario.txt"

status "running"
JELLYBRIDGE_MUTE=1 JELLYBRIDGE_NO_PLAYBACK_REPORTS=1 JELLYBRIDGE_DEVICE_ID="jellybridge-vm-$NAME" \
JELLYBRIDGE_INPUT_SCRIPT="$W/scenario.txt" RUST_LOG=jellybridge=debug \
    "$BIN" > "$W/out/app.log" 2>&1 &
APP=$!
for _ in $(seq 1 75); do sleep 1; kill -0 $APP 2>/dev/null || break; done
if kill -0 $APP 2>/dev/null; then
    kill $APP; wait $APP 2>/dev/null; echo "app was running at the end: yes" >> "$W/out/app.log"
else
    wait $APP; echo "app exited early with status $?" >> "$W/out/app.log"
fi
put "$W/out/app.log" app.log
for f in "$W"/shots/*.png; do [[ -e "$f" ]] && put "$f" "$(basename "$f")"; done

# Clean removal leaves nothing behind but the user's config and cache.
case "$MODE" in
package)
    if command -v apt-get >/dev/null; then $SUDO apt-get remove -y jellybridge >/dev/null
    elif command -v dnf >/dev/null; then $SUDO dnf remove -y jellybridge >/dev/null
    elif command -v zypper >/dev/null; then $SUDO zypper --non-interactive remove jellybridge >/dev/null
    elif command -v pacman >/dev/null; then $SUDO pacman -R --noconfirm jellybridge >/dev/null; fi
    left="$(ls -d /usr/bin/jellybridge /usr/lib/jellybridge /usr/share/applications/co.oddbench.JellyBridge.desktop 2>/dev/null)" ;;
tar)
    (cd "$W/jellybridge-${VERSION}-x86_64" && ./uninstall.sh)
    left="$(ls -d "$HOME/.local/bin/jellybridge" "$HOME/.local/lib/jellybridge" 2>/dev/null)" ;;
*) left="" ;;
esac
echo "left after removal: ${left:-nothing}" >> "$W/out/install-check.txt"
put "$W/out/install-check.txt" install-check.txt
put "$W/out/guest.log" guest.log
status "done"
