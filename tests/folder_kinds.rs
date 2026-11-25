//! Folder kinds with their own lookup rule. Private folders: a function in
//! `D/private` is visible only to code of files in `D` (and to its private
//! siblings), after local functions and before the current folder and the
//! search path. Also: files changed on disk are seen by the next top-level
//! evaluation. Package folders: `+a` directly inside the current folder or a
//! search path folder is the package `a`, called as `a.fn(x)` or through the
//! import list of the running workspace. Class folders: `@Cls` directly
//! inside one of those folders (or a package folder) holds the classdef file
//! `Cls.m` and, each in its own file, further methods. Temporary folder trees
//! only.
use matrust::{Array, Engine};
use std::{
    fs,
    path::{Path, PathBuf},
};

fn folder() -> tempfile::TempDir {
    tempfile::tempdir().unwrap()
}
fn real(path: &Path) -> PathBuf {
    // Windows canonical paths carry a verbatim \\?\ prefix the engine never shows.
    let path = path.canonicalize().unwrap();
    match path.to_str().and_then(|p| p.strip_prefix(r"\\?\")) {
        Some(plain) => PathBuf::from(plain),
        None => path,
    }
}
fn quoted(path: &Path) -> String {
    format!("'{}'", path.to_string_lossy().replace('\'', "''"))
}
fn engine(current: &Path) -> Engine {
    let mut e = Engine::default();
    e.set_current_folder(current).unwrap();
    e
}
fn run(e: &mut Engine, code: &str) -> String {
    e.execute(code)
        .unwrap_or_else(|error| panic!("{code}: {error}"))
        .output
        .iter()
        .map(|s| format!("{s}\n"))
        .collect()
}
fn fails(e: &mut Engine, code: &str) -> String {
    match e.execute(code) {
        Ok(_) => panic!("{code}: expected an error"),
        Err(message) => message,
    }
}
fn text(e: &Engine, name: &str) -> String {
    match e.workspace().get(name).unwrap() {
        Array::Character(c) => String::from_utf16_lossy(c.units()),
        x => panic!("{name}: unexpected {}", x.class()),
    }
}
fn scalar(e: &Engine, name: &str) -> f64 {
    match e.workspace().get(name).unwrap() {
        Array::Numeric(m) => m.data()[0],
        Array::Logical(m) => f64::from(m.data()[0]),
        x => panic!("{name}: unexpected {}", x.class()),
    }
}
fn numbers(e: &Engine, name: &str) -> Vec<f64> {
    match e.workspace().get(name).unwrap() {
        Array::Numeric(m) => m.data().to_vec(),
        x => panic!("{name}: unexpected {}", x.class()),
    }
}
/// Write `text` to `dir/<part>/.../<file>`, creating the folders.
fn write(dir: &Path, parts: &[&str], text: &str) -> PathBuf {
    let (file, folders) = parts.split_last().unwrap();
    let mut path = dir.to_path_buf();
    for part in folders {
        path = path.join(part);
    }
    fs::create_dir_all(&path).unwrap();
    let path = path.join(file);
    fs::write(&path, text).unwrap();
    path
}
/// `root/tbx/f.m` (y = 10*h(x)) and its private helper `root/tbx/private/h.m` (x + 1).
fn toolbox(root: &Path) {
    write(
        root,
        &["tbx", "f.m"],
        "function y = f(x)\ny = 10 * h(x);\nend\n",
    );
    write(
        root,
        &["tbx", "private", "h.m"],
        "function y = h(x)\ny = x + 1;\nend\n",
    );
}
fn undefined(message: &str, name: &str) {
    assert!(
        message.contains(&format!("Undefined function or variable '{name}'")),
        "{message}"
    );
}

// a) The usual toolbox layout: genpath leaves private folders off the path,
// yet the toolbox function reaches its private helper.
#[test]
fn toolbox_function_calls_its_private_helper() {
    let root = folder();
    toolbox(root.path());
    write(
        root.path(),
        &["main.m"],
        "addpath(genpath('tbx'));\nr = f(2);\np = path;\n",
    );
    let mut e = engine(root.path());
    run(&mut e, "main");
    assert_eq!(scalar(&e, "r"), 30.);
    assert_eq!(
        text(&e, "p"),
        real(root.path()).join("tbx").to_string_lossy(),
        "genpath must not list the private folder"
    );
    // The path is kept; later lines and run('main') work the same way.
    run(&mut e, "r2 = f(3); run('main'); r3 = r + f(0);");
    assert_eq!(scalar(&e, "r2"), 40.);
    assert_eq!(scalar(&e, "r3"), 40.);
}

// b) No file runs at the Command Window: private functions are not visible
// there, even with the toolbox folder as the current folder.
#[test]
fn private_functions_are_hidden_from_the_command_window() {
    let root = folder();
    toolbox(root.path());
    write(
        root.path(),
        &["tbx", "probe.m"],
        "function [w, c, cf, cm] = probe()\nw = which('h');\nc = exist('h');\ncf = exist('h', 'file');\ncm = exist('h.m');\nwhich h\nend\n",
    );
    let tbx = root.path().join("tbx");
    let private_h = real(&tbx)
        .join("private")
        .join("h.m")
        .to_string_lossy()
        .into_owned();
    for inside in [false, true] {
        let mut e = engine(if inside { &tbx } else { root.path() });
        if !inside {
            run(&mut e, "addpath('tbx');");
        }
        undefined(&fails(&mut e, "h(2)"), "h");
        undefined(&fails(&mut e, "y = feval('h', 2);"), "h");
        undefined(&fails(&mut e, "k = @h; k(2)"), "h");
        let shown = run(&mut e, "which h");
        assert_eq!(shown.trim(), "'h' not found.", "inside={inside}");
        run(
            &mut e,
            "c = exist('h'); cf = exist('h', 'file'); w = which('h');",
        );
        assert_eq!(scalar(&e, "c"), 0.);
        assert_eq!(scalar(&e, "cf"), 0.);
        assert_eq!(text(&e, "w"), "");
        // Inside a file of the folder the same questions find the private file.
        let shown = run(&mut e, "[w, c, cf, cm] = probe();");
        assert_eq!(text(&e, "w"), private_h, "inside={inside}");
        assert_eq!(shown.trim(), private_h, "inside={inside}");
        assert_eq!(scalar(&e, "c"), 2.);
        assert_eq!(scalar(&e, "cf"), 2.);
        assert_eq!(scalar(&e, "cm"), 2.);
    }
}

// c) Two toolboxes with a private function of the same name never mix.
#[test]
fn each_folder_gets_its_own_private_function() {
    let root = folder();
    let r = root.path();
    write(r, &["A", "fa.m"], "function y = fa()\ny = h();\nend\n");
    write(
        r,
        &["A", "private", "h.m"],
        "function y = h()\ny = 1;\nend\n",
    );
    write(r, &["B", "fb.m"], "function y = fb()\ny = h();\nend\n");
    write(
        r,
        &["B", "private", "h.m"],
        "function y = h()\ny = 2;\nend\n",
    );
    // A function of another folder cannot see either of them.
    write(
        r,
        &["C", "fc.m"],
        "function y = fc()\ntry\n    y = h();\ncatch\n    y = -1;\nend\nend\n",
    );
    let mut e = engine(r);
    run(
        &mut e,
        "addpath('A', 'B', 'C'); r = [fa() fb() fc() fa() fb() fc()];",
    );
    assert_eq!(numbers(&e, "r"), [1., 2., -1., 1., 2., -1.]);
    run(&mut e, "r = [fb() fc() fa()];");
    assert_eq!(numbers(&e, "r"), [2., -1., 1.]);
}

// d) Private comes before the current folder, the search path and builtins,
// and after local functions; outside the folder nothing changes.
#[test]
fn private_function_shadows_path_functions_and_builtins() {
    let root = folder();
    let r = root.path();
    write(
        r,
        &["tbx", "f.m"],
        "function [a, b, c] = f()\na = h();\nb = max(3, 4);\nc = builtin('max', 3, 4);\nend\n",
    );
    write(
        r,
        &["tbx", "private", "h.m"],
        "function y = h()\ny = 'private';\nend\n",
    );
    write(
        r,
        &["tbx", "private", "max.m"],
        "function y = max(a, b)\ny = -(a + b);\nend\n",
    );
    // Same names in the current folder and on the path.
    write(r, &["h.m"], "function y = h()\ny = 'current';\nend\n");
    write(r, &["other", "h.m"], "function y = h()\ny = 'path';\nend\n");
    write(r, &["other", "g.m"], "function y = g()\ny = h();\nend\n");
    // A local function wins over the private one.
    write(
        r,
        &["tbx", "floc.m"],
        "function y = floc()\ny = h();\nend\nfunction y = h()\ny = 'local';\nend\n",
    );
    let mut e = engine(r);
    run(
        &mut e,
        "addpath('other', 'tbx'); [a, b, c] = f(); out = h(); m = max(3, 4); l = floc(); viag = g();",
    );
    assert_eq!(text(&e, "a"), "private");
    assert_eq!(scalar(&e, "b"), -7.);
    assert_eq!(scalar(&e, "c"), 4.);
    assert_eq!(text(&e, "out"), "current");
    assert_eq!(scalar(&e, "m"), 4.);
    assert_eq!(text(&e, "l"), "local");
    assert_eq!(text(&e, "viag"), "current");
    // Without the current-folder file the path file answers outside callers.
    fs::remove_file(r.join("h.m")).unwrap();
    run(&mut e, "[a, b] = f(); out = h(); viag = g();");
    assert_eq!(text(&e, "a"), "private");
    assert_eq!(scalar(&e, "b"), -7.);
    assert_eq!(text(&e, "out"), "path");
    assert_eq!(text(&e, "viag"), "path");
}

// e) Private siblings, a script as the caller, a command-syntax statement,
// feval and str2func inside the folder.
#[test]
fn private_siblings_scripts_and_name_based_calls() {
    let root = folder();
    let r = root.path();
    write(
        r,
        &["tbx", "private", "h.m"],
        "function y = h(x)\ny = g(x) + 1;\nend\n",
    );
    write(
        r,
        &["tbx", "private", "g.m"],
        "function y = g(x)\ny = 2 * x;\nend\n",
    );
    // Command syntax exists for a fixed set of command words; a private
    // function named like one of them takes the statement inside the folder.
    write(
        r,
        &["tbx", "private", "what.m"],
        "function what(t)\nfprintf('private what:%s\\n', t);\nend\n",
    );
    // A private script runs in the caller's workspace.
    write(
        r,
        &["tbx", "private", "setv.m"],
        "from_private_script = 77;\n",
    );
    write(r, &["tbx", "s.m"], "sv = h(5);\nwhat hello\nsetv\n");
    write(
        r,
        &["tbx", "byname.m"],
        "function [a, b, c, d] = byname()\na = feval('h', 1);\nk = str2func('h');\nb = k(2);\nc = cellfun(@h, {3, 4});\nk2 = str2func('@h');\nd = k2(5);\nend\n",
    );
    let mut e = engine(r);
    let shown = run(&mut e, "addpath('tbx'); s");
    assert_eq!(scalar(&e, "sv"), 11.);
    assert_eq!(scalar(&e, "from_private_script"), 77.);
    assert_eq!(shown.trim(), "private what:hello");
    run(&mut e, "[a, b, c, d] = byname();");
    assert_eq!(scalar(&e, "a"), 3.);
    assert_eq!(scalar(&e, "b"), 5.);
    assert_eq!(numbers(&e, "c"), [7., 9.]);
    assert_eq!(scalar(&e, "d"), 11.);
    // The private sibling is not visible one level up, nor the script.
    undefined(&fails(&mut e, "g(1)"), "g");
    undefined(&fails(&mut e, "setv"), "setv");
}

// e) A handle made inside the folder stays bound to the private file.
#[test]
fn handle_to_a_private_function_works_outside_its_folder() {
    let root = folder();
    let r = root.path();
    toolbox(r);
    write(r, &["tbx", "mk.m"], "function k = mk()\nk = @h;\nend\n");
    // A private function taking varargin, reached through a bound handle.
    write(
        r,
        &["tbx", "private", "count.m"],
        "function n = count(varargin)\nn = numel(varargin);\nend\n",
    );
    write(
        r,
        &["tbx", "mkc.m"],
        "function k = mkc()\nk = @count;\nend\n",
    );
    write(
        r,
        &["other", "apply.m"],
        "function y = apply(k, x)\ny = k(x) + feval(k, x);\nend\n",
    );
    // A function named h elsewhere must not capture the bound handle.
    write(r, &["other", "h.m"], "function y = h(x)\ny = -x;\nend\n");
    let mut e = engine(r);
    run(&mut e, "addpath('tbx', 'other'); k = mk(); r1 = k(4);");
    assert_eq!(scalar(&e, "r1"), 5.);
    // Later lines, another caller file, another current folder.
    run(
        &mut e,
        "r2 = k(1); r3 = apply(k, 10); r4 = cellfun(k, {1, 2}); s = func2str(k); info = functions(k); file = info.file; kind = info.type; plain = h(4);",
    );
    assert_eq!(scalar(&e, "r2"), 2.);
    assert_eq!(scalar(&e, "r3"), 22.);
    assert_eq!(numbers(&e, "r4"), [2., 3.]);
    assert_eq!(text(&e, "s"), "h");
    assert_eq!(text(&e, "kind"), "simple");
    assert_eq!(
        text(&e, "file"),
        real(r)
            .join("tbx")
            .join("private")
            .join("h.m")
            .to_string_lossy()
    );
    assert_eq!(scalar(&e, "plain"), -4.);
    run(
        &mut e,
        "c = mkc(); n = [c() c(1, 2, 3) feval(c, 'a', 'b')];",
    );
    assert_eq!(numbers(&e, "n"), [0., 3., 2.]);
    run(
        &mut e,
        "cd('other'); r5 = k(7); rmpath(fullfile('..', 'tbx')); r6 = k(8);",
    );
    assert_eq!(scalar(&e, "r5"), 8.);
    assert_eq!(scalar(&e, "r6"), 9.);
    // The bound file is gone: the handle no longer resolves.
    fs::remove_file(r.join("tbx").join("private").join("h.m")).unwrap();
    undefined(&fails(&mut e, "k(1)"), "h");
}

// A classdef file in a private folder: its constructor resolves like a
// private function.
#[test]
fn class_file_in_a_private_folder_constructs_inside_the_folder() {
    let root = folder();
    let r = root.path();
    write(
        r,
        &["tbx", "private", "Pt.m"],
        "classdef Pt\n    properties\n        v\n    end\n    methods\n        function obj = Pt(v)\n            obj.v = v;\n        end\n        function y = twice(obj)\n            y = 2 * obj.v;\n        end\n    end\nend\n",
    );
    write(
        r,
        &["tbx", "usept.m"],
        "function y = usept(x)\np = Pt(x);\ny = p.v + twice(p);\nend\n",
    );
    let mut e = engine(r);
    run(&mut e, "addpath('tbx'); y = usept(3);");
    assert_eq!(scalar(&e, "y"), 9.);
    undefined(&fails(&mut e, "p = Pt(3);"), "Pt");
}

// A file run as a document (CLI `matrust file.m`, desktop Run) is a running
// file: its folder's private functions are visible; plain text is not.
#[test]
fn document_run_sees_the_private_folder_of_its_file() {
    let root = folder();
    let r = root.path();
    toolbox(r);
    let source = "v = h(1);\nw = local(2);\nfunction y = local(x)\ny = h(x) + 100;\nend\n";
    let document = write(r, &["tbx", "demo.m"], source);
    let mut e = engine(r);
    e.execute_document(source, &document).unwrap();
    assert_eq!(scalar(&e, "v"), 2.);
    assert_eq!(scalar(&e, "w"), 103.);
    undefined(&fails(&mut e, source), "h");
}

// f) The next top-level evaluation sees files edited, created or deleted on
// disk, with no rehash; inside one evaluation the loaded file is kept until
// rehash, a path change or cd.
#[test]
fn changed_private_files_are_seen_by_the_next_evaluation() {
    let root = folder();
    let r = root.path();
    toolbox(r);
    write(
        r,
        &["tbx", "f2.m"],
        "function y = f2()\ntry\n    y = newp();\ncatch\n    y = -1;\nend\nend\n",
    );
    // A folder that has no private subfolder yet.
    write(
        r,
        &["bare", "f3.m"],
        "function y = f3()\ntry\n    y = late();\ncatch\n    y = -1;\nend\nend\n",
    );
    let h = r.join("tbx").join("private").join("h.m");
    let mut e = engine(r);
    run(
        &mut e,
        "addpath('tbx', 'bare'); r1 = f(2); m1 = f2(); m2 = f3();",
    );
    assert_eq!(scalar(&e, "r1"), 30.);
    assert_eq!(scalar(&e, "m1"), -1.);
    assert_eq!(scalar(&e, "m2"), -1.);
    // Edited: a different body and size.
    fs::write(
        &h,
        "function y = h(x)\n% edited helper\ny = x + 1000;\nend\n",
    )
    .unwrap();
    // Created after a miss, in an existing and in a new private folder.
    write(
        r,
        &["tbx", "private", "newp.m"],
        "function y = newp()\ny = 5;\nend\n",
    );
    write(
        r,
        &["bare", "private", "late.m"],
        "function y = late()\ny = 6;\nend\n",
    );
    run(&mut e, "r2 = f(2); m1 = f2(); m2 = f3();");
    assert_eq!(scalar(&e, "r2"), 10020.);
    assert_eq!(scalar(&e, "m1"), 5.);
    assert_eq!(scalar(&e, "m2"), 6.);
    // Deleted.
    fs::remove_file(&h).unwrap();
    undefined(&fails(&mut e, "r3 = f(2);"), "h");
    // A native .mr file added beside a .m file wins on the next evaluation.
    fs::write(&h, "function y = h(x)\ny = 1;\nend\n").unwrap();
    run(&mut e, "r4 = f(0);");
    assert_eq!(scalar(&e, "r4"), 10.);
    fs::write(h.with_extension("mr"), "function y = h(x)\ny = 2;\nend\n").unwrap();
    run(&mut e, "r5 = f(0);");
    assert_eq!(scalar(&e, "r5"), 20.);
}

// f) The same for the current folder and for search path folders.
#[test]
fn changed_folder_and_path_files_are_seen_by_the_next_evaluation() {
    let (cur, d) = (folder(), folder());
    let in_cur = cur.path().join("fcur.m");
    let on_path = d.path().join("fpath.m");
    fs::write(&in_cur, "function y = fcur()\ny = 1;\nend\n").unwrap();
    fs::write(&on_path, "function y = fpath()\ny = 2;\nend\n").unwrap();
    let mut e = engine(cur.path());
    let caught = "try; n = fnew(); catch; n = -1; end; try; p = pnew(); catch; p = -1; end";
    run(
        &mut e,
        &format!(
            "addpath({}); a = fcur(); b = fpath(); {caught}",
            quoted(d.path())
        ),
    );
    assert_eq!(numbers(&e, "a"), [1.]);
    assert_eq!(numbers(&e, "b"), [2.]);
    assert_eq!(scalar(&e, "n"), -1.);
    assert_eq!(scalar(&e, "p"), -1.);
    fs::write(&in_cur, "function y = fcur()\n% edited\ny = 10 + 1;\nend\n").unwrap();
    fs::write(
        &on_path,
        "function y = fpath()\n% edited\ny = 20 + 2;\nend\n",
    )
    .unwrap();
    fs::write(
        cur.path().join("fnew.m"),
        "function y = fnew()\ny = 3;\nend\n",
    )
    .unwrap();
    fs::write(
        d.path().join("pnew.m"),
        "function y = pnew()\ny = 4;\nend\n",
    )
    .unwrap();
    run(&mut e, &format!("a = fcur(); b = fpath(); {caught}"));
    assert_eq!(scalar(&e, "a"), 11.);
    assert_eq!(scalar(&e, "b"), 22.);
    assert_eq!(scalar(&e, "n"), 3.);
    assert_eq!(scalar(&e, "p"), 4.);
    fs::remove_file(&in_cur).unwrap();
    fs::remove_file(&on_path).unwrap();
    undefined(&fails(&mut e, "a = fcur();"), "fcur");
    undefined(&fails(&mut e, "b = fpath();"), "fpath");
}

// f) Inside one evaluation a loaded private file is kept (no file access per
// call); rehash, a path change, cd and clear functions each drop the private
// lookups too.
#[test]
fn rehash_path_change_and_cd_drop_private_lookups() {
    for drop_cache in [
        "rehash",
        "addpath('extra')",
        "cd('extra'); cd('..')",
        "clear functions",
    ] {
        let root = folder();
        let r = root.path();
        toolbox(r);
        fs::create_dir(r.join("extra")).unwrap();
        let h = r.join("tbx").join("private").join("h.m");
        let mut e = engine(r);
        run(
            &mut e,
            &format!(
                "addpath('tbx'); a = f(2);\nfid = fopen({}, 'w'); fprintf(fid, 'function y = h(x)\\ny = x + 500;\\nend\\n'); fclose(fid);\nb = f(2);\n{drop_cache};\nc = f(2);",
                quoted(&h)
            ),
        );
        assert_eq!(scalar(&e, "a"), 30., "{drop_cache}");
        assert_eq!(scalar(&e, "b"), 30., "{drop_cache}");
        assert_eq!(scalar(&e, "c"), 5020., "{drop_cache}");
    }
}

// g) Calls in a loop are answered from the per-evaluation cache: 1e5 calls of
// a path function, and of a private function from inside its folder, stay far
// from the time 1e5 file lookups would take.
#[test]
fn repeated_calls_do_not_touch_the_disk() {
    let root = folder();
    let r = root.path();
    toolbox(r);
    write(
        r,
        &["tbx", "inc.m"],
        "function y = inc(x)\ny = x + 1;\nend\n",
    );
    write(
        r,
        &["tbx", "loop_private.m"],
        "function s = loop_private(n)\ns = 0;\nfor k = 1:n\n    s = s + h(k) - k + abs(-1) - 1;\nend\nend\n",
    );
    let mut e = Engine::with_limits(matrust::engine::Limits::local());
    e.set_current_folder(r).unwrap();
    let start = std::time::Instant::now();
    run(
        &mut e,
        "addpath('tbx'); s = 0; for k = 1:1e5, s = inc(s); end; t = loop_private(1e5);",
    );
    let elapsed = start.elapsed();
    assert_eq!(scalar(&e, "s"), 1e5);
    assert_eq!(scalar(&e, "t"), 1e5);
    assert!(
        elapsed < std::time::Duration::from_secs(20),
        "2e5 cached calls took {elapsed:?}"
    );
}

// ----- 9b: package folders and import -----

/// The package `a` inside `dir`: functions `fn` (through the package's own
/// private helper), `two`, `noargs`, `inc`, `sibling`, `qualified`, the
/// script `s`, the class `Cls` and the sub-package `a.b` with `g`.
fn package(dir: &Path) {
    write(
        dir,
        &["+a", "fn.m"],
        "function y = fn(x)\n% FN adds one hundred to twice its input.\n%   y = a.fn(x)\ny = h(x) + 100;\nend\n",
    );
    write(
        dir,
        &["+a", "private", "h.m"],
        "function y = h(x)\ny = 2 * x;\nend\n",
    );
    write(
        dir,
        &["+a", "two.m"],
        "function [p, q] = two(x)\np = x;\nq = 10 * x;\nend\n",
    );
    write(
        dir,
        &["+a", "noargs.m"],
        "function y = noargs()\ny = 42;\nend\n",
    );
    write(
        dir,
        &["+a", "inc.m"],
        "function y = inc(x)\ny = x + 1;\nend\n",
    );
    // A bare call of another function of the same package does not resolve.
    write(
        dir,
        &["+a", "sibling.m"],
        "function y = sibling(x)\ny = fn(x);\nend\n",
    );
    write(
        dir,
        &["+a", "qualified.m"],
        "function y = qualified(x)\ny = a.fn(x) + 1;\nend\n",
    );
    write(dir, &["+a", "s.m"], "from_package_script = 5;\n");
    write(
        dir,
        &["+a", "+b", "g.m"],
        "function [p, q] = g(x)\np = x + 1;\nq = x + 2;\nend\n",
    );
    write(
        dir,
        &["+a", "Cls.m"],
        "classdef Cls\n    properties\n        v\n    end\n    properties (Constant)\n        K = 9;\n    end\n    methods\n        function obj = Cls(v)\n            if nargin > 0\n                obj.v = v;\n            end\n        end\n        function y = twice(obj)\n            y = 2 * obj.v;\n        end\n    end\n    methods (Static)\n        function y = triple(x)\n            y = 3 * x;\n        end\n        function [p, q] = pair(x)\n            p = x;\n            q = -x;\n        end\n        function obj = make(v)\n            obj = a.Cls(v + 1);\n        end\n    end\nend\n",
    );
}
fn unresolved(message: &str, name: &str) {
    assert!(
        message.contains(&format!("Unable to resolve the name '{name}'.")),
        "{message}"
    );
}
fn not_importable(message: &str, entry: &str) {
    assert!(
        message.contains(&format!(
            "Import argument '{entry}' cannot be found or cannot be imported."
        )),
        "{message}"
    );
}

// Qualified calls: functions, sub-packages, several outputs, no parentheses,
// statements, a package script, and the private folder of the package.
#[test]
fn package_functions_are_called_by_qualified_name() {
    let root = folder();
    let r = root.path();
    package(r);
    let mut e = engine(r);
    run(
        &mut e,
        "r1 = a.fn(1); r2 = a.b.g(1); [p, q] = a.b.g(1); [p2, q2] = a.two(4); [~, q3] = a.two(5);",
    );
    assert_eq!(scalar(&e, "r1"), 102.);
    assert_eq!(scalar(&e, "r2"), 2.);
    assert_eq!((scalar(&e, "p"), scalar(&e, "q")), (2., 3.));
    assert_eq!((scalar(&e, "p2"), scalar(&e, "q2")), (4., 40.));
    assert_eq!(scalar(&e, "q3"), 50.);
    // No parentheses, inside expressions, as arguments and in lists.
    run(
        &mut e,
        "r3 = a.noargs; r4 = a.noargs + 1; r5 = [a.noargs, a.fn(0)]; r6 = a.fn(a.b.g(1)); c = {a.noargs, a.fn(1)}; r7 = c{1} + c{2}; r8 = a.qualified(1); r9 = abs(-a.noargs);",
    );
    assert_eq!(scalar(&e, "r3"), 42.);
    assert_eq!(scalar(&e, "r4"), 43.);
    assert_eq!(numbers(&e, "r5"), [42., 100.]);
    assert_eq!(scalar(&e, "r6"), 104.);
    assert_eq!(scalar(&e, "r7"), 144.);
    assert_eq!(scalar(&e, "r8"), 103.);
    assert_eq!(scalar(&e, "r9"), 42.);
    // Statements: the result goes to ans; a script runs in this workspace.
    let shown = run(&mut e, "a.noargs");
    assert!(shown.contains("ans") && shown.contains("42"), "{shown}");
    assert_eq!(scalar(&e, "ans"), 42.);
    assert_eq!(run(&mut e, "a.fn(2);"), "");
    assert_eq!(scalar(&e, "ans"), 104.);
    run(&mut e, "a.b.g(7);");
    assert_eq!(scalar(&e, "ans"), 8.);
    run(&mut e, "a.s");
    assert_eq!(scalar(&e, "from_package_script"), 5.);
    run(&mut e, "clear from_package_script; a.s();");
    assert_eq!(scalar(&e, "from_package_script"), 5.);
    // The package's private helper is not visible from outside the package.
    undefined(&fails(&mut e, "h(1)"), "h");
    // A function of a path folder calls package functions the same way.
    write(
        r,
        &["tbx", "usepkg.m"],
        "function [y, z] = usepkg(x)\ny = a.fn(x);\n[~, z] = a.b.g(x);\nend\n",
    );
    run(&mut e, "addpath('tbx'); [y, z] = usepkg(3);");
    assert_eq!((scalar(&e, "y"), scalar(&e, "z")), (106., 5.));
}

// Unknown names: in an existing package the reference text; without a
// package and without a variable the usual undefined error.
#[test]
fn unknown_package_names_report_the_qualified_name() {
    let root = folder();
    let r = root.path();
    package(r);
    let mut e = engine(r);
    unresolved(&fails(&mut e, "a.nothere(1)"), "a.nothere");
    unresolved(&fails(&mut e, "a.nothere"), "a.nothere");
    unresolved(&fails(&mut e, "x = a.nothere;"), "a.nothere");
    unresolved(
        &fails(&mut e, "[p, q] = a.Cls.missing(1);"),
        "a.Cls.missing",
    );
    unresolved(&fails(&mut e, "x = a.b.nothere(1) + 1;"), "a.b.nothere");
    unresolved(&fails(&mut e, "[p, q] = a.nothere(1);"), "a.nothere");
    let message = fails(&mut e, "x = a.Cls.nothere(1);");
    assert!(
        message
            .contains("The class a.Cls has no Constant property or Static method named 'nothere'."),
        "{message}"
    );
    undefined(&fails(&mut e, "zz.nothere(1)"), "zz");
    undefined(&fails(&mut e, "x = zz.nothere;"), "zz");
    undefined(&fails(&mut e, "feval('a.nothere', 1)"), "a.nothere");
    // A script is not a function.
    let message = fails(&mut e, "x = a.s;");
    assert!(
        message.contains("Attempt to execute SCRIPT a.s as a function"),
        "{message}"
    );
    // Inside a package function a sibling needs its qualified name.
    undefined(&fails(&mut e, "y = a.sibling(1);"), "fn");
    // Package folders are not on the path: no unqualified call from outside.
    undefined(&fails(&mut e, "y = fn(1);"), "fn");
    undefined(&fails(&mut e, "y = g(1);"), "g");
}

// A classdef file directly in the package: constructor, methods of the
// object in later lines, static methods and Constant properties.
#[test]
fn package_class_constructs_and_calls_static_methods() {
    let root = folder();
    let r = root.path();
    package(r);
    let mut e = engine(r);
    run(
        &mut e,
        "p = a.Cls(3); v = p.v; t = p.twice(); c = class(p); s = a.Cls.triple(2); k = a.Cls.K; is = isa(p, 'a.Cls'); d = a.Cls; dv = isempty(d.v);",
    );
    assert_eq!(scalar(&e, "v"), 3.);
    assert_eq!(scalar(&e, "t"), 6.);
    assert_eq!(text(&e, "c"), "a.Cls");
    assert_eq!(scalar(&e, "s"), 6.);
    assert_eq!(scalar(&e, "k"), 9.);
    assert_eq!(scalar(&e, "is"), 1.);
    assert_eq!(scalar(&e, "dv"), 1.);
    // The object keeps working in later lines, from any folder on the path.
    run(
        &mut e,
        "w = p.v + twice(p); q = a.Cls.make(4); qv = q.v; [m, n] = a.Cls.pair(5); p.v = 10; w2 = p.twice();",
    );
    assert_eq!(scalar(&e, "w"), 9.);
    assert_eq!(scalar(&e, "qv"), 5.);
    assert_eq!((scalar(&e, "m"), scalar(&e, "n")), (5., -5.));
    assert_eq!(scalar(&e, "w2"), 20.);
    let shown = run(&mut e, "a.Cls.triple(4)");
    assert!(shown.contains("12"), "{shown}");
    let shown = run(&mut e, "p");
    assert!(shown.contains("Cls with properties"), "{shown}");
    // Imported, the class is used by its plain name.
    run(
        &mut e,
        "import a.Cls; p2 = Cls(7); v2 = p2.v; s2 = Cls.triple(3); c2 = class(p2);",
    );
    assert_eq!(scalar(&e, "v2"), 7.);
    assert_eq!(scalar(&e, "s2"), 9.);
    assert_eq!(text(&e, "c2"), "a.Cls");
    run(&mut e, "clear import");
    undefined(&fails(&mut e, "p3 = Cls(7);"), "Cls");
}

// One package in several folders: the names merge; per name the first folder
// in lookup order (current folder, then the path) wins.
#[test]
fn package_spread_over_folders_merges_in_lookup_order() {
    let root = folder();
    let r = root.path();
    for (dir, own) in [("t1", "f1"), ("t2", "f2")] {
        write(
            r,
            &[dir, "+a", &format!("{own}.m")],
            &format!("function y = {own}()\ny = '{dir}';\nend\n"),
        );
        write(
            r,
            &[dir, "+a", "both.m"],
            &format!("function y = both()\ny = '{dir}';\nend\n"),
        );
        write(
            r,
            &[dir, "+a", "+b", &format!("{own}.m")],
            &format!("function y = {own}()\ny = 'sub {dir}';\nend\n"),
        );
    }
    let mut e = engine(r);
    run(
        &mut e,
        "addpath('t1', 't2'); x1 = a.f1(); x2 = a.f2(); xb = a.both(); s1 = a.b.f1(); s2 = a.b.f2(); w = which('a.both'); every = which('a.both', '-all'); n = numel(every); second = every{2};",
    );
    assert_eq!(text(&e, "x1"), "t1");
    assert_eq!(text(&e, "x2"), "t2");
    assert_eq!(text(&e, "xb"), "t1");
    assert_eq!(text(&e, "s1"), "sub t1");
    assert_eq!(text(&e, "s2"), "sub t2");
    let both = |dir: &str| {
        real(r)
            .join(dir)
            .join("+a")
            .join("both.m")
            .to_string_lossy()
            .into_owned()
    };
    assert_eq!(text(&e, "w"), both("t1"));
    assert_eq!(scalar(&e, "n"), 2.);
    assert_eq!(text(&e, "second"), both("t2"));
    // A path change reorders the folders, inside one evaluation too.
    run(
        &mut e,
        "before = a.both(); addpath('t2'); after = a.both(); w = which('a.both');",
    );
    assert_eq!(text(&e, "before"), "t1");
    assert_eq!(text(&e, "after"), "t2");
    assert_eq!(text(&e, "w"), both("t2"));
    // The current folder comes before the path.
    write(
        r,
        &["+a", "both.m"],
        "function y = both()\ny = 'current';\nend\n",
    );
    run(&mut e, "xb = a.both(); x1 = a.f1();");
    assert_eq!(text(&e, "xb"), "current");
    assert_eq!(text(&e, "x1"), "t1");
    // cd moves the lookup with it: t1 is now the current folder, before t2.
    run(&mut e, "cd('t1'); xb = a.both(); cd('..'); xc = a.both();");
    assert_eq!(text(&e, "xb"), "t1");
    assert_eq!(text(&e, "xc"), "current");
}

// Variables always win: a struct named like the package keeps field access.
#[test]
fn variable_named_like_a_package_keeps_field_access() {
    let root = folder();
    let r = root.path();
    package(r);
    write(
        r,
        &["shadow.m"],
        "function [x, y] = shadow(a)\nx = a.fn;\ny = a.b.g;\nend\n",
    );
    let mut e = engine(r);
    run(
        &mut e,
        "a = struct('fn', 7, 'b', struct('g', 8)); r1 = a.fn; r2 = a.b.g; a.fn = 70; r3 = a.fn;",
    );
    assert_eq!(scalar(&e, "r1"), 7.);
    assert_eq!(scalar(&e, "r2"), 8.);
    assert_eq!(scalar(&e, "r3"), 70.);
    let message = fails(&mut e, "a.noargs");
    assert!(message.contains("non-existent field 'noargs'"), "{message}");
    run(
        &mut e,
        "clear a; r4 = a.fn(1); [x, y] = shadow(struct('fn', 1, 'b', struct('g', 2)));",
    );
    assert_eq!(scalar(&e, "r4"), 102.);
    assert_eq!((scalar(&e, "x"), scalar(&e, "y")), (1., 2.));
}

// which, exist, help, feval, str2func and @ see the same file as a call.
#[test]
fn name_based_lookups_agree_on_package_functions() {
    let root = folder();
    let r = root.path();
    package(r);
    let mut e = engine(r);
    let file = |parts: &[&str]| {
        let mut path = real(r);
        for part in parts {
            path = path.join(part);
        }
        path.to_string_lossy().into_owned()
    };
    run(
        &mut e,
        "w = which('a.fn'); wg = which('a.b.g'); wc = which('a.Cls'); w0 = which('a.nothere'); every = which('a.fn', '-all'); n = numel(every); first = every{1};",
    );
    assert_eq!(text(&e, "w"), file(&["+a", "fn.m"]));
    assert_eq!(text(&e, "wg"), file(&["+a", "+b", "g.m"]));
    assert_eq!(text(&e, "wc"), file(&["+a", "Cls.m"]));
    assert_eq!(text(&e, "w0"), "");
    assert_eq!(scalar(&e, "n"), 1.);
    assert_eq!(text(&e, "first"), file(&["+a", "fn.m"]));
    assert_eq!(run(&mut e, "which a.fn").trim(), file(&["+a", "fn.m"]));
    assert_eq!(
        run(&mut e, "which a.nothere").trim(),
        "'a.nothere' not found."
    );
    run(
        &mut e,
        "e1 = exist('a.fn'); e2 = exist('a.fn', 'file'); e3 = exist('a.b.g'); e4 = exist('a.nothere'); e5 = exist('a.h'); e6 = exist('zz.fn');",
    );
    assert_eq!(scalar(&e, "e1"), 2.);
    assert_eq!(scalar(&e, "e2"), 2.);
    assert_eq!(scalar(&e, "e3"), 2.);
    assert_eq!(scalar(&e, "e4"), 0.);
    assert_eq!(scalar(&e, "e5"), 0.);
    assert_eq!(scalar(&e, "e6"), 0.);
    let help = run(&mut e, "help a.fn");
    assert_eq!(
        help.lines().map(str::trim).collect::<Vec<_>>(),
        ["FN adds one hundred to twice its input.", "y = a.fn(x)"]
    );
    assert_eq!(run(&mut e, "help('a.fn')"), help);
    let help = run(&mut e, "help a.b.g");
    assert!(help.contains("No help found for a.b.g.m."), "{help}");
    assert!(e.execute("help a.nothere").is_err());
    run(
        &mut e,
        "f1 = feval('a.fn', 1); k = str2func('a.fn'); f2 = k(1); k2 = @a.fn; f3 = k2(1); s = func2str(k2); f4 = cellfun(@a.fn, {1, 2}); [p, q] = feval('a.b.g', 1); k3 = @a.b.g; [p2, q2] = k3(1); k4 = str2func('@a.fn'); f5 = k4(2); f6 = arrayfun(@a.inc, [1 2 3]); f7 = feval(@a.noargs); f8 = cellfun('a.inc', {5});",
    );
    assert_eq!(scalar(&e, "f1"), 102.);
    assert_eq!(scalar(&e, "f2"), 102.);
    assert_eq!(scalar(&e, "f3"), 102.);
    assert_eq!(text(&e, "s"), "a.fn");
    assert_eq!(numbers(&e, "f4"), [102., 104.]);
    assert_eq!((scalar(&e, "p"), scalar(&e, "q")), (2., 3.));
    assert_eq!((scalar(&e, "p2"), scalar(&e, "q2")), (2., 3.));
    assert_eq!(scalar(&e, "f5"), 104.);
    assert_eq!(numbers(&e, "f6"), [2., 3., 4.]);
    assert_eq!(scalar(&e, "f7"), 42.);
    assert_eq!(scalar(&e, "f8"), 6.);
    // A handle resolves by its qualified name wherever it is called.
    write(
        r,
        &["other", "apply.m"],
        "function y = apply(k, x)\ny = k(x);\nend\n",
    );
    run(&mut e, "addpath('other'); f9 = apply(@a.fn, 3);");
    assert_eq!(scalar(&e, "f9"), 106.);
    undefined(&fails(&mut e, "k5 = @a.nothere; k5(1)"), "a.nothere");
}

// import a.* at the Command Window lasts until clear import; inside a
// function it lasts for that call; a script adds to its caller's list.
#[test]
fn wildcard_import_follows_the_workspace_that_runs_it() {
    let root = folder();
    let r = root.path();
    package(r);
    write(
        r,
        &["useimp.m"],
        "function [y, n, first] = useimp(x)\nimport a.*\ny = fn(x) + noargs;\nL = import;\nn = numel(L);\nfirst = L{1};\nend\n",
    );
    write(
        r,
        &["noimp.m"],
        "function [y, n] = noimp()\nn = numel(import);\ntry\n    y = fn(1);\ncatch\n    y = -1;\nend\nend\n",
    );
    write(r, &["impscript.m"], "import a.b.*\n");
    write(
        r,
        &["viascript.m"],
        "function [y, n] = viascript()\nimpscript\ny = g(1);\nn = numel(import);\nend\n",
    );
    let mut e = engine(r);
    // A function's list is its own and is dropped on return.
    run(&mut e, "[y, n, first] = useimp(1); n0 = numel(import);");
    assert_eq!(scalar(&e, "y"), 144.);
    assert_eq!(scalar(&e, "n"), 1.);
    assert_eq!(text(&e, "first"), "a.*");
    assert_eq!(scalar(&e, "n0"), 0.);
    undefined(&fails(&mut e, "fn(1)"), "fn");
    // The Command Window list lasts over lines and is not seen by functions.
    run(&mut e, "import a.*");
    run(
        &mut e,
        "r1 = fn(1); r2 = noargs; [p, q] = two(3); [y, n] = noimp();",
    );
    assert_eq!(scalar(&e, "r1"), 102.);
    assert_eq!(scalar(&e, "r2"), 42.);
    assert_eq!((scalar(&e, "p"), scalar(&e, "q")), (3., 30.));
    assert_eq!(scalar(&e, "y"), -1.);
    assert_eq!(scalar(&e, "n"), 0.);
    // Statements and a package script through the list.
    let shown = run(&mut e, "noargs");
    assert!(shown.contains("42"), "{shown}");
    run(&mut e, "s");
    assert_eq!(scalar(&e, "from_package_script"), 5.);
    // which and exist follow the list; a sub-package needs its own entry.
    run(&mut e, "w = which('fn'); c = exist('fn');");
    assert_eq!(
        text(&e, "w"),
        real(r).join("+a").join("fn.m").to_string_lossy()
    );
    assert_eq!(scalar(&e, "c"), 2.);
    undefined(&fails(&mut e, "g(1)"), "g");
    run(&mut e, "clear import");
    undefined(&fails(&mut e, "fn(1)"), "fn");
    run(&mut e, "n0 = numel(import); c = exist('fn');");
    assert_eq!(scalar(&e, "n0"), 0.);
    assert_eq!(scalar(&e, "c"), 0.);
    // A script's import lands in the workspace it runs in.
    run(&mut e, "[y, n] = viascript(); n0 = numel(import);");
    assert_eq!(scalar(&e, "y"), 2.);
    assert_eq!(scalar(&e, "n"), 1.);
    assert_eq!(scalar(&e, "n0"), 0.);
    run(&mut e, "impscript");
    run(&mut e, "y = g(4);");
    assert_eq!(scalar(&e, "y"), 5.);
    // clear import inside a function is refused; the list is unchanged.
    write(r, &["clr.m"], "function clr()\nclear import\nend\n");
    let message = fails(&mut e, "clr()");
    assert!(message.contains("clear import"), "{message}");
    run(&mut e, "y = g(5);");
    assert_eq!(scalar(&e, "y"), 6.);
}

// Precedence: variables, explicit imports, local functions, wildcard
// imports, private functions, the current folder, the path, builtins.
#[test]
fn import_precedence_and_builtin_shadowing() {
    let root = folder();
    let r = root.path();
    package(r);
    write(
        r,
        &["+a", "max.m"],
        "function y = max(varargin)\ny = -1;\nend\n",
    );
    // Local functions: after an explicit import, before a wildcard import.
    write(
        r,
        &["prec.m"],
        "function [e, w, k1, k2] = prec()\nimport a.fn\ne = fn(1);\nimport a.b.*\nw = g(1);\nk1 = func2str(@fn);\nk2 = func2str(@g);\nend\nfunction y = fn(x)\ny = -1;\nend\nfunction y = g(x)\ny = -5;\nend\n",
    );
    // Wildcard imports: before private functions, the current folder and the path.
    write(
        r,
        &["private", "inc.m"],
        "function y = inc(x)\ny = -3;\nend\n",
    );
    write(r, &["fn.m"], "function y = fn(x)\ny = -7;\nend\n");
    write(
        r,
        &["other", "noargs.m"],
        "function y = noargs()\ny = -9;\nend\n",
    );
    write(
        r,
        &["prec2.m"],
        "function [p0, c0, t0, p1, c1, t1] = prec2()\np0 = inc(1);\nc0 = fn(1);\nt0 = noargs();\nimport a.*\np1 = inc(1);\nc1 = fn(1);\nt1 = noargs();\nend\n",
    );
    let mut e = engine(r);
    run(
        &mut e,
        "addpath('other'); [e1, w1, k1, k2] = prec(); [p0, c0, t0, p1, c1, t1] = prec2();",
    );
    assert_eq!(scalar(&e, "e1"), 102.);
    assert_eq!(scalar(&e, "w1"), -5.);
    assert_eq!(text(&e, "k1"), "a.fn");
    assert_eq!(text(&e, "k2"), "g");
    assert_eq!(scalar(&e, "p0"), -3.);
    assert_eq!(scalar(&e, "c0"), -7.);
    assert_eq!(scalar(&e, "t0"), -9.);
    assert_eq!(scalar(&e, "p1"), 2.);
    assert_eq!(scalar(&e, "c1"), 102.);
    assert_eq!(scalar(&e, "t1"), 42.);
    // An imported name shadows a builtin, explicitly or by wildcard.
    run(
        &mut e,
        "m0 = max(3, 4); import a.max; m1 = max(3, 4); b1 = builtin('max', 3, 4);",
    );
    assert_eq!(scalar(&e, "m0"), 4.);
    assert_eq!(scalar(&e, "m1"), -1.);
    assert_eq!(scalar(&e, "b1"), 4.);
    run(
        &mut e,
        "m2 = max(3, 4); clear import; m3 = max(3, 4); import a.*; m4 = max(3, 4);",
    );
    assert_eq!(scalar(&e, "m2"), -1.);
    assert_eq!(scalar(&e, "m3"), 4.);
    assert_eq!(scalar(&e, "m4"), -1.);
    // A variable wins over an imported name.
    run(
        &mut e,
        "clear import; import a.fn; fn = [5 6 7]; v = fn(1); clear fn; f = fn(1);",
    );
    assert_eq!(scalar(&e, "v"), 5.);
    assert_eq!(scalar(&e, "f"), 102.);
    // A handle made to an imported name keeps the qualified name.
    run(
        &mut e,
        "k = @fn; s = func2str(k); clear import; kv = k(1); cv = fn(1);",
    );
    assert_eq!(text(&e, "s"), "a.fn");
    assert_eq!(scalar(&e, "kv"), 102.);
    assert_eq!(scalar(&e, "cv"), -7.);
}

// The import list: several entries, the function form, L = import, the bare
// statement, duplicates, and the error for names that do not exist.
#[test]
fn import_list_forms_and_errors() {
    let root = folder();
    let r = root.path();
    package(r);
    let mut e = engine(r);
    run(
        &mut e,
        "L0 = import; n0 = numel(L0); s0 = size(L0); isc = iscell(L0);",
    );
    assert_eq!(scalar(&e, "n0"), 0.);
    assert_eq!(numbers(&e, "s0"), [0., 1.]);
    assert_eq!(scalar(&e, "isc"), 1.);
    assert_eq!(run(&mut e, "import"), "");
    assert_eq!(run(&mut e, "import a.* a.b.g"), "");
    run(
        &mut e,
        "L = import; s = size(L); l1 = L{1}; l2 = L{2}; ok = iscellstr(L); r1 = fn(1); r2 = g(1);",
    );
    assert_eq!(numbers(&e, "s"), [2., 1.]);
    assert_eq!(text(&e, "l1"), "a.*");
    assert_eq!(text(&e, "l2"), "a.b.g");
    assert_eq!(scalar(&e, "ok"), 1.);
    assert_eq!(scalar(&e, "r1"), 102.);
    assert_eq!(scalar(&e, "r2"), 2.);
    let shown = run(&mut e, "import");
    assert_eq!(
        shown.lines().map(str::trim).collect::<Vec<_>>(),
        ["a.*", "a.b.g"]
    );
    // The function form; an entry already listed is not added twice.
    run(
        &mut e,
        "clear import; import('a.fn'); import('a.b.*', 'a.fn'); L = import('a.fn'); n = numel(L); l1 = L{1}; l2 = L{2}; r1 = fn(2); [p, q] = g(1);",
    );
    assert_eq!(scalar(&e, "n"), 2.);
    assert_eq!(text(&e, "l1"), "a.fn");
    assert_eq!(text(&e, "l2"), "a.b.*");
    assert_eq!(scalar(&e, "r1"), 104.);
    assert_eq!((scalar(&e, "p"), scalar(&e, "q")), (2., 3.));
    // Names that do not exist; nothing is added by a failing import.
    run(&mut e, "clear import");
    not_importable(&fails(&mut e, "import a.nothere"), "a.nothere");
    not_importable(&fails(&mut e, "import a.b.nothere"), "a.b.nothere");
    not_importable(&fails(&mut e, "import zz.fn"), "zz.fn");
    not_importable(&fails(&mut e, "import zz.*"), "zz.*");
    not_importable(&fails(&mut e, "import java.util.*"), "java.util.*");
    not_importable(
        &fails(&mut e, "import java.util.HashMap"),
        "java.util.HashMap",
    );
    not_importable(&fails(&mut e, "import('fn')"), "fn");
    not_importable(&fails(&mut e, "import a.fn a.nothere"), "a.nothere");
    run(&mut e, "n = numel(import);");
    assert_eq!(scalar(&e, "n"), 0.);
    // A variable named import is a variable.
    run(
        &mut e,
        "import = 3; v = import + 1; clear import; n = numel(import);",
    );
    assert_eq!(scalar(&e, "v"), 4.);
    assert_eq!(scalar(&e, "n"), 0.);
}

// Without any folder (as in the browser build) the import list still lists
// and clears, and package names are undefined.
#[test]
fn import_without_a_folder_lists_and_clears() {
    let mut e = Engine::default();
    run(
        &mut e,
        "L = import; n = numel(L); clear import; n2 = numel(import);",
    );
    assert_eq!(scalar(&e, "n"), 0.);
    assert_eq!(scalar(&e, "n2"), 0.);
    assert_eq!(run(&mut e, "import"), "");
    not_importable(&fails(&mut e, "import a.*"), "a.*");
    not_importable(&fails(&mut e, "import a.fn"), "a.fn");
    undefined(&fails(&mut e, "a.fn(1)"), "a");
    undefined(&fails(&mut e, "y = feval('a.fn', 1);"), "a.fn");
    run(&mut e, "c = exist('a.fn'); w = which('a.fn');");
    assert_eq!(scalar(&e, "c"), 0.);
    assert_eq!(text(&e, "w"), "");
}

// clear all empties the import list from the command prompt only (MathWorks
// clear page, ItemType table, row all): not inside a script, a function or a
// document run.
#[test]
fn clear_all_keeps_the_import_list_inside_scripts_and_functions() {
    let dir = folder();
    write(
        dir.path(),
        &["+pk", "fn.m"],
        "function y = fn(x)\ny = 1000*x;\nend\n",
    );
    let script = "import pk.fn\nclear all; close all; clc\ndisp(fn(2))\n";
    let s = write(dir.path(), &["s.m"], script);
    write(
        dir.path(),
        &["f.m"],
        "function r = f()\nimport pk.fn\nclear all\nr = fn(2);\nend\n",
    );
    write(dir.path(), &["s2.m"], "clear all\ndisp(fn(2))\n");
    let mut e = engine(dir.path());
    assert!(run(&mut e, "s").contains("2000"));
    // The script's import is the base list: the Command Window clears it.
    undefined(&fails(&mut e, "clear all; y = fn(2);"), "fn");
    let mut e = engine(dir.path());
    assert!(run(&mut e, "disp(f())").contains("2000"));
    let mut e = engine(dir.path());
    assert!(run(&mut e, "import pk.fn; s2").contains("2000"));
    let mut e = engine(dir.path());
    let output = e.execute_document(script, &s).unwrap().output.join("\n");
    assert!(output.contains("2000"), "{output}");
    let mut e = engine(dir.path());
    undefined(&fails(&mut e, "import pk.fn; clear all; y = fn(2);"), "fn");
}

// Caches: a file added to, edited in or deleted from a package folder, and a
// new package folder, are seen by the next top-level evaluation; inside one
// evaluation the loaded file is kept until rehash, a path change, cd or
// clear functions.
#[test]
fn changed_package_files_are_seen_by_the_next_evaluation() {
    let root = folder();
    let r = root.path();
    package(r);
    write(
        r,
        &["viaimport.m"],
        "function y = viaimport()\nimport a.*\ntry\n    y = late(1);\ncatch\n    y = -1;\nend\nend\n",
    );
    let mut e = engine(r);
    run(&mut e, "r1 = a.fn(1); m1 = viaimport();");
    assert_eq!(scalar(&e, "r1"), 102.);
    assert_eq!(scalar(&e, "m1"), -1.);
    unresolved(&fails(&mut e, "a.late(1)"), "a.late");
    undefined(&fails(&mut e, "c.f()"), "c");
    let late = write(
        r,
        &["+a", "late.m"],
        "function y = late(x)\ny = x + 1;\nend\n",
    );
    write(r, &["+c", "f.m"], "function y = f()\ny = 11;\nend\n");
    write(r, &["+a", "+n", "f.m"], "function y = f()\ny = 12;\nend\n");
    run(
        &mut e,
        "r2 = a.late(1); m2 = viaimport(); r3 = c.f(); r4 = a.n.f();",
    );
    assert_eq!(scalar(&e, "r2"), 2.);
    assert_eq!(scalar(&e, "m2"), 2.);
    assert_eq!(scalar(&e, "r3"), 11.);
    assert_eq!(scalar(&e, "r4"), 12.);
    fs::write(
        &late,
        "function y = late(x)\n% edited\ny = x + 1000;\nend\n",
    )
    .unwrap();
    run(&mut e, "r2 = a.late(1); m2 = viaimport();");
    assert_eq!(scalar(&e, "r2"), 1001.);
    assert_eq!(scalar(&e, "m2"), 1001.);
    fs::remove_file(&late).unwrap();
    unresolved(&fails(&mut e, "a.late(1)"), "a.late");
    run(&mut e, "m2 = viaimport();");
    assert_eq!(scalar(&e, "m2"), -1.);
    // Inside one evaluation: kept until the lookups are dropped.
    for drop_cache in [
        "rehash",
        "addpath('extra')",
        "cd('extra'); cd('..')",
        "clear functions",
    ] {
        let root = folder();
        let r = root.path();
        package(r);
        fs::create_dir(r.join("extra")).unwrap();
        let inc = r.join("+a").join("inc.m");
        let mut e = engine(r);
        run(
            &mut e,
            &format!(
                "x = a.inc(1);\nfid = fopen({}, 'w'); fprintf(fid, 'function y = inc(x)\\ny = x + 500;\\nend\\n'); fclose(fid);\ny = a.inc(1);\n{drop_cache};\nz = a.inc(1);",
                quoted(&inc)
            ),
        );
        assert_eq!(scalar(&e, "x"), 2., "{drop_cache}");
        assert_eq!(scalar(&e, "y"), 2., "{drop_cache}");
        assert_eq!(scalar(&e, "z"), 501., "{drop_cache}");
    }
}

// The 9a toolbox scenario with a package: addpath(genpath('tbx')) leaves the
// private and package folders off the path, yet the toolbox function reaches
// its private helper and its package function (which has its own private h).
#[test]
fn toolbox_function_uses_a_private_helper_and_a_package_function() {
    let root = folder();
    let r = root.path();
    toolbox(r);
    package(&r.join("tbx"));
    write(
        r,
        &["tbx", "f.m"],
        "function y = f(x)\ny = 10 * h(x) + a.fn(x);\nend\n",
    );
    write(
        r,
        &["tbx", "sub", "viasub.m"],
        "function y = viasub(x)\nimport a.b.*\ny = g(x) + a.inc(x);\nend\n",
    );
    write(
        r,
        &["main.m"],
        "addpath(genpath('tbx'));\nr = f(2);\nv = viasub(1);\np = path;\n",
    );
    let mut e = engine(r);
    run(&mut e, "main");
    // tbx/private/h: x + 1; tbx/+a/private/h: 2 * x.
    assert_eq!(scalar(&e, "r"), 10. * 3. + 104.);
    assert_eq!(scalar(&e, "v"), 4.);
    let tbx = real(r).join("tbx");
    assert_eq!(
        text(&e, "p"),
        format!(
            "{}{}{}",
            tbx.to_string_lossy(),
            if cfg!(windows) { ';' } else { ':' },
            tbx.join("sub").to_string_lossy()
        ),
        "genpath must not list package or private folders"
    );
    run(
        &mut e,
        "r2 = f(3); q = a.fn(1); run('main'); r3 = r + a.noargs;",
    );
    assert_eq!(scalar(&e, "r2"), 146.);
    assert_eq!(scalar(&e, "q"), 102.);
    assert_eq!(scalar(&e, "r3"), 176.);
}

// 1e5 qualified and imported calls are answered from the per-evaluation
// cache, far from the time 1e5 folder lookups would take.
#[test]
fn repeated_package_calls_do_not_touch_the_disk() {
    let root = folder();
    let r = root.path();
    package(r);
    write(
        r,
        &["loop_import.m"],
        "function s = loop_import(n)\nimport a.*\ns = 0;\nfor k = 1:n\n    s = inc(s) + abs(-1) - 1;\nend\nend\n",
    );
    let mut e = Engine::with_limits(matrust::engine::Limits::local());
    e.set_current_folder(r).unwrap();
    let start = std::time::Instant::now();
    run(
        &mut e,
        "s = 0; for k = 1:1e5, s = a.inc(s); end; t = loop_import(1e5);",
    );
    let elapsed = start.elapsed();
    assert_eq!(scalar(&e, "s"), 1e5);
    assert_eq!(scalar(&e, "t"), 1e5);
    assert!(
        elapsed < std::time::Duration::from_secs(20),
        "2e5 cached package calls took {elapsed:?}"
    );
}

// ----- 9c: class folders (@Cls) with methods in separate files -----

/// The class folder `@Pt` inside `dir`: the value class Pt (x, y, a private
/// property) with the methods `norm2`, `both`, `hidden` (private) and the
/// static `unit` declared without a body, the undeclared method files
/// `scale` and `reveal`, and the private helper `@Pt/private/helper.m`.
fn point_class(dir: &Path) {
    write(
        dir,
        &["@Pt", "Pt.m"],
        "classdef Pt\n    properties\n        x = 0\n        y = 0\n    end\n    properties (Access = private)\n        secret = 7\n    end\n    methods\n        function obj = Pt(x, y)\n            if nargin > 0\n                obj.x = x;\n                obj.y = y;\n            end\n        end\n        function r = sum2(obj)\n            r = obj.x + obj.y;\n        end\n        r = norm2(obj)\n        [s, d] = both(obj)\n    end\n    methods (Static)\n        p = unit(k)\n    end\n    methods (Access = private)\n        r = hidden(obj)\n    end\nend\n",
    );
    write(
        dir,
        &["@Pt", "norm2.m"],
        "function r = norm2(obj)\n% NORM2 distance from the origin.\nr = sqrt(obj.x^2 + obj.y^2);\nend\n",
    );
    write(
        dir,
        &["@Pt", "scale.m"],
        "function obj = scale(obj, k)\nobj.x = obj.x * k;\nobj.y = obj.y * k;\nend\n",
    );
    write(
        dir,
        &["@Pt", "both.m"],
        "function [s, d] = both(obj)\ns = obj.x + obj.y;\nd = obj.x - obj.y;\nend\n",
    );
    write(
        dir,
        &["@Pt", "unit.m"],
        "function p = unit(k)\np = Pt(k, 0);\nend\n",
    );
    write(
        dir,
        &["@Pt", "hidden.m"],
        "function r = hidden(obj)\nr = obj.secret;\nend\n",
    );
    // A method file uses a private method, a private property and the
    // private folder of the class folder.
    write(
        dir,
        &["@Pt", "reveal.m"],
        "function r = reveal(obj)\nr = hidden(obj) + obj.secret + helper(obj.x);\nend\n",
    );
    write(
        dir,
        &["@Pt", "private", "helper.m"],
        "function y = helper(x)\ny = 100 * x;\nend\n",
    );
}
fn cell_text(e: &Engine, name: &str) -> Vec<String> {
    match e.workspace().get(name).unwrap() {
        Array::Cell(c) => (0..c.len())
            .map(|i| match c.get_linear(i).unwrap().as_ref() {
                Array::Character(t) => String::from_utf16_lossy(t.units()),
                x => panic!("{name}: unexpected {}", x.class()),
            })
            .collect(),
        x => panic!("{name}: unexpected {}", x.class()),
    }
}

// A value class in a class folder: the constructor, a method of the classdef
// file, a declared method file and an undeclared one, in both call forms and
// with several outputs.
#[test]
fn class_folder_defines_a_class_with_method_files() {
    let root = folder();
    let r = root.path();
    point_class(r);
    let mut e = engine(r);
    run(
        &mut e,
        "p = Pt(3, 4); c = class(p); s = sum2(p); n1 = norm2(p); n2 = p.norm2(); q = scale(p, 2); qx = q.x; q2 = p.scale(3); q2y = q2.y; px = p.x;",
    );
    assert_eq!(text(&e, "c"), "Pt");
    assert_eq!(scalar(&e, "s"), 7.);
    assert_eq!(scalar(&e, "n1"), 5.);
    assert_eq!(scalar(&e, "n2"), 5.);
    assert_eq!(scalar(&e, "qx"), 6.);
    assert_eq!(scalar(&e, "q2y"), 12.);
    assert_eq!(scalar(&e, "px"), 3., "a value object is not changed");
    // Several outputs, both call forms; the object from an earlier line.
    run(
        &mut e,
        "[a, b] = both(p); [c2, d2] = p.both(); [e2, f2] = both(Pt(1, 5)); n3 = norm2(scale(p, 2));",
    );
    assert_eq!((scalar(&e, "a"), scalar(&e, "b")), (7., -1.));
    assert_eq!((scalar(&e, "c2"), scalar(&e, "d2")), (7., -1.));
    assert_eq!((scalar(&e, "e2"), scalar(&e, "f2")), (6., -4.));
    assert_eq!(scalar(&e, "n3"), 10.);
    // Statements: the value shows as ans; feval and cellfun find the method.
    let shown = run(&mut e, "norm2(p)");
    assert!(shown.contains("ans") && shown.contains('5'), "{shown}");
    let shown = run(&mut e, "p.scale(2)");
    assert!(shown.contains("Pt with properties"), "{shown}");
    run(
        &mut e,
        "n4 = feval('norm2', p); n5 = cellfun(@norm2, {p, Pt(6, 8)});",
    );
    assert_eq!(scalar(&e, "n4"), 5.);
    assert_eq!(numbers(&e, "n5"), [5., 10.]);
    // Listings and queries include the file methods (public ones).
    run(
        &mut e,
        "m = methods(p); i1 = ismethod(p, 'norm2'); i2 = ismethod(p, 'scale'); i3 = ismethod(p, 'hidden'); i4 = ismethod(p, 'nothere'); m2 = methods('Pt');",
    );
    assert_eq!(
        cell_text(&e, "m"),
        ["Pt", "both", "norm2", "reveal", "scale", "sum2", "unit"]
    );
    assert_eq!(cell_text(&e, "m2"), cell_text(&e, "m"));
    assert_eq!(scalar(&e, "i1"), 1.);
    assert_eq!(scalar(&e, "i2"), 1.);
    assert_eq!(scalar(&e, "i3"), 0.);
    assert_eq!(scalar(&e, "i4"), 0.);
    // which and exist answer as for a plain classdef file, with the file of
    // the class folder.
    let class_file = real(r).join("@Pt").join("Pt.m");
    run(
        &mut e,
        "w = which('Pt'); x1 = exist('Pt'); x2 = exist('Pt', 'file');",
    );
    assert_eq!(text(&e, "w"), class_file.to_string_lossy());
    assert_eq!(scalar(&e, "x1"), 2.);
    assert_eq!(scalar(&e, "x2"), 2.);
    let shown = run(&mut e, "which Pt");
    assert_eq!(shown.trim(), class_file.to_string_lossy());
    // A method file is not a function of its own: without an object the
    // name is unknown.
    undefined(&fails(&mut e, "norm2(5)"), "norm2");
}

// The same class as a plain classdef file answers exist() with the same value.
#[test]
fn class_folder_wins_over_a_class_file_of_the_same_folder() {
    let root = folder();
    let r = root.path();
    write(
        r,
        &["Pt.m"],
        "classdef Pt\n    properties\n        x = -1\n        y = -1\n    end\n    methods\n        function obj = Pt(varargin)\n        end\n    end\nend\n",
    );
    let mut e = engine(r);
    run(
        &mut e,
        "p = Pt(3, 4); px = p.x; x1 = exist('Pt'); w = which('Pt');",
    );
    assert_eq!(scalar(&e, "px"), -1.);
    assert_eq!(scalar(&e, "x1"), 2.);
    assert_eq!(text(&e, "w"), real(r).join("Pt.m").to_string_lossy());
    // With @Pt next to it, the class folder is the class.
    point_class(r);
    run(
        &mut e,
        "p = Pt(3, 4); px = p.x; n = norm2(p); x1 = exist('Pt'); w = which('Pt');",
    );
    assert_eq!(scalar(&e, "px"), 3.);
    assert_eq!(scalar(&e, "n"), 5.);
    assert_eq!(scalar(&e, "x1"), 2.);
    assert_eq!(
        text(&e, "w"),
        real(r).join("@Pt").join("Pt.m").to_string_lossy()
    );
    let shown = run(&mut e, "which Pt -all");
    let lines: Vec<&str> = shown
        .lines()
        .map(str::trim)
        .filter(|l| !l.is_empty())
        .collect();
    assert_eq!(lines.len(), 2, "{shown}");
    assert!(lines[0].starts_with(&*real(r).join("@Pt").join("Pt.m").to_string_lossy()));
    assert!(lines[1].starts_with(&*real(r).join("Pt.m").to_string_lossy()));
    // Path position decides between folders (R2026a+ rule): a plain file
    // of the current folder comes before a class folder on the path.
    let other = folder();
    let o = other.path();
    write(
        o,
        &["Pt.m"],
        "function y = Pt(varargin)\ny = 'function';\nend\n",
    );
    let mut e = engine(o);
    run(&mut e, "a = Pt(1, 2);");
    assert_eq!(text(&e, "a"), "function");
    run(
        &mut e,
        &format!("addpath({}); b = Pt(1, 2); bc = class(b);", quoted(r)),
    );
    assert_eq!(text(&e, "bc"), "char");
}

// A Static method in its own file, declared in methods (Static); a private
// method file and the class's private members used from a method file; the
// private folder of the class folder.
#[test]
fn class_folder_static_private_and_helper_files() {
    let root = folder();
    let r = root.path();
    point_class(r);
    let mut e = engine(r);
    run(
        &mut e,
        "u = Pt.unit(5); ux = u.x; uy = u.y; v = Pt.unit(2).x;",
    );
    assert_eq!(scalar(&e, "ux"), 5.);
    assert_eq!(scalar(&e, "uy"), 0.);
    assert_eq!(scalar(&e, "v"), 2.);
    let shown = run(&mut e, "Pt.unit(9)");
    assert!(shown.contains("Pt with properties"), "{shown}");
    // reveal: hidden(obj) = 7, obj.secret = 7, helper(3) = 300.
    run(&mut e, "p = Pt(3, 4); t = reveal(p); t2 = p.reveal();");
    assert_eq!(scalar(&e, "t"), 314.);
    assert_eq!(scalar(&e, "t2"), 314.);
    // Outside the class the private method, the private property and the
    // helper of the private folder stay hidden.
    let message = fails(&mut e, "h = hidden(p);");
    assert!(
        message.contains("Cannot access private method 'hidden' of class 'Pt'."),
        "{message}"
    );
    let message = fails(&mut e, "h = p.hidden();");
    assert!(
        message.contains("Cannot access private method 'hidden' of class 'Pt'."),
        "{message}"
    );
    let message = fails(&mut e, "h = p.secret;");
    assert!(
        message.contains("Cannot access private property 'secret' of class 'Pt'."),
        "{message}"
    );
    undefined(&fails(&mut e, "h = helper(1);"), "helper");
    // A method that is not Static is not called through the class name.
    let message = fails(&mut e, "n = Pt.norm2();");
    assert!(
        message.contains("no Constant property or Static method named 'norm2'"),
        "{message}"
    );
    // The classdef file's methods reach the private folder as well.
    write(
        r,
        &["@Pt", "Pt.m"],
        "classdef Pt\n    properties\n        x = 2\n    end\n    methods\n        function r = viahelper(obj)\n            r = helper(obj.x) + 1;\n        end\n    end\nend\n",
    );
    run(&mut e, "p = Pt(); t3 = viahelper(p);");
    assert_eq!(scalar(&e, "t3"), 201.);
}

// A method file is a file of its own: local functions, mfilename, error
// locations, nargin/nargout, chained calls and a call without parentheses.
#[test]
fn method_file_runs_as_its_own_file() {
    let root = folder();
    let r = root.path();
    point_class(r);
    write(
        r,
        &["@Pt", "info.m"],
        "function [name, count, outs] = info(obj, varargin)\nname = mfilename;\ncount = nargin;\nouts = nargout + twice(obj.x);\nend\nfunction y = twice(x)\ny = 2 * x;\nend\n",
    );
    write(
        r,
        &["@Pt", "broken.m"],
        "function r = broken(obj)\nr = obj.x;\nr = undefined_thing(r);\nend\n",
    );
    let mut e = engine(r);
    run(
        &mut e,
        "p = Pt(3, 4); [nm, c, o] = info(p, 1, 2); [nm2, c2] = p.info(); n = p.scale(2).norm2(); m = p.norm2; t = Pt(1, 1).sum2;",
    );
    assert_eq!(text(&e, "nm"), "info");
    assert_eq!(scalar(&e, "c"), 3.);
    assert_eq!(scalar(&e, "o"), 3. + 6.);
    assert_eq!(text(&e, "nm2"), "info");
    assert_eq!(scalar(&e, "c2"), 1.);
    assert_eq!(scalar(&e, "n"), 10.);
    assert_eq!(scalar(&e, "m"), 5.);
    assert_eq!(scalar(&e, "t"), 2.);
    // The local function of a method file is not a method and not visible
    // outside the file.
    undefined(&fails(&mut e, "y = twice(p);"), "twice");
    let message = fails(&mut e, "b = broken(p);");
    let file = real(r).join("@Pt").join("broken.m");
    assert!(message.contains(&*file.to_string_lossy()), "{message}");
    assert!(message.contains("Line 3"), "{message}");
    assert!(message.contains("undefined_thing"), "{message}");
}

// A handle class: a method file changes the object it is given.
#[test]
fn class_folder_handle_class_method_file_mutates_the_object() {
    let root = folder();
    let r = root.path();
    write(
        r,
        &["@Ctr", "Ctr.m"],
        "classdef Ctr < handle\n    properties\n        n = 0\n    end\n    methods\n        bump(obj, d)\n        r = total(obj)\n    end\nend\n",
    );
    write(
        r,
        &["@Ctr", "bump.m"],
        "function bump(obj, d)\nobj.n = obj.n + d;\nend\n",
    );
    write(
        r,
        &["@Ctr", "total.m"],
        "function r = total(obj)\nr = obj.n;\nend\n",
    );
    // An undeclared delete method file runs at delete(h).
    write(
        r,
        &["@Ctr", "delete.m"],
        "function delete(obj)\nfprintf('bye %d\\n', obj.n);\nend\n",
    );
    let mut e = engine(r);
    run(
        &mut e,
        "h = Ctr(); bump(h, 2); h.bump(3); g = h; g.bump(10); n = h.n; t = total(h);",
    );
    assert_eq!(scalar(&e, "n"), 15.);
    assert_eq!(scalar(&e, "t"), 15.);
    run(&mut e, "h.bump(1); n = total(g);");
    assert_eq!(scalar(&e, "n"), 16.);
    let shown = run(&mut e, "delete(h); v = isvalid(g);");
    assert!(shown.contains("bye 16"), "{shown}");
    assert_eq!(scalar(&e, "v"), 0.);
}

// A subclass in its own class folder inherits the method files of its
// superclass, overrides one with a file of its own and calls the inherited
// version from it.
#[test]
fn class_folder_subclass_inherits_method_files() {
    let root = folder();
    let r = root.path();
    point_class(r);
    write(
        r,
        &["@Pt3", "Pt3.m"],
        "classdef Pt3 < Pt\n    properties\n        z = 0\n    end\n    methods\n        function obj = Pt3(x, y, z)\n            obj = obj@Pt(x, y);\n            obj.z = z;\n        end\n    end\nend\n",
    );
    write(
        r,
        &["@Pt3", "norm2.m"],
        "function r = norm2(obj)\nr = sqrt(norm2@Pt(obj)^2 + obj.z^2);\nend\n",
    );
    let mut e = engine(r);
    run(
        &mut e,
        "p = Pt3(3, 4, 12); n = norm2(p); n2 = p.norm2(); q = scale(p, 2); c = class(q); qz = q.z; qx = q.x; [s, d] = both(p); t = reveal(p); ia = isa(p, 'Pt'); m = methods(p);",
    );
    assert_eq!(scalar(&e, "n"), 13.);
    assert_eq!(scalar(&e, "n2"), 13.);
    assert_eq!(text(&e, "c"), "Pt3");
    assert_eq!(scalar(&e, "qz"), 12.);
    assert_eq!(scalar(&e, "qx"), 6.);
    assert_eq!((scalar(&e, "s"), scalar(&e, "d")), (7., -1.));
    assert_eq!(scalar(&e, "t"), 314.);
    assert_eq!(scalar(&e, "ia"), 1.);
    assert_eq!(
        cell_text(&e, "m"),
        ["Pt3", "both", "norm2", "reveal", "scale", "sum2", "unit"]
    );
    // The inherited Static method file, through the subclass name.
    run(&mut e, "u = Pt3.unit(4); uc = class(u);");
    assert_eq!(text(&e, "uc"), "Pt");
    // The superclass keeps its own version.
    run(&mut e, "n = norm2(Pt(3, 4));");
    assert_eq!(scalar(&e, "n"), 5.);
}

// Dispatch: for a Pt as first argument the method file wins over a function
// of the current folder, of the path, and over a builtin; other arguments
// reach those as before.
#[test]
fn method_file_shadows_functions_only_for_its_class() {
    let root = folder();
    let r = root.path();
    point_class(r);
    write(
        r,
        &["scale.m"],
        "function y = scale(a, k)\ny = -a * k;\nend\n",
    );
    write(
        r,
        &["lib", "norm2.m"],
        "function y = norm2(a)\ny = 'path';\nend\n",
    );
    // A builtin name as a method file.
    write(
        r,
        &["@Pt", "numel.m"],
        "function n = numel(obj)\nn = 42;\nend\n",
    );
    let mut e = engine(r);
    run(
        &mut e,
        "addpath('lib'); p = Pt(3, 4); a = scale(2, 5); b = scale(p, 5); bx = b.x; c = norm2(1); d = norm2(p); f = numel(p); g = numel([1 2 3]);",
    );
    assert_eq!(scalar(&e, "a"), -10.);
    assert_eq!(scalar(&e, "bx"), 15.);
    assert_eq!(text(&e, "c"), "path");
    assert_eq!(scalar(&e, "d"), 5.);
    assert_eq!(scalar(&e, "f"), 42.);
    assert_eq!(scalar(&e, "g"), 3.);
    // The same from inside a function, and as statements.
    write(
        r,
        &["mix.m"],
        "function [a, b] = mix(p)\na = scale(2, 5);\nq = scale(p, 5);\nb = q.y;\nend\n",
    );
    run(&mut e, "[a, b] = mix(p);");
    assert_eq!((scalar(&e, "a"), scalar(&e, "b")), (-10., 20.));
    let shown = run(&mut e, "scale(2, 5)");
    assert!(shown.contains("-10"), "{shown}");
}

// Errors: a method in the classdef file and in a file of its own; a
// declaration without a body outside a class folder; a declared method
// without its file; a method file that is not that function.
#[test]
fn class_folder_definition_errors_name_the_files() {
    let root = folder();
    let r = root.path();
    point_class(r);
    let twice = write(
        r,
        &["@Pt", "sum2.m"],
        "function r = sum2(obj)\nr = 0;\nend\n",
    );
    let mut e = engine(r);
    let message = fails(&mut e, "p = Pt(1, 2);");
    let class_file = real(r).join("@Pt").join("Pt.m");
    assert!(
        message.contains("the method 'sum2' of class 'Pt' is defined twice"),
        "{message}"
    );
    assert!(
        message.contains(&*class_file.to_string_lossy()),
        "{message}"
    );
    assert!(message.contains("(line 16)"), "{message}");
    assert!(
        message.contains(&*real(r).join("@Pt").join("sum2.m").to_string_lossy()),
        "{message}"
    );
    fs::remove_file(&twice).unwrap();
    run(&mut e, "p = Pt(1, 2); s = sum2(p);");
    assert_eq!(scalar(&e, "s"), 3.);
    // A plain classdef file keeps rejecting a declaration without a body.
    write(
        r,
        &["Plain.m"],
        "classdef Plain\n    methods\n        r = m(obj)\n    end\nend\n",
    );
    let message = fails(&mut e, "q = Plain();");
    assert!(
        message.contains("method declarations without a body are not supported yet (line 3)"),
        "{message}"
    );
    // Declared, but the class folder has no such file: the class loads and
    // the call names what is missing.
    fs::remove_file(r.join("@Pt").join("both.m")).unwrap();
    run(
        &mut e,
        "p = Pt(1, 2); n = norm2(p); i = ismethod(p, 'both');",
    );
    assert_eq!(scalar(&e, "i"), 1.);
    let message = fails(&mut e, "[a, b] = both(p);");
    assert!(
        message.contains("The method 'both' of class 'Pt' is declared without a body")
            && message.contains("has no file both.m"),
        "{message}"
    );
    // A file of the class folder that does not define its function.
    write(r, &["@Pt", "notes.m"], "x = 1;\n");
    write(
        r,
        &["@Pt", "other.m"],
        "function r = different(obj)\nr = 1;\nend\n",
    );
    for name in ["notes", "other"] {
        let message = fails(&mut e, &format!("{name}(p)"));
        assert!(
            message.contains(&format!("must start with the function '{name}'")),
            "{message}"
        );
    }
    // The constructor cannot be declared without a body.
    write(
        r,
        &["@Q", "Q.m"],
        "classdef Q\n    methods\n        obj = Q(x)\n    end\nend\n",
    );
    let message = fails(&mut e, "q = Q(1);");
    assert!(
        message.contains(
            "the constructor Q must be defined, with its body, in the classdef file (line 3)"
        ),
        "{message}"
    );
}

// An old-style class folder (@Cls/Cls.m is a function using class()) is
// reported, not run.
#[test]
fn old_style_class_folder_is_reported() {
    let root = folder();
    let r = root.path();
    write(
        r,
        &["@Old", "Old.m"],
        "function obj = Old(v)\ns.v = v;\nobj = class(s, 'Old');\nend\n",
    );
    write(
        r,
        &["@Old", "display.m"],
        "function display(obj)\ndisp(obj.v);\nend\n",
    );
    // Methods for a builtin class: no class file, nothing to report.
    write(
        r,
        &["@double", "twice.m"],
        "function y = twice(x)\ny = 2 * x;\nend\n",
    );
    let mut e = engine(r);
    for code in ["o = Old(1);", "Old(1)", "o = feval('Old', 1);"] {
        let message = fails(&mut e, code);
        assert!(
            message.ends_with(
                "Old-style @Old class folders (class() constructors) are not supported; use a classdef file."
            ),
            "{code}: {message}"
        );
    }
    run(&mut e, "d = double(int8(3)); x = exist('Old');");
    assert_eq!(scalar(&e, "d"), 3.);
    assert_eq!(scalar(&e, "x"), 2.);
}

// A class folder inside a package folder is the class pkg.Cls.
#[test]
fn class_folder_in_a_package() {
    let root = folder();
    let r = root.path();
    write(
        r,
        &["tbx", "+geo", "@Seg", "Seg.m"],
        "classdef Seg\n    properties\n        a = 0\n        b = 1\n    end\n    methods\n        function obj = Seg(a, b)\n            if nargin > 0\n                obj.a = a;\n                obj.b = b;\n            end\n        end\n        r = len(obj)\n    end\n    methods (Static)\n        s = unit()\n        [lo, hi] = ends(s)\n    end\nend\n",
    );
    write(
        r,
        &["tbx", "+geo", "@Seg", "len.m"],
        "function r = len(obj)\nr = gap(obj.a, obj.b);\nend\n",
    );
    write(
        r,
        &["tbx", "+geo", "@Seg", "grow.m"],
        "function obj = grow(obj, d)\nobj.b = obj.b + d;\nend\n",
    );
    write(
        r,
        &["tbx", "+geo", "@Seg", "unit.m"],
        "function s = unit()\ns = geo.Seg(0, 1);\nend\n",
    );
    write(
        r,
        &["tbx", "+geo", "@Seg", "ends.m"],
        "function [lo, hi] = ends(s)\nlo = s.a;\nhi = s.b;\nend\n",
    );
    write(
        r,
        &["tbx", "+geo", "@Seg", "private", "gap.m"],
        "function d = gap(a, b)\nd = b - a;\nend\n",
    );
    let mut e = engine(r);
    run(
        &mut e,
        "addpath('tbx'); s = geo.Seg(2, 5); c = class(s); l = len(s); l2 = s.len(); g = grow(s, 10); gl = g.len(); u = geo.Seg.unit(); ul = len(u); [lo, hi] = geo.Seg.ends(g); ia = isa(s, 'geo.Seg');",
    );
    assert_eq!(text(&e, "c"), "geo.Seg");
    assert_eq!(scalar(&e, "l"), 3.);
    assert_eq!(scalar(&e, "l2"), 3.);
    assert_eq!(scalar(&e, "gl"), 13.);
    assert_eq!(scalar(&e, "ul"), 1.);
    assert_eq!((scalar(&e, "lo"), scalar(&e, "hi")), (2., 15.));
    assert_eq!(scalar(&e, "ia"), 1.);
    // The object keeps working in later lines; which and exist see the file.
    run(
        &mut e,
        "l3 = len(s) + s.grow(1).len(); w = which('geo.Seg'); x = exist('geo.Seg'); m = methods(s);",
    );
    assert_eq!(scalar(&e, "l3"), 7.);
    assert_eq!(
        text(&e, "w"),
        real(r)
            .join("tbx")
            .join("+geo")
            .join("@Seg")
            .join("Seg.m")
            .to_string_lossy()
    );
    assert_eq!(scalar(&e, "x"), 2.);
    assert_eq!(cell_text(&e, "m"), ["Seg", "ends", "grow", "len", "unit"]);
    // Imported, the class goes by its plain name.
    run(
        &mut e,
        "import geo.Seg; t = Seg(1, 4); tl = len(t); tc = class(t); clear import",
    );
    assert_eq!(scalar(&e, "tl"), 3.);
    assert_eq!(text(&e, "tc"), "geo.Seg");
    // The plain name is not a class outside the package.
    undefined(&fails(&mut e, "t = Seg(1, 4);"), "Seg");
}

// which, exist and help see a method file in an @ folder before that method
// has ever been called. The class sits on the search path, not in the
// current folder, so the scan is not "the folder we just wrote".
#[test]
fn which_exist_help_see_method_files_before_any_call() {
    let root = folder();
    let r = root.path();
    let lib = folder();
    let l = lib.path();
    write(
        l,
        &["@Mk", "Mk.m"],
        "classdef Mk\n    properties\n        v = 0\n    end\n    methods\n        function obj = Mk(v)\n            if nargin > 0\n                obj.v = v;\n            end\n        end\n    end\nend\n",
    );
    write(
        l,
        &["@Mk", "span.mr"],
        "%SPAN  Native width.\nfunction y = span(obj)\ny = obj.v;\nend\n",
    );
    write(
        l,
        &["@Mk", "span.m"],
        "%SPAN  Legacy width.\nfunction y = span(obj)\ny = obj.v;\nend\n",
    );
    write(
        l,
        &["@Mk", "hidden.m"],
        "%HIDDEN  A file in the class folder.\nfunction y = hidden(obj)\ny = obj.v;\nend\n",
    );
    write(
        l,
        &["@Mk", "private", "gap.m"],
        "%GAP  Not a method.\nfunction y = gap(x)\ny = x;\nend\n",
    );
    write(
        l,
        &["@Mk", "plot.m"],
        "%PLOT  Not the builtin.\nfunction plot(obj)\nend\n",
    );
    write(
        l,
        &["+box", "@Bk", "Bk.m"],
        "classdef Bk\n    methods\n        function obj = Bk()\n        end\n    end\nend\n",
    );
    write(
        l,
        &["+box", "@Bk", "rimw.m"],
        "%EDGE  An edge of a box.\nfunction y = rimw(obj)\ny = 1;\nend\n",
    );
    let span_mr = real(l).join("@Mk").join("span.mr");
    let span_m = real(l).join("@Mk").join("span.m");
    let hidden = real(l).join("@Mk").join("hidden.m");
    let plot_m = real(l).join("@Mk").join("plot.m");
    let mk = real(l).join("@Mk").join("Mk.m");
    let edge = real(l).join("+box").join("@Bk").join("rimw.m");
    let bk = real(l).join("+box").join("@Bk").join("Bk.m");
    let mr = span_mr.to_string_lossy().into_owned();
    let mm = span_m.to_string_lossy().into_owned();
    let mk_text = mk.to_string_lossy().into_owned();
    let edge_text = edge.to_string_lossy().into_owned();
    let bk_text = bk.to_string_lossy().into_owned();
    let method = format!("{mr}  % Mk method");
    let legacy = format!("{mm}  % Mk method");

    let mut e = engine(r);
    run(&mut e, &format!("addpath({});", quoted(l)));

    // Nothing has called span, edge, hidden or the constructors.
    assert_eq!(run(&mut e, "which span").trim(), method);
    run(&mut e, "w = which('span');");
    assert_eq!(text(&e, "w"), mr);
    let lines = |shown: &str| {
        shown
            .lines()
            .map(str::trim)
            .filter(|line| !line.is_empty())
            .map(str::to_owned)
            .collect::<Vec<_>>()
    };
    assert_eq!(
        lines(&run(&mut e, "which span -all")),
        vec![method.clone(), legacy.clone()]
    );
    run(&mut e, "c = which('span', '-all');");
    assert_eq!(cell_text(&e, "c"), [mr.clone(), mm.clone()]);
    assert_eq!(run(&mut e, "which Mk.span").trim(), method);
    run(&mut e, "w = which('Mk.span');");
    assert_eq!(text(&e, "w"), mr);
    assert_eq!(run(&mut e, "which Mk/span").trim(), method);
    assert_eq!(run(&mut e, "which @Mk/span").trim(), method);
    // `span.m` is class span, method m — not the file span.m.
    assert_eq!(run(&mut e, "which span.m").trim(), "'span.m' not found.");
    assert_eq!(run(&mut e, "which Mk").trim(), mk_text);
    assert_eq!(run(&mut e, "which Mk -all").trim(), mk_text);
    assert_eq!(
        run(&mut e, "which hidden").trim(),
        format!("{}  % Mk method", hidden.display())
    );
    assert_eq!(run(&mut e, "which gap").trim(), "'gap' not found.");
    let shown = run(&mut e, "which plot");
    assert!(shown.trim().starts_with("built-in ("), "{shown}");
    assert!(!shown.contains("method"), "{shown}");
    let shown = run(&mut e, "which plot -all");
    let plot_lines = lines(&shown);
    assert!(plot_lines[0].starts_with("built-in ("), "{shown}");
    assert_eq!(plot_lines[1], format!("{}  % Mk method", plot_m.display()));

    run(
        &mut e,
        "e1 = exist('span'); e2 = exist('span', 'file'); e3 = exist('span.m'); e4 = exist('span.mr'); e5 = exist('Mk'); e6 = exist('gap'); e7 = exist('rimw'); e8 = exist('box.Bk'); e9 = exist('plot'); e10 = exist('plot', 'builtin'); e11 = exist('hidden');",
    );
    assert_eq!(scalar(&e, "e1"), 2.);
    assert_eq!(scalar(&e, "e2"), 2.);
    assert_eq!(scalar(&e, "e3"), 2.);
    assert_eq!(scalar(&e, "e4"), 2.);
    assert_eq!(scalar(&e, "e5"), 2., "the class file is unchanged");
    assert_eq!(
        scalar(&e, "e6"),
        0.,
        "a private-folder file is not a method"
    );
    assert_eq!(scalar(&e, "e7"), 0., "a package method is not a bare name");
    assert_eq!(scalar(&e, "e8"), 2.);
    assert_eq!(scalar(&e, "e9"), 5.);
    assert_eq!(scalar(&e, "e10"), 5.);
    assert_eq!(scalar(&e, "e11"), 2.);

    let help = run(&mut e, "help Mk.span");
    assert!(help.contains("SPAN  Native width."), "{help}");
    assert!(!help.contains("Legacy width."), "{help}");
    assert_eq!(run(&mut e, "help Mk/span"), help);
    assert_eq!(run(&mut e, "help @Mk/span"), help);
    let help_mk = run(&mut e, "help Mk");
    assert!(help_mk.contains("Mk not found."), "{help_mk}");
    assert!(!help_mk.contains("Native width."), "{help_mk}");
    assert!(!help_mk.contains("Methods"), "{help_mk}");
    let bare = run(&mut e, "help span");
    assert!(bare.contains("span not found."), "{bare}");
    let err = fails(&mut e, "doc Mk.span");
    assert!(err.contains("is not supported yet"), "{err}");
    assert!(fails(&mut e, "doc Mk/span").contains("is not supported yet"));

    assert_eq!(run(&mut e, "which box.Bk").trim(), bk_text);
    assert_eq!(run(&mut e, "which box.Bk -all").trim(), bk_text);
    run(&mut e, "w = which('box.Bk');");
    assert_eq!(text(&e, "w"), bk_text);
    let edge_line = format!("{edge_text}  % box.Bk method");
    assert_eq!(run(&mut e, "which box.Bk.rimw").trim(), edge_line);
    run(&mut e, "w = which('box.Bk.rimw');");
    assert_eq!(text(&e, "w"), edge_text);
    assert_eq!(run(&mut e, "which('+box/@Bk/rimw')").trim(), edge_line);
    assert_eq!(run(&mut e, "which rimw").trim(), "'rimw' not found.");
    let help_edge = run(&mut e, "help box.Bk.rimw");
    assert!(help_edge.contains("EDGE  An edge of a box."), "{help_edge}");
    assert_eq!(run(&mut e, "help('+box/@Bk/rimw')"), help_edge);
    assert!(fails(&mut e, "help box.Bk").contains("is not supported yet"));

    // Constructors only: the methods themselves are still uncalled.
    // which(name, obj) uses the object's class folder, not every @ folder.
    run(&mut e, "p = Mk(3); b = box.Bk(); n = 1;");
    run(
        &mut e,
        "w = which('span', p); w2 = which('span(p)'); w3 = which('span', b); w4 = which('rimw', b); w5 = which('span(n)');",
    );
    assert_eq!(text(&e, "w"), mr);
    assert_eq!(text(&e, "w2"), mr);
    assert_eq!(text(&e, "w3"), "", "Bk has no span method file");
    assert_eq!(text(&e, "w4"), edge_text);
    assert_eq!(
        text(&e, "w5"),
        mr,
        "a numeric variable resolves the plain name"
    );
    assert_eq!(run(&mut e, "which('span', p)").trim(), method);
    assert_eq!(run(&mut e, "which('rimw', b)").trim(), edge_line);
    assert_eq!(run(&mut e, "which('span', b)").trim(), "'span' not found.");
    // The command form cannot carry parentheses; the quoted forms above do.
    assert!(fails(&mut e, "which span(p)").contains("Quote the complete file command argument"));
    assert_eq!(run(&mut e, "which span").trim(), method);

    // A plain file in the current folder wins the single answer. -all still
    // marks the method file as a method, not as shadowed.
    let plain = real(r).join("span.m");
    write(r, &["span.m"], "function y = span(x)\ny = x;\nend\n");
    let plain_text = plain.to_string_lossy().into_owned();
    assert_eq!(run(&mut e, "which span").trim(), plain_text);
    assert_eq!(
        lines(&run(&mut e, "which span -all")),
        vec![plain_text.clone(), method, legacy]
    );
    run(&mut e, "w = which('span', p); w2 = which('span', b);");
    assert_eq!(text(&e, "w"), mr);
    assert_eq!(text(&e, "w2"), plain_text);
    run(&mut e, "e1 = exist('span');");
    assert_eq!(scalar(&e, "e1"), 2.);
}

// Caches: a method file added to the class folder is callable from the next
// top-level evaluation, an edited one takes effect then, a deleted one is
// gone; inside one evaluation the loaded file is kept until rehash.
#[test]
fn changed_method_files_are_seen_by_the_next_evaluation() {
    let root = folder();
    let r = root.path();
    point_class(r);
    let mut e = engine(r);
    run(
        &mut e,
        "p = Pt(3, 4); n = norm2(p); i = ismethod(p, 'late');",
    );
    assert_eq!(scalar(&e, "n"), 5.);
    assert_eq!(scalar(&e, "i"), 0.);
    undefined(&fails(&mut e, "v = late(p);"), "late");
    let late = write(
        r,
        &["@Pt", "late.m"],
        "function r = late(obj)\nr = obj.x + 1;\nend\n",
    );
    run(
        &mut e,
        "v = late(p); v2 = p.late(); i = ismethod(p, 'late');",
    );
    assert_eq!(scalar(&e, "v"), 4.);
    assert_eq!(scalar(&e, "v2"), 4.);
    assert_eq!(scalar(&e, "i"), 1.);
    fs::write(
        &late,
        "function r = late(obj)\n% edited\nr = obj.x + 1000;\nend\n",
    )
    .unwrap();
    run(&mut e, "v = late(p);");
    assert_eq!(scalar(&e, "v"), 1003.);
    fs::remove_file(&late).unwrap();
    undefined(&fails(&mut e, "v = late(p);"), "late");
    // A new class folder is found by the next evaluation as well.
    undefined(&fails(&mut e, "k = Knew();"), "Knew");
    write(
        r,
        &["@Knew", "Knew.m"],
        "classdef Knew\n    properties\n        v = 5\n    end\nend\n",
    );
    write(
        r,
        &["@Knew", "get5.m"],
        "function r = get5(obj)\nr = obj.v;\nend\n",
    );
    run(&mut e, "k = Knew(); kv = get5(k);");
    assert_eq!(scalar(&e, "kv"), 5.);
    // Inside one evaluation: the loaded method file is kept until the
    // lookups are dropped (rehash, a path change, cd, or clear functions).
    for drop_cache in [
        "rehash",
        "addpath('extra')",
        "cd('extra'); cd('..')",
        "clear functions",
    ] {
        let root = folder();
        let r = root.path();
        point_class(r);
        fs::create_dir(r.join("extra")).unwrap();
        let file = r.join("@Pt").join("norm2.m");
        let mut e = engine(r);
        run(
            &mut e,
            &format!(
                "p = Pt(3, 4); x = norm2(p);\nfid = fopen({}, 'w'); fprintf(fid, 'function r = norm2(obj)\\nr = 500;\\nend\\n'); fclose(fid);\ny = norm2(p);\n{drop_cache};\nz = norm2(p);",
                quoted(&file)
            ),
        );
        assert_eq!(scalar(&e, "x"), 5., "{drop_cache}");
        assert_eq!(scalar(&e, "y"), 5., "{drop_cache}");
        assert_eq!(scalar(&e, "z"), 500., "{drop_cache}");
    }
}

// The 9a/9b toolbox scenario with a class folder: addpath(genpath('tbx'))
// leaves the private, package and class folders off the path, yet the toolbox
// function reaches its private helper, its package function and its class.
#[test]
fn toolbox_function_uses_a_private_helper_a_package_function_and_a_class_folder() {
    let root = folder();
    let r = root.path();
    toolbox(r);
    package(&r.join("tbx"));
    point_class(&r.join("tbx"));
    write(
        r,
        &["tbx", "f.m"],
        "function y = f(x)\np = Pt(x, 2 * x);\nq = scale(p, 2);\ny = 10 * h(x) + a.fn(x) + norm2(Pt(3, 4)) + q.sum2() + Pt.unit(1).x;\nend\n",
    );
    write(
        r,
        &["tbx", "sub", "viasub.m"],
        "function y = viasub(x)\nimport a.b.*\np = Pt(x, 0);\ny = g(x) + a.inc(x) + reveal(p);\nend\n",
    );
    write(
        r,
        &["main.m"],
        "addpath(genpath('tbx'));\nr = f(2);\nv = viasub(1);\np = path;\npt = Pt(6, 8);\n",
    );
    let mut e = engine(r);
    run(&mut e, "main");
    // tbx/private/h: x + 1; tbx/+a/private/h: 2 * x; scale(Pt(2, 4), 2).sum2() = 12.
    assert_eq!(scalar(&e, "r"), 10. * 3. + 104. + 5. + 12. + 1.);
    // g(1) = 2, a.inc(1) = 2, reveal(Pt(1, 0)) = 7 + 7 + 100.
    assert_eq!(scalar(&e, "v"), 118.);
    let tbx = real(r).join("tbx");
    assert_eq!(
        text(&e, "p"),
        format!(
            "{}{}{}",
            tbx.to_string_lossy(),
            if cfg!(windows) { ';' } else { ':' },
            tbx.join("sub").to_string_lossy()
        ),
        "genpath must not list class, package or private folders"
    );
    // The path is kept: the object made by the script works in later lines.
    run(
        &mut e,
        "n = norm2(pt); r2 = f(3); run('main'); r3 = r + pt.scale(2).x;",
    );
    assert_eq!(scalar(&e, "n"), 10.);
    assert_eq!(scalar(&e, "r2"), 10. * 4. + 106. + 5. + 18. + 1.);
    assert_eq!(scalar(&e, "r3"), 152. + 12.);
}

// 1e5 calls of a method file are answered from the per-evaluation cache.
#[test]
fn repeated_method_file_calls_do_not_touch_the_disk() {
    let root = folder();
    let r = root.path();
    point_class(r);
    let mut e = Engine::with_limits(matrust::engine::Limits::local());
    e.set_current_folder(r).unwrap();
    let start = std::time::Instant::now();
    run(
        &mut e,
        "p = Pt(3, 4); s = 0; for k = 1:1e5, s = s + norm2(p); end",
    );
    let elapsed = start.elapsed();
    assert_eq!(scalar(&e, "s"), 5e5);
    assert!(
        elapsed < std::time::Duration::from_secs(20),
        "1e5 cached method file calls took {elapsed:?}"
    );
}

// Review fixes of fix line 9. A plain call `m(obj)` reaches an explicitly
// imported, local, wildcard-imported or private `m` before the method of the
// object's class; a current-folder or path `m` comes after the method.
// `obj.m()` always calls the method.
#[test]
fn imported_local_and_private_functions_precede_object_methods() {
    let root = folder();
    let r = root.path();
    let two =
        |value: &str| format!("function [y, z] = norm2(obj)\ny = {value};\nz = {value};\nend\n");
    write(
        r,
        &["@Pt", "Pt.m"],
        "classdef Pt\n    properties\n        x = 0\n    end\n    methods\n        function obj = Pt(x)\n            obj.x = x;\n        end\n    end\nend\n",
    );
    write(r, &["@Pt", "norm2.m"], &two("100 + obj.x"));
    write(r, &["D", "private", "norm2.m"], &two("1"));
    write(r, &["+a", "norm2.m"], &two("3"));
    write(r, &["lib", "norm2.m"], &two("9"));
    // Expression, several outputs, statement (ans) and member call.
    let body = "y1 = norm2(p);\n[y2, z] = norm2(p);\nnorm2(p);\ny3 = ans;\nm = p.norm2();\ny = [y1 y2 y3 m];\n";
    write(
        r,
        &["D", "callpriv.m"],
        &format!("function y = callpriv(p)\n{body}end\n"),
    );
    write(
        r,
        &["D", "calllocal.m"],
        &format!(
            "function y = calllocal(p)\n{body}end\nfunction [y, z] = norm2(obj)\ny = 2;\nz = 2;\nend\n"
        ),
    );
    write(
        r,
        &["D", "callimp.m"],
        &format!("function y = callimp(p)\nimport a.norm2\n{body}end\n"),
    );
    write(
        r,
        &["D", "callwild.m"],
        &format!("function y = callwild(p)\nimport a.*\n{body}end\n"),
    );
    write(
        r,
        &["E", "callpath.m"],
        &format!("function y = callpath(p)\n{body}end\n"),
    );
    let mut e = engine(r);
    run(
        &mut e,
        "addpath('D', 'E', 'lib'); p = Pt(4); a1 = callpriv(p); a2 = calllocal(p); a3 = callimp(p); a4 = callwild(p); a5 = callpath(p); n = norm2(p); [n2, z] = norm2(p);",
    );
    assert_eq!(numbers(&e, "a1"), [1., 1., 1., 104.]);
    assert_eq!(numbers(&e, "a2"), [2., 2., 2., 104.]);
    assert_eq!(numbers(&e, "a3"), [3., 3., 3., 104.]);
    assert_eq!(numbers(&e, "a4"), [3., 3., 3., 104.]);
    // lib/norm2.m (9) is on the path: the method wins.
    assert_eq!(numbers(&e, "a5"), [104., 104., 104., 104.]);
    assert_eq!(scalar(&e, "n"), 104.);
    assert_eq!(scalar(&e, "n2"), 104.);
}

// The codex fixture: `import a.choose` (or `a.*`) at the Command Window.
#[test]
fn imported_function_precedes_a_method_file_at_the_command_window() {
    let root = folder();
    let r = root.path();
    write(r, &["@C", "C.m"], "classdef C\nend\n");
    write(
        r,
        &["@C", "choose.m"],
        "function y = choose(obj)\ny = 2;\nend\n",
    );
    write(
        r,
        &["+a", "choose.m"],
        "function y = choose(obj)\ny = 1;\nend\n",
    );
    let mut e = engine(r);
    run(
        &mut e,
        "p = C(); r0 = choose(p); import a.choose; r1 = choose(p); m1 = p.choose();",
    );
    assert_eq!(scalar(&e, "r0"), 2.);
    assert_eq!(scalar(&e, "r1"), 1.);
    assert_eq!(scalar(&e, "m1"), 2.);
    run(
        &mut e,
        "clear import; import a.*; r2 = choose(p); choose(p); r3 = ans; m2 = p.choose();",
    );
    assert_eq!(scalar(&e, "r2"), 1.);
    assert_eq!(scalar(&e, "r3"), 1.);
    assert_eq!(scalar(&e, "m2"), 2.);
}

// An import never changes the class of an existing object; a new `C()`
// after it constructs the imported class.
#[test]
fn import_does_not_change_the_class_of_an_existing_object() {
    let root = folder();
    let r = root.path();
    let class = |k: u32| {
        format!(
            "classdef C\n    methods\n        function y = marker(obj)\n            y = {k};\n        end\n    end\nend\n"
        )
    };
    write(r, &["C.m"], &class(1));
    write(r, &["+a", "C.m"], &class(2));
    let mut e = engine(r);
    run(
        &mut e,
        "p = C(); before = p.marker(); import a.C; after = p.marker(); m = marker(p); pc = class(p); q = C(); qc = class(q); qm = q.marker(); both = [before after];",
    );
    assert_eq!(numbers(&e, "both"), [1., 1.]);
    assert_eq!(scalar(&e, "m"), 1.);
    assert_eq!(text(&e, "pc"), "C");
    assert_eq!(text(&e, "qc"), "a.C");
    assert_eq!(scalar(&e, "qm"), 2.);
    // In a later line, and with a wildcard import.
    run(
        &mut e,
        "clear import; import a.*; w1 = p.marker(); w2 = marker(p); w = C(); wc = class(w);",
    );
    assert_eq!(scalar(&e, "w1"), 1.);
    assert_eq!(scalar(&e, "w2"), 1.);
    assert_eq!(text(&e, "wc"), "a.C");
}

// Class folders follow path position (R2026a+): a plain Foo.m of the
// current folder comes before lib/@Foo on the path.
#[test]
fn current_folder_function_precedes_a_class_folder_on_the_path() {
    let root = folder();
    let r = root.path();
    write(r, &["Foo.m"], "function y = Foo()\ny = 1;\nend\n");
    write(
        r,
        &["lib", "@Foo", "Foo.m"],
        "classdef Foo\n    properties\n        v = 2\n    end\nend\n",
    );
    let mut e = engine(r);
    run(
        &mut e,
        "addpath('lib'); x = Foo(); w = which('Foo'); every = which('Foo', '-all'); n = numel(every); second = every{2};",
    );
    assert_eq!(scalar(&e, "x"), 1.);
    assert_eq!(text(&e, "w"), real(r).join("Foo.m").to_string_lossy());
    assert_eq!(scalar(&e, "n"), 2.);
    assert_eq!(
        text(&e, "second"),
        real(r)
            .join("lib")
            .join("@Foo")
            .join("Foo.m")
            .to_string_lossy()
    );
    // Without the current-folder file the class folder on the path is found.
    fs::remove_file(r.join("Foo.m")).unwrap();
    run(&mut e, "y = Foo(); yv = y.v;");
    assert_eq!(scalar(&e, "yv"), 2.);
}

// `a.C.probe();` as a statement gets nargout 0, as `C.probe();` after an
// import; `x = a.C.probe();` gets 1. Also for a class folder in a package.
#[test]
fn package_static_method_statement_gets_no_outputs() {
    let root = folder();
    let r = root.path();
    let class = |name: &str| {
        format!(
            "classdef {name}\n    methods (Static)\n        function y = probe()\n            fprintf('requested=%d\\n', nargout);\n            y = nargout;\n        end\n    end\nend\n"
        )
    };
    write(r, &["+a", "C.m"], &class("C"));
    write(r, &["+b", "@D", "D.m"], &class("D"));
    let mut e = engine(r);
    let shown = run(&mut e, "a.C.probe();");
    assert_eq!(shown.trim(), "requested=0");
    assert_eq!(scalar(&e, "ans"), 0.);
    let shown = run(&mut e, "import a.C; C.probe();");
    assert_eq!(shown.trim(), "requested=0");
    let shown = run(&mut e, "x = a.C.probe();");
    assert_eq!(shown.trim(), "requested=1");
    assert_eq!(scalar(&e, "x"), 1.);
    let shown = run(&mut e, "b.D.probe(); x2 = b.D.probe();");
    assert_eq!(shown.trim(), "requested=0\nrequested=1");
    assert_eq!(scalar(&e, "x2"), 1.);
}

// Review fixes (fix line 9 whole-step reviews): handles keep the lookup of
// the code that made them, by-name calls dispatch to methods before path
// functions, evalin looks functions up where its workspace is, which
// follows the call order.

fn shown_path(path: &Path) -> String {
    path.to_string_lossy().into_owned()
}

// A named handle keeps what its name meant where it was made: made at the
// Command Window, or in a file without such a private function, it never
// reaches a private, imported or local function of the code that calls it.
#[test]
fn named_handle_keeps_the_lookup_of_the_code_that_made_it() {
    let root = folder();
    let r = root.path();
    write(
        r,
        &["D", "usecb.m"],
        "function y = usecb(cb)\ny = cb(0);\nend\n",
    );
    write(
        r,
        &["D", "private", "sin.m"],
        "function y = sin(x)\ny = 123;\nend\n",
    );
    write(
        r,
        &["D", "private", "onlypriv.m"],
        "function y = onlypriv(x)\ny = 5;\nend\n",
    );
    // Callers that import a function of that name, or have a local one.
    write(r, &["+a", "sin.m"], "function y = sin(x)\ny = 77;\nend\n");
    write(
        r,
        &["D", "useimp.m"],
        "function y = useimp(cb)\nimport a.*\ny = [cb(0) sin(0)];\nend\n",
    );
    write(
        r,
        &["D", "useexp.m"],
        "function y = useexp(cb)\nimport a.sin\ny = [cb(0) sin(0)];\nend\n",
    );
    write(
        r,
        &["D", "uselocal.m"],
        "function y = uselocal(cb)\ny = [cb(0) feval(cb, 0) cellfun(cb, {0}) sin(0)];\nend\nfunction y = sin(x)\ny = 55;\nend\n",
    );
    // By name, and for a handle made there, the functions of D's own code
    // are the candidates, as before.
    write(
        r,
        &["D", "byname.m"],
        "function y = byname()\nk = @sin;\ns = str2func('sin');\ny = [feval('sin', 0) k(0) s(0) cellfun('sin', {0})];\nend\n",
    );
    write(r, &["D", "mk.m"], "function k = mk()\nk = @sin;\nend\n");
    // A file without a private sin makes the handle and hands it to D.
    write(
        r,
        &["other", "plain.m"],
        "function y = plain()\nk = str2func('sin');\ny = [usecb(@sin) usecb(k) k(0)];\nend\n",
    );
    // A handle to a local function, called by a file of another folder
    // while its own file runs: the local function, not D's private one.
    write(
        r,
        &["D", "private", "twice.m"],
        "function y = twice(x)\ny = 99;\nend\n",
    );
    write(
        r,
        &["other", "localcb.m"],
        "function y = localcb()\nk = str2func('twice');\ny = [usecb(@twice) usecb(k) usecb(@localcb2)];\nend\nfunction y = twice(x)\ny = 2 * x + 1;\nend\nfunction y = localcb2(x)\ny = 4;\nend\n",
    );
    let mut e = engine(r);
    // The reviewer's fixture.
    run(
        &mut e,
        "addpath('D', 'other'); k = @sin; before = k(0); after = usecb(k); r = [before after];",
    );
    assert_eq!(numbers(&e, "r"), [0., 0.]);
    run(
        &mut e,
        "s = str2func('sin'); s2 = str2func('@sin'); r = [usecb(s) usecb(s2) usecb(@sin) feval(k, 0)];",
    );
    assert_eq!(numbers(&e, "r"), [0., 0., 0., 0.]);
    run(
        &mut e,
        "i = useimp(k); x = useexp(k); l = uselocal(k); p = plain(); c = localcb();",
    );
    assert_eq!(numbers(&e, "i"), [0., 77.]);
    assert_eq!(numbers(&e, "x"), [0., 77.]);
    assert_eq!(numbers(&e, "l"), [0., 0., 0., 55.]);
    assert_eq!(numbers(&e, "p"), [0., 0., 0.]);
    assert_eq!(numbers(&e, "c"), [1., 1., 4.]);
    // A name undefined where the handle was made stays undefined inside D.
    undefined(&fails(&mut e, "u = @onlypriv; usecb(u)"), "onlypriv");
    undefined(
        &fails(&mut e, "u = str2func('onlypriv'); usecb(u)"),
        "onlypriv",
    );
    undefined(&fails(&mut e, "usecb(@onlypriv)"), "onlypriv");
    // Made inside D the name is the private function, wherever it is called.
    run(
        &mut e,
        "b = byname(); m = mk(); r = [m(0) usecb(m) feval(m, 0)];",
    );
    assert_eq!(numbers(&e, "b"), [123., 123., 123., 123.]);
    assert_eq!(numbers(&e, "r"), [123., 123., 123.]);
    // The base import list does not reach a handle made before the import,
    // and a handle made under it keeps the imported function after it.
    run(
        &mut e,
        "import a.sin; r1 = [k(0) sin(0)]; ki = @sin; clear import; r2 = [ki(0) usecb(ki) sin(0)]; q = func2str(ki);",
    );
    assert_eq!(numbers(&e, "r1"), [0., 77.]);
    assert_eq!(numbers(&e, "r2"), [77., 77., 0.]);
    assert_eq!(text(&e, "q"), "a.sin");
}

// A handle a function makes to itself outlasts the call: a private function
// stays bound to its file, a package function keeps the qualified name, a
// function on the path is found there again.
#[test]
fn handle_to_the_main_function_of_its_file_outlasts_the_call() {
    let root = folder();
    let r = root.path();
    let itself = |name: &str, base: u32| {
        format!(
            "function y = {name}(n)\nif nargin == 0\n    y = @{name};\nelse\n    y = n + {base};\nend\nend\n"
        )
    };
    write(r, &["D", "private", "rec.m"], &itself("rec", 1000));
    write(
        r,
        &["D", "getrec.m"],
        "function k = getrec()\nk = rec();\nend\n",
    );
    write(r, &["+a", "selfp.m"], &itself("selfp", 2000));
    write(r, &["other", "selfo.m"], &itself("selfo", 3000));
    write(
        r,
        &["other", "usecb.m"],
        "function y = usecb(cb)\ny = cb(1);\nend\n",
    );
    let mut e = engine(r);
    run(
        &mut e,
        "addpath('D', 'other'); k = getrec(); p = a.selfp(); o = selfo(); r = [k(1) usecb(k) p(1) usecb(p) o(1) usecb(o)];",
    );
    assert_eq!(numbers(&e, "r"), [1001., 1001., 2001., 2001., 3001., 3001.]);
    run(
        &mut e,
        "sk = func2str(k); sp = func2str(p); so = func2str(o); i = functions(k); file = i.file; r = [k(2) p(2) o(2)];",
    );
    assert_eq!(text(&e, "sk"), "rec");
    assert_eq!(text(&e, "sp"), "a.selfp");
    assert_eq!(text(&e, "so"), "selfo");
    assert_eq!(
        text(&e, "file"),
        shown_path(&real(r).join("D").join("private").join("rec.m"))
    );
    assert_eq!(numbers(&e, "r"), [1002., 2002., 3002.]);
}

/// `@Pt/Pt.m` with one property and the method files `norm2` (100 + x) and
/// `both` (x + 1, x - 1); `lib/norm2.m` (9) and `lib/both.m` (-1, -2).
fn point_and_path_functions(r: &Path) {
    write(
        r,
        &["@Pt", "Pt.m"],
        "classdef Pt\n    properties\n        x = 0\n    end\n    methods\n        function obj = Pt(x)\n            if nargin > 0\n                obj.x = x;\n            end\n        end\n    end\nend\n",
    );
    write(
        r,
        &["@Pt", "norm2.m"],
        "function r = norm2(obj)\nr = 100 + obj.x;\nend\n",
    );
    write(
        r,
        &["@Pt", "both.m"],
        "function [s, d] = both(obj)\ns = obj.x + 1;\nd = obj.x - 1;\nend\n",
    );
    write(
        r,
        &["lib", "norm2.m"],
        "function y = norm2(a)\ny = 9;\nend\n",
    );
    write(
        r,
        &["lib", "both.m"],
        "function [s, d] = both(a)\ns = -1;\nd = -2;\nend\n",
    );
}

// feval, a named handle and cellfun call the method of an object argument
// before a path function of that name, as the plain call does.
#[test]
fn by_name_calls_reach_the_method_before_a_path_function() {
    let root = folder();
    let r = root.path();
    point_and_path_functions(r);
    write(
        r,
        &["lib", "viah.m"],
        "function r = viah(p)\nk = @norm2;\nr = [k(p) feval('norm2', p) feval(k, p) cellfun(k, {p}) cellfun('norm2', {p}) k(1)];\nend\n",
    );
    let mut e = engine(r);
    // The reviewer's fixture.
    run(
        &mut e,
        "addpath('lib'); p = Pt(4); k = @norm2; s = str2func('norm2'); r = [norm2(p) feval('norm2', p) k(p) cellfun(@norm2, {p}) cellfun('norm2', {p}) s(p) feval(k, p)];",
    );
    assert_eq!(numbers(&e, "r"), [104.; 7]);
    // Any other argument still reaches the path function.
    run(
        &mut e,
        "r = [norm2(1) feval('norm2', 1) k(1) cellfun(@norm2, {1}) cellfun('norm2', {1})];",
    );
    assert_eq!(numbers(&e, "r"), [9.; 5]);
    // Several outputs, a handle made and called inside a file, statements.
    run(
        &mut e,
        "[s1, d1] = feval('both', p); b = @both; [s2, d2] = b(p); [s3, d3] = feval(b, 0); [s4, d4] = feval('both', 0); v = viah(p);",
    );
    assert_eq!((scalar(&e, "s1"), scalar(&e, "d1")), (5., 3.));
    assert_eq!((scalar(&e, "s2"), scalar(&e, "d2")), (5., 3.));
    assert_eq!((scalar(&e, "s3"), scalar(&e, "d3")), (-1., -2.));
    assert_eq!((scalar(&e, "s4"), scalar(&e, "d4")), (-1., -2.));
    assert_eq!(numbers(&e, "v"), [104., 104., 104., 104., 104., 9.]);
    let shown = run(&mut e, "feval('norm2', p)");
    assert!(shown.contains("104"), "{shown}");
    let shown = run(&mut e, "k(p)");
    assert!(shown.contains("104"), "{shown}");
    // A name that is no method: object queries and path functions as before.
    write(r, &["lib", "tag.m"], "function y = tag(a)\ny = 8;\nend\n");
    run(
        &mut e,
        "t = @tag; c = feval('class', p); r = [feval('tag', p) t(p) cellfun(@tag, {p}) feval('isa', p, 'Pt')];",
    );
    assert_eq!(text(&e, "c"), "Pt");
    assert_eq!(numbers(&e, "r"), [8., 8., 8., 1.]);
}

// The functions of the calling code keep their place before the method:
// by name, and for a handle made there (reference precedence: imports,
// local functions and private functions come before object methods).
#[test]
fn by_name_calls_put_own_functions_before_the_method() {
    let root = folder();
    let r = root.path();
    point_and_path_functions(r);
    write(
        r,
        &["D", "private", "norm2.m"],
        "function y = norm2(a)\ny = 1;\nend\n",
    );
    write(
        r,
        &["D", "bypriv.m"],
        "function y = bypriv(p)\nk = @norm2;\ny = [feval('norm2', p) k(p) cellfun('norm2', {p}) cellfun(k, {p})];\nend\n",
    );
    write(
        r,
        &["D", "bylocal.m"],
        "function y = bylocal(p)\nk = @norm2;\ny = [feval('norm2', p) k(p) cellfun(@norm2, {p})];\nend\nfunction r = norm2(obj)\nr = 2;\nend\n",
    );
    write(
        r,
        &["+a", "norm2.m"],
        "function y = norm2(a)\ny = 3;\nend\n",
    );
    write(
        r,
        &["D", "byimp.m"],
        "function y = byimp(p)\nimport a.norm2\nk = @norm2;\ny = [feval('norm2', p) k(p)];\nend\n",
    );
    write(
        r,
        &["D", "bywild.m"],
        "function y = bywild(p)\nimport a.*\nk = @norm2;\ny = [feval('norm2', p) k(p)];\nend\n",
    );
    // Without any of those the method is found from inside D too.
    write(
        r,
        &["E", "bymethod.m"],
        "function y = bymethod(p)\nk = @norm2;\ny = [feval('norm2', p) k(p)];\nend\n",
    );
    let mut e = engine(r);
    run(
        &mut e,
        "addpath('lib', 'D', 'E'); p = Pt(4); a = bypriv(p); b = bylocal(p); c = byimp(p); d = bywild(p); m = bymethod(p);",
    );
    assert_eq!(numbers(&e, "a"), [1.; 4]);
    assert_eq!(numbers(&e, "b"), [2.; 3]);
    assert_eq!(numbers(&e, "c"), [3.; 2]);
    assert_eq!(numbers(&e, "d"), [3.; 2]);
    assert_eq!(numbers(&e, "m"), [104.; 2]);
}

// evalin looks functions up where its workspace is: 'base' is the Command
// Window (no private folder, the base import list), 'caller' the caller's
// file and import list. eval keeps the context of the file it is in.
#[test]
fn evalin_resolves_functions_in_the_target_workspace() {
    let root = folder();
    let r = root.path();
    write(
        r,
        &["D", "private", "h.m"],
        "function y = h()\ny = 7;\nend\n",
    );
    write(r, &["other", "h.m"], "function y = h()\ny = 9;\nend\n");
    write(r, &["+a", "h.m"], "function y = h()\ny = 3;\nend\n");
    // The reviewer's fixtures, with eval and a plain call next to them.
    write(
        r,
        &["D", "callevalin.m"],
        "function y = callevalin()\ny = [evalin('base', 'h()') eval('h()') h()];\nend\n",
    );
    write(
        r,
        &["D", "impi.m"],
        "function y = impi()\nimport a.h\ny = [evalin('base', 'h()') eval('h()') h()];\nend\n",
    );
    write(
        r,
        &["D", "callcaller.m"],
        "function y = callcaller()\ny = [evalin('caller', 'h()') h()];\nend\n",
    );
    // Callers in another folder: a private function, an import, a local one.
    write(
        r,
        &["E", "private", "h.m"],
        "function y = h()\ny = 11;\nend\n",
    );
    write(
        r,
        &["E", "outer.m"],
        "function y = outer()\ny = [callcaller() callevalin()];\nend\n",
    );
    write(
        r,
        &["E", "outerimp.m"],
        "function y = outerimp()\nimport a.h\ny = callcaller();\nend\n",
    );
    write(
        r,
        &["E", "outerlocal.m"],
        "function y = outerlocal()\ny = callcaller();\nend\nfunction y = h()\ny = 13;\nend\n",
    );
    // The running function's own lookup is back after evalin, also after
    // an error in it; the variables of the workspace are used as before.
    write(
        r,
        &["D", "afterr.m"],
        "function y = afterr()\ntry\n    evalin('base', 'error(''t:e'', ''boom'')');\ncatch\nend\ny = [h() evalin('base', 'v + h()')];\nend\n",
    );
    // A script runs in the base workspace: 'caller' is the script's file,
    // 'base' stays the Command Window.
    write(
        r,
        &["S", "private", "h.m"],
        "function y = h()\ny = 15;\nend\n",
    );
    write(
        r,
        &["S", "s.m"],
        "sown = h();\nsc = callcaller();\nsb = callevalin();\n",
    );
    let mut e = engine(r);
    run(
        &mut e,
        "addpath('D', 'E', 'S', 'other'); v = 5; b = callevalin(); i = impi(); c = callcaller();",
    );
    assert_eq!(numbers(&e, "b"), [9., 7., 7.]);
    assert_eq!(numbers(&e, "i"), [9., 3., 3.]);
    assert_eq!(numbers(&e, "c"), [9., 7.]);
    run(
        &mut e,
        "o = outer(); oi = outerimp(); ol = outerlocal(); f = afterr();",
    );
    assert_eq!(numbers(&e, "o"), [11., 7., 9., 7., 7.]);
    assert_eq!(numbers(&e, "oi"), [3., 7.]);
    assert_eq!(numbers(&e, "ol"), [13., 7.]);
    assert_eq!(numbers(&e, "f"), [7., 14.]);
    run(&mut e, "s");
    assert_eq!(scalar(&e, "sown"), 15.);
    assert_eq!(numbers(&e, "sc"), [15., 7.]);
    assert_eq!(numbers(&e, "sb"), [9., 7., 7.]);
    // The base import list is the one 'base' uses.
    run(
        &mut e,
        "import a.h; bi = callevalin(); ci = callcaller(); clear import; bn = callevalin();",
    );
    assert_eq!(numbers(&e, "bi"), [3., 7., 7.]);
    assert_eq!(numbers(&e, "ci"), [3., 7.]);
    assert_eq!(numbers(&e, "bn"), [9., 7., 7.]);
}

// which follows the call order inside a running file: a local function
// comes before a wildcard import (and after an explicit one), then the
// private folder and the path. A local function is shown as `file (name)`,
// the statement adds `% Local function of file`.
#[test]
fn which_follows_the_call_order_inside_a_file() {
    let root = folder();
    let r = root.path();
    write(r, &["+a", "h.m"], "function y = h()\ny = 3;\nend\n");
    write(
        r,
        &["D", "private", "h.m"],
        "function y = h()\ny = 7;\nend\n",
    );
    write(r, &["lib", "h.m"], "function y = h()\ny = 9;\nend\n");
    // The reviewer's fixture: a wildcard import and a local function.
    write(
        r,
        &["D", "f.m"],
        "function [y, w, c, m] = f()\nimport a.*\ny = h();\nw = which('h');\nc = which('h', '-all');\nm = which('f');\nwhich h\nwhich h -all\nend\nfunction y = h()\ny = 2;\nend\n",
    );
    // An explicit import comes before the local function.
    write(
        r,
        &["D", "g.m"],
        "function [y, w, c] = g()\nimport a.h\ny = h();\nw = which('h');\nc = which('h', '-all');\nend\nfunction y = h()\ny = 2;\nend\n",
    );
    // Neither: the private function, as before.
    write(
        r,
        &["D", "n.m"],
        "function [y, w, c] = n()\ny = h();\nw = which('h');\nc = which('h', '-all');\nend\n",
    );
    let base = real(r);
    let in_f = format!("{} (h)", shown_path(&base.join("D").join("f.m")));
    let in_g = format!("{} (h)", shown_path(&base.join("D").join("g.m")));
    let package = shown_path(&base.join("+a").join("h.m"));
    let private = shown_path(&base.join("D").join("private").join("h.m"));
    let path = shown_path(&base.join("lib").join("h.m"));
    let mut e = engine(r);
    let shown = run(&mut e, "addpath('D', 'lib'); [y, w, c, m] = f();");
    assert_eq!(scalar(&e, "y"), 2.);
    assert_eq!(text(&e, "w"), in_f);
    assert_eq!(
        cell_text(&e, "c"),
        [&in_f, &package, &private, &path].map(String::clone)
    );
    assert_eq!(text(&e, "m"), shown_path(&base.join("D").join("f.m")));
    assert_eq!(
        shown.lines().collect::<Vec<_>>(),
        [
            format!("{in_f}  % Local function of f"),
            format!("{in_f}  % Local function of f"),
            format!("{package}  % Shadowed"),
            format!("{private}  % Shadowed"),
            format!("{path}  % Shadowed"),
        ]
    );
    run(&mut e, "[y, w, c] = g();");
    assert_eq!(scalar(&e, "y"), 3.);
    assert_eq!(text(&e, "w"), package);
    assert_eq!(
        cell_text(&e, "c"),
        [&package, &in_g, &private, &path].map(String::clone)
    );
    run(&mut e, "[y, w, c] = n();");
    assert_eq!(scalar(&e, "y"), 7.);
    assert_eq!(text(&e, "w"), private);
    assert_eq!(cell_text(&e, "c"), [&private, &path].map(String::clone));
    // At the Command Window none of the file's functions are candidates;
    // a function of the submitted text is a local function without a file.
    run(&mut e, "y = h(); w = which('h'); c = which('h', '-all');");
    assert_eq!(scalar(&e, "y"), 9.);
    assert_eq!(text(&e, "w"), path);
    assert_eq!(cell_text(&e, "c"), std::slice::from_ref(&path));
    let shown = run(
        &mut e,
        "y = h(); w = which('h'); which h\nfunction y = h()\ny = 4;\nend\n",
    );
    assert_eq!(scalar(&e, "y"), 4.);
    assert_eq!(text(&e, "w"), "local function");
    assert_eq!(shown.trim(), "h is a local function.");
}

// Inside a plain path file `norm2.m`, `norm2(obj)` dispatches to the object's
// method: the file's own primary function does not shadow it like a local one.
#[test]
fn primary_function_does_not_shadow_object_method() {
    let root = folder();
    let r = root.path();
    write(
        r,
        &["@Pt", "Pt.m"],
        "classdef Pt\n    properties\n        x\n    end\n    methods\n        function obj = Pt(x)\n            obj.x = x;\n        end\n    end\nend\n",
    );
    write(
        r,
        &["@Pt", "norm2.m"],
        "function y = norm2(obj)\ny = 100 + obj.x;\nend\n",
    );
    write(
        r,
        &["lib", "norm2.m"],
        "function y = norm2(p)\nif isnumeric(p)\n    y = norm2(Pt(p));\nelse\n    y = 9;\nend\nend\n",
    );
    let mut e = engine(r);
    run(&mut e, "addpath('lib'); r = norm2(4);");
    assert_eq!(scalar(&e, "r"), 104.);
}

// clear all drops a private-file cache inside one evaluation. The edit is the
// same size and the same second, and this run does not check mtimes, so the
// call before clear all still sees the old body. The search path stays, so
// the toolbox function is found again afterwards.
#[test]
fn clear_all_drops_private_lookups_and_keeps_the_path() {
    let root = folder();
    let r = root.path();
    toolbox(r);
    let h = r.join("tbx").join("private").join("h.m");
    let mut e = engine(r);
    run(
        &mut e,
        &format!(
            "addpath('tbx'); a = f(2);\nfid = fopen({}, 'w'); fprintf(fid, 'function y = h(x)\\ny = x + 500;\\nend\\n'); fclose(fid);\nb = f(2);\nif b ~= 30, error('cached call saw the edit'); end\nclear all;\nc = f(2);",
            quoted(&h)
        ),
    );
    assert!(!e.workspace().contains_key("a"));
    assert!(!e.workspace().contains_key("b"));
    assert_eq!(scalar(&e, "c"), 5020.);
}

// Package-qualified superclasses. A plain name is not looked up in the
// package that holds the class file (MathWorks "Packages Create Namespaces":
// the classdef line needs the package prefix; import shortens names inside
// functions, not this line).
const PKG_A: &str = "classdef A
    properties
        v = 0
    end
    methods
        function obj = A(v)
            if nargin > 0
                obj.v = v;
            end
            fprintf('A:%d\\n', nargin);
        end
        function y = twice(obj)
            y = 2 * obj.v;
        end
    end
end
";

#[test]
fn package_superclass_plain_files() {
    let root = folder();
    let r = root.path();
    write(r, &["+pkg", "A.m"], PKG_A);
    write(
        r,
        &["B.m"],
        "classdef B < pkg.A
    properties
        w = 1
    end
    methods
        function obj = B(v, w)
            obj = obj@pkg.A(v);
            if nargin > 1
                obj.w = w;
            end
        end
        function y = sum2(obj)
            y = obj.twice() + obj.w;
        end
    end
end
",
    );
    // The same package: the full name is required, and class() keeps it.
    write(
        r,
        &["+pkg", "Sub.m"],
        "classdef Sub < pkg.A
    methods
        function obj = Sub(v)
            obj = obj@pkg.A(v);
        end
    end
end
",
    );
    write(
        r,
        &["+a", "+b", "A.m"],
        "classdef A
    properties
        n = 0
    end
    methods
        function obj = A(n)
            if nargin > 0
                obj.n = n;
            end
            fprintf('N:%d\\n', nargin);
        end
    end
end
",
    );
    write(
        r,
        &["C.m"],
        "classdef C < a.b.A
    methods
        function obj = C(n)
            obj = obj@a.b.A(n);
        end
        function y = plus1(obj)
            y = obj.n + 1;
        end
    end
end
",
    );
    // (Sealed) on a subclass of a package class, and a handle mixed in.
    write(
        r,
        &["+pkg", "H.m"],
        "classdef H < handle
    properties
        k = 0
    end
    methods
        function obj = H(k)
            if nargin > 0
                obj.k = k;
            end
            fprintf('H:%d\\n', nargin);
        end
    end
end
",
    );
    write(
        r,
        &["S.m"],
        "classdef (Sealed) S < handle & pkg.H
    properties
        extra = 5
    end
    methods
        function obj = S(k)
            obj = obj@pkg.H(k);
        end
    end
end
",
    );
    let mut e = engine(r);
    let shown = run(
        &mut e,
        "b = B(3, 4); c = class(b); ia = isa(b, 'pkg.A'); ib = isa(b, 'B'); y = sum2(b); y2 = b.twice(); bv = b.v; bw = b.w; s = superclasses(b);",
    );
    assert!(shown.contains("A:1") && !shown.contains("A:0"), "{shown}");
    assert_eq!(text(&e, "c"), "B");
    assert_eq!(scalar(&e, "ia"), 1.);
    assert_eq!(scalar(&e, "ib"), 1.);
    assert_eq!(scalar(&e, "y"), 10.);
    assert_eq!(scalar(&e, "y2"), 6.);
    assert_eq!(scalar(&e, "bv"), 3.);
    assert_eq!(scalar(&e, "bw"), 4.);
    assert_eq!(cell_text(&e, "s"), ["pkg.A"]);
    let shown = run(
        &mut e,
        "p = pkg.Sub(5); pc = class(p); pia = isa(p, 'pkg.A'); pv = p.v; pt = p.twice();",
    );
    assert!(shown.contains("A:1") && !shown.contains("A:0"), "{shown}");
    assert_eq!(text(&e, "pc"), "pkg.Sub");
    assert_eq!(scalar(&e, "pia"), 1.);
    assert_eq!(scalar(&e, "pv"), 5.);
    assert_eq!(scalar(&e, "pt"), 10.);
    let shown = run(
        &mut e,
        "q = C(8); qc = class(q); qn = q.n; qy = plus1(q); qia = isa(q, 'a.b.A'); qs = superclasses(q);",
    );
    assert!(shown.contains("N:1") && !shown.contains("N:0"), "{shown}");
    assert_eq!(text(&e, "qc"), "C");
    assert_eq!(scalar(&e, "qn"), 8.);
    assert_eq!(scalar(&e, "qy"), 9.);
    assert_eq!(scalar(&e, "qia"), 1.);
    assert_eq!(cell_text(&e, "qs"), ["a.b.A"]);
    let shown = run(
        &mut e,
        "h = S(2); hc = class(h); hk = h.k; he = h.extra; hh = isa(h, 'handle'); hp = isa(h, 'pkg.H'); hs = superclasses(h);",
    );
    assert!(shown.contains("H:1") && !shown.contains("H:0"), "{shown}");
    assert_eq!(text(&e, "hc"), "S");
    assert_eq!(scalar(&e, "hk"), 2.);
    assert_eq!(scalar(&e, "he"), 5.);
    assert_eq!(scalar(&e, "hh"), 1.);
    assert_eq!(scalar(&e, "hp"), 1.);
    assert_eq!(cell_text(&e, "hs"), ["pkg.H", "handle"]);
    write(r, &["Child.m"], "classdef Child < S\nend\n");
    let message = fails(&mut e, "child = Child();");
    assert!(
        message.contains("class 'S' is Sealed and cannot be subclassed"),
        "{message}"
    );
}

#[test]
fn package_folder_class_inherits_package_superclass_method_file() {
    let root = folder();
    let r = root.path();
    write(
        r,
        &["+pkg", "@A", "A.m"],
        "classdef A
    properties
        v = 0
    end
    methods
        function obj = A(v)
            if nargin > 0
                obj.v = v;
            end
            fprintf('A:%d\\n', nargin);
        end
        r = len(obj)
    end
end
",
    );
    write(
        r,
        &["+pkg", "@A", "len.m"],
        "function r = len(obj)\nr = obj.v + 10;\nend\n",
    );
    write(
        r,
        &["+pkg", "@B", "B.m"],
        "classdef B < pkg.A
    properties
        w = 0
    end
    methods
        function obj = B(v, w)
            obj = obj@pkg.A(v);
            obj.w = w;
        end
        function y = via(obj)
            y = len@pkg.A(obj) + obj.w;
        end
    end
end
",
    );
    let mut e = engine(r);
    let shown = run(
        &mut e,
        "b = pkg.B(3, 4); c = class(b); ia = isa(b, 'pkg.A'); ib = isa(b, 'pkg.B'); r = len(b); r2 = b.len(); y = via(b); bv = b.v; bw = b.w; s = superclasses(b);",
    );
    assert!(shown.contains("A:1") && !shown.contains("A:0"), "{shown}");
    assert_eq!(text(&e, "c"), "pkg.B");
    assert_eq!(scalar(&e, "ia"), 1.);
    assert_eq!(scalar(&e, "ib"), 1.);
    assert_eq!(scalar(&e, "r"), 13.);
    assert_eq!(scalar(&e, "r2"), 13.);
    assert_eq!(scalar(&e, "y"), 17.);
    assert_eq!(scalar(&e, "bv"), 3.);
    assert_eq!(scalar(&e, "bw"), 4.);
    assert_eq!(cell_text(&e, "s"), ["pkg.A"]);
    let shown = run(&mut e, "superclasses(b)");
    assert!(
        shown.contains("Superclasses for class pkg.B:") && shown.contains("pkg.A"),
        "{shown}"
    );
}

#[test]
fn package_superclass_missing_and_unqualified_name_inside_package() {
    let root = folder();
    let r = root.path();
    write(r, &["+pkg", "A.m"], PKG_A);
    write(r, &["+pkg", "Bare.m"], "classdef Bare < A\nend\n");
    write(
        r,
        &["+pkg", "@Bare2", "Bare2.m"],
        "classdef Bare2 < A\nend\n",
    );
    write(r, &["M.m"], "classdef M < pkg.Nope\nend\n");
    write(r, &["Uses.m"], "classdef Uses < A\nend\n");
    let mut e = engine(r);
    let message = fails(&mut e, "b = pkg.Bare();");
    assert!(
        message.contains("the superclass 'A' of class 'pkg.Bare'"),
        "{message}"
    );
    assert!(message.contains("Bare.m"), "{message}");
    assert!(message.contains("(line 1)"), "{message}");
    // import does not shorten the classdef superclass line.
    let message = fails(&mut e, "import pkg.*; b = pkg.Bare();");
    assert!(
        message.contains("the superclass 'A' of class 'pkg.Bare'"),
        "{message}"
    );
    let message = fails(&mut e, "import pkg.A; b = pkg.Bare2();");
    assert!(
        message.contains("the superclass 'A' of class 'pkg.Bare2'"),
        "{message}"
    );
    let message = fails(&mut e, "import pkg.A; u = Uses();");
    assert!(
        message.contains("the superclass 'A' of class 'Uses'"),
        "{message}"
    );
    // The package class itself still constructs, and a missing dotted
    // superclass names that dotted name.
    run(&mut e, "a = pkg.A(3); av = a.v; clear import");
    assert_eq!(scalar(&e, "av"), 3.);
    let message = fails(&mut e, "m = M();");
    assert!(
        message.contains("the superclass 'pkg.Nope' of class 'M' cannot be found"),
        "{message}"
    );
    assert!(message.contains("(line 1)"), "{message}");
}

/// `F/mk.m` returns handles made inside it: anonymous ones calling its local
/// function `loc` (x + 100), its private `priv` (x + 200) and `fn` through
/// `import pkg.fn` (`lib/+pkg/fn.m`, x + 300), and `@loc`. `F/mks.m` is a
/// script with a local function. `other/` shadows every one of those names
/// (current folder, local function and private folder of its files).
fn anonymous_scope_tree(r: &Path) {
    write(
        r,
        &["F", "mk.m"],
        "function hs = mk()\nimport pkg.fn\nhs = {@(x) loc(x), @(x) priv(x), @(x) fn(x), @loc, @(x) @(y) loc(x) + priv(y)};\nend\nfunction y = loc(x)\ny = x + 100;\nend\n",
    );
    write(
        r,
        &["F", "private", "priv.m"],
        "function y = priv(x)\ny = x + 200;\nend\n",
    );
    write(
        r,
        &["F", "mks.m"],
        "hsc = @(x) priv(x) + sloc(x);\nfunction y = sloc(x)\ny = 1000;\nend\n",
    );
    write(
        r,
        &["lib", "+pkg", "fn.m"],
        "function y = fn(x)\nif nargin < 1\n    x = 0;\nend\ny = x + 300;\nend\n",
    );
    write(
        r,
        &["other", "loc.m"],
        "function y = loc(x)\ny = -1;\nend\n",
    );
    write(
        r,
        &["other", "priv.m"],
        "function y = priv(x)\ny = -2;\nend\n",
    );
    write(r, &["other", "fn.m"], "function y = fn(x)\ny = -3;\nend\n");
    write(
        r,
        &["other", "private", "priv.m"],
        "function y = priv(x)\ny = -5;\nend\n",
    );
    // A file of another folder with its own local loc, private priv and
    // import list calls the handles, directly, by feval and through cellfun.
    write(
        r,
        &["other", "callit.m"],
        "function y = callit(h, x)\nimport pkg.*\ny = [h(x) feval(h, x) cellfun(h, {x}) arrayfun(h, x) loc(x) priv(x)];\nend\nfunction y = loc(x)\ny = -4;\nend\n",
    );
}

// An anonymous function made in a file keeps that file's scope for the names
// its body uses: local functions, private folder and import list, wherever
// it is called (Command Window after cd, a file of another folder, cellfun).
// Variables keep today's snapshot; func2str and display do not change.
#[test]
fn anonymous_function_keeps_the_scope_of_the_file_that_made_it() {
    let root = folder();
    let r = root.path();
    anonymous_scope_tree(r);
    let mut e = engine(r);
    run(
        &mut e,
        "addpath('F', 'lib'); hs = mk(); h1 = hs{1}; h2 = hs{2}; h3 = hs{3}; h4 = hs{4}; h5 = hs{5}; r0 = [h1(1) h2(1) h3(1) h4(1)];",
    );
    assert_eq!(numbers(&e, "r0"), [101., 201., 301., 101.]);
    // Another current folder whose files shadow every name.
    run(
        &mut e,
        "cd('other'); plain = [loc(1) priv(1) fn(1)]; r = [h1(1) h2(1) h3(1) h4(1)];",
    );
    assert_eq!(numbers(&e, "plain"), [-1., -2., -3.]);
    assert_eq!(numbers(&e, "r"), [101., 201., 301., 101.]);
    // From a file of another folder, with its own local, private and imports.
    run(
        &mut e,
        "c1 = callit(h1, 2); c2 = callit(h2, 2); c3 = callit(h3, 2); c4 = callit(h4, 2);",
    );
    assert_eq!(numbers(&e, "c1"), [102., 102., 102., 102., -4., -5.]);
    assert_eq!(numbers(&e, "c2"), [202., 202., 202., 202., -4., -5.]);
    assert_eq!(numbers(&e, "c3"), [302., 302., 302., 302., -4., -5.]);
    assert_eq!(numbers(&e, "c4"), [102., 102., 102., 102., -4., -5.]);
    // cellfun/arrayfun at the Command Window, a handle calling a handle, a
    // handle made by a handle, and copies in a cell and a struct field.
    run(
        &mut e,
        "u = [cellfun(h1, {3, 4}) cellfun(h2, {3, 4}) arrayfun(h3, [3 4])]; g = @(k, x) k(x); v = [g(h1, 5) g(h2, 5) g(h3, 5)]; inner = h5(1); w = inner(2); s.f = h3; cc = {h2}; k = h1; copies = [s.f(6) cc{1}(6) k(6)];",
    );
    assert_eq!(numbers(&e, "u"), [103., 104., 203., 204., 303., 304.]);
    assert_eq!(numbers(&e, "v"), [105., 205., 305.]);
    assert_eq!(scalar(&e, "w"), 303.);
    assert_eq!(numbers(&e, "copies"), [306., 206., 106.]);
    // The callers' own lookup is unchanged after the calls.
    run(&mut e, "after = [loc(1) priv(1) fn(1)];");
    assert_eq!(numbers(&e, "after"), [-1., -2., -3.]);
    run(
        &mut e,
        "t = {func2str(h1), func2str(h2), func2str(h3), func2str(h4)}; i4 = functions(h4); kind = i4.type;",
    );
    assert_eq!(
        cell_text(&e, "t"),
        ["@(x)loc(x)", "@(x)priv(x)", "@(x)fn(x)", "loc"]
    );
    assert_eq!(text(&e, "kind"), "scopedfunction");
    let shown = run(&mut e, "h1");
    assert_eq!(shown, "h1 = \n\n    @(x)loc(x)\n\n");
    // An edited private file is seen by the next evaluation, as for calls.
    write(
        r,
        &["F", "private", "priv.m"],
        "function y = priv(x)\ny = x + 700;\nend\n",
    );
    run(&mut e, "e2 = h2(1);");
    assert_eq!(scalar(&e, "e2"), 701.);
}

// Made in a script file, and in a document run (CLI `matrust file.m`,
// desktop Run): the script's local functions and its folder's private folder.
#[test]
fn anonymous_function_made_in_a_script_or_document_keeps_its_file() {
    let root = folder();
    let r = root.path();
    anonymous_scope_tree(r);
    let source =
        "hd = @(x) priv(x);\nhl = @(x) dloc(x) + priv(x);\nfunction y = dloc(x)\ny = 2000;\nend\n";
    let document = write(r, &["F", "demo.m"], source);
    let mut e = engine(r);
    run(&mut e, "addpath('F', 'lib'); mks; a = hsc(1);");
    assert_eq!(scalar(&e, "a"), 1201.);
    e.execute_document(source, &document).unwrap();
    run(
        &mut e,
        "cd('other'); r = [hsc(1) hd(1) hl(1) callit(hd, 1)];",
    );
    assert_eq!(
        numbers(&e, "r"),
        [1201., 201., 2201., 201., 201., 201., 201., -4., -5.]
    );
    // A handle made at the Command Window does not see a document's files.
    let probe = "hp = @(x) priv(x);\n";
    let doc2 = write(r, &["F", "probe.m"], "z = hp(1);\n");
    run(&mut e, probe);
    e.execute_document("z = hp(1);\n", &doc2).unwrap();
    assert_eq!(scalar(&e, "z"), -2.);
}

// At the Command Window there is no file: a handle keeps the base import
// list it was made under, and only that one.
#[test]
fn anonymous_function_keeps_the_command_window_import_list() {
    let root = folder();
    let r = root.path();
    anonymous_scope_tree(r);
    let mut e = engine(r);
    run(
        &mut e,
        "addpath('lib'); h0 = @(x) fn(x); import pkg.*; h = @() fn(); clear import; a = h();",
    );
    assert_eq!(scalar(&e, "a"), 300.);
    run(&mut e, "b = h(); c = cellfun(@(k) k(), {h});");
    assert_eq!(scalar(&e, "b"), 300.);
    assert_eq!(scalar(&e, "c"), 300.);
    undefined(&fails(&mut e, "fn()"), "fn");
    // Made before the import: the import never reaches it.
    undefined(&fails(&mut e, "h0(1)"), "fn");
    run(&mut e, "import pkg.*");
    undefined(&fails(&mut e, "h0(1)"), "fn");
    // A file's import list does not reach a Command Window handle it calls.
    run(&mut e, "clear import; cd('other'); x = callit(h0, 1);");
    assert_eq!(numbers(&e, "x"), [-3., -3., -3., -3., -4., -5.]);
}
