import { EngineClient, type Figure, type WorkspaceEntry } from './engine-client';
import { examples } from './examples';
import { encodeCode, decodeCode, MAX_CODE_BYTES } from '../lib/share';
import { copyText, toast } from '../lib/ui';
const editor = document.querySelector<HTMLTextAreaElement>('#editor')!;
const picker = document.querySelector<HTMLSelectElement>('#example-picker')!;
const runButton = document.querySelector<HTMLButtonElement>('#run')!;
const stopButton = document.querySelector<HTMLButtonElement>('#stop')!;
const resetButton = document.querySelector<HTMLButtonElement>('#reset')!;
const output = document.querySelector<HTMLElement>('#command-output')!;
const status = document.querySelector<HTMLElement>('#run-status')!;
const gutter = document.querySelector<HTMLElement>('#line-numbers')!;
const loading = document.querySelector<HTMLElement>('#engine-loading')!;
const progress = document.querySelector<HTMLProgressElement>('#engine-progress')!;
const label = document.querySelector<HTMLElement>('#loading-label')!;
const preview = document.querySelector<HTMLElement>('#preview-banner')!;
const figures = document.querySelector<HTMLElement>('#figures')!;
const figurePlaceholder = document.querySelector<HTMLElement>('#figure-placeholder')!;
const workspaceBody = document.querySelector<HTMLTableSectionElement>('#workspace')!;
const dialog = document.querySelector<HTMLDialogElement>('#figure-dialog')!;
const largeImage = document.querySelector<HTMLImageElement>('#figure-large')!;
const figureUrls: string[] = [];
let busy = false;
let action = 0;
let outputLength = 0;
const client = new EngineClient(value => {
  loading.hidden = !value;
  if (!value) return;
  label.textContent = value.preview ? 'Loading preview engine…' : value.total ? `Loading MatRust (about ${(value.total/1048576).toFixed(1)} MB, once)` : 'Loading MatRust (measuring download size, once)…';
  if (value.total) { progress.max = value.total; progress.value = Math.min(value.loaded, value.total); }
  else progress.removeAttribute('value');
}, value => { preview.hidden = !value; });
function lineNumbers() {
  const lines = Math.min(editor.value.split('\n').length, MAX_CODE_BYTES);
  gutter.textContent = Array.from({ length: lines }, (_,i) => String(i+1)).join('\n');
  gutter.scrollTop = editor.scrollTop;
}
function setBusy(value: boolean) {
  busy = value; runButton.disabled = value; stopButton.disabled = !value; resetButton.disabled = value; picker.disabled = value;
  status.textContent = value ? 'Running…' : 'Ready';
}
function append(text: string, error = false) {
  const MAX_OUTPUT = 1048576;
  if (outputLength >= MAX_OUTPUT) return;
  const available = MAX_OUTPUT - outputLength;
  const pre = document.createElement('pre');
  pre.classList.toggle('error', error);
  pre.textContent = text.slice(0, available) + (text.length > available ? '\n[Output truncated at 1 MB]' : '');
  outputLength += Math.min(text.length, available);
  output.append(pre);
  output.scrollTop = output.scrollHeight;
}
function clearFigures() {
  dialog.close(); largeImage.removeAttribute('src');
  figureUrls.forEach(url => URL.revokeObjectURL(url)); figureUrls.length = 0;
  figures.replaceChildren(); figures.hidden = true; figurePlaceholder.hidden = false;
}
function renderFigures(items: Figure[]) {
  clearFigures();
  if (!items.length) return;
  figures.hidden = false; figurePlaceholder.hidden = true;
  for (const figure of items.slice(0,16)) {
    // SVG is displayed as an image, never inserted into the document. Image-mode
    // SVG disables scripts and external resources, including on shared-code runs.
    if (typeof figure.svg !== 'string' || figure.svg.length > 4194304) continue;
    const url = URL.createObjectURL(new Blob([figure.svg], { type: 'image/svg+xml' }));
    figureUrls.push(url);
    const card = document.createElement('figure'); card.className = 'figure-card';
    const button = document.createElement('button'); button.type = 'button'; button.className = 'figure-open'; button.setAttribute('aria-label', `Enlarge figure ${figure.id}`);
    const image = document.createElement('img'); image.src = url; image.alt = `Figure ${figure.id} from the script`; image.width = 480; image.height = 240;
    button.append(image); button.addEventListener('click', () => { largeImage.src = url; dialog.showModal(); });
    const caption = document.createElement('figcaption');
    const name = document.createElement('span'); name.textContent = `Figure ${figure.id}`;
    const download = document.createElement('a'); download.href = url; download.download = `matrust-figure-${figure.id}.svg`; download.textContent = 'Download SVG';
    caption.append(name, download); card.append(button, caption); figures.append(card);
  }
}
function renderWorkspace(items: WorkspaceEntry[]) {
  workspaceBody.replaceChildren();
  if (!items.length) {
    const row = workspaceBody.insertRow(); const cell = row.insertCell(); cell.colSpan = 4; cell.className = 'workspace-empty'; cell.textContent = 'Workspace is empty.'; return;
  }
  for (const item of items.slice(0,1000)) {
    const row = workspaceBody.insertRow();
    for (const value of [item.name, item.class, Array.isArray(item.size) ? item.size.join(' × ') : item.size, item.value]) {
      const cell = row.insertCell(); cell.textContent = String(value).slice(0,4096);
    }
  }
}
async function run() {
  if (busy) return;
  if (new TextEncoder().encode(editor.value).length > MAX_CODE_BYTES) { toast('Code exceeds the 64 KB limit.'); return; }
  const current = ++action;
  setBusy(true); output.replaceChildren(); outputLength = 0;
  try {
    const result = await client.run(editor.value);
    if (current !== action) return;
    if (result.output) append(result.output);
    if (result.error) append(result.error, true);
    if (!result.output && !result.error) append('Done.');
    renderFigures(result.figures ?? []);
    renderWorkspace(await client.workspace());
    if (current === action) { setBusy(false); status.textContent = result.error ? 'Error' : `${Number(result.ms).toFixed(0)} ms${preview.hidden ? '' : ' · preview'}`; }
  } catch (error) {
    if (current !== action) return;
    append(error instanceof Error ? error.message : 'The engine could not run this script.', true);
    renderWorkspace([]); clearFigures();
  } finally { if (current === action && busy) setBusy(false); }
}
runButton.addEventListener('click', () => { void run(); });
stopButton.addEventListener('click', () => {
  action++; client.stop(); setBusy(false); append('Execution stopped. Workspace cleared.', true); renderWorkspace([]); clearFigures(); status.textContent = 'Stopped';
});
resetButton.addEventListener('click', () => {
  void (async () => {
    if (busy) return;
    const current = ++action;
    setBusy(true); status.textContent = 'Resetting…';
    try {
      // Clearing an unused console never downloads the engine.
      await client.reset();
      if (current !== action) return;
      editor.value = examples[Number(picker.value)].code;
      output.replaceChildren(); outputLength = 0; append('Session reset.');
      renderWorkspace([]); clearFigures(); lineNumbers();
      history.replaceState(null, '', location.pathname + location.search);
    } catch (error) {
      if (current === action) append(error instanceof Error ? error.message : 'Reset failed.', true);
    } finally { if (current === action) setBusy(false); }
  })();
});
picker.addEventListener('change', () => { editor.value = examples[Number(picker.value)].code; lineNumbers(); });
editor.addEventListener('input', lineNumbers);
editor.addEventListener('scroll', () => { gutter.scrollTop = editor.scrollTop; });
// Tab indents. Esc then Tab leaves the editor, so keyboard users are never trapped.
let tabLeaves = false;
editor.addEventListener('keydown', event => {
  if (event.key === 'Escape') { tabLeaves = true; return; }
  const leave = tabLeaves;
  tabLeaves = false;
  if (event.key === 'Tab' && !event.shiftKey && !leave) {
    event.preventDefault(); editor.setRangeText('    ', editor.selectionStart, editor.selectionEnd, 'end'); lineNumbers();
  }
});
editor.addEventListener('blur', () => { tabLeaves = false; });
document.addEventListener('keydown', event => {
  if (event.key === 'Enter' && (event.ctrlKey || event.metaKey)) { event.preventDefault(); void run(); }
});
document.querySelector('#share')!.addEventListener('click', () => {
  void (async () => {
    try {
      const code = await encodeCode(editor.value);
      const url = new URL(location.href); url.hash = `code=${code}`;
      history.replaceState(null, '', url);
      await copyText(url.href, 'Link copied');
    } catch (error) { toast(error instanceof Error ? error.message : 'Could not create a share link.'); }
  })();
});
// The issue form opens with the share link and the output filled in (GitHub caps URL length).
const report = document.querySelector<HTMLAnchorElement>('#report')!;
report.addEventListener('click', event => {
  event.preventDefault();
  void (async () => {
    const form = new URL(report.href);
    try {
      const link = new URL(location.href); link.hash = `code=${await encodeCode(editor.value)}`;
      form.searchParams.set('script', link.href.length <= 4000 ? link.href : editor.value.slice(0, 2000));
    } catch { form.searchParams.set('script', editor.value.slice(0, 2000)); }
    form.searchParams.set('got', (output.textContent ?? '').slice(0, 1500));
    window.open(form.href, '_blank', 'noopener');
  })();
});
document.querySelector('#figure-close')!.addEventListener('click', () => dialog.close());
window.addEventListener('pagehide', () => { client.dispose(); clearFigures(); });
async function restore() {
  const params = new URLSearchParams(location.hash.slice(1));
  if (!params.has('code')) return;
  try { editor.value = await decodeCode(params.get('code')!); lineNumbers(); await run(); }
  catch (error) { output.replaceChildren(); outputLength = 0; append(error instanceof Error ? error.message : 'Invalid share link.', true); }
}
lineNumbers();
window.addEventListener('hashchange', () => { if (!busy) void restore(); });
void restore();
