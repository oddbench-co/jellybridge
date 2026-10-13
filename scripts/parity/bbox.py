"""Bounding box (CSS px) of pixels that differ from the background colour in a
region, for both screenshots: where something is drawn and how big it is.

usage: bbox.py web.png app.png x0 y0 x1 y1 [threshold]
"""
import sys
import numpy as np
from PIL import Image

def box(path, region, threshold):
    image = np.asarray(Image.open(path).convert("RGB"), dtype=np.int16)
    x0, y0, x1, y1 = (round(v * 1.5) for v in region)
    crop = image[y0:y1, x0:x1]
    background = crop[0, 0]
    lit = np.abs(crop - background).max(axis=2) > threshold
    ys, xs = np.nonzero(lit)
    if len(xs) == 0:
        return None
    return tuple(round(v / 1.5, 1) for v in (xs.min() + x0, ys.min() + y0, xs.max() + 1 + x0, ys.max() + 1 + y0))

region = [float(v) for v in sys.argv[3:7]]
threshold = int(sys.argv[7]) if len(sys.argv) > 7 else 40
for name, path in (("web", sys.argv[1]), ("app", sys.argv[2])):
    b = box(path, region, threshold)
    print(name, b, "size", None if b is None else (round(b[2] - b[0], 1), round(b[3] - b[1], 1)))
