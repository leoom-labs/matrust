export function escapeHtml(text: string): string {
  return text.replace(/[&<>"']/g, char => ({ '&': '&amp;', '<': '&lt;', '>': '&gt;', '"': '&quot;', "'": '&#39;' })[char]!);
}
export function highlight(code: string): string {
  const tokens = /(%[^\n]*|'(?:[^']|'')*'|\b\d+(?:\.\d+)?\b|\b[A-Za-z_]\w*(?=\())/g;
  let html = '', start = 0;
  for (const match of code.matchAll(tokens)) {
    const token = match[0], index = match.index!;
    html += escapeHtml(code.slice(start, index));
    const kind = token.startsWith('%') ? 'comment' : token.startsWith("'") ? 'string' : /^\d/.test(token) ? 'number' : 'function';
    html += `<span class="code-${kind}">${escapeHtml(token)}</span>`;
    start = index + token.length;
  }
  return html + escapeHtml(code.slice(start));
}
