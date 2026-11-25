//! User search path: lookup order (current folder, path folders, builtins),
//! path/addpath/rmpath/genpath/restoredefaultpath/rehash. Temporary folders only.
use matrust::{Array, Engine};
use std::{
    fs,
    path::{Path, PathBuf},
};

const SEP: &str = if cfg!(windows) { ";" } else { ":" };

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
/// A function file `name.m` returning the text `tag`.
fn tagged(dir: &Path, name: &str, tag: &str) {
    fs::write(
        dir.join(format!("{name}.m")),
        format!("function y = {name}()\n% {name} from {tag}\ny = '{tag}';\nend\n"),
    )
    .unwrap();
}
fn joined(folders: &[&Path]) -> String {
    folders
        .iter()
        .map(|f| real(f).to_string_lossy().into_owned())
        .collect::<Vec<_>>()
        .join(SEP)
}

#[test]
fn path_folders_resolve_in_order_with_begin_and_end() {
    let (cur, a, b) = (folder(), folder(), folder());
    tagged(a.path(), "who_am_i", "a");
    tagged(b.path(), "who_am_i", "b");
    let mut e = engine(cur.path());
    let (qa, qb) = (quoted(a.path()), quoted(b.path()));
    run(&mut e, &format!("addpath({qa}); r1 = who_am_i();"));
    assert_eq!(text(&e, "r1"), "a");
    // Default -begin: b goes on top; the order is kept across runs.
    run(
        &mut e,
        &format!("addpath({qb}); r2 = who_am_i(); p = path;"),
    );
    assert_eq!(text(&e, "r2"), "b");
    assert_eq!(text(&e, "p"), joined(&[b.path(), a.path()]));
    // -end moves b to the bottom; inside one run the new order applies at once.
    run(
        &mut e,
        &format!("r3 = who_am_i(); addpath({qb}, '-end'); r4 = who_am_i(); p = path;"),
    );
    assert_eq!(text(&e, "r3"), "b");
    assert_eq!(text(&e, "r4"), "a");
    assert_eq!(text(&e, "p"), joined(&[a.path(), b.path()]));
    // Several folders keep their order at the top; -frozen is accepted.
    run(
        &mut e,
        &format!(
            "restoredefaultpath; old = addpath({qb}, {qa}, '-frozen'); p = path; r5 = who_am_i();"
        ),
    );
    assert_eq!(text(&e, "old"), "");
    assert_eq!(text(&e, "p"), joined(&[b.path(), a.path()]));
    assert_eq!(text(&e, "r5"), "b");
    // A pathsep-separated argument (genpath output).
    run(
        &mut e,
        &format!("restoredefaultpath; addpath([{qa} pathsep {qb}]); p = path;"),
    );
    assert_eq!(text(&e, "p"), joined(&[a.path(), b.path()]));
}

#[test]
fn current_folder_wins_over_path_folders() {
    let (cur, a) = (folder(), folder());
    tagged(cur.path(), "who_am_i", "current");
    tagged(a.path(), "who_am_i", "a");
    let mut e = engine(cur.path());
    run(
        &mut e,
        &format!(
            "addpath({}); r = who_am_i(); w = which('who_am_i');",
            quoted(a.path())
        ),
    );
    assert_eq!(text(&e, "r"), "current");
    assert_eq!(
        text(&e, "w"),
        real(cur.path()).join("who_am_i.m").to_string_lossy()
    );
    // A path folder equal to the current folder is not searched twice.
    run(
        &mut e,
        &format!(
            "addpath({}); n = numel(which('who_am_i', '-all'));",
            quoted(cur.path())
        ),
    );
    assert_eq!(scalar(&e, "n"), 2.);
}

#[test]
fn path_file_shadows_builtin_until_removed() {
    let (cur, d) = (folder(), folder());
    fs::write(
        d.path().join("mean.m"),
        "function y = mean(x)\ny = 42;\nend\n",
    )
    .unwrap();
    let mut e = engine(cur.path());
    let q = quoted(d.path());
    run(
        &mut e,
        &format!(
            "m1 = mean([1 2 3]); addpath({q}); m2 = mean([1 2 3]); w = which('mean'); rmpath({q}); m3 = mean([1 2 3]);"
        ),
    );
    assert_eq!(scalar(&e, "m1"), 2.);
    assert_eq!(scalar(&e, "m2"), 42.);
    assert_eq!(
        text(&e, "w"),
        real(d.path()).join("mean.m").to_string_lossy()
    );
    assert_eq!(scalar(&e, "m3"), 2.);
}

#[test]
fn cached_miss_resolves_after_addpath_and_rmpath_hides_again() {
    let (cur, d) = (folder(), folder());
    tagged(d.path(), "only_on_path", "d");
    let mut e = engine(cur.path());
    let q = quoted(d.path());
    run(
        &mut e,
        &format!(
            "try; only_on_path(); miss = 0; catch; miss = 1; end\naddpath({q}); r = only_on_path();\nrmpath({q}); try; only_on_path(); gone = 0; catch err; gone = 1; msg = err.message; end"
        ),
    );
    assert_eq!(scalar(&e, "miss"), 1.);
    assert_eq!(text(&e, "r"), "d");
    assert_eq!(scalar(&e, "gone"), 1.);
    assert!(
        text(&e, "msg").contains("only_on_path"),
        "{}",
        text(&e, "msg")
    );
}

#[test]
fn which_exist_help_feval_and_handles_agree() {
    let (cur, a, b) = (folder(), folder(), folder());
    fs::write(
        a.path().join("twice.m"),
        "function y = twice(x)\n% TWICE doubles its input.\ny = 2*x;\nend\n",
    )
    .unwrap();
    fs::write(
        b.path().join("twice.m"),
        "function y = twice(x)\n% TWICE from b.\ny = 3*x;\nend\n",
    )
    .unwrap();
    let mut e = engine(cur.path());
    run(
        &mut e,
        &format!(
            "addpath({}, {}); w = which('twice'); all = which('twice', '-all'); x = exist('twice'); xf = exist('twice', 'file'); xm = exist('twice.m');\nf1 = feval('twice', 2); h = str2func('twice'); f2 = h(2); g = @twice; f3 = g(2); fi = functions(g); ff = fi.file;",
            quoted(a.path()),
            quoted(b.path())
        ),
    );
    let first = real(a.path()).join("twice.m");
    assert_eq!(text(&e, "w"), first.to_string_lossy());
    assert_eq!(text(&e, "ff"), first.to_string_lossy());
    for name in ["f1", "f2", "f3"] {
        assert_eq!(scalar(&e, name), 4., "{name}");
    }
    for name in ["x", "xf", "xm"] {
        assert_eq!(scalar(&e, name), 2., "{name}");
    }
    match e.workspace().get("all").unwrap() {
        Array::Cell(c) => assert_eq!(c.len(), 2),
        x => panic!("which -all: {}", x.class()),
    }
    let help = matrust::help::help_text("twice", &e.help_path());
    assert!(help.contains("TWICE doubles its input."), "{help}");
    let printed = run(&mut e, "which twice -all");
    assert!(printed.starts_with(&*first.to_string_lossy()), "{printed}");
    assert!(printed.contains("% Shadowed"), "{printed}");
    let printed = run(&mut e, "help twice");
    assert!(printed.contains("TWICE doubles its input."), "{printed}");
    // A name nowhere on the path.
    run(
        &mut e,
        "w0 = which('not_here_anywhere'); x0 = exist('not_here_anywhere');",
    );
    assert_eq!(text(&e, "w0"), "");
    assert_eq!(scalar(&e, "x0"), 0.);
}

#[test]
fn run_finds_scripts_on_the_path() {
    let (cur, d) = (folder(), folder());
    fs::write(d.path().join("setx.m"), "x_from_script = 5;\n").unwrap();
    let mut e = engine(cur.path());
    run(
        &mut e,
        &format!(
            "addpath({}); run('setx'); a = x_from_script; clear x_from_script; run('setx.m'); b = x_from_script;",
            quoted(d.path())
        ),
    );
    assert_eq!(scalar(&e, "a"), 5.);
    assert_eq!(scalar(&e, "b"), 5.);
}

#[test]
fn genpath_walks_sorted_and_skips_excluded_folders() {
    let root = folder();
    let r = root.path();
    for sub in [
        "b",
        "a/x",
        "a/private",
        ".git",
        "@cls",
        "+pkg",
        "private",
        "resources",
        "c/d",
    ] {
        fs::create_dir_all(r.join(sub)).unwrap();
    }
    fs::write(r.join("file.m"), "x=1;\n").unwrap();
    let mut e = engine(r);
    run(
        &mut e,
        &format!(
            "p = genpath({}); q = genpath('a'); z = genpath('no_such_folder');",
            quoted(r)
        ),
    );
    let s = |p: &Path| format!("{}{SEP}", p.to_string_lossy());
    let expected = [
        r.to_path_buf(),
        r.join("a"),
        r.join("a").join("x"),
        r.join("b"),
        r.join("c"),
        r.join("c").join("d"),
    ]
    .iter()
    .map(|p| s(p))
    .collect::<String>();
    assert_eq!(text(&e, "p"), expected);
    let current = real(e.current_folder().unwrap());
    assert_eq!(
        text(&e, "q"),
        format!(
            "{}{}",
            s(&current.join("a")),
            s(&current.join("a").join("x"))
        )
    );
    assert_eq!(text(&e, "z"), "");
    // addpath(genpath(...)) adds every listed folder in order.
    run(
        &mut e,
        &format!(
            "restoredefaultpath; addpath(genpath({})); n = numel(strsplit(path, pathsep));",
            quoted(r)
        ),
    );
    assert_eq!(scalar(&e, "n"), 6.);
}

#[test]
fn missing_folders_and_unknown_removals_warn() {
    let (cur, d) = (folder(), folder());
    let mut e = engine(cur.path());
    let q = quoted(d.path());
    let printed = run(
        &mut e,
        &format!("addpath('no_such_dir_xyz', {q}); p = path;"),
    );
    assert!(
        printed.contains("Warning: Name is nonexistent or not a directory: no_such_dir_xyz"),
        "{printed}"
    );
    assert_eq!(text(&e, "p"), joined(&[d.path()]));
    let printed = run(&mut e, "rmpath('not_on_path_xyz'); p = path;");
    assert!(
        printed.contains("Warning: \"not_on_path_xyz\" not found in path."),
        "{printed}"
    );
    assert_eq!(text(&e, "p"), joined(&[d.path()]));
}

#[test]
fn path_set_and_concatenate_forms() {
    let (cur, a, b) = (folder(), folder(), folder());
    let mut e = engine(cur.path());
    let (qa, qb) = (quoted(a.path()), quoted(b.path()));
    run(
        &mut e,
        &format!("p0 = path; path({qa}); p1 = path; old = path({qa}, {qb}); p2 = path;"),
    );
    assert_eq!(text(&e, "p0"), "");
    assert_eq!(text(&e, "p1"), joined(&[a.path()]));
    assert_eq!(text(&e, "old"), joined(&[a.path()]));
    assert_eq!(text(&e, "p2"), joined(&[a.path(), b.path()]));
    // Empty entries are dropped; duplicates kept once.
    run(
        &mut e,
        &format!("path([pathsep pathsep {qb} pathsep pathsep {qb} pathsep]); p3 = path;"),
    );
    assert_eq!(text(&e, "p3"), joined(&[b.path()]));
    // `path` without an output prints one folder per line.
    run(&mut e, &format!("path({qa}, {qb});"));
    let printed = run(&mut e, "path");
    assert_eq!(
        printed,
        format!(
            "{}\n{}\n",
            real(a.path()).display(),
            real(b.path()).display()
        )
    );
    // A variable named path still wins over the function.
    run(&mut e, "path = 3; v = path; clear path");
    assert_eq!(scalar(&e, "v"), 3.);
    // Relative names resolve against the current folder.
    fs::create_dir(cur.path().join("rel")).unwrap();
    run(&mut e, "path('rel'); p4 = path;");
    assert_eq!(text(&e, "p4"), joined(&[&cur.path().join("rel")]));
    // Command syntax for addpath and rmpath.
    run(
        &mut e,
        "restoredefaultpath\naddpath rel\np5 = path;\nrmpath rel\np6 = path;",
    );
    assert_eq!(text(&e, "p5"), joined(&[&cur.path().join("rel")]));
    assert_eq!(text(&e, "p6"), "");
}

#[test]
fn restoredefaultpath_clears_user_folders() {
    let (cur, d) = (folder(), folder());
    tagged(d.path(), "only_on_path", "d");
    let mut e = engine(cur.path());
    run(
        &mut e,
        &format!("addpath({}); r = only_on_path();", quoted(d.path())),
    );
    assert_eq!(text(&e, "r"), "d");
    run(
        &mut e,
        "restoredefaultpath; p = path; try; only_on_path(); ok = 0; catch; ok = 1; end",
    );
    assert_eq!(text(&e, "p"), "");
    assert_eq!(scalar(&e, "ok"), 1.);
}

#[test]
fn rehash_finds_a_file_created_in_a_path_folder() {
    let (cur, d) = (folder(), folder());
    let mut e = engine(cur.path());
    let q = quoted(d.path());
    run(
        &mut e,
        &format!(
            "addpath({q}); try; made_later(); before = 0; catch; before = 1; end\nfid = fopen(fullfile({q}, 'made_later.m'), 'w'); fprintf(fid, 'function y = made_later\\ny = 7;\\nend\\n'); fclose(fid);\nrehash path\ny = made_later();"
        ),
    );
    assert_eq!(scalar(&e, "before"), 1.);
    assert_eq!(scalar(&e, "y"), 7.);
    assert!(e.execute("rehash nonsense_option").is_err());
}

#[test]
fn rehash_replaces_a_cached_hit_with_an_earlier_path_file() {
    for rehash in ["rehash", "rehash path"] {
        let (cur, a, b) = (folder(), folder(), folder());
        tagged(b.path(), "f", "b");
        let mut e = engine(cur.path());
        run(
            &mut e,
            &format!(
                "addpath({}, {}); before = f();\nfid = fopen({}, 'w'); fprintf(fid, 'function y = f\\ny = ''a'';\\nend\\n'); fclose(fid);\n{rehash}\nafter = f();",
                quoted(a.path()),
                quoted(b.path()),
                quoted(&a.path().join("f.m"))
            ),
        );
        assert_eq!(text(&e, "before"), "b", "{rehash}");
        assert_eq!(text(&e, "after"), "a", "{rehash}");
    }
}

#[test]
fn run_with_an_explicit_folder_does_not_fall_back_to_the_path() {
    let (cur, path, empty) = (folder(), folder(), folder());
    fs::write(path.path().join("f.m"), "from_path = 1;\n").unwrap();
    for folder in [empty.path(), cur.path()] {
        let mut e = engine(cur.path());
        let target = folder.join("f");
        run(
            &mut e,
            &format!(
                "addpath({}); run('f'); clear from_path; try; run({}); failed = 0; message = ''; catch err; failed = 1; message = err.message; end; run('f');",
                quoted(path.path()),
                quoted(&target)
            ),
        );
        assert_eq!(scalar(&e, "failed"), 1.);
        let message = text(&e, "message");
        assert!(
            message.contains(&format!("{} not found.", target.display())),
            "{message}"
        );
        assert_eq!(scalar(&e, "from_path"), 1.);
    }
}

/// Use engine-local environment overrides: parallel tests never mutate process
/// HOME or preferences, and no test reads or writes real user settings.
fn preferences(root: &Path) -> Engine {
    let home = root.join("home");
    fs::create_dir_all(&home).unwrap();
    let mut e = engine(root);
    for (key, value) in [
        ("MATRUST_PREFDIR", root.join("prefs")),
        ("HOME", home.clone()),
        ("USERPROFILE", home),
        ("APPDATA", root.join("roaming")),
        ("XDG_CONFIG_HOME", root.join("config")),
    ] {
        run(&mut e, &format!("setenv('{key}', {});", quoted(&value)));
    }
    run(
        &mut e,
        "setenv('MATRUST_SESSION_PATH',''); setenv('LOCALAPPDATA','');",
    );
    e
}

fn cli(root: &Path) -> std::process::Command {
    let mut cmd = std::process::Command::new(env!("CARGO_BIN_EXE_matrust"));
    cmd.current_dir(root)
        .env("MATRUST_PREFDIR", root.join("prefs"))
        .env("HOME", root.join("home"))
        .env("USERPROFILE", root.join("home"))
        .env_remove("MATRUST_SESSION_PATH")
        .env_remove("LOCALAPPDATA");
    cmd
}

#[test]
fn prefdir_creates_overridden_settings_folder_without_default_engine_io() {
    let root = folder();
    let mut e = preferences(root.path());
    assert!(!root.path().join("prefs").exists());
    run(&mut e, "p0=path; d=prefdir;");
    assert_eq!(text(&e, "p0"), "");
    assert_eq!(text(&e, "d"), root.path().join("prefs").to_string_lossy());
    assert!(root.path().join("prefs").is_dir());
    fs::remove_dir(root.path().join("prefs")).unwrap();
    run(&mut e, "d=prefdir;");
    assert!(root.path().join("prefs").is_dir());
    assert!(e.execute("prefdir(1)").is_err());
}

#[test]
fn prefdir_reuses_desktop_settings_then_platform_folder() {
    let root = folder();
    let mut e = preferences(root.path());
    let session = root.path().join("desktop/session.json");
    run(
        &mut e,
        &format!(
            "setenv('MATRUST_PREFDIR',''); setenv('MATRUST_SESSION_PATH',{}); d=prefdir;",
            quoted(&session)
        ),
    );
    assert_eq!(text(&e, "d"), session.parent().unwrap().to_string_lossy());
    let local = root.path().join("local");
    run(
        &mut e,
        &format!(
            "setenv('MATRUST_SESSION_PATH',''); setenv('LOCALAPPDATA',{}); d=prefdir;",
            quoted(&local)
        ),
    );
    assert_eq!(text(&e, "d"), local.join("MatRust").to_string_lossy());
    run(&mut e, "setenv('LOCALAPPDATA',''); d=prefdir;");
    let expected = if cfg!(windows) {
        root.path().join("roaming").join("MatRust")
    } else if cfg!(target_os = "macos") {
        root.path().join("home/Library/Application Support/MatRust")
    } else {
        root.path().join("config/matrust")
    };
    assert_eq!(text(&e, "d"), expected.to_string_lossy());
    if !cfg!(windows) && !cfg!(target_os = "macos") {
        run(&mut e, "setenv('XDG_CONFIG_HOME',''); d=prefdir;");
        assert_eq!(
            text(&e, "d"),
            root.path().join("home/.config/matrust").to_string_lossy()
        );
    }
}

#[test]
fn userpath_set_reset_clear_persist_and_update_live_lookup() {
    let root = folder();
    let (a, b, other) = (folder(), folder(), folder());
    tagged(a.path(), "user_choice", "a");
    tagged(b.path(), "user_choice", "b");
    let mut e = preferences(root.path());
    let default = root.path().join("home").join("Documents").join("MatRust");
    run(&mut e, "u=userpath; p=path;");
    assert_eq!(text(&e, "u"), default.to_string_lossy());
    assert_eq!(text(&e, "p"), "");
    run(&mut e, "userpath;");
    assert_eq!(text(&e, "ans"), default.to_string_lossy());
    assert!(
        !default.exists(),
        "query must not create the default user folder"
    );
    run(
        &mut e,
        &format!(
            "addpath({}); userpath({}); u=userpath; p=path; r=user_choice();",
            quoted(other.path()),
            quoted(a.path())
        ),
    );
    assert_eq!(text(&e, "u"), real(a.path()).to_string_lossy());
    assert_eq!(text(&e, "p"), joined(&[a.path(), other.path()]));
    assert_eq!(text(&e, "r"), "a");
    assert!(e.execute("userpath('missing_folder')").is_err());
    assert!(e.execute("userpath('')").is_err());
    run(
        &mut e,
        &format!("userpath({}); p=path; r=user_choice();", quoted(b.path())),
    );
    assert_eq!(text(&e, "p"), joined(&[b.path(), other.path()]));
    assert_eq!(text(&e, "r"), "b");
    let mut next = preferences(root.path());
    run(&mut next, "u=userpath; p=path;");
    assert_eq!(text(&next, "u"), real(b.path()).to_string_lossy());
    assert_eq!(
        text(&next, "p"),
        "",
        "default engines do not load preferences"
    );
    assert!(next.spath_load_saved_path().is_empty());
    run(&mut next, "p=path; r=user_choice();");
    assert_eq!(text(&next, "p"), joined(&[b.path()]));
    assert_eq!(text(&next, "r"), "b");
    run(&mut e, "userpath('clear'); u=userpath; p=path;");
    assert_eq!(text(&e, "u"), "");
    assert_eq!(text(&e, "p"), joined(&[other.path()]));
    let mut next = preferences(root.path());
    assert!(next.spath_load_saved_path().is_empty());
    run(&mut next, "u=userpath; p=path;");
    assert_eq!(text(&next, "u"), "");
    assert_eq!(text(&next, "p"), "");
    // Reset persists the default without creating it; add it only if it exists.
    run(&mut e, "userpath('reset'); u=userpath; p=path;");
    assert_eq!(text(&e, "u"), default.to_string_lossy());
    assert_eq!(text(&e, "p"), joined(&[other.path()]));
    fs::create_dir_all(&default).unwrap();
    run(&mut e, "userpath('reset'); p=path;");
    assert_eq!(text(&e, "p"), joined(&[&default, other.path()]));
    let mut next = preferences(root.path());
    assert!(next.spath_load_saved_path().is_empty());
    run(&mut next, "u=userpath; p=path;");
    assert_eq!(text(&next, "u"), default.to_string_lossy());
    assert_eq!(text(&next, "p"), joined(&[&default]));
}

#[test]
fn savepath_round_trip_quotes_spaces_order_and_load_once() {
    let root = folder();
    let quoted_folder = root.path().join("a 'quoted' folder");
    let second = root.path().join("second folder");
    let user = root.path().join("home").join("Documents").join("MatRust");
    for p in [&quoted_folder, &second, &user] {
        fs::create_dir_all(p).unwrap();
    }
    tagged(&quoted_folder, "saved_choice", "first");
    tagged(&second, "saved_choice", "second");
    let mut e = preferences(root.path());
    run(
        &mut e,
        &format!(
            "addpath({}, {}); status=savepath;",
            quoted(&quoted_folder),
            quoted(&second)
        ),
    );
    assert_eq!(scalar(&e, "status"), 0.);
    let pathdef = root.path().join("prefs/pathdef.m");
    let saved = fs::read_to_string(&pathdef).unwrap();
    assert!(saved.starts_with("function p = pathdef\np = [ ...\n"));
    assert!(saved.contains("a ''quoted'' folder' pathsep ...\n"));
    run(&mut e, "restoredefaultpath; p=path;");
    assert_eq!(text(&e, "p"), "");
    assert_eq!(fs::read_to_string(&pathdef).unwrap(), saved);
    // Saved folders plus the default userpath, always on top at startup.
    let mut next = preferences(root.path());
    assert!(next.spath_load_saved_path().is_empty());
    run(&mut next, "p=path; r=saved_choice();");
    assert_eq!(text(&next, "p"), joined(&[&user, &quoted_folder, &second]));
    assert_eq!(text(&next, "r"), "first");
    run(&mut next, "restoredefaultpath;");
    assert!(next.spath_load_saved_path().is_empty());
    run(&mut next, "p=path;");
    assert_eq!(
        text(&next, "p"),
        "",
        "startup only loads once, across execute/reset_run"
    );
    // Explicit relative destinations use the engine's current folder.
    run(&mut e, "status=savepath('explicit.m');");
    assert_eq!(scalar(&e, "status"), 0.);
    assert_eq!(
        fs::read_to_string(root.path().join("explicit.m")).unwrap(),
        "function p = pathdef\np = [ ...\n'' ...\n];\nend\n"
    );
    run(&mut e, "status=savepath; userpath('clear');");
    let mut empty = preferences(root.path());
    assert!(empty.spath_load_saved_path().is_empty());
    run(&mut empty, "p=path;");
    assert_eq!(text(&empty, "p"), "");
}

#[test]
fn userpath_changes_remove_this_sessions_old_folder_after_another_session_writes() {
    let root = folder();
    let (a, b, c) = (folder(), folder(), folder());
    let mut first = preferences(root.path());
    run(&mut first, &format!("userpath({});", quoted(a.path())));
    let mut second = preferences(root.path());
    run(&mut second, &format!("userpath({});", quoted(b.path())));
    run(
        &mut first,
        &format!("userpath({}); p=path; u=userpath;", quoted(c.path())),
    );
    assert_eq!(text(&first, "p"), joined(&[c.path()]));
    assert_eq!(text(&first, "u"), real(c.path()).to_string_lossy());
    run(&mut second, "p=path; u=userpath;");
    assert_eq!(text(&second, "p"), joined(&[b.path()]));
    assert_eq!(text(&second, "u"), real(b.path()).to_string_lossy());
}

#[test]
fn savepath_status_and_warning_on_write_failure() {
    let root = folder();
    let mut e = preferences(root.path());
    assert_eq!(run(&mut e, "status=savepath;"), "");
    assert_eq!(scalar(&e, "status"), 0.);
    assert_eq!(run(&mut e, "status=savepath('missing/pathdef.m');"), "");
    assert_eq!(scalar(&e, "status"), 1.);
    let printed = run(&mut e, "savepath('missing/pathdef.m');");
    assert_eq!(printed.matches("Warning:").count(), 1, "{printed}");
    assert!(printed.contains("savepath:"), "{printed}");
    assert!(e.execute("savepath(42)").is_err());
    assert!(e.execute("[a,b]=savepath").is_err());
    fs::write(root.path().join("not_a_folder"), "file").unwrap();
    run(
        &mut e,
        &format!(
            "setenv('MATRUST_PREFDIR',{}); status=savepath;",
            quoted(&root.path().join("not_a_folder"))
        ),
    );
    assert_eq!(scalar(&e, "status"), 1.);
}

#[test]
fn cli_batch_and_interactive_startup_load_saved_path_and_userpath() {
    use std::{io::Write, process::Stdio};
    let root = folder();
    let saved = root.path().join("saved 'work' folder");
    let user = root.path().join("user folder");
    fs::create_dir_all(&saved).unwrap();
    fs::create_dir_all(&user).unwrap();
    tagged(&saved, "startup_only", "saved");
    tagged(&saved, "user_priority", "saved");
    tagged(&user, "user_priority", "user");
    let mut e = preferences(root.path());
    run(
        &mut e,
        &format!(
            "addpath({}); userpath({}); status=savepath;",
            quoted(&saved),
            quoted(&user)
        ),
    );
    let result = cli(root.path())
        .args(["-batch", "disp(startup_only()); disp(user_priority());"])
        .output()
        .unwrap();
    assert!(result.status.success(), "{result:?}");
    assert_eq!(
        String::from_utf8_lossy(&result.stdout).replace("\r\n", "\n"),
        "saved\nuser\n"
    );
    assert!(result.stderr.is_empty(), "{result:?}");
    let mut child = cli(root.path())
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    child
        .stdin
        .take()
        .unwrap()
        .write_all(b"disp(startup_only());\ndisp(user_priority());\nexit\n")
        .unwrap();
    let result = child.wait_with_output().unwrap();
    assert!(result.status.success(), "{result:?}");
    assert_eq!(
        String::from_utf8_lossy(&result.stdout).replace("\r\n", "\n"),
        "saved\nuser\n"
    );
    assert!(result.stderr.is_empty(), "{result:?}");
}

#[test]
fn startup_rejects_malformed_pathdef_without_executing_it() {
    let root = folder();
    let mut e = preferences(root.path());
    run(&mut e, "userpath('clear'); status=savepath;");
    let pathdef = root.path().join("prefs/pathdef.m");
    let valid = fs::read_to_string(&pathdef).unwrap();
    let bad = [
        "disp('PATHDEF_EXECUTED'); error('must not run');".to_owned(),
        valid.replace("'' ...", "'unterminated ..."),
        format!("{valid}disp('PATHDEF_EXECUTED');\n"),
        valid.replace("'' ...", "fullfile('a','b') ..."),
        valid.replace("'' ...", "'a'b' ..."),
    ];
    for source in bad {
        fs::write(&pathdef, source).unwrap();
        let mut e = preferences(root.path());
        let warnings = e.spath_load_saved_path();
        assert_eq!(warnings.len(), 1, "{warnings:?}");
        assert!(
            warnings[0].contains("Unparseable pathdef.m"),
            "{warnings:?}"
        );
        run(&mut e, "p=path;");
        assert_eq!(text(&e, "p"), "");
        let result = cli(root.path())
            .args(["-batch", "disp(isempty(path));"])
            .output()
            .unwrap();
        assert!(result.status.success(), "{result:?}");
        assert_eq!(String::from_utf8_lossy(&result.stdout).trim(), "1");
        let err = String::from_utf8_lossy(&result.stderr);
        assert_eq!(err.matches("Warning:").count(), 1, "{err}");
        assert!(err.contains("Unparseable pathdef.m"), "{err}");
        assert!(!err.contains("PATHDEF_EXECUTED"), "{err}");
    }
}

#[test]
fn startup_warns_once_per_missing_saved_folder_and_keeps_existing_order() {
    let root = folder();
    let mut e = preferences(root.path());
    let dirs: Vec<_> = ["first", "missing1", "missing2", "last"]
        .iter()
        .map(|s| root.path().join(s))
        .collect();
    for p in &dirs {
        fs::create_dir(p).unwrap();
    }
    run(
        &mut e,
        &format!(
            "userpath('clear'); addpath({}); status=savepath;",
            dirs.iter().map(|p| quoted(p)).collect::<Vec<_>>().join(",")
        ),
    );
    fs::remove_dir(&dirs[1]).unwrap();
    fs::remove_dir(&dirs[2]).unwrap();
    let mut e = preferences(root.path());
    let warnings = e.spath_load_saved_path();
    assert_eq!(warnings.len(), 2, "{warnings:?}");
    assert!(warnings[0].contains("missing1"), "{warnings:?}");
    assert!(warnings[1].contains("missing2"), "{warnings:?}");
    run(&mut e, "p=path;");
    assert_eq!(text(&e, "p"), joined(&[&dirs[0], &dirs[3]]));
}

#[cfg(unix)]
#[test]
fn startup_skips_fifo_pathdef_without_blocking() {
    use std::io::ErrorKind;
    use std::process::{Command, Stdio};
    use std::time::{Duration, Instant};

    let root = folder();
    let prefs = root.path().join("prefs");
    fs::create_dir_all(&prefs).unwrap();
    let fifo = prefs.join("pathdef.m");
    match Command::new("mkfifo").arg(&fifo).status() {
        Ok(status) => assert!(status.success(), "mkfifo failed: {status:?}"),
        Err(e) if e.kind() == ErrorKind::NotFound => return,
        Err(e) => panic!("mkfifo failed: {e}"),
    }

    let mut child = cli(root.path())
        .args(["-batch", "disp(1)"])
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    let deadline = Instant::now() + Duration::from_secs(10);
    loop {
        if child.try_wait().unwrap().is_some() {
            break;
        }
        if Instant::now() >= deadline {
            let _ = child.kill();
            let _ = child.wait();
            panic!("matrust -batch hung on FIFO pathdef.m");
        }
        std::thread::sleep(Duration::from_millis(50));
    }
    let result = child.wait_with_output().unwrap();
    assert!(result.status.success(), "{result:?}");
    assert_eq!(String::from_utf8_lossy(&result.stdout).trim(), "1");
    let err = String::from_utf8_lossy(&result.stderr);
    assert!(err.contains("Warning:"), "{err}");
}
