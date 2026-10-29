#!/usr/bin/env python3
"""Renders the blog.

Reads blog/*.md (front matter between --- lines, then markdown) and writes into the
site output directory:

  blog/index.html                 the post list
  blog/<slug>/index.html          one page per post, with its own Open Graph meta
  blog/<slug>.md                  the post's source, for "View as Markdown"
  blog/feed.xml                   RSS
  og/blog-<slug>.html             the Open Graph card source
  assets/og-blog-<slug>.png       the card rendered with headless Chrome
  assets/hero-blog-<slug>.png     a 1600x800 picture for the top of the post, unless the
                                  front matter names an `image`

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
    key = None
    for line in m.group(1).splitlines():
        if line.startswith("- ") and key:
            meta.setdefault(key, [])
            if isinstance(meta[key], list):
                meta[key].append(line[2:].strip())
            continue
        k, _, v = line.partition(":")
        key = k.strip()
        meta[key] = v.strip() if v.strip() else []
    for key in ("title", "date", "description"):
        if key not in meta:
            sys.exit(f"build-blog: {path.name} is missing {key}")
    date = dt.date.fromisoformat(meta["date"])
    slug = meta.get("slug") or re.sub(r"^\d{4}-\d{2}-\d{2}-", "", path.stem)
    source = m.group(2)
    body = markdown.markdown(source, extensions=["fenced_code", "tables", "smarty", "toc"], output_format="html5")
    # The in-article contents: h2 headings, which the toc extension has given ids.
    toc = [(i, re.sub(r"<[^>]+>", "", t)) for i, t in re.findall(r'<h2 id="([^"]+)">(.*?)</h2>', body)]
    words = len(re.findall(r"\w+", source))
    tags = meta.get("tags", [])
    if isinstance(tags, str):
        tags = [t.strip() for t in tags.split(",") if t.strip()]
    learn = meta.get("learn", [])
    return {
        "title": meta["title"],
        "date": date,
        "author": meta.get("author", ""),
        "role": meta.get("role", ""),
        "description": meta["description"],
        "about": meta.get("about", ""),
        "audience": meta.get("audience", ""),
        "learn": learn if isinstance(learn, list) else [learn],
        "tags": tags,
        "image": meta.get("image", ""),
        "slug": slug,
        "body": body,
        "source": text,
        "toc": toc,
        "minutes": max(1, round(words / 230)),
        "draft": str(meta.get("draft", "")).lower() in ("true", "yes"),
    }


def e(s):
    return html.escape(s, quote=True)


CSS_V = ""  # set in main() to a hash of blog.css, so browsers never keep a stale copy


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
        f'<link rel="alternate" type="application/rss+xml" title="Cahoots journal" href="{SITE}/blog/feed.xml">',
        '<link rel="icon" href="/assets/mark.svg" type="image/svg+xml">',
        '<link rel="stylesheet" href="/style.css">',
        f'<link rel="stylesheet" href="/blog/blog.css?v={CSS_V}">',
        '<link rel="stylesheet" href="/brand.css">',
        "</head>",
    ]
    return "\n".join(out)


TOP = f"""<header class="top">
  <div class="wrap">
    <a class="brand" href="/"><img src="/assets/mark.svg" alt="" width="22" height="22"><span>cahoots</span><span class="brand-sub">Journal</span></a>
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
      <a class="brand" href="/"><img src="/assets/mark.svg" alt="" width="22" height="22"><span>cahoots</span></a>
      <p class="fine">Made for people who trust each other. Open source under MIT or Apache-2.0.</p>
    </div>
    <div class="foot-links">
      <a href="/docs/">Docs</a>
      <a href="/blog/">Journal</a>
      <a href="/app/">App</a>
      <a href="{GITHUB}">GitHub</a>
    </div>
  </div>
</footer>"""

BRAND_SUB_CSS = '<style>.brand-sub { color: var(--fg-3); font-weight: 500; margin-left: -2px; } .brand-sub::before { content: "/"; margin-right: 8px; color: var(--line-2); }</style>'


def long_date(d):
    return d.strftime("%B %-d, %Y")


ICONS = {
    "x": '<svg viewBox="0 0 24 24" width="16" height="16" fill="currentColor" aria-hidden="true"><path d="M18.2 2h3.3l-7.2 8.3L22.8 22h-6.6l-5.2-6.8L5 22H1.7l7.7-8.8L1.2 2H8l4.7 6.2L18.2 2zm-1.2 18h1.8L7.1 3.9H5.2L17 20z"/></svg>',
    "linkedin": '<svg viewBox="0 0 24 24" width="16" height="16" fill="currentColor" aria-hidden="true"><path d="M20.4 2H3.6C2.7 2 2 2.7 2 3.6v16.8c0 .9.7 1.6 1.6 1.6h16.8c.9 0 1.6-.7 1.6-1.6V3.6c0-.9-.7-1.6-1.6-1.6zM8 19H5V9h3v10zM6.5 7.7a1.7 1.7 0 1 1 0-3.4 1.7 1.7 0 0 1 0 3.4zM19 19h-3v-4.9c0-1.2 0-2.7-1.6-2.7s-1.9 1.3-1.9 2.6V19h-3V9h2.9v1.4h.1c.4-.8 1.4-1.6 2.8-1.6 3 0 3.6 2 3.6 4.6V19z"/></svg>',
    "hn": '<svg viewBox="0 0 24 24" width="16" height="16" fill="currentColor" fill-rule="evenodd" aria-hidden="true"><path d="M2 2h20v20H2V2zm9 11.3L7.2 6h2.1l2.7 5.3L14.7 6h2.1L13 13.3V18h-2v-4.7z"/></svg>',
    "link": '<svg viewBox="0 0 24 24" width="16" height="16" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" aria-hidden="true"><path d="M10 13a5 5 0 0 0 7.5.5l3-3a5 5 0 0 0-7-7l-1.7 1.7"/><path d="M14 11a5 5 0 0 0-7.5-.5l-3 3a5 5 0 0 0 7 7l1.7-1.7"/></svg>',
    "rss": '<svg viewBox="0 0 24 24" width="16" height="16" fill="currentColor" aria-hidden="true"><circle cx="5" cy="19" r="2"/><path d="M3 10a11 11 0 0 1 11 11h-3a8 8 0 0 0-8-8v-3zm0-7a18 18 0 0 1 18 18h-3A15 15 0 0 0 3 6V3z"/></svg>',
    "github": '<svg viewBox="0 0 24 24" width="16" height="16" fill="currentColor" aria-hidden="true"><path d="M12 .5A11.5 11.5 0 0 0 8.4 22.9c.6.1.8-.3.8-.6v-2c-3.2.7-3.9-1.5-3.9-1.5-.5-1.3-1.3-1.7-1.3-1.7-1-.7.1-.7.1-.7 1.2.1 1.8 1.2 1.8 1.2 1 1.8 2.7 1.3 3.4 1 .1-.8.4-1.3.7-1.6-2.6-.3-5.3-1.3-5.3-5.7 0-1.3.5-2.3 1.2-3.1-.1-.3-.5-1.5.1-3.1 0 0 1-.3 3.2 1.2a11 11 0 0 1 5.8 0c2.2-1.5 3.2-1.2 3.2-1.2.6 1.6.2 2.8.1 3.1.8.8 1.2 1.8 1.2 3.1 0 4.4-2.7 5.4-5.3 5.7.4.4.8 1.1.8 2.2v3.2c0 .3.2.7.8.6A11.5 11.5 0 0 0 12 .5z"/></svg>',
    "back": '<svg viewBox="0 0 24 24" width="14" height="14" fill="none" stroke="currentColor" stroke-width="2.2" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true"><path d="M15 5l-7 7 7 7"/></svg>',
    "cal": '<svg viewBox="0 0 24 24" width="15" height="15" fill="none" stroke="currentColor" stroke-width="1.8" stroke-linecap="round" aria-hidden="true"><rect x="3" y="5" width="18" height="16" rx="2"/><path d="M3 10h18M8 3v4M16 3v4"/></svg>',
    "md": '<svg viewBox="0 0 24 24" width="15" height="15" fill="none" stroke="currentColor" stroke-width="1.8" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true"><rect x="2" y="5" width="20" height="14" rx="2"/><path d="M6 15V9l3 3 3-3v6M16 9v6M14 13l2 2 2-2"/></svg>',
}


def initials(name):
    parts = [w for w in name.split() if w]
    return "".join(w[0] for w in parts[:2]).upper() or "C"


def hero_src(p):
    return p["image"] or f"/assets/hero-blog-{p['slug']}.png"


def hero_card(p):
    """A 1600x800 picture for the top of a post: a Numbers Station page of digit groups,
    seeded by the slug so every post gets its own, with the site's glow behind it."""
    import hashlib
    h = hashlib.sha256(p["slug"].encode()).digest()
    rng = int.from_bytes(h, "big")
    rows = []
    for r in range(9):
        groups = []
        for g in range(7):
            rng, v = divmod(rng, 100000) if rng > 10**6 else (int.from_bytes(hashlib.sha256(f"{p['slug']}{r}{g}".encode()).digest(), "big"), rng % 100000)
            groups.append(f"{v:05d}")
        rows.append(" ".join(groups))
    seq = int.from_bytes(h[:2], "big")
    lines = "".join(f'<div class="row"><span class="seq">seq {seq + i}</span>{row}</div>' for i, row in enumerate(rows))
    return f"""<!doctype html><html lang="en"><head><meta charset="utf-8">
<link href="{FONTS}" rel="stylesheet">
<style>
html, body {{ margin: 0; width: 1600px; height: 800px; overflow: hidden; }}
body {{ background: #0a0a0b; color: #f4f4f5; font-family: "JetBrains Mono", monospace; position: relative; -webkit-font-smoothing: antialiased; }}
.glow {{ position: absolute; left: 50%; top: 50%; width: 1500px; height: 900px; transform: translate(-50%, -50%); background: radial-gradient(ellipse at center, rgba(229,163,58,0.18), rgba(229,163,58,0.05) 45%, transparent 70%); filter: blur(30px); }}
.page {{ position: absolute; inset: 0; padding: 88px 120px; display: flex; flex-direction: column; justify-content: center; gap: 22px; font-size: 40px; letter-spacing: 0.14em; color: #e5a33a; }}
.row {{ white-space: nowrap; opacity: .92; }}
.row:nth-child(2n) {{ opacity: .55; }}
.row:nth-child(3n) {{ opacity: .38; }}
.seq {{ display: inline-block; width: 190px; font-size: 20px; letter-spacing: 0; color: #6b6b74; }}
.mark {{ position: absolute; right: 64px; bottom: 56px; display: flex; align-items: center; gap: 14px; font-family: "Inter Tight", sans-serif; font-weight: 600; font-size: 26px; color: #f4f4f5; }}
.mark img {{ width: 34px; height: 34px; }}
.fade {{ position: absolute; inset: 0; background: linear-gradient(90deg, rgba(10,10,11,0.85), rgba(10,10,11,0) 30%, rgba(10,10,11,0) 70%, rgba(10,10,11,0.85)); }}
</style></head><body>
<div class="glow"></div>
<div class="page">{lines}</div>
<div class="fade"></div>
<div class="mark"><img src="../assets/mark.svg" alt="">Cahoots</div>
</body></html>
"""


def card(p):
    tags = "".join(f'<span class="tag">{e(t)}</span>' for t in p["tags"])
    return f"""      <article class="pcard">
        <a class="pcard-img" href="/blog/{p['slug']}/" tabindex="-1" aria-hidden="true"><img src="{e(hero_src(p))}" alt="" loading="lazy"></a>
        <div class="pcard-body">
          <h3><a href="/blog/{p['slug']}/">{e(p['title'])}</a></h3>
          <p>{e(p['description'])}</p>
          <div class="details">{tags}<span class="dot">·</span><span class="date">{ICONS['cal']}<time datetime="{p['date'].isoformat()}">{long_date(p['date'])}</time></span></div>
        </div>
      </article>"""


def post_page(p, others):
    url = f"{SITE}/blog/{p['slug']}/"
    image = f"{SITE}/assets/og-blog-{p['slug']}.png"
    tags = "".join(f'<span class="tag">{e(t)}</span>' for t in p["tags"])
    author = ""
    if p["author"]:
        author = f"""<span class="dot">·</span><span class="author"><span class="avatar" aria-hidden="true">{e(initials(p['author']))}</span><span><span class="name">{e(p['author'])}</span>{f'<span class="role">{e(p["role"])}</span>' if p['role'] else ''}</span></span>"""
    toc = ""
    if len(p["toc"]) >= 2:
        toc = f"""<aside class="toc" aria-label="In this article">
      <div class="toc-inner">
        <div class="toc-head">In this article</div>
        {''.join(f'<a href="#{e(i)}" data-id="{e(i)}">{e(t)}</a>' for i, t in p['toc'])}
      </div>
    </aside>"""
    learn = "".join(f"<li>{e(x)}</li>" for x in p["learn"])
    about = ""
    if p["about"] or p["audience"] or p["learn"]:
        about = f"""<div class="about">
        {f'<p><strong>About:</strong> {e(p["about"])}</p>' if p['about'] else ''}
        {f'<p><strong>Audience:</strong> {e(p["audience"])}</p>' if p['audience'] else ''}
        {f'<p><strong>What you’ll learn:</strong></p><ul>{learn}</ul>' if p['learn'] else ''}
        <p><strong>Reading time:</strong> ~{p['minutes']} minute{'s' if p['minutes'] != 1 else ''}</p>
      </div>"""
    from urllib.parse import quote
    share = f"""<div class="share">
        <span class="share-label">Share:</span>
        <a href="https://x.com/intent/post?url={quote(url)}&text={quote(p['title'])}" rel="noopener" target="_blank" aria-label="Share on X">{ICONS['x']}</a>
        <a href="https://www.linkedin.com/sharing/share-offsite/?url={quote(url)}" rel="noopener" target="_blank" aria-label="Share on LinkedIn">{ICONS['linkedin']}</a>
        <a href="https://news.ycombinator.com/submitlink?u={quote(url)}&t={quote(p['title'])}" rel="noopener" target="_blank" aria-label="Submit to Hacker News">{ICONS['hn']}</a>
        <button type="button" class="copy" data-url="{url}">{ICONS['link']}<span>Copy link</span></button>
      </div>"""
    related = ""
    if others:
        related = f"""<section class="related">
    <h2>More posts</h2>
    <div class="pgrid">
{chr(10).join(card(o) for o in others[:2])}
    </div>
  </section>"""
    return f"""{head(p['title'], p['description'], url, image, article=p)}
{BRAND_SUB_CSS}
<body class="blog-body">
<a class="skip" href="#main">Skip to content</a>
{TOP}
<main class="blog-wrap post-wrap" id="main">
  <div class="post-layout">
    <article class="post-main">
      <a class="back" href="/blog/">{ICONS['back']}Journal</a>
      <h1>{e(p['title'])}</h1>
      <div class="details">{tags}{'<span class="dot">·</span>' if tags else ''}<span class="date">{ICONS['cal']}<time datetime="{p['date'].isoformat()}">{long_date(p['date'])}</time></span>{author}</div>
      <img class="phero" src="{e(hero_src(p))}" alt="" width="1600" height="800">
      <div class="actions"><a class="btn small" href="/blog/{p['slug']}.md">{ICONS['md']}View as Markdown</a></div>
      {about}
      <div class="post">
{p['body']}
      </div>
      {share}
      <div class="follow">
        <p>Follow the build. New posts land here first, with the code and hardware on GitHub.</p>
        <div class="pcta"><a class="btn small" href="/blog/feed.xml">{ICONS['rss']}RSS feed</a><a class="btn small" href="{GITHUB}">{ICONS['github']}GitHub</a></div>
      </div>
    </article>
    {toc}
  </div>
  {related}
</main>
{FOOT}
<script>
(() => {{
  const b = document.querySelector(".copy");
  if (b) b.addEventListener("click", async () => {{
    try {{ await navigator.clipboard.writeText(b.dataset.url); b.querySelector("span").textContent = "Copied"; setTimeout(() => b.querySelector("span").textContent = "Copy link", 1600); }} catch {{}}
  }});
  const links = [...document.querySelectorAll(".toc a")];
  if (!links.length || !("IntersectionObserver" in window)) return;
  const hs = links.map((a) => document.getElementById(a.dataset.id)).filter(Boolean);
  let current = null;
  const set = (id) => {{ if (id === current) return; current = id; links.forEach((a) => a.classList.toggle("on", a.dataset.id === id)); }};
  const pick = () => {{
    let id = hs[0]?.id;
    for (const h of hs) if (h.getBoundingClientRect().top <= 120) id = h.id;
    set(id);
  }};
  addEventListener("scroll", pick, {{ passive: true }});
  pick();
}})();
</script>
</body>
</html>
"""


def index_page(posts):
    url = f"{SITE}/blog/"
    image = f"{SITE}/assets/og-blog.png"
    return f"""{head("Cahoots journal", "Ideas, experiments, and progress on a network built between people.", url, image)}
{BRAND_SUB_CSS}
<body class="blog-body">
<a class="skip" href="#main">Skip to content</a>
{TOP}
<main class="blog-wrap" id="main">
  <header class="blog-head">
    <p class="eyebrow">Notes from the workshop</p>
    <h1>The Cahoots journal.</h1>
    <p>Ideas, experiments, and progress on a network built between people.</p>
    <div class="pcta"><a class="btn small" href="/blog/feed.xml">{ICONS['rss']}RSS feed</a></div>
  </header>
  <div class="pgrid">
{chr(10).join(card(p) for p in posts)}
  </div>
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
  <title>Cahoots journal</title>
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


def render_png(chrome, src, dest, size="1200,630"):
    subprocess.run([chrome, "--headless=new", "--disable-gpu", "--hide-scrollbars", "--no-sandbox",
                    f"--window-size={size}", "--virtual-time-budget=4000",
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
    global CSS_V
    import hashlib
    CSS_V = hashlib.sha256(css.read_bytes()).hexdigest()[:10]
    if css.resolve() != (blog / "blog.css").resolve():
        shutil.copy(css, blog / "blog.css")
    og_css = ROOT / "site" / "og" / "og.css"
    if og_css.resolve() != (out / "og" / "og.css").resolve():
        shutil.copy(og_css, out / "og" / "og.css")

    (blog / "index.html").write_text(index_page(posts), encoding="utf-8")
    (blog / "feed.xml").write_text(feed(posts), encoding="utf-8")
    heroes = []
    cards = [("blog", og_card("Building in the open.", "The protocol, the code, the hardware, and the mistakes.",
                              "cahoots.network/blog", "rss · cahoots.network/blog/feed.xml"))]
    for p in posts:
        d = blog / p["slug"]
        d.mkdir(exist_ok=True)
        (d / "index.html").write_text(post_page(p, [o for o in posts if o is not p]), encoding="utf-8")
        (blog / f"{p['slug']}.md").write_text(p["source"], encoding="utf-8")
        cards.append((f"blog-{p['slug']}", og_card(p["title"], p["description"],
                                                   f"cahoots.network/blog/{p['slug']}", long_date(p["date"]))))
        if not p["image"]:
            heroes.append((f"blog-{p['slug']}", hero_card(p)))
    for name, src in cards:
        (out / "og" / f"{name}.html").write_text(src, encoding="utf-8")
    for name, src in heroes:
        (out / "og" / f"hero-{name}.html").write_text(src, encoding="utf-8")
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
    for name, _ in heroes:
        dest = out / "assets" / f"hero-{name}.png"
        render_png(chrome, out / "og" / f"hero-{name}.html", dest, size="1600,800")
        print(f"build-blog: wrote {dest.relative_to(out)}")


if __name__ == "__main__":
    main()
