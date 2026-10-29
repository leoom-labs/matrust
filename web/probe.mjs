// Run snippets against web/pkg in Node and print a summary of each result:
// node web/probe.mjs "x = 1:3" "plot(x)"
import { readFileSync } from "node:fs";
const pkg = new URL("./pkg/", import.meta.url);
const { initSync, Session } = await import(new URL("matrust_web.js", pkg));
initSync({ module: readFileSync(new URL("matrust_web_bg.wasm", pkg)) });
const s = new Session();
for (const code of process.argv.slice(2)) {
    try {
        const r = JSON.parse(s.run(code));
        const tail = r.output.slice(-120);
        console.log(JSON.stringify({ chars: r.output.length, error: r.error, figures: r.figures.length, ms: r.ms, head: r.output.slice(0, 300), tail }));
    } catch (error) {
        console.log(`THROWN: ${error}`);
    }
}
