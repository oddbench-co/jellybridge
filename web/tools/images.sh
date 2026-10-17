#!/bin/bash
# Regenerates static/img/screens/*.webp from the app's screenshots in ../docs/screenshots.
# Two widths per screenshot (640 and 1280 px) for srcset; quality 80 is visually lossless on UI captures.
set -euo pipefail
cd "$(dirname "$0")/.."
for name in home library detail player; do
  for width in 640 1280; do
    magick "../docs/screenshots/$name.jpg" -resize "${width}x" -strip -quality 80 -define webp:method=6 \
      "static/img/screens/$name-$width.webp"
  done
done
ls -l static/img/screens
