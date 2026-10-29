// An owned development preview for targeted browser probes, not the acceptance gate.
process.env.ASTRO_TELEMETRY_DISABLED = '1';
const { dev } = await import('astro');
import { mkdir, writeFile } from 'node:fs/promises';
await mkdir(new URL('../test-results/',import.meta.url),{recursive:true});
await writeFile(new URL('../test-results/preview.pid',import.meta.url),String(process.pid));
const server = await dev({ root:new URL('../',import.meta.url), server:{host:'127.0.0.1',port:4322} });
console.log(`Owned preview PID ${process.pid}`);
for (const signal of ['SIGINT','SIGTERM']) process.on(signal,async()=>{ await server.stop();process.exit(0); });
