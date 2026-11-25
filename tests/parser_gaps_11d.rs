//! Parser gaps from the teaching-script corpus (fix line 11d): a statement on
//! the line of `else`/`otherwise`/`try` without a comma, `%{ ... %}` block
//! comments (alone on their lines, nested), and the desktop Enter check
//! (`is_incomplete_in_workspace`) seeing the Command Window workspace.
use matrust::Engine;
use matrust::syntax::{is_incomplete, is_incomplete_in_workspace, parse};
use std::path::Path;

fn engine(dir: &Path) -> Engine {
    let mut e = Engine::default();
    e.set_file_directory(dir).unwrap();
    e
}
fn value(e: &Engine, name: &str) -> Vec<f64> {
    e.variables
        .get(name)
        .unwrap_or_else(|| panic!("no variable {name}"))
        .data()
        .to_vec()
}
/// Run `source` as the script file `name.m` (the parse_detailed path).
fn run_script(e: &mut Engine, dir: &Path, name: &str, source: &str) -> String {
    std::fs::write(dir.join(format!("{name}.m")), source).unwrap();
    e.execute(name)
        .unwrap_or_else(|err| panic!("{name}: {err}"))
        .output
        .join("\n")
}

#[test]
fn statement_after_else_on_the_same_line() {
    let dir = tempfile::tempdir().unwrap();
    let mut e = engine(dir.path());
    // tests/fixtures/command_syntax_corpus/cism_BarVariableThick.m:52-56
    let script = "ys = [];\nfor nBar = 1:3\n    if     nBar == 1, ymin = 3e-2;\n    elseif nBar == 2, ymin = 2e-2;\n    else               ymin = 0.9e-2;\n    end\n    ys(end+1) = ymin;\nend\n";
    run_script(&mut e, dir.path(), "gap_bars", script);
    assert_eq!(value(&e, "ys"), [3e-2, 2e-2, 0.9e-2]);
    // Command Window text, tab after else, CRLF lines.
    e.execute("k = 0;\r\nif k\r\n  y = 1;\r\nelse\ty = 2;\r\nend")
        .unwrap();
    assert_eq!(value(&e, "y"), [2.]);
    e.execute("if false, y = 1; else y = 3; end").unwrap();
    assert_eq!(value(&e, "y"), [3.]);
    // `else` is a keyword: the next name starts a statement, here a command.
    let out = e
        .execute("if false\n disp no\nelse disp hi\nend")
        .unwrap()
        .output
        .join("\n");
    assert!(out.contains("hi") && !out.contains("no"), "{out}");
    // `else if` stays a nested if with its own end.
    e.execute("if false\nelse if true\n z = 5;\nend\nend")
        .unwrap();
    assert_eq!(value(&e, "z"), [5.]);
    assert!(parse("if false\nelse if true\n z = 5;\nend").is_err());
    // Inside a function file.
    std::fs::write(
        dir.path().join("pick.m"),
        "function r = pick(k)\n    if k > 0\n        r = 1;\n    else r = -1;\n    end\nend\n",
    )
    .unwrap();
    e.execute("a = pick(2); b = pick(-2);").unwrap();
    assert_eq!((value(&e, "a"), value(&e, "b")), (vec![1.], vec![-1.]));
}

#[test]
fn statement_after_otherwise_and_try_on_the_same_line() {
    let dir = tempfile::tempdir().unwrap();
    let mut e = engine(dir.path());
    let script = "out = [];\nfor k = 1:3\n    switch k\n        case 1\n            v = 10;\n        otherwise v = 20 + k;\n    end\n    out(end+1) = v;\nend\n";
    run_script(&mut e, dir.path(), "gap_cases", script);
    assert_eq!(value(&e, "out"), [10., 22., 23.]);
    let out = e
        .execute("switch 'x'\n case 'y'\n  disp no\n otherwise disp yes\nend")
        .unwrap()
        .output
        .join("\n");
    assert!(out.contains("yes") && !out.contains("no"), "{out}");
    std::fs::write(
        dir.path().join("guard.m"),
        "function r = guard(k)\n    v = [7 8];\n    try r = v(k);\n    catch\n        r = 0;\n    end\nend\n",
    )
    .unwrap();
    e.execute("g1 = guard(2); g2 = guard(5);").unwrap();
    assert_eq!((value(&e, "g1"), value(&e, "g2")), (vec![8.], vec![0.]));
}

#[test]
fn block_comments_hold_any_text() {
    let dir = tempfile::tempdir().unwrap();
    let mut e = engine(dir.path());
    // Shaped like tests/fixtures/command_syntax_corpus/encmp100_gorillas_v3.m:12-52.
    let script = "x = 1;\n%{\n    Requirements for Version 3\n    ==========================\n        (a) Banana hits the top of the oposit player's building. In this\n    5. The angle and velocity of Kong's throw [ ( { \"open\n%}\ny = 2;\n";
    run_script(&mut e, dir.path(), "gap_gorillas", script);
    assert_eq!((value(&e, "x"), value(&e, "y")), (vec![1.], vec![2.]));
    // Indented markers with trailing blanks, CRLF, inside a function body.
    std::fs::write(
        dir.path().join("twice.m"),
        "function r = twice(v)\r\n    %{   \r\n    it's [not code\r\n  \t%}\t\r\n    r = 2*v;\r\nend\r\n",
    )
    .unwrap();
    e.execute("t = twice(4);").unwrap();
    assert_eq!(value(&e, "t"), [8.]);
    // Command Window text too.
    e.execute("w = 1;\n%{\nw = 'it''s\n%}\nw = w + 1;").unwrap();
    assert_eq!(value(&e, "w"), [2.]);
}

#[test]
fn block_comments_nest() {
    let dir = tempfile::tempdir().unwrap();
    let mut e = engine(dir.path());
    let script =
        "n = 1;\n%{\nouter\n%{\ninner 'quote\n%}\nstill a comment: player's (\n%}\nn = n + 1;\n";
    run_script(&mut e, dir.path(), "gap_nested", script);
    assert_eq!(value(&e, "n"), [2.]);
    // `%}` with text after it does not close the block.
    let script = "m = 1;\n%{\n%} not a closer\nm = 100;\n%}\nm = m + 1;\n";
    run_script(&mut e, dir.path(), "gap_notcloser", script);
    assert_eq!(value(&e, "m"), [2.]);
}

#[test]
fn percent_brace_with_text_is_an_ordinary_comment() {
    let dir = tempfile::tempdir().unwrap();
    let mut e = engine(dir.path());
    let script = "%{ one-line comment\np = 3;\nq = 4; %{\n%}\nr = p + q;\n";
    run_script(&mut e, dir.path(), "gap_plain", script);
    assert_eq!(value(&e, "r"), [7.]);
    // The lines after `%{ text` are code: a quote there is still an error.
    assert!(parse("%{ text\nit's\n%}").is_err());
    assert!(parse("%{\nit's\n%}").is_ok());
}

#[test]
fn enter_check_sees_the_workspace() {
    let x = |name: &str| name == "x";
    // A workspace variable keeps expression syntax: the bracket waits.
    assert!(is_incomplete_in_workspace("x [", &x));
    assert!(is_incomplete_in_workspace("x [1 2", &x));
    assert!(is_incomplete_in_workspace("x (1,", &x));
    // Closed, the misplaced bracket is a real error and Enter reports it.
    assert!(!is_incomplete_in_workspace("x [1 2]", &x));
    // Without the variable, 11a's command rule: `x('[')`, complete.
    assert!(!is_incomplete("x ["));
    assert!(!is_incomplete_in_workspace("myfn [", &x));
    assert!(!is_incomplete_in_workspace("myfn [1 2", &x));
    // Lexical variables count in both variants.
    assert!(is_incomplete("x = [1 2];\nx ["));
    // The rest of is_incomplete is unchanged by the workspace.
    for src in [
        "if 1",
        "x = [1 2",
        "x = 1 + ...",
        "if 1\n x = 1\nelse x = 2",
    ] {
        assert!(is_incomplete_in_workspace(src, &x), "{src:?}");
        assert!(is_incomplete(src), "{src:?}");
    }
    for src in ["x - 1", "x = = [1 2", "if 1\n x = 1\nelse x = 2\nend", "x"] {
        assert!(!is_incomplete_in_workspace(src, &x), "{src:?}");
        assert!(!is_incomplete(src), "{src:?}");
    }
}

#[test]
fn enter_waits_for_an_open_block_comment() {
    for src in ["%{", "x = 1;\n%{\ny = 2;", "%{\n%{\n%}"] {
        assert!(is_incomplete(src), "{src:?}");
    }
    for src in ["%{\nit's\n%}", "%{ text", "x = 1; %{", "%{\n%{\n%}\n%}"] {
        assert!(!is_incomplete(src), "{src:?}");
    }
}
