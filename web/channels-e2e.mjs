// End-to-end check of channels in the browser app, against a real relay started here.
// Person A makes a circle (a two-person ceremony through the relay with a helper, who then
// leaves), starts a group chat channel and copies its link. Person B opens the link in a fresh
// browser, reads A's first post and asks to post. A allows it; B posts; A reads it. A revokes B,
// which retires the link and starts new keys; B is told the link no longer works and cannot read
// what A posts next.
//   node web/channels-e2e.mjs   (needs the wasm build in web/app/pkg, `npm install` in web/, and
//                                `cargo build -p cahoots-station --features objects,channels,gate`)
// Screenshots land in web/e2e-out/ as channel-<person>-NN-<step>.png.
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

// A relay of our own, with channels and the gate. The app's default relay is never reached.
const stationBin = join(process.env.CARGO_TARGET_DIR || join(here, "..", "target"), "debug", "cahoots-station");
const BUILD = "cargo build -p cahoots-station --features objects,channels,gate";
if (!existsSync(stationBin)) { console.error(`build the relay first: ${BUILD}`); process.exit(1); }
const freePort = await new Promise((r) => { const s = createServer(); s.listen(0, "127.0.0.1", () => { const p = s.address().port; s.close(() => r(p)); }); });
const stationUrl = `http://127.0.0.1:${freePort}`;
const station = spawn(stationBin, ["--listen", `127.0.0.1:${freePort}`, "--db", join(mkdtempSync(join(out, "relay-")), "station.db")], { stdio: ["ignore", "ignore", "inherit"] });
const stop = () => { try { server.close(); station.kill(); } catch {} };
for (let i = 0; ; i++) {
  try { if ((await fetch(`${stationUrl}/healthz`)).ok) break; } catch {}
  if (i > 100) { console.error("the relay did not start"); stop(); process.exit(1); }
  await new Promise((r) => setTimeout(r, 100));
}
const info = await (await fetch(`${stationUrl}/v1/info`)).json();
for (const f of ["objects", "channels", "gate"]) {
  if (!info.features?.includes(f)) { console.error(`the relay at ${stationBin} has no "${f}" feature (it reports ${JSON.stringify(info.features)}). Rebuild it: ${BUILD}`); stop(); process.exit(1); }
}

const browser = await chromium.launch({ channel: "chrome" });
const errors = [];
// B's reads are refused once A revokes B; those refusals are the point of the last step.
let refusalsExpected = false;
async function person(name, viewport) {
  const ctx = await browser.newContext({ viewport, deviceScaleFactor: 2, isMobile: viewport.width < 500, hasTouch: viewport.width < 500 });
  await ctx.addInitScript(() => { navigator.clipboard.writeText = (t) => { window.__clip = t; return Promise.resolve(); }; });
  await ctx.route("https://relay.cahoots.network/**", (r) => r.abort());
  const page = await ctx.newPage();
  page.on("pageerror", (e) => errors.push(`${name}: ${e.message}`));
  page.on("console", (m) => {
    if (m.type() !== "error") return;
    const url = m.location()?.url || "";
    if (url.includes("relay.cahoots.network")) return; // the shipped default relay, blocked here
    if (refusalsExpected && name === "reader" && url.includes("/v1/channels/") && /401/.test(m.text())) return;
    errors.push(`${name} console: ${m.text()} ${url}`);
  });
  await page.goto(base);
  await page.waitForSelector(".screen");
  let shot = 0;
  const snap = async (label) => { await page.waitForTimeout(260); await page.screenshot({ path: join(out, `channel-${name}-${String(++shot).padStart(2, "0")}-${label}.png`), timeout: 10000 }); };
  const clip = () => page.evaluate(() => window.__clip);
  const paste = async (text) => { await page.fill("textarea.input", text); await page.click("text=Use this"); };
  const menu = async () => { await page.click('button[aria-label="More options"]'); await page.waitForSelector(".sheet-bg.on"); };
  const closeSheet = async () => { await page.keyboard.press("Escape"); await page.waitForTimeout(250); };
  return { page, ctx, snap, clip, paste, menu, closeSheet, name };
}

const people = [];
const onFail = async (e) => {
  console.error("failed:", e.message);
  for (const p of people) { try { await p.page.screenshot({ path: join(out, `channel-${p.name}-FAIL.png`) }); console.error(p.name, "title:", await p.page.$eval(".bar .title", (x) => x.textContent), "status:", await p.page.$eval(".status", (x) => x.textContent).catch(() => "")); } catch {} }
  if (errors.length) console.error("page errors:\n" + errors.join("\n"));
  await browser.close(); stop(); process.exit(1);
};
process.on("unhandledRejection", onFail);
process.on("uncaughtException", onFail);
const wordsOf = (p) => p.$$eval(".words div", (els) => els.map((e) => e.textContent.replace(/^\d+/, "").trim()).join(" "));

// 1. A circle for A. The app needs two people, so a helper joins through the relay and leaves.
const viaStation = async (p) => p.ctx.route("https://relay.cahoots.network/**", async (r) => {
  const u = new URL(r.request().url());
  await r.fulfill({ response: await r.fetch({ url: stationUrl + u.pathname + u.search }) });
});
const A = await person("owner", { width: 390, height: 844 });
const H = await person("helper", { width: 390, height: 844 });
people.push(A, H);
await viaStation(A); await viaStation(H);
await A.page.getByRole("button", { name: "Create a circle", exact: true }).click();
await A.page.fill("input[type=text]", "garden club");
await A.page.getByRole("button", { name: "Continue", exact: true }).click();
await A.page.getByRole("button", { name: "Make my contribution", exact: true }).click();
await A.page.waitForSelector("text=Show this code to everyone joining");
await A.menu(); await A.page.getByRole("button", { name: "Copy the code", exact: true }).click(); const offer = await A.clip(); await A.closeSheet();
await H.page.getByRole("button", { name: "Join a circle", exact: true }).click();
await H.page.getByRole("button", { name: "No camera? Paste a code", exact: true }).click();
await H.paste(offer);
await H.page.waitForSelector("text=Which seat are you?");
await H.page.getByRole("button", { name: "Make my contribution", exact: true }).click();
await H.page.waitForSelector('.bar .title:has-text("Show this to the host")', { timeout: 15000 });
await H.menu(); await H.page.getByRole("button", { name: "Copy my join code", exact: true }).click(); const joined = await H.clip(); await H.closeSheet();
await A.page.waitForSelector("text=Seat 2 committed but was not scanned here.", { timeout: 15000 });
await A.menu(); await A.page.getByRole("button", { name: "Paste a join code", exact: true }).click(); await A.paste(joined); await A.closeSheet();
await A.page.waitForSelector('.bar .title:has-text("Everyone is scanned")', { timeout: 15000 });
await A.page.getByRole("button", { name: "Continue", exact: true }).click();
for (const p of [A, H]) await p.page.waitForSelector("text=Step 4 of 4", { timeout: 20000 });
if (await wordsOf(A.page) !== await wordsOf(H.page)) throw new Error("check words differ");
await A.page.click("text=They match"); await H.page.click("text=They match");
await A.page.waitForSelector(".circle-dashboard .chatlist");
// A points the circle at the relay started here, the one that carries channels.
await A.menu(); await A.page.click('button:text-is("Relay address")');
await A.page.fill('input[type="url"]', stationUrl);
await A.page.click('button:text-is("Save and connect")');
await A.page.waitForSelector('.status:has-text("Connected")');
await H.ctx.unrouteAll({ behavior: "ignoreErrors" }); await H.ctx.close(); people.splice(people.indexOf(H), 1);
await A.ctx.unrouteAll({ behavior: "ignoreErrors" }); await A.ctx.route("https://relay.cahoots.network/**", (r) => r.abort());
console.log("circle ready");

// 2. A starts a group chat channel and copies the link.
await A.page.getByRole("button", { name: "Circle tools", exact: true }).click();
await A.page.getByRole("link", { name: "Start a channel", exact: true }).click();
await A.page.waitForSelector("text=The relay keeps encrypted messages only. Anyone with the link can read.");
await A.page.getByLabel("Channel name", { exact: true }).fill("Garden news");
await A.page.locator(".seg button").filter({ hasText: /^Group chat$/ }).click();
await A.snap("start");
await A.page.getByRole("button", { name: "Start", exact: true }).click();
await A.page.waitForSelector("text=Anyone you forward this link to can read until you retire it.", { timeout: 15000 });
await A.snap("share");
await A.page.getByRole("button", { name: "Copy the link", exact: true }).click();
const link = await A.clip();
if (!/^https:\/\/cahoots\.network\/app\/#\/ch\/127\.0\.0\.1:\d+\/[0-9a-f]{32}\/[A-Za-z0-9_-]{43}$/.test(link || "")) throw new Error(`channel link: ${link}`);
const channelId = link.split("/")[7];
await A.page.getByRole("button", { name: "Open the channel", exact: true }).click();
await A.page.waitForSelector("text=End-to-end encrypted; the relay keeps ciphertext only");
await A.page.waitForSelector("text=anyone with the link can read and post");
await A.page.getByLabel("Message", { exact: true }).fill("first post from A");
await A.page.keyboard.press("Enter");
await A.page.waitForSelector('.bubble.out:has-text("first post from A")');
await A.snap("first-post");
// What the relay holds is ciphertext: the stream needs a credential, and the link's secret never reached it.
if ((await fetch(`${stationUrl}/v1/channels/${channelId}/messages`)).status !== 401) throw new Error("an unauthenticated read of the channel was not refused");
console.log("channel started:", channelId);

// 3. B opens the link in a fresh browser and reads A's first post.
const B = await person("reader", { width: 1280, height: 900 });
people.push(B);
await B.page.goto(base + link.slice(link.indexOf("#")));
await B.page.waitForSelector("text=Garden news");
await B.page.waitForSelector("text=anyone with the link can read and post");
await B.snap("join");
await B.page.getByLabel("Your name", { exact: true }).fill("Bea");
await B.page.getByRole("button", { name: "Open", exact: true }).click();
await B.page.waitForSelector('.bubble.in:has-text("first post from A")', { timeout: 15000 });
if ((await B.page.evaluate(() => location.hash)).split("/").length !== 3) throw new Error("the link's secret stayed in the address bar");
await B.page.waitForSelector("text=Waiting for the owner to let you post.");
if (!(await B.page.getByLabel("Message", { exact: true }).isDisabled())) throw new Error("B can post before the owner allows it");
await B.snap("reading");
console.log("B reads by link and asked to post");

// 4. A allows B's request.
await A.menu(); await A.page.getByRole("button", { name: "Requests", exact: true }).click();
await A.page.waitForSelector('.request:has-text("Bea")', { timeout: 15000 });
await A.page.waitForSelector('.request:has-text("read, post")');
await A.snap("requests");
await A.page.locator(".request").filter({ hasText: "Bea" }).getByRole("button", { name: "Allow", exact: true }).click();
await A.page.waitForSelector("text=No requests are waiting.", { timeout: 15000 });
await A.page.getByRole("button", { name: "Back", exact: true }).click();

// 5. B posts; A reads it under B's name.
await B.page.waitForSelector('input[aria-label="Message"]:enabled', { timeout: 30000 });
await B.page.getByLabel("Message", { exact: true }).fill("hello from B");
await B.page.keyboard.press("Enter");
await B.page.waitForSelector('.bubble.out:has-text("hello from B")');
await A.page.waitForSelector('.bubble.in:has-text("hello from B")', { timeout: 30000 });
await A.page.waitForSelector('.bubble.in .meta:has-text("Bea")');
await A.snap("reads-b"); await B.snap("posted");
console.log("B posts and A reads it");

// 6. A revokes B, which retires the link and starts new keys.
await A.menu(); await A.page.getByRole("button", { name: "Members", exact: true }).click();
await A.page.waitForSelector('.member:has-text("Bea")');
await A.snap("members");
refusalsExpected = true;
A.page.once("dialog", (d) => d.accept());
await A.page.locator(".member").filter({ hasText: "Bea" }).getByRole("button", { name: "Revoke", exact: true }).click();
await A.page.waitForSelector("text=No one else is a member yet.", { timeout: 15000 });
await A.page.getByRole("button", { name: "Back", exact: true }).click();
await A.page.getByLabel("Message", { exact: true }).fill("after the revocation");
await A.page.keyboard.press("Enter");
await A.page.waitForSelector('.bubble.out:has-text("after the revocation")');
await A.snap("after-revoke");

// 7. B is shut out: the link no longer works, and A's next post never shows.
await B.page.waitForSelector("text=This link no longer works. Ask for a new one.", { timeout: 40000 });
await B.page.waitForTimeout(3000);
if (await B.page.locator('.bubble:has-text("after the revocation")').count()) throw new Error("B read a post made after the revocation");
if (!(await B.page.getByLabel("Message", { exact: true }).isDisabled())) throw new Error("B can still post after the revocation");
await B.snap("shut-out");
// B reloads: still shut out.
await B.page.reload();
await B.page.waitForSelector("text=This link no longer works. Ask for a new one.", { timeout: 15000 });
if (await B.page.locator('.bubble:has-text("after the revocation")').count()) throw new Error("B read the later post after a reload");
const rejected = await A.page.$$(".bubble.rejected");
if (rejected.length) throw new Error(`${rejected.length} rejected messages on A's screen`);
console.log("B is shut out after the revocation");

for (const p of people) await p.ctx.unrouteAll({ behavior: "ignoreErrors" });
await browser.close();
stop();
if (errors.length) { console.error("page errors:\n" + errors.join("\n")); process.exit(1); }
console.log("channels e2e ok: start, share, read by link, request, allow, post, revoke, shut out. Screenshots in", out);
