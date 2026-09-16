#!/usr/bin/env python3
"""Generate the original RF-EQ branding assets RackForge requires.

RackForge validates three PNGs by exact size: a 512x512 icon, a 1600x400
banner and a 1920x1080 splash. They are drawn here rather than committed as
opaque binaries so the visual identity is reviewable, reproducible, and
unmistakably this project's own work: a frequency grid, a response curve,
and a dot at each band's centre.

Run:  python tools/generate-branding.py
"""

from __future__ import annotations

import math
from pathlib import Path

from PIL import Image, ImageDraw, ImageFont

ROOT = Path(__file__).resolve().parents[1]
OUTPUT = ROOT / "plugin" / "package" / "branding"

FONT_CANDIDATES = (
    Path("C:/Windows/Fonts/arialbd.ttf"),
    Path("C:/Windows/Fonts/segoeuib.ttf"),
    Path("/usr/share/fonts/truetype/dejavu/DejaVuSans-Bold.ttf"),
)

# The palette also lives in rackforge-plugin.toml and web/play.html; keep the
# three in step.
PANEL = (14, 18, 22)
PANEL_LIGHT = (27, 35, 43)
INK = (232, 236, 239)
MUTED = (139, 150, 160)
ACCENT = (46, 196, 182)
ACCENT_DIM = (21, 90, 83)
STEEL = (174, 182, 189)
GRID = (36, 46, 54)

# The curve the artwork draws: the "Piano Air" setting with a little more of
# everything, so it reads at icon size. (band, frequency, gain dB, Q)
CURVE = (
    ("hpf", 40.0, 0.0, 0.7071),
    ("low_shelf", 120.0, -3.0, 1.0),
    ("peak", 500.0, 2.5, 1.0),
    ("peak", 3000.0, -2.0, 1.4),
    ("high_shelf", 8000.0, 5.0, 1.0),
)
LOW_HZ = 20.0
HIGH_HZ = 20000.0
SPAN_DB = 9.0


def font(size: int) -> ImageFont.FreeTypeFont:
    for candidate in FONT_CANDIDATES:
        if candidate.exists():
            return ImageFont.truetype(str(candidate), size)
    return ImageFont.load_default()


def field(size: tuple[int, int]) -> Image.Image:
    """The dark brushed field everything sits on."""
    width, height = size
    image = Image.new("RGB", size, PANEL)
    draw = ImageDraw.Draw(image)
    step = max(6, height // 90)
    for y in range(0, height, step):
        shade = 4 if (y // step) % 2 == 0 else 0
        draw.line([(0, y), (width, y)], fill=(PANEL[0] + shade, PANEL[1] + shade, PANEL[2] + shade))
    return image


def response_db(frequency: float) -> float:
    """The analogue prototypes of the cookbook bands, in dB at `frequency`.
    Close enough to the digital curve to draw with."""
    total = 0.0
    for band, centre, gain, q in CURVE:
        w = frequency / centre
        if band == "hpf":
            magnitude = (w * w) / math.sqrt((1 - w * w) ** 2 + (w / q) ** 2)
            total += 20 * math.log10(max(1e-6, magnitude))
            continue
        a = 10 ** (gain / 40)
        if band == "peak":
            num = complex(1 - w * w, a * w / q)
            den = complex(1 - w * w, w / (a * q))
        elif band == "low_shelf":
            num = a * complex(a - w * w, math.sqrt(a) / q * w)
            den = complex(1 - a * w * w, math.sqrt(a) / q * w)
        else:
            num = a * complex(1 - a * w * w, math.sqrt(a) / q * w)
            den = complex(a - w * w, math.sqrt(a) / q * w)
        total += 20 * math.log10(abs(num) / abs(den))
    return total


def curve(
    draw: ImageDraw.ImageDraw,
    box: tuple[float, float, float, float],
    stroke: int,
    dots: bool = True,
) -> None:
    """The frequency grid across `box`, the response over it, a dot per band."""
    left, top, right, bottom = box
    middle = (top + bottom) / 2
    half = (bottom - top) / 2
    octaves = math.log2(HIGH_HZ / LOW_HZ)

    def x_of(frequency: float) -> float:
        return left + (right - left) * math.log2(frequency / LOW_HZ) / octaves

    def y_of(gain: float) -> float:
        return middle - max(-SPAN_DB, min(SPAN_DB, gain)) / SPAN_DB * half

    decade = 10.0
    while decade <= HIGH_HZ:
        for multiple in range(1, 10):
            frequency = decade * multiple
            if LOW_HZ <= frequency <= HIGH_HZ:
                weight = max(1, stroke // 5) if multiple == 1 else 1
                draw.line([(x_of(frequency), top), (x_of(frequency), bottom)], fill=GRID, width=weight)
        decade *= 10
    for gain in (-6.0, -3.0, 0.0, 3.0, 6.0):
        colour = STEEL if gain == 0.0 else GRID
        draw.line([(left, y_of(gain)), (right, y_of(gain))], fill=colour, width=max(1, stroke // 5))

    points = []
    steps = int(right - left)
    for i in range(steps + 1):
        frequency = LOW_HZ * (HIGH_HZ / LOW_HZ) ** (i / max(1, steps))
        points.append((left + i, y_of(response_db(frequency))))
    draw.line(points, fill=ACCENT, width=stroke)

    if dots:
        # A dot where each band sits on the curve: a shelf's corner is
        # halfway up it, the high-pass's cutoff three decibels down.
        for _, centre, _, _ in CURVE:
            x, y = x_of(centre), y_of(response_db(centre))
            r = stroke * 1.6
            draw.ellipse([x - r, y - r, x + r, y + r], fill=PANEL_LIGHT, outline=ACCENT, width=max(2, stroke // 2))


def make_icon() -> None:
    size = (512, 512)
    image = field(size)
    draw = ImageDraw.Draw(image)
    draw.rounded_rectangle([28, 28, 484, 484], radius=72, fill=PANEL_LIGHT, outline=STEEL, width=6)
    curve(draw, (70, 130, 442, 362), stroke=10)
    label = font(64)
    draw.text((256, 428), "EQ", font=label, fill=INK, anchor="mm")
    image.save(OUTPUT / "icon.png")


def make_banner() -> None:
    size = (1600, 400)
    image = field(size)
    draw = ImageDraw.Draw(image)
    curve(draw, (60, 60, 900, 340), stroke=8)
    title = font(96)
    draw.text((1240, 160), "RF-EQ", font=title, fill=INK, anchor="mm")
    subtitle = font(34)
    draw.text((1240, 240), "Eight-band parametric equaliser", font=subtitle, fill=ACCENT, anchor="mm")
    image.save(OUTPUT / "banner.png")


def make_splash() -> None:
    size = (1920, 1080)
    image = field(size)
    draw = ImageDraw.Draw(image)
    title = font(150)
    draw.text((960, 200), "RF-EQ", font=title, fill=INK, anchor="mm")
    subtitle = font(44)
    draw.text((960, 320), "Eight-band parametric equaliser", font=subtitle, fill=ACCENT, anchor="mm")
    draw.rounded_rectangle([160, 420, 1760, 920], radius=40, fill=PANEL_LIGHT, outline=STEEL, width=4)
    curve(draw, (220, 460, 1700, 880), stroke=10)
    footer = font(34)
    draw.text(
        (960, 990),
        "high-pass  ·  low shelf  ·  two peaks  ·  high shelf  ·  trim and bypass",
        font=footer,
        fill=MUTED,
        anchor="mm",
    )
    image.save(OUTPUT / "splash.png")


def main() -> None:
    OUTPUT.mkdir(parents=True, exist_ok=True)
    make_icon()
    make_banner()
    make_splash()
    for name, expected in (("icon.png", (512, 512)), ("banner.png", (1600, 400)), ("splash.png", (1920, 1080))):
        with Image.open(OUTPUT / name) as image:
            assert image.size == expected, f"{name} is {image.size}, expected {expected}"
            assert image.mode in ("RGB", "RGBA"), f"{name} is {image.mode}"
        print(f"wrote {OUTPUT / name}")


if __name__ == "__main__":
    main()
