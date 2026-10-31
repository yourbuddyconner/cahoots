// Build the journal first, then run: node scripts/check-blog.mjs
// Uses the browser test dependencies in web/. Screenshots go to /tmp/cahoots-blog-checks.
import assert from 'node:assert/strict';
import { createRequire } from 'node:module';
import { createServer } from 'node:http';
import { createReadStream, existsSync, mkdirSync, statSync, readFileSync } from 'node:fs';
import { extname, join, resolve } from 'node:path';
import { fileURLToPath } from 'node:url';
const root = resolve(fileURLToPath(import.meta.url), '../..');
const require = createRequire(join(root, 'web/package.json'));
const { chromium } = require('playwright');
const site = join(root, 'site');
assert.doesNotMatch(readFileSync(join(site, 'blog/feed.xml'), 'utf8'), /term-definition|class="defined-term"/, 'RSS keeps plain article HTML');
const output = '/tmp/cahoots-blog-checks';
mkdirSync(output, { recursive: true });
const types = { '.html':'text/html', '.css':'text/css', '.js':'text/javascript', '.svg':'image/svg+xml', '.png':'image/png', '.jpg':'image/jpeg' };
const server = createServer((req, res) => {
  let path = resolve(site, '.' + decodeURIComponent(new URL(req.url, 'http://localhost').pathname));
  if (existsSync(path) && statSync(path).isDirectory()) path = join(path, 'index.html');
  if (!path.startsWith(site + '/') || !existsSync(path)) { res.writeHead(404); res.end(); return; }
  res.writeHead(200, { 'content-type': types[extname(path)] || 'application/octet-stream' });
  createReadStream(path).pipe(res);
});
await new Promise(resolve => server.listen(0, '127.0.0.1', resolve));
const browser = await chromium.launch({ channel: 'chrome' });
const url = `http://127.0.0.1:${server.address().port}/blog/privacy-hardware-needs-a-renaissance/`;
try {
  for (const width of [1280, 375]) {
    const context = await browser.newContext({ viewport:{ width, height:900 }, reducedMotion:'reduce', hasTouch:width < 500, isMobile:width < 500 });
    const page = await context.newPage();
    const errors = [];
    page.on('pageerror', error => errors.push(error.message));
    await page.goto(url);
    const progress = page.getByRole('progressbar', { name:'Article reading progress' });
    assert.equal(await progress.count(), 1, 'Article has a reading progress indicator');
    assert.equal(Number(await progress.getAttribute('aria-valuenow')), 0);
    const terms = page.locator('.post .defined-term');
    assert.ok(await terms.count() >= 8, 'New vocabulary has definitions');
    const grant = page.locator('.post > p .defined-term').filter({ has:page.locator('dfn', { hasText:/^grant$/i }) }).first();
    await grant.scrollIntoViewIfNeeded();
    await grant.focus();
    const definitionId = await grant.getAttribute('aria-describedby');
    const tooltip = page.locator(`[id="${definitionId}"]`);
    await tooltip.waitFor({ state:'visible' });
    assert.match(await tooltip.innerText(), /permission/i);
    await page.keyboard.press('Escape');
    await tooltip.waitFor({ state:'hidden' });
    await grant.blur();
    if (width < 500) await grant.tap(); else await grant.hover();
    await tooltip.waitFor({ state:'visible' });
    const bounds = await tooltip.boundingBox();
    assert.ok(bounds.x >= 8 && bounds.x + bounds.width <= width - 8, 'Definition fits the viewport');
    const barBounds = await progress.boundingBox();
    const navBounds = await page.locator('.top').boundingBox();
    const articleBounds = await page.locator('.post').boundingBox();
    assert.ok(barBounds.y >= navBounds.y + navBounds.height - 1, 'Reading bar stays below the navigation');
    assert.ok(Math.abs(barBounds.x - articleBounds.x) < 1 && Math.abs(barBounds.width - articleBounds.width) < 1,
      'Reading bar follows the article column');
    if (width > 500) {
      await page.mouse.move(bounds.x + bounds.width / 2, bounds.y + bounds.height + 5);
      await tooltip.hover();
      await page.waitForTimeout(250);
      assert.ok(await tooltip.isVisible(), 'A hovered definition stays open after crossing the gap');
    }
    await page.screenshot({ path:`${output}/definition-${width}.png` });
    await page.keyboard.press('Escape');
    const middle = Number(await progress.getAttribute('aria-valuenow'));
    assert.ok(middle > 0 && middle < 100, 'Progress tracks the article body');
    const details = page.locator('#messaging-protocol-design');
    assert.equal(await details.getAttribute('open'), null, 'Digression starts collapsed');
    await details.locator('summary').click();
    assert.notEqual(await details.getAttribute('open'), null);
    assert.equal(await details.locator('.protocol-diagram').count(), 2, 'Two diagrams explain the protocol');
    await details.screenshot({ path:`${output}/detour-${width}.png` });
    const beforeCollapse = Number(await progress.getAttribute('aria-valuenow'));
    await details.locator('summary').click();
    await page.waitForFunction(() => !document.querySelector('#messaging-protocol-design').open);
    await page.evaluate(() => document.querySelector('.post').scrollIntoView({ block:'end' }));
    await page.waitForFunction(() => Number(document.querySelector('.reading-progress').getAttribute('aria-valuenow')) === 100);
    assert.ok(beforeCollapse >= 0 && beforeCollapse <= 100);
    await page.setViewportSize({ width:width === 1280 ? 1024 : 390, height:760 });
    await page.evaluate(() => document.querySelector('.post').scrollIntoView({ block:'end' }));
    await page.waitForFunction(() => Number(document.querySelector('.reading-progress').getAttribute('aria-valuenow')) === 100);
    assert.ok(await page.evaluate(() => document.documentElement.scrollWidth <= innerWidth), 'Page has no horizontal overflow');
    assert.deepEqual(errors, []);
    await context.close();
    console.log(`${width}px: definitions by focus, pointer/touch, Escape; progress at start/middle/end; disclosure, resize, and diagrams passed.`);
  }
  const noScript = await browser.newContext({ javaScriptEnabled:false, viewport:{ width:375, height:900 } });
  const page = await noScript.newPage();
  await page.goto(url);
  await page.locator('#messaging-protocol-design summary').click();
  assert.equal(await page.locator('.protocol-diagram').count(), 2);
  const first = page.locator('.defined-term').first();
  await first.focus();
  await first.locator('[role="tooltip"]').waitFor({ state:'visible' });
  assert.ok(await page.locator('.reading-progress').isHidden(), 'No static progress indicator when JavaScript is off');
  await noScript.close();
  console.log(`No-JavaScript disclosure and definitions passed. Screenshots: ${output}`);
} finally {
  await browser.close();
  await new Promise(resolve => server.close(resolve));
}
