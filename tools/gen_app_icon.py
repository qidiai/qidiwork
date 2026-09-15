"""Generate the QidiWork Workbench app icon (gui/app-icon.png).

Reproducible source of the brand icon: 1024x1024 rounded square,
blue diagonal-lit gradient background, centered geometric white "Q".
Supersampled at 4096px then downscaled for smooth edges.

Usage: python tools/gen_app_icon.py
"""

import math

from PIL import Image, ImageDraw, ImageFilter

S = 4096          # supersample canvas
OUT = 1024        # final size
K = S // OUT      # scale factor from 1024-space to canvas space
RADIUS = int(OUT * 0.22) * K  # corner radius of the rounded square

# background gradient corners (top-left -> bottom-right)
C_TOP = (78, 146, 255)     # #4E92FF
C_BOT = (12, 51, 196)      # #0C33C4


def lerp(a, b, t):
    return tuple(int(round(a[i] + (b[i] - a[i]) * t)) for i in range(3))


def rounded_mask(size, radius):
    m = Image.new("L", (size, size), 0)
    ImageDraw.Draw(m).rounded_rectangle(
        [0, 0, size - 1, size - 1], radius=radius, fill=255
    )
    return m


def background():
    # diagonal gradient via anti-diagonal bands: color depends on (x + y)
    img = Image.new("RGB", (S, S), C_BOT)
    d = ImageDraw.Draw(img)
    span = 2 * S
    step = S // 256
    for band in range(0, span, step):
        color = lerp(C_TOP, C_BOT, min(1.0, band / span))
        q1 = (min(band, S), max(0, band - S))
        q2 = (min(band + step, S), max(0, band + step - S))
        q3 = (max(0, band + step - S), min(band + step, S))
        q4 = (max(0, band - S), min(band, S))
        d.polygon([q1, q2, q3, q4], fill=color)
    # soft top-left highlight (lit feel)
    glow = Image.new("L", (512, 512), 0)
    gd = ImageDraw.Draw(glow)
    for i in range(256):
        a = int(70 * (1.0 - i / 256) ** 1.6)
        gd.ellipse([256 - i * 2, 256 - i * 2, 256 + i * 2, 256 + i * 2], fill=a)
    glow = glow.resize((S, S)).filter(ImageFilter.GaussianBlur(S // 40))
    img.paste(Image.new("RGB", (S, S), (255, 255, 255)), (0, 0), glow)
    # subtle bottom-right shade for depth
    shade = Image.new("L", (512, 512), 0)
    sd = ImageDraw.Draw(shade)
    for i in range(256):
        a = int(55 * (1.0 - i / 256) ** 1.6)
        sd.ellipse([256 - i * 2, 256 - i * 2, 256 + i * 2, 256 + i * 2], fill=a)
    shade = shade.resize((S, S)).filter(ImageFilter.GaussianBlur(S // 40))
    img.paste(Image.new("RGB", (S, S), (4, 20, 110)), (0, 0), shade)
    return img


def capsule(draw, p1, p2, width, fill):
    """Rounded-end bar from p1 to p2 (1024-space coords)."""
    x1, y1 = p1[0] * K, p1[1] * K
    x2, y2 = p2[0] * K, p2[1] * K
    w = width * K / 2.0
    dx, dy = x2 - x1, y2 - y1
    ln = math.hypot(dx, dy)
    ux, uy = -dy / ln * w, dx / ln * w
    draw.polygon(
        [(x1 + ux, y1 + uy), (x2 + ux, y2 + uy), (x2 - ux, y2 - uy), (x1 - ux, y1 - uy)],
        fill=fill,
    )
    r = w
    draw.ellipse([x1 - r, y1 - r, x1 + r, y1 + r], fill=fill)
    draw.ellipse([x2 - r, y2 - r, x2 + r, y2 + r], fill=fill)


def glyph_q():
    """White geometric Q as an alpha mask (ring + 45-degree tail)."""
    m = Image.new("L", (S, S), 0)
    d = ImageDraw.Draw(m)
    cx, cy = 500 * K, 486 * K   # optical center (tail adds bottom-right weight)
    r_out = 292 * K
    r_in = 215 * K
    d.ellipse([cx - r_out, cy - r_out, cx + r_out, cy + r_out], fill=255)
    d.ellipse([cx - r_in, cy - r_in, cx + r_in, cy + r_in], fill=0)
    # tail: starts inside the counter, crosses the ring at 45 degrees
    capsule(d, (492, 478), (742, 728), 82, 255)
    return m


def main():
    icon = background().convert("RGBA")
    icon.paste((255, 255, 255, 255), (0, 0), glyph_q())
    icon.putalpha(rounded_mask(S, RADIUS))
    icon = icon.resize((OUT, OUT), Image.LANCZOS)
    out_path = "gui/app-icon.png"
    icon.save(out_path, "PNG")
    print("written", out_path)


if __name__ == "__main__":
    main()
