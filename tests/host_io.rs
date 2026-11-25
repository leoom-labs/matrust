//! Console input through the engine's host hook: `input`, bare `pause`,
//! `system` and `display`, driven by a scripted host with queued answers.
use matrust::{Engine, HostIo};
use std::{
    collections::VecDeque,
    io::Write,
    process::{Command, Stdio},
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, Ordering},
    },
    thread,
    time::{Duration, Instant},
};

/// Answers from a queue; with `wait` set, an empty queue waits like a user
/// who has not answered yet, until the engine's cancel flag is set.
struct Scripted {
    answers: VecDeque<String>,
    prompts: Arc<Mutex<Vec<String>>>,
    wait: Option<Arc<AtomicBool>>,
}

impl HostIo for Scripted {
    fn read_line(&mut self, prompt: &str) -> Option<String> {
        self.prompts.lock().unwrap().push(prompt.to_owned());
        if let Some(answer) = self.answers.pop_front() {
            return Some(answer);
        }
        let cancel = self.wait.as_ref()?;
        while !cancel.load(Ordering::Relaxed) {
            thread::sleep(Duration::from_millis(5));
        }
        None
    }
}

fn engine(answers: &[&str]) -> (Engine, Arc<Mutex<Vec<String>>>) {
    let mut engine = Engine::default();
    let prompts = Arc::new(Mutex::new(Vec::new()));
    engine.set_host_io(Some(Box::new(Scripted {
        answers: answers.iter().map(|a| (*a).to_owned()).collect(),
        prompts: prompts.clone(),
        wait: None,
    })));
    (engine, prompts)
}

fn waiting_engine() -> Engine {
    let mut engine = Engine::default();
    let cancel = engine.cancellation();
    engine.set_host_io(Some(Box::new(Scripted {
        answers: VecDeque::new(),
        prompts: Arc::default(),
        wait: Some(cancel),
    })));
    engine
}

fn output(engine: &mut Engine, source: &str) -> String {
    engine
        .execute(source)
        .unwrap_or_else(|e| panic!("{source}: {e}"))
        .output
        .join("\n")
}

fn scalar(engine: &Engine, name: &str) -> f64 {
    engine.variables[name].as_scalar().unwrap()
}

#[test]
fn input_evaluates_the_answer_in_the_current_workspace() {
    let (mut e, prompts) = engine(&["a * 2 + 1"]);
    output(&mut e, "a = 4; x = input('n? ');");
    assert_eq!(scalar(&e, "x"), 9.);
    assert_eq!(*prompts.lock().unwrap(), ["n? "]);
}

#[test]
fn input_evaluates_matrices_and_shows_the_prompt_with_held_text() {
    let (mut e, prompts) = engine(&["[1 2; 3 4]"]);
    output(&mut e, "fprintf('Size: '); m = input('');");
    assert_eq!(e.variables["m"].data(), [1., 3., 2., 4.]);
    assert_eq!(*prompts.lock().unwrap(), ["Size: "]);
}

#[test]
fn input_with_s_returns_the_line_as_text() {
    let (mut e, _) = engine(&["1+1 is two"]);
    output(&mut e, "s = input('Name: ', 's');");
    assert_eq!(output(&mut e, "disp(ischar(s))").trim(), "1");
    assert_eq!(output(&mut e, "disp(s)"), "1+1 is two");
}

#[test]
fn input_empty_line_returns_empty() {
    let (mut e, _) = engine(&[""]);
    output(&mut e, "x = input('n? ');");
    assert_eq!(output(&mut e, "disp(isempty(x))").trim(), "1");
    assert_eq!(output(&mut e, "disp(class(x))"), "double");
}

#[test]
fn input_prints_the_error_and_asks_again_after_a_bad_answer() {
    let (mut e, prompts) = engine(&["no_such_name + 1", "1 +", "6 * 7"]);
    let text = output(&mut e, "x = input('n? ');");
    assert_eq!(scalar(&e, "x"), 42.);
    assert_eq!(*prompts.lock().unwrap(), ["n? ", "n? ", "n? "]);
    let errors: Vec<_> = text.lines().filter(|l| l.starts_with("Error")).collect();
    assert_eq!(errors.len(), 2, "{text}");
    assert!(errors[0].contains("no_such_name"), "{text}");
}

#[test]
fn input_without_an_answer_fails() {
    let (mut e, _) = engine(&[]);
    let error = e.execute("x = input('n? ');").unwrap_err();
    assert!(error.contains("no answer"), "{error}");
    // The default host (no host set) has no input either.
    let error = Engine::default().execute("x = input('n? ');").unwrap_err();
    assert!(error.contains("no answer"), "{error}");
}

#[test]
fn bare_pause_waits_for_one_answer_and_pause_off_skips_it() {
    let (mut e, prompts) = engine(&["anything"]);
    output(&mut e, "pause; x = 1;");
    assert_eq!(*prompts.lock().unwrap(), [""]);
    let (mut e, prompts) = engine(&[]);
    let start = Instant::now();
    output(&mut e, "pause('off'); pause; pause(5); s = pause('query');");
    assert!(start.elapsed() < Duration::from_secs(1));
    assert!(prompts.lock().unwrap().is_empty());
    assert_eq!(output(&mut e, "disp(s)"), "off");
}

#[test]
fn timed_pause_lasts_its_duration() {
    let mut e = Engine::default();
    let start = Instant::now();
    output(&mut e, "pause(0.1)");
    let elapsed = start.elapsed();
    assert!(elapsed >= Duration::from_millis(95), "{elapsed:?}");
    assert!(elapsed < Duration::from_millis(600), "{elapsed:?}");
}

/// Run `source` on a thread, set the cancel flag after 150 ms and return the
/// error and the time from the cancel to the end of the run.
fn cancel_during(mut e: Engine, source: impl Into<String>) -> (String, Duration) {
    let cancel = e.cancellation();
    let source = source.into();
    let run = thread::spawn(move || e.execute(&source));
    thread::sleep(Duration::from_millis(150));
    let cancelled = Instant::now();
    cancel.store(true, Ordering::Relaxed);
    let result = run.join().unwrap();
    (result.unwrap_err(), cancelled.elapsed())
}

#[test]
fn cancel_ends_a_waiting_input_promptly() {
    let (error, took) = cancel_during(waiting_engine(), "x = input('n? ');");
    assert!(error.contains("Calculation cancelled"), "{error}");
    assert!(took < Duration::from_millis(500), "{took:?}");
    let (error, took) = cancel_during(waiting_engine(), "pause");
    assert!(error.contains("Calculation cancelled"), "{error}");
    assert!(took < Duration::from_millis(500), "{took:?}");
}

#[test]
fn cancel_ends_a_long_pause_promptly() {
    let (error, took) = cancel_during(Engine::default(), "pause(10)");
    assert!(error.contains("Calculation cancelled"), "{error}");
    assert!(took < Duration::from_millis(500), "{took:?}");
}

#[test]
fn system_returns_status_and_output() {
    let mut e = Engine::default();
    output(&mut e, "[status, out] = system('echo hi');");
    assert_eq!(scalar(&e, "status"), 0.);
    // The returned text keeps the command's line ending.
    let text = output(&mut e, "disp(double(out))").replace("13", "");
    let codes: Vec<&str> = text.split_whitespace().collect();
    assert_eq!(codes, ["104", "105", "10"], "{text}");
    // With one output the command's output is printed as well.
    let printed = output(&mut e, "s = system('echo hi');");
    assert_eq!(printed.trim_end(), "hi");
    assert_eq!(scalar(&e, "s"), 0.);
    // A failing command reports its exit status and its error stream.
    output(
        &mut e,
        "[status, out] = system('echo oops 1>&2 && exit 3');",
    );
    assert_eq!(scalar(&e, "status"), 3.);
    assert!(output(&mut e, "disp(out)").contains("oops"));
}

#[test]
fn system_uses_the_engine_folder_and_environment() {
    let root = tempfile::tempdir().unwrap();
    let mut e = Engine::default();
    e.set_current_folder(root.path()).unwrap();
    let expected_folder = root.path().to_string_lossy().into_owned();
    let cwd_command = if cfg!(windows) { "cd" } else { "pwd" };
    output(&mut e, &format!("[status, out] = system('{cwd_command}');"));
    assert_eq!(scalar(&e, "status"), 0.);
    assert!(
        output(&mut e, "disp(out)").contains(&expected_folder),
        "system did not use the engine current folder"
    );

    let variable_command = if cfg!(windows) {
        "echo %MATRUST_T%"
    } else {
        "echo $MATRUST_T"
    };
    output(&mut e, "setenv('MATRUST_T', '42');");
    output(
        &mut e,
        &format!("[status, out] = system('{variable_command}');"),
    );
    assert_eq!(scalar(&e, "status"), 0.);
    assert_eq!(output(&mut e, "disp(out)").trim(), "42");

    let unset_command = if cfg!(windows) {
        "if defined MATRUST_T (echo set) else (echo unset)"
    } else {
        "if [ -z ${MATRUST_T+x} ]; then echo unset; else echo set; fi"
    };
    output(&mut e, "setenv('MATRUST_T');");
    output(
        &mut e,
        &format!("[status, out] = system('{unset_command}');"),
    );
    assert_eq!(scalar(&e, "status"), 0.);
    assert_eq!(output(&mut e, "disp(out)").trim(), "unset");
}

#[test]
fn system_cancellation_kills_the_command_tree() {
    let command = if cfg!(windows) {
        "x = system('ping -n 30 127.0.0.1 >nul & echo done');"
    } else {
        "x = system('sleep 30; echo done');"
    };
    let (error, took) = cancel_during(Engine::default(), command);
    assert!(error.contains("Calculation cancelled"), "{error}");
    // A surviving child would hold the call for ~30 s. Killing a process tree on a loaded Windows
    // machine took 2.3 s once the suites ran ~200 tests per process (batch 1103 gate).
    assert!(took < Duration::from_secs(5), "{took:?}");
}

#[cfg(not(windows))]
#[test]
fn system_cancellation_does_not_leave_a_grandchild() {
    let directory = tempfile::tempdir().unwrap();
    let marker = directory.path().join("grandchild-survived");
    let command = format!(
        "x = system('(sleep 0.4; touch {}) & sleep 30');",
        marker.to_string_lossy()
    );
    let (error, took) = cancel_during(Engine::default(), command);
    assert!(error.contains("Calculation cancelled"), "{error}");
    assert!(took < Duration::from_secs(1), "{took:?}");
    thread::sleep(Duration::from_millis(600));
    assert!(
        !marker.exists(),
        "a command grandchild survived cancellation"
    );
}

#[cfg(not(windows))]
#[test]
fn system_with_closed_pipes_stays_cancellable() {
    let (error, took) = cancel_during(Engine::default(), "x = system('exec >&- 2>&-; sleep 30');");
    assert!(error.contains("Calculation cancelled"), "{error}");
    assert!(took < Duration::from_secs(1), "{took:?}");
}

#[test]
fn display_matches_the_unsuppressed_echo() {
    let mut e = Engine::default();
    output(&mut e, "A = [1 2; 3 4]; s = struct('a', 1, 'b', 'text');");
    for name in ["A", "s"] {
        let echoed = output(&mut e, name);
        assert!(echoed.contains(&format!("{name} =")), "{echoed}");
        assert_eq!(output(&mut e, &format!("display({name})")), echoed);
    }
    // An expression has no name: no "ans =" header, the value as disp shows it
    // (MATLAB display documentation; GNU Octave 10.3 display(5) prints 5).
    let expression = output(&mut e, "display(A + 1)");
    assert_eq!(expression, output(&mut e, "disp(A + 1)"));
}

#[test]
fn display_handles_text_and_cells_like_echo() {
    let mut e = Engine::default();
    for (expression, shown) in [
        ("display('abc')", "disp('abc')"),
        ("display(\"abc\")", "disp(\"abc\")"),
        ("display(true)", "disp(true)"),
        ("display(int8(7))", "disp(int8(7))"),
        ("display({1, 'two'})", "disp({1, 'two'})"),
    ] {
        assert_eq!(
            output(&mut e, expression),
            output(&mut e, shown),
            "{expression}"
        );
    }
    assert_eq!(output(&mut e, "display(\"abc\")"), "abc");
    output(&mut e, "c = {1, 'two'}; t = 'abc'; s = \"abc\";");
    for name in ["c", "t", "s"] {
        let echoed = output(&mut e, name);
        assert!(echoed.contains(&format!("{name} =")), "{echoed}");
        assert_eq!(output(&mut e, &format!("display({name})")), echoed);
    }
}

#[test]
fn cli_input_reads_standard_input() {
    let mut child = Command::new(env!("CARGO_BIN_EXE_matrust"))
        .args(["--eval", "x=input('n? '); disp(x*2)"])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    child.stdin.take().unwrap().write_all(b"3\n").unwrap();
    let result = child.wait_with_output().unwrap();
    let stdout = String::from_utf8_lossy(&result.stdout);
    assert!(result.status.success(), "{stdout}");
    // Piped input is not echoed, so the answer's output follows the prompt.
    let stdout = stdout.replace("\r\n", "\n");
    assert!(stdout.starts_with("n? "), "{stdout}");
    assert_eq!(stdout[3..].trim(), "6", "{stdout}");
}
