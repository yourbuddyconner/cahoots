// End-to-end check of the browser app: two isolated browser contexts run a full ceremony through the
// paste fallbacks (headless Chrome has no camera), then exchange a one-time-pad message.
//   node web/e2e.mjs            (needs the wasm build in web/app/pkg and playwright in video/node_modules)
// Screenshots land in web/e2e-out/.
import { createRequire } from "node:module";
import { createServer } from "node:http";
import { createReadStream, existsSync, mkdirSync, statSync } from "node:fs";
import { extname, join, resolve } from "node:path";
import { fileURLToPath } from "node:url";

const here = resolve(fileURLToPath(import.meta.url), "..");
const { chromium } = createRequire(join(here, "../video/package.json"))("playwright");
const appDir = join(here, "app");
const out = join(here, "e2e-out");
mkdirSync(out, { recursive: true });

const types = { ".html": "text/html", ".js": "text/javascript", ".mjs": "text/javascript", ".css": "text/css", ".svg": "image/svg+xml", ".json": "application/json", ".wasm": "application/wasm", ".png": "image/png" };
const server = createServer((req, res) => {
  let p = join(appDir, decodeURIComponent(new URL(req.url, "http://x").pathname));
  if (existsSync(p) && statSync(p).isDirectory()) p = join(p, "index.html");
  if (!p.startsWith(appDir) || !existsSync(p)) { res.writeHead(404); res.end(); return; }
  res.writeHead(200, { "content-type": types[extname(p)] || "application/octet-stream" });
  createReadStream(p).pipe(res);
});
await new Promise((r) => server.listen(0, "127.0.0.1", r));
const base = `http://127.0.0.1:${server.address().port}/`;

const browser = await chromium.launch({ channel: "chrome" });
const errors = [];
async function person(name, viewport) {
  const ctx = await browser.newContext({ viewport, deviceScaleFactor: 2, isMobile: viewport.width < 500, hasTouch: viewport.width < 500 });
  // Capture what the app copies, since headless clipboards are unreliable.
  await ctx.addInitScript(() => { navigator.clipboard.writeText = (t) => { window.__clip = t; return Promise.resolve(); }; });
  const page = await ctx.newPage();
  page.on("pageerror", (e) => errors.push(`${name}: ${e.message}`));
  page.on("console", (m) => { if (m.type() === "error") errors.push(`${name} console: ${m.text()}`); });
  await page.goto(base);
  await page.waitForSelector(".screen");
  let shot = 0;
  const snap = async (label) => { await page.waitForTimeout(260); await page.screenshot({ path: join(out, `${name}-${String(++shot).padStart(2, "0")}-${label}.png`) }); };
  const clip = () => page.evaluate(() => window.__clip);
  const paste = async (text) => { await page.fill("textarea.input", text); await page.click("text=Use this"); };
  const menu = async () => { await page.click('button[aria-label="More options"]'); await page.waitForSelector(".sheet-bg.on"); };
  const closeSheet = async () => { await page.keyboard.press("Escape"); await page.waitForTimeout(250); };
  return { page, snap, clip, paste, menu, closeSheet, name };
}

const people = [];
const onFail = async (e) => {
  console.error("failed:", e.message);
  for (const p of people) { try { await p.page.screenshot({ path: join(out, `${p.name}-FAIL.png`) }); console.error(p.name, "title:", await p.page.$eval(".bar .title", (x) => x.textContent), "status:", await p.page.$eval(".status", (x) => x.textContent).catch(() => "")); } catch {} }
  if (errors.length) console.error("page errors:\n" + errors.join("\n"));
  await browser.close(); server.close(); process.exit(1);
};
process.on("unhandledRejection", onFail);
process.on("uncaughtException", onFail);
const L = await person("leader", { width: 390, height: 844 });
const J = await person("joiner", { width: 1280, height: 900 });
people.push(L, J);

// Leader: set up and commit.
await L.snap("home");
await L.page.click("text=Start a ceremony");
await L.snap("new");
await L.page.fill("input[type=text]", "e2e-test");
await L.page.click("text=Continue");
await L.snap("pick");
await L.page.click("text=Make my contribution");
await L.page.waitForSelector("text=Step 1 of 3");
await L.snap("commit");
await L.menu(); await L.page.click("text=Copy as text"); const commitL = await L.clip(); await L.closeSheet();
if (!commitL?.startsWith("cahoots-commit:0:")) throw new Error(`leader commit text: ${commitL}`);

// Joiner: join from the leader's commitment, commit.
await J.snap("home");
await J.page.click("text=Join one");
await J.snap("join");
await J.page.click("text=No camera? Paste a code");
await J.paste(commitL);
await J.page.waitForSelector("text=Which seat are you?");
await J.snap("pick");
const seat1Disabled = await J.page.$eval('.seg button[data-v="0"]', (b) => b.disabled);
if (!seat1Disabled) throw new Error("seat 1 should be marked taken for the joiner");
await J.page.click("text=Make my contribution");
await J.page.waitForSelector("text=Step 2 of 3"); // the joiner already holds the leader's commitment, so it is in reveal
await J.snap("reveal");
await J.menu(); await J.page.click("text=Copy as text"); const revealJ = await J.clip(); await J.closeSheet();
if (!revealJ?.startsWith("cahoots-reveal:1:")) throw new Error(`joiner reveal text: ${revealJ}`);

// Leader takes the joiner's reveal as commitment plus reveal. With two people that completes the
// leader's ceremony at once, so the leader lands on the check words and shares its reveal from there.
await L.menu(); await L.page.click("text=Paste a code"); await L.paste(revealJ);
await L.page.waitForSelector('button:text-is("They have scanned it")');
await L.closeSheet();
await L.snap("reveal-done");
await L.menu(); await L.page.click(".sheet >> text=Copy as text"); const revealL = await L.clip(); await L.closeSheet();
if (!revealL?.startsWith("cahoots-reveal:0:")) throw new Error(`leader reveal text: ${revealL}`);

// Joiner takes the leader's reveal and completes; both confirm the others have scanned.
await J.menu(); await J.page.click("text=Paste a code"); await J.paste(revealL);
await J.page.waitForSelector('button:text-is("They have scanned it")');
await J.closeSheet();
await J.page.click('button:text-is("They have scanned it")');
await J.page.waitForSelector("text=Step 3 of 3");
await J.snap("words");
await L.page.click('button:text-is("They have scanned it")');
await L.page.waitForSelector("text=Step 3 of 3");
await L.snap("words");
const wordsOf = (p) => p.$$eval(".words div", (els) => els.map((e) => e.textContent.replace(/^\d+/, "").trim()).join(" "));
const wL = await wordsOf(L.page), wJ = await wordsOf(J.page);
if (wL !== wJ) throw new Error(`check words differ: ${wL} / ${wJ}`);
console.log("check words:", wL);
await L.page.click("text=They match"); await J.page.click("text=They match");
await L.page.waitForSelector(".budget"); await J.page.waitForSelector(".budget");
await L.snap("hub"); await J.snap("hub");

// Hello words: what the leader says is what the joiner expects, both move together, and the number survives a reload.
await L.page.click("text=Hello words"); await L.page.waitForSelector(".hello");
await J.page.click("text=Hello words"); await J.page.waitForSelector(".hello");
await L.snap("hello");
const hello = (p) => p.$$eval(".hello .pair", (els) => els.map((e) => e.textContent.trim()));
const [lMine, lTheirs] = await hello(L.page);
const [jMine, jTheirs] = await hello(J.page);
if (lMine !== jTheirs || lTheirs !== jMine) throw new Error(`hello words disagree: leader ${lMine}/${lTheirs}, joiner ${jMine}/${jTheirs}`);
if (lMine === lTheirs || lMine.split(" ").length !== 2) throw new Error(`hello words look wrong: ${lMine}`);
console.log("hello words:", lMine, "/", lTheirs);
await L.page.click('button:text-is("Next hello")'); await J.page.click('button:text-is("Next hello")');
await L.page.waitForSelector('.bar .title small:has-text("#2")'); await J.page.waitForSelector('.bar .title small:has-text("#2")');
const [lMine2, lTheirs2] = await hello(L.page);
const [jMine2, jTheirs2] = await hello(J.page);
if (lMine2 !== jTheirs2 || lTheirs2 !== jMine2) throw new Error("hello words disagree after next");
if (lMine2 === lMine) throw new Error("next hello did not change the words");
await L.snap("hello-next");
await J.page.reload(); await J.page.waitForSelector(".hello");
const [jMine3] = await hello(J.page);
if (jMine3 !== jMine2) throw new Error("hello number did not survive a reload");
await L.page.click('button[aria-label="Back"]'); await L.page.waitForSelector(".budget");
await J.page.click('button[aria-label="Back"]'); await J.page.waitForSelector(".budget");

// Leader sends a one-time-pad message as text; joiner reads it.
await L.page.click("text=Send a message");
await L.snap("send");
await L.page.fill("textarea.input", "meet at the trailhead at 9");
await L.page.click('button:text-is("Encrypt")');
await L.page.waitForSelector('.bar .title:has-text("Encrypted")');
await L.snap("encrypted");
await L.page.click('button:text-is("Copy as text")'); const wire = await L.clip();
await J.page.click("text=Read a message");
await J.page.click("text=No camera? Paste the text");
await J.paste(wire);
await J.page.waitForSelector('.bar .title:has-text("Decrypted")');
await J.snap("decrypted");
const plain = await J.page.$eval(".plain", (e) => e.textContent);
if (plain !== "meet at the trailhead at 9") throw new Error(`decrypted: ${plain}`);

// Replay is refused.
await J.page.click("text=Read another");
await J.page.click("text=No camera? Paste the text");
await J.paste(wire);
await J.page.waitForSelector("text=Possible pad reuse");
await J.snap("replay-refused");

// Home shows the cahoot with its budget.
await L.page.goto(base);
await L.page.waitForSelector(".row-item");
await L.snap("home-with-cahoot");

await browser.close();
server.close();
if (errors.length) { console.error("page errors:\n" + errors.join("\n")); process.exit(1); }
console.log("e2e ok: two-member ceremony, OTP message, replay refused. Screenshots in", out);
