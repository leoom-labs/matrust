const input = document.querySelector<HTMLInputElement>('#function-search')!;
const rows = [...document.querySelectorAll<HTMLElement>('#functions-list li')];
const count = document.querySelector('#search-count')!;
let index: { name: string; summary: string }[] | undefined;
let pending: Promise<void> | undefined;
async function loadIndex() {
  if (!pending) pending = fetch('/docs/functions-index.json').then(response => {
    if (!response.ok) throw new Error('Index unavailable');
    return response.json();
  }).then(data => { index = data; }).catch(() => {
    // The server-rendered list remains searchable offline.
    index = rows.map(row => ({name: row.dataset.name!, summary: row.querySelector('.function-summary')!.textContent!}));
  });
  return pending;
}
async function search() {
  await loadIndex();
  const query = input.value.trim().toLowerCase();
  const matches = new Set(index!.filter(fn => `${fn.name} ${fn.summary}`.toLowerCase().includes(query)).map(fn => fn.name));
  let visible = 0;
  rows.forEach(row => { row.hidden = !matches.has(row.dataset.name!); if (!row.hidden) visible++; });
  count.textContent = visible ? `${visible} function${visible===1?'':'s'}` : 'No functions found. Try a different search.';
}
input.addEventListener('input', () => { void search(); });
input.addEventListener('focus', () => { void loadIndex(); });
document.addEventListener('keydown', event => {
  if (event.key === '/' && !event.ctrlKey && !event.metaKey && !event.altKey && !(event.target instanceof HTMLInputElement || event.target instanceof HTMLTextAreaElement || (event.target instanceof HTMLElement && event.target.isContentEditable))) { event.preventDefault(); input.focus(); }
});
