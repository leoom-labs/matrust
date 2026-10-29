# MatRust in the browser

The engine compiled to WebAssembly: a MATLAB console with text output and SVG
figures, for the website's "Try online". The page runs it in a Web Worker.

## Build

Needs the Rust target `wasm32-unknown-unknown` (`rustup target add wasm32-unknown-unknown`)
and Node for the smoke test.

```
bash web/build.sh      # release build -> web/pkg, prints raw and gzipped .wasm size
node web/smoke.mjs     # checks web/pkg in Node; exits nonzero on a miss
node web/probe.mjs "x = 1:3" "plot(x)"   # runs snippets, prints a summary of each result
node web/bench.mjs     # median run time of a few kernels (optional pkg dir to compare builds)
```

`build.sh` installs `wasm-bindgen-cli` at the version pinned in `web/Cargo.toml`
if the one on PATH differs, and runs `wasm-opt -Oz` only when it is on PATH.
`web/pkg/` holds `matrust_web.js`, `matrust_web_bg.wasm`, `worker.js` and
TypeScript declarations. Serve these files together; `.wasm` as `application/wasm`.

`web/` is its own crate (`matrust-web`) with its own `Cargo.lock` and
`web/target`. It uses MatRust without default features, so MAT-file v7.3
(HDF5, a C library) is left out.

## API

`matrust_web.js` (wasm-bindgen `--target web`):

```
init(): Promise<void>               // default export; loads the .wasm
new Session()                       // fresh engine
session.run(code: string): string   // JSON {"output", "error", "figures", "ms"}
session.reset(): void               // clear workspace, figures and state
session.workspace(): string         // JSON [{"name", "class", "size", "value"}]
version(): string                   // MatRust version
```

- `output`: the Command Window text of the run, in order, including `ans = ...`.
  When a statement fails, the output of the statements before it is kept.
- `error`: the error message, or `null`.
- `figures`: `[{"id", "svg"}]`, every open figure after the run (figures stay open
  across runs, as in MATLAB), as light-theme SVG, sorted by id.
- `ms`: run time in milliseconds.
- `workspace()`: rows sorted by name; `size` is `"3x3"`, `value` is the desktop
  Workspace VALUE text (a scalar, a short vector, or `"3x3 double"`).

`worker.js` (`new Worker(url, { type: "module" })`) answers
`{type: "run" | "reset" | "workspace" | "version", id, code?}` with `{id, result}`.
On a failure it adds `error` (a failed run still gets its JSON result). A run
cannot be interrupted: to stop it, terminate the worker and start a new one.

## Limits

- The engine's own per-run limits apply (steps, output size and lines, figures).
  The returned text is also capped at 1 MB with a note at the end.
- No file system: `save`, `load`, `fopen`, `readmatrix`, `writematrix`, `cd`,
  `dir`, `pwd`, `exist(name, 'file')`, `run`, figure export and the other file
  commands fail with "Files are not available in the browser version."
- One thread: kernels that use several cores on the desktop run serially.
- `clock`, `now` and `date` use the browser's local time; `cputime` counts
  wall time since its first use; `pause` waits by polling the clock.
- A Rust panic aborts the call; `worker.js` rethrows it, so the page's `worker.onerror` ends the worker and the next run starts a new one.
