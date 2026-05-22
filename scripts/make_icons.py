#!/usr/bin/env python3
"""Generate Tauri app/tray icons. Run from project root."""
import os
import shutil
import subprocess
import sys
from PIL import Image, ImageDraw, ImageFont

OUT_DIR = "src-tauri/icons"

def font(size):
    for path in (
        "/System/Library/Fonts/HelveticaNeue.ttc",
        "/System/Library/Fonts/Helvetica.ttc",
        "/System/Library/Fonts/Supplemental/Arial.ttf",
    ):
        if os.path.exists(path):
            try:
                return ImageFont.truetype(path, size)
            except Exception:
                pass
    return ImageFont.load_default()

def make_base(size=1024):
    img = Image.new("RGBA", (size, size), (0, 0, 0, 0))
    d = ImageDraw.Draw(img)
    pad = size * 0.05
    d.rounded_rectangle(
        (pad, pad, size - pad, size - pad),
        radius=int(size * 0.20),
        fill=(75, 110, 180, 255),
    )
    f = font(int(size * 0.66))
    d.text((size / 2, size / 2 + size * 0.02), "M", fill=(255, 255, 255, 255),
           font=f, anchor="mm")
    return img

def main():
    os.makedirs(OUT_DIR, exist_ok=True)
    base = make_base(1024)

    base.save(f"{OUT_DIR}/icon.png")

    sizes = {
        "32x32.png": 32,
        "128x128.png": 128,
        "128x128@2x.png": 256,
    }
    for name, sz in sizes.items():
        base.resize((sz, sz), Image.LANCZOS).save(f"{OUT_DIR}/{name}")

    # Windows .ico
    base.resize((256, 256), Image.LANCZOS).save(
        f"{OUT_DIR}/icon.ico",
        format="ICO",
        sizes=[(16, 16), (32, 32), (48, 48), (64, 64), (128, 128), (256, 256)],
    )

    # macOS .icns via iconutil
    iconset = f"{OUT_DIR}/icon.iconset"
    os.makedirs(iconset, exist_ok=True)
    for sz, name in [
        (16, "16x16"), (32, "16x16@2x"), (32, "32x32"), (64, "32x32@2x"),
        (128, "128x128"), (256, "128x128@2x"), (256, "256x256"),
        (512, "256x256@2x"), (512, "512x512"), (1024, "512x512@2x"),
    ]:
        base.resize((sz, sz), Image.LANCZOS).save(f"{iconset}/icon_{name}.png")
    subprocess.run(
        ["iconutil", "-c", "icns", iconset, "-o", f"{OUT_DIR}/icon.icns"],
        check=True,
    )
    shutil.rmtree(iconset)

    print(f"icons generated in {OUT_DIR}/")

if __name__ == "__main__":
    main()
