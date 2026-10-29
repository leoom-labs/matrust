use matrust::{Engine, VERSION, engine::HELP};
use std::{
    io::{self, BufRead, IsTerminal, Write},
    process::ExitCode,
};

fn run() -> Result<(), String> {
    let mut args = std::env::args().skip(1);
    let mut source = None;
    let mut source_directory = None;
    let mut source_path = None;
    let mut svg = None;
    let mut threads = std::thread::available_parallelism()
        .map_or(1, usize::from)
        .min(8);
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--help" | "-h" => {
                println!(
                    "MatRust {VERSION} — Rust numerical workspace\n\nmatrust [script.mr | script.m | --eval EXPR] [--svg plot.svg] [--threads N]\nmatrust                     interactive console\nmatrust bench [options]     benchmark MatRust (matrust bench --help)\n\n{HELP}"
                );
                return Ok(());
            }
            "--version" | "-V" => {
                println!("MatRust {VERSION}");
                return Ok(());
            }
            "--eval" | "-e" => {
                source = Some(args.next().ok_or("--eval requires an expression")?);
                source_path = None;
            }
            "--svg" => svg = Some(args.next().ok_or("--svg requires a filename")?),
            "--threads" => {
                threads = args
                    .next()
                    .ok_or("--threads requires a count")?
                    .parse::<usize>()
                    .map_err(|_| "Invalid thread count")?;
                if !(1..=64).contains(&threads) {
                    return Err("Thread count must be between 1 and 64".into());
                }
            }
            _ if arg.starts_with('-') => return Err(format!("Unknown option {arg}")),
            _ => {
                if source.is_some() {
                    return Err("Supply one script or --eval expression".into());
                }
                let metadata = std::fs::metadata(&arg).map_err(|e| e.to_string())?;
                if metadata.len() > 1_000_000 {
                    return Err("Script exceeds 1 MB".into());
                }
                source = Some(std::fs::read_to_string(&arg).map_err(|e| format!("{arg}: {e}"))?);
                let path = std::path::absolute(&arg).map_err(|e| e.to_string())?;
                source_directory = path.parent().map(std::path::Path::to_path_buf);
                source_path = Some(path);
            }
        }
    }
    matrust::initialize_worker_pool(threads)?;
    let mut engine = Engine::default();
    let directory = source_directory.unwrap_or(std::env::current_dir().map_err(|e| e.to_string())?);
    engine.set_function_directory(&directory)?;
    engine.set_file_directory(&directory)?;
    if let Some(source) = source {
        print_document_execution(&mut engine, &source, source_path.as_deref())?;
    } else {
        let stdin = io::stdin();
        let interactive = stdin.is_terminal();
        if interactive {
            println!("MatRust {VERSION}\nType help for commands; exit to quit.");
        }
        let mut lines = stdin.lock().lines();
        loop {
            if interactive {
                print!(">> ");
                io::stdout().flush().map_err(|e| e.to_string())?;
            }
            let Some(line) = lines.next() else { break };
            let line = line.map_err(|e| e.to_string())?;
            if matches!(line.trim(), "exit" | "quit") {
                break;
            }
            if let Err(e) = print_execution(&mut engine, &line) {
                if interactive {
                    eprintln!("Error: {e}");
                } else {
                    return Err(e);
                }
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
    Ok(())
}
fn print_execution(engine: &mut Engine, source: &str) -> Result<(), String> {
    print_document_execution(engine, source, None)
}
fn print_document_execution(
    engine: &mut Engine,
    source: &str,
    path: Option<&std::path::Path>,
) -> Result<(), String> {
    let result = match path {
        Some(path) => engine.execute_document(source, path),
        None => engine.execute(source),
    };
    let output = match &result {
        Ok(result) => &result.output,
        Err(_) => &engine.failed_output.output,
    };
    for output in output {
        println!("{output}");
    }
    result.map(|_| ())
}
fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    if args.first().is_some_and(|arg| arg == "bench") {
        return ExitCode::from(matrust::bench::cli::run(&args[1..]));
    }
    match run() {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            eprintln!("Error: {e}");
            ExitCode::FAILURE
        }
    }
}
