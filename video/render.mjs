// Renders frames of the launch video with Playwright driving the installed Chrome.
//   node render.mjs [--frames a-b] [--every n] [--out out/frames] [--list f1,f2,...]
// Bundles src/scene.ts to out/scene.js first, serves the repository root, then screenshots each frame.
import { build } from "esbuild";
import { chromium } from "playwright";
import { createServer } from "node:http";
import { createReadStream, existsSync, mkdirSync, statSync } from "node:fs";
import { extname, join, resolve } from "node:path";
import { fileURLToPath } from "node:url";

const here = resolve(fileURLToPath(import.meta.url), "..");
const root = resolve(here, "..");
const args = process.argv.slice(2);
const flag = (n, d) => { const i = args.indexOf(n); return i >= 0 ? args[i + 1] : d; };
const outDir = resolve(here, flag("--out", "out/frames"));
const every = Number(flag("--every", "1"));
mkdirSync(outDir, { recursive: true });

await build({
  entryPoints: [join(here, "src/scene.ts")],
  bundle: true,
  format: "esm",
  outfile: join(here, "out/scene.js"),
  external: ["three", "three/addons/*"],
  logLevel: "warning",
});

const types = { ".html": "text/html", ".js": "text/javascript", ".mjs": "text/javascript", ".svg": "image/svg+xml", ".json": "application/json", ".css": "text/css" };
const server = createServer((req, res) => {
  const p = join(root, decodeURIComponent(new URL(req.url, "http://x").pathname));
  if (!p.startsWith(root) || !existsSync(p) || statSync(p).isDirectory()) { res.writeHead(404); res.end(); return; }
  res.writeHead(200, { "content-type": types[extname(p)] || "application/octet-stream" });
  createReadStream(p).pipe(res);
});
await new Promise((r) => server.listen(0, "127.0.0.1", r));
const port = server.address().port;

const browser = await chromium.launch({
  channel: "chrome",
  headless: true,
  args: ["--use-angle=metal", "--enable-gpu-rasterization", "--ignore-gpu-blocklist", "--enable-unsafe-swiftshader"],
});
const page = await browser.newPage({ viewport: { width: 1920, height: 1080 }, deviceScaleFactor: 1 });
page.on("pageerror", (e) => { console.error("page error:", e.message); });
page.on("console", (m) => { if (m.type() === "error") console.error("console:", m.text()); });
await page.goto(`http://127.0.0.1:${port}/video/src/index.html`);
await page.waitForFunction(() => window.ready === true, null, { timeout: 60000 });
const total = await page.evaluate(() => window.TOTAL);

let frames;
if (flag("--list")) frames = flag("--list").split(",").map(Number);
else {
  const [a, b] = (flag("--frames", `0-${total - 1}`)).split("-").map(Number);
  frames = [];
  for (let f = a; f <= b; f += every) frames.push(f);
}
const t0 = Date.now();
// Prime: the first frames compile shaders; render frame 0 twice so timings are honest.
await page.evaluate((f) => window.renderFrame(f), frames[0]);
for (let i = 0; i < frames.length; i++) {
  const f = frames[i];
  await page.evaluate((f) => window.renderFrame(f), f);
  await page.screenshot({ path: join(outDir, `f${String(f).padStart(5, "0")}.png`), type: "png", animations: "disabled" });
  if (i % 50 === 0 || i === frames.length - 1) {
    const dt = (Date.now() - t0) / 1000;
    console.log(`frame ${f}  ${i + 1}/${frames.length}  ${dt.toFixed(0)}s  ${(dt / (i + 1)).toFixed(2)} s/frame`);
  }
}
await browser.close();
server.close();
