#!/usr/bin/env python3
"""Draws static/img/mountains.svg, the About page landscape: three ridges and a pine forest in the
site's violet-to-indigo palette. Seeded, so a rerun gives the same file.

    python3 tools/landscape.py
"""
import random
from pathlib import Path

W, H = 1600, 420
rng = random.Random(7)


def ridge(base, amplitude, step, jitter):
    """A jagged ridge line from left to right, closed along the bottom edge."""
    points, x = [], -20
    while x <= W + 20:
        points.append((x, base - rng.uniform(0, amplitude)))
        x += rng.uniform(step * 0.6, step * 1.4)
        points.append((x, base - rng.uniform(0, amplitude * jitter)))
        x += rng.uniform(step * 0.6, step * 1.4)
    d = "M" + " L".join(f"{x:.0f},{y:.0f}" for x, y in points)
    return f"{d} L{W + 20},{H} L-20,{H} Z"


def pine(x, ground, height):
    """A pine: a stack of three widening triangles on a short trunk."""
    w = height * 0.36
    parts = [f"M{x - w * 0.08:.1f},{ground:.1f} h{w * 0.16:.1f} v{-height * 0.18:.1f} h{-w * 0.16:.1f} Z"]
    for i, (top, bottom, half) in enumerate(((0.0, 0.45, 0.42), (0.22, 0.7, 0.62), (0.45, 0.92, 0.85))):
        y0, y1 = ground - height + height * top, ground - height + height * bottom
        parts.append(f"M{x:.1f},{y0:.1f} L{x + w * half:.1f},{y1:.1f} L{x - w * half:.1f},{y1:.1f} Z")
    return "".join(parts)


def forest(ground, count, low, high, xs=None):
    xs = xs or sorted(rng.uniform(0, W) for _ in range(count))
    return "".join(pine(x, ground + rng.uniform(-6, 6), rng.uniform(low, high)) for x in xs)


svg = f"""<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 {W} {H}" width="{W}" height="{H}" preserveAspectRatio="xMidYMax slice">
<defs>
<linearGradient id="sky" x1="0" y1="0" x2="0" y2="1"><stop offset="0" stop-color="#f5f4ff" stop-opacity="0"/><stop offset="1" stop-color="#e4e0ff"/></linearGradient>
<linearGradient id="far" x1="0" y1="0" x2="0" y2="1"><stop offset="0" stop-color="#c3bcf6"/><stop offset="1" stop-color="#dcd7fb"/></linearGradient>
<linearGradient id="mid" x1="0" y1="0" x2="0" y2="1"><stop offset="0" stop-color="#7c70e0"/><stop offset="1" stop-color="#a59cee"/></linearGradient>
<linearGradient id="near" x1="0" y1="0" x2="0" y2="1"><stop offset="0" stop-color="#3f33a6"/><stop offset="1" stop-color="#5a4fc4"/></linearGradient>
</defs>
<rect width="{W}" height="{H}" fill="url(#sky)"/>
<path fill="url(#far)" d="{ridge(230, 150, 70, 0.5)}"/>
<path fill="url(#mid)" d="{ridge(300, 120, 60, 0.45)}"/>
<path fill="#6a5fd6" opacity="0.9" d="{forest(318, 60, 28, 52)}"/>
<path fill="url(#near)" d="{ridge(360, 50, 90, 0.6)}"/>
<path fill="#2c2380" d="{forest(372, 70, 40, 80)}"/>
<path fill="#1c165a" d="{forest(404, 26, 110, 190, [rng.uniform(0, 260) for _ in range(9)] + [rng.uniform(1340, W) for _ in range(9)] + [rng.uniform(260, 1340) for _ in range(8)])}"/>
<rect y="{H - 24}" width="{W}" height="24" fill="#1c165a"/>
</svg>
"""
out = Path(__file__).resolve().parent.parent / "static/img/mountains.svg"
out.write_text(svg)
print(out, len(svg), "bytes")
