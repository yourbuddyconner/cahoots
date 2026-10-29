# Site art

`gen.py` draws the site's pixel art from text maps: the back room at the top of
the landing page, the five app icons, a sigil and a mock of the badge's words
screen. It needs only Python 3, no packages.

Run it from the repo root after editing a map:

    python3 site/art/gen.py

It writes `site/assets/place.svg`, `site/assets/icons/*.svg`,
`site/assets/sigil.svg` and `site/assets/screen-words.svg`. Commit the SVGs
with the script.

Each map is a block of text, one character per cell. In the back room, each
character is one of the sixteen colours in `palette()`, which are built from
the theme colours in `site/style.css`. In the icons, `#` is ink and `.` is
empty.

Run the tests:

    python3 -m unittest discover -s site/art

(`python3 -m unittest site/art/test_gen.py` does not work, because Python
reads `site` as its own built-in module.)
