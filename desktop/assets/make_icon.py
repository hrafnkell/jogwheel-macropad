#!/usr/bin/env python3
"""Renders the Jogwheel app icon (assets/icon-*.png, assets/jogwheel.ico).
Run from desktop/: python3 assets/make_icon.py   (needs Pillow)"""
from PIL import Image, ImageDraw, ImageFilter
import math, os

S = 1024  # draw big, downscale for smooth edges


def lerp(a, b, t):
    return tuple(int(a[i] + (b[i] - a[i]) * t) for i in range(len(a)))


def render():
    img = Image.new("RGBA", (S, S), (0, 0, 0, 0))
    # Tile background: deep slate with a vertical gradient
    bg = Image.new("RGBA", (S, S))
    top, bot = (44, 52, 72, 255), (20, 24, 34, 255)
    for y in range(S):
        ImageDraw.Draw(bg).line([(0, y), (S, y)], fill=lerp(top, bot, y / S))
    mask = Image.new("L", (S, S), 0)
    ImageDraw.Draw(mask).rounded_rectangle([40, 40, S - 40, S - 40], radius=210, fill=255)
    img.paste(bg, (0, 0), mask)
    d = ImageDraw.Draw(img)

    # Knobs (left/right) with a blue pointer, screen in the middle
    knob_r, ky = 136, 330
    for cx, ang in ((236, -35), (S - 236, 50)):
        shadow = Image.new("RGBA", (S, S), (0, 0, 0, 0))
        ImageDraw.Draw(shadow).ellipse([cx - knob_r, ky - knob_r + 18, cx + knob_r, ky + knob_r + 18], fill=(0, 0, 0, 120))
        img.alpha_composite(shadow.filter(ImageFilter.GaussianBlur(14)))
        d = ImageDraw.Draw(img)
        d.ellipse([cx - knob_r, ky - knob_r, cx + knob_r, ky + knob_r], fill=(214, 220, 232, 255))
        d.ellipse([cx - knob_r + 22, ky - knob_r + 22, cx + knob_r - 22, ky + knob_r - 22], fill=(236, 240, 247, 255))
        a = math.radians(ang)
        x0, y0 = cx + math.sin(a) * 30, ky - math.cos(a) * 30
        x1, y1 = cx + math.sin(a) * (knob_r - 34), ky - math.cos(a) * (knob_r - 34)
        d.line([(x0, y0), (x1, y1)], fill=(59, 130, 246, 255), width=38)
        for (px_, py_) in ((x0, y0), (x1, y1)):
            d.ellipse([px_ - 19, py_ - 19, px_ + 19, py_ + 19], fill=(59, 130, 246, 255))

    # OLED screen with cyan glow
    sx0, sy0, sx1, sy1 = 400, 250, S - 400, 410
    glow = Image.new("RGBA", (S, S), (0, 0, 0, 0))
    ImageDraw.Draw(glow).rounded_rectangle([sx0 - 10, sy0 - 10, sx1 + 10, sy1 + 10], radius=30, fill=(103, 232, 249, 110))
    img.alpha_composite(glow.filter(ImageFilter.GaussianBlur(22)))
    d = ImageDraw.Draw(img)
    d.rounded_rectangle([sx0, sy0, sx1, sy1], radius=22, fill=(8, 12, 20, 255), outline=(103, 232, 249, 255), width=10)
    d.rectangle([sx0 + 26, sy0 + 30, sx1 - 26, sy0 + 58], fill=(103, 232, 249, 255))
    for i in range(2):
        y = sy0 + 88 + i * 40
        d.rectangle([sx0 + 26, y, sx1 - 26, y + 20], fill=(103, 232, 249, 150))

    # Two rows of keys
    cols, gap, kx0, kx1 = 5, 26, 130, S - 130
    kw = (kx1 - kx0 - gap * (cols - 1)) / cols
    for row, y in enumerate((560, 560 + kw + gap)):
        for c in range(cols):
            x = kx0 + c * (kw + gap)
            d.rounded_rectangle([x, y + 14, x + kw, y + kw + 14], radius=34, fill=(10, 13, 20, 200))
            col = (59, 130, 246, 255) if (row, c) == (0, 0) else (226, 232, 240, 255)
            d.rounded_rectangle([x, y, x + kw, y + kw], radius=34, fill=col)
    return img


def main():
    here = os.path.dirname(os.path.abspath(__file__))
    big = render()
    sizes = [16, 24, 32, 48, 64, 128, 256, 512]
    imgs = {s: big.resize((s, s), Image.LANCZOS) for s in sizes}
    for s in (32, 64, 128, 256, 512):
        imgs[s].save(os.path.join(here, f"icon-{s}.png"))
    imgs[256].save(os.path.join(here, "jogwheel.ico"), sizes=[(s, s) for s in (16, 24, 32, 48, 64, 128, 256)])


if __name__ == "__main__":
    main()
