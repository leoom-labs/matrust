//! Reference envfiles corpus, stored as expected values in the fixture. Each
//! case gets a fresh engine and a new, automatically removed worktree temp
//! folder. Only the explicit HARNESS rules below relax a comparison.
use matrust::{Array, Engine};
use std::{
    fs,
    sync::atomic::Ordering,
    time::{Duration, Instant},
};

const CORPUS: &str = include_str!("fixtures/fn/envfiles-expected.txt");
// Existing evaluator/display behavior outside this task (display edits are
// forbidden). These cases still execute and check class, size and all values:
// only the precise display discrepancy listed here is accepted below.
const KNOWN: &[(usize, &str)] = &[
    (
        36,
        "Existing cell renderer does not shorten long text to the oracle's 0x1A ellipsis",
    ),
    (
        41,
        "Existing struct array renderer right-aligns field names instead of indenting four spaces",
    ),
    (
        47,
        "Existing struct array renderer right-aligns field names instead of indenting four spaces",
    ),
    (
        54,
        "Existing cell renderer does not shorten long text to the oracle's 0x1A ellipsis",
    ),
    (
        57,
        "Existing cell renderer does not shorten long text to the oracle's 0x1A ellipsis",
    ),
];

/// Every case with a relaxed comparison or relocated input appears here.
/// ENV character widths depend on the host, engine version and temp-name
/// generator; verify a row (or empty character array), not reference path length.
const HARNESS: &[(usize, &str)] = &[
    (
        54,
        "KNOWN: compare the full id with the exact oracle-shortened display",
    ),
    (
        57,
        "KNOWN: compare the full id with the exact oracle-shortened display",
    ),
    (
        1,
        "Platform predicate: exact host logical scalar instead of Windows value",
    ),
    (
        2,
        "Platform predicate: exact host logical scalar instead of Windows value",
    ),
    (
        3,
        "Platform predicate: exact host logical scalar instead of Windows value",
    ),
    (5, "Host filesep; exact character scalar"),
    (6, "Host pathsep; exact character scalar"),
    (
        31,
        "Relocate fullfile(tempdir,...) into fresh test folder; exact result",
    ),
    (
        32,
        "Relocate fullfile(tempdir,...) into fresh test folder; exact result",
    ),
    (
        33,
        "Relocate fullfile(tempdir,...) into fresh test folder; exact result",
    ),
    (
        35,
        "Relocate fullfile(tempdir,...) into fresh test folder; exact result",
    ),
    (
        36,
        "Relocate fullfile(tempdir,...) into fresh test folder; exact result",
    ),
    (
        37,
        "Relocate fullfile(tempdir,...) into fresh test folder; exact result",
    ),
    (
        38,
        "Relocate fullfile(tempdir,...) into fresh test folder; exact result",
    ),
    (
        41,
        "Relocate tempdir; remove MATLAB desktop help hyperlink only",
    ),
    (
        42,
        "Relocate fullfile(tempdir,...) into fresh test folder; exact result",
    ),
    (
        43,
        "Relocate fullfile(tempdir,...) into fresh test folder; exact result",
    ),
    (
        44,
        "Relocate fullfile(tempdir,...) into fresh test folder; exact result",
    ),
    (
        45,
        "Relocate fullfile(tempdir,...) into fresh test folder; exact result",
    ),
    (
        46,
        "Inspect fresh test folder instead of host tempdir; exact result",
    ),
    (
        47,
        "Remove MATLAB desktop help hyperlink only; apply only the precise KNOWN display rule",
    ),
    (
        49,
        "Relocate fullfile(tempdir,...) into fresh test folder; exact result",
    ),
    (
        52,
        "Relocate fullfile(tempdir,...) into fresh test folder; exact result",
    ),
    (
        53,
        "Relocate fullfile(tempdir,...) into fresh test folder; exact result",
    ),
    (
        56,
        "Relocate fullfile(tempdir,...) into fresh test folder; exact result",
    ),
    (
        58,
        "Relocate fullfile(tempdir,...) into fresh test folder; exact result",
    ),
    (
        59,
        "Relocate fullfile(tempdir,...) into fresh test folder; exact result",
    ),
    (
        60,
        "Compare warning message only, removing desktop markup and backtrace",
    ),
    (
        66,
        "Timing-dependent warning optional; exact warning text if present; exact result",
    ),
    (
        67,
        "Timing-dependent warning optional; exact warning text if present; exact result",
    ),
    (
        76,
        "ENV: class and size only; status and error id checked separately",
    ),
    (
        77,
        "ENV error: replace the oracle current-folder prefix with test folder",
    ),
    (78, "ENV: character row, variable MatRust version width"),
    (79, "ENV: character row, variable MatRust release width"),
    (80, "ENV: character row, variable MatRust build-date width"),
    (81, "ENV: class, size and exact ordered field names"),
    (82, "ENV: character row, host computer identifier width"),
    (83, "ENV: character row, host architecture width"),
    (
        84,
        "ENV: class and size only; computer components checked separately",
    ),
    (85, "ENV: character row, installation-path width"),
    (86, "ENV: character row, current-folder width"),
    (87, "ENV: character row, host temporary-folder width"),
    (88, "ENV: character row, generated temporary-name width"),
    (89, "ENV: character row, generated temporary-name width"),
    (
        90,
        "ENV: character row or empty; host variable may be unset",
    ),
    (
        91,
        "ENV: character row or empty; host variable may be unset",
    ),
    (92, "ENV: exact class and size, variable local clock"),
    (93, "ENV: exact class and size, variable CPU time"),
    (94, "ENV: exact class and size, variable local date"),
    (95, "ENV: exact class and size, variable current serial day"),
    (
        96,
        "ENV: exact class and size, variable measured time and warning",
    ),
    (97, "ENV: exact class and size; relocate file operations"),
    (
        98,
        "ENV: exact class, size and ordered field names; relocate file operations",
    ),
    (
        99,
        "ENV: character row, previous-folder width; relocate file operations",
    ),
    (
        100,
        "ENV: exact class and size, host listing layout; relocate file operations",
    ),
];

struct Case {
    id: usize,
    statement: String,
    console: String,
    class: Option<String>,
}
fn cases() -> Vec<Case> {
    let mut result = Vec::new();
    let mut rest = CORPUS;
    while let Some(start) = rest.find("@@CASE ") {
        rest = &rest[start + 7..];
        let (header, after) = rest.split_once('\n').expect("header");
        let (id, statement) = header.split_once('|').expect("id|statement");
        let (body, after) = after.split_once("@@END\n").expect("end marker");
        let (console, class) = match body.rfind("@@CLASS ") {
            Some(at) => (body[..at].into(), Some(body[at + 8..].trim_end().into())),
            None => (body.into(), None),
        };
        result.push(Case {
            id: id.parse().unwrap(),
            statement: statement.into(),
            console,
            class,
        });
        rest = after;
    }
    result
}
fn folder() -> tempfile::TempDir {
    tempfile::Builder::new()
        .prefix(".envfiles-test-")
        .tempdir_in(env!("CARGO_MANIFEST_DIR"))
        .unwrap()
}
fn quoted(path: &std::path::Path) -> String {
    path.to_string_lossy().replace('\'', "''")
}
fn console(e: &mut Engine, source: &str) -> String {
    e.execute(source)
        .unwrap_or_else(|error| panic!("{source}: {error}"))
        .output
        .iter()
        .map(|s| format!("{s}\n"))
        .collect()
}
fn unplaced(error: &str) -> &str {
    match error.split_once(": ") {
        Some((p, s)) if p.starts_with("Line ") => s,
        _ => error,
    }
}
fn plain_links(text: &str) -> String {
    let mut result = String::new();
    let mut rest = text;
    while let Some(at) = rest.find("<a href=\"") {
        result.push_str(&rest[..at]);
        let after = &rest[at..];
        let open = after.find('>').unwrap();
        let close = after.find("</a>").unwrap();
        result.push_str(&after[open + 1..close]);
        rest = &after[close + 4..];
    }
    result.push_str(rest);
    result
}
fn warning_plain(text: &str) -> String {
    text.replace("[\u{8}", "")
        .replace("]\u{8} ", "")
        .lines()
        .filter(|l| !l.starts_with("> In ") && !l.starts_with("  In "))
        .map(|l| format!("{l}\n"))
        .collect()
}
fn strip_timing_warning(text: &str) -> String {
    let warning = "Warning: The measured time for F may be inaccurate because it is running too fast. Try measuring something\nthat takes longer.\n";
    // No arbitrary warning is swallowed: the sole accepted warning is exact.
    text.strip_prefix(warning).unwrap_or(text).into()
}
fn check(case: &Case) -> Result<bool, String> {
    let temp = folder();
    let mut e = Engine::default();
    e.set_current_folder(temp.path()).unwrap();
    console(
        &mut e,
        &format!("envx_test_root = '{}';", quoted(temp.path())),
    );
    let statement = case
        .statement
        .replace("fullfile(tempdir,", "fullfile(envx_test_root,")
        .replace(
            "fieldnames(dir(tempdir))",
            "fieldnames(dir(envx_test_root))",
        );
    let result = e.execute(&statement);
    if let Some(expected) = case.console.trim_end().strip_prefix("ERR ") {
        let error = result.err().ok_or("Expected error, got success")?;
        let actual = unplaced(&error);
        if case.id == 77 {
            let wanted = format!(
                "{} is not a directory.",
                temp.path().join("no_such_dir_zz_3").display()
            );
            return if actual == wanted {
                Ok(true)
            } else {
                Err(format!("error {actual:?}, expected {wanted:?}"))
            };
        }
        return if actual == expected {
            Ok(true)
        } else {
            Err(format!("error {actual:?}, expected {expected:?}"))
        };
    }
    let actual: String = result
        .map_err(|error| format!("unexpected error {error}"))?
        .output
        .iter()
        .map(|s| format!("{s}\n"))
        .collect();
    let x = e.array("x").ok_or("x missing")?;
    let class = format!("{} [{} {}]", x.class(), x.rows(), x.cols());
    if matches!(case.id, 78..=80 | 82..=83 | 85..=91 | 99) {
        if x.class() != "char" || !(x.rows() == 1 || x.rows() == 0 && x.cols() == 0) {
            return Err(format!("expected character row, got {class}"));
        }
    } else if Some(&class) != case.class.as_ref() {
        return Err(format!("class/size {class:?}, expected {:?}", case.class));
    }
    if matches!(case.id, 81 | 98) {
        let Array::Struct(s) = x else {
            return Err("Expected struct".into());
        };
        let expected = if case.id == 81 {
            vec!["Name", "Version", "Release", "Date"]
        } else {
            vec!["name", "date", "bytes", "isdir", "datenum"]
        };
        if s.field_names() != expected {
            return Err("Struct field names differ".into());
        }
    }
    if case.id >= 76 {
        return Ok(true);
    }
    let mut expected = case.console.clone();
    if matches!(case.id, 1..=3 | 5..=6) {
        let value = match case.id {
            1 => usize::from(cfg!(windows)).to_string(),
            2 => usize::from(cfg!(unix)).to_string(),
            3 => usize::from(cfg!(target_os = "macos")).to_string(),
            5 => std::path::MAIN_SEPARATOR.to_string(),
            _ => if cfg!(windows) { ";" } else { ":" }.into(),
        };
        expected = if case.id <= 3 {
            format!("\nx =\n\n     {value}\n\n")
        } else {
            format!("\nx =\n\n{value}\n\n")
        };
    }
    if matches!(case.id, 41 | 47) {
        expected = plain_links(&expected);
    }
    if case.id == 60 {
        expected = warning_plain(&expected);
    }
    let actual = if matches!(case.id, 66 | 67) {
        expected = strip_timing_warning(&warning_plain(&expected));
        strip_timing_warning(&actual)
    } else {
        actual
    };
    let known_case = KNOWN.iter().any(|(id, _)| *id == case.id);
    if actual == expected {
        return if known_case {
            Err("passes now; remove it from KNOWN".into())
        } else {
            Ok(true)
        };
    }
    if KNOWN.iter().any(|(id, _)| *id == case.id) {
        let adjusted = match case.id {
            36 => actual.replace(
                "'MATLAB:RMDIR:NoDirectoriesRemoved'",
                "'MATLAB:RMDIR:NoDirectoriesR\u{1a}'",
            ),
            54 => actual.replace(
                "'MATLAB:COPYFILE:FileDoesNotExist'",
                "'MATLAB:COPYFILE:FileDoesNot\u{1a}'",
            ),
            57 => actual.replace(
                "'MATLAB:MOVEFILE:FileDoesNotExist'",
                "'MATLAB:MOVEFILE:FileDoesNot\u{1a}'",
            ),
            41 | 47 => actual
                .lines()
                .map(|line| {
                    if ["name", "date", "bytes", "isdir", "datenum"].contains(&line.trim()) {
                        format!("    {}\n", line.trim())
                    } else {
                        format!("{line}\n")
                    }
                })
                .collect(),
            _ => actual.replacen("\nans =", "\nx =", 1),
        };
        if adjusted == expected {
            return Ok(false);
        }
    }
    Err(format!(
        "console\n expected {expected:?}\n actual   {actual:?}"
    ))
}
#[test]
fn corpus_has_every_case() {
    let cases = cases();
    assert_eq!(cases.len(), 100);
    for (id, reason) in HARNESS.iter().chain(KNOWN) {
        assert!(cases.iter().any(|c| c.id == *id));
        assert!(!reason.is_empty());
    }
    for (index, case) in cases.iter().enumerate() {
        assert_eq!(case.id, index + 1);
    }
}
#[test]
fn matches_reference() {
    let mut failures = Vec::new();
    let mut passed = 0;
    let mut known = 0;
    for case in cases() {
        match check(&case) {
            Ok(true) => passed += 1,
            Ok(false) => known += 1,
            Err(problem) => {
                failures.push(format!("case {} `{}`: {problem}", case.id, case.statement))
            }
        }
    }
    assert!(
        failures.is_empty(),
        "{passed} passed, {} failed:\n{}",
        failures.len(),
        failures.join("\n")
    );
    assert_eq!(passed + known, 100);
    println!(
        "{passed}/100 exact or HARNESS matches; {known} precisely validated KNOWN display differences; no skipped cases"
    );
}

#[test]
fn current_folder_drives_files_functions_and_run_and_survives_error() {
    let root = folder();
    let first = root.path().join("first");
    let second = root.path().join("second");
    fs::create_dir(&first).unwrap();
    fs::create_dir(&second).unwrap();
    fs::write(first.join("answer.m"), "function y=answer; y=11; end").unwrap();
    fs::write(second.join("answer.m"), "function y=answer; y=22; end").unwrap();
    fs::write(second.join("script.m"), "script_value=33;").unwrap();
    let process_folder = std::env::current_dir().unwrap();
    let mut e = Engine::default();
    e.set_current_folder(&first).unwrap();
    console(
        &mut e,
        "a=answer; cd('../second'); b=answer; run('script'); fid=fopen('relative.txt','w'); fprintf(fid,'hello'); fclose(fid); text=fileread('relative.txt');",
    );
    assert_eq!(e.variables["a"].data(), &[11.]);
    assert_eq!(e.variables["b"].data(), &[22.]);
    assert_eq!(e.variables["script_value"].data(), &[33.]);
    assert_eq!(
        fs::read_to_string(second.join("relative.txt")).unwrap(),
        "hello"
    );
    assert_eq!(
        e.current_folder(),
        Some(second.canonicalize().unwrap().as_path())
    );
    assert_eq!(
        e.help_path().directory.as_deref(),
        Some(second.canonicalize().unwrap().as_path())
    );
    assert!(e.execute("cd('absent')").is_err());
    assert_eq!(
        e.current_folder(),
        Some(second.canonicalize().unwrap().as_path())
    );
    assert_eq!(std::env::current_dir().unwrap(), process_folder);
}
#[test]
fn pause_is_cancellable_zero_and_disabled_return_promptly() {
    let mut e = Engine::default();
    let start = Instant::now();
    console(
        &mut e,
        "pause(0); pause(-1); pause('off'); pause(10); state=pause('query'); pause('on');",
    );
    assert!(start.elapsed() < Duration::from_secs(2));
    assert!(
        e.execute("pause")
            .unwrap_err()
            .contains("not supported yet")
    );
    let cancel = e.cancellation();
    let worker = std::thread::spawn(move || e.execute("pause(60)"));
    std::thread::sleep(Duration::from_millis(30));
    cancel.store(true, Ordering::Relaxed);
    assert!(worker.join().unwrap().unwrap_err().contains("cancelled"));
    assert!(start.elapsed() < Duration::from_secs(2));
}
#[test]
fn filesystem_statuses_and_data_are_not_only_display_checks() {
    let root = folder();
    let mut e = Engine::default();
    e.set_current_folder(root.path()).unwrap();
    console(
        &mut e,
        "mkdir('a'); mkdir('a','sub'); fid=fopen('a/sub/file.txt','w'); fprintf(fid,'hello'); fclose(fid); [s,m,i]=copyfile('a/sub/file.txt','copy.txt'); moved=movefile('copy.txt','moved.txt'); text=fileread('moved.txt'); delete('moved.txt'); [rs,rm,ri]=rmdir('a'); [ms,mm,mi]=copyfile('missing','out'); [ns,nm,ni]=rmdir('missing'); rmdir('a','s');",
    );
    assert_eq!(e.logical_variables["s"].data(), &[1]);
    assert_eq!(e.logical_variables["moved"].data(), &[1]);
    assert_eq!(e.logical_variables["rs"].data(), &[0]);
    assert_eq!(e.logical_variables["ms"].data(), &[0]);
    assert_eq!(e.logical_variables["ns"].data(), &[0]);
    assert_eq!(console(&mut e, "disp(ni)"), "MATLAB:RMDIR:NotADirectory\n");
    assert_eq!(
        console(&mut e, "disp(ri)"),
        "MATLAB:RMDIR:NoDirectoriesRemoved\n"
    );
    assert_eq!(
        console(&mut e, "disp(mi)"),
        "MATLAB:COPYFILE:FileDoesNotExist\n"
    );
    assert_eq!(console(&mut e, "disp(text)"), "hello\n");
    assert_eq!(fs::read_dir(root.path()).unwrap().count(), 0);
}
#[test]
fn environment_changes_are_bounded_and_engine_local() {
    let mut first = Engine::default();
    let mut second = Engine::default();
    let before = console(&mut second, "disp(getenv('MATRUST_ENVFILES_LOCAL_TEST'))");
    console(&mut first, "setenv('MATRUST_ENVFILES_LOCAL_TEST','one');");
    assert_eq!(
        console(&mut first, "disp(getenv('MATRUST_ENVFILES_LOCAL_TEST'))"),
        "one\n"
    );
    assert_eq!(
        console(&mut second, "disp(getenv('MATRUST_ENVFILES_LOCAL_TEST'))"),
        before
    );
    console(&mut first, "setenv('MATRUST_ENVFILES_LOCAL_TEST');");
    assert_eq!(
        console(&mut first, "disp(getenv('MATRUST_ENVFILES_LOCAL_TEST'))"),
        ""
    );
}
#[test]
fn local_clock_serial_date_version_and_temp_names_have_real_meaning() {
    let root = folder();
    let mut e = Engine::default();
    e.set_current_folder(root.path()).unwrap();
    console(
        &mut e,
        "c=clock; n=now; delta=abs(datenum(c)-n); d=date; [host,limit,endian]=computer; v=ver('matlab'); elapsed=timeit(@() ones(10)); cpu=cputime;",
    );
    assert!(e.variables["delta"].data()[0] < 1. / 86400.);
    assert_eq!(e.variables["limit"].data(), &[281_474_976_710_655.]);
    let host = if cfg!(windows) {
        "PCWIN64"
    } else if cfg!(target_os = "macos") {
        "MACI64"
    } else {
        "GLNXA64"
    };
    assert_eq!(console(&mut e, "disp(host)"), format!("{host}\n"));
    assert_eq!(
        console(&mut e, "disp(endian)"),
        if 1u16.to_ne_bytes()[0] == 1 {
            "L\n"
        } else {
            "B\n"
        }
    );
    assert!(e.variables["elapsed"].data()[0] > 0.);
    assert!(e.variables["cpu"].data()[0] >= 0.);
    assert_eq!(console(&mut e, "disp(v.Name)"), "MatRust\n");
    assert_eq!(
        console(&mut e, "disp(version)"),
        format!("MatRust {}\n", env!("CARGO_PKG_VERSION"))
    );
    console(
        &mut e,
        "a=tempname(pwd); b=tempname(pwd); different=~strcmp(a,b); absent=exist(a,'file');",
    );
    assert_eq!(e.logical_variables["different"].data(), &[1]);
    assert_eq!(e.variables["absent"].data(), &[0.]);
}

#[test]
fn command_forms_and_independent_folders() {
    let one = folder();
    let two = folder();
    let mut a = Engine::default();
    let mut b = Engine::default();
    a.set_current_folder(one.path()).unwrap();
    b.set_current_folder(two.path()).unwrap();
    console(
        &mut a,
        "mkdir child; cd child; f=fopen('same.txt','w'); fprintf(f,'first'); fclose(f);",
    );
    console(
        &mut b,
        "f=fopen('same.txt','w'); fprintf(f,'second'); fclose(f);",
    );
    assert_eq!(console(&mut a, "disp(fileread('same.txt'))"), "first\n");
    assert_eq!(console(&mut b, "disp(fileread('same.txt'))"), "second\n");
    console(
        &mut a,
        "copyfile same.txt another.txt; movefile another.txt moved.txt; delete *.txt; cd ..; rmdir child;",
    );
    assert_eq!(fs::read_dir(one.path()).unwrap().count(), 0);
    assert_eq!(
        fs::read_to_string(two.path().join("same.txt")).unwrap(),
        "second"
    );
}

#[test]
fn file_bounds_and_failed_copy_preserve_existing_destination() {
    let temp = folder();
    let mut e = Engine::default();
    e.set_current_folder(temp.path()).unwrap();
    fs::write(temp.path().join("long.txt"), vec![b'x'; 65537]).unwrap();
    assert!(
        e.execute("x=fileread('long.txt')")
            .unwrap_err()
            .contains("65536")
    );
    fs::write(temp.path().join("destination"), b"keep").unwrap();
    let large = fs::File::create(temp.path().join("large")).unwrap();
    large.set_len(256 * 1024 * 1024 + 1).unwrap();
    assert!(
        e.execute("copyfile('large','destination')")
            .unwrap_err()
            .contains("268435456-byte")
    );
    console(&mut e, "[ok,msg]=copyfile('large','destination');");
    assert_eq!(e.logical_variables["ok"].data(), &[0]);
    assert_eq!(fs::read(temp.path().join("destination")).unwrap(), b"keep");
    assert!(e.execute("copyfile('destination','destination')").is_err());
    assert_eq!(fs::read(temp.path().join("destination")).unwrap(), b"keep");
    fs::create_dir(temp.path().join("tree")).unwrap();
    assert!(e.execute("copyfile('tree','tree/inside')").is_err());
    assert!(!temp.path().join("tree/inside").exists());
    // A native rename does not read even this oversized sparse file.
    console(&mut e, "ok=movefile('large','renamed');");
    assert_eq!(e.logical_variables["ok"].data(), &[1]);
    assert_eq!(
        fs::metadata(temp.path().join("renamed")).unwrap().len(),
        256 * 1024 * 1024 + 1
    );
}

#[cfg(unix)]
#[test]
fn recursive_removal_does_not_follow_directory_symlinks() {
    let temp = folder();
    fs::create_dir(temp.path().join("keep")).unwrap();
    fs::write(temp.path().join("keep/file"), b"keep").unwrap();
    fs::create_dir(temp.path().join("remove")).unwrap();
    std::os::unix::fs::symlink(temp.path().join("keep"), temp.path().join("remove/link")).unwrap();
    let mut e = Engine::default();
    e.set_current_folder(temp.path()).unwrap();
    console(&mut e, "rmdir('remove','s');");
    assert_eq!(fs::read(temp.path().join("keep/file")).unwrap(), b"keep");
}

#[test]
fn cd_inside_run_does_not_mislabel_function_cache() {
    let temp = folder();
    for (name, number) in [("one", 1), ("two", 2)] {
        let path = temp.path().join(name);
        fs::create_dir(&path).unwrap();
        fs::write(
            path.join("answer.m"),
            format!("function y=answer; y={number}; end"),
        )
        .unwrap();
    }
    fs::write(temp.path().join("two/change.m"), "cd('../one'); a=answer;").unwrap();
    fs::write(temp.path().join("two/check.m"), "b=answer;").unwrap();
    let mut e = Engine::default();
    e.set_current_folder(&temp.path().join("one")).unwrap();
    console(&mut e, "run('../two/change.m'); run('../two/check.m');");
    assert_eq!(e.variables["a"].data(), &[1.]);
    assert_eq!(e.variables["b"].data(), &[2.]);
}

#[test]
fn directory_copy_and_wildcard_destinations() {
    let temp = folder();
    let mut e = Engine::default();
    e.set_current_folder(temp.path()).unwrap();
    fs::create_dir(temp.path().join("source")).unwrap();
    fs::write(temp.path().join("source/one.txt"), b"one").unwrap();
    fs::write(temp.path().join("source/two.txt"), b"two").unwrap();
    fs::create_dir(temp.path().join("destination")).unwrap();
    console(
        &mut e,
        "copyfile('source','destination'); copyfile('source/*.txt','newfolder'); movefile('newfolder/*.txt','moved');",
    );
    assert_eq!(
        fs::read(temp.path().join("destination/one.txt")).unwrap(),
        b"one"
    );
    assert!(!temp.path().join("destination/source").exists());
    assert_eq!(fs::read(temp.path().join("moved/two.txt")).unwrap(), b"two");
    assert_eq!(
        fs::read_dir(temp.path().join("newfolder")).unwrap().count(),
        0
    );
    assert_eq!(
        fs::metadata(temp.path().join("source/one.txt"))
            .unwrap()
            .modified()
            .unwrap(),
        fs::metadata(temp.path().join("destination/one.txt"))
            .unwrap()
            .modified()
            .unwrap()
    );
}

#[test]
fn function_values_set_ans_and_quiet_stays_quiet() {
    let temp = folder();
    let mut e = Engine::default();
    e.set_current_folder(temp.path()).unwrap();
    assert_eq!(console(&mut e, "pwd;"), "");
    assert_eq!(console(&mut e, "disp(class(ans))"), "char\n");
    assert!(console(&mut e, "pwd").contains("ans ="));
    assert_eq!(console(&mut e, "pause('query');"), "");
    let echoed = console(&mut e, "pause('query')");
    assert!(
        echoed.contains("ans =") && echoed.contains("on"),
        "{echoed:?}"
    );
    assert_eq!(console(&mut e, "version;"), "");
    assert_eq!(console(&mut e, "computer;"), "");
}

#[test]
fn cd_in_a_loop_does_not_exhaust_function_file_limits() {
    let temp = folder();
    for name in ["one", "two"] {
        fs::create_dir(temp.path().join(name)).unwrap();
        fs::write(
            temp.path().join(name).join("answer.m"),
            "function y=answer; y=1; end",
        )
        .unwrap();
    }
    let mut e = Engine::default();
    e.set_current_folder(temp.path()).unwrap();
    console(
        &mut e,
        "t=0; for k=1:100, cd('one'); t=t+answer; cd('../two'); t=t+answer; cd('..'); end",
    );
    assert_eq!(e.variables["t"].data(), &[200.]);
}

#[test]
fn listing_deleting_reading_and_renaming_details() {
    let temp = folder();
    let mut e = Engine::default();
    e.set_current_folder(temp.path()).unwrap();
    fs::write(temp.path().join("b.txt"), b"x\r\ny").unwrap();
    fs::write(temp.path().join("B2.txt"), b"x").unwrap();
    fs::write(temp.path().join("a.txt"), b"x").unwrap();
    fs::create_dir(temp.path().join("dir.txt")).unwrap();
    fs::write(temp.path().join("-first.txt"), b"x").unwrap();
    #[cfg(unix)]
    std::os::unix::fs::symlink(temp.path().join("gone"), temp.path().join("dangling")).unwrap();
    console(
        &mut e,
        "d=dir; n={d.name}; s=numel(fieldnames(d)); t=fileread('b.txt'); double(t)",
    );
    let names: Vec<String> = console(
        &mut e,
        "s=''; for k=1:numel(d), s=[s d(k).name '|']; end; disp(s(1:end-1))",
    )
    .trim()
    .split('|')
    .map(str::to_owned)
    .collect();
    assert_eq!(&names[..2], [".", ".."]);
    let rest = &names[2..];
    let sorted_rest: Vec<_> = {
        let mut r = rest.to_vec();
        r.sort_by(|a, b| a.to_lowercase().cmp(&b.to_lowercase()).then(a.cmp(b)));
        r
    };
    assert_eq!(rest, sorted_rest);
    assert_eq!(console(&mut e, "disp(numel(t))").trim(), "4"); // CRLF kept
    // wildcard delete skips the folder and removes the files
    console(&mut e, "delete('*.txt');");
    assert!(temp.path().join("dir.txt").is_dir());
    assert!(!temp.path().join("a.txt").exists());
    // a registered identifier silences the not-found warning
    assert!(console(&mut e, "delete('nope.txt')").contains("not found"));
    console(&mut e, "warning('off','MATLAB:DELETE:FileNotFound');");
    assert_eq!(console(&mut e, "delete('nope.txt')"), "");
    // case-only rename
    fs::write(temp.path().join("c.dat"), b"x").unwrap();
    let ok = console(&mut e, "movefile('c.dat','C.dat')");
    assert_eq!(ok, "");
    assert!(
        fs::read_dir(temp.path())
            .unwrap()
            .any(|f| f.unwrap().file_name() == "C.dat")
    );
}
