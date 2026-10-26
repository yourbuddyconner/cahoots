# cahoots.network

The landing page, docs site and blog. Static files, no bundler. Deployed to GitHub Pages by `.github/workflows/pages.yml` on every push to `main`, which also builds the browser app into `/app`, copies the markdown docs into `/docs/md`, and renders the blog.

## Preview locally

```sh
mkdir -p site/docs/md && cp README.md site/docs/md/readme.md && cp docs/*.md site/docs/md/
python3 scripts/build-blog.py --out site      # needs `pip install markdown`; add --skip-og without Chrome
cd site && python3 -m http.server 8787
```

## Blog

Posts are markdown files in `blog/`, named `YYYY-MM-DD-slug.md`, with front matter for `title`, `date`, `description` and optionally `author`, `role`, `tags` (comma separated), `about`, `audience`, `learn` (a `- ` list, shown in the box above the post), `image` (a 2:1 picture for the top of the post), `slug` and `draft: true`. `scripts/build-blog.py` renders `/blog/`, `/blog/<slug>/` (with an "In this article" list from the h2 headings, a share row and the post's markdown at `/blog/<slug>.md`), an RSS feed at `/blog/feed.xml`, one Open Graph card per post (`assets/og-blog-<slug>.png`, from the same `og/og.css` as the other cards) and, when no `image` is given, a generated 1600x800 hero (`assets/hero-blog-<slug>.png`), all with headless Chrome. The rendered output under `site/blog/` and the generated images are gitignored; `site/blog/blog.css` is the only committed file there.

The exploded badge in `main.js` is the manufactured badge as designed, built in `badge.js` from swept outlines and primitives so nothing is fetched: a credit-card footprint 2.9 mm thick, a clear polycarbonate tray and top sheet with a lanyard slot, a 1.54" e-paper, the Baochip-1x on a black board, a CR2016 coin cell in a board cutout, two dome buttons, and the stacking link: four magnets in the floor, four gold pads on the face, four spring fingers underneath. 1 mm is 0.05 scene units, so dimensions in the file are the real ones. `rig.js` holds the studio environment, lights, the backdrop and the ambient-occlusion composer.

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
