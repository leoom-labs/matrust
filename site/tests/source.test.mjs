import { test } from 'node:test';
import assert from 'node:assert/strict';
import { readFileSync } from 'node:fs';
import { Worker } from 'node:worker_threads';
import { splitFunctions, functions } from '../src/lib/functions.ts';
import { detectPlatform } from '../src/lib/platform.ts';
import { encodeCode, decodeCode, plainCodeHash, MAX_CODE_BYTES } from '../src/lib/share.ts';
import release from '../src/data/release.json' with { type:'json' };

test('help split preserves every source section, its first example and evidence notes', () => {
  const source = readFileSync(new URL('../../assets/help/functions.md', import.meta.url),'utf8');
  const count = new Set([...source.matchAll(/^## ([^\r\n]+)/gm)].map(match=>match[1])).size;
  assert.equal(functions.length,count);
  assert.ok(count>750);
  assert.equal(new Set(functions.map(fn=>fn.name)).size,count);
  assert.ok(functions.find(fn=>fn.name==='linspace').example.includes('linspace'));
  const input = '## demo\nFirst summary.\nSyntax: demo(A)\nExample: demo([1 2])\nCompatibility notes: Only scalar input; matrices are rejected.\n';
  const result = splitFunctions(input)[0];
  assert.equal(result.summary,'First summary.');
  assert.equal(result.example,'demo([1 2])');
  assert.ok(result.html.includes('matrices are rejected'));
  assert.ok(result.html.includes('Only scalar input'));
  assert.throws(()=>splitFunctions('## bad/name\nHelp'),/Invalid/);
  assert.equal(splitFunctions('## a\nFirst\n## a\nSecond')[0].summary,'Second');
  assert.ok(splitFunctions('## a\nFirst\n## a\nSecond')[0].html.includes('First'));
  assert.ok(functions.find(fn=>fn.name==='containers.Map'));
  assert.match(functions.find(fn=>fn.name==='norm').html,/Vectors only/);
  assert.match(functions.find(fn=>fn.name==='norm').html,/Vector and matrix norms/);
});
test('release fallback is complete and version-consistent', () => {
  assert.match(release.version,/^v\d+\.\d+\.\d+$/);
  assert.deepEqual(release.assets.map(asset=>asset.os).sort(),['linux','macos','windows']);
  for (const asset of release.assets) {
    assert.match(asset.sha256,/^[a-f0-9]{64}$/);
    assert.ok(asset.size>20000000);
    assert.ok(asset.url.endsWith('/'+asset.name));
    assert.ok(asset.url.includes(`/${release.version}/`));
    assert.ok(asset.name.includes(release.version.slice(1)));
  }
});
test('platform detection excludes mobile devices', () => {
  assert.equal(detectPlatform('Windows NT 10.0'),'windows');
  assert.equal(detectPlatform('Macintosh; Intel Mac OS X'),'macos');
  assert.equal(detectPlatform('X11; Linux x86_64'),'linux');
  assert.equal(detectPlatform('Linux; Android 14'),null);
  assert.equal(detectPlatform('iPhone; CPU iPhone OS 17_0 like Mac OS X'),null);
  assert.equal(detectPlatform('unknown'),null);
});
test('versioned sharing round trips Unicode, plain and compressed links', async () => {
  const code = "s = 'שלום π';\ndisp(s)\n".repeat(200);
  const compressed = await encodeCode(code);
  assert.match(compressed,/^v1z\./);
  assert.equal(await decodeCode(compressed),code);
  assert.equal(await decodeCode(plainCodeHash(code).slice(6)),code);
  assert.equal(await decodeCode(await encodeCode('')),'');
  await assert.rejects(decodeCode('v9.Zg'),/Unsupported/);
  await assert.rejects(decodeCode('v1.%%%%'),/Invalid/);
  await assert.rejects(encodeCode('a'.repeat(MAX_CODE_BYTES+1)),/limit/);
});
test('compressed payloads cannot expand beyond the code bound', async () => {
  const bytes = new TextEncoder().encode('a'.repeat(MAX_CODE_BYTES+1));
  const compressed = new Uint8Array(await new Response(new Blob([bytes]).stream().pipeThrough(new CompressionStream('deflate'))).arrayBuffer());
  await assert.rejects(decodeCode('v1z.'+Buffer.from(compressed).toString('base64url')),/limit/);
});
test('mock worker implements run, workspace, reset and version contract', async () => {
  const worker = new Worker(new URL('../scripts/mock-node-runner.mjs',import.meta.url));
  let id=0;
  const request = (type,code) => new Promise((resolve,reject) => { worker.once('message',resolve);worker.once('error',reject);worker.postMessage({type,code,id:++id}); });
  try {
    const response = await request('run','anything');
    const result = JSON.parse(response.result);
    assert.equal(response.id,1);
    assert.match(result.output,/canned/);
    assert.equal(result.error,null);
    assert.match(result.figures[0].svg,/^<svg/);
    const workspace = JSON.parse((await request('workspace')).result);
    assert.equal(workspace[0].class,'double');
    await request('reset');
    assert.deepEqual(JSON.parse((await request('workspace')).result),[]);
    assert.match((await request('version')).result,/preview/);
  } finally { await worker.terminate(); }
});
test('sharing works without CompressionStream', async () => {
  const compression=globalThis.CompressionStream;
  try {
    globalThis.CompressionStream=undefined;
    const encoded=await encodeCode("disp('π')");
    assert.match(encoded,/^v1\./);
    assert.equal(await decodeCode(encoded),"disp('π')");
  } finally {globalThis.CompressionStream=compression;}
});
test('help text renders verbatim: apostrophes, Markdown and HTML characters, code listings', () => {
  const decode = html => html.replace(/<[^>]+>/g, '').replace(/&(amp|lt|gt|quot|#39);/g, (_, name) => ({ amp:'&', lt:'<', gt:'>', quot:'"', '#39':"'" })[name]).trim();
  const line = "Interpreter must be 'none' & <b>x</b> *a* _b_ `c` [l](u) #1 \\n ~s~ 1. - > |t| \"q\" www.example.com";
  assert.equal(decode(splitFunctions(`## demo\n${line}\n`)[0].html), line);
  const annotation = functions.find(fn => fn.name === 'annotation').html;
  assert.ok(decode(annotation).includes("Interpreter must be 'none'"));
  const listing = functions.find(fn => fn.name === 'function').html;
  assert.ok(listing.includes('<pre><code>function y = twice(x)\ny = 2 * x;\nend</code></pre>'));
  for (const fn of functions) {
    assert.doesNotMatch(fn.html, /&amp;(#\d+|[a-z]+);/, fn.name);
    const text = decode(fn.html);
    for (const source of fn.text.split('\n').map(line => line.trim()).filter(line => line && !/^(Syntax|Example):/.test(line))) assert.ok(text.includes(source), `${fn.name}: ${source}`);
  }
});
