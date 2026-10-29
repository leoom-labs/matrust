// Focused development proof. The orchestrator runs the separate production gate.
import { chromium } from 'playwright';
import AxeBuilder from '@axe-core/playwright';
import assert from 'node:assert/strict';
import { mkdir, writeFile } from 'node:fs/promises';
// PROBE_BASE: a running site, e.g. `npm run preview -- --port 4391 --ignore-lock`.
const base = process.env.PROBE_BASE ?? 'http://127.0.0.1:4322';
await mkdir(new URL('../test-results/probe/',import.meta.url),{recursive:true});
const browser = await chromium.launch({headless:true});
const errors=[];
try {
  for (const colorScheme of ['light','dark']) {
    for (const viewport of [{width:1440,height:900},{width:390,height:844}]) {
      const context=await browser.newContext({colorScheme,viewport});
      const page=await context.newPage();
      page.on('pageerror',error=>errors.push(error.message));
      await page.route('https://api.github.com/**',route=>route.abort());
      for(const path of ['/','/download/','/try/','/docs/functions/annotation/']) {
        const response=await page.goto(base+path);
        assert.equal(response.status(),200,`${path} HTTP`);
        await page.waitForLoadState('networkidle');
        await page.evaluate(()=>document.fonts.ready);
        assert.ok(await page.locator('h1').isVisible(),`${path} h1`);
        assert.ok(await page.evaluate(()=>document.documentElement.scrollWidth<=innerWidth),`${path} overflow ${colorScheme} ${viewport.width}`);
        const axe=await new AxeBuilder({page}).analyze();
        const issues=axe.violations.filter(issue=>['serious','critical'].includes(issue.impact));
        assert.deepEqual(issues.map(issue=>({id:issue.id,nodes:issue.nodes.map(n=>n.target)})),[],`axe ${path} ${colorScheme}`);
        const slug=path==='/'?'home':path.replaceAll('/','_');
        await page.screenshot({path:new URL(`../test-results/probe/${slug}-${colorScheme}-${viewport.width}.png`,import.meta.url).pathname,fullPage:true});
      }
      await context.close();
    }
  }
  const context=await browser.newContext({permissions:['clipboard-read','clipboard-write']});
  const page=await context.newPage();
  page.on('pageerror',error=>errors.push(error.message));
  await page.route('**/engine/worker.js',route=>route.fulfill({status:404,body:'not found'}));
  await page.goto(base+'/try/');
  await page.locator('#run').click();
  await page.waitForFunction(()=>document.querySelector('#command-output').textContent.includes('55'));
  assert.equal(await page.locator('.figure-card').count(),1);
  assert.ok(await page.locator('#preview-banner').isVisible());
  assert.match(await page.locator('#workspace').textContent(),/double/);
  await page.locator('#editor').fill("disp('שלום π')");
  await page.locator('#share').click();
  await page.waitForFunction(()=>location.hash.startsWith('#code='));
  await page.reload();
  assert.equal(await page.locator('#editor').inputValue(),"disp('שלום π')");
  await page.waitForFunction(()=>!document.querySelector('#run').disabled);
  await page.locator('#run').click();
  await page.locator('#stop').click();
  assert.match(await page.locator('#command-output').textContent(),/Execution stopped/);
  await page.locator('#run').click();
  await page.waitForFunction(()=>document.querySelector('#command-output').textContent.includes('55'));
  await page.goto(base+'/docs/');
  await page.keyboard.press('/');
  await page.locator('#function-search').fill('linspace');
  await page.waitForFunction(()=>!document.querySelector('[data-name="linspace"]').hidden && document.querySelector('[data-name="abs"]').hidden);
  await page.locator('[data-name="linspace"] a').click();
  assert.match(await page.getByRole('link',{name:'Try it',exact:true}).getAttribute('href'),/#code=/);
  await context.close();
  assert.deepEqual(errors,[],'browser page errors');
  const result='PASS: 16 representative page/theme/viewport renders, no overflow or serious/critical axe findings; mock run/figure/workspace, Unicode share restore, Stop recovery and function search.\n';
  await writeFile(new URL('../test-results/probe/result.txt',import.meta.url),result);
  console.log(result);
} finally {await browser.close();}
