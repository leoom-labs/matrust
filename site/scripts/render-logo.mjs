// Extract the Inter SemiBold M outline from the bundled TrueType font.
// The committed SVGs need no font request or fallback font to display correctly.
import { readFile, writeFile } from 'node:fs/promises';
import assert from 'node:assert/strict';
const b = await readFile(new URL('../public/fonts/Inter-SemiBold.ttf',import.meta.url));
const tables = {};
for (let i=0;i<b.readUInt16BE(4);i++) {
  const at=12+i*16;
  tables[b.toString('ascii',at,at+4)] = b.readUInt32BE(at+8);
}
const cmap=tables.cmap;
let format4;
for(let i=0;i<b.readUInt16BE(cmap+2);i++) {
  const at=cmap+4+i*8;
  const offset=cmap+b.readUInt32BE(at+4);
  if(b.readUInt16BE(offset)===4) format4=offset;
}
assert.ok(format4,'Font needs a format-4 cmap');
const c=format4, segments=b.readUInt16BE(c+6)/2, ends=c+14, starts=ends+segments*2+2, deltas=starts+segments*2, ranges=deltas+segments*2;
let glyph;
for(let i=0;i<segments;i++) {
  if(77<b.readUInt16BE(starts+i*2)||77>b.readUInt16BE(ends+i*2)) continue;
  const range=b.readUInt16BE(ranges+i*2), delta=b.readInt16BE(deltas+i*2);
  glyph=range ? b.readUInt16BE(ranges+i*2+range+2*(77-b.readUInt16BE(starts+i*2))) : 77;
  glyph=(glyph+delta)&65535; break;
}
assert.ok(glyph,'Missing M glyph');
const units=b.readUInt16BE(tables.head+18), long=b.readInt16BE(tables.head+50)===1;
const glyphOffset=tables.glyf+(long?b.readUInt32BE(tables.loca+glyph*4):b.readUInt16BE(tables.loca+glyph*2)*2);
const contours=b.readInt16BE(glyphOffset);
assert.ok(contours>0,'Expected simple M outline');
const endpoints=Array.from({length:contours},(_,i)=>b.readUInt16BE(glyphOffset+10+i*2));
const points=endpoints.at(-1)+1;
let at=glyphOffset+10+contours*2;
const instructions=b.readUInt16BE(at); at+=2+instructions;
const flags=[];
while(flags.length<points) { const flag=b[at++]; flags.push(flag); if(flag&8){ const repeat=b[at++]; for(let j=0;j<repeat;j++)flags.push(flag); } }
const coordinates=[];
for(const axis of [0,1]) {
  let value=0;
  const values=[];
  for(const flag of flags) {
    const short=flag&(axis?4:2), same=flag&(axis?32:16);
    if(short) { const delta=b[at++];value+=same?delta:-delta; }
    else if(!same) {value+=b.readInt16BE(at);at+=2;}
    values.push(value);
  }
  coordinates.push(values);
}
const metrics=b.readUInt16BE(tables.hhea+34);
const advance=b.readUInt16BE(tables.hmtx+Math.min(glyph,metrics-1)*4);
let first=0,path='';
for(const end of endpoints) {
  const contour=[];
  for(let i=first;i<=end;i++) {
    const next=i===end?first:i+1;
    contour.push({x:coordinates[0][i],y:coordinates[1][i],on:!!(flags[i]&1)});
    if(!(flags[i]&1) && !(flags[next]&1)) contour.push({x:(coordinates[0][i]+coordinates[0][next])/2,y:(coordinates[1][i]+coordinates[1][next])/2,on:true});
  }
  const start=contour.findIndex(point=>point.on);
  assert.ok(start>=0);
  const ordered=[...contour.slice(start),...contour.slice(0,start)];
  path+=`M${ordered[0].x} ${ordered[0].y}`;
  for(let i=1;i<=ordered.length;i++) {
    const point=ordered[i%ordered.length];
    if(point.on) path+=`L${point.x} ${point.y}`;
    else { const next=ordered[(i+1)%ordered.length];path+=`Q${point.x} ${point.y} ${next.x} ${next.y}`;i++; }
  }
  path+='Z';first=end+1;
}
const scale=64/units, x=50-advance*scale/2;
const outline=`<path d="${path}" transform="translate(${x} 74) scale(${scale} ${-scale})" fill="#fff"/>`;
await writeFile(new URL('../public/favicon.svg',import.meta.url),`<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 100 100"><rect width="100" height="100" rx="22" fill="#C2410C"/>${outline}</svg>\n`);
await writeFile(new URL('../src/components/Logo.astro',import.meta.url),`<svg viewBox="0 0 100 100" width="32" height="32" aria-hidden="true"><rect width="100" height="100" rx="22" fill="var(--accent-fill)"/>${outline}</svg>\n`);
console.log('Exported bundled Inter SemiBold M to Logo.astro and favicon.svg.');
