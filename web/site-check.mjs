// Check of the landing page in site/: serves site/ on a free port, loads index.html in headless Chrome
// and asserts the page's structure. One line per assertion; exits non-zero if any fails.
//   node web/site-check.mjs            (needs `npm install` in web/ and Google Chrome)
import { createRequire } from "node:module";
import { createServer } from "node:http";
import { createReadStream, existsSync, statSync } from "node:fs";
import { extname, join, resolve } from "node:path";
import { fileURLToPath } from "node:url";

const here = resolve(fileURLToPath(import.meta.url), "..");
const { chromium } = createRequire(join(here, "package.json"))("playwright");
const siteDir = resolve(here, "..", "site");

const types = { ".html": "text/html", ".js": "text/javascript", ".mjs": "text/javascript", ".css": "text/css", ".svg": "image/svg+xml", ".json": "application/json", ".png": "image/png", ".ico": "image/x-icon" };
const server = createServer((req, res) => {
  let p = join(siteDir, decodeURIComponent(new URL(req.url, "http://x").pathname));
  if (existsSync(p) && statSync(p).isDirectory()) p = join(p, "index.html");
  if (!p.startsWith(siteDir) || !existsSync(p)) { res.writeHead(404); res.end(); return; }
  res.writeHead(200, { "content-type": types[extname(p)] || "application/octet-stream" });
  createReadStream(p).pipe(res);
});
await new Promise((r) => server.listen(0, "127.0.0.1", r));
const base = `http://127.0.0.1:${server.address().port}/`;

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
  await page.waitForTimeout(300);

  const facts = await page.evaluate(() => {
    const q = (s) => [...document.querySelectorAll(s)];
    const card = document.getElementById("card");
    return {
      h1: document.querySelector("h1")?.textContent.trim() ?? null,
      steps: q("ol.steps li").length,
      apps: q(".app").length,
      ways: q(".way").length,
      front: !!card?.querySelector('img[src$="v1-front.png"]'),
      sigil: !!card?.querySelector('img[src$="sigil.svg"]'),
      soon: q(".soon").map((e) => e.textContent.trim()),
      three: q('script[src*="three"]').length + q('script[type="importmap"]').filter((s) => s.textContent.includes("three")).length,
      ids: ["what", "how", "ways", "card", "feed", "open"].filter((id) => !document.getElementById(id)),
      broken: q("img").filter((i) => i.complete && i.naturalWidth === 0).map((i) => i.getAttribute("src")),
    };
  });
  check("no console errors or failed requests at 1280 px", errors.length === 0, errors.join("; "));
  check('the H1 is "Get in cahoots."', facts.h1 === "Get in cahoots.", JSON.stringify(facts.h1));
  check("six ol.steps li", facts.steps === 6, `found ${facts.steps}`);
  check("five .app cards", facts.apps === 5, `found ${facts.apps}`);
  check("three .way columns", facts.ways === 3, `found ${facts.ways}`);
  check('#card has img[src$="v1-front.png"]', facts.front);
  check('#card has img[src$="sigil.svg"]', facts.sigil);
  check('every .soon label reads "being built"', facts.soon.length > 0 && facts.soon.every((t) => t === "being built"), JSON.stringify(facts.soon));
  check("no three.js script or import map", facts.three === 0, `found ${facts.three}`);
  check("sections what, how, ways, card, feed, open exist", facts.ids.length === 0, `missing ${facts.ids.join(", ")}`);
  check("every image loads", facts.broken.length === 0, facts.broken.join(", "));
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
  await pctx.close();
} finally {
  await browser.close();
  server.close();
}

console.log(failed ? `${failed} failed` : "all passed");
process.exit(failed ? 1 : 0);
