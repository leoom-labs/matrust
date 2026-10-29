# Privacy

MatRust sends **nothing by itself, ever**. There is no telemetry, no usage counting, no crash
reporting and no update check. The desktop app, the `matrust` command and the engine contain no
network code at all.

## The one way out: `matrust bench --share`

Benchmark results can be shared, and only when you start it:

1. You run `matrust bench --share FILE.json` on a result file of your choice.
2. MatRust prints the exact text, saves it next to that file as `FILE.json.share.md`, and asks
   "Open the browser? [y/N]".
3. On yes, it opens the empty "Benchmark result" issue form of the MatRust repository on GitHub in
   your own browser. The link holds no data.
4. You paste the text, change or cut anything you like, and press Submit. The issue is **public**,
   under **your** GitHub account. GitHub's own privacy terms apply to what you submit.

Until you press Submit nothing has left your computer. MatRust itself makes no request; your
browser does, when you open the form.

## What the text holds

A table for reading, then the same data as JSON. Every field:

| Field | Value |
|---|---|
| `format` | `1`, the layout of this text |
| `matrust_version` | MatRust's version, `0.1.0` |
| `commit` | the git commit the program was built from, or `null` |
| `date` | the day of the run (UTC), `2026-10-26`; no time of day |
| `threads` | the worker threads used, `4` |
| `machine.os`, `machine.os_version` | `Ubuntu` and `24.04`, `macOS` and `14.5`, `Microsoft Windows 11 Pro` and `10.0.22631` |
| `machine.arch` | `x86_64` or `aarch64` |
| `machine.cpu` | the CPU model, `12th Gen Intel(R) Core(TM) i7-12700KF` |
| `machine.logical_cores` | `20` |
| `machine.ram_gib` | total memory, rounded to whole GiB, `63` |
| `cases[].id` | a built-in case's id (`fft_2p20`); your own cases become `user-1`, `user-2`, ... in file order |
| `cases[].source` | `builtin` only when both the id and the case text equal a built-in case; else `user` |
| `cases[].sha256` | the SHA-256 of the case text (setup, op and check) or of the script file. Equal cases give equal hashes. It cannot be turned back into the text, but someone who guesses your exact text can confirm the guess |
| `cases[].runs` | the number of timed runs |
| `cases[].median_ms`, `min_ms`, `max_ms` | the timings in milliseconds, to 1 µs |
| `cases[].error_kind` | `none`, `unsupported`, `error` or `timeout`; never the message |

A real example, from `matrust bench --cases mix.csv --threads 4` with two built-in cases and one of
the user's own:

```json
{
  "format": 1,
  "matrust_version": "0.1.0",
  "commit": null,
  "date": "2026-10-26",
  "threads": 4,
  "machine": {
    "os": "Ubuntu",
    "os_version": "24.04",
    "arch": "x86_64",
    "cpu": "12th Gen Intel(R) Core(TM) i7-12700KF",
    "logical_cores": 20,
    "ram_gib": 63
  },
  "cases": [
    {
      "id": "sum_1e6",
      "source": "builtin",
      "sha256": "c9a8d404e4f3a223950726d28e1bc6f6d1466c4f3c93ed22d84740452e48ec8c",
      "runs": 200,
      "median_ms": 0.457,
      "min_ms": 0.436,
      "max_ms": 1.14,
      "error_kind": "none"
    },
    {
      "id": "fft_2p20",
      "source": "builtin",
      "sha256": "042d71ab067a351c44cf03bd33ae9b9f5b506debb4d6c02cbd3252398af3dc7e",
      "runs": 21,
      "median_ms": 45.511,
      "min_ms": 39.74,
      "max_ms": 57.778,
      "error_kind": "none"
    },
    {
      "id": "user-1",
      "source": "user",
      "sha256": "bd5daa66a903003d244a3a5db7d4e94748882460a3b36f39464ae81e56750c4a",
      "runs": 200,
      "median_ms": 0.064,
      "min_ms": 0.061,
      "max_ms": 0.073,
      "error_kind": "none"
    }
  ]
}
```

## Never sent

- Anything about MATLAB: its timings, its results, its version, or whether you ran it. No option
  adds them ([why](../BENCHMARKS.md#timings-from-other-programs)).
- Your case ids, your setup, op and check text, and the contents of your scripts.
- Result checksums.
- Error messages.
- File and folder names, paths, your user name, the computer's name, IP addresses and environment
  variables. (GitHub sees your connection when you submit, as for any page.)
- The time of day of the run.

## Check it yourself

- `matrust bench --share FILE.json --no-open` prints the text and the form's address, and starts
  no other program.
- What you paste is `FILE.json.share.md`, word for word. Read it first.
- The text is built in `src/bench/share.rs` from an explicit list of fields. Its tests fail when a
  field is added, or when a private case id, a folder name or the word MATLAB reaches the text
  (`src/bench/share.rs`, `tests/bench_cli.rs`).
