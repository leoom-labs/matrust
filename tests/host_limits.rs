//! Per-host run limits: the `matrust` CLI runs past the browser step limit, `--max-steps`
//! puts one back, and cancellation ends an endless loop under local limits.
use matrust::{Engine, engine::Limits};
use std::{
    process::{Command, Output, Stdio},
    sync::atomic::Ordering,
    time::{Duration, Instant},
};

fn matrust(args: &[&str]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_matrust"))
        .args(args)
        .stdin(Stdio::null())
        .output()
        .expect("run matrust")
}

fn describe(output: &Output) -> String {
    format!(
        "status {:?}\nstdout:\n{}\nstderr:\n{}",
        output.status.code(),
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    )
}

#[test]
fn cli_runs_past_the_browser_step_limit() {
    let output = matrust(&["--eval", "s=0; for i=1:1500000, s=s+i; end; disp(s)"]);
    assert!(output.status.success(), "{}", describe(&output));
    assert_eq!(
        String::from_utf8_lossy(&output.stdout).trim(),
        "1.1250e+12",
        "{}",
        describe(&output)
    );
}

#[test]
fn cli_max_steps_restores_a_step_limit() {
    let output = matrust(&["--max-steps", "1000", "--eval", "for i=1:2000, end"]);
    assert!(!output.status.success(), "{}", describe(&output));
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains("Execution exceeds 1000 statement/loop steps"),
        "{}",
        describe(&output)
    );
    let output = matrust(&["--max-steps", "5000", "--eval", "for i=1:2000, end"]);
    assert!(output.status.success(), "{}", describe(&output));
    let help = matrust(&["--help"]);
    assert!(
        String::from_utf8_lossy(&help.stdout).contains("--max-steps N"),
        "{}",
        describe(&help)
    );
    let output = matrust(&["--max-steps", "x", "--eval", "1"]);
    assert!(!output.status.success(), "{}", describe(&output));
}

#[test]
fn local_limits_stop_an_endless_loop_within_a_second_of_cancel() {
    let mut engine = Engine::with_limits(Limits::local());
    let cancel = engine.cancellation();
    let run = std::thread::spawn(move || {
        let error = engine.execute("while true, end").unwrap_err();
        (error, Instant::now())
    });
    std::thread::sleep(Duration::from_millis(500));
    assert!(!run.is_finished(), "the loop ended before cancel");
    let cancelled = Instant::now();
    cancel.store(true, Ordering::Relaxed);
    let (error, ended) = run.join().unwrap();
    assert!(error.ends_with("Calculation cancelled"), "{error}");
    assert!(ended.duration_since(cancelled) < Duration::from_secs(1));
}

#[test]
fn nd_display_respects_host_limit_across_pages() {
    for source in ["A=ones(1,1,2)", "A=ones(1,1,2); disp(A)"] {
        let mut roomy = Engine::with_limits(Limits {
            display_bytes: 1024,
            ..Limits::local()
        });
        roomy.execute(source).unwrap();
        let mut small = Engine::with_limits(Limits {
            display_bytes: 32,
            ..Limits::local()
        });
        let error = small.execute(source).unwrap_err();
        assert!(error.contains("Displayed value exceeds"), "{error}");
    }
}

#[test]
fn string_display_respects_host_limit() {
    for source in ["A=[\"alpha\" \"beta\"]", "A=[\"alpha\" \"beta\"]; disp(A)"] {
        let mut roomy = Engine::with_limits(Limits {
            display_bytes: 1024,
            ..Limits::local()
        });
        roomy.execute(source).unwrap();
        let mut small = Engine::with_limits(Limits {
            display_bytes: 8,
            ..Limits::local()
        });
        let error = small.execute(source).unwrap_err();
        assert!(
            error.contains("String display exceeds output limit"),
            "{error}"
        );
    }
}

/// Ctrl+C (SIGINT): the first cancels the run, a second within 2 s exits with 130.
#[cfg(unix)]
#[test]
fn cli_interrupt_cancels_then_exits() {
    fn interrupt(child: &std::process::Child) {
        let status = Command::new("kill")
            .args(["-INT", &child.id().to_string()])
            .status()
            .unwrap();
        assert!(status.success());
    }
    let spawn = || {
        Command::new(env!("CARGO_BIN_EXE_matrust"))
            .args(["--eval", "while true, end"])
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .unwrap()
    };
    let child = spawn();
    std::thread::sleep(Duration::from_millis(700));
    interrupt(&child);
    let output = child.wait_with_output().unwrap();
    assert_eq!(output.status.code(), Some(1), "{}", describe(&output));
    assert!(
        String::from_utf8_lossy(&output.stderr).contains("Calculation cancelled"),
        "{}",
        describe(&output)
    );

    // Two interrupts within 2 s quit the console that waits for input.
    let child = Command::new(env!("CARGO_BIN_EXE_matrust"))
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    std::thread::sleep(Duration::from_millis(700));
    interrupt(&child);
    std::thread::sleep(Duration::from_millis(100));
    interrupt(&child);
    // Closing stdin ends the console normally; give the handler time to exit first.
    std::thread::sleep(Duration::from_millis(500));
    let output = child.wait_with_output().unwrap();
    assert_eq!(output.status.code(), Some(130), "{}", describe(&output));
}
