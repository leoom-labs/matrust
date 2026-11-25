//! Import options: detectImportOptions, setvartype, setvaropts, getvaropts,
//! preview, readtimetable and readtable/readmatrix/readcell with options.
//! The tests write their fixtures into a temporary folder; the expected
//! numbers come from pandas (tool/ref/import_options.py writes the same
//! files and prints them), the .xlsx values are a writetable round trip.
use matrust::{Array, Engine};
use std::fs;

const PATIENTS: &str = "LastName,Age,Height,Weight,Smoker\nSmith,38,71,176.5,1\nJohnson,43,69,163,0\nWilliams,,64,131,0\nJones,40,67,133.25,1\nBrown,49,64,119,0\n";
const SENSOR: &str = "Sensor log exported 2024-03-01\nStation: North field\nTime\tTemp\tPressure\tStatus\ns\tdegC\tkPa\t-\n0\t20.5\t101.3\tok\n10\t20.7\t101.2\tok\n20\t21.1\t101\twarn\n30\t21.4\t100.9\tok\n";
const PRICES: &str = "Item;Price;Qty\nApple;1,25;10\nPear;0,8;5\nPlum;2,5;12\n";
const SURVEY: &str =
    "id,score,group,weight\n1,7.5,A,60.2\n2,NA,B,\n3,8.25,,71\n4,6,A,NA\n5,,B,58.5\n";
const EVENTS: &str = "Date,Due,Elapsed,Value\n2024-01-15,15/01/2024,00:10:00,1.5\n2024-02-29,29/02/2024,01:30:15,2.25\n2024-03-31,31/03/2024,12:00:00,3\n";

/// A fresh engine in a temporary folder holding the fixtures.
fn engine() -> (Engine, tempfile::TempDir) {
    let dir = tempfile::tempdir().unwrap();
    let long: String = std::iter::once("x,y\n".to_owned())
        .chain((1..=12).map(|k| format!("{k},{}\n", k * k)))
        .collect();
    for (name, text) in [
        ("patients.csv", PATIENTS),
        ("sensor.txt", SENSOR),
        ("prices.csv", PRICES),
        ("survey.csv", SURVEY),
        ("events.csv", EVENTS),
        ("long.csv", long.as_str()),
    ] {
        fs::write(dir.path().join(name), text).unwrap();
    }
    let mut e = Engine::default();
    e.set_current_folder(dir.path()).unwrap();
    (e, dir)
}

#[track_caller]
fn run(e: &mut Engine, source: &str) -> String {
    match e.execute(source) {
        Ok(x) => x.output.join("\n"),
        Err(error) => panic!("{source}: {error}"),
    }
}

#[track_caller]
fn fails(e: &mut Engine, source: &str, part: &str) {
    match e.execute(source) {
        Ok(x) => panic!("{source} should fail, printed {:?}", x.output),
        Err(error) => {
            let error = error.to_string();
            assert!(error.contains(part), "{source}: {error:?} lacks {part:?}");
        }
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

#[test]
fn comma_file_detects_names_types_and_data_lines() {
    let (mut e, _dir) = engine();
    run(&mut e, "opts = detectImportOptions('patients.csv');");
    check(
        &mut e,
        "strcmp(class(opts), 'matlab.io.text.DelimitedTextImportOptions')",
    );
    check(
        &mut e,
        "isa(opts, 'matlab.io.text.DelimitedTextImportOptions')",
    );
    check(&mut e, "exist('detectImportOptions') == 2");
    check(&mut e, "isequal(opts.Delimiter, {','})");
    check(
        &mut e,
        "isequal(opts.VariableNames, {'LastName', 'Age', 'Height', 'Weight', 'Smoker'})",
    );
    check(
        &mut e,
        "isequal(opts.VariableTypes, {'char', 'double', 'double', 'double', 'double'})",
    );
    check(
        &mut e,
        "isequal(opts.SelectedVariableNames, opts.VariableNames)",
    );
    check(&mut e, "isequal(opts.DataLines, [2 Inf])");
    check(
        &mut e,
        "opts.VariableNamesLine == 1 && opts.VariableUnitsLine == 0",
    );
    check(&mut e, "opts.RowNamesColumn == 0");
    check(
        &mut e,
        "strcmp(opts.MissingRule, 'fill') && strcmp(opts.ImportErrorRule, 'fill')",
    );
    check(
        &mut e,
        "strcmp(opts.ExtraColumnsRule, 'addvars') && strcmp(opts.EmptyLineRule, 'skip')",
    );
    check(&mut e, "strcmp(opts.Whitespace, '\\b\\t ')");
    check(&mut e, "isequal(opts.LineEnding, {'\\n', '\\r', '\\r\\n'})");
    // Values as pandas.read_csv reads them (tool/ref/import_options.py).
    run(&mut e, "T = readtable('patients.csv', opts);");
    check(&mut e, "isequal(size(T), [5 5])");
    check(&mut e, "isequaln(T.Age, [38; 43; NaN; 40; 49])");
    check(&mut e, "isequal(T.Weight, [176.5; 163; 131; 133.25; 119])");
    check(
        &mut e,
        "iscellstr(T.LastName) && strcmp(T.LastName{3}, 'Williams')",
    );
}

#[test]
fn setvartype_converts_to_string_categorical_logical_and_numbers() {
    let (mut e, _dir) = engine();
    run(&mut e, "opts = detectImportOptions('patients.csv');");
    run(&mut e, "o1 = setvartype(opts, 'LastName', 'string');");
    check(&mut e, "strcmp(o1.VariableTypes{1}, 'string')");
    // The original keeps its types: functions return changed copies.
    check(&mut e, "strcmp(opts.VariableTypes{1}, 'char')");
    run(&mut e, "T = readtable('patients.csv', o1);");
    check(
        &mut e,
        "isstring(T.LastName) && T.LastName(2) == \"Johnson\"",
    );
    run(
        &mut e,
        "o2 = setvartype(opts, {'LastName', 'Smoker'}, {'categorical', 'logical'}); T = readtable('patients.csv', o2);",
    );
    check(&mut e, "iscategorical(T.LastName)");
    check(
        &mut e,
        "isequal(categories(T.LastName), {'Brown'; 'Johnson'; 'Jones'; 'Smith'; 'Williams'})",
    );
    check(
        &mut e,
        "islogical(T.Smoker) && isequal(T.Smoker, logical([1; 0; 0; 1; 0]))",
    );
    // Back to double, and integer and single types by index.
    run(
        &mut e,
        "o3 = setvartype(o2, 'Smoker', 'double'); o3 = setvartype(o3, 2, 'int32'); o3 = setvartype(o3, [3 4], 'single'); T = readtable('patients.csv', o3);",
    );
    check(
        &mut e,
        "isa(T.Smoker, 'double') && isequal(T.Smoker, [1; 0; 0; 1; 0])",
    );
    check(
        &mut e,
        "isa(T.Age, 'int32') && isequal(T.Age, int32([38; 43; 0; 40; 49]))",
    );
    check(
        &mut e,
        "isa(T.Weight, 'single') && T.Weight(4) == single(133.25)",
    );
    // Every variable at once.
    run(
        &mut e,
        "o4 = setvartype(opts, 'string'); T = readtable('patients.csv', o4);",
    );
    check(&mut e, "all(strcmp(o4.VariableTypes, 'string'))");
    check(
        &mut e,
        "isstring(T.Age) && T.Age(1) == \"38\" && ismissing(T.Age(3))",
    );
    // A string array selects too.
    run(
        &mut e,
        "o5 = setvartype(opts, [\"Age\" \"Height\"], 'double');",
    );
    check(
        &mut e,
        "isequal(o5.VariableTypes(2:3), {'double', 'double'})",
    );
}

#[test]
fn selected_variables_fill_values_and_missing_rules() {
    let (mut e, _dir) = engine();
    run(&mut e, "opts = detectImportOptions('patients.csv');");
    run(
        &mut e,
        "opts.SelectedVariableNames = {'Weight', 'Age'}; T = readtable('patients.csv', opts);",
    );
    check(
        &mut e,
        "isequal(T.Properties.VariableNames, {'Weight', 'Age'})",
    );
    check(
        &mut e,
        "isequaln(T{:, :}, [176.5 38; 163 43; 131 NaN; 133.25 40; 119 49])",
    );
    run(
        &mut e,
        "opts.SelectedVariableNames = [1 5]; T = readtable('patients.csv', opts);",
    );
    check(
        &mut e,
        "isequal(T.Properties.VariableNames, {'LastName', 'Smoker'})",
    );
    run(
        &mut e,
        "opts = detectImportOptions('patients.csv'); opts = setvaropts(opts, 'Age', 'FillValue', 0); T = readtable('patients.csv', opts);",
    );
    check(&mut e, "isequal(T.Age, [38; 43; 0; 40; 49])");
    run(&mut e, "vo = getvaropts(opts, 'Age');");
    check(
        &mut e,
        "strcmp(class(vo), 'matlab.io.NumericVariableImportOptions')",
    );
    check(
        &mut e,
        "vo.FillValue == 0 && strcmp(vo.Name, 'Age') && strcmp(vo.Type, 'double')",
    );
    check(
        &mut e,
        "isempty(vo.TreatAsMissing) && strcmp(vo.DecimalSeparator, '.')",
    );
    run(&mut e, "vs = getvaropts(opts, {'LastName', 'Age'});");
    check(&mut e, "numel(vs) == 2");
    check(
        &mut e,
        "strcmp(class(vs(1)), 'matlab.io.TextVariableImportOptions') && vs(2).FillValue == 0",
    );
    check(&mut e, "strcmp(opts.VariableOptions(3).Name, 'Height')");
    check(&mut e, "isnan(opts.VariableOptions(4).FillValue)");
    // MissingRule 'omitrow' drops Williams (no Age), as pandas dropna.
    run(
        &mut e,
        "o = detectImportOptions('patients.csv'); o.MissingRule = 'omitrow'; T = readtable('patients.csv', o);",
    );
    check(&mut e, "height(T) == 4 && isequal(T.Age, [38; 43; 40; 49])");
    check(
        &mut e,
        "isequal(T.LastName, {'Smith'; 'Johnson'; 'Jones'; 'Brown'})",
    );
    // Value semantics: a copy changes alone.
    run(&mut e, "o2 = o; o2.DataLines = [3 Inf];");
    check(
        &mut e,
        "isequal(o.DataLines, [2 Inf]) && isequal(o2.DataLines, [3 Inf])",
    );
    run(&mut e, "T = readtable('patients.csv', o2);");
    check(&mut e, "isequal(T.LastName, {'Johnson'; 'Jones'; 'Brown'})");
    // Indexed assignment into a property.
    run(
        &mut e,
        "o.VariableNames{2} = 'Years'; T = readtable('patients.csv', o);",
    );
    check(
        &mut e,
        "strcmp(o.VariableNames{2}, 'Years') && isequal(T.Years, [38; 43; 40; 49])",
    );
    // Variable options changed through VariableOptions.
    run(
        &mut e,
        "o.VariableOptions(2).FillValue = -5; o.MissingRule = 'fill'; T = readtable('patients.csv', o);",
    );
    check(&mut e, "isequal(T.Years, [38; 43; -5; 40; 49])");
    run(
        &mut e,
        "v = getvaropts(o, 'Height'); v.FillValue = 7; o.VariableOptions(3) = v;",
    );
    check(&mut e, "o.VariableOptions(3).FillValue == 7");
}

#[test]
fn tab_file_with_header_lines_and_a_units_row() {
    let (mut e, _dir) = engine();
    run(&mut e, "opts = detectImportOptions('sensor.txt');");
    check(&mut e, "isequal(opts.Delimiter, {'\\t'})");
    check(&mut e, "strcmp(opts.Whitespace, '\\b ')");
    check(
        &mut e,
        "isequal(opts.VariableNames, {'Time', 'Temp', 'Pressure', 'Status'})",
    );
    check(
        &mut e,
        "isequal(opts.VariableTypes, {'double', 'double', 'double', 'char'})",
    );
    check(&mut e, "isequal(opts.DataLines, [5 Inf])");
    check(
        &mut e,
        "opts.VariableNamesLine == 3 && opts.VariableUnitsLine == 4",
    );
    run(&mut e, "T = readtable('sensor.txt', opts);");
    check(&mut e, "isequal(T.Time, [0; 10; 20; 30])");
    check(&mut e, "isequal(T.Temp, [20.5; 20.7; 21.1; 21.4])");
    check(&mut e, "isequal(T.Pressure, [101.3; 101.2; 101; 100.9])");
    check(&mut e, "isequal(T.Status, {'ok'; 'ok'; 'warn'; 'ok'})");
    check(
        &mut e,
        "isequal(T.Properties.VariableUnits, {'s', 'degC', 'kPa', '-'})",
    );
    // The same table from readtable's own options.
    run(
        &mut e,
        "T2 = readtable('sensor.txt', 'Delimiter', '\\t', 'VariableNamesLine', 3, 'DataLines', [5 Inf]);",
    );
    check(
        &mut e,
        "isequal(T2.Properties.VariableNames, T.Properties.VariableNames) && isequal(T2.Temp, T.Temp)",
    );
    run(
        &mut e,
        "T3 = readtable('sensor.txt', 'NumHeaderLines', 2, 'Delimiter', 'tab', 'DataLines', [6 7]);",
    );
    check(&mut e, "isequal(T3.Time, [10; 20])");
}

#[test]
fn semicolon_file_with_decimal_commas() {
    let (mut e, _dir) = engine();
    run(&mut e, "opts = detectImportOptions('prices.csv');");
    check(&mut e, "isequal(opts.Delimiter, {';'})");
    check(
        &mut e,
        "isequal(opts.VariableTypes, {'char', 'double', 'double'})",
    );
    check(
        &mut e,
        "strcmp(getvaropts(opts, 'Price').DecimalSeparator, ',')",
    );
    run(&mut e, "T = readtable('prices.csv', opts);");
    check(
        &mut e,
        "isequal(T.Price, [1.25; 0.8; 2.5]) && isequal(T.Qty, [10; 5; 12])",
    );
    check(&mut e, "sum(T.Price .* T.Qty) == 46.5");
    // With a '.' decimal separator the comma numbers are import errors.
    run(
        &mut e,
        "o = setvaropts(opts, 'Price', 'DecimalSeparator', '.'); T = readtable('prices.csv', o);",
    );
    check(&mut e, "all(isnan(T.Price))");
    run(
        &mut e,
        "T = readtable('prices.csv', 'Delimiter', ';', 'DecimalSeparator', ',', 'SelectedVariableNames', 'Price');",
    );
    check(
        &mut e,
        "isequal(T.Price, [1.25; 0.8; 2.5]) && width(T) == 1",
    );
}

#[test]
fn missing_fields_na_treat_as_missing_and_rules() {
    let (mut e, _dir) = engine();
    run(&mut e, "opts = detectImportOptions('survey.csv');");
    check(
        &mut e,
        "isequal(opts.VariableTypes, {'double', 'double', 'char', 'double'})",
    );
    run(&mut e, "T = readtable('survey.csv', opts);");
    check(&mut e, "isequaln(T.score, [7.5; NaN; 8.25; 6; NaN])");
    check(&mut e, "isequaln(T.weight, [60.2; NaN; 71; NaN; 58.5])");
    check(&mut e, "isequal(T.group, {'A'; 'B'; ''; 'A'; 'B'})");
    run(
        &mut e,
        "o = setvaropts(opts, 'weight', 'FillValue', -1); T = readtable('survey.csv', o);",
    );
    check(&mut e, "isequal(T.weight, [60.2; -1; 71; -1; 58.5])");
    run(
        &mut e,
        "o = setvaropts(opts, 'score', 'TreatAsMissing', 'NA'); o.MissingRule = 'omitrow'; o.SelectedVariableNames = {'id', 'score'}; T = readtable('survey.csv', o);",
    );
    check(
        &mut e,
        "isequal(T.id, [1; 3; 4]) && isequal(T.score, [7.5; 8.25; 6])",
    );
    check(
        &mut e,
        "isequal(getvaropts(o, 'score').TreatAsMissing, {'NA'})",
    );
    // 'NA' is an import error (not missing) without TreatAsMissing.
    run(
        &mut e,
        "o = opts; o.ImportErrorRule = 'omitrow'; o.SelectedVariableNames = {'id', 'score'}; T = readtable('survey.csv', o);",
    );
    check(&mut e, "isequal(T.id, [1; 3; 4; 5])");
    // Every variable but id has an empty field (pandas dropna(axis=1)).
    run(
        &mut e,
        "o = opts; o.MissingRule = 'omitvar'; T = readtable('survey.csv', o);",
    );
    check(&mut e, "isequal(T.Properties.VariableNames, {'id'})");
    run(&mut e, "o = opts; o.MissingRule = 'error';");
    fails(
        &mut e,
        "readtable('survey.csv', o)",
        "MissingRule is 'error'",
    );
    // readtable's own options.
    run(
        &mut e,
        "T = readtable('survey.csv', 'TreatAsMissing', 'NA', 'MissingRule', 'omitrow', 'SelectedVariableNames', {'id', 'score'});",
    );
    check(&mut e, "isequal(T.id, [1; 3; 4])");
    run(
        &mut e,
        "T = readtable('survey.csv', 'TextType', 'string', 'SelectedVariableNames', 'group');",
    );
    check(&mut e, "isstring(T.group) && ismissing(T.group(3))");
}

#[test]
fn datetime_and_duration_columns_and_readtimetable() {
    let (mut e, _dir) = engine();
    run(&mut e, "opts = detectImportOptions('events.csv');");
    check(
        &mut e,
        "isequal(opts.VariableTypes, {'datetime', 'datetime', 'duration', 'double'})",
    );
    run(&mut e, "T = readtable('events.csv', opts);");
    check(
        &mut e,
        "isdatetime(T.Date) && strcmp(T.Date.Format, 'yyyy-MM-dd')",
    );
    check(
        &mut e,
        "strcmp(T.Due.Format, 'dd/MM/yyyy') && isequal(T.Date, T.Due)",
    );
    // pandas: day gaps 45 and 31; dates 2024-01-15, 2024-02-29, 2024-03-31.
    check(&mut e, "isequal(days(diff(T.Date)), [45; 31])");
    check(
        &mut e,
        "isequal([year(T.Date) month(T.Date) day(T.Date)], [2024 1 15; 2024 2 29; 2024 3 31])",
    );
    check(
        &mut e,
        "isduration(T.Elapsed) && isequal(seconds(T.Elapsed), [600; 5415; 43200])",
    );
    run(&mut e, "TT = readtimetable('events.csv');");
    check(&mut e, "istimetable(TT) && height(TT) == 3");
    check(&mut e, "strcmp(TT.Properties.DimensionNames{1}, 'Date')");
    check(
        &mut e,
        "isequal(TT.Properties.VariableNames, {'Due', 'Elapsed', 'Value'})",
    );
    check(&mut e, "isequal(TT.Value, [1.5; 2.25; 3])");
    run(
        &mut e,
        "TT = readtimetable('events.csv', 'RowTimes', 'Due');",
    );
    check(
        &mut e,
        "strcmp(TT.Properties.DimensionNames{1}, 'Due') && isequal(TT.Properties.VariableNames, {'Date', 'Elapsed', 'Value'})",
    );
    run(
        &mut e,
        "TT = readtimetable('events.csv', opts, 'RowTimes', 'Elapsed');",
    );
    check(&mut e, "isduration(TT.Properties.RowTimes)");
    run(
        &mut e,
        "TT = readtimetable('events.csv', 'RowTimes', seconds([1; 2; 3]));",
    );
    check(
        &mut e,
        "strcmp(TT.Properties.DimensionNames{1}, 'Time') && width(TT) == 4",
    );
    run(
        &mut e,
        "o = detectImportOptions('events.csv', 'DatetimeType', 'text', 'TextType', 'string');",
    );
    check(
        &mut e,
        "isequal(o.VariableTypes(1:2), {'string', 'string'})",
    );
    run(
        &mut e,
        "o = setvaropts(opts, 'Date', 'InputFormat', 'yyyy-MM-dd', 'DatetimeFormat', 'dd-MMM-yyyy'); T = readtable('events.csv', o);",
    );
    check(
        &mut e,
        "strcmp(T.Date.Format, 'dd-MMM-yyyy') && isequal(T.Date, T.Due)",
    );
}

#[test]
fn spreadsheet_written_by_writetable_reads_back_with_sheet_and_range() {
    let (mut e, _dir) = engine();
    run(
        &mut e,
        "T0 = table([1; 2; 3; 4], [10.5; 20.25; 30; 40.75], {'a'; 'b'; 'c'; 'd'}, 'VariableNames', {'ID', 'Score', 'Tag'}); writetable(T0, 'book.xlsx', 'Sheet', 'Data');",
    );
    run(
        &mut e,
        "opts = detectImportOptions('book.xlsx', 'Sheet', 'Data');",
    );
    check(
        &mut e,
        "strcmp(class(opts), 'matlab.io.spreadsheet.SpreadsheetImportOptions')",
    );
    check(
        &mut e,
        "strcmp(opts.Sheet, 'Data') && strcmp(opts.DataRange, 'A2')",
    );
    check(&mut e, "strcmp(opts.VariableNamesRange, 'A1')");
    check(
        &mut e,
        "isequal(opts.VariableNames, {'ID', 'Score', 'Tag'})",
    );
    check(
        &mut e,
        "isequal(opts.VariableTypes, {'double', 'double', 'char'})",
    );
    run(&mut e, "T = readtable('book.xlsx', opts);");
    check(
        &mut e,
        "isequal(T.ID, T0.ID) && isequal(T.Score, T0.Score) && isequal(T.Tag, T0.Tag)",
    );
    run(
        &mut e,
        "T = readtable('book.xlsx', 'Sheet', 'Data', 'Range', 'A1:B3');",
    );
    check(
        &mut e,
        "isequal(T.Properties.VariableNames, {'ID', 'Score'}) && isequal(T{:, :}, [1 10.5; 2 20.25])",
    );
    run(
        &mut e,
        "o = opts; o.DataRange = 'A3'; T = readtable('book.xlsx', o);",
    );
    check(&mut e, "isequal(T.ID, [2; 3; 4])");
    run(
        &mut e,
        "o = opts; o.SelectedVariableNames = {'ID', 'Score'}; M = readmatrix('book.xlsx', o);",
    );
    check(&mut e, "isequal(M, [T0.ID T0.Score])");
    run(&mut e, "C = readcell('book.xlsx', opts);");
    check(
        &mut e,
        "isequal(size(C), [4 3]) && strcmp(C{2, 3}, 'b') && C{4, 2} == 40.75",
    );
    // A second sheet, by index.
    run(
        &mut e,
        "writetable(table([7; 8], 'VariableNames', {'k'}), 'book.xlsx', 'Sheet', 'More'); T = readtable('book.xlsx', 'Sheet', 2);",
    );
    check(&mut e, "isequal(T.k, [7; 8])");
    fails(
        &mut e,
        "detectImportOptions('book.xlsx', 'Sheet', 'Nope')",
        "Sheet 'Nope' does not exist",
    );
}

#[test]
fn readmatrix_and_readcell_follow_the_options() {
    let (mut e, _dir) = engine();
    run(
        &mut e,
        "opts = detectImportOptions('patients.csv'); opts.SelectedVariableNames = {'Age', 'Height', 'Weight'}; M = readmatrix('patients.csv', opts);",
    );
    check(
        &mut e,
        "isequaln(M, [38 71 176.5; 43 69 163; NaN 64 131; 40 67 133.25; 49 64 119])",
    );
    run(
        &mut e,
        "opts.MissingRule = 'omitrow'; M = readmatrix('patients.csv', opts);",
    );
    check(
        &mut e,
        "isequal(size(M), [4 3]) && isequal(M(:, 1), [38; 43; 40; 49])",
    );
    run(
        &mut e,
        "C = readcell('patients.csv', detectImportOptions('patients.csv'));",
    );
    check(
        &mut e,
        "isequal(size(C), [5 5]) && strcmp(C{1, 1}, 'Smith')",
    );
    check(&mut e, "isnan(C{3, 2}) && C{4, 4} == 133.25");
    // Text variables read by readmatrix are import errors (NaN).
    run(
        &mut e,
        "M = readmatrix('patients.csv', detectImportOptions('patients.csv'));",
    );
    check(
        &mut e,
        "isequal(size(M), [5 5]) && all(isnan(M(:, 1))) && M(1, 2) == 38",
    );
}

#[test]
fn preview_reads_the_first_eight_rows() {
    let (mut e, _dir) = engine();
    run(
        &mut e,
        "opts = detectImportOptions('long.csv'); P = preview('long.csv', opts);",
    );
    check(&mut e, "istable(P) && height(P) == 8");
    check(&mut e, "isequal(P.x, (1:8)') && isequal(P.y, ((1:8).^2)')");
}

#[test]
fn display_lists_the_main_properties() {
    let (mut e, _dir) = engine();
    let text = run(&mut e, "opts = detectImportOptions('patients.csv')");
    for part in [
        "DelimitedTextImportOptions with properties:",
        "Format Properties:",
        "Delimiter: {','}",
        "Replacement Properties:",
        "MissingRule: 'fill'",
        "VariableNames: {'LastName'  'Age'  'Height' ... and 2 more}",
        "VariableTypes: {'char'  'double'  'double' ... and 2 more}",
        "VariableOptions: Show all 5 VariableOptions",
        "Location Properties:",
        "DataLines: [2 Inf]",
        "VariableNamesLine: 1",
        "To display a preview of the table, use preview",
    ] {
        assert!(text.contains(part), "{part:?} missing from\n{text}");
    }
    let text = run(&mut e, "v = getvaropts(opts, 'Age')");
    for part in [
        "NumericVariableImportOptions with properties:",
        "Name: 'Age'",
        "Type: 'double'",
        "FillValue: NaN",
        "TreatAsMissing: {}",
        "DecimalSeparator: '.'",
    ] {
        assert!(text.contains(part), "{part:?} missing from\n{text}");
    }
    let text = run(&mut e, "vs = opts.VariableOptions");
    assert!(
        text.contains("1x5 heterogeneous VariableImportOptions"),
        "{text}"
    );
    run(
        &mut e,
        "writetable(table([1; 2], 'VariableNames', {'n'}), 'one.xlsx');",
    );
    let text = run(&mut e, "s = detectImportOptions('one.xlsx')");
    for part in [
        "SpreadsheetImportOptions with properties:",
        "Sheet: ''",
        "DataRange: 'A2' (Start Cell)",
        "VariableNamesRange: 'A1'",
    ] {
        assert!(text.contains(part), "{part:?} missing from\n{text}");
    }
    run(&mut e, "c = {opts};");
    let text = run(&mut e, "c");
    assert!(
        text.contains("[1x1 matlab.io.text.DelimitedTextImportOptions]"),
        "{text}"
    );
}

#[test]
fn error_cases_name_the_problem() {
    let (mut e, _dir) = engine();
    fails(
        &mut e,
        "detectImportOptions('nofile.csv')",
        "Unable to find or open 'nofile.csv'",
    );
    run(&mut e, "opts = detectImportOptions('patients.csv');");
    fails(
        &mut e,
        "setvartype(opts, 'Nope', 'double')",
        "unknown variable name 'Nope'",
    );
    fails(
        &mut e,
        "opts.SelectedVariableNames = {'Age', 'Nope'};",
        "unknown variable name 'Nope'",
    );
    fails(&mut e, "getvaropts(opts, 9)", "out of range");
    fails(
        &mut e,
        "setvartype(opts, 'Age', 'float')",
        "'float' is not a supported variable type",
    );
    fails(&mut e, "opts.DataLines = [5 2];", "increasing order");
    fails(&mut e, "opts.DataLines = [2 4; 3 6];", "must not overlap");
    fails(&mut e, "opts.MissingRule = 'skip';", "MissingRule must be");
    fails(
        &mut e,
        "setvaropts(opts, 'LastName', 'DecimalSeparator', ',')",
        "'DecimalSeparator' is not a property of TextVariableImportOptions",
    );
    fails(
        &mut e,
        "readtimetable('patients.csv')",
        "no datetime or duration variable",
    );
    fails(
        &mut e,
        "readtimetable('events.csv', 'RowTimes', 'Nope')",
        "unknown variable name 'Nope'",
    );
    fails(
        &mut e,
        "preview('patients.csv')",
        "give a file name and import options",
    );
    fails(
        &mut e,
        "readtable('patients.csv', opts, 'Delimiter', ';')",
        "only ReadVariableNames and ReadRowNames",
    );
    fails(&mut e, "opts.Nope = 1;", "Unrecognized property 'Nope'");
    fails(
        &mut e,
        "x = opts.Nope;",
        "No appropriate method, property, or field 'Nope'",
    );
    // The options object was not changed by the failed assignments.
    check(&mut e, "isequal(opts.DataLines, [2 Inf])");
}

#[test]
fn ranges_names_row_names_extra_columns_and_empty_lines() {
    let (mut e, dir) = engine();
    // Range in a text file: names from its first line, its columns only.
    run(&mut e, "T = readtable('patients.csv', 'Range', 'B1:C3');");
    check(
        &mut e,
        "isequal(T.Properties.VariableNames, {'Age', 'Height'}) && isequal(T{:, :}, [38 71; 43 69])",
    );
    // VariableNamingRule: 'modify' makes names valid, 'preserve' keeps them.
    fs::write(dir.path().join("names.csv"), "Unit Price,2nd\n1.5,7\n").unwrap();
    run(&mut e, "m = detectImportOptions('names.csv');");
    check(&mut e, "isequal(m.VariableNames, {'UnitPrice', 'x2nd'})");
    run(
        &mut e,
        "p = detectImportOptions('names.csv', 'VariableNamingRule', 'preserve');",
    );
    check(
        &mut e,
        "isequal(p.VariableNames, {'Unit Price', '2nd'}) && strcmp(p.VariableNamingRule, 'preserve')",
    );
    // ReadVariableNames false: Var1, Var2, ... and data from line 1.
    run(
        &mut e,
        "n = detectImportOptions('long.csv', 'ReadVariableNames', false);",
    );
    check(
        &mut e,
        "isequal(n.VariableNames, {'Var1', 'Var2'}) && isequal(n.DataLines, [1 Inf]) && n.VariableNamesLine == 0",
    );
    // Row names from the first column.
    run(
        &mut e,
        "R = readtable('patients.csv', 'ReadRowNames', true);",
    );
    check(
        &mut e,
        "isequal(R.Properties.RowNames, {'Smith'; 'Johnson'; 'Williams'; 'Jones'; 'Brown'}) && width(R) == 4",
    );
    run(
        &mut e,
        "o = detectImportOptions('patients.csv'); o.RowNamesColumn = 1; R = readtable('patients.csv', o);",
    );
    check(
        &mut e,
        "strcmp(R.Properties.RowNames{2}, 'Johnson') && R{'Jones', 'Age'} == 40",
    );
    // A line with more fields than variables.
    fs::write(dir.path().join("extra.csv"), "a,b\n1,2\n3,4,5\n").unwrap();
    run(
        &mut e,
        "o = detectImportOptions('extra.csv'); T = readtable('extra.csv', o);",
    );
    check(
        &mut e,
        "isequal(T.Properties.VariableNames, {'a', 'b', 'ExtraVar1'}) && isequal(T.ExtraVar1, {''; '5'})",
    );
    run(
        &mut e,
        "o.ExtraColumnsRule = 'ignore'; T = readtable('extra.csv', o);",
    );
    check(&mut e, "width(T) == 2 && isequal(T.b, [2; 4])");
    run(&mut e, "o.ExtraColumnsRule = 'error';");
    fails(
        &mut e,
        "readtable('extra.csv', o)",
        "ExtraColumnsRule is 'error'",
    );
    // Blank lines: skipped, or rows of missing values.
    fs::write(
        dir.path().join("blank.csv"),
        "a,b\n1,2\n\n3,4\n# note\n5,6\n",
    )
    .unwrap();
    run(
        &mut e,
        "o = detectImportOptions('blank.csv', 'CommentStyle', '#'); T = readtable('blank.csv', o);",
    );
    check(&mut e, "isequal(T.a, [1; 3; 5])");
    run(
        &mut e,
        "o.EmptyLineRule = 'read'; T = readtable('blank.csv', o);",
    );
    check(&mut e, "isequaln(T.a, [1; NaN; 3; 5])");
    // readmatrix OutputType.
    run(
        &mut e,
        "o = detectImportOptions('long.csv'); M = readmatrix('long.csv', o, 'OutputType', 'int16');",
    );
    check(
        &mut e,
        "isa(M, 'int16') && isequal(size(M), [12 2]) && M(12, 2) == 144",
    );
}

/// The help page examples of the new functions, run in a temporary folder
/// (tests/help_system.rs skips them so it does not write into the checkout).
#[test]
fn import_help_examples_run_in_a_temporary_folder() {
    let pages = matrust::help::parse_pages(include_str!("../assets/help/functions.md"));
    let mut ran = 0;
    for name in [
        "detectImportOptions",
        "setvartype",
        "getvaropts",
        "setvaropts",
        "preview",
        "readtimetable",
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
    assert!(ran >= 6, "only {ran} examples ran");
}

#[test]
fn row_times_omitted_by_import_error_rule_is_an_error() {
    let (mut e, dir) = engine();
    fs::write(dir.path().join("bad_times.csv"), "v,t\n1,bad\n2,bad\n").unwrap();
    run(
        &mut e,
        "o = detectImportOptions('bad_times.csv'); o = setvartype(o, 't', 'datetime'); o.ImportErrorRule = 'omitvar';",
    );
    fails(
        &mut e,
        "TT = readtimetable('bad_times.csv', o, 'RowTimes', 't');",
        "omitted",
    );
}
