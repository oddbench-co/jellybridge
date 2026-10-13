"""Ranks the regions where web and app differ most, in CSS px.

usage: hotspots.py web.png app.png [count] [threshold]
"""
import sys
import numpy as np
from PIL import Image

a = np.asarray(Image.open(sys.argv[1]).convert("RGB"), dtype=np.int16)
b = np.asarray(Image.open(sys.argv[2]).convert("RGB"), dtype=np.int16)
count = int(sys.argv[3]) if len(sys.argv) > 3 else 12
threshold = int(sys.argv[4]) if len(sys.argv) > 4 else 40
d = np.abs(a - b).max(2) > threshold
h, w = d.shape
bs = 60
grid = d[: h // bs * bs, : w // bs * bs].reshape(h // bs, bs, w // bs, bs).mean((1, 3))
cells = sorted(((grid[i, j], i, j) for i in range(grid.shape[0]) for j in range(grid.shape[1])), reverse=True)
for v, i, j in cells[:count]:
    print(f"{v:.2f} css x {j*bs/1.5:.0f}-{(j+1)*bs/1.5:.0f} y {i*bs/1.5:.0f}-{(i+1)*bs/1.5:.0f}")
