# SPDX-License-Identifier: AGPL-3.0-or-later
# Copyright (C) 2026 SlammingProgramming and Claude Companion contributors
"""Draw Claude Companion's own icon and tray artwork. Run from the repository root (needs Pillow): python -I scripts/make-art.py

The mark: two overlapping window outlines (Claude Desktop and the terminal), the second holding a
prompt chevron, on an indigo tile. Every size is drawn from the same numbers, 8x oversampled.
"""
from pathlib import Path

from PIL import Image, ImageDraw, ImageFilter, ImageOps

# Geometry on a 1024 canvas (the tile is 100..924).
TILE = (100, 100, 924, 924, 186)
LEFT = (280, 330, 580, 630)
RIGHT = (444, 394, 744, 694)
RADIUS = 56
STROKE = 44
CHEVRON = [(632, 478), (694, 544), (632, 610)]
CHEV_W = 34
UNDER = ((652, 646), (706, 646))
UNDER_W = 28
INK_TOP = (122, 150, 255)
INK_BOT = (38, 54, 150)
OUT = Path("crates/app")


def rrect(d, box, r, **kw):
    d.rounded_rectangle(box, radius=r, **kw)


def outline(d, box, color, scale):
    x0, y0, x1, y1 = [v * scale for v in box]
    h = STROKE * scale / 2
    rrect(d, (x0 - h, y0 - h, x1 + h, y1 + h), (RADIUS + STROKE / 2) * scale, outline=color, width=int(STROKE * scale))


def mark(size, box, color, lens=None, scale=8):
    """The two outlines and the chevron, mapped from the 1024 canvas into `box` of a `size` square."""
    s = size * scale
    k = (box[2] - box[0]) / 1024 * scale  # canvas unit -> pixels
    ox, oy = box[0] * scale, box[1] * scale

    img = Image.new("RGBA", (s, s), (0, 0, 0, 0))
    d = ImageDraw.Draw(img)

    def pt(p):
        return (ox + p[0] * k, oy + p[1] * k)

    def sq(b):
        return (ox + b[0] * k, oy + b[1] * k, ox + b[2] * k, oy + b[3] * k)

    def edge(b, width_units):
        x0, y0, x1, y1 = sq(b)
        h = width_units * k / 2
        r = (RADIUS + width_units / 2) * k
        d.rounded_rectangle((x0 - h, y0 - h, x1 + h, y1 + h), radius=r, outline=color, width=max(1, round(width_units * k)))

    if lens is not None:
        lx0, ly0 = RIGHT[0], RIGHT[1]
        lx1, ly1 = LEFT[2], LEFT[3]
        layer = Image.new("RGBA", (s, s), (0, 0, 0, 0))
        ImageDraw.Draw(layer).rectangle(sq((lx0, ly0, lx1, ly1)), fill=lens)
        # Keep only the part inside both outlines' rounded corners.
        mask = Image.new("L", (s, s), 0)
        md = ImageDraw.Draw(mask)
        md.rounded_rectangle(sq(LEFT), radius=RADIUS * k, fill=255)
        mask2 = Image.new("L", (s, s), 0)
        ImageDraw.Draw(mask2).rounded_rectangle(sq(RIGHT), radius=RADIUS * k, fill=255)
        both = ImageChops_multiply(mask, mask2)
        img.paste(layer, (0, 0), both)
    edge(LEFT, STROKE)
    edge(RIGHT, STROKE)
    w = max(1, round(CHEV_W * k))
    pts = [pt(p) for p in CHEVRON]
    d.line(pts, fill=color, width=w, joint="curve")
    for p in (pts[0], pts[-1]):
        d.ellipse((p[0] - w / 2, p[1] - w / 2, p[0] + w / 2, p[1] + w / 2), fill=color)
    uw = max(1, round(UNDER_W * k))
    a, b = pt(UNDER[0]), pt(UNDER[1])
    d.line((a, b), fill=color, width=uw)
    for p in (a, b):
        d.ellipse((p[0] - uw / 2, p[1] - uw / 2, p[0] + uw / 2, p[1] + uw / 2), fill=color)
    return img


def ImageChops_multiply(a, b):
    from PIL import ImageChops

    return ImageChops.multiply(a, b)


def tile(size, scale=4):
    s = size * scale
    k = size / 1024 * scale
    grad = Image.linear_gradient("L").resize((s, s))
    bg = ImageOps.colorize(grad, INK_TOP, INK_BOT).convert("RGBA")
    mask = Image.new("L", (s, s), 0)
    ImageDraw.Draw(mask).rounded_rectangle(
        (TILE[0] * k, TILE[1] * k, TILE[2] * k, TILE[3] * k), radius=TILE[4] * k, fill=255
    )
    out = Image.new("RGBA", (s, s), (0, 0, 0, 0))
    out.paste(bg, (0, 0), mask)
    # A soft light from the top, as a highlight.
    glow = Image.new("RGBA", (s, s), (255, 255, 255, 0))
    gd = ImageDraw.Draw(glow)
    gd.ellipse((s * 0.1, -s * 0.35, s * 0.9, s * 0.45), fill=(255, 255, 255, 52))
    glow = glow.filter(ImageFilter.GaussianBlur(s * 0.06))
    out.paste(glow, (0, 0), Image.composite(glow.getchannel("A"), Image.new("L", (s, s), 0), mask))
    # The mark, with a soft shadow beneath.
    half = size // 2
    fg = mark(half, (0, 0, half, half), (255, 255, 255, 255), lens=(255, 255, 255, 108), scale=8)
    fg = fg.resize((s, s), Image.LANCZOS)
    shadow = Image.new("RGBA", (s, s), (0, 0, 0, 0))
    shadow.putalpha(fg.getchannel("A").point(lambda v: int(v * 0.35)))
    shadow = shadow.filter(ImageFilter.GaussianBlur(s * 0.018))
    sh = Image.new("RGBA", (s, s), (20, 24, 70, 0))
    sh.putalpha(shadow.getchannel("A"))
    sh = ImageChops_offset(sh, 0, round(s * 0.012))
    out.alpha_composite(sh)
    out.alpha_composite(fg)
    return out.resize((size, size), Image.LANCZOS)


def ImageChops_offset(img, dx, dy):
    from PIL import ImageChops

    return ImageChops.offset(img, dx, dy)


def template(size, attention=False):
    """Black on transparent, for the macOS menu bar (the system tints it)."""
    pad = size * 0.06
    box = (pad - 0, pad, size - pad, size - pad)
    # The mark spans 280..744 by 330..694 of the canvas: zoom the canvas so that fills the box.
    big = mark_in_box(size, (280, 330, 744, 694), box)
    if attention:
        s = size * 8
        d = ImageDraw.Draw(big)
        r = size * 0.15 * 8
        cx, cy = size * 0.82 * 8, size * 0.2 * 8
        d.ellipse((cx - r * 1.5, cy - r * 1.5, cx + r * 1.5, cy + r * 1.5), fill=(0, 0, 0, 0))
        big2 = big.copy()
        # Clear a halo, then the dot.
        halo = Image.new("L", big.size, 0)
        ImageDraw.Draw(halo).ellipse((cx - r * 1.6, cy - r * 1.6, cx + r * 1.6, cy + r * 1.6), fill=255)
        big2.paste((0, 0, 0, 0), (0, 0), halo)
        ImageDraw.Draw(big2).ellipse((cx - r, cy - r, cx + r, cy + r), fill=(0, 0, 0, 255))
        big = big2
    return big.resize((size, size), Image.LANCZOS)


def mark_in_box(size, src, dst, scale=8):
    """The mark drawn so the canvas region `src` fills `dst` (in output pixels)."""
    zoom = (dst[2] - dst[0]) / (src[2] - src[0])
    big = size * scale
    # Render on a virtual 1024 canvas scaled by `zoom`, shifted so `src` lands on `dst`.
    k = zoom * scale
    ox = dst[0] * scale - src[0] * k
    oy = dst[1] * scale - src[1] * k
    img = Image.new("RGBA", (big, big), (0, 0, 0, 0))
    d = ImageDraw.Draw(img)
    color = (0, 0, 0, 255)

    def pt(p):
        return (ox + p[0] * k, oy + p[1] * k)

    def edge(b):
        x0, y0 = pt((b[0], b[1]))
        x1, y1 = pt((b[2], b[3]))
        h = STROKE * k / 2
        d.rounded_rectangle((x0 - h, y0 - h, x1 + h, y1 + h), radius=(RADIUS + STROKE / 2) * k, outline=color, width=max(1, round(STROKE * k)))

    edge(LEFT)
    edge(RIGHT)
    w = max(1, round(CHEV_W * k))
    pts = [pt(p) for p in CHEVRON]
    d.line(pts, fill=color, width=w, joint="curve")
    for p in (pts[0], pts[-1]):
        d.ellipse((p[0] - w / 2, p[1] - w / 2, p[0] + w / 2, p[1] + w / 2), fill=color)
    return img


def svg(color_mode):
    def rect(b, extra=""):
        x0, y0, x1, y1 = b
        return f'<rect x="{x0}" y="{y0}" width="{x1 - x0}" height="{y1 - y0}" rx="{RADIUS}" {extra}/>'

    chevron = " ".join(f"{x},{y}" for x, y in CHEVRON)
    if color_mode == "tile":
        return f"""<svg xmlns="http://www.w3.org/2000/svg" width="1024" height="1024" viewBox="0 0 1024 1024">
  <defs>
    <linearGradient id="bg" x1="0" y1="0" x2="0.3" y2="1">
      <stop offset="0" stop-color="rgb{INK_TOP}"/>
      <stop offset="1" stop-color="rgb{INK_BOT}"/>
    </linearGradient>
    <clipPath id="left">{rect(LEFT)}</clipPath>
    <filter id="shadow" x="-30%" y="-30%" width="160%" height="160%">
      <feDropShadow dx="0" dy="12" stdDeviation="18" flood-color="#141846" flood-opacity="0.35"/>
    </filter>
  </defs>
  <rect x="{TILE[0]}" y="{TILE[1]}" width="{TILE[2] - TILE[0]}" height="{TILE[3] - TILE[1]}" rx="{TILE[4]}" fill="url(#bg)"/>
  <g filter="url(#shadow)" fill="none" stroke="#FFFFFF" stroke-width="{STROKE}" stroke-linecap="round" stroke-linejoin="round">
    {rect(RIGHT, 'fill="#FFFFFF" fill-opacity="0.42" stroke="none" clip-path="url(#left)"')}
    {rect(LEFT)}
    {rect(RIGHT)}
    <polyline points="{chevron}" stroke-width="{CHEV_W}"/>
    <line x1="{UNDER[0][0]}" y1="{UNDER[0][1]}" x2="{UNDER[1][0]}" y2="{UNDER[1][1]}" stroke-width="{UNDER_W}"/>
  </g>
</svg>
"""
    return f"""<svg xmlns="http://www.w3.org/2000/svg" width="36" height="36" viewBox="280 330 464 364">
  <g fill="none" stroke="#000000" stroke-width="{STROKE}" stroke-linecap="round" stroke-linejoin="round">
    {rect(LEFT)}
    {rect(RIGHT)}
    <polyline points="{chevron}" stroke-width="{CHEV_W}"/>
  </g>
</svg>
"""


def main():
    art = Path("assets")
    master = tile(1024)
    master.save(art / "icon.png")
    (art / "icon.svg").write_text(svg("tile"), encoding="utf-8")
    master.resize((512, 512), Image.LANCZOS).save(OUT / "resources/linux/claude-companion.png")
    tile(64).save(OUT / "resources/tray/color.png")
    template(36).save(OUT / "resources/tray/template.png")
    template(36, attention=True).save(OUT / "resources/tray/template-attention.png")
    for name in ("template.svg", "template-attention.svg"):
        (OUT / "resources/tray" / name).write_text(svg("template"), encoding="utf-8")
    master.save(OUT / "resources/windows/claude-companion.ico", sizes=[(n, n) for n in (16, 24, 32, 48, 64, 128, 256)])
    master.save(OUT / "resources/macos/AppIcon.icns", sizes=[(n, n) for n in (16, 32, 64, 128, 256, 512, 1024)])
    print("drawn")


main()
