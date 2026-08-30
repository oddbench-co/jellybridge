#!/usr/bin/env bash
set -euo pipefail

APP_NAME="jellybridge"
LINK_PATH="${LINK_PATH:-/usr/local/bin/jellybridge}"

SCRIPT_DIR="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")" && pwd)"
PROJECT_DIR="$(cd -- "$SCRIPT_DIR/.." && pwd)"
APP_OUTPUT_DIR="$PROJECT_DIR/build/compose/binaries/main/app"

case "$(uname -s)" in
    Darwin)
        INSTALL_DIR="${INSTALL_DIR:-/Applications/$APP_NAME.app}"
        PACKAGE_DIR="$APP_OUTPUT_DIR/$APP_NAME.app"
        LAUNCHER_REL_PATH="Contents/MacOS/$APP_NAME"
        ;;
    *)
        INSTALL_DIR="${INSTALL_DIR:-/opt/jellybridge}"
        PACKAGE_DIR="$APP_OUTPUT_DIR/$APP_NAME"
        LAUNCHER_REL_PATH="bin/$APP_NAME"
        ;;
esac

if [[ -z "$INSTALL_DIR" || "$INSTALL_DIR" == "/" ]]; then
    echo "Refusing to install to unsafe INSTALL_DIR: $INSTALL_DIR" >&2
    exit 1
fi

cd "$PROJECT_DIR"

echo "==> Building $APP_NAME..."
./gradlew createDistributable

if [[ ! -x "$PACKAGE_DIR/$LAUNCHER_REL_PATH" ]]; then
    echo "Expected launcher not found: $PACKAGE_DIR/$LAUNCHER_REL_PATH" >&2
    exit 1
fi

echo "==> Deleting previous install at $INSTALL_DIR..."
sudo rm -rf "$INSTALL_DIR"

echo "==> Installing fresh build to $INSTALL_DIR..."
sudo cp -R "$PACKAGE_DIR" "$INSTALL_DIR"

echo "==> Updating launcher symlink at $LINK_PATH..."
sudo ln -sfn "$INSTALL_DIR/$LAUNCHER_REL_PATH" "$LINK_PATH"

echo "==> Done. Run: $APP_NAME"
