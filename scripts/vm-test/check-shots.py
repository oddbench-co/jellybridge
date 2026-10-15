#!/usr/bin/env python3
"""Sanity-checks a guest run's screenshots: each exists and is not blank
(the player frame must show picture, not black). check-shots.py RESULT_DIR..."""
import sys, os
from PIL import Image, ImageStat
ok = True
for d in sys.argv[1:]:
    for name in ("01-home.png", "02-detail.png", "03-search.png", "04-player.png"):
        p = os.path.join(d, name)
        if not os.path.exists(p):
            print(f"{os.path.basename(d)} {name}: MISSING"); ok = False; continue
        st = ImageStat.Stat(Image.open(p).convert("L"))
        mean, dev = st.mean[0], st.stddev[0]
        good = dev > 12 and mean > 8
        ok &= good
        print(f"{os.path.basename(d)} {name}: {'ok' if good else 'BLANK?'} mean={mean:.0f} stddev={dev:.0f}")
sys.exit(0 if ok else 1)
