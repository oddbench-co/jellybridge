#!/usr/bin/env bash
set -e

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
cd "$SCRIPT_DIR"

echo "==> Building JellyBridge..."
./gradlew packageAppImage

echo "==> Installing to /opt/jellybridge..."
sudo rm -rf /opt/jellybridge
sudo cp -r build/compose/binaries/main/app/jellybridge /opt/jellybridge
sudo ln -sf /opt/jellybridge/bin/jellybridge /usr/local/bin/jellybridge

echo "==> Done. Run: jellybridge"
