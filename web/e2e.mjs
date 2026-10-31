// End-to-end check of the browser app: two isolated browser contexts run a full ceremony through the
// paste fallbacks (headless Chrome has no camera), exchange a one-time-pad message, then chat through
// a Numbers Station started here. Then three run a codes-only ceremony, and two hold a ceremony
// through the relay (the station started here stands in for the default relay) and chat.
//   node web/e2e.mjs            (needs the wasm build in web/app/pkg, `npm install` in web/, and
//                                `cargo build -p cahoots-station`)
// Screenshots land in web/e2e-out/.
import { createRequire } from "node:module";
import { spawn } from "node:child_process";
import { createServer } from "node:http";
import { createReadStream, existsSync, mkdirSync, mkdtempSync, statSync } from "node:fs";
import { extname, join, resolve } from "node:path";
import { fileURLToPath } from "node:url";

const here = resolve(fileURLToPath(import.meta.url), "..");
const { chromium } = createRequire(join(here, "package.json"))("playwright");
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

// A relay of our own. The app ships pointing at relay.cahoots.network; the test never reaches it.
const stationBin = join(process.env.CARGO_TARGET_DIR || join(here, "..", "target"), "debug", "cahoots-station");
if (!existsSync(stationBin)) { console.error("build the relay first: cargo build -p cahoots-station"); process.exit(1); }
const freePort = await new Promise((r) => { const s = createServer(); s.listen(0, "127.0.0.1", () => { const p = s.address().port; s.close(() => r(p)); }); });
const stationUrl = `http://127.0.0.1:${freePort}`;
const station = spawn(stationBin, ["--listen", `127.0.0.1:${freePort}`, "--db", join(mkdtempSync(join(out, "relay-")), "station.db")], { stdio: ["ignore", "ignore", "inherit"] });
for (let i = 0; ; i++) {
  try { if ((await fetch(`${stationUrl}/healthz`)).ok) break; } catch {}
  if (i > 100) { console.error("the relay did not start"); process.exit(1); }
  await new Promise((r) => setTimeout(r, 100));
}

const browser = await chromium.launch({ channel: "chrome" });
const errors = [];
async function person(name, viewport, showWelcome = false) {
  const ctx = await browser.newContext({ viewport, deviceScaleFactor: 2, isMobile: viewport.width < 500, hasTouch: viewport.width < 500 });
  // Capture what the app copies, since headless clipboards are unreliable.
  await ctx.addInitScript(({ skipWelcome }) => { if (skipWelcome) localStorage.setItem("cahoots.welcomeSeen", "1"); navigator.clipboard.writeText = (t) => { window.__clip = t; return Promise.resolve(); }; }, { skipWelcome: !showWelcome });
  await ctx.route("https://relay.cahoots.network/**", (r) => r.abort());
  const page = await ctx.newPage();
  page.on("pageerror", (e) => errors.push(`${name}: ${e.message}`));
  // The app's first poll goes to the shipped default relay, which this test blocks; that failed load is expected.
  page.on("console", (m) => { if (m.type() === "error" && !(m.location()?.url || "").includes("relay.cahoots.network")) errors.push(`${name} console: ${m.text()}`); });
  await page.goto(base);
  await page.waitForSelector(".screen");
  let shot = 0;
  const snap = async (label) => { await page.waitForTimeout(260); await page.screenshot({ path: join(out, `${name}-${String(++shot).padStart(2, "0")}-${label}.png`), timeout: 10000 }); };
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
  await browser.close(); server.close(); station.kill(); process.exit(1);
};
process.on("unhandledRejection", onFail);
process.on("uncaughtException", onFail);
const wordsOf = (p) => p.$$eval(".words div", (els) => els.map((e) => e.textContent.replace(/^\d+/, "").trim()).join(" "));
// First-use guide: completion, skipping, reopening, and preserving the requested route.
if (!process.argv.includes("--ceremony-only")) {
  const W = await person("welcome", { width: 390, height: 844 }, true);
  people.push(W);
  await W.page.goto(`${base}#/new`);
  for (let index = 0; index < 4; index++) {
    await W.page.locator(`[data-welcome-page="${index}"]`).waitFor();
    await W.snap(`guide-${index}`);
    await W.page.getByRole("button", { name: index === 3 ? "Get started" : "Continue", exact: true }).click();
  }
  await W.page.getByRole("heading", { name: "Bring your people together.", exact: true }).waitFor();
  if (!W.page.url().endsWith("#/new")) throw new Error("Onboarding lost the requested route");
  await W.page.reload();
  await W.page.waitForSelector(".screen:not(.welcome-screen)");
  if (await W.page.locator(".welcome-screen").count()) throw new Error("Completed welcome returned");
  await W.page.goto(base);
  await W.menu();
  await W.page.getByRole("button", { name: "How Cahoots works", exact: true }).click();
  await W.page.locator('[data-welcome-page="0"]').waitFor();
  await W.page.getByRole("button", { name: "Continue", exact: true }).click();
  await W.page.getByRole("button", { name: "Back", exact: true }).click();
  await W.page.locator('[data-welcome-page="0"]').waitFor();
  await W.page.getByRole("button", { name: "Close guide", exact: true }).click();
  await W.page.waitForSelector(".entry-screen");
  const S = await person("welcome-skip", { width: 375, height: 667 }, true);
  people.push(S);
  await S.page.getByRole("button", { name: "Skip for now", exact: true }).click();
  await S.page.reload();
  await S.page.waitForSelector(".entry-screen");
  if (await S.page.locator(".welcome-screen").count()) throw new Error("Skipped welcome returned");
  await S.page.context().close();
  await W.page.context().close();
}
if (process.argv.includes("--welcome-only")) {
  if (errors.length) throw new Error(errors.join("\n"));
  console.log("Welcome checks passed: completion, skip, persistence, reopening, and route preservation.");
  await browser.close(); server.close(); station.kill();
  process.exit(0);
}
if (!process.argv.includes("--ceremony-only")) {
const L = await person("leader", { width: 390, height: 844 });
const J = await person("joiner", { width: 1280, height: 900 });
people.push(L, J);

// Leader: set up and commit.
await L.page.waitForSelector('.entry-screen', { timeout: 3000 });
if (await L.page.getByRole('link', { name: 'Explore', exact: true }).count()) throw new Error('Entry should not split the app into Circles and Explore');
await L.snap("home");
await L.page.click("text=Create a circle");
await L.snap("new");
await L.page.fill("input[type=text]", "e2e-test");
await L.page.click("text=Codes only");
await L.page.click("text=Continue");
await L.snap("pick");
await L.page.click("text=Join this circle");
await L.page.waitForSelector("text=Step 1 of 4");
await L.snap("commit");
await L.menu(); await L.page.click("text=Copy as text"); const commitL = await L.clip(); await L.closeSheet();
if (!commitL?.startsWith("cahoots-commit:0:")) throw new Error(`leader commit text: ${commitL}`);

// Joiner: join from the leader's commitment, commit.
await J.snap("home");
await J.page.click("text=Join a circle");
await J.snap("join");
await J.page.click("text=No camera? Paste a code");
await J.paste(commitL);
await J.page.waitForSelector("text=Which seat are you?");
await J.snap("pick");
const seat1Disabled = await J.page.$eval('.seg button[data-v="0"]', (b) => b.disabled);
if (!seat1Disabled) throw new Error("seat 1 should be marked taken for the joiner");
await J.page.click("text=Join this circle");
await J.page.waitForSelector("text=Step 2 of 4"); // the joiner already holds the leader's commitment, so it is in reveal
await J.snap("reveal");
if (!(await J.page.getByRole('button', { name: 'Scan the remaining step 2 codes to continue', exact: true }).isDisabled())) throw new Error('Cannot confirm sharing before collecting the missing reveal');
await J.menu(); await J.page.click("text=Copy as text"); const revealJ = await J.clip(); await J.closeSheet();
if (!revealJ?.startsWith("cahoots-reveal:1:")) throw new Error(`joiner reveal text: ${revealJ}`);

// Regression: receiving the joiner's reveal must not hide the leader's reveal.
// The core can already sign, but the other screen still needs the leader's step-2 code.
await L.menu(); await L.page.click("text=Paste a code"); await L.paste(revealJ);
await L.page.waitForSelector("text=Step 2 of 4", { timeout: 3000 });
await L.closeSheet();
await L.page.waitForSelector('button:text-is("They have scanned my step 2 code"):enabled');
await L.page.waitForSelector('.half:visible .cap:has-text("seat 1 · step 2")');
await L.snap("reveal-handoff");
await L.page.click('button:text-is("They have scanned my step 2 code")');
await L.page.waitForSelector("text=Step 3 of 4");
await L.snap("sign");
await L.menu(); await L.page.click(".sheet >> text=Copy my step 2 code"); const revealL = await L.clip(); await L.closeSheet();
if (!revealL?.startsWith("cahoots-reveal:0:")) throw new Error(`leader reveal text: ${revealL}`);
await L.menu(); await L.page.click(".sheet >> text=Copy as text"); const attestL = await L.clip(); await L.closeSheet();
if (!attestL?.startsWith("cahoots-attest:0:")) throw new Error(`leader signature text: ${attestL}`);

// Joiner gets the leader's signature before its reveal: it is kept until the reveal arrives.
await J.menu(); await J.page.click("text=Paste a code"); await J.paste(attestL);
await J.page.waitForSelector("text=Seat 1 is already on step 3");
if (!(await J.page.getByRole('button', { name: 'Scan the remaining step 2 codes to continue', exact: true }).isDisabled())) throw new Error('An early signature must not bypass a missing reveal');
await J.closeSheet();
await J.menu(); await J.page.click("text=Paste a code"); await J.paste(revealL);
// Even with an early signature buffered, the joiner must confirm sharing its reveal.
await J.page.waitForSelector('button:text-is("They have scanned my step 2 code"):enabled');
await J.closeSheet();
await J.page.waitForSelector("text=Step 2 of 4");
await J.page.click('button:text-is("They have scanned my step 2 code")');
await J.page.waitForSelector('button:text-is("They have scanned it")');
await J.snap("sign-done");
await J.menu(); await J.page.click(".sheet >> text=Copy as text"); const attestJ = await J.clip(); await J.closeSheet();
if (!attestJ?.startsWith("cahoots-attest:1:")) throw new Error(`joiner signature text: ${attestJ}`);

// Leader takes the joiner's signature and completes; both confirm the other has scanned.
await L.menu(); await L.page.click("text=Paste a code"); await L.paste(attestJ);
await L.page.waitForSelector('button:text-is("They have scanned it")');
await L.closeSheet();
await L.snap("sign-done");
await J.page.click('button:text-is("They have scanned it")');
await J.page.waitForSelector("text=Step 4 of 4");
await J.snap("words");
await L.page.click('button:text-is("They have scanned it")');
await L.page.waitForSelector("text=Step 4 of 4");
await L.snap("words");
const wL = await wordsOf(L.page), wJ = await wordsOf(J.page);
if (wL !== wJ) throw new Error(`check words differ: ${wL} / ${wJ}`);
console.log("check words:", wL);
await L.page.click("text=They match"); await J.page.click("text=They match");
await L.page.waitForSelector('.circle-dashboard .composer', { timeout: 3000 });
await J.page.waitForSelector('.circle-dashboard .composer', { timeout: 3000 });
await J.page.waitForSelector('.dashboard-rail');
await J.page.waitForSelector('.dashboard-aside');
await L.page.getByRole('link', { name: 'People', exact: true }).click();
await L.page.getByRole('heading', { name: 'The people in this circle.', exact: true }).waitFor();
await L.page.getByRole('button', { name: 'Name this person', exact: true }).click();
await L.page.getByLabel('Name on this device', { exact: true }).fill('Jordan');
await L.page.getByRole('button', { name: 'Save name', exact: true }).click();
await L.page.getByRole('heading', { name: 'Jordan', exact: true }).waitFor();
await L.snap('people');
await L.page.reload();
await L.page.getByRole('heading', { name: 'Jordan', exact: true }).waitFor();
await L.page.getByRole('button', { name: 'Back', exact: true }).click();
await L.page.waitForSelector('.circle-dashboard .composer');
await L.page.getByLabel('Message', { exact: true }).fill('Keep this draft while I check the network');
for (const p of [L, J]) {
  await p.page.getByRole('link', { name: 'Network', exact: true }).click();
  await p.page.getByRole('heading', { name: 'Network', exact: true }).waitFor();
  await p.page.getByRole('heading', { name: 'Connections aren’t shown here yet', exact: true }).waitFor();
  await p.snap('network');
  await p.page.getByRole('link', { name: 'Conversation', exact: true }).click();
  await p.page.waitForSelector('.composer');
}
if (await L.page.getByLabel('Message', { exact: true }).inputValue() !== 'Keep this draft while I check the network') throw new Error('Switching circle tools discarded the unsent draft');
await L.page.setViewportSize({ width: 1280, height: 900 });
await L.page.waitForSelector('.dashboard-aside');
if (await L.page.getByLabel('Message', { exact: true }).inputValue() !== 'Keep this draft while I check the network') throw new Error('Resizing discarded the draft');
await L.page.setViewportSize({ width: 390, height: 844 });
await L.page.getByLabel('Message', { exact: true }).fill('');
await L.snap("hub"); await J.snap("hub");

// The saved circle is state version 5 and carries the seat key, the signed manifest and the circle root.
const saved = (p) => p.evaluate(() => new Promise((res, rej) => {
  const req = indexedDB.open("cahoots");
  req.onsuccess = () => { const g = req.result.transaction("circles").objectStore("circles").getAll(); g.onsuccess = () => res(g.result.map((r) => r.state[0])); g.onerror = () => rej(g.error); };
  req.onerror = () => rej(req.error);
}));
for (const p of [L, J]) { const v = await saved(p.page); if (v.length !== 1 || v[0] !== 5) throw new Error(`${p.name} saved state versions: ${v}`); }

// Hello words: what the leader says is what the joiner expects, both move together, and the number survives a reload.
await L.page.getByRole('button', { name: 'Circle tools', exact: true }).click();
await L.page.getByRole('link', { name: 'Hello words', exact: true }).click();
await L.page.waitForSelector('.hello');
await J.page.click("text=Hello words"); await J.page.waitForSelector(".hello");
await L.snap("hello");
const hello = (p) => p.$$eval(".hello .pair", (els) => els.map((e) => e.textContent.trim()));
const [lMine, lTheirs] = await hello(L.page);
const [jMine, jTheirs] = await hello(J.page);
if (lMine !== jTheirs || lTheirs !== jMine) throw new Error(`hello words disagree: leader ${lMine}/${lTheirs}, joiner ${jMine}/${jTheirs}`);
if (lMine === lTheirs || lMine.split(" ").length !== 2) throw new Error(`hello words look wrong: ${lMine}`);
console.log("hello words:", lMine, "/", lTheirs);
L.page.once('dialog', dialog => dialog.dismiss());
await L.page.click('button:text-is("Next hello")');
if ((await hello(L.page))[0] !== lMine) throw new Error('Canceling next hello changed the words');
L.page.once('dialog', dialog => dialog.accept());
J.page.once('dialog', dialog => dialog.accept());
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
await L.page.click('button[aria-label="Back"]'); await L.page.waitForSelector(".circle-dashboard");
await J.page.click('button[aria-label="Back"]'); await J.page.waitForSelector(".circle-dashboard");


// Chat: the app opens on its default relay; each member points it at the one started here, then they talk.
const setRelay = async (p) => {
  await p.page.waitForSelector(".circle-dashboard .chatlist");
  await p.menu(); await p.page.click('button:text-is("Relay address")');
  await p.page.fill('input[type="url"]', stationUrl);
  await p.page.click('button:text-is("Save and connect")');
  await p.page.waitForSelector('.status:has-text("Connected")');
};
await setRelay(L); await setRelay(J);
// Relay configuration must not discard the people names saved in the same settings row.
await L.page.getByRole('link', { name: 'People', exact: true }).click();
await L.page.getByRole('heading', { name: 'Jordan', exact: true }).waitFor({ timeout: 3000 });
await L.page.getByRole('button', { name: 'Back', exact: true }).click();
await L.page.waitForSelector('.status:has-text("Connected")');
await L.snap("chat-empty");
let releasePost;
let signalPost;
const postStarted = new Promise(resolve => { signalPost = resolve; });
const postRelease = new Promise(resolve => { releasePost = resolve; });
await L.page.route(`${stationUrl}/v1/circles/**/messages`, async route => {
  if (route.request().method() === 'POST') { signalPost(); await postRelease; }
  await route.continue();
});
await L.page.fill('input[placeholder="Message"]', "hello from the ring");
await L.page.click('button:text-is("Send")');
await postStarted;
await L.page.getByRole('link', { name: 'People', exact: true }).click();
await L.page.getByRole('link', { name: 'Conversation', exact: true }).click();
if (!(await L.page.getByRole('button', { name: /^Send(?:ing…)?$/ }).isDisabled())) throw new Error('Returning to a pending send enabled duplicate submission');
releasePost();
await L.page.waitForSelector('.bubble.out:has-text("hello from the ring")');
await J.page.waitForSelector('.bubble.in:has-text("hello from the ring")', { timeout: 15000 });
await J.page.fill('input[placeholder="Message"]', "reading you");
await J.page.keyboard.press("Enter");
await L.page.waitForSelector('.bubble.in:has-text("reading you")', { timeout: 15000 });
await L.page.waitForSelector('.bubble.in .meta:has-text("Jordan")');
const rejected = await L.page.$$(".bubble.rejected");
if (rejected.length) throw new Error(`${rejected.length} rejected chat messages`);
console.log("chat exchanged; capturing chat screens");
await L.snap("chat"); await J.snap("chat");
const headRes = await (await fetch(`${stationUrl}/v1/head`)).json();
if (headRes.seq !== 1) throw new Error(`relay head seq ${headRes.seq}, expected 1 (two messages)`);
// Leader sends a one-time-pad message as text; joiner reads it.
await L.page.getByRole('button', { name: 'Circle tools', exact: true }).click();
await L.page.getByRole('link', { name: 'Send a message', exact: true }).click();
await L.snap("send");
await L.page.fill("textarea.input", "meet at the trailhead at 9");
await L.page.click('button:text-is("Encrypt")');
await L.page.waitForSelector('.bar .title:has-text("Encrypted")');
await L.snap("encrypted");
await L.page.click('button:text-is("Copy as text")'); const wire = await L.clip();
await J.page.getByRole('link', { name: 'Read a message', exact: true }).click();
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

// Home shows the circle with its budget.
await L.page.goto(base);
await L.page.waitForSelector(".row-item");
await L.snap("home-with-circle");
await L.page.locator('.entry-list .row-item').first().click();
await L.page.getByRole('button', { name: 'Circle tools', exact: true }).click();
await L.page.getByRole('button', { name: 'Storage & security', exact: true }).click();
L.page.once('dialog', dialog => dialog.accept());
await L.page.getByRole('button', { name: 'Remove circle from this browser', exact: true }).click();
await L.page.waitForSelector('.entry-screen');
await L.page.waitForTimeout(250);
if (await L.page.locator('.sheet-bg.on').count()) throw new Error('Removing a circle left a modal over the entry screen');

}

// Three participants: every incoming reveal and the outgoing handoff are separate.
console.log("starting three-member ceremony");
const group = [];
for (let i = 0; i < 3; i++) {
  const p = await person(`group-${i + 1}`, { width: i === 0 ? 375 : 390, height: i === 0 ? 667 : 844 });
  group.push(p); people.push(p);
}
const copyStage = async p => { await p.menu(); await p.page.getByRole('button', { name: 'Copy as text', exact: true }).click(); const text = await p.clip(); await p.closeSheet(); return text; };
const receive = async (p, text) => { await p.menu(); await p.page.getByRole('button', { name: 'Paste a code', exact: true }).click(); await p.paste(text); await p.closeSheet(); };
await group[0].page.getByRole('button', { name: 'Create a circle', exact: true }).click();
await group[0].page.fill('input[type=text]', 'three members');
await group[0].page.getByRole('button', { name: 'More', exact: true }).click();
await group[0].page.locator('.seg button').filter({ hasText: 'Small' }).click();
await group[0].page.getByRole('button', { name: 'Codes only', exact: true }).click();
await group[0].page.getByRole('button', { name: 'Continue', exact: true }).click();
await group[0].page.getByRole('button', { name: 'Join this circle', exact: true }).click();
await group[0].page.waitForSelector('text=Step 1 of 4');
const commits = [await copyStage(group[0])];
for (let i = 1; i < 3; i++) {
  const p = group[i];
  await p.page.getByRole('button', { name: 'Join a circle', exact: true }).click();
  await p.page.getByRole('button', { name: 'No camera? Paste a code', exact: true }).click();
  await p.paste(commits[0]);
  await p.page.waitForSelector('text=Which seat are you?');
  await p.page.locator(`.seg button[data-v="${i}"]`).click();
  await p.page.getByRole('button', { name: 'Join this circle', exact: true }).click();
  await p.page.waitForSelector('text=Step 1 of 4');
  commits.push(await copyStage(p));
}
await receive(group[0], commits[1]);
await receive(group[0], commits[2]);
await receive(group[1], commits[2]);
await receive(group[2], commits[1]);
const reveals = [];
for (const p of group) {
  await p.page.waitForSelector('text=Step 2 of 4');
  reveals.push(await copyStage(p));
}
await receive(group[0], reveals[1]);
if (!(await group[0].page.getByRole('button', { name: 'Scan the remaining step 2 codes to continue', exact: true }).isDisabled())) throw new Error('Three-member ceremony advanced with one reveal missing');
await group[0].page.waitForSelector('text=Received 1 of 2 reveals');
await receive(group[0], reveals[2]);
for (let i = 1; i < 3; i++) for (let j = 0; j < 3; j++) if (i !== j) await receive(group[i], reveals[j]);
for (const p of group) {
  await p.page.waitForSelector('text=Received 2 of 2 reveals');
  await p.page.waitForSelector('button:text-is("Everyone has scanned my step 2 code"):enabled');
  await p.page.waitForSelector('text=Step 2 of 4');
  const layout = await p.page.evaluate(() => {
    const action = document.querySelector('.actions button').getBoundingClientRect();
    const qr = document.querySelector('.half .qr svg').getBoundingClientRect();
    const header = document.querySelector('.bar').getBoundingClientRect();
    return { overflow: document.documentElement.scrollWidth > innerWidth, actionVisible: action.top >= 0 && action.bottom <= innerHeight, headerVisible: header.top >= 0 && header.height >= 50, screenScroll: document.querySelector('.screen').scrollTop, windowScroll: scrollY, qrVisible: qr.top >= header.bottom && qr.bottom <= action.top && qr.width >= 180 && qr.height >= 180, qr: { top: qr.top, bottom: qr.bottom, width: qr.width, height: qr.height }, actionTop: action.top };
  });
  if (layout.overflow || !layout.actionVisible || !layout.qrVisible || !layout.headerVisible) throw new Error(`Ceremony layout inaccessible: ${JSON.stringify(layout)}`);
  // Decode a rendered QR frame at its displayed physical size, not a mocked code.
  const readable = await p.page.evaluate(async () => {
    const { qr_decode_rgba } = await import('./pkg/cahoots_wasm.js');
    const svg = document.querySelector('.half .qr svg');
    const rect = svg.getBoundingClientRect();
    const image = new Image();
    const loaded = new Promise((resolve, reject) => { image.onload = resolve; image.onerror = reject; });
    image.src = 'data:image/svg+xml;charset=utf-8,' + encodeURIComponent(svg.outerHTML);
    await loaded;
    const canvas = document.createElement('canvas');
    canvas.width = canvas.height = Math.floor(Math.min(rect.width, rect.height) * devicePixelRatio);
    const ctx = canvas.getContext('2d');
    ctx.drawImage(image, 0, 0, canvas.width, canvas.height);
    return !!qr_decode_rgba(canvas.width, canvas.height, ctx.getImageData(0, 0, canvas.width, canvas.height).data);
  });
  if (!readable) throw new Error('Displayed reveal QR does not decode');
  await p.snap('reveal-handoff');
  await p.page.getByRole('button', { name: 'Scan step 2 codes', exact: true }).click();
  const cameraFits = await p.page.evaluate(() => {
    const camera = document.querySelector('.half:last-child .cam').getBoundingClientRect();
    const action = document.querySelector('.actions').getBoundingClientRect();
    return camera.height >= 140 && camera.bottom <= action.top;
  });
  if (!cameraFits) throw new Error('Compact camera preview is obscured');
  await p.page.getByRole('button', { name: 'Show my step 2 code', exact: true }).click();
  await p.page.getByRole('button', { name: 'Everyone has scanned my step 2 code', exact: true }).click();
  await p.page.waitForSelector('text=Step 3 of 4');
  const signatureShown = await p.page.evaluate(() => {
    const qr = document.querySelector('.half .qr svg').getBoundingClientRect();
    return qr.width >= 180 && qr.height >= 180 && qr.bottom <= innerHeight;
  });
  if (!signatureShown) throw new Error('Signature collection screen has no readable QR');
  await p.snap('collect-signatures');
}
const signatures = [];
for (const p of group) signatures.push(await copyStage(p));
for (let i = 0; i < 3; i++) for (let j = 2; j >= 0; j--) if (i !== j) await receive(group[i], signatures[j]);
const groupWords = [];
for (const p of group) {
  const signatureFits = await p.page.evaluate(() => {
    const qr = document.querySelector('.qr svg').getBoundingClientRect();
    const action = document.querySelector('.actions').getBoundingClientRect();
    return qr.top >= document.querySelector('.bar').getBoundingClientRect().bottom && qr.bottom <= action.top;
  });
  if (!signatureFits) throw new Error('Signature code is obscured on a compact phone');
  await p.snap('signature-handoff');
  await p.page.getByRole('button', { name: 'Everyone has scanned it', exact: true }).click();
  await p.page.waitForSelector('text=Step 4 of 4');
  groupWords.push(await wordsOf(p.page));
}
if (new Set(groupWords).size !== 1) throw new Error('Three-member check words differ');
console.log('three-member ceremony ok: partial reveals, explicit handoffs, signatures and matching words; compact-phone actions visible.');

// Through the relay: the host shows one code, the joiner reads it (pasted here), picks a seat,
// shows its own code to the host, and the rounds go through the relay. The station started here answers for the default relay.
console.log("starting a ceremony through the relay");
const viaStation = async (p) => p.page.context().route("https://relay.cahoots.network/**", async (r) => {
  const u = new URL(r.request().url());
  await r.fulfill({ response: await r.fetch({ url: stationUrl + u.pathname + u.search }) });
});
const H = await person("relay-host", { width: 390, height: 844 });
const G = await person("relay-guest", { width: 390, height: 844 });
people.push(H, G);
await viaStation(H); await viaStation(G);
await H.page.getByRole('button', { name: 'Create a circle', exact: true }).click();
await H.page.fill('input[type=text]', 'through the relay');
if (!(await H.page.locator('.seg button.on').filter({ hasText: 'Through the relay' }).count())) throw new Error('Through the relay should be the default');
await H.page.getByRole('button', { name: 'Continue', exact: true }).click();
await H.page.getByRole('button', { name: 'Join this circle', exact: true }).click();
await H.page.waitForSelector('text=Show this code to everyone joining');
await H.snap('relay-offer');
const offerFits = await H.page.evaluate(() => { const qr = document.querySelector('.qr svg').getBoundingClientRect(); return qr.width >= 180 && qr.height >= 180 && qr.bottom <= innerHeight; });
if (!offerFits) throw new Error('The join code does not fit the screen');
await H.menu(); await H.page.getByRole('button', { name: 'Copy the code', exact: true }).click(); const offer = await H.clip(); await H.closeSheet();
if (!offer?.startsWith('cahoots:ceremony:v2:')) throw new Error(`offer text: ${offer}`);
// The code on the host's screen reads back as the offer, at its displayed size.
const shown = await H.page.evaluate(async () => {
  const { qr_decode_rgba } = await import('./pkg/cahoots_wasm.js');
  const svg = document.querySelector('.half .qr svg');
  const rect = svg.getBoundingClientRect();
  const image = new Image();
  const loaded = new Promise((resolve, reject) => { image.onload = resolve; image.onerror = reject; });
  image.src = 'data:image/svg+xml;charset=utf-8,' + encodeURIComponent(svg.outerHTML);
  await loaded;
  const canvas = document.createElement('canvas');
  canvas.width = canvas.height = Math.floor(Math.min(rect.width, rect.height) * devicePixelRatio);
  const ctx = canvas.getContext('2d');
  ctx.drawImage(image, 0, 0, canvas.width, canvas.height);
  return qr_decode_rgba(canvas.width, canvas.height, ctx.getImageData(0, 0, canvas.width, canvas.height).data);
});
if (shown !== offer) throw new Error(`the code on screen reads as ${shown}`);
await G.page.getByRole('button', { name: 'Join a circle', exact: true }).click();
await G.page.waitForSelector("text=Scan the host's code");
await G.page.getByRole('button', { name: 'No camera? Paste a code', exact: true }).click();
await G.paste(offer);
await G.page.waitForSelector('text=Which seat are you?');
if (!(await G.page.$eval('.seg button[data-v="0"]', (b) => b.disabled))) throw new Error('seat 1 is the host\'s');
await G.snap('relay-pick');
await G.page.getByRole('button', { name: 'Join this circle', exact: true }).click();
// The joiner shows its code to the host, and the host scans it (pasted here, as the offer was).
await G.page.waitForSelector('.bar .title:has-text("Show this to the host")', { timeout: 15000 });
await G.snap('relay-join-code');
await G.menu(); await G.page.getByRole('button', { name: 'Copy my join code', exact: true }).click(); const joined = await G.clip(); await G.closeSheet();
if (!joined?.startsWith('cahoots:joined:v1:')) throw new Error(`join code text: ${joined}`);
await H.page.waitForSelector('text=Seat 2 committed but was not scanned here.', { timeout: 15000 });
await H.snap('relay-unscanned');
await H.menu(); await H.page.getByRole('button', { name: 'Paste a join code', exact: true }).click(); await H.paste(joined); await H.closeSheet();
await H.page.waitForSelector('.bar .title:has-text("Everyone is scanned")', { timeout: 15000 });
await H.snap('relay-everyone-scanned');
await H.page.getByRole('button', { name: 'Continue', exact: true }).click();
for (const p of [H, G]) await p.page.waitForSelector('text=Step 4 of 4', { timeout: 20000 });
const wH = await wordsOf(H.page), wG = await wordsOf(G.page);
if (wH !== wG) throw new Error(`relay check words differ: ${wH} / ${wG}`);
console.log("relay check words:", wH);
// The ceremony went through the station started here: each member's setup, its reveal in two
// parts (a 64 KiB pad) and its signature. The public listing does not show the channel.
const channelHex = Buffer.from(offer.slice('cahoots:ceremony:v2:'.length), 'base64url').subarray(0, 16).toString('hex');
const held = await (await fetch(`${stationUrl}/v1/circles/${channelHex}/messages`)).json();
if (held.entries.length !== 8) throw new Error(`the station holds ${held.entries.length} ceremony messages, expected 8`);
const listed = await (await fetch(`${stationUrl}/v1/circles`)).json();
if (listed.circles.some((c) => c.circle_id === channelHex)) throw new Error('the ceremony channel is listed publicly');
await G.snap('relay-words');
await H.page.click("text=They match"); await G.page.click("text=They match");
for (const p of [H, G]) await p.page.waitForSelector('.status:has-text("Connected")', { timeout: 10000 });
await G.page.fill('input[placeholder="Message"]', "made through the relay");
await G.page.keyboard.press("Enter");
await H.page.waitForSelector('.bubble.in:has-text("made through the relay")', { timeout: 15000 });
await H.snap('relay-chat');
console.log('relay ceremony ok: one code, a seat, the rounds through the relay, matching words, then chat.');
// The chat screen long-polls through the proxy; let those go before the browser closes.
for (const p of [H, G]) await p.page.context().unrouteAll({ behavior: 'ignoreErrors' });

await browser.close();
server.close();
station.kill();
if (errors.length) { console.error("page errors:\n" + errors.join("\n")); process.exit(1); }
console.log(process.argv.includes("--ceremony-only") ? "e2e ok: three-member ceremony and compact QR/camera layouts." : "e2e ok: two- and three-member ceremonies, a ceremony through the relay, chat through a relay, OTP message, replay refused.", "Screenshots in", out);
