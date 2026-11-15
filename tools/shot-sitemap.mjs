// Screenshot the sitemap pages as a browser renders them.
//
// This exercises the content negotiation end to end rather than asserting on
// headers: Playwright navigates for real, so Chrome sends the
// `Sec-Fetch-Mode: navigate` / `Sec-Fetch-Dest: document` pair that
// `wants_html` looks for. If the server got that wrong, these screenshots show
// Chrome's raw XML tree and that is the finding.
//
// Run from the vuuut checkout with the dev server up:
//   node tools/shot-sitemap.mjs [baseUrl] [outDir]
import { chromium } from '/Users/anisulkibria/Documents/Rust/Ghost/vuuut-click/node_modules/playwright/index.mjs';

const base = process.argv[2] || 'http://localhost:1971';
const outDir = process.argv[3] || '/tmp/sitemap-shots';
const executablePath =
  process.env.CHROME_PATH ||
  `${process.env.HOME}/Library/Caches/ms-playwright/chromium-1248/chrome-mac-arm64/Google Chrome for Testing.app/Contents/MacOS/Google Chrome for Testing`;

const pages = [
  ['index', '/sitemap.xml'],
  ['posts', '/sitemap-posts.xml'],
  ['tags', '/sitemap-tags.xml'],
];

const browser = await chromium.launch({ executablePath });
let failed = 0;

for (const scheme of ['light', 'dark']) {
  const context = await browser.newContext({
    viewport: { width: 1200, height: 900 },
    deviceScaleFactor: 2,
    colorScheme: scheme,
  });
  const page = await context.newPage();
  // Anything the page logs is worth seeing: a failed XSLT load shows up here.
  page.on('console', (m) => console.log(`  [console:${m.type()}] ${m.text()}`));
  page.on('pageerror', (e) => console.log(`  [pageerror] ${e.message}`));

  for (const [name, path] of pages) {
    // `load`, not `networkidle`: an XSLT-transformed document never reports
    // network idle in Chrome, so waiting for it just times out.
    const response = await page.goto(base + path, { waitUntil: 'load' });
    const status = response ? response.status() : 0;
    // Did the server send the page, or the XML? Chrome renders raw XML into a
    // document too, so the test is for the page's own markup.
    const { transformed, heading, links } = await page.evaluate(() => ({
      transformed: !!document.querySelector('main p.eyebrow'),
      heading: (document.querySelector('h1') || {}).textContent || '',
      links: document.querySelectorAll('main a[href]').length,
    }));
    const file = `${outDir}/${name}-${scheme}.png`;
    await page.screenshot({ path: file, fullPage: true });
    const verdict = transformed ? "page" : "RAW XML — negotiation did not fire";
    console.log(
      `${scheme.padEnd(5)} ${path.padEnd(22)} ${status} ${verdict}; h1=${JSON.stringify(heading.trim())}; links=${links}`,
    );
    if (!transformed) failed++;
  }
  await context.close();
}

await browser.close();
console.log(failed ? `\n${failed} URL(s) served XML to a browser navigation` : '\nevery URL served the page');
process.exit(failed ? 1 : 0);
