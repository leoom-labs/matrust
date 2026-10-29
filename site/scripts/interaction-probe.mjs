// Focused development checks for interactions omitted from the visual probe.
import { chromium } from 'playwright';
import assert from 'node:assert/strict';
import AxeBuilder from '@axe-core/playwright';
const base='http://127.0.0.1:4322';
const browser=await chromium.launch({headless:true});
try {
  for(const [os,userAgent] of [
    ['windows','Mozilla/5.0 (Windows NT 10.0; Win64; x64) Chrome/130.0.0.0'],
    ['macos','Mozilla/5.0 (Macintosh; Intel Mac OS X 14_0) Chrome/130.0.0.0'],
    ['linux','Mozilla/5.0 (X11; Linux x86_64) Chrome/130.0.0.0'],
  ]) {
    const context=await browser.newContext({userAgent});
    await context.addInitScript(()=>Object.defineProperty(navigator,'userAgentData',{value:undefined}));
    const page=await context.newPage();
    await page.route('https://api.github.com/**',route=>route.abort());
    await page.goto(base+'/download/');await page.waitForLoadState('networkidle');
    assert.equal(await page.locator('.release-card').first().getAttribute('data-os'),os);
    assert.equal(await page.locator('.release-card').count(),3);
    await page.goto(base+'/');await page.waitForLoadState('networkidle');
    assert.ok((await page.locator('[data-os-download]').first().getAttribute('href')).includes('-'+os+'-'));
    await context.close();
  }
  const mobileContext=await browser.newContext({viewport:{width:390,height:844},colorScheme:'light'});
  const page=await mobileContext.newPage();
  await page.goto(base+'/');await page.waitForLoadState('networkidle');
  await page.getByRole('button',{name:'Switch to dark theme'}).click();
  await page.reload();await page.waitForLoadState('networkidle');
  assert.equal(await page.locator('html').getAttribute('data-theme'),'dark');
  assert.equal(await page.locator('[data-dark-source]').getAttribute('media'),'all');
  await page.getByRole('button',{name:'Menu',exact:true}).focus();await page.keyboard.press('Enter');
  assert.ok(await page.locator('#mobile-menu').isVisible());await page.keyboard.press('Escape');
  assert.equal(await page.locator('#mobile-menu').isVisible(),false);
  assert.equal(await page.getByRole('button',{name:'Menu',exact:true}).evaluate(element=>element===document.activeElement),true);
  await page.route('**/engine/worker.js',route=>route.fulfill({status:404,body:'missing'}));
  await page.goto(base+'/try/');await page.waitForLoadState('networkidle');
  await page.locator('#editor').press('Control+Enter');
  await page.waitForFunction(()=>document.querySelector('#command-output').textContent.includes('55'));
  await page.waitForFunction(()=>!document.querySelector('#run').disabled);
  await page.locator('#reset').click();
  await page.waitForFunction(()=>document.querySelector('#command-output').textContent.includes('Session reset'));
  assert.match(await page.locator('#workspace').textContent(),/Workspace is empty/);
  await page.unroute('**/engine/worker.js');
  await page.route('**/engine/worker.js',route=>route.fulfill({contentType:'text/javascript',body:`self.onmessage=({data})=>{if(data.code==='hang')return;self.postMessage({id:data.id,result:JSON.stringify(data.type==='workspace'?[]:{output:'Recovered',error:null,figures:[],ms:1})})}` }));
  await page.route('**/engine/matrust_web_bg.wasm',route=>route.fulfill({contentType:'application/wasm',body:Buffer.from([0,97,115,109,1,0,0,0])}));
  await page.goto(base+'/try/');await page.waitForLoadState('networkidle');
  await page.locator('#editor').fill('hang');await page.locator('#run').click();
  await page.waitForFunction(()=>document.querySelector('#command-output').textContent.includes('Stopped after 10 seconds'),{},{timeout:15000});
  await page.locator('#editor').fill('disp(1)');await page.locator('#run').click();
  await page.waitForFunction(()=>document.querySelector('#command-output').textContent.includes('Recovered'));
  for(const path of ['/privacy/','/404.html','/docs/functions/containers.Map/','/docs/functions/isequalwithequalnans/','/docs/functions/norm/']) {
    await page.goto(base+path);await page.waitForLoadState('networkidle');
    assert.ok(await page.locator('h1').isVisible(),path);
    assert.ok(await page.evaluate(()=>document.documentElement.scrollWidth<=innerWidth),path+' overflow');
    const axe=await new AxeBuilder({page}).analyze();
    assert.deepEqual(axe.violations.filter(issue=>['serious','critical'].includes(issue.impact)).map(issue=>issue.id),[],path+' axe');
    await page.screenshot({path:new URL('../test-results/probe/edge-'+path.replaceAll('/','_')+'-dark-390.png',import.meta.url).pathname,fullPage:true});
  }
  await mobileContext.close();
  console.log('PASS: 3 OS assets, fallback cards, saved theme, keyboard mobile menu, Ctrl+Enter, Reset, 10-second timeout and restart; privacy/404 and dotted, longest, duplicate function pages render without overflow or serious/critical axe issues.');
} finally {await browser.close();}
