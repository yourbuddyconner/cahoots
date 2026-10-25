#!/usr/bin/env python3
"""Renders the blog.

Reads blog/*.md (front matter between --- lines, then markdown) and writes into the
site output directory:

  blog/index.html                 the post list
  blog/<slug>/index.html          one page per post, with its own Open Graph meta
  blog/feed.xml                   RSS
  og/blog-<slug>.html             the Open Graph card source
  assets/og-blog-<slug>.png       the card rendered with headless Chrome

Run from the repository root:

  python3 scripts/build-blog.py --out _site        # in the Pages workflow
  python3 scripts/build-blog.py --out site         # local preview, outputs are gitignored

Needs the `markdown` package and Chrome (or CHROME=/path/to/chrome). --skip-og renders
the pages without the images.
"""
import argparse
import datetime as dt
import html
import os
import re
import shutil
import subprocess
import sys
from email.utils import format_datetime
from pathlib import Path

try:
    import markdown
except ImportError:
    sys.exit("build-blog: the markdown package is missing (pip install markdown)")

ROOT = Path(__file__).resolve().parent.parent
POSTS = ROOT / "blog"
SITE = "https://cahoots.network"
FONTS = "https://fonts.googleapis.com/css2?family=Inter+Tight:wght@500;600&family=Inter:wght@400;500&family=JetBrains+Mono:wght@400;500&display=swap"
GITHUB = "https://github.com/yourbuddyconner/cahoots"


def read_post(path):
    text = path.read_text(encoding="utf-8")
    m = re.match(r"^---\n(.*?)\n---\n(.*)$", text, re.S)
    if not m:
        sys.exit(f"build-blog: {path.name} has no front matter")
    meta = {}
    for line in m.group(1).splitlines():
        k, _, v = line.partition(":")
        meta[k.strip()] = v.strip()
    for key in ("title", "date", "description"):
        if key not in meta:
            sys.exit(f"build-blog: {path.name} is missing {key}")
    date = dt.date.fromisoformat(meta["date"])
    slug = meta.get("slug") or re.sub(r"^\d{4}-\d{2}-\d{2}-", "", path.stem)
    body = markdown.markdown(m.group(2), extensions=["fenced_code", "tables", "smarty"], output_format="html5")
    return {
        "title": meta["title"],
        "date": date,
        "author": meta.get("author", ""),
        "description": meta["description"],
        "slug": slug,
        "body": body,
        "draft": meta.get("draft", "").lower() in ("true", "yes"),
    }


def e(s):
    return html.escape(s, quote=True)


def head(title, description, url, image, article=None):
    out = [
        '<!doctype html>', '<html lang="en">', "<head>", '<meta charset="utf-8">',
        '<meta name="viewport" content="width=device-width, initial-scale=1">',
        f"<title>{e(title)}</title>",
        f'<meta name="description" content="{e(description)}">',
        f'<meta property="og:type" content="{"article" if article else "website"}">',
        '<meta property="og:site_name" content="Cahoots">',
        f'<meta property="og:title" content="{e(title)}">',
        f'<meta property="og:description" content="{e(description)}">',
        f'<meta property="og:url" content="{url}">',
        f'<meta property="og:image" content="{image}">',
        '<meta property="og:image:width" content="1200">',
        '<meta property="og:image:height" content="630">',
        '<meta name="twitter:card" content="summary_large_image">',
        f'<meta name="twitter:title" content="{e(title)}">',
        f'<meta name="twitter:description" content="{e(description)}">',
        f'<meta name="twitter:image" content="{image}">',
    ]
    if article:
        out.append(f'<meta property="article:published_time" content="{article["date"].isoformat()}">')
        if article["author"]:
            out.append(f'<meta property="article:author" content="{e(article["author"])}">')
    out += [
        f'<link rel="canonical" href="{url}">',
        f'<link rel="alternate" type="application/rss+xml" title="Cahoots blog" href="{SITE}/blog/feed.xml">',
        '<link rel="icon" href="/assets/mark.svg" type="image/svg+xml">',
        '<link rel="preconnect" href="https://fonts.googleapis.com">',
        '<link rel="preconnect" href="https://fonts.gstatic.com" crossorigin>',
        f'<link href="{FONTS}" rel="stylesheet">',
        '<link rel="stylesheet" href="/style.css">',
        '<link rel="stylesheet" href="/blog/blog.css">',
        "</head>",
    ]
    return "\n".join(out)


TOP = f"""<header class="top">
  <div class="wrap">
    <a class="brand" href="/"><img src="/assets/mark.svg" alt="" width="22" height="22"><span>Cahoots</span><span class="brand-sub">Blog</span></a>
    <nav aria-label="Primary">
      <a href="/">Home</a>
      <a href="/docs/">Docs</a>
      <a href="{GITHUB}">GitHub</a>
      <a class="nav-cta" href="/app/">Open the app</a>
    </nav>
  </div>
</header>"""

FOOT = f"""<footer>
  <div class="wrap foot">
    <div>
      <a class="brand" href="/"><img src="/assets/mark.svg" alt="" width="22" height="22"><span>Cahoots</span></a>
      <p class="fine"><em>In cahoots</em>: secretly working together. A group that shares a secret is a cahoot. Open source under MIT or Apache-2.0.</p>
    </div>
    <div class="foot-links">
      <a href="/docs/">Docs</a>
      <a href="/blog/">Blog</a>
      <a href="/app/">App</a>
      <a href="{GITHUB}">GitHub</a>
    </div>
  </div>
</footer>"""

BRAND_SUB_CSS = '<style>.brand-sub { color: var(--fg-3); font-weight: 500; margin-left: -2px; } .brand-sub::before { content: "/"; margin-right: 8px; color: var(--line-2); }</style>'


def long_date(d):
    return d.strftime("%B %-d, %Y")


def post_page(p):
    url = f"{SITE}/blog/{p['slug']}/"
    image = f"{SITE}/assets/og-blog-{p['slug']}.png"
    by = []
    if p["author"]:
        by.append(f"<span>{e(p['author'])}</span>")
    by.append(f'<span><time datetime="{p["date"].isoformat()}">{long_date(p["date"])}</time></span>')
    return f"""{head(p['title'], p['description'], url, image, article=p)}
{BRAND_SUB_CSS}
<body class="blog-body">
{TOP}
<main class="blog-wrap">
<article>
  <header class="post-head">
    <p class="eyebrow"><a href="/blog/" style="text-decoration:none;color:inherit">Blog</a></p>
    <h1>{e(p['title'])}</h1>
    <p class="lede">{e(p['description'])}</p>
    <p class="byline">{''.join(by)}</p>
  </header>
  <div class="post">
{p['body']}
  </div>
  <footer class="post-foot">
    <a href="/blog/">← All posts</a>
    <a href="{GITHUB}/blob/main/blog/{p['file']}">Edit this post on GitHub</a>
  </footer>
</article>
</main>
{FOOT}
</body>
</html>
"""


def index_page(posts):
    url = f"{SITE}/blog/"
    image = f"{SITE}/assets/og-blog.png"
    items = []
    for p in posts:
        items.append(f"""    <li><a href="/blog/{p['slug']}/">
      <div class="date"><time datetime="{p['date'].isoformat()}">{long_date(p['date'])}</time></div>
      <h2>{e(p['title'])}</h2>
      <p>{e(p['description'])}</p>
    </a></li>""")
    return f"""{head("Cahoots blog", "Building Cahoots in the open: the protocol, the code, the hardware, and the mistakes.", url, image)}
{BRAND_SUB_CSS}
<body class="blog-body">
{TOP}
<main class="blog-wrap">
  <header class="blog-head">
    <h1>Blog</h1>
    <p>Building Cahoots in the open: the protocol, the code, the hardware, and the mistakes. <a href="/blog/feed.xml">RSS</a></p>
  </header>
  <ul class="post-list">
{chr(10).join(items)}
  </ul>
</main>
{FOOT}
</body>
</html>
"""


def feed(posts):
    items = []
    for p in posts:
        url = f"{SITE}/blog/{p['slug']}/"
        pub = format_datetime(dt.datetime.combine(p["date"], dt.time(12, 0), tzinfo=dt.timezone.utc))
        items.append(f"""  <item>
    <title>{e(p['title'])}</title>
    <link>{url}</link>
    <guid>{url}</guid>
    <pubDate>{pub}</pubDate>
    <description>{e(p['description'])}</description>
    <content:encoded><![CDATA[{p['body']}]]></content:encoded>
  </item>""")
    return f"""<?xml version="1.0" encoding="UTF-8"?>
<rss version="2.0" xmlns:atom="http://www.w3.org/2005/Atom" xmlns:content="http://purl.org/rss/1.0/modules/content/">
<channel>
  <title>Cahoots blog</title>
  <link>{SITE}/blog/</link>
  <atom:link href="{SITE}/blog/feed.xml" rel="self" type="application/rss+xml"/>
  <description>Building Cahoots in the open.</description>
  <language>en</language>
{chr(10).join(items)}
</channel>
</rss>
"""


def og_card(title, lede, foot_left, foot_right, sub="Blog"):
    # Long titles get a smaller face so they stay inside the card.
    size = "sm" if len(title) > 28 else ""
    return f"""<!doctype html><html lang="en"><head><meta charset="utf-8">
<link href="{FONTS}" rel="stylesheet">
<link rel="stylesheet" href="og.css"><style>h1 {{ max-width: 980px; }} .lede {{ max-width: 820px; }}</style></head><body>
<div class="glow"></div>
<div class="card">
  <div class="brand"><img src="../assets/mark.svg" alt="">Cahoots<span class="sub">{e(sub)}</span></div>
  <div>
    <h1 class="{size}">{e(title)}</h1>
    <p class="lede">{e(lede)}</p>
  </div>
  <div class="foot"><span>{e(foot_left)}</span><span>{e(foot_right)}</span></div>
</div>
</body></html>
"""


def find_chrome():
    if os.environ.get("CHROME"):
        return os.environ["CHROME"]
    for c in ("google-chrome", "google-chrome-stable", "chromium", "chromium-browser",
              "/Applications/Google Chrome.app/Contents/MacOS/Google Chrome"):
        if shutil.which(c) or os.path.exists(c):
            return c
    return None


def render_png(chrome, src, dest):
    subprocess.run([chrome, "--headless=new", "--disable-gpu", "--hide-scrollbars", "--no-sandbox",
                    "--window-size=1200,630", "--virtual-time-budget=4000",
                    f"--screenshot={dest}", f"file://{src}"],
                   check=True, stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL)


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("--out", required=True, help="site output directory (the one holding index.html)")
    ap.add_argument("--skip-og", action="store_true", help="do not render the Open Graph images")
    ap.add_argument("--drafts", action="store_true", help="include posts marked draft: true")
    args = ap.parse_args()
    out = Path(args.out).resolve()

    posts = []
    for path in sorted(POSTS.glob("*.md")):
        p = read_post(path)
        p["file"] = path.name
        if p["draft"] and not args.drafts:
            continue
        posts.append(p)
    posts.sort(key=lambda p: p["date"], reverse=True)

    blog = out / "blog"
    blog.mkdir(parents=True, exist_ok=True)
    (out / "og").mkdir(exist_ok=True)
    (out / "assets").mkdir(exist_ok=True)
    css = ROOT / "site" / "blog" / "blog.css"
    if css.resolve() != (blog / "blog.css").resolve():
        shutil.copy(css, blog / "blog.css")
    og_css = ROOT / "site" / "og" / "og.css"
    if og_css.resolve() != (out / "og" / "og.css").resolve():
        shutil.copy(og_css, out / "og" / "og.css")

    (blog / "index.html").write_text(index_page(posts), encoding="utf-8")
    (blog / "feed.xml").write_text(feed(posts), encoding="utf-8")
    cards = [("blog", og_card("Building in the open.", "The protocol, the code, the hardware, and the mistakes.",
                              "cahoots.network/blog", "rss · cahoots.network/blog/feed.xml"))]
    for p in posts:
        d = blog / p["slug"]
        d.mkdir(exist_ok=True)
        (d / "index.html").write_text(post_page(p), encoding="utf-8")
        cards.append((f"blog-{p['slug']}", og_card(p["title"], p["description"],
                                                   f"cahoots.network/blog/{p['slug']}", long_date(p["date"]))))
    for name, src in cards:
        (out / "og" / f"{name}.html").write_text(src, encoding="utf-8")
    print(f"build-blog: {len(posts)} post(s) -> {blog}")

    if args.skip_og:
        return
    chrome = find_chrome()
    if not chrome:
        sys.exit("build-blog: Chrome not found; set CHROME=/path/to/chrome or pass --skip-og")
    for name, _ in cards:
        dest = out / "assets" / f"og-{name}.png"
        render_png(chrome, out / "og" / f"{name}.html", dest)
        print(f"build-blog: wrote {dest.relative_to(out)}")


if __name__ == "__main__":
    main()
