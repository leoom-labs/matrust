//! Whitespace-delimited text. Numeric fixture checked with GNU Octave 10.3 dlmread.
use matrust::{Array, Engine};
use std::fs;

fn run(e: &mut Engine, source: &str) {
    e.execute(source)
        .unwrap_or_else(|err| panic!("{source}: {err}"));
}
fn check(e: &mut Engine, expression: &str) {
    run(e, &format!("ok__ = {expression};"));
    match e.array("ok__") {
        Some(Array::Logical(a)) => assert!(
            !a.data().is_empty() && a.data().iter().all(|v| *v != 0),
            "{expression}"
        ),
        Some(Array::Numeric(a)) => assert!(
            !a.data().is_empty() && a.data().iter().all(|v| *v != 0.0),
            "{expression}"
        ),
        other => panic!("{expression}: {other:?}"),
    }
}

#[test]
fn import_space_delimited() {
    let dir = tempfile::tempdir().unwrap();
    for (name, contents) in [
        ("numbers.txt", "1.5 2.5\n3 4"),
        ("runs.txt", "  1.5 \t 2.5  \r\n \t 3   4 \t\r\n"),
        ("mixed.txt", "1 2\t3\n4\t5 6\n"),
        ("headers.txt", "a b\n1 2\n"),
        ("quoted.txt", "name value\n\"hello world\" 2\n"),
    ] {
        fs::write(dir.path().join(name), contents).unwrap();
    }
    let mut e = Engine::default();
    e.set_current_folder(dir.path()).unwrap();
    check(&mut e, "isequal(readmatrix('numbers.txt'), [1.5 2.5; 3 4])");
    for file in ["numbers.txt", "runs.txt"] {
        for delimiter in [
            "' '",
            "'space'",
            "'whitespace'",
            "{' ','\\t'}",
            "{'\\t',' '}",
        ] {
            check(
                &mut e,
                &format!("isequal(readmatrix('{file}', 'Delimiter', {delimiter}), [1.5 2.5; 3 4])"),
            );
        }
    }
    check(
        &mut e,
        "isequal(readmatrix('mixed.txt','Delimiter',{'\\t',' '}), [1 2 3;4 5 6])",
    );
    check(
        &mut e,
        "isequal(readcell('mixed.txt','Delimiter',' '), {1,2,3;4,5,6})",
    );
    run(
        &mut e,
        "T = readtable('headers.txt'); opts = detectImportOptions('headers.txt'); C = readcell('headers.txt');",
    );
    check(
        &mut e,
        "isequal(T.Properties.VariableNames, {'a','b'}) && T.a == 1 && T.b == 2",
    );
    check(
        &mut e,
        "isequal(opts.Delimiter, {' '}) && isequal(opts.VariableNames, {'a','b'})",
    );
    check(&mut e, "isequal(C, {'a','b';1,2})");
    for delimiter in ["' '", "'space'", "'whitespace'", "{' ','\\t'}"] {
        run(
            &mut e,
            &format!(
                "T = readtable('headers.txt','Delimiter',{delimiter}); C = readcell('headers.txt','Delimiter',{delimiter}); opts = detectImportOptions('headers.txt','Delimiter',{delimiter});"
            ),
        );
        check(
            &mut e,
            "isequal(T.Properties.VariableNames, {'a','b'}) && T.a == 1 && T.b == 2 && isequal(C, {'a','b';1,2}) && isequal(opts.VariableNames, {'a','b'})",
        );
    }
    run(&mut e, "C = readcell('quoted.txt');");
    check(&mut e, "isequal(C, {'name','value';'hello world',2})");
    for delimiter in ["' '", "'space'", "'whitespace'", "{' ','\\t'}"] {
        run(
            &mut e,
            &format!("writematrix([1.5 2.5;3 4], 'out.txt', 'Delimiter', {delimiter});"),
        );
        assert_eq!(
            fs::read_to_string(dir.path().join("out.txt")).unwrap(),
            "1.5 2.5\n3 4\n"
        );
        check(&mut e, "isequal(readmatrix('out.txt'), [1.5 2.5;3 4])");
    }
}

#[test]
fn import_space_delimited_preserves_tab_delimiter() {
    let dir = tempfile::tempdir().unwrap();
    fs::write(
        dir.path().join("cities.txt"),
        "city name\tvalue\nNew York \t8\n",
    )
    .unwrap();
    let mut e = Engine::default();
    e.set_current_folder(dir.path()).unwrap();
    run(
        &mut e,
        "C = readcell('cities.txt'); T = readtable('cities.txt'); opts = detectImportOptions('cities.txt');",
    );
    check(&mut e, "isequal(C, {'city name','value';'New York',8})");
    check(
        &mut e,
        "isequal(T.Properties.VariableNames, {'cityName','value'}) && T.value == 8",
    );
    check(&mut e, "isequal(opts.Delimiter, {'\\t'})");
}

#[test]
fn import_space_delimited_whole_step_regressions() {
    let dir = tempfile::tempdir().unwrap();
    let mut e = Engine::default();
    e.set_current_folder(dir.path()).unwrap();

    // Finding 1: quoted multi-line CSV fields keep quote state across lines.
    fs::write(dir.path().join("multiline.csv"), "\"x\ny\",2\n").unwrap();
    check(
        &mut e,
        "isequal(readcell('multiline.csv'), {sprintf('x\\ny'), 2})",
    );
    fs::write(
        dir.path().join("multiline_bom.csv"),
        b"\xEF\xBB\xBF\"x\r\ny\",2\r\n",
    )
    .unwrap();
    check(
        &mut e,
        "isequal(readcell('multiline_bom.csv'), {sprintf('x\\r\\ny'), 2})",
    );

    // Finding 2: auto detection keeps rows whose whitespace split agrees.
    fs::write(dir.path().join("mixed_ws.txt"), "1 2\n3\t4\n").unwrap();
    check(&mut e, "isequal(readmatrix('mixed_ws.txt'), [1 2;3 4])");

    // Finding 3: whitespace-only lines are skipped under EmptyLineRule='skip'.
    fs::write(dir.path().join("blank_ws.txt"), "1 2\n   \n3 4\n").unwrap();
    check(&mut e, "isequal(readmatrix('blank_ws.txt'), [1 2;3 4])");
    run(
        &mut e,
        "T_blank = readtable('blank_ws.txt', detectImportOptions('blank_ws.txt'));",
    );
    check(
        &mut e,
        "height(T_blank) == 2 && isequal(T_blank.Var1, [1;3])",
    );
    run(
        &mut e,
        "T_blank2 = readtable('blank_ws.txt', 'ReadVariableNames', false);",
    );
    check(
        &mut e,
        "height(T_blank2) == 2 && isequal(T_blank2.Var1, [1;3])",
    );

    // Finding 4: quoted header fields are not split in the numeric scan.
    fs::write(dir.path().join("quoted_hdr.txt"), "\"1 2\" 3\n4 5\n").unwrap();
    check(&mut e, "isequal(readmatrix('quoted_hdr.txt'), [4 5])");

    // Finding 5: headerless delimiter lists work in readmatrix.
    fs::write(dir.path().join("multi_delim.txt"), "1,2;3\n").unwrap();
    check(
        &mut e,
        "isequal(readmatrix('multi_delim.txt', 'Delimiter', {',',';'}), [1 2 3])",
    );

    // Finding 6: delimiter lists only split on listed delimiters in readcell.
    fs::write(dir.path().join("tab_city.txt"), "New York\t8\n").unwrap();
    check(
        &mut e,
        "isequal(readcell('tab_city.txt', 'Delimiter', {'tab','\\t'}), {'New York', 8})",
    );
    check(
        &mut e,
        "isequal(readcell('multi_delim.txt', 'Delimiter', {',',';'}), {1, 2, 3})",
    );

    // Finding 7: one-element lists like {' '} equal the scalar.
    fs::write(dir.path().join("one_row.txt"), "1 2\n").unwrap();
    check(
        &mut e,
        "isequal(readmatrix('one_row.txt', 'Delimiter', {' '}), [1 2])",
    );
    check(
        &mut e,
        "isequal(readcell('one_row.txt', 'Delimiter', {' '}), {1, 2})",
    );

    // Finding 8: list route validates options and pairs like scalar route.
    fs::write(dir.path().join("two_rows.txt"), "1 2\n3 4\n").unwrap();
    assert!(
        e.execute("readmatrix('two_rows.txt', 'Delimiter', {' ','\\t'}, 'NumHeaderLines', 1)")
            .is_err()
    );
    assert!(
        e.execute("readmatrix('two_rows.txt', 'Delimiter', {' ','\\t'}, 'Delimiter')")
            .is_err()
    );
}

#[test]
fn import_space_delimited_large_file_and_budget() {
    let dir = tempfile::tempdir().unwrap();
    let big = "1 2\n".repeat(300_000);
    fs::write(dir.path().join("big.txt"), big).unwrap();

    let mut e = Engine::default();
    e.set_current_folder(dir.path()).unwrap();
    check(&mut e, "isequal(size(readmatrix('big.txt')), [300000 2])");

    // Forced small memory budget returns a recoverable error without aborting.
    let mut small = Engine::with_limits(matrust::engine::Limits {
        memory_bytes: 1024 * 1024,
        ..Default::default()
    });
    small.set_current_folder(dir.path()).unwrap();
    let err = small.execute("A = readmatrix('big.txt');").unwrap_err();
    assert!(err.contains("budget"), "{err}");
}
