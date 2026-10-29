// Check of the landing page in site/: serves site/ on a free port, loads index.html in headless Chrome
// and asserts the page's structure. One line per assertion; exits non-zero if any fails.
//   node web/site-check.mjs            (needs `npm install` in web/ and Google Chrome)
import { createRequire } from "node:module";
import { createServer } from "node:http";
import { createReadStream, existsSync, statSync, mkdtempSync, mkdirSync, cpSync, copyFileSync, readFileSync, rmSync, readdirSync } from "node:fs";
import { tmpdir } from "node:os";
import { extname, join, resolve } from "node:path";
import { fileURLToPath } from "node:url";

const here = resolve(fileURLToPath(import.meta.url), "..");
const { chromium } = createRequire(join(here, "package.json"))("playwright");
// SITE_DIR points the check at another copy of site/, for example an older commit's.
// SHOTS_DIR, when set, is where the check saves a full-page screenshot at 390 px (phone.png).
const siteDir = resolve(process.env.SITE_DIR || join(here, "..", "site"));

const types = { ".html": "text/html", ".js": "text/javascript", ".mjs": "text/javascript", ".css": "text/css", ".svg": "image/svg+xml", ".json": "application/json", ".png": "image/png", ".ico": "image/x-icon" };
function serve(root) {
  const srv = createServer((req, res) => {
    let p = join(root, decodeURIComponent(new URL(req.url, "http://x").pathname));
    if (existsSync(p) && statSync(p).isDirectory()) p = join(p, "index.html");
    if (!p.startsWith(root) || !existsSync(p)) { res.writeHead(404); res.end(); return; }
    res.writeHead(200, { "content-type": types[extname(p)] || "application/octet-stream" });
    createReadStream(p).pipe(res);
  });
  return new Promise((r) => srv.listen(0, "127.0.0.1", () => r({ srv, base: `http://127.0.0.1:${srv.address().port}/` })));
}
const { srv: server, base } = await serve(siteDir);

let failed = 0;
function check(name, ok, detail = "") {
  console.log(`${ok ? "PASS" : "FAIL"}  ${name}${!ok && detail ? `  (${detail})` : ""}`);
  if (!ok) failed++;
}

const browser = await chromium.launch({ channel: "chrome" });
try {
  // Desktop.
  const errors = [];
  const ctx = await browser.newContext({ viewport: { width: 1280, height: 900 } });
  const page = await ctx.newPage();
  page.on("pageerror", (e) => errors.push(e.message));
  page.on("console", (m) => { if (m.type() === "error") errors.push(m.text()); });
  page.on("requestfailed", (r) => errors.push(`request failed: ${r.url()}`));
  page.on("response", (r) => { if (r.status() >= 400 && r.url().startsWith(base)) errors.push(`${r.status()} ${r.url()}`); });
  await page.goto(base, { waitUntil: "load" });
  // Load every image, lazy ones included, so the image assertion checks all of them.
  await page.evaluate(async () => {
    for (const i of document.images) i.loading = "eager";
    await Promise.all([...document.images].map((i) => i.decode().catch(() => {})));
  });
  await page.waitForTimeout(300);

  const facts = await page.evaluate(() => {
    const q = (s) => [...document.querySelectorAll(s)];
    const card = document.getElementById("card");
    return {
      h1: document.querySelector("h1")?.textContent.trim() ?? null,
      heroAlt: document.querySelector(".scene > img")?.getAttribute("alt") ?? null,
      heroSrc: document.querySelector(".scene > img")?.getAttribute("src") ?? null,
      heroRendering: (() => { const i = document.querySelector(".scene > img"); return i ? getComputedStyle(i).imageRendering : null; })(),
      h1InScene: !!document.querySelector(".scene h1"),
      h1Font: (() => { const h = document.querySelector("h1"); return h ? getComputedStyle(h).fontFamily : ""; })(),
      steps: q("ol.steps li").length,
      apps: q(".app").length,
      ways: q(".way").length,
      front: !!card?.querySelector('img[src$="v1-front.png"]'),
      sigil: !!card?.querySelector('img[src$="sigil.svg"]'),
      punks: !!card?.querySelector('img[src$="punks.svg"]'),
      punkCopy: /your punk, drawn from the card's own randomness, the same in every circle/.test(card?.textContent ?? ""),
      soon: q(".soon").length,
      text: document.body.innerText,
      desc: document.querySelector('meta[name="description"]')?.getAttribute("content") ?? null,
      three: q('script[src*="three"]').length + q('script[type="importmap"]').filter((s) => s.textContent.includes("three")).length,
      ids: ["what", "how", "ways", "card", "feed", "open"].filter((id) => !document.getElementById(id)),
      images: q("img").length,
      broken: q("img").filter((i) => !i.complete || i.naturalWidth === 0).map((i) => i.getAttribute("src")),
    };
  });
  check("no console errors or failed requests at 1280 px", errors.length === 0, errors.join("; "));
  check('the H1 is "Get in cahoots."', facts.h1 === "Get in cahoots.", JSON.stringify(facts.h1));
  const alt = "A rooftop at golden hour: five people at a round table, their cards on it, the city behind";
  check("the hero image is place.svg with the rooftop alt text", facts.heroSrc === "assets/place.svg" && facts.heroAlt === alt, `${facts.heroSrc} ${JSON.stringify(facts.heroAlt)}`);
  check("the hero image is not drawn pixelated", !/pixelated|crisp-edges/.test(facts.heroRendering ?? ""), facts.heroRendering);
  check("the H1 sits over the poster, set in Fraunces", facts.h1InScene && /Fraunces/.test(facts.h1Font), `${facts.h1InScene} ${facts.h1Font}`);
  check("six ol.steps li", facts.steps === 6, `found ${facts.steps}`);
  check("four .app cards", facts.apps === 4, `found ${facts.apps}`);
  check("three .way columns", facts.ways === 3, `found ${facts.ways}`);
  check('#card has img[src$="v1-front.png"]', facts.front);
  check('#card has img[src$="sigil.svg"]', facts.sigil);
  check('#card has img[src$="punks.svg"]', facts.punks);
  check("#card calls the picture at rest your punk", facts.punkCopy);
  check("no .soon elements", facts.soon === 0, `found ${facts.soon}`);
  check('no "being built" text on the page', !/being built/i.test(facts.text));
  check('no "tunnel" in the page text', !/tunnel/i.test(facts.text));
  const tagline = "A private network for people who have met in person.";
  check("the meta description is the tagline", facts.desc === tagline, JSON.stringify(facts.desc));
  check("no three.js script or import map", facts.three === 0, `found ${facts.three}`);
  check("sections what, how, ways, card, feed, open exist", facts.ids.length === 0, `missing ${facts.ids.join(", ")}`);
  check(`every image loads (${facts.images} checked)`, facts.broken.length === 0, facts.broken.join(", "));
  await ctx.close();

  // Phone.
  const pctx = await browser.newContext({ viewport: { width: 390, height: 844 }, isMobile: true, hasTouch: true, deviceScaleFactor: 2 });
  const phone = await pctx.newPage();
  const perrors = [];
  phone.on("pageerror", (e) => perrors.push(e.message));
  phone.on("console", (m) => { if (m.type() === "error") perrors.push(m.text()); });
  await phone.goto(base, { waitUntil: "load" });
  await phone.waitForTimeout(300);
  const sw = await phone.evaluate(() => document.documentElement.scrollWidth);
  check("no console errors at 390 px", perrors.length === 0, perrors.join("; "));
  check("at 390 px the page is no wider than 390", sw <= 390, `scrollWidth ${sw}`);
  const pf = await phone.evaluate(() => {
    const img = document.querySelector(".scene > img");
    const h1 = document.querySelector(".scene h1")?.getBoundingClientRect();
    const ir = img?.getBoundingClientRect();
    const apps = [...document.querySelectorAll(".app")].map((a) => a.getBoundingClientRect());
    const t = document.querySelector(".ticker");
    const tr = t?.getBoundingClientRect();
    const ts = t && getComputedStyle(t);
    return {
      pos: img ? getComputedStyle(img).objectPosition : null,
      h1Inside: !!h1 && !!ir && h1.left >= ir.left && h1.right <= ir.right && h1.top >= ir.top && h1.bottom <= ir.top + ir.height * 0.4,
      fit: img ? getComputedStyle(img).objectFit : null,
      lefts: apps.map((r) => Math.round(r.left)),
      stacked: apps.every((r, i) => i === 0 || r.top >= apps[i - 1].bottom - 1),
      ticker: !!t && tr.width > 0 && tr.height > 0 && ts.display !== "none" && ts.visibility !== "hidden",
    };
  });
  check("at 390 px the headline fits inside the top of the poster", pf.h1Inside);
  check("at 390 px the hero image is cropped from the centre", pf.fit === "cover" && /^(50%|center)/.test(pf.pos ?? ""), `${pf.fit} ${pf.pos}`);
  check("at 390 px the four .app cards share one left edge", pf.lefts.length === 4 && new Set(pf.lefts).size === 1, pf.lefts.join(","));
  check("at 390 px each .app card sits below the previous", pf.stacked);
  check("at 390 px the ticker is visible", pf.ticker);
  if (process.env.SHOTS_DIR) {
    const shotDir = resolve(process.env.SHOTS_DIR);
    mkdirSync(shotDir, { recursive: true });
    await phone.screenshot({ path: join(shotDir, "phone.png"), fullPage: true });
  }
  const running = (pg) => pg.evaluate(() => {
    const tr = document.querySelector(".ticker-track");
    return tr.getAnimations().filter((a) => a.playState === "running").length;
  });
  check("without reduced motion the ticker animates", (await running(phone)) > 0);
  await pctx.close();
  const rctx = await browser.newContext({ viewport: { width: 390, height: 844 }, reducedMotion: "reduce" });
  const rp = await rctx.newPage();
  await rp.emulateMedia({ reducedMotion: "reduce" });
  await rp.goto(base, { waitUntil: "load" });
  await rp.waitForTimeout(300);
  const ra = await rp.evaluate(() => {
    const tr = document.querySelector(".ticker-track");
    return { n: tr.getAnimations().filter((a) => a.playState === "running").length, name: getComputedStyle(tr).animationName };
  });
  check("with reduced motion the ticker track has no running animation", ra.n === 0, `${ra.n} running, animation-name ${ra.name}`);
  await rctx.close();

  // Docs shell. Assemble a temp _site the way .github/workflows/pages.yml does (keep the two lists in step),
  // then open every DOCS id as a hash route.
  const repo = resolve(here, "..");
  const tmp = mkdtempSync(join(tmpdir(), "cahoots-site-"));
  let dsrv;
  try {
    const out = join(tmp, "_site");
    cpSync(join(repo, "site"), out, { recursive: true });
    mkdirSync(join(out, "docs/md"), { recursive: true });
    mkdirSync(join(out, "docs/diagrams"), { recursive: true });
    const md = (from, to) => copyFileSync(join(repo, from), join(out, "docs/md", to));
    md("README.md", "readme.md");
    for (const f of readdirSync(join(repo, "docs")).filter((f) => f.endsWith(".md"))) md(join("docs", f), f);
    for (const f of readdirSync(join(repo, "docs/diagrams")).filter((f) => f.endsWith(".svg"))) copyFileSync(join(repo, "docs/diagrams", f), join(out, "docs/diagrams", f));
    md("web/README.md", "web.md");
    md("crates/station/README.md", "station.md");
    md("crates/chat/README.md", "chat-cli.md");
    md("crates/host/README.md", "host.md");
    md("crates/ADAPTERS.md", "adapters.md");
    md("crates/rp2350/README.md", "rp2350.md");
    md("hardware/v1/README.md", "v1-board.md");
    const ids = [...readFileSync(join(repo, "site/docs/docs.js"), "utf8").matchAll(/\bid: "([a-z0-9-]+)"/g)].map((m) => m[1]);
    const d = await serve(out);
    dsrv = d.srv;
    const dctx = await browser.newContext({ viewport: { width: 1280, height: 900 } });
    const dp = await dctx.newPage();
    const bad = [];
    for (const id of ids) {
      await dp.goto(`${d.base}docs/#/${id}`, { waitUntil: "load" });
      await dp.waitForSelector("#content h1, #content p:not(.loading)", { timeout: 15000 }).catch(() => {});
      const r = await dp.evaluate(() => ({ h: !!document.querySelector("#content h1"), fail: /isn't published yet|could not load/i.test(document.getElementById("content").textContent) }));
      if (!r.h || r.fail) bad.push(id);
    }
    check(`every docs id renders a heading and loads (${ids.length} checked)`, ids.length >= 20 && bad.length === 0, `ids ${ids.length}; bad: ${bad.join(", ")}`);
    const need = ["use-cases", "ceremony-guide", "glossary", "protocol", "chat", "threat-model", "tiers", "diy-badge", "v1-board", "manufacturing", "rp2350", "gatherings", "attested-data", "plan", "web", "station", "chat-cli", "host", "adapters"];
    check("docs list names the grouped ids, including v1-board, manufacturing and rp2350", need.every((i) => ids.includes(i)), `missing ${need.filter((i) => !ids.includes(i)).join(", ")}`);
    // Links: the board page's link to another doc is a route, and its link to a file in its own folder lands on GitHub at the right path.
    await dp.goto(`${d.base}docs/#/v1-board`, { waitUntil: "load" });
    await dp.waitForSelector("#content h1");
    const hrefs = await dp.$$eval("#content a[href]", (as) => as.map((a) => a.getAttribute("href")));
    check("v1-board links to manufacturing as a docs route", hrefs.includes("#/manufacturing"), hrefs.filter((h) => /manufact/.test(h)).join(", "));
    await dp.goto(`${d.base}docs/#/rp2350`, { waitUntil: "load" });
    await dp.waitForSelector("#content h1");
    const rh = await dp.$$eval("#content a[href]", (as) => as.map((a) => a.getAttribute("href")));
    check("rp2350 links to design.py on GitHub at hardware/v1/design.py", rh.includes("https://github.com/yourbuddyconner/cahoots/blob/main/hardware/v1/design.py"), rh.filter((h) => /design\.py/.test(h)).join(", "));
    check("rp2350 links to a doc section as a docs route", rh.includes("#/diy-badge#raspberry-pi-pico-2"), rh.filter((h) => /diy/.test(h)).join(", "));
    check("rp2350 links to another crate README on GitHub, not the overview", rh.includes("https://github.com/yourbuddyconner/cahoots/blob/main/crates/esp32s3/README.md#what-the-board-does"), rh.filter((h) => /esp32/.test(h)).join(", "));
    await dctx.close();
  } finally {
    dsrv?.close();
    rmSync(tmp, { recursive: true, force: true });
  }
} finally {
  await browser.close();
  server.close();
}

console.log(failed ? `${failed} failed` : "all passed");
process.exit(failed ? 1 : 0);
