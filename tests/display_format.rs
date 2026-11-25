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
            // `evalc` saved the cell-text ellipsis (U+2026) as the substitute byte 0x1A (Q23).
            expected: strip_links(expected).replace('\u{1a}', "\u{2026}"),
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
    assert_eq!(replay(PROBE), 202);
}

#[test]
fn matches_reference_second_probes_byte_for_byte() {
    let probe2 = cases(PROBE2);
    assert_eq!(probe2.len(), 56);
    assert!(probe2.iter().all(|c| !c.expected.starts_with("ERR ")));
    assert_eq!(replay(PROBE2), 56);
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

/// MathWorks empty-array and string captions (review F1, 2026-11-12).
#[test]
fn empty_and_string_assignment_captions_review_f1112() {
    let mut e = Engine::default();
    let cases = [
        ("B = zeros(0,5)", "0\u{00d7}5 empty double matrix"),
        ("C = ones(3,0,5)", "3\u{00d7}0\u{00d7}5 empty double array"),
        ("A = zeros(0,0,'uint8')", "0\u{00d7}0 empty uint8 matrix"),
        ("S = strings(0,5)", "0\u{00d7}5 empty string array"),
        ("chr = ''", "0\u{00d7}0 empty char array"),
        ("G = zeros(0,3,'single')", "0\u{00d7}3 empty single matrix"),
        ("L = logical(zeros(0,2))", "0\u{00d7}2 empty logical array"),
        (
            "Z = complex(zeros(0,5), zeros(0,5))",
            "0\u{00d7}5 empty double matrix",
        ),
        ("M = []", "[]"),
        ("Z0 = complex(zeros(0,0), zeros(0,0))", "[]"),
        ("str = strings(2,3)", "2\u{00d7}3 string"),
    ];
    for (code, caption) in cases {
        let text = console(&mut e, code);
        assert!(
            text.contains(caption),
            "{code} -> {text:?} missing {caption}"
        );
        assert!(
            !text.contains("Empty matrix:") && !text.contains("Empty array:"),
            "{code} -> {text:?}"
        );
    }
    let scalar = console(&mut e, "s = \"only\"");
    assert!(scalar.contains("\"only\""), "{scalar:?}");
    assert!(!scalar.contains("string"), "{scalar:?}");
    let wide = console(&mut e, "str = strings(2,3)");
    assert!(!wide.contains("string array"), "{wide:?}");
    assert_eq!(console(&mut e, "disp(zeros(0,5))"), "");
    assert_eq!(console(&mut e, "disp('')"), "");
    assert_eq!(console(&mut e, "disp(strings(0,5))"), "");
    assert_eq!(console(&mut e, "disp(ones(3,0,5))"), "");
    let cell = console(&mut e, "c = cell(0,3)");
    assert!(
        cell.contains("Empty cell array: 0-by-3"),
        "cell header changed: {cell:?}"
    );
    let zero = console(&mut e, "z = complex(zeros(0,5), zeros(0,5))");
    assert!(!zero.contains("complex"), "{zero:?}");
}

/// Integer display: scalar echo/disp follows the public MathWorks int32/uint64
/// pages (class label line `  int32` and 3 leading spaces for scalars); array,
/// matrix, and N-D page widths follow GNU Octave 10.3.0 (`tool/ref-octave.sh`).
#[test]
fn int_display_scalars_echo_and_all_classes() {
    let mut e = Engine::default();
    assert_eq!(console(&mut e, "disp(int32(3))"), "   3\n\n");
    assert_eq!(console(&mut e, "disp(uint64(2))"), "   2\n\n");

    let echo = console(&mut e, "x = int32(3)");
    assert_eq!(echo, "\nx =\n\n  int32\n\n   3\n\n");

    for cls in [
        "int8", "uint8", "int16", "uint16", "int32", "uint32", "int64", "uint64",
    ] {
        let out = console(&mut e, &format!("v = {cls}(7)"));
        assert_eq!(
            out,
            format!("\nv =\n\n  {cls}\n\n   7\n\n"),
            "{cls}: {out:?}"
        );
    }

    e.execute("format compact").unwrap();
    assert_eq!(console(&mut e, "disp(int32(3))"), "   3\n");
    assert_eq!(console(&mut e, "x = int32(3)"), "x =\n  int32\n   3\n");
    e.execute("format loose").unwrap();
}

#[test]
fn int_display_arrays_matrices_nd_empty_and_map_count() {
    let mut e = Engine::default();
    // Octave 10.3.0: disp(int32([3 40])) -> '   3  40'
    assert_eq!(console(&mut e, "disp(int32([3 40]))"), "   3  40\n\n");
    // Octave 10.3.0: disp(int8([-5 100; 7 0])) -> '    -5   100\n     7     0'
    assert_eq!(
        console(&mut e, "disp(int8([-5 100; 7 0]))"),
        "    -5   100\n     7     0\n\n"
    );
    // Empty uint8
    assert_eq!(console(&mut e, "disp(uint8([]))"), "");
    assert_eq!(
        console(&mut e, "u = uint8([])"),
        "\nu =\n\n  0\u{00d7}0 empty uint8 matrix\n\n"
    );
    // N-D int16 page (Octave 10.3.0: '  1  3\n  2  4')
    let nd_disp = console(&mut e, "disp(reshape(int16(1:8), 2, 2, 2))");
    assert_eq!(
        nd_disp,
        "(:,:,1) =\n\n  1  3\n  2  4\n\n\n(:,:,2) =\n\n  5  7\n  6  8\n"
    );
    let nd_echo = console(&mut e, "a = reshape(int16(1:8), 2, 2, 2)");
    assert!(
        nd_echo.contains("a(:,:,1) =\n\n  int16\n\n  1  3\n  2  4"),
        "{nd_echo:?}"
    );
    assert!(
        nd_echo.contains("a(:,:,2) =\n\n  int16\n\n  5  7\n  6  8"),
        "{nd_echo:?}"
    );
    // containers.Map Count is uint64
    e.execute("m = containers.Map({'a','b'}, {1, 2});").unwrap();
    assert_eq!(console(&mut e, "disp(m.Count)"), "   2\n\n");
    assert_eq!(
        console(&mut e, "m.Count"),
        "\nans =\n\n  uint64\n\n   2\n\n"
    );
}

#[test]
fn int_display_signed_min_widths_match_octave() {
    let mut e = Engine::default();
    // GNU Octave 10.3.0: 6 spaces before 0, 2 spaces before 32512.
    assert_eq!(
        console(&mut e, "disp(int16([-32768;0;32512]))"),
        "  -32768\n      0\n  32512\n\n"
    );
    // GNU Octave 10.3.0: 4 spaces before 0, 2 spaces before 127.
    assert_eq!(
        console(&mut e, "disp(int8([-128;0;127]))"),
        "  -128\n    0\n  127\n\n"
    );
}

#[test]
fn int_display_large_array_obeys_budget() {
    let mut small = Engine::with_limits(matrust::engine::Limits {
        display_bytes: 2 * 1024 * 1024,
        ..Default::default()
    });
    small.execute("x = zeros(8000000, 1, 'uint8');").unwrap();
    let err = small.execute("disp(x)").unwrap_err();
    assert!(
        err.contains("output") || err.contains("large") || err.contains("MiB"),
        "{err}"
    );

    let mut e = Engine::default();
    e.execute("y = zeros(9000000, 1, 'uint8');").unwrap();
    let err = e.execute("disp(y)").unwrap_err();
    assert!(
        err.contains("output") || err.contains("large") || err.contains("MiB"),
        "{err}"
    );
}

fn visible(engine: &mut Engine, source: &str) -> String {
    console(engine, source).trim().to_owned()
}

/// Engineering notation is not `shortE`/`longE` with a new name: the exponent
/// is a multiple of 3 and the text is `mantissa` + `e` + sign + three exponent digits.
#[test]
fn engineering_notation_prints_powers_of_one_thousand() {
    let mut e = Engine::default();
    console(&mut e, "format shortEng");
    assert_eq!(visible(&mut e, "disp(12345)"), "12.3450e+003");
    assert_eq!(visible(&mut e, "disp(0.01234567890123456)"), "12.3457e-003");
    assert_eq!(visible(&mut e, "disp(-12345)"), "-12.3450e+003");
    assert_eq!(visible(&mut e, "disp(0)"), "0");
    assert_eq!(visible(&mut e, "disp(NaN)"), "NaN");
    assert_eq!(visible(&mut e, "disp(Inf)"), "Inf");
    assert_eq!(visible(&mut e, "disp(-Inf)"), "-Inf");
    assert_eq!(visible(&mut e, "disp([NaN 12345])"), "NaN  12.3450e+003");
    // 999.99995 rounds to 1000.0000 at 4 decimals, then to the next power of 1000.
    assert_eq!(visible(&mut e, "disp(999.99995)"), "1.0000e+003");
    assert_eq!(visible(&mut e, "disp(-999.99995)"), "-1.0000e+003");
    assert_eq!(visible(&mut e, "disp(999.94999)"), "999.9500e+000");
    assert_eq!(
        visible(&mut e, "disp([12345 0.01234567890123456])"),
        "12.3450e+003  12.3457e-003"
    );
    assert_eq!(
        visible(&mut e, "disp([12345; 0.01234567890123456])"),
        "12.3450e+003\n  12.3457e-003"
    );
    assert_eq!(visible(&mut e, "disp(3+4i)"), "3.0000e+000 + 4.0000e+000i");
    assert_eq!(
        visible(&mut e, "disp(12345-0.01234567890123456i)"),
        "12.3450e+003 - 12.3457e-003i"
    );
    assert!(visible(&mut e, "disp({12345})").contains("12.3450e+003"));
    console(&mut e, "s.a = 12345;");
    assert!(visible(&mut e, "disp(s)").contains("12.3450e+003"));

    console(
        &mut e,
        "x = [12345 0.01234567890123456]; z = 12345-0.01234567890123456i;",
    );
    let x_before = e.variables["x"].data().to_vec();
    let z_before = e.complex_variables["z"].data().to_vec();
    console(&mut e, "disp(x); disp(z);");
    assert_eq!(e.variables["x"].data(), x_before.as_slice());
    assert_eq!(e.complex_variables["z"].data(), z_before.as_slice());

    // Stored bits, so the text cannot depend on dividing by an overflowing power of ten.
    e.variables
        .insert("tiny".into(), matrust::Matrix::scalar(f64::from_bits(1)));
    e.variables
        .insert("huge".into(), matrust::Matrix::scalar(f64::MAX));
    e.variables
        .insert("negzero".into(), matrust::Matrix::scalar(-0.0));
    assert_eq!(visible(&mut e, "disp(tiny)"), "4.9407e-324");
    assert_eq!(e.variables["tiny"].data()[0].to_bits(), 1);
    assert_eq!(visible(&mut e, "disp(huge)"), "179.7693e+306");
    assert_eq!(e.variables["huge"].data()[0].to_bits(), f64::MAX.to_bits());
    assert_eq!(visible(&mut e, "disp(negzero)"), "0");
    assert!(e.variables["negzero"].data()[0].is_sign_negative());

    console(&mut e, "a = 1e-320; b = 1e308;");
    assert!(e.variables["a"].data()[0].is_finite() && e.variables["a"].data()[0] != 0.);
    assert!(e.variables["b"].data()[0].is_finite());
    let a_bits = e.variables["a"].data()[0].to_bits();
    let b_bits = e.variables["b"].data()[0].to_bits();
    assert_eq!(visible(&mut e, "disp(a)"), "9.9999e-321");
    assert_eq!(visible(&mut e, "disp(b)"), "100.0000e+306");
    assert_eq!(e.variables["a"].data()[0].to_bits(), a_bits);
    assert_eq!(e.variables["b"].data()[0].to_bits(), b_bits);
    assert_eq!(visible(&mut e, "disp(realmin)"), "22.2507e-309");

    let logical = visible(&mut e, "disp([true false])");
    assert!(!logical.contains('e'), "{logical}");

    // GNU Octave 10.3 `format short eng` anchors.
    assert_eq!(visible(&mut e, "disp(1)"), "1.0000e+000");
    assert_eq!(visible(&mut e, "disp(999999)"), "999.9990e+003");
    assert_eq!(visible(&mut e, "disp(1e-310)"), "100.0000e-312");
    assert_eq!(visible(&mut e, "disp(single(12345))"), "12.3450e+003");
    assert_eq!(visible(&mut e, "x = 0"), "x =\n\n  0");

    console(&mut e, "format longEng");
    assert_eq!(
        visible(&mut e, "disp(0.01234567890123456)"),
        "12.345678901234560e-003"
    );
    // Octave prints 12.345000000000001e+003 (binary 12345/1000); the mantissa here
    // comes from exact decimal digits, so the last digit stays 0.
    assert_eq!(visible(&mut e, "disp(12345)"), "12.345000000000000e+003");
    assert_eq!(
        visible(&mut e, "disp(single(0.0123456789))"),
        // Octave: 12.3456793e-003 (single arithmetic); stored value 0.012345679104...
        "12.3456791e-003"
    );
    assert_eq!(visible(&mut e, "disp(single(pi))"), "3.1415927e+000");
    assert_eq!(visible(&mut e, "disp(single(12345))"), "12.3450000e+003");
    assert_eq!(visible(&mut e, "disp(tiny)"), "4.940656458412465e-324");
    assert_eq!(e.variables["tiny"].data()[0].to_bits(), 1);

    // Published scientific, general and default text is unchanged.
    console(&mut e, "format short e");
    assert_eq!(visible(&mut e, "disp(12345)"), "12345");
    assert_eq!(visible(&mut e, "disp(0.01234567890123456)"), "1.2346e-02");
    assert_eq!(visible(&mut e, "disp(pi)"), "3.1416e+00");
    console(&mut e, "format long e");
    assert_eq!(
        visible(&mut e, "disp(0.01234567890123456)"),
        "1.234567890123456e-02"
    );
    console(&mut e, "format short g");
    assert_eq!(visible(&mut e, "disp(pi)"), "3.1416");
    console(&mut e, "format long g");
    assert_eq!(visible(&mut e, "disp(pi)"), "3.14159265358979");
    console(&mut e, "format");
    assert_eq!(visible(&mut e, "disp(pi)"), "3.1416");
    assert_eq!(e.display, matrust::display_format::DisplayFormat::default());
}
