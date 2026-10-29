use matrust::{Engine, files};
use std::path::Path;
fn main() -> Result<(), String> {
    let reference = files::load_workspace(Path::new("target/evidence/user-functions-oracle.mat"))?;
    for workers in [1, 8] {
        rayon::ThreadPoolBuilder::new()
            .num_threads(workers)
            .build()
            .map_err(|e| e.to_string())?
            .install(|| -> Result<(), String> {
                let mut e = Engine::default();
                e.execute(include_str!(
                    "../tests/fixtures/scripts/user_function_cases.m"
                ))?;
                let actual = &e.variables["ans"];
                let expected = &reference["R"];
                if (actual.rows(), actual.cols()) != (expected.rows(), expected.cols())
                    || actual.data() != expected.data()
                {
                    return Err("MATLAB function cases differ in shape or values".into());
                }
                let mut flags = Vec::new();
                flags.push(
                    if e.execute("[~,v]=unassigned();function [unused,y]=unassigned;y=7;end")
                        .is_ok()
                    {
                        e.variables["v"].data()[0]
                    } else {
                        -1.
                    },
                );
                for source in [
                    "[~]=unassigned();function y=unassigned;end",
                    "unassigned();function y=unassigned;end",
                ] {
                    flags.push(if e.execute(source).is_ok() { 1. } else { -1. });
                }
                if flags != reference["flags"].data() {
                    return Err(format!(
                        "Unassigned-output behavior differs: Rust {flags:?}, MATLAB {:?}",
                        reference["flags"].data()
                    ));
                }
                println!(
                    "MATRUST_USER_FUNCTIONS_REFERENCE_OK workers={workers} values={}",
                    expected.len() + flags.len()
                );
                Ok(())
            })?;
    }
    Ok(())
}
