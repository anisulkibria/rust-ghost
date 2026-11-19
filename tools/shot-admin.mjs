// Screenshot admin screens as a browser renders them, signed in.
//
// Run against a scratch instance (see docs/module-pattern.md and the dev
// notes): the first visit completes `/ghost/setup/` if the site has no owner,
// so a fresh database is all this needs.
//
//   node tools/shot-admin.mjs [baseUrl] [outDir] [paths…]
import { chromium } from '/Users/anisulkibria/Documents/Rust/Ghost/vuuut-click/node_modules/playwright/index.mjs';

const base = process.argv[2] || 'http://localhost:1975';
const outDir = process.argv[3] || '/tmp/admin-shots';
const argPaths = process.argv.slice(4);
const executablePath =
  process.env.CHROME_PATH ||
  `${process.env.HOME}/Library/Caches/ms-playwright/chromium-1248/chrome-mac-arm64/Google Chrome for Testing.app/Contents/MacOS/Google Chrome for Testing`;

const OWNER = { name: 'Ada Lovelace', email: 'owner@example.com', password: 'polish-pass-123456' };

const pages = argPaths.length
  ? argPaths.map((p) => [p.replace(/[^a-z0-9]+/gi, '-').replace(/^-|-$/g, '') || 'root', p])
  : [
      ['dashboard', '/ghost/dashboard/'],
      ['posts', '/ghost/posts/'],
      ['modules', '/ghost/modules/'],
      ['settings', '/ghost/settings/'],
    ];

const browser = await chromium.launch({ executablePath });

async function signIn(page) {
  await page.goto(base + '/ghost/', { waitUntil: 'load' });
  if (page.url().includes('/setup')) {
    await page.fill('input[name="name"]', OWNER.name);
    await page.fill('input[name="email"]', OWNER.email);
    await page.fill('input[name="password"]', OWNER.password);
    const blog = await page.$('input[name="blogTitle"], input[name="title"]');
    if (blog) await blog.fill('Polish');
    await page.click('button[type="submit"], input[type="submit"]');
    await page.waitForLoadState('load');
  }
  if (page.url().includes('/signin')) {
    await page.fill('input[name="email"]', OWNER.email);
    await page.fill('input[name="password"]', OWNER.password);
    await page.click('button[type="submit"], input[type="submit"]');
    await page.waitForLoadState('load');
  }
  return page.url();
}

for (const scheme of ['light', 'dark']) {
  const context = await browser.newContext({
    viewport: { width: 1440, height: 1000 },
    deviceScaleFactor: 2,
    colorScheme: scheme,
  });
  const page = await context.newPage();
  page.on('pageerror', (e) => console.log(`  [pageerror] ${e.message}`));
  console.log(`${scheme}: signed in at ${await signIn(page)}`);

  // The scheme is the signed-in user's own choice, kept in their accessibility
  // JSON — a signed-in account with nothing saved is "light" whatever the OS
  // says, so emulating the OS alone photographs the light screen twice. Pick
  // it from the rail's Appearance menu, the way a person would.
  await page.click('.theme-menu > summary');
  await page.click(`.theme-menu .menu-item[data-choice="${scheme}"]`);
  await page.waitForTimeout(400);

  for (const [name, path] of pages) {
    const response = await page.goto(base + path, { waitUntil: 'load' });
    // The `.rise` entrance is staggered by `--i * 70ms` and fills backwards,
    // so a shot taken too early catches the later sections at opacity 0 —
    // which looks like a page that failed to render rather than one mid
    // animation. Wait past the longest delay.
    await page.waitForTimeout(1200);
    const file = `${outDir}/${name}-${scheme}.png`;
    // Viewport-only by default: a full-page shot of a layout with sticky
    // chrome and backdrop filters paints the sticky pieces twice and blurs
    // the rest, which is a picture of the screenshotter, not the design.
    await page.screenshot({ path: file, fullPage: process.env.FULL_PAGE === '1' });
    console.log(`  ${scheme.padEnd(5)} ${path.padEnd(28)} ${response ? response.status() : 0} → ${file}`);
  }
  await context.close();
}

await browser.close();
