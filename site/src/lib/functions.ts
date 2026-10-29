import { existsSync, readFileSync } from 'node:fs';
import { resolve } from 'node:path';
import { marked } from 'marked';
import { escapeHtml } from './highlight.ts';
export interface FunctionHelp { name: string; summary: string; text: string; html: string; example: string | null; }
const codeBlock = (code: string) => `<pre><code>${escapeHtml(code)}</code></pre>`;
export function splitFunctions(source: string): FunctionHelp[] {
  const headings = [...source.matchAll(/^## ([^\r\n]+)\r?$/gm)];
  const entries = headings.map((match, i) => {
    const name = match[1].trim();
    if (!/^[A-Za-z][\w]*(?:\.[A-Za-z][\w]*)*$/.test(name)) throw new Error(`Invalid help heading: ${name}`);
    const text = source.slice(match.index! + match[0].length, headings[i+1]?.index ?? source.length).trim();
    const lines = text.split(/\r?\n/);
    const summary = lines.find(line => line.trim()) ?? '';
    const example = lines.find(line => /^Example:\s*/.test(line))?.replace(/^Example:\s*/, '').trim() || null;
    // Help is line-oriented plain text, not authored Markdown. Each line becomes a
    // paragraph with every ASCII punctuation mark backslash-escaped, so marked shows
    // it verbatim and does the HTML escaping itself (escaping HTML first would
    // double-escape entities such as &#39;). Indented lines are a code listing.
    const blocks: string[] = [];
    for (let i = 0; i < lines.length; i++) {
      if (!lines[i].trim()) continue;
      if (/^\s/.test(lines[i])) {
        const listing = [lines[i]];
        while (/^\s+\S/.test(lines[i+1] ?? '')) listing.push(lines[++i]);
        const indent = Math.min(...listing.map(line => /^\s*/.exec(line)![0].length));
        blocks.push(codeBlock(listing.map(line => line.slice(indent).trimEnd()).join('\n')));
        continue;
      }
      const field = /^(Syntax|Example):\s*(.*)$/.exec(lines[i]);
      blocks.push(field ? `**${field[1]}**\n\n${codeBlock(field[2])}` : lines[i].trim().replace(/[!-\/:-@\[-`{-~]/g, '\\$&'));
    }
    return { name, summary, text, html: marked.parse(blocks.join('\n\n'), { async: false }) as string, example };
  });
  const grouped = new Map<string, FunctionHelp[]>();
  for (const entry of entries) grouped.set(entry.name, [...(grouped.get(entry.name) ?? []), entry]);
  return [...grouped.values()].map(group => {
    const latest = group[group.length - 1];
    if (group.length === 1) return latest;
    const notice = '<p class="muted">The source help has multiple entries for this name. They are preserved below in source order, including differing compatibility notes.</p>';
    return { ...latest, text: group.map(entry => entry.text).join('\n\n'), example: group.find(entry => entry.example)?.example ?? null,
      html: notice + group.map((entry, index) => `<section><h2>Source entry ${index+1}</h2>${entry.html}</section>`).join('') };
  }).sort((a,b) => a.name.localeCompare(b.name));
}
// Resolve from the invoked project root, not a generated server chunk's import.meta.url.
// Both the documented site/ commands and root-level source proofs are supported.
const rootHelp = resolve(process.cwd(), 'assets/help/functions.md');
const helpPath = existsSync(rootHelp) ? rootHelp : resolve(process.cwd(), '../assets/help/functions.md');
export const functions = splitFunctions(readFileSync(helpPath, 'utf8'));
