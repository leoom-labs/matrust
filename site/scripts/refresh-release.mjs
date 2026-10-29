import { readFile, writeFile } from 'node:fs/promises';
const repository = 'https://api.github.com/repos/leoom-labs/matrust';
async function json(path) {
  const response = await fetch(repository + path, { headers: { Accept: 'application/vnd.github+json', 'User-Agent': 'MatRust-release-refresh' } });
  if (!response.ok) throw new Error(`GitHub ${response.status}: ${path}`);
  return response.json();
}
// This fallback is deliberately pinned to the initial release, including prereleases.
const tag = process.argv[2] ?? JSON.parse(await readFile(new URL('../src/data/release.json', import.meta.url), 'utf8')).version;
const release = await json(`/releases/tags/${tag}`);
const platforms = [['windows', /windows.*\.zip$/i], ['macos', /macos.*\.tar\.gz$/i], ['linux', /linux.*\.tar\.gz$/i]];
const assets = [];
for (const [os, pattern] of platforms) {
  const asset = release.assets.find(asset => pattern.test(asset.name));
  if (!asset) throw new Error(`Missing ${os} archive`);
  const checksum = release.assets.find(file => file.name === `${asset.name}.sha256`);
  if (!checksum) throw new Error(`Missing checksum for ${asset.name}`);
  const response = await fetch(checksum.browser_download_url);
  if (!response.ok) throw new Error(`Checksum HTTP ${response.status}`);
  const text = await response.text();
  const sha256 = text.match(/^([a-f0-9]{64})\s+\*?(.+)$/im);
  if (!sha256 || sha256[2].trim() !== asset.name) throw new Error(`Invalid checksum for ${asset.name}`);
  assets.push({ os, name: asset.name, size: asset.size, sha256: sha256[1].toLowerCase(), url: asset.browser_download_url });
}
await writeFile(new URL('../src/data/release.json', import.meta.url), JSON.stringify({ version: release.tag_name, url: release.html_url, assets }, null, 2) + '\n');
console.log(`Saved verified checksums and sizes for ${assets.length} ${release.tag_name} archives.`);
