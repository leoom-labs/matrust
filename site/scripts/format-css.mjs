// Dependency-free formatter for this site's plain CSS; preserves quoted values.
import { readFile, writeFile } from 'node:fs/promises';
const path=new URL('../src/styles/global.css',import.meta.url);
const source=await readFile(path,'utf8');
const lines=[];
let token='',depth=0,quote='',parentheses=0,escape=false;
function line(value) { if(value.trim()) lines.push('  '.repeat(depth)+value.trim()); }
for(const char of source) {
  if(quote) {
    token+=char;
    if(escape) escape=false;
    else if(char==='\\') escape=true;
    else if(char===quote) quote='';
    continue;
  }
  if(char==='"'||char==="'") {quote=char;token+=char;continue;}
  if(char==='(') parentheses++;
  if(char===')') parentheses--;
  if(!parentheses && char==='{') {line(token+' {');token='';depth++;}
  else if(!parentheses && char==='}') {line(token);token='';depth--;line('}');if(!depth)lines.push('');}
  else if(!parentheses && char===';') {line(token+';');token='';}
  else token+=char;
}
line(token);
await writeFile(path,lines.join('\n').trim()+'\n');
console.log('Formatted src/styles/global.css.');
