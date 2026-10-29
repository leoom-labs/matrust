import fallback from '../data/release.json';
import { visitorPlatform } from '../lib/platform';
import { copyText } from '../lib/ui';
interface ReleaseAsset { name: string; size: number; browser_download_url: string; digest?: string | null; }
interface GithubRelease { tag_name: string; assets: ReleaseAsset[]; }
interface Cached { timestamp: number; release: GithubRelease; }
const cacheKey = 'matrust-release-v1';
const endpoint = 'https://api.github.com/repos/leoom-labs/matrust';
const patterns: Record<string, RegExp> = { windows: /windows.*\.zip$/i, macos: /macos.*\.tar\.gz$/i, linux: /linux.*\.tar\.gz$/i };
const commands: Record<string,string> = { windows: 'Get-FileHash .\\{file} -Algorithm SHA256', macos: 'shasum -a 256 {file}', linux: 'sha256sum {file}' };
const container = document.querySelector('#release-assets')!;
const os = visitorPlatform();
const recommended = os && container.querySelector<HTMLElement>(`[data-os="${os}"]`);
if (recommended) { container.prepend(recommended); recommended.classList.add('recommended'); recommended.querySelector<HTMLElement>('[data-recommended]')!.hidden = false; }
container.querySelectorAll<HTMLElement>('[data-os]').forEach(card => card.querySelector('button')!.addEventListener('click', () => copyText(card.querySelector('[data-sha256]')!.textContent!, 'SHA-256 copied')));
function isRelease(value: unknown): value is GithubRelease {
  if (!value || typeof value !== 'object') return false;
  const release = value as GithubRelease;
  return typeof release.tag_name === 'string' && Array.isArray(release.assets) && release.assets.every(asset => asset && typeof asset.name === 'string' && typeof asset.browser_download_url === 'string' && typeof asset.size === 'number' && (asset.digest === null || asset.digest === undefined || typeof asset.digest === 'string'));
}
function render(release: GithubRelease): boolean {
  // Update atomically: never pair a new archive with a checksum from a different version.
  const matches = fallback.assets.map(bundled => {
    const asset = release.assets.find(asset => typeof asset.name === 'string' && patterns[bundled.os].test(asset.name));
    if (!asset || !Number.isSafeInteger(asset.size) || asset.size <= 0 || !asset.browser_download_url?.startsWith('https://github.com/leoom-labs/matrust/releases/download/')) return null;
    const digest = asset.digest?.match(/^sha256:([a-f0-9]{64})$/i)?.[1]?.toLowerCase();
    const sha256 = digest || (asset.name === bundled.name && release.tag_name === fallback.version ? bundled.sha256 : null);
    return sha256 ? { ...asset, os: bundled.os, sha256 } : null;
  });
  if (matches.some(asset => !asset)) return false;
  for (const asset of matches) {
    if (!asset) continue;
    const card = container.querySelector<HTMLElement>(`[data-os="${asset.os}"]`)!;
    card.querySelector('[data-filename]')!.textContent = asset.name;
    card.querySelector('[data-size]')!.textContent = `${(asset.size/1048576).toFixed(1)} MB · ${asset.size.toLocaleString('en-US')} bytes`;
    card.querySelector('[data-sha256]')!.textContent = asset.sha256;
    card.querySelector<HTMLAnchorElement>('[data-asset-download]')!.href = asset.browser_download_url;
    document.querySelector(`[data-verify="${asset.os}"]`)!.textContent = commands[asset.os].replace('{file}', asset.name);
  }
  document.querySelector('#release-status')!.textContent = `Version ${release.tag_name.replace(/^v/,'')} · Release data from GitHub.`;
  return true;
}
async function fetchRelease(path: string): Promise<unknown> {
  const response = await fetch(endpoint + path, { signal: AbortSignal.timeout(5000), headers: { Accept: 'application/vnd.github+json' } });
  if (!response.ok) throw new Error(`GitHub ${response.status}`);
  return response.json();
}
async function refresh() {
  try {
    const cached: Cached = JSON.parse(localStorage.getItem(cacheKey) || 'null');
    if (cached && Number.isFinite(cached.timestamp) && Date.now() >= cached.timestamp && Date.now() - cached.timestamp < 600000 && isRelease(cached.release) && render(cached.release)) return;
  } catch { /* Bundled cards work without storage. */ }
  let release: unknown;
  try { release = await fetchRelease('/releases/latest'); } catch { /* Prereleases are excluded from latest. */ }
  if (!isRelease(release) || !render(release)) {
    try { const releases = await fetchRelease('/releases?per_page=1'); release = Array.isArray(releases) ? releases[0] : null; } catch { return; }
  }
  if (isRelease(release) && render(release)) {
    try { localStorage.setItem(cacheKey, JSON.stringify({ timestamp: Date.now(), release })); } catch { /* Optional cache. */ }
  }
}
void refresh();
