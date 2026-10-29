# Benchmarks

`matrust bench` times MatRust on a built-in suite, on your own cases or scripts, or on one expression.

## Method

Every case has three parts: `setup` (untimed), `op` (timed) and `check` (one number that sums up the result).

- Result check: in a fresh workspace, run setup, op once, then check. That number is the checksum.
- Timing: in another fresh workspace, run setup once (untimed), op twice as warm-up, then timed runs of op. At least 5 timed runs; stop once they add up to 1 s; never more than 200.
- A run is one evaluation of op as text, so parsing and dispatch are included.
- The table shows the median of the timed runs and their range (fastest to slowest).
- Timings are warm: the warm-ups absorb first-call costs. Start-up time is not measured.
- Threads: `--threads N` sets MatRust's worker threads (1 to 64). The default is all logical cores, at most 64.
- A difference smaller than the range is not a result.
- Each case is one operation at one size. A result is not a general speed claim. Close other programs while it runs; a busy computer widens the ranges.

## Run it

The built-in suite (about 20 cases, under a minute):

```
matrust bench
matrust bench --threads 1
matrust bench --only sum_1e6,fft_2p20
```

Your own cases, in a CSV file with the same format as [bench/cases.csv](bench/cases.csv):

```
matrust bench --cases mycases.csv
```

- First line: `id,setup,op,check`. Then one case per line.
- `id`: letters, digits, `_` or `-`.
- `op` must end with `;`, so that the timed runs do not print.
- `check` gives one real number.
- A field that contains a comma goes in double quotes; `""` inside quotes is one quote.

A script (repeatable; each script is one case, `script-1`, `script-2`, ...):

```
matrust bench --script analysis.m
```

The exact script file runs with its folder on the function path, so functions in that folder are found. A neighboring `.mr` file does not replace the selected `.m` script. Its name must be a MATLAB name (a letter, then letters, digits or `_`). A script has no check.

One expression or function call:

```
matrust bench --op "y = myfilter(x);" --setup "x = sin((1:1e5)'*0.01);" --check "sum(y)" --path ./lib
```

`--path` names the folder with your functions. `--setup` and `--check` are optional.

Other options:

- `--out FILE.json`: the result file. Default: `matrust-bench-<YYYYMMDD-HHMMSS>-<pid>.json` (UTC) in the current folder.
- `matrust bench --help` lists every option.
- Exit codes: 0 ok, 1 some case failed, 2 a usage error.

## Reading the table

| Column | Meaning |
|---|---|
| case | the case id |
| MatRust ms, range | median time of op in milliseconds, then fastest-slowest |

A failed case shows `unsupported`, `error` or `timeout` instead of a time. The messages are listed under the table.

## The result file

The JSON file holds the MatRust version, the git commit when the build has one, the UTC time, the thread count, the machine (OS, architecture, CPU model, logical cores, RAM in GiB), and per case: id, source (`builtin` or `user`), SHA-256, runs, median, min and max in ms, checksum and the error message.

## Timings from other programs

MatRust does not publish or collect timings from MATLAB. Do not include them in a shared result.

## Sharing results

```
matrust bench --share matrust-bench-20261026-140312-4242.json
```

- It prints the exact text: a table and the same data as JSON.
- It saves that text next to the result file as `<file>.share.md`.
- It asks "Open the browser? [y/N]". Yes opens the empty "Benchmark result" issue form on GitHub. You paste the text and press Submit. The issue is public, under your GitHub account.
- MatRust sends nothing itself. `--no-open` prints the form's address instead of asking.

What the text holds and what it never holds: [docs/privacy.md](docs/privacy.md).

## The built-in suite

| case | measures |
|---|---|
| sum_1e6 | sum of 1e6 values |
| mtimes_500 | 500x500 matrix multiply |
| fft_2p20 | FFT of 2^20 values |
| exp_sin_1e6 | element-wise `exp`, `sin`, `sqrt` on 1e6 values |
| std_cols_1000 | `std` of each column of a 1000x1000 matrix |
| mldivide_500 | `A\b` with a 500x500 matrix |
| sort_1e6 | sort of 1e6 values |
| cumsum_diff_1e6 | `diff(cumsum(x))` on 1e6 values |
| for_loop_2e5 | a scalar `for` loop, 2e5 steps (interpreter speed) |
| anon_calls_1e5 | 1e5 calls of an anonymous function in a loop |
| cellfun_5e4 | `cellfun` with an anonymous function over 5e4 cells |
| struct_field_1e5 | struct field reads and writes in a loop |
| logical_index_1e6 | `x(x > 0.5)` on 1e6 values |
| filter_1e6 | IIR `filter` over 1e6 samples |
| conv_1e5 | `conv` of 1e5 and 200 samples |
| interp1_1e6 | linear `interp1` at 1e6 points |
| bsxfun_1000 | `bsxfun(@minus, ...)` on a 1000x1000 matrix |
| sprintf_5000 | `sprintf` of 5000 numbers |
| num2str_1e4 | 1e4 calls of `num2str` |
| strsplit_9000 | `strsplit` of a 51000-character text |
| regexprep_9000x20 | 20 `regexprep` calls on a 51000-character text |

The data is computed, not random, so every run times the same values.
