#!/usr/bin/env bash
set -euo pipefail

APP_NAME="jellybridge"
INSTALL_DIR="${INSTALL_DIR:-/opt/jellybridge}"
LINK_PATH="${LINK_PATH:-/usr/local/bin/jellybridge}"

SCRIPT_DIR="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")" && pwd)"
PROJECT_DIR="$(cd -- "$SCRIPT_DIR/.." && pwd)"
PACKAGE_DIR="$PROJECT_DIR/build/compose/binaries/main/app/$APP_NAME"
LAUNCHER_PATH="$PACKAGE_DIR/bin/$APP_NAME"

if [[ -z "$INSTALL_DIR" || "$INSTALL_DIR" == "/" ]]; then
    echo "Refusing to install to unsafe INSTALL_DIR: $INSTALL_DIR" >&2
    exit 1
fi

cd "$PROJECT_DIR"

echo "==> Building $APP_NAME..."
./gradlew packageAppImage

if [[ ! -x "$LAUNCHER_PATH" ]]; then
    echo "Expected launcher not found: $LAUNCHER_PATH" >&2
    exit 1
fi

echo "==> Deleting previous install at $INSTALL_DIR..."
sudo rm -rf "$INSTALL_DIR"

echo "==> Installing fresh build to $INSTALL_DIR..."
sudo cp -R "$PACKAGE_DIR" "$INSTALL_DIR"

echo "==> Updating launcher symlink at $LINK_PATH..."
sudo ln -sfn "$INSTALL_DIR/bin/$APP_NAME" "$LINK_PATH"

echo "==> Done. Run: $APP_NAME"
