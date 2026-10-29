// Browser shot of the demo video (tool/demo-video.sh): records matrust.dev/try/ running the demo script.
// node scripts/demo-browser.mjs OUT_DIR [base]
// Writes OUT_DIR/web.webm (1280x744, the page area above the caption strip) and prints
// {"start": s, "end": s}: the span of the recording to keep, in seconds from its first frame.
import { chromium } from 'playwright';
import { mkdtempSync, renameSync, readdirSync, rmSync } from 'node:fs';
import { join } from 'node:path';

const out = process.argv[2];
const base = process.argv[3] ?? 'https://matrust.dev';
if (!out) throw new Error('usage: demo-browser.mjs OUT_DIR [base]');
const size = { width: 1280, height: 744 };
const lines = [
  't = linspace(0, 2*pi, 200);',
  'y = sin(3*t) .* exp(-t/4);',
  'plot(t, y), grid on',
  "title('Damped sine')",
];

const videoDir = mkdtempSync(join(out, 'pw-'));
const browser = await chromium.launch();
const context = await browser.newContext({ viewport: size, recordVideo: { dir: videoDir, size } });

// Warm-up page: loads and compiles the engine once so the recorded run does not wait on the download.
const warm = await context.newPage();
await warm.goto(base + '/try/');
await warm.locator('#run').click();
await warm.waitForFunction(() => document.querySelector('#command-output').textContent.trim().length > 0
  && !document.querySelector('#command-output').textContent.includes('Run a script'), null, { timeout: 90000 });
await warm.close();

const page = await context.newPage();
const t0 = Date.now();
const at = () => (Date.now() - t0) / 1000;
await page.goto(base + '/try/');
await page.waitForLoadState('networkidle');
await page.waitForTimeout(800);
const start = at();
await page.waitForTimeout(1000);
const editor = page.locator('#editor');
await editor.click();
await page.keyboard.press('Control+A');
await page.keyboard.press('Delete');
await page.waitForTimeout(300);
for (const [i, line] of lines.entries()) {
  await page.keyboard.type(line, { delay: 25 });
  if (i < lines.length - 1) await page.keyboard.press('Enter');
  await page.waitForTimeout(300);
}
await page.waitForTimeout(600);
await page.locator('#run').click();
await page.locator('#figures img').first().waitFor({ timeout: 60000 });
await page.waitForTimeout(700);
// Bring the whole figure card into view (it starts just below the fold at this height).
for (let i = 0; i < 10; i++) { await page.mouse.wheel(0, 16); await page.waitForTimeout(30); }
await page.waitForTimeout(5500);
const end = at();
const video = page.video();
await page.close();
await context.close();
await browser.close();
renameSync(await video.path(), join(out, 'web.webm'));
for (const f of readdirSync(videoDir)) rmSync(join(videoDir, f));
rmSync(videoDir, { recursive: true });
console.log(JSON.stringify({ start, end }));
