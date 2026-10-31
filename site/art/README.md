# Site art

`gen.py` draws the site's pictures: the poster at the top of the landing page,
four app icons, a pixel punk for each card, and a mock of the badge's words
screen. It needs only Python 3, no packages.

Run it from the repo root after editing:

    python3 site/art/gen.py

It writes these files. Commit them with the script.

- `site/assets/place.svg`, the poster
- `site/assets/icons/chat.svg`, `login.svg`, `seal.svg` and `sign.svg`
- `site/assets/sigil.svg`, the punk for the seed `b"cahoots"`
- `site/assets/punks.svg`, twelve punks in a grid, from `PUNK_SEEDS`
- `site/assets/screen-words.svg`, the words screen for a circle called rooftop

The poster, `poster()`, is a rooftop terrace at golden hour: big cumulus
clouds over a city and a bay, and five people at a round table with their cards
on it. It is flat vector shapes on a 1600 by 900 canvas in seven colours (sky,
cloud white, cream, terracotta, deep green, amber and skyline blue, listed once
as `POSTER_COLOURS`), one gradient for the sky and a light grain filter over
everything. The sun is low on the right, so lit edges face right and shadows
fall to the left. Each cloud comes from `cumulus()`: an outline of points, built
up in rows of round lobes, each a cream disc under a smaller white one.

The icons are 24 by 24 glyphs, two amber strokes each, in `ICONS`.

A punk is a 24 by 24 one-bit picture, ink (`INK`) on warm white (`PAPER`).
`punk(seed)` hashes the seed with SHA-256 and picks one trait per layer.
`punk_layers(seed)` gives the layers and `punk_pixels(seed)` the finished grid.

The words screen, `words_screen()`, follows `words()` in
`crates/badge/src/screens.rs`: a header, a rule and six words.

After changing the poster, render the Open Graph cards again, which use it as
their background:

    sh site/og/render.sh

Run the tests:

    python3 -m unittest discover -s site/art

(`python3 -m unittest site/art/test_gen.py` does not work, because Python
reads `site` as its own built-in module.)

The journal's small diagrams reuse the same `punk_pixels()` generator. Run
`python3 site/art/journal.py` to regenerate `site/blog/img/circle-and-tools.svg`
and the three `cahoot-alice.svg`, `cahoot-bob.svg`, and `cahoot-casey.svg`
portraits used in the article's responsive diagrams. The portraits are
illustrations, not identity credentials. Commit the generated SVGs with the script.
