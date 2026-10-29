# Site and docs implementation plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** A landing page that sells a private network for people who have met in person, with a pixel-art hero, the apps as the product and the card as a chapter; the docs reorganised and swept of stale hardware facts.

**Architecture:** Static site as today (`site/`, no bundler). A Python script `site/art/gen.py` draws the hero scene, the app icons and the sigil mock as SVG from text maps, committed like the diagrams. `site/index.html` is rewritten; `site/style.css` extended; `site/main.js` reduced to the ticker and the feed sample; `badge.js`/`rig.js` deleted. `site/docs/docs.js` regrouped. Docs edited in place. OG cards re-rendered.

**Tech Stack:** HTML, CSS, vanilla JS, Python 3 (stdlib only) for the art script, headless Chrome via `site/og/render.sh` for OG images, Playwright in `web/` for a visual check.

**Spec:** `docs/superpowers/specs/2026-10-28-site-and-docs-design.md`

## Global Constraints

- Copy rules: plain words, no coined names, no em dashes, no crypto clichés (see the spec's anti-goals). Every claim about keys in the chip, attestation, logins from the card and the sigil carries the "being built" label on the page.
- Theme tokens from `site/style.css` `:root` are the only colours; the pixel palette is sixteen colours derived from them and listed once in `gen.py`.
- No new external scripts or fonts; three.js and its import map are removed.
- The hero SVG stays under 60 kB; the page loads with no console errors.
- Nothing on the page contradicts `docs/`: six ceremony steps, V1 as the board being built, assurance words from `tiers.md`.
- Commit after every task, plain-sentence messages.

## Review Focus

- A phone-width viewport must show the hero cropped from the centre with no horizontal scroll, the five app cards in one column, and the ticker. Test in Task 4.
- The docs shell must open every route in the new `DOCS` list, including `manufacturing` and `v1-board`, with no 404 for its markdown. Test in Task 6.
- The sweep must leave no sentence in the listed docs saying V1 lacks a secure element, is unordered, or is T1 as a hardware fact. Test in Task 7 (grep).
- The OG image must render the new headline over the scene at 1200 by 630. Test in Task 8.
- `prefers-reduced-motion` must stop the ticker. Test in Task 4.

---

### Task 1: The art script: palette, scene, icons, sigil

**Files:**
- Create: `site/art/gen.py`, `site/art/README.md`
- Create (output): `site/assets/place.svg`, `site/assets/icons/{chat,login,tunnel,seal,sign}.svg`, `site/assets/sigil.svg`, `site/assets/screen-words.svg`
- Test: `site/art/test_gen.py` (stdlib `unittest`)

**Interfaces:**
- Produces: `python3 site/art/gen.py` writes every asset; functions `palette() -> dict[str, str]` (sixteen names to hex, built from the theme tokens), `render(map_text: str, palette_keys: dict[str, str], cell: int) -> str` (SVG with one `<rect>` per run of same-colour cells in a row, `shape-rendering="crispEdges"`), `scene() -> str` (96 by 56 cells), `icon(name) -> str` (16 by 16, one bit, amber on transparent), `sigil(seed: bytes) -> str` (a 24 by 24 one-bit mark: symmetric across the vertical axis, drawn from SHA-256 of the seed, with a 2-cell border and at least 30% and at most 70% ink), `words_screen(circle, tier, words) -> str` (200 by 200 one-bit mock of `crates/badge/src/screens.rs::words`: header line with the circle name and `TRNG · T1`, six words one per row).

- [ ] **Step 1: Write the failing tests**: `render_merges_runs` (a 3 by 1 map "aab" gives two rects), `scene_size_and_palette` (viewBox 96 by 56 cells scaled, every fill in the palette), `scene_under_60k`, `sigil_symmetric_and_bounded_ink`, `sigil_deterministic`, `icons_one_bit`, `words_screen_has_six_rows`.
- [ ] **Step 2: Run** `python3 -m unittest site/art/test_gen.py`: FAIL.
- [ ] **Step 3: Implement**. The scene's map is authored in the script as a list of 56 strings of 96 characters, each character a palette key. Composition, left to right: a door at the far left with a strip of light under it; a round table in the centre with five seated figures (different hair and jackets, two colours each), three cards on the table glowing amber with a small white rectangle for each screen; a hanging lamp above casting a warm circle; a window at the right with a dark blue night and a few lit windows in a far building; a shelf with bottles top right; floorboards in two greys. Keep it readable at 25% scale. Icons: chat (two overlapping speech rectangles), login (a key), tunnel (two nodes and a line), seal (a padlock), sign (a pen stroke with a check).
- [ ] **Step 4: Run** the tests: PASS. Run the script; open each SVG in a browser or Playwright screenshot and look at the scene once at full size and once at 320 px wide; fix the map until the figures and the table read.
- [ ] **Step 5: Commit** `git add site/art site/assets && git commit -m "site: a pixel-drawn back room, the app icons, a sigil and a words screen, all from one script"`.

### Task 2: The landing page

**Files:**
- Modify: `site/index.html` (rewrite), `site/style.css`, `site/main.js` (ticker and feed sample only)
- Delete: `site/badge.js`, `site/rig.js`
- Copy: `hardware/v1/front.png` to `site/assets/v1-front.png`

**Interfaces:**
- Consumes: the assets from Task 1.
- Produces: the page as the spec's "The page" section describes, section ids `what`, `how`, `ways`, `card`, `feed`, `open`.

- [ ] **Step 1: Write a check script** `web/site-check.mjs` (Playwright, reusing `web/node_modules`) that loads `site/index.html` through a static server on a free port, asserts: no console errors; the H1 is "Get in cahoots."; six `ol.steps li`; five `.app` cards; three `.way` columns; the `#card` section has an `img[src$="v1-front.png"]` and an `img[src$="sigil.svg"]`; every `.soon` label text is "being built"; no `script[src*="three"]`; at 390 px width `document.documentElement.scrollWidth <= 390`. Run it: FAIL (old page).
- [ ] **Step 2: Rewrite** `index.html` with the spec's copy, verbatim where the spec gives it, written in its voice elsewhere; the hero scene as an `<img>` with the alt "A back room at night: five people at a round table, their cards glowing"; the mock screen as `screen-words.svg` with the words from the README's example; the apps grid with `.soon` labels on Chat (the phone app), Log in, Sign, and on the card's chip sentences; the feed sample as now.
- [ ] **Step 3: Style**: the hero as a full-width image with a gradient to the background and the copy over it at the bottom left on wide screens and below it on phones; `.app` cards 5 across to 1 across; `.ways` three columns to one; the card section two columns to one; `image-rendering: pixelated` on the pixel assets; `prefers-reduced-motion` pauses the ticker.
- [ ] **Step 4: Run** the check script: PASS. Screenshot at 1280 and 390 wide into the scratch folder and look at them.
- [ ] **Step 5: Commit** `git commit -am "site: the page sells the network: a place, what members can do, how you get in, three ways in, the card as a chapter; the 3D badge leaves"`.

### Task 3: The docs shell and the build

**Files:**
- Modify: `site/docs/docs.js` (groups per the spec, including `manufacturing` and `v1-board`), `site/docs/index.html` (the scene at a third height, the lede), `site/docs/docs.css`, `.github/workflows/pages.yml` (copy `crates/rp2350/README.md` as `rp2350.md` and `hardware/v1/README.md` as `v1-board.md`; rewrite the V1 README's relative image links to the raw GitHub URLs in the copy step with `sed`, or document that images in that page link to GitHub)

- [ ] **Step 1: Extend** `web/site-check.mjs` to open the docs shell with every `DOCS` id as a hash route against a local copy of `docs/*.md` placed where the workflow puts them (the script does the copy into a temp `_site`), asserting each renders a heading and no "could not load" text.
- [ ] **Step 2: Run**: FAIL (new ids missing).
- [ ] **Step 3: Implement**.
- [ ] **Step 4: Run**: PASS.
- [ ] **Step 5: Commit** `git commit -am "docs site: start here, the system, the card, ideas, project, reference; the board and the Pico firmware are published"`.

### Task 4: Mobile and motion checks

**Files:**
- Modify: `site/style.css` as the checks demand
- Test: `web/site-check.mjs` already asserts the phone width; add `prefers-reduced-motion` emulation asserting the ticker track has no running animation.

- [ ] **Step 1: Add** the assertions; run: note failures.
- [ ] **Step 2: Fix** the CSS.
- [ ] **Step 3: Run**: PASS. Commit `git commit -am "site: phone layout and reduced motion"`.

### Task 5: README and glossary

**Files:**
- Modify: `README.md` (the opening three sentences become the new story; "cahoot (a circle)" becomes "a **circle**"), `docs/glossary.md` (card, circle gains the app's word, sigil marked as being built, contact card, attested as a plain-word entry pointing at the chip design's rung), `web/app/index.html` and `manifest.json` description line, `site/og/*.html` headlines and descriptions

- [ ] **Step 1: Edit**.
- [ ] **Step 2: Check** links resolve: `grep -o '\](\([^)]*\.md[^)]*\))' README.md docs/glossary.md` each target exists.
- [ ] **Step 3: Commit** `git commit -am "readme and glossary: the network first, the card as one way in; new words: card, sigil, contact card, attested"`.

### Task 6: The docs sweep

**Files:**
- Modify: every file the new `DOCS` list names, plus `hardware/v1/README.md` and `crates/rp2350/README.md` where they contradict today: `docs/manufacturing.md` (status: quoted, five boards about to be ordered, SE051 on the board; the ESP32-S3 paragraph becomes past tense), `docs/tiers.md` (hardware facts only: V1 carries the SE051 and attestation is being built; the levels untouched), `docs/diy-badge.md` (V1 has the SE051; the tier column says "T1 today, chip being wired"), `docs/use-cases.md` (section 3 table replaced by a sentence and a link), `docs/gatherings.md` and `docs/attested-data.md` (only wrong hardware facts, if any)

- [ ] **Step 1: Grep** for the stale phrases: `grep -rn -i "no secure element\|not yet reviewed or ordered\|not reviewed or ordered\|V1 is T1\|no Cahoots board is at T2\|next revision" docs/ hardware/v1/README.md crates/rp2350/README.md`; list each hit.
- [ ] **Step 2: Fix** each; re-run the grep until it is empty or every remaining hit is intentional (say which in the commit message).
- [ ] **Step 3: Commit** `git commit -am "docs: a sweep for what the hardware is today: V1 carries the SE051, five boards quoted, the Pico 2 runs the firmware"`.

### Task 7: Open Graph images

**Files:**
- Modify: `site/og/home.html`, `site/og/docs.html`, `site/og/app.html`, `site/og/og.css` (the scene behind the headline), run `site/og/render.sh`
- Output: `site/assets/og-home.png`, `og-docs.png`, `og-app.png`

- [ ] **Step 1: Edit** the three cards: headline "Get in cahoots." / "How it works." / "Try it in your browser.", the one-line description from the spec, the scene as the background with a dark gradient.
- [ ] **Step 2: Render** with `site/og/render.sh`; check each PNG is 1200 by 630 (`sips -g pixelWidth -g pixelHeight`).
- [ ] **Step 3: Commit** `git commit -am "og cards: the place behind the headline"`.

### Task 8: Final look

- [ ] **Step 1: Serve** `site/` locally per `site/README.md`, run `web/site-check.mjs` once more, take full-page screenshots at 1440 and 390 into the scratch folder, and read the page top to bottom as the audience would.
- [ ] **Step 2: Fix** anything that reads wrong; commit `git commit -am "site: last pass"`.
