//! Replays the reference's console text (`evalc` of each statement) through
//! MatRust and compares it byte for byte. Two corpora, same block format:
//! `@@CASE <id>|<statement>`, the exact output, `@@END`.
//!
//! - `tests/fixtures/display/expected.txt`: ids `1`..`65` run under `format short`,
//!   `L1`..`L20` under `format long`. The `disp` cases all have the id `D`; they
//!   are numbered `D1`..`Dn` in file order.
//! - `tests/fixtures/display/probe_expected.txt` (from `probe_cases.m`): ids `P*`
//!   run under `format short`, `PL*` under `format long`, `PC*` under
//!   `format compact`.
//! - `tests/fixtures/display/probe2_expected.txt` (from `probe2_cases.m`): ids `Q*`,
//!   `QL*` (`format long`) and `QC*` (`format compact`); it reruns the struct
//!   probes the first probe script broke.
//!
//! The reference's console was 107 characters wide (`CommandWindowSize`, P154), the
//! engine's default width.
//!
//! The tests read copies in `tests/fixtures/display/` (the Windows gate syncs code paths only).
use matrust::Engine;

const REFERENCE: &str = include_str!("fixtures/display/expected.txt");
const PROBE: &str = include_str!("fixtures/display/probe_expected.txt");
const PROBE2: &str = include_str!("fixtures/display/probe2_expected.txt");

/// `probe_cases.m` stored each `evalc` result in `s`, so every later `s.x = ...`
/// failed in the reference too; `probe2_cases.m` reruns them (Q1..Q6, QL45).
const PROBE_BUG: &str = "oracle block is ERR: the probe script's own `s` broke `s.x = ...`";

/// Cases MatRust cannot run yet, with the reason.
const SKIPPED: &[(&str, &str)] = &[
    ("P113", PROBE_BUG),
    ("P114", PROBE_BUG),
    ("P115", PROBE_BUG),
    ("P118", PROBE_BUG),
    ("PL184", PROBE_BUG),
    (
        "P83",
        "the evaluator keeps the zero imaginary part of `3.5+0i` (MATLAB drops it); not a display rule",
    ),
    (
        "Q23",
        "the reference shortens long text in a too-wide cell row to 18 characters and a mark `evalc` saved as \\x1a; rule not derived",
    ),
];

/// MatRust has no `magic`, root graphics properties or `repmat` of a cell yet;
/// the same values as literals show the same display.
const LITERALS: &[(&str, &str)] = &[
    ("magic(4)", "[16 2 3 13; 5 11 10 8; 9 7 6 12; 4 14 15 1]"),
    ("magic(3)", "[8 1 6; 3 5 7; 4 9 2]"),
    ("get(0,'CommandWindowSize')", "[107 34]"),
];

/// `repmat({'abc'},1,3)` as the literal `{'abc','abc','abc'}`.
fn expand_cell_repmat(statement: &str) -> String {
    let Some(start) = statement.find("repmat({") else {
        return statement.into();
    };
    let args = &statement[start + "repmat({".len()..];
    let (element, rest) = args.split_once("},1,").expect("repmat({x},1,n)");
    let (count, rest) = rest.split_once(')').expect("repmat count");
    let count: usize = count.parse().expect("repmat count");
    format!(
        "{}{{{}}}{}",
        &statement[..start],
        vec![element; count].join(","),
        rest
    )
}

/// MATLAB's desktop wraps class names in help links inside `evalc` output
/// (`<a href="...">struct</a>`); MatRust prints the plain text.
fn strip_links(text: &str) -> String {
    let mut plain = String::new();
    let mut rest = text;
    while let Some(start) = rest.find("<a href=\"") {
        let after = &rest[start..];
        let Some(open_end) = after.find('>') else {
            break;
        };
        let Some(close) = after.find("</a>") else {
            break;
        };
        plain.push_str(&rest[..start]);
        plain.push_str(&after[open_end + 1..close]);
        rest = &after[close + "</a>".len()..];
    }
    plain.push_str(rest);
    plain
}

struct Case {
    id: String,
    statement: String,
    expected: String,
}

fn cases(reference: &str) -> Vec<Case> {
    let mut cases = Vec::new();
    let mut disp_cases = 0;
    let mut rest = reference;
    while let Some(start) = rest.find("@@CASE ") {
        rest = &rest[start + "@@CASE ".len()..];
        let (header, after) = rest.split_once('\n').expect("case header line");
        let (id, statement) = header.split_once('|').expect("id|statement");
        let (expected, after) = after.split_once("@@END\n").expect("case end marker");
        let id = if id == "D" {
            disp_cases += 1;
            format!("D{disp_cases}")
        } else {
            id.to_string()
        };
        cases.push(Case {
            id,
            statement: statement.into(),
            expected: strip_links(expected),
        });
        rest = after;
    }
    cases
}

/// What `evalc` would capture: the console prints each entry and a newline.
fn console(engine: &mut Engine, source: &str) -> String {
    let result = engine
        .execute(source)
        .unwrap_or_else(|e| panic!("{source}: {e}"));
    result
        .output
        .iter()
        .map(|entry| format!("{entry}\n"))
        .collect()
}

#[test]
fn reference_has_every_case() {
    let first = cases(REFERENCE);
    assert_eq!(first.len(), 96);
    assert_eq!(
        first.iter().filter(|c| c.id.starts_with('D')).count(),
        11,
        "disp cases"
    );
    assert_eq!(
        first.iter().filter(|c| c.id.starts_with('L')).count(),
        20,
        "format long cases"
    );
    let probe = cases(PROBE);
    assert_eq!(probe.len(), 207);
    assert_eq!(
        probe.iter().filter(|c| c.id.starts_with("PL")).count(),
        26,
        "format long probes"
    );
    assert_eq!(
        probe.iter().filter(|c| c.id.starts_with("PC")).count(),
        21,
        "format compact probes"
    );
    for (id, _) in SKIPPED {
        assert!(
            first
                .iter()
                .chain(&probe)
                .chain(&cases(PROBE2))
                .any(|c| c.id == *id),
            "skipped case {id} is in an oracle"
        );
    }
    for case in &probe {
        assert_eq!(
            case.expected.starts_with("ERR "),
            SKIPPED.contains(&(case.id.as_str(), PROBE_BUG)),
            "probe {} is skipped exactly when the reference failed",
            case.id
        );
    }
}

/// Runs every case of one corpus in a fresh engine; returns how many passed.
fn replay(reference: &str) -> usize {
    let mut failures = Vec::new();
    let mut passed = 0;
    for case in cases(reference) {
        if SKIPPED.iter().any(|(id, _)| *id == case.id) {
            continue;
        }
        let mut engine = Engine::default();
        let tag = case.id.trim_start_matches(['P', 'Q']);
        if tag.starts_with('L') {
            console(&mut engine, "format long");
        } else if tag.starts_with('C') {
            console(&mut engine, "format compact");
        }
        let mut statement = case.statement.clone();
        for (call, literal) in LITERALS {
            statement = statement.replace(call, literal);
        }
        let statement = expand_cell_repmat(&statement);
        let actual = match engine.execute(&statement) {
            Ok(result) => result
                .output
                .iter()
                .map(|entry| format!("{entry}\n"))
                .collect(),
            Err(e) => format!("ERROR {e}"),
        };
        if actual == case.expected {
            passed += 1;
        } else {
            failures.push(format!(
                "case {} `{}`\n  expected {:?}\n  actual   {:?}",
                case.id, case.statement, case.expected, actual
            ));
        }
    }
    assert!(
        failures.is_empty(),
        "{} passed, {} failed:\n{}",
        passed,
        failures.len(),
        failures.join("\n")
    );
    passed
}

#[test]
fn matches_reference_byte_for_byte() {
    assert_eq!(replay(REFERENCE), 96);
}

#[test]
fn matches_reference_probes_byte_for_byte() {
    assert_eq!(replay(PROBE), 201);
}

#[test]
fn matches_reference_second_probes_byte_for_byte() {
    let probe2 = cases(PROBE2);
    assert_eq!(probe2.len(), 56);
    assert!(probe2.iter().all(|c| !c.expected.starts_with("ERR ")));
    assert_eq!(replay(PROBE2), 55);
}

/// Not reference-checked: `format compact` and a bare `format` were never run on the reference.
#[test]
fn format_commands_switch_precision_and_spacing() {
    let mut e = Engine::default();
    assert_eq!(console(&mut e, "x = pi"), "\nx =\n\n    3.1416\n\n");
    console(&mut e, "format long");
    assert_eq!(
        console(&mut e, "x = pi"),
        "\nx =\n\n   3.141592653589793\n\n"
    );
    console(&mut e, "format compact");
    assert_eq!(console(&mut e, "x = pi"), "x =\n   3.141592653589793\n");
    assert_eq!(console(&mut e, "disp(pi)"), "   3.141592653589793\n");
    console(&mut e, "format short");
    assert_eq!(console(&mut e, "x = [1 2 3]"), "x =\n     1     2     3\n");
    console(&mut e, "format loose");
    assert_eq!(console(&mut e, "x = 6"), "\nx =\n\n     6\n\n");
    console(&mut e, "format long\nformat compact");
    console(&mut e, "format");
    assert_eq!(console(&mut e, "x = pi"), "\nx =\n\n    3.1416\n\n");
    console(&mut e, "format('long');");
    assert_eq!(
        console(&mut e, "x = 0.5"),
        "\nx =\n\n   0.500000000000000\n\n"
    );
    assert!(e.execute("format bank").is_err());
    assert!(e.execute("format long short").is_err());
}

/// Not reference-checked: `format compact` with column blocks.
#[test]
fn compact_format_wraps_without_blank_lines() {
    let mut e = Engine::default();
    console(&mut e, "format compact");
    assert_eq!(
        console(&mut e, "x = 1:20"),
        format!(
            "x =\n  Columns 1 through 17\n{}\n  Columns 18 through 20\n    18    19    20\n",
            (1..=17).map(|n| format!("{n:>6}")).collect::<String>()
        )
    );
}

/// The reference names a two-column block `Columns 18 through 19` (P144); the single
/// `Column N` header is not reference-checked.
#[test]
fn console_width_sets_the_wrapping() {
    let mut e = Engine::default();
    e.set_console_width(40);
    let text = console(&mut e, "x = 1:20");
    assert!(text.contains("  Columns 1 through 6\n"), "{text}");
    assert!(text.contains("  Columns 19 through 20\n"), "{text}");
    e.set_console_width(10);
    let text = console(&mut e, "x = 1:3");
    assert!(text.contains("  Column 1\n"), "{text}");
    assert!(text.contains("  Column 3\n"), "{text}");
}

#[test]
fn cell_and_struct_elements_use_the_short_formatter() {
    let mut e = Engine::default();
    assert_eq!(
        console(&mut e, "c = {pi, 1e10, true, 3+4i, [], 'a', {1}}"),
        "\nc = \n\n    [3.1416]    [1.0000e+10]    [1]    [3.0000 + 4.0000i]    []    'a'    {1x1 cell}\n\n"
    );
    assert_eq!(console(&mut e, "s.a = pi; s.b = [1.5 2]; s.c = 0.001;"), "");
    assert_eq!(
        console(&mut e, "s"),
        "\ns = \n\n    a: 3.1416\n    b: [1.5000 2]\n    c: 1.0000e-03\n\n"
    );
    console(&mut e, "format long");
    assert_eq!(console(&mut e, "t.a = pi;"), "");
    assert_eq!(
        console(&mut e, "t"),
        "\nt = \n\n    a: 3.141592653589793\n\n"
    );
}

#[test]
fn indexed_assignment_echo_uses_the_same_formatter() {
    let mut e = Engine::default();
    console(&mut e, "A = [1 2 3];");
    assert_eq!(
        console(&mut e, "A(2) = 5"),
        "\nA =\n\n     1     5     3\n\n"
    );
    assert_eq!(
        console(&mut e, "A(2) = 5.5"),
        "\nA =\n\n    1.0000    5.5000    3.0000\n\n"
    );
}

#[test]
fn echo_larger_than_the_output_budget_fails_after_the_assignment() {
    let mut e = Engine::default();
    let error = e.execute("x = zeros(1000, 1000)").unwrap_err();
    assert!(error.contains("2 MiB"), "{error}");
    assert_eq!(e.variables["x"].len(), 1_000_000);
    assert!(e.execute("x(1, 1) = 1;").is_ok());
}

#[test]
fn disp_of_character_variables_and_empty_values() {
    let mut e = Engine::default();
    assert_eq!(console(&mut e, "t = 'hi'; disp(t)"), "hi\n");
    // The reference prints nothing for an empty char, matrix or cell (P128-P131).
    assert_eq!(console(&mut e, "disp('')"), "");
    assert_eq!(console(&mut e, "disp(zeros(0, 3))"), "");
    assert_eq!(console(&mut e, "disp({})"), "");
    assert_eq!(console(&mut e, "disp(cell(0, 3))"), "");
}
