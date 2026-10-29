#!/usr/bin/env python3
"""Draws the site's pixel art. Run it after editing:

    python3 site/art/gen.py

It writes site/assets/place.svg (the back room on the landing page), the five
app icons in site/assets/icons/, a sigil in site/assets/sigil.svg and a mock of
the badge's words screen in site/assets/screen-words.svg. Each picture is a
text map, one character per cell, turned into rects with crisp edges. The
SVGs are committed so the site needs no build step.
"""
import hashlib
from html import escape
from pathlib import Path

HERE = Path(__file__).parent
ASSETS = HERE.parent / "assets"

# The theme tokens from site/style.css :root.
BG = "#0a0a0b"
BG2 = "#101012"
FG = "#f4f4f5"
FG2 = "#a1a1aa"
FG3 = "#6b6b74"
AMBER = "#e5a33a"
# The two hues the spec adds: a cold blue for the night outside, a green for one screen.
NIGHT = "#2a4a86"
GREEN = "#6fcf7c"


def mix(a, b, t):
    """The colour a fraction t of the way from hex a to hex b."""
    ca = [int(a[i:i + 2], 16) for i in (1, 3, 5)]
    cb = [int(b[i:i + 2], 16) for i in (1, 3, 5)]
    return "#" + "".join(f"{round(x + (y - x) * t):02x}" for x, y in zip(ca, cb))


def palette():
    """The sixteen colours, by the character that stands for each in a map."""
    return {
        ".": BG,                    # the darkest shadow, black hair, the e-paper's ink
        ",": BG2,                   # deep shadow, the far buildings
        "-": mix(BG2, FG3, 0.22),   # the wall away from the lamp, dark floorboards
        "=": mix(BG2, FG3, 0.5),    # light floorboards, the window and door frames
        "3": FG3,                   # a grey jacket, a bottle
        "2": FG2,                   # grey hair, a light jacket, the window sill
        "W": FG,                    # the screens, the bulb, the stars, white hair
        "c": mix(FG, AMBER, 0.3),   # warm white: a cream jumper, the moon, the e-paper
        "a": AMBER,                 # the cards, lit windows, ginger hair
        "A": mix(AMBER, FG, 0.45),  # the cards' faces, faces in the light, light under the door
        "o": mix(AMBER, BG, 0.45),  # the table top, a brown jacket, faces in shade
        "u": mix(AMBER, BG, 0.72),  # the door, the chairs, the lamp shade, dark wood
        "h": mix(BG2, AMBER, 0.16), # the wall in the lamp's light
        "b": mix(BG, NIGHT, 0.55),  # the night sky, a blue jacket's shade
        "B": NIGHT,                 # a blue jacket, the sky over the city
        "g": GREEN,                 # the phone's screen
    }


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


# The back room, 96 by 56 cells. The lamp hangs over the table in the middle;
# the door is at the far left, the window and the shelf at the right.
SCENE_CELL = 10
SCENE = """
,,--------------------------h-h-hhhhhhhhhhhhhhh.hhhhhhhhhhhhhhhhh-h-h-------------------------,,
,,---------------------------h-hhhhhhhhhhhhhhhh.hhhhhhhhhhhhhhhh-h----------------------------,,
,,,,,,,,,,,,,,,,,,,,,,,,,,u,u,uuuuuuuuuuuuuuuuu.uuuuuuuuuuuuuuuuu,u,u,u,,,,,,,,,,,,,,,,,,,,,,,,,
,,-------------------------h-hhhhhhhhhhhhhhhhhh.hhhhhhhhhhhhhhh========================-------,,
,,----------------------h-h-hhhhhhhhhhhhhhhhhhh.hhhhhhhhhhhhhhh=bbbbbbbbbb==bbbbbbbbbb=-----b-,,
,,-==============----------h-hhhhhhhhhhhhhhhhhh.hhhhhhhhhhhhhhh=bbbbbbbbbb==bbbbbWbbbb=-----b-,,
,,-=ouuuuuuuuuuu=-------h-h-hhhhhhhhhhhhhhhhhhh.hhhhhhhhhhhhhhh=bbWbbbbbbb==bbcWbbbbbb=--u--b-,3
,,-=o,,,,,,,,,,u=----------hhhhhhhhhhhhhhhhhhh.ouuhhhhhhhhhhhhh=bbbbbbbbbb==bbccbbbbbb=--u-BBB,3
,,-=o,uuuuuuuu,u=-----uuuuuuuuuuhhhhhhhhhhhhh.ouuuuhhhhhhhhhhhh=bbbbbbbWbb==bbbbbbbbbb=-oooB2B33
,,-=o,uuuuuuuu,u=-----u,,,,,,,,uhhhhhhhhhhhh.ouuuuuuhhhhhhhhhhh=bbbbbbbbbb==bbbbbbbbWb=-oaoBBB32
,,-=o,uuuuuuuu,u=-----u,,,,,c,,uhhhhhhhhhhh.ouuuuuuuuhhhhhhhhhh=bbbbWbbbbb==bbbbbbbbbb=-oooBBB33
,,-=o,uuuuuuuu,u=-----u,,,3,,,,uhhhhhhhhhh.ouuuuuuuuuuhhhhhhhhh=bbbbbbbbbb==bbbbbbbbbb=-33333333
,,-=o,uuuuuuuu,u=-----u,,333,3,uhhhhhhhhhhaaaaaaaaaaaahhhhhhhhh========================-,,,,,,,,
,,-=o,uuuuuuuu,u=-----u,33333-3uhhhhhhhhhhhhhhcWWchhhhhhhhhhhhh=bBbBbBbBbB==,BbBbBbBbB=---u---,,
,,-=o,uuuuuuuu,u=-----u--------uhhhhhhhhhhhhhhhhhhhhhhhhhhhhhhh=BbB,,,BbBb==,bBbB,,,Bb=---u---,,
,,-=o,uuuuuuuu,u=-----uuuuuuuuuuhhhhhhhhhhhhhhhhhhhhhhhhhhhhhhh=BBB,a,BBBB==,BBBB,a,BB=---u--o,,
,,-=o,,,,,,,,,,u=-------h-h-hhhhhhhhhhhhhhhhhhhhhhhhhhhhhhhhhhh=,,,,,,BBBB==,,,,,,,,BB=--uuu-o,,
,,-=ouuuuuuuuuuu=----------h-hhhhhhhhhhhhhhhhhhhhhhhhhhhhhhhhhh=,,,,,,,,,,==,,,,,,,,,,=--uouooo,
,,-=ouuuuuuuuoau=-------h-h-hhhhhhhhhhhhhhhhhhhhhhhhhhhhhhhhhhh=,a,,a,,,,,==,,a,,,c,,,=--uuuoAo,
,,-=o,,,,,,,,,,u=----------h-hhhhhhhhhhhhhhhhhhhhhhhhhhhhhhhWWh=,,,,,,,a,,==a,,,,,,,,a=--uuuooo,
,,-=o,uuuuuuuu,u=---------h-h-hhhhhhhhhhhhhhhh....hhhhhhhhhhWWh=,,,,,,,,,,==,,,,,,,,,,=-33333333
,,-=o,uuuuuuuu,u=------------h-hhaaaahhhhhhhh......hhhhhhhhWWWW========================-,,,,,,,,
,,-=o,uuuuuuuu,u=-2222------h-h-aaaaaahhhhhhh.AAAA.hhhhhhhWWWWWW2222222222uuuu2222222222------,,
,,-=o,uuuuuuuu,u=222222--------haAAAAahhhhhhh..AA..hhhhhhhWooooW,,,,,,,,,uuuuuu,,,,,,,,,------,,
,,-=o,uuuuuuuu,u=222AAA-----h-h-a.AA.ahhhhhhhhAAAohhhhhhhhW.oo.Wh-h------AAAuuu---------------,,
,,-=o,uuuuuuuu,u=22AA.A---------aAAAoahhhhhhhhAAAohhhhhhhhhooou----------A.AAuu---------------,,
,,-=o,uuuuuuuu,u=22AAAAA------h-aAAAoahhhhhhhhhAAhhhhhhhhhhooou-h-------AAAAAuu---------------,,
,,-=o,uuuuuuuu,u=-2AAAA--------a--AA-hah-hhhhh..c..hhhhh-h-hoo-----------AAAAu----------------,,
,,-=o,,,,,,,,,,u=--AAo---------a-33c33a-h-h-....c...,-h-h-hccccc----------oAA-----------------,,
,,-=ouuuuuuuuuuu=BBBB----------a333c33a=-h-.....c...,,---cccccccc2---------oooo---------------,,
,,,=ouuuuuuuuuuuBBBBBB,,,,,,,,3a333c33a==,,.........,,,,ccccccccc22,,,,,,,oooooo,,,,,,,,,,,,,,,,
...=AAAAAAAAAAuABBBBBBB.......333333333==....,.....,.,..ccccccccc22......ooooooo.u..............
.,,,aaaaaaaaaauaBBBBbBBB,,,,,,33=33333=3=,,..,.....,.,,,cc2ccccc2c2,,,,,ooouoooo,u,,,,,,,,,,,,,.
.,,ooooooooooouoBBBBbbBBA,,,,,33=33333=3=,,..,.....,.,,,cc2ccccc2c2,,,,Aoouuoooo,u,,,,,,,,,,,,,.
--uuuuuuuuuuuuuuBBBBBb--------33=33333=3=------u--------cc2ccccc2c2-------uooooo-u----,---------
-u-u-u-u-u-u-uuuBBBBBB-----------uuuuuuuuuuuuuuuuuuuuuuuuuuuuu-,----------oooooo-u----,---------
-----------,--u-BBBBBB-----ooo333AoooA333oo...AoooA...oocccoooooccco------oooooo-u----------,---
-----------,--u-BBBBBB--oooooo==AoooooA==oo,,AoooooA,,oo22ooooooo22oooo---oooooo-u----------,---
-----------,--u-======oooooooooooooooooaaaoogooooooooooaaaooooooooooooooo,,,,,,,-u----------,---
=====,========u=======ooooooooooooooooaaaaao,oooooooooaaaaaoooooooooooooo,,,,,,,=u==============
=====,========uuuuuuuuoooooooooooooooaaAWAaaoooaaaoooaaAWAaaooooooooooooouuuuuuuuu==============
=====,========u=======ooooooooooooooooaaaaaoooaaaaaoooaaaaaoooooooooooooo========u==============
--------------u-------uuoooooooooooooooaaaoooaaAWAaaoooaaaooooooooooooouu--------u----------,---
--------------u-------,,uuuoooooooooooooooooooaaaaaooooooooooooooooouuu,,--------u----------,---
--------------u-------,=,,,uuuuuuooooooooooooooaaaoooooooooooouuuuuu,,,,,--------u----------,---
--------------u-------,==--,,,,,,uuuuuuuuuuuuuuouuuuuuuuuuuuuu,,,,,,---,,--------u----------,---
==============u=,================,,,,,,,,,,,,,,u,,,,,,,,,,,,,,=========,,========u============,=
==============u=,============================uu,u,,====,===============,,========u============,=
==============u=,============================uuuu,,====,===============,,========u============,=
==============u=,=====....===================uuuu,,====,==============....=======u============,=
----------,---u--------u----------------,,,,,uuuu,,,,,,-----------------u--------u--------------
----------,---u--------u-------,,,,,,,,,,uuuuuuuuuuuuuu,,,,,,,,,--------u--------u--------------
----------,-----------------,,,,,,,,,,,,,,,,,,,,,,,,,,,,,,,,,,,,,,,-----------------------------
----------,--------------------,,,,,,,,,,,,,,,,,,,,,,,,,,,,,,,,,--------------------------------
----------,-----------------------------,,,,,,,,,,,,,,,-----------------------------------------
----------,------------------------------------------,------------------------------------------
"""


def scene():
    return render(
        SCENE,
        palette(),
        SCENE_CELL,
        title="A back room late at night: five people round a table, their cards glowing amber under a hanging lamp.",
        background="-",
    )


# The app icons, 16 by 16, one bit: "#" is ink.
ICONS = {
    "chat": """
................
.#########......
.#.......#......
.#.......#......
.#...##########.
.#...#........#.
.#####........#.
...#.#........#.
...#.#........#.
.....#........#.
.....##########.
...........#....
...........##...
...........#.#..
................
................
""",
    "login": """
................
................
................
................
..####..........
.#....#.........
#..##..#........
#.#..#.#########
#.#..#.#########
#..##..#....#.##
.#....#.....#.##
..####..........
................
................
................
................
""",
    "tunnel": """
................
................
................
................
................
................
.####......####.
.#..#......#..#.
.#..########..#.
.#..#......#..#.
.####......####.
................
................
................
................
................
""",
    "seal": """
................
.....######.....
....#......#....
...#........#...
...#........#...
...#........#...
.##############.
.#............#.
.#.....##.....#.
.#.....##.....#.
.#......#.....#.
.#......#.....#.
.#............#.
.##############.
................
................
""",
    "sign": """
................
.............##.
............#..#
...........#..#.
..........#..#..
.........#..#...
........#..#....
.......#..#.....
......###.......
......##........
.....#..........
................
.#...#..........
..#.#...........
...#............
................
""",
}


def icon(name):
    return render(ICONS[name], {"#": palette()["a"]}, 1)


# The sigil: 24 by 24 cells with a 2-cell border, drawn on a 10 by 10 grid of
# 2-cell blocks and mirrored left to right, so it reads as a mark, not noise.
SIGIL_CELL = 8


def sigil(seed):
    """A one-bit mark from SHA-256 of seed, the same seed giving the same mark."""
    digest = hashlib.sha256(seed).digest()
    while True:
        bits = int.from_bytes(digest, "big")
        half = [[(bits >> (y * 5 + x)) & 1 for x in range(5)] for y in range(10)]
        ink = sum(map(sum, half)) / 50
        if 0.3 <= ink <= 0.7:
            break
        digest = hashlib.sha256(digest).digest()
    rows = []
    for by in range(12):
        blocks = [0] * 12
        if 1 <= by <= 10:
            left = half[by - 1]
            blocks[1:11] = left + left[::-1]
        row = "".join("#" * 2 if b else "." * 2 for b in blocks)
        rows += [row, row]
    pal = palette()
    return render("\n".join(rows), {"#": pal["."], ".": pal["c"]}, SIGIL_CELL, background=".")


# The badge's words screen, after words() in crates/badge/src/screens.rs: the
# header with the circle name and the labels, a rule at y 26, and the six words
# in rows 28 pixels apart from y 30. No footer.
MONO = "'JetBrains Mono', ui-monospace, SFMono-Regular, Menlo, monospace"


def words_screen(circle, tier, words):
    pal = palette()
    paper, ink = pal["c"], pal["."]
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
    write(ASSETS / "place.svg", scene())
    for name in ICONS:
        write(ASSETS / "icons" / f"{name}.svg", icon(name))
    write(ASSETS / "sigil.svg", sigil(b"cahoots"))
    write(
        ASSETS / "screen-words.svg",
        words_screen("back room", "TRNG · T1", ["orbit", "maple", "fossil", "drum", "velvet", "harbor"]),
    )


if __name__ == "__main__":
    main()
