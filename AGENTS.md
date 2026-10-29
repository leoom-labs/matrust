# MatRust development requirements

The user requires fast, stable, lightweight implementations and multicore execution when it helps.

- Keep the numerical engine independent of the optional desktop UI. Do not add browser runtimes.
- Preserve immutable shared column-major storage. Avoid full matrix copies on assignment or workspace inspection.
- Prefer proven numerical kernels over handwritten scalar implementations of dense linear algebra. Never substitute explicit inversion for solving a linear system.
- Use bounded parallelism and measured size thresholds; do not parallelize small operations by default. Avoid nested thread pools and lock contention in kernels.
- Benchmark changed numerical paths in release mode, with both one worker and multiple workers. Record hardware, sizes, repetitions and limitations. Do not claim universal or maximal speed from a narrow benchmark.
- Bound dimensions, expression complexity, workspace size and output. Reject unsupported syntax explicitly. Keep long computation off the UI thread and retain cooperative cancellation.
- Add meaningful correctness tests for new mathematical behavior; derive expected values from published references, mpmath or existing fixtures. Preserve the origin and limitations of REA evidence.
- Run `scripts/build.ps1 -Desktop -Check` on Windows for formatting, release tests, Clippy and the desktop release build. The script selects project-local 64-bit tools before unrelated system tools.
- Application source lives in `src/`; `rea-review/` is an unmodified upstream inspection checkout. `tools/` contains development tools, not application runtime dependencies.
