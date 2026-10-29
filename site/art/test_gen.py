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


class Scene(unittest.TestCase):
    def setUp(self):
        self.svg = gen.scene()

    def test_scene_size_and_palette(self):
        rows = gen.SCENE.strip("\n").split("\n")
        self.assertEqual(len(rows), 56)
        self.assertTrue(all(len(r) == 96 for r in rows))
        self.assertIn(f'viewBox="0 0 {96 * gen.SCENE_CELL} {56 * gen.SCENE_CELL}"', self.svg)
        pal = gen.palette()
        self.assertEqual(len(pal), 16)
        self.assertLessEqual(fills(self.svg), set(pal.values()))

    def test_scene_under_60k(self):
        self.assertLess(len(self.svg.encode()), 60_000)


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
    def test_icons_one_bit(self):
        amber = gen.palette()["a"]
        for name in gen.ICONS:
            svg = gen.icon(name)
            self.assertIn('viewBox="0 0 16 16"', svg, name)
            self.assertEqual(fills(svg), {amber}, name)
            self.assertTrue(cells(svg, 1), name)


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
