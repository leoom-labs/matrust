//! cell2mat, cellstr, deal, structfun, strtok, assert, warning, eval, rat and
//! rats against expected values. `tests/fixtures/fn/lang-expected.txt` holds every
//! case of `tests/fixtures/fn/lang-cases.txt` in this format:
//! `@@CASE <id>|<statement>`, the `evalc` console text, `@@CLASS <class> <size>`
//! of `x` (absent on error), `@@END`; an error case holds `ERR <message>`.
//!
//! Each case runs in a fresh engine. The console text must match byte for byte,
//! then the class and size of `x`; for an error, the message without MatRust's
//! `Line 1, column N: ` location.
//!
//! The reference's desktop wraps a warning in `[\x08...]\x08` markup and adds `> In ...`
//! stack lines (cases 37-39); MatRust prints the one line `Warning: <message>`,
//! so the markup and stack lines are removed from the expected text.
use matrust::Engine;

const CORPUS: &str = include_str!("fixtures/fn/lang-expected.txt");

/// Cases that cannot pass for a reason outside this function family, with
/// the reason. Every other case must match.
const KNOWN: &[(&str, &str)] = &[];

fn console(engine: &mut Engine, source: &str) -> String {
    engine
        .execute(source)
        .unwrap_or_else(|e| panic!("{source}: {e}"))
        .output
        .iter()
        .map(|entry| format!("{entry}\n"))
        .collect()
}

#[test]
fn eval_display_respects_requested_outputs_and_inner_statements() {
    let mut engine = Engine::default();
    let five = console(&mut engine, "2 + 3");
    for source in ["eval('2 + 3')", "eval('2 + 3');", "h = @eval; h('2 + 3');"] {
        assert_eq!(console(&mut engine, source), five, "{source}");
    }
    for source in [
        "eval('2 + 3;')",
        "eval('2 + 3;');",
        "x = eval('2 + 3');",
        "x = eval('2 + 3;');",
    ] {
        assert_eq!(console(&mut engine, source), "", "{source}");
    }
    assert_eq!(engine.variables["x"].data(), &[5.]);
    let named = console(&mut engine, "x = 5");
    assert_eq!(console(&mut engine, "x = eval('2 + 3')"), named);
    let named = console(&mut engine, "v = 3");
    assert_eq!(console(&mut engine, "eval('v');"), named);
    assert_eq!(console(&mut engine, "eval('v;')"), "");
    assert_eq!(console(&mut engine, "x = eval('v');"), "");
    assert_eq!(engine.variables["x"].data(), &[3.]);
    assert_eq!(
        console(&mut engine, "eval('error(''bad'')', '2 + 3');"),
        five
    );
    assert_eq!(
        console(&mut engine, "x = eval('error(''bad'')', '2 + 3');"),
        ""
    );
    assert_eq!(engine.variables["x"].data(), &[5.]);
}

#[test]
fn structfun_named_builtin_multiple_outputs() {
    let mut engine = Engine::default();
    engine
        .execute("[n,d] = structfun(@rat, struct('a', 0.75, 'b', 0.5));")
        .unwrap();
    assert_eq!(engine.variables["n"].data(), &[3., 1.]);
    assert_eq!(engine.variables["d"].data(), &[4., 2.]);
    engine.execute("[n,d] = structfun(@rat, struct('a', 0.75), 'UniformOutput', false); assert(n.a == 3); assert(d.a == 4);").unwrap();
    // The ErrorHandler also receives the requested output count.
    engine.execute("[n,d] = structfun(@(v) error('bad'), struct('a', 2), 'ErrorHandler', @recover);\nfunction [n,d] = recover(e,v)\nn = v; d = e.index;\nend").unwrap();
    assert_eq!(engine.variables["n"].data(), &[2.]);
    assert_eq!(engine.variables["d"].data(), &[1.]);
    // A user function with the same name still takes precedence.
    engine.execute("[n,d] = structfun(@rat, struct('a', 0.75));\nfunction [n,d] = rat(v)\nn = 8; d = 9;\nend").unwrap();
    assert_eq!(engine.variables["n"].data(), &[8.]);
    assert_eq!(engine.variables["d"].data(), &[9.]);
}

#[test]
fn structfun_handled_errors_restore_diagnostic_locations() {
    let mut engine = Engine::default();
    let error = engine.execute("x = structfun(@bad, struct('a', 1), 'ErrorHandler', @(e,v) e.index);\nerror('later');\nfunction y = bad(v)\nerror('callback');\nend").unwrap_err();
    assert!(error.starts_with("Line 2,"), "{error}");
    assert!(error.ends_with("later"), "{error}");
    assert_eq!(engine.variables["x"].data(), &[1.]);

    let dir = tempfile::tempdir_in(env!("CARGO_MANIFEST_DIR")).unwrap();
    std::fs::write(
        dir.path().join("bad.m"),
        "function y = bad(v)\nerror('callback');\nend",
    )
    .unwrap();
    engine.set_function_directory(dir.path()).unwrap();
    let error = engine
        .execute(
            "x = structfun(@bad, struct('a', 1), 'ErrorHandler', @(e,v) e.index);\nerror('later');",
        )
        .unwrap_err();
    assert!(error.starts_with("Line 2,"), "{error}");
    assert!(engine.file_diagnostic.is_none());
    std::fs::write(
        dir.path().join("recover.m"),
        "function y = recover(e,v)\nerror('handler');\nend",
    )
    .unwrap();
    let error = engine
        .execute("x = structfun(@bad, struct('a', 1), 'ErrorHandler', @recover);")
        .unwrap_err();
    assert!(error.contains("handler"), "{error}");
    assert!(
        engine
            .file_diagnostic
            .as_ref()
            .unwrap()
            .path
            .ends_with("recover.m")
    );
}

#[test]
fn strtok_large_delimiter_sets_and_utf16() {
    let mut engine = Engine::default();
    engine.execute("delims = char(128:65535); s = blanks(65536); [t,r] = strtok(s, delims); assert(isequal(t,s)); assert(isempty(r));").unwrap();
    engine
        .execute("[t,r] = strtok(delims, delims); assert(isempty(t)); assert(isempty(r));")
        .unwrap();
    engine.execute("[t,r] = strtok({char([256 97 65535 98]), 'abc'}, char([65535 256 256])); assert(isequal(t, {'a', 'abc'})); assert(isequal(r{1}, char([65535 98]))); assert(isempty(r{2}));").unwrap();
    engine
        .execute("[t,r] = strtok('abc', ''); assert(isequal(t,'abc')); assert(isempty(r));")
        .unwrap();
}

struct Case {
    id: String,
    statement: String,
    console: String,
    /// `(class, [rows cols])` of x, or the error message.
    outcome: Result<(String, String), String>,
}

fn cases() -> Vec<Case> {
    let mut cases = Vec::new();
    let mut rest = CORPUS;
    while let Some(start) = rest.find("@@CASE ") {
        rest = &rest[start + "@@CASE ".len()..];
        let (header, after) = rest.split_once('\n').expect("case header line");
        let (id, statement) = header.split_once('|').expect("id|statement");
        let (body, after) = after.split_once("@@END\n").expect("case end marker");
        let (console, outcome) = if let Some(message) = body.strip_prefix("ERR ") {
            (
                String::new(),
                Err(message.trim_end_matches('\n').to_owned()),
            )
        } else {
            let (console, class) = body.split_once("@@CLASS ").expect("class line");
            let (class, size) = class.trim_end().split_once(' ').expect("class size");
            (console.to_owned(), Ok((class.to_owned(), size.to_owned())))
        };
        cases.push(Case {
            id: id.into(),
            statement: statement.into(),
            console: plain_warnings(&console),
            outcome,
        });
        rest = after;
    }
    cases
}

/// Drop the reference's `[\x08` `]\x08 ` warning markup and the `> In` stack lines.
fn plain_warnings(text: &str) -> String {
    let text = text.replace("]\u{8} ", "").replace("[\u{8}", "");
    text.lines()
        .filter(|line| !line.starts_with("> In ") && !line.starts_with("  In "))
        .map(|line| format!("{line}\n"))
        .collect()
}

/// An error message without its "Line 1, column N: " location.
fn unplaced(error: &str) -> &str {
    match error.split_once(": ") {
        Some((place, text)) if place.starts_with("Line ") => text,
        _ => error,
    }
}

/// What the case gives in MatRust, in the corpus's terms; `None` matches.
fn mismatch(case: &Case) -> Option<String> {
    let mut engine = Engine::default();
    let result = engine.execute(&case.statement);
    match (&case.outcome, result) {
        (Err(expected), Err(error)) => {
            let actual = unplaced(&error);
            (actual != expected).then(|| format!("error {expected:?}, got {actual:?}"))
        }
        (Err(expected), Ok(_)) => Some(format!("error {expected:?}, got success")),
        (Ok(_), Err(error)) => Some(format!("success, got error {error:?}")),
        (Ok((class, size)), Ok(execution)) => {
            let console: String = execution
                .output
                .iter()
                .map(|entry| format!("{entry}\n"))
                .collect();
            if console != case.console {
                return Some(format!(
                    "console\n  expected {:?}\n  actual   {console:?}",
                    case.console
                ));
            }
            let Some(x) = engine.array("x") else {
                return Some("no variable x".into());
            };
            let actual = (x.class(), format!("[{} {}]", x.rows(), x.cols()));
            (actual != (class.as_str(), size.clone()))
                .then(|| format!("class/size {class} {size}, got {} {}", actual.0, actual.1))
        }
    }
}

#[test]
fn corpus_has_every_case() {
    let cases = cases();
    assert_eq!(cases.len(), 59);
    for (i, case) in cases.iter().enumerate() {
        assert_eq!(case.id, (i + 1).to_string());
    }
    assert_eq!(
        cases.iter().filter(|c| c.outcome.is_err()).count(),
        13,
        "error cases"
    );
    for (id, _) in KNOWN {
        assert!(cases.iter().any(|c| c.id == *id), "known case {id} exists");
    }
    // The markup filter leaves exactly MatRust's one-line warnings.
    assert_eq!(cases[36].console, "Warning: Watch out\n");
    assert_eq!(cases[38].console, "Warning: With id 2\n");
}

#[test]
fn matches_reference() {
    let mut failures = Vec::new();
    let mut passed = 0;
    for case in cases() {
        let known = KNOWN.iter().any(|(id, _)| *id == case.id);
        match mismatch(&case) {
            None if known => failures.push(format!(
                "case {} `{}` passes now; remove it from KNOWN",
                case.id, case.statement
            )),
            None => passed += 1,
            Some(_) if known => {}
            Some(problem) => {
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
    assert_eq!(passed, 59 - KNOWN.len());
}

/// Behaviour the corpus does not cover: eval works in the caller's workspace,
/// a warning identifier switched off stays silent for the session, and rat's
/// tolerance argument.
#[test]
fn session_state_and_options() {
    let mut engine = Engine::default();
    // The console text, as in `mismatch`.
    let console = |engine: &mut Engine, source: &str| -> String {
        engine
            .execute(source)
            .unwrap_or_else(|e| panic!("{source}: {e}"))
            .output
            .iter()
            .map(|entry| format!("{entry}\n"))
            .collect()
    };
    assert_eq!(console(&mut engine, "a = 2; eval('b = a * 3;');"), "");
    assert_eq!(engine.array("b").map(|b| b.class()), Some("double"));
    assert_eq!(
        console(&mut engine, "c = cellstr(vertcat('ab ', 'cd '))"),
        "\nc = \n\n    'ab'\n    'cd'\n\n"
    );
    console(&mut engine, "warning('off', 'my:quiet')");
    assert_eq!(
        console(&mut engine, "warning('my:quiet', 'hidden %d', 1)"),
        ""
    );
    assert_eq!(
        console(&mut engine, "warning('my:loud', 'shown %d', 2)"),
        "Warning: shown 2\n"
    );
    console(&mut engine, "warning('on', 'my:quiet')");
    assert_eq!(
        console(&mut engine, "warning('my:quiet', 'back')"),
        "Warning: back\n"
    );
    assert_eq!(
        console(&mut engine, "[n, d] = rat(pi, 1e-2); disp([n d])"),
        "    22     7\n\n"
    );
    assert_eq!(console(&mut engine, "disp(rat(pi, 1e-2))"), "3 + 1/(7)\n");
    assert_eq!(
        console(
            &mut engine,
            "y = structfun(@(v) error('my:bad', 'no'), struct('a', 1), 'ErrorHandler', @(e, v) e.index * 10)"
        ),
        "\ny =\n\n    10\n\n"
    );
    let error = engine.execute("x = eval('1 +* 2')").unwrap_err();
    assert!(error.contains("Error: "), "{error}");
    console(&mut engine, "eval('x = [1', 'x = 5;')");
    assert_eq!(engine.array("x").map(|x| x.class()), Some("double"));
}
