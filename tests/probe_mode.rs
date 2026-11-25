//! Diagnostic `MATRUST_PROBE=1` mode: logs catchable statement errors and continues.

use std::{
    fs,
    path::Path,
    process::{Command, Output, Stdio},
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
};

fn run_batch(cwd: &Path, args: &[&str], probe: bool) -> (Option<i32>, String, String) {
    let mut cmd = Command::new(env!("CARGO_BIN_EXE_matrust"));
    cmd.args(args)
        .current_dir(cwd)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    if probe {
        cmd.env("MATRUST_PROBE", "1");
    } else {
        cmd.env_remove("MATRUST_PROBE");
    }
    let Output {
        status,
        stdout,
        stderr,
    } = cmd.output().unwrap();
    (
        status.code(),
        String::from_utf8_lossy(&stdout).replace("\r\n", "\n"),
        String::from_utf8_lossy(&stderr).replace("\r\n", "\n"),
    )
}

fn probe_lines(stderr: &str) -> Vec<&str> {
    stderr
        .lines()
        .filter(|line| line.starts_with("MATRUST-PROBE: "))
        .collect()
}

#[test]
fn logs_three_failing_statements_and_prints_later_disp() {
    let dir = tempfile::tempdir().unwrap();
    let script = dir.path().join("three_errors.m");
    fs::write(
        &script,
        "missing_one();\nmissing_two();\nmissing_three();\ndisp('after_errors');\n",
    )
    .unwrap();

    let (code, stdout, stderr) = run_batch(dir.path(), &["three_errors.m"], true);
    assert_eq!(code, Some(0), "stderr: {stderr}");
    assert_eq!(stdout, "after_errors\n");
    let lines = probe_lines(&stderr);
    assert_eq!(lines.len(), 3, "stderr: {stderr}");
    assert!(lines[0].contains("three_errors.m:1:"), "{}", lines[0]);
    assert!(lines[1].contains("three_errors.m:2:"), "{}", lines[1]);
    assert!(lines[2].contains("three_errors.m:3:"), "{}", lines[2]);
}

#[test]
fn without_env_var_stops_at_first_error() {
    let dir = tempfile::tempdir().unwrap();
    let code_text = "missing_one(); missing_two(); disp('after_errors');";

    let (code, stdout, stderr) = run_batch(dir.path(), &["-batch", code_text], false);
    assert_eq!(code, Some(1), "stderr: {stderr}");
    assert_eq!(stdout, "");
    assert!(probe_lines(&stderr).is_empty(), "stderr: {stderr}");
    assert!(stderr.contains("missing_one"), "stderr: {stderr}");
}

#[test]
fn error_inside_try_reaches_catch_without_probe_line() {
    let dir = tempfile::tempdir().unwrap();
    let script = dir.path().join("try_test.m");
    fs::write(
            &script,
            "try\n    missing_inside_try();\n    disp('not_reached');\ncatch err\n    disp(['caught:' err.message]);\nend\ndisp('after_try');\n",
        )
        .unwrap();

    let (code, stdout, stderr) = run_batch(dir.path(), &["try_test.m"], true);
    assert_eq!(code, Some(0), "stderr: {stderr}");
    assert!(probe_lines(&stderr).is_empty(), "stderr: {stderr}");
    assert!(stdout.contains("caught:"), "stdout: {stdout}");
    assert!(stdout.contains("after_try\n"), "stdout: {stdout}");
    assert!(!stdout.contains("not_reached"), "stdout: {stdout}");
}

#[test]
fn error_in_function_file_continues_in_that_function() {
    let dir = tempfile::tempdir().unwrap();
    fs::write(
        dir.path().join("helper_fn.m"),
        "function y = helper_fn()\n    missing_in_fn();\n    y = 42;\nend\n",
    )
    .unwrap();
    fs::write(
        dir.path().join("caller.m"),
        "res = helper_fn();\ndisp(res);\n",
    )
    .unwrap();

    let (code, stdout, stderr) = run_batch(dir.path(), &["caller.m"], true);
    assert_eq!(code, Some(0), "stderr: {stderr}");
    assert_eq!(stdout.trim(), "42");
    let lines = probe_lines(&stderr);
    assert_eq!(lines.len(), 1, "stderr: {stderr}");
    assert!(lines[0].contains("helper_fn.m:2:"), "{}", lines[0]);
    assert!(lines[0].contains("missing_in_fn"), "{}", lines[0]);
}

#[test]
fn if_condition_error_unwinds() {
    let dir = tempfile::tempdir().unwrap();
    let script = dir.path().join("if_cond.m");
    fs::write(
        &script,
        "missing_stmt();\nif missing_cond\n    disp('inside');\nend\ndisp('after_if');\n",
    )
    .unwrap();

    let (code, stdout, stderr) = run_batch(dir.path(), &["if_cond.m"], true);
    assert_eq!(code, Some(1), "stderr: {stderr}");
    assert_eq!(stdout, "");
    let lines = probe_lines(&stderr);
    assert_eq!(lines.len(), 1, "stderr: {stderr}");
    assert!(lines[0].contains("if_cond.m:1:"), "{}", lines[0]);
    assert!(stderr.contains("missing_cond"), "stderr: {stderr}");
}

#[test]
fn caps_at_200_probe_lines_then_unwinds() {
    let dir = tempfile::tempdir().unwrap();
    let mut src = String::new();
    for i in 1..=205 {
        src.push_str(&format!("missing_{i}();\n"));
    }
    src.push_str("disp('unreachable');\n");
    let script = dir.path().join("cap.m");
    fs::write(&script, src).unwrap();

    let (code, stdout, stderr) = run_batch(dir.path(), &["cap.m"], true);
    assert_eq!(code, Some(1), "stderr: {stderr}");
    assert_eq!(stdout, "");
    let lines = probe_lines(&stderr);
    assert_eq!(lines.len(), 200, "stderr: {stderr}");
    assert!(stderr.contains("missing_201"), "stderr: {stderr}");
}

#[test]
fn called_function_control_errors_unwind_through_caller() {
    let dir = tempfile::tempdir().unwrap();
    for (name, control) in [
        ("if", "if missing_cond\nend"),
        ("elseif", "if false\nelseif missing_cond\nend"),
        ("while", "while missing_cond\nend"),
        ("for", "for k = 1:missing_cond\nend"),
        ("switch", "switch missing_cond\nend"),
    ] {
        fs::write(
            dir.path().join("helper_fn.m"),
            format!("function y = helper_fn()\n{control}\ny = 1;\nend\n"),
        )
        .unwrap();
        fs::write(
            dir.path().join("caller.m"),
            "helper_fn();\ndisp('later');\n",
        )
        .unwrap();
        let (code, stdout, stderr) = run_batch(dir.path(), &["caller.m"], true);
        assert_eq!(code, Some(1), "{name}: {stderr}");
        assert!(probe_lines(&stderr).is_empty(), "{name}: {stderr}");
        assert!(!stdout.contains("later"), "{name}: {stdout}");
        assert!(stderr.contains("missing_cond"), "{name}: {stderr}");
    }
}

#[test]
fn caught_control_error_allows_later_probe_recovery() {
    let dir = tempfile::tempdir().unwrap();
    fs::write(
        dir.path().join("helper_fn.m"),
        "function y = helper_fn()\nif missing_cond\nend\ny = 1;\nend\n",
    )
    .unwrap();
    fs::write(dir.path().join("caller.m"), "try\nhelper_fn();\ncatch err\ndisp(['caught:' err.message]);\nend\nmissing_later();\ndisp('after');\n").unwrap();
    let (code, stdout, stderr) = run_batch(dir.path(), &["caller.m"], true);
    assert_eq!(code, Some(0), "{stderr}");
    assert!(stdout.contains("caught:"), "{stdout}");
    assert!(stdout.contains("after"), "{stdout}");
    let lines = probe_lines(&stderr);
    assert_eq!(lines.len(), 1, "{stderr}");
    assert!(lines[0].contains("missing_later"), "{stderr}");
}

#[test]
fn temporary_cleanup_step_limit_is_not_a_recovered_pass() {
    let dir = tempfile::tempdir().unwrap();
    fs::write(dir.path().join("bare.m"), "missing_stmt();\n").unwrap();
    let (code, stdout, stderr) = run_batch(dir.path(), &["--max-steps", "1", "bare.m"], true);
    assert_eq!(code, Some(0), "bare recovery: {stderr}");
    assert_eq!(stdout, "");
    assert_eq!(probe_lines(&stderr).len(), 1, "{stderr}");

    // The statement uses the only step. Sweeping its temporary onCleanup
    // charges the next step and must unwind, not log a probe line or pass.
    fs::write(
        dir.path().join("cleanup.m"),
        "missing_stmt(onCleanup(@() 1));\n",
    )
    .unwrap();
    let (code, stdout, stderr) = run_batch(dir.path(), &["--max-steps", "1", "cleanup.m"], true);
    assert_eq!(code, Some(1), "{stderr}");
    assert_eq!(stdout, "");
    assert!(
        probe_lines(&stderr).is_empty(),
        "step limit must not be logged as recoverable: {stderr}"
    );
    assert!(stderr.contains("1 statement/loop steps"), "{stderr}");
}

/// Host that cancels the run when cleanup prints. The flag flips only inside
/// the sweep, after the failing statement has already been judged catchable.
struct CancelOnOutput(Arc<AtomicBool>);

impl matrust::engine::HostIo for CancelOnOutput {
    fn read_line(&mut self, _prompt: &str) -> Option<String> {
        None
    }
    fn streams_output(&self) -> bool {
        true
    }
    fn show_output(&mut self, lines: &[String], _clear: bool) -> bool {
        if lines.iter().any(|line| line.contains('x')) {
            self.0.store(true, Ordering::Relaxed);
        }
        false
    }
}

fn cleanup_cancellation_child() {
    let mut engine = matrust::Engine::default();
    let cancel = engine.cancellation();
    engine.set_host_io(Some(Box::new(CancelOnOutput(cancel))));
    // The first temporary sets the cancel flag while it is swept. The next
    // temporary's step check must then fail the sweep and the run.
    let error = engine
        .execute("missing_stmt(onCleanup(@() disp('x')), onCleanup(@() 1));")
        .expect_err("cleanup cancellation must fail the run");
    assert!(error.contains("Calculation cancelled"), "{error}");
}

#[test]
fn temporary_cleanup_cancellation_is_not_a_recovered_pass() {
    // Own process: MATRUST_PROBE must not leak into the rest of this binary.
    if std::env::var_os("PROBE_CLEANUP_CHILD").is_some() {
        cleanup_cancellation_child();
        return;
    }
    let output = Command::new(std::env::current_exe().unwrap())
        .args([
            "--exact",
            "--test-threads=1",
            "probe_mode::temporary_cleanup_cancellation_is_not_a_recovered_pass",
        ])
        .env("PROBE_CLEANUP_CHILD", "1")
        .env("MATRUST_PROBE", "1")
        .output()
        .unwrap();
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        output.status.success(),
        "stdout:\n{}\nstderr:\n{stderr}",
        String::from_utf8_lossy(&output.stdout)
    );
    assert!(
        !stderr.contains("MATRUST-PROBE"),
        "cancellation must not be logged as recoverable: {stderr}"
    );
}
