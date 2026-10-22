// Exercise actual Askama output offline. POST responses are mocked here;
// persistence and native forms are covered by the Rust route tests.
const { firefox } = require('playwright');
const assert = require('node:assert/strict');
const fs = require('node:fs/promises');
const path = require('node:path');

(async () => {
  const browser = await firefox.launch({ headless: true });
  const output = path.resolve('.build/browser');
  await fs.mkdir(output, { recursive: true });
  const pages = ['for-you', 'discover', 'saved', 'search', 'story', 'story-discover', 'settings'];
  try {
    for (const colorScheme of ['light', 'dark']) {
      for (const width of [360, 390]) {
        const context = await browser.newContext({ viewport: { width, height: 844 }, reducedMotion: "reduce", colorScheme });
        const page = await context.newPage();
        const errors = [];
        page.on('pageerror', e => errors.push(e.message));
        const posts = [];
        await context.route('https://wagwan.test/**', async route => {
          const request = route.request();
          if (request.method() === 'POST') {
            posts.push({ path: new URL(request.url()).pathname, body: request.postData() });
            return route.fulfill({ status: 200, contentType: 'application/json', body: '{}' });
          }
          const name = new URL(request.url()).pathname.slice(1).replace('.html', '');
          if (!pages.includes(name)) return route.fulfill({ status: 204 });
          return route.fulfill({ contentType: 'text/html', body: await fs.readFile(`.build/ui/${name}.html`, 'utf8') });
        });
        for (const name of pages) {
          await page.goto(`https://wagwan.test/${name}.html`);
          assert.ok(await page.locator('main').isVisible(), name);
          assert.ok(await page.evaluate(() => document.documentElement.scrollWidth <= innerWidth), `${name} overflows at ${width}`);
          await page.screenshot({ path: `${output}/${name}-${width}-${colorScheme}.png`, fullPage: true });
        }
        await page.goto('https://wagwan.test/story.html');
        const firstSentence = page.locator('.body a.s').first();
        await firstSentence.click();
        const sheet = page.getByRole('dialog');
        await sheet.waitFor({ state: 'visible' });
        assert.equal(await sheet.getAttribute('aria-hidden'), 'false');
        assert.ok(await sheet.getByRole('link', { name: /^Open in / }).isVisible());
        await page.keyboard.press('Escape');
        assert.equal(await page.locator('#sheet').getAttribute('aria-hidden'), 'true');
        assert.ok(await firstSentence.evaluate(e => e === document.activeElement));
        await page.locator('#more > summary').click();
        await sheet.getByRole('button', { name: 'Close', exact: true }).waitFor();
        assert.ok(await sheet.evaluate(e => e.contains(document.activeElement)), 'More must receive focus');
        const menu = sheet.locator('.more-menu');
        const bounds = await menu.boundingBox();
        assert.ok(bounds.y >= 0 && bounds.y + bounds.height <= 844, 'More menu must fit on screen');
        await page.screenshot({ path: `${output}/more-${width}-${colorScheme}.png` });
        await sheet.getByRole('button', { name: 'Close', exact: true }).click();
        const save = page.locator('[data-action="save"] button');
        const prior = await save.getAttribute('aria-pressed');
        await save.click();
        await page.locator('#toast.on').waitFor();
        assert.equal(await save.getAttribute('aria-pressed'), String(prior !== 'true'));
        await page.getByRole('button', { name: 'Undo', exact: true }).click();
        await page.waitForFunction(() => document.querySelector('#toast-t').textContent === 'Change undone');
        assert.equal(await save.getAttribute('aria-pressed'), prior);
        assert.equal(posts.length, 2);
        assert.match(posts[0].path, /^\/s\/\d+\/save$/);
        assert.notEqual(posts[0].body, posts[1].body);
        assert.deepEqual(errors, []);
        await context.close();
        console.log(`PASS: ${width}px ${colorScheme}, seven pages, source/More sheets, save and undo`);
      }
    }
  } finally {
    await browser.close();
  }
})().catch(error => { console.error(error); process.exitCode = 1; });
