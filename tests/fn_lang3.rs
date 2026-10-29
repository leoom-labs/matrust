//! Replay the reference lang3 corpus (tests/fixtures/fn/lang3-expected.txt).
//! The reference ran every case in one workspace, clearing only `x` before each,
//! so the cases replay in order in one engine and one current folder; cases
//! that use tempdir get their own folder inside this worktree.
use matrust::Engine;

const REFERENCE: &str = include_str!("fixtures/fn/lang3-expected.txt");
const CASES: usize = 188;

const SCOPED: &str = "A handle to a local function records its file (functions reports it) but is still called by name, so it cannot run outside that file (function-handle core).";
const NESTED: &str =
    "Nested functions (a function block inside a function) are not parsed by this engine yet.";
const EMPTY_STRUCT: &str =
    "Empty struct-array display right-aligns field names (display code, listed in FOLLOWUPS).";
const MAP: &str =
    "containers.Map is added by another branch; this engine cannot construct one yet.";

/// Cases that cannot pass for a reason outside this task.
const KNOWN: &[(usize, &str)] = &[
    (
        16,
        "Engine-wide concatenation error wording: 'Horizontal concatenation requires matching row counts'.",
    ),
    (43, EMPTY_STRUCT),
    (45, EMPTY_STRUCT),
    (54, MAP),
    (101, SCOPED),
    (104, NESTED),
    (105, NESTED),
    (
        108,
        "Cell display shows function handles as [1x1 function_handle] instead of @name (display code).",
    ),
    (110, SCOPED),
    (
        144,
        "Engine-wide matrix power error wording differs from the reference's 'Inputs must be a scalar and a square matrix.'",
    ),
    (
        158,
        "Oracle harness artefact: MATLAB turns diary off inside evalc, so the file stayed empty; diary writing is tested in diary_writes_command_window_text.",
    ),
    (
        162,
        "Oracle harness artefact: MATLAB turns diary off inside evalc, so the file kept only 'old'; diary appending is tested in diary_writes_command_window_text.",
    ),
];

const TEMPDIR: &str =
    "Bind tempdir to a per-case folder so files stay inside the worktree; exact result.";
const LINKS: &str = "Strip MATLAB desktop help-link markup, keeping its visible text.";
const WARNING: &str =
    "Compare the [Warning: ...] line only, removing the desktop backtrace and control bytes.";
const ENV: &str = "ENV: class and recorded size only (no error); character-size exceptions are individually documented in ENV_CHAR_SIZE.";

/// ENV character sizes that depend on the machine or reference runner. Only these
/// cases may differ from the recorded column count; class and rows stay exact.
/// Paths/rendered paths must remain nonempty. mexext checks the platform's exact
/// length (6 on Linux/Windows, 9 on macOS), not just a positive length.
const ENV_CHAR_SIZE: &[(usize, &str)] = &[
    (168, "which('sin') includes the installation directory."),
    (
        169,
        "Printed which sin includes the installation directory and host display text.",
    ),
    (
        170,
        "which('ismethod') includes the installation directory.",
    ),
    (171, "which('for') includes the installation directory."),
    (
        173,
        "which of the helper file includes the runner's current directory.",
    ),
    (174, "what.path includes the machine's temporary directory."),
    (
        175,
        "Printed what includes the temporary directory and host display text.",
    ),
    (
        177,
        "The local-function file includes the runner's current directory.",
    ),
    (
        178,
        "dbstack('-completenames') includes the runner's current directory.",
    ),
    (
        179,
        "Printed dbstack includes oracle runner frames and desktop links with host paths.",
    ),
    (
        181,
        "DiaryFile includes the machine's temporary directory from earlier cases.",
    ),
    (
        182,
        "mexext is mexw64/mexa64 (6 characters) or mexmaci64 (9), depending on the platform.",
    ),
];

/// Every case compared with a relaxed rule. Everything else is exact.
const HARNESS: &[(usize, &str)] = &[
    (39, TEMPDIR),
    (40, TEMPDIR),
    (41, TEMPDIR),
    (42, TEMPDIR),
    (43, LINKS),
    (45, LINKS),
    (97, LINKS),
    (150, WARNING),
    (151, WARNING),
    (158, TEMPDIR),
    (159, TEMPDIR),
    (160, TEMPDIR),
    (161, TEMPDIR),
    (162, TEMPDIR),
    (168, ENV),
    (169, ENV),
    (170, ENV),
    (171, ENV),
    (172, ENV),
    (173, ENV),
    (174, ENV),
    (175, ENV),
    (176, ENV),
    (177, ENV),
    (178, ENV),
    (179, ENV),
    (180, ENV),
    (181, ENV),
    (182, ENV),
    (183, ENV),
    (184, ENV),
    (185, ENV),
    (186, ENV),
    (187, ENV),
    (188, ENV),
];

struct Case<'a> {
    id: usize,
    source: &'a str,
    text: &'a str,
    class: Option<&'a str>,
}
fn cases() -> Vec<Case<'static>> {
    REFERENCE
        .split("@@CASE ")
        .skip(1)
        .map(|block| {
            let (header, block) = block.split_once('\n').unwrap();
            let (id, source) = header.split_once('|').unwrap();
            let (body, _) = block.split_once("@@END").unwrap();
            let (text, class) = if let Some((text, class)) = body.split_once("@@CLASS ") {
                (text, Some(class.trim()))
            } else {
                (body, None)
            };
            Case {
                id: id.parse().unwrap(),
                source,
                text,
                class,
            }
        })
        .collect()
}
fn rule(id: usize) -> Option<&'static str> {
    HARNESS.iter().find(|(n, _)| *n == id).map(|(_, r)| *r)
}
fn plain(text: &str, id: usize) -> String {
    let mut s = text.to_owned();
    if rule(id) == Some(LINKS) {
        while let Some(at) = s.find("<a href=\"") {
            let start = at + s[at..].find('>').unwrap() + 1;
            let end = start + s[start..].find("</a>").unwrap();
            let visible = s[start..end].to_owned();
            s.replace_range(at..end + 4, &visible);
        }
    }
    if rule(id) == Some(WARNING) {
        while let Some(at) = s.find("[\u{8}> In ") {
            let end = at + s[at..].find("]\u{8} \n").unwrap() + "]\u{8} \n".len();
            s.replace_range(at..end, "");
        }
        s = s
            .replace("[\u{8}Warning: ", "Warning: ")
            .replace("]\u{8} \n", "\n");
    }
    s
}
/// `class [rows cols]` as (class, rows, cols).
fn metadata_parts(m: &str) -> Option<(&str, usize, usize)> {
    let (class, size) = m.split_once(" [")?;
    let (rows, cols) = size.strip_suffix(']')?.split_once(' ')?;
    Some((class, rows.parse().ok()?, cols.parse().ok()?))
}
/// Default ENV rule: both class and recorded size must match exactly.
fn env_matches(actual: Option<&str>, expected: Option<&str>) -> bool {
    let (Some(a), Some(e)) = (
        actual.and_then(metadata_parts),
        expected.and_then(metadata_parts),
    ) else {
        return false;
    };
    a == e
}

fn env_case_matches(id: usize, actual: Option<&str>, expected: Option<&str>) -> bool {
    if !ENV_CHAR_SIZE.iter().any(|(n, _)| *n == id) {
        return env_matches(actual, expected);
    }
    let (Some(("char", 1, cols)), Some(("char", 1, expected_cols))) = (
        actual.and_then(metadata_parts),
        expected.and_then(metadata_parts),
    ) else {
        return false;
    };
    if id == 182 {
        cols == if cfg!(target_os = "macos") {
            9
        } else {
            expected_cols
        }
    } else {
        cols > 0 && expected_cols > 0
    }
}

#[test]
fn corpus_has_all_cases_and_documented_exceptions() {
    let all = cases();
    assert_eq!(all.len(), CASES);
    for (i, case) in all.iter().enumerate() {
        assert_eq!(case.id, i + 1);
    }
    for (n, reason) in KNOWN.iter().chain(HARNESS) {
        assert!(!reason.is_empty());
        assert!(all.iter().any(|c| c.id == *n));
    }
    for (n, reason) in ENV_CHAR_SIZE {
        assert!(!reason.is_empty());
        assert_eq!(rule(*n), Some(ENV));
        assert!(
            all.iter()
                .any(|c| c.id == *n && c.class.is_some_and(|m| m.starts_with("char [1 ")))
        );
    }
    // Every case that names tempdir gets its own folder.
    for case in &all {
        if case.source.contains("tempdir") && rule(case.id) != Some(ENV) {
            assert_eq!(rule(case.id), Some(TEMPDIR), "case {}", case.id);
        }
    }
}

fn test_root() -> std::path::PathBuf {
    std::env::temp_dir()
}
fn names(engine: &Engine) -> Vec<String> {
    engine.workspace().into_keys().collect()
}

/// Run one case in the shared engine, as the reference did: `format short;
/// clear x`, then the statement. Returns (matches, actual text, class).
fn run_case(engine: &mut Engine, case: &Case) -> (bool, String, Option<String>) {
    engine.execute("format short; clear x").unwrap();
    let scratch = tempfile::tempdir_in(test_root()).unwrap();
    let bound = case.source.contains("tempdir");
    if bound {
        let mut path = scratch.path().display().to_string().replace('\'', "''");
        path.push(std::path::MAIN_SEPARATOR);
        engine.execute(&format!("tempdir='{path}';")).unwrap();
    }
    let result = engine.execute(case.source);
    let actual = match result {
        Ok(execution) => execution
            .output
            .iter()
            .map(|s| format!("{s}\n"))
            .collect::<String>(),
        Err(error) => format!(
            "ERR {}\n",
            engine
                .diagnostic
                .as_ref()
                .map_or(error.as_str(), |d| d.message.as_str())
        ),
    };
    let metadata = engine.array("x").map(|a| {
        engine.execute("lang3_class=class(x);").unwrap();
        let Some(matrust::Array::Character(c)) = engine.array("lang3_class") else {
            panic!("class must return text")
        };
        format!(
            "{} [{} {}]",
            c.to_utf8(&Default::default()).unwrap(),
            a.rows(),
            a.cols()
        )
    });
    // The harness's own names leave the shared workspace again.
    engine.remove_array("lang3_class");
    if bound {
        engine.remove_array("tempdir");
    }
    let matches = match rule(case.id) {
        Some(ENV) => {
            !actual.starts_with("ERR ")
                && env_case_matches(case.id, metadata.as_deref(), case.class)
        }
        _ => {
            plain(&actual, case.id) == plain(case.text, case.id)
                && (case.class.is_none() || metadata.as_deref() == case.class)
        }
    };
    (matches, actual, metadata)
}

#[test]
fn replay_reference() {
    let folder = tempfile::tempdir_in(test_root()).unwrap();
    let mut engine = Engine::default();
    engine.set_file_directory(folder.path()).unwrap();
    engine.set_function_directory(folder.path()).unwrap();
    let mut passed = 0;
    let mut failures = Vec::new();
    for case in cases() {
        // 166-167 clear the whole workspace the earlier cases built up.
        let before = names(&engine);
        if case.id == 166 {
            assert!(before.len() > 20, "accumulated workspace: {before:?}");
        }
        let (matches, actual, metadata) = run_case(&mut engine, &case);
        match case.id {
            166 => assert_eq!(names(&engine), ["cv_l3k", "x"]),
            167 => assert_eq!(names(&engine), ["x"]),
            _ => {}
        }
        let mut report = |label: &str| {
            failures.push(format!(
                "case {}{label} `{}`\n expected {:?} {:?}\n actual   {:?} {:?}",
                case.id,
                case.source,
                plain(case.text, case.id),
                case.class,
                plain(&actual, case.id),
                metadata
            ))
        };
        if let Some((_, reason)) = KNOWN.iter().find(|(n, _)| *n == case.id) {
            if matches {
                report(" (KNOWN now passes; remove its exception)");
                continue;
            }
            eprintln!("KNOWN {}: {reason}", case.id);
        } else if matches {
            passed += 1;
        } else {
            report("");
        }
    }
    assert!(
        failures.is_empty(),
        "{passed}/{} passed; {} KNOWN; {} failures:\n{}",
        CASES - KNOWN.len(),
        KNOWN.len(),
        failures.len(),
        failures.join("\n")
    );
    assert_eq!(passed, CASES - KNOWN.len());
}

#[test]
fn env_rule_compares_class_and_size() {
    // Rev2 #4: a positive length does not waive the recorded size by default.
    assert!(!env_matches(Some("char [1 1]"), Some("char [1 57]")));
    assert!(!env_matches(Some("char [1 57]"), Some("char [1 74]")));
    assert!(env_matches(Some("char [1 57]"), Some("char [1 57]")));
    assert!(!env_matches(Some("char [0 0]"), Some("char [1 57]")));
    assert!(!env_matches(Some("double [1 1]"), Some("char [1 57]")));
    assert!(!env_matches(Some("double [1 2]"), Some("double [1 1]")));
    assert!(env_matches(Some("logical [1 1]"), Some("logical [1 1]")));
    assert!(!env_matches(None, Some("struct [1 1]")));
    assert!(!env_case_matches(
        0,
        Some("char [1 1]"),
        Some("char [1 57]")
    ));
    assert!(!env_case_matches(
        182,
        Some("char [1 1]"),
        Some("char [1 6]")
    ));
    assert!(env_case_matches(
        182,
        Some(if cfg!(target_os = "macos") {
            "char [1 9]"
        } else {
            "char [1 6]"
        }),
        Some("char [1 6]")
    ));
    // Only explicitly listed path cases allow a changed column count.
    assert!(env_case_matches(
        177,
        Some("char [1 74]"),
        Some("char [1 57]")
    ));
    assert!(!env_case_matches(
        177,
        Some("char [0 0]"),
        Some("char [1 57]")
    ));
    assert!(!env_case_matches(
        177,
        Some("char [2 57]"),
        Some("char [1 57]")
    ));
    assert!(!env_case_matches(
        177,
        Some("double [1 57]"),
        Some("char [1 57]")
    ));
}

fn text(e: &Engine, name: &str) -> String {
    let Some(matrust::Array::Character(c)) = e.array(name) else {
        panic!("{name} must be text")
    };
    c.to_utf8(&Default::default()).unwrap()
}

/// MATLAB disables diary inside evalc (the reference harness), so cases 158 and
/// 162 cannot show what diary writes; this checks it directly.
#[test]
fn diary_writes_command_window_text() {
    let folder = tempfile::tempdir_in(test_root()).unwrap();
    let mut e = Engine::default();
    e.set_file_directory(folder.path()).unwrap();
    std::fs::write(folder.path().join("old.txt"), "old\n").unwrap();
    e.execute("diary('d.txt'); disp(42); fprintf('abc\\n'); y = 3\ndiary off; disp(5)")
        .unwrap();
    let written = std::fs::read_to_string(folder.path().join("d.txt")).unwrap();
    assert_eq!(written, "    42\n\nabc\n\ny =\n\n     3\n\n");
    // An existing file is appended to; toggling resumes the same file.
    e.execute("diary old.txt\ndisp(7)\ndiary\ndisp(8)\ndiary\ndisp(9)\ndiary off")
        .unwrap();
    let written = std::fs::read_to_string(folder.path().join("old.txt")).unwrap();
    assert_eq!(written, "old\n     7\n\n     9\n\n");
    e.execute("s = get(0, 'Diary'); f = get(0, 'DiaryFile');")
        .unwrap();
    assert_eq!(text(&e, "s"), "off");
    assert!(text(&e, "f").ends_with("old.txt"), "{}", text(&e, "f"));
}

#[test]
fn inline_objects_evaluate_in_their_own_workspace() {
    let mut e = Engine::default();
    e.execute("t = 100; f = inline('t + a', 't', 'a'); y = f(1, 2); z = feval(f, 3, 4); g = vectorize(inline('x^2')); w = g([1 2 3]);")
        .unwrap();
    assert_eq!(e.variables["y"].data(), &[3.]);
    assert_eq!(e.variables["z"].data(), &[7.]);
    assert_eq!(e.variables["w"].data(), &[1., 4., 9.]);
    e.execute("c = class(f); o = isobject(f); m = ismethod(f, 'argnames');")
        .unwrap();
    assert_eq!(text(&e, "c"), "inline");
    let error = e.execute("f(1)").unwrap_err();
    assert!(
        error.contains("Not enough inputs to inline function."),
        "{error}"
    );
}

#[test]
fn clearvars_keeps_globals_and_selects_by_pattern() {
    let mut e = Engine::default();
    e.execute(
        "a1 = 1; a2 = 2; b1 = 3; keep = 4; clearvars a* -except a2; clearvars -regexp ^b\\d$;",
    )
    .unwrap();
    for (name, present) in [("a1", false), ("a2", true), ("b1", false), ("keep", true)] {
        assert_eq!(e.contains_array(name), present, "{name}");
    }
    e.execute("clearvars -except keep").unwrap();
    assert!(e.contains_array("keep") && !e.contains_array("a2"));
}

#[test]
fn dbstack_reports_function_names_and_lines() {
    let folder = tempfile::tempdir_in(test_root()).unwrap();
    std::fs::write(
        folder.path().join("lg3_outer.m"),
        "function s = lg3_outer()\n% comment\ns = lg3_inner();\nend\nfunction s = lg3_inner()\n\ns = dbstack('-completenames');\nend\n",
    )
    .unwrap();
    let mut e = Engine::default();
    e.set_file_directory(folder.path()).unwrap();
    e.execute("s = lg3_outer(); n = {s.name}; l = [s.line]; f = s(1).file; k = numel(dbstack);")
        .unwrap();
    assert_eq!(e.variables["l"].data(), &[7., 3.]);
    assert_eq!(e.variables["k"].data(), &[0.]);
    assert!(text(&e, "f").ends_with("lg3_outer.m"));
    // Inside eval/evalc the frame stays on the line of the eval call (case 179).
    std::fs::write(
        folder.path().join("lg3_evalc.m"),
        "function y = lg3_evalc()\ny = evalc('dbstack');\nend\n",
    )
    .unwrap();
    e.execute("t = lg3_evalc();").unwrap();
    assert_eq!(text(&e, "t"), "> In lg3_evalc (line 2)\n");
}

fn engine_in(folder: &std::path::Path) -> Engine {
    let mut e = Engine::default();
    e.set_file_directory(folder).unwrap();
    e.set_function_directory(folder).unwrap();
    e
}
fn number(e: &Engine, name: &str) -> Vec<f64> {
    e.variables[name].data().to_vec()
}

/// Review 1: `builtin` must not reach a user function of the same name, so an
/// overload can delegate to the builtin without recursing.
#[test]
fn builtin_skips_user_functions_of_that_name() {
    let folder = tempfile::tempdir_in(test_root()).unwrap();
    std::fs::write(
        folder.path().join("sum.m"),
        "function s = sum(varargin)\ns = builtin('sum', varargin{:}) + 100;\nend\n",
    )
    .unwrap();
    std::fs::write(
        folder.path().join("max.m"),
        "function [m, i] = max(v)\n[m, i] = builtin('max', v);\nm = m + 100;\nend\n",
    )
    .unwrap();
    let mut e = engine_in(folder.path());
    e.execute("a = sum([1 2 3]); b = builtin('sum', [1 2 3]); [m, i] = max([3 9 2]); [p, q] = builtin('max', [3 9 2]);")
        .unwrap();
    assert_eq!(number(&e, "a"), [106.]);
    assert_eq!(number(&e, "b"), [6.]);
    assert_eq!((number(&e, "m"), number(&e, "i")), (vec![109.], vec![2.]));
    assert_eq!((number(&e, "p"), number(&e, "q")), (vec![9.], vec![2.]));
    // Callbacks the builtin runs still see the user function.
    e.execute("c = builtin('arrayfun', @(v) sum(v), [1 2]);")
        .unwrap();
    assert_eq!(number(&e, "c"), [101., 102.]);
}

/// Review 2: a diary file that cannot be opened or written is reported, and
/// the diary does not stay on.
#[test]
fn diary_reports_unwritable_files() {
    let folder = tempfile::tempdir_in(test_root()).unwrap();
    let mut e = engine_in(folder.path());
    let error = e
        .execute("diary(fullfile('no_such_dir_l3', 'd.txt'))")
        .unwrap_err();
    assert!(error.contains("Could not open diary file"), "{error}");
    e.execute("s = get(0, 'Diary');").unwrap();
    assert_eq!(text(&e, "s"), "off");
    std::fs::create_dir(folder.path().join("sub")).unwrap();
    e.execute("diary(fullfile('sub', 'd.txt')); disp(1)")
        .unwrap();
    std::fs::remove_dir_all(folder.path().join("sub")).unwrap();
    let error = e.execute("disp(2)").unwrap_err();
    assert!(error.contains("could not be written"), "{error}");
    let after = e.execute("disp(3); s = get(0, 'Diary');").unwrap();
    assert_eq!(after.output, ["     3\n"]);
    assert_eq!(text(&e, "s"), "off");
}

/// Review 3: `clearvars -global` clears the global store, including globals
/// no longer linked here or declared only inside functions.
#[test]
fn clearvars_global_clears_the_global_store() {
    let folder = tempfile::tempdir_in(test_root()).unwrap();
    std::fs::write(
        folder.path().join("lg3_setg.m"),
        "function lg3_setg()\nglobal gf\ngf = 7;\nend\n",
    )
    .unwrap();
    let mut e = engine_in(folder.path());
    e.execute("global g; g = 4; clearvars g; clearvars -global g; global g; a = isempty(g);")
        .unwrap();
    assert_eq!(e.logical_variables["a"].data(), [1]);
    e.execute("keep = 1; lg3_setg(); clearvars -global; global gf; b = isempty(gf);")
        .unwrap();
    assert_eq!(e.logical_variables["b"].data(), [1]);
    assert!(e.contains_array("keep"));
}

/// Rev2 #1: clearing globals must bypass both variable and function shadows.
#[test]
fn rev2_clearvars_bypasses_shadowed_clear() {
    let folder = tempfile::tempdir_in(test_root()).unwrap();
    let mut e = engine_in(folder.path());
    e.execute("clear=1; global g; g=4; clearvars -global g; global g; x=g;")
        .unwrap();
    assert!(number(&e, "x").is_empty());
    assert_eq!(number(&e, "clear"), [1.]);
    e.remove_array("clear");
    std::fs::write(
        folder.path().join("clear.m"),
        "function clear(varargin)\nerror('shadow must not run');\nend\n",
    )
    .unwrap();
    std::fs::write(
        folder.path().join("lg3_clear_global.m"),
        "function lg3_clear_global()\nclearvars -global g;\nend\n",
    )
    .unwrap();
    // A suspended caller's global declaration must also be unlinked.
    e.execute("global g; g=4; lg3_clear_global(); global g; x=g;")
        .unwrap();
    assert!(number(&e, "x").is_empty());
    // Ordinary clearvars unlinks the local declaration but retains its value.
    e.execute("g=5; clearvars g; g=9; global g; x=g;").unwrap();
    assert_eq!(number(&e, "x"), [5.]);
}

/// Rev2 #2: a failed switch turns off the previously active diary too.
#[test]
fn rev2_diary_failed_switch_turns_recording_off() {
    let folder = tempfile::tempdir_in(test_root()).unwrap();
    let mut e = engine_in(folder.path());
    e.execute("diary('old.txt'); fprintf('before');").unwrap();
    e.execute("caught=false; try, diary(fullfile('missing', 'new.txt')); catch, caught=true; end; state=get(0,'Diary'); fprintf('after');")
        .unwrap();
    assert_eq!(e.logical_variables["caught"].data(), [1]);
    assert_eq!(text(&e, "state"), "off");
    assert_eq!(
        std::fs::read_to_string(folder.path().join("old.txt")).unwrap(),
        "before\n"
    );
    e.execute("diary on; fprintf('resumed'); diary off;")
        .unwrap();
    assert_eq!(
        std::fs::read_to_string(folder.path().join("old.txt")).unwrap(),
        "before\nresumed"
    );
}

/// Rev2 #3: enumerate the running document, even before it is saved to disk.
#[test]
fn rev2_localfunctions_in_execute_document() {
    let folder = tempfile::tempdir_in(test_root()).unwrap();
    let path = folder.path().join("lg3_document.m");
    let source = "function lg3_document()\nglobal locals\nlocals=localfunctions;\nend\nfunction y=z_local()\ny=1;\nend\nfunction y=a_local()\ny=2;\nend\n";
    let mut e = engine_in(folder.path());
    e.execute_document(source, &path).unwrap();
    e.execute("global locals; n1=func2str(locals{1}); n2=func2str(locals{2}); s=functions(locals{1}); file=s.file;")
        .unwrap();
    assert_eq!(
        (
            e.array("locals").unwrap().rows(),
            e.array("locals").unwrap().cols()
        ),
        (2, 1)
    );
    assert_eq!(
        (text(&e, "n1"), text(&e, "n2")),
        ("z_local".into(), "a_local".into())
    );
    assert_eq!(std::path::Path::new(&text(&e, "file")), path);
    // The editor text wins over an older on-disk definition.
    std::fs::write(&path, "function lg3_document()\nend\n").unwrap();
    e.execute_document(source, &path).unwrap();
    e.execute("global locals; count=numel(locals);").unwrap();
    assert_eq!(number(&e, "count"), [2.]);
    // A script's top level still has no localfunctions result.
    e.execute_document("x=localfunctions;\nfunction y=sub()\ny=1;\nend\n", &path)
        .unwrap();
    assert_eq!(
        (e.array("x").unwrap().rows(), e.array("x").unwrap().cols()),
        (0, 0)
    );
}

/// Review 4: `feval` of an inline object as a statement.
#[test]
fn feval_statement_on_inline_object() {
    let mut e = Engine::default();
    let run = e.execute("f = inline('t+1'); feval(f, 2)").unwrap();
    assert_eq!(run.output.concat(), "\nans =\n\n     3\n");
    assert_eq!(number(&e, "ans"), [3.]);
}

/// Review 5: `fprintf` text without a newline reaches the diary even when the
/// diary is turned off before the line is finished.
#[test]
fn diary_records_text_without_newline() {
    let folder = tempfile::tempdir_in(test_root()).unwrap();
    let mut e = engine_in(folder.path());
    let run = e
        .execute("diary('d.txt'); fprintf('abc'); diary off;")
        .unwrap();
    assert_eq!(run.output, ["abc"]);
    let written = std::fs::read_to_string(folder.path().join("d.txt")).unwrap();
    assert_eq!(written, "abc");
    // Held text from before `diary on` is not recorded.
    e.execute("fprintf('x'); diary on; fprintf('y\\n'); diary off")
        .unwrap();
    let written = std::fs::read_to_string(folder.path().join("d.txt")).unwrap();
    assert_eq!(written, "abcy\n");
}

/// Review 6: a named handle to a user function gets the column contract, not
/// the expanded arrays.
#[test]
fn bsxfun_user_function_sees_columns() {
    let folder = tempfile::tempdir_in(test_root()).unwrap();
    std::fs::write(
        folder.path().join("lg3_cols.m"),
        "function c = lg3_cols(a, b)\nc = a(:) + b(:);\nend\n",
    )
    .unwrap();
    let mut e = engine_in(folder.path());
    e.execute("x = bsxfun(@lg3_cols, [1 2 3], [10; 20]);")
        .unwrap();
    let x = &e.variables["x"];
    assert_eq!((x.rows(), x.cols()), (2, 3));
    assert_eq!(x.data(), [11., 21., 12., 22., 13., 23.]);
}

/// Review rev3: only elementwise builtins take expanded arrays (the reference gives
/// [10 20;20 40] for @mtimes, checked 2026-10-24), and expansion never calls a
/// user `repmat`.
#[test]
fn rev3_bsxfun_mtimes_and_repmat_overload() {
    let folder = tempfile::tempdir_in(test_root()).unwrap();
    std::fs::write(
        folder.path().join("repmat.m"),
        "function r = repmat(varargin)\nerror('user repmat called');\nend\n",
    )
    .unwrap();
    let mut e = engine_in(folder.path());
    e.execute("x = bsxfun(@mtimes, [1 2], [10; 20]); y = bsxfun(@plus, [1 2], [10; 20]);")
        .unwrap();
    assert_eq!(e.variables["x"].data(), [10., 20., 20., 40.]);
    assert_eq!(e.variables["y"].data(), [11., 21., 12., 22.]);
}

/// Review 7: many repeats of one name stay linear.
#[test]
fn genvarname_many_duplicates() {
    let mut e = Engine::default();
    let start = std::time::Instant::now();
    e.execute("n = genvarname(cellstr(repmat('a', 20000, 1))); first = n{1}; second = n{2}; last = n{end}; m = genvarname({'b', 'b1', 'b', 'b'});")
        .unwrap();
    assert!(start.elapsed().as_secs() < 20, "{:?}", start.elapsed());
    assert_eq!(
        (text(&e, "first"), text(&e, "second"), text(&e, "last")),
        ("a".into(), "a1".into(), "a19999".into())
    );
    e.execute("m2 = m{2}; m3 = m{3}; m4 = m{4};").unwrap();
    assert_eq!(
        (text(&e, "m2"), text(&e, "m3"), text(&e, "m4")),
        ("b1".into(), "b2".into(), "b3".into())
    );
}

/// Review 8 (case 177): a handle to a local function reports its file.
#[test]
fn functions_reports_local_function_file() {
    let folder = tempfile::tempdir_in(test_root()).unwrap();
    std::fs::write(
        folder.path().join("h_lg3_fh.m"),
        "function f = h_lg3_fh()\nf = @lg3sub;\nend\nfunction y = lg3sub(t)\ny = 2*t;\nend\n",
    )
    .unwrap();
    let mut e = engine_in(folder.path());
    e.execute(
        "s = functions(h_lg3_fh()); t = s.type; f = s.file; p = s.parentage; p1 = p{1}; p2 = p{2};",
    )
    .unwrap();
    assert_eq!(text(&e, "t"), "scopedfunction");
    assert!(text(&e, "f").ends_with("h_lg3_fh.m"), "{}", text(&e, "f"));
    assert_eq!(
        (text(&e, "p1"), text(&e, "p2")),
        ("lg3sub".into(), "h_lg3_fh".into())
    );
}

/// `[p, n, e] = fileparts(f)` with the reference's rules.
#[test]
fn fileparts_three_outputs() {
    let mut e = Engine::default();
    e.execute("[p, n, x] = fileparts('/a/b/file.tar.gz'); [p2, n2, x2] = fileparts('noext'); [p3, n3] = fileparts('dir/'); [~, n4, x4] = fileparts('.bashrc');")
        .unwrap();
    assert_eq!(
        (text(&e, "p"), text(&e, "n"), text(&e, "x")),
        ("/a/b".into(), "file.tar".into(), ".gz".into())
    );
    assert_eq!(
        (text(&e, "p2"), text(&e, "n2"), text(&e, "x2")),
        ("".into(), "noext".into(), "".into())
    );
    assert_eq!((text(&e, "p3"), text(&e, "n3")), ("dir".into(), "".into()));
    assert_eq!(
        (text(&e, "n4"), text(&e, "x4")),
        ("".into(), ".bashrc".into())
    );
    let error = e.execute("[a, b, c, d] = fileparts('x.m')").unwrap_err();
    assert!(error.contains("Too many output arguments."), "{error}");
}

/// Rev2 #5: a Windows drive-relative prefix belongs to the folder output.
#[test]
fn rev2_fileparts_drive_relative() {
    let mut e = Engine::default();
    e.execute("[p,n,ext]=fileparts('C:foo.m');").unwrap();
    let expected = if cfg!(windows) {
        ("C:", "foo", ".m")
    } else {
        ("", "C:foo", ".m")
    };
    assert_eq!(
        (text(&e, "p"), text(&e, "n"), text(&e, "ext")),
        (expected.0.into(), expected.1.into(), expected.2.into())
    );
}

#[test]
#[ignore = "manual release benchmark; includes parsing and workspace updates"]
fn bsxfun_benchmark() {
    for workers in [1, 4] {
        let pool = rayon::ThreadPoolBuilder::new()
            .num_threads(workers)
            .build()
            .unwrap();
        pool.install(|| {
            for (size, repetitions) in [(64, 200), (1000, 20), (3000, 5)] {
                let mut engine = Engine::default();
                engine
                    .execute(&format!("a = 1:{size}; b = (1:{size})';"))
                    .unwrap();
                for (label, source) in [
                    ("named", "c = bsxfun(@plus, a, b);"),
                    ("anonymous", "c = bsxfun(@(p, q) p - 2*q, a, b);"),
                ] {
                    engine.execute(source).unwrap();
                    let start = std::time::Instant::now();
                    for _ in 0..repetitions {
                        engine.execute(source).unwrap();
                    }
                    let micros = start.elapsed().as_secs_f64() * 1e6 / repetitions as f64;
                    eprintln!(
                        "workers={workers} {label} size={size}x{size} repetitions={repetitions} mean_us={micros:.1}"
                    );
                }
            }
        });
    }
}
