# Draws the Reprise logo and writes every file made from it:
#   data/icons/hicolor/{scalable,16x16,24x24,32x32,48x48}/apps/reprise.*
#   crates/reprisekit/src/art/logo/r{16,24,32,48}.png (coverage masks)
#   docs/logo/reprise-lockup{,-white}.svg
#
# The mark is an R drawn as one ribbon: stem, top bar and bowl, whose lower
# stroke folds over into the leg, with a thin gap along the leg's top edge
# where it lies over the bowl. The master is drawn on a 64-unit grid; each
# icon size has its own spec with strokes and the gap on whole pixels, and
# 16 px is drawn by hand (HAND_16) so the gap stays a clean stair-step.
#
# Run from the repository root (needs rsvg-convert):
#   uv run --with shapely --with fonttools --with pillow python -I scripts/logo/build.py
import io
import math
import subprocess
from dataclasses import dataclass
from pathlib import Path

from fontTools.pens.svgPathPen import SVGPathPen
from fontTools.pens.transformPen import TransformPen
from fontTools.ttLib import TTFont
from fontTools.varLib.instancer import instantiateVariableFont
from PIL import Image
from shapely.geometry import LineString
from shapely.ops import unary_union

ROOT = Path(__file__).resolve().parents[2]
ICONS = ROOT / "data/icons/hicolor"
MASKS = ROOT / "crates/reprisekit/src/art/logo"
DOCS = ROOT / "docs/logo"

BLUE = "#1450ff"
INK = "#1a1a1a"
Q = 64  # arc segments per quarter circle

# Adwaita Sans is under the SIL Open Font License; the lockup carries its
# glyphs as outlines.
FONT = "/usr/share/fonts/Adwaita/AdwaitaSans-Regular.ttf"
WEIGHT = 600  # "Reprise"
WEIGHT_LIGHT = 400  # " Desktop"


@dataclass
class Spec:
    size: int
    w: float  # stroke width
    gap: float  # between the leg and the bowl it folds over
    stem_x: float  # stem centerline
    top_y: float  # top bar centerline
    base_y: float  # where the stem and the leg end (cap centres)
    bowl_r: float  # bowl centerline radius
    bowl_x: float  # where the top bar starts curving
    fold_x: float  # where the bowl's lower stroke folds into the leg
    leg_end_x: float


SPECS = {
    64: Spec(64, w=10, gap=1.8, stem_x=9, top_y=9, base_y=55, bowl_r=12,
             bowl_x=41, fold_x=24, leg_end_x=55),
    48: Spec(48, w=8, gap=1.5, stem_x=6, top_y=6, base_y=42, bowl_r=9,
             bowl_x=33, fold_x=18, leg_end_x=42),
    32: Spec(32, w=5, gap=1.5, stem_x=4.5, top_y=4.5, base_y=27.5, bowl_r=6,
             bowl_x=21.5, fold_x=12, leg_end_x=27.5),
    24: Spec(24, w=4, gap=1, stem_x=3, top_y=3, base_y=21, bowl_r=4.5,
             bowl_x=16.5, fold_x=9, leg_end_x=21),
}

# '#' full, '+' 2/3, '-' 1/3, '.' clear.
HAND_16 = """
................
.+##########-...
.############+..
.#############-.
.###.......-###.
.###........###.
.###.......-###.
.###..+#.#####+.
.###.-###.####-.
.###..+###.##-..
.###...+###+....
.###....+###+...
.###.....+###+..
.###......+###+.
.+#+.......-##+.
................
"""
HAND_ALPHA = {"#": 255, "+": 170, "-": 85, ".": 0}


def mark(s: Spec):
    h = s.w / 2
    cy = s.top_y + s.bowl_r
    low = cy + s.bowl_r
    pts = [(s.stem_x, s.base_y), (s.stem_x, s.top_y), (s.bowl_x, s.top_y)]
    for i in range(1, 2 * Q):
        a = -math.pi / 2 + math.pi * i / (2 * Q)
        pts.append((s.bowl_x + s.bowl_r * math.cos(a), cy + s.bowl_r * math.sin(a)))
    pts += [(s.bowl_x, low), (s.fold_x, low)]
    body = LineString(pts).buffer(h, quad_segs=Q, join_style=1)
    leg = LineString([(s.fold_x, low), (s.leg_end_x, s.base_y)]).buffer(h, quad_segs=Q)
    body = body.difference(leg.buffer(s.gap, quad_segs=Q))
    return unary_union([body, leg])


def path_d(geom) -> str:
    parts = []
    for p in getattr(geom, "geoms", [geom]):
        for ring in [p.exterior, *p.interiors]:
            c = list(ring.coords)[:-1]
            parts.append("M" + " L".join(f"{x:.3f},{y:.3f}" for x, y in c) + "Z")
    return "".join(parts)


def mark_svg(s: Spec, fill: str) -> str:
    return (
        f'<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 {s.size} {s.size}" '
        f'width="{s.size}" height="{s.size}"><path fill="{fill}" d="{path_d(mark(s))}"/></svg>\n'
    )


def coverage(size: int) -> Image.Image:
    """The mark at `size` px as an 8-bit coverage image."""
    if size == 16:
        im = Image.new("L", (16, 16))
        for y, row in enumerate(HAND_16.strip().splitlines()):
            for x, c in enumerate(row):
                im.putpixel((x, y), HAND_ALPHA[c])
        return im
    png = subprocess.run(
        ["rsvg-convert", "-"], input=mark_svg(SPECS[size], "#000").encode(),
        capture_output=True, check=True,
    ).stdout
    return Image.open(io.BytesIO(png)).convert("RGBA").getchannel("A")


def tinted(cov: Image.Image, hex_rgb: str) -> Image.Image:
    im = Image.new("RGBA", cov.size, hex_rgb)
    im.putalpha(cov)
    return im


def word(text: str, cap_px: float, weight: int, x: float = 0.0):
    """`text` as one SVG path starting at `x`, baseline at y=0, and the pen's
    x after it."""
    font = instantiateVariableFont(TTFont(FONT), {"wght": weight})
    k = cap_px / font["OS/2"].sCapHeight
    cmap, glyphs, hmtx = font.getBestCmap(), font.getGlyphSet(), font["hmtx"]
    pen = SVGPathPen(glyphs)
    for ch in text:
        g = cmap[ord(ch)]
        glyphs[g].draw(TransformPen(pen, (k, 0, 0, -k, x, 0)))
        x += hmtx[g][0] * k
    return pen.getCommands(), x


def lockup(fill_mark: str, fill_text: str) -> str:
    cap = 40.0
    mark_h = 56.0  # the master's ink spans 4..60
    k = cap * 1.3 / mark_h  # the mark stands a little taller than the caps
    pad = 8.0
    d_name, x = word("Reprise", cap, WEIGHT)
    d_kind, w_word = word(" Desktop", cap, WEIGHT_LIGHT, x)
    d_word = d_name + d_kind
    mark_w = mark_h * k
    gap = 0.42 * cap
    base = pad + (mark_h * k - cap) / 2 + cap
    width = pad * 2 + mark_w + gap + w_word
    height = pad * 2 + mark_h * k
    m = pad - 4 * k
    return (
        f'<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 {width:.1f} {height:.1f}">'
        f'<path fill="{fill_mark}" transform="translate({m:.3f} {m:.3f}) scale({k:.4f})" '
        f'd="{path_d(mark(SPECS[64]))}"/>'
        f'<path fill="{fill_text}" transform="translate({pad + mark_w + gap:.3f} {base:.3f})" '
        f'd="{d_word}"/></svg>\n'
    )


def main():
    scalable = ICONS / "scalable/apps/reprise.svg"
    scalable.parent.mkdir(parents=True, exist_ok=True)
    scalable.write_text(mark_svg(SPECS[64], BLUE))
    MASKS.mkdir(parents=True, exist_ok=True)
    for size in (16, 24, 32, 48):
        cov = coverage(size)
        cov.save(MASKS / f"r{size}.png", optimize=True)
        icon = ICONS / f"{size}x{size}/apps/reprise.png"
        icon.parent.mkdir(parents=True, exist_ok=True)
        tinted(cov, BLUE).save(icon, optimize=True)
    DOCS.mkdir(parents=True, exist_ok=True)
    (DOCS / "reprise-lockup.svg").write_text(lockup(BLUE, INK))
    (DOCS / "reprise-lockup-white.svg").write_text(lockup("#ffffff", "#ffffff"))


if __name__ == "__main__":
    main()
