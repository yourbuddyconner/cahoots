# cahoots.network

The landing page and docs site. Static files, no bundler. Deployed to GitHub Pages by `.github/workflows/pages.yml` on every push to `main`, which also builds the browser app into `/app` and copies the markdown docs into `/docs/md`.

## Preview locally

```sh
mkdir -p site/docs/md && cp README.md site/docs/md/readme.md && cp docs/*.md site/docs/md/
cd site && python3 -m http.server 8787
```

The exploded badge in `main.js` is the reference hardware design built from three.js primitives: an ID-1 card footprint, two polycarbonate shells with four screws and a lanyard slot, a 1.54" e-paper on a ribbon, the Baochip-1x on a black board, a four-pin magnetic touch link on each short edge, a 400 mAh LiPo, USB-C, and one confirm button. 1 mm is 0.05 scene units, so dimensions in the file are the real ones.

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
