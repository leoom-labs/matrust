//! Replays the reference date corpus (`tests/fixtures/fn/dates-cases.txt`, run
//! with `evalc` and saved verbatim) through MatRust and compares the console
//! text byte for byte, the class and the size; for error cases, the message.
//! Block format: `@@CASE <id>|<statement>`, the console text, `@@CLASS <class>
//! <size>` (absent on error), `@@END`; an error case holds `ERR <message>`.
//! Cases marked `LONG` in the case list ran under `format long`; their ids
//! are in `LONG` below (the fixture does not carry the mark).
use matrust::Engine;

const CORPUS: &str = include_str!("fixtures/fn/dates-expected.txt");

/// Ids of the cases run under `format long` (tests/fixtures/fn/dates-cases.txt).
const LONG: &[&str] = &[
    "1", "2", "3", "4", "5", "6", "7", "8", "9", "32", "33", "34", "35", "36",
];

/// Cases that cannot pass for a reason outside this task, with the reason.
const KNOWN: &[(&str, &str)] = &[];

struct Case {
    id: String,
    statement: String,
    console: String,
    class: Option<String>,
}

fn cases() -> Vec<Case> {
    let mut cases = Vec::new();
    let mut rest = CORPUS;
    while let Some(start) = rest.find("@@CASE ") {
        rest = &rest[start + "@@CASE ".len()..];
        let (header, after) = rest.split_once('\n').expect("case header line");
        let (id, statement) = header.split_once('|').expect("id|statement");
        let (body, after) = after.split_once("@@END\n").expect("case end marker");
        let (console, class) = match body.rfind("@@CLASS ") {
            Some(at) => (
                body[..at].to_string(),
                Some(body[at + "@@CLASS ".len()..].trim_end().to_string()),
            ),
            None => (body.to_string(), None),
        };
        cases.push(Case {
            id: id.into(),
            statement: statement.into(),
            console,
            class,
        });
        rest = after;
    }
    cases
}

fn text(output: &[String]) -> String {
    output.iter().map(|entry| format!("{entry}\n")).collect()
}

/// Mismatch description, or None when the case matches.
fn check(case: &Case) -> Option<String> {
    let mut engine = Engine::default();
    if LONG.contains(&case.id.as_str()) {
        engine.execute("format long").expect("format long");
    }
    let result = engine.execute(&case.statement);
    if let Some(message) = case.console.trim_end().strip_prefix("ERR ") {
        return match result {
            Err(error) if error == message => None,
            Err(error) => Some(format!("error {error:?}, expected {message:?}")),
            Ok(_) => Some(format!("no error, expected {message:?}")),
        };
    }
    let actual = match result {
        Ok(result) => text(&result.output),
        Err(error) => return Some(format!("error {error}")),
    };
    if actual != case.console {
        return Some(format!(
            "console\n    expected {:?}\n    actual   {:?}",
            case.console, actual
        ));
    }
    let class = engine
        .execute("disp([class(x) ' ' mat2str(size(x))])")
        .map(|r| text(&r.output).trim_end().to_string())
        .unwrap_or_else(|e| format!("class/size failed: {e}"));
    let expected = case.class.as_deref().unwrap_or("");
    (class != expected).then(|| format!("class {class:?}, expected {expected:?}"))
}

#[test]
fn corpus_has_every_case() {
    let cases = cases();
    assert_eq!(cases.len(), 42);
    for (id, _) in KNOWN {
        assert!(cases.iter().any(|c| c.id == *id), "KNOWN case {id} exists");
    }
    for id in LONG {
        assert!(cases.iter().any(|c| c.id == *id), "LONG case {id} exists");
    }
    assert!(
        cases
            .iter()
            .all(|c| c.class.is_some() || c.console.starts_with("ERR ")),
        "every case has a class line or an error"
    );
}

#[test]
fn matches_reference_byte_for_byte() {
    let mut failures = Vec::new();
    let mut passed = 0;
    for case in cases() {
        if KNOWN.iter().any(|(id, _)| *id == case.id) {
            continue;
        }
        match check(&case) {
            None => passed += 1,
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
    assert_eq!(passed + KNOWN.len(), 42);
}

/// Console text of `statement`, ending in one newline (`disp` of a matrix
/// adds a blank line after it).
fn value(engine: &mut Engine, statement: &str) -> String {
    let output = engine
        .execute(statement)
        .unwrap_or_else(|e| panic!("{statement}: {e}"))
        .output;
    let text = text(&output);
    if text.is_empty() {
        text
    } else {
        format!("{}\n", text.trim_end_matches('\n'))
    }
}

/// Not reference-checked: forms beyond the corpus that follow the same rules.
#[test]
fn round_trips_and_explicit_rejections() {
    let mut e = Engine::default();
    // Every format code renders; text written by codes 0, 1, 23, 29 and 31 reads back.
    for code in 0..=31 {
        value(&mut e, &format!("s = datestr(736390.75, {code});"));
    }
    for (code, back) in [
        (0, "736390.75"),
        (31, "736390.75"),
        (1, "736390"),
        (23, "736390"),
        (29, "736390"),
    ] {
        value(&mut e, &format!("n = datenum(datestr(736390.75, {code}));"));
        assert_eq!(
            e.variables["n"].data(),
            [back.parse::<f64>().unwrap()],
            "code {code}"
        );
    }
    assert_eq!(
        value(&mut e, "disp(datestr(736390.75, 'QQ-yyyy'))"),
        "Q1-2016\n"
    );
    assert_eq!(
        value(&mut e, "disp(datestr(736390, 'ddd d m'))"),
        "Tue T M\n"
    );
    // Rounding carries into the next day.
    assert_eq!(
        value(
            &mut e,
            "disp(datestr(736390.9999999, 'yyyy-mm-dd HH:MM:SS'))"
        ),
        "2016-03-02 00:00:00\n"
    );
    // A char matrix of dates reads row by row.
    assert_eq!(
        value(
            &mut e,
            "t = datestr([736390 736391], 29); disp(datenum(t, 'yyyy-mm-dd')')"
        ),
        "      736390      736391\n"
    );
    // Multiple datevec outputs are columns.
    assert_eq!(
        value(&mut e, "[y, m, d] = datevec(736390); disp([y m d])"),
        "        2016           3           1\n"
    );
    assert_eq!(
        value(&mut e, "disp(addtodate(736390, -1, 'year'))"),
        "      736024\n"
    );
    assert_eq!(value(&mut e, "disp(eomday(1900, 2))"), "    28\n");
    assert_eq!(value(&mut e, "disp(weekday(1))"), "     7\n");
    for (statement, message) in [
        ("datenum('3:15 PM')", "cannot read"),
        ("datenum('15:00', 'HH:MM')", "without a year"),
        ("datestr(736390, 32)", "codes 0 to 31"),
        ("datestr(NaN)", "finite"),
        ("addtodate(736390, 1.5, 'month')", "integer"),
        ("addtodate(736390, 1, 'week')", "field"),
        ("c = calendar();", "current month"),
        ("etime([2016 1 1], [2016 1 1])", "6 columns"),
        ("[a, b, c] = weekday(1)", "Too many output"),
    ] {
        let error = e.execute(statement).unwrap_err();
        assert!(error.contains(message), "{statement}: {error}");
    }
}

#[test]
fn huge_finite_months_return_errors() {
    let mut e = Engine::default();
    for month in [
        "2.6127533560619555e17",
        "-2.6127533560619555e17",
        "1.7976931348623157e308",
        "-1.7976931348623157e308",
    ] {
        for statement in [
            format!("datenum(2016, {month}, 1)"),
            format!("datenum([2016 {month} 1])"),
            format!("etime([2016 {month} 1 0 0 0], [2016 1 1 0 0 0])"),
        ] {
            assert_eq!(
                e.execute(&statement).unwrap_err(),
                "Line 1, column 1: Date month must be below 1e15 in magnitude."
            );
        }
    }
}

#[test]
fn datestr_rejects_oversized_output_before_rendering_rows() {
    let mut e = Engine::default();
    // The later invalid serial must not be visited after the size is known to
    // exceed the limit. The original renderer returned a finite-date error.
    let expected = format!(
        "Line 1, column 1: {}",
        matrust::CharacterRow::checked_len(2, 32_769).unwrap_err()
    );
    for statement in [
        "datestr([1 NaN], repmat('x',1,32769))",
        "datestr(ones(100000,1), repmat('x',1,65536))",
        "datestr(1, repmat('mmmm',1,16384))",
    ] {
        assert_eq!(e.execute(statement).unwrap_err(), expected, "{statement}");
    }
    // Exact limits, including surrogate pairs and variable-width month names.
    for statement in [
        "s = datestr([1 2], repmat('x',1,32768)); shape = size(s);",
        "s = datestr([1 2], repmat('😀',1,16384)); shape = size(s);",
    ] {
        value(&mut e, statement);
        assert_eq!(e.variables["shape"].data(), [2.0, 32768.0]);
    }
}

#[test]
fn datevec_rejects_oversized_six_column_output() {
    let mut e = Engine::default();
    value(&mut e, "t = zeros(3000000,1);");
    for statement in ["datevec(t)", "[y,m,d,h,mi,s] = datevec(t)"] {
        assert_eq!(
            e.execute(statement).unwrap_err(),
            "Line 1, column 1: Matrix exceeds the 16777216-element limit"
        );
    }
}

/// Additional reference results supplied by the reviewer for fix round 1,
/// separate from the unchanged 42-case fixture (not a new local reference run).
#[test]
fn reference_date_vector_heuristic_is_only_for_datestr_six_columns() {
    let mut e = Engine::default();
    // datevec([2016 3 1]) -> 3x6 [5 7 8 0 0 0; 0 1 3 0 0 0; 0 1 1 0 0 0]
    value(
        &mut e,
        "x = datevec([2016 3 1]); expected = [5 7 8 0 0 0; 0 1 3 0 0 0; 0 1 1 0 0 0];",
    );
    assert_eq!((e.variables["x"].rows(), e.variables["x"].cols()), (3, 6));
    assert_eq!(e.variables["x"].data(), e.variables["expected"].data());
    // weekday([2016 3 1]) -> [6 2 7]
    value(&mut e, "x = weekday([2016 3 1]);");
    assert_eq!((e.variables["x"].rows(), e.variables["x"].cols()), (1, 3));
    assert_eq!(e.variables["x"].data(), [6.0, 2.0, 7.0]);
    // datestr([2016 3 1]) -> 3 rows 08-Jul-0005, 03-Jan-0000, 01-Jan-0000
    assert_eq!(
        value(&mut e, "s = datestr([2016 3 1])"),
        "\ns =\n\n08-Jul-0005\n03-Jan-0000\n01-Jan-0000\n"
    );
    value(&mut e, "shape = size(s);");
    assert_eq!(e.variables["shape"].data(), [3.0, 11.0]);
    // datestr([2016 3 1 8 5 9]) -> 01-Mar-2016 08:05:09 (corpus case 23)
    assert_eq!(
        value(&mut e, "disp(datestr([2016 3 1 8 5 9]))"),
        "01-Mar-2016 08:05:09\n"
    );
    // Even plausible six-column inputs remain serial arrays for these functions.
    value(
        &mut e,
        "x = datevec([2016 3 1 8 5 9]); w = weekday([2016 3 1 8 5 9]);",
    );
    assert_eq!((e.variables["x"].rows(), e.variables["x"].cols()), (6, 6));
    assert_eq!(e.variables["w"].data(), [6.0, 2.0, 7.0, 7.0, 4.0, 1.0]);
    assert_eq!(
        value(&mut e, "s = datestr([2016 13 1 8 5 9],29)"),
        "\ns =\n\n0005-07-08\n0000-01-13\n0000-01-01\n0000-01-08\n0000-01-05\n0000-01-09\n"
    );
}

#[test]
fn calendar_requires_scalar_year_and_month() {
    let mut e = Engine::default();
    for statement in [
        "calendar([2016 2017],2)",
        "calendar(2016,[2 3])",
        "calendar([2016;2017],2)",
        "calendar(2016,[2;3])",
        "calendar([],2)",
        "calendar(2016,[])",
    ] {
        assert_eq!(
            e.execute(statement).unwrap_err(),
            "Line 1, column 1: YEAR and MONTH must be scalars."
        );
    }
}
