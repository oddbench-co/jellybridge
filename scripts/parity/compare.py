"""Side by side (web | app) and a difference map, for one screen.

usage: compare.py web.png app.png out.png
The difference map shows in red where the two disagree by more than a small
threshold; the summary prints the share of differing pixels per region.
"""
import sys
import numpy as np
from PIL import Image

web = Image.open(sys.argv[1]).convert("RGB")
app = Image.open(sys.argv[2]).convert("RGB")
if app.size != web.size:
    print("size mismatch", web.size, app.size)
    app = app.resize(web.size)
a = np.asarray(web, dtype=np.int16)
b = np.asarray(app, dtype=np.int16)
diff = np.abs(a - b).max(axis=2)
mask = diff > 24
print(f"differing pixels: {mask.mean() * 100:.2f}%")
h, w = mask.shape
for name, (y0, y1) in {"header": (0, int(h * 0.05)), "top third": (0, h // 3), "middle": (h // 3, 2 * h // 3), "bottom": (2 * h // 3, h)}.items():
    print(f"  {name}: {mask[y0:y1].mean() * 100:.2f}%")
heat = (a * 0.35).astype(np.uint8)
heat[mask] = [255, 40, 40]
out = Image.new("RGB", (w * 3, h))
out.paste(web, (0, 0))
out.paste(app, (w, 0))
out.paste(Image.fromarray(heat), (2 * w, 0))
out = out.resize((w * 3 // 2, h // 2))
out.save(sys.argv[3])
