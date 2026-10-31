#!/usr/bin/env python3
"""Generate the journal's cahoot portraits and circle diagram using the site's face generator.

Run from the repository root: python3 site/art/journal.py
"""
from html import escape
from pathlib import Path
from gen import INK, PAPER, PUNK_SEEDS, punk_pixels

OUT = Path(__file__).resolve().parents[1] / 'blog' / 'img'


def face(seed):
    # One-cell paper margin, as in the app. Runs keep the committed SVGs small.
    runs = []
    for y, row in enumerate(punk_pixels(seed)):
        x = 0
        while x < len(row):
            if row[x] != '#':
                x += 1
                continue
            end = x + 1
            while end < len(row) and row[end] == '#':
                end += 1
            runs.append(f'M{x + 1} {y + 1}h{end - x}v1h-{end - x}z')
            x = end
    return f'<rect width="26" height="26" fill="{PAPER}"/><path fill="{INK}" d="{"".join(runs)}"/>'


def main():
    OUT.mkdir(parents=True, exist_ok=True)
    portraits = [face(seed) for seed in PUNK_SEEDS[:3]]
    for name, portrait in zip(('alice', 'bob', 'casey'), portraits):
        title = escape(f'Cahoot portrait for {name.title()}, an illustrative member')
        (OUT / f'cahoot-{name}.svg').write_text(
            f'<svg xmlns="http://www.w3.org/2000/svg" width="78" height="78" viewBox="0 0 26 26" shape-rendering="crispEdges" role="img"><title>{title}</title>{portrait}</svg>\n')
    people = ''.join(f'<svg x="{x}" y="54" width="64" height="64" viewBox="0 0 26 26" shape-rendering="crispEdges">{portrait}</svg>'
                     for x, portrait in zip((168, 288, 408), portraits))
    (OUT / 'circle-and-tools.svg').write_text(f'''<svg xmlns="http://www.w3.org/2000/svg" width="640" height="480" viewBox="0 0 640 480" role="img" aria-labelledby="title desc">
  <title id="title">A circle the group can carry between tools</title>
  <desc id="desc">Three cahoot portraits represent people meeting in person. Their circle records its members and shared secrets, then supplies separate keys for chat, files, and private connections. The portraits are illustrations, not identity credentials.</desc>
  <rect width="640" height="480" rx="4" fill="#f6f6f7"/>
  <g font-family="Arial, Helvetica, sans-serif" text-anchor="middle">
    <text x="320" y="34" fill="#50525a" font-size="22">Meet in person</text>
    {people}
    <g fill="none" stroke="#babdc6" stroke-width="2" stroke-dasharray="4 5">
      <path d="M200 130l54 38"/><path d="M320 130v38"/><path d="M440 130l-54 38"/>
    </g>
    <rect x="150" y="174" width="340" height="106" rx="53" fill="#fff" stroke="#304ed1" stroke-width="2"/>
    <text x="320" y="218" fill="#17181b" font-size="30" font-family="Georgia, 'Times New Roman', serif">Our circle</text>
    <text x="320" y="250" fill="#50525a" font-size="21">Members + shared secrets</text>
    <g fill="none" stroke="#304ed1" stroke-width="2" stroke-linecap="round" stroke-linejoin="round">
      <path d="M320 281v76"/><path d="M116 357v-35h408v35"/>
    </g>
    <text x="320" y="306" fill="#50525a" font-size="20" paint-order="stroke" stroke="#f6f6f7" stroke-width="9">Separate keys for each use</text>
    <g fill="#fff" stroke="#e0e1e5" stroke-width="1.5">
      <rect x="24" y="358" width="184" height="70" rx="4"/><rect x="228" y="358" width="184" height="70" rx="4"/><rect x="432" y="358" width="184" height="70" rx="4"/>
    </g>
    <g fill="#17181b" font-size="24"><text x="116" y="400">Chat</text><text x="320" y="400">Files</text><text x="524" y="400">Connections</text></g>
    <text x="320" y="463" fill="#50525a" font-size="21">The tools can change. The circle stays ours.</text>
  </g>
</svg>\n''')


if __name__ == '__main__':
    main()
