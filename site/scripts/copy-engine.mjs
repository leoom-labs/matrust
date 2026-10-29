import { readdir, mkdir, copyFile, stat, writeFile } from 'node:fs/promises';
const source = new URL('../../web/pkg/', import.meta.url);
const target = new URL('../public/engine/', import.meta.url);
let files = [];
try { files = await readdir(source, { withFileTypes: true }); }
catch (error) {
  if (error.code !== 'ENOENT') throw error;
  console.log('web/pkg is absent; retaining any supplied public/engine files. Preview worker is available.');
}
if (files.length) {
  await mkdir(target, { recursive: true });
  for (const file of files) {
    if (file.isFile()) await copyFile(new URL(file.name, source), new URL(file.name, target));
  }
  console.log(`Copied ${files.filter(file => file.isFile()).length} engine files.`);
}
// This also handles a bundle copied directly into public/engine by the orchestrator.
try {
  const wasm = await stat(new URL('matrust_web_bg.wasm', target));
  await writeFile(new URL('size.json', target), JSON.stringify({ bytes: wasm.size }) + '\n');
} catch (error) { if (error.code !== 'ENOENT') throw error; }
