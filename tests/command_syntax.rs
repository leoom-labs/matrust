//! MATLAB command syntax for any name with one or more words (fix line 11a):
//! `warning off all`, `axis equal tight`, `myfn foo bar`, `format long g`.
//! Rule from the public "Command vs. Function Syntax" page: after a name
//! that is not a variable, whitespace and then a word that does not read as
//! an operator followed by whitespace make a call with char arguments.
use matrust::Engine;
use matrust::syntax::{Expr, Statement, StatementKind, parse};
use std::path::Path;

/// `myfn` stores its arguments in the base workspace and returns them.
const MYFN: &str = "function out = myfn(varargin)\nassignin('base', 'captured', varargin);\nout = varargin;\nend\n";

fn engine(dir: &Path) -> Engine {
    std::fs::write(dir.join("myfn.m"), MYFN).unwrap();
    let mut e = Engine::default();
    e.set_file_directory(dir).unwrap();
    e
}
fn truth(e: &mut Engine, expression: &str) -> bool {
    e.execute(&format!("zz__ = double(isequal({expression}));"))
        .unwrap();
    e.variables["zz__"].data() == [1.]
}
fn output(e: &mut Engine, source: &str) -> String {
    e.execute(source).unwrap().output.join("\n")
}
fn expressions(statements: &[Statement]) -> Vec<Expr> {
    statements
        .iter()
        .flat_map(|s| match &s.kind {
            StatementKind::Evaluate(e) => vec![e.expr.clone()],
            StatementKind::Function(f) => expressions(&f.body),
            StatementKind::For { body, .. } | StatementKind::While { body, .. } => {
                expressions(body)
            }
            StatementKind::Try { body, handler, .. } => {
                let mut found = expressions(body);
                found.extend(expressions(handler));
                found
            }
            _ => Vec::new(),
        })
        .collect()
}
/// The call each statement of `source` makes with char words, or None for
/// any other expression.
fn calls(source: &str) -> Vec<Option<(String, Vec<String>)>> {
    let statements = parse(source).unwrap_or_else(|e| panic!("{source}: {e}"));
    expressions(&statements)
        .into_iter()
        .map(|expr| match expr {
            Expr::Call(name, args) => args
                .into_iter()
                .map(|a| match a {
                    Expr::String(text) => Some(text),
                    _ => None,
                })
                .collect::<Option<Vec<_>>>()
                .map(|words| (name, words)),
            _ => None,
        })
        .collect()
}
fn call(name: &str, words: &[&str]) -> Option<(String, Vec<String>)> {
    Some((name.into(), words.iter().map(|w| w.to_string()).collect()))
}

#[test]
fn any_name_takes_words_as_char_arguments() {
    for (source, name, words) in [
        ("format long g", "format", &["long", "g"][..]),
        ("close all force", "close", &["all", "force"]),
        ("close all hidden", "close", &["all", "hidden"]),
        ("warning off all", "warning", &["off", "all"]),
        ("axis equal tight", "axis", &["equal", "tight"]),
        ("myfn foo bar", "myfn", &["foo", "bar"]),
        ("myfn 'two words' x;", "myfn", &["two words", "x"]),
        ("myfn 'it''s'", "myfn", &["it's"]),
        ("myfn a'b c'd", "myfn", &["ab cd"]),
        ("myfn -1", "myfn", &["-1"]),
        ("a -b", "a", &["-b"]),
        ("a ==b", "a", &["==b"]),
        ("ls ./d", "ls", &["./d"]),
        ("myfn \"s\"", "myfn", &["\"s\""]),
        ("myfn x+1 [1 2]", "myfn", &["x+1", "[1", "2]"]),
        ("myfn a b % c d", "myfn", &["a", "b"]),
        ("myfn\ta\t b", "myfn", &["a", "b"]),
        ("myfn a ...\n b", "myfn", &["a", "b"]),
        ("hold on", "hold", &["on"]),
        ("disp hello world", "disp", &["hello world"]),
        ("disp x+1", "disp", &["x+1"]),
        ("disp 'two words'", "disp", &["two words"]),
    ] {
        assert_eq!(calls(source), vec![call(name, words)], "{source}");
    }
    // `;` and `,` end the command; the next statement follows.
    assert_eq!(
        calls("myfn a; myfn b c, myfn d"),
        vec![
            call("myfn", &["a"]),
            call("myfn", &["b", "c"]),
            call("myfn", &["d"])
        ]
    );
    assert!(parse("myfn 'open").unwrap_err().contains("not terminated"));
    // A statement on the line of try/catch/otherwise.
    assert_eq!(
        calls("try myfn a b\ncatch disp hi\nend"),
        vec![call("myfn", &["a", "b"]), call("disp", &["hi"])]
    );
}

#[test]
fn expressions_and_variables_keep_expression_syntax() {
    // Assignment, `(`, an operator with whitespace after it, or no space.
    for source in [
        "a = b", "a =b", "a (1)", "a - b", "a == b", "a ~= b", "a .* b", "ls ./ d", "ls./d", "a'",
        "a : b",
    ] {
        assert_eq!(calls(source), vec![None], "{source}");
    }
    // A name assigned earlier in the code unit is a variable: `x -1` subtracts.
    for source in [
        "x = 3; x -1",
        "x(2) = 3; x -1",
        "s.a = 1; s -1",
        "[p, x] = deal(1, 2); x -1",
        "for x = 1:2, x -1, end",
        "global x\nx -1",
        "try\nerror('e');\ncatch x\nx -1\nend",
        "function f(x)\nx -1\nend",
        "function [y, x] = f\nx -1\nend",
    ] {
        let found = calls(source);
        assert_eq!(found.last(), Some(&None), "{source}: {found:?}");
    }
    // The next function is a new code unit: its x is not the first one's.
    assert_eq!(
        calls("function f(x)\nend\nfunction g\nx -1\nend"),
        vec![call("x", &["-1"])]
    );
    // Before the assignment it is still a command.
    assert_eq!(calls("x -1\nx = 1;")[0], call("x", &["-1"]));
}

#[test]
fn commands_run_in_the_command_window() {
    let dir = tempfile::tempdir().unwrap();
    let mut e = engine(dir.path());
    e.execute("myfn foo bar").unwrap();
    assert!(truth(&mut e, "captured, {'foo', 'bar'}"));
    e.execute("myfn 'two words' x;").unwrap();
    assert!(truth(&mut e, "captured, {'two words', 'x'}"));
    e.execute("myfn -1").unwrap();
    assert!(truth(&mut e, "captured, {'-1'}"));
    e.execute("myfn a b % comment").unwrap();
    assert!(truth(&mut e, "captured, {'a', 'b'}"));
    // `;` suppresses the output, `,` shows it.
    assert_eq!(output(&mut e, "myfn quiet;"), "");
    assert!(output(&mut e, "myfn shown, myfn last;").contains("ans"));
    assert!(truth(&mut e, "captured, {'last'}"));
    // Variable -> expression, also across runs and inside eval.
    e.execute("x = 3; x -1").unwrap();
    assert!(truth(&mut e, "ans, 2"));
    e.execute("x -2").unwrap();
    assert!(truth(&mut e, "ans, 1"));
    e.execute("eval('x -3')").unwrap();
    assert!(truth(&mut e, "ans, 0"));
    e.execute("a = 5; b = 2; a - b").unwrap();
    assert!(truth(&mut e, "ans, 3"));
    e.execute("a == b").unwrap();
    assert!(truth(&mut e, "ans, false"));
    // Command words are text, never evaluated.
    assert_eq!(output(&mut e, "disp x+1"), "x+1");
    assert_eq!(output(&mut e, "disp hello world"), "hello world");
    assert_eq!(output(&mut e, "disp 'hello'"), "hello");
}

#[test]
fn warning_off_all_and_axis_equal_tight() {
    let mut e = Engine::default();
    assert!(output(&mut e, "warning('w:x', 'loud')").contains("loud"));
    e.execute("warning off all").unwrap();
    assert_eq!(output(&mut e, "warning('w:x', 'quiet')"), "");
    e.execute("warning on all").unwrap();
    assert!(output(&mut e, "warning('w:x', 'back')").contains("back"));

    e.execute("plot(1:3); axis([0 5 0 5]); axis equal tight")
        .unwrap();
    let axes = &e.figure.as_ref().unwrap().axes;
    assert!(axes.extras.equal_aspect && axes.xlim.is_none() && axes.ylim.is_none());
}

#[test]
fn two_word_options_reach_their_function() {
    // These parse into two-argument calls, which run since fix 11b1.
    let mut e = Engine::default();
    e.execute("format long g").unwrap();
    assert_eq!(output(&mut e, "disp(pi)").trim(), "3.14159265358979");
    for source in ["figure; close all force", "figure; close all hidden"] {
        e.execute(source)
            .unwrap_or_else(|error| panic!("{source}: {error}"));
        assert!(e.figure_ids().is_empty(), "{source}");
    }
    e.execute("format short").unwrap();
    // One word keeps working.
    e.execute("format long").unwrap();
    assert_eq!(output(&mut e, "disp(pi)").trim(), "3.141592653589793");
    e.execute("format short").unwrap();
}

const LINES: &str = "warning off all
x = 3;
x -1;
first = ans;
myfn foo bar % comment
c1 = ans;
myfn 'two words' x, c2 = ans;
myfn -1;
c3 = ans;
axis equal tight
";

#[test]
fn commands_in_function_and_script_files() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(dir.path().join("cmdscript.m"), LINES).unwrap();
    std::fs::write(
        dir.path().join("cmdfn.m"),
        format!(
            "function out = cmdfn(n)\n{LINES}n -1;\nsecond = ans;\nout = {{first, second, c1, c2, c3}};\nend\n"
        ),
    )
    .unwrap();
    let mut e = engine(dir.path());
    e.execute("cmdscript").unwrap();
    assert!(truth(
        &mut e,
        "{first, c1, c2, c3}, {2, {'foo', 'bar'}, {'two words', 'x'}, {'-1'}}"
    ));
    assert!(e.figure.as_ref().unwrap().axes.extras.equal_aspect);

    let mut e = engine(dir.path());
    e.execute("r = cmdfn(10);").unwrap();
    assert!(truth(
        &mut e,
        "r, {2, 9, {'foo', 'bar'}, {'two words', 'x'}, {'-1'}}"
    ));
    assert!(e.figure.as_ref().unwrap().axes.extras.equal_aspect);
}

#[test]
fn file_commands_are_unchanged() {
    let dir = tempfile::tempdir().unwrap();
    let mut e = engine(dir.path());
    e.execute("v = 5; w = 6; save data v w; clear v w; load data v")
        .unwrap();
    assert!(truth(&mut e, "v, 5"));
    assert!(!e.contains_array("w"));
    e.execute("delete data.mat").unwrap();
    assert!(!dir.path().join("data.mat").exists());
    // The general rule leaves these names to the file-command rules.
    assert_eq!(calls("clear all"), vec![call("clear", &["all"])]);
    assert_eq!(
        calls("print -dpng out"),
        vec![call("print", &["-dpng", "out"])]
    );
}

// Fix line 12: subplot, figure and view read number words as numbers, as
// MATLAB's own functions do; rng and pause already did.

#[test]
fn subplot_number_word_matches_numeric_call() {
    let mut e = Engine::default();
    e.execute("subplot(2,1,1); want = get(gca, 'Position'); close all; subplot 211; got = get(gca, 'Position');")
        .unwrap();
    assert!(truth(&mut e, "got, want"));
}

#[test]
fn subplot_three_number_words_match_numeric_call() {
    let mut e = Engine::default();
    e.execute("subplot(2,1,1); want = get(gca, 'Position'); close all; subplot 2 1 1; got = get(gca, 'Position');")
        .unwrap();
    assert!(truth(&mut e, "got, want"));
    assert!(e.execute("subplot two 1 1").is_err());
}

#[test]
fn figure_number_word_makes_that_figure_current() {
    let mut e = Engine::default();
    e.execute("figure 3; n = get(gcf, 'Number');").unwrap();
    assert!(truth(&mut e, "n, 3"));
    assert!(e.execute("figure three").is_err());
}

#[test]
fn view_number_words_set_default_views() {
    let mut e = Engine::default();
    e.execute("plot3(1:3, 1:3, 1:3); view 2; v2 = view; view 3; v3 = view;")
        .unwrap();
    assert!(truth(&mut e, "v2, [0 90]"));
    assert!(truth(&mut e, "v3, [-37.5 30]"));
    assert!(e.execute("view sideways").is_err());
}

#[test]
fn rng_number_word_and_default_match_function_calls() {
    let mut e = Engine::default();
    e.execute("rng(0); a = rand(1, 3); rng 0; b = rand(1, 3);")
        .unwrap();
    assert!(truth(&mut e, "a, b"));
    e.execute(
        "rand(1, 5); rng('default'); c = rand(1, 3); rand(1, 5); rng default; d = rand(1, 3);",
    )
    .unwrap();
    assert!(truth(&mut e, "c, d"));
}

#[test]
fn pause_number_words_return() {
    let mut e = Engine::default();
    let start = std::time::Instant::now();
    e.execute("pause 0").unwrap();
    e.execute("pause 0.5").unwrap();
    assert!(start.elapsed() >= std::time::Duration::from_millis(400));
}
