#!/usr/bin/env python3
"""Draws the JellyBridge icon and logo, and renders every file made from them.

The mark is a round badge: a suspension bridge crosses the horizon and the
play button rises above it like the sun. Everything is drawn here, so the
artwork is reproducible and editable as code.

    scripts/make-logo.py

Writes, from the repository root:
    rust/app/assets/icon.svg    the badge (vector source)
    rust/app/assets/icon.png    512 px, embedded in the app; the Linux and
                                Windows builds resize it for their icon sets
    rust/app/assets/icon.icns   macOS app icon
    rust/app/assets/brand.png   badge and wordmark in one line, for the app's
                                dark header and sign-in screen (brand.svg beside it)
    logo.svg, logo.png          badge with the "JellyBridge" wordmark (README)

Needs rsvg-convert, Pillow and fontTools. The wordmark is Noto Sans (the
app's own UI font, OFL), converted to outlines so no font must be installed.
"""
import io
import subprocess
from pathlib import Path

from fontTools.pens.svgPathPen import SVGPathPen
from fontTools.pens.transformPen import TransformPen
from fontTools.ttLib import TTFont
from PIL import Image

ROOT = Path(__file__).resolve().parent.parent
ASSETS = ROOT / "rust/app/assets"
VIOLET, BLUE = "#9440F5", "#2C7DF7"
INK, MUTED = "#15132B", "#5C5F78"


def badge(prefix=""):
    """The icon's drawing in a 512x512 box; prefix keeps ids unique when embedded."""
    g, clip = f"{prefix}sky", f"{prefix}round"
    white = "#FFFFFF"

    def line(x1, y1, x2, y2, width):
        return (f'<line x1="{x1}" y1="{y1}" x2="{x2}" y2="{y2}" stroke="{white}" '
                f'stroke-width="{width}" stroke-linecap="round"/>')

    tower_left, tower_right, tower_top, deck = 150, 362, 236, 346
    sag = 318
    control = 2 * sag - tower_top  # quadratic control point that puts the dip at `sag`

    def cable_y(x):
        t = (x - tower_left) / (tower_right - tower_left)
        return (1 - t) ** 2 * tower_top + 2 * t * (1 - t) * control + t ** 2 * tower_top

    hangers = "".join(line(x, round(cable_y(x) + 8), x, deck, 7) for x in (192, 226, 286, 320))
    side_hangers = line(104, 300, 104, deck, 7) + line(408, 300, 408, deck, 7)
    return f"""<defs>
  <linearGradient id="{g}" x1="0" y1="0" x2="1" y2="1">
    <stop offset="0" stop-color="{VIOLET}"/><stop offset="1" stop-color="{BLUE}"/>
  </linearGradient>
  <clipPath id="{clip}"><circle cx="256" cy="256" r="232"/></clipPath>
</defs>
<g clip-path="url(#{clip})">
  <rect x="0" y="0" width="512" height="512" fill="url(#{g})"/>
  <rect x="0" y="{deck}" width="512" height="{512 - deck}" fill="#0B0A2A" opacity="0.18"/>
  <path d="M231,118 Q218,110 218,125 L218,215 Q218,230 231,222 L302,178 Q314,170 302,162 Z" fill="{white}"/>
  {hangers}{side_hangers}
  <path d="M{tower_left},{tower_top + 14} Q256,{control} {tower_right},{tower_top + 14}" fill="none" stroke="{white}" stroke-width="14" stroke-linecap="round"/>
  <path d="M{tower_left},{tower_top + 14} Q100,316 18,334" fill="none" stroke="{white}" stroke-width="12" stroke-linecap="round"/>
  <path d="M{tower_right},{tower_top + 14} Q412,316 494,334" fill="none" stroke="{white}" stroke-width="12" stroke-linecap="round"/>
  {line(tower_left, tower_top, tower_left, 402, 18)}{line(tower_right, tower_top, tower_right, 402, 18)}
  <rect x="0" y="{deck - 10}" width="512" height="20" fill="{white}"/>
</g>"""


def svg(width, height, body):
    return (f'<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 {width} {height}" '
            f'width="{width}" height="{height}">{body}</svg>\n')


def text_path(font_file, text, size, x, y):
    """`text` as one SVG path in `font_file`, baseline at y, starting at x. Returns (path d, advance width)."""
    font = TTFont(font_file)
    glyphs, cmap = font.getGlyphSet(), font.getBestCmap()
    scale = size / font["head"].unitsPerEm
    pen = SVGPathPen(glyphs)
    cursor = 0
    for char in text:
        name = cmap[ord(char)]
        glyphs[name].draw(TransformPen(pen, (scale, 0, 0, -scale, x + cursor, y)))
        cursor += glyphs[name].width * scale
    return pen.getCommands(), cursor


def logo():
    """Square logo: the badge above the two-tone wordmark and the tagline."""
    width = height = 1200
    bold, regular = ASSETS / "fonts/NotoSans-Bold.ttf", ASSETS / "fonts/NotoSans-Regular.ttf"
    _, jelly_w = text_path(bold, "Jelly", 150, 0, 0)
    _, bridge_w = text_path(bold, "Bridge", 150, 0, 0)
    start = (width - jelly_w - bridge_w) / 2
    jelly, _ = text_path(bold, "Jelly", 150, start, 830)
    bridge, _ = text_path(bold, "Bridge", 150, start + jelly_w, 830)
    _, tag_w = text_path(regular, "Desktop client for Jellyfin", 54, 0, 0)
    tagline, _ = text_path(regular, "Desktop client for Jellyfin", 54, (width - tag_w) / 2, 926)
    word = (f'<defs><linearGradient id="word" gradientUnits="userSpaceOnUse" x1="{start + jelly_w}" y1="0" '
            f'x2="{start + jelly_w + bridge_w}" y2="0"><stop offset="0" stop-color="{VIOLET}"/>'
            f'<stop offset="1" stop-color="{BLUE}"/></linearGradient></defs>')
    return svg(width, height,
               f'<rect width="{width}" height="{height}" fill="#FFFFFF"/>{word}'
               f'<g transform="translate(380 206) scale(0.859375)">{badge("logo-")}</g>'
               f'<path d="{jelly}" fill="{INK}"/><path d="{bridge}" fill="url(#word)"/>'
               f'<path d="{tagline}" fill="{MUTED}"/>')


def brand():
    """One-line lockup for the app's dark header: badge, then the wordmark.

    "Jelly" is white and "Bridge" a lighter gradient than the logo's, so both
    stay readable on the near-black header."""
    bold = ASSETS / "fonts/NotoSans-Bold.ttf"
    size, gap = 300, 96
    _, jelly_w = text_path(bold, "Jelly", size, 0, 0)
    _, bridge_w = text_path(bold, "Bridge", size, 0, 0)
    # Cap height of Noto Sans is 0.714 em: centre the capitals on the badge.
    baseline = 256 + 0.714 * size / 2
    start = 512 + gap
    jelly, _ = text_path(bold, "Jelly", size, start, baseline)
    bridge, _ = text_path(bold, "Bridge", size, start + jelly_w, baseline)
    width = round(start + jelly_w + bridge_w + 8)
    word = (f'<defs><linearGradient id="brand-word" gradientUnits="userSpaceOnUse" x1="{start + jelly_w}" y1="0" '
            f'x2="{start + jelly_w + bridge_w}" y2="0"><stop offset="0" stop-color="#B58AFF"/>'
            f'<stop offset="1" stop-color="#6AAEFF"/></linearGradient></defs>')
    # Room below the baseline for the descenders of "y" and "g".
    height = 512
    return svg(width, height, f'{word}{badge("brand-")}<path d="{jelly}" fill="#FFFFFF"/>'
                              f'<path d="{bridge}" fill="url(#brand-word)"/>')


def render(svg_file, size):
    png = subprocess.run(["rsvg-convert", "-w", str(size), "-h", str(size), str(svg_file)],
                         check=True, capture_output=True).stdout
    return Image.open(io.BytesIO(png)).convert("RGBA")


def main():
    icon_svg = ASSETS / "icon.svg"
    icon_svg.write_text(svg(512, 512, badge()))
    render(icon_svg, 512).save(ASSETS / "icon.png", optimize=True)
    # Pillow writes every icns size from the largest image it is given.
    render(icon_svg, 1024).save(ASSETS / "icon.icns")
    brand_svg = ASSETS / "brand.svg"
    brand_svg.write_text(brand())
    # 180 px tall: the app draws it 1.7em high and downscales once at start-up.
    png = subprocess.run(["rsvg-convert", "-h", "180", str(brand_svg)], check=True, capture_output=True).stdout
    Image.open(io.BytesIO(png)).save(ASSETS / "brand.png", optimize=True)
    logo_svg = ROOT / "logo.svg"
    logo_svg.write_text(logo())
    render(logo_svg, 1200).convert("RGB").save(ROOT / "logo.png", optimize=True)
    for path in (icon_svg, ASSETS / "icon.png", ASSETS / "icon.icns", brand_svg, ASSETS / "brand.png",
                 logo_svg, ROOT / "logo.png"):
        print(f"{path.relative_to(ROOT)}  {path.stat().st_size:,} bytes")


if __name__ == "__main__":
    main()
