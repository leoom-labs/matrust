//! datetime and duration. Expected values come from Python's datetime and
//! numpy.datetime64 (tests/fixtures/dates/datetime.json, written by
//! tool/refs/dates/datetime_ref.py) and from hand derivations in the
//! proleptic Gregorian calendar (tests/fixtures/dates/datetime_hand.json).
use matrust::Engine;
use serde_json::Value as Json;

fn text(output: &[String]) -> String {
    output.iter().map(|entry| format!("{entry}\n")).collect()
}

fn run(engine: &mut Engine, code: &str) -> String {
    match engine.execute(code) {
        Ok(r) => text(&r.output),
        Err(e) => panic!("{code}: {e}"),
    }
}

fn error(engine: &mut Engine, code: &str) -> String {
    match engine.execute(code) {
        Ok(_) => panic!("{code}: no error"),
        Err(e) => e,
    }
}

/// The char value of `code`, read back with disp.
fn char_value(engine: &mut Engine, code: &str) -> Result<String, String> {
    engine.execute(&format!("x__ = {code};"))?;
    let out = engine.execute("disp(x__)")?;
    Ok(text(&out.output).trim_end_matches('\n').to_string())
}

fn number(engine: &mut Engine, code: &str) -> Result<f64, String> {
    engine.execute(&format!("x__ = double({code});"))?;
    let m = engine.variables.get("x__").ok_or("no numeric result")?;
    if m.len() != 1 {
        return Err(format!("{} elements", m.len()));
    }
    Ok(m.data()[0])
}

fn check_fixture(source: &str) -> (usize, Vec<String>) {
    let json: Json = serde_json::from_str(source).expect("fixture JSON");
    let mut failures = Vec::new();
    let mut ran = 0;
    for case in json["cases"].as_array().expect("cases") {
        ran += 1;
        let code = case["code"].as_str().unwrap();
        let mut engine = Engine::default();
        if let Some(expected) = case["text"].as_str() {
            match char_value(&mut engine, code) {
                Ok(actual) if actual == expected => {}
                Ok(actual) => failures.push(format!("{code}: {actual:?}, expected {expected:?}")),
                Err(e) => failures.push(format!("{code}: error {e}")),
            }
        } else {
            let expected = case["num"].as_f64().unwrap();
            let tol = case["tol"].as_f64().unwrap();
            match number(&mut engine, code) {
                Ok(actual) if (actual - expected).abs() <= tol => {}
                Ok(actual) => failures.push(format!(
                    "{code}: {actual:?}, expected {expected:?} (diff {:e}, tol {tol:e})",
                    actual - expected
                )),
                Err(e) => failures.push(format!("{code}: error {e}")),
            }
        }
    }
    (ran, failures)
}

#[test]
fn python_reference_cases() {
    let (ran, failures) = check_fixture(include_str!("fixtures/dates/datetime.json"));
    assert!(ran > 150, "only {ran} cases");
    assert!(
        failures.is_empty(),
        "{} failed:\n{}",
        failures.len(),
        failures.join("\n")
    );
}

#[test]
fn hand_derived_cases() {
    let (ran, failures) = check_fixture(include_str!("fixtures/dates/datetime_hand.json"));
    assert!(ran >= 25, "only {ran} cases");
    assert!(
        failures.is_empty(),
        "{} failed:\n{}",
        failures.len(),
        failures.join("\n")
    );
}

#[test]
fn datenum_round_trip_over_years_1_to_9999() {
    // Every 7919th day plus an odd time of day; 100 microseconds is the bar.
    let mut e = Engine::default();
    run(
        &mut e,
        "n = datenum(1, 1, 1) + [0:7919:3652058, 3652058] + 0.123456789; t = datetime(n, 'ConvertFrom', 'datenum'); r = datenum(t); err = max(abs(r - n)) * 86400;",
    );
    let err = e.variables["err"].data()[0];
    assert!(err < 1e-4, "datenum round trip error {err} s");
    run(&mut e, "y = year(t([1 end]));");
    assert_eq!(e.variables["y"].data(), &[1.0, 9999.0]);
}

#[test]
fn posixtime_round_trip_1900_to_2100() {
    let mut e = Engine::default();
    run(
        &mut e,
        "p = posixtime(datetime(1900,1,1)) + (0:997:73048) * 86400 + 0.654321; t = datetime(p, 'ConvertFrom', 'posixtime'); err = max(abs(posixtime(t) - p));",
    );
    let err = e.variables["err"].data()[0];
    assert!(err < 1e-6, "posixtime round trip error {err} s");
}

#[test]
fn now_matches_the_numeric_clock() {
    let mut e = Engine::default();
    run(
        &mut e,
        "t = datetime('now'); c = class(t); s = size(t); gap = abs(datenum(t) - now);",
    );
    assert_eq!(char_value(&mut e, "c").unwrap(), "datetime");
    assert_eq!(e.variables["s"].data(), &[1.0, 1.0]);
    assert!(e.variables["gap"].data()[0] < 2.0 / 86400.0);
    run(
        &mut e,
        "u = datetime('now', 'TimeZone', 'UTC'); z = u.TimeZone; d = floor(now); k = class(now);",
    );
    assert_eq!(char_value(&mut e, "z").unwrap(), "UTC");
    assert_eq!(char_value(&mut e, "k").unwrap(), "double");
    run(
        &mut e,
        "a = datetime('today'); b = datetime('tomorrow') - a; h = hours(b); m = timeofday(a);",
    );
    assert_eq!(e.variables["h"].data(), &[24.0]);
    assert_eq!(char_value(&mut e, "char(m)").unwrap(), "00:00:00");
    // clock and date keep their numeric and text results.
    run(&mut e, "c6 = clock; dt = date;");
    assert_eq!(char_value(&mut e, "class(c6)").unwrap(), "double");
    assert_eq!(char_value(&mut e, "mat2str(size(c6))").unwrap(), "[1 6]");
    assert_eq!(char_value(&mut e, "class(dt)").unwrap(), "char");
}

#[test]
fn display_of_scalar_row_column_empty_and_nat() {
    let mut e = Engine::default();
    assert_eq!(
        run(&mut e, "t = datetime(2024, 2, 29, 12, 30, 0)"),
        "\nt =\n\n  datetime\n\n   29-Feb-2024 12:30:00\n\n"
    );
    assert_eq!(
        run(&mut e, "t = datetime(2024, 1, 30) + (0:2)"),
        "\nt =\n\n  1x3 datetime array\n\n   30-Jan-2024   31-Jan-2024   01-Feb-2024\n\n"
    );
    assert_eq!(
        run(&mut e, "t = [datetime(2024, 1, 1); NaT]"),
        "\nt =\n\n  2x1 datetime array\n\n   01-Jan-2024\n           NaT\n\n"
    );
    assert_eq!(
        run(&mut e, "t = datetime(2024, 1, 5):datetime(2024, 1, 4)"),
        "\nt =\n\n  1x0 empty datetime array\n\n"
    );
    assert_eq!(run(&mut e, "n = NaT"), "\nn =\n\n  datetime\n\n   NaT\n\n");
    assert_eq!(
        run(&mut e, "d = [duration(1, 30, 0); -duration(100, 0, 0)]"),
        "\nd =\n\n  2x1 duration array\n\n     01:30:00\n   -100:00:00\n\n"
    );
    assert_eq!(
        run(&mut e, "h = hours(1.5)"),
        "\nh =\n\n  duration\n\n   1.5 hr\n\n"
    );
    assert_eq!(
        run(&mut e, "disp(datetime(2024, 1, 1))"),
        "   01-Jan-2024\n"
    );
    run(&mut e, "format compact");
    assert_eq!(run(&mut e, "n = NaT"), "n =\n  datetime\n   NaT\n");
}

#[test]
fn class_size_and_workspace_queries() {
    let mut e = Engine::default();
    run(
        &mut e,
        "t = datetime(2024, 1, 1) + (0:3)'; d = hours([1 2 3]);",
    );
    for (code, expected) in [
        ("class(t)", "datetime"),
        ("class(d)", "duration"),
        ("mat2str(size(t))", "[4 1]"),
        ("num2str(numel(d))", "3"),
        (
            "mat2str([isdatetime(t) isduration(d) isdatetime(1) isduration(t)])",
            "[true true false false]",
        ),
        (
            "mat2str([isa(t, 'datetime') isnumeric(t) isempty(t) isrow(d)])",
            "[true false false true]",
        ),
        ("mat2str(isnat([datetime(2024,1,1) NaT]))", "[false true]"),
    ] {
        assert_eq!(char_value(&mut e, code).unwrap(), expected, "{code}");
    }
}

#[test]
fn every_format_letter_both_ways() {
    let mut e = Engine::default();
    run(&mut e, "t = datetime(2024, 3, 5, 14, 7, 9.123456789);");
    for (format, expected) in [
        ("yyyy", "2024"),
        ("yy", "24"),
        ("uuuu", "2024"),
        ("M", "3"),
        ("MM", "03"),
        ("MMM", "Mar"),
        ("MMMM", "March"),
        ("d", "5"),
        ("dd", "05"),
        ("DDD", "065"),
        ("eee", "Tue"),
        ("eeee", "Tuesday"),
        ("HH", "14"),
        ("hh", "02"),
        ("mm", "07"),
        ("ss", "09"),
        ("S", "1"),
        ("SSSSSSSSS", "123456789"),
        ("a", "PM"),
        ("yyyy-MM-dd''T''HH:mm:ss", "2024-03-05T14:07:09"),
    ] {
        let rendered = char_value(&mut e, &format!("char(t, '{format}')")).unwrap();
        assert_eq!(rendered, expected, "format {format}");
    }
    // Reading back: each format that holds a full date gives the same fields.
    for format in [
        "yyyy-MM-dd HH:mm:ss.SSSSSSSSS",
        "dd-MMM-yyyy hh:mm:ss.SSSSSSSSS a",
        "eeee, MMMM d, yyyy HH:mm:ss.SSSSSSSSS",
        "yyyy DDD HH:mm:ss.SSSSSSSSS",
        "yy/M/d H:m:s.SSSSSSSSS",
    ] {
        let code = format!(
            "char(datetime(char(t, '{format}'), 'InputFormat', '{format}'), 'yyyy-MM-dd HH:mm:ss.SSSSSSSSS')"
        );
        assert_eq!(
            char_value(&mut e, &code).unwrap(),
            "2024-03-05 14:07:09.123456789",
            "{format}"
        );
    }
    assert!(error(&mut e, "t.Format = 'yyyy-QQ';").contains("Unsupported datetime format"));
}

#[test]
fn duration_formats() {
    let mut e = Engine::default();
    for (code, expected) in [
        ("char(duration(1, 30, 0))", "01:30:00"),
        ("char(duration(1, 2, 3.25), 'hh:mm:ss.SS')", "01:02:03.25"),
        ("char(duration(26, 2, 3), 'dd:hh:mm:ss')", "01:02:02:03"),
        ("char(duration(1, 2, 3), 'mm:ss')", "62:03"),
        ("char(duration(1, 2, 3), 'hh:mm')", "01:02"),
        ("char(duration(123, 0, 0))", "123:00:00"),
        ("char(-duration(0, 1, 30))", "-00:01:30"),
        ("char(seconds(90))", "90 sec"),
        ("char(minutes(1.5))", "1.5 min"),
        ("char(hours(1.5))", "1.5 hr"),
        ("char(days(1))", "1 day"),
        ("char(days(2))", "2 days"),
        ("char(years(2))", "2 yrs"),
        ("char(milliseconds(1500))", "1.5 sec"),
        ("char(seconds(NaN))", "NaN sec"),
        ("char(duration(NaN, 0, 0))", "NaN"),
        (
            "char(duration('12:15:30.25', 'Format', 'hh:mm:ss.SS'))",
            "12:15:30.25",
        ),
        ("char(duration('01:02:03:04'))", "26:03:04"),
        ("char(duration('05:30', 'InputFormat', 'mm:ss'))", "05:30"),
        (
            "char(duration('05:30', 'InputFormat', 'mm:ss', 'Format', 'hh:mm:ss'))",
            "00:05:30",
        ),
        ("char(duration([1 30 0; 2 0 0]))", "01:30:00\n02:00:00"),
    ] {
        assert_eq!(char_value(&mut e, code).unwrap(), expected, "{code}");
    }
    assert!(error(&mut e, "d = duration('1:2:x');").contains("element 1"));
    run(
        &mut e,
        "d = duration({'01:00:00', 'bad'}, 'InputFormat', 'hh:mm:ss');",
    );
    assert_eq!(
        char_value(&mut e, "mat2str(isnan(seconds(d)))").unwrap(),
        "[false true]"
    );
}

#[test]
fn unit_functions_convert_both_ways() {
    let mut e = Engine::default();
    for (code, expected) in [
        ("seconds(minutes(2))", 120.0),
        ("minutes(hours(1))", 60.0),
        ("hours(days(1.5))", 36.0),
        ("days(hours(36))", 1.5),
        ("days(years(1))", 365.2425),
        ("milliseconds(seconds(2))", 2000.0),
        ("seconds(duration(1, 2, 3, 500))", 3723.5),
    ] {
        assert_eq!(number(&mut e, code).unwrap(), expected, "{code}");
    }
}

#[test]
fn arithmetic_and_days_as_doubles() {
    let mut e = Engine::default();
    for (code, expected) in [
        (
            "char(datetime(2024, 3, 1) - datetime(2024, 2, 1))",
            "696:00:00",
        ),
        (
            "char(datetime(2024, 1, 1) + hours(36), 'yyyy-MM-dd HH:mm')",
            "2024-01-02 12:00",
        ),
        (
            "char(hours(36) + datetime(2024, 1, 1), 'yyyy-MM-dd HH:mm')",
            "2024-01-02 12:00",
        ),
        (
            "char(datetime(2024, 1, 1) - 0.25, 'yyyy-MM-dd HH:mm')",
            "2023-12-31 18:00",
        ),
        ("char(1 + datetime(2024, 1, 1), 'yyyy-MM-dd')", "2024-01-02"),
        ("char(duration(1, 0, 0) + 1)", "25:00:00"),
        ("char(2 - duration(1, 0, 0))", "47:00:00"),
        ("char(duration(1, 0, 0) * 3)", "03:00:00"),
        ("char(2 * duration(1, 0, 0))", "02:00:00"),
        ("char(duration(3, 0, 0) / 2)", "01:30:00"),
        ("char(duration(3, 0, 0) ./ [1 3])", "03:00:00\n01:00:00"),
        ("char(abs(-duration(1, 0, 0)))", "01:00:00"),
        ("char(+duration(1, 0, 0))", "01:00:00"),
        (
            "char(datetime(2024, 1, 31) + days([0 1]), 'MM-dd')",
            "01-31\n02-01",
        ),
        (
            "char(duration([1 2; 3 4], 0, 0) * [0 1; 1 0])",
            "02:00:00\n04:00:00\n01:00:00\n03:00:00",
        ),
    ] {
        assert_eq!(char_value(&mut e, code).unwrap(), expected, "{code}");
    }
    assert_eq!(
        number(&mut e, "duration(3, 0, 0) ./ duration(1, 30, 0)").unwrap(),
        2.0
    );
    assert!(
        error(&mut e, "x = datetime(2024,1,1) + datetime(2024,1,1);").contains("not supported")
    );
    assert!(error(&mut e, "x = 1 - datetime(2024,1,1);").contains("not supported"));
    assert!(error(&mut e, "x = duration(1,0,0) * duration(1,0,0);").contains("multiplied"));
    assert!(error(&mut e, "x = hours([1 2]) * hours([1; 2]);").contains("multiplied"));
    assert!(error(&mut e, "x = -datetime(2024,1,1);").contains("Unary minus"));
    assert!(error(&mut e, "x = double(datetime(2024,1,1));").contains("datenum"));
    assert!(error(&mut e, "x = double(hours(1));").contains("seconds"));
    assert!(
        error(&mut e, "x = num2str(datetime(2024,1,1));")
            .ends_with("num2str: datetime input is not supported, use char or string")
    );
    // NaT and NaN propagate through arithmetic.
    assert_eq!(char_value(&mut e, "char(NaT + 1)").unwrap(), "NaT");
    assert_eq!(
        char_value(&mut e, "char(datetime(2024,1,1) + hours(NaN))").unwrap(),
        "NaT"
    );
    assert!(
        number(&mut e, "seconds(NaT - datetime(2024,1,1))")
            .unwrap()
            .is_nan()
    );
}

#[test]
fn comparisons_with_nat_nan_and_text() {
    let mut e = Engine::default();
    run(
        &mut e,
        "t = [datetime(2024, 1, 1), NaT, datetime(2024, 1, 3)]; u = datetime(2024, 1, 2);",
    );
    for (code, expected) in [
        ("mat2str(t < u)", "[true false false]"),
        ("mat2str(t > u)", "[false false true]"),
        ("mat2str(t == t)", "[true false true]"),
        ("mat2str(t ~= t)", "[false true false]"),
        ("mat2str(t >= '02-Jan-2024')", "[false false true]"),
        ("mat2str('2024-01-01' == t)", "[true false false]"),
        ("mat2str(hours([1 NaN 3]) < hours(2))", "[true false false]"),
        ("mat2str(hours(24) == 1)", "true"),
        ("mat2str(hours([1 NaN]) ~= hours(1))", "[false true]"),
    ] {
        assert_eq!(char_value(&mut e, code).unwrap(), expected, "{code}");
    }
    assert!(
        error(
            &mut e,
            "x = datetime(2024,1,1,'TimeZone','UTC') < datetime(2024,1,1);"
        )
        .contains("time zone")
    );
    assert!(error(&mut e, "x = datetime(2024,1,1) < 5;").contains("not defined"));
}

#[test]
fn sort_min_max_mean_unique_with_missing_values() {
    let mut e = Engine::default();
    run(
        &mut e,
        "t = [datetime(2024,1,3), NaT, datetime(2024,1,1), datetime(2024,1,2)]; f = 'MM-dd';",
    );
    for (code, expected) in [
        ("strjoin(cellstr(sort(t), f), ' ')", "01-01 01-02 01-03 NaT"),
        (
            "strjoin(cellstr(sort(t, 'descend'), f), ' ')",
            "NaT 01-03 01-02 01-01",
        ),
        ("char(min(t), f)", "01-01"),
        ("char(max(t), f)", "01-03"),
        ("char(min(t, [], 'includenat'))", "NaT"),
        ("char(max(t, [], 2, 'includenat'))", "NaT"),
        ("char(mean(t))", "NaT"),
        ("char(mean(t, 'omitnat'), 'MM-dd HH:mm')", "01-02 00:00"),
        (
            "char(mean([datetime(2024,1,1) datetime(2024,1,2)]), 'MM-dd HH:mm')",
            "01-01 12:00",
        ),
        (
            "strjoin(cellstr(unique([t t]), f), ' ')",
            "01-01 01-02 01-03 NaT NaT",
        ),
        (
            "strjoin(cellstr(min(t, datetime(2024,1,2)), f), ' ')",
            "01-02 01-02 01-01 01-02",
        ),
        (
            "strjoin(cellstr(sort(hours([3 NaN 1]))), ',')",
            "1 hr,3 hr,NaN hr",
        ),
        ("char(max(hours([3 NaN 1])))", "3 hr"),
        ("char(mean(hours([3 NaN 1])))", "NaN hr"),
        ("char(mean(hours([3 NaN 1]), 'omitnan'))", "2 hr"),
        ("char(sum(hours([1 2 3])))", "6 hr"),
        (
            "char(diff([datetime(2024,1,1) datetime(2024,1,3) datetime(2024,1,6)]))",
            "48:00:00\n72:00:00",
        ),
        ("strjoin(cellstr(diff(hours([1 4 9]))), ',')", "3 hr,5 hr"),
        ("mat2str([issorted(sort(t)) issorted(t)])", "[true false]"),
    ] {
        assert_eq!(char_value(&mut e, code).unwrap(), expected, "{code}");
    }
    run(
        &mut e,
        "[s, i] = sort(t); [m, k] = max(t); [u, ia, ic] = unique([t(3) t(3) t(1)]);",
    );
    assert_eq!(e.variables["i"].data(), &[3.0, 4.0, 1.0, 2.0]);
    assert_eq!(e.variables["k"].data(), &[1.0]);
    assert_eq!(e.variables["ia"].data(), &[1.0, 3.0]);
    assert_eq!(e.variables["ic"].data(), &[1.0, 1.0, 2.0]);
    run(
        &mut e,
        "a = isequal(NaT, NaT); b = isequaln(NaT, NaT); c = isequal(t(1), datetime(2024,1,3)); d = isequal(hours(1), minutes(60));",
    );
    for (name, expected) in [("a", false), ("b", true), ("c", true), ("d", true)] {
        assert_eq!(
            char_value(&mut e, &format!("mat2str({name})")).unwrap(),
            expected.to_string(),
            "{name}"
        );
    }
}

#[test]
fn colon_ranges() {
    let mut e = Engine::default();
    for (code, expected) in [
        (
            "strjoin(cellstr(datetime(2024,1,1):datetime(2024,1,3), 'dd'), ' ')",
            "01 02 03",
        ),
        (
            "strjoin(cellstr(datetime(2024,1,1):hours(12):datetime(2024,1,2), 'dd HH'), ' ')",
            "01 00 01 12 02 00",
        ),
        (
            "strjoin(cellstr(datetime(2024,1,1):hours(10):datetime(2024,1,2), 'dd HH'), ' ')",
            "01 00 01 10 01 20",
        ),
        (
            "strjoin(cellstr(datetime(2024,1,3):-1:datetime(2024,1,1), 'dd'), ' ')",
            "03 02 01",
        ),
        (
            "strjoin(cellstr(datetime(2024,1,1):0.5:datetime(2024,1,2), 'dd HH'), ' ')",
            "01 00 01 12 02 00",
        ),
        (
            "mat2str(size(datetime(2024,1,2):datetime(2024,1,1)))",
            "[1 0]",
        ),
        (
            "mat2str(size(datetime(2024,1,1):-1:datetime(2024,1,2)))",
            "[1 0]",
        ),
        (
            "strjoin(cellstr(hours(1):minutes(30):hours(2)), ' ')",
            "1 hr 1.5 hr 2 hr",
        ),
    ] {
        assert_eq!(char_value(&mut e, code).unwrap(), expected, "{code}");
    }
    // The endpoint is included only when a whole number of steps lands on it.
    run(
        &mut e,
        "a = datetime(2024,1,1); r = a:hours(10):(a + hours(30)); n1 = numel(r); r = a:hours(10):(a + hours(30) - seconds(1e-9)); n2 = numel(r);",
    );
    assert_eq!(e.variables["n1"].data(), &[4.0]);
    assert_eq!(e.variables["n2"].data(), &[3.0]);
}

#[test]
fn indexing_deletion_and_assignment_from_text() {
    let mut e = Engine::default();
    run(
        &mut e,
        "t = datetime(2024, 1, 1) + (0:3); t.Format = 'yyyy-MM-dd';",
    );
    for (code, expected) in [
        ("char(t(2))", "2024-01-02"),
        ("strjoin(cellstr(t([4 1])), ' ')", "2024-01-04 2024-01-01"),
        ("char(t(end))", "2024-01-04"),
        (
            "strjoin(cellstr(t(t > '2024-01-02')), ' ')",
            "2024-01-03 2024-01-04",
        ),
    ] {
        assert_eq!(char_value(&mut e, code).unwrap(), expected, "{code}");
    }
    run(
        &mut e,
        "t(2) = '2030-05-06'; t(1) = []; t(end + 2) = datetime(2031, 1, 1);",
    );
    assert_eq!(
        char_value(&mut e, "strjoin(cellstr(t), ' ')").unwrap(),
        "2030-05-06 2024-01-03 2024-01-04 NaT 2031-01-01"
    );
    run(&mut e, "t([1 2]) = {'2000-01-01', '2000-01-02'};");
    assert_eq!(char_value(&mut e, "char(t(2))").unwrap(), "2000-01-02");
    run(
        &mut e,
        "n(3) = datetime(2024, 1, 1); d = hours([1 2]); d(4) = 1;",
    );
    assert_eq!(
        char_value(&mut e, "strjoin(cellstr(n), ' ')").unwrap(),
        "NaT NaT 01-Jan-2024"
    );
    assert_eq!(
        char_value(&mut e, "strjoin(cellstr(d), ',')").unwrap(),
        "1 hr,2 hr,0 hr,24 hr"
    );
    run(
        &mut e,
        "m = reshape(datetime(2024,1,1) + (0:5), 2, 3); m(:, 2) = [];",
    );
    assert_eq!(char_value(&mut e, "mat2str(size(m))").unwrap(), "[2 2]");
    assert!(error(&mut e, "t(1) = 5;").contains("Cannot assign"));
    assert!(error(&mut e, "x = [1 2]; x(1) = datetime(2024,1,1);").contains("Cannot assign"));
    assert!(error(&mut e, "t(2) = 'not a date';").contains("Could not convert"));
}

#[test]
fn concatenation_rules() {
    let mut e = Engine::default();
    for (code, expected) in [
        (
            "[datetime(2024,1,1,'Format','yyyy'), datetime(2025,1,1)].Format",
            "yyyy",
        ),
        (
            "char([datetime(2024,1,1,'Format','yyyy-MM-dd'), '2024-02-03'])",
            "2024-01-01\n2024-02-03",
        ),
        (
            "char([datetime(2024,1,1); datetime(2024,1,2)], 'dd')",
            "01\n02",
        ),
        (
            "[datetime(2024,1,1,'TimeZone','UTC'), datetime(2024,1,2,'TimeZone','UTC')].TimeZone",
            "UTC",
        ),
        ("char([hours(1), 0.5])", "1 hr \n12 hr"),
        ("char([duration(1,0,0); 1])", "01:00:00\n24:00:00"),
        ("char([[] datetime(2024,1,1)], 'yyyy')", "2024"),
        (
            "strjoin(cellstr(horzcat(hours(1), hours(2))), ' ')",
            "1 hr 2 hr",
        ),
        ("mat2str(size(vertcat(hours(1), hours(2))))", "[2 1]"),
    ] {
        assert_eq!(char_value(&mut e, code).unwrap(), expected, "{code}");
    }
    assert!(
        error(
            &mut e,
            "x = [datetime(2024,1,1,'TimeZone','UTC'), datetime(2024,1,1)];"
        )
        .contains("with and without a time zone")
    );
    assert!(error(&mut e, "x = [datetime(2024,1,1), 5];").contains("Cannot concatenate"));
    assert!(error(&mut e, "x = [hours(1), 'a'];").contains("Cannot concatenate"));
}

#[test]
fn properties_get_set_with_carry_and_copy_on_write() {
    let mut e = Engine::default();
    run(&mut e, "t = datetime(2024, 1, 31, 10, 20, 30.5);");
    for (code, expected) in [
        ("t.Year", 2024.0),
        ("t.Month", 1.0),
        ("t.Day", 31.0),
        ("t.Hour", 10.0),
        ("t.Minute", 20.0),
        ("t.Second", 30.5),
        ("year(t)", 2024.0),
        ("month(t)", 1.0),
        ("day(t)", 31.0),
        ("hour(t)", 10.0),
        ("minute(t)", 20.0),
        ("second(t)", 30.5),
        ("day(t, 'dayofweek')", 4.0),
    ] {
        assert_eq!(number(&mut e, code).unwrap(), expected, "{code}");
    }
    run(
        &mut e,
        "b = t; t.Month = 2; t.Format = 'yyyy-MM-dd HH:mm:ss.S';",
    );
    assert_eq!(
        char_value(&mut e, "char(t)").unwrap(),
        "2024-03-02 10:20:30.5"
    );
    assert_eq!(
        char_value(&mut e, "char(b)").unwrap(),
        "31-Jan-2024 10:20:31"
    );
    assert_eq!(
        char_value(&mut e, "b.Format").unwrap(),
        "dd-MMM-yyyy HH:mm:ss"
    );
    run(&mut e, "t.Second = -0.5; t.Hour = 24; t.Day = 0;");
    assert_eq!(
        char_value(&mut e, "char(t)").unwrap(),
        "2024-02-29 00:19:59.5"
    );
    run(
        &mut e,
        "v = datetime(2024, 1, 1) + (0:2); w = v; v.Year = [2000 2001 2002]; v.Day(2) = 15;",
    );
    assert_eq!(
        char_value(&mut e, "strjoin(cellstr(v, 'yyyy-MM-dd'), ' ')").unwrap(),
        "2000-01-01 2001-01-15 2002-01-03"
    );
    assert_eq!(
        char_value(&mut e, "strjoin(cellstr(w, 'yyyy-MM-dd'), ' ')").unwrap(),
        "2024-01-01 2024-01-02 2024-01-03"
    );
    run(
        &mut e,
        "z = datetime(2024, 1, 1, 12, 0, 0); z.TimeZone = 'UTC'; zz = z; zz.TimeZone = '';",
    );
    assert_eq!(char_value(&mut e, "z.TimeZone").unwrap(), "UTC");
    assert_eq!(
        char_value(&mut e, "char(z)").unwrap(),
        "01-Jan-2024 12:00:00"
    );
    assert_eq!(
        char_value(&mut e, "char(zz)").unwrap(),
        "01-Jan-2024 12:00:00"
    );
    assert!(error(&mut e, "z.TimeZone = 'Europe/Paris';").contains("not supported yet"));
    run(&mut e, "d = hours(1); d2 = d; d.Format = 'm';");
    assert_eq!(char_value(&mut e, "char(d)").unwrap(), "60 min");
    assert_eq!(char_value(&mut e, "char(d2)").unwrap(), "1 hr");
    assert!(error(&mut e, "q = t.Nonexistent;").contains("Unrecognized property"));
    run(
        &mut e,
        "[y, m, dd] = ymd(datetime(2024, 3, 15)); [h, mi, s] = hms(datetime(2024, 1, 1, 13, 5, 7.25));",
    );
    assert_eq!(
        [&e.variables["y"], &e.variables["m"], &e.variables["dd"]].map(|v| v.data()[0]),
        [2024.0, 3.0, 15.0]
    );
    assert_eq!(
        [&e.variables["h"], &e.variables["mi"], &e.variables["s"]].map(|v| v.data()[0]),
        [13.0, 5.0, 7.25]
    );
    run(
        &mut e,
        "dv = datevec(datetime(2024, 3, 15, 10, 20, 30)); [yy, mo] = datevec(datetime(2024, 3, 15));",
    );
    assert_eq!(
        e.variables["dv"].data(),
        &[2024.0, 3.0, 15.0, 10.0, 20.0, 30.0]
    );
    assert_eq!(e.variables["mo"].data(), &[3.0]);
    assert_eq!(
        char_value(&mut e, "datestr(datetime(2024, 3, 15, 10, 20, 30))").unwrap(),
        "15-Mar-2024 10:20:30"
    );
    assert_eq!(
        char_value(&mut e, "strjoin(month(datetime(2024, 3, 1), 'name'), '')").unwrap(),
        "March"
    );
    assert_eq!(
        char_value(
            &mut e,
            "strjoin(day(datetime(2024, 3, 1), 'shortname'), '')"
        )
        .unwrap(),
        "Fri"
    );
}

#[test]
fn construction_forms() {
    let mut e = Engine::default();
    for (code, expected) in [
        ("char(datetime([2024 2025], 1, 1), 'yyyy')", "2024\n2025"),
        (
            "char(datetime(2024, 1, 1, 0, 0, 1, 500), 'HH:mm:ss.SSS')",
            "00:00:01.500",
        ),
        (
            "char(datetime([2024 1 2; 2025 3 4]), 'yyyy-MM-dd')",
            "2024-01-02\n2025-03-04",
        ),
        (
            "char(datetime([2024 1 2 3 4 5]), 'yyyy-MM-dd HH:mm:ss')",
            "2024-01-02 03:04:05",
        ),
        ("char(datetime(2024, NaN, 1))", "NaT"),
        ("char(datetime(Inf, 1, 1))", "NaT"),
        ("char(datetime(\"2024-05-06\"), 'yyyy-MM-dd')", "2024-05-06"),
        (
            "char(datetime(['2024-05-06'; '2024-05-07']), 'dd')",
            "06\n07",
        ),
        ("char(datetime(string({'2024-05-06'})), 'dd')", "06"),
        (
            "char(datetime('05-May-2024 10:11:12'), 'HH:mm:ss')",
            "10:11:12",
        ),
        ("datetime('2024-01-01').Format", "dd-MMM-yyyy"),
        (
            "datetime('2024-01-01 10:00:00').Format",
            "dd-MMM-yyyy HH:mm:ss",
        ),
        // The InputFormat only reads the text (fix round 2).
        (
            "datetime('2024-01-01', 'InputFormat', 'yyyy-MM-dd').Format",
            "dd-MMM-yyyy",
        ),
        ("datetime(2024, 1, 1).Format", "dd-MMM-yyyy"),
        (
            "char(datetime(2024, 1, 1) + hours(1))",
            "01-Jan-2024 01:00:00",
        ),
        ("char(NaT(1, 2))", "NaT\nNaT"),
        ("mat2str(size(NaT(2)))", "[2 2]"),
        (
            "char(datetime('2024.05.06', 'Format', 'yyyy.MM.dd'))",
            "2024.05.06",
        ),
    ] {
        assert_eq!(char_value(&mut e, code).unwrap(), expected, "{code}");
    }
    assert!(error(&mut e, "x = datetime('garbage');").contains("'garbage' (element 1)"));
    run(&mut e, "[r, c] = size(datetime(2024, 1, 1) + (0:2)');");
    assert_eq!(
        (e.variables["r"].data()[0], e.variables["c"].data()[0]),
        (3.0, 1.0)
    );
    assert!(error(&mut e, "x = datetime(2024, 1e15, 1);").contains("1e15"));
    assert!(error(&mut e, "x = datetime(2024, 1.5, 1);").contains("integers"));
    assert!(
        error(&mut e, "x = datetime(5, 'ConvertFrom', 'epochtime');").contains("not supported yet")
    );
    assert!(
        error(
            &mut e,
            "x = datetime('now', 'TimeZone', 'America/New_York');"
        )
        .contains("not supported yet")
    );
    assert!(
        error(&mut e, "x = reshape(datetime(2024,1,1) + (0:7), 2, 2, 2);")
            .contains("N-D datetime arrays are not supported yet")
    );
}

#[test]
fn text_conversions() {
    let mut e = Engine::default();
    run(
        &mut e,
        "t = [datetime(2024, 1, 1), NaT]; s = string(t); c = cellstr(t); k = char(t, 'yyyy');",
    );
    assert_eq!(char_value(&mut e, "class(s)").unwrap(), "string");
    assert_eq!(char_value(&mut e, "char(s(1))").unwrap(), "01-Jan-2024");
    assert_eq!(char_value(&mut e, "c{2}").unwrap(), "NaT");
    assert_eq!(char_value(&mut e, "k").unwrap(), "2024\nNaT ");
}

#[test]
fn saving_is_refused_before_writing() {
    let dir = std::env::temp_dir().join(format!("matrust_dt_{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let mut e = Engine::default();
    run(&mut e, "t = datetime(2024, 1, 1); d = hours(1); x = 5;");
    for (vars, flag) in [
        ("'t'", ""),
        ("'d'", ""),
        ("'t'", ", '-v7.3'"),
        ("'d'", ", '-v7.3'"),
    ] {
        let path = dir.join("dt.mat");
        let _ = std::fs::remove_file(&path);
        let code = format!(
            "save('{}', {vars}{flag})",
            path.display().to_string().replace('\\', "/")
        );
        let message = error(&mut e, &code);
        assert!(
            message.contains("datetime/duration variables cannot be saved yet"),
            "{code}: {message}"
        );
        assert!(!path.exists(), "{code} wrote a file");
    }
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn existing_date_functions_keep_numeric_behaviour() {
    let mut e = Engine::default();
    assert_eq!(number(&mut e, "datenum(2016, 3, 1)").unwrap(), 736390.0);
    assert_eq!(
        char_value(&mut e, "datestr(736390.75, 'yyyy-mm-dd HH:MM:SS')").unwrap(),
        "2016-03-01 18:00:00"
    );
    assert_eq!(number(&mut e, "weekday(736390)").unwrap(), 3.0);
    run(&mut e, "v = datevec(736390.5);");
    assert_eq!(e.variables["v"].data(), &[2016.0, 3.0, 1.0, 12.0, 0.0, 0.0]);
    assert!(number(&mut e, "floor(now)").unwrap() > 7e5);
}

// ---- fix round 1 ----

/// The size of `code` as "RxC".
fn dims(engine: &mut Engine, code: &str) -> String {
    char_value(engine, &format!("mat2str(size({code}))")).unwrap()
}

#[test]
fn diff_is_iterative_and_validates_the_order_like_numeric_diff() {
    let mut e = Engine::default();
    // Cubes: orders 1..4 are [1 7 19 37 61], [6 12 18 24], [6 6 6], [0 0].
    run(&mut e, "h = hours([0 1 8 27 64 125]);");
    for (code, expected) in [
        ("diff(h)", "[1 7 19 37 61]"),
        ("diff(h, 2)", "[6 12 18 24]"),
        ("diff(h, 3)", "[6 6 6]"),
        ("diff(h, 4)", "[0 0]"),
        ("diff(h, 5)", "0"),
        ("diff(h', 3)", "[6;6;6]"),
        ("diff(hours([1 4 9; 2 6 12]), 2, 2)", "[2;2]"),
        ("diff(hours([1 4 9; 2 6 12]), 1, 1)", "[1 2 3]"),
        ("diff(h, [], 2)", "[1 7 19 37 61]"),
        ("diff(h, 0)", "[0 1 8 27 64 125]"),
    ] {
        let hours = if expected.starts_with('[') || expected == "0" {
            format!("mat2str(hours({code}))")
        } else {
            code.to_string()
        };
        assert_eq!(char_value(&mut e, &hours).unwrap(), expected, "{code}");
    }
    // The same orders on datetimes with gaps of 1, 2, 4 and 8 days.
    run(
        &mut e,
        "t = datetime(2024,1,1) + days([0 1 3 7 15]); \
         x = hours(diff(t)); y = hours(diff(t, 2)); z = hours(diff(t, 3));",
    );
    assert_eq!(e.variables["x"].data(), &[24.0, 48.0, 96.0, 192.0]);
    assert_eq!(e.variables["y"].data(), &[24.0, 48.0, 96.0]);
    assert_eq!(e.variables["z"].data(), &[24.0, 48.0]);
    // An order of at least the length gives the empty result numeric diff
    // gives; a huge order costs nothing and does not recurse.
    for (code, expected) in [
        ("diff(h, 6)", "[1 0]"),
        ("diff(h, 100000)", "[1 0]"),
        ("diff(h', 100000)", "[0 1]"),
        ("diff(h, 16777216)", "[1 0]"),
        ("diff(t, 100000)", "[1 0]"),
        ("diff(t', 100000)", "[0 1]"),
        ("diff(hours([1 4 9; 2 6 12]), 5, 2)", "[2 0]"),
        ("diff(hours([1 4 9; 2 6 12]), 5, 1)", "[0 3]"),
        ("diff(hours(zeros(3, 0)), 2, 1)", "[1 0]"),
    ] {
        assert_eq!(dims(&mut e, code), expected, "{code}");
    }
    assert_eq!(
        numeric_numel(&mut e, "diff(t, 100000)"),
        0.0,
        "empty result has no elements"
    );
    // The order is validated before any work is done.
    for code in [
        "diff(h, -1)",
        "diff(h, 1.5)",
        "diff(h, NaN)",
        "diff(h, 50331649)",
        "diff(h, 'x')",
    ] {
        let message = error(&mut e, &format!("r = {code};"));
        assert!(message.contains("diff:"), "{code}: {message}");
    }
    // Numeric diff's work bound applies: 1e6 differences of 3e6 elements.
    let message = error(&mut e, "r = diff(hours(1:3000000), 1000000);");
    assert!(message.contains("work limit"), "{message}");
}

fn numeric_numel(engine: &mut Engine, code: &str) -> f64 {
    number(engine, &format!("numel({code})")).unwrap()
}

#[test]
fn nested_datetime_and_duration_compare_equal_in_cells_and_structs() {
    let mut e = Engine::default();
    run(
        &mut e,
        "u = [datetime(2024,1,1) datetime(2024,6,1)]; n = [datetime(2024,1,1) NaT]; \
         z = datetime(2024,1,1,'TimeZone','UTC'); \
         p = [hours(1) hours(2)]; q = [hours(1) hours(NaN)]; \
         s1.a = n; s1.b = q; s2 = s1; \
         c1 = {u, {p}}; c2 = {u, {p}};",
    );
    for (code, expected) in [
        // without missing values both functions agree
        ("isequal({u}, {u})", true),
        ("isequaln({u}, {u})", true),
        ("isequal({p}, {p})", true),
        ("isequaln({p}, {p})", true),
        ("isequal(c1, c2)", true),
        ("isequaln(c1, c2)", true),
        (
            "isequal({u}, {[datetime(2024,1,1) datetime(2024,6,2)]})",
            false,
        ),
        (
            "isequaln({u}, {[datetime(2024,1,1) datetime(2024,6,2)]})",
            false,
        ),
        ("isequaln({p}, {[hours(1) hours(3)]})", false),
        ("isequal({u}, {u'})", false),
        ("isequaln({u}, {u'})", false),
        // NaT and NaN: isequal false, isequaln true
        ("isequal({n}, {n})", false),
        ("isequaln({n}, {n})", true),
        ("isequal({q}, {q})", false),
        ("isequaln({q}, {q})", true),
        ("isequal(s1, s2)", false),
        ("isequaln(s1, s2)", true),
        ("isequaln({{n}}, {{n}})", true),
        (
            "isequaln({n}, {[datetime(2024,1,1) datetime(2024,1,1)]})",
            false,
        ),
        // a time zone label is part of the value
        ("isequaln({z}, {z})", true),
        ("isequaln({z}, {datetime(2024,1,1)})", false),
        ("isequal({z}, {datetime(2024,1,1)})", false),
        // different classes
        ("isequaln({u}, {p})", false),
        ("isequaln({datetime(2024,1,1)}, {1})", false),
        // the direct forms keep working
        ("isequaln(n, n)", true),
        ("isequal(n, n)", false),
    ] {
        assert_eq!(
            char_value(&mut e, &format!("mat2str({code})")).unwrap(),
            expected.to_string(),
            "{code}"
        );
    }
}

/// Runs `setup` then `code`; returns how long the run took when `code` runs
/// to its end and when a cancellation arrives a tenth of that time in.
fn cancel_speedup(setup: &str, code: &str) -> (f64, f64) {
    use std::sync::atomic::Ordering;
    let mut e = Engine::default();
    run(&mut e, setup);
    let start = std::time::Instant::now();
    run(&mut e, code);
    let full = start.elapsed();
    let stop = e.cancellation();
    let delay = full / 10;
    let watcher = std::thread::spawn({
        let stop = stop.clone();
        move || {
            std::thread::sleep(delay);
            stop.store(true, Ordering::Relaxed);
        }
    });
    let start = std::time::Instant::now();
    let message = error(&mut e, code);
    let cancelled = start.elapsed();
    watcher.join().unwrap();
    stop.store(false, Ordering::Relaxed);
    assert!(message.contains("cancelled"), "{code}: {message}");
    eprintln!(
        "{code}: full {:.3} s, cancelled {:.3} s",
        full.as_secs_f64(),
        cancelled.as_secs_f64()
    );
    // The engine is usable again afterwards.
    run(&mut e, "ok__ = 1;");
    (full.as_secs_f64(), cancelled.as_secs_f64())
}

#[test]
fn one_long_lane_can_be_cancelled_mid_sort() {
    // One 3M-element column: a single lane, so only per-element polling helps.
    let (full, cancelled) = cancel_speedup(
        "t = datetime(2024,1,1) + days(mod((1:3000000)' * 7919, 1000003));",
        "s = sort(t);",
    );
    // Too fast to measure on this machine: the cancelled error is the check.
    if full > 0.3 {
        assert!(
            cancelled < 0.7 * full,
            "cancelled run took {cancelled:.3} s of a {full:.3} s sort"
        );
    }
}

#[test]
fn repeated_differences_of_a_long_lane_can_be_cancelled() {
    // 200k elements, order 1000: 2e8 subtractions, inside the work bound.
    let (full, cancelled) = cancel_speedup(
        "h = hours(mod((1:200000)' * 7919, 1009));",
        "d = diff(h, 1000);",
    );
    if full > 0.3 {
        assert!(
            cancelled < 0.7 * full,
            "cancelled run took {cancelled:.3} s of a {full:.3} s diff"
        );
    }
}

#[test]
fn issorted_honours_the_direction() {
    let mut e = Engine::default();
    run(
        &mut e,
        "up = datetime(2024,1,1) + days([0 1 1 5]); down = fliplr(up); \
         upn = [up NaT]; downn = [NaT down]; mixed = up([2 1 3 4]); \
         hup = hours([1 2 2 9]); hdown = fliplr(hup); hupn = [hup NaN]; hdownn = [NaN hdown];",
    );
    for (code, expected) in [
        (
            "[issorted(up) issorted(up, 'ascend') issorted(up, 'descend')]",
            "[true true false]",
        ),
        (
            "[issorted(down) issorted(down, 'ascend') issorted(down, 'descend')]",
            "[false false true]",
        ),
        // NaT is last when ascending, first when descending
        (
            "[issorted(upn) issorted(upn, 'ascend') issorted(upn, 'descend')]",
            "[true true false]",
        ),
        (
            "[issorted(downn) issorted(downn, 'ascend') issorted(downn, 'descend')]",
            "[false false true]",
        ),
        (
            "[issorted(mixed) issorted(mixed, 'descend')]",
            "[false false]",
        ),
        (
            "[issorted(hup) issorted(hup, 'ascend') issorted(hup, 'descend')]",
            "[true true false]",
        ),
        (
            "[issorted(hdown) issorted(hdown, 'ascend') issorted(hdown, 'descend')]",
            "[false false true]",
        ),
        ("[issorted(hupn) issorted(hupn, 'descend')]", "[true false]"),
        (
            "[issorted(hdownn) issorted(hdownn, 'ascend') issorted(hdownn, 'descend')]",
            "[false false true]",
        ),
        ("issorted(up, 'DESCEND')", "false"),
        ("issorted(down, 'Descend')", "true"),
        // one element, equal elements and empties are sorted both ways
        (
            "[issorted(up(1), 'descend') issorted(up(1), 'ascend')]",
            "[true true]",
        ),
        ("issorted(repmat(up(1), 1, 3), 'descend')", "true"),
        ("issorted(NaT(1, 3), 'descend')", "true"),
        (
            "[issorted(up([]), 'descend') issorted(hours([]), 'ascend')]",
            "[true true]",
        ),
        // a column is checked down its length
        ("issorted(down', 'descend')", "true"),
    ] {
        assert_eq!(
            char_value(&mut e, &format!("mat2str({code})")).unwrap(),
            expected,
            "{code}"
        );
    }
    for code in [
        "issorted(up, 'monotonic')",
        "issorted(up, 'descending')",
        "issorted(up, 1)",
        "issorted(up, 'ascend', 'descend')",
        "issorted(hup, 'strictascend')",
        "issorted(hup, 2)",
    ] {
        let message = error(&mut e, &format!("r = {code};"));
        assert!(message.contains("issorted"), "{code}: {message}");
        assert!(message.contains("'ascend'"), "{code}: {message}");
    }
}

#[test]
fn input_format_with_an_unusable_letter_is_an_error_not_nan() {
    let mut e = Engine::default();
    // duration: the unit letters only print, other letters do not exist
    for (code, letter) in [
        ("duration('1.5', 'InputFormat', 'h')", "'h'"),
        ("duration('90', 'InputFormat', 's')", "'s'"),
        ("duration('1', 'InputFormat', 'y')", "'y'"),
        ("duration('01:30', 'InputFormat', 'hh:mm:xx')", "'x'"),
        ("duration('01:30', 'InputFormat', 'HH:mm')", "'H'"),
        ("duration('1', 'InputFormat', 'q')", "'q'"),
    ] {
        let message = error(&mut e, &format!("d = {code};"));
        assert!(message.contains("duration:"), "{code}: {message}");
        assert!(message.contains(letter), "{code}: {message}");
        assert!(message.contains("InputFormat"), "{code}: {message}");
    }
    // datetime: an unknown letter names itself
    for (code, letter) in [
        ("datetime('2024', 'InputFormat', 'yyyyZ')", "'Z'"),
        (
            "datetime('2024-01-01', 'InputFormat', 'yyyy-MM-dd G')",
            "'G'",
        ),
        (
            "datetime('2024', 'InputFormat', 'yyyy', 'Format', 'yyyyQ')",
            "'Q'",
        ),
    ] {
        let message = error(&mut e, &format!("t = {code};"));
        assert!(message.contains(letter), "{code}: {message}");
    }
    // text that merely does not match is still NaN / NaT
    assert_eq!(
        char_value(
            &mut e,
            "mat2str(isnan(seconds(duration({'01:00:00', 'oops'}, 'InputFormat', 'hh:mm:ss'))))"
        )
        .unwrap(),
        "[false true]"
    );
    assert_eq!(
        char_value(
            &mut e,
            "mat2str(isnat(datetime({'2024-01-01', 'oops'}, 'InputFormat', 'yyyy-MM-dd')))"
        )
        .unwrap(),
        "[false true]"
    );
    // the digital forms still read
    assert_eq!(
        char_value(&mut e, "char(duration('01:30', 'InputFormat', 'hh:mm'))").unwrap(),
        "01:30"
    );
}

// ---- fix round 2 ----
// Expected values are derived by hand in the proleptic Gregorian calendar
// (the inputs are a few whole days or hours apart).

#[test]
fn reductions_take_all_and_vecdim_and_reject_unknown_options() {
    let mut e = Engine::default();
    // 01-Jan 03-Jan
    // 04-Jan NaT
    run(
        &mut e,
        "t = [datetime(2024,1,1), datetime(2024,1,3); datetime(2024,1,4), NaT]; \
         d = hours([1 2; 3 10]);",
    );
    let ymd = |e: &mut Engine, code: &str| {
        char_value(e, &format!("char({code}, 'yyyy-MM-dd HH:mm')")).unwrap()
    };
    // min/max: 'all', vector dimensions, 'linear', 'includenat'.
    assert_eq!(ymd(&mut e, "max(t, [], 'all')"), "2024-01-04 00:00");
    assert_eq!(ymd(&mut e, "min(t, [], 'all')"), "2024-01-01 00:00");
    assert_eq!(ymd(&mut e, "max(t, [], [1 2])"), "2024-01-04 00:00");
    assert_eq!(ymd(&mut e, "max(t, [], [2 1 3])"), "2024-01-04 00:00");
    assert_eq!(dims(&mut e, "max(t, [], 'all')"), "[1 1]");
    assert_eq!(dims(&mut e, "max(t, [], [1 3])"), "[1 2]");
    assert_eq!(dims(&mut e, "max(t, [], 3)"), "[2 2]");
    assert_eq!(dims(&mut e, "max(t, [], [3 4])"), "[2 2]");
    assert_eq!(
        char_value(&mut e, "char(max(t, [], 'all', 'includenat'))").unwrap(),
        "NaT"
    );
    run(
        &mut e,
        "[m, i] = max(t, [], 'all'); [n, j] = min(t, [], 'all');",
    );
    assert_eq!(e.variables["i"].data(), &[2.0]);
    assert_eq!(e.variables["j"].data(), &[1.0]);
    run(
        &mut e,
        "[m, i] = max(t, [], 2); [m, k] = max(t, [], 2, 'linear');",
    );
    assert_eq!(e.variables["i"].data(), &[2.0, 1.0]);
    assert_eq!(e.variables["k"].data(), &[3.0, 2.0]);
    assert_eq!(
        char_value(&mut e, "char(max(d, [], 'all'))").unwrap(),
        "10 hr"
    );
    // sum (durations only), mean, median.
    assert_eq!(number(&mut e, "hours(sum(d, 'all'))").unwrap(), 16.0);
    assert_eq!(number(&mut e, "hours(sum(d, [1 2]))").unwrap(), 16.0);
    assert_eq!(
        char_value(&mut e, "mat2str(hours(sum(d, 1)))").unwrap(),
        "[4 12]"
    );
    assert_eq!(dims(&mut e, "sum(d, 3)"), "[2 2]");
    assert!(error(&mut e, "x = sum(t, 'all');").contains("cannot be added"));
    assert_eq!(number(&mut e, "hours(mean(d, 'all'))").unwrap(), 4.0);
    assert_eq!(char_value(&mut e, "char(mean(t, 'all'))").unwrap(), "NaT");
    // (0 + 2 + 3) / 3 days after 01-Jan.
    assert_eq!(ymd(&mut e, "mean(t, 'all', 'omitnat')"), "2024-01-02 16:00");
    assert_eq!(ymd(&mut e, "mean(t, [1 2], 'omitnat')"), "2024-01-02 16:00");
    assert_eq!(number(&mut e, "hours(median(d, 'all'))").unwrap(), 2.5);
    assert_eq!(number(&mut e, "hours(median(d, [1 2]))").unwrap(), 2.5);
    assert_eq!(
        char_value(&mut e, "mat2str(hours(median(d)))").unwrap(),
        "[2 6]"
    );
    assert_eq!(
        char_value(&mut e, "mat2str(hours(median(d, 2)))").unwrap(),
        "[1.5;6.5]"
    );
    assert_eq!(
        char_value(&mut e, "mat2str(isnan(hours(median([hours(1) NaN]))))").unwrap(),
        "true"
    );
    assert_eq!(
        number(&mut e, "hours(median([hours(1) NaN], 'omitnan'))").unwrap(),
        1.0
    );
    assert_eq!(char_value(&mut e, "char(median(t, 'all'))").unwrap(), "NaT");
    assert_eq!(
        ymd(&mut e, "median(t, 'all', 'omitnat')"),
        "2024-01-03 00:00"
    );
    assert_eq!(
        ymd(&mut e, "median(t, 2, 'omitnat')"),
        "2024-01-02 00:00\n2024-01-04 00:00"
    );
    // An even count averages the two middle values.
    assert_eq!(
        ymd(&mut e, "median([datetime(2024,1,1), datetime(2024,1,2)])"),
        "2024-01-01 12:00"
    );
    // Unknown options are errors that name them; nothing is ignored.
    for (code, named) in [
        ("x = max(t, [], 'bogus');", "'bogus'"),
        ("x = max(t, [], 1, 'omitnat', 'oops');", "'oops'"),
        ("x = min(t, t, 'sideways');", "'sideways'"),
        ("x = mean(t, 'foo');", "'foo'"),
        ("x = mean(t, 1, 'linear');", "'linear'"),
        ("x = median(d, 'x');", "'x'"),
        ("x = sum(d, 'all', 'omitnat');", "'omitnat'"),
        ("x = sum(d, 1, 2);", "dimension first"),
        ("x = max(t, [], [1 1]);", "repeat"),
        ("x = max(t, [], 0);", "positive integer"),
        ("x = mean(d, 1.5);", "positive integer"),
        ("x = min(t, t, 2);", "two input arrays"),
    ] {
        let message = error(&mut e, code);
        assert!(message.contains(named), "{code}: {message}");
    }
}

#[test]
fn concatenation_drops_0x0_operands() {
    let mut e = Engine::default();
    run(&mut e, "t = datetime(2024, 1, 5);");
    assert_eq!(dims(&mut e, "[NaT(0,0), t]"), "[1 1]");
    assert_eq!(dims(&mut e, "[t; NaT(0,0)]"), "[1 1]");
    assert_eq!(
        char_value(&mut e, "char([NaT(0,0), t], 'dd')").unwrap(),
        "05"
    );
    // Growing from datetime.empty in a loop.
    run(
        &mut e,
        "g = datetime.empty; for k = 1:3, g = [g, datetime(2024, 1, k)]; end",
    );
    assert_eq!(dims(&mut e, "g"), "[1 3]");
    assert_eq!(char_value(&mut e, "char(g, 'dd')").unwrap(), "01\n02\n03");
    run(
        &mut e,
        "h = duration.empty; for k = 1:2, h = [h; hours(k)]; end",
    );
    assert_eq!(char_value(&mut e, "mat2str(hours(h))").unwrap(), "[1;2]");
    assert_eq!(dims(&mut e, "[duration.empty, hours(1)]"), "[1 1]");
    assert_eq!(dims(&mut e, "[[], hours(1)]"), "[1 1]");
    assert_eq!(dims(&mut e, "[hours(1); []]"), "[1 1]");
    assert_eq!(dims(&mut e, "[[], t]"), "[1 1]");
    // A 0x0 operand decides neither the format nor the time zone.
    run(
        &mut e,
        "u = datetime(2024, 1, 1, 'TimeZone', 'UTC'); x = [datetime.empty, u];",
    );
    assert_eq!(char_value(&mut e, "x.TimeZone").unwrap(), "UTC");
    assert_eq!(
        char_value(&mut e, "[duration.empty, minutes(90)].Format").unwrap(),
        "m"
    );
    // datetime.empty / duration.empty with sizes.
    assert_eq!(
        char_value(&mut e, "class(datetime.empty)").unwrap(),
        "datetime"
    );
    assert_eq!(
        char_value(&mut e, "class(duration.empty)").unwrap(),
        "duration"
    );
    assert_eq!(dims(&mut e, "datetime.empty"), "[0 0]");
    assert_eq!(dims(&mut e, "datetime.empty(0, 3)"), "[0 3]");
    assert_eq!(dims(&mut e, "duration.empty(2, 0)"), "[2 0]");
    assert!(error(&mut e, "x = datetime.empty(2);").contains("must be zero"));
    // Other empty shapes must still fit, like dense concatenation.
    assert!(
        error(&mut e, "x = [datetime.empty(1, 0); t];").contains("not consistent"),
        "1x0 above 1x1"
    );
}

#[test]
fn for_loops_over_datetime_and_duration_ranges() {
    let mut e = Engine::default();
    run(
        &mut e,
        "n = 0; for d = datetime(2024,1,1):datetime(2024,1,3), n = n + 1; last = d; end",
    );
    assert_eq!(e.variables["n"].data(), &[3.0]);
    assert_eq!(
        char_value(&mut e, "char(last, 'yyyy-MM-dd')").unwrap(),
        "2024-01-03"
    );
    assert_eq!(char_value(&mut e, "class(last)").unwrap(), "datetime");
    run(
        &mut e,
        "k = 0; for d = datetime(2024,1,1):hours(6):datetime(2024,1,2), k = k + 1; end",
    );
    assert_eq!(e.variables["k"].data(), &[5.0]);
    // 1 + 1.5 + 2 + 2.5 + 3 hours.
    run(
        &mut e,
        "s = 0; for d = hours(1):minutes(30):hours(3), s = s + hours(d); end",
    );
    assert_eq!(e.variables["s"].data(), &[10.0]);
    run(
        &mut e,
        "m = 0; for d = datetime(2024,1,3):datetime(2024,1,1), m = m + 1; end",
    );
    assert_eq!(e.variables["m"].data(), &[0.0]);
    // A stored array iterates its columns.
    run(
        &mut e,
        "t = datetime(2024,1,1) + days(0:2); c = 0; for x = t, c = c + day(x); end",
    );
    assert_eq!(e.variables["c"].data(), &[6.0]);
}

#[test]
fn input_format_does_not_set_the_display_format() {
    let mut e = Engine::default();
    for (code, expected) in [
        (
            "datetime('2024.05.01', 'InputFormat', 'yyyy.MM.dd').Format",
            "dd-MMM-yyyy",
        ),
        (
            "char(datetime('2024.05.01', 'InputFormat', 'yyyy.MM.dd'))",
            "01-May-2024",
        ),
        (
            "datetime('2024-05-01 10:20', 'InputFormat', 'yyyy-MM-dd HH:mm').Format",
            "dd-MMM-yyyy HH:mm:ss",
        ),
        (
            "char(datetime('2024-05-01 10:20', 'InputFormat', 'yyyy-MM-dd HH:mm'))",
            "01-May-2024 10:20:00",
        ),
        (
            "datetime('2024.05.01', 'InputFormat', 'yyyy.MM.dd', 'Format', 'preserveinput').Format",
            "yyyy.MM.dd",
        ),
        (
            "char(datetime('2024.05.01', 'InputFormat', 'yyyy.MM.dd', 'Format', 'PreserveInput'))",
            "2024.05.01",
        ),
        (
            "datetime('2024-01-05 10:20:30', 'Format', 'preserveinput').Format",
            "yyyy-MM-dd HH:mm:ss",
        ),
        (
            "datetime('05-Jan-2024', 'Format', 'preserveinput').Format",
            "dd-MMM-yyyy",
        ),
        (
            "datetime(2024, 1, 1, 'Format', 'preserveinput').Format",
            "dd-MMM-yyyy",
        ),
        (
            "datetime('2024.05.01', 'InputFormat', 'yyyy.MM.dd', 'Format', 'yyyy/MM/dd').Format",
            "yyyy/MM/dd",
        ),
    ] {
        assert_eq!(char_value(&mut e, code).unwrap(), expected, "{code}");
    }
    let shown = run(
        &mut e,
        "t = datetime('2024.05.01', 'InputFormat', 'yyyy.MM.dd')",
    );
    assert!(shown.contains("01-May-2024"), "{shown}");
}

#[test]
fn string_of_nat_and_nan_is_missing() {
    let mut e = Engine::default();
    run(
        &mut e,
        "s = string([datetime(2024, 1, 1), NaT]); r = string([hours(1), hours(NaN)]); \
         q = string(duration(NaN, 0, 0));",
    );
    assert_eq!(
        char_value(&mut e, "mat2str(ismissing(s))").unwrap(),
        "[false true]"
    );
    assert_eq!(
        char_value(&mut e, "mat2str(ismissing(r))").unwrap(),
        "[false true]"
    );
    assert_eq!(char_value(&mut e, "mat2str(ismissing(q))").unwrap(), "true");
    assert_eq!(
        char_value(&mut e, "mat2str(ismissing(string(NaT, 'yyyy')))").unwrap(),
        "true"
    );
    assert!(run(&mut e, "z = string(NaT)").contains("<missing>"));
    assert_eq!(char_value(&mut e, "char(s(1))").unwrap(), "01-Jan-2024");
    // char and cellstr keep the text.
    assert_eq!(char_value(&mut e, "char(NaT)").unwrap(), "NaT");
    run(&mut e, "c = cellstr(NaT);");
    assert_eq!(char_value(&mut e, "c{1}").unwrap(), "NaT");
    assert_eq!(char_value(&mut e, "char(hours(NaN))").unwrap(), "NaN hr");
}

#[test]
fn a_cancel_flag_stops_large_elementwise_operations() {
    use std::sync::atomic::Ordering;
    let mut e = Engine::default();
    run(
        &mut e,
        "t = datetime(2024, 1, 1) + seconds(0:999999); d = hours(0:999999); x = 0;",
    );
    let stop = e.cancellation();
    for code in [
        "x = posixtime(t);",
        "x = year(t);",
        "x = t.Hour;",
        "x = string(t);",
        "x = char(d);",
        "x = t';",
        "x = -d;",
        "x = d ./ d;",
        "x = t - t;",
        "x = t + d;",
        "x = t < t;",
        "x = isnat(t);",
        "x = [d, 1];",
        "x = isequal(t, t);",
        "x = timeofday(t);",
        "x = abs(d);",
        "x = median(t);",
        "x = max(d, [], 'all');",
        "[x, y] = ymd(t);",
    ] {
        stop.store(true, Ordering::Relaxed);
        let message = error(&mut e, code);
        stop.store(false, Ordering::Relaxed);
        assert!(message.contains("cancelled"), "{code}: {message}");
        assert_eq!(e.variables["x"].data(), &[0.0], "{code} changed x");
    }
    // The same statements run to their end without the flag.
    run(&mut e, "x = posixtime(t); y = t - t;");
    assert_eq!(e.variables["x"].len(), 1_000_000);
}

#[test]
fn shifts_beyond_the_supported_years_are_errors_not_nat() {
    let mut e = Engine::default();
    // 40000 years of 365.2425 days are 100 Gregorian 400-year cycles of
    // 146097 days: inside the supported range, landing on 1-Jan-42024.
    run(&mut e, "f = datetime(2024, 1, 1) + years(40000);");
    assert_eq!(number(&mut e, "year(f)").unwrap(), 42024.0);
    assert_eq!(number(&mut e, "month(f)").unwrap(), 1.0);
    assert_eq!(number(&mut e, "day(f)").unwrap(), 1.0);
    for code in [
        "x = datetime(2024, 1, 1) + years(2e8);",
        "x = datetime(2024, 1, 1) - years(1e12);",
        "x = datetime(2024, 1, 1) + 1e15;",
        "x = datetime(2024, 1, 1) + milliseconds(1e25);",
    ] {
        let message = error(&mut e, code);
        assert!(message.contains("out of range"), "{code}: {message}");
    }
    // NaN and Inf shifts still give NaT.
    assert_eq!(
        char_value(&mut e, "char(datetime(2024, 1, 1) + hours(NaN))").unwrap(),
        "NaT"
    );
}

#[test]
fn review_1111_datetime_display_rounds_fractional_seconds() {
    // MathWorks R2018a: displayed fractional seconds round (the R2017b SSS
    // truncation of a stored .474999999 to .474 was a bug). Python float
    // 1.6 ms is 1600000 ns; round-half-away to milliseconds is 2.
    let mut e = Engine::default();
    for (code, expected) in [
        (
            "char(datetime(2024,1,1,0,0,0,1.6),'HH:mm:ss.SSS')",
            "00:00:00.002",
        ),
        (
            "char(datetime(2024,1,31,10,20,30.5))",
            "31-Jan-2024 10:20:31",
        ),
        (
            "char(datetime(2024,1,1,0,0,1.996),'HH:mm:ss.SS')",
            "00:00:02.00",
        ),
        ("char(datetime(2024,1,1,0,0,59.6))", "01-Jan-2024 00:01:00"),
        ("char(datetime(2024,1,1,0,59,59.6))", "01-Jan-2024 01:00:00"),
    ] {
        assert_eq!(char_value(&mut e, code).unwrap(), expected, "{code}");
    }
    // Rounding is display-only.
    assert_eq!(
        number(&mut e, "second(datetime(2024,1,31,10,20,30.5))").unwrap(),
        30.5
    );
}

#[test]
fn review_1111_duration_text_rounds_half_away() {
    // Same digital-clock rounding as datetime. Python: 1.235*1000 is the
    // exact float 1235, so hh:mm:ss.SS rounds the half hundredth away from
    // zero to 01.24; 1.996*1000 is 1996 and rounds to 02.00.
    let mut e = Engine::default();
    for (code, expected) in [
        ("char(duration(0,0,1.6))", "00:00:02"),
        ("char(duration(0,0,59.6))", "00:01:00"),
        ("char(duration(0,0,-1.5))", "-00:00:02"),
        ("char(duration(0,0,1.235),'hh:mm:ss.SS')", "00:00:01.24"),
        ("char(duration(0,0,1.996),'hh:mm:ss.SS')", "00:00:02.00"),
        ("char(duration(0,1,30.5),'hh:mm')", "00:02"),
    ] {
        assert_eq!(char_value(&mut e, code).unwrap(), expected, "{code}");
    }
}

#[test]
fn review_s1113_huge_duration_hour_field_stays_finite() {
    // milliseconds(1e307) is past exact nanoseconds. Rounding through 1 ns
    // overflows to Inf. The hour field is 1e307/3_600_000 ≈ 2.777777777777778e300.
    let mut e = Engine::default();
    let text = char_value(&mut e, "char(milliseconds(1e307),'hh:mm:ss.SSSSSSSSS')").unwrap();
    let text = text.trim();
    assert!(!text.to_ascii_lowercase().contains("inf"), "{text}");
    let got: f64 = text
        .parse()
        .unwrap_or_else(|_| panic!("not a number: {text}"));
    let want = 1e307 / 3_600_000.0;
    assert!(
        (got - want).abs() <= 8. * f64::EPSILON * want.abs(),
        "got {got:.17e}, want {want:.17e}, text {text}"
    );
}

#[test]
fn review_1111_calendar_duration_plus_fixed_duration() {
    // MathWorks calendarDuration arithmetic: a fixed duration is elapsed
    // time, and hours greater than 24 do not fold into calendar days
    // (calendarDuration(0,0,0,25,0,0) stays 25 h).
    let mut e = Engine::default();
    assert_eq!(
        char_value(&mut e, "char(caldays(1)+hours(12))").unwrap(),
        "1d 12h 0m 0s"
    );
    assert_eq!(
        char_value(&mut e, "char(hours(30)-caldays(1))").unwrap(),
        "-1d 30h 0m 0s"
    );
    run(
        &mut e,
        "assert(isequal(caldays(1)+hours(12), calendarDuration(0,0,1,12,0,0))); assert(isequal(hours(30)-caldays(1), calendarDuration(0,0,-1,30,0,0))); assert(isequal(datetime(2024,1,31)+(calmonths(1)+days(1)), datetime(2024,3,1)));",
    );
}
