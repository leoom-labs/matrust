//! A bare variable name echoes under that name and does not touch `ans` (as in the reference).
use matrust::Engine;

fn run(source: &str) -> String {
    let mut engine = Engine::default();
    let result = engine.execute(source).expect("runs");
    result.output.iter().map(|e| format!("{e}\n")).collect()
}

#[test]
fn bare_numeric_logical_char_and_cell_names_echo_their_own_name() {
    for (setup, name) in [
        ("y = 3;", "y"),
        ("b = true;", "b"),
        ("t = 'hi';", "t"),
        ("c = {1, 'a'};", "c"),
        ("z = 1 + 2i;", "z"),
        ("k = int8(5);", "k"),
    ] {
        let out = run(&format!("{setup}\n{name}"));
        assert!(out.contains(&format!("{name} =")), "{name}: {out}");
        assert!(!out.contains("ans ="), "{name}: {out}");
    }
}

#[test]
fn bare_name_leaves_ans_unchanged_and_semicolon_prints_nothing() {
    let out = run("1 + 1;\ny = 7;\ny\ny;\nans");
    assert!(out.contains("y =\n\n     7"), "{out}");
    assert!(out.contains("ans =\n\n     2"), "{out}");
    assert_eq!(out.matches("y =").count(), 1, "{out}");
}
