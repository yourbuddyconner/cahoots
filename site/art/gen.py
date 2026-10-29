#!/usr/bin/env python3
"""Draws the site's pictures. Run it after editing:

    python3 site/art/gen.py

It writes site/assets/place.svg (the poster at the top of the landing page),
the four app icons in site/assets/icons/, one punk in site/assets/sigil.svg,
twelve in site/assets/punks.svg and a mock of the badge's words screen in
site/assets/screen-words.svg. The poster
is flat vector shapes in seven colours with a grain over them; the punks are a
text map turned into rects with crisp edges. The SVGs are committed so the site
needs no build step.
"""
import hashlib
import random
from html import escape
from pathlib import Path

HERE = Path(__file__).parent
ASSETS = HERE.parent / "assets"

# The amber from site/style.css :root, used for the icons and the poster's lights.
AMBER = "#e5a33a"
# The punks and the words screen: ink on the e-paper's warm white.
INK = "#0a0a0b"
PAPER = "#f0dcbd"


def render(map_text, palette_keys, cell, title=None, background=None):
    """An SVG of a text map: one rect per run of same-coloured cells in a row.

    Characters missing from palette_keys are left transparent. With background
    set to a key, that colour fills the whole picture and its runs are skipped.
    Rects are grouped by colour so each fill is written once.
    """
    rows = map_text.strip("\n").split("\n")
    w, h = max(len(r) for r in rows) * cell, len(rows) * cell
    runs = {}
    for y, row in enumerate(rows):
        x = 0
        while x < len(row):
            ch, end = row[x], x
            while end < len(row) and row[end] == ch:
                end += 1
            if ch in palette_keys and ch != background:
                runs.setdefault(ch, []).append(
                    f'<rect x="{x * cell}" y="{y * cell}" width="{(end - x) * cell}" height="{cell}"/>'
                )
            x = end
    out = [
        f'<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 {w} {h}" '
        f'width="{w}" height="{h}" shape-rendering="crispEdges"'
        + (' role="img"' if title else "") + ">"
    ]
    if title:
        out.append(f"<title>{escape(title)}</title>")
    if background:
        out.append(f'<rect width="{w}" height="{h}" fill="{palette_keys[background]}"/>')
    for ch, rects in runs.items():
        out.append(f'<g fill="{palette_keys[ch]}">' + "".join(rects) + "</g>")
    out.append("</svg>")
    return "\n".join(out) + "\n"


# The poster at the top of the landing page: a rooftop terrace at golden hour,
# drawn as flat shapes on a 1600 by 900 canvas with a light grain over it.
# Seven colours, and one gradient for the sky. The sun is low on the right, so
# lit sides face right and shadows fall to the left.
P_SKY = "#3b7dd8"
P_CLOUD = "#f6f1e7"
P_CREAM = "#e9d8b4"
P_TERRA = "#c9693a"
P_GREEN = "#2f5e3e"
P_AMBER = AMBER
P_NAVY = "#1e2f4f"
P_SKY_TOP = "#2f6bc9"
P_SKY_HORIZON = "#7fb0ee"
POSTER_COLOURS = (P_SKY, P_CLOUD, P_CREAM, P_TERRA, P_GREEN, P_AMBER, P_NAVY)
POSTER_W, POSTER_H = 1600, 900
HORIZON = 560   # the far shore of the bay
PARAPET = 630   # the top of the terrace wall
FLOOR = 676     # where the terrace floor meets the wall
TABLE = (800, 728, 222, 52)  # the table top: centre x, centre y, rx, ry


def n(v):
    """A number for SVG: whole numbers without a decimal point, others to one place."""
    v = round(v, 1)
    return str(int(v)) if v == int(v) else str(v)


def pts(points):
    return " ".join(f"{n(x)},{n(y)}" for x, y in points)


def circle(x, y, r, fill, extra=""):
    return f'<circle cx="{n(x)}" cy="{n(y)}" r="{n(r)}" fill="{fill}"{extra}/>'


def ellipse(x, y, rx, ry, fill, extra=""):
    return f'<ellipse cx="{n(x)}" cy="{n(y)}" rx="{n(rx)}" ry="{n(ry)}" fill="{fill}"{extra}/>'


def poly(points, fill, extra=""):
    return f'<polygon points="{pts(points)}" fill="{fill}"{extra}/>'


def path(d, fill, extra=""):
    return f'<path d="{d}" fill="{fill}"{extra}/>'


def line(points, stroke, width, extra=""):
    return (f'<polyline points="{pts(points)}" fill="none" stroke="{stroke}" stroke-width="{n(width)}" '
            f'stroke-linecap="round" stroke-linejoin="round"{extra}/>')


def op(o):
    return f' opacity="{o}"'


def cumulus(rng, profile, base, rows=(1.0, 0.64, 0.34), lobe=(40, 120)):
    """A cumulus cloud built up from rows of round lobes over a flat base.

    profile is a list of (x, top y) points along the cloud's outline. Each row is
    the outline pulled down toward the base; every lobe is a cream disc with a
    smaller white disc set toward the sun, so each one leaves a cream rim on its
    shaded side, and the lower rows leave creases over the rows above them.
    """
    out = []
    xs = [x for x, _ in profile]

    def top(x):
        for (x0, y0), (x1, y1) in zip(profile, profile[1:]):
            if x0 <= x <= x1:
                return y0 + (y1 - y0) * (x - x0) / (x1 - x0)
        return base

    for k, f in enumerate(rows):
        inset = (xs[-1] - xs[0]) * 0.08 * k
        x0, x1 = xs[0] + inset, xs[-1] - inset
        body = [(x0, base)]
        lobes = []
        x = x0
        while x <= x1:
            h = (base - top(x)) * f
            if h > 8:
                r = min(max(lobe[0], h * 0.3), lobe[1]) * rng.uniform(0.75, 1.2)
                lobes.append((x, base - h + r * 0.95, r))
                body.append((x, base - h + r * 1.2))
            x += (lobes[-1][2] if lobes else lobe[0]) * rng.uniform(0.85, 1.15)
        body.append((x1, base))
        out.append(poly(body, P_CLOUD))
        # All the row's cream first, then its white, so a row reads as one mass
        # with a shaded rim, and the creases come between rows.
        out.append(f'<g fill="{P_CREAM}">' + "".join(
            f'<circle cx="{n(cx)}" cy="{n(cy)}" r="{n(r)}"/>' for cx, cy, r in lobes) + "</g>")
        if k == len(rows) - 1 and len(rows) > 2:
            # The lowest row is in the cloud's own shade: cream, cooled by the sky.
            out.append(f'<g fill="{P_SKY}"{op(0.12)}>' + "".join(
                f'<circle cx="{n(cx)}" cy="{n(cy)}" r="{n(r)}"/>' for cx, cy, r in lobes) + "</g>")
        shrink, shift = 0.86 - 0.05 * k, 0.1 + 0.03 * k
        out.append(f'<g fill="{P_CLOUD}">' + "".join(
            f'<circle cx="{n(cx + r * shift)}" cy="{n(cy - r * (shift + 0.02))}" r="{n(r * shrink)}"/>' for cx, cy, r in lobes) + "</g>")
    # The flat underside sits in its own shadow, with the sky showing through it.
    w = xs[-1] - xs[0]
    under = [(xs[0] + w * 0.14, base - 12), (xs[-1] - w * 0.14, base - 12), (xs[-1] - w * 0.2, base), (xs[0] + w * 0.2, base)]
    out.append(poly(under, P_CREAM))
    out.append(poly(under, P_SKY, op(0.2)))
    return out


def sky(rng):
    out = [
        '<defs><linearGradient id="sky" x1="0" y1="0" x2="0" y2="1">'
        f'<stop offset="0" stop-color="{P_SKY_TOP}"/><stop offset="1" stop-color="{P_SKY_HORIZON}"/>'
        '</linearGradient>'
        f'<clipPath id="above"><rect width="{POSTER_W}" height="{HORIZON + 2}"/></clipPath>'
        '<filter id="grain" x="0" y="0" width="100%" height="100%">'
        '<feTurbulence type="fractalNoise" baseFrequency="0.8" numOctaves="3" seed="4" stitchTiles="stitch"/>'
        '<feColorMatrix values="0 0 0 0 0.12 0 0 0 0 0.18 0 0 0 0 0.31 0 0 0 1.6 -0.62"/>'
        '</filter></defs>',
        f'<rect width="{POSTER_W}" height="{PARAPET + 10}" fill="url(#sky)"/>',
    ]
    # The low sun behind the right-hand cloud, and its glow on the haze.
    out.append(circle(900, 506, 140, P_CLOUD, op(0.12)))
    out.append(circle(900, 506, 80, P_CLOUD, op(0.22)))
    out.append(circle(900, 506, 40, P_CLOUD))
    out.append('<g clip-path="url(#above)">')
    # The left tower.
    out += cumulus(rng, [(-120, 300), (-40, 150), (60, 70), (170, 34), (280, 64), (370, 150), (420, 236),
                         (530, 290), (610, 336), (680, 400), (740, 480), (770, 548)], 548)
    # The right tower, the biggest, lit full on by the sun.
    out += cumulus(rng, [(1010, 548), (1020, 470), (1050, 390), (1110, 330), (1180, 280), (1220, 200), (1280, 110), (1350, 50), (1440, 26),
                         (1540, 60), (1620, 120), (1720, 200)], 548)
    # The bank along the horizon, under the headline.
    out += cumulus(rng, [(560, 540), (620, 480), (700, 450), (760, 470), (820, 540)],
                   552, rows=(1.0, 0.5), lobe=(30, 54))
    out.append("</g>")
    return out


def skyline(rng):
    out = []
    # The far hills and the far city, hazy.
    out.append(path(f"M0 {HORIZON} L0 520 Q160 480 330 508 Q470 530 600 512 L600 {HORIZON} Z", P_SKY, op(0.7)))
    out.append(path(f"M1000 {HORIZON} L1000 518 Q1180 476 1360 500 Q1500 516 1600 498 L1600 {HORIZON} Z", P_SKY, op(0.7)))
    far, near = [], []
    x = -10
    while x < POSTER_W:
        w = rng.randint(26, 54)
        h = rng.randint(14, 46)
        far.append((x, w, h))
        x += w - rng.randint(0, 8)
    out.append(f'<g fill="{P_NAVY}"{op(0.42)}>' + "".join(
        f'<rect x="{x}" y="{HORIZON - h - 8}" width="{w}" height="{h + 8}"/>' for x, w, h in far) + "</g>")
    x = -10
    while x < POSTER_W:
        w = rng.randint(30, 76)
        tall = 520 < x < 1100 and rng.random() < 0.35
        h = rng.randint(60, 104) if tall else rng.randint(16, 52)
        near.append((x, w, h))
        x += w + rng.randint(-6, 6)
    g = []
    lit = []
    for x, w, h in near:
        y = HORIZON - h
        g.append(f'<rect x="{x}" y="{y}" width="{w}" height="{h}"/>')
        # The sun catches the right-hand faces and a few windows.
        lit.append(f'<rect x="{x + w - 6}" y="{y}" width="6" height="{h}"/>')
        if h > 40:
            for wy in range(y + 8, HORIZON - 8, 12):
                for wx in range(x + 6, x + w - 12, 10):
                    if rng.random() < 0.18:
                        lit.append(f'<rect x="{wx}" y="{wy}" width="4" height="5"/>')
    # Two landmarks: a spire and a stepped tower.
    g.append(poly([(744, HORIZON), (744, 470), (756, 470), (762, 400), (768, 470), (780, 470), (780, HORIZON)], P_NAVY))
    g.append(poly([(1236, HORIZON), (1236, 488), (1248, 488), (1248, 462), (1262, 462), (1262, 440), (1272, 440),
                   (1272, 462), (1286, 462), (1286, 488), (1298, 488), (1298, HORIZON)], P_NAVY))
    out.append(f'<g fill="{P_NAVY}">' + "".join(g) + "</g>")
    out.append(f'<g fill="{P_AMBER}"{op(0.55)}>' + "".join(lit) + "</g>")
    # The bay: water in the sky's blue, the city's reflection, ripples and the sun's path.
    out.append(f'<rect y="{HORIZON}" width="{POSTER_W}" height="{PARAPET - HORIZON + 4}" fill="{P_SKY}"/>')
    out.append(f'<g fill="{P_NAVY}"{op(0.22)}>' + "".join(
        f'<rect x="{x}" y="{HORIZON}" width="{w}" height="{min(h, 40) * 0.5:.0f}"/>' for x, w, h in near) + "</g>")
    rip = []
    for _ in range(46):
        y = rng.uniform(HORIZON + 6, PARAPET - 4)
        x = rng.uniform(0, POSTER_W)
        w = rng.uniform(20, 70) * (y - HORIZON) / 40
        rip.append(f'<rect x="{x:.0f}" y="{y:.0f}" width="{w:.0f}" height="2" rx="1"/>')
    out.append(f'<g fill="{P_CLOUD}"{op(0.5)}>' + "".join(rip) + "</g>")
    glint = []
    for i in range(8):
        y = HORIZON + 6 + i * 8
        w = 30 + i * 9
        glint.append(f'<rect x="{900 - w / 2:.0f}" y="{y}" width="{w}" height="3" rx="1.5"/>')
    out.append(f'<g fill="{P_AMBER}"{op(0.8)}>' + "".join(glint) + "</g>")
    return out


def terrace():
    out = []
    # The wall, its cap and the sun on its face.
    out.append(f'<rect y="{PARAPET}" width="{POSTER_W}" height="{FLOOR - PARAPET + 2}" fill="{P_CREAM}"/>')
    out.append(f'<rect y="{PARAPET - 6}" width="{POSTER_W}" height="12" fill="{P_CLOUD}"/>')
    out.append(f'<rect y="{PARAPET + 6}" width="{POSTER_W}" height="5" fill="{P_NAVY}"{op(0.12)}/>')
    # The floor: terracotta tiles running to a point on the horizon.
    out.append(f'<rect y="{FLOOR}" width="{POSTER_W}" height="{POSTER_H - FLOOR}" fill="{P_TERRA}"/>')
    vx, vy = 800, 380
    grout = []
    for i in range(-14, 15):
        bx = 800 + i * 150
        t0 = (FLOOR - vy) / (POSTER_H - vy)
        grout.append(f"M{n(vx + (bx - vx) * t0)} {FLOOR}L{n(bx)} {POSTER_H}")
    y = FLOOR
    step = 10
    while y < POSTER_H:
        grout.append(f"M0 {n(y)}H{POSTER_W}")
        step *= 1.38
        y += step
    out.append(f'<path d="{"".join(grout)}" fill="none" stroke="{P_CREAM}" stroke-width="2"{op(0.32)}/>')
    # The sun comes in low from the right and lights a wide band of the floor.
    out.append(poly([(1600, 700), (1600, 900), (420, 900), (760, 700)], P_AMBER, op(0.28)))
    # The wall throws a band of shade onto the near edge of the floor.
    out.append(f'<rect y="{FLOOR}" width="{POSTER_W}" height="10" fill="{P_NAVY}"{op(0.18)}/>')
    return out


def person_back(x, y, jacket, skin, hair, style):
    """A person on the far side of the table, facing us. y is the top of the shoulders."""
    out = []
    if style == "long":
        out.append(path(f"M{x - 30} {y - 34}Q{x - 34} {y + 24} {x - 22} {y + 30}H{x + 22}Q{x + 34} {y + 24} {x + 30} {y - 34}Z", hair))
    if style == "curly":
        for dx, dy, r in [(-22, -48, 17), (0, -60, 19), (22, -48, 17), (-26, -26, 14), (26, -26, 14)]:
            out.append(circle(x + dx, y + dy, r, hair))
    out.append(path(f"M{x - 58} {y + 90}V{y + 34}Q{x - 58} {y} {x - 26} {y - 2}H{x + 26}Q{x + 58} {y} {x + 58} {y + 34}V{y + 90}Z", jacket))
    out.append(path(f"M{x + 20} {y - 2}H{x + 26}Q{x + 58} {y} {x + 58} {y + 34}V{y + 90}H{x + 34}Z", P_AMBER, op(0.22)))
    out.append(f'<rect x="{x - 9}" y="{y - 16}" width="18" height="20" fill="{skin}"/>')
    out.append(ellipse(x, y - 38, 25, 29, skin))
    out.append(path(f"M{x - 25} {y - 38}A25 29 0 0 0 {x} {y - 9}V{y - 67}A25 29 0 0 0 {x - 25} {y - 38}Z", P_NAVY, op(0.07)))
    if style in ("short", "long"):
        out.append(path(f"M{x - 27} {y - 36}Q{x - 30} {y - 72} {x} {y - 70}Q{x + 30} {y - 72} {x + 27} {y - 36}Q{x + 18} {y - 56} {x - 6} {y - 52}Q{x - 20} {y - 50} {x - 27} {y - 36}Z", hair))
    if style == "bun":
        out.append(circle(x + 4, y - 74, 13, hair))
        out.append(path(f"M{x - 26} {y - 38}Q{x - 28} {y - 70} {x} {y - 68}Q{x + 28} {y - 70} {x + 26} {y - 38}Q{x} {y - 58} {x - 26} {y - 38}Z", hair))
    return out


def arms_back(x, y, jacket, skin, reach):
    """Forearms resting on the table, hands toward the middle."""
    out = []
    for side in (-1, 1):
        sx, sy = x + side * 44, y + 22
        ex, ey = x + side * 52, y + 62
        hx, hy = x + side * reach, y + 74
        out.append(line([(sx, sy), (ex, ey), (hx, hy)], jacket, 26))
        out.append(ellipse(hx - side * 4, hy + 1, 11, 8, skin))
    return out


def person_side(x, y, facing, jacket, skin, hair, style, chair):
    """A person at the side of the table, seen side on. y is the top of the shoulders, facing is 1 for right."""
    f = facing
    out = []
    seat = y + 112
    # The chair behind and under them.
    out.append(line([(x - f * 44, y + 6), (x - f * 40, seat), (x - f * 42, 862)], chair, 8))
    out.append(line([(x - f * 44, seat), (x + f * 48, seat)], chair, 10))
    out.append(line([(x + f * 40, seat), (x + f * 44, 862)], chair, 8))
    # Legs: thigh to the knee under the table, shin down to the floor.
    kx = x + f * 92
    out.append(line([(x, seat - 10), (kx, seat - 12)], P_NAVY, 30))
    out.append(line([(kx, seat - 12), (kx + f * 6, 850)], P_NAVY, 24))
    out.append(ellipse(kx + f * 16, 854, 20, 8, P_NAVY))
    # Body.
    out.append(path(f"M{x - 30} {seat}V{y + 30}Q{x - 30} {y} {x} {y}Q{x + 30} {y} {x + 30} {y + 30}V{seat}Z", jacket))
    # The sun is on the right, so the lit edge of the body is its right-hand side.
    out.append(path(f"M{x + 8} {y + 2}Q{x + 30} {y + 4} {x + 30} {y + 30}V{seat}H{x + 8}Z", P_AMBER, op(0.22)))
    # Head, a little forward.
    hx, hy = x + f * 8, y - 36
    if style == "long":
        out.append(path(f"M{hx - f * 2} {hy - 20}Q{hx - f * 34} {hy - 10} {hx - f * 30} {hy + 44}H{hx - f * 4}Z", hair))
    out.append(f'<rect x="{x - 8}" y="{y - 16}" width="16" height="20" fill="{skin}"/>')
    out.append(ellipse(hx, hy, 25, 29, skin))
    out.append(ellipse(hx + f * 22, hy + 4, 6, 8, skin))
    if style == "curly":
        for dx, dy, r in [(-10, -22, 17), (10, -24, 16), (-22, -6, 14)]:
            out.append(circle(hx + f * dx, hy + dy, r, hair))
    else:
        out.append(path(f"M{hx - f * 26} {hy + 6}Q{hx - f * 30} {hy - 34} {hx + f * 4} {hy - 32}Q{hx + f * 28} {hy - 30} {hx + f * 24} {hy - 10}Q{hx + f * 2} {hy - 20} {hx - f * 12} {hy + 8}Z", hair))
    # The near arm, forearm on the table.
    out.append(line([(x + f * 4, y + 24), (x + f * 26, y + 68), (x + f * 84, y + 66)], jacket, 24))
    out.append(ellipse(x + f * 94, y + 66, 11, 8, skin))
    return out


def card(x, y, angle):
    """A card lying on the table: navy, with a white screen and the amber chip."""
    return (f'<g transform="translate({n(x)} {n(y)}) scale(1 0.5) rotate({angle})">'
            f'<rect x="-34" y="-22" width="68" height="44" rx="6" fill="{P_NAVY}"/>'
            f'<rect x="-27" y="-15" width="38" height="30" rx="2" fill="{P_CLOUD}"/>'
            f'<circle cx="22" cy="0" r="5" fill="{P_AMBER}"/></g>')


def table():
    cx, cy, rx, ry = TABLE
    out = []
    # Long shadow to the left, the pedestal, the base.
    out.append(poly([(cx - 30, 864), (cx + 40, 864), (cx - 330, 890), (cx - 470, 884)], P_NAVY, op(0.3)))
    out.append(poly([(cx - 14, cy + 20), (cx + 14, cy + 20), (cx + 18, 852), (cx - 18, 852)], P_NAVY))
    out.append(ellipse(cx, 858, 70, 12, P_NAVY))
    out.append(ellipse(cx, cy + 12, rx, ry, P_CREAM))
    out.append(ellipse(cx, cy, rx, ry, P_CLOUD))
    out.append(path(f"M{cx - rx} {cy}A{rx} {ry} 0 0 0 {cx} {cy + ry}A{rx * 0.8} {ry * 0.8} 0 0 1 {cx - rx} {cy}Z", P_CREAM, op(0.6)))
    return out


def table_things():
    cx, cy = TABLE[0], TABLE[1]
    out = [
        card(cx - 120, cy + 6, -12),
        card(cx - 30, cy + 22, 6),
        card(cx + 70, cy + 4, -4),
        card(cx + 150, cy + 22, 14),
    ]
    # Two glasses of something amber, and a carafe of water.
    for gx, gy in [(cx - 64, cy - 22), (cx + 112, cy - 26)]:
        out.append(f'<rect x="{gx - 9}" y="{gy - 30}" width="18" height="30" rx="3" fill="{P_CLOUD}"/>')
        out.append(f'<rect x="{gx - 7}" y="{gy - 18}" width="14" height="16" rx="2" fill="{P_AMBER}"/>')
        out.append(ellipse(gx, gy, 9, 3, P_NAVY, op(0.25)))
    return out


def planter(x0, x1, top, rng, tree):
    """A cream planter box with a bush in it, and maybe a small tree."""
    out = []
    if tree:
        tx, ty = tree
        out.append(line([(tx, top), (tx - 6, ty + 90), (tx + 4, ty + 30)], P_NAVY, 10))
        for dx, dy, r in [(-60, 40, 46), (0, 0, 56), (58, 34, 48), (-26, 70, 44), (34, 82, 42), (-84, 92, 34), (86, 96, 34)]:
            out.append(circle(tx + dx, ty + dy, r, P_GREEN))
            out.append(circle(tx + dx + r * 0.25, ty + dy - r * 0.2, r * 0.55, P_AMBER, op(0.22)))
    # Bushes: rows of green lobes, lit on the right.
    for row, (dy, rr) in enumerate([(-70, 44), (-34, 50), (0, 46)]):
        x = x0 + 10 + row * 8
        while x < x1 - 10:
            r = rr * rng.uniform(0.8, 1.15)
            out.append(circle(x, top + dy, r, P_GREEN))
            out.append(circle(x + r * 0.3, top + dy - r * 0.28, r * 0.5, P_AMBER, op(0.2)))
            x += r * 1.2
    out.append(f'<rect x="{x0}" y="{top}" width="{x1 - x0}" height="{POSTER_H - top}" fill="{P_CREAM}"/>')
    out.append(f'<rect x="{x0 - 8}" y="{top - 8}" width="{x1 - x0 + 16}" height="16" fill="{P_CLOUD}"/>')
    out.append(f'<rect x="{x0}" y="{top + 8}" width="{x1 - x0}" height="8" fill="{P_NAVY}"{op(0.14)}/>')
    return out


def lights():
    """A string of bulbs across the terrace between two poles."""
    out = []
    poles = [(150, 330), (1450, 330)]
    for px, py in poles:
        out.append(f'<rect x="{px - 5}" y="{py - 6}" width="10" height="{POSTER_H - py}" fill="{P_NAVY}"/>')
    strands = [((150, 336), (1450, 336), 110), ((150, 336), (-40, 292), 30), ((1450, 336), (1640, 286), 30)]
    for (x0, y0), (x1, y1), sag in strands:
        cx, cy = (x0 + x1) / 2, (y0 + y1) / 2 + sag * 2
        out.append(f'<path d="M{x0} {y0}Q{n(cx)} {n(cy)} {x1} {y1}" fill="none" stroke="{P_NAVY}" stroke-width="2.5"/>')
        count = max(3, int(abs(x1 - x0) / 58))
        glow, bulbs, hi = [], [], []
        for i in range(1, count):
            t = i / count
            bx = (1 - t) ** 2 * x0 + 2 * (1 - t) * t * cx + t * t * x1
            by = (1 - t) ** 2 * y0 + 2 * (1 - t) * t * cy + t * t * y1
            glow.append(f'<circle cx="{n(bx)}" cy="{n(by + 12)}" r="17"/>')
            bulbs.append(f'<ellipse cx="{n(bx)}" cy="{n(by + 12)}" rx="6.5" ry="8"/>')
            hi.append(f'<circle cx="{n(bx + 1.5)}" cy="{n(by + 10)}" r="2.6"/>')
            out.append(f'<rect x="{n(bx - 2)}" y="{n(by)}" width="4" height="6" fill="{P_NAVY}"/>')
        out.append(f'<g fill="{P_AMBER}"{op(0.28)}>' + "".join(glow) + "</g>")
        out.append(f'<g fill="{P_AMBER}">' + "".join(bulbs) + "</g>")
        out.append(f'<g fill="{P_CLOUD}">' + "".join(hi) + "</g>")
    return out


def poster():
    rng = random.Random(7)
    cx, cy = TABLE[0], TABLE[1]
    out = [
        f'<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 {POSTER_W} {POSTER_H}" width="{POSTER_W}" '
        f'height="{POSTER_H}" role="img" preserveAspectRatio="xMidYMid slice">',
        "<title>A rooftop at golden hour: five people at a round table, their cards on it, the city behind</title>",
    ]
    out += sky(rng)
    out += skyline(rng)
    out += terrace()
    # Three people on the far side of the table.
    back = [
        (cx - 118, 640, P_GREEN, P_CREAM, P_NAVY, "short", 40),
        (cx, 628, P_AMBER, P_TERRA, P_NAVY, "curly", 30),
        (cx + 118, 640, P_NAVY, P_CREAM, P_TERRA, "long", 40),
    ]
    for x, y, jacket, skin, hair, style, _ in back:
        out += person_back(x, y, jacket, skin, hair, style)
    out += table()
    for x, y, jacket, skin, _, _, reach in back:
        out += arms_back(x, y, jacket, skin, reach)
    out += table_things()
    # Two at the sides.
    out += person_side(cx - 268, 662, 1, P_GREEN, P_CREAM, P_NAVY, "bun", P_CLOUD)
    out += person_side(cx + 268, 662, -1, P_AMBER, P_TERRA, P_CREAM, "short", P_CLOUD)
    out += planter(-20, 250, 760, rng, (120, 520))
    out += planter(1360, 1640, 770, rng, None)
    out += lights()
    out.append(f'<rect width="{POSTER_W}" height="{POSTER_H}" fill="{P_NAVY}" filter="url(#grain)" opacity="0.16"/>')
    out.append("</svg>")
    return "\n".join(out) + "\n"


# The app icons: flat 24 by 24 glyphs, two amber strokes each.
ICONS = {
    # A speech bubble with a line of text in it.
    "chat": ("M5 5h14a2 2 0 0 1 2 2v8a2 2 0 0 1-2 2h-8l-5 4v-4H5a2 2 0 0 1-2-2V7a2 2 0 0 1 2-2z", "M8 11h8"),
    # A key: the bow and the shaft with its bit.
    "login": ("M3 12a4.5 4.5 0 1 0 9 0a4.5 4.5 0 1 0-9 0z", "M12 12h9v3.5M17.5 12v2.5"),
    # A padlock: the shackle and the body.
    "seal": ("M8 10V7a4 4 0 0 1 8 0v3", "M6 10h12a1 1 0 0 1 1 1v8a1 1 0 0 1-1 1H6a1 1 0 0 1-1-1v-8a1 1 0 0 1 1-1z"),
    # A pen, and the signature it leaves.
    "sign": ("M15 4l5 5L10 19H5v-5z", "M3 21c3-2 4 1 7-1s4 1 7-1"),
}


def icon(name):
    first, second = ICONS[name]
    return (
        '<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 24 24" width="24" height="24" fill="none" '
        f'stroke="{AMBER}" stroke-width="2" stroke-linecap="round" stroke-linejoin="round">\n'
        f'<path d="{first}"/>\n<path d="{second}"/>\n</svg>\n'
    )


# The punk: the picture a card shows at rest. A 24 by 24 one-bit face in the
# style of CryptoPunks, turned to the left: the face on the left, the ear and
# the back of the head on the right, the neck under the back. It is built from
# six layers drawn in order, head, hair, accessory, mouth, eyes, hat; each
# layer picks one trait with one byte of SHA-256 of the seed, by weight out of
# 256, so the same seed always gives the same punk.
#
# A sprite is (x, y, rows): rows of "#" ink, "o" paper and "_" nothing, with
# its top left corner at x, y. Head maps also use "s" for skin, which is plain
# paper or a sparse dot pattern. A hat removes the hair on and above its brim.
PUNK_CELL = 8
# The twelve on the landing page, picked from "punk 1" upward so that between
# them they show every hat, hair, eyewear and mouth.
PUNK_SEEDS = [f"punk {i}".encode() for i in (13, 23, 28, 101, 7, 63, 17, 112, 36, 21, 52, 106)]

HEADS = {
    # The common head: flat top, straight face, the jaw cut in under the mouth.
    "square": (0, 0, [
        "________________________",
        "________________________",
        "________________________",
        "________________________",
        "________________________",
        "_______#########________",
        "______#sssssssss#_______",
        "______#sssssssss#_______",
        "______#sssssssss#_______",
        "______#sssssssss#_______",
        "______#sssssssss#_______",
        "______#ssssssssss#______",
        "______#ssssssssss#______",
        "______#ssssssssss#______",
        "______#ssssssssss#______",
        "______#sssssssss#_______",
        "______#sssssssss#_______",
        "______#sssssssss#_______",
        "_______#ssssssss#_______",
        "________#####sss#_______",
        "____________#sss#_______",
        "____________#sss#_______",
        "____________#sss#_______",
        "____________#sss#_______",
    ]),
    # Rounder: the top corners and the jaw cut on a slant.
    "round": (0, 0, [
        "________________________",
        "________________________",
        "________________________",
        "________________________",
        "________________________",
        "________#######_________",
        "_______#sssssss#________",
        "______#sssssssss#_______",
        "______#sssssssss#_______",
        "______#sssssssss#_______",
        "______#sssssssss#_______",
        "______#ssssssssss#______",
        "______#ssssssssss#______",
        "______#ssssssssss#______",
        "______#ssssssssss#______",
        "______#sssssssss#_______",
        "______#sssssssss#_______",
        "_______#ssssssss#_______",
        "________#sssssss#_______",
        "_________####sss#_______",
        "____________#sss#_______",
        "____________#sss#_______",
        "____________#sss#_______",
        "____________#sss#_______",
    ]),
    # Slim: set one cell further back, with a tapered jaw and a thin neck.
    "slim": (0, 0, [
        "________________________",
        "________________________",
        "________________________",
        "________________________",
        "________________________",
        "_________#######________",
        "________#sssssss#_______",
        "_______#sssssssss#______",
        "_______#sssssssss#______",
        "_______#sssssssss#______",
        "_______#sssssssss#______",
        "_______#ssssssssss#_____",
        "_______#ssssssssss#_____",
        "_______#ssssssssss#_____",
        "_______#ssssssssss#_____",
        "_______#sssssssss#______",
        "_______#sssssssss#______",
        "________#ssssssss#______",
        "_________#sssssss#______",
        "__________#####ss#______",
        "______________#ss#______",
        "______________#ss#______",
        "______________#ss#______",
        "______________#ss#______",
    ]),
}

SKINS = {"plain": 160, "dotted": 96}


def skin_ink(x, y):
    """The dotted skin: a checker, which reads as a mid tone."""
    return (x + y) % 2 == 0


def head_sprite(shape):
    x0, y0, rows = HEADS[shape]
    return (x0, y0, [row.replace("s", "o") for row in rows])


def skin_cells(shape):
    x0, y0, rows = HEADS[shape]
    return [(x0 + x, y0 + y) for y, row in enumerate(rows) for x, ch in enumerate(row) if ch == "s"]


HEAD_WEIGHTS = {"square": 128, "round": 64, "slim": 64}

PUNK_LAYERS = {
    "head": {
        "bounds": (0, 0, 24, 24),
        "traits": {},
    },
    "hair": {
        "bounds": (1, 0, 22, 23),
        "traits": {
            "bald": (60, (0, 0, [])),
            "short": (70, (5, 3, [
                "___#######____",
                "__##########__",
                "_############_",
                "#############_",
                "#_#_#__######_",
                "___________###",
                "___________##_",
            ])),
            "mohawk": (40, (10, 0, [
                "_#_",
                "###",
                "###",
                "###",
                "###",
                "###",
            ])),
            "long": (46, (5, 4, [
                "__#########___",
                "_###########__",
                "###_______###_",
                "##________###_",
                "##________####",
                "#_________####",
                "#_________####",
                "__________####",
                "__________####",
                "__________####",
                "___________###",
                "___________###",
                "___________###",
                "___________###",
                "___________##_",
            ])),
            "wild": (40, (3, 1, [
                "___#__#___#__#___",
                "__###_##_###_##__",
                "_###############_",
                "#################",
                "_###############_",
                "##################",
                "_##_#__#_#__######",
                "_#____________###",
                "______________##_",
            ])),
        },
    },
    "accessory": {
        "bounds": (9, 13, 21, 24),
        "traits": {
            "none": (170, (0, 0, [])),
            "earring": (54, (16, 16, [
                "_#_",
                "#o#",
                "_#_",
            ])),
            "chain": (32, (11, 20, [
                "_#####_",
                "#_____#",
                "_#####_",
                "___#___",
            ])),
        },
    },
    "mouth": {
        "bounds": (0, 10, 17, 22),
        "traits": {
            "plain": (80, (8, 17, ["###"])),
            "smile": (44, (8, 16, [
                "___#",
                "###_",
            ])),
            "moustache": (34, (7, 15, [
                "_###_",
                "#####",
            ])),
            "beard": (40, (6, 15, [
                "_______####",
                "###########",
                "##ooo######",
                "###########",
                "_#########_",
                "__#####____",
            ])),
            "pipe": (30, (1, 12, [
                "_#______",
                "#_______",
                "_#______",
                "#_______",
                "________",
                "####_###",
                "#oo#####",
                "#oo#____",
                "_##_____",
            ])),
            "cigarette": (28, (1, 11, [
                "#______",
                "_#_____",
                "#______",
                "_#_____",
                "_______",
                "#######",
                "#ooooo#",
                "#######",
            ])),
        },
    },
    "eyes": {
        "bounds": (5, 9, 19, 18),
        "traits": {
            "plain": (120, (7, 9, [
                "oooooooo",
                "o##oo##o",
                "oooooooo",
                "o#ooo#oo",
                "oooooooo",
                "oooooooo",
            ])),
            "shades": (56, (7, 11, [
                "##########",
                "#o###o####",
                "_###_###__",
            ])),
            "3D glasses": (44, (7, 10, [
                "##########",
                "#ooo#o#o#_",
                "#ooo##o##_",
                "#########_",
            ])),
            "monocle": (36, (8, 10, [
                "___#####",
                "oo_#ooo#",
                "#o_#o#o#",
                "oo_#ooo#",
                "___#####",
                "_______#",
                "________",
                "_______#",
            ])),
        },
    },
    "hat": {
        "bounds": (0, 0, 24, 10),
        "traits": {
            "none": (100, (0, 0, [])),
            "beanie": (34, (6, 1, [
                "____###____",
                "____###____",
                "__#######__",
                "_#########_",
                "###########",
                "#o#o#o#o#o#",
                "###########",
            ])),
            "cap": (36, (1, 3, [
                "_______#########_",
                "______###########",
                "______###########",
                "_################",
                "#################",
            ])),
            "fedora": (28, (4, 2, [
                "_____##_##_____",
                "____#######____",
                "____#######____",
                "____#######____",
                "_#############_",
                "###############",
            ])),
            "top hat": (22, (4, 0, [
                "___#########___",
                "___#########___",
                "___#########___",
                "___#########___",
                "___#########___",
                "___#ooooooo#___",
                "___#########___",
                "_#############_",
            ])),
            "bandana": (22, (5, 4, [
                "__#########___",
                "_#o#o#o#o#o##_",
                "#############_",
                "____________###",
                "____________#_#",
            ])),
            "crown": (14, (7, 1, [
                "#___#___#",
                "##_###_##",
                "#o#o#o#o#",
                "#########",
                "#########",
            ])),
        },
    },
}
PUNK_LAYERS["head"]["traits"] = {
    f"{shape}, {skin}": (HEAD_WEIGHTS[shape] * SKINS[skin] // 256, head_sprite(shape))
    for shape in HEADS
    for skin in SKINS
}

# The row of each hat's brim: hair on this row and above it is not drawn, and
# under a hat only short or long hair shows, at the back of the head from this
# column; a mohawk or wild hair is hidden whole, so nothing spills past the brim.
HAT_BRIM = {"beanie": 7, "cap": 7, "fedora": 7, "top hat": 7, "bandana": 6, "crown": 5}
HAIR_UNDER_HAT_X = 15
HAIR_UNDER_HAT = {"short", "long"}

# How far the eyes and the mouth move on each head, as (dx, dy), so they keep a
# cell of skin between them and the outline. The beard covers the jaw and stays put.
FACE_SHIFT = {
    "square": {"eyes": (0, 0), "mouth": (1, 0)},
    "round": {"eyes": (0, 0), "mouth": (1, -1)},
    "slim": {"eyes": (1, 0), "mouth": (2, -1)},
}
UNSHIFTED = {"beard"}


def layer_pixels(layer, trait, shape):
    """The cells one layer's trait draws on a head of this shape."""
    px = sprite_pixels(PUNK_LAYERS[layer]["traits"][trait][1])
    dx, dy = FACE_SHIFT[shape].get(layer, (0, 0))
    if trait in UNSHIFTED:
        dx = dy = 0
    return [(x + dx, y + dy, ch) for x, y, ch in px]


def sprite_pixels(sprite):
    """The (x, y, ch) of every drawn cell of a sprite, ch being "#" or "o"."""
    x0, y0, rows = sprite
    for dy, row in enumerate(rows):
        for dx, ch in enumerate(row):
            if ch != "_":
                yield x0 + dx, y0 + dy, ch


def pick(traits, byte):
    """The trait whose range of the 256 byte values holds byte."""
    for name, (weight, _) in traits.items():
        if byte < weight:
            return name
        byte -= weight
    raise ValueError("weights do not add up to 256")


def punk_traits(seed):
    digest = hashlib.sha256(seed).digest()
    return {layer: pick(spec["traits"], digest[i]) for i, (layer, spec) in enumerate(PUNK_LAYERS.items())}


def punk_layers(seed):
    """Each layer's cells for this seed, in drawing order, after the hat clips the hair.

    Under a hat only short or long hair below the brim at the back of the head is kept,
    and any hair cell left with no hair beside it is dropped.
    """
    traits = punk_traits(seed)
    shape = traits["head"].split(", ")[0]
    out = []
    for layer in PUNK_LAYERS:
        px = layer_pixels(layer, traits[layer], shape)
        if layer == "hair" and traits["hat"] in HAT_BRIM:
            brim = HAT_BRIM[traits["hat"]]
            keep = traits["hair"] in HAIR_UNDER_HAT
            px = [(x, y, ch) for x, y, ch in px if keep and y > brim and x >= HAIR_UNDER_HAT_X]
        if layer == "hair":
            ink = {(x, y) for x, y, ch in px if ch == "#"}
            px = [(x, y, ch) for x, y, ch in px
                  if any((x + dx, y + dy) in ink for dx in (-1, 0, 1) for dy in (-1, 0, 1) if dx or dy)]
        out.append((layer, px))
    return out


def punk_pixels(seed):
    """The punk as 24 rows of "#" ink and "." paper.

    Dotted skin is a checker laid on the skin once the head is drawn, kept off
    the outline; each later layer first clears the dots around its own ink, so
    every feature sits on a ring of plain paper and stays readable.
    """
    grid = [["."] * 24 for _ in range(24)]
    shape, skin = punk_traits(seed)["head"].split(", ")
    dots = set()

    def near(x, y):
        return [(x + dx, y + dy) for dy in (-1, 0, 1) for dx in (-1, 0, 1)
                if 0 <= x + dx < 24 and 0 <= y + dy < 24]

    for layer, px in punk_layers(seed):
        for x, y, ch in px:
            if ch == "#":
                for cell in near(x, y):
                    if cell in dots:
                        dots.discard(cell)
                        grid[cell[1]][cell[0]] = "."
        for x, y, ch in px:
            grid[y][x] = "#" if ch == "#" else "."
            dots.discard((x, y))
        if layer == "head" and skin == "dotted":
            for x, y in skin_cells(shape):
                if skin_ink(x, y):
                    dots.add((x, y))
            for x, y in dots:
                grid[y][x] = "#"
    return ["".join(r) for r in grid]


def punk(seed):
    """A one-bit punk from SHA-256 of seed: ink on the e-paper's warm white."""
    return render("\n".join(punk_pixels(seed)), {"#": INK, ".": PAPER}, PUNK_CELL, background=".")


# The grid of twelve on the landing page: six by two, a four-cell gap between
# punks and around the edge, drawn as one map at four pixels a cell.
GRID_CELL = 4
GRID_GAP = 4


def punk_grid_size():
    cols, rows = 6, 2
    w = cols * 24 + (cols + 1) * GRID_GAP
    h = rows * 24 + (rows + 1) * GRID_GAP
    return w * GRID_CELL, h * GRID_CELL


def punks(seeds):
    w, h = (v // GRID_CELL for v in punk_grid_size())
    grid = [["."] * w for _ in range(h)]
    for i, seed in enumerate(seeds):
        ox = GRID_GAP + (i % 6) * (24 + GRID_GAP)
        oy = GRID_GAP + (i // 6) * (24 + GRID_GAP)
        for y, row in enumerate(punk_pixels(seed)):
            for x, ch in enumerate(row):
                grid[oy + y][ox + x] = ch
    return render("\n".join("".join(r) for r in grid), {"#": INK, ".": PAPER}, GRID_CELL, background=".")


# The badge's words screen, after words() in crates/badge/src/screens.rs: the
# header with the circle name and the labels, a rule at y 26, and the six words
# in rows 28 pixels apart from y 30. No footer.
MONO = "'JetBrains Mono', ui-monospace, SFMono-Regular, Menlo, monospace"


def words_screen(circle, tier, words):
    paper, ink = PAPER, INK
    out = [
        '<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 200 200" width="200" height="200" '
        f'role="img" font-family="{MONO}">',
        f"<title>The badge showing the six check words of the circle {escape(circle)}</title>",
        f'<rect width="200" height="200" fill="{paper}"/>',
        f'<g fill="{ink}">',
        f'<text x="4" y="17" font-size="13">{escape(circle)}</text>',
        f'<text x="196" y="17" font-size="13" text-anchor="end">{escape(tier)}</text>',
        '<rect x="0" y="26" width="200" height="1"/>',
    ]
    for i, w in enumerate(words):
        out.append(
            f'<text class="word" x="100" y="{30 + 28 * i + 20}" font-size="22" '
            f'font-weight="700" text-anchor="middle">{escape(w)}</text>'
        )
    out += ["</g>", "</svg>"]
    return "\n".join(out) + "\n"


def write(path, svg):
    path.parent.mkdir(parents=True, exist_ok=True)
    path.write_text(svg)
    print(f"wrote {path.relative_to(HERE.parent.parent)} ({len(svg.encode())} bytes)")


def main():
    write(ASSETS / "place.svg", poster())
    for name in ICONS:
        write(ASSETS / "icons" / f"{name}.svg", icon(name))
    write(ASSETS / "sigil.svg", punk(b"cahoots"))
    write(ASSETS / "punks.svg", punks(PUNK_SEEDS))
    # The same twelve for the docs page about the faces.
    write(HERE.parent.parent / "docs" / "diagrams" / "cahoots.svg", punks(PUNK_SEEDS))
    write(
        ASSETS / "screen-words.svg",
        words_screen("rooftop", "TRNG · T1", ["orbit", "maple", "fossil", "drum", "velvet", "harbor"]),
    )


if __name__ == "__main__":
    main()
