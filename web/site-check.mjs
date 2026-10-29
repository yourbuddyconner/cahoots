// Check of the landing page in site/ and the docs shell: serves site/ on a free port, loads index.html in
// headless Chrome and asserts the page's structure, then opens every docs route. One line per assertion; exits non-zero if any fails.
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
    const stage = document.getElementById("badge-stage");
    return {
      title: document.title,
      h1: document.querySelector("h1")?.innerText.replace(/\s+/g, " ").trim() ?? null,
      h1Font: (() => { const h = document.querySelector("h1"); return h ? getComputedStyle(h).fontFamily : ""; })(),
      nav: q("header nav a, .site-header nav a, body > nav a, nav[aria-label='Primary'] a").map((a) => [a.textContent.replace(/\s+/g, " ").trim(), a.getAttribute("href")]),
      steps: q("#how ol.steps li").length,
      stepHeads: q("#how ol.steps h3").map((h) => h.textContent.trim()),
      uses: q("#what article").length,
      heroCta: document.querySelector(".hero a.button")?.getAttribute("href") ?? null,
      heroHow: document.querySelector('.hero a[href^="#"]')?.getAttribute("href") ?? null,
      badgeFallback: !!stage?.querySelector('img[src$="v1-front.png"]'),
      badgeCanvas: !!stage?.querySelector("canvas#badge"),
      links: q("main a[href]").map((a) => a.getAttribute("href")),
      desc: document.querySelector('meta[name="description"]')?.getAttribute("content") ?? null,
      ogTitle: document.querySelector('meta[property="og:title"]')?.getAttribute("content") ?? null,
      footer: document.querySelector("footer")?.innerText ?? "",
      text: document.body.innerText,
      images: q("img").length,
      broken: q("img").filter((i) => !i.complete || i.naturalWidth === 0).map((i) => i.getAttribute("src")),
      cdn: q("script[src]").map((s) => s.getAttribute("src")).filter((s) => /^https?:/.test(s)),
    };
  });
  check("no console errors or failed requests at 1280 px", errors.length === 0, errors.join("; "));
  check('the H1 reads "Meet in person. Stay in cahoots."', facts.h1 === "Meet in person. Stay in cahoots.", JSON.stringify(facts.h1));
  check("the H1 is set in a serif", /serif/i.test(facts.h1Font) && !/sans-serif/i.test(facts.h1Font.split(",")[0]), facts.h1Font);
  const nav = [["How it works", "#how"], ["Docs", "docs/"], ["Journal", "blog/"], ["Open the app ↗", "app/"]];
  check("the primary nav reads How it works, Docs, Journal, Open the app", JSON.stringify(facts.nav) === JSON.stringify(nav), JSON.stringify(facts.nav));
  check("the hero's first button opens the app and its second points at #how", facts.heroCta === "app/" && facts.heroHow === "#how", `${facts.heroCta} ${facts.heroHow}`);
  check("three steps under #how: get together, check the words, take your circle with you", facts.steps === 3 && JSON.stringify(facts.stepHeads) === JSON.stringify(["Get together.", "Check the words.", "Take your circle with you."]), `${facts.steps}: ${facts.stepHeads.join(" | ")}`);
  check("three uses under #what", facts.uses === 3, `found ${facts.uses}`);
  check("the badge section shows the real board photo and has a canvas for the model", facts.badgeFallback && facts.badgeCanvas, `${facts.badgeFallback} ${facts.badgeCanvas}`);
  for (const l of ["docs/#/ceremony-guide", "docs/#/threat-model", "docs/#/diy-badge", "docs/#/web", "docs/#/adapters", "app/"]) check(`a link to ${l}`, facts.links.includes(l));
  check("the meta description and OG title are set", !!facts.desc && !!facts.ogTitle, `${facts.desc} | ${facts.ogTitle}`);
  check("the footer names the licence", /MIT \/ Apache-2\.0/.test(facts.footer), JSON.stringify(facts.footer.slice(0, 80)));
  check("no script loads from a CDN on the landing page", facts.cdn.length === 0, facts.cdn.join(", "));
  const insider = ["assurance", "grants", "sigil", "pad-seeded", "custodian"].filter((w) => new RegExp(`\\b${w}`, "i").test(facts.text));
  check("no assurance, grants, sigil, pad-seeded or custodian in the page text", insider.length === 0, insider.join(", "));
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
    const uses = [...document.querySelectorAll("#what article")].map((a) => a.getBoundingClientRect());
    const steps = [...document.querySelectorAll("#how ol.steps li")].map((a) => a.getBoundingClientRect());
    const h1 = document.querySelector("h1").getBoundingClientRect();
    return {
      usesStacked: uses.length === 3 && uses.every((r, i) => i === 0 || r.top >= uses[i - 1].bottom - 1),
      stepsStacked: steps.length === 3 && steps.every((r, i) => i === 0 || r.top >= steps[i - 1].bottom - 1),
      h1Fits: h1.left >= 0 && h1.right <= 390,
    };
  });
  check("at 390 px the headline fits the screen", pf.h1Fits);
  check("at 390 px the three uses stack", pf.usesStacked);
  check("at 390 px the three steps stack", pf.stepsStacked);
  if (process.env.SHOTS_DIR) {
    const shotDir = resolve(process.env.SHOTS_DIR);
    mkdirSync(shotDir, { recursive: true });
    await phone.screenshot({ path: join(shotDir, "phone.png"), fullPage: true });
  }
  await pctx.close();

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
    const ids = [...readFileSync(join(repo, "site/docs/catalog.js"), "utf8").matchAll(/\bid: '([a-z0-9-]+)'/g)].map((m) => m[1]);
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
    const need = ["readme", "apps", "ceremony-guide", "web", "status", "station", "hardware", "security", "use-cases", "glossary", "protocol", "chat", "threat-model", "tiers", "diy-badge", "v1-board", "manufacturing", "rp2350", "plan", "host", "adapters"];
    check("the catalog names the reading path and the references", need.every((i) => ids.includes(i)), `missing ${need.filter((i) => !ids.includes(i)).join(", ")}`);
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
