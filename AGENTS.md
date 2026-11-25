# MatRust worker rules

- Fast, stable, lightweight; multicore only when it pays (measured size thresholds, no nested pools).
- Engine independent of the optional desktop UI; no browser runtimes.
- Keep immutable shared column-major storage; no full copies on assignment. Use proven kernels; never invert to solve.
- Bound dimensions, expression size, workspace and output; reject unsupported syntax explicitly; long work off the UI thread, cancellable.
- Benchmark changed numerical paths in release mode (1 and N workers); record hardware and limits.
- New math needs correctness tests with expected values from published refs, mpmath, SciPy/NumPy, Octave or fixtures (`tool/ref-python.sh`; keep the generating script in `tool/refs/<area>/`). Never MATLAB.
- Parallel workstreams: new helpers as free functions or module-prefixed, not new generic `impl Engine` methods. "Not supported yet" tests use far-future names (`uifigure`, `uicontrol`, `syms`).
- New test file: add to a suite via `python3 tool/suites-assign.py --write`.
- New builtin: add to `src/session.rs` FUNCTIONS; every help example must run (tests/help_system.rs).
- Source in `src/`; `rea-review/` is upstream, unmodified.
