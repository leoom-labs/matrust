// Smoke test of the browser build in Node: node web/smoke.mjs (after web/build.sh).
// Loads the shipped web/pkg module: first through worker.js (the page's path,
// with fetch and the worker global stubbed), then calls Session directly.
// Prints one line per check and exits nonzero on any miss.
import { readFile } from "node:fs/promises";
import { fileURLToPath } from "node:url";

const pkg = new URL("./pkg/", import.meta.url);
const failures = [];
function check(name, ok, detail = "") {
    console.log(`${ok ? "ok  " : "FAIL"} ${name}${ok ? "" : `: ${detail}`}`);
    if (!ok) failures.push(name);
}
const errors = [];
console.error = (...args) => errors.push(args.join(" "));

// --- worker.js: wasm-bindgen's init() fetches the .wasm next to the module.
const realFetch = globalThis.fetch;
globalThis.fetch = async (url, ...rest) => {
    const href = String(url);
    if (!href.startsWith("file:")) return realFetch(url, ...rest);
    const bytes = await readFile(fileURLToPath(href));
    return new Response(bytes, { headers: { "Content-Type": "application/wasm" } });
};
const replies = new Map();
// As in a real worker, `self` is the global object (js-sys looks up `self` first).
globalThis.self = globalThis;
globalThis.postMessage = (message) => replies.get(message.id)?.(message);
await import(new URL("worker.js", pkg));
let nextId = 1;
function ask(type, code) {
    const id = nextId++;
    return new Promise((resolve) => {
        replies.set(id, resolve);
        self.onmessage({ data: { type, id, code } });
    });
}
const v = await ask("version");
check("worker version", typeof v.result === "string" && /^\d+\.\d+\.\d+/.test(v.result), JSON.stringify(v));
const w1 = await ask("run", "y = 2 + 3");
const r1 = JSON.parse(w1.result);
check("worker run", w1.id === 2 && r1.error === null && /y =\s+5/.test(r1.output), w1.result);
const w2 = await ask("workspace");
check("worker workspace", JSON.parse(w2.result).some((row) => row.name === "y"), w2.result);
const w3 = await ask("reset");
check("worker reset", w3.result === null && w3.error === undefined, JSON.stringify(w3));
const w4 = await ask("workspace");
check("worker workspace after reset", w4.result === "[]", w4.result);
const w5 = await ask("bogus");
check("worker unknown type", w5.result === null && /Unknown message type/.test(w5.error), JSON.stringify(w5));

// --- Session API, on the module instance the worker initialized.
const { Session, version } = await import(new URL("matrust_web.js", pkg));
check("version()", version() === v.result, version());
const s = new Session();
const run = (code) => JSON.parse(s.run(code));
const shape = (r) =>
    typeof r.output === "string" &&
    (r.error === null || typeof r.error === "string") &&
    Array.isArray(r.figures) &&
    typeof r.ms === "number";

let r = run("disp(magic(3))");
check("disp(magic(3))", shape(r) && r.error === null && /8\s+1\s+6\n\s+3\s+5\s+7\n\s+4\s+9\s+2/.test(r.output), JSON.stringify(r));

r = run("x = 1:3");
check("x = 1:3 display", r.error === null && /^x =\n/m.test(r.output) && /1\s+2\s+3/.test(r.output), JSON.stringify(r));

r = run("disp(7); undefined_fn(1)");
check("undefined_fn(1) error", typeof r.error === "string" && r.error.includes("undefined_fn"), JSON.stringify(r));
check("output before the error is kept", r.output.trim() === "7", JSON.stringify(r));

r = run("plot(1:10)");
check("plot(1:10) figure", r.error === null && r.figures.length === 1 && r.figures[0].id === 1 &&
    r.figures[0].svg.startsWith("<svg"), JSON.stringify({ ...r, figures: r.figures.map((f) => [f.id, f.svg.slice(0, 40)]) }));
r = run("plot(1:10)");
check("re-running a plot returns its figure again", r.error === null && r.figures.length === 1 && r.figures[0].id === 1, JSON.stringify(r.figures.map((f) => f.id)));
r = run("z = 1;");
check("open figures stay listed", r.error === null && r.figures.map((f) => f.id).join() === "1", JSON.stringify(r.figures.map((f) => f.id)));
r = run("hold on; plot(2:11); figure(2); bar([1 2 3])");
check("all open figures", r.error === null && r.figures.map((f) => f.id).join() === "1,2", JSON.stringify(r.figures.map((f) => f.id)));

r = run("tic; pause(0.05); t = toc; disp(t >= 0.04); tic; toc");
check("tic; toc and pause", r.error === null && /^\s*1\s*\n/.test(r.output) && /Elapsed time is [\d.]+ seconds/.test(r.output), JSON.stringify(r));

r = run("c = clock; disp(c(1) >= 2026 && c(2) >= 1 && c(2) <= 12); d = now; disp(d > 739000)");
check("clock and now", r.error === null && r.output.split(/\s+/).join("") === "11", JSON.stringify(r));

const noFiles = "Files are not available in the browser version.";
for (const code of ["save('a.mat')", "save a.mat", "load a.mat", "fid = fopen('a.txt')", "[fid, msg] = fopen('a.txt')",
    "readmatrix('a.csv')", "writematrix([1 2], 'a.csv')", "cd ..", "dir", "pwd", "run('a.m')", "savefig('a.fig')",
    "textread('a.txt', '%f')", "[a,b] = textread('a.txt', '%f%f')", "zip('a.zip','a.txt')",
    "unzip('a.zip')", "gzip('a.txt')", "gunzip('a.gz')", "tar('a.tar','a.txt')", "untar('a.tar')",
    "imfinfo('a.png')", "audioinfo('a.wav')", "isdir('a')", "which('sin')",
    "delete(char([97 46 116 120 116]))", 'delete("a.txt")', "delete('a.txt')", "delete a.txt", "tempdir", "what", "ls", "mkdir('a')", "type('a.m')"]) {
    r = run(code);
    check(`files: ${code}`, r.error === `Line 1, column 1: ${noFiles}`, JSON.stringify(r));
}
for (const code of ["exist('a.txt')", "exist('a.txt','file')", "exist('a','dir')",
    "kind='file'; exist('a.txt',kind)", "exist(char([97 46 116 120 116]),char([102 105 108 101]))",
    'exist("a.txt","file")', "exist('sin','file')", "isfile('a.txt')", "isfolder('a')",
    "isfile(char([97]))", 'isfolder("a")']) {
    r = run(code);
    check(`no browser files: ${code}`, r.error === null && /ans =\s+0\s*$/.test(r.output), JSON.stringify(r));
}
r = run("e=exist('a.txt','file'); f=isfile('a.txt'); d=isfolder('a'); disp([e f d]); disp(islogical(f) && islogical(d))");
check("assigned file queries and logical predicates", r.error === null && r.output.split(/\s+/).join(" ").trim() === "0 0 0 1", JSON.stringify(r));
r = run("f=@exist; disp(f('a.txt','file')); disp(builtin('exist','a','dir'))");
check("file queries through handles and builtin", r.error === null && r.output.split(/\s+/).join(" ").trim() === "0 0", JSON.stringify(r));
r = run("x=1; disp(exist('x')); disp(exist('sin')); disp(exist('x','var')); disp(exist('sin','builtin'))");
check("exist on variables and builtins", r.error === null && r.output.split(/\s+/).join(" ").trim() === "1 5 1 5", JSON.stringify(r));

// Kernels that use Rayon or GEMM above their size thresholds run on the one thread.
r = run("x = sort(rand(1, 2e6)); disp(issorted(x)); A = rand(300); B = A * A; disp(size(B))");
check("parallel kernels run serially", r.error === null && r.output.split(/\s+/).join(" ").trim() === "1 300 300", JSON.stringify(r));

r = run("disp(rand(1200, 100))");
check("output capped at 1 MB", r.error === null && r.output.length < 1048576 + 200 && r.output.endsWith("at most 1 MB of text.]"),
    `${r.output.length} chars, error ${r.error}`);

const ws = JSON.parse(s.workspace());
const x = ws.find((row) => row.name === "x");
const c = ws.find((row) => row.name === "c");
check("workspace() rows", Array.isArray(ws) && ws.every((row) => ["name", "class", "size", "value"].every((k) => typeof row[k] === "string")), s.workspace());
check("workspace() matrix row", x && x.class === "double" && x.size === "1x2000000" && x.value === "1x2000000 double", JSON.stringify(x));
check("workspace() vector row", c && c.size === "1x6" && /^\[2\d{3} /.test(c.value), JSON.stringify(c));

s.reset();
check("reset() clears workspace", s.workspace() === "[]", s.workspace());
r = run("x");
check("reset() clears variables", typeof r.error === "string" && r.error.includes("x"), JSON.stringify(r));
r = run("plot(1:3)");
check("reset() clears figures", r.figures.length === 1 && r.figures[0].id === 1, JSON.stringify(r.figures.map((f) => f.id)));

check("no panic or console errors", errors.length === 0, errors.join(" | "));
if (failures.length) {
    console.log(`smoke: ${failures.length} failed: ${failures.join(", ")}`);
    process.exit(1);
}
console.log("smoke: all checks passed");
