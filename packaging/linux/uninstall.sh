#!/usr/bin/env sh
# Removes what install.sh put into ~/.local. Your sign-in and settings in
# ~/.config/jellybridge and the artwork cache in ~/.cache/jellybridge stay;
# delete those folders too for a clean slate.
set -eu
PREFIX="${PREFIX:-$HOME/.local}"
rm -f "$PREFIX/bin/jellybridge" "$PREFIX/share/applications/dev.nk.jellybridge.desktop" \
      "$PREFIX/share/metainfo/dev.nk.jellybridge.metainfo.xml"
rm -rf "$PREFIX/lib/jellybridge" "$PREFIX/share/doc/jellybridge"
rm -f "$PREFIX"/share/icons/hicolor/*/apps/dev.nk.jellybridge.png
command -v update-desktop-database >/dev/null 2>&1 && update-desktop-database -q "$PREFIX/share/applications" || true
echo "JellyBridge is removed."
