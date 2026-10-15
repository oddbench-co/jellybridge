#!/usr/bin/env sh
# Installs JellyBridge for the current user into ~/.local (no root needed).
# Run it from the unpacked folder: ./install.sh   Remove with ./uninstall.sh
set -eu
HERE="$(cd -- "$(dirname -- "$0")" && pwd)"
PREFIX="${PREFIX:-$HOME/.local}"
mkdir -p "$PREFIX"
cp -R "$HERE/usr/." "$PREFIX/"
command -v update-desktop-database >/dev/null 2>&1 && update-desktop-database -q "$PREFIX/share/applications" || true
command -v gtk-update-icon-cache >/dev/null 2>&1 && gtk-update-icon-cache -q -t "$PREFIX/share/icons/hicolor" || true
echo "JellyBridge is installed. Start it from your app menu, or run: $PREFIX/bin/jellybridge"
case ":$PATH:" in *":$PREFIX/bin:"*) ;; *) echo "Note: $PREFIX/bin is not on your PATH, so the menu entry is the easiest way to start it." ;; esac
