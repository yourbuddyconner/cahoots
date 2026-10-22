# cahoots.network

The landing page and docs site. Static files, no bundler. Deployed to GitHub Pages by `.github/workflows/pages.yml` on every push to `main`, which also builds the browser app into `/app` and copies the markdown docs into `/docs/md`.

## Preview locally

```sh
mkdir -p site/docs/md && cp README.md site/docs/md/readme.md && cp docs/*.md site/docs/md/
cd site && python3 -m http.server 8787
```

The exploded badge in `main.js` is the manufactured badge as designed, built in `badge.js` from swept outlines and primitives so nothing is fetched: a credit-card footprint 2.9 mm thick, a clear polycarbonate tray and top sheet with a lanyard slot, a 1.54" e-paper, the Baochip-1x on a black board, a CR2016 coin cell in a board cutout, two dome buttons, and the stacking link: four magnets in the floor, four gold pads on the face, four spring fingers underneath. 1 mm is 0.05 scene units, so dimensions in the file are the real ones. `rig.js` holds the studio environment, lights, the backdrop and the ambient-occlusion composer, and is shared with the launch video.

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
