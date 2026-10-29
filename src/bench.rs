//! Benchmark runner behind `matrust bench`. It times MatRust only.
//!
//! Run rules. Per case, a fresh engine runs setup, op once and check: that
//! value is the checksum. Then another fresh engine runs setup once (untimed),
//! op twice as warm-up and timed op runs until the stop rule holds. Workspace
//! names starting with `bench_` are reserved for the runner.
//!
//! Error messages and checksums stay on the user's computer: the screen and
//! the local result file. The share payload ([`share::SharePayload`]) is built
//! from an explicit field list and never reads them.
pub mod cli;
pub mod machine;
pub mod share;

use crate::{Array, Engine};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::io::Read;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

/// GitHub repository whose issue form receives shared results.
pub const RESULTS_REPO: &str = "leoom-labs/matrust";
/// The built-in suite, `bench/cases.csv`.
pub const BUILTIN_CASES: &str = include_str!("../bench/cases.csv");

pub const WARMUP_RUNS: usize = 2;
pub const MIN_RUNS: usize = 5;
pub const MAX_RUNS: usize = 200;
pub const TIME_BUDGET: Duration = Duration::from_secs(1);
/// Bound on every file the tool reads: cases, scripts, result files.
pub const MAX_INPUT_BYTES: usize = 1_000_000;
pub const CASES_HEADER: &str = "id,setup,op,check";
const CHECK_NAME: &str = "bench_checksum";

#[derive(Clone, Debug, PartialEq)]
pub struct Case {
    pub id: String,
    pub setup: String,
    /// Empty for `--script` cases, which run [`Case::script`] instead.
    pub op: String,
    /// Empty only for `--op` and `--script` cases: no checksum.
    pub check: String,
    /// Function folder (`--path`, the folder of a `--script`).
    pub directory: Option<PathBuf>,
    /// Exact document supplied by `--script`, independent of name lookup.
    pub script: Option<Script>,
    /// [`case_sha256`] of the text, or of the script file's bytes.
    pub sha256: String,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Script {
    pub path: PathBuf,
    pub source: std::sync::Arc<str>,
}

impl Case {
    pub fn new(id: &str, setup: &str, op: &str, check: &str) -> Self {
        Self {
            id: id.into(),
            setup: setup.into(),
            op: op.into(),
            check: check.into(),
            directory: None,
            script: None,
            sha256: case_sha256(setup, op, check),
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Source {
    Builtin,
    User,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ErrorKind {
    None,
    Unsupported,
    Error,
    Timeout,
}

impl ErrorKind {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::None => "none",
            Self::Unsupported => "unsupported",
            Self::Error => "error",
            Self::Timeout => "timeout",
        }
    }

    /// A MatRust error caused by a missing feature rather than by the case.
    pub fn classify(message: &str) -> Self {
        let message = message.to_ascii_lowercase();
        let unsupported = [
            "not supported",
            "does not support",
            "unsupported",
            "not implemented",
            "undefined function",
        ];
        if unsupported.iter().any(|text| message.contains(text)) {
            Self::Unsupported
        } else {
            Self::Error
        }
    }
}

pub fn sha256_hex(bytes: &[u8]) -> String {
    Sha256::digest(bytes)
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect()
}

/// SHA-256 of `setup + "\0" + op + "\0" + check`, exactly as given.
pub fn case_sha256(setup: &str, op: &str, check: &str) -> String {
    sha256_hex(format!("{setup}\0{op}\0{check}").as_bytes())
}

/// Read at most [`MAX_INPUT_BYTES`]; a longer file is refused, not truncated.
pub fn read_bounded(path: &Path) -> Result<Vec<u8>, String> {
    let file = std::fs::File::open(path).map_err(|e| format!("{}: {e}", path.display()))?;
    let mut bytes = Vec::new();
    file.take(MAX_INPUT_BYTES as u64 + 1)
        .read_to_end(&mut bytes)
        .map_err(|e| format!("{}: {e}", path.display()))?;
    if bytes.len() > MAX_INPUT_BYTES {
        return Err(format!("{}: exceeds 1 MB", path.display()));
    }
    Ok(bytes)
}

pub fn read_bounded_text(path: &Path) -> Result<String, String> {
    String::from_utf8(read_bounded(path)?)
        .map_err(|_| format!("{}: not UTF-8 text", path.display()))
}

/// Write to a temporary name in the same folder, then rename.
pub fn write_atomically(path: &Path, contents: &[u8]) -> Result<(), String> {
    use std::io::Write;
    path.file_name()
        .ok_or_else(|| format!("{}: not a file name", path.display()))?;
    let parent = match path.parent() {
        Some(parent) if !parent.as_os_str().is_empty() => parent,
        _ => Path::new("."),
    };
    // A random name opened with O_EXCL: a file or link planted in a shared
    // folder cannot redirect the write.
    let mut temporary = tempfile::Builder::new()
        .prefix(".matrust-bench-")
        .tempfile_in(parent)
        .map_err(|e| format!("{}: {e}", path.display()))?;
    temporary
        .write_all(contents)
        .map_err(|e| format!("{}: {e}", path.display()))?;
    temporary
        .persist(path)
        .map(drop)
        .map_err(|e| format!("{}: {}", path.display(), e.error))
}

/// One record per line. A field may be wrapped in double quotes (needed when
/// it holds commas); `""` inside quotes stands for one quote character.
pub fn parse_csv_line(line: &str) -> Result<Vec<String>, String> {
    #[derive(PartialEq)]
    enum State {
        Start,
        Plain,
        Quoted,
        Closed,
    }
    let mut fields = Vec::new();
    let mut field = String::new();
    let mut state = State::Start;
    let mut chars = line.chars().peekable();
    while let Some(c) = chars.next() {
        state = match (state, c) {
            (State::Quoted, '"') if chars.peek() == Some(&'"') => {
                chars.next();
                field.push('"');
                State::Quoted
            }
            (State::Quoted, '"') => State::Closed,
            (State::Quoted, c) => {
                field.push(c);
                State::Quoted
            }
            (_, ',') => {
                fields.push(std::mem::take(&mut field));
                State::Start
            }
            (State::Start, '"') => State::Quoted,
            (State::Closed, _) => return Err("text after a closing quote".into()),
            (_, '"') => return Err("quote inside an unquoted field".into()),
            (_, c) => {
                field.push(c);
                State::Plain
            }
        };
    }
    if state == State::Quoted {
        return Err("unterminated quoted field".into());
    }
    fields.push(field);
    Ok(fields)
}

fn valid_id(id: &str) -> bool {
    !id.is_empty()
        && id
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-')
}

/// A cases file: header `id,setup,op,check`, then one case per line. Fields
/// retain their parsed text; the SHA-256 includes whitespace in code fields.
pub fn parse_cases(text: &str) -> Result<Vec<Case>, String> {
    let mut lines = text
        .trim_start_matches('\u{feff}')
        .lines()
        .enumerate()
        .filter(|(_, line)| !line.trim().is_empty());
    match lines.next() {
        Some((_, header)) if header.trim() == CASES_HEADER => {}
        _ => return Err(format!("first line must be the header {CASES_HEADER}")),
    }
    let mut cases: Vec<Case> = Vec::new();
    for (index, line) in lines {
        let line_error = |message: &str| format!("line {}: {message}", index + 1);
        let fields = parse_csv_line(line).map_err(|e| line_error(&e))?;
        let [id, setup, op, check] = <[String; 4]>::try_from(fields)
            .map_err(|f| line_error(&format!("expected 4 fields, found {}", f.len())))?;
        let id = id.trim().to_string();
        if !valid_id(&id) {
            return Err(line_error("id must be letters, digits, _ or -"));
        }
        if cases.iter().any(|case| case.id == id) {
            return Err(line_error(&format!("duplicate id {id}")));
        }
        // Without the semicolon the timed runs would include printing y.
        if !op.trim_end().ends_with(';') {
            return Err(line_error("op must end with ';'"));
        }
        if check.trim().is_empty() {
            return Err(line_error("check is empty"));
        }
        cases.push(Case::new(&id, &setup, &op, &check));
    }
    if cases.is_empty() {
        return Err("no cases".into());
    }
    Ok(cases)
}

/// The compiled-in suite.
pub fn builtin_cases() -> Vec<Case> {
    parse_cases(BUILTIN_CASES).expect("bench/cases.csv parses")
}

/// Cases in file order, restricted to `only` when given.
pub fn select<'a>(cases: &'a [Case], only: Option<&[String]>) -> Result<Vec<&'a Case>, String> {
    let Some(only) = only else {
        return Ok(cases.iter().collect());
    };
    if let Some(unknown) = only.iter().find(|id| !cases.iter().any(|c| &c.id == *id)) {
        return Err(format!("unknown case id {unknown}"));
    }
    Ok(cases.iter().filter(|c| only.contains(&c.id)).collect())
}

/// `builtin` only when the id and the text both equal a compiled-in case.
pub fn source_of(id: &str, sha256: &str, builtin: &[Case]) -> Source {
    if builtin.iter().any(|c| c.id == id && c.sha256 == sha256) {
        Source::Builtin
    } else {
        Source::User
    }
}

/// Stop rule: at least `MIN_RUNS` timed runs, then stop once they add up to
/// `TIME_BUDGET`; never more than `MAX_RUNS`.
pub fn should_stop(runs: usize, timed: Duration) -> bool {
    runs >= MAX_RUNS || (runs >= MIN_RUNS && timed >= TIME_BUDGET)
}

pub fn median(values: &[f64]) -> f64 {
    let mut sorted = values.to_vec();
    sorted.sort_by(f64::total_cmp);
    let mid = sorted.len() / 2;
    if sorted.len().is_multiple_of(2) {
        (sorted[mid - 1] + sorted[mid]) / 2.
    } else {
        sorted[mid]
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Measurement {
    pub runs: usize,
    pub median_ms: f64,
    pub min_ms: f64,
    pub max_ms: f64,
}

impl Measurement {
    pub fn from_times(times_ms: &[f64]) -> Self {
        Self {
            runs: times_ms.len(),
            median_ms: median(times_ms),
            min_ms: times_ms.iter().copied().fold(f64::INFINITY, f64::min),
            max_ms: times_ms.iter().copied().fold(f64::NEG_INFINITY, f64::max),
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct CaseRun {
    pub measurement: Measurement,
    /// `None` when the case has no check.
    pub checksum: Option<f64>,
}

fn fresh_engine(case: &Case) -> Result<Engine, String> {
    let mut engine = Engine::default();
    if let Some(directory) = &case.directory {
        engine.set_function_directory(directory)?;
    }
    Ok(engine)
}

fn execute_op(engine: &mut Engine, case: &Case) -> Result<(), String> {
    match &case.script {
        Some(script) => engine.execute_document(&script.source, &script.path),
        None => engine.execute(&case.op),
    }
    .map(|_| ())
    .map_err(|e| format!("op: {e}"))
}

/// Fresh engine: setup, op once, check.
pub fn checksum(case: &Case) -> Result<Option<f64>, String> {
    let mut engine = fresh_engine(case)?;
    engine
        .execute(&case.setup)
        .map_err(|e| format!("setup: {e}"))?;
    execute_op(&mut engine, case)?;
    if case.check.is_empty() {
        return Ok(None);
    }
    engine
        .execute(&format!("{CHECK_NAME} = {};", case.check))
        .map_err(|e| format!("check: {e}"))?;
    match engine.array(CHECK_NAME) {
        Some(Array::Numeric(value)) if value.len() == 1 => Ok(Some(value.data()[0])),
        _ => Err("check: must give one real double scalar".into()),
    }
}

/// Fresh engine: setup once, warm-up runs, then timed runs until the stop rule.
pub fn time_case(case: &Case) -> Result<Measurement, String> {
    let mut engine = fresh_engine(case)?;
    engine
        .execute(&case.setup)
        .map_err(|e| format!("setup: {e}"))?;
    for _ in 0..WARMUP_RUNS {
        execute_op(&mut engine, case)?;
    }
    let mut times = Vec::new();
    let mut timed = Duration::ZERO;
    while !should_stop(times.len(), timed) {
        let start = Instant::now();
        execute_op(&mut engine, case)?;
        let elapsed = start.elapsed();
        timed += elapsed;
        times.push(elapsed.as_secs_f64() * 1e3);
    }
    Ok(Measurement::from_times(&times))
}

/// The checksum run, then the timed run.
pub fn run_case(case: &Case) -> Result<CaseRun, String> {
    let checksum = checksum(case)?;
    let measurement = time_case(case)?;
    Ok(CaseRun {
        measurement,
        checksum,
    })
}

pub fn is_sha256(text: &str) -> bool {
    text.len() == 64 && text.bytes().all(|b| matches!(b, b'0'..=b'9' | b'a'..=b'f'))
}

/// Milliseconds with about three significant digits.
pub fn format_ms(value: f64) -> String {
    if value >= 100. {
        format!("{value:.0}")
    } else if value >= 10. {
        format!("{value:.1}")
    } else if value >= 1. {
        format!("{value:.2}")
    } else {
        format!("{value:.3}")
    }
}

/// UTC `(year, month, day, hour, minute, second)` of a Unix time.
pub fn utc_parts(unix_seconds: u64) -> (i64, u32, u32, u32, u32, u32) {
    // Howard Hinnant's civil_from_days.
    let days = (unix_seconds / 86_400) as i64 + 719_468;
    let era = days.div_euclid(146_097);
    let day_of_era = days.rem_euclid(146_097);
    let year_of_era =
        (day_of_era - day_of_era / 1_460 + day_of_era / 36_524 - day_of_era / 146_096) / 365;
    let day_of_year = day_of_era - (365 * year_of_era + year_of_era / 4 - year_of_era / 100);
    let month_index = (5 * day_of_year + 2) / 153;
    let day = (day_of_year - (153 * month_index + 2) / 5 + 1) as u32;
    let month = if month_index < 10 {
        month_index + 3
    } else {
        month_index - 9
    } as u32;
    let year = year_of_era + era * 400 + i64::from(month <= 2);
    let seconds = unix_seconds % 86_400;
    (
        year,
        month,
        day,
        (seconds / 3_600) as u32,
        (seconds / 60 % 60) as u32,
        (seconds % 60) as u32,
    )
}

/// The local result file, `matrust-bench-*.json`. Unknown fields are ignored:
/// files from older versions may hold a `matlab` part, which nothing reads.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ResultFile {
    pub format: u32,
    pub matrust_version: String,
    pub commit: Option<String>,
    /// UTC, `2026-10-26T14:03:12Z`.
    pub timestamp: String,
    pub threads: usize,
    pub machine: machine::Machine,
    pub cases: Vec<CaseResult>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct CaseResult {
    pub id: String,
    pub source: Source,
    pub sha256: String,
    pub runs: usize,
    pub median_ms: Option<f64>,
    pub min_ms: Option<f64>,
    pub max_ms: Option<f64>,
    pub checksum: Option<f64>,
    pub error_kind: ErrorKind,
    /// Local only; the share payload has `error_kind` alone.
    pub error: Option<String>,
}

impl CaseResult {
    /// A case's [`run_case`] result; a failure keeps its message.
    pub fn from_run(case: &Case, source: Source, run: Result<CaseRun, String>) -> Self {
        let (measurement, checksum, error) = match run {
            Ok(run) => (Some(run.measurement), run.checksum, None),
            Err(e) => (None, None, Some(e)),
        };
        Self {
            id: case.id.clone(),
            source,
            sha256: case.sha256.clone(),
            runs: measurement.map_or(0, |m| m.runs),
            median_ms: measurement.map(|m| m.median_ms),
            min_ms: measurement.map(|m| m.min_ms),
            max_ms: measurement.map(|m| m.max_ms),
            checksum,
            error_kind: error
                .as_deref()
                .map_or(ErrorKind::None, ErrorKind::classify),
            error,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn strings(values: &[&str]) -> Vec<String> {
        values.iter().map(|v| v.to_string()).collect()
    }

    #[test]
    fn csv_line_quoting() {
        assert_eq!(
            parse_csv_line("a,b,,c").unwrap(),
            strings(&["a", "b", "", "c"])
        );
        assert_eq!(
            parse_csv_line("id,\"x=repmat(a,1,2);\",y=x';,\"say \"\"hi\"\"\"").unwrap(),
            strings(&["id", "x=repmat(a,1,2);", "y=x';", "say \"hi\""])
        );
        assert_eq!(parse_csv_line("\"\",x").unwrap(), strings(&["", "x"]));
        assert_eq!(
            parse_csv_line("a,\"b,c").unwrap_err(),
            "unterminated quoted field"
        );
        assert_eq!(
            parse_csv_line("a,\"b\"c").unwrap_err(),
            "text after a closing quote"
        );
        assert_eq!(
            parse_csv_line("a,b\"c").unwrap_err(),
            "quote inside an unquoted field"
        );
    }

    #[test]
    fn cases_file() {
        let text = "\u{feff}id,setup,op,check\r\n\
                    one,\"n=3; A=repmat(1,n,n);\",y=A*A;,sum(y(:))\r\n\
                    \r\n\
                    two-b,,y=1;,y\r\n";
        let cases = parse_cases(text).unwrap();
        assert_eq!(
            cases,
            [
                Case::new("one", "n=3; A=repmat(1,n,n);", "y=A*A;", "sum(y(:))"),
                Case::new("two-b", "", "y=1;", "y"),
            ]
        );
        let error = |text: &str| parse_cases(text).unwrap_err();
        assert_eq!(
            error("id,op\na,y=1;"),
            "first line must be the header id,setup,op,check"
        );
        assert_eq!(error("id,setup,op,check\n"), "no cases");
        assert_eq!(
            error("id,setup,op,check\na,,y=1;\n"),
            "line 2: expected 4 fields, found 3"
        );
        assert_eq!(
            error("id,setup,op,check\na,,y=1;,y\na,,y=2;,y"),
            "line 3: duplicate id a"
        );
        assert_eq!(
            error("id,setup,op,check\na.b,,y=1;,y"),
            "line 2: id must be letters, digits, _ or -"
        );
        assert_eq!(
            error("id,setup,op,check\na,,y=1,y"),
            "line 2: op must end with ';'"
        );
        assert_eq!(
            error("id,setup,op,check\na,,y=1;, "),
            "line 2: check is empty"
        );
        assert_eq!(
            error("id,setup,op,check\na,\"x,y=1;,y"),
            "line 2: unterminated quoted field"
        );
    }

    #[test]
    fn case_selection() {
        let cases = parse_cases("id,setup,op,check\na,,y=1;,y\nb,,y=2;,y\nc,,y=3;,y").unwrap();
        let ids = |only: Option<&[String]>| -> Vec<String> {
            select(&cases, only)
                .unwrap()
                .iter()
                .map(|c| c.id.clone())
                .collect()
        };
        assert_eq!(ids(None), ["a", "b", "c"]);
        assert_eq!(ids(Some(&strings(&["c", "a"]))), ["a", "c"]);
        assert_eq!(
            select(&cases, Some(&strings(&["a", "z"]))).unwrap_err(),
            "unknown case id z"
        );
    }

    #[test]
    fn stop_rule() {
        let ms = Duration::from_millis;
        assert!(!should_stop(0, ms(0)));
        assert!(!should_stop(4, ms(60_000)));
        assert!(!should_stop(5, ms(999)));
        assert!(should_stop(5, ms(1000)));
        assert!(!should_stop(199, ms(10)));
        assert!(should_stop(200, ms(10)));
    }

    #[test]
    fn median_and_range() {
        assert_eq!(median(&[3., 1., 2.]), 2.);
        assert_eq!(median(&[4., 1., 3., 2.]), 2.5);
        assert_eq!(
            Measurement::from_times(&[4., 1., 3., 2.]),
            Measurement {
                runs: 4,
                median_ms: 2.5,
                min_ms: 1.,
                max_ms: 4.
            }
        );
    }

    #[test]
    fn sha256_vectors() {
        // FIPS 180-2 test vector.
        assert_eq!(
            sha256_hex(b"abc"),
            "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad"
        );
        // printf 'n=3;\0y=n*2;\0y' | sha256sum
        assert_eq!(
            case_sha256("n=3;", "y=n*2;", "y"),
            "de18b3af6ad6821089eb9763c2c26c23f25459d85138dda1854c049f4e7bedd4"
        );
        assert_ne!(case_sha256("a", "b", ""), case_sha256("a", "", "b"));
    }

    #[test]
    fn csv_hash_preserves_parsed_whitespace() {
        let cases = parse_cases(
            "id,setup,op,check\r\nquoted,\" x=3; \t\",\" y=x*2;  \",\" y \"\r\nplain, x=3; \t, y=x*2;  , y \r\n",
        ).unwrap();
        for case in &cases {
            assert_eq!(
                (&*case.setup, &*case.op, &*case.check),
                (" x=3; \t", " y=x*2;  ", " y ")
            );
            assert_eq!(case.sha256, sha256_hex(b" x=3; \t\0 y=x*2;  \0 y "));
            assert_ne!(case.sha256, case_sha256("x=3;", "y=x*2;", "y"));
            assert_eq!(checksum(case).unwrap(), Some(6.));
        }
    }

    #[test]
    fn builtin_source_needs_id_and_text() {
        let suite = builtin_cases();
        let first = &suite[0];
        assert_eq!(source_of(&first.id, &first.sha256, &suite), Source::Builtin);
        assert_eq!(source_of(&first.id, &"0".repeat(64), &suite), Source::User);
        assert_eq!(source_of("mine", &first.sha256, &suite), Source::User);
    }

    #[test]
    fn error_kinds() {
        assert_eq!(
            ErrorKind::classify("op: Undefined function or variable 'f'."),
            ErrorKind::Unsupported
        );
        assert_eq!(
            ErrorKind::classify("op: xcorr: matrix input is not supported yet"),
            ErrorKind::Unsupported
        );
        assert_eq!(
            ErrorKind::classify("op: Index exceeds matrix dimensions"),
            ErrorKind::Error
        );
    }

    #[test]
    fn unsupported_real_engine_messages() {
        let mut engine = Engine::default();
        let message = engine.execute("uicontrol();").unwrap_err();
        assert!(
            message.contains("MatRust does not support yet."),
            "{message}"
        );
        assert_eq!(ErrorKind::classify(&message), ErrorKind::Unsupported);
        let case = Case::new("unsupported", "", "uicontrol();", "");
        assert_eq!(
            CaseResult::from_run(&case, Source::User, run_case(&case)).error_kind,
            ErrorKind::Unsupported
        );
        for name in ["uicontrol", "sum", "no_such_bench_function"] {
            assert_eq!(
                ErrorKind::classify(&crate::help::unknown_function(name)),
                ErrorKind::Unsupported
            );
        }
    }

    #[test]
    fn case_result_from_run() {
        let case = Case::new("a", "", "y=1;", "y");
        let ok = Ok(CaseRun {
            measurement: Measurement {
                runs: 5,
                median_ms: 1.25,
                min_ms: 1.,
                max_ms: 2.,
            },
            checksum: Some(0.1),
        });
        let done = CaseResult::from_run(&case, Source::Builtin, ok);
        assert_eq!(
            (done.runs, done.median_ms, done.min_ms, done.max_ms),
            (5, Some(1.25), Some(1.), Some(2.))
        );
        assert_eq!(
            (done.checksum, done.error_kind),
            (Some(0.1), ErrorKind::None)
        );
        assert_eq!((done.source, done.error), (Source::Builtin, None));
        assert_eq!(done.sha256, case.sha256);
        let failed = CaseResult::from_run(
            &case,
            Source::User,
            Err("op: Undefined function 'f'".into()),
        );
        assert_eq!(
            (failed.runs, failed.median_ms, failed.checksum),
            (0, None, None)
        );
        assert_eq!(failed.error_kind, ErrorKind::Unsupported);
        assert_eq!(failed.error.as_deref(), Some("op: Undefined function 'f'"));
    }

    #[test]
    fn utc_calendar() {
        assert_eq!(utc_parts(0), (1970, 1, 1, 0, 0, 0));
        // date -u -d @1792936992
        assert_eq!(utc_parts(1_792_936_992), (2026, 10, 25, 14, 3, 12));
        assert_eq!(utc_parts(951_782_400), (2000, 2, 29, 0, 0, 0));
    }

    #[test]
    fn bounded_reads_and_atomic_writes() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("big.csv");
        std::fs::write(&path, vec![b'x'; MAX_INPUT_BYTES + 1]).unwrap();
        assert!(read_bounded(&path).unwrap_err().ends_with("exceeds 1 MB"));
        std::fs::write(&path, vec![b'x'; MAX_INPUT_BYTES]).unwrap();
        assert_eq!(read_bounded(&path).unwrap().len(), MAX_INPUT_BYTES);
        let out = dir.path().join("out.json");
        write_atomically(&out, b"{}").unwrap();
        assert_eq!(std::fs::read(&out).unwrap(), b"{}");
        assert_eq!(std::fs::read_dir(dir.path()).unwrap().count(), 2);
    }

    #[cfg(unix)]
    #[test]
    fn atomic_write_replaces_a_planted_link_without_following_it() {
        let dir = tempfile::tempdir().unwrap();
        let victim = dir.path().join("victim.txt");
        std::fs::write(&victim, b"keep").unwrap();
        let out = dir.path().join("r.json");
        std::os::unix::fs::symlink(&victim, &out).unwrap();
        write_atomically(&out, b"{}").unwrap();
        assert_eq!(std::fs::read(&victim).unwrap(), b"keep");
        assert!(std::fs::symlink_metadata(&out).unwrap().is_file());
        assert_eq!(std::fs::read(&out).unwrap(), b"{}");
    }
}
