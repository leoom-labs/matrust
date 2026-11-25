//! table (part 1). Expected values are worked by hand from the published
//! MATLAB table documentation (semantics only; GNU Octave 10.3 has no table
//! type) and this project's display convention (help table). CSV cell values
//! of tests/fixtures/tables/people.csv agree with pandas
//! (tool/refs/tables/people_ref.py).
use matrust::{Array, Engine};
use std::fs;

const FIXTURES: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/tests/fixtures/tables");

fn output(e: &mut Engine, code: &str) -> String {
    match e.execute(code) {
        Ok(r) => r.output.concat(),
        Err(err) => panic!("{code}: {err}"),
    }
}

fn run(e: &mut Engine, code: &str) {
    output(e, code);
}

#[track_caller]
fn fails(e: &mut Engine, code: &str) -> String {
    match e.execute(code) {
        Ok(r) => panic!("{code}: no error, printed {:?}", r.output),
        Err(err) => err,
    }
}

#[track_caller]
fn fails_with(e: &mut Engine, code: &str, part: &str) {
    let err = fails(e, code);
    assert!(err.contains(part), "{code}: {err:?} lacks {part:?}");
}

/// `expr` is a nonempty all-true logical or numeric value.
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

/// A fresh engine in a temporary folder holding the CSV fixtures.
fn engine_in_folder() -> (Engine, tempfile::TempDir) {
    let dir = tempfile::tempdir().unwrap();
    for entry in fs::read_dir(FIXTURES).unwrap() {
        let entry = entry.unwrap();
        fs::copy(entry.path(), dir.path().join(entry.file_name())).unwrap();
    }
    let mut e = Engine::default();
    e.set_current_folder(dir.path()).unwrap();
    (e, dir)
}

const PEOPLE: &str = "T = table({'Ann';'Bob';'Cy'}, [31;45;7], 'VariableNames', {'Name','Age'});";

#[test]
fn constructors_take_names_from_variables_and_options() {
    let mut e = Engine::default();
    run(
        &mut e,
        "Age = [31; 45]; Name = {'Ann'; 'Bob'}; T = table(Age, Name);",
    );
    check(&mut e, "istable(T) && ~istable(5)");
    check(
        &mut e,
        "isequal(T.Properties.VariableNames, {'Age', 'Name'})",
    );
    check(
        &mut e,
        "isequal(size(T), [2 2]) && height(T) == 2 && width(T) == 2",
    );
    // Expressions take Var<position>.
    run(&mut e, "U = table(Age, [1; 2], Age + 1);");
    check(
        &mut e,
        "isequal(U.Properties.VariableNames, {'Age', 'Var2', 'Var3'})",
    );
    run(
        &mut e,
        "V = table([1; 2], 'VariableNames', {'x'}, 'RowNames', {'a', 'b'});",
    );
    check(
        &mut e,
        "isequal(V.Properties.RowNames, {'a'; 'b'}) && isequal(V.x, [1; 2])",
    );
    run(
        &mut e,
        "W = table([1; 2], [3 4; 5 6], 'VariableNames', [\"p\", \"q\"]);",
    );
    check(
        &mut e,
        "isequal(W.q, [3 4; 5 6]) && width(W) == 2 && numel(W) == 4",
    );
    run(&mut e, "E = table();");
    check(
        &mut e,
        "isequal(size(E), [0 0]) && isempty(E) && istable(E)",
    );
    check(&mut e, "strcmp(class(T), 'table')");
    check(
        &mut e,
        "isequal(T.Properties.DimensionNames, {'Row', 'Variables'})",
    );
    fails_with(
        &mut e,
        "table([1; 2], [1; 2; 3])",
        "All table variables must have the same number of rows.",
    );
    fails_with(
        &mut e,
        "x = [1; 2]; table(x, x)",
        "Duplicate table variable name: 'x'.",
    );
    fails_with(
        &mut e,
        "table([1; 2], [3; 4], 'VariableNames', {'a', 'a'})",
        "Duplicate table variable name: 'a'.",
    );
    fails_with(
        &mut e,
        "table([1; 2], 'VariableNames', {'a', 'b'})",
        "one name for each variable",
    );
    fails_with(
        &mut e,
        "table([1; 2], 'VariableNames', {''})",
        "Invalid table variable name",
    );
    fails_with(
        &mut e,
        "table([1; 2], 'VariableNames', {repmat('a', 1, 64)})",
        "Invalid table variable name",
    );
    run(
        &mut e,
        "T = table([1; 2], [3; 4], 'VariableNames', {'observed(%)', 'Pol.'});",
    );
    check(
        &mut e,
        "isequal(T.Properties.VariableNames, {'observed(%)', 'Pol.'}) && isequal(T.('observed(%)'), [1; 2]) && isequal(T.('Pol.'), [3; 4])",
    );
    run(
        &mut e,
        "T = table([1; 2], [3; 4], 'VariableNames', {'1a', 'b c'});",
    );
    check(
        &mut e,
        "isequal(T.Properties.VariableNames, {'1a', 'b c'}) && isequal(T.('1a'), [1; 2]) && isequal(T.('b c'), [3; 4])",
    );
    fails_with(
        &mut e,
        "table([1; 2], 'RowNames', {'r', 'r'})",
        "Duplicate row name: 'r'.",
    );
}

#[test]
fn array_cell_struct_conversions_and_back() {
    let mut e = Engine::default();
    run(&mut e, "M = [1 2; 3 4; 5 6]; T = array2table(M);");
    check(
        &mut e,
        "isequal(T.Properties.VariableNames, {'M1', 'M2'}) && isequal(T.M2, [2; 4; 6])",
    );
    run(&mut e, "T = array2table(magic(3));");
    check(
        &mut e,
        "isequal(T.Properties.VariableNames, {'Var1', 'Var2', 'Var3'})",
    );
    run(
        &mut e,
        "T = array2table([1 2; 3 4], 'VariableNames', {'a', 'b'}, 'RowNames', {'r1', 'r2'});",
    );
    check(
        &mut e,
        "isequal(T.b, [2; 4]) && isequal(T.Properties.RowNames, {'r1'; 'r2'})",
    );
    check(&mut e, "isequal(table2array(T), [1 2; 3 4])");
    // cell2table: scalar numbers -> double column, text -> cell column.
    run(
        &mut e,
        "C = {'Ann', 31, true; 'Bob', 45, false}; T = cell2table(C, 'VariableNames', {'Name', 'Age', 'Ok'});",
    );
    check(
        &mut e,
        "isequal(T.Age, [31; 45]) && strcmp(class(T.Age), 'double')",
    );
    check(
        &mut e,
        "isequal(T.Name, {'Ann'; 'Bob'}) && islogical(T.Ok) && isequal(T.Ok, [true; false])",
    );
    check(
        &mut e,
        "isequal(T.Properties.VariableNames, {'Name', 'Age', 'Ok'})",
    );
    run(&mut e, "T2 = cell2table({1, 'x'; 'y', 2});");
    check(&mut e, "iscell(T2.Var1) && iscell(T2.Var2)");
    // table2cell holds the text itself for cellstr variables.
    check(
        &mut e,
        "isequal(table2cell(T), {'Ann', 31, true; 'Bob', 45, false})",
    );
    // struct arrays: one row per element.
    run(
        &mut e,
        "S = struct('a', {1, 2, 3}, 'b', {'x', 'yy', 'z'}); T = struct2table(S);",
    );
    check(
        &mut e,
        "isequal(T.a, [1; 2; 3]) && isequal(T.b, {'x'; 'yy'; 'z'})",
    );
    run(&mut e, "R = table2struct(T);");
    check(
        &mut e,
        "isequal(size(R), [3 1]) && R(2).a == 2 && strcmp(R(2).b, 'yy')",
    );
    run(&mut e, "Q = table2struct(T, 'ToScalar', true);");
    check(
        &mut e,
        "isequal(Q.a, [1; 2; 3]) && isequal(Q.b, {'x'; 'yy'; 'z'})",
    );
    // scalar struct: each field one variable with its rows.
    run(
        &mut e,
        "s.p = [1; 2]; s.q = [3 4; 5 6]; T = struct2table(s);",
    );
    check(&mut e, "height(T) == 2 && isequal(T.q, [3 4; 5 6])");
    run(&mut e, "s.r = [1; 2; 3];");
    fails_with(&mut e, "struct2table(s)", "same number of rows");
    // table2array errors on incompatible variable types.
    run(&mut e, PEOPLE);
    fails_with(
        &mut e,
        "table2array(T)",
        "Unable to concatenate the table variables 'Name' and 'Age'",
    );
    run(&mut e, "L = table([1; 2], logical([1; 0]));");
    check(&mut e, "isequal(table2array(L), [1 1; 2 0])");
}

#[test]
fn dot_access_set_add_delete() {
    let mut e = Engine::default();
    run(&mut e, PEOPLE);
    check(
        &mut e,
        "T.Age(2) == 45 && isequal(T.('Name'), {'Ann'; 'Bob'; 'Cy'})",
    );
    run(&mut e, "n = 'Age'; a = T.(n);");
    check(&mut e, "isequal(a, [31; 45; 7]) && T.Age(end) == 7");
    check(&mut e, "strcmp(T.Name{2}, 'Bob')");
    run(&mut e, "T.Age(2) = 46;");
    check(&mut e, "isequal(T.Age, [31; 46; 7])");
    run(&mut e, "T.Name{3} = 'Cyd';");
    check(&mut e, "strcmp(T.Name{3}, 'Cyd')");
    run(&mut e, "T.Age = [1; 2; 3];");
    check(&mut e, "isequal(T.Age, [1; 2; 3])");
    run(&mut e, "T.Score = [0.5; 0.25; 1];");
    check(
        &mut e,
        "isequal(T.Properties.VariableNames, {'Name', 'Age', 'Score'}) && T.Score(2) == 0.25",
    );
    run(&mut e, "T.M = [1 2; 3 4; 5 6]; v = T.M(2, 1);");
    check(&mut e, "v == 3 && isequal(T.M(:, 2), [2; 4; 6])");
    run(&mut e, "T.Age = [];");
    check(
        &mut e,
        "isequal(T.Properties.VariableNames, {'Name', 'Score', 'M'})",
    );
    fails_with(
        &mut e,
        "T.Bad = [1; 2]",
        "the number of rows must match the height of the table",
    );
    fails_with(
        &mut e,
        "x = T.Nope",
        "Unrecognized table variable name 'Nope'.",
    );
    fails_with(
        &mut e,
        "T.Nope = []",
        "Unrecognized table variable name 'Nope'.",
    );
    // An empty table takes the height of its first variable.
    run(&mut e, "E = table(); E.x = [1; 2; 3];");
    check(&mut e, "isequal(size(E), [3 1])");
}

#[test]
fn paren_selection_with_every_selector_kind() {
    let mut e = Engine::default();
    run(
        &mut e,
        "T = table([10; 20; 30; 40], {'a'; 'b'; 'c'; 'd'}, [1; 0; 1; 0], 'VariableNames', {'x', 'y', 'z'});",
    );
    run(&mut e, "S = T(2:3, :);");
    check(
        &mut e,
        "istable(S) && isequal(S.x, [20; 30]) && isequal(S.y, {'b'; 'c'})",
    );
    run(&mut e, "S = T([4 1], [3 1]);");
    check(
        &mut e,
        "isequal(S.Properties.VariableNames, {'z', 'x'}) && isequal(S.x, [40; 10])",
    );
    run(&mut e, "S = T(logical([1 0 0 1]), logical([0 1 1]));");
    check(&mut e, "isequal(S.y, {'a'; 'd'}) && width(S) == 2");
    run(&mut e, "S = T(end, 'y');");
    check(&mut e, "isequal(S.y, {'d'}) && isequal(size(S), [1 1])");
    run(&mut e, "S = T(:, {'z', 'x'});");
    check(
        &mut e,
        "isequal(S.Properties.VariableNames, {'z', 'x'}) && height(S) == 4",
    );
    run(&mut e, "S = T(:, [\"y\", \"x\"]);");
    check(&mut e, "isequal(S.Properties.VariableNames, {'y', 'x'})");
    run(&mut e, "S = T(T.x > 15, :);");
    check(&mut e, "isequal(S.x, [20; 30; 40])");
    run(&mut e, "S = T(2:end, 2:end);");
    check(&mut e, "isequal(size(S), [3 2])");
    run(&mut e, "S = T([], :);");
    check(&mut e, "isequal(size(S), [0 3])");
    fails_with(&mut e, "T(5, :)", "Row index exceeds table dimensions.");
    fails_with(
        &mut e,
        "T(1, 4)",
        "Variable index exceeds table dimensions.",
    );
    fails_with(&mut e, "T(1, 'w')", "Unrecognized table variable name 'w'.");
    fails_with(
        &mut e,
        "T(2)",
        "Subscripting into a table using one subscript is not supported",
    );
    fails_with(&mut e, "T('r1', :)", "no row names");
}

#[test]
fn brace_reads_and_writes() {
    let mut e = Engine::default();
    run(
        &mut e,
        "T = table([1; 2; 3], [4; 5; 6], {'a'; 'b'; 'c'}, 'VariableNames', {'x', 'y', 'c'});",
    );
    check(&mut e, "isequal(T{:, {'x', 'y'}}, [1 4; 2 5; 3 6])");
    check(&mut e, "T{2, 'y'} == 5 && isequal(T{:, 1}, [1; 2; 3])");
    check(&mut e, "isequal(T{2:3, 'c'}, {'b'; 'c'})");
    fails_with(
        &mut e,
        "T{1, :}",
        "Unable to concatenate the table variables 'x' and 'c'",
    );
    run(&mut e, "T{2, 'y'} = 50;");
    check(&mut e, "isequal(T.y, [4; 50; 6])");
    run(&mut e, "T{1:2, {'x', 'y'}} = [7 8; 9 10];");
    check(
        &mut e,
        "isequal(T.x, [7; 9; 3]) && isequal(T.y, [8; 10; 6])",
    );
    run(&mut e, "T{:, 'x'} = 0;");
    check(&mut e, "isequal(T.x, [0; 0; 0])");
    run(&mut e, "T{3, 'c'} = 'zz';");
    check(&mut e, "isequal(T.c, {'a'; 'b'; 'zz'})");
    run(&mut e, "T{1, 'c'} = {'q'};");
    check(&mut e, "strcmp(T.c{1}, 'q')");
    run(&mut e, "N = table([1; 2], [3; 4]);");
    check(&mut e, "isequal(N.Variables, [1 3; 2 4])");
}

#[test]
fn deleting_rows_and_variables_and_appending_rows() {
    let mut e = Engine::default();
    run(&mut e, PEOPLE);
    run(&mut e, "T(2, :) = [];");
    check(
        &mut e,
        "isequal(T.Name, {'Ann'; 'Cy'}) && isequal(T.Age, [31; 7])",
    );
    run(&mut e, "T(end+1, :) = {'Dan', 52};");
    check(
        &mut e,
        "height(T) == 3 && strcmp(T.Name{3}, 'Dan') && T.Age(3) == 52",
    );
    run(&mut e, "T(:, 'Name') = [];");
    check(
        &mut e,
        "isequal(T.Properties.VariableNames, {'Age'}) && isequal(T.Age, [31; 7; 52])",
    );
    run(&mut e, "T(logical([1 0 1]), :) = [];");
    check(&mut e, "isequal(T.Age, 7)");
    run(&mut e, PEOPLE);
    run(&mut e, "T(2, :) = {'Bo', 1};");
    check(
        &mut e,
        "isequal(T.Name, {'Ann'; 'Bo'; 'Cy'}) && isequal(T.Age, [31; 1; 7])",
    );
    run(&mut e, "T([1 3], :) = T([3 1], :);");
    check(
        &mut e,
        "isequal(T.Name, {'Cy'; 'Bo'; 'Ann'}) && isequal(T.Age, [7; 1; 31])",
    );
    run(&mut e, "T(1, 'Age') = {100};");
    check(&mut e, "T.Age(1) == 100");
    // A right-side table's variables are matched by name, in any order.
    run(
        &mut e,
        "R = table([8; 9], {'P'; 'Q'}, 'VariableNames', {'Age', 'Name'}); T(1:2, :) = R;",
    );
    check(
        &mut e,
        "isequal(T.Name, {'P'; 'Q'; 'Ann'}) && isequal(T.Age, [8; 9; 31])",
    );
    run(
        &mut e,
        "T(3, {'Age', 'Name'}) = table({'Z'}, 4, 'VariableNames', {'Name', 'Age'});",
    );
    check(&mut e, "strcmp(T.Name{3}, 'Z') && T.Age(3) == 4");
    fails_with(
        &mut e,
        "T(1, :) = table({'x'}, 1, 'VariableNames', {'Name', 'Years'})",
        "The right side table's variable 'Years' is not one of the variables being assigned to.",
    );
    fails_with(
        &mut e,
        "T(1, 'Age') = table(1, 'VariableNames', {'Name'})",
        "variable 'Name' is not one of the variables being assigned to",
    );
    check(
        &mut e,
        "isequal(T.Name, {'P'; 'Q'; 'Z'}) && isequal(T.Age, [8; 9; 4])",
    );
    fails_with(
        &mut e,
        "T(1, :) = 5",
        "must be another table or a cell array",
    );
    fails_with(
        &mut e,
        "T(1, :) = {1}",
        "1x1 but the assignment selects 1x2",
    );
    fails_with(&mut e, "T(2, 1) = []", "T(rows, :) = []");
    fails_with(&mut e, "T(5, 'Age') = {1}", "assign every variable");
}

#[test]
fn row_names_index_and_follow_rows() {
    let mut e = Engine::default();
    run(
        &mut e,
        "T = table([1; 2; 3], {'x'; 'y'; 'z'}, 'VariableNames', {'n', 't'}, 'RowNames', {'a', 'b', 'c'});",
    );
    run(&mut e, "S = T({'c', 'a'}, :);");
    check(
        &mut e,
        "isequal(S.n, [3; 1]) && isequal(S.Properties.RowNames, {'c'; 'a'})",
    );
    check(&mut e, "T{'b', 'n'} == 2 && isequal(T('b', :).t, {'y'})");
    run(&mut e, "S = T(\"c\", :);");
    check(&mut e, "S.n == 3");
    check(&mut e, "isequal(T.Row, {'a'; 'b'; 'c'})");
    run(&mut e, "S = T([1 1], :);");
    check(&mut e, "isequal(S.Properties.RowNames, {'a'; 'a_1'})");
    run(&mut e, "U = sortrows(T, 'n', 'descend');");
    check(&mut e, "isequal(U.Properties.RowNames, {'c'; 'b'; 'a'})");
    run(&mut e, "U = sortrows(T, 'Row', 'descend');");
    check(&mut e, "isequal(U.n, [3; 2; 1])");
    fails_with(&mut e, "T('q', :)", "Unrecognized row name 'q'.");
    run(&mut e, "T(end+1, :) = {4, 'w'};");
    check(
        &mut e,
        "isequal(T.Properties.RowNames, {'a'; 'b'; 'c'; 'Row4'})",
    );
}

#[test]
fn properties_get_set_and_validate() {
    let mut e = Engine::default();
    run(&mut e, "T = table([1; 2], [3; 4]);");
    run(&mut e, "P = T.Properties;");
    check(
        &mut e,
        "isstruct(P) && isequal(P.VariableNames, {'Var1', 'Var2'})",
    );
    check(
        &mut e,
        "isequal(fieldnames(P), {'Description'; 'DimensionNames'; 'VariableNames'; 'VariableDescriptions'; 'VariableUnits'; 'RowNames'})",
    );
    check(
        &mut e,
        "isempty(P.RowNames) && isempty(P.Description) && isempty(P.VariableUnits)",
    );
    run(&mut e, "T.Properties.VariableNames = {'a', 'b'};");
    check(&mut e, "isequal(T.a, [1; 2]) && isequal(T.b, [3; 4])");
    run(&mut e, "T.Properties.VariableNames{2} = 'c';");
    check(&mut e, "isequal(T.Properties.VariableNames, {'a', 'c'})");
    run(&mut e, "T.Properties.VariableNames = [\"p\", \"q\"];");
    check(&mut e, "isequal(T.Properties.VariableNames, {'p', 'q'})");
    run(&mut e, "T.Properties.RowNames = {'r1', 'r2'};");
    check(&mut e, "T{'r2', 'q'} == 4");
    run(&mut e, "T.Properties.RowNames = {};");
    check(&mut e, "isempty(T.Properties.RowNames)");
    run(&mut e, "T.Properties.Description = 'Two columns';");
    check(&mut e, "strcmp(T.Properties.Description, 'Two columns')");
    run(&mut e, "T.Properties.VariableUnits = {'m', 's'};");
    check(&mut e, "isequal(T.Properties.VariableUnits, {'m', 's'})");
    run(
        &mut e,
        "T.Properties.VariableDescriptions = {'length', ''};",
    );
    check(
        &mut e,
        "isequal(T.Properties.VariableDescriptions, {'length', ''})",
    );
    run(&mut e, "T.Properties.DimensionNames = {'Obs', 'Data'};");
    check(
        &mut e,
        "isequal(T.Properties.DimensionNames, {'Obs', 'Data'})",
    );
    // Units follow their variables through selection.
    run(&mut e, "S = T(:, [2 1]);");
    check(&mut e, "isequal(S.Properties.VariableUnits, {'s', 'm'})");
    fails_with(
        &mut e,
        "T.Properties.VariableNames = {'a'}",
        "one name for each variable",
    );
    fails_with(
        &mut e,
        "T.Properties.VariableNames = {'a', 'a'}",
        "Duplicate table variable name: 'a'.",
    );
    fails_with(
        &mut e,
        "T.Properties.VariableNames = {'a', ''}",
        "Invalid table variable name: ''",
    );
    fails_with(
        &mut e,
        "T.Properties.VariableNames = {'a', repmat('x', 1, 64)}",
        "Invalid table variable name",
    );
    fails_with(
        &mut e,
        "T.Properties.RowNames = {'x', 'x'}",
        "Duplicate row name: 'x'.",
    );
    fails_with(
        &mut e,
        "T.Properties.RowNames = {'x'}",
        "number of row names must match",
    );
    fails_with(
        &mut e,
        "T.Properties.VariableUnits = {'m'}",
        "one entry per table variable",
    );
    fails_with(
        &mut e,
        "T.Properties.Foo = 1",
        "Unrecognized table property 'Foo'.",
    );
    // A failed set leaves the table unchanged.
    check(&mut e, "isequal(T.Properties.VariableNames, {'p', 'q'})");
    // Arbitrary non-empty variable labels (up to 63 chars) work on update and renamevars.
    run(&mut e, "T.Properties.VariableNames = {'1a', 'b c'};");
    check(
        &mut e,
        "isequal(T.Properties.VariableNames, {'1a', 'b c'}) && isequal(T.('1a'), [1; 2]) && isequal(T.('b c'), [3; 4])",
    );
    run(
        &mut e,
        "T = renamevars(T, {'1a', 'b c'}, {'observed(%)', 'Pol.'});",
    );
    check(
        &mut e,
        "isequal(T.Properties.VariableNames, {'observed(%)', 'Pol.'}) && isequal(T.('observed(%)'), [1; 2]) && isequal(T.('Pol.'), [3; 4])",
    );
    run(&mut e, "T.('new col') = [5; 6];");
    check(
        &mut e,
        "isequal(T.Properties.VariableNames, {'observed(%)', 'Pol.', 'new col'}) && isequal(T.('new col'), [5; 6])",
    );
}

#[test]
fn writes_copy_on_write() {
    let mut e = Engine::default();
    run(&mut e, PEOPLE);
    run(
        &mut e,
        "B = T; T.Age(1) = 9; T.Name{2} = 'Z'; T(3, :) = []; T.Properties.VariableNames{1} = 'N';",
    );
    check(
        &mut e,
        "isequal(B.Age, [31; 45; 7]) && isequal(B.Name, {'Ann'; 'Bob'; 'Cy'})",
    );
    check(
        &mut e,
        "isequal(B.Properties.VariableNames, {'Name', 'Age'}) && height(B) == 3",
    );
    check(&mut e, "isequal(T.Age, [9; 45]) && strcmp(T.N{2}, 'Z')");
    run(&mut e, "C = B; C{1, 'Age'} = 0;");
    check(&mut e, "B.Age(1) == 31 && C.Age(1) == 0");
}

const MIXED: &str = "d = datetime(2024, [3; 1; 2], 1); du = hours([5; 1; 3]); \
T = table([2; 1; 2], logical([1; 0; 1]), int32([7; 8; 9]), {'c'; 'a'; 'b'}, [\"z\"; \"x\"; \"y\"], ['ab'; 'cd'; 'ef'], d, du, [1 2; 3 4; 5 6], \
'VariableNames', {'n', 'l', 'i', 'c', 's', 'ch', 'd', 'du', 'm'});";

#[test]
fn mixed_variables_index_and_concatenate() {
    let mut e = Engine::default();
    run(&mut e, MIXED);
    run(&mut e, "S = T(2:3, :);");
    check(
        &mut e,
        "isequal(S.n, [1; 2]) && isequal(S.l, logical([0; 1])) && isequal(S.i, int32([8; 9]))",
    );
    check(
        &mut e,
        "strcmp(class(S.i), 'int32') && isequal(S.c, {'a'; 'b'}) && isequal(S.s, [\"x\"; \"y\"])",
    );
    check(
        &mut e,
        "isequal(S.ch, ['cd'; 'ef']) && isequal(S.d, datetime(2024, [1; 2], 1))",
    );
    check(
        &mut e,
        "isequal(S.du, hours([1; 3])) && isequal(S.m, [3 4; 5 6])",
    );
    check(
        &mut e,
        "isequal(T{3, 'm'}, [5 6]) && isequal(T{:, {'n', 'm'}}, [2 1 2; 1 3 4; 2 5 6])",
    );
    check(
        &mut e,
        "isequal(T{:, {'n', 'l', 'i'}}, int32([2 1 7; 1 0 8; 2 1 9]))",
    );
    // Vertical: names in another order follow the first table.
    run(&mut e, "V = [T; T(:, [9 8 7 6 5 4 3 2 1])];");
    check(
        &mut e,
        "height(V) == 6 && isequal(V.Properties.VariableNames, T.Properties.VariableNames)",
    );
    check(
        &mut e,
        "isequal(V.d, [d; d]) && isequal(V.s, [\"z\"; \"x\"; \"y\"; \"z\"; \"x\"; \"y\"])",
    );
    check(
        &mut e,
        "isequal(V.m, [1 2; 3 4; 5 6; 1 2; 3 4; 5 6]) && isequal(V.ch(4, :), 'ab')",
    );
    check(
        &mut e,
        "strcmp(class(V.i), 'int32') && isequal(V.du, [du; du])",
    );
    run(
        &mut e,
        "T1 = table([1; 2], {'a'; 'b'}, 'VariableNames', {'x', 'y'}); T2 = table({'c'}, 3, 'VariableNames', {'y', 'x'});",
    );
    run(&mut e, "V = [T1; T2];");
    check(
        &mut e,
        "isequal(V.x, [1; 2; 3]) && isequal(V.y, {'a'; 'b'; 'c'})",
    );
    check(
        &mut e,
        "isequal(vertcat(T1, T2), V) && isequal([table(); T1], T1)",
    );
    // Horizontal: same height, distinct names.
    run(&mut e, "H = [T1, table([5; 6], 'VariableNames', {'z'})];");
    check(
        &mut e,
        "isequal(H.Properties.VariableNames, {'x', 'y', 'z'}) && isequal(H.z, [5; 6])",
    );
    check(
        &mut e,
        "isequal(horzcat(T1, table([5; 6], 'VariableNames', {'z'})), H)",
    );
    fails_with(&mut e, "[T1, T1]", "Duplicate table variable name: 'x'.");
    fails_with(&mut e, "[T1, table([1; 2; 3])]", "same number of rows");
    fails_with(&mut e, "[T1; table([1; 2], [3; 4])]", "same variable names");
    fails_with(
        &mut e,
        "[T1; 5]",
        "Tables can only be concatenated with other tables.",
    );
}

#[test]
fn sortrows_head_tail_and_isequal() {
    let mut e = Engine::default();
    run(&mut e, MIXED);
    // n = [2;1;2], d = Mar, Jan, Feb: n ascending, then d ascending.
    run(&mut e, "[S, I] = sortrows(T, {'n', 'd'});");
    check(
        &mut e,
        "isequal(I, [2; 3; 1]) && isequal(S.d, datetime(2024, [1; 2; 3], 1))",
    );
    run(
        &mut e,
        "[S, I] = sortrows(T, {'n', 'd'}, {'ascend', 'descend'});",
    );
    check(&mut e, "isequal(I, [2; 1; 3])");
    run(&mut e, "[S, I] = sortrows(T, 'c', 'descend');");
    check(
        &mut e,
        "isequal(I, [1; 3; 2]) && isequal(S.c, {'c'; 'b'; 'a'})",
    );
    run(&mut e, "[S, I] = sortrows(T, -1);");
    check(&mut e, "isequal(I, [1; 3; 2])");
    run(&mut e, "[S, I] = sortrows(T, {'du'});");
    check(
        &mut e,
        "isequal(I, [2; 3; 1]) && isequal(S.du, hours([1; 3; 5]))",
    );
    run(&mut e, "[S, I] = sortrows(T, 's');");
    check(&mut e, "isequal(I, [2; 3; 1])");
    run(&mut e, "[S, I] = sortrows(T, 'ch', 'descend');");
    check(&mut e, "isequal(I, [3; 2; 1])");
    run(&mut e, "[S, I] = sortrows(T, [2 3]);");
    check(&mut e, "isequal(I, [2; 1; 3])");
    // All variables in order: n ties rows 1 and 3, l ties too, i (7 < 9) decides.
    run(&mut e, "S = sortrows(T);");
    check(
        &mut e,
        "isequal(S.n, [1; 2; 2]) && isequal(S.d, datetime(2024, [1; 3; 2], 1))",
    );
    // NaN sorts last ascending, first descending.
    run(
        &mut e,
        "N = table([2; NaN; 1]); [~, I] = sortrows(N); [~, J] = sortrows(N, 1, 'descend');",
    );
    check(&mut e, "isequal(I, [3; 1; 2]) && isequal(J, [2; 1; 3])");
    // Datetimes 1 ns apart (one f64 step near 2024 is ~8 us) keep their
    // order; NaT sorts last ascending, first descending.
    run(
        &mut e,
        "t0 = datetime(2024, 6, 1, 12, 0, 0); t1 = datetime(2024, 6, 1, 12, 0, 1e-9);",
    );
    check(&mut e, "t0 < t1");
    run(
        &mut e,
        "N = table([t1; t0]); [~, I] = sortrows(N); [~, J] = sortrows(N, 'descend');",
    );
    check(&mut e, "isequal(I, [2; 1]) && isequal(J, [1; 2])");
    run(
        &mut e,
        "N = table([t0; t1]); [~, I] = sortrows(N); [~, J] = sortrows(N, 'descend');",
    );
    check(&mut e, "isequal(I, [1; 2]) && isequal(J, [2; 1])");
    run(
        &mut e,
        "N = table([t1; NaT; t0]); [~, I] = sortrows(N); [~, J] = sortrows(N, 'descend');",
    );
    check(&mut e, "isequal(I, [3; 1; 2]) && isequal(J, [2; 1; 3])");
    fails_with(
        &mut e,
        "sortrows(T, 'nope')",
        "Unrecognized table variable name 'nope'.",
    );
    // head and tail.
    run(&mut e, "L = table((1:10)', 'VariableNames', {'k'});");
    check(
        &mut e,
        "isequal(head(L).k, (1:8)') && isequal(tail(L, 3).k, [8; 9; 10])",
    );
    check(
        &mut e,
        "isequal(head(L, 20).k, (1:10)') && height(head(L, 0)) == 0",
    );
    check(&mut e, "isequal(head(magic(4), 2), [16 2 3 13; 5 11 10 8])");
    // isequal: names, row names and values.
    check(&mut e, "isequal(T, T) && ~isequal(T, T(:, [2 1 3:9]))");
    run(&mut e, "U = T; U.n(1) = 5;");
    check(&mut e, "~isequal(T, U)");
    run(
        &mut e,
        "A = table([1; 2], 'RowNames', {'a', 'b'}); B = table([1; 2]);",
    );
    check(&mut e, "~isequal(A, B) && isequal(A.Var1, B.Var1)");
}

#[test]
fn display_is_exact() {
    let mut e = Engine::default();
    assert_eq!(
        output(
            &mut e,
            "T = table({'Ann';'Bob';'Cy'}, [31;45;7], 'VariableNames', {'Name','Age'})"
        ),
        "\nT =\n\n  3x2 table\n\n    Name       Age\n    _______    ___\n\n    {'Ann'}     31\n    {'Bob'}     45\n    {'Cy'}       7\n"
    );
    assert_eq!(
        output(&mut e, "disp(T)"),
        "  3x2 table\n\n    Name       Age\n    _______    ___\n\n    {'Ann'}     31\n    {'Bob'}     45\n    {'Cy'}       7\n"
    );
    // Row names: a first column without a header.
    assert_eq!(
        output(
            &mut e,
            "R = table([1.5; 2], 'VariableNames', {'x'}, 'RowNames', {'r1', 'row2'})"
        ),
        "\nR =\n\n  2x1 table\n\n            x\n            ______\n\n    r1      1.5000\n    row2         2\n"
    );
    // A two-column variable under one header, logical and string values.
    assert_eq!(
        output(
            &mut e,
            "M = table([1 2; 3 4], logical([1; 0]), [\"ab\"; \"c\"], 'VariableNames', {'M', 'ok', 's'})"
        ),
        "\nM =\n\n  2x3 table\n\n    M         ok       s\n    ______    _____    ____\n\n    1    2     true    \"ab\"\n    3    4    false    \"c\"\n"
    );
    // A long string shows in full (no preview cut); the column widens.
    let long = "abcdefghijklmnopqrstuvwxyz0123456789ABCD";
    assert_eq!(long.chars().count(), 40);
    assert_eq!(
        output(
            &mut e,
            &format!("L = table([\"{long}\"; \"x\"], 'VariableNames', {{'s'}})")
        ),
        format!(
            "\nL =\n\n  2x1 table\n\n    s\n    {}\n\n    \"{long}\"\n    \"x\"\n",
            "_".repeat(42)
        )
    );
    assert_eq!(
        output(&mut e, "E = table()"),
        "\nE =\n\n  0x0 empty table\n"
    );
    assert_eq!(output(&mut e, "disp(E)"), "  0x0 empty table\n");
    assert_eq!(
        output(&mut e, "Z = table(zeros(0, 1), zeros(0, 1))"),
        "\nZ =\n\n  0x2 empty table\n"
    );
    // datetime and duration show their text, left-aligned.
    assert_eq!(
        output(
            &mut e,
            "D = table(datetime(2024, 1, [1; 2]), hours([1; 10]), 'VariableNames', {'t', 'h'})"
        ),
        "\nD =\n\n  2x2 table\n\n    t              h\n    ___________    _____\n\n    01-Jan-2024    1 hr\n    02-Jan-2024    10 hr\n"
    );
}

#[test]
fn queries_class_and_whos() {
    let mut e = Engine::default();
    run(&mut e, PEOPLE);
    check(
        &mut e,
        "isequal(size(T), [3 2]) && size(T, 1) == 3 && size(T, 2) == 2 && size(T, 3) == 1",
    );
    run(&mut e, "[r, c] = size(T);");
    check(&mut e, "r == 3 && c == 2");
    check(
        &mut e,
        "numel(T) == 6 && ~isempty(T) && ndims(T) == 2 && length(T) == 3",
    );
    check(&mut e, "height(ones(4, 2)) == 4 && width(ones(4, 2)) == 2");
    let whos = output(&mut e, "whos");
    let line = whos
        .lines()
        .find(|l| l.trim_start().starts_with("T "))
        .unwrap();
    assert!(line.contains("table"), "{whos}");
    assert!(line.contains("3 x 2"), "{whos}");
}

#[test]
fn readtable_names_types_and_options() {
    let (mut e, _dir) = engine_in_folder();
    run(&mut e, "T = readtable('people.csv');");
    check(
        &mut e,
        "isequal(T.Properties.VariableNames, {'Name', 'Age', 'Height', 'LastName', 'x1st', 'Age_1'})",
    );
    check(
        &mut e,
        "isequal(T.Name, {'Ann'; 'Lee, Jr.'; 'Cy'}) && isequal(T.Age, [31; 45; 7])",
    );
    check(
        &mut e,
        "T.Height(1) == 1.6 && isnan(T.Height(2)) && T.Height(3) == 1.2",
    );
    check(
        &mut e,
        "isequal(T.LastName, {'Smith'; 'O\"Brien'; 'Ng'}) && isequal(T.Age_1, [1; 2; 3])",
    );
    run(&mut e, "S = readtable('people.csv', 'TextType', 'string');");
    check(
        &mut e,
        "isstring(S.Name) && S.Name(2) == \"Lee, Jr.\" && isequal(S.Age, [31; 45; 7])",
    );
    run(
        &mut e,
        "H = readtable('headers.txt', 'NumHeaderLines', 2, 'Delimiter', ';');",
    );
    check(
        &mut e,
        "isequal(H.Properties.VariableNames, {'a', 'b'}) && isequal(H.a, [1; 2]) && isequal(H.b, {'x'; 'y'})",
    );
    run(
        &mut e,
        "N = readtable('noheader.csv', 'ReadVariableNames', false);",
    );
    check(
        &mut e,
        "isequal(N.Properties.VariableNames, {'Var1', 'Var2'}) && isequal(N{:, :}, [1 2; 3 4])",
    );
    // Range, spreadsheets and the import options are tests/import_options.rs.
    fails_with(
        &mut e,
        "readtable('people.csv', 'Format', '%s%f')",
        "unsupported option 'Format'",
    );
    fails_with(&mut e, "readtable('nope.csv')", "readtable: unable to open");
    fails_with(
        &mut e,
        "readtable('book.xlsx')",
        "Unable to find or open 'book.xlsx'",
    );
}

#[test]
fn writetable_round_trip_and_row_names() {
    let (mut e, dir) = engine_in_folder();
    run(
        &mut e,
        "T = table([1.5; -2; NaN], {'plain'; 'a,b'; 'say \"hi\"'}, logical([1; 0; 1]), 'VariableNames', {'x', 'txt', 'ok'});",
    );
    run(&mut e, "writetable(T, 'out.csv');");
    let text = fs::read_to_string(dir.path().join("out.csv")).unwrap();
    assert_eq!(
        text,
        "x,txt,ok\n1.5,plain,1\n-2,\"a,b\",0\nNaN,\"say \"\"hi\"\"\",1\n"
    );
    run(&mut e, "R = readtable('out.csv');");
    check(
        &mut e,
        "isequaln(R.x, T.x) && isequal(R.txt, T.txt) && isequal(R.ok, [1; 0; 1])",
    );
    // Numeric + text round trip is exact.
    run(
        &mut e,
        "P = table([1; 2; 3], {'a'; 'b'; 'c'}, 'VariableNames', {'n', 's'}); writetable(P, 'p.txt'); Q = readtable('p.txt');",
    );
    check(&mut e, "isequal(P, Q)");
    run(
        &mut e,
        "W = table([1; 2], 'VariableNames', {'v'}, 'RowNames', {'r1', 'r2'}); writetable(W, 'w.csv', 'WriteRowNames', true);",
    );
    let text = fs::read_to_string(dir.path().join("w.csv")).unwrap();
    assert_eq!(text, "Row,v\nr1,1\nr2,2\n");
    run(
        &mut e,
        "writetable(W, 'w2.csv', 'WriteVariableNames', false, 'Delimiter', ';');",
    );
    assert_eq!(
        fs::read_to_string(dir.path().join("w2.csv")).unwrap(),
        "1\n2\n"
    );
    run(
        &mut e,
        "D = table(datetime(2024, 1, [1; 2]), [1 2; 3 4], 'VariableNames', {'t', 'm'}); writetable(D, 'd.csv');",
    );
    assert_eq!(
        fs::read_to_string(dir.path().join("d.csv")).unwrap(),
        "t,m_1,m_2\n01-Jan-2024,1,2\n02-Jan-2024,3,4\n"
    );
    // Dates stay text in part 1.
    run(&mut e, "D2 = readtable('d.csv');");
    check(&mut e, "iscell(D2.t) && isequal(D2.m_2, [2; 4])");
    fails_with(
        &mut e,
        "writetable(5, 'x.csv')",
        "writetable: the first input must be a table",
    );
    fails_with(
        &mut e,
        "writetable(T, 'x.csv', 'Bogus', 1)",
        "unsupported option 'Bogus'",
    );
}

#[test]
fn unsupported_and_error_cases() {
    let (mut e, _dir) = engine_in_folder();
    run(&mut e, PEOPLE);
    fails_with(
        &mut e,
        "save('t.mat', 'T')",
        "save: table variables cannot be saved yet",
    );
    fails_with(
        &mut e,
        "save('t.mat', 'T', '-v7.3')",
        "save: table variables cannot be saved yet",
    );
    fails_with(
        &mut e,
        "table(sparse([1; 0]))",
        "table: sparse variables are not supported yet",
    );
    fails_with(
        &mut e,
        "T.S = sparse([1; 0; 2])",
        "table: sparse variables are not supported yet",
    );
    fails_with(&mut e, "table([1; 2], 'Size', [2 1])", "not supported yet");
    fails_with(&mut e, "x = T + 1", "");
    // The bound on variables.
    fails_with(
        &mut e,
        "array2table(zeros(1, 10001))",
        "at most 10000 variables",
    );
}

/// `T.Properties` and the dimension names ('Row', 'Variables') are reserved:
/// no constructor or setter may create a variable with such a name.
#[test]
fn reserved_variable_names_are_rejected() {
    let mut e = Engine::default();
    for (code, name) in [
        ("table(1, 'VariableNames', {'Properties'})", "Properties"),
        ("table(1, 'VariableNames', {'Row'})", "Row"),
        (
            "table(1, 2, 'VariableNames', {'a', 'Variables'})",
            "Variables",
        ),
        ("array2table([1 2], 'VariableNames', {'Row', 'b'})", "Row"),
        (
            "cell2table({1, 2}, 'VariableNames', {'a', 'Properties'})",
            "Properties",
        ),
        ("struct2table(struct('Row', {1; 2}))", "Row"),
        ("struct2table(struct('Properties', 5))", "Properties"),
    ] {
        fails_with(
            &mut e,
            code,
            &format!("Invalid table variable name: '{name}'."),
        );
    }
    // A variable name taken from the argument is checked like any other.
    run(&mut e, "Row = [1; 2];");
    fails_with(&mut e, "table(Row)", "Invalid table variable name: 'Row'.");
    // Another DimensionNames frees 'Row' and takes the new names.
    run(
        &mut e,
        "R = table(1, 'VariableNames', {'Row'}, 'DimensionNames', {'Obs', 'Data'});",
    );
    check(
        &mut e,
        "R.Row == 1 && isequal(R.Properties.DimensionNames, {'Obs', 'Data'})",
    );
    fails_with(
        &mut e,
        "table(1, 'VariableNames', {'Obs'}, 'DimensionNames', {'Obs', 'Data'})",
        "Invalid table variable name: 'Obs'.",
    );
    fails_with(
        &mut e,
        "table(1, 'DimensionNames', {'Properties', 'Data'})",
        "Invalid table dimension name: 'Properties'.",
    );
    fails_with(
        &mut e,
        "table(1, 'DimensionNames', {'bad name', 'Data'})",
        "Invalid table dimension name: 'bad name'.",
    );
    // Setters: the table stays as it was, and later unrelated sets work.
    run(
        &mut e,
        "T = table([1; 2], [3; 4], 'VariableNames', {'a', 'b'});",
    );
    fails_with(
        &mut e,
        "T.Properties.VariableNames = {'a', 'Row'}",
        "Invalid table variable name: 'Row'.",
    );
    fails_with(
        &mut e,
        "T.Properties.VariableNames{2} = 'Properties'",
        "Invalid table variable name: 'Properties'.",
    );
    fails_with(
        &mut e,
        "T.Variables = [5; 6]",
        "Invalid table variable name: 'Variables'.",
    );
    fails_with(
        &mut e,
        "T.Row = [5; 6]",
        "Invalid table variable name: 'Row'.",
    );
    fails_with(
        &mut e,
        "T.Properties.DimensionNames = {'a', 'Data'}",
        "Invalid table dimension name: 'a'.",
    );
    fails_with(
        &mut e,
        "T.Properties.DimensionNames = {'Properties', 'Data'}",
        "Invalid table dimension name: 'Properties'.",
    );
    check(
        &mut e,
        "isequal(T.Properties.VariableNames, {'a', 'b'}) && isequal(T.Properties.DimensionNames, {'Row', 'Variables'})",
    );
    run(&mut e, "T.Properties.Description = 'still works';");
    run(&mut e, "T.Properties.DimensionNames = {'Obs', 'Data'};");
    check(
        &mut e,
        "strcmp(T.Properties.Description, 'still works') && isequal(T.Obs, T.Properties.RowNames)",
    );
    // T.Row / T.Properties never return a variable.
    run(
        &mut e,
        "U = table([1; 2], 'VariableNames', {'x'}, 'RowNames', {'p'; 'q'});",
    );
    check(
        &mut e,
        "isequal(U.Row, {'p'; 'q'}) && isstruct(U.Properties)",
    );
}

/// readtable: headers 'Row', 'Properties', 'Variables' follow the duplicate rule.
#[test]
fn readtable_renames_headers_that_collide_with_reserved_names() {
    let (mut e, dir) = engine_in_folder();
    fs::write(
        dir.path().join("reserved.csv"),
        "Row,Properties,Variables,Row,x\n1,2,3,4,5\n6,7,8,9,10\n",
    )
    .unwrap();
    run(&mut e, "W = readtable('reserved.csv');");
    check(
        &mut e,
        "isequal(W.Properties.VariableNames, {'Row_1', 'Properties_1', 'Variables_1', 'Row_2', 'x'})",
    );
    check(
        &mut e,
        "isequal(W.Row_1, [1; 6]) && isequal(W.Row_2, [4; 9]) && isequal(W.Properties_1, [2; 7]) && isequal(W.x, [5; 10])",
    );
    run(&mut e, "W.Properties.Description = 'read';");
    check(
        &mut e,
        "strcmp(W.Properties.Description, 'read') && height(W) == 2",
    );
}

/// Writes to a table with tens of thousands of row names rebuild the table
/// record; the row-name uniqueness check must not be quadratic.
#[test]
fn row_names_check_stays_linear_on_large_tables() {
    let mut e = Engine::default();
    run(
        &mut e,
        "n = 50000; names = strsplit(sprintf('r%d,', 1:n), ','); names = names(1:n);",
    );
    run(
        &mut e,
        "T = table((1:n)', 'RowNames', names, 'VariableNames', {'x'});",
    );
    check(
        &mut e,
        "height(T) == 50000 && strcmp(T.Properties.RowNames{50000}, 'r50000')",
    );
    let start = std::time::Instant::now();
    for _ in 0..3 {
        run(&mut e, "T.x(1) = 5;");
    }
    let elapsed = start.elapsed();
    check(
        &mut e,
        "T.x(1) == 5 && T.x(2) == 2 && T{'r49999', 'x'} == 49999",
    );
    assert!(
        elapsed < std::time::Duration::from_millis(500),
        "3 writes to a 50,000-row table with row names took {elapsed:?}"
    );
    // The duplicate check still fires, and names the first repeat.
    fails_with(
        &mut e,
        "T.Properties.RowNames{50000} = 'r7'",
        "Duplicate row name: 'r7'.",
    );
}

#[test]
fn handle_variables_are_rejected_but_cells_of_handles_work() {
    let mut e = Engine::default();
    let message = "table: function handle variables are not supported yet";
    fails_with(&mut e, "table(@sin)", message);
    fails(&mut e, "array2table(@sin)");
    fails_with(&mut e, "struct2table(struct('f', @sin))", message);
    run(&mut e, "T = table([1; 2; 3], 'VariableNames', {'k'});");
    fails_with(&mut e, "T.f = @sin", message);
    fails_with(&mut e, "T.f = @(x) x + 1", message);
    check(&mut e, "width(T) == 1");
    // A cell variable holding handles is an ordinary cell variable.
    run(
        &mut e,
        "H = table({@sin; @cos; @(x) x + 1}, [1; 2; 3], 'VariableNames', {'f', 'k'});",
    );
    check(&mut e, "iscell(H.f) && isa(H.f{3}, 'function_handle')");
    run(&mut e, "S = H([3 1], :); g1 = S.f{1}; g2 = S.f{2};");
    check(&mut e, "g1(4) == 5 && g2(0) == 0 && isequal(S.k, [3; 1])");
    run(&mut e, "G = [H; H(1, :)]; g4 = G.f{4};");
    check(&mut e, "height(G) == 4 && g4(0) == 0");
    run(&mut e, "h = plot(1:3);");
    fails_with(
        &mut e,
        "table(h)",
        "table: graphics handle variables are not supported yet",
    );
}

/// `[T1, T2]` with row names on both sides: same set -> T2's rows follow
/// T1's order; different sets -> error; one side without row names keeps
/// the other's.
#[test]
fn horizontal_concatenation_aligns_row_names() {
    let mut e = Engine::default();
    run(
        &mut e,
        "A = table([1; 2; 3], 'RowNames', {'a'; 'b'; 'c'}, 'VariableNames', {'x'});",
    );
    run(
        &mut e,
        "B = table([30; 10; 20], {'z3'; 'z1'; 'z2'}, 'RowNames', {'c'; 'a'; 'b'}, 'VariableNames', {'y', 'w'});",
    );
    run(&mut e, "H = [A, B];");
    check(
        &mut e,
        "isequal(H.Properties.VariableNames, {'x', 'y', 'w'}) && isequal(H.Properties.RowNames, {'a'; 'b'; 'c'})",
    );
    check(
        &mut e,
        "isequal(H.y, [10; 20; 30]) && isequal(H.w, {'z1'; 'z2'; 'z3'}) && isequal(H.x, [1; 2; 3])",
    );
    check(&mut e, "H{'b', 'y'} == 20 && strcmp(H{'c', 'w'}, 'z3')");
    check(&mut e, "isequal(horzcat(A, B), H)");
    // The inputs are unchanged.
    check(
        &mut e,
        "isequal(B.y, [30; 10; 20]) && isequal(B.Properties.RowNames, {'c'; 'a'; 'b'})",
    );
    // Same order: unchanged. Three tables: each follows the first.
    run(
        &mut e,
        "C = table([4; 5; 6], 'RowNames', {'a'; 'b'; 'c'}, 'VariableNames', {'p'}); D = table([9; 8; 7], 'RowNames', {'c'; 'b'; 'a'}, 'VariableNames', {'q'});",
    );
    run(&mut e, "J = [A, C, D];");
    check(
        &mut e,
        "isequal(J.p, [4; 5; 6]) && isequal(J.q, [7; 8; 9]) && isequal(J.Properties.RowNames, {'a'; 'b'; 'c'})",
    );
    // Different names: error naming the missing one.
    run(
        &mut e,
        "Z = table([1; 2; 3], 'RowNames', {'a'; 'b'; 'zz'}, 'VariableNames', {'v'});",
    );
    fails_with(&mut e, "[A, Z]", "'c' is not a row name of table 2");
    fails_with(&mut e, "[Z, A]", "'zz' is not a row name of table 2");
    fails_with(
        &mut e,
        "horzcat(A, C, Z)",
        "'c' is not a row name of table 3",
    );
    // One side without row names keeps the other's (in either order).
    run(&mut e, "N = table([7; 8; 9], 'VariableNames', {'n'});");
    run(&mut e, "L = [N, A]; R = [A, N];");
    check(
        &mut e,
        "isequal(L.Properties.RowNames, {'a'; 'b'; 'c'}) && isequal(R.Properties.RowNames, {'a'; 'b'; 'c'})",
    );
    check(
        &mut e,
        "isequal(L.n, [7; 8; 9]) && isequal(R.n, [7; 8; 9]) && isequal(L.x, [1; 2; 3])",
    );
    // Names equal but the first table has none, the second two have: aligned to the second.
    run(&mut e, "M = [N, B, A];");
    check(
        &mut e,
        "isequal(M.Properties.RowNames, {'c'; 'a'; 'b'}) && isequal(M.x, [3; 1; 2]) && isequal(M.y, [30; 10; 20])",
    );
}
