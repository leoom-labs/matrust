use matrust::{Array, Engine};

fn run(e: &mut Engine, code: &str) -> String {
    e.execute(code)
        .unwrap_or_else(|err| panic!("{code}: {err}"))
        .output
        .concat()
}

fn check(code: &str) {
    let mut e = Engine::default();
    run(&mut e, code);
}

fn numbers(e: &mut Engine, code: &str) -> Vec<f64> {
    run(e, &format!("result__={code};"));
    match e.array("result__").unwrap() {
        Array::Numeric(m) => m.data().to_vec(),
        x => panic!("expected numbers: {x:?}"),
    }
}

fn expected(e: &mut Engine, code: &str, values: &[f64]) {
    let actual = numbers(e, code);
    assert_eq!(actual.len(), values.len(), "{code}: {actual:?}");
    for (a, b) in actual.iter().zip(values) {
        assert!(
            (a - b).abs() < 1e-9 || (a.is_nan() && b.is_nan()),
            "{code}: {actual:?} != {values:?}"
        );
    }
}

#[test]
fn summary_prints_table_and_timetable() {
    let mut e = Engine::default();
    let out = run(
        &mut e,
        "T = table([1; NaN; 3; 5], [true; false; true; true], categorical({'a'; 'b'; 'a'; 'a'}), [\"x\"; \"y\"; \"z\"; \"w\"], datetime(2024,1,[1;2;3;4]), 'VariableNames', {'Num', 'Flag', 'Cat', 'Txt', 'Dt'});\n\
         T.Properties.VariableDescriptions = {'Numeric col', '', 'Category col', '', ''};\n\
         summary(T);",
    );
    for expected_sub in [
        "Num:",
        "4x1 double",
        "Description:  Numeric col",
        "Min",
        "Median",
        "Max",
        "NumMissing",
        "Flag:",
        "4x1 logical",
        "True",
        "False",
        "Cat:",
        "4x1 categorical",
        "Txt:",
        "4x1 string",
        "Dt:",
        "4x1 datetime",
    ] {
        assert!(
            out.contains(expected_sub),
            "missing '{expected_sub}' in summary output:\n{out}"
        );
    }

    let tt_out = run(
        &mut e,
        "TT = timetable(seconds([1; 2; 3]), [10; 20; 30], 'VariableNames', {'Val'});\n\
         summary(TT);",
    );
    assert!(tt_out.contains("RowTimes:"), "missing RowTimes:\n{tt_out}");
    assert!(tt_out.contains("Val:"), "missing Val:\n{tt_out}");
}

#[test]
fn summary_struct_output_and_unknown_types() {
    check(
        "T = table([1; NaN; 3; 5], [true; false; true; true], categorical({'a'; 'b'; 'a'; 'a'}), [\"x\"; \"y\"; \"z\"; \"w\"], datetime(2024,1,[1;2;3;4]), {1; 2; 3; 4}, 'VariableNames', {'Num', 'Flag', 'Cat', 'Txt', 'Dt', 'Misc'});\n\
         T.Properties.VariableDescriptions = {'Numeric col', '', '', '', '', ''};\n\
         s = summary(T);\n\
         assert(isstruct(s));\n\
         assert(isequal(s.Num.Size, [4 1]));\n\
         assert(strcmp(s.Num.Type, 'double'));\n\
         assert(strcmp(s.Num.Description, 'Numeric col'));\n\
         assert(s.Num.Min == 1 && s.Num.Median == 3 && s.Num.Max == 5 && s.Num.NumMissing == 1);\n\
         assert(s.Flag.True == 3 && s.Flag.False == 1);\n\
         assert(isequal(s.Cat.Counts, [3; 1]));\n\
         assert(strcmp(s.Txt.Type, 'string'));\n\
         assert(s.Dt.Min == datetime(2024,1,1) && s.Dt.Max == datetime(2024,1,4) && s.Dt.NumMissing == 0);\n\
         assert(strcmp(s.Misc.Type, 'cell') && isequal(s.Misc.Size, [4 1]));",
    );
    check(
        "TT = timetable(seconds([1; 2; 3]), [10; 20; 30], 'VariableNames', {'Val'});\n\
         s = summary(TT);\n\
         assert(s.Time.Min == seconds(1) && s.Time.Median == seconds(2) && s.Time.Max == seconds(3));\n\
         assert(s.Val.Min == 10 && s.Val.Max == 30);",
    );
}

#[test]
fn retime_regular_timestep_and_samplerate() {
    let mut e = Engine::default();
    run(
        &mut e,
        "tt = timetable(seconds((1:4)'), [1; 2; 3; 4]);\n\
         r1 = retime(tt, 'regular', 'mean', 'TimeStep', seconds(2));\n\
         r2 = retime(tt, seconds((0:2:4)'), 'mean');\n\
         r3 = retime(tt, 'regular', 'mean', 'SampleRate', 0.5);",
    );
    expected(&mut e, "r1.Var1", &[1.0, 2.5, 4.0]);
    expected(&mut e, "seconds(r1.Time)", &[0.0, 2.0, 4.0]);
    run(
        &mut e,
        "assert(isequal(r1.Var1, r2.Var1) && isequal(r1.Time, r2.Time));\n\
         assert(isequal(r1.Var1, r3.Var1) && isequal(r1.Time, r3.Time));",
    );

    // Non-aggregating regular grid spans past last sample if needed
    expected(
        &mut e,
        "retime(tt, 'regular', 'linear', 'TimeStep', seconds(2)).Var1",
        &[0.0, 2.0, 4.0],
    );
    // Synchronize with 'regular' and 'TimeStep'
    check(
        "a = timetable(seconds([1; 3]), [10; 30], 'VariableNames', {'a'});\n\
         b = timetable(seconds([2; 4]), [20; 40], 'VariableNames', {'b'});\n\
         c = synchronize(a, b, 'regular', 'mean', 'TimeStep', seconds(2));\n\
         assert(isequal(seconds(c.Time), [0; 2; 4]));\n\
         assert(isequal(c.a, [10; 30; NaN]) || (c.a(1)==10 && c.a(2)==30 && isnan(c.a(3))));",
    );
}

#[test]
fn textscan_end_of_line() {
    let mut e = Engine::default();
    // CR-only string
    run(
        &mut e,
        "s_cr = ['a,1' char(13) 'b,2' char(13) 'c,3'];\n\
         c = textscan(s_cr, '%s %f', 'Delimiter', ',', 'EndOfLine', '\\r');",
    );
    expected(&mut e, "c{2}", &[1.0, 2.0, 3.0]);
    run(&mut e, "assert(isequal(c{1}, {'a'; 'b'; 'c'}));");

    // CRLF-only EndOfLine
    run(
        &mut e,
        "s_crlf = ['1,2' char(13) char(10) '3,4'];\n\
         c2 = textscan(s_crlf, '%f %f', 'Delimiter', ',', 'EndOfLine', '\\r\\n');",
    );
    expected(&mut e, "c2{1}", &[1.0, 3.0]);
    expected(&mut e, "c2{2}", &[2.0, 4.0]);

    // Empty EndOfLine ('') treats newlines as not ending a record
    run(
        &mut e,
        "s_none = ['1,2' char(10) '3,4'];\n\
         c3 = textscan(s_none, '%s %s %s', 'Delimiter', ',', 'EndOfLine', '');",
    );
    run(
        &mut e,
        "assert(strcmp(c3{1}{1}, '1'));\n\
         assert(strcmp(c3{2}{1}, ['2' char(10) '3']));\n\
         assert(strcmp(c3{3}{1}, '4'));",
    );

    // HeaderLines with EndOfLine '\r'
    run(
        &mut e,
        "s_hdr = ['hdr' char(13) '10' char(13) '20'];\n\
         c4 = textscan(s_hdr, '%f', 'HeaderLines', 1, 'EndOfLine', '\\r');",
    );
    expected(&mut e, "c4{1}", &[10.0, 20.0]);

    // EndOfLine '\r\n' means CR, LF, or CRLF
    run(
        &mut e,
        "c5 = textscan(sprintf('1,2\\r3,4'), '%f%f', 'Delimiter', ',', 'EndOfLine', '\\r\\n');\n\
         c6 = textscan(sprintf('1,2\\n3,4'), '%f%f', 'Delimiter', ',', 'EndOfLine', '\\r\\n');",
    );
    expected(&mut e, "c5{1}", &[1.0, 3.0]);
    expected(&mut e, "c5{2}", &[2.0, 4.0]);
    expected(&mut e, "c6{1}", &[1.0, 3.0]);
    expected(&mut e, "c6{2}", &[2.0, 4.0]);

    // Single-character EndOfLine other than \r or \n
    run(&mut e, "c7 = textscan('1:2', '%f', 'EndOfLine', ':');");
    expected(&mut e, "c7{1}", &[1.0, 2.0]);
}

#[test]
fn summary_zero_width_multicolumn_duration_realmax_and_defaults() {
    let mut e = Engine::default();
    // Zero-width numeric and duration variables
    run(&mut e, "summary(table(seconds(zeros(2,0))));");
    check(
        "s_dur0 = summary(table(seconds(zeros(2,0))));\n\
         assert(isequal(s_dur0.Var1.Size, [2 0]));\n\
         assert(isempty(s_dur0.Var1.Min));\n\
         s_num0 = summary(table(zeros(2,0)));\n\
         assert(isequal(s_num0.Var1.Size, [2 0]));\n\
         assert(isempty(s_num0.Var1.Min));\n\
         assert(isempty(s_num0.Var1.Mean));\n\
         assert(isempty(s_num0.Var1.Std));",
    );

    // Multi-column duration prints all columns
    let dur_out = run(&mut e, "summary(table(seconds([1 10; 2 20])));");
    assert!(
        dur_out.contains("1 sec") && dur_out.contains("10 sec"),
        "expected both duration columns in output:\n{dur_out}"
    );
    assert!(
        dur_out.contains("2 sec") && dur_out.contains("20 sec"),
        "expected both duration column maxes in output:\n{dur_out}"
    );

    // Median of large finite values stays finite
    check(
        "s_big = summary(table([realmax; realmax]));\n\
         assert(isfinite(s_big.Var1.Median) && s_big.Var1.Median == realmax);\n\
         assert(isfinite(s_big.Var1.Mean) && s_big.Var1.Mean == realmax);\n\
         assert(s_big.Var1.Std == 0);",
    );

    // Default numeric statistics include Mean and Std
    let num_out = run(&mut e, "summary(table([1; 3]));");
    assert!(
        num_out.contains("Mean") && num_out.contains("Std"),
        "expected Mean and Std in summary output:\n{num_out}"
    );
    check(
        "s_def = summary(table([1; 3]));\n\
         assert(s_def.Var1.Min == 1);\n\
         assert(s_def.Var1.Median == 2);\n\
         assert(s_def.Var1.Max == 3);\n\
         assert(s_def.Var1.Mean == 2);\n\
         assert(abs(s_def.Var1.Std - sqrt(2)) < 1e-12);\n\
         assert(s_def.Var1.NumMissing == 0);",
    );
}

#[test]
fn retime_datetime_samplerate_and_fillwithconstant() {
    let mut e = Engine::default();
    // Exact integer nanosecond arithmetic for SampleRate on absolute datetime
    check(
        "tt = timetable(datetime(2024,1,1), 7);\n\
         r_sr = retime(tt, 'regular', 'mean', 'SampleRate', 1e6);\n\
         r_ts = retime(tt, 'regular', 'mean', 'TimeStep', seconds(1e-6));\n\
         assert(isequal(r_sr.Var1, 7));\n\
         assert(isequal(r_sr.Time, r_ts.Time));",
    );
    check(
        "tt2 = timetable(datetime(2024,1,1) + seconds([0; 1]), [10; 20]);\n\
         r_frac = retime(tt2, 'regular', 'mean', 'SampleRate', 3);\n\
         assert(height(r_frac) == 4);",
    );

    // fillwithconstant for regular and explicit grids with default and custom Constant
    run(
        &mut e,
        "tt3 = timetable(seconds([0; 2]), [1; 3]);\n\
         r_reg0 = retime(tt3, 'regular', 'fillwithconstant', 'TimeStep', seconds(1));\n\
         r_reg9 = retime(tt3, 'regular', 'fillwithconstant', 'TimeStep', seconds(1), 'Constant', 9);\n\
         r_exp0 = retime(tt3, seconds([0; 1; 2]), 'fillwithconstant');\n\
         r_exp5 = retime(tt3, seconds([0; 1; 2]), 'fillwithconstant', 'Constant', -5);",
    );
    expected(&mut e, "r_reg0.Var1", &[1.0, 0.0, 3.0]);
    expected(&mut e, "r_reg9.Var1", &[1.0, 9.0, 3.0]);
    expected(&mut e, "r_exp0.Var1", &[1.0, 0.0, 3.0]);
    expected(&mut e, "r_exp5.Var1", &[1.0, -5.0, 3.0]);
}
