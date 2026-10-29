"""Tests for the art script. Run from the repo root:

    python3 -m unittest discover -s site/art
"""
import re
import sys
import unittest
from pathlib import Path

sys.path.insert(0, str(Path(__file__).parent))
import gen  # noqa: E402

FILL = re.compile(r'fill="(#[0-9a-f]{6})"')
HEX = re.compile(r"#[0-9a-fA-F]{6}\b")
STOP = re.compile(r'stop-color="(#[0-9a-f]{6})"')
RECT = re.compile(r'<rect x="(\d+)" y="(\d+)" width="(\d+)" height="(\d+)"')


def fills(svg):
    return set(FILL.findall(svg))


def cells(svg, cell):
    """The set of (x, y) cells covered by rects, in cell units."""
    out = set()
    for x, y, w, h in RECT.findall(svg):
        x, y, w, h = (int(v) // cell for v in (x, y, w, h))
        for dy in range(h):
            for dx in range(w):
                out.add((x + dx, y + dy))
    return out


class Render(unittest.TestCase):
    def test_render_merges_runs(self):
        svg = gen.render("aab", {"a": "#111111", "b": "#222222"}, 4)
        self.assertEqual(len(RECT.findall(svg)), 2)
        self.assertIn('shape-rendering="crispEdges"', svg)
        self.assertIn('viewBox="0 0 12 4"', svg)


class Poster(unittest.TestCase):
    def setUp(self):
        self.svg = gen.poster()

    def test_poster_size(self):
        self.assertIn('viewBox="0 0 1600 900"', self.svg)
        self.assertLess(len(self.svg.encode()), 120_000)

    def test_poster_colours(self):
        """Only the seven palette colours, plus the two stops of the sky gradient."""
        allowed = set(gen.POSTER_COLOURS) | {gen.P_SKY_TOP, gen.P_SKY_HORIZON}
        self.assertEqual(len(gen.POSTER_COLOURS), 7)
        self.assertLessEqual(set(HEX.findall(self.svg)), allowed)
        self.assertEqual(set(STOP.findall(self.svg)), {gen.P_SKY_TOP, gen.P_SKY_HORIZON})
        self.assertEqual(self.svg.count("<linearGradient"), 1)
        self.assertNotIn("<radialGradient", self.svg)

    def test_poster_grain(self):
        self.assertIn("<feTurbulence", self.svg)
        self.assertIn('filter="url(#grain)"', self.svg)

    def test_poster_title(self):
        self.assertIn(
            "<title>A rooftop at golden hour: five people at a round table, their cards on it, the city behind</title>",
            self.svg,
        )


class Punk(unittest.TestCase):
    def test_punk_deterministic(self):
        self.assertEqual(gen.punk(b"a"), gen.punk(b"a"))
        self.assertEqual(gen.punk_traits(b"a"), gen.punk_traits(b"a"))

    def test_punk_is_24_by_24(self):
        for i in range(40):
            seed = f"seed {i}".encode()
            rows = gen.punk_pixels(seed)
            self.assertEqual(len(rows), 24)
            self.assertTrue(all(len(r) == 24 and set(r) <= {"#", "."} for r in rows))
            svg = gen.punk(seed)
            side = 24 * gen.PUNK_CELL
            self.assertIn(f'viewBox="0 0 {side} {side}"', svg)
            self.assertIn('shape-rendering="crispEdges"', svg)
            ink = cells(svg, gen.PUNK_CELL)
            self.assertTrue(all(0 <= x < 24 and 0 <= y < 24 for x, y in ink))
            self.assertEqual(ink, {(x, y) for y, r in enumerate(rows) for x, c in enumerate(r) if c == "#"})

    def test_punk_one_bit(self):
        self.assertEqual(fills(gen.punk(b"cahoots")), {gen.INK, gen.PAPER})

    def test_each_layer_draws_within_its_bounds(self):
        self.assertEqual(
            list(gen.PUNK_LAYERS), ["head", "hair", "accessory", "mouth", "eyes", "hat"]
        )
        for layer, spec in gen.PUNK_LAYERS.items():
            x0, y0, x1, y1 = spec["bounds"]
            self.assertTrue(0 <= x0 < x1 <= 24 and 0 <= y0 < y1 <= 24, layer)
            self.assertEqual(sum(w for w, _ in spec["traits"].values()), 256, layer)
            self.assertGreaterEqual(len(spec["traits"]), 2, layer)
            for name in spec["traits"]:
                for shape in gen.HEADS:
                    for x, y, ch in gen.layer_pixels(layer, name, shape):
                        self.assertIn(ch, "#o", (layer, name))
                        self.assertTrue(x0 <= x < x1 and y0 <= y < y1, (layer, name, shape, x, y))

    def test_eyes_and_mouth_keep_clear_of_the_outline(self):
        """Plain eyes and the plain, smiling and moustached mouths keep a paper
        cell between their ink and the head's outline, and never draw on it."""
        for shape in gen.HEADS:
            outline = {(x, y) for x, y, ch in gen.sprite_pixels(gen.head_sprite(shape)) if ch == "#"}
            for layer, name in [("eyes", "plain"), ("mouth", "plain"), ("mouth", "smile"), ("mouth", "moustache")]:
                for x, y, ch in gen.layer_pixels(layer, name, shape):
                    self.assertNotIn((x, y), outline, (shape, layer, name, x, y))
                    if ch == "#":
                        near = {(x + dx, y + dy) for dx in (-1, 0, 1) for dy in (-1, 0, 1)}
                        self.assertFalse(near & outline, (shape, layer, name, x, y))

    def test_dotted_skin_leaves_the_eye_band_clear(self):
        for i in range(300):
            seed = f"seed {i}".encode()
            t = gen.punk_traits(seed)
            shape, skin = t["head"].split(", ")
            if skin != "dotted" or t["eyes"] != "plain":
                continue
            rows = gen.punk_pixels(seed)
            dx = gen.FACE_SHIFT[shape]["eyes"][0]
            for y in (9, 11, 13, 14):
                self.assertEqual(rows[y][7 + dx:15 + dx], "." * 8, (seed, y))

    def test_hat_replaces_the_top_of_the_hair(self):
        for i in range(300):
            seed = f"seed {i}".encode()
            layers = dict(gen.punk_layers(seed))
            traits = gen.punk_traits(seed)
            brim = gen.HAT_BRIM.get(traits["hat"])
            if brim is not None:
                if traits["hair"] not in gen.HAIR_UNDER_HAT:
                    self.assertEqual(layers["hair"], [], traits)
                for x, y, _ in layers["hair"]:
                    self.assertTrue(y > brim and x >= gen.HAIR_UNDER_HAT_X, (traits, x, y))

    def test_no_stray_hair_pixels(self):
        for i in range(300):
            hair = {(x, y) for x, y, ch in dict(gen.punk_layers(f"seed {i}".encode()))["hair"] if ch == "#"}
            for x, y in hair:
                near = {(x + dx, y + dy) for dx in (-1, 0, 1) for dy in (-1, 0, 1)} - {(x, y)}
                self.assertTrue(near & hair, (i, x, y))

    def test_twelve_seeds_twelve_punks(self):
        self.assertEqual(len(gen.PUNK_SEEDS), 12)
        images = {"".join(gen.punk_pixels(s)) for s in gen.PUNK_SEEDS}
        self.assertEqual(len(images), 12)

    def test_punk_grid_is_six_by_two(self):
        svg = gen.punks(gen.PUNK_SEEDS)
        w, h = gen.punk_grid_size()
        self.assertIn(f'viewBox="0 0 {w} {h}"', svg)
        self.assertGreater(w, h * 2)
        self.assertIn('shape-rendering="crispEdges"', svg)
        self.assertEqual(fills(svg), {gen.INK, gen.PAPER})


class Icons(unittest.TestCase):
    def test_icons_two_amber_strokes(self):
        for name in gen.ICONS:
            svg = gen.icon(name)
            self.assertIn('viewBox="0 0 24 24"', svg, name)
            self.assertEqual(svg.count("<path "), 2, name)
            self.assertEqual(set(HEX.findall(svg)), {gen.AMBER}, name)
            self.assertIn(f'stroke="{gen.AMBER}"', svg, name)
            self.assertIn('fill="none"', svg, name)


class WordsScreen(unittest.TestCase):
    def test_words_screen_has_six_rows(self):
        words = ["orbit", "maple", "fossil", "drum", "velvet", "harbor"]
        svg = gen.words_screen("rooftop", "TRNG · T1", words)
        self.assertIn('viewBox="0 0 200 200"', svg)
        self.assertIn(">rooftop<", svg)
        self.assertIn(">TRNG · T1<", svg)
        rows = re.findall(r'<text class="word"[^>]* y="(\d+)"[^>]*>([^<]+)<', svg)
        self.assertEqual([w for _, w in rows], words)
        self.assertEqual(len({y for y, _ in rows}), 6)
        self.assertEqual(len(fills(svg)), 2)


if __name__ == "__main__":
    unittest.main()
