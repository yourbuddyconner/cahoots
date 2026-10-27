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
def mini_badge(s, x, y, w=48, h=54, screen=None, screen_fill=PAPER, pins=True):
    """A small badge seen from the front: shell, e-paper, three pins underneath."""
    s.rect(x, y, w, h, fill=BG2, stroke=LINE, r=6)
    s.rect(x + 6, y + 7, w - 12, h - 22, fill=screen_fill, stroke="none", r=2)
    if screen:
        s.lines(x + w / 2, y + 17, screen, size=6.5, fill=INK, anchor="middle", lh=8, mono=True)
    if pins:
        for i in range(3):
            s.circle(x + w / 2 - 8 + i * 8, y + h - 6, 1.8, fill=AMBER, stroke="none")


def wrap(text, width):
    words, lines, cur = text.split(), [], ""
    for w in words:
        if len(cur) + len(w) + 1 > width:
            lines.append(cur); cur = w
        else:
            cur = (cur + " " + w).strip()
    lines.append(cur)
    return lines


def ceremony():
    steps = [
        ("Set up", "One person names the group and its size. Every screen shows it. Everyone confirms."),
        ("Draw", "Each device makes random bytes and seals a fingerprint of them before it sees anyone else's."),
        ("Swap", "Phones scan each other's codes. Badges pass them up and down the pile. Fingerprints first, then the bytes."),
        ("Mix", "Every device combines all the bytes into one secret. One honest member makes it random."),
        ("Sign", "Every device signs the list of seats with a key it made for this group, and collects the others' signatures."),
        ("Say the words", "Six words appear on every screen. Read them aloud. If they match, you're in cahoots."),
    ]
    W, n, gap, m = 960, len(steps), 14, 24
    cw = (W - 2 * m - gap * (n - 1)) / n  # column width
    rail_y, box_y, box_h = 44, 76, 118
    title_y = box_y + box_h + 34
    body_y = title_y + 24
    s = Svg(W, body_y + 6 * 18 + 40, "The ceremony in six steps")
    # the rail
    s.line(m + cw / 2, rail_y, W - m - cw / 2, rail_y, color=LINE, sw=1.5)
    for i, (name, body) in enumerate(steps):
        x = m + i * (cw + gap)
        cx = x + cw / 2
        # numbered node
        s.circle(cx, rail_y, 13, fill=BG, stroke=AMBER, sw=1.5)
        s.text(cx, rail_y + 4.5, str(i + 1), size=12, fill=AMBER, anchor="middle", mono=True, weight=500)
        # picture box
        s.rect(x, box_y, cw, box_h, fill=BG2, stroke=LINE_SOFT, r=12)
        by = box_y + box_h / 2
        if i == 0:
            mini_badge(s, cx - 24, by - 27, screen=["meetup", "4 of 4"])
            s.text(cx, by + 40, "confirm", size=9.5, fill=FG3, anchor="middle", mono=True)
        elif i == 1:
            mini_badge(s, cx - 24, by - 27, screen=["7f3a91", "c0e2b8"])
            # a seal: small amber lock to the right
            lx, ly = cx + 36, by - 6
            s.rect(lx - 7, ly, 14, 11, fill=AMBER, stroke="none", r=2)
            s.path(f"M{lx - 4} {ly} v-4 a4 4 0 0 1 8 0 v4", stroke=AMBER, sw=1.8)
        elif i == 2:
            # a pile of four cards, each resting on the one below, pads touching
            cw_, ch_, step = 64, 11, 15
            for k in range(4):
                yy = by + 28 - k * step
                xx = cx - cw_ / 2
                s.rect(xx, yy, cw_, ch_, fill=BG2, stroke=LINE, r=3)
                s.rect(xx + 6, yy + 3, 16, 5, fill=PAPER, stroke="none", r=1)
                for j in range(4):
                    s.circle(xx + cw_ - 22 + j * 5, yy + ch_ - 2.5, 1.4, fill=AMBER, stroke="none")
            s.text(cx, by - 40, "pile", size=9.5, fill=FG3, anchor="middle", mono=True)
        elif i == 3:
            # four short bars fold into one
            for k in range(4):
                yy = by - 30 + k * 14
                s.rect(cx - 62, yy, 36, 7, fill=FG3, stroke="none", r=2)
                s.path(f"M{cx - 26} {yy + 3.5} C {cx - 8} {yy + 3.5}, {cx - 8} {by - 3.5}, {cx + 6} {by - 3.5}", stroke=LINE, sw=1)
            s.text(cx - 14, by + 2, "⊕", size=12, fill=FG2, anchor="middle")
            s.rect(cx + 6, by - 7, 52, 14, fill=AMBER, stroke="none", r=3)
            s.text(cx + 32, by + 3.5, "secret", size=8.5, fill=INK, anchor="middle", mono=True, weight=500)
        elif i == 4:
            # the roster, one tick per seat
            for k in range(4):
                yy = by - 26 + k * 15
                s.rect(cx - 34, yy, 40, 8, fill=FG3, stroke="none", r=2)
                s.path(f"M{cx + 14} {yy + 4} l3 3 l6 -7", stroke=AMBER, sw=1.8)
            s.text(cx, by + 46, "signed", size=9.5, fill=FG3, anchor="middle", mono=True)
        else:
            mini_badge(s, cx - 40, by - 34, w=80, h=68, screen=["sunset  valve", "present  raven", "curtain  pudding"])
            s.text(cx, by + 48, "\u201cmatch\u201d", size=9.5, fill=FG3, anchor="middle", mono=True)
        s.text(cx, title_y, name, size=16, fill=FG, anchor="middle", weight=600)
        s.lines(cx, body_y, wrap(body, 22), size=12, fill=FG2, anchor="middle", lh=18)
    # the one thing to remember
    s.text(W / 2, s.h - 20, "Steps 2 and 3 are what stop anyone from choosing their bytes after seeing everyone else\u2019s.", size=12, fill=FG3, anchor="middle")
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
        ("Hardware", [("M0", "Repo", "done"), ("M1", "Core", "done"), ("M1b", "Sealed reveals", "done"), ("M2", "Phone app", "done"), ("M1c", "Seats and manifest", "done"), ("M4a", "Simulator", "done"), ("M4e", "Custodian", "done"), ("M4b", "V1 board", "next"), ("M4f", "Key store", "next"), ("M4h", "Grants from the badge", "next"), ("M4g", "Attested readings", "later"), ("M4d", "Badge over USB", "later"), ("M3", "Hosted seat", "later"), ("M4c", "Badge OS, hosted", "later"), ("M5", "V2 card", "later"), ("M6", "Approvals from a badge", "later"), ("M7", "Meetup kit", "later")]),
        ("Toolkit", [("S1", "File and tunnel keys", "done"), ("S4", "More tunnels", "part"), ("S2", "Approvals", "part"), ("S6", "Grants as web tokens", "later"), ("S7", "Sealed objects", "later"), ("S3", "Recovery", "later"), ("S5", "Hosted keys", "later")]),
        ("Networks", [("N1", "WireGuard", "done"), ("N2", "Control messages", "done"), ("N3", "Headless nodes", "later"), ("N5", "Federation", "later")]),
        ("Chat", [("C1", "Chat crate", "done"), ("C2", "The feed", "done"), ("C3", "CLI demo", "done"), ("C4", "Chat in the app", "done"), ("C6", "Checkpoints", "next"), ("C5", "Hosted feed", "later")]),
        ("Apps", [("A1", "Property demos", "part"), ("A2", "App distribution", "later"), ("A3", "Runtime", "later"), ("A4", "Protocol v2", "later")]),
    ]
    colors = {"done": (AMBER, INK, AMBER), "part": ("rgba(229,163,58,0.18)", AMBER, AMBER), "next": (BG2, FG, FG2), "later": ("none", FG3, LINE)}
    rowh = 58

    def width(k, label):
        return 7.2 * len(label) + 7.2 * len(k) + 30

    # a track whose pills overflow the line wraps onto another; count those lines first so the legend lands below the last one
    extra = 0
    for _, items in tracks:
        x = 120
        for k, label, _ in items:
            w = width(k, label)
            if x + w > 940:
                x = 120
                extra += 1
            x += w + 8
    s = Svg(960, 40 + rowh * len(tracks) + 36 * extra + 36 + 70, "Roadmap by track")
    y = 40
    for name, items in tracks:
        s.text(20, y + 24, name, size=13, fill=FG, weight=600)
        s.line(110, y + 19, 940, y + 19, color=LINE_SOFT)
        x = 120
        y0 = y
        for k, label, st in items:
            fill, fg, stroke = colors[st]
            w = width(k, label)
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


# 7. Pico 2 on the bench --------------------------------------------------------
PICO_LEFT = ["GP0", "GP1", "GND", "GP2", "GP3", "GP4", "GP5", "GND", "GP6", "GP7", "GP8", "GP9", "GND", "GP10", "GP11", "GP12", "GP13", "GND", "GP14", "GP15"]
PICO_RIGHT = ["VBUS", "VSYS", "GND", "3V3_EN", "3V3", "ADC_REF", "GP28", "GND", "GP27", "GP26", "RUN", "GP22", "GND", "GP21", "GP20", "GP19", "GP18", "GND", "GP17", "GP16"]  # pins 40 down to 21


def pico(s, x, y, used, pitch=19):
    """A Pico 2 seen component side up, USB at the top, pin labels inside the outline so wires leave clear of them.
    Returns {pin number: (x, y)} of each pin's pad. `used` maps a pin number to a colour for its pad and label."""
    w, h = 190, 52 + pitch * 20
    s.rect(x, y, w, h, fill=BG2, stroke=LINE, r=6)
    s.rect(x + w / 2 - 14, y - 6, 28, 16, fill=FG3, stroke="none", r=3)  # the USB plug
    s.text(x + w / 2 - 22, y + h - 10, "Pico 2", size=12, fill=FG, anchor="middle", weight=600)
    s.text(x + w / 2 + 26, y + h - 10, "RP2350", size=9, fill=FG3, anchor="middle", mono=True)
    pads = {}
    for i in range(20):
        yy = y + 24 + i * pitch
        for n, px, name, side in ((i + 1, x + 8, PICO_LEFT[i], -1), (40 - i, x + w - 8, PICO_RIGHT[i], 1)):
            c = used.get(n)
            s.circle(px, yy, 3.2, fill=c or BG, stroke=c or LINE, sw=1)
            pads[n] = (px, yy)
            fg = c or (FG3 if name == "GND" else FG2)
            anchor = "start" if side < 0 else "end"
            lx = px + 9 if side < 0 else px - 9
            s.text(lx, yy + 3.5, f"{n}", size=8, fill=FG3, anchor=anchor, mono=True)
            lx2 = lx + 18 if side < 0 else lx - 18
            s.text(lx2, yy + 3.5, name, size=9.5, fill=fg, anchor=anchor, mono=True, weight=500 if c else 400)
    return pads


def wire(s, a, b, color, bend):
    """A wire from pad a to pad b: straight to x = bend, a short diagonal, then straight to b."""
    (x1, y1), (x2, y2) = a, b
    d = 20 if x2 > x1 else -20
    s.path(f"M{x1} {y1} H{bend} L{bend + d} {y2} H{x2}", stroke=color, sw=1.8)


def pico2_wiring():
    """One Pico 2 with the Waveshare module and the two buttons, on V1's pin map."""
    s = Svg(960, 500, "Wiring a Pico 2 for the badge")
    s.text(480, 24, "A Pico 2 on the bench: the Waveshare 1.54″ module and two buttons, on V1’s pin map", size=13, fill=FG, anchor="middle", weight=600)
    SIG, PWR, GNDC = AMBER, PAPER, FG2
    used = {36: PWR, 38: GNDC, 27: SIG, 26: SIG, 25: SIG, 24: SIG, 22: SIG, 21: SIG, 19: SIG, 20: SIG, 18: GNDC}
    bx0 = 390
    pads = pico(s, bx0, 48, used)
    # the module: eight wires, in the order that keeps them from crossing
    mx, my = 790, 70
    s.rect(mx, my, 150, 240, fill=BG2, stroke=LINE, r=8)
    s.text(mx + 75, my + 20, "e-paper module", size=11, fill=FG, anchor="middle", weight=600)
    s.text(mx + 75, my + 34, "Waveshare 1.54″ V2", size=9, fill=FG3, anchor="middle", mono=True)
    header = [("GND", 38, GNDC), ("VCC", 36, PWR), ("BUSY", 27, SIG), ("RST", 26, SIG), ("DIN", 25, SIG), ("CLK", 24, SIG), ("CS", 22, SIG), ("DC", 21, SIG)]
    for i, (name, pin, c) in enumerate(header):
        yy = my + 60 + i * 22
        s.circle(mx, yy, 3.2, fill=c, stroke="none")
        s.text(mx + 10, yy + 3.5, name, size=10, fill=c, mono=True, weight=500)
        wire(s, pads[pin], (mx, yy), c, 690)
    # the buttons, on the rows of pins 19 and 20, each between its pin and ground; one wire takes the ground to pin 18
    sx = 250
    _, y18 = pads[18]
    _, y20 = pads[20]
    for name, pin, py in (("A  left", 19, pads[19][1]), ("B  right", 20, y20 + 34)):
        s.line(sx - 30, py, sx - 10, py, color=SIG, sw=1.8)
        s.line(sx - 10, py, sx + 10, py - 10, color=SIG, sw=1.8)  # the open contact
        s.circle(sx - 10, py, 2.2, fill=BG, stroke=SIG, sw=1.2)
        s.circle(sx + 10, py, 2.2, fill=BG, stroke=SIG, sw=1.2)
        s.line(sx + 10, py, sx + 40, py, color=SIG, sw=1.8)
        wire(s, (sx + 40, py), pads[pin], SIG, 330)
        s.text(sx, py + 16, name, size=10, fill=SIG, anchor="middle", mono=True, weight=500)
    s.line(sx - 30, y18, sx - 30, y20 + 34, color=GNDC, sw=1.8)  # the ground bus joining both switches
    s.line(sx - 30, y18, pads[18][0], y18, color=GNDC, sw=1.8)
    s.text(sx - 38, y18 + 3.5, "GND", size=10, fill=GNDC, anchor="end", mono=True)
    s.lines(20, 70, ["Physical pins count from 1 at the", "USB end, down the left side, then", "up the right side to 40.", "", "Signals: amber. 3.3 V: white.", "Ground: grey. The firmware enables", "the button pull-ups, so a button", "needs no resistor. Any GND pin", "will do for either ground wire."], size=11, fill=FG2, lh=16)
    s.save("pico2-wiring.svg")


def pico2_pile():
    """Three Pico 2s chained over UART as a pile, with the laptop below the bottom one."""
    s = Svg(960, 420, "Three Pico 2s as a pile")
    s.text(480, 24, "A bench pile: each board’s UART0 is its link up and its UART1 its link down, crossed to the next board", size=13, fill=FG, anchor="middle", weight=600)
    UP, DOWN, GNDC = AMBER, PAPER, FG2  # a wire's colour is the direction its data travels
    rows = (("GP0  TX", 1, UP), ("GP1  RX", 2, DOWN), ("GP4  TX", 6, DOWN), ("GP5  RX", 7, UP), ("GND", 3, GNDC))
    ys = (150, 174, 228, 252, 292)
    boards = []
    for i in range(3):
        x = 80 + i * 300
        s.rect(x, 70, 180, 250, fill=BG2, stroke=LINE, r=8)
        s.rect(x + 90 - 14, 64, 28, 14, fill=FG3, stroke="none", r=3)
        s.text(x + 90, 100, f"card {i + 1}", size=13, fill=FG, anchor="middle", weight=600)
        s.text(x + 90, 116, "bottom" if i == 0 else "top" if i == 2 else "middle", size=10, fill=FG3, anchor="middle", mono=True)
        s.text(x + 90, 138, "UART0: link up", size=8.5, fill=FG3, anchor="middle", mono=True)
        s.text(x + 90, 216, "UART1: link down", size=8.5, fill=FG3, anchor="middle", mono=True)
        pins = {}
        for (name, pin, c), yy in zip(rows, ys):
            lx, rx_ = x + 10, x + 170
            s.circle(lx, yy, 3.2, fill=c, stroke="none")
            s.circle(rx_, yy, 3.2, fill=c, stroke="none")
            s.text(x + 90, yy + 3.5, f"{name}  pin {pin}", size=9.5, fill=c, anchor="middle", mono=True)
            pins[pin] = (lx, rx_, yy)
        boards.append(pins)
    for i in range(2):
        a, b = boards[i], boards[i + 1]
        # the lower board's link up meets the upper board's link down: TX to RX each way, ground to ground
        for pa, pb, c in ((1, 7, UP), (2, 6, DOWN), (3, 3, GNDC)):
            (_, x1, y1), (x2, _, y2) = a[pa], b[pb]
            mid = (x1 + x2) / 2
            s.path(f"M{x1} {y1} H{mid - 10} L{mid + 10} {y2} H{x2}", stroke=c, sw=1.8)
    # the laptop below card 1, over USB, standing in for the pile below it
    s.rect(100, 350, 140, 44, fill="none", stroke=LINE, r=6)
    s.text(170, 369, "laptop", size=11, fill=FG, anchor="middle", weight=600)
    s.text(170, 384, "cahoots-host pile --serial", size=8.5, fill=FG3, anchor="middle", mono=True)
    s.line(170, 350, 170, 320, color=FG3, sw=1.8, dash="4 3")
    s.text(182, 338, "USB", size=9, fill=FG3, mono=True)
    s.text(560, 380, "Amber carries data up the pile, white carries it down, grey is ground.", size=11, fill=FG2, anchor="middle")
    s.text(560, 398, "Every board keeps its own USB for power and flashing. Join all the grounds.", size=11, fill=FG2, anchor="middle")
    s.save("pico2-pile.svg")

if __name__ == "__main__":
    ceremony()
    pad()
    modes()
    station()
    tiers()
    roadmap()
    pico2_wiring()
    pico2_pile()
