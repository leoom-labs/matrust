//! Pandas/SciPy oracle: tool/refs/timetables/reference.py. Calendar edge
//! cases also follow the proleptic Gregorian calendar by hand.
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
const DATA: &str = "t=datetime(2023,1,1)+hours([0;12;24;36;48]); x=[1;3;5;7;9]; TT=timetable(t,x);";

#[test]
fn construction_datetime_and_duration() {
    check(
        "t=datetime(2023,1,[1;2;3]); x=[10;20;30]; TT=timetable(t,x); assert(strcmp(class(TT),'timetable')); assert(istimetable(TT)); assert(height(TT)==3); assert(width(TT)==1); assert(isequal(TT.x,x)); assert(isequal(TT.Time,t)); assert(isequal(TT.Properties.RowTimes,t));",
    );
    check(
        "t=seconds([0;1]); TT=timetable([3;4],'RowTimes',t,'VariableNames',{'value'}); assert(isequal(TT.value,[3;4])); assert(isequal(TT.Time,t));",
    );
}
#[test]
fn sample_rate_and_start_time() {
    check(
        "TT=timetable([4;5;6],'SampleRate',2); assert(isequal(seconds(TT.Time),[0;.5;1])); TT=timetable([4;5],'SampleRate',1,'StartTime',datetime(2024,1,1)); assert(isequal(TT.Time,datetime(2024,1,1)+seconds([0;1])));",
    );
}
#[test]
fn time_step_and_dimension_name() {
    check(
        "TT=timetable([3;4;5],'TimeStep',minutes(15),'StartTime',hours(2),'DimensionNames',{'Stamp','Vars'}); assert(isequal(minutes(TT.Stamp),[120;135;150])); assert(isequal(TT.Properties.RowTimes,TT.Stamp));",
    );
}
#[test]
fn indexing_properties_and_copy_on_write() {
    check(
        "TT=timetable(seconds([0;1;2]),[3;4;5]); B=TT(1:2,:); assert(istimetable(B)); assert(isequal(seconds(B.Time),[0;1])); assert(isequal(B.Var1,[3;4])); [h,w]=size(B); assert(h==2 && w==1); C=TT; TT.Var1=[9;8;7]; assert(isequal(C.Var1,[3;4;5])); TT.Properties.DimensionNames={'Stamp','Vars'}; assert(isequal(TT.Stamp,seconds([0;1;2])));",
    );
}
#[test]
fn display_has_time_first() {
    let mut e = Engine::default();
    let text = run(
        &mut e,
        "TT=timetable(datetime(2024,1,[1;2]),[4;5]); disp(TT)",
    );
    assert!(
        text.contains("timetable") && text.contains("Time") && text.contains("01-Jan-2024"),
        "{text}"
    );
    assert!(text.find("Time") < text.find("Var1"));
}
#[test]
fn table_conversion_round_trip() {
    check(
        "t=datetime(2024,1,[1;2]); x=[4;5]; T=table(t,x); TT=table2timetable(T); assert(istimetable(TT)); assert(isequal(TT.t,t)); assert(isequal(TT.x,x)); T2=timetable2table(TT); assert(strcmp(class(T2),'table')); assert(isequal(T2.t,t)); assert(isequal(T2.x,x)); TT2=table2timetable(table(x),'RowTimes',t); assert(isequal(TT2.Time,t));",
    );
}
#[test]
fn daily_aggregations_match_pandas() {
    let reference: serde_json::Value =
        serde_json::from_str(include_str!("fixtures/timetables/reference.json")).unwrap();
    let mut e = Engine::default();
    run(&mut e, DATA);
    for (method, key) in [
        ("mean", "mean"),
        ("sum", "sum"),
        ("min", "min"),
        ("max", "max"),
        ("count", "count"),
        ("firstvalue", "first"),
        ("lastvalue", "last"),
    ] {
        run(&mut e, &format!("R=retime(TT,'daily','{method}');"));
        let values: Vec<f64> = reference["daily"][key]
            .as_array()
            .unwrap()
            .iter()
            .map(|x| x.as_f64().unwrap())
            .collect();
        expected(&mut e, "R.x", &values);
    }
}
#[test]
fn explicit_interpolation_matches_pandas_and_scipy() {
    let reference: serde_json::Value =
        serde_json::from_str(include_str!("fixtures/timetables/reference.json")).unwrap();
    let mut e = Engine::default();
    run(&mut e, DATA);
    run(&mut e, "q=datetime(2023,1,1)+hours([0;6;18;30]);");
    for method in ["previous", "next", "nearest", "linear", "spline"] {
        run(&mut e, &format!("R=retime(TT,q,'{method}');"));
        let values: Vec<f64> = reference[method]
            .as_array()
            .unwrap()
            .iter()
            .map(|x| x.as_f64().unwrap())
            .collect();
        expected(&mut e, "R.x", &values);
    }
}
#[test]
fn default_missing_and_multi_column_variable() {
    let mut e = Engine::default();
    run(
        &mut e,
        "TT=timetable(seconds([0;2]),[1,2;5,6]); R=retime(TT,seconds([0;1;2]));",
    );
    expected(&mut e, "R.Var1", &[1., f64::NAN, 5., 2., f64::NAN, 6.]);
    run(&mut e, "R=retime(TT,seconds([0;1;2]),'linear');");
    expected(&mut e, "R.Var1", &[1., 3., 5., 2., 4., 6.]);
}
#[test]
fn duplicate_aggregation_and_missing_values() {
    let mut e = Engine::default();
    run(
        &mut e,
        "TT=timetable(hours([0;0;1;24;25]),[1;3;NaN;NaN;NaN]); R=retime(TT,'daily','mean');",
    );
    expected(&mut e, "R.Var1", &[2., f64::NAN]);
    run(&mut e, "R=retime(TT,'daily','sum');");
    expected(&mut e, "R.Var1", &[4., 0.]);
    run(&mut e, "R=retime(TT,'daily','count');");
    expected(&mut e, "R.Var1", &[2., 0.]);
}
#[test]
fn monthly_yearly_and_small_fixed_grids() {
    check(
        "TT=timetable(datetime(2024,[1;1;2],[1;31;1]),[1;3;5]); R=retime(TT,'monthly','sum'); assert(isequal(R.Var1,[4;5])); R=retime(TT,'yearly','sum'); assert(R.Var1==9);",
    );
    for unit in ["hourly", "minutely", "secondly"] {
        let mut e = Engine::default();
        run(&mut e, "TT=timetable(seconds([0;1]),[2;4]);");
        run(
            &mut e,
            &format!("R=retime(TT,'{unit}','sum'); assert(sum(R.Var1)==6);"),
        );
    }
}
#[test]
fn synchronize_union_intersection_and_suffixes() {
    let mut e = Engine::default();
    run(
        &mut e,
        "A=timetable(seconds([0;2]),[1;3],'VariableNames',{'x'}); B=timetable(seconds([1;2]),[8;9],'VariableNames',{'x'}); C=synchronize(A,B);",
    );
    expected(&mut e, "seconds(C.Time)", &[0., 1., 2.]);
    expected(&mut e, "C.x_TT1", &[1., f64::NAN, 3.]);
    expected(&mut e, "C.x_TT2", &[f64::NAN, 8., 9.]);
    run(&mut e, "C=synchronize(A,B,'intersection');");
    expected(&mut e, "seconds(C.Time)", &[2.]);
    expected(&mut e, "C.x_TT1", &[3.]);
    expected(&mut e, "C.x_TT2", &[9.]);
}
#[test]
fn synchronize_first_commonrange_and_methods() {
    check(
        "A=timetable(seconds([0;2;4]),[0;2;4],'VariableNames',{'a'}); B=timetable(seconds([1;3;5]),[10;30;50],'VariableNames',{'b'}); C=synchronize(A,B,'commonrange','linear'); assert(isequal(seconds(C.Time),[1;2;3;4])); assert(isequal(C.a,[1;2;3;4])); assert(isequal(C.b,[10;20;30;40])); C=synchronize(A,B,'first','nearest'); assert(isequal(C.Time,A.Time)); assert(isequal(C.b,[10;30;50]));",
    );
}
#[test]
fn calendar_duration_construction_display_and_class() {
    let mut e = Engine::default();
    let text = run(
        &mut e,
        "cd=calendarDuration(1,2,3); assert(strcmp(class(cd),'calendarDuration')); assert(iscalendarduration(cd)); assert(~isduration(cd)); disp(cd)",
    );
    assert!(text.contains("1y 2mo 3d"), "{text}");
    let text = run(&mut e, "disp(calendarDuration(0,1,5,2,0,0))");
    assert!(text.contains("1mo 5d 2h 0m 0s"), "{text}");
}
#[test]
fn calendar_builders_split_arithmetic_and_shape() {
    check(
        "cd=calyears(1)+calmonths(2)+calweeks(1)+caldays(3); [y,m,d]=split(cd,{'years','months','days'}); assert(y==1 && m==2 && d==10); cd=2*cd-caldays(1); [y,m,d]=split(cd,{'years','months','days'}); assert(y==2 && m==4 && d==19); a=calmonths([1;2;3]); assert(isequal(size(a),[3,1])); assert(calmonths(a(2))==2); assert(isequal(calmonths(a'),[1,2,3])); assert(isequal(calmonths([calmonths(1),calmonths(2)]),[1,2]));",
    );
}
#[test]
fn calendar_month_end_clamps_and_preserves_clock() {
    check(
        "assert(isequal(datetime(2023,1,31)+calmonths(1),datetime(2023,2,28))); assert(isequal(datetime(2024,1,31)+calmonths(1),datetime(2024,2,29))); assert(isequal(datetime(2024,3,31)-calmonths(1),datetime(2024,2,29))); assert(isequal(datetime(2024,2,29)+calyears(1),datetime(2025,2,28))); assert(isequal(datetime(2024,1,31,23,30,0)+calendarDuration(0,1,1,2,0,0),datetime(2024,3,2,1,30,0)));",
    );
}
#[test]
fn between_round_trips_and_negative_calendar_units() {
    check(
        "a=datetime(2023,1,31,10,0,0); b=datetime(2024,3,3,12,0,0); cd=between(a,b); assert(iscalendarduration(cd)); assert(isequal(a+cd,b)); assert(isequal(b+between(b,a),a)); assert(isequal(datetime(2024,3,31)+calmonths(-1),datetime(2024,2,29))); assert(isequal(-caldays(2),caldays(-2)));",
    );
}
#[test]
fn errors_are_explicit_and_bounded() {
    for (code, part) in [
        (
            "retime(timetable(seconds([0;1]),[1;2]),'daily','unknown')",
            "not supported",
        ),
        ("timetable([1;2],'SampleRate',0)", "positive"),
        ("timetable(seconds([0;1]),[1;2;3])", "height"),
        ("timetable([1;2],'UnsupportedOption',1)", "RowTimes"),
        ("calmonths(.5)", "integers"),
        (
            "retime(timetable(seconds([1;0]),[1;2]),seconds([0;1]),'linear')",
            "sorted",
        ),
        (
            "retime(timetable(seconds([0;1]),[1;2]),datetime(2024,1,1),'linear')",
            "match",
        ),
    ] {
        let err = Engine::default().execute(code).unwrap_err();
        assert!(err.contains(part), "{code}: {err}");
    }
}
#[test]
fn help_pages_exist() {
    for name in [
        "timetable",
        "retime",
        "synchronize",
        "calendarDuration",
        "calyears",
        "calmonths",
        "calweeks",
        "caldays",
        "table2timetable",
        "timetable2table",
        "istimetable",
        "iscalendarduration",
        "between",
    ] {
        let page = matrust::help::page(name).unwrap_or_else(|| panic!("missing help {name}"));
        assert!(
            page.contains("Syntax:")
                && page.contains("Example:")
                && page.contains("Differences from MATLAB:"),
            "{name}"
        );
    }
}

#[test]
fn calendar_time_step_and_timetable_concatenation() {
    check(
        "TT=timetable([1;2;3],'TimeStep',calmonths(1),'StartTime',datetime(2024,1,31)); assert(isequal(TT.Time,datetime(2024,[1;2;3],[31;29;31]))); A=TT(1:2,:); B=TT(3,:); C=[A;B]; assert(isequal(C.Time,TT.Time)); assert(~istable(C)); D=timetable(TT.Time,[4;5;6],'VariableNames',{'x'}); E=[TT,D]; assert(width(E)==2); assert(isequal(E.Time,TT.Time));",
    );
}

#[test]
fn spline_nonlinear_and_extrapolation_match_scipy() {
    let reference: serde_json::Value =
        serde_json::from_str(include_str!("fixtures/timetables/reference.json")).unwrap();
    let mut e = Engine::default();
    run(
        &mut e,
        "TT=timetable(seconds([0;1;2;4]),[0;1;8;64]); R=retime(TT,seconds([-1;.5;1.5;3;5]),'spline');",
    );
    let values: Vec<f64> = reference["nonlinear_spline"]
        .as_array()
        .unwrap()
        .iter()
        .map(|x| x.as_f64().unwrap())
        .collect();
    expected(&mut e, "R.Var1", &values);
    run(&mut e, "R=retime(TT,seconds([-1;5]),'linear');");
    expected(&mut e, "R.Var1", &[-1., 92.]);
}

#[test]
fn duplicate_filling_and_calendar_missing_equality() {
    check(
        "TT=timetable(seconds([0;0;1]),[2;4;6]); R=retime(TT,seconds([0;1])); assert(isequal(R.Var1,[2;6])); R=retime(TT,seconds([0;1]),'previous'); assert(isequal(R.Var1,[4;6])); assert(isequal({calmonths(1)},{calmonths(1)})); assert(~isequaln({calmonths(1)},{seconds(NaN)}));",
    );
}

#[test]
#[ignore = "release-mode timing probe; run with MATRUST_BENCH_WORKERS=1 and 4"]
fn benchmark_timetable_paths() {
    let workers = std::env::var("MATRUST_BENCH_WORKERS")
        .unwrap_or("1".into())
        .parse()
        .unwrap();
    matrust::initialize_worker_pool(workers).unwrap();
    let mut e = Engine::default();
    run(
        &mut e,
        "t=seconds([0:65535]'); x=sin([0:65535]'); TT=timetable(t,x); q=seconds([.5:2:65534.5]'); dates=datetime(2024,1,1)+days([0:65535]');",
    );
    for (name, source) in [
        ("mean", "R=retime(TT,q,'mean');"),
        ("linear", "R=retime(TT,q,'linear');"),
        ("spline", "R=retime(TT,q,'spline');"),
        ("calendar", "R=dates+calmonths(1);"),
    ] {
        run(&mut e, source);
        let start = std::time::Instant::now();
        for _ in 0..10 {
            run(&mut e, source);
        }
        eprintln!(
            "{name}: workers={workers}, source_rows=65536, iterations=10, elapsed_ms={:.3}",
            start.elapsed().as_secs_f64() * 1000.
        );
    }
}

#[test]
fn heterogeneous_fill_preserves_types_and_temporal_metadata() {
    check(
        "t=seconds([0;2]); stamp=datetime(2024,1,[1;2],'TimeZone','UTC'); label=[\"a\";\"b\"]; flag=[true;false]; n=uint64([3;4]); TT=timetable(t,stamp,label,flag,n); R=retime(TT,seconds([0;1;2])); assert(strcmp(class(R.stamp),'datetime')); assert(strcmp(R.stamp.TimeZone,'UTC')); assert(isnat(R.stamp(2))); assert(ismissing(R.label(2))); assert(strcmp(class(R.flag),'logical')); assert(strcmp(class(R.n),'uint64')); assert(R.n(2)==uint64(0)); assert(isequal(R.stamp([1;3]),stamp));",
    );
}

#[test]
fn timetable_time_assignment_sort_and_equality() {
    check(
        "A=timetable(seconds([2;0;1]),[2;0;1]); B=sortrows(A); assert(isequal(seconds(B.Time),[0;1;2])); assert(isequal(B.Var1,[0;1;2])); C=B; C.Properties.RowTimes=seconds([3;4;5]); assert(~isequal(B,C)); assert(isequal(seconds(B.Time),[0;1;2])); C.Time=seconds([0;1;2]); assert(isequal(B,C)); C(2,:)=[]; assert(isequal(seconds(C.Time),[0;2]));",
    );
}

#[test]
fn zero_rows_zero_variables_and_empty_synchronization() {
    check("E=timetable(); assert(istimetable(E)); assert(height(E)==0 && width(E)==0);");
    check(
        "TT=timetable(seconds(zeros(0,1)),zeros(0,1)); R=retime(TT,seconds([0;1])); assert(height(R)==2); assert(all(isnan(R.Var1))); A=timetable(seconds([0;1])); assert(width(A)==0); assert(height(A)==2); B=A(1,:); assert(isequal(seconds(B.Time),0));",
    );
}

#[test]
fn row_time_vector_orientation_is_normalized() {
    check(
        "TT=timetable(seconds([0,1,2]),[3;4;5]); assert(isequal(size(TT.Time),[3,1])); R=retime(TT,seconds([0,.5,1]),'linear'); assert(isequal(R.Var1,[3;3.5;4])); TT.Properties.RowTimes=seconds([4,5,6]); assert(isequal(seconds(TT.Time),[4;5;6]));",
    );
}

#[test]
fn excessive_resampling_grid_is_rejected_before_allocation() {
    let error = Engine::default()
        .execute("TT=timetable(seconds([0;1000000000]),[1;2]); R=retime(TT,'secondly','sum');")
        .unwrap_err();
    assert!(error.contains("workspace exceeds"), "{error}");
}

#[test]
fn review_timetable_scalar_text_and_categorical_variables() {
    check(
        "T=timetable(seconds(0),\"hello\"); assert(istimetable(T)); assert(height(T)==1 && width(T)==1); assert(T.Var1==\"hello\"); T=timetable(seconds(0),categorical({'a'})); assert(iscategorical(T.Var1)); assert(isequal(cellstr(T.Var1),{'a'}));",
    );
    check(
        "T=timetable(seconds(0),\"RowTimes\"); assert(T.Var1==\"RowTimes\"); T=timetable(seconds(0),\"hello\",'VariableNames',{'greeting'}); assert(T.greeting==\"hello\"); T=timetable(\"hello\",'RowTimes',seconds(0)); assert(T.Var1==\"hello\");",
    );
}

#[test]
fn review_empty_timetable_concatenation_keeps_rowtimes() {
    check(
        "T=timetable(); U=[T;T]; assert(istimetable(U)); assert(height(U)==0 && width(U)==0); assert(isduration(U.Properties.RowTimes)); assert(isequal(size(U.Properties.RowTimes),[0 1])); V=[T,T]; assert(istimetable(V)); V=[T;[];T]; assert(istimetable(V));",
    );
}

#[test]
fn review_retime_explicit_final_bin_includes_its_right_edge() {
    check(
        "T=timetable(seconds([0;1;2]),[1;2;3]); R=retime(T,seconds([0;1]),'sum'); assert(isequal(R.Var1,[1;5])); T=timetable(seconds([0;1;2;3]),[1;2;3;100]); R=retime(T,seconds([0;1]),'sum'); assert(isequal(R.Var1,[1;5])); R=retime(T,seconds([0;1;2]),'sum'); assert(isequal(R.Var1,[1;2;103]));",
    );
}

#[test]
fn review_1111_retime_previous_next_outside_are_missing() {
    // MathWorks retime: 'previous' has no preceding row before the first
    // row time, and 'next' has no following row after the last. The
    // 'previous','EndValues',1000 example fills only outside that range.
    // https://www.mathworks.com/help/matlab/ref/timetable.retime.html
    check(
        "TT=timetable(seconds([1;2]),[10;20]); R=retime(TT,seconds([0;1;2]),'previous'); assert(isequaln(R.Var1,[NaN;10;20])); R=retime(TT,seconds([1;2;3]),'previous'); assert(isequal(R.Var1,[10;20;20])); R=retime(TT,seconds([1;2;3]),'next'); assert(isequaln(R.Var1,[10;20;NaN])); R=retime(TT,seconds([0;1;2]),'next'); assert(isequal(R.Var1,[10;10;20])); A=timetable(seconds([0;2]),[1;3],'VariableNames',{'a'}); B=timetable(seconds([1;2]),[8;9],'VariableNames',{'b'}); C=synchronize(A,B,'union','previous'); assert(isequaln(C.a,[1;1;3])); assert(isequaln(C.b,[NaN;8;9]));",
    );
}

#[test]
fn review_1111_sample_rate_row_time_is_round_i_over_fs() {
    // Time = StartTime + (i-1)/Fs. Python: round(3 * 1e9 / 3) = 1000000000 ns
    // (row 4 is 1 s) and round(3000000 * 1e9 / 3) = 1e15 ns (1e6 s).
    check(
        "TT=timetable(zeros(4,1),'SampleRate',3); assert(isequal(seconds(TT.Time(4)),1)); n=3000001; TT=timetable(zeros(n,1),'SampleRate',3); assert(isequal(seconds(TT.Time(n)),1e6));",
    );
}
