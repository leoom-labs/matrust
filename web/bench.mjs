// Median of 5 run times for a few kernels: node web/bench.mjs [pkg-dir]
// (default web/pkg). Compares wasm builds, e.g. opt-level "z" against "s".
import { readFileSync } from "node:fs";
import { pathToFileURL } from "node:url";
const pkg = process.argv[2] ? pathToFileURL(process.argv[2] + "/") : new URL("./pkg/", import.meta.url);
const { initSync, Session } = await import(new URL("matrust_web.js", pkg));
initSync({ module: readFileSync(new URL("matrust_web_bg.wasm", pkg)) });
const s = new Session();
const cases = [
    "A = rand(400); B = A * A;",
    "x = sort(rand(1, 2e6));",
    "y = fft(rand(1, 2^20));",
    "s = 0; for i = 1:300000, s = s + i; end",
    "plot(rand(1, 1e5));",
];
for (const code of cases) {
    const ms = [];
    for (let i = 0; i < 5; i++) ms.push(JSON.parse(s.run(code)).ms);
    ms.sort((a, b) => a - b);
    console.log(`${ms[2].toFixed(1).padStart(8)} ms  ${code}`);
}
