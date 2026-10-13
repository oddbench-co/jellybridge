"""The same region from both screenshots, web above app, at full resolution.

usage: crop.py web.png app.png out.png x0 y0 x1 y1   (CSS px; images are 1.5x)
"""
import sys
from PIL import Image, ImageDraw

web = Image.open(sys.argv[1]).convert("RGB")
app = Image.open(sys.argv[2]).convert("RGB")
x0, y0, x1, y1 = (round(float(v) * 1.5) for v in sys.argv[4:8])
a = web.crop((x0, y0, x1, y1))
b = app.crop((x0, y0, x1, y1))
out = Image.new("RGB", (a.width, a.height * 2 + 6), (255, 0, 255))
out.paste(a, (0, 0))
out.paste(b, (0, a.height + 6))
out.save(sys.argv[3])
