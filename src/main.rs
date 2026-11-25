use matrust::{
    Engine, HostIo, VERSION,
    engine::{HELP, Limits},
};
use std::{
    io::{self, BufRead, IoSlice, IsTerminal, Write},
    process::ExitCode,
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, Ordering},
    },
    time::{Duration, Instant},
};

/// First Ctrl+C cancels the running calculation; a second one within 2 s exits with 130.
fn install_interrupt(cancel: Arc<AtomicBool>) {
    let last = Mutex::new(None::<Instant>);
    let result = ctrlc::set_handler(move || {
        let now = Instant::now();
        let mut last = last.lock().unwrap_or_else(|e| e.into_inner());
        if last.is_some_and(|t| now.duration_since(t) < Duration::from_secs(2)) {
            std::process::exit(130);
        }
        *last = Some(now);
        cancel.store(true, Ordering::Relaxed);
    });
    if let Err(e) = result {
        eprintln!("Warning: Ctrl+C cannot cancel calculations: {e}");
    }
}

/// Why a run failed; each maps to an exit status (see `--help`).
enum Failure {
    /// Status 2: a bad command line. `hint` adds the pointer to --help.
    Usage { message: String, hint: bool },
    /// Status 1: the script failed (parse or runtime error, step limit, cancel).
    Script(String),
}
impl From<String> for Failure {
    fn from(message: String) -> Self {
        Failure::Script(message)
    }
}
impl From<&str> for Failure {
    fn from(message: &str) -> Self {
        Failure::Script(message.into())
    }
}
fn usage(message: impl Into<String>) -> Failure {
    Failure::Usage {
        message: message.into(),
        hint: true,
    }
}
/// A script path that does not exist or cannot be read keeps its own message.
fn unreadable(message: impl Into<String>) -> Failure {
    Failure::Usage {
        message: message.into(),
        hint: false,
    }
}

const USAGE: &str = "matrust [script.mr | script.m | --eval STATEMENT | -batch STATEMENT] [--svg plot.svg] [--threads N] [--max-steps N]
matrust                     interactive console
matrust bench [options]     benchmark MatRust (matrust bench --help)

--eval, -e STATEMENT        run STATEMENT and exit
-batch, --batch STATEMENT   run STATEMENT and exit, never reading the console; a script
                            name (-batch myscript) runs myscript.m from the current folder
--max-steps N               stop a run after N statement/loop steps (for automated runs)
CLI runs have no step or per-run output limit; each displayed value is limited to 32 MiB. Ctrl+C stops a calculation; a second Ctrl+C within 2 s quits.
Errors are written to standard error; fprintf(2, ...) writes there too.

Exit status:
  0    success, or exit/quit without a status
  1    the script failed: parse or runtime error, --max-steps stop, Ctrl+C
  2    usage error: unknown option, missing value, unreadable script, two sources
  n    exit(n) or quit(n), n an integer from 0 to 255
  130  a second Ctrl+C within 2 s";

/// Returns the exit status: 0, or the one `exit(n)`/`quit(n)` asked for.
fn run(output_error: Arc<Mutex<Option<io::Error>>>) -> Result<u8, Failure> {
    let mut args = std::env::args().skip(1);
    let mut source = None;
    let mut source_directory = None;
    let mut source_path = None;
    let mut svg = None;
    let mut limits = Limits::local();
    let mut threads = std::thread::available_parallelism()
        .map_or(1, usize::from)
        .min(8);
    let one_source = |source: &Option<String>| {
        if source.is_some() {
            Err(usage("Supply one script or one --eval/-batch statement"))
        } else {
            Ok(())
        }
    };
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--help" | "-h" => {
                println!("MatRust {VERSION} — Rust numerical workspace\n\n{USAGE}\n\n{HELP}");
                return Ok(0);
            }
            "--version" | "-V" => {
                println!("MatRust {VERSION}");
                return Ok(0);
            }
            // -batch runs like --eval: a source given means the console is
            // never read, and the CLI never opens a window.
            "--eval" | "-e" | "-batch" | "--batch" => {
                one_source(&source)?;
                let what = if matches!(arg.as_str(), "--eval" | "-e") {
                    "an expression"
                } else {
                    "a statement"
                };
                source = Some(
                    args.next()
                        .ok_or_else(|| usage(format!("{arg} requires {what}")))?,
                );
                source_path = None;
            }
            "--svg" => {
                svg = Some(
                    args.next()
                        .ok_or_else(|| usage("--svg requires a filename"))?,
                )
            }
            "--max-steps" => {
                let steps = args
                    .next()
                    .ok_or_else(|| usage("--max-steps requires a count"))?
                    .parse::<usize>()
                    .map_err(|_| usage("Invalid step count"))?;
                if steps == 0 {
                    return Err(usage("Step count must be at least 1"));
                }
                limits.steps = Some(steps);
            }
            "--threads" => {
                threads = args
                    .next()
                    .ok_or_else(|| usage("--threads requires a count"))?
                    .parse::<usize>()
                    .map_err(|_| usage("Invalid thread count"))?;
                if !(1..=64).contains(&threads) {
                    return Err(usage("Thread count must be between 1 and 64"));
                }
            }
            _ if arg.starts_with('-') => return Err(usage(format!("Unknown option {arg}"))),
            _ => {
                one_source(&source)?;
                let path = std::path::absolute(&arg).map_err(|e| unreadable(e.to_string()))?;
                source = Some(matrust::files::read_script(&path).map_err(unreadable)?);
                source_directory = path.parent().map(std::path::Path::to_path_buf);
                source_path = Some(path);
            }
        }
    }
    matrust::initialize_worker_pool(threads)?;
    let mut engine = Engine::with_limits(limits);
    let cancel = engine.cancellation();
    engine.set_host_io(Some(Box::new(ConsoleHost {
        output_error: output_error.clone(),
        cancel: cancel.clone(),
        failed: false,
    })));
    install_interrupt(cancel.clone());
    let directory = source_directory.unwrap_or(std::env::current_dir().map_err(|e| e.to_string())?);
    engine.set_function_directory(&directory)?;
    engine.set_file_directory(&directory)?;
    for warning in engine.spath_load_saved_path() {
        eprintln!("{warning}");
    }
    if let Some(source) = source {
        print_document_execution(&mut engine, &source, source_path.as_deref(), &output_error)?;
        // exit/quit stops now: later steps such as --svg are skipped.
        if let Some(status) = engine.take_exit_request() {
            return Ok(status);
        }
    } else {
        let stdin = io::stdin();
        let interactive = stdin.is_terminal();
        if interactive {
            println!("MatRust {VERSION}\nType help for commands; exit to quit.");
        }
        loop {
            if interactive {
                print!(">> ");
                io::stdout().flush().map_err(|e| e.to_string())?;
            }
            // The lock is released between commands: `input` reads stdin too.
            let mut line = String::new();
            if stdin
                .lock()
                .read_line(&mut line)
                .map_err(|e| e.to_string())?
                == 0
            {
                break;
            }
            let line = line.strip_suffix('\n').unwrap_or(&line);
            let line = line.strip_suffix('\r').unwrap_or(line);
            if matches!(line.trim(), "exit" | "quit") {
                break;
            }
            // A Ctrl+C at the prompt must not cancel the next line.
            cancel.store(false, Ordering::Relaxed);
            if let Err(e) = print_document_execution(&mut engine, line, None, &output_error) {
                if output_error.lock().unwrap().is_some() {
                    return Err(e.into());
                }
                if interactive {
                    eprintln!("Error: {e}");
                } else {
                    return Err(e.into());
                }
            }
            // The keywords above end the console with 0; exit(n) is the builtin.
            if let Some(status) = engine.take_exit_request() {
                return Ok(status);
            }
        }
    }
    if let Some(path) = svg {
        let figure = engine
            .figure
            .as_ref()
            .ok_or("No plot to export; call plot(x,y) first")?;
        std::fs::write(&path, figure.svg()).map_err(|e| format!("{path}: {e}"))?;
        println!("Saved {path}");
    }
    Ok(0)
}
/// `input` and `pause` read from stdin; output before a prompt prints first.
struct ConsoleHost {
    output_error: Arc<Mutex<Option<io::Error>>>,
    cancel: Arc<AtomicBool>,
    failed: bool,
}
impl ConsoleHost {
    fn record_error(&mut self, result: io::Result<()>) -> bool {
        if let Err(error) = result {
            self.output_error.lock().unwrap().get_or_insert(error);
            self.failed = true;
            self.cancel.store(true, Ordering::Relaxed);
        }
        !self.failed
    }
}
impl HostIo for ConsoleHost {
    fn streams_output(&self) -> bool {
        true
    }
    fn read_line(&mut self, prompt: &str) -> Option<String> {
        if self.failed {
            return None;
        }
        let mut stdout = io::stdout().lock();
        if !self.record_error(write!(stdout, "{prompt}").and_then(|()| stdout.flush())) {
            return None;
        }
        drop(stdout);
        let mut line = String::new();
        match io::stdin().lock().read_line(&mut line) {
            Ok(0) | Err(_) => None,
            Ok(_) => Some(line),
        }
    }
    fn show_output(&mut self, lines: &[String], _clear: bool) -> bool {
        if self.failed {
            return true;
        }
        let mut stdout = io::stdout().lock();
        self.record_error(write_console_lines(&mut stdout, lines));
        true
    }
    /// The CLI owns its process: `run` reads the status and returns it from main.
    fn request_exit(&mut self, _status: u8) -> bool {
        true
    }
    fn write_error(&mut self, text: &str) -> bool {
        let mut stderr = io::stderr().lock();
        // A failed error stream has nowhere to report to; the text is dropped.
        let _ = stderr
            .write_all(text.as_bytes())
            .and_then(|()| stderr.flush());
        true
    }
}
fn write_console_lines(stdout: &mut impl Write, lines: &[String]) -> io::Result<()> {
    for line in lines {
        let parts = [IoSlice::new(line.as_bytes()), IoSlice::new(b"\n")];
        let written = loop {
            match stdout.write_vectored(&parts) {
                Err(error) if error.kind() == io::ErrorKind::Interrupted => continue,
                result => break result?,
            }
        };
        if written < line.len() {
            stdout.write_all(&line.as_bytes()[written..])?;
            stdout.write_all(b"\n")?;
        } else if written == line.len() {
            stdout.write_all(b"\n")?;
        }
    }
    stdout.flush()
}
fn print_document_execution(
    engine: &mut Engine,
    source: &str,
    path: Option<&std::path::Path>,
    output_error: &Mutex<Option<io::Error>>,
) -> Result<(), String> {
    let result = match path {
        Some(path) => engine.execute_document(source, path),
        None => engine.execute(source),
    };
    let output = match &result {
        Ok(result) => &result.output,
        Err(_) => &engine.failed_output.output,
    };
    let mut error = output_error.lock().unwrap();
    if error.is_none()
        && let Err(write_error) = write_console_lines(&mut io::stdout().lock(), output)
    {
        *error = Some(write_error);
    }
    if error.is_some() {
        return Err("Standard output failed".into());
    }
    result.map(|_| ())
}
fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    if args.first().is_some_and(|arg| arg == "bench") {
        return ExitCode::from(matrust::bench::cli::run(&args[1..]));
    }
    let output_error = Arc::new(Mutex::new(None));
    let result = run(output_error.clone());
    if let Some(error) = output_error.lock().unwrap().as_ref() {
        if error.kind() != io::ErrorKind::BrokenPipe {
            eprintln!("matrust: cannot write to standard output: {error}");
        }
        return ExitCode::FAILURE;
    }
    match result {
        Ok(status) => ExitCode::from(status),
        Err(Failure::Script(e)) => {
            eprintln!("Error: {e}");
            ExitCode::FAILURE
        }
        Err(Failure::Usage { message, hint }) => {
            if hint {
                eprintln!("Error: {message} (see matrust --help)");
            } else {
                eprintln!("Error: {message}");
            }
            ExitCode::from(2)
        }
    }
}
