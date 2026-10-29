//! `matrust bench [options]`. Exit codes: 0 ok, 1 some case failed, 2 usage.
use super::share::{NOTICE, SharePayload, issue_url, opener};
use super::{
    Case, CaseResult, ErrorKind, ResultFile, Script, builtin_cases, case_sha256, format_ms,
    machine, parse_cases, read_bounded, read_bounded_text, run_case, select, sha256_hex, source_of,
    utc_parts, write_atomically,
};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::{Duration, Instant, SystemTime};

pub const USAGE: &str = "\
matrust bench [options]

Times MatRust on the built-in suite, your cases, a script or one expression.
Results go to a local JSON file.

What to time (pick one; default: the built-in suite):
  --cases FILE.csv        your cases, same format as bench/cases.csv (id,setup,op,check)
  --script FILE.m         time running a script; repeatable, one case each
  --op \"expr;\"            time one expression or function call
    --setup \"expr\"        run once before timing (untimed)
    --check \"expr\"        one number saved as the result's checksum
    --path DIR            folder with your functions

Options:
  --threads N|all         worker threads, 1-64 (default: all cores, at most 64)
  --only id,id            run only these cases (built-in suite or --cases)
  --out FILE.json         result file (default: matrust-bench-<date>-<time>-<pid>.json)

Share MatRust's numbers:
  --share FILE.json [--no-open]   print the public text, then offer to open the GitHub form

See BENCHMARKS.md and docs/privacy.md.";

#[derive(Debug, PartialEq)]
enum Mode {
    Builtin,
    Cases(PathBuf),
    Scripts(Vec<PathBuf>),
    Op {
        op: String,
        setup: String,
        check: String,
        path: Option<PathBuf>,
    },
    Share {
        file: PathBuf,
        open: bool,
    },
}

#[derive(Debug, PartialEq)]
struct Options {
    mode: Mode,
    threads: usize,
    only: Option<Vec<String>>,
    out: Option<PathBuf>,
}

/// Exit 2 for usage and input files, 1 for failures while running.
enum Failure {
    Usage(String),
    Run(String),
}

fn usage(message: impl Into<String>) -> Failure {
    Failure::Usage(message.into())
}

pub fn default_threads() -> usize {
    std::thread::available_parallelism()
        .map_or(1, usize::from)
        .min(64)
}

fn parse_threads(text: &str) -> Result<usize, String> {
    if text == "all" {
        return Ok(default_threads());
    }
    match text.parse::<usize>() {
        Ok(count) if (1..=64).contains(&count) => Ok(count),
        _ => Err("--threads must be a number from 1 to 64, or all".into()),
    }
}

/// `None` for `--help`.
fn parse_args(args: &[String]) -> Result<Option<Options>, String> {
    let mut seen: Vec<&str> = Vec::new();
    let (mut cases, mut scripts, mut op, mut setup, mut check, mut path) =
        (None, Vec::new(), None, None, None, None);
    let (mut threads, mut only, mut out, mut share) = (None, None, None, None);
    let mut no_open = false;
    let mut args = args.iter();
    while let Some(arg) = args.next() {
        let flag = arg.as_str();
        if seen.contains(&flag) && flag != "--script" {
            return Err(format!("{flag} is given twice"));
        }
        seen.push(flag);
        let mut value = || {
            args.next()
                .cloned()
                .ok_or_else(|| format!("{flag} requires a value"))
        };
        match flag {
            "--help" | "-h" => return Ok(None),
            "--cases" => cases = Some(PathBuf::from(value()?)),
            "--script" => scripts.push(PathBuf::from(value()?)),
            "--op" => op = Some(value()?),
            "--setup" => setup = Some(value()?),
            "--check" => check = Some(value()?),
            "--path" => path = Some(PathBuf::from(value()?)),
            "--threads" => threads = Some(parse_threads(&value()?)?),
            "--only" => {
                let ids: Vec<String> = value()?
                    .split(',')
                    .map(|id| id.trim().to_string())
                    .filter(|id| !id.is_empty())
                    .collect();
                if ids.is_empty() {
                    return Err("--only requires at least one case id".into());
                }
                only = Some(ids);
            }
            "--out" => out = Some(PathBuf::from(value()?)),
            "--share" => share = Some(PathBuf::from(value()?)),
            "--no-open" => no_open = true,
            _ => return Err(format!("Unknown option {arg}")),
        }
    }
    let mut options = Options {
        mode: Mode::Builtin,
        threads: threads.unwrap_or_else(default_threads),
        only,
        out,
    };
    if let Some(file) = share {
        if seen
            .iter()
            .any(|flag| !matches!(*flag, "--share" | "--no-open"))
        {
            return Err("--share stands alone; only --no-open may go with it".into());
        }
        options.mode = Mode::Share {
            file,
            open: !no_open,
        };
        return Ok(Some(options));
    }
    if no_open {
        return Err("--no-open goes with --share".into());
    }
    let modes = [cases.is_some(), !scripts.is_empty(), op.is_some()];
    if modes.iter().filter(|given| **given).count() > 1 {
        return Err("Give only one of --cases, --script and --op".into());
    }
    if op.is_none() {
        for (flag, given) in [
            ("--setup", setup.is_some()),
            ("--check", check.is_some()),
            ("--path", path.is_some()),
        ] {
            if given {
                return Err(format!("{flag} goes with --op"));
            }
        }
    }
    if options.only.is_some() && (op.is_some() || !scripts.is_empty()) {
        return Err("--only goes with the built-in suite or --cases".into());
    }
    options.mode = if let Some(file) = cases {
        Mode::Cases(file)
    } else if !scripts.is_empty() {
        Mode::Scripts(scripts)
    } else if let Some(op) = op {
        Mode::Op {
            op,
            setup: setup.unwrap_or_default(),
            check: check.unwrap_or_default(),
            path,
        }
    } else {
        Mode::Builtin
    };
    Ok(Some(options))
}

/// `--script FILE.m`: execute the exact document, with its folder on the path.
fn script_case(index: usize, path: &Path) -> Result<Case, String> {
    let shown = path.display();
    if path.extension().and_then(|e| e.to_str()) != Some("m") {
        return Err(format!("{shown}: --script needs a .m file"));
    }
    let name = path
        .file_stem()
        .and_then(|s| s.to_str())
        .unwrap_or_default();
    let mut chars = name.chars();
    let identifier = chars.next().is_some_and(|c| c.is_ascii_alphabetic())
        && chars.all(|c| c.is_ascii_alphanumeric() || c == '_')
        && name.len() <= 63;
    if !identifier {
        return Err(format!(
            "{shown}: a script name must be a MATLAB name (a letter, then letters, digits or _)"
        ));
    }
    let bytes = read_bounded(path)?;
    let sha256 = sha256_hex(&bytes);
    let source = String::from_utf8(bytes).map_err(|_| format!("{shown}: not UTF-8 text"))?;
    let absolute = std::path::absolute(path).map_err(|e| format!("{shown}: {e}"))?;
    Ok(Case {
        id: format!("script-{index}"),
        setup: String::new(),
        op: String::new(),
        check: String::new(),
        directory: absolute.parent().map(Path::to_path_buf),
        script: Some(Script {
            path: absolute,
            source: source.into(),
        }),
        sha256,
    })
}

/// `--op`: the text exactly as given.
fn op_case(op: &str, setup: &str, check: &str, path: Option<&Path>) -> Result<Case, String> {
    if [op, setup, check]
        .iter()
        .any(|text| text.contains(['\r', '\n']))
    {
        return Err("--op, --setup and --check must be one line each".into());
    }
    if !op.trim_end().ends_with(';') {
        return Err("--op must end with ';' (otherwise every run prints its result)".into());
    }
    let directory = match path {
        Some(path) => {
            let absolute = std::path::absolute(path).map_err(|e| e.to_string())?;
            if !absolute.is_dir() {
                return Err(format!("--path {}: not a folder", path.display()));
            }
            Some(absolute)
        }
        None => None,
    };
    Ok(Case {
        id: "op".into(),
        setup: setup.into(),
        op: op.into(),
        check: check.into(),
        directory,
        script: None,
        sha256: case_sha256(setup, op, check),
    })
}

fn load_cases(options: &Options) -> Result<Vec<Case>, String> {
    let cases = match &options.mode {
        Mode::Builtin => builtin_cases(),
        Mode::Cases(path) => parse_cases(&read_bounded_text(path)?)
            .map_err(|e| format!("{}: {e}", path.display()))?,
        Mode::Scripts(paths) => paths
            .iter()
            .enumerate()
            .map(|(index, path)| script_case(index + 1, path))
            .collect::<Result<_, _>>()?,
        Mode::Op {
            op,
            setup,
            check,
            path,
        } => vec![op_case(op, setup, check, path.as_deref())?],
        Mode::Share { .. } => unreachable!("--share runs no cases"),
    };
    Ok(select(&cases, options.only.as_deref())?
        .into_iter()
        .cloned()
        .collect())
}

/// The commit, when the build set `MATRUST_GIT_COMMIT`.
pub fn build_commit() -> Option<String> {
    option_env!("MATRUST_GIT_COMMIT")
        .map(str::trim)
        .filter(|c| (7..=40).contains(&c.len()) && c.bytes().all(|b| b.is_ascii_hexdigit()))
        .map(str::to_string)
}

pub fn run(args: &[String]) -> u8 {
    let options = match parse_args(args) {
        Ok(Some(options)) => options,
        Ok(None) => {
            println!("{USAGE}");
            return 0;
        }
        Err(e) => {
            eprintln!("Error: {e}\nSee matrust bench --help");
            return 2;
        }
    };
    let result = match &options.mode {
        Mode::Share { file, open } => share(file, *open),
        _ => bench(&options),
    };
    match result {
        Ok(true) => 0,
        Ok(false) => 1,
        Err(Failure::Usage(e)) => {
            eprintln!("Error: {e}");
            2
        }
        Err(Failure::Run(e)) => {
            eprintln!("Error: {e}");
            1
        }
    }
}

/// Probe the destination before executing any user code. Do not touch an
/// existing result file: the final write still uses an atomic rename.
fn validate_output(path: &Path) -> Result<(), String> {
    let fail = |e| format!("--out {}: {e}", path.display());
    if path.file_name().is_none() || path.is_dir() {
        return Err(fail("not a file name".to_string()));
    }
    let parent = path
        .parent()
        .filter(|p| !p.as_os_str().is_empty())
        .unwrap_or_else(|| Path::new("."));
    let probe = tempfile::Builder::new()
        .prefix(".matrust-bench-write-")
        .tempfile_in(parent)
        .map_err(|e| fail(e.to_string()))?;
    probe.close().map_err(|e| fail(e.to_string()))
}

fn bench(options: &Options) -> Result<bool, Failure> {
    let cases = load_cases(options).map_err(usage)?;
    let started = SystemTime::now()
        .duration_since(SystemTime::UNIX_EPOCH)
        .map_or(0, |d| d.as_secs());
    let (year, month, day, hour, minute, second) = utc_parts(started);
    let out = options.out.clone().unwrap_or_else(|| {
        PathBuf::from(format!(
            "matrust-bench-{year:04}{month:02}{day:02}-{hour:02}{minute:02}{second:02}-{}.json",
            std::process::id()
        ))
    });
    validate_output(&out).map_err(usage)?;
    crate::initialize_worker_pool(options.threads).map_err(Failure::Run)?;

    let machine = machine::detect();
    let commit = build_commit();
    println!(
        "MatRust {} (commit {})",
        crate::VERSION,
        commit.as_deref().unwrap_or("unknown")
    );
    println!("Machine: {}", machine.line());
    println!("Threads: {}", options.threads);

    let builtin = builtin_cases();
    let mut results = Vec::with_capacity(cases.len());
    for (index, case) in cases.iter().enumerate() {
        eprintln!("MatRust {}/{}: {}", index + 1, cases.len(), case.id);
        let source = source_of(&case.id, &case.sha256, &builtin);
        results.push(CaseResult::from_run(case, source, run_case(case)));
    }

    let file = ResultFile {
        format: 1,
        matrust_version: crate::VERSION.into(),
        commit,
        timestamp: format!("{year:04}-{month:02}-{day:02}T{hour:02}:{minute:02}:{second:02}Z"),
        threads: options.threads,
        machine,
        cases: results,
    };
    println!();
    print!("{}", table(&file));
    let failed: Vec<&CaseResult> = file.cases.iter().filter(|c| c.error.is_some()).collect();
    if !failed.is_empty() {
        println!("\nFailed in MatRust:");
        for case in &failed {
            println!(
                "  {}: {}",
                case.id,
                case.error.as_deref().unwrap_or_default()
            );
        }
    }
    let json = serde_json::to_string_pretty(&file).map_err(|e| Failure::Run(e.to_string()))?;
    write_atomically(&out, format!("{json}\n").as_bytes()).map_err(Failure::Run)?;
    println!("\nSaved {}", out.display());
    println!(
        "To share MatRust's numbers: matrust bench --share {}",
        out.display()
    );
    Ok(failed.is_empty())
}

/// One row per case, columns padded to the widest cell.
fn table(file: &ResultFile) -> String {
    let range = |min: Option<f64>, max: Option<f64>| match (min, max) {
        (Some(min), Some(max)) => format!("{}-{}", format_ms(min), format_ms(max)),
        _ => "-".into(),
    };
    let median = |median: Option<f64>, kind: ErrorKind| match median {
        Some(median) => format_ms(median),
        None if kind != ErrorKind::None => kind.as_str().into(),
        None => "-".into(),
    };
    let mut rows = vec![vec![
        "case".to_string(),
        "MatRust ms".into(),
        "range".into(),
    ]];
    for case in &file.cases {
        rows.push(vec![
            case.id.clone(),
            median(case.median_ms, case.error_kind),
            range(case.min_ms, case.max_ms),
        ]);
    }
    let columns = rows[0].len();
    let widths: Vec<usize> = (0..columns)
        .map(|c| {
            rows.iter()
                .map(|row| row[c].chars().count())
                .max()
                .unwrap_or(0)
        })
        .collect();
    let mut text = String::new();
    for row in rows {
        let cells: Vec<String> = row
            .iter()
            .zip(&widths)
            .map(|(cell, width)| format!("{cell:<width$}"))
            .collect();
        text.push_str(cells.join("  ").trim_end());
        text.push('\n');
    }
    text
}

fn share(file: &Path, open: bool) -> Result<bool, Failure> {
    let shown = file.display();
    let text = read_bounded_text(file).map_err(usage)?;
    let result: ResultFile = serde_json::from_str(&text)
        .map_err(|e| usage(format!("{shown}: not a matrust bench result file ({e})")))?;
    let payload = SharePayload::from_result(&result).map_err(|e| usage(format!("{shown}: {e}")))?;
    let body = payload.body();
    let mut name = file.as_os_str().to_owned();
    name.push(".share.md");
    let saved = PathBuf::from(name);
    write_atomically(&saved, body.as_bytes()).map_err(Failure::Run)?;
    println!("{body}");
    println!("{NOTICE}");
    let url = issue_url();
    let paste = format!("Paste the text from {} into the form.", saved.display());
    if !open {
        println!("Issue form: {url}\n{paste}");
        return Ok(true);
    }
    print!("Open the browser? [y/N] ");
    let _ = std::io::stdout().flush();
    let mut answer = String::new();
    let _ = std::io::stdin().read_line(&mut answer);
    if !matches!(answer.trim().to_ascii_lowercase().as_str(), "y" | "yes") {
        println!("Not opened. The form is at {url}\n{paste}");
        return Ok(true);
    }
    let (program, args) = opener(&url);
    // Without a desktop session xdg-open may run the browser itself and wait
    // for it to close: give the opener 3 s, then leave it running.
    let opened = match Command::new(program)
        .args(&args)
        .stdin(Stdio::null())
        .spawn()
    {
        Ok(mut child) => {
            let deadline = Instant::now() + Duration::from_secs(3);
            loop {
                match child.try_wait() {
                    Ok(Some(status)) => break status.success(),
                    Ok(None) if Instant::now() < deadline => {
                        std::thread::sleep(Duration::from_millis(50));
                    }
                    _ => break true,
                }
            }
        }
        Err(_) => false,
    };
    if opened {
        println!("Opened {url}\n{paste}");
    } else {
        println!("Could not start a browser. Open {url}\n{paste}");
    }
    Ok(true)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parse(args: &[&str]) -> Result<Option<Options>, String> {
        parse_args(&args.iter().map(|a| a.to_string()).collect::<Vec<_>>())
    }

    #[test]
    fn options() {
        let options = parse(&["--threads", "3", "--only", "a, b", "--out", "r.json"])
            .unwrap()
            .unwrap();
        assert_eq!(options.mode, Mode::Builtin);
        assert_eq!(options.threads, 3);
        assert_eq!(options.only, Some(vec!["a".into(), "b".into()]));
        assert_eq!(options.out, Some("r.json".into()));
        assert_eq!(parse(&[]).unwrap().unwrap().threads, default_threads());
        assert_eq!(
            parse(&["--threads", "all"]).unwrap().unwrap().threads,
            default_threads()
        );
        let scripts = parse(&["--script", "a.m", "--script", "b.m"])
            .unwrap()
            .unwrap();
        assert_eq!(
            scripts.mode,
            Mode::Scripts(vec!["a.m".into(), "b.m".into()])
        );
        let op = parse(&["--op", "y=f(x);", "--setup", "x=1;", "--path", "lib"])
            .unwrap()
            .unwrap();
        assert_eq!(
            op.mode,
            Mode::Op {
                op: "y=f(x);".into(),
                setup: "x=1;".into(),
                check: String::new(),
                path: Some("lib".into())
            }
        );
        let share = parse(&["--share", "r.json", "--no-open"]).unwrap().unwrap();
        assert_eq!(
            share.mode,
            Mode::Share {
                file: "r.json".into(),
                open: false
            }
        );
        assert_eq!(parse(&["--help"]).unwrap(), None);
    }

    #[test]
    fn usage_errors() {
        for args in [
            &["--threads", "0"][..],
            &["--threads", "65"],
            &["--threads"],
            &["--fast"],
            &["--cases", "a.csv", "--op", "y=1;"],
            &["--script", "a.m", "--op", "y=1;"],
            &["--script", "a.m", "--path", "lib"],
            &["--setup", "x=1;"],
            &["--check", "y"],
            &["--op", "y=1;", "--only", "op"],
            &["--share", "r.json", "--threads", "2"],
            &["--share", "r.json", "--script", "a.m"],
            &["--no-open"],
            &["--out", "a.json", "--out", "b.json"],
            &["--only", ","],
        ] {
            assert!(parse(args).is_err(), "{args:?}");
        }
        // MatRust no longer runs or imports MATLAB.
        for flag in ["--matlab", "--matlab-timeout", "--import-matlab"] {
            assert_eq!(
                parse(&[flag, "auto"]).unwrap_err(),
                format!("Unknown option {flag}")
            );
        }
    }

    #[test]
    fn op_and_script_cases() {
        let case = op_case("y = x + 1;", " x = 1;", "", None).unwrap();
        assert_eq!(case.sha256, case_sha256(" x = 1;", "y = x + 1;", ""));
        assert_eq!((case.id.as_str(), case.directory), ("op", None));
        assert!(op_case("y = 1", "", "", None).is_err());
        assert!(op_case("y = 1;\nz = 2;", "", "", None).is_err());
        assert!(op_case("y = 1;", "", "", Some(Path::new("/nonexistent/lib"))).is_err());
        let dir = tempfile::tempdir().unwrap();
        let script = dir.path().join("my_run.m");
        std::fs::write(&script, "x = 1;\n").unwrap();
        let case = script_case(2, &script).unwrap();
        assert_eq!((case.id.as_str(), case.op.as_str()), ("script-2", ""));
        assert_eq!(case.script.as_ref().unwrap().source.as_ref(), "x = 1;\n");
        assert_eq!(case.sha256, sha256_hex(b"x = 1;\n"));
        assert_eq!(case.directory.as_deref(), Some(dir.path()));
        assert!(script_case(1, &dir.path().join("my-run.m")).is_err());
        assert!(script_case(1, &dir.path().join("run.mr")).is_err());
    }

    #[test]
    fn result_table() {
        let case = CaseResult {
            id: "a".into(),
            source: super::super::Source::User,
            sha256: "0".repeat(64),
            runs: 5,
            median_ms: Some(12.345),
            min_ms: Some(11.),
            max_ms: Some(150.),
            checksum: Some(1.),
            error_kind: ErrorKind::None,
            error: None,
        };
        let failed = CaseResult {
            id: "long_case".into(),
            median_ms: None,
            min_ms: None,
            max_ms: None,
            error_kind: ErrorKind::Unsupported,
            error: Some("op: nope".into()),
            ..case.clone()
        };
        let file = ResultFile {
            format: 1,
            matrust_version: "0.1.0".into(),
            commit: None,
            timestamp: String::new(),
            threads: 1,
            machine: Default::default(),
            cases: vec![case, failed],
        };
        assert_eq!(
            table(&file),
            "case       MatRust ms   range\n\
             a          12.3         11.0-150\n\
             long_case  unsupported  -\n"
        );
    }
}
