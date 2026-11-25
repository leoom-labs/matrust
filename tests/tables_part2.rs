//! Tables part 2: variable operations, joins, grouping and missing data.
//! Expected values are worked by hand from the published MATLAB
//! documentation semantics (GNU Octave 10.3 has no table type and no
//! fillmissing/rmmissing); fill and group results agree with pandas
//! (ffill/bfill, linear interpolation, groupby) on the same data.
use matrust::{Array, Engine};

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
fn fails_with(e: &mut Engine, code: &str, part: &str) {
    match e.execute(code) {
        Ok(r) => panic!("{code}: no error, printed {:?}", r.output),
        Err(err) => assert!(err.contains(part), "{code}: {err:?} lacks {part:?}"),
    }
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

const T3: &str = "T = table([1;2;3], [4;5;6], 'VariableNames', {'a','b'});";

#[test]
fn addvars_places_and_names_new_variables() {
    let mut e = Engine::default();
    run(&mut e, T3);
    run(&mut e, "z = [7;8;9]; A = addvars(T, z);");
    check(
        &mut e,
        "isequal(A.Properties.VariableNames, {'a','b','z'}) && isequal(A.z, [7;8;9])",
    );
    run(&mut e, "A = addvars(T, z, 'Before', 'b');");
    check(&mut e, "isequal(A.Properties.VariableNames, {'a','z','b'})");
    run(
        &mut e,
        "A = addvars(T, [0;0;0], 'After', 1, 'NewVariableNames', 'c');",
    );
    check(&mut e, "isequal(A.Properties.VariableNames, {'a','c','b'})");
    run(&mut e, "A = addvars(T, [0;0;0], {'x';'y';'w'});");
    check(
        &mut e,
        "isequal(A.Properties.VariableNames, {'a','b','Var3','Var4'}) && isequal(A.Var4, {'x';'y';'w'})",
    );
    fails_with(&mut e, "addvars(T, [1;2])", "same number of rows");
    fails_with(&mut e, "addvars(T, z, 'Before', 'nope')", "nope");
    fails_with(
        &mut e,
        "addvars(T, [1;2;3], 'NewVariableNames', 'a')",
        "Duplicate",
    );
}

#[test]
fn removevars_renamevars_movevars() {
    let mut e = Engine::default();
    run(
        &mut e,
        "T = table([1;2], [3;4], [5;6], 'VariableNames', {'a','b','c'});",
    );
    check(
        &mut e,
        "isequal(removevars(T, 'b').Properties.VariableNames, {'a','c'})",
    );
    check(&mut e, "isequal(width(removevars(T, [1 3])), 1)");
    check(
        &mut e,
        "isequal(width(removevars(T, [true false true])), 1)",
    );
    run(&mut e, "R = renamevars(T, 'b', 'beta');");
    check(
        &mut e,
        "isequal(R.Properties.VariableNames, {'a','beta','c'}) && isequal(R.beta, [3;4])",
    );
    run(&mut e, "R = renamevars(T, [1 3], [\"x\" \"z\"]);");
    check(&mut e, "isequal(R.Properties.VariableNames, {'x','b','z'})");
    check(
        &mut e,
        "isequal(movevars(T, 'a', 'After', 'c').Properties.VariableNames, {'b','c','a'})",
    );
    check(
        &mut e,
        "isequal(movevars(T, {'c','b'}, 'Before', 1).Properties.VariableNames, {'c','b','a'})",
    );
    check(
        &mut e,
        "isequal(movevars(T, 'a').Properties.VariableNames, {'b','c','a'})",
    );
    fails_with(&mut e, "renamevars(T, {'a','b'}, {'x'})", "one new name");
    fails_with(&mut e, "renamevars(T, 'a', 'b')", "Duplicate");
    fails_with(&mut e, "removevars(T, 'q')", "q");
    fails_with(&mut e, "movevars(T, 'a', 'Before', 'a')", "moved variables");
}

#[test]
fn fillmissing_methods_on_vectors() {
    let mut e = Engine::default();
    run(&mut e, "x = [NaN 1 NaN 3 NaN NaN];");
    check(&mut e, "isequal(fillmissing(x, 'linear'), [0 1 2 3 4 5])");
    check(
        &mut e,
        "isequaln(fillmissing(x, 'previous'), [NaN 1 1 3 3 3])",
    );
    check(
        &mut e,
        "isequaln(fillmissing(x, 'next'), [1 1 3 3 NaN NaN])",
    );
    check(&mut e, "isequal(fillmissing(x, 'nearest'), [1 1 3 3 3 3])");
    check(
        &mut e,
        "isequal(fillmissing(x, 'constant', 0), [0 1 0 3 0 0])",
    );
    check(
        &mut e,
        "isequaln(fillmissing(x, 'linear', 'EndValues', 'none'), [NaN 1 2 3 NaN NaN])",
    );
    check(
        &mut e,
        "isequal(fillmissing(x, 'linear', 'EndValues', 9), [9 1 2 3 9 9])",
    );
    check(
        &mut e,
        "isequal(fillmissing(x, 'previous', 'EndValues', 'nearest'), [1 1 1 3 3 3])",
    );
    // MATLAB doc example: A = [1 3 NaN 4 NaN NaN 5].
    check(
        &mut e,
        "max(abs(fillmissing([1 3 NaN 4 NaN NaN 5], 'linear') - [1 3 3.5 4 13/3 14/3 5])) < 1e-12",
    );
    run(&mut e, "[F, tf] = fillmissing(x, 'previous');");
    check(&mut e, "isequal(tf, logical([0 0 1 0 1 1]))");
    check(&mut e, "isequal(fillmissing([1 2 3], 'linear'), [1 2 3])");
}

#[test]
fn fillmissing_matrices_text_and_errors() {
    let mut e = Engine::default();
    run(&mut e, "M = [1 NaN; NaN 4; 3 NaN];");
    check(
        &mut e,
        "isequaln(fillmissing(M, 'previous'), [1 NaN; 1 4; 3 4])",
    );
    check(
        &mut e,
        "isequaln(fillmissing(M, 'previous', 2), [1 1; NaN 4; 3 3])",
    );
    check(
        &mut e,
        "isequal(fillmissing(M, 'linear'), [1 NaN; 2 4; 3 NaN]) == 0",
    );
    check(
        &mut e,
        "isequaln(fillmissing(M, 'linear'), [1 NaN; 2 4; 3 NaN])",
    );
    check(
        &mut e,
        "isequal(fillmissing({'a';'';'c'}, 'previous'), {'a';'a';'c'})",
    );
    check(
        &mut e,
        "isequal(fillmissing({'a';'';'c'}, 'constant', 'z'), {'a';'z';'c'})",
    );
    check(
        &mut e,
        "isequal(fillmissing([\"a\" missing \"c\"], 'next'), [\"a\" \"c\" \"c\"])",
    );
    check(
        &mut e,
        "isequal(fillmissing(single([1 NaN 3]), 'previous'), single([1 1 3]))",
    );
    check(
        &mut e,
        "isequal(fillmissing(int8([1 2]), 'linear'), int8([1 2]))",
    );
    fails_with(&mut e, "fillmissing({'a';''}, 'linear')", "linear");
    fails_with(&mut e, "fillmissing([1 NaN], 'spline')", "not supported");
    fails_with(
        &mut e,
        "fillmissing([1 NaN], 'wobble')",
        "unknown fill method",
    );
    fails_with(&mut e, "fillmissing([1 NaN], 'constant')", "fill value");
}

#[test]
fn rmmissing_rows_columns_and_threshold() {
    let mut e = Engine::default();
    run(&mut e, "[B, tf] = rmmissing([1 NaN 3]);");
    check(&mut e, "isequal(B, [1 3]) && isequal(tf, logical([0 1 0]))");
    run(&mut e, "M = [1 NaN; 3 4; NaN NaN];");
    check(&mut e, "isequal(rmmissing(M), [3 4])");
    check(&mut e, "isequal(size(rmmissing(M, 2)), [3 0])");
    check(
        &mut e,
        "isequaln(rmmissing(M, 'MinNumMissing', 2), [1 NaN; 3 4])",
    );
    check(&mut e, "isequal(rmmissing({'a';'';'b'}), {'a';'b'})");
    check(&mut e, "isequal(rmmissing([\"x\"; missing]), \"x\")");
    check(&mut e, "isequal(rmmissing([1;2]), [1;2])");
    fails_with(&mut e, "rmmissing(M, 3)", "dimension");
    fails_with(
        &mut e,
        "rmmissing(M, 'MinNumMissing', -1)",
        "nonnegative integer",
    );
    check(
        &mut e,
        "isequal(size(rmmissing([1;2], 'MinNumMissing', 0)), [0 1])",
    );
}

#[test]
fn standardize_missing_values() {
    let mut e = Engine::default();
    check(
        &mut e,
        "isequaln(standardizeMissing([1 -99 3 -99], -99), [1 NaN 3 NaN])",
    );
    check(
        &mut e,
        "isequaln(standardizeMissing([1 -99 0], [-99 0]), [1 NaN NaN])",
    );
    run(
        &mut e,
        "s = standardizeMissing([\"a\" \"N/A\" \"c\"], \"N/A\");",
    );
    check(&mut e, "isequal(ismissing(s), [false true false])");
    check(
        &mut e,
        "isequal(standardizeMissing({'a';'NA'}, 'NA'), {'a';''})",
    );
    check(&mut e, "isequal(standardizeMissing([1 2], 5), [1 2])");
}

#[test]
fn missing_data_in_tables() {
    let mut e = Engine::default();
    run(
        &mut e,
        "T = table([1;NaN;3;NaN], {'a';'b';'';'d'}, [-99;2;3;4], 'VariableNames', {'x','s','y'});",
    );
    check(
        &mut e,
        "isequal(ismissing(T), logical([0 0 0; 1 0 0; 0 1 0; 1 0 0]))",
    );
    run(&mut e, "[R, tf] = rmmissing(T);");
    check(
        &mut e,
        "height(R) == 1 && isequal(R.x, 1) && isequal(tf, logical([0;1;1;1]))",
    );
    check(&mut e, "height(rmmissing(T, 'DataVariables', 's')) == 3");
    run(&mut e, "F = fillmissing(T, 'previous');");
    check(
        &mut e,
        "isequal(F.x, [1;1;3;3]) && isequal(F.s, {'a';'b';'b';'d'})",
    );
    run(
        &mut e,
        "F = fillmissing(T, 'constant', 0, 'DataVariables', 'x');",
    );
    check(&mut e, "isequal(F.x, [1;0;3;0]) && isequal(F.s, T.s)");
    run(
        &mut e,
        "[F, tf] = fillmissing(T, 'linear', 'DataVariables', {'x'});",
    );
    check(
        &mut e,
        "isequal(F.x, [1;2;3;4]) && isequal(tf(:,1), logical([0;1;0;1])) && ~any(tf(:,2))",
    );
    fails_with(&mut e, "fillmissing(T, 'linear')", "'s'");
    run(&mut e, "S = standardizeMissing(T, -99);");
    check(&mut e, "isequaln(S.y, [NaN;2;3;4])");
    run(
        &mut e,
        "S = standardizeMissing(T, -99, 'DataVariables', 'x');",
    );
    check(&mut e, "isequal(S.y, [-99;2;3;4])");
}

#[test]
fn ismissing_datetime_and_duration() {
    let mut e = Engine::default();
    check(
        &mut e,
        "isequal(ismissing([datetime(2020,1,1) NaT]), [false true])",
    );
    check(&mut e, "isequal(ismissing(seconds([1 NaN])), [false true])");
    check(&mut e, "isequal(ismissing([1 NaN]), [false true])");
}

/// isequaln on tables and timetables: like isequal (names, row names and
/// times, every variable), with missing values (NaN, NaT, <missing>)
/// equal to each other (MATLAB isequaln reference page).
#[test]
fn isequaln_compares_tables_with_missing_values() {
    let mut e = Engine::default();
    run(
        &mut e,
        "T = table([1;NaN], [\"x\"; missing], [NaT; datetime(2020,1,1)], 'VariableNames', {'a','s','d'}); \
         U = table([1;2], 'VariableNames', {'a'}); \
         TT = timetable(datetime(2024,1,1) + hours(0:1)', [1;NaN]);",
    );
    check(
        &mut e,
        "isequaln(U, U) && isequaln(T, T) && isequaln(TT, TT)",
    );
    check(
        &mut e,
        "isequaln(T, T, T) && ~isequal(T, T) && isequal(U, U)",
    );
    check(
        &mut e,
        "~isequaln(T, T(:, {'a','d','s'})) && ~isequaln(U, table([1;3], 'VariableNames', {'a'}))",
    );
    check(
        &mut e,
        "~isequaln(U, renamevars(U, 'a', 'b')) && ~isequaln(U, table2array(U))",
    );
    run(
        &mut e,
        "TT2 = TT; TT2.Properties.RowTimes = datetime(2024,1,1) + hours([0;5]);",
    );
    check(
        &mut e,
        "~isequaln(TT, TT2) && ~isequaln(TT, timetable2table(TT))",
    );
}

#[test]
fn findgroups_numbers_and_ids() {
    let mut e = Engine::default();
    run(&mut e, "[G, ID] = findgroups({'b';'a';'b';'';'a'});");
    check(
        &mut e,
        "isequaln(G, [2;1;2;NaN;1]) && isequal(ID, {'a';'b'})",
    );
    run(&mut e, "[G, ID] = findgroups([3 1 3 2]);");
    check(&mut e, "isequal(G, [3 1 3 2]) && isequal(ID, [1 2 3])");
    run(
        &mut e,
        "[G, A, B] = findgroups([1;1;2;2], [\"x\";\"y\";\"x\";\"x\"]);",
    );
    check(
        &mut e,
        "isequal(G, [1;2;3;3]) && isequal(A, [1;1;2]) && isequal(B, [\"x\";\"y\";\"x\"])",
    );
    run(
        &mut e,
        "T = table([2;1;2], {'p';'q';'p'}, 'VariableNames', {'n','s'}); [G, TID] = findgroups(T);",
    );
    check(
        &mut e,
        "isequal(G, [2;1;2]) && istable(TID) && isequal(TID.n, [1;2]) && isequal(TID.s, {'q';'p'})",
    );
    check(&mut e, "isequaln(findgroups([NaN; 5]), [NaN; 1])");
    check(&mut e, "isequal(findgroups(logical([1 0 1])), [2 1 2])");
    fails_with(&mut e, "findgroups([1;2], [1;2;3])", "one value per row");
}

#[test]
fn splitapply_by_group() {
    let mut e = Engine::default();
    run(&mut e, "x = [10;20;30;40;50]; G = [2;1;2;NaN;1];");
    check(&mut e, "isequal(splitapply(@mean, x, G), [35; 20])");
    check(
        &mut e,
        "isequal(splitapply(@(a, b) sum(a .* b), x, [1;1;1;1;1], G), [70; 40])",
    );
    check(
        &mut e,
        "isequal(splitapply(@(m) max(m, [], 1), [1 5; 2 6; 3 7], [1;2;1]), [3 7; 2 6])",
    );
    run(
        &mut e,
        "[mx, mn] = splitapply(@(v) deal(max(v), min(v)), x, G);",
    );
    check(&mut e, "isequal(mx, [50; 30]) && isequal(mn, [20; 10])");
    run(&mut e, "c = splitapply(@(v) {v'}, x, G);");
    check(&mut e, "iscell(c) && isequal(c{1}, [20 50])");
    run(&mut e, "T = table([1;2;3], [4;5;6]);");
    check(
        &mut e,
        "isequal(splitapply(@(a, b) sum(a + b), T, [1;1;2]), [12; 9])",
    );
    fails_with(&mut e, "splitapply(@sum, [1;2;3], [1;3;3])", "no rows");
    fails_with(
        &mut e,
        "splitapply(@sum, [1;2;3], [1;2])",
        "one row per group",
    );
}

const PEOPLE: &str = "T = table({'b';'a';'b';'c';'a'}, [1;2;3;NaN;5], [2;4;6;8;10], 'VariableNames', {'k','x','y'});";

#[test]
fn groupsummary_tables() {
    let mut e = Engine::default();
    run(&mut e, PEOPLE);
    run(&mut e, "S = groupsummary(T, 'k', 'mean');");
    check(
        &mut e,
        "isequal(S.Properties.VariableNames, {'k','GroupCount','mean_x','mean_y'})",
    );
    check(
        &mut e,
        "isequal(S.k, {'a';'b';'c'}) && isequal(S.GroupCount, [2;2;1])",
    );
    check(
        &mut e,
        "isequaln(S.mean_x, [3.5; 2; NaN]) && isequal(S.mean_y, [7; 4; 8])",
    );
    run(
        &mut e,
        "S = groupsummary(T, 'k', {'sum','max','nummissing'}, 'x');",
    );
    check(
        &mut e,
        "isequal(S.Properties.VariableNames, {'k','GroupCount','sum_x','max_x','nummissing_x'})",
    );
    check(
        &mut e,
        "isequaln(S.sum_x, [7; 4; 0]) && isequaln(S.max_x, [5; 3; NaN]) && isequal(S.nummissing_x, [0;0;1])",
    );
    run(&mut e, "S = groupsummary(T, 'k');");
    check(&mut e, "width(S) == 2 && isequal(S.GroupCount, [2;2;1])");
    run(&mut e, "S = groupsummary(T, 'k', @(v) numel(v), 'y');");
    check(
        &mut e,
        "isequal(S.Properties.VariableNames{3}, 'fun1_y') && isequal(S.fun1_y, [2;2;1])",
    );
    run(
        &mut e,
        "S = groupsummary(T, {'k'}, {'median','std','range','mode'}, 'y');",
    );
    check(
        &mut e,
        "isequal(S.median_y, [7;4;8]) && max(abs(S.std_y - [sqrt(18); sqrt(8); 0])) < 1e-12",
    );
    check(
        &mut e,
        "isequal(S.range_y, [6;4;0]) && isequal(S.mode_y, [4;2;8])",
    );
    run(
        &mut e,
        "U = table([1;NaN;1], [5;6;7], 'VariableNames', {'g','v'});",
    );
    check(
        &mut e,
        "isequal(groupsummary(U, 'g', 'sum').sum_v, [12; 6])",
    );
    check(
        &mut e,
        "height(groupsummary(U, 'g', 'sum', 'IncludeMissingGroups', false)) == 1",
    );
    fails_with(&mut e, "groupsummary(T, 'k', 'wobble')", "not supported");
    fails_with(&mut e, "groupsummary(T, 'k', 'mean', 'k')", "numeric");
}

#[test]
fn groupsummary_arrays() {
    let mut e = Engine::default();
    run(
        &mut e,
        "[B, BG, BC] = groupsummary([1;2;3;4], [1;2;1;2], 'sum');",
    );
    check(
        &mut e,
        "isequal(B, [4;6]) && isequal(BG, [1;2]) && isequal(BC, [2;2])",
    );
    run(
        &mut e,
        "B = groupsummary([1 10; 2 20; 3 30], {'x';'y';'x'}, 'mean');",
    );
    check(&mut e, "isequal(B, [2 20; 2 20])");
}

#[test]
fn varfun_whole_and_grouped() {
    let mut e = Engine::default();
    run(&mut e, PEOPLE);
    run(&mut e, "V = varfun(@sum, T, 'InputVariables', {'x','y'});");
    check(
        &mut e,
        "isequal(V.Properties.VariableNames, {'sum_x','sum_y'}) && isnan(V.sum_x) && V.sum_y == 30",
    );
    run(&mut e, "V = varfun(@(v) v * 2, T, 'InputVariables', 'y');");
    check(
        &mut e,
        "isequal(V.Properties.VariableNames, {'Fun_y'}) && isequal(V.Fun_y, [4;8;12;16;20])",
    );
    check(
        &mut e,
        "isequal(varfun(@max, T, 'InputVariables', [3 2], 'OutputFormat', 'uniform'), [10 NaN]) == 0",
    );
    check(
        &mut e,
        "isequaln(varfun(@max, T, 'InputVariables', [3 2], 'OutputFormat', 'uniform'), [10 5])",
    );
    run(&mut e, "C = varfun(@numel, T, 'OutputFormat', 'cell');");
    check(&mut e, "iscell(C) && isequal(size(C), [1 3]) && C{2} == 5");
    run(
        &mut e,
        "V = varfun(@sum, T, 'GroupingVariables', 'k', 'InputVariables', 'y');",
    );
    check(
        &mut e,
        "isequal(V.Properties.VariableNames, {'k','GroupCount','sum_y'}) && isequal(V.sum_y, [14;8;8])",
    );
    fails_with(&mut e, "varfun(@sum, [1 2])", "table");
}

#[test]
fn rowfun_rows_and_groups() {
    let mut e = Engine::default();
    run(&mut e, PEOPLE);
    run(
        &mut e,
        "R = rowfun(@(a, b) a + b, T, 'InputVariables', {'x','y'}, 'OutputVariableNames', 's');",
    );
    check(
        &mut e,
        "isequal(R.Properties.VariableNames, {'s'}) && isequaln(R.s, [3;6;9;NaN;15])",
    );
    run(
        &mut e,
        "R = rowfun(@(a, b) a * b, T, 'InputVariables', {'x','y'});",
    );
    check(&mut e, "isequal(R.Properties.VariableNames, {'Var1'})");
    run(
        &mut e,
        "u = rowfun(@(a, b) b - a, T, 'InputVariables', [2 3], 'OutputFormat', 'uniform');",
    );
    check(&mut e, "isequaln(u, [1;2;3;NaN;5])");
    run(
        &mut e,
        "[p, q] = rowfun(@(a, b) deal(a, b), T, 'InputVariables', [2 3], 'OutputFormat', 'uniform');",
    );
    check(&mut e, "isequal(q, [2;4;6;8;10])");
    run(
        &mut e,
        "R = rowfun(@(v) sum(v), T, 'InputVariables', {'x','y'}, 'SeparateInputs', false);",
    );
    check(&mut e, "isequaln(R.Var1, [3;6;9;NaN;15])");
    run(
        &mut e,
        "R = rowfun(@(v) upper(v), T, 'InputVariables', 'k', 'ExtractCellContents', true, 'OutputFormat', 'cell');",
    );
    check(&mut e, "isequal(R{1}, 'B')");
    run(
        &mut e,
        "R = rowfun(@(v) max(v), T, 'GroupingVariables', 'k', 'InputVariables', 'y', 'OutputVariableNames', 'top');",
    );
    check(
        &mut e,
        "isequal(R.Properties.VariableNames, {'k','GroupCount','top'}) && isequal(R.top, [10;6;8])",
    );
    run(
        &mut e,
        "R = rowfun(@(a, b) deal(a, b), T, 'InputVariables', [2 3], 'NumOutputs', 2);",
    );
    check(
        &mut e,
        "isequal(R.Properties.VariableNames, {'Var1','Var2'})",
    );
    fails_with(
        &mut e,
        "rowfun(@(a) a, T, 'InputVariables', 'y', 'OutputVariableNames', {'a','b'})",
        "output",
    );
}

const JOIN: &str = "L = table([1;2;3], {'a';'b';'c'}, 'VariableNames', {'id','name'}); \
                    R = table([3;1;4;1], [30;10;40;11], 'VariableNames', {'id','v'});";

#[test]
fn innerjoin_sorted_by_key_with_indices() {
    let mut e = Engine::default();
    run(&mut e, JOIN);
    run(&mut e, "[J, il, ir] = innerjoin(L, R);");
    check(
        &mut e,
        "isequal(J.Properties.VariableNames, {'id','name','v'})",
    );
    check(
        &mut e,
        "isequal(J.id, [1;1;3]) && isequal(J.name, {'a';'a';'c'}) && isequal(J.v, [10;11;30])",
    );
    check(&mut e, "isequal(il, [1;1;3]) && isequal(ir, [2;4;1])");
    run(
        &mut e,
        "R2 = renamevars(R, 'id', 'key'); J = innerjoin(L, R2, 'LeftKeys', 'id', 'RightKeys', 'key');",
    );
    check(
        &mut e,
        "isequal(J.Properties.VariableNames, {'id','name','v'}) && height(J) == 3",
    );
    run(
        &mut e,
        "J = innerjoin(L, R, 'Keys', 'id', 'RightVariables', {});",
    );
    check(&mut e, "isequal(J.Properties.VariableNames, {'id','name'})");
    run(
        &mut e,
        "A = table([1;2], [5;6], 'VariableNames', {'id','w'}); B = table([2;1], [7;8], 'VariableNames', {'id','w'});",
    );
    run(&mut e, "J = innerjoin(A, B, 'Keys', 'id');");
    check(
        &mut e,
        "isequal(J.Properties.VariableNames, {'id','w_A','w_B'}) && isequal(J.w_B, [8;7])",
    );
    run(
        &mut e,
        "P = table({'x';'y'}, [1;2], 'VariableNames', {'k','a'}); Q = table([\"y\";\"x\"], [9;8], 'VariableNames', {'k','b'});",
    );
    check(&mut e, "isequal(innerjoin(P, Q).b, [8;9])");
    check(
        &mut e,
        "height(innerjoin(table(NaN, 'VariableNames', {'id'}), table(NaN, 'VariableNames', {'id'}))) == 0",
    );
    fails_with(
        &mut e,
        "innerjoin(L, table([1], 'VariableNames', {'zz'}))",
        "common variable",
    );
    fails_with(&mut e, "innerjoin(L, [1 2])", "tables");
}

#[test]
fn outerjoin_types_and_merged_keys() {
    let mut e = Engine::default();
    run(
        &mut e,
        "L = table([1;2;3], {'a';'b';'c'}, 'VariableNames', {'id','name'}); R = table([3;1;4], [30;10;40], 'VariableNames', {'id','v'});",
    );
    run(&mut e, "[O, il, ir] = outerjoin(L, R);");
    check(
        &mut e,
        "isequal(O.Properties.VariableNames, {'id_L','name','id_R','v'})",
    );
    check(
        &mut e,
        "isequaln(O.id_L, [1;2;3;NaN]) && isequaln(O.id_R, [1;NaN;3;4]) && isequal(O.name, {'a';'b';'c';''})",
    );
    check(&mut e, "isequal(il, [1;2;3;0]) && isequal(ir, [2;0;1;3])");
    run(&mut e, "O = outerjoin(L, R, 'MergeKeys', true);");
    check(
        &mut e,
        "isequal(O.Properties.VariableNames, {'id','name','v'}) && isequal(O.id, [1;2;3;4]) && isequaln(O.v, [10;NaN;30;40])",
    );
    run(
        &mut e,
        "O = outerjoin(L, R, 'Type', 'left', 'MergeKeys', true);",
    );
    check(&mut e, "isequal(O.id, [1;2;3])");
    run(
        &mut e,
        "O = outerjoin(L, R, 'Type', 'right', 'MergeKeys', true);",
    );
    check(
        &mut e,
        "isequal(O.id, [1;3;4]) && isequal(O.name, {'a';'c';''})",
    );
    run(
        &mut e,
        "S = table([\"p\";\"q\"], [1;2], 'VariableNames', {'s','id'}); O = outerjoin(S, R, 'Keys', 'id', 'MergeKeys', true);",
    );
    check(
        &mut e,
        "isequal(ismissing(O.s), [false;false;true;true]) && isequal(O.id, [1;2;3;4])",
    );
    run(
        &mut e,
        "B = table([1;5], [true;true], 'VariableNames', {'id','f'}); O = outerjoin(B, R, 'MergeKeys', true);",
    );
    check(
        &mut e,
        "islogical(O.f) && isequal(O.f, [true;false;false;true])",
    );
    fails_with(&mut e, "outerjoin(L, R, 'Type', 'middle')", "Type");
    fails_with(
        &mut e,
        "innerjoin(L, R, 'MergeKeys', true)",
        "outerjoin option",
    );
}

#[test]
fn join_requires_one_match_per_left_row() {
    let mut e = Engine::default();
    run(
        &mut e,
        "L = table([1;2;3], {'a';'b';'c'}, 'VariableNames', {'id','name'});",
    );
    run(
        &mut e,
        "[J, ir] = join(table([3;1], [7;8], 'VariableNames', {'id','w'}), L);",
    );
    check(
        &mut e,
        "isequal(J.Properties.VariableNames, {'id','w','name'}) && isequal(J.name, {'c';'a'}) && isequal(ir, [3;1])",
    );
    fails_with(
        &mut e,
        "join(table(9, 'VariableNames', {'id'}), L)",
        "no match",
    );
    fails_with(
        &mut e,
        "join(table(1, 'VariableNames', {'id'}), [L; L])",
        "several",
    );
    check(&mut e, "isequal(join([\"a\" \"b\"]), \"a b\")");
}

#[test]
fn categorical_keys_missing_values_and_joins() {
    let mut e = Engine::default();
    run(&mut e, "c = categorical({'lo';'hi';'lo';''}, {'lo','hi'});");
    // Groups follow category order (lo before hi), undefined is in no group.
    run(&mut e, "[G, ID] = findgroups(c);");
    check(
        &mut e,
        "isequaln(G, [1;2;1;NaN]) && isequal(cellstr(ID), {'lo';'hi'})",
    );
    check(&mut e, "isequal(ismissing(c), [false;false;false;true])");
    check(
        &mut e,
        "isequal(cellstr(fillmissing(c, 'previous')), {'lo';'hi';'lo';'lo'})",
    );
    check(
        &mut e,
        "numel(rmmissing(c)) == 3 && iscategorical(rmmissing(c))",
    );
    run(
        &mut e,
        "T = table(c, [1;2;3;4], 'VariableNames', {'k','v'}); S = groupsummary(T, 'k', 'sum');",
    );
    check(
        &mut e,
        "iscategorical(S.k) && isequal(S.sum_v, [4;2;4]) && isundefined(S.k(3))",
    );
    run(
        &mut e,
        "L = table(categorical({'x';'y'}), [1;2], 'VariableNames', {'k','a'}); R = table(categorical({'y';'z';'x'}), [8;9;7], 'VariableNames', {'k','b'});",
    );
    check(&mut e, "isequal(innerjoin(L, R).b, [7;8])");
    run(&mut e, "O = outerjoin(L, R, 'MergeKeys', true);");
    check(
        &mut e,
        "isequal(cellstr(O.k), {'x';'y';'z'}) && isequaln(O.a, [1;2;NaN])",
    );
    run(&mut e, "O = outerjoin(R, L, 'Type', 'left');");
    check(&mut e, "isequal(isundefined(O.k_L), [false;false;true])");
}

#[test]
fn demo_output_matches_pandas_reference() {
    let root = env!("CARGO_MANIFEST_DIR");
    let dir = format!("{root}/tests/fixtures/tables");
    let mut e = Engine::default();
    e.set_current_folder(std::path::Path::new(&dir)).unwrap();
    let got = output(&mut e, "run('tables_part2_demo.mr')");
    let want = std::fs::read_to_string(format!("{dir}/tables_part2_demo.expected")).unwrap();
    // Engine output pieces carry no line breaks; compare the text itself.
    let flat = |t: &str| t.replace(['\r', '\n'], "");
    assert_eq!(flat(&got), flat(&want));
    assert_eq!(want.lines().count(), 6);
}

#[test]
fn review_round_bounds_missing_keys_and_options() {
    let mut e = Engine::default();
    fails_with(
        &mut e,
        "splitapply(@sum, 1, 1e20)",
        "larger than the number of rows",
    );
    fails_with(
        &mut e,
        "rowfun(@sum, table(1), 'NumOutputs', Inf)",
        "NumOutputs",
    );
    fails_with(
        &mut e,
        "L = table(ones(8000,1), 'VariableNames', {'k'}); innerjoin(L, L)",
        "at most",
    );
    // Keys with a missing element stay apart by their other elements.
    run(
        &mut e,
        "T = table([1;2], [NaN;NaN], [10;20], 'VariableNames', {'a','b','x'}); S = groupsummary(T, {'a','b'}, 'sum');",
    );
    check(
        &mut e,
        "isequal(S.a, [1;2]) && isequal(S.sum_x, [10;20]) && isequal(S.GroupCount, [1;1])",
    );
    run(
        &mut e,
        "T = table([1;NaN], [10;20], 'VariableNames', {'g','x'});",
    );
    check(
        &mut e,
        "isequal(varfun(@sum, T, 'GroupingVariables', 'g').sum_x, 10)",
    );
    check(
        &mut e,
        "isequal(rowfun(@sum, T, 'GroupingVariables', 'g', 'InputVariables', 'x').Var1, 10)",
    );
    check(
        &mut e,
        "isequal(groupsummary([10;20], [1;NaN], 'sum', 'IncludeMissingGroups', false), 10)",
    );
    fails_with(
        &mut e,
        "groupsummary(T, 'g', 'IncludeEmptyGroups', true)",
        "not supported",
    );
    run(
        &mut e,
        "R = rowfun(@(x) (1:x)', table([1;2]), 'OutputFormat', 'cell');",
    );
    check(&mut e, "isequal(R{1}, 1) && isequal(R{2}, [1;2])");
    run(
        &mut e,
        "C = table(categorical({'a'}), 'VariableNames', {'c'}); U = [C; C];",
    );
    check(&mut e, "height(U) == 2 && iscategorical(U.c)");
    check(
        &mut e,
        "isequal(cellstr(splitapply(@(x) x(1), categorical({'a';'b'}), [1;2])), {'a';'b'})",
    );
    run(
        &mut e,
        "K = categorical({'lo';'hi'}, {'lo','hi'}, 'Ordinal', true); L = table(K, 'VariableNames', {'k'}); J = innerjoin(L, L);",
    );
    check(&mut e, "isequal(cellstr(J.k), {'lo';'hi'})");
    run(
        &mut e,
        "J = join(table(1, 10, 'VariableNames', {'k','v'}), table(1, 20, 'VariableNames', {'k','v'}), 'Keys', 'k', 'KeepOneCopy', 'v');",
    );
    check(
        &mut e,
        "isequal(J.Properties.VariableNames, {'k','v'}) && J.v == 10",
    );
    check(
        &mut e,
        "isequal(fillmissing([NaN 1; NaN 2], 'constant', [10 20]), [10 1; 10 2])",
    );
    check(
        &mut e,
        "isequal(fillmissing([NaN 1; NaN 2; 3 3], 'previous', 'EndValues', [10 20]), [10 1; 10 2; 3 3])",
    );
    check(
        &mut e,
        "isequal(isundefined(standardizeMissing(categorical({'a','NA'}), 'NA')), [false true])",
    );
    run(
        &mut e,
        "[tf, loc] = ismember(categorical({'a','b'}), \"a\");",
    );
    check(&mut e, "isequal(tf, [true false]) && isequal(loc, [1 0])");
}

#[test]
fn categorical_and_datetime_variables_take_element_assignment() {
    // Reference semantics: NaN assigns <undefined> into a categorical and
    // NaT into a datetime; new text adds a category.
    let mut e = Engine::default();
    run(
        &mut e,
        "T = table(categorical({'a';'b';'a'}), datetime(2024,1,1:3)', [1;2;3], 'VariableNames', {'c','d','x'});",
    );
    run(&mut e, "T.c(2) = 'a'; T.c(end) = NaN; T.c(1) = 'z';");
    check(&mut e, "isequal(isundefined(T.c)', [false false true])");
    check(&mut e, "isequal(cellstr(T.c(1:2))', {'z', 'a'})");
    check(&mut e, "isequal(categories(T.c)', {'a', 'b', 'z'})");
    run(
        &mut e,
        "T.d([1 3]) = NaN; T.d(2) = datetime(2025,5,5); T.d(3) = datetime(2026,1,2);",
    );
    check(&mut e, "isequal(isnat(T.d)', [true false false])");
    check(
        &mut e,
        "T.d(2) == datetime(2025,5,5) && T.d(3) == datetime(2026,1,2)",
    );
    check(&mut e, "isequal(T.x, [1;2;3]) && height(T) == 3");
    // The same on plain variables.
    run(
        &mut e,
        "C = categorical({'p';'q'}); C(1) = NaN; D = datetime(2024,1,1:2); D(2) = NaN;",
    );
    check(
        &mut e,
        "isequal(isundefined(C)', [true false]) && isequal(isnat(D), [false true])",
    );
    // Growing a variable past the table height still errors.
    fails_with(&mut e, "T.c(5) = 'a';", "assigning past the end");
}

#[test]
fn review_1111_empty_keys_and_key_types() {
    // MathWorks join/innerjoin: Keys must name a variable. Numeric classes
    // (double, single, int8..uint64) compare by value; an integer that is an
    // exact double converts when the other side is floating. Logical is not numeric
    // (isnumeric). Duration, datetime, and categorical join only their own
    // class. Char, string, and cellstr join each other.
    // https://www.mathworks.com/help/matlab/ref/innerjoin.html
    // https://www.mathworks.com/help/matlab/ref/isnumeric.html
    // https://www.mathworks.com/help/matlab/matlab_prog/valid-combinations-of-unlike-classes.html
    let mut e = Engine::default();
    run(
        &mut e,
        "L = table([1;2;3],[10;20;30],'VariableNames',{'id','v'}); R = table(100,'VariableNames',{'extra'});",
    );
    fails_with(
        &mut e,
        "join(L, R, 'Keys', {})",
        "Keys must name at least one variable",
    );
    fails_with(
        &mut e,
        "innerjoin(L, R, 'Keys', [])",
        "Keys must name at least one variable",
    );
    fails_with(
        &mut e,
        "outerjoin(L, R, 'Keys', false)",
        "Keys must name at least one variable",
    );
    fails_with(
        &mut e,
        "innerjoin(table(seconds([1;2]),'VariableNames',{'t'}), table([1000;2000],'VariableNames',{'t'}))",
        "incompatible key types",
    );
    fails_with(
        &mut e,
        "innerjoin(table(logical([true;false]),'VariableNames',{'t'}), table([1;0],'VariableNames',{'t'}))",
        "incompatible key types",
    );
    fails_with(
        &mut e,
        "innerjoin(table(datetime(2024,1,1),'VariableNames',{'t'}), table(1,'VariableNames',{'t'}))",
        "incompatible key types",
    );
    fails_with(
        &mut e,
        "innerjoin(table(categorical({'a'}),'VariableNames',{'k'}), table(\"a\",'VariableNames',{'k'}))",
        "incompatible key types",
    );
    // int32([1;2]) matches double [2;1] by value; result sorted 1 then 2.
    run(
        &mut e,
        "J = innerjoin(table(int32([1;2]),[10;20],'VariableNames',{'k','v'}), table([2;1],[7;8],'VariableNames',{'k','w'}));",
    );
    check(&mut e, "isequal(J.w, [8;7]) && isequal(J.v, [10;20])");
    // single and double already share a value; int8 and int32 match exactly.
    run(
        &mut e,
        "J = innerjoin(table(single([1;2]),[10;20],'VariableNames',{'k','v'}), table([2;1],[7;8],'VariableNames',{'k','w'}));",
    );
    check(&mut e, "isequal(J.w, [8;7])");
    run(
        &mut e,
        "J = innerjoin(table(int8([1;2]),[10;20],'VariableNames',{'k','v'}), table(int32([2;1]),[7;8],'VariableNames',{'k','w'}));",
    );
    check(&mut e, "isequal(J.w, [8;7])");
    run(
        &mut e,
        "J = innerjoin(table(seconds([1;2]),[10;20],'VariableNames',{'t','v'}), table(seconds([2;1]),[7;8],'VariableNames',{'t','w'}));",
    );
    check(&mut e, "isequal(J.w, [8;7])");
    run(
        &mut e,
        "J = innerjoin(table(logical([false;true]),[10;20],'VariableNames',{'k','v'}), table(logical([true;false]),[7;8],'VariableNames',{'k','w'}));",
    );
    check(&mut e, "isequal(J.w, [8;7])");
    check(
        &mut e,
        "isequal(innerjoin(table({'x';'y'},[1;2],'VariableNames',{'k','a'}), table([\"y\";\"x\"],[9;8],'VariableNames',{'k','b'})).b, [8;9])",
    );
    // A missing component stays unmatched.
    check(
        &mut e,
        "height(innerjoin(table([1;1],[NaN;5],'VariableNames',{'a','b'}), table([1;1],[NaN;5],'VariableNames',{'a','b'}))) == 1",
    );
}

#[test]
fn review_s1113_inexact_integer_key_does_not_match_rounded_double() {
    // uint64(2^53)+1 is 9007199254740993. Casting it to double rounds to
    // 2^53, so the keys must not match. Equal values of different numeric
    // classes still match: uint64(5) with double 5, and uint64(2^53) with
    // double 2^53.
    let mut e = Engine::default();
    run(
        &mut e,
        "L = table(uint64(2^53)+1, 10, 'VariableNames', {'k','v'}); R = table(2^53, 20, 'VariableNames', {'k','w'}); J = innerjoin(L, R);",
    );
    check(&mut e, "height(J) == 0");
    run(
        &mut e,
        "L = table([uint64(5); uint64(2^53)], [1;2], 'VariableNames', {'k','v'}); R = table([5; 2^53], [8;9], 'VariableNames', {'k','w'}); J = innerjoin(L, R);",
    );
    check(&mut e, "isequal(J.v, [1;2]) && isequal(J.w, [8;9])");
}

#[test]
fn review_1111_outerjoin_missing_component_sorts_inside_its_column() {
    // MathWorks outerjoin: rows are sorted by the key values. sort puts NaN
    // last within a column, not after every complete composite key.
    // https://www.mathworks.com/help/matlab/ref/outerjoin.html
    // https://www.mathworks.com/help/matlab/ref/sort.html
    // L keys (1, NaN), (3, 10); R keys (2, 0), (3, 10) -> order 1, 2, 3.
    let mut e = Engine::default();
    run(
        &mut e,
        "L = table([1;3],[NaN;10],'VariableNames',{'a','b'}); R = table([2;3],[0;10],'VariableNames',{'a','b'}); [O, il, ir] = outerjoin(L, R, 'MergeKeys', true);",
    );
    check(
        &mut e,
        "isequaln(O.a, [1;2;3]) && isequaln(O.b, [NaN;0;10]) && isequal(il, [1;0;2]) && isequal(ir, [0;1;2])",
    );
}
