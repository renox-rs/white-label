// node check-fill.js <base-url> <path>...   env: EMAIL, PASSWORD (default admin@example.com / password123)
// Opens each page at desktop and phone width and checks the standard-grid rules. Exit code 1 on any failure.
const { chromium } = require('playwright');
const fs = require('fs');
const [base, ...paths] = process.argv.slice(2);
if (!base || !paths.length) { console.error('usage: node check-fill.js <base-url> <path>...'); process.exit(2); }
const out = '/tmp/standard-grid'; fs.mkdirSync(out, { recursive: true });
(async () => {
  const browser = await chromium.launch();
  const ctx = await browser.newContext({ viewport: { width: 1440, height: 900 } });
  const page = await ctx.newPage();
  await page.goto(base + '/login');
  await page.fill('input[name=email]', process.env.EMAIL || 'admin@example.com');
  await page.fill('input[name=password]', process.env.PASSWORD || 'password123');
  await Promise.all([page.waitForNavigation().catch(() => {}), page.click('button[type=submit]')]);
  let failed = 0;
  for (const p of paths) {
    const name = p.replace(/\W+/g, '_') || 'root';
    const problems = [];
    await page.setViewportSize({ width: 1440, height: 900 });
    const res = await page.goto(base + p);
    if (!res || res.status() !== 200) { console.log(`FAIL ${p}: status ${res && res.status()}`); failed++; continue; }
    const d = await page.evaluate(() => {
      const grid = document.querySelector('.rx-grid');
      const table = document.querySelector('main table:not(.rx-grid table)');
      const q = (s) => !!(grid && grid.querySelector(s));
      const parent = grid && grid.closest('.rx-grid-fill') ? grid.closest('.rx-grid-fill').parentElement : (grid && grid.parentElement);
      return {
        grid: !!grid, plainTable: !!table && !grid,
        docScroll: document.documentElement.scrollHeight > innerHeight + 2,
        bottom: grid ? Math.round((grid.closest('.rx-grid-fill') || grid).getBoundingClientRect().bottom) : null,
        wrapped: !!(grid && (grid.closest('.rx-card, .rx-stack, .rx-table-card'))),
        parentClass: parent ? parent.className : null,
        search: q('input[type=search], input[name=q], .rx-grid-search'),
        perPage: q('select'),
        columns: q('[data-grid-columns], .rx-grid-columns, button[aria-label*=olumn]'),
        pager: q('.rx-pagination, .rx-grid-pager, nav[aria-label*=agination]') || /\d+\s*[–-]\s*\d+\s+of\s+\d+/.test(grid ? grid.innerText : ''),
        vh: innerHeight,
      };
    });
    await page.screenshot({ path: `${out}/${name}-desktop.png` });
    if (!d.grid) problems.push(d.plainTable ? 'plain <table>, not a grid' : 'no grid on the page');
    else {
      if (d.docScroll) problems.push('the document scrolls on desktop (fill rule)');
      if (d.wrapped) problems.push('the grid is wrapped in rx-card / rx-stack');
      if (d.bottom !== null && d.bottom < d.vh - 80) problems.push(`the grid ends at ${d.bottom}px of ${d.vh}px: it does not fill the screen`);
      if (!d.search) problems.push('no search box (a grid needs searchable columns)');
      if (!d.perPage) problems.push('no page-size choice');
      if (!d.pager) problems.push('no pagination / row count');
    }
    await page.setViewportSize({ width: 390, height: 844 });
    await page.goto(base + p);
    const phone = await page.evaluate(() => document.documentElement.scrollWidth > innerWidth + 2);
    await page.screenshot({ path: `${out}/${name}-phone.png`, fullPage: false });
    if (phone) problems.push('the page scrolls sideways at 390px');
    if (problems.length) { failed++; console.log(`FAIL ${p}\n  - ${problems.join('\n  - ')}`); } else console.log(`ok   ${p}`);
  }
  await browser.close();
  console.log(failed ? `${failed} page(s) fail the standard grid` : 'all pages pass');
  process.exit(failed ? 1 : 0);
})();
