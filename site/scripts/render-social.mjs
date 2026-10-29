import { chromium } from 'playwright';
import { readFile } from 'node:fs/promises';
const font = (await readFile(new URL('../public/fonts/Inter-SemiBold.ttf', import.meta.url))).toString('base64');
const browser = await chromium.launch({ headless:true });
try {
  const page = await browser.newPage({ viewport:{ width:1200, height:630 }, deviceScaleFactor:1 });
  await page.setContent(`<!doctype html><html lang="en"><head><meta charset="utf-8"><style>@font-face{font-family:Inter;src:url(data:font/ttf;base64,${font});font-weight:600}*{box-sizing:border-box}body{margin:0;background:#F7F5F2;color:#1C1917;font-family:Inter,sans-serif;padding:64px}header{display:flex;align-items:center;gap:24px;font-size:32px}svg{width:80px;height:80px}h1{font-size:48px;line-height:1.15;letter-spacing:-.04em;font-weight:600;margin:64px 0 24px;max-width:800px}p{font-size:24px;line-height:1.5;color:#57534E;margin:0}footer{margin-top:48px;font-size:18px;color:#C2410C}</style></head><body><header><svg viewBox="0 0 100 100"><rect width="100" height="100" rx="22" fill="#C2410C"/><text x="50" y="74" text-anchor="middle" fill="white" font-family="Inter" font-weight="600" font-size="64">M</text></svg>MatRust</header><h1>Numerical computing.<br>Built in Rust.</h1><p>A native workspace for scripts, data and figures.</p><footer>matrust.dev · Open source · Alpha</footer></body></html>`);
  await page.evaluate(() => document.fonts.ready);
  await page.screenshot({ path:new URL('../public/social-card.png', import.meta.url).pathname });
  console.log('Rendered public/social-card.png (1200 × 630), using the bundled Inter font.');
} finally { await browser.close(); }
