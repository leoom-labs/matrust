import { test, expect } from 'playwright/test';
import AxeBuilder from '@axe-core/playwright';
import { readFileSync, mkdirSync } from 'node:fs';
import { detectPlatform } from '../src/lib/platform';
const bundled = JSON.parse(readFileSync(new URL('../src/data/release.json', import.meta.url), 'utf8'));
const ver = bundled.version.slice(1); // bundled release, e.g. 0.1.1
const help = readFileSync(new URL('../../assets/help/functions.md', import.meta.url), 'utf8');
const functions = [...new Set([...help.matchAll(/^## ([^\r\n]+)\r?$/gm)].map(match => match[1].trim()))];
// Rendered in every theme and viewport: plain, duplicate, dotted, apostrophe, longest and plotting help.
const sampleFunctions = ['linspace', 'norm', 'containers.Map', 'annotation', 'isequalwithequalnans', 'plot'];
const routes = ['/', '/download/', '/try/', '/docs/', '/privacy/', '/404.html', ...sampleFunctions.map(name => `/docs/functions/${name}/`)];
const viewports = [{ name:'desktop', width:1440, height:900 }, { name:'mobile', width:390, height:844 }];
mkdirSync(new URL('../test-results/screenshots/', import.meta.url), { recursive:true });
for (const colorScheme of ['light','dark'] as const) {
  for (const viewport of viewports) {
    test.describe(`${colorScheme} ${viewport.name}`, () => {
      test.use({ colorScheme, viewport: { width:viewport.width, height:viewport.height } });
      for (const path of routes) {
        test(`renders ${path} without overflow or serious accessibility issues`, async ({ page }) => {
          await page.route('https://api.github.com/**', route => route.abort());
          await page.goto(path);
          await page.evaluate(() => document.fonts.ready);
          await expect(page.locator('h1')).toBeVisible();
          expect(await page.evaluate(() => document.documentElement.scrollWidth <= window.innerWidth)).toBe(true);
          const results = await new AxeBuilder({ page }).analyze();
          expect(results.violations.filter(issue => issue.impact === 'serious' || issue.impact === 'critical')).toEqual([]);
          const slug = path === '/' ? 'home' : path.replace(/[^a-zA-Z0-9-]/g,'_');
          await page.screenshot({ path: `test-results/screenshots/${slug}-${colorScheme}-${viewport.name}.png`, fullPage: true });
        });
      }
    });
  }
}
test('every function page is built with a heading and no double-escaped text', () => {
  expect(functions.length).toBeGreaterThan(750);
  for (const name of [...sampleFunctions, ...functions]) {
    const html = readFileSync(new URL(`../dist/docs/functions/${name}/index.html`, import.meta.url), 'utf8');
    expect(html, name).toMatch(/<h1[\s>]/);
    expect(html, name).not.toMatch(/&amp;(#\d+|[a-z]+);/);
  }
});
test('header shows Download once, as the primary button', async ({ page }) => {
  await page.setViewportSize({ width:1440, height:900 });
  await page.goto('/');
  await expect(page.locator('.header-nav a', { hasText:'Download' })).toHaveCount(0);
  await expect(page.locator('.header-download')).toBeVisible();
});
for (const width of [1440, 900]) test(`download buttons line up across platform cards at ${width} px`, async ({ page }) => {
  await page.setViewportSize({ width, height:900 });
  await page.route('https://api.github.com/**', route => route.abort());
  await page.goto('/download/');
  await expect(page.locator('.release-card.recommended [data-recommended]')).toBeVisible();
  for (const selector of ['.release-card h2', '[data-asset-download]']) {
    const tops = await page.locator(selector).evaluateAll(items => items.map(item => Math.round(item.getBoundingClientRect().top)));
    expect(tops).toHaveLength(3);
    expect(new Set(tops).size, selector).toBe(1);
  }
});
test('bundled release renders all assets with API blocked', async ({ page }) => {
  await page.route('https://api.github.com/**', route => route.abort());
  await page.goto('/download/');
  await expect(page.locator('.release-card')).toHaveCount(3);
  for (const os of ['windows','macos','linux']) {
    await expect(page.locator(`[data-os="${os}"] [data-sha256]`)).toHaveText(/^[a-f0-9]{64}$/);
    await expect(page.locator(`[data-os="${os}"] [data-asset-download]`)).toHaveAttribute('href', new RegExp(`v${ver}/MatRust-${ver}-${os}`));
  }
});
const agents = [
 ['windows','Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 Chrome/130.0.0.0 Safari/537.36'],
 ['macos','Mozilla/5.0 (Macintosh; Intel Mac OS X 14_0) AppleWebKit/537.36 Chrome/130.0.0.0 Safari/537.36'],
 ['linux','Mozilla/5.0 (X11; Linux x86_64) AppleWebKit/537.36 Chrome/130.0.0.0 Safari/537.36'],
];
for (const [os,userAgent] of agents) test(`detects ${os} download`, async ({ browser }) => {
  expect(detectPlatform(userAgent)).toBe(os);
  const context = await browser.newContext({ userAgent });
  // Use the supplied UA rather than the host's client hints.
  await context.addInitScript(() => { Object.defineProperty(navigator, 'userAgentData', { value: undefined }); });
  const page = await context.newPage();
  await page.route('https://api.github.com/**', route => route.abort());
  await page.goto('/download/');
  await expect(page.locator('.release-card').first()).toHaveAttribute('data-os',os);
  await expect(page.locator('.recommended')).toHaveAttribute('data-os',os);
  await page.goto('/');
  await expect(page.locator('[data-os-download]').first()).toHaveAttribute('href',new RegExp(`-${os}-`));
  await context.close();
});
test.describe('console with mock transport', () => {
  test.beforeEach(async ({ page }) => {
    await page.route('**/engine/worker.js', route => route.fulfill({ status:404, body:'Not found' }));
  });
  test('loads lazily, runs, shows a figure and workspace', async ({ page }) => {
    let engineRequests = 0;
    page.on('request', request => { if (request.url().includes('/engine/')) engineRequests++; });
    await page.goto('/try/');
    await expect(page.locator('#editor')).toHaveValue(/x = 1:5/);
    expect(engineRequests).toBe(0);
    await page.getByRole('button', { name:'Run', exact:true }).click();
    await expect(page.locator('#preview-banner')).toHaveText('Preview engine: results are canned');
    await expect(page.locator('#command-output')).toContainText('55');
    await expect(page.locator('.figure-card')).toHaveCount(1);
    await expect(page.locator('#workspace')).toContainText('double');
    await page.getByRole('button', { name:'Enlarge figure 1' }).click();
    await expect(page.getByRole('dialog')).toBeVisible();
    await page.keyboard.press('Escape');
    await expect(page.locator('#figure-dialog')).not.toBeVisible();
  });
  test('shares Unicode code and restores it on reload', async ({ page, context }) => {
    await context.grantPermissions(['clipboard-read','clipboard-write']);
    await page.goto('/try/');
    const code = "label = 'שלום π';\ndisp(label)";
    await page.locator('#editor').fill(code);
    await page.getByRole('button', { name:'Share', exact:true }).click();
    await expect(page).toHaveURL(/#code=v1z?\./);
    await expect(page.locator('#toast')).toHaveText('Link copied');
    await page.reload();
    await expect(page.locator('#editor')).toHaveValue(code);
    await expect(page.locator('#command-output')).toContainText('55');
  });
  test('keyboard run, Tab indentation, Stop and recovery', async ({ page }) => {
    await page.goto('/try/');
    await page.locator('#editor').fill('disp(2)');
    await page.locator('#editor').press('Control+Enter');
    await expect(page.locator('#command-output')).toContainText('55');
    await expect(page.locator('#run')).toBeEnabled();
    await page.locator('#editor').focus();
    await page.keyboard.press('Home');
    await page.keyboard.press('Tab');
    await expect(page.locator('#editor')).toHaveValue('    disp(2)');
    await page.getByRole('button', { name:'Run', exact:true }).click();
    await page.getByRole('button', { name:'Stop', exact:true }).click();
    await expect(page.locator('#command-output')).toContainText('Execution stopped');
    await expect(page.locator('#workspace')).toContainText('Workspace is empty');
    await page.getByRole('button', { name:'Run', exact:true }).click();
    await expect(page.locator('#command-output')).toContainText('55');
  });
  test('Esc then Tab moves focus out of the editor', async ({ page }) => {
    await page.goto('/try/');
    const editor = page.locator('#editor');
    await expect(page.locator('#editor-keys')).toContainText('Esc then Tab leaves the editor');
    await editor.fill('x = 1;');
    await editor.press('Tab');
    await expect(editor).toBeFocused();
    await expect(editor).toHaveValue('x = 1;    ');
    await editor.press('Escape');
    await page.keyboard.press('Tab');
    await expect(editor).not.toBeFocused();
    await expect(editor).toHaveValue('x = 1;    ');
    await editor.focus();
    await page.keyboard.press('Tab');
    await expect(editor).toHaveValue('x = 1;        ');
  });
  test('an engine crash during a run clears the workspace and recovers on the next Run', async ({ page }) => {
    await page.unroute('**/engine/worker.js');
    await page.route('**/engine/worker.js', route => route.fulfill({ contentType:'text/javascript', body: `self.onmessage = ({data}) => { if (data.code === 'crash') throw new Error('engine crash'); self.postMessage({id:data.id,result:JSON.stringify(data.type==='workspace'?[]:{output:'Recovered',error:null,figures:[],ms:1})}); };` }));
    await page.route('**/engine/matrust_web_bg.wasm', route => route.fulfill({ contentType:'application/wasm', body:Buffer.from([0,97,115,109,1,0,0,0]) }));
    await page.goto('/try/');
    await page.locator('#editor').fill('crash');
    await page.getByRole('button', { name:'Run', exact:true }).click();
    await expect(page.locator('#command-output')).toContainText('The engine stopped. The workspace was cleared.');
    await expect(page.locator('#command-output')).not.toContainText('could not load');
    await expect(page.locator('#workspace')).toContainText('Workspace is empty');
    await page.locator('#editor').fill('disp(1)');
    await page.getByRole('button', { name:'Run', exact:true }).click();
    await expect(page.locator('#command-output')).toContainText('Recovered');
  });
  test('a worker that fails to load says so and loads again on the next Run', async ({ page }) => {
    await page.unroute('**/engine/worker.js');
    let broken = true;
    await page.route('**/engine/worker.js', route => route.fulfill({ contentType:'text/javascript', body: broken ? 'self.onmessage = (' : `self.onmessage = ({data}) => self.postMessage({id:data.id,result:JSON.stringify(data.type==='workspace'?[]:{output:'Loaded',error:null,figures:[],ms:1})});` }));
    await page.route('**/engine/matrust_web_bg.wasm', route => route.fulfill({ contentType:'application/wasm', body:Buffer.from([0,97,115,109,1,0,0,0]) }));
    await page.goto('/try/');
    await page.getByRole('button', { name:'Run', exact:true }).click();
    await expect(page.locator('#command-output')).toContainText('The engine worker could not load. Try Run again.');
    broken = false;
    await page.getByRole('button', { name:'Run', exact:true }).click();
    await expect(page.locator('#command-output')).toContainText('Loaded');
  });
  test('automatically stops a runaway worker and recovers', async ({ page }) => {
    await page.unroute('**/engine/worker.js');
    await page.route('**/engine/worker.js', route => route.fulfill({ contentType:'text/javascript', body: `self.onmessage = ({data}) => { if (data.code === 'hang') return; self.postMessage({id:data.id,result:JSON.stringify(data.type==='workspace'?[]:{output:'Recovered',error:null,figures:[],ms:1})}); };` }));
    await page.route('**/engine/matrust_web_bg.wasm', route => route.fulfill({ contentType:'application/wasm', body:Buffer.from([0,97,115,109,1,0,0,0]) }));
    await page.goto('/try/');
    await page.locator('#editor').fill('hang');
    await page.getByRole('button', { name:'Run', exact:true }).click();
    await expect(page.locator('#command-output')).toContainText('Stopped after 10 seconds', { timeout:15000 });
    await page.locator('#editor').fill('disp(1)');
    await page.getByRole('button', { name:'Run', exact:true }).click();
    await expect(page.locator('#command-output')).toContainText('Recovered');
  });
});
test('function search, page and encoded example', async ({ page }) => {
  await page.goto('/docs/');
  await page.keyboard.press('/');
  await expect(page.locator('#function-search')).toBeFocused();
  await page.locator('#function-search').fill('linspace');
  await expect(page.locator('li[data-name="linspace"]')).toBeVisible();
  await page.locator('li[data-name="linspace"] a').click();
  await expect(page.locator('h1')).toHaveText('linspace');
  await expect(page.locator('.function-help')).toContainText('Syntax');
  await expect(page.getByRole('link', { name:'Try it', exact:true })).toHaveAttribute('href',/^\/try\/#code=v1\./);
});
test('theme persists and mobile menu works by keyboard', async ({ page }) => {
  await page.setViewportSize({ width:390, height:844 });
  await page.emulateMedia({ colorScheme:'light' });
  await page.goto('/');
  await page.getByRole('button', { name:'Switch to dark theme' }).click();
  await expect(page.locator('html')).toHaveAttribute('data-theme','dark');
  await page.reload();
  await expect(page.locator('html')).toHaveAttribute('data-theme','dark');
  await expect(page.locator('[data-dark-source]')).toHaveAttribute('media','all');
  await page.getByRole('button', { name:'Menu', exact:true }).focus();
  await page.keyboard.press('Enter');
  await expect(page.locator('#mobile-menu')).toBeVisible();
  await page.keyboard.press('Escape');
  await expect(page.locator('#mobile-menu')).not.toBeVisible();
  await expect(page.getByRole('button', { name:'Menu', exact:true })).toBeFocused();
});

test('prerelease API fallback is cached and checksum matches the live asset', async ({ page }) => {
  let latestCalls=0, listCalls=0;
  const fallback = JSON.parse(readFileSync(new URL('../src/data/release.json',import.meta.url),'utf8'));
  const release={tag_name:'v9.9.9-preview', assets:fallback.assets.map((asset: {name:string;size:number;url:string;sha256:string})=>({name:asset.name.replace(ver,'9.9.9'),size:asset.size,browser_download_url:asset.url.replaceAll(ver,'9.9.9'),digest:'sha256:'+asset.sha256}))};
  await page.route('https://api.github.com/repos/leoom-labs/matrust/releases/latest', route=>{latestCalls++;return route.fulfill({status:404,json:{message:'Not found'}});});
  await page.route('https://api.github.com/repos/leoom-labs/matrust/releases?per_page=1',route=>{listCalls++;return route.fulfill({json:[release]});});
  await page.goto('/download/');
  await expect(page.locator('#release-status')).toContainText('9.9.9-preview');
  await expect(page.locator('[data-filename]').first()).toContainText('9.9.9');
  await page.reload();
  await expect(page.locator('#release-status')).toContainText('9.9.9-preview');
  expect(latestCalls).toBe(1);expect(listCalls).toBe(1);
});

test('missing live checksums keep complete bundled download cards', async ({ page }) => {
  await page.route('https://api.github.com/**',route=>route.fulfill({json:{tag_name:'v99',assets:[{name:'MatRust-99-linux-x64.tar.gz',size:123,browser_download_url:'https://github.com/leoom-labs/matrust/releases/download/v99/MatRust-99-linux-x64.tar.gz'}]}}));
  await page.goto('/download/');
  await expect(page.locator('.release-card')).toHaveCount(3);
  await expect(page.locator('[data-filename]').first()).toContainText(ver);
  await expect(page.locator('#release-status')).toContainText('bundled');
});
