# Site art

`gen.py` draws the site's pictures: the poster at the top of the landing page,
the five app icons, a sigil and a mock of the badge's words screen. It needs
only Python 3, no packages.

Run it from the repo root after editing:

    python3 site/art/gen.py

It writes `site/assets/place.svg`, `site/assets/icons/*.svg`,
`site/assets/sigil.svg` and `site/assets/screen-words.svg`. Commit the SVGs
with the script.

The poster, `poster()`, is a rooftop terrace at golden hour: big cumulus
clouds over a city and a bay, and five people at a round table with their cards
on it. It is flat vector shapes on a 1600 by 900 canvas in seven colours (sky,
cloud white, cream, terracotta, deep green, amber and skyline blue, listed once
as `POSTER_COLOURS`), one gradient for the sky and a light grain filter over
everything. The sun is low on the right, so lit edges face right and shadows
fall to the left. Each cloud comes from `cumulus()`: an outline of points, built
up in rows of round lobes, each a cream disc under a smaller white one.

The icons are 24 by 24 glyphs, two amber strokes each, in `ICONS`.

The sigil is a text map, one character per cell, in two colours from the
pixel palette in `palette()`.

After changing the poster, render the Open Graph cards again, which use it as
their background:

    sh site/og/render.sh

Run the tests:

    python3 -m unittest discover -s site/art

(`python3 -m unittest site/art/test_gen.py` does not work, because Python
reads `site` as its own built-in module.)
