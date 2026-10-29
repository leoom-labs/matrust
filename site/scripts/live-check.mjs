// Check a deployed site's console runs the real browser engine, not the mock fallback.
// node scripts/live-check.mjs [base]   (default https://matrust.dev)
import { chromium } from 'playwright';
import assert from 'node:assert/strict';

const base = process.argv[2] ?? 'https://matrust.dev';
const browser = await chromium.launch();
const page = await browser.newPage();
const errors = [];
page.on('pageerror', error => errors.push(error.message));
await page.goto(base + '/try/');
// The mock worker cannot do linear algebra warnings; the real engine prints the singular-matrix warning.
await page.locator('#editor').fill("x = sum(primes(100));\ndisp(x)\ny = inv([1 2;2 4]);");
await page.locator('#run').click();
await page.waitForFunction(() => document.querySelector('#command-output').textContent.includes('1060'), null, { timeout: 60000 });
const out = await page.locator('#command-output').textContent();
assert.match(out, /singular to working precision/);
assert.equal(await page.locator('#preview-banner').isVisible(), false, 'mock preview banner shown');
assert.deepEqual(errors, []);
console.log('live check ok:', base);
await browser.close();
