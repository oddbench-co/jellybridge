#!/usr/bin/env bash
# Packs every crate the app depends on, as pinned in Cargo.lock, into
# target/linux/dist/jellybridge-<version>-vendor.tar.xz. Distribution builds
# that run without network (Fedora COPR, see packaging/copr/) unpack it next
# to the source and build offline; it is also the corresponding source of
# the Rust dependencies for a release. Needs cargo (any recent version).
set -euo pipefail
PROJECT_DIR="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")/.." && pwd)"
VERSION="$(sed -n 's/^version = "\(.*\)"/\1/p' "$PROJECT_DIR/rust/app/Cargo.toml" | head -1)"
DIST="$PROJECT_DIR/target/linux/dist"
OUT="$DIST/jellybridge-$VERSION-vendor.tar.xz"
TMP="$(mktemp -d)"
trap 'rm -rf "$TMP"' EXIT
mkdir -p "$DIST"
(cd "$PROJECT_DIR" && cargo vendor --locked --versioned-dirs "$TMP/vendor" > "$TMP/config.toml")
# One file, the same bytes for the same lockfile.
tar -C "$TMP" --sort=name --owner=0 --group=0 --numeric-owner --mtime='2000-01-01 00:00Z' \
    -cf - vendor | xz -T0 -6 > "$OUT"
ls -la "$OUT"
