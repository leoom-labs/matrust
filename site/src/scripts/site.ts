import release from '../data/release.json';
import { visitorPlatform, platformNames } from '../lib/platform';
const themeButton = document.querySelector<HTMLButtonElement>('#theme-toggle')!;
function syncTheme() {
  const dark = document.documentElement.dataset.theme === 'dark';
  themeButton.textContent = dark ? 'Light' : 'Dark';
  themeButton.setAttribute('aria-label', `Switch to ${dark ? 'light' : 'dark'} theme`);
  document.querySelectorAll<HTMLSourceElement>('[data-dark-source]').forEach(source => { source.media = dark ? 'all' : 'not all'; });
  document.querySelector('meta[name="theme-color"]')?.setAttribute('content', dark ? '#121418' : '#F7F5F2');
}
syncTheme();
themeButton.addEventListener('click', () => {
  const next = document.documentElement.dataset.theme === 'dark' ? 'light' : 'dark';
  document.documentElement.dataset.theme = next;
  try { localStorage.setItem('matrust-theme', next); } catch { /* Private storage can be unavailable. */ }
  syncTheme();
});
matchMedia('(prefers-color-scheme: dark)').addEventListener('change', event => {
  let saved; try { saved = localStorage.getItem('matrust-theme'); } catch { /* Use system preference. */ }
  if (saved !== 'dark' && saved !== 'light') {
    document.documentElement.dataset.theme = event.matches ? 'dark' : 'light'; syncTheme();
  }
});
const menu = document.querySelector<HTMLDialogElement>('#mobile-menu')!;
document.querySelector('#menu-toggle')?.addEventListener('click', () => menu.showModal());
document.querySelector('#menu-close')?.addEventListener('click', () => menu.close());
menu.querySelectorAll('a').forEach(link => link.addEventListener('click', () => menu.close()));
menu.addEventListener('click', event => { if (event.target === menu) { const r = menu.getBoundingClientRect(); if (event.clientX < r.left || event.clientX > r.right || event.clientY < r.top || event.clientY > r.bottom) menu.close(); } });
const os = visitorPlatform();
const asset = release.assets.find(asset => asset.os === os);
if (os && asset) document.querySelectorAll<HTMLAnchorElement>('[data-os-download]').forEach(link => {
  link.textContent = `Download for ${platformNames[os]}`;
  link.href = asset.url;
});
