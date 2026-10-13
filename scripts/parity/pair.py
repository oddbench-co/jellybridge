"""Crops one region (CSS px) from web and app, side by side at device size.

usage: pair.py web.png app.png out.png x0 y0 x1 y1 [scale]
"""
import sys
from PIL import Image

a, b = Image.open(sys.argv[1]).convert("RGB"), Image.open(sys.argv[2]).convert("RGB")
box = tuple(int(float(v) * 1.5) for v in sys.argv[4:8])
scale = float(sys.argv[8]) if len(sys.argv) > 8 else 1.0
ca, cb = a.crop(box), b.crop(box)
w, h = ca.size
out = Image.new("RGB", (w * 2 + 8, h), (255, 0, 0))
out.paste(ca, (0, 0))
out.paste(cb, (w + 8, 0))
if scale != 1.0:
    out = out.resize((int(out.width * scale), int(out.height * scale)), Image.LANCZOS)
out.save(sys.argv[3])
