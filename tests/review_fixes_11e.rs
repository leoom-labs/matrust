//! Fix line 11 whole-step review fixes (11e): script names come back after a
//! local function's `end`; a document (editor/browser Run, a script file run
//! from the shell) decides command syntax lexically, without the workspace;
//! `c {2} = 5` with `c` unknown assigns again.
use matrust::Engine;
use matrust::syntax::{Expr, StatementKind, parse};
use std::path::Path;

/// `capture.m` returns its arguments, so a command call is visible.
fn engine(dir: &Path) -> Engine {
    std::fs::write(
        dir.join("capture.m"),
        "function out = capture(varargin)\nout = varargin;\nend\n",
    )
    .unwrap();
    let mut e = Engine::default();
    e.set_file_directory(dir).unwrap();
    e.set_function_directory(dir).unwrap();
    e
}
fn truth(e: &mut Engine, expression: &str) -> bool {
    e.execute(&format!("zz__ = double(isequal({expression}));"))
        .unwrap();
    e.variables["zz__"].data() == [1.]
}
fn words(output: &str) -> Vec<&str> {
    output.split_whitespace().collect()
}
/// Run `source` as the script file `name.m`, by name and as a document.
fn script_outputs(dir: &Path, name: &str, source: &str) -> [String; 2] {
    let path = dir.join(format!("{name}.m"));
    std::fs::write(&path, source).unwrap();
    let by_name = engine(dir)
        .execute(name)
        .unwrap_or_else(|err| panic!("{name}: {err}"))
        .output
        .join("\n");
    let document = engine(dir)
        .execute_document(source, &path)
        .unwrap_or_else(|err| panic!("{name} document: {err}"))
        .output
        .join("\n");
    [by_name, document]
}

const LOCAL: &str = "function y = f(t)\n  y = t;\nend\n";

#[test]
fn script_names_survive_a_local_function_block() {
    let dir = tempfile::tempdir().unwrap();
    // Review report item 1: `x -1` after the block subtracts.
    let source = format!("x = 1:100;\n{LOCAL}x -1;\ndisp(ans(1:2))\n");
    for out in script_outputs(dir.path(), "after_block", &source) {
        assert_eq!(words(&out), ["0", "1"], "{out}");
    }
    let source = format!("x = 3;\n{LOCAL}x -1;\ndisp(ans)\n");
    for out in script_outputs(dir.path(), "after_block_scalar", &source) {
        assert_eq!(words(&out), ["2"], "{out}");
    }
    // Two local functions with blocks inside; the script goes on after each.
    let source = "x = 10;\nfunction y = g(t)\n  if t > 0\n    for k = 1:2, y = t; end\n  else y = 0;\n  end\nend\nx -1;\na = ans;\nfunction h\n  try, capture a; catch, end\nend\nx -2;\ndisp([a ans g(4)])\n";
    for out in script_outputs(dir.path(), "two_blocks", source) {
        assert_eq!(words(&out), ["9", "8", "4"], "{out}");
    }
    // Inside the local function a script name is not a variable: command.
    let source = "capture = 3;\nfunction r = inner()\n  capture -1;\n  r = ans;\nend\ncapture -1;\ndisp([class(inner()) ' ' num2str(ans)])\n";
    for out in script_outputs(dir.path(), "inside_block", source) {
        assert_eq!(words(&out), ["cell", "2"], "{out}");
    }
}

#[test]
fn function_file_functions_scope_separately() {
    let dir = tempfile::tempdir().unwrap();
    let mut e = engine(dir.path());
    // With ends and without: `x` is assigned in pf only, so in helper
    // `x -1` is the command `x('-1')` and calls x.m.
    std::fs::write(
        dir.path().join("x.m"),
        "function out = x(varargin)\nout = varargin;\nend\n",
    )
    .unwrap();
    std::fs::write(
        dir.path().join("pf.m"),
        "function r = pf()\n  x = 5;\n  x -1;\n  r = {ans, helper()};\nend\nfunction r = helper()\n  x -1;\n  r = ans;\nend\n",
    )
    .unwrap();
    std::fs::write(
        dir.path().join("pg.m"),
        "function r = pg()\n  x = 5;\n  x -1;\n  r = {ans, helper()};\nfunction r = helper()\n  x -1;\n  r = ans;\n",
    )
    .unwrap();
    e.execute("a = pf(); b = pg();").unwrap();
    assert!(truth(&mut e, "a, {4, {'-1'}}"));
    assert!(truth(&mut e, "b, {4, {'-1'}}"));
}

#[test]
fn nested_local_function_in_a_script() {
    let dir = tempfile::tempdir().unwrap();
    // Per docs/tasks/2026-11-19-corpus1k2-nested-functions-rest.md (corpus item 2 piece 2),
    // end-terminated local functions in scripts may contain nested functions, and the lexer's
    // block count stays balanced across the inner end so following script statements still run.
    let source = "x = 1:100;\nfunction y = f(t)\n  y = g(t);\n  function z = g(u)\n    z = u;\n  end\nend\nx -1;\ndisp(ans(1:2))\n";
    parse(source).unwrap();
    let path = dir.path().join("nested.m");
    std::fs::write(&path, source).unwrap();
    let out = engine(dir.path()).execute_document(source, &path).unwrap();
    assert_eq!(words(&out.output.join("\n")), ["0", "1"]);
}

#[test]
fn documents_parse_without_the_workspace() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("doc.m");
    let cases = [
        ("close = 5;", "clear all; close all; disp ok"),
        ("grid = 0;", "clear grid; plot(1:3); grid on; disp ok"),
        (
            "legend = {1};",
            "clear; clc; plot(1:3); legend show; disp ok",
        ),
    ];
    for (setup, document) in cases {
        // Editor Run of a saved file, `matrust doc.m`.
        let mut e = engine(dir.path());
        e.execute(setup).unwrap();
        let out = e
            .execute_document(document, &path)
            .unwrap_or_else(|err| panic!("{setup} {document}: {err}"));
        assert_eq!(words(&out.output.join("\n")), ["ok"], "{document}");
        // Unsaved editor tab, browser Run.
        let mut e = engine(dir.path());
        e.execute(setup).unwrap();
        let out = e
            .execute_script(document)
            .unwrap_or_else(|err| panic!("{setup} {document}: {err}"));
        assert_eq!(words(&out.output.join("\n")), ["ok"], "{document}");
        // A selection's document parses lexically; the selected text is
        // Command Window text.
        let mut e = engine(dir.path());
        e.execute(setup).unwrap();
        let start = document.rfind("disp").unwrap();
        let out = e
            .execute_selection(document, start..document.len())
            .unwrap_or_else(|err| panic!("{setup} {document}: {err}"));
        assert_eq!(words(&out.output.join("\n")), ["ok"], "{document}");
    }
    // A document's own assignments still make variables.
    let out = engine(dir.path())
        .execute_document("y = 2; y -1; disp(ans)", &path)
        .unwrap();
    assert_eq!(words(&out.output.join("\n")), ["1"]);
}

#[test]
fn command_window_keeps_the_workspace() {
    let dir = tempfile::tempdir().unwrap();
    let mut e = engine(dir.path());
    e.execute("close = 5;").unwrap();
    // As before 11e: `close` is a workspace variable, so `close all` is not
    // a command and the line does not parse.
    let error = e.execute("close all").unwrap_err();
    assert!(error.contains("Expected a statement separator"), "{error}");
    let error = e.execute("clear all; close all; disp ok").unwrap_err();
    assert!(error.contains("Expected a statement separator"), "{error}");
    // eval keeps it too; `x -1` subtracts with a workspace x.
    e.execute("x = 3; x -1;").unwrap();
    assert_eq!(e.variables["ans"].data(), [2.]);
    e.execute("eval('x -2;')").unwrap();
    assert_eq!(e.variables["ans"].data(), [1.]);
}

#[test]
fn brace_assignment_with_a_blank_assigns() {
    let dir = tempfile::tempdir().unwrap();
    let mut e = engine(dir.path());
    let out = e.execute("c {2} = 5; disp(class(c))").unwrap();
    assert_eq!(words(&out.output.join("\n")), ["cell"]);
    assert!(truth(&mut e, "c, {[], 5}"));
    e.execute("d {1, 2} = 'x';").unwrap();
    assert!(truth(&mut e, "d, {[], 'x'}"));
    for out in script_outputs(dir.path(), "brace_assign", "c {2} = 5;\ndisp(class(c))\n") {
        assert_eq!(words(&out), ["cell"], "{out}");
    }
    // Without `=` (or with `==`) a brace word stays a command argument.
    let command = |source: &str| -> Option<Vec<String>> {
        let statements = parse(source).unwrap_or_else(|e| panic!("{source}: {e}"));
        match &statements[0].kind {
            StatementKind::Evaluate(e) => match &e.expr {
                Expr::Call(_, args) => args
                    .iter()
                    .map(|a| match a {
                        Expr::String(text) => Some(text.clone()),
                        _ => None,
                    })
                    .collect(),
                _ => None,
            },
            _ => None,
        }
    };
    assert_eq!(command("c {2}"), Some(vec!["{2}".to_owned()]));
    assert_eq!(command("disp {x}"), Some(vec!["{x}".to_owned()]));
    assert_eq!(
        command("c {2} == 5"),
        Some(vec!["{2}".to_owned(), "==".to_owned(), "5".to_owned()])
    );
    assert_eq!(command("c {2} = 5"), None);
}
