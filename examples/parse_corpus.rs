//! Parse-only check of MATLAB script/function files (no evaluation, no files written).
//!
//! Usage: `cargo run --release --example parse_corpus -- a.m b.m ...`
//! Prints `ok <file>` or `FAIL <file>:<line>: <message>` per file, then
//! `parse failures: N/M`. Uses `syntax::parse_detailed`, the entry the engine
//! uses for script and function files on the path (`run`, `myscript`).
use std::process::ExitCode;

fn main() -> ExitCode {
    let paths: Vec<String> = std::env::args().skip(1).collect();
    if paths.is_empty() {
        eprintln!("usage: parse_corpus <file.m>...");
        return ExitCode::from(2);
    }
    let mut failures = 0usize;
    for path in &paths {
        let bytes = match std::fs::read(path) {
            Ok(bytes) => bytes,
            Err(e) => {
                failures += 1;
                println!("FAIL {path}:0: cannot read: {e}");
                continue;
            }
        };
        // The engine reads script files with `read_to_string`, so a non-UTF-8
        // file (Latin-1 comments) fails to load there too: count it.
        let source = match String::from_utf8(bytes) {
            Ok(text) => text,
            Err(e) => {
                failures += 1;
                println!("FAIL {path}:0: not valid UTF-8: {}", e.utf8_error());
                continue;
            }
        };
        match matrust::syntax::parse_detailed(&source) {
            Ok(_) => println!("ok {path}"),
            Err(d) => {
                failures += 1;
                let message = d.message.replace(['\r', '\n'], " ");
                println!("FAIL {path}:{}: {message}", d.line);
            }
        }
    }
    println!("parse failures: {failures}/{}", paths.len());
    ExitCode::SUCCESS
}
