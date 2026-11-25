//! Cargo's test auto-discovery is off (`autotests = false`): every top-level
//! tests/*.rs file runs only as a module of a suite in tests/suites/. This
//! guard keeps a new test file from silently never running.
use std::collections::BTreeSet;
use std::fs;
use std::path::Path;

fn rust_files(dir: &Path) -> BTreeSet<String> {
    fs::read_dir(dir)
        .unwrap_or_else(|e| panic!("read {}: {e}", dir.display()))
        .map(|entry| entry.expect("directory entry").path())
        .filter(|path| path.is_file() && path.extension().is_some_and(|e| e == "rs"))
        .map(|path| path.file_name().unwrap().to_string_lossy().into_owned())
        .collect()
}

#[test]
fn every_test_file_is_in_a_suite() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR"));
    let tests = root.join("tests");
    let suites = tests.join("suites");
    let manifest = fs::read_to_string(root.join("Cargo.toml")).expect("read Cargo.toml");
    let top_level = rust_files(&tests);
    let mut listed = BTreeSet::new();
    let mut problems = Vec::new();
    for suite in rust_files(&suites) {
        let target = format!("path = \"tests/suites/{suite}\"");
        if !manifest.contains(&target) {
            problems.push(format!(
                "tests/suites/{suite} has no [[test]] entry in Cargo.toml"
            ));
        }
        let text = fs::read_to_string(suites.join(&suite)).expect("read suite");
        for line in text.lines() {
            let Some(rest) = line.trim().strip_prefix("#[path = \"../") else {
                continue;
            };
            let name = rest.split('"').next().unwrap_or_default().to_owned();
            if !top_level.contains(&name) {
                problems.push(format!(
                    "tests/suites/{suite} lists tests/{name}, which does not exist"
                ));
            }
            if !listed.insert(name.clone()) {
                problems.push(format!("tests/{name} is listed in more than one suite"));
            }
        }
    }
    for name in top_level.difference(&listed) {
        problems.push(format!(
            "tests/{name} is in no suite: add it to a file in tests/suites/"
        ));
    }
    assert!(problems.is_empty(), "{}", problems.join("\n"));
}
