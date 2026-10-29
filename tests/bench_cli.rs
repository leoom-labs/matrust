//! `matrust bench`: modes, sharing, older result files, usage errors.
use matrust::bench::{self, ErrorKind, ResultFile, Source, case_sha256, sha256_hex};
use std::path::{Path, PathBuf};
use std::process::{Command, Output, Stdio};

fn matrust(dir: &Path, args: &[&str]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_matrust"))
        .arg("bench")
        .args(args)
        .current_dir(dir)
        .stdin(Stdio::null())
        .output()
        .expect("run matrust")
}

fn stdout(output: &Output) -> String {
    String::from_utf8_lossy(&output.stdout).into_owned()
}

fn describe(output: &Output) -> String {
    format!(
        "status {:?}\nstdout:\n{}\nstderr:\n{}",
        output.status.code(),
        stdout(output),
        String::from_utf8_lossy(&output.stderr)
    )
}

fn result(path: &Path) -> ResultFile {
    serde_json::from_str(&std::fs::read_to_string(path).unwrap()).unwrap()
}

#[test]
fn builtin_cases_run_with_finite_checks() {
    let cases = bench::builtin_cases();
    assert!(cases.len() >= 18, "{} built-in cases", cases.len());
    for case in &cases {
        match bench::checksum(case) {
            Ok(Some(value)) => assert!(value.is_finite(), "{}: check gave {value}", case.id),
            other => panic!("{}: {other:?}", case.id),
        }
    }
}

#[test]
fn builtin_suite_writes_result_file() {
    let dir = tempfile::tempdir().unwrap();
    let output = matrust(
        dir.path(),
        &["--only", "sum_1e6", "--threads", "2", "--out", "r.json"],
    );
    assert_eq!(output.status.code(), Some(0), "{}", describe(&output));
    let text = stdout(&output);
    assert!(text.starts_with("MatRust "), "{text}");
    assert!(text.contains("\nMachine: "), "{text}");
    assert!(text.contains("\nThreads: 2\n"), "{text}");
    assert!(!text.to_lowercase().contains("matlab"), "{text}");
    let file = result(&dir.path().join("r.json"));
    assert_eq!((file.format, file.threads), (1, 2));
    assert!(file.timestamp.ends_with('Z') && file.timestamp.len() == 20);
    assert_eq!(file.cases.len(), 1);
    let case = &file.cases[0];
    assert_eq!(
        (case.id.as_str(), case.source),
        ("sum_1e6", Source::Builtin)
    );
    assert_eq!(case.error_kind, ErrorKind::None);
    assert!(case.runs >= 5 && case.min_ms <= case.median_ms && case.median_ms <= case.max_ms);
    assert!(case.checksum.is_some_and(f64::is_finite));
    let json = std::fs::read_to_string(dir.path().join("r.json")).unwrap();
    assert!(!json.contains("matlab"), "{json}");
    // Default name: matrust-bench-<YYYYMMDD-HHMMSS>-<pid>.json in the current folder.
    let output = matrust(dir.path(), &["--only", "sum_1e6", "--threads", "1"]);
    assert_eq!(output.status.code(), Some(0), "{}", describe(&output));
    let names: Vec<String> = std::fs::read_dir(dir.path())
        .unwrap()
        .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
        .filter(|name| name.starts_with("matrust-bench-"))
        .collect();
    assert_eq!(names.len(), 1, "{names:?}");
    let parts: Vec<&str> = names[0].trim_end_matches(".json").split('-').collect();
    assert_eq!(parts.len(), 5, "{names:?}");
    assert!(parts[2].len() == 8 && parts[3].len() == 6, "{names:?}");
}

#[test]
fn script_op_and_cases_modes_run_tiny_cases() {
    let dir = tempfile::tempdir().unwrap();
    let scripts = dir.path().join("scripts");
    std::fs::create_dir(&scripts).unwrap();
    std::fs::write(scripts.join("tiny_one.m"), "y = 2 + 3;\n").unwrap();
    std::fs::write(scripts.join("tiny_two.m"), "z = helper_add(1, 2);\n").unwrap();
    std::fs::write(
        scripts.join("helper_add.m"),
        "function s = helper_add(a, b)\ns = a + b;\nend\n",
    )
    .unwrap();
    let output = matrust(
        dir.path(),
        &[
            "--script",
            "scripts/tiny_one.m",
            "--script",
            "scripts/tiny_two.m",
            "--threads",
            "1",
            "--out",
            "s.json",
        ],
    );
    assert_eq!(output.status.code(), Some(0), "{}", describe(&output));
    let file = result(&dir.path().join("s.json"));
    let ids: Vec<&str> = file.cases.iter().map(|c| c.id.as_str()).collect();
    assert_eq!(ids, ["script-1", "script-2"]);
    assert_eq!(file.cases[0].sha256, sha256_hex(b"y = 2 + 3;\n"));
    assert!(file.cases.iter().all(|c| c.error_kind == ErrorKind::None
        && c.checksum.is_none()
        && c.source == Source::User));

    let lib = dir.path().join("lib");
    std::fs::create_dir(&lib).unwrap();
    std::fs::write(
        lib.join("twice.m"),
        "function y = twice(x)\ny = 2 * x;\nend\n",
    )
    .unwrap();
    let output = matrust(
        dir.path(),
        &[
            "--op",
            "y = twice(x);",
            "--setup",
            "x = 21;",
            "--check",
            "y",
            "--path",
            "lib",
            "--threads",
            "1",
            "--out",
            "o.json",
        ],
    );
    assert_eq!(output.status.code(), Some(0), "{}", describe(&output));
    let file = result(&dir.path().join("o.json"));
    assert_eq!(file.cases[0].id, "op");
    assert_eq!(file.cases[0].checksum, Some(42.));
    assert_eq!(
        file.cases[0].sha256,
        case_sha256("x = 21;", "y = twice(x);", "y")
    );

    std::fs::write(
        dir.path().join("mine.csv"),
        "id,setup,op,check\nadd_one,x=1;,y=x+1;,y\nbroken,,y=no_such_function(1);,y\n",
    )
    .unwrap();
    let output = matrust(
        dir.path(),
        &["--cases", "mine.csv", "--threads", "1", "--out", "c.json"],
    );
    assert_eq!(output.status.code(), Some(1), "{}", describe(&output));
    let text = stdout(&output);
    assert!(text.contains("broken: op: "), "{text}");
    let file = result(&dir.path().join("c.json"));
    assert_eq!(file.cases[0].checksum, Some(2.));
    assert_eq!(file.cases[1].error_kind, ErrorKind::Unsupported);
    assert!(
        file.cases[1]
            .error
            .as_deref()
            .unwrap()
            .contains("no_such_function")
    );
}

/// Words that must never reach the public text.
const PRIVATE: [&str; 5] = ["secret", "alice", "/home", "matlab", "undefined"];

fn private_result(dir: &Path) -> PathBuf {
    std::fs::write(
        dir.join("cases.csv"),
        "id,setup,op,check\nsecret-name,p='/home/alice/data';,y=numel(p);,y\n\
         secret-fail,,y=secret_fn('/home/alice');,y\n",
    )
    .unwrap();
    let output = matrust(
        dir,
        &["--cases", "cases.csv", "--threads", "1", "--out", "r.json"],
    );
    assert_eq!(output.status.code(), Some(1), "{}", describe(&output));
    // As an older version wrote it with MATLAB results: sharing must still
    // leave that part out.
    let path = dir.join("r.json");
    let mut json: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(&path).unwrap()).unwrap();
    json["matlab"] = old_matlab_part("secret-name");
    std::fs::write(&path, serde_json::to_string_pretty(&json).unwrap()).unwrap();
    let file = result(&path);
    assert!(
        file.cases[1]
            .error
            .as_deref()
            .unwrap()
            .contains("secret_fn")
    );
    path
}

/// The `matlab` part of a result file from a version that still ran MATLAB.
fn old_matlab_part(id: &str) -> serde_json::Value {
    serde_json::json!({
        "version": "reference",
        "threads": 1,
        "cases": [{
            "id": id,
            "runs": 7,
            "median_ms": 1.0,
            "min_ms": 0.9,
            "max_ms": 1.1,
            "checksum": 16.0,
            "error_kind": "none",
            "results_match": "yes",
            "ratio": "2.0x slower (ranges overlap)"
        }]
    })
}

#[test]
fn older_result_file_with_matlab_part_still_loads_and_shares() {
    let dir = tempfile::tempdir().unwrap();
    let builtin = &bench::builtin_cases()[0];
    let old = format!(
        r#"{{
  "format": 1,
  "matrust_version": "0.1.0",
  "commit": null,
  "timestamp": "2026-10-26T14:03:12Z",
  "threads": 4,
  "machine": {{
    "os": "Ubuntu",
    "os_version": "24.04",
    "arch": "x86_64",
    "cpu": "Test CPU",
    "logical_cores": 8,
    "ram_gib": 16
  }},
  "cases": [
    {{
      "id": "{id}",
      "source": "builtin",
      "sha256": "{sha}",
      "runs": 5,
      "median_ms": 2.5,
      "min_ms": 2.0,
      "max_ms": 3.0,
      "checksum": 1.5,
      "error_kind": "none",
      "error": null
    }}
  ],
  "matlab": {matlab}
}}
"#,
        id = builtin.id,
        sha = builtin.sha256,
        matlab = old_matlab_part(&builtin.id)
    );
    let path = dir.path().join("old.json");
    std::fs::write(&path, &old).unwrap();
    let file = result(&path);
    assert_eq!((file.format, file.threads, file.cases.len()), (1, 4, 1));
    assert_eq!(
        (file.cases[0].id.as_str(), file.cases[0].median_ms),
        (builtin.id.as_str(), Some(2.5))
    );
    let rewritten = serde_json::to_string(&file).unwrap();
    assert!(!rewritten.contains("matlab"), "{rewritten}");
    let output = matrust(dir.path(), &["--share", "old.json", "--no-open"]);
    assert_eq!(output.status.code(), Some(0), "{}", describe(&output));
    let saved = std::fs::read_to_string(dir.path().join("old.json.share.md")).unwrap();
    assert!(
        saved.contains(&format!("| {} | builtin | 5 |", builtin.id)),
        "{saved}"
    );
    for word in ["matlab", "reference", "slower"] {
        assert!(
            !saved.to_lowercase().contains(word),
            "share text holds {word}"
        );
    }
}

/// A fake browser opener that records its arguments.
#[cfg(unix)]
fn fake_openers(dir: &Path) -> (PathBuf, std::ffi::OsString) {
    use std::os::unix::fs::PermissionsExt;
    let bin = dir.join("bin");
    std::fs::create_dir(&bin).unwrap();
    let marker = dir.join("opened.txt");
    for name in ["xdg-open", "open"] {
        let script = bin.join(name);
        std::fs::write(
            &script,
            format!(
                "#!/bin/sh\nprintf '%s\\n' \"$@\" > '{}'\n",
                marker.display()
            ),
        )
        .unwrap();
        std::fs::set_permissions(&script, std::fs::Permissions::from_mode(0o755)).unwrap();
    }
    let mut path = bin.into_os_string();
    path.push(":");
    path.push(std::env::var_os("PATH").unwrap_or_default());
    (marker, path)
}

#[test]
fn share_no_open_prints_payload_and_url_and_spawns_nothing() {
    let dir = tempfile::tempdir().unwrap();
    let file = private_result(dir.path());
    let mut command = Command::new(env!("CARGO_BIN_EXE_matrust"));
    command
        .args(["bench", "--share", "r.json", "--no-open"])
        .current_dir(dir.path())
        .stdin(Stdio::null());
    #[cfg(unix)]
    let (marker, path) = fake_openers(dir.path());
    #[cfg(unix)]
    command.env("PATH", &path);
    let output = command.output().unwrap();
    assert_eq!(output.status.code(), Some(0), "{}", describe(&output));
    #[cfg(unix)]
    assert!(!marker.exists(), "--no-open started a program");
    let text = stdout(&output);
    let saved = std::fs::read_to_string(dir.path().join("r.json.share.md")).unwrap();
    let (body, rest) = text
        .split_once(bench::share::NOTICE)
        .expect("notice printed");
    assert_eq!(body.trim_end(), saved.trim_end());
    let url = bench::share::issue_url();
    assert!(rest.contains(&format!("Issue form: {url}\n")), "{rest}");
    assert!(url.ends_with("/issues/new?template=benchmark.yml"));
    for word in PRIVATE {
        assert!(
            !body.to_lowercase().contains(word),
            "body holds {word}:\n{body}"
        );
        assert!(
            !saved.to_lowercase().contains(word),
            "saved text holds {word}"
        );
    }
    assert!(body.contains("| user-1 | user |"), "{body}");
    assert!(body.contains("| user-2 | user |"), "{body}");
    assert!(body.contains("| unsupported |"), "{body}");
    let json = body
        .split("```json\n")
        .nth(1)
        .unwrap()
        .split("\n```")
        .next()
        .unwrap();
    let payload: serde_json::Value = serde_json::from_str(json).unwrap();
    let local = result(&file);
    assert_eq!(
        payload["cases"][0]["sha256"],
        local.cases[0].sha256.as_str()
    );
    assert_eq!(payload["cases"][1]["error_kind"], "unsupported");
    assert!(payload.get("matlab").is_none() && payload["cases"][0].get("checksum").is_none());
}

#[cfg(unix)]
#[test]
fn share_opens_the_form_only_on_yes() {
    let dir = tempfile::tempdir().unwrap();
    private_result(dir.path());
    let (marker, path) = fake_openers(dir.path());
    let share = |answer: &str| {
        let mut child = Command::new(env!("CARGO_BIN_EXE_matrust"))
            .args(["bench", "--share", "r.json"])
            .current_dir(dir.path())
            .env("PATH", &path)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .spawn()
            .unwrap();
        use std::io::Write;
        child
            .stdin
            .take()
            .unwrap()
            .write_all(answer.as_bytes())
            .unwrap();
        child.wait_with_output().unwrap()
    };
    let output = share("n\n");
    assert_eq!(output.status.code(), Some(0), "{}", describe(&output));
    assert!(stdout(&output).contains("Open the browser? [y/N] "));
    assert!(!marker.exists());
    let output = share("y\n");
    assert_eq!(output.status.code(), Some(0), "{}", describe(&output));
    let opened = std::fs::read_to_string(&marker).unwrap();
    assert_eq!(opened.trim(), bench::share::issue_url());
}

#[test]
fn usage_errors_exit_2() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(dir.path().join("a.m"), "y = 1;\n").unwrap();
    for args in [
        &["--fast"][..],
        &["--threads", "0"],
        &["--script", "a.m", "--path", "."],
        &["--cases", "c.csv", "--op", "y=1;"],
        &["--share", "r.json", "--threads", "2"],
        &["--no-open"],
        &["--cases", "missing.csv"],
        &["--only", "no_such_case"],
        &["--op", "y = 1"],
        &["--share", "missing.json", "--no-open"],
    ] {
        let output = matrust(dir.path(), args);
        assert_eq!(
            output.status.code(),
            Some(2),
            "{args:?}\n{}",
            describe(&output)
        );
    }
    let help = matrust(dir.path(), &["--help"]);
    assert_eq!(help.status.code(), Some(0));
    assert!(stdout(&help).contains("--share FILE.json"));
    assert!(!stdout(&help).to_lowercase().contains("matlab"));
    let top = Command::new(env!("CARGO_BIN_EXE_matrust"))
        .arg("--help")
        .output()
        .unwrap();
    assert!(stdout(&top).contains("matrust bench"));
}

#[test]
fn invalid_output_is_rejected_before_user_code_runs() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(dir.path().join("blocker"), "unchanged").unwrap();
    for out in ["missing/result.json", "blocker/result.json", "."] {
        let output = matrust(
            dir.path(),
            &[
                "--op",
                "fid=fopen('executed','w'); fclose(fid);",
                "--threads",
                "1",
                "--out",
                out,
            ],
        );
        assert_eq!(output.status.code(), Some(2), "{}", describe(&output));
        assert!(!dir.path().join("executed").exists());
        assert!(!String::from_utf8_lossy(&output.stderr).contains("MatRust 1/"));
    }
    assert_eq!(
        std::fs::read_to_string(dir.path().join("blocker")).unwrap(),
        "unchanged"
    );
    assert_eq!(std::fs::read_dir(dir.path()).unwrap().count(), 1);
}

#[cfg(unix)]
#[test]
fn unwritable_output_parent_is_usage_error() {
    use std::os::unix::fs::PermissionsExt;
    let dir = tempfile::tempdir().unwrap();
    let parent = dir.path().join("readonly");
    std::fs::create_dir(&parent).unwrap();
    std::fs::set_permissions(&parent, std::fs::Permissions::from_mode(0o555)).unwrap();
    // Elevated users can write through mode bits; run this check unprivileged.
    let can_write = tempfile::tempfile_in(&parent).is_ok();
    let output = matrust(
        dir.path(),
        &[
            "--op",
            "fid=fopen('executed','w'); fclose(fid);",
            "--threads",
            "1",
            "--out",
            "readonly/result.json",
        ],
    );
    std::fs::set_permissions(&parent, std::fs::Permissions::from_mode(0o755)).unwrap();
    if can_write {
        return;
    }
    assert_eq!(output.status.code(), Some(2), "{}", describe(&output));
    assert!(!dir.path().join("executed").exists());
    assert_eq!(std::fs::read_dir(&parent).unwrap().count(), 0);
}

#[test]
fn script_runs_selected_m_even_beside_mr() {
    let dir = tempfile::tempdir().unwrap();
    let source = "assert(strcmp(mfilename, 'chosen')); y = helper_add(2,3); assert(y == 5);\n";
    std::fs::write(dir.path().join("chosen.m"), source).unwrap();
    std::fs::write(dir.path().join("chosen.mr"), "error('wrong script');\n").unwrap();
    std::fs::write(
        dir.path().join("helper_add.m"),
        "function y=helper_add(a,b)\ny=a+b;\nend\n",
    )
    .unwrap();
    let output = matrust(
        dir.path(),
        &["--script", "chosen.m", "--threads", "1", "--out", "r.json"],
    );
    assert_eq!(output.status.code(), Some(0), "{}", describe(&output));
    let file = result(&dir.path().join("r.json"));
    assert_eq!(file.cases[0].sha256, sha256_hex(source.as_bytes()));
    assert!(file.cases[0].runs >= 5);
}

#[test]
fn clearing_scripts_preserve_matrust_runner_state() {
    let dir = tempfile::tempdir().unwrap();
    for clear in ["clear", "clearvars", "clear all"] {
        std::fs::write(
            dir.path().join("clearing.m"),
            format!("{clear}; y=42; assert(y==42);\n"),
        )
        .unwrap();
        let output = matrust(
            dir.path(),
            &[
                "--script",
                "clearing.m",
                "--threads",
                "1",
                "--out",
                "r.json",
            ],
        );
        let file = result(&dir.path().join("r.json"));
        if clear == "clear all" {
            // The engine explicitly rejects cache/global clearing; the bench
            // must preserve that limitation, not silently rewrite user code.
            assert_eq!(output.status.code(), Some(1), "{}", describe(&output));
            assert_eq!(file.cases[0].error_kind, ErrorKind::Unsupported);
            assert!(
                file.cases[0]
                    .error
                    .as_ref()
                    .unwrap()
                    .contains("clear all is unsupported")
            );
        } else {
            assert_eq!(output.status.code(), Some(0), "{}", describe(&output));
            assert_eq!(file.cases[0].error_kind, ErrorKind::None);
            assert!(file.cases[0].runs >= 5);
        }
    }
}

#[test]
fn matlab_options_are_unknown() {
    let dir = tempfile::tempdir().unwrap();
    for args in [
        &["--matlab", "auto"][..],
        &["--matlab-timeout", "60"],
        &["--import-matlab", "m.csv"],
    ] {
        let output = matrust(dir.path(), args);
        assert_eq!(output.status.code(), Some(2), "{}", describe(&output));
        let stderr = String::from_utf8_lossy(&output.stderr);
        assert!(
            stderr.starts_with(&format!("Error: Unknown option {}\n", args[0])),
            "{stderr}"
        );
        assert!(stdout(&output).is_empty(), "{}", describe(&output));
    }
    assert_eq!(std::fs::read_dir(dir.path()).unwrap().count(), 0);
}
