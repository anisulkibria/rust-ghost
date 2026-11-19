// Close-up of the list checkboxes: the select-all in the column head, an
// unticked row and a ticked one, in one frame, at 4x so the drawn box can
// actually be judged.
//
//   node tools/shot-checks.mjs [baseUrl]
import { chromium } from '/Users/anisulkibria/Documents/Rust/Ghost/vuuut-click/node_modules/playwright/index.mjs';

const base = process.argv[2] || 'http://localhost:1975';
const executablePath =
  process.env.CHROME_PATH ||
  `${process.env.HOME}/Library/Caches/ms-playwright/chromium-1248/chrome-mac-arm64/Google Chrome for Testing.app/Contents/MacOS/Google Chrome for Testing`;

const browser = await chromium.launch({ executablePath });

for (const scheme of ['light', 'dark']) {
  const context = await browser.newContext({
    viewport: { width: 1440, height: 900 },
    deviceScaleFactor: 4,
  });
  const page = await context.newPage();
  await page.goto(base + '/ghost/');
  if (page.url().includes('signin')) {
    await page.fill('input[name="email"]', 'owner@example.com');
    await page.fill('input[name="password"]', 'polish-pass-123456');
    await page.click('button[type="submit"]');
    await page.waitForLoadState('load');
  }
  await page.click('.theme-menu > summary');
  await page.click(`.theme-menu .menu-item[data-choice="${scheme}"]`);
  await page.waitForTimeout(400);

  await page.goto(base + '/ghost/posts/');
  await page.waitForTimeout(900);
  const boxes = await page.$$('.row-check input[name="ids"]');
  if (boxes[1]) await boxes[1].click();
  await page.waitForTimeout(400);

  const list = await page.$('.post-list');
  const bb = await list.boundingBox();
  await page.screenshot({
    path: `/tmp/admin-shots/checks-${scheme}.png`,
    clip: { x: bb.x, y: bb.y, width: 430, height: 330 },
  });
  console.log(`${scheme} ok`);
  await context.close();
}

await browser.close();
