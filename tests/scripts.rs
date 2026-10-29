//! Script files called by name or run(): caller workspace, lookup order, limits.
use matrust::{Array, Engine};
use std::{
    path::Path,
    sync::atomic::{AtomicBool, Ordering},
};

fn scalar(e: &Engine, name: &str) -> f64 {
    e.variables[name].data()[0]
}
fn text(e: &Engine, name: &str) -> String {
    let Array::Character(c) = e.array(name).unwrap() else {
        panic!("{name} is not character")
    };
    c.to_utf8(&AtomicBool::new(false)).unwrap()
}
fn folder(files: &[(&str, &str)]) -> (tempfile::TempDir, Engine) {
    let dir = tempfile::tempdir().unwrap();
    for (name, source) in files {
        let path = dir.path().join(name);
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(path, source).unwrap();
    }
    let mut e = Engine::default();
    e.set_function_directory(dir.path()).unwrap();
    e.set_file_directory(dir.path()).unwrap();
    (dir, e)
}

#[test]
fn scripts_share_the_caller_workspace_and_nest() {
    let (_dir, mut e) = folder(&[
        ("setx.m", "x = base + 1;"),
        ("outer.m", "inner;\nz = w + 1;"),
        ("inner.m", "w = 5;"),
        ("ret.m", "a1 = 1;\nif a1 == 1\n  return\nend\na2 = 2;"),
    ]);
    // Reads the caller's base, writes x; a bare script call sets no ans.
    e.execute("base = 41;\nsetx\ny = x * 2;").unwrap();
    assert_eq!((scalar(&e, "x"), scalar(&e, "y")), (42., 84.));
    assert!(e.array("ans").is_none());
    e.execute("outer;").unwrap();
    assert_eq!((scalar(&e, "w"), scalar(&e, "z")), (5., 6.));
    // return leaves only the script.
    e.execute("ret; after = 3;").unwrap();
    assert_eq!((scalar(&e, "a1"), scalar(&e, "after")), (1., 3.));
    assert!(e.array("a2").is_none());
    // Scripts are not retained functions for a targeted variable clear.
    e.execute("setx = 1; clear setx").unwrap();
    assert!(e.array("setx").is_none());
}

#[test]
fn scripts_called_from_functions_use_the_function_workspace() {
    let (_dir, mut e) = folder(&[
        ("setx.m", "x = base + 1;"),
        ("f.m", "function r = f(a)\nbase = a;\nsetx;\nr = x;\nend"),
    ]);
    e.execute("base = 1; R = f(10);").unwrap();
    assert_eq!(scalar(&e, "R"), 11.);
    assert_eq!(scalar(&e, "base"), 1.);
    assert!(e.array("x").is_none());
    // A local function also hosts the script.
    e.execute("R = g(5);\nfunction r = g(base)\nsetx;\nr = x;\nend")
        .unwrap();
    assert_eq!(scalar(&e, "R"), 6.);
}

#[test]
fn scripts_resolve_after_variables_and_local_functions_and_shadow_builtin_statements() {
    let (_dir, mut e) = folder(&[
        ("sum.m", "ran_sum_script = 1;"),
        ("pick.mr", "which_one = 1;"),
        ("pick.m", "which_one = 2;"),
        ("setx.m", "x = 7;"),
    ]);
    // Reference behavior with a folder script sum.m: the statement `sum` runs the script,
    // a call with arguments or an output reaches the builtin.
    e.execute("sum;").unwrap();
    assert_eq!(scalar(&e, "ran_sum_script"), 1.);
    e.execute("clear ran_sum_script; r = sum([1 2]); sum([3 4]);")
        .unwrap();
    assert_eq!((scalar(&e, "r"), scalar(&e, "ans")), (3., 7.));
    assert!(e.array("ran_sum_script").is_none());
    // A variable shadows the script.
    e.execute("clear ran_sum_script; sum = [8 9]; sum;")
        .unwrap();
    assert!(e.array("ran_sum_script").is_none());
    e.execute("clear sum").unwrap();
    // Native .mr wins over .m, as for functions.
    e.execute("pick;").unwrap();
    assert_eq!(scalar(&e, "which_one"), 1.);
    // A local function of the running source precedes the folder script.
    e.execute("setx;\nfunction r = setx\nr = 99;\nend").unwrap();
    assert_eq!(scalar(&e, "ans"), 99.);
    assert!(e.array("x").is_none());
}

#[test]
fn scripts_called_with_arguments_or_outputs_report_matlab_errors() {
    let (dir, mut e) = folder(&[("setx.m", "x = 7;")]);
    let path = dir.path().canonicalize().unwrap().join("setx.m");
    let expected = format!(
        "Attempt to execute SCRIPT setx as a function:\n{}",
        path.display()
    );
    for source in [
        "q = setx;",
        "setx(1);",
        "q = setx();",
        "q = 1 + setx;",
        "[a, b] = setx(2);",
    ] {
        let error = e.execute(source).unwrap_err();
        assert!(error.contains(&expected), "{source}: {error}");
    }
    assert!(e.array("q").is_none() && e.array("x").is_none());
    // Reference behavior: the statement `setx()` runs the script like `setx`.
    e.execute("setx();").unwrap();
    assert_eq!(scalar(&e, "x"), 7.);
}

#[test]
fn scripts_see_the_local_functions_of_the_calling_file() {
    let (_dir, mut e) = folder(&[
        // The reference probe: callsloc -> useloc -> locf gives r = 5.
        (
            "callsloc.m",
            "function r = callsloc\nuseloc;\nr = v;\nend\nfunction y = locf\ny = 5;\nend",
        ),
        ("useloc.m", "v = locf;"),
        (
            "nested.m",
            "function r = nested\nouter_s;\nr = v;\nend\nfunction y = locf\ny = 6;\nend",
        ),
        ("outer_s.m", "useloc;"),
        (
            "own.m",
            "function r = own\nownloc;\nr = v;\nend\nfunction y = locf\ny = 7;\nend",
        ),
        ("ownloc.m", "v = locf;\nfunction y = locf\ny = 8;\nend"),
        (
            "callsfn.m",
            "function r = callsfn\nusefn;\nr = v;\nend\nfunction y = locf\ny = 9;\nend",
        ),
        ("usefn.m", "v = getloc;"),
        ("getloc.m", "function y = getloc\ny = locf;\nend"),
    ]);
    e.execute("r = callsloc;").unwrap();
    assert_eq!(scalar(&e, "r"), 5.);
    // A nested script still sees the host function's helpers.
    e.execute("r = nested;").unwrap();
    assert_eq!(scalar(&e, "r"), 6.);
    // The script's own local function comes first.
    e.execute("r = own;").unwrap();
    assert_eq!(scalar(&e, "r"), 8.);
    // A function called from the script sees only its own file.
    let error = e.execute("r = callsfn;").unwrap_err();
    assert!(error.contains("locf"), "{error}");
    // After the calls the helpers are private again.
    assert!(e.execute("q = locf;").is_err());
    assert!(e.execute("useloc;").is_err());
    // Submitted source with a local function hosts a script too.
    e.execute("useloc;\nfunction y = locf\ny = 10;\nend")
        .unwrap();
    assert_eq!(scalar(&e, "v"), 10.);
}

#[test]
fn script_mfilename_names_the_script_and_restores_the_caller() {
    let (dir, mut e) = folder(&[("who_am_i.m", "nm = mfilename; fp = mfilename('fullpath');")]);
    e.execute("who_am_i; after = mfilename;").unwrap();
    assert_eq!(text(&e, "nm"), "who_am_i");
    assert!(
        Path::new(&text(&e, "fp")).ends_with("who_am_i"),
        "{}",
        text(&e, "fp")
    );
    assert_eq!(text(&e, "after"), "");
    let main = dir.path().join("main.m");
    e.execute_document("who_am_i; after = mfilename;", &main)
        .unwrap();
    assert_eq!(
        (text(&e, "nm"), text(&e, "after")),
        ("who_am_i".into(), "main".into())
    );
    // A script error restores the document identity too.
    std::fs::write(
        dir.path().join("fails.m"),
        "nm = mfilename;\nerror('stop');",
    )
    .unwrap();
    assert!(e.execute_document("fails;", &main).is_err());
    e.execute_document("after = mfilename;", &main).unwrap();
    assert_eq!(
        (text(&e, "nm"), text(&e, "after")),
        ("fails".into(), "main".into())
    );
}

#[test]
fn run_accepts_names_extensions_relative_and_absolute_paths() {
    let (dir, mut e) = folder(&[
        ("setx.m", "x = 3;"),
        ("helper.m", "where = 1;"),
        (
            "sub/other.m",
            "helper;\nonm = mfilename;\nsave('in_sub.mat', 'where');",
        ),
        ("sub/helper.m", "where = 2;"),
        ("sub/fails.m", "error('inside sub');"),
        ("fn.m", "function r = fn\nr = 12;\nend"),
        ("notes.txt", "x = 1;"),
        ("1bad.m", "x = 1;"),
    ]);
    e.execute("run('setx'); a = x; clear x; run('setx.m');")
        .unwrap();
    assert_eq!((scalar(&e, "a"), scalar(&e, "x")), (3., 3.));
    // A path's folder is the lookup and file folder while it runs.
    e.execute("run('sub/other.m'); s = where; helper; m = where;")
        .unwrap();
    assert_eq!((scalar(&e, "s"), scalar(&e, "m")), (2., 1.));
    assert_eq!(text(&e, "onm"), "other");
    assert!(dir.path().join("sub/in_sub.mat").is_file());
    assert!(!dir.path().join("in_sub.mat").exists());
    let absolute = dir.path().join("sub").join("other.m");
    e.execute(&format!("clear where; run('{}');", absolute.display()))
        .unwrap();
    assert_eq!(scalar(&e, "where"), 2.);
    // Folder caches are kept, so a loop does not reload past the file limit.
    e.execute("for k = 1:100\n run('sub/other'); run('setx');\nend")
        .unwrap();
    // The caller folder is restored after an error.
    assert!(e.execute("run('sub/fails.m');").is_err());
    e.execute("helper;").unwrap();
    assert_eq!(scalar(&e, "where"), 1.);
    // A function file runs as the statement `fn;` would.
    e.execute("run('fn');").unwrap();
    assert_eq!(scalar(&e, "ans"), 12.);
    // Reference behavior: run('nonexistent_zz') errors with "nonexistent_zz not found."
    for (source, message) in [
        ("run('missing');", "missing not found."),
        ("run('sub/missing.m');", "sub/missing.m not found."),
        ("run('nofolder/setx.m');", "nofolder/setx.m not found."),
        ("run('setx.txt');", "setx.txt not found."),
        ("run('setx.mr');", "setx.mr not found."),
        ("run('1bad');", "1bad not found."),
        (
            "run('notes.txt');",
            "RUN cannot execute the file 'notes.txt'. RUN requires a valid script: a .m or .mr file whose name starts with a letter, then uses only letters, digits or '_'.",
        ),
        ("run('1bad.m');", "RUN cannot execute the file '1bad.m'"),
        ("run('a', 'b');", "run requires one script name or path"),
        ("q = run('setx');", "Too many output arguments"),
    ] {
        let error = e.execute(source).unwrap_err();
        assert!(error.contains(message), "{source}: {error}");
    }
    e.execute("run('');").unwrap();
    assert!(matrust::session::FUNCTIONS.contains(&"run"));
}

#[test]
fn script_errors_report_the_script_line_and_keep_earlier_assignments() {
    let (_dir, mut e) = folder(&[
        ("bad.m", "a = 1;\nb = undefined_thing_q + 1;\n"),
        ("calls_bad.m", "c = 1;\nbad;"),
    ]);
    let error = e.execute("k = 1;\ncalls_bad;").unwrap_err();
    assert!(
        error.contains("In calls_bad (") && error.contains("In bad ("),
        "{error}"
    );
    assert!(error.contains("bad.m): Line 2, column"), "{error}");
    assert_eq!(e.diagnostic.as_ref().unwrap().line, 2);
    let file = e.file_diagnostic.as_ref().unwrap();
    assert!(file.path.ends_with("bad.m"));
    assert_eq!(file.diagnostic.line, 2);
    assert_eq!((scalar(&e, "a"), scalar(&e, "c")), (1., 1.));
    // A script parse error names the file.
    std::fs::write(_dir.path().join("broken.m"), "y = (;\n").unwrap();
    let error = e.execute("broken;").unwrap_err();
    assert!(error.contains("broken.m"), "{error}");
    assert!(
        e.file_diagnostic
            .as_ref()
            .unwrap()
            .path
            .ends_with("broken.m")
    );
}

#[test]
fn scripts_share_depth_step_size_and_cancellation_limits() {
    let (dir, mut e) = folder(&[
        ("again.m", "n = n + 1;\nagain;"),
        ("spin.m", "while true\nend"),
        ("busy.m", "while true\n  x = sin(ones(1, 65536));\nend"),
        (
            "deep.m",
            "function deep(k)\nif k > 0\n  deep(k - 1);\nelse\n  n = 0;\n  again;\nend\nend",
        ),
    ]);
    // Recursion stops at the shared depth limit, not a stack overflow.
    let error = e.execute("n = 0; again;").unwrap_err();
    assert!(error.contains("depth exceeds 32"), "{error}");
    assert_eq!(scalar(&e, "n"), 32.);
    let error = e.execute("deep(20);").unwrap_err();
    assert!(error.contains("depth exceeds 32"), "{error}");
    let error = e.execute("spin;").unwrap_err();
    assert!(error.contains("1000000"), "{error}");
    std::fs::write(dir.path().join("huge.m"), "%".repeat(1_000_001)).unwrap();
    let error = e.execute("huge;").unwrap_err();
    assert!(error.contains("at most 1 MB"), "{error}");
    let stop = e.cancellation();
    let thread = std::thread::spawn(move || {
        std::thread::sleep(std::time::Duration::from_millis(10));
        stop.store(true, Ordering::Relaxed);
    });
    let error = e.execute("keep = 11; busy;").unwrap_err();
    thread.join().unwrap();
    assert!(error.contains("cancelled"), "{error}");
    assert_eq!(scalar(&e, "keep"), 11.);
    e.cancellation().store(false, Ordering::Relaxed);
    e.execute("n = 0; again2 = 1;").unwrap();
}

#[test]
fn nested_runs_back_into_a_folder_share_one_file_cache() {
    // top -> sub/outer -> ../inner (top folder) -> sub/leaf (sub again).
    let (_dir, mut e) = folder(&[
        ("inner.m", "run('sub/leaf');"),
        ("sub/outer.m", "run('../inner');"),
        ("sub/leaf.m", "k = k + 1;"),
    ]);
    // 3 files; a cache lost on return would reload leaf each pass and hit
    // the 64-file limit well before 100 iterations.
    e.execute("k = 0; for i = 1:100\n  run('sub/outer');\nend")
        .unwrap();
    assert_eq!(scalar(&e, "k"), 100.);
    // Same through bare names after the nested runs.
    e.execute("k = 0; for i = 1:100\n  run('inner');\nend")
        .unwrap();
    assert_eq!(scalar(&e, "k"), 100.);
}
