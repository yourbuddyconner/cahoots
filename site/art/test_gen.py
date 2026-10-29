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


class Sigil(unittest.TestCase):
    def grid(self, seed):
        svg = gen.sigil(seed)
        ink = cells(svg, gen.SIGIL_CELL)
        return svg, ink

    def test_sigil_symmetric_and_bounded_ink(self):
        for i in range(40):
            _, ink = self.grid(f"seed {i}".encode())
            for x, y in ink:
                self.assertIn((23 - x, y), ink)
                self.assertTrue(2 <= x < 22 and 2 <= y < 22, (x, y))
            share = len(ink) / (20 * 20)
            self.assertGreaterEqual(share, 0.30)
            self.assertLessEqual(share, 0.70)

    def test_sigil_deterministic(self):
        self.assertEqual(gen.sigil(b"a"), gen.sigil(b"a"))
        self.assertNotEqual(gen.sigil(b"a"), gen.sigil(b"b"))


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
        svg = gen.words_screen("back room", "TRNG · T1", words)
        self.assertIn('viewBox="0 0 200 200"', svg)
        self.assertIn(">back room<", svg)
        self.assertIn(">TRNG · T1<", svg)
        rows = re.findall(r'<text class="word"[^>]* y="(\d+)"[^>]*>([^<]+)<', svg)
        self.assertEqual([w for _, w in rows], words)
        self.assertEqual(len({y for y, _ in rows}), 6)
        self.assertEqual(len(fills(svg)), 2)


if __name__ == "__main__":
    unittest.main()
