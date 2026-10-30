// Landing-page navigation, responsive layout, and social-preview checks.
// Run: node web/landing-e2e.mjs (uses the Playwright dependency in web/).
// Review screenshots in /tmp/cahoots-landing-checks/.
import assert from 'node:assert/strict';
import { createServer } from 'node:http';
import { createReadStream, existsSync, mkdirSync, readFileSync, statSync } from 'node:fs';
import { extname, join, resolve } from 'node:path';
import { fileURLToPath } from 'node:url';
import { chromium } from 'playwright';

const root = resolve(fileURLToPath(new URL('../site/', import.meta.url)));
const output = '/tmp/cahoots-landing-checks';
mkdirSync(output, { recursive: true });
const types = { '.html': 'text/html', '.css': 'text/css', '.js': 'text/javascript', '.svg': 'image/svg+xml', '.png': 'image/png', '.glb': 'model/gltf-binary' };
const server = createServer((req, res) => {
  let path = resolve(root, '.' + new URL(req.url, 'http://localhost').pathname);
  if (path !== root && !path.startsWith(root + '/')) { res.writeHead(403).end(); return; }
  if (existsSync(path) && statSync(path).isDirectory()) path = join(path, 'index.html');
  if (!existsSync(path)) { res.writeHead(404).end(); return; }
  res.writeHead(200, { 'content-type': types[extname(path)] || 'application/octet-stream' });
  createReadStream(path).pipe(res);
});
await new Promise((done) => server.listen(0, '127.0.0.1', done));
let browser;
let failures = 0;
async function check(name, run) {
  try { await run(); console.log(`PASS ${name}`); }
  catch (error) { failures++; console.error(`FAIL ${name}: ${error.message}`); }
}
try {
  browser = await chromium.launch({ channel: 'chrome' });
  const page = await browser.newPage({ viewport: { width: 390, height: 844 }, reducedMotion: 'reduce' });
  page.setDefaultTimeout(5000);
  const base = `http://127.0.0.1:${server.address().port}`;
  await page.goto(base);
  await check('mobile navigation supports keyboard, dismissal, and resizing', async () => {
    const toggle = page.getByRole('button', { name: 'Open navigation', exact: true });
    assert.equal(await toggle.count(), 1, 'mobile navigation toggle is missing');
    const nav = page.getByRole('navigation', { name: 'Primary', exact: true });
    assert.equal(await nav.isVisible(), false);
    const bounds = await toggle.boundingBox();
    assert(bounds.width >= 44 && bounds.height >= 44, 'menu button must be at least 44 × 44');
    await toggle.focus();
    await page.keyboard.press('Enter');
    const close = page.getByRole('button', { name: 'Close navigation', exact: true });
    assert.equal(await close.getAttribute('aria-expanded'), 'true');
    for (const name of ['How it works', 'Docs', 'Journal', 'Open the app']) {
      const link = nav.getByRole('link', { name, exact: false });
      assert(await link.isVisible(), `${name} is missing from mobile navigation`);
      assert((await link.boundingBox()).height >= 44, `${name} touch target is too small`);
    }
    await page.keyboard.press('Tab');
    assert(await nav.getByRole('link', { name: 'How it works' }).evaluate(el => el === document.activeElement));
    await page.screenshot({ path: join(output, 'mobile-menu.png') });
    await page.keyboard.press('Escape');
    assert.equal(await nav.isVisible(), false);
    assert(await toggle.evaluate(el => el === document.activeElement));
    await toggle.click();
    await page.locator('#hero-title').click();
    assert.equal(await nav.isVisible(), false, 'outside click must close navigation');
    await toggle.click();
    await nav.getByRole('link', { name: 'How it works' }).click();
    assert.equal(await nav.isVisible(), false, 'following a link must close navigation');
    assert.equal(new URL(page.url()).hash, '#how');
    await page.setViewportSize({ width: 1280, height: 900 });
    await nav.waitFor({ state: 'visible' });
    assert(await nav.isVisible(), 'desktop navigation must remain visible after resize');
    assert.equal(await toggle.isVisible(), false);
    await page.setViewportSize({ width: 390, height: 844 });
    await nav.waitFor({ state: 'hidden' });
    assert.equal(await nav.isVisible(), false, 'returning to mobile must reset the menu');
  });
  await check('landing page fits narrow phones, tablets, and desktop', async () => {
    for (const width of [320, 375, 390, 720, 768, 1024, 1440]) {
      await page.setViewportSize({ width, height: 900 });
      await page.goto(base);
      const overflow = await page.evaluate(() => ({ viewport: innerWidth, content: document.documentElement.scrollWidth }));
      assert(overflow.content <= overflow.viewport, `${width}px viewport has ${overflow.content}px of content`);
      const headings = await page.locator('h1, h2').evaluateAll(elements => elements.map(el => ({ text: el.textContent, left: el.getBoundingClientRect().left, right: el.getBoundingClientRect().right })));
      for (const heading of headings) assert(heading.left >= 0 && heading.right <= width, `${heading.text} extends past ${width}px`);
      if ([320, 390, 1440].includes(width)) await page.screenshot({ path: join(output, `landing-${width}.png`), fullPage: true });
    }
  });
  await check('navigation remains available without JavaScript', async () => {
    const context = await browser.newContext({ javaScriptEnabled: false, viewport: { width: 320, height: 800 } });
    try {
      const fallback = await context.newPage();
      await fallback.goto(base);
      const nav = fallback.getByRole('navigation', { name: 'Primary', exact: true });
      for (const name of ['How it works', 'Docs', 'Journal', 'Open the app']) assert(await nav.getByRole('link', { name, exact: false }).isVisible(), `${name} must work without JavaScript`);
      assert(await fallback.evaluate(() => document.documentElement.scrollWidth <= innerWidth));
    } finally { await context.close(); }
  });
  await check('landing page declares a full-size accessible social preview', async () => {
    await page.goto(base);
    const image = await page.locator('meta[property="og:image"]').getAttribute('content');
    assert(image, 'Open Graph image is missing');
    assert.equal(new URL(image).origin, 'https://cahoots.network');
    assert.equal(await page.locator('meta[property="og:image:width"]').getAttribute('content'), '1200');
    assert.equal(await page.locator('meta[property="og:image:height"]').getAttribute('content'), '630');
    assert(await page.locator('meta[property="og:image:alt"]').getAttribute('content'));
    assert.equal(await page.locator('meta[name="twitter:card"]').getAttribute('content'), 'summary_large_image');
    assert.equal(await page.locator('meta[name="twitter:image"]').getAttribute('content'), image);
    const png = readFileSync(join(root, new URL(image).pathname));
    assert.equal(png.subarray(1, 4).toString(), 'PNG');
    assert.equal(png.readUInt32BE(16), 1200);
    assert.equal(png.readUInt32BE(20), 630);
  });
  console.log(`Screenshots: ${output}`);
} finally {
  await browser?.close();
  await new Promise(done => server.close(done));
}
process.exitCode = failures ? 1 : 0;
