#!/usr/bin/env python3
# SPDX-License-Identifier: GPL-3.0-or-later
"""Regenerate the three platform logos and all installer icon formats.

Sources (hand-drawn line art, white background) live next to this script:
  backup.png  — floppy + up arrow (the constant "backup" base)
  tux.png / mac.png / windows.png — the per-OS markers

Output (transparent):
  <here>/linux-logo.png  macos-logo.png  windows-logo.png        (1024, RGBA)
  python/packaging/icons/hicolor/<size>/apps/backuptool.png      (Linux .deb)
  python/packaging/backuptool.icns                               (macOS .app)
  python/packaging/win/backuptool.ico                            (Windows)

Run:  python3 assets/logos/make-logos.py   (needs Pillow)
"""
import os
from PIL import Image, ImageDraw

HERE = os.path.dirname(os.path.abspath(__file__))
ROOT = os.path.abspath(os.path.join(HERE, "..", ".."))
PKG = os.path.join(ROOT, "python", "packaging")


def key_white(im):
    """Make the near-white scan background transparent with a smooth edge ramp."""
    im = im.convert("RGBA")
    px = im.load()
    w, h = im.size
    out = Image.new("RGBA", (w, h), (0, 0, 0, 0))
    op = out.load()
    for y in range(h):
        for x in range(w):
            r, g, b, _ = px[x, y]
            lum = 0.299 * r + 0.587 * g + 0.114 * b
            alpha = 0 if lum >= 235 else 255 if lum <= 120 else int((235 - lum) / 115 * 255)
            if alpha:
                op[x, y] = (r, g, b, alpha)
    return out


def autocrop(im, pad=4):
    bb = im.getbbox()
    if not bb:
        return im
    l, t, r, b = bb
    return im.crop((max(0, l - pad), max(0, t - pad),
                    min(im.size[0], r + pad), min(im.size[1], b + pad)))


def scale_w(im, w):
    f = w / im.size[0]
    return im.resize((w, int(im.size[1] * f)), Image.LANCZOS)


def fit(im, maxw, maxh):
    f = min(maxw / im.size[0], maxh / im.size[1])
    return im.resize((int(im.size[0] * f), int(im.size[1] * f)), Image.LANCZOS)


def load(name):
    return autocrop(key_white(Image.open(os.path.join(HERE, name))))


def compose(marker_file):
    """backup floppy as the base; OS marker laid onto the label, where the arrow points."""
    C = 1024
    cv = Image.new("RGBA", (C, C), (0, 0, 0, 0))
    bk = scale_w(load("backup.png"), 880)
    px, py = (C - bk.size[0]) // 2, (C - bk.size[1]) // 2
    cv.alpha_composite(bk, (px, py))
    bw, bh = bk.size
    logo = fit(load(marker_file), int(bw * 0.46), int(bh * 0.36))
    lw, lh = logo.size
    cx, cy = px + int(bw * 0.40), py + int(bh * 0.63)
    lx, ly = cx - lw // 2, cy - lh // 2
    pad = 16
    mask = Image.new("L", (C, C), 0)
    ImageDraw.Draw(mask).rounded_rectangle(
        (lx - pad, ly - pad, lx + lw + pad, ly + lh + pad), radius=22, fill=255)
    cv.paste((0, 0, 0, 0), (0, 0), mask)   # erase floppy lines under the marker
    cv.alpha_composite(logo, (lx, ly))
    return cv


def main():
    logos = {"linux": "tux.png", "macos": "mac.png", "windows": "windows.png"}
    imgs = {}
    for osname, marker in logos.items():
        img = compose(marker)
        img.save(os.path.join(HERE, f"{osname}-logo.png"))
        imgs[osname] = img
        print(f"  logo: {osname}-logo.png")

    # Linux: hicolor PNG set referenced by the .desktop "Icon=backuptool"
    for size in (16, 32, 48, 64, 128, 256, 512):
        d = os.path.join(PKG, "icons", "hicolor", f"{size}x{size}", "apps")
        os.makedirs(d, exist_ok=True)
        imgs["linux"].resize((size, size), Image.LANCZOS).save(
            os.path.join(d, "backuptool.png"))
    print("  linux:  python/packaging/icons/hicolor/*/apps/backuptool.png")

    # Windows: multi-resolution .ico
    imgs["windows"].save(
        os.path.join(PKG, "win", "backuptool.ico"), format="ICO",
        sizes=[(16, 16), (24, 24), (32, 32), (48, 48), (64, 64), (128, 128), (256, 256)])
    print("  windows: python/packaging/win/backuptool.ico")

    # macOS: .icns
    imgs["macos"].save(os.path.join(PKG, "backuptool.icns"), format="ICNS")
    print("  macos:  python/packaging/backuptool.icns")


if __name__ == "__main__":
    main()
