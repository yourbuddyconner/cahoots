#!/usr/bin/env python3
"""Generates the SVG diagrams in this directory. Run it after editing:

    python3 docs/diagrams/gen.py

The SVGs are committed so GitHub and the docs site render them without a build
step. The docs site inlines them so they pick up the page fonts.
"""
from pathlib import Path

HERE = Path(__file__).parent

BG = "#0e0e10"
BG2 = "#151518"
LINE = "rgba(255,255,255,0.14)"
LINE_SOFT = "rgba(255,255,255,0.08)"
FG = "#f4f4f5"
FG2 = "#a1a1aa"
FG3 = "#6b6b74"
AMBER = "#e5a33a"
PAPER = "#e6e2d8"
INK = "#17191b"
SANS = "Inter, -apple-system, 'Segoe UI', Helvetica, Arial, sans-serif"
MONO = "'JetBrains Mono', ui-monospace, SFMono-Regular, Menlo, monospace"


class Svg:
    def __init__(self, w, h, title):
        self.w, self.h = w, h
        self.parts = [
            f'<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 {w} {h}" width="{w}" height="{h}" role="img" aria-labelledby="t">',
            f"<title id=\"t\">{title}</title>",
            "<defs>",
            f'<marker id="arrow" viewBox="0 0 10 10" refX="9" refY="5" markerWidth="7" markerHeight="7" orient="auto-start-reverse"><path d="M0 0L10 5L0 10z" fill="{FG2}"/></marker>',
            f'<marker id="arrow-amber" viewBox="0 0 10 10" refX="9" refY="5" markerWidth="7" markerHeight="7" orient="auto-start-reverse"><path d="M0 0L10 5L0 10z" fill="{AMBER}"/></marker>',
            f'<pattern id="hatch" patternUnits="userSpaceOnUse" width="6" height="6" patternTransform="rotate(45)"><line x1="0" y1="0" x2="0" y2="6" stroke="{FG3}" stroke-width="1.5"/></pattern>',
            "</defs>",
            f'<rect width="{w}" height="{h}" rx="16" fill="{BG}"/>',
        ]

    def rect(self, x, y, w, h, fill=BG2, stroke=LINE, r=10, dash=None, sw=1):
        d = f' stroke-dasharray="{dash}"' if dash else ""
        self.parts.append(f'<rect x="{x}" y="{y}" width="{w}" height="{h}" rx="{r}" fill="{fill}" stroke="{stroke}" stroke-width="{sw}"{d}/>')

    def text(self, x, y, s, size=14, fill=FG, anchor="start", mono=False, weight=400, ls=None):
        fam = MONO if mono else SANS
        extra = f' letter-spacing="{ls}"' if ls else ""
        self.parts.append(
            f'<text x="{x}" y="{y}" font-family="{fam}" font-size="{size}" font-weight="{weight}" fill="{fill}" text-anchor="{anchor}"{extra}>{esc(s)}</text>'
        )

    def lines(self, x, y, items, size=13, fill=FG2, anchor="start", lh=None, mono=False):
        lh = lh or round(size * 1.45)
        for i, s in enumerate(items):
            self.text(x, y + i * lh, s, size=size, fill=fill, anchor=anchor, mono=mono)

    def arrow(self, x1, y1, x2, y2, color=FG2, amber=False, dash=None):
        m = "arrow-amber" if amber else "arrow"
        d = f' stroke-dasharray="{dash}"' if dash else ""
        self.parts.append(f'<line x1="{x1}" y1="{y1}" x2="{x2}" y2="{y2}" stroke="{color}" stroke-width="1.5" marker-end="url(#{m})"{d}/>')

    def line(self, x1, y1, x2, y2, color=LINE, sw=1, dash=None):
        d = f' stroke-dasharray="{dash}"' if dash else ""
        self.parts.append(f'<line x1="{x1}" y1="{y1}" x2="{x2}" y2="{y2}" stroke="{color}" stroke-width="{sw}"{d}/>')

    def path(self, d, stroke=FG2, fill="none", sw=1.5, marker=False, amber=False):
        m = f' marker-end="url(#{"arrow-amber" if amber else "arrow"})"' if marker else ""
        self.parts.append(f'<path d="{d}" stroke="{stroke}" fill="{fill}" stroke-width="{sw}"{m}/>')

    def circle(self, cx, cy, r, fill="none", stroke=LINE, sw=1):
        self.parts.append(f'<circle cx="{cx}" cy="{cy}" r="{r}" fill="{fill}" stroke="{stroke}" stroke-width="{sw}"/>')

    def raw(self, s):
        self.parts.append(s)

    def save(self, name):
        self.parts.append("</svg>")
        (HERE / name).write_text("\n".join(self.parts) + "\n")
        print("wrote", name)


def esc(s):
    return s.replace("&", "&amp;").replace("<", "&lt;").replace(">", "&gt;")


def badge(s, x, y, label, w=58, h=78, words=None):
    """A small badge: shell, screen, three pins."""
    s.rect(x, y, w, h, fill=BG2, stroke=LINE, r=8)
    s.rect(x + 8, y + 10, w - 16, 34, fill=PAPER, stroke="none", r=3)
    if words:
        s.lines(x + w / 2, y + 24, words, size=8, fill=INK, anchor="middle", lh=11, mono=True)
    for i in range(3):
        s.circle(x + w / 2 - 10 + i * 10, y + h - 8, 2.2, fill=AMBER, stroke="none")
    s.text(x + w / 2, y + h + 18, label, size=12, fill=FG2, anchor="middle")


# 1. Ceremony -----------------------------------------------------------------
def ceremony():
    steps = [
        ("Set up", "One person names the group and the member count. Every device shows it. Everyone confirms."),
        ("Draw", "Each device makes its own random bytes and locks them in with a fingerprint, before it sees anyone else's."),
        ("Touch", "Devices connect in a ring. First the fingerprints go around, then the bytes. Nothing uses a radio."),
        ("Mix", "Every device combines all the bytes into one secret. One honest member is enough to make it random."),
        ("Say the words", "Six words appear on every screen. Read them aloud. If they match, the group is in cahoots."),
    ]
    rowh = 92
    top = 28
    s = Svg(960, top + rowh * len(steps) + 44, "The ceremony in five steps")
    x_num, x_name, x_body = 36, 84, 330
    for i, (name, body) in enumerate(steps):
        y = top + i * rowh
        s.rect(20, y, 920, rowh - 12, fill=BG2, stroke=LINE, r=12)
        s.text(x_num, y + 48, f"{i + 1:02d}", size=13, fill=AMBER, mono=True)
        s.text(x_name, y + 49, name, size=20, fill=FG, weight=600)
        # wrap the body to two lines of about 70 characters
        words, lines, cur = body.split(), [], ""
        for w in words:
            if len(cur) + len(w) + 1 > 72:
                lines.append(cur); cur = w
            else:
                cur = (cur + " " + w).strip()
        lines.append(cur)
        s.lines(x_body, y + 36 if len(lines) > 1 else y + 47, lines, size=14.5, fill=FG2, lh=22)
        if i < len(steps) - 1:
            s.arrow(x_num + 8, y + rowh - 12, x_num + 8, y + rowh - 1, color=FG3)
    s.text(480, top + rowh * len(steps) + 22, "Steps 2 and 3 are what stop anyone from choosing their bytes after seeing everyone else's.", size=12.5, fill=FG3, anchor="middle")
    s.save("ceremony.svg")


# 2. Pad and slices -----------------------------------------------------------
def pad():
    s = Svg(960, 330, "The pad: everyone holds all of it, each member sends from their own part")
    s.text(20, 36, "One shared secret, held in full by every member", size=16, fill=FG, weight=600)
    s.text(20, 58, "256 KiB of random bytes. Each byte is used once, then erased on every device.", size=13, fill=FG2)

    x0, y0, W, H = 20, 86, 920, 64
    members = ["Ana", "Ben", "Cy", "Dee"]
    sw = W / len(members)
    s.rect(x0, y0, W, H, fill=BG2, stroke=LINE, r=8)
    for i, m in enumerate(members):
        x = x0 + i * sw
        if i:
            s.line(x, y0, x, y0 + H, color=LINE)
        s.text(x + 12, y0 + 22, f"{m}'s part", size=12.5, fill=FG, weight=500)
        s.text(x + 12, y0 + 42, "only they send from here", size=11, fill=FG3)
    # spent ranges
    spent = [(x0 + 12, 60), (x0 + sw + 12, 28), (x0 + sw + 48, 90), (x0 + 3 * sw + 12, 40)]
    for x, w in spent:
        s.rect(x, y0 + H - 14, w, 8, fill="url(#hatch)", stroke="none", r=2)
    s.rect(x0 + 2 * sw + 12, y0 + H - 14, 120, 8, fill=AMBER, stroke="none", r=2)

    # legend
    ly = y0 + H + 30
    s.rect(20, ly - 7, 22, 8, fill="url(#hatch)", stroke="none", r=2)
    s.text(50, ly + 1, "spent and erased", size=12, fill=FG2)
    s.rect(190, ly - 7, 22, 8, fill=AMBER, stroke="none", r=2)
    s.text(220, ly + 1, "being spent now: Cy is sending a message", size=12, fill=FG2)

    # explanation rows
    rows = [
        ("Everyone holds the whole pad", "so anyone in the group can read any message."),
        ("Each member sends from their own part", "so two people never use the same bytes by accident."),
        ("Every device erases bytes as they are used", "so a device lost later cannot decrypt what was already sent."),
    ]
    y = ly + 44
    for k, v in rows:
        s.circle(26, y - 4, 3, fill=AMBER, stroke="none")
        s.text(40, y, k, size=13, fill=FG, weight=500)
        s.text(380, y, v, size=13, fill=FG2)
        y += 30
    s.save("pad.svg")


# 3. Two modes ----------------------------------------------------------------
def modes():
    s = Svg(960, 360, "Two ways to spend the pad")
    cols = [
        (
            "Short messages",
            "One pad byte per message byte",
            "unbreakable",
            [
                "A message of 60 bytes uses 60 pad bytes plus 32 for the seal.",
                "No computer, now or later, can read it without the pad.",
                "Budget: about 2,000 messages from a 256 KiB pad.",
            ],
            True,
        ),
        (
            "Everything else",
            "32 pad bytes become one session key",
            "hedged",
            [
                "The 32 bytes seed a normal key exchange: WireGuard, files, chat.",
                "An attacker has to break the math and have the pad.",
                "Budget: about 8,000 sessions of any size from the same pad.",
            ],
            False,
        ),
    ]
    for i, (title, sub, tag, body, pure) in enumerate(cols):
        x = 20 + i * 470
        s.rect(x, 24, 450, 300, fill=BG2, stroke=LINE, r=12)
        s.text(x + 20, 56, title, size=18, fill=FG, weight=600)
        s.text(x + 20, 78, sub, size=13, fill=FG2)
        tw = 11 * len(tag) + 20
        s.rect(x + 450 - 20 - tw, 42, tw, 22, fill="none", stroke=AMBER if pure else LINE, r=11)
        s.text(x + 450 - 20 - tw / 2, 57, tag, size=11, fill=AMBER if pure else FG2, anchor="middle", mono=True)
        # little picture
        py = 104
        if pure:
            for j in range(12):
                s.rect(x + 20 + j * 22, py, 18, 18, fill=AMBER, stroke="none", r=3)
                s.rect(x + 20 + j * 22, py + 34, 18, 18, fill=PAPER, stroke="none", r=3)
            s.text(x + 20 + 12 * 22 + 10, py + 13, "pad bytes", size=11, fill=FG3, mono=True)
            s.text(x + 20 + 12 * 22 + 10, py + 47, "message", size=11, fill=FG3, mono=True)
            s.text(x + 20, py + 84, "one for one, then both are gone", size=12, fill=FG3)
        else:
            for j in range(3):
                s.rect(x + 20 + j * 22, py, 18, 18, fill=AMBER, stroke="none", r=3)
            s.text(x + 20 + 76, py + 13, "32 bytes", size=11, fill=FG3, mono=True)
            s.arrow(x + 150, py + 9, x + 200, py + 9, color=FG3)
            s.rect(x + 206, py - 4, 92, 26, fill="none", stroke=LINE, r=13)
            s.text(x + 252, py + 13, "key exchange", size=11, fill=FG2, anchor="middle", mono=True)
            s.arrow(x + 304, py + 9, x + 350, py + 9, color=FG3)
            for j in range(4):
                s.rect(x + 356 + (j % 2) * 22, py - 2 + (j // 2) * 22, 18, 18, fill=PAPER, stroke="none", r=3)
            s.text(x + 356, py + 58, "any amount", size=11, fill=FG3, mono=True)
            s.text(x + 20, py + 84, "the bytes seed a key, then they are gone", size=12, fill=FG3)
        s.lines(x + 20, py + 120, body, size=12.5, fill=FG2, lh=22)
    s.text(480, 346, "Both modes share one pad and one ledger. A group can use both.", size=12, fill=FG3, anchor="middle")
    s.save("modes.svg")


# 4. The feed -----------------------------------------------------------------
def station():
    s = Svg(960, 320, "Anyone can read the feed. Only the group can read the messages.")
    badge(s, 60, 60, "Ana", words=["sunset", "valve"])
    badge(s, 60, 180, "Ben", words=["sunset", "valve"])
    badge(s, 842, 60, "Cy", words=["sunset", "valve"])
    badge(s, 842, 180, "Dee", words=["sunset", "valve"])

    fx, fy, fw, fh = 300, 60, 360, 200
    s.rect(fx, fy, fw, fh, fill=BG2, stroke=LINE, r=12)
    s.text(fx + 18, fy + 28, "The feed", size=15, fill=FG, weight=600)
    s.text(fx + fw - 18, fy + 28, "public · append-only", size=11, fill=FG3, anchor="end", mono=True)
    s.line(fx + 18, fy + 42, fx + fw - 18, fy + 42, color=LINE)
    groups = ["38271 90112 47730 05918", "66204 18837 29951 73406", "02198 54471 88320 61557", "91536 27084 10663 45209"]
    for i, g in enumerate(groups):
        s.text(fx + 18, fy + 70 + i * 26, f"seq {40 + i}", size=11, fill=FG3, mono=True)
        s.text(fx + 90, fy + 70 + i * 26, g, size=12.5, fill=AMBER, mono=True, ls="0.06em")
    s.text(fx + 18, fy + fh - 16, "This is all the operator ever sees.", size=11, fill=FG3)

    for y in (100, 220):
        s.arrow(130, y, fx - 6, y, color=FG2)
        s.arrow(fx + fw + 6, y, 836, y, color=FG2)
    s.text(215, 90, "encrypt, post", size=11, fill=FG3, anchor="middle", mono=True)
    s.text(750, 90, "read, decrypt", size=11, fill=FG3, anchor="middle", mono=True)
    s.text(480, 300, "Anyone can read the feed. Only these four can read the messages, or post as one of them.", size=12, fill=FG3, anchor="middle")
    s.save("station.svg")


# 5. Tiers --------------------------------------------------------------------
def tiers():
    s = Svg(960, 440, "Device tiers: a group is as protected as its least protected device")
    cx, cy = 230, 200
    rings = [
        (170, "T0", "Phone"),
        (138, "T1", "Any board"),
        (106, "T2", "Sealed"),
        (74, "T3", "Key store"),
        (42, "T4", "Inspected"),
    ]
    for r, k, n in rings:
        s.circle(cx, cy, r, fill="none", stroke=AMBER if k == "T4" else LINE, sw=1.5 if k == "T4" else 1)
    s.circle(cx, cy, 42, fill="rgba(229,163,58,0.10)", stroke="none")
    for r, k, n in rings:
        s.text(cx, cy - r + 16, k, size=11, fill=AMBER if k == "T4" else FG3, anchor="middle", mono=True)
    s.text(cx, cy + 4, "T4", size=14, fill=AMBER, anchor="middle", mono=True, weight=500)

    rows = [
        ("T0", "Phone or laptop", ["Runs the protocol correctly. The operating", "system decides how well secrets are wiped."]),
        ("T1", "Any microcontroller", ["Wipes used bytes for real.", "A stolen board still gives up its secret."]),
        ("T2", "Sealed board", ["Locked firmware, encrypted storage, a PIN.", "A copied chip is useless without the PIN."]),
        ("T3", "Hardware key store", ["The chip guards the secret itself.", "Too many wrong PINs and it wipes everything."]),
        ("T4", "Inspected", ["Someone checked the chip under a microscope", "against the public design."]),
    ]
    x = 450
    y = 60
    for k, n, d in rows:
        on = k == "T4"
        s.text(x, y, k, size=12, fill=AMBER if on else FG3, mono=True)
        s.text(x + 40, y, n, size=14, fill=FG, weight=500)
        s.lines(x + 40, y + 20, d, size=12, fill=FG2, lh=17)
        s.line(x, y + 50, 940, y + 50, color=LINE_SOFT)
        y += 66
    s.lines(x, y + 2, ["Four T3 badges and one phone make a T0 group.", "The ceremony shows this before anyone confirms."], size=12, fill=FG3, lh=17)
    s.save("tiers.svg")


# 6. Roadmap ------------------------------------------------------------------
def roadmap():
    tracks = [
        ("Hardware", [("M0", "Repo", "done"), ("M1", "Core", "done"), ("M2", "Phone app", "done"), ("M4a", "Simulator", "done"), ("M4b", "First board", "next"), ("M3", "Hosted seat", "later"), ("M5", "Our badge", "later"), ("M7", "Meetup kit", "later")]),
        ("Toolkit", [("S1", "File and tunnel keys", "done"), ("S4", "More tunnels", "part"), ("S2", "Approvals", "later"), ("S3", "Recovery", "later"), ("S5", "Hosted keys", "later")]),
        ("Networks", [("N1", "WireGuard", "next"), ("N2", "Control messages", "next"), ("N3", "Headless nodes", "later"), ("N4", "Mesh radios", "later"), ("N5", "Federation", "later"), ("N6", "LoRa feed", "later")]),
        ("Chat", [("C1", "Chat crate", "done"), ("C2", "The feed", "done"), ("C3", "CLI demo", "done"), ("C4", "Chat in the app", "done"), ("C5", "Hosted feed", "later")]),
        ("Apps", [("A1", "Property demos", "later"), ("A2", "App distribution", "later"), ("A3", "Runtime", "later"), ("A4", "Protocol v2", "later")]),
    ]
    colors = {"done": (AMBER, INK, AMBER), "part": ("rgba(229,163,58,0.18)", AMBER, AMBER), "next": (BG2, FG, FG2), "later": ("none", FG3, LINE)}
    rowh = 58
    s = Svg(960, 40 + rowh * len(tracks) + 36 + 70, "Roadmap by track")
    y = 40
    for name, items in tracks:
        s.text(20, y + 24, name, size=13, fill=FG, weight=600)
        s.line(110, y + 19, 940, y + 19, color=LINE_SOFT)
        x = 120
        y0 = y
        for k, label, st in items:
            fill, fg, stroke = colors[st]
            w = 7.2 * len(label) + 7.2 * len(k) + 30
            if x + w > 940:
                x = 120
                y += 36
            s.rect(x, y + 6, w, 28, fill=fill, stroke=stroke, r=14)
            s.text(x + 12, y + 24, k, size=11, fill=fg, mono=True, weight=500)
            s.text(x + 12 + 7.2 * len(k) + 6, y + 24, label, size=12.5, fill=fg)
            x += w + 8
        y += rowh
    # legend
    ly = y + 14
    lx = 20
    for st, label in (("done", "done"), ("part", "partly done"), ("next", "up next"), ("later", "later")):
        fill, fg, stroke = colors[st]
        s.rect(lx, ly - 10, 14, 14, fill=fill, stroke=stroke, r=7)
        s.text(lx + 22, ly + 1, label, size=12, fill=FG2)
        lx += 7.2 * len(label) + 50
    s.save("roadmap.svg")


# 7. DIY badge wiring -------------------------------------------------------------
PICO_LEFT = ["GP0", "GP1", "GND", "GP2", "GP3", "GP4", "GP5", "GND", "GP6", "GP7", "GP8", "GP9", "GND", "GP10", "GP11", "GP12", "GP13", "GND", "GP14", "GP15"]
PICO_RIGHT = ["VBUS", "VSYS", "GND", "3V3_EN", "3V3", "ADC_VREF", "GP28", "GND", "GP27", "GP26", "RUN", "GP22", "GND", "GP21", "GP20", "GP19", "GP18", "GND", "GP17", "GP16"]
WIRES = [  # (side, index from the USB end, label on the breakout, colour, Qwiic wire colour name)
    ("right", 2, "GND", "#8a8f96", "black"),
    ("right", 4, "3V3", "#e06c5c", "red"),
    ("left", 5, "SDA", "#5c9ee0", "blue"),
    ("left", 6, "SCL", "#e5c75c", "yellow"),
]


def wiring():
    s = Svg(960, 700, "Wiring the secure element to the Pico: four wires into the header beside the Pico's pins")
    bx, by, bw, bh = 250, 50, 180, 440
    s.rect(bx, by, bw, bh, fill="#1b4d3e", stroke=LINE, r=10)
    s.rect(bx + bw / 2 - 26, by - 10, 52, 28, fill="#9aa0a6", stroke="none", r=4)
    s.text(bx + bw / 2, by + 8, "USB", size=10, fill=INK, anchor="middle", mono=True)
    s.text(bx + bw / 2, by + bh / 2, "Pico 2", size=16, fill=FG, anchor="middle", weight=600)
    s.text(bx + bw / 2, by + bh / 2 + 18, "USB at the top", size=10.5, fill="#bcd", anchor="middle")
    pitch = (bh - 50) / 19
    pin_y = lambda i: by + 32 + i * pitch
    for i in range(20):
        y = pin_y(i)
        for side, names, x, anchor, lx in (("left", PICO_LEFT, bx, "end", bx - 12), ("right", PICO_RIGHT, bx + bw, "start", bx + bw + 12)):
            used = next((w for w in WIRES if w[0] == side and w[1] == i), None)
            s.circle(x, y, 4.5, fill=used[3] if used else "#d4af37", stroke="none")
            s.text(lx, y + 4, names[i], size=10.5, fill=used[3] if used else FG3, anchor=anchor, mono=True, weight=600 if used else 400)
            s.text(bx + (16 if side == "left" else bw - 16), y + 3.5, str(i + 1 if side == "left" else 40 - i), size=8, fill="#8fb", anchor="middle", mono=True)
    # Breakout below the Pico, where it sits in the stack
    kw, kh = 170, 90
    kx, ky = bx + bw / 2 - kw / 2, by + bh + 70
    s.rect(kx, ky, kw, kh, fill="#1f2a44", stroke=LINE, r=8)
    s.text(kx + kw / 2, ky + 54, "Trust M breakout", size=12, fill=FG, anchor="middle", weight=600)
    s.text(kx + kw / 2, ky + 72, "Qwiic / STEMMA QT port on top", size=10, fill=FG2, anchor="middle")
    port_x0 = kx + kw / 2 - 24
    s.rect(port_x0 - 6, ky - 4, 60, 22, fill="#333", stroke=LINE, r=3)
    # Wires leave the port in its pin order and run up the outside of each side's labels
    for n, (side, i, label, colour, cname) in enumerate(WIRES):
        px = port_x0 + n * 16
        y = pin_y(i)
        x_end = bx - 52 if side == "left" else bx + bw + 52   # just outside the pin label, which is drawn in the wire's colour
        x_out = bx - 95 if side == "left" else bx + bw + 95
        d = f"M{px} {ky - 4} L{px} {ky - 30 - n * 6} C {px} {ky - 60 - n * 6}, {x_out} {ky - 60 - n * 6}, {x_out} {ky - 80} L{x_out} {y + 14} Q {x_out} {y}, {x_out + (12 if side == 'left' else -12)} {y} L{x_end} {y}"
        s.path(d, stroke=colour, sw=3)
    s.text(port_x0 + 24, ky + 34, "black · red · blue · yellow", size=9, fill=FG3, anchor="middle", mono=True)
    s.text(560, 90, "Four wires, in the order the port gives them.", size=14, fill=FG, weight=600)
    s.lines(560, 116, [
        "Count pins from the USB end. Each wire goes into the",
        "hat's female header right beside the Pico pin, so it",
        "shares the pin without soldering.",
        "",
        "GND  → right side, 3rd pin (pin 38)",
        "3V3  → right side, 5th pin (pin 36). Never the 4th:",
        "        that is 3V3_EN and holds the board in reset.",
        "SDA  → left side, 6th pin (pin 6, GP4, I2C0 SDA)",
        "SCL  → left side, 7th pin (pin 7, GP5, I2C0 SCL)",
        "",
        "Trust the order of the wires in the plug, not their",
        "colours. Cables from different makers differ.",
        "",
        "The screen hat uses SPI1 (GP8 to GP13) and the",
        "buttons use GP2, GP3, GP15 to GP21. Nothing",
        "collides with GP4 and GP5.",
    ], size=12, fill=FG2, lh=19)
    s.save("diy-wiring.svg")


def stack():
    s = Svg(960, 300, "The stack: screen hat on top, secure element wedged between, Pico below")
    cx = 330
    layers = [("Screen hat (Waveshare Pico-LCD-1.3)", 60, "#2a2f3a", "faces out: 240 × 240 screen, joystick, four buttons"),
              ("Trust M breakout + cable", 118, "#1f2a44", "lies flat between the boards, port toward the USB end"),
              ("Pico 2 with headers", 176, "#1b4d3e", "pins up into the hat's header; USB at the end")]
    for name, y, fill, note in layers:
        s.rect(cx - 220, y, 440, 40, fill=fill, stroke=LINE, r=8)
        s.text(cx, y + 25, name, size=13, fill=FG, anchor="middle", weight=600)
        s.text(cx + 250, y + 25, note, size=12, fill=FG2)
    for x in (cx - 190, cx + 190):
        s.line(x, 100, x, 176, color="#d4af37", sw=2)
    s.text(cx - 200, 148, "header", size=10, fill="#d4af37", anchor="end", mono=True)
    s.rect(cx - 240, 40, 480, 196, fill="none", stroke=LINE_SOFT, r=14, dash="6 6")
    s.text(cx, 262, "A snap case holds the three together. Nothing is glued or soldered.", size=12, fill=FG3, anchor="middle")
    s.save("diy-stack.svg")


if __name__ == "__main__":
    ceremony()
    pad()
    modes()
    station()
    tiers()
    roadmap()
    wiring()
    stack()
