// Preview transport only. Deliberately never evaluates submitted code.
// Every numerical value below is canned and must remain labelled in the UI.
let workspace = [];
self.onmessage = ({ data }) => {
  const { type, id } = data;
  if (type === 'reset') { workspace = []; self.postMessage({ id, result: null }); return; }
  if (type === 'workspace') { self.postMessage({ id, result: JSON.stringify(workspace) }); return; }
  if (type === 'version') { self.postMessage({ id, result: '0.1.0-preview' }); return; }
  if (type !== 'run') return;
  workspace = [{ name: 'x', class: 'double', size: '1 × 5', value: '[1 2 3 4 5]' }, { name: 'y', class: 'double', size: '1 × 5', value: '[1 4 9 16 25]' }];
  const svg = '<svg xmlns="http://www.w3.org/2000/svg" width="480" height="240" viewBox="0 0 480 240"><rect width="480" height="240" fill="#FFFFFF"/><g stroke="#DDD8D0"><path d="M48 24V200H456"/><path d="M48 112H456M48 68H456M48 156H456"/></g><path d="M48 112C82 20 116 20 150 112S218 204 252 112S320 20 354 112S422 204 456 112" fill="none" stroke="#C2410C" stroke-width="3"/><text x="48" y="228" font-family="monospace" font-size="14" fill="#57534E">Preview figure · canned sine wave</text></svg>';
  setTimeout(() => self.postMessage({ id, result: JSON.stringify({ output: 'Preview output (canned)\n     1     4     9    16    25\n    55\n', error: null, figures: [{ id: 1, svg }], ms: 1 }) }), 200);
};
