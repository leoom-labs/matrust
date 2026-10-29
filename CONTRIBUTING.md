# Contributing to MatRust

MatRust is an alpha project. Bug reports, missing-function reports and pull requests are welcome.

## Report a bug

Open an issue on GitHub.

Include:

- The MatRust version (`matrust --version`).
- Your operating system.
- The smallest script that shows the problem.
- What you expected, and how you checked it (a hand calculation, a published formula, or another program whose licence lets you publish the output).

## Build and test

```
cargo fmt --all -- --check
cargo clippy --all-targets --features desktop -- -D warnings
cargo test --release --features desktop
```

The full test suite is long. While you work, run only the test files you touched:

```
cargo test --release --features desktop --test <name>
```

On Windows, `scripts\build.ps1 -Desktop -Check` runs formatting, tests, Clippy and the release build.
Some desktop tests need a real display and fail on headless Linux.

## Code rules

These rules come from `AGENTS.md` and `clippy.toml`.

- The numerical engine (`src/engine`) must not depend on the desktop UI.
- Matrix storage is immutable, shared and column-major. Do not copy a whole matrix on assignment or when listing the workspace.
- Never invert a matrix to solve a linear system. Use a solver.
- Use bounded parallelism. Parallelize only above a size threshold you measured, and avoid nested thread pools.
- Bound dimensions, expression size, workspace size and output. Keep long work off the UI thread and keep cooperative cancellation.
- Reject unsupported syntax and arguments with a clear error.
- `f64::mul_add` is banned by `clippy.toml`. Use the `Fma` trait in `src/engine.rs` (`libm::fma`). The C runtime `fma` is wrong in the last bit on some Windows builds.
- Use the `libm` crate for trigonometric functions where platform results differ. Windows `f64::sin` and `f64::cos` are inaccurate near pi/2.
- Keep each file's line endings. Many files use CRLF. Check `git diff --stat` after a scripted edit: a converted file shows as a whole-file change.

## Add a function

A new function needs code, a help page and tests.

1. Add the name to the `NAMES` list of the module in `src/engine/` that fits it (for example `src/engine/matrix_build.rs`), and handle it in that module's dispatch `match`. Add the name to `FUNCTIONS` in `src/session.rs`.
2. Add a `## name` page to `assets/help/functions.md`. The format is described at the top of that file. State the differences from MATLAB.
3. Add tests under `tests/`.
4. Put expected results under `tests/fixtures/` and say where they come from (a hand calculation, a published formula, or another program whose licence lets you publish the output).

## Pull requests

Before you open one, rebase on the current `main` branch and run the commands in Build and test.

- One topic per pull request.
- Include tests.
- `cargo fmt` and Clippy must be clean.

## Licence

MatRust is under the GNU GPL, version 3 or later (`LICENSE`). By contributing you agree that your work is licensed under the same terms.
