//! Replay the reference lang3 corpus (tests/fixtures/fn/lang3-expected.txt).
//! The reference ran every case in one workspace, clearing only `x` before each,
//! so the cases replay in order in one engine and one current folder; cases
//! that use tempdir get their own folder inside this worktree.
use matrust::Engine;

const REFERENCE: &str = include_str!("fixtures/fn/lang3-expected.txt");
const CASES: usize = 188;

const SCOPED: &str = "A handle to a local function records its file (functions reports it) but is still called by name, so it cannot run outside that file (function-handle core).";
const MAP: &str =
    "containers.Map is added by another branch; this engine cannot construct one yet.";

/// Cases that cannot pass for a reason outside this task.
const KNOWN: &[(usize, &str)] = &[
    (
        16,
        "Engine-wide concatenation error wording: 'Horizontal concatenation requires matching row counts'.",
    ),
    (54, MAP),
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

/// A function runs a separate private script by file dispatch. Predicted rows
/// at `s = dbstack` (line 1 of the script): the script, then the function at
/// its call site (line 2). No base frame before or after.
#[test]
fn dbstack_script_called_from_function_names_the_caller() {
    let folder = tempfile::tempdir_in(test_root()).unwrap();
    let private = folder.path().join("private");
    std::fs::create_dir(&private).unwrap();
    std::fs::write(
        folder.path().join("lg3_host.m"),
        "function [caller, n1, n2, l1, l2, f1, f2] = lg3_host()\nlg3_leaf_script;\nend\n",
    )
    .unwrap();
    std::fs::write(
        private.join("lg3_leaf_script.m"),
        "s = dbstack;\ncaller = s(2).name;\nn1 = s(1).name;\nn2 = s(2).name;\nl1 = s(1).line;\nl2 = s(2).line;\nf1 = s(1).file;\nf2 = s(2).file;\n",
    )
    .unwrap();
    let mut e = engine_in(folder.path());
    e.execute("top = numel(dbstack); [caller, n1, n2, l1, l2, f1, f2] = lg3_host(); after = numel(dbstack);")
        .unwrap();
    assert_eq!(number(&e, "top"), [0.]);
    assert_eq!(number(&e, "after"), [0.]);
    assert_eq!(text(&e, "caller"), "lg3_host");
    assert_eq!(
        (text(&e, "n1"), text(&e, "n2")),
        ("lg3_leaf_script".into(), "lg3_host".into())
    );
    assert_eq!((number(&e, "l1"), number(&e, "l2")), (vec![1.], vec![2.]));
    assert_eq!(
        (text(&e, "f1"), text(&e, "f2")),
        ("lg3_leaf_script.m".into(), "lg3_host.m".into())
    );
}

/// Outer named script, then the function, then the script it runs.
#[test]
fn dbstack_document_buffer_matches_disk() {
    let folder = tempfile::tempdir_in(env!("CARGO_MANIFEST_DIR")).unwrap();
    let path = folder.path().join("lg3_buffer.m");
    let source = "% first\r\n% second\r\ns = dbstack;\r\n";
    let mut e = engine_in(folder.path());
    for disk in [source, "old = 1;"] {
        std::fs::write(&path, disk).unwrap();
        e.execute_document(source, &path).unwrap();
        e.execute("n = s(1).name; f = s(1).file; l = s(1).line; count = numel(s);")
            .unwrap();
        assert_eq!(std::fs::read(&path).unwrap(), disk.as_bytes());
        assert_eq!(text(&e, "n"), "lg3_buffer");
        assert_eq!(text(&e, "f"), "lg3_buffer.m");
        assert_eq!(number(&e, "count"), [1.]);
        assert_eq!(number(&e, "l"), [3.]);
    }
    std::fs::write(
        folder.path().join("lg3_buffer_host.m"),
        "function s = lg3_buffer_host()\nlg3_buffer_inner;\nend\n",
    )
    .unwrap();
    std::fs::write(folder.path().join("lg3_buffer_inner.m"), "s = dbstack;\n").unwrap();
    let nested = "% first\r\n% second\r\ns = lg3_buffer_host();\r\n";
    e.execute_document(nested, &path).unwrap();
    e.execute("n1=s(1).name; n2=s(2).name; n3=s(3).name; l1=s(1).line; l2=s(2).line; l3=s(3).line; f3=s(3).file;").unwrap();
    assert_eq!(
        (text(&e, "n1"), text(&e, "n2"), text(&e, "n3")),
        (
            "lg3_buffer_inner".into(),
            "lg3_buffer_host".into(),
            "lg3_buffer".into()
        )
    );
    assert_eq!(
        (number(&e, "l1"), number(&e, "l2"), number(&e, "l3")),
        (vec![1.], vec![2.], vec![3.])
    );
    assert_eq!(text(&e, "f3"), "lg3_buffer.m");
    assert_eq!(std::fs::read(&path).unwrap(), b"old = 1;");
    assert!(
        e.execute_document("% first\n% second\nerror('buffer failure');\n", &path)
            .is_err()
    );
    e.execute("empty = numel(dbstack);").unwrap();
    assert_eq!(number(&e, "empty"), [0.]);
    e.execute_document(source, &path).unwrap();
    e.execute("l = s(1).line;").unwrap();
    assert_eq!(number(&e, "l"), [3.]);
    assert_eq!(std::fs::read(&path).unwrap(), b"old = 1;");
}

/// Predicted at `s = dbstack` (script line 1): lg3_in line 1, lg3_run line 2,
/// lg3_drive line 1. `caller` is the function, not the outer script.
/// `dbstack(1)` drops the script. `-completenames` keeps the same names and
/// caller lines with full paths. Printing uses those frames and stays on the
/// evalc line (no eval frame). The second output stays 1.
#[test]
fn dbstack_outer_script_function_inner_script() {
    let folder = tempfile::tempdir_in(test_root()).unwrap();
    let dir = folder.path().canonicalize().unwrap();
    std::fs::write(
        dir.join("lg3_run.m"),
        "function [caller, s, c, o, printed, k] = lg3_run()\nlg3_in;\nend\n",
    )
    .unwrap();
    std::fs::write(
        dir.join("lg3_in.m"),
        "s = dbstack;\nc = dbstack('-completenames');\no = dbstack(1);\ncaller = s(2).name;\nprinted = evalc('dbstack');\n[st, k] = dbstack;\n",
    )
    .unwrap();
    let drive = "\
[caller, s, c, o, printed, k] = lg3_run();
";
    std::fs::write(dir.join("lg3_drive.m"), drive).unwrap();
    let mut e = engine_in(&dir);
    e.execute_document(drive, &dir.join("lg3_drive.m")).unwrap();
    e.execute(
        "n1=s(1).name; n2=s(2).name; n3=s(3).name; \
         l1=s(1).line; l2=s(2).line; l3=s(3).line; \
         f1=s(1).file; f2=s(2).file; f3=s(3).file; \
         cn1=c(1).name; cn2=c(2).name; cn3=c(3).name; \
         cl1=c(1).line; cl2=c(2).line; cl3=c(3).line; \
         cf1=c(1).file; cf2=c(2).file; cf3=c(3).file; \
         on1=o(1).name; on2=o(2).name; ol1=o(1).line; ol2=o(2).line; \
         of1=o(1).file; of2=o(2).file; ns=numel(s); no=numel(o); base=numel(dbstack);",
    )
    .unwrap();
    assert_eq!(number(&e, "base"), [0.]);
    assert_eq!((number(&e, "ns"), number(&e, "no")), (vec![3.], vec![2.]));
    assert_eq!(text(&e, "caller"), "lg3_run");
    assert_eq!(number(&e, "k"), [1.]);
    assert_eq!(
        (text(&e, "n1"), text(&e, "n2"), text(&e, "n3")),
        ("lg3_in".into(), "lg3_run".into(), "lg3_drive".into())
    );
    assert_eq!(
        (number(&e, "l1"), number(&e, "l2"), number(&e, "l3")),
        (vec![1.], vec![2.], vec![1.])
    );
    assert_eq!(
        (text(&e, "f1"), text(&e, "f2"), text(&e, "f3")),
        ("lg3_in.m".into(), "lg3_run.m".into(), "lg3_drive.m".into())
    );
    // completenames call is script line 2; caller lines stay on the call sites.
    assert_eq!(
        (text(&e, "cn1"), text(&e, "cn2"), text(&e, "cn3")),
        ("lg3_in".into(), "lg3_run".into(), "lg3_drive".into())
    );
    assert_eq!(
        (number(&e, "cl1"), number(&e, "cl2"), number(&e, "cl3")),
        (vec![2.], vec![2.], vec![1.])
    );
    assert_eq!(
        (text(&e, "cf1"), text(&e, "cf2"), text(&e, "cf3")),
        (
            dir.join("lg3_in.m").display().to_string(),
            dir.join("lg3_run.m").display().to_string(),
            dir.join("lg3_drive.m").display().to_string(),
        )
    );
    assert_eq!(
        (text(&e, "on1"), text(&e, "on2")),
        ("lg3_run".into(), "lg3_drive".into())
    );
    assert_eq!((number(&e, "ol1"), number(&e, "ol2")), (vec![2.], vec![1.]));
    assert_eq!(
        (text(&e, "of1"), text(&e, "of2")),
        ("lg3_run.m".into(), "lg3_drive.m".into())
    );
    // evalc('dbstack') is script line 5. No eval frame, no base caller.
    assert_eq!(
        text(&e, "printed"),
        "> In lg3_in (line 5)\n  In lg3_run (line 2)\n  In lg3_drive (line 1)\n"
    );
}

/// Named script calls a function that calls an inner function in the same file.
/// File lines: mid calls leaf at line 2; leaf's `dbstack('-completenames')` is
/// line 5, plain `dbstack` is line 6, and `evalc('dbstack')` is line 8.
/// Predicted stack: lg3_leaf line 5, lg3_mid line 2, lg3_top line 1.
#[test]
fn dbstack_script_function_inner_function() {
    let folder = tempfile::tempdir_in(test_root()).unwrap();
    let dir = folder.path().canonicalize().unwrap();
    std::fs::write(
        dir.join("lg3_mid.m"),
        "function [stack, plain, omit, printed, k] = lg3_mid()\n\
         [stack, plain, omit, printed, k] = lg3_leaf();\n\
         end\n\
         function [stack, plain, omit, printed, k] = lg3_leaf()\n\
         stack = dbstack('-completenames');\n\
         plain = dbstack;\n\
         omit = dbstack('-completenames', 1);\n\
         printed = evalc('dbstack');\n\
         [st, k] = dbstack;\n\
         end\n",
    )
    .unwrap();
    let top = "\
[lg3_stack, lg3_plain, lg3_omit, lg3_print, lg3_k] = lg3_mid();
";
    std::fs::write(dir.join("lg3_top.m"), top).unwrap();
    let mut e = engine_in(&dir);
    e.execute_document(top, &dir.join("lg3_top.m")).unwrap();
    e.execute(
        "n1=lg3_stack(1).name; n2=lg3_stack(2).name; n3=lg3_stack(3).name; \
         l1=lg3_stack(1).line; l2=lg3_stack(2).line; l3=lg3_stack(3).line; \
         f1=lg3_stack(1).file; f2=lg3_stack(2).file; f3=lg3_stack(3).file; \
         p1=lg3_plain(1).name; p3=lg3_plain(3).name; pl1=lg3_plain(1).line; pl2=lg3_plain(2).line; pl3=lg3_plain(3).line; \
         pf1=lg3_plain(1).file; pf3=lg3_plain(3).file; \
         o1=lg3_omit(1).name; o2=lg3_omit(2).name; ol1=lg3_omit(1).line; ol2=lg3_omit(2).line; \
         of1=lg3_omit(1).file; of2=lg3_omit(2).file; \
         ns=numel(lg3_stack); no=numel(lg3_omit); base=numel(dbstack);",
    )
    .unwrap();
    let mid = dir.join("lg3_mid.m").display().to_string();
    let top_path = dir.join("lg3_top.m").display().to_string();
    assert_eq!(number(&e, "base"), [0.]);
    assert_eq!(number(&e, "lg3_k"), [1.]);
    assert_eq!((number(&e, "ns"), number(&e, "no")), (vec![3.], vec![2.]));
    assert_eq!(
        (text(&e, "n1"), text(&e, "n2"), text(&e, "n3")),
        ("lg3_leaf".into(), "lg3_mid".into(), "lg3_top".into())
    );
    assert_eq!(
        (number(&e, "l1"), number(&e, "l2"), number(&e, "l3")),
        (vec![5.], vec![2.], vec![1.])
    );
    assert_eq!(
        (text(&e, "f1"), text(&e, "f2"), text(&e, "f3")),
        (mid.clone(), mid.clone(), top_path.clone())
    );
    assert_eq!(
        (text(&e, "p1"), text(&e, "p3")),
        ("lg3_leaf".into(), "lg3_top".into())
    );
    assert_eq!(
        (number(&e, "pl1"), number(&e, "pl2"), number(&e, "pl3")),
        (vec![6.], vec![2.], vec![1.])
    );
    assert_eq!(
        (text(&e, "pf1"), text(&e, "pf3")),
        ("lg3_mid.m".into(), "lg3_top.m".into())
    );
    assert_eq!(
        (text(&e, "o1"), text(&e, "o2")),
        ("lg3_mid".into(), "lg3_top".into())
    );
    assert_eq!((number(&e, "ol1"), number(&e, "ol2")), (vec![2.], vec![1.]));
    assert_eq!((text(&e, "of1"), text(&e, "of2")), (mid, top_path));
    assert_eq!(
        text(&e, "lg3_print"),
        "> In lg3_leaf (line 8)\n  In lg3_mid (line 2)\n  In lg3_top (line 1)\n"
    );
}

/// Nested runs of one script are two frames. A later run of that file is one
/// frame, and the base stack stays empty.
#[test]
fn dbstack_nested_script_is_not_the_same_file_twice() {
    let folder = tempfile::tempdir_in(test_root()).unwrap();
    let dir = folder.path().canonicalize().unwrap();
    let nested = "\
if exist('lg3_flag','var')
nest = dbstack;
else
lg3_flag = 1;
lg3_self;
end
";
    std::fs::write(dir.join("lg3_self.m"), nested).unwrap();
    std::fs::write(dir.join("lg3_once.m"), "seq = dbstack;\n").unwrap();
    let mut e = engine_in(&dir);
    e.execute_document(nested, &dir.join("lg3_self.m")).unwrap();
    e.execute(
        "n=numel(nest); a=nest(1).name; b=nest(2).name; la=nest(1).line; lb=nest(2).line; \
         fa=nest(1).file; fb=nest(2).file; top=numel(dbstack);",
    )
    .unwrap();
    assert_eq!(number(&e, "n"), [2.]);
    assert_eq!(
        (text(&e, "a"), text(&e, "b")),
        ("lg3_self".into(), "lg3_self".into())
    );
    assert_eq!((number(&e, "la"), number(&e, "lb")), (vec![2.], vec![5.]));
    assert_eq!(
        (text(&e, "fa"), text(&e, "fb")),
        ("lg3_self.m".into(), "lg3_self.m".into())
    );
    assert_eq!(number(&e, "top"), [0.]);
    // The flag is set, so this run does not nest. One frame, at the dbstack line.
    e.execute_document(nested, &dir.join("lg3_self.m")).unwrap();
    e.execute("n2=numel(nest); l2=nest(1).line; name2=nest(1).name; top2=numel(dbstack);")
        .unwrap();
    assert_eq!(number(&e, "n2"), [1.]);
    assert_eq!(number(&e, "l2"), [2.]);
    assert_eq!(text(&e, "name2"), "lg3_self");
    assert_eq!(number(&e, "top2"), [0.]);
    e.execute("lg3_once; n3=numel(seq); l3=seq(1).line; f3=seq(1).file; lg3_once; n4=numel(seq); top3=numel(dbstack);")
        .unwrap();
    assert_eq!((number(&e, "n3"), number(&e, "n4")), (vec![1.], vec![1.]));
    assert_eq!(number(&e, "l3"), [1.]);
    assert_eq!(text(&e, "f3"), "lg3_once.m");
    assert_eq!(number(&e, "top3"), [0.]);
}

/// CRLF is one line break. Predicted: script dbstack at line 1, call site line 2.
#[test]
fn dbstack_script_lines_keep_crlf() {
    let folder = tempfile::tempdir_in(test_root()).unwrap();
    let dir = folder.path().canonicalize().unwrap();
    std::fs::write(
        dir.join("lg3_cr_host.m"),
        b"function [caller, l1, l2] = lg3_cr_host()\r\nlg3_cr_script;\r\nend\r\n",
    )
    .unwrap();
    std::fs::write(
        dir.join("lg3_cr_script.m"),
        b"s = dbstack;\r\ncaller = s(2).name;\r\nl1 = s(1).line;\r\nl2 = s(2).line;\r\n",
    )
    .unwrap();
    let mut e = engine_in(&dir);
    e.execute("[caller, l1, l2] = lg3_cr_host();").unwrap();
    assert_eq!(text(&e, "caller"), "lg3_cr_host");
    assert_eq!((number(&e, "l1"), number(&e, "l2")), (vec![1.], vec![2.]));
}

/// A script error drops its frame and restores the caller's byte. The next
/// statement in the caller reports its own line. An uncaught error leaves
/// the base stack empty.
#[test]
fn dbstack_script_frame_ends_on_error() {
    let folder = tempfile::tempdir_in(test_root()).unwrap();
    let dir = folder.path().canonicalize().unwrap();
    std::fs::write(dir.join("lg3_bad.m"), "error('lg3 boom');\n").unwrap();
    std::fs::write(
        dir.join("lg3_try.m"),
        "function [name, line, n] = lg3_try()\ntry\nlg3_bad;\ncatch\nend\ns = dbstack;\nname = s.name;\nline = s.line;\nn = numel(s);\nend\n",
    )
    .unwrap();
    std::fs::write(
        dir.join("lg3_throws.m"),
        "function lg3_throws()\nlg3_bad;\nend\n",
    )
    .unwrap();
    std::fs::write(
        dir.join("lg3_after.m"),
        "function line = lg3_after()\ns = dbstack;\nline = s.line;\nend\n",
    )
    .unwrap();
    let mut e = engine_in(&dir);
    e.execute("[name, line, n] = lg3_try();").unwrap();
    assert_eq!(text(&e, "name"), "lg3_try");
    assert_eq!(number(&e, "line"), [6.]);
    assert_eq!(number(&e, "n"), [1.]);
    let err = e.execute("lg3_throws").unwrap_err();
    assert!(err.contains("lg3_bad") && err.contains("lg3 boom"), "{err}");
    e.execute("left = numel(dbstack); again = lg3_after();")
        .unwrap();
    assert_eq!(number(&e, "left"), [0.]);
    assert_eq!(number(&e, "again"), [2.]);
}

/// Anonymous frames stay hidden. Interactive `dbstack`, `eval('dbstack')`,
/// and a returned script leave an empty stack. Bad arguments still error.
#[test]
fn dbstack_anonymous_and_interactive_stay_empty() {
    let folder = tempfile::tempdir_in(test_root()).unwrap();
    let dir = folder.path().canonicalize().unwrap();
    std::fs::write(
        dir.join("lg3_anon.m"),
        "function [name, line, n] = lg3_anon()\nh = @() dbstack;\ns = h();\nname = s.name;\nline = s.line;\nn = numel(s);\nend\n",
    )
    .unwrap();
    std::fs::write(
        dir.join("lg3_ev.m"),
        "function [name, line, n] = lg3_ev()\ns = eval('dbstack');\nname = s.name;\nline = s.line;\nn = numel(s);\nend\n",
    )
    .unwrap();
    let mut e = engine_in(&dir);
    e.execute("s = dbstack; direct = numel(s);").unwrap();
    assert_eq!(number(&e, "direct"), [0.]);
    e.execute("f = @() dbstack; s = f(); anon = numel(s); ev = eval('dbstack'); evn = numel(ev);")
        .unwrap();
    assert_eq!(number(&e, "anon"), [0.]);
    assert_eq!(number(&e, "evn"), [0.]);
    e.execute("[name, line, n] = lg3_anon();").unwrap();
    assert_eq!(text(&e, "name"), "lg3_anon");
    assert_eq!(number(&e, "line"), [3.]);
    assert_eq!(number(&e, "n"), [1.]);
    e.execute("[name, line, n] = lg3_ev(); empty = numel(dbstack);")
        .unwrap();
    assert_eq!(text(&e, "name"), "lg3_ev");
    assert_eq!(number(&e, "line"), [2.]);
    assert_eq!(number(&e, "n"), [1.]);
    assert_eq!(number(&e, "empty"), [0.]);
    let bad = e.execute("dbstack('zz')").unwrap_err();
    assert!(bad.contains("only legal arguments to dbstack"), "{bad}");
    let neg = e.execute("dbstack(-1)").unwrap_err();
    assert!(neg.contains("only legal arguments to dbstack"), "{neg}");
    let many = e.execute("[a, b, c] = dbstack").unwrap_err();
    assert!(many.contains("Too many output arguments."), "{many}");
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

#[test]
fn fix14_warning_state_save_query_restore() {
    let mut e = Engine::default();
    // Script pattern: save state, silence, emit (suppressed), restore, emit (printed).
    e.execute("lastwarn('initial', 'init:id');").unwrap();
    let out = e
        .execute("s = warning('off', 'a:b'); [m0, id0] = lastwarn;")
        .unwrap();
    assert!(out.output.is_empty());
    assert_eq!(text(&e, "m0"), "initial");
    assert_eq!(text(&e, "id0"), "init:id");

    let s = e.array("s").unwrap();
    let matrust::Array::Struct(sa) = s else {
        panic!("expected struct");
    };
    assert_eq!(sa.len(), 1);
    let get_field = |st: &matrust::StructArray, idx: usize, name: &str| -> String {
        let fi = st.field_index(name).unwrap();
        match st.get(fi, idx).unwrap().as_ref() {
            matrust::Array::Character(c) => c
                .to_utf8(&std::sync::atomic::AtomicBool::new(false))
                .unwrap(),
            other => panic!("unexpected field type: {other:?}"),
        }
    };
    assert_eq!(get_field(&sa, 0, "identifier"), "a:b");
    assert_eq!(get_field(&sa, 0, "state"), "on");

    // Suppressed warning prints nothing, still updates lastwarn.
    let out = e
        .execute("warning('a:b', 'x'); [m1, id1] = lastwarn;")
        .unwrap();
    assert!(out.output.is_empty());
    assert_eq!(text(&e, "m1"), "x");
    assert_eq!(text(&e, "id1"), "a:b");

    // Query specific id while off.
    e.execute("q = warning('query', 'a:b');").unwrap();
    let matrust::Array::Struct(qa) = e.array("q").unwrap() else {
        panic!("expected query struct");
    };
    assert_eq!(get_field(&qa, 0, "identifier"), "a:b");
    assert_eq!(get_field(&qa, 0, "state"), "off");

    // Query without output prints state.
    let q_out = e.execute("warning('query', 'a:b')").unwrap();
    assert_eq!(q_out.output.join("\n"), "Warning 'a:b' is 'off'.");

    // w = warning returns all states including 'all'.
    e.execute("w = warning;").unwrap();
    let matrust::Array::Struct(wa) = e.array("w").unwrap() else {
        panic!("expected struct array");
    };
    assert_eq!(wa.len(), 2);
    assert_eq!(get_field(&wa, 0, "identifier"), "all");
    assert_eq!(get_field(&wa, 0, "state"), "on");
    assert_eq!(get_field(&wa, 1, "identifier"), "a:b");
    assert_eq!(get_field(&wa, 1, "state"), "off");

    // Restore struct s: warning('a:b', 'x') prints again, and lastwarn unchanged by warning(s).
    e.execute("lastwarn('keep', 'keep:id'); warning(s); [m2, id2] = lastwarn;")
        .unwrap();
    assert_eq!(text(&e, "m2"), "keep");
    assert_eq!(text(&e, "id2"), "keep:id");
    let out = e.execute("warning('a:b', 'x');").unwrap();
    assert_eq!(out.output, vec!["Warning: x"]);

    // warning('off', 'all') / warning('on', 'all') save & restore.
    let out = e
        .execute("s_all = warning('off', 'all'); warning('b:c', 'hidden');")
        .unwrap();
    assert!(out.output.is_empty());
    e.execute("warning(s_all);").unwrap();
    let out = e.execute("warning('b:c', 'shown');").unwrap();
    assert_eq!(out.output, vec!["Warning: shown"]);

    // warning('on', id) returns previous state.
    e.execute("s_off = warning('off', 'c:d'); s_off2 = warning('off', 'c:d'); s_on = warning('on', 'c:d');").unwrap();
    let matrust::Array::Struct(s1) = e.array("s_off").unwrap() else {
        panic!("expected struct");
    };
    let matrust::Array::Struct(s2) = e.array("s_off2").unwrap() else {
        panic!("expected struct");
    };
    let matrust::Array::Struct(s3) = e.array("s_on").unwrap() else {
        panic!("expected struct");
    };
    assert_eq!(get_field(&s1, 0, "state"), "on");
    assert_eq!(get_field(&s2, 0, "state"), "off");
    assert_eq!(get_field(&s3, 0, "state"), "off");
}

#[test]
fn fix14_warning_formats_typed_arguments_and_continues() {
    for (arg, expected) in [
        ("int16(3)", "Warning: value=3"),
        ("single(4)", "Warning: value=4"),
    ] {
        let mut e = Engine::default();
        let out = e
            .execute(&format!("warning('value=%d', {arg}); after = 42;"))
            .unwrap();
        assert_eq!(out.output, vec![expected]);
        assert_eq!(number(&e, "after"), vec![42.]);
    }
}

#[test]
fn fix14_warning_statement_runs_folder_script_first() {
    let folder = tempfile::tempdir_in(env!("CARGO_MANIFEST_DIR")).unwrap();
    std::fs::write(folder.path().join("warning.m"), "ran_warning_script = 1;").unwrap();
    let mut e = engine_in(folder.path());
    for source in ["warning", "clear ran_warning_script; warning()"] {
        let out = e.execute(source).unwrap();
        assert!(out.output.is_empty());
        assert_eq!(number(&e, "ran_warning_script"), vec![1.]);
    }
}

#[test]
fn fix14_help_warning_and_whos_describe_supported_forms() {
    let mut e = Engine::default();
    let warning = e.execute("help warning").unwrap().output.join(" ");
    assert!(warning.contains("warning('query', id)"), "{warning}");
    assert!(warning.contains("warning(s)"), "{warning}");
    assert!(
        !warning.contains("querying the state (warning('query', ...), s = warning)"),
        "{warning}"
    );

    let whos = e.execute("help whos").unwrap().output.join(" ");
    assert!(whos.contains("wildcard patterns"), "{whos}");
    assert!(
        !whos.contains("No workspace name filters, wildcard filters"),
        "{whos}"
    );
}

#[test]
fn fix14_warning_scalar_string_actions() {
    let mut e = Engine::default();
    let out = e
        .execute(r#"id = "a:b"; s = warning("off", "a:b"); q = warning("query", id);"#)
        .unwrap();
    assert!(out.output.is_empty());
    e.execute(
        "assert(strcmp(s.identifier, 'a:b')); assert(strcmp(s.state, 'on')); \
         assert(strcmp(q.identifier, 'a:b')); assert(strcmp(q.state, 'off'));",
    )
    .unwrap();
    let out = e.execute(r#"warning("query", "a:b")"#).unwrap();
    assert_eq!(out.output, vec!["Warning 'a:b' is 'off'."]);
}

#[test]
fn fix14_warning_restore_inherited_state() {
    let mut e = Engine::default();
    let out = e
        .execute(
            "w0 = warning; s = warning('off', 'a:b'); warning(s); \
             assert(isequal(warning, w0)); warning('off', 'all'); warning('a:b', 'x');",
        )
        .unwrap();
    assert!(out.output.is_empty());

    e.execute(
        "s = warning('on', 'a:b'); warning(s); q = warning('query', 'a:b'); \
         assert(strcmp(q.state, 'off')); assert(numel(warning) == 1); \
         warning('on', 'a:b'); w1 = warning; s = warning('off', 'a:b'); warning(s); \
         assert(isequal(warning, w1));",
    )
    .unwrap();

    let out = e
        .execute(
            "warning(w0); for k = 1:1100, id = sprintf('test:id%d', k); \
             s = warning('off', id); warning(s); end; assert(isequal(warning, w0));",
        )
        .unwrap();
    assert!(out.output.is_empty());
}

#[test]
fn fix14_whos_arguments_and_wildcards() {
    let mut e = Engine::default();
    e.execute("alpha = [1 2; 3 4]; beta = 'hi'; apple = 1+2i;")
        .unwrap();

    let bare = e.execute("whos").unwrap().output.join("\n");
    assert!(bare.contains("alpha"));
    assert!(bare.contains("beta"));
    assert!(bare.contains("apple"));

    let cmd = e.execute("whos alpha beta").unwrap().output.join("\n");
    assert!(cmd.contains("alpha"));
    assert!(cmd.contains("beta"));
    assert!(!cmd.contains("apple"));

    let func = e
        .execute("whos('alpha', 'beta')")
        .unwrap()
        .output
        .join("\n");
    assert_eq!(func, cmd);
    let strings = e
        .execute(r#"whos("alpha", "beta")"#)
        .unwrap()
        .output
        .join("\n");
    assert_eq!(strings, cmd);

    let wild = e.execute("whos a*").unwrap().output.join("\n");
    assert!(wild.contains("alpha"));
    assert!(wild.contains("apple"));
    assert!(!wild.contains("beta"));

    let wild_fn = e.execute("whos('a*')").unwrap().output.join("\n");
    assert_eq!(wild_fn, wild);

    let none = e.execute("whos nonexistent_var").unwrap();
    assert!(none.output.is_empty());

    // -file keeps working.
    let dir = std::env::temp_dir();
    let path = dir.join(format!("matrust_fix14_whos_{}.mat", std::process::id()));
    let path_str = path.to_string_lossy().replace('\'', "''");
    e.execute(&format!("save('{path_str}', 'alpha', 'beta')"))
        .unwrap();
    let file_out = e
        .execute(&format!("whos('-file', '{path_str}', 'alpha')"))
        .unwrap()
        .output
        .join("\n");
    let all_file = e
        .execute(&format!("whos('-file', '{path_str}')"))
        .unwrap()
        .output;
    let string_file = e.execute(&format!(
        r#"filename = string('{path_str}'); whos("-file", filename)"#
    ));
    let _ = std::fs::remove_file(&path);
    assert!(file_out.contains("alpha"));
    assert!(!file_out.contains("beta"));
    assert_eq!(string_file.unwrap().output, all_file);
}

#[test]
fn fix14_vecnorm_typed_nd_and_result_class() {
    let mut e = Engine::default();
    e.execute(
        "a = reshape(single(1:8), 2, 2, 2); \
         v3 = vecnorm(a, 1, 3); assert(isequal(v3, single([6 10; 8 12]))); \
         v2 = vecnorm(a, single(1), int16(2)); assert(isequal(size(v2), [2 1 2])); \
         assert(isequal(v2, reshape(single([4 6 12 14]), 2, 1, 2))); \
         v_int = vecnorm(reshape(int16(1:8), 2, 2, 2), 1, 3); \
         assert(strcmp(class(v_int), 'double')); assert(isequal(v_int, [6 10; 8 12])); \
         v = vecnorm(single([3 4])); assert(strcmp(class(v), 'single')); \
         assert(isequal(v, single(5))); \
         assert(isequal(vecnorm(single([-3 4]), 2, 3), single([3 4])));",
    )
    .unwrap();
}

#[test]
fn fix14_vecnorm_result_memory_limit() {
    let mut e = Engine::with_limits(matrust::engine::Limits {
        evaluation_units: None,
        ..Default::default()
    });
    let err = e
        .execute("v = vecnorm(zeros(10000, 0, 10000), 2, 2);")
        .unwrap_err();
    assert!(
        err.to_string()
            .contains("Statement exceeds the 256 MiB memory limit"),
        "{err}"
    );
    assert!(e.array("v").is_none());
    e.execute("v = vecnorm(zeros(2, 0, 3), 2, 2); assert(isequal(v, zeros(2, 1, 3)));")
        .unwrap();
}

#[test]
fn fix14_vecnorm_cases() {
    let mut e = Engine::default();
    e.execute(
        "a = [3 4; 6 8]; \
         v_row = vecnorm(a, 2, 2); \
         v_def = vecnorm(a); \
         v_rvec = vecnorm([3 -4]); \
         v_cvec = vecnorm([3; -4]); \
         b = [1 -2 3; -4 5 -6]; \
         v_1_1 = vecnorm(b, 1, 1); \
         v_1_2 = vecnorm(b, 1, 2); \
         v_inf_1 = vecnorm(b, Inf, 1); \
         v_inf_2 = vecnorm(b, Inf, 2); \
         v_3_1 = vecnorm([1 2; 3 4], 3, 1); \
         v_cplx = vecnorm([3+4i, 0; 0, 5-12i]); \
         v_cplx2 = vecnorm([3+4i, 6+8i], 2, 2); \
         v_3d = vecnorm(reshape(1:8, 2, 2, 2), 2, 3); \
         v_emp_p = vecnorm([3 4], [], 2); \
         v_over = vecnorm([3 -4], 2, 3);",
    )
    .unwrap();

    let mat = |eng: &Engine, name: &str| -> (Vec<usize>, Vec<f64>) {
        match eng.array(name).unwrap() {
            matrust::Array::Numeric(m) => (m.dims().into_owned(), m.data().to_vec()),
            other => panic!("{name}: expected numeric, got {other:?}"),
        }
    };
    let close = |actual: &[f64], expected: &[f64]| {
        assert_eq!(actual.len(), expected.len());
        for (a, b) in actual.iter().zip(expected) {
            assert!((a - b).abs() <= 1e-12 * b.abs().max(1.0), "{a} vs {b}");
        }
    };

    assert_eq!(mat(&e, "v_row"), (vec![2, 1], vec![5.0, 10.0]));
    let (d, v) = mat(&e, "v_def");
    assert_eq!(d, vec![1, 2]);
    close(&v, &[45.0_f64.sqrt(), 80.0_f64.sqrt()]);
    assert_eq!(mat(&e, "v_rvec"), (vec![1, 1], vec![5.0]));
    assert_eq!(mat(&e, "v_cvec"), (vec![1, 1], vec![5.0]));
    assert_eq!(mat(&e, "v_1_1"), (vec![1, 3], vec![5.0, 7.0, 9.0]));
    assert_eq!(mat(&e, "v_1_2"), (vec![2, 1], vec![6.0, 15.0]));
    assert_eq!(mat(&e, "v_inf_1"), (vec![1, 3], vec![4.0, 5.0, 6.0]));
    assert_eq!(mat(&e, "v_inf_2"), (vec![2, 1], vec![3.0, 6.0]));
    let (d3, v3) = mat(&e, "v_3_1");
    assert_eq!(d3, vec![1, 2]);
    close(&v3, &[28.0_f64.cbrt(), 72.0_f64.cbrt()]);
    assert_eq!(mat(&e, "v_cplx"), (vec![1, 2], vec![5.0, 13.0]));
    let (dc2, vc2) = mat(&e, "v_cplx2");
    assert_eq!(dc2, vec![1, 1]);
    close(&vc2, &[125.0_f64.sqrt()]);
    let (d3d, v3d) = mat(&e, "v_3d");
    assert_eq!(d3d, vec![2, 2]);
    close(
        &v3d,
        &[
            1.0_f64.hypot(5.0),
            2.0_f64.hypot(6.0),
            3.0_f64.hypot(7.0),
            4.0_f64.hypot(8.0),
        ],
    );
    assert_eq!(mat(&e, "v_emp_p"), (vec![1, 1], vec![5.0]));
    assert_eq!(mat(&e, "v_over"), (vec![1, 2], vec![3.0, 4.0]));

    for bad in [
        "vecnorm([1 2], 0)",
        "vecnorm([1 2], -1)",
        "vecnorm([1 2], 2, 0)",
        "vecnorm([1 2], 2, 1.5)",
    ] {
        assert!(e.execute(bad).is_err(), "expected error for {bad}");
    }
}

#[test]
fn fix14e_warning_sparse_reordered_roundtrip_handle_and_vecnorm() {
    // Item 2: warning('value=%d', sparse(3))
    let mut e = Engine::default();
    let out = e.execute("warning('value=%d', sparse(3));").unwrap();
    assert_eq!(out.output, vec!["Warning: value=3"]);

    // Item 12: warning(s) with reordered snapshot applies 'all' first, then per-ID
    let mut e = Engine::default();
    let out = e
        .execute("warning('off','x:y'); s=warning; warning(s([2 1])); warning('x:y','unexpected');")
        .unwrap();
    assert!(out.output.is_empty());

    // Item 13: snapshot round-trip when id matches inherited 'all' state
    let mut e = Engine::default();
    e.execute("warning('on','a:b'); s=warning; warning(s); assert(isequal(s,warning));")
        .unwrap();

    // Item 14: zero-output request survives function handle call
    let mut e = Engine::default();
    let out = e.execute("f=@warning; f('query','a:b');").unwrap();
    assert_eq!(out.output, vec!["Warning 'a:b' is 'on'."]);

    // Items 15 & 16: vecnorm complex Inf+NaNi -> NaN, and small p with realmin does not overflow
    let mut e = Engine::default();
    e.execute("v1 = vecnorm(complex(Inf, NaN)); v2 = vecnorm([realmin realmin], 1/1024);")
        .unwrap();
    let v1 = match e.array("v1").unwrap() {
        matrust::Array::Numeric(m) => m.data()[0],
        other => panic!("v1: expected numeric, got {other:?}"),
    };
    assert!(v1.is_nan(), "expected NaN, got {v1}");
    let v2 = match e.array("v2").unwrap() {
        matrust::Array::Numeric(m) => m.data()[0],
        other => panic!("v2: expected numeric, got {other:?}"),
    };
    assert!((v2 - 4.0).abs() <= 1e-11, "expected ~4, got {v2}");
}
