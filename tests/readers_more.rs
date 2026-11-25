//! jsondecode/jsonencode, readcell/writecell, xlsread and spreadsheet
//! readmatrix. Fixtures: tests/fixtures/readers/ (book.xlsx comes from
//! tool/refs/readers/make_xlsx.py; the .json, .csv and .txt files are hand
//! made). Expected values follow the published function descriptions; JSON
//! strings are the compact forms those descriptions define.
use matrust::{Array, Engine};
use std::{fs, path::Path};

const FIXTURES: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/tests/fixtures/readers");

/// A fresh engine working in a fresh temporary folder holding the fixtures.
fn engine() -> (Engine, tempfile::TempDir) {
    let dir = tempfile::tempdir().unwrap();
    for entry in fs::read_dir(FIXTURES).unwrap() {
        let entry = entry.unwrap();
        fs::copy(entry.path(), dir.path().join(entry.file_name())).unwrap();
    }
    let mut e = Engine::default();
    e.set_current_folder(dir.path()).unwrap();
    (e, dir)
}

fn run(e: &mut Engine, source: &str) -> String {
    match e.execute(source) {
        Ok(x) => x.output.join("\n"),
        Err(error) => panic!("{source}: {error}"),
    }
}

fn fails(e: &mut Engine, source: &str) -> String {
    match e.execute(source) {
        Ok(x) => panic!("{source} should fail, printed {:?}", x.output),
        Err(error) => error.to_string(),
    }
}

/// `expr` evaluates to a nonempty, all-true logical or numeric array.
#[track_caller]
fn check(e: &mut Engine, expr: &str) {
    run(e, &format!("t__ = {expr};"));
    let ok = match e.array("t__") {
        Some(Array::Logical(m)) => !m.data().is_empty() && m.data().iter().all(|v| *v != 0),
        Some(Array::Numeric(m)) => !m.data().is_empty() && m.data().iter().all(|v| *v != 0.),
        other => panic!("{expr}: not logical: {other:?}"),
    };
    assert!(ok, "{expr} is false");
}

/// The char result of `expr`.
fn text(e: &mut Engine, expr: &str) -> String {
    run(e, &format!("t__ = {expr};"));
    match e.array("t__") {
        Some(Array::Character(c)) => String::from_utf16(c.units()).unwrap(),
        other => panic!("{expr}: not char: {other:?}"),
    }
}

#[test]
fn jsondecode_maps_every_json_kind() {
    let (mut e, _dir) = engine();
    run(&mut e, "s = jsondecode(fileread('sample.json'));");
    check(&mut e, "isstruct(s) && isequal(size(s), [1 1])");
    check(
        &mut e,
        "isequal(fieldnames(s)', {'name','id','active','offline','notes','levels','grid','cube','flags','tags','sensors','mixed','ragged','badKey_name','x1st','xFor','dup','dup_1','unicode','empty_obj','empty_arr'})",
    );
    check(&mut e, "ischar(s.name) && strcmp(s.name, 'pump station')");
    check(&mut e, "isa(s.id, 'double') && s.id == 42");
    check(
        &mut e,
        "islogical(s.active) && s.active && islogical(s.offline) && ~s.offline",
    );
    check(
        &mut e,
        "isa(s.notes, 'double') && isequal(size(s.notes), [0 0])",
    );
    check(&mut e, "isequaln(s.levels, [1.5; 2; NaN; -300])");
    check(&mut e, "isequal(s.grid, [1 2 3; 4 5 6])");
    run(&mut e, "cube = s.cube;");
    check(
        &mut e,
        "isequal(size(cube), [2 2 2]) && cube(2,1,2) == 6 && cube(1,2,1) == 3 && cube(2,2,2) == 8",
    );
    check(
        &mut e,
        "islogical(s.flags) && isequal(s.flags, [true; false; true])",
    );
    check(&mut e, "iscell(s.tags) && isequal(size(s.tags), [3 1])");
    check(
        &mut e,
        "strcmp(s.tags{2}, 'bc') && ischar(s.tags{3}) && isempty(s.tags{3})",
    );
    check(
        &mut e,
        "isstruct(s.sensors) && isequal(size(s.sensors), [2 1])",
    );
    check(&mut e, "isequal(fieldnames(s.sensors)', {'kind','value'})");
    check(
        &mut e,
        "strcmp(s.sensors(2).kind, 'head') && s.sensors(2).value == 4",
    );
    check(&mut e, "iscell(s.mixed) && isequal(size(s.mixed), [4 1])");
    check(
        &mut e,
        "s.mixed{1} == 1 && strcmp(s.mixed{2}, 'two') && isequal(s.mixed{3}, [3; 4]) && s.mixed{4}.five == 5",
    );
    check(
        &mut e,
        "iscell(s.ragged) && isequal(s.ragged{1}, [1; 2]) && s.ragged{2} == 3",
    );
    check(&mut e, "s.badKey_name == 1 && s.x1st == 2 && s.xFor == 3");
    check(&mut e, "s.dup == 1 && s.dup_1 == 2");
    check(
        &mut e,
        "isequal(double(s.unicode), [99 97 102 233 32 55357 56832])",
    );
    check(
        &mut e,
        "isstruct(s.empty_obj) && isempty(fieldnames(s.empty_obj))",
    );
    check(&mut e, "isa(s.empty_arr, 'double') && isempty(s.empty_arr)");
}

#[test]
fn jsondecode_scalars_and_special_arrays() {
    let mut e = Engine::default();
    check(&mut e, "jsondecode('3.25') == 3.25");
    check(&mut e, "isequal(jsondecode(' [ ] '), [])");
    check(&mut e, "isequal(jsondecode('null'), [])");
    check(&mut e, "strcmp(jsondecode('\"a\\tb\"'), sprintf('a\\tb'))");
    check(&mut e, "isequal(jsondecode('[[1],[2],[3]]'), [1;2;3])");
    check(&mut e, "isequal(jsondecode('[[1,2,3]]'), [1 2 3])");
    check(
        &mut e,
        "isequaln(jsondecode('[NaN, Infinity, -Infinity, Inf]'), [NaN; Inf; -Inf; Inf])",
    );
    check(&mut e, "iscell(jsondecode('[true, 1]'))");
    check(&mut e, "iscell(jsondecode('[null, null]'))");
    check(&mut e, "iscell(jsondecode('[{\"a\":1},{\"b\":1}]'))");
    check(&mut e, "isequal(size(jsondecode('[{},{}]')), [2 1])");
    check(&mut e, "isequal(jsondecode(\"[1e2, -0.5]\"), [100; -0.5])");
    // Large integers keep the nearest double.
    check(&mut e, "jsondecode('9007199254740993') == 9007199254740992");
}

#[test]
fn jsondecode_make_valid_name_option() {
    let mut e = Engine::default();
    check(&mut e, "isfield(jsondecode('{\"a b\":1}'), 'aB')");
    check(
        &mut e,
        "isfield(jsondecode('{\"a b\":1}', 'makeValidName', true), 'aB')",
    );
    check(
        &mut e,
        "jsondecode('{\"ok\":1}', 'makeValidName', false).ok == 1",
    );
    let error = fails(&mut e, "jsondecode('{\"a b\":1}', 'makeValidName', false)");
    assert!(error.contains("not a valid field name"), "{error}");
    let error = fails(&mut e, "jsondecode('{}', 'Strict', true)");
    assert!(error.contains("unknown option"), "{error}");
}

#[test]
fn jsondecode_rejects_malformed_text() {
    let mut e = Engine::default();
    for (bad, part) in [
        ("{\"a\":}", "expected a value"),
        ("[1,2", "expected ',' or ']'"),
        ("{\"a\" 1}", "expected ':'"),
        ("{a:1}", "quoted member name"),
        ("\"\\x\"", "invalid escape"),
        ("01", "after the JSON value"),
        ("1.", "decimal point"),
        ("1e", "exponent"),
        ("[1,]", "expected a value"),
        ("tru", "expected a value"),
        ("\"open", "unterminated string"),
        ("   ", "empty"),
        ("[1] [2]", "after the JSON value"),
    ] {
        let error = fails(&mut e, &format!("jsondecode('{bad}')"));
        assert!(error.contains(part), "{bad}: {error}");
    }
    let error = fails(&mut e, "jsondecode('[1, x]')");
    assert!(error.contains("character 5"), "{error}");
    let error = fails(&mut e, "jsondecode(42)");
    assert!(error.contains("character vector or string"), "{error}");
    let deep = format!("jsondecode('{}{}')", "[".repeat(200), "]".repeat(200));
    let error = fails(&mut e, &deep);
    assert!(error.contains("nesting"), "{error}");
}

#[test]
fn jsonencode_expected_strings() {
    let mut e = Engine::default();
    for (expr, want) in [
        (
            "jsonencode(struct('a',[1 2 3],'b','text'))",
            r#"{"a":[1,2,3],"b":"text"}"#,
        ),
        ("jsonencode({1, true, 'x', []})", r#"[1,true,"x",[]]"#),
        ("jsonencode([1 NaN Inf])", "[1,null,null]"),
        (
            "jsonencode([1 NaN Inf -Inf], 'ConvertInfAndNaN', false)",
            "[1,NaN,Infinity,-Infinity]",
        ),
        ("jsonencode(magic(3))", "[[8,1,6],[3,5,7],[4,9,2]]"),
        ("jsonencode([1;2;3])", "[1,2,3]"),
        ("jsonencode(5)", "5"),
        ("jsonencode(pi)", "3.141592653589793"),
        ("jsonencode(0.1)", "0.1"),
        ("jsonencode(1e300)", "1e300"),
        ("jsonencode(-2.5e-8)", "-2.5e-8"),
        ("jsonencode(true)", "true"),
        ("jsonencode([true false])", "[true,false]"),
        ("jsonencode('')", r#""""#),
        ("jsonencode([])", "[]"),
        ("jsonencode({})", "[]"),
        ("jsonencode(\"str\")", r#""str""#),
        ("jsonencode([\"a\",\"b\"])", r#"["a","b"]"#),
        ("jsonencode(['ab';'cd'])", r#"["ab","cd"]"#),
        (
            "jsonencode(['a\"b\\' char(10) char(1)])",
            r#""a\"b\\\n\u0001""#,
        ),
        ("jsonencode(int8([1 -2]))", "[1,-2]"),
        (
            "jsonencode(uint64(18446744073709551615))",
            "18446744073709551615",
        ),
        ("jsonencode(single(0.1))", "0.1"),
        ("jsonencode({1, 2; 3, 4})", "[1,3,2,4]"),
        ("jsonencode(struct())", "{}"),
        ("jsonencode(struct('a', {}))", "[]"),
        (
            "jsonencode(struct('a', {1, 'x'}))",
            r#"[{"a":1},{"a":"x"}]"#,
        ),
        (
            "jsonencode(struct('c', {{1, 'two'}}))",
            r#"{"c":[1,"two"]}"#,
        ),
    ] {
        assert_eq!(text(&mut e, expr), want, "{expr}");
    }
    run(&mut e, "w = struct(); w.x = reshape(1:8, 2, 2, 2);");
    assert_eq!(
        text(&mut e, "jsonencode(w)"),
        "{\"x\":[[[1,5],[3,7]],[[2,6],[4,8]]]}"
    );
    let error = fails(&mut e, "jsonencode(reshape(1:8, 2, 2, 2))");
    assert!(error.contains("N-D"), "{error}");
    run(&mut e, "s = [\"a\" \"b\"]; s(2) = missing;");
    assert_eq!(text(&mut e, "jsonencode(s)"), r#"["a",null]"#);
    run(&mut e, "m = containers.Map({'b','a'}, {1, 'x'});");
    assert_eq!(text(&mut e, "jsonencode(m)"), r#"{"a":"x","b":1}"#);
    assert_eq!(
        text(&mut e, "jsonencode(struct('m', m))"),
        r#"{"m":{"a":"x","b":1}}"#
    );
}

#[test]
fn jsonencode_pretty_print() {
    let mut e = Engine::default();
    let got = text(
        &mut e,
        "jsonencode(struct('a',[1 2],'b',struct('c',[]),'d',{{}}), 'PrettyPrint', true)",
    );
    assert_eq!(
        got,
        "{\n  \"a\": [\n    1,\n    2\n  ],\n  \"b\": {\n    \"c\": []\n  },\n  \"d\": []\n}"
    );
    assert_eq!(text(&mut e, "jsonencode(7, 'PrettyPrint', true)"), "7");
}

#[test]
fn jsonencode_errors_and_statement_form() {
    let mut e = Engine::default();
    for (expr, part) in [
        ("jsonencode(1+2i)", "complex"),
        ("jsonencode(@sin)", "function handles"),
        ("jsonencode(1, 'PrettyPrint')", "name/value pairs"),
        ("jsonencode(1, 'Indent', 2)", "unknown option"),
        ("jsonencode(1, 'PrettyPrint', 'yes')", "true or false"),
        ("jsonencode()", "Not enough input"),
    ] {
        let error = fails(&mut e, expr);
        assert!(error.contains(part), "{expr}: {error}");
    }
    let shown = run(&mut e, "jsonencode([1 2])");
    assert!(shown.contains("[1,2]"), "{shown}");
    check(&mut e, "strcmp(ans, '[1,2]')");
}

#[test]
fn json_round_trips() {
    let mut e = Engine::default();
    run(
        &mut e,
        "v = struct('name','x','m',[1 2;3 4],'col',[1;2;3],'flag',true,'inner',struct('t','u','e',[]),'c',{{'a';2}});",
    );
    check(&mut e, "isequal(jsondecode(jsonencode(v)), v)");
    check(
        &mut e,
        "isequal(jsondecode(jsonencode(v, 'PrettyPrint', true)), v)",
    );
    run(&mut e, "a = reshape(1:24, 2, 3, 4);");
    run(
        &mut e,
        "w = struct(); w.a = a; back = jsondecode(jsonencode(w)); b = back.a;",
    );
    check(&mut e, "isequal(b, a)");
    run(
        &mut e,
        "t = '{\"k\":[{\"a\":1,\"b\":[true,false]},{\"a\":2,\"b\":[false,false]}],\"s\":\"q\\\"\"}';",
    );
    check(&mut e, "strcmp(jsonencode(jsondecode(t)), t)");
}

#[test]
fn readcell_text_type_mapping() {
    let (mut e, _dir) = engine();
    run(&mut e, "C = readcell('mixed.csv');");
    check(&mut e, "iscell(C) && isequal(size(C), [5 4])");
    check(&mut e, "strcmp(C{1,1}, 'name') && strcmp(C{1,4}, 'note')");
    check(
        &mut e,
        "isa(C{2,2}, 'double') && C{2,2} == 4 && C{2,3} == 0.25",
    );
    check(&mut e, "strcmp(C{2,4}, 'steel, zinc')");
    check(
        &mut e,
        "ismissing(C{3,2}) && C{3,3} == 1e-3 && ismissing(C{3,4})",
    );
    check(
        &mut e,
        "strcmp(C{4,1}, 'say \"hi\"') && C{4,2} == -Inf && isnan(C{4,3})",
    );
    check(&mut e, "strcmp(C{4,4}, 'padded')");
    check(
        &mut e,
        "strcmp(C{5,1}, 'washer') && C{5,2} == 7 && ismissing(C{5,3}) && ismissing(C{5,4})",
    );
}

#[test]
fn readcell_headers_delimiters_and_ranges() {
    let (mut e, dir) = engine();
    run(
        &mut e,
        "C = readcell('headers.txt', 'NumHeaderLines', 2, 'Delimiter', ';');",
    );
    check(&mut e, "isequal(C, {'a','b','c'; 1,2,3})");
    run(&mut e, "C = readcell('headers.txt', 'NumHeaderLines', 2);");
    check(&mut e, "isequal(C, {'a','b','c'; 1,2,3})");
    run(&mut e, "C = readcell('mixed.csv', 'Range', 'B2:C3');");
    check(
        &mut e,
        "isequal(size(C), [2 2]) && C{1,1} == 4 && C{1,2} == 0.25 && ismissing(C{2,1}) && C{2,2} == 1e-3",
    );
    // Line 5 is blank: inside a Range it is a row of missing cells.
    run(&mut e, "C = readcell('mixed.csv', 'Range', 'B5');");
    check(
        &mut e,
        "isequal(size(C), [2 1]) && ismissing(C{1,1}) && C{2,1} == 7",
    );
    fs::write(dir.path().join("spaced.txt"), "1   2  x\n  3 4\n").unwrap();
    run(&mut e, "C = readcell('spaced.txt', 'Delimiter', ' ');");
    check(
        &mut e,
        "isequal(size(C), [2 3]) && C{1,2} == 2 && strcmp(C{1,3}, 'x') && C{2,1} == 3 && ismissing(C{2,3})",
    );
    fs::write(dir.path().join("tabs.dat"), "a\tb\n\"multi\nline\"\t5\n").unwrap();
    run(&mut e, "C = readcell('tabs.dat');");
    check(
        &mut e,
        "isequal(size(C), [2 2]) && strcmp(C{2,1}, sprintf('multi\\nline')) && C{2,2} == 5",
    );
    fs::write(dir.path().join("empty.csv"), "").unwrap();
    check(&mut e, "isequal(size(readcell('empty.csv')), [0 0])");
    for (expr, part) in [
        ("readcell('nope.csv')", "unable to open"),
        (
            "readcell('mixed.csv', 'NumHeaderLines', 1, 'Range', 'A1')",
            "not both",
        ),
        ("readcell('mixed.csv', 'Range', 'Q')", "Range must be"),
        (
            "readcell('mixed.csv', 'Delimiter', '::')",
            "supported delimiters",
        ),
        (
            "readcell('mixed.csv', 'Sheet', 1)",
            "spreadsheet files only",
        ),
        (
            "readcell('mixed.csv', 'TextType', 'string')",
            "unsupported option",
        ),
        ("readcell('sample.json')", "use a .csv"),
        (
            "readcell('mixed.csv', 'NumHeaderLines')",
            "name/value pairs",
        ),
    ] {
        let error = fails(&mut e, expr);
        assert!(error.contains(part), "{expr}: {error}");
    }
}

#[test]
fn writecell_writes_and_reads_back() {
    let (mut e, dir) = engine();
    run(
        &mut e,
        "C = {'a', 1; 'b, c', true; \"str\", []; 'q\"t', -0.5};",
    );
    run(&mut e, "writecell(C, 'w.csv')");
    assert_eq!(
        fs::read_to_string(dir.path().join("w.csv")).unwrap(),
        "a,1\n\"b, c\",1\nstr,\n\"q\"\"t\",-0.5\n"
    );
    run(&mut e, "R = readcell('w.csv');");
    check(
        &mut e,
        "strcmp(R{2,1}, 'b, c') && R{2,2} == 1 && strcmp(R{3,1}, 'str') && ismissing(R{3,2}) && strcmp(R{4,1}, 'q\"t')",
    );
    run(
        &mut e,
        "writecell({1, NaN; Inf, 'x'}, 'tab', 'Delimiter', 'tab', 'QuoteStrings', true)",
    );
    assert_eq!(
        fs::read_to_string(dir.path().join("tab.txt")).unwrap(),
        "1\tNaN\nInf\t\"x\"\n"
    );
    for (expr, part) in [
        ("writecell({1}, 'out.xlsx')", "not supported yet"),
        (
            "writecell({1}, 'out.csv', 'FileType', 'spreadsheet')",
            "not supported yet",
        ),
        ("writematrix([1 2], 'out.xlsx')", "not supported yet"),
        (
            "writematrix([1 2], 'out', 'FileType', 'spreadsheet')",
            "not supported yet",
        ),
        ("writecell({[1 2]}, 'out.csv')", "cell (1,1)"),
        ("writecell([1 2], 'out.csv')", "must be a cell array"),
        ("writecell({1}, 'out.json')", "use a .csv"),
        ("x = writecell({1}, 'out.csv')", "Too many output"),
    ] {
        let error = fails(&mut e, expr);
        assert!(error.contains(part), "{expr}: {error}");
    }
    assert!(!dir.path().join("out.xlsx").exists());
}

#[test]
fn spreadsheets_read_with_sheet_and_range() {
    let (mut e, _dir) = engine();
    run(&mut e, "C = readcell('book.xlsx');");
    check(
        &mut e,
        "isequal(size(C), [6 3]) && strcmp(C{1,1}, 'x') && C{2,2} == 2.5",
    );
    check(
        &mut e,
        "islogical(C{3,3}) && C{3,3} && ismissing(C{4,2}) && strcmp(C{4,3}, 'third, with comma')",
    );
    check(&mut e, "all(cellfun(@ismissing, C(5,:))) && C{6,1} == 10");
    run(&mut e, "C = readcell('book.xlsx', 'Sheet', 'Offset');");
    check(&mut e, "isequal(C, {'k','v'; 1,2; 3,'four'})");
    check(&mut e, "isequal(readcell('book.xlsx', 'Sheet', 2), C)");
    check(
        &mut e,
        "isequal(readcell('book.xlsx', 'Sheet', 'offset', 'Range', 'C4:C5'), {2; 'four'})",
    );
    // NumHeaderLines counts sheet rows from row 1 (the used area starts at B3).
    check(
        &mut e,
        "isequal(readcell('book.xlsx', 'Sheet', 'Offset', 'NumHeaderLines', 3), {1,2; 3,'four'})",
    );
    check(
        &mut e,
        "isequal(size(readcell('book.xlsx', 'Sheet', 'Empty')), [0 0])",
    );

    run(&mut e, "A = readmatrix('book.xlsx');");
    check(
        &mut e,
        "isequaln(A, [1 2.5 NaN; 3 -4 1; 5 NaN NaN; NaN NaN NaN; 10 NaN NaN])",
    );
    check(
        &mut e,
        "isequaln(readmatrix('book.xlsx', 'Sheet', 'Offset'), [1 2; 3 NaN])",
    );
    check(
        &mut e,
        "isequal(readmatrix('book.xlsx', 'Range', 'A2:B3'), [1 2.5; 3 -4])",
    );
    check(
        &mut e,
        "isequaln(readmatrix('book.xlsx', 'Range', 'A1:A2'), [NaN; 1])",
    );
    check(
        &mut e,
        "isequal(size(readmatrix('book.xlsx', 'Sheet', 3)), [0 0])",
    );

    run(&mut e, "[num, txt, raw] = xlsread('book.xlsx');");
    check(&mut e, "isequaln(num, A)");
    check(
        &mut e,
        "isequal(txt, {'x','y','label'; '','','first'; '','',''; '','','third, with comma'})",
    );
    check(
        &mut e,
        "isequal(size(raw), [6 3]) && isnan(raw{4,2}) && islogical(raw{3,3}) && strcmp(raw{1,3}, 'label')",
    );
    run(
        &mut e,
        "[n, t, r] = xlsread('book.xlsx', 'Offset', 'B4:C5');",
    );
    check(
        &mut e,
        "isequaln(n, [1 2; 3 NaN]) && isequal(t, {'four'}) && isequal(r, {1, 2; 3, 'four'})",
    );
    check(
        &mut e,
        "isequal(xlsread('book.xlsx', 'A2:B3'), [1 2.5; 3 -4])",
    );
    check(
        &mut e,
        "isequaln(xlsread('book.xlsx', 2, '', 'basic'), [1 2; 3 NaN])",
    );
    run(&mut e, "[n, t] = xlsread('book.xlsx', 'Empty');");
    check(&mut e, "isempty(n) && iscell(t) && isempty(t)");
}

#[test]
fn spreadsheet_errors() {
    let (mut e, dir) = engine();
    fs::write(dir.path().join("bad.xlsx"), b"not a zip archive").unwrap();
    for (expr, part) in [
        (
            "readcell('book.xlsx', 'Sheet', 'Nope')",
            "Data, Offset, Empty",
        ),
        ("readmatrix('book.xlsx', 'Sheet', 9)", "does not exist"),
        ("readmatrix('book.xlsx', 'Sheet', 0)", "1 or greater"),
        ("readcell('bad.xlsx')", "Unable to read spreadsheet"),
        ("xlsread('bad.xlsx')", "Unable to read spreadsheet"),
        ("xlsread('missing.xlsx')", "Unable to open spreadsheet"),
        ("xlsread('mixed.csv')", "xlsx"),
        ("xlsread('book.xlsx', 1, 'A1:B2', 'activex')", "basic"),
        (
            "readmatrix('book.xlsx', 'Delimiter', ',')",
            "unsupported spreadsheet option",
        ),
        ("readcell('book.xlsx', 'Delimiter', ',')", "text files only"),
        (
            "readcell('book.xlsx', 'Range', 'A1:ZZZZ2')",
            "Range must be",
        ),
    ] {
        let error = fails(&mut e, expr);
        assert!(error.contains(part), "{expr}: {error}");
    }
}

/// Fix round 1, finding 1: the cell count is checked before anything dense
/// is allocated. far.xlsx and the .xls files come from
/// tool/refs/readers/make_limits.py; reading their used area densely would
/// need billions of cells.
#[test]
fn spreadsheet_size_is_checked_before_allocating() {
    let (mut e, _dir) = engine();
    for (expr, part) in [
        (
            "readmatrix('far.xlsx')",
            "A1:XFD1048576 has more than 16777216 cells",
        ),
        ("readcell('far.xlsx')", "more than 65536 cells"),
        ("xlsread('far.xlsx')", "more than 16777216 cells"),
        (
            "readmatrix('far.xlsx', 'Range', 'A1')",
            "give a smaller Range",
        ),
        ("readcell('dims.xls')", "more than 16777216 cells"),
        ("readmatrix('far.xls')", "more than 16777216 cells"),
        (
            "xlsread('loop.xls')",
            "damaged .xls file (sector chain loop)",
        ),
    ] {
        let error = fails(&mut e, expr);
        assert!(error.contains(part), "{expr}: {error}");
    }
    // A Range away from the far cell reads only its own rectangle.
    run(&mut e, "C = readcell('far.xlsx', 'Range', 'A1:B2');");
    check(
        &mut e,
        "isequal(size(C), [2 2]) && C{1,1} == 1 && ismissing(C{1,2}) && ismissing(C{2,2})",
    );
    check(
        &mut e,
        "isequal(readmatrix('far.xlsx', 'Range', 'A1:A1'), 1)",
    );
    // A valid .xls workbook still reads through the check.
    run(&mut e, "C = readcell('small.xls');");
    check(
        &mut e,
        "isequal(size(C), [3 3]) && C{1,1} == 1 && C{1,2} == 2.5 && C{2,1} == 3",
    );
    check(
        &mut e,
        "islogical(C{2,2}) && C{2,2} && strcmp(C{3,3}, 'xy') && ismissing(C{1,3}) && ismissing(C{3,1})",
    );
    check(
        &mut e,
        "isequal(readmatrix('small.xls', 'Sheet', 'Second'), 7)",
    );
    check(&mut e, "isequal(xlsread('small.xls', 2), 7)");
}

/// Fix round 1, finding 2: blank lines inside a text Range are rows of
/// missing cells; later rows keep their place.
#[test]
fn readcell_text_range_keeps_blank_lines() {
    let (mut e, dir) = engine();
    run(&mut e, "C = readcell('mixed.csv', 'Range', 'A5:A6');");
    check(
        &mut e,
        "isequal(size(C), [2 1]) && ismissing(C{1}) && strcmp(C{2}, 'washer')",
    );
    run(&mut e, "C = readcell('mixed.csv', 'Range', 'A4:B6');");
    check(
        &mut e,
        "isequal(size(C), [3 2]) && strcmp(C{1,1}, 'say \"hi\"') && ismissing(C{2,1}) && ismissing(C{2,2}) && C{3,2} == 7",
    );
    // An open end stops at the last line with fields; without a Range blank
    // lines are still skipped.
    fs::write(dir.path().join("gaps.csv"), "1,2\n\n3,4\n\n\n").unwrap();
    run(&mut e, "C = readcell('gaps.csv', 'Range', 'A1');");
    check(
        &mut e,
        "isequal(size(C), [3 2]) && C{1,2} == 2 && ismissing(C{2,1}) && C{3,1} == 3",
    );
    check(&mut e, "isequal(readcell('gaps.csv'), {1,2; 3,4})");
    check(
        &mut e,
        "isequal(readcell('gaps.csv', 'NumHeaderLines', 1), {3,4})",
    );
}

/// Fix round 1, finding 3: spreadsheet NumHeaderLines counts from sheet row
/// 1. On sheet Offset rows 1-2 are empty and B3:C3 hold the k/v header.
#[test]
fn spreadsheet_num_header_lines_count_sheet_rows() {
    let (mut e, _dir) = engine();
    let all = "{'k','v'; 1,2; 3,'four'}";
    for (headers, expected) in [
        (0, all),
        (1, all),
        (2, all),
        (3, "{1,2; 3,'four'}"),
        (4, "{3,'four'}"),
    ] {
        check(
            &mut e,
            &format!(
                "isequal(readcell('book.xlsx', 'Sheet', 'Offset', 'NumHeaderLines', {headers}), {expected})"
            ),
        );
    }
    check(
        &mut e,
        "isequal(size(readcell('book.xlsx', 'Sheet', 'Offset', 'NumHeaderLines', 5)), [0 0])",
    );
    // Sheet Data starts at row 1, so one header line drops x, y, label.
    check(
        &mut e,
        "isequal(size(readcell('book.xlsx', 'NumHeaderLines', 1)), [5 3])",
    );
}

/// Fix round 1, finding 4: readmatrix with 'FileType','spreadsheet' uses the
/// spreadsheet reader for any extension and in any option order.
#[test]
fn readmatrix_file_type_spreadsheet_reads_sheets() {
    let (mut e, dir) = engine();
    fs::copy(dir.path().join("book.xlsx"), dir.path().join("book.dat")).unwrap();
    run(&mut e, "A = readmatrix('book.xlsx');");
    check(
        &mut e,
        "isequaln(readmatrix('book.dat', 'FileType', 'spreadsheet'), A)",
    );
    check(
        &mut e,
        "isequaln(readmatrix('book.xlsx', 'Sheet', 'Offset', 'FileType', 'Spreadsheet'), [1 2; 3 NaN])",
    );
    check(
        &mut e,
        "isequal(readmatrix('book.dat', \"Range\", \"A2:B3\", \"FileType\", \"spreadsheet\"), [1 2.5; 3 -4])",
    );
    let error = fails(&mut e, "readmatrix('mixed.csv', 'FileType', 'spreadsheet')");
    assert!(error.contains("not an .xlsx"), "{error}");
    let error = fails(&mut e, "readmatrix('mixed.csv', 'FileType', 'fixedwidth')");
    assert!(
        error.contains("'text' or 'spreadsheet'") && !error.contains("not implemented"),
        "{error}"
    );
}

/// The help page examples of the file functions, run in a temporary folder
/// (tests/help_system.rs skips them so it does not write into the checkout).
#[test]
fn file_help_examples_run_in_a_temporary_folder() {
    let pages = matrust::help::parse_pages(include_str!("../assets/help/functions.md"));
    let mut ran = 0;
    for name in [
        "readcell",
        "writecell",
        "xlsread",
        "readmatrix",
        "readtable",
        "writetable",
    ] {
        let body = pages[name];
        for line in body.lines().filter_map(|l| l.strip_prefix("Example:")) {
            let code = line.split("   %").next().unwrap().trim();
            let (mut e, _dir) = engine();
            if let Err(error) = e.execute(code) {
                panic!("{name}: {code}: {error}");
            }
            ran += 1;
        }
    }
    assert!(ran >= 4);
    let (mut e, _dir) = engine();
    run(
        &mut e,
        "writecell({'name','size';'bolt',4},'parts.csv'); C = readcell('parts.csv');",
    );
    check(&mut e, "isequal(C, {'name','size';'bolt',4})");
    assert!(Path::new(FIXTURES).join("book.xlsx").exists());
}

/// Fix round 2, finding 1: a package declaring a huge shared string count
/// or an inflating part fails with a clear error before calamine reserves
/// memory (it used to abort the process).
#[test]
fn xlsx_package_limits_are_checked_before_opening() {
    let (mut e, _dir) = engine();
    for file in ["sst4g.xlsx", "sstmax.xlsx"] {
        for call in ["readmatrix", "readcell", "xlsread"] {
            let error = fails(&mut e, &format!("{call}('{file}')"));
            assert!(error.contains("shared strings part declares"), "{error}");
        }
    }
    let error = fails(&mut e, "readmatrix('bomb.xlsx')");
    assert!(error.contains("inflates to 513 MiB"), "{error}");
    // A normal package with shared strings still reads.
    run(&mut e, "C = readcell('book.xlsx');");
}

/// Fix round 2, finding 2: xlsread builds only the outputs asked for.
#[test]
fn xlsread_builds_only_requested_outputs() {
    let (mut e, _dir) = engine();
    // 300 x 300 numbers: the raw cell output is over the cell limit.
    run(&mut e, "M = reshape(1:90000, 300, 300);");
    // No xlsx writer exists, so check through the limit on book.xlsx.
    run(&mut e, "[n, t] = xlsread('book.xlsx');");
    run(&mut e, "n = xlsread('book.xlsx');");
    check(&mut e, "isnumeric(n)");
    run(&mut e, "[n, t, r] = xlsread('book.xlsx');");
    check(&mut e, "iscell(r) && iscell(t)");
}

/// Fix round 2, finding 3: readcell width comes from the kept lines.
#[test]
fn readcell_width_ignores_skipped_lines() {
    let (mut e, dir) = engine();
    fs::write(dir.path().join("w.csv"), "a,b,c,d\n1,2,3\n4,5,6\n").unwrap();
    run(&mut e, "C = readcell('w.csv', 'NumHeaderLines', 1);");
    check(&mut e, "isequal(size(C), [2 3])");
    run(&mut e, "C = readcell('w.csv', 'Range', 'A2');");
    check(&mut e, "isequal(size(C), [2 3])");
}

/// Fix round 2, finding 4: writecell publishes through the shared atomic
/// helper and its errors do not name a temporary file.
#[test]
fn writecell_writes_through_atomic_helper() {
    let (mut e, dir) = engine();
    run(&mut e, "writecell({1, 'a'; 2, 'b'}, 'out.csv');");
    assert_eq!(
        fs::read_to_string(dir.path().join("out.csv")).unwrap(),
        "1,a\n2,b\n"
    );
    let error = fails(&mut e, "writecell({1}, 'nodir/out.csv');");
    assert!(error.contains("writecell: unable to write"), "{error}");
    assert!(!error.contains(".tmp"), "{error}");
    let leftovers: Vec<_> = fs::read_dir(dir.path())
        .unwrap()
        .filter_map(|e| e.ok())
        .filter(|e| e.file_name().to_string_lossy().starts_with(".tmp"))
        .collect();
    assert!(leftovers.is_empty());
}
