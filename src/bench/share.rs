//! `matrust bench --share`: the public payload and the issue text. Nothing
//! here sends anything; the user pastes the text into GitHub's form.
//!
//! The payload is built field by field from the result file. It never reads
//! `error` or `checksum` (nor the `matlab` part of older files), and user
//! cases lose their ids.
use super::{ErrorKind, ResultFile, Source, builtin_cases, is_sha256, source_of};
use serde::Serialize;

/// Printed before the prompt; the only place the tool mentions MATLAB.
pub const NOTICE: &str = "Nothing is sent until you paste this into the GitHub form and press \
     Submit. The issue is public, under your GitHub account. MATLAB results are never included.";

#[derive(Debug, PartialEq, Serialize)]
pub struct SharePayload {
    pub format: u32,
    pub matrust_version: String,
    pub commit: Option<String>,
    /// The result's UTC day, `2026-10-26`.
    pub date: String,
    pub threads: usize,
    pub machine: ShareMachine,
    pub cases: Vec<ShareCase>,
}

#[derive(Debug, PartialEq, Serialize)]
pub struct ShareMachine {
    pub os: Option<String>,
    pub os_version: Option<String>,
    pub arch: String,
    pub cpu: Option<String>,
    pub logical_cores: Option<usize>,
    pub ram_gib: Option<u64>,
}

#[derive(Debug, PartialEq, Serialize)]
pub struct ShareCase {
    /// The built-in id, or `user-1..N` in file order.
    pub id: String,
    pub source: Source,
    pub sha256: String,
    pub runs: usize,
    pub median_ms: Option<f64>,
    pub min_ms: Option<f64>,
    pub max_ms: Option<f64>,
    pub error_kind: ErrorKind,
}

/// The issue form; the text goes in by paste, never in the URL.
pub fn issue_url() -> String {
    format!(
        "https://github.com/{}/issues/new?template=benchmark.yml",
        super::RESULTS_REPO
    )
}

/// The program and arguments that open `url` in the default browser.
pub fn opener(url: &str) -> (&'static str, Vec<String>) {
    if cfg!(windows) {
        (
            "rundll32",
            vec!["url.dll,FileProtocolHandler".into(), url.into()],
        )
    } else if cfg!(target_os = "macos") {
        ("open", vec![url.into()])
    } else {
        ("xdg-open", vec![url.into()])
    }
}

fn plain(text: &str, max: usize) -> bool {
    !text.is_empty() && text.len() <= max && !text.chars().any(char::is_control)
}

/// Machine strings come from the OS; anything odd is dropped, not shared.
fn plain_or_none(text: &Option<String>) -> Option<String> {
    text.as_deref()
        .filter(|text| plain(text, 200))
        .map(str::to_string)
}

/// Whole microseconds are enough.
fn round_ms(value: Option<f64>) -> Option<f64> {
    value
        .filter(|v| v.is_finite())
        .map(|v| (v * 1e3).round() / 1e3)
}

impl SharePayload {
    pub fn from_result(result: &ResultFile) -> Result<Self, String> {
        if result.format != 1 {
            return Err(format!("result file format {} is not 1", result.format));
        }
        let version = &result.matrust_version;
        if !plain(version, 32)
            || !version
                .chars()
                .all(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '-' | '+'))
        {
            return Err("matrust_version is not a version".into());
        }
        if let Some(commit) = &result.commit
            && (!(7..=40).contains(&commit.len()) || !commit.bytes().all(|b| b.is_ascii_hexdigit()))
        {
            return Err("commit is not a git commit hash".into());
        }
        let date = result.timestamp.get(..10).unwrap_or_default();
        let digits_at = |range: std::ops::Range<usize>| {
            date.get(range)
                .is_some_and(|d| d.bytes().all(|b| b.is_ascii_digit()))
        };
        if !(digits_at(0..4) && digits_at(5..7) && digits_at(8..10))
            || date.as_bytes()[4] != b'-'
            || date.as_bytes()[7] != b'-'
        {
            return Err("timestamp does not start with a date".into());
        }
        let builtin = builtin_cases();
        let mut users = 0;
        let mut cases = Vec::with_capacity(result.cases.len());
        for case in &result.cases {
            if !is_sha256(&case.sha256) {
                return Err("a case sha256 is not a SHA-256".into());
            }
            let source = source_of(&case.id, &case.sha256, &builtin);
            let id = if source == Source::Builtin {
                case.id.clone()
            } else {
                users += 1;
                format!("user-{users}")
            };
            cases.push(ShareCase {
                id,
                source,
                sha256: case.sha256.clone(),
                runs: case.runs,
                median_ms: round_ms(case.median_ms),
                min_ms: round_ms(case.min_ms),
                max_ms: round_ms(case.max_ms),
                error_kind: case.error_kind,
            });
        }
        let machine = &result.machine;
        let arch = &machine.arch;
        Ok(Self {
            format: 1,
            matrust_version: version.clone(),
            commit: result.commit.clone(),
            date: date.to_string(),
            threads: result.threads,
            machine: ShareMachine {
                os: plain_or_none(&machine.os),
                os_version: plain_or_none(&machine.os_version),
                arch: if plain(arch, 32) {
                    arch.clone()
                } else {
                    "unknown".into()
                },
                cpu: plain_or_none(&machine.cpu),
                logical_cores: machine.logical_cores,
                ram_gib: machine.ram_gib,
            },
            cases,
        })
    }

    /// The issue text: a table, then the payload as a fenced JSON block.
    pub fn body(&self) -> String {
        let cell = |text: &str| text.replace('|', "\\|");
        let ms = |value: Option<f64>| value.map(super::format_ms).unwrap_or_else(|| "-".into());
        let machine = &self.machine;
        let mut machine_line = vec![
            match (&machine.os, &machine.os_version) {
                (Some(os), Some(version)) => format!("{os} {version}"),
                (Some(os), None) => os.clone(),
                (None, _) => "unknown OS".into(),
            },
            machine.arch.clone(),
            machine.cpu.clone().unwrap_or_else(|| "unknown CPU".into()),
        ];
        if let Some(cores) = machine.logical_cores {
            machine_line.push(format!("{cores} logical cores"));
        }
        if let Some(ram) = machine.ram_gib {
            machine_line.push(format!("{ram} GiB RAM"));
        }
        let mut text = format!(
            "MatRust {} (commit {}), {} thread{}, {}.\n\nMachine: {}.\n\n",
            self.matrust_version,
            self.commit.as_deref().unwrap_or("unknown"),
            self.threads,
            if self.threads == 1 { "" } else { "s" },
            self.date,
            cell(&machine_line.join(", "))
        );
        text.push_str("| case | source | runs | median ms | min ms | max ms | error |\n");
        text.push_str("|---|---|---:|---:|---:|---:|---|\n");
        for case in &self.cases {
            let source = match case.source {
                Source::Builtin => "builtin",
                Source::User => "user",
            };
            text.push_str(&format!(
                "| {} | {source} | {} | {} | {} | {} | {} |\n",
                case.id,
                case.runs,
                ms(case.median_ms),
                ms(case.min_ms),
                ms(case.max_ms),
                case.error_kind.as_str()
            ));
        }
        let json = serde_json::to_string_pretty(self).expect("payload serializes");
        text.push_str(&format!("\n```json\n{json}\n```\n"));
        text
    }
}

#[cfg(test)]
mod tests {
    use super::super::{CaseResult, machine::Machine};
    use super::*;
    use serde_json::Value;

    fn sample_result() -> ResultFile {
        let suite = builtin_cases();
        let user = |id: &str, setup: &str| CaseResult {
            id: id.into(),
            source: Source::User,
            sha256: super::super::case_sha256(setup, "y=f(x);", "y"),
            runs: 9,
            median_ms: Some(2.000_000_4),
            min_ms: Some(1.5),
            max_ms: Some(3.),
            checksum: Some(42.),
            error_kind: ErrorKind::None,
            error: None,
        };
        let mut failed = user("secret-name-2", "cd('/home/alice/secret')");
        failed.error_kind = ErrorKind::Unsupported;
        failed.error = Some("op: Undefined function 'secret_fn' in /home/alice".into());
        ResultFile {
            format: 1,
            matrust_version: "0.1.0".into(),
            commit: None,
            timestamp: "2026-10-26T14:03:12Z".into(),
            threads: 8,
            machine: Machine {
                os: Some("Ubuntu".into()),
                os_version: Some("24.04".into()),
                arch: "x86_64".into(),
                cpu: Some("AMD Ryzen 9 7950X".into()),
                logical_cores: Some(32),
                ram_gib: Some(62),
            },
            cases: vec![
                CaseResult {
                    id: suite[0].id.clone(),
                    source: Source::Builtin,
                    sha256: suite[0].sha256.clone(),
                    ..user("x", "")
                },
                user("secret-name", "addpath('/home/alice')"),
                // A built-in id with edited text is a user case.
                CaseResult {
                    id: suite[1].id.clone(),
                    source: Source::Builtin,
                    ..user("x", "edited")
                },
                failed,
            ],
        }
    }

    /// Every key path of a JSON value; array elements share one path.
    fn key_paths(value: &Value, prefix: &str, out: &mut std::collections::BTreeSet<String>) {
        match value {
            Value::Object(map) => {
                for (key, value) in map {
                    let path = format!("{prefix}{key}");
                    out.insert(path.clone());
                    key_paths(value, &format!("{path}."), out);
                }
            }
            Value::Array(items) => {
                for item in items {
                    key_paths(item, &format!("{prefix}[]."), out);
                }
            }
            _ => {}
        }
    }

    #[test]
    fn payload_keys_are_exactly_the_allowed_set() {
        let payload = SharePayload::from_result(&sample_result()).unwrap();
        let mut keys = std::collections::BTreeSet::new();
        key_paths(&serde_json::to_value(&payload).unwrap(), "", &mut keys);
        let allowed = [
            "format",
            "matrust_version",
            "commit",
            "date",
            "threads",
            "machine",
            "machine.os",
            "machine.os_version",
            "machine.arch",
            "machine.cpu",
            "machine.logical_cores",
            "machine.ram_gib",
            "cases",
            "cases.[].id",
            "cases.[].source",
            "cases.[].sha256",
            "cases.[].runs",
            "cases.[].median_ms",
            "cases.[].min_ms",
            "cases.[].max_ms",
            "cases.[].error_kind",
        ];
        assert_eq!(keys, allowed.iter().map(|k| k.to_string()).collect());
    }

    #[test]
    fn payload_and_body_hold_nothing_private() {
        let payload = SharePayload::from_result(&sample_result()).unwrap();
        let json = serde_json::to_string(&payload).unwrap().to_lowercase();
        let body = payload.body().to_lowercase();
        for word in ["secret", "alice", "/home", "matlab", "undefined"] {
            assert!(!json.contains(word), "payload holds {word}: {json}");
            assert!(!body.contains(word), "body holds {word}: {body}");
        }
        assert!(NOTICE.contains("MATLAB results are never included."));
    }

    #[test]
    fn user_cases_are_renamed_in_order() {
        let result = sample_result();
        let payload = SharePayload::from_result(&result).unwrap();
        let ids: Vec<(&str, Source)> = payload
            .cases
            .iter()
            .map(|c| (c.id.as_str(), c.source))
            .collect();
        let first = builtin_cases()[0].id.clone();
        assert_eq!(
            ids,
            [
                (first.as_str(), Source::Builtin),
                ("user-1", Source::User),
                ("user-2", Source::User),
                ("user-3", Source::User),
            ]
        );
        assert_eq!(payload.cases[1].sha256, result.cases[1].sha256);
        assert_eq!(payload.cases[1].median_ms, Some(2.));
        assert_eq!(payload.cases[3].error_kind, ErrorKind::Unsupported);
        assert_eq!(payload.date, "2026-10-26");
    }

    #[test]
    fn body_layout() {
        let payload = SharePayload::from_result(&sample_result()).unwrap();
        let body = payload.body();
        assert!(body.starts_with(
            "MatRust 0.1.0 (commit unknown), 8 threads, 2026-10-26.\n\n\
             Machine: Ubuntu 24.04, x86_64, AMD Ryzen 9 7950X, 32 logical cores, 62 GiB RAM.\n\n\
             | case | source | runs | median ms | min ms | max ms | error |\n"
        ));
        assert!(body.contains("| user-1 | user | 9 | 2.00 | 1.50 | 3.00 | none |\n"));
        let json = body
            .split("```json\n")
            .nth(1)
            .unwrap()
            .strip_suffix("\n```\n")
            .unwrap();
        let parsed: Value = serde_json::from_str(json).unwrap();
        assert_eq!(parsed, serde_json::to_value(&payload).unwrap());
    }

    #[test]
    fn suspicious_files_are_refused() {
        let mut result = sample_result();
        result.cases[1].sha256 = "/home/alice".into();
        assert!(SharePayload::from_result(&result).is_err());
        let mut result = sample_result();
        result.commit = Some("not a hash".into());
        assert!(SharePayload::from_result(&result).is_err());
        let mut result = sample_result();
        result.timestamp = "yesterday".into();
        assert!(SharePayload::from_result(&result).is_err());
        let mut result = sample_result();
        result.machine.cpu = Some("line\nbreak".into());
        assert_eq!(
            SharePayload::from_result(&result).unwrap().machine.cpu,
            None
        );
    }

    #[test]
    fn issue_form_url_and_opener() {
        assert_eq!(
            issue_url(),
            "https://github.com/leoom-labs/matrust/issues/new?template=benchmark.yml"
        );
        let (program, args) = opener("https://example.org/?a=1&b=2");
        assert_eq!(args.last().unwrap(), "https://example.org/?a=1&b=2");
        if cfg!(windows) {
            assert_eq!(
                (program, args[0].as_str()),
                ("rundll32", "url.dll,FileProtocolHandler")
            );
        } else if cfg!(target_os = "macos") {
            assert_eq!(program, "open");
        } else {
            assert_eq!(program, "xdg-open");
        }
    }
}
