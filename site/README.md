# cahoots.network

The landing page, docs site and blog. Static files, no bundler. Deployed to GitHub Pages by `.github/workflows/pages.yml` on every push to `main`, which also builds the browser app into `/app`, copies the markdown docs into `/docs/md`, and renders the blog.

## Preview locally

```sh
mkdir -p site/docs/md site/docs/diagrams && cp README.md site/docs/md/readme.md && cp docs/*.md site/docs/md/ && cp docs/diagrams/*.svg site/docs/diagrams/
cp web/README.md site/docs/md/web.md && cp crates/station/README.md site/docs/md/station.md && cp crates/chat/README.md site/docs/md/chat-cli.md
cp crates/host/README.md site/docs/md/host.md && cp crates/ADAPTERS.md site/docs/md/adapters.md
cp crates/rp2350/README.md site/docs/md/rp2350.md && cp hardware/v1/README.md site/docs/md/v1-board.md
python3 scripts/build-blog.py --out site      # needs `pip install markdown`; add --skip-og without Chrome
cd site && python3 -m http.server 8787
```

## Blog

Posts are markdown files in `blog/`, named `YYYY-MM-DD-slug.md`, with front matter for `title`, `date`, `description` and optionally `author`, `role`, `tags` (comma separated), `about`, `audience`, `learn` (a `- ` list, shown in the box above the post), `image` (a 2:1 picture for the top of the post), `slug` and `draft: true`. `scripts/build-blog.py` renders `/blog/`, `/blog/<slug>/` (with an "In this article" list from the h2 headings, a share row and the post's markdown at `/blog/<slug>.md`), an RSS feed at `/blog/feed.xml`, one Open Graph card per post (`assets/og-blog-<slug>.png`, from the same `og/og.css` as the other cards) and, when no `image` is given, a generated 1600x800 hero (`assets/hero-blog-<slug>.png`), all with headless Chrome. The rendered output under `site/blog/` and the generated images are gitignored; `site/blog/blog.css` is the only committed file there.

The pictures on the landing page are drawn by `site/art/gen.py`, the way `docs/diagrams/gen.py` draws the diagrams: the poster of a rooftop at golden hour at the top (`assets/place.svg`, flat shapes in seven colours with a light grain), the four app icons (`assets/icons/`, two amber strokes each), a card's pixel punk (`assets/sigil.svg`) and twelve of them (`assets/punks.svg`) and the badge's words screen (`assets/screen-words.svg`). `site/og/render.sh` renders the poster to `assets/place.png` and the Open Graph cards on top of it. Run `python3 site/art/gen.py` after changing the script and commit the output; `python3 -m unittest discover -s site/art` checks it. `assets/v1-front.png` is the board render copied from `hardware/v1/front.png`.

## One-time setup

1. GitHub → repository → Settings → Pages → Source: **GitHub Actions**.
2. Settings → Pages → Custom domain: `cahoots.network`, then tick **Enforce HTTPS** once the certificate is issued. `site/CNAME` already carries the name.
3. DNS at the registrar for `cahoots.network`:

   | Type | Name | Value |
   |---|---|---|
   | A | @ | 185.199.108.153 |
   | A | @ | 185.199.109.153 |
   | A | @ | 185.199.110.153 |
   | A | @ | 185.199.111.153 |
   | CNAME | www | yourbuddyconner.github.io |

4. `cahoots.tools`: a forwarding rule at the registrar to `https://cahoots.network` (301), or the same records if it should also serve the site. Only one custom domain can be set on the Pages site.
