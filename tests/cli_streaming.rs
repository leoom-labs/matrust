//! CLI output is flushed at completed-line boundaries, including from local functions.
use std::{
    io::{BufRead, BufReader, Read},
    process::{Child, Command, Stdio},
    sync::mpsc,
    thread,
    time::Duration,
};

fn script(source: &str) -> (tempfile::TempDir, std::path::PathBuf) {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("streaming.m");
    std::fs::write(&path, source).unwrap();
    (directory, path)
}

fn child(path: &std::path::Path) -> Child {
    Command::new(env!("CARGO_BIN_EXE_matrust"))
        .arg(path)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap()
}

fn output(child: Child) -> (std::process::ExitStatus, String, String) {
    let result = child.wait_with_output().unwrap();
    (
        result.status,
        String::from_utf8(result.stdout).unwrap(),
        String::from_utf8(result.stderr).unwrap(),
    )
}

#[test]
fn completed_lines_reach_stdout_before_a_long_script_finishes() {
    let (_directory, path) =
        script("fprintf('first\\n');\nt = tic;\nwhile toc(t) < 1.5, end\nfprintf('second\\n');\n");
    let mut child = child(&path);
    let stdout = child.stdout.take().unwrap();
    let (sent, received) = mpsc::channel();
    let reader = thread::spawn(move || {
        let mut stdout = BufReader::new(stdout);
        let mut first = String::new();
        stdout.read_line(&mut first).unwrap();
        sent.send(first).unwrap();
        let mut rest = String::new();
        stdout.read_to_string(&mut rest).unwrap();
        rest
    });

    assert_eq!(
        received.recv_timeout(Duration::from_secs(1)).unwrap(),
        "first\n"
    );
    let status = child.wait().unwrap();
    let mut stderr = String::new();
    child
        .stderr
        .take()
        .unwrap()
        .read_to_string(&mut stderr)
        .unwrap();
    assert!(status.success(), "{stderr}");
    assert_eq!(reader.join().unwrap(), "second\n");
}

#[test]
fn streamed_output_is_not_repeated_after_an_error() {
    let (_directory, path) =
        script("fprintf('one\\n'); fprintf('two\\n'); fprintf('three\\n'); error('broken');\n");
    let (status, stdout, stderr) = output(child(&path));
    assert!(!status.success(), "{stdout}");
    assert_eq!(stdout, "one\ntwo\nthree\n");
    assert!(stderr.contains("broken"), "{stderr}");
}

#[test]
fn evalc_keeps_captured_output_off_stdout() {
    let (_directory, path) =
        script("captured = evalc('fprintf(''hidden\\n'')');\nfprintf('captured:%s', captured);\n");
    let (status, stdout, stderr) = output(child(&path));
    assert!(status.success(), "{stderr}");
    assert_eq!(stdout, "captured:hidden\n");
}

#[test]
fn local_function_output_streams_before_the_function_returns() {
    let (_directory, path) = script(
        "work();\nfunction work()\nfprintf('first\\n');\nt = tic;\nwhile toc(t) < 1.5, end\nfprintf('second\\n');\nend\n",
    );
    let mut child = child(&path);
    let stdout = child.stdout.take().unwrap();
    let (sent, received) = mpsc::channel();
    let reader = thread::spawn(move || {
        let mut stdout = BufReader::new(stdout);
        let mut first = String::new();
        stdout.read_line(&mut first).unwrap();
        sent.send(first).unwrap();
        let mut rest = String::new();
        stdout.read_to_string(&mut rest).unwrap();
        rest
    });

    assert_eq!(
        received.recv_timeout(Duration::from_secs(1)).unwrap(),
        "first\n"
    );
    let status = child.wait().unwrap();
    let mut stderr = String::new();
    child
        .stderr
        .take()
        .unwrap()
        .read_to_string(&mut stderr)
        .unwrap();
    assert!(status.success(), "{stderr}");
    assert_eq!(reader.join().unwrap(), "second\n");
}

#[test]
fn held_fprintf_text_joins_the_next_completed_line() {
    let (_directory, path) = script("fprintf('a'); fprintf('b\\n');\n");
    let (status, stdout, stderr) = output(child(&path));
    assert!(status.success(), "{stderr}");
    assert_eq!(stdout, "ab\n");
}

#[test]
fn closed_stdout_stops_the_run_quietly() {
    let (_directory, path) = script(
        "fprintf('first\\n');\nfor i = 1:1000000, fprintf('more\\n'); end\nerror('should not reach');\n",
    );
    let mut child = child(&path);
    let mut reader = BufReader::new(child.stdout.take().unwrap());
    let mut first = String::new();
    reader.read_line(&mut first).unwrap();
    assert_eq!(first, "first\n");
    drop(reader);
    let deadline = std::time::Instant::now() + Duration::from_secs(5);
    loop {
        if child.try_wait().unwrap().is_some() {
            break;
        }
        if std::time::Instant::now() >= deadline {
            child.kill().unwrap();
            child.wait().unwrap();
            panic!("CLI did not stop after its stdout reader closed");
        }
        thread::sleep(Duration::from_millis(10));
    }
    let (status, _, stderr) = output(child);
    assert!(!status.success());
    assert!(stderr.is_empty(), "{stderr}");
}

#[cfg(target_os = "linux")]
#[test]
fn full_stdout_reports_the_write_error_and_stops() {
    let result = Command::new(env!("CARGO_BIN_EXE_matrust"))
        .args(["--eval", "disp(1); error('should not reach');"])
        .stdin(Stdio::null())
        .stdout(
            std::fs::File::options()
                .write(true)
                .open("/dev/full")
                .unwrap(),
        )
        .output()
        .unwrap();
    assert!(!result.status.success());
    let stderr = String::from_utf8(result.stderr).unwrap();
    assert!(
        stderr.starts_with("matrust: cannot write to standard output: "),
        "{stderr}"
    );
    assert_eq!(stderr.lines().count(), 1, "{stderr}");
    assert!(!stderr.contains("should not reach"), "{stderr}");
}
