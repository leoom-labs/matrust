//! The command-line contract for automated runs: exit status, `-batch`,
//! `exit`/`quit` and which stream carries what. Each test checks stdout,
//! stderr and the exit status separately.
//!
//! | status | meaning                                                          |
//! |--------|------------------------------------------------------------------|
//! | 0      | success, or `exit`/`quit` without a status                       |
//! | 1      | script error (parse, runtime, --max-steps stop, first Ctrl+C)    |
//! | 2      | usage error (unknown flag, missing value, unreadable script, two sources) |
//! | n      | `exit(n)` / `quit(n)`, n an integer 0..255                        |
//! | 130    | second Ctrl+C within 2 s (tests/host_limits.rs)                  |
use std::{
    io::Write,
    path::PathBuf,
    process::{Command, Output, Stdio},
};

fn fixtures() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/cli")
}

fn run_with(args: &[&str], stdin: Option<&str>) -> Output {
    let mut child = Command::new(env!("CARGO_BIN_EXE_matrust"))
        .args(args)
        .current_dir(fixtures())
        .stdin(if stdin.is_some() {
            Stdio::piped()
        } else {
            Stdio::null()
        })
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    if let Some(text) = stdin {
        let mut pipe = child.stdin.take().unwrap();
        pipe.write_all(text.as_bytes()).unwrap();
    }
    child.wait_with_output().unwrap()
}

fn run(args: &[&str]) -> Output {
    run_with(args, None)
}

/// (status, stdout, stderr); stdout keeps its exact bytes, line ends normalized.
fn parts(output: &Output) -> (Option<i32>, String, String) {
    (
        output.status.code(),
        String::from_utf8_lossy(&output.stdout).replace("\r\n", "\n"),
        String::from_utf8_lossy(&output.stderr).replace("\r\n", "\n"),
    )
}

/// A usage error: status 2, nothing on stdout, one stderr line naming the problem
/// and pointing at --help.
fn assert_usage_error(args: &[&str], needle: &str) {
    let (code, stdout, stderr) = parts(&run(args));
    assert_eq!(code, Some(2), "{args:?}: stderr {stderr:?}");
    assert_eq!(stdout, "", "{args:?}");
    assert_eq!(stderr.lines().count(), 1, "{args:?}: {stderr:?}");
    assert!(stderr.contains(needle), "{args:?}: {stderr:?}");
    assert!(
        stderr.contains("see matrust --help"),
        "{args:?}: {stderr:?}"
    );
}

// ---- 1. exit status --------------------------------------------------------

#[test]
fn success_exits_0() {
    let (code, stdout, stderr) = parts(&run(&["-e", "disp('ok')"]));
    assert_eq!(code, Some(0));
    assert_eq!(stdout, "ok\n");
    assert_eq!(stderr, "");
}

#[test]
fn runtime_error_exits_1_with_message_on_stderr_only() {
    let (code, stdout, stderr) = parts(&run(&["-e", "error('x')"]));
    assert_eq!(code, Some(1));
    assert_eq!(stdout, "");
    // The existing text: "Error: Line 1, column 1: x".
    assert!(stderr.starts_with("Error: "), "{stderr:?}");
    assert!(stderr.ends_with(": x\n"), "{stderr:?}");
}

#[test]
fn parse_error_exits_1_and_runs_nothing() {
    let (code, stdout, stderr) = parts(&run(&["parse_error.m"]));
    assert_eq!(code, Some(1));
    assert_eq!(stdout, "");
    assert!(stderr.starts_with("Error: "), "{stderr:?}");
}

#[test]
fn max_steps_stop_exits_1() {
    let (code, stdout, stderr) = parts(&run(&[
        "--max-steps",
        "100",
        "-e",
        "disp('go'); while true, end",
    ]));
    assert_eq!(code, Some(1));
    assert_eq!(stdout, "go\n");
    assert!(stderr.contains("step"), "{stderr:?}");
}

#[test]
fn unknown_flag_exits_2() {
    assert_usage_error(&["--nope"], "Unknown option --nope");
}

#[test]
fn missing_flag_values_exit_2() {
    assert_usage_error(&["--eval"], "--eval requires");
    assert_usage_error(&["-batch"], "-batch requires");
    assert_usage_error(&["--threads"], "--threads requires");
    assert_usage_error(&["--max-steps"], "--max-steps requires");
    assert_usage_error(&["--svg"], "--svg requires");
    assert_usage_error(&["--threads", "many"], "Invalid thread count");
}

#[test]
fn missing_script_exits_2_with_its_existing_message() {
    let (code, stdout, stderr) = parts(&run(&["missing.m"]));
    assert_eq!(code, Some(2));
    assert_eq!(stdout, "");
    assert_eq!(stderr.lines().count(), 1, "{stderr:?}");
    assert!(stderr.starts_with("Error: "), "{stderr:?}");
}

#[test]
fn two_sources_exit_2() {
    for args in [
        &["-e", "disp(1)", "-e", "disp(2)"][..],
        &["batch_script.m", "-e", "disp(2)"],
        &["-e", "disp(2)", "batch_script.m"],
        &["-batch", "disp(1)", "batch_script.m"],
        &["-batch", "disp(1)", "--eval", "disp(2)"],
    ] {
        assert_usage_error(args, "Supply one script");
    }
}

#[test]
fn help_documents_the_exit_statuses_and_batch() {
    let (code, stdout, stderr) = parts(&run(&["--help"]));
    assert_eq!(code, Some(0));
    assert_eq!(stderr, "");
    for needle in ["-batch", "Exit status", "130", "exit(n)"] {
        assert!(stdout.contains(needle), "--help lacks {needle}");
    }
}

// ---- 2. -batch -------------------------------------------------------------

#[test]
fn batch_runs_a_statement_like_eval() {
    for flag in ["-batch", "--batch"] {
        let (code, stdout, stderr) = parts(&run(&[flag, "x = 2 + 3; disp(x)"]));
        assert_eq!(code, Some(0), "{flag}: {stderr}");
        // disp of a number prints its existing padded form.
        assert_eq!(stdout.trim(), "5", "{flag}");
        assert_eq!(stderr, "", "{flag}");
    }
}

#[test]
fn batch_never_reads_the_console() {
    // Lines on stdin would run if the REPL started after the statement.
    let output = run_with(&["-batch", "disp('done')"], Some("disp('repl')\n"));
    let (code, stdout, stderr) = parts(&output);
    assert_eq!(code, Some(0));
    assert_eq!(stdout, "done\n");
    assert_eq!(stderr, "");
}

#[test]
fn batch_runs_a_script_from_the_current_folder_by_name() {
    let (code, stdout, stderr) = parts(&run(&["-batch", "batch_script"]));
    assert_eq!(code, Some(0), "{stderr}");
    assert_eq!(stdout, "from batch_script\n");
    assert_eq!(stderr, "");
}

#[test]
fn batch_error_exits_1() {
    let (code, stdout, stderr) = parts(&run(&["-batch", "disp('a'); error('boom')"]));
    assert_eq!(code, Some(1));
    assert_eq!(stdout, "a\n");
    assert!(stderr.starts_with("Error: "), "{stderr:?}");
    assert!(stderr.ends_with(": boom\n"), "{stderr:?}");
}

#[test]
fn batch_plot_opens_no_window_and_finishes() {
    let (code, stdout, stderr) = parts(&run(&["-batch", "plot(1:3); disp('plotted')"]));
    assert_eq!(code, Some(0), "{stderr}");
    assert_eq!(stdout, "plotted\n");
    assert_eq!(stderr, "");
}

// ---- 3. exit and quit ------------------------------------------------------

#[test]
fn exit_with_a_status_after_output() {
    let (code, stdout, stderr) = parts(&run(&["-batch", "disp(1); exit(3)"]));
    assert_eq!(code, Some(3));
    assert_eq!(stdout.trim(), "1");
    assert_eq!(stderr, "");
}

#[test]
fn quit_with_a_status_in_eval() {
    let (code, stdout, stderr) = parts(&run(&["-e", "disp('q'); quit(7); disp('no')"]));
    assert_eq!(code, Some(7));
    assert_eq!(stdout, "q\n");
    assert_eq!(stderr, "");
}

#[test]
fn bare_exit_and_quit_exit_0() {
    for source in ["exit", "quit", "exit()", "quit(0)", "exit(255)"] {
        let (code, stdout, stderr) = parts(&run(&["-e", source]));
        let expected = if source == "exit(255)" { 255 } else { 0 };
        assert_eq!(code, Some(expected), "{source}: {stderr}");
        assert_eq!(stdout, "", "{source}");
        assert_eq!(stderr, "", "{source}");
    }
}

#[test]
fn exit_in_a_script_stops_before_later_statements() {
    let (code, stdout, stderr) = parts(&run(&["exit_midway.m"]));
    assert_eq!(code, Some(0));
    assert_eq!(stdout, "before\n");
    assert_eq!(stderr, "");
}

#[test]
fn exit_in_a_function_is_not_caught_by_try() {
    let (code, stdout, stderr) = parts(&run(&["exit_in_function.m"]));
    assert_eq!(code, Some(5));
    assert_eq!(stdout, "one\n");
    assert_eq!(stderr, "");
}

#[test]
fn held_fprintf_text_is_flushed_before_exit() {
    let (code, stdout, stderr) = parts(&run(&["-e", "fprintf('partial'); exit(2)"]));
    assert_eq!(code, Some(2));
    assert_eq!(stdout, "partial\n");
    assert_eq!(stderr, "");
}

#[test]
fn invalid_exit_status_is_a_script_error() {
    for source in [
        "exit(256)",
        "exit(-1)",
        "quit(1.5)",
        "exit('x')",
        "exit(1, 2)",
    ] {
        let (code, stdout, stderr) = parts(&run(&["-e", &format!("disp('a'); {source}")]));
        assert_eq!(code, Some(1), "{source}: {stderr}");
        assert_eq!(stdout, "a\n", "{source}");
        assert!(stderr.starts_with("Error: "), "{source}: {stderr:?}");
        assert!(stderr.contains("0 to 255"), "{source}: {stderr:?}");
    }
}

#[test]
fn repl_keywords_and_builtin_exit() {
    // The keyword ends the console without running later lines.
    let output = run_with(&[], Some("disp('a')\nexit\ndisp('b')\n"));
    let (code, stdout, stderr) = parts(&output);
    assert_eq!(code, Some(0));
    assert_eq!(stdout, "a\n");
    assert_eq!(stderr, "");
    // quit(n) on a console line is the builtin and sets the status.
    let output = run_with(&[], Some("disp('a')\nquit(4)\ndisp('b')\n"));
    let (code, stdout, stderr) = parts(&output);
    assert_eq!(code, Some(4));
    assert_eq!(stdout, "a\n");
    assert_eq!(stderr, "");
}

// ---- 4. streams ------------------------------------------------------------

#[test]
fn errors_go_to_stderr_and_earlier_output_stays_on_stdout() {
    let (code, stdout, stderr) = parts(&run(&[
        "-e",
        "disp('out'); fprintf(1, 'one\\n'); y = undefined_thing + 1",
    ]));
    assert_eq!(code, Some(1));
    assert_eq!(stdout, "out\none\n");
    assert!(stderr.starts_with("Error: "), "{stderr:?}");
    assert!(stderr.contains("undefined_thing"), "{stderr:?}");
}

#[test]
fn fprintf_to_file_id_2_writes_stderr() {
    let (code, stdout, stderr) = parts(&run(&[
        "-e",
        "fprintf('a\\n'); fprintf(2, 'problem %d\\n', 7); fprintf(1, 'b\\n')",
    ]));
    assert_eq!(code, Some(0));
    assert_eq!(stdout, "a\nb\n");
    assert_eq!(stderr, "problem 7\n");
}

#[test]
fn batch_root_default_axes_font_size() {
    let (code, stdout, stderr) = parts(&run(&[
        "-batch",
        "set(0,'DefaultAxesFontSize',14); ax=axes; disp(ax.FontSize)",
    ]));
    assert_eq!(code, Some(0), "stderr: {stderr}");
    assert_eq!(stdout.trim(), "14");
    assert_eq!(stderr, "");
}

#[test]
fn fix13_ascii_batch_load() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(dir.path().join("d.txt"), "1 2 3\n4 5 6\n").unwrap();
    let output = Command::new(env!("CARGO_BIN_EXE_matrust"))
        .current_dir(dir.path())
        .args(["-batch", "load d.txt; disp(sum(d(:)))"])
        .output()
        .unwrap();
    let (status, stdout, stderr) = parts(&output);
    assert_eq!(status, Some(0), "{stderr}");
    assert!(stdout.contains("21"), "{stdout}");
}

#[test]
fn batch_headless_waitbar_and_anonymous_nargin() {
    let (code, stdout, stderr) = parts(&run(&[
        "-batch",
        "h=waitbar(0); waitbar(1,h); close(h); disp(nargin(@(a,b)a))",
    ]));
    assert_eq!(code, Some(0), "{stderr}");
    assert_eq!(stdout.trim(), "2");
    assert_eq!(stderr, "");
}
