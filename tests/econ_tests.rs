//! Independent expected values: tool/refs/econ/tests.py (statsmodels 0.15,
//! arch 9.0, NumPy/SciPy regressions, published-table interpolation; no MATLAB).
//! Explicit lags avoid statsmodels autolag; PACF is efficient OLS or biased YW;
//! MATLAB bounds use NumSTD=2, not statsmodels' normal 0.975 quantile. KPSS
//! defaults to a trend with bandwidth 0, rather than statsmodels' constant/auto.
use matrust::{Array, Engine, Matrix};
#[allow(clippy::excessive_precision, dead_code)]
mod reference {
    include!("fixtures/econ/reference.rs");
}
fn run(e: &mut Engine, s: &str) {
    e.execute(s).unwrap_or_else(|err| panic!("{s}: {err}"));
}
fn check(e: &mut Engine, s: &str) {
    run(e, &format!("assert({s});"));
}
fn data(e: &Engine, n: &str) -> Vec<f64> {
    match e.array(n).unwrap() {
        Array::Numeric(m) => m.data().to_vec(),
        a => panic!("{n}: {a:?}"),
    }
}
fn near(e: &Engine, n: &str, want: &[f64], tol: f64) {
    let got = data(e, n);
    assert_eq!(got.len(), want.len(), "{n}");
    for (i, (&a, &b)) in got.iter().zip(want).enumerate() {
        assert!(
            (a - b).abs() <= tol,
            "{n}[{i}]: {a:0.17e} != {b:0.17e} ({})",
            (a - b).abs()
        );
    }
}
fn series(e: &mut Engine, i: usize, row: bool) {
    e.variables.insert(
        "y".into(),
        Matrix::from_column_vec(
            if row { 1 } else { 128 },
            if row { 128 } else { 1 },
            reference::SERIES[i].to_vec(),
        )
        .unwrap(),
    );
}
fn fails(e: &mut Engine, s: &str, id: &str) {
    let err = e.execute(s).unwrap_err();
    assert!(err.contains(id), "{s}: {err}");
}

#[test]
fn correlograms_three_series_both_pacf_estimators_and_bounds() {
    let mut e = Engine::default();
    for i in 0..3 {
        for row in [false, true] {
            series(&mut e, i, row);
            e.variables.insert(
                "z".into(),
                Matrix::from_column_vec(128, 1, reference::SERIES[0].to_vec()).unwrap(),
            );
            run(
                &mut e,
                "[a,l,b]=autocorr(y,NumLags=8);[p,pl,pb]=parcorr(y,NumLags=8);[w,wl,wb]=parcorr(y,NumLags=8,Method='yule-walker');[x,xl,xb]=crosscorr(y,z,NumLags=8);",
            );
            near(&e, "a", &reference::ACF[i], 1e-12);
            near(&e, "p", &reference::PACF_OLS[i], 1e-12);
            near(&e, "w", &reference::PACF_YW[i], 1e-12);
            near(&e, "x", &reference::XCF[i], 1e-12);
            for n in ["b", "pb", "wb", "xb"] {
                near(&e, n, &[reference::BOUND, -reference::BOUND], 1e-12);
            }
            near(&e, "l", &(0..=8).map(|v| v as f64).collect::<Vec<_>>(), 0.);
            near(
                &e,
                "xl",
                &(-8..=8).map(|v| v as f64).collect::<Vec<_>>(),
                0.,
            );
            for n in ["a", "p", "w", "x", "l", "xl"] {
                let m = e.array(n).unwrap();
                assert_eq!(m.rows() == 1, row, "{n}");
            }
            run(
                &mut e,
                "[a,l,b]=autocorr(y,8,2,2);[p,l,pb]=parcorr(y,8,2,2);[x,l,xb]=crosscorr(y,z,8,2);",
            );
            near(
                &e,
                "b",
                &[reference::BARTLETT[i], -reference::BARTLETT[i]],
                1e-12,
            );
            near(&e, "p", &reference::PACF_OLS[i], 1e-12);
            near(&e, "x", &reference::XCF[i], 1e-12);
            run(&mut e, "[a,l,b]=autocorr(y,[],[],1.96);n=numel(a);");
            near(&e, "n", &[21.], 0.);
        }
    }
}
#[test]
fn all_test_statistics_and_pvalues_match_independent_references() {
    let mut e = Engine::default();
    for &(i, expression, want) in reference::CASES {
        series(&mut e, i, false);
        run(&mut e, expression);
        near(&e, "s", &want[..1], 1e-10);
        near(
            &e,
            "p",
            &want[1..2],
            if expression.contains("lbqtest") || expression.contains("archtest") {
                1e-10
            } else {
                1e-8
            },
        );
        near(&e, "c", &want[2..], 1e-10);
        // Decision must use the nonstandard table critical value and tail.
        let right = expression.contains("kpsstest")
            || expression.contains("Test='F'")
            || expression.contains("lbqtest")
            || expression.contains("archtest");
        let expected = if right {
            want[0] > want[2]
        } else {
            want[0] < want[2]
        };
        check(&mut e, &format!("h=={}", usize::from(expected)));
    }
}
#[test]
fn regression_outputs_match_statsmodels_level_regressions() {
    let mut e = Engine::default();
    series(&mut e, 0, false);
    for &(expr, extract, want) in reference::REG_CASES {
        run(&mut e, expr);
        run(&mut e, extract);
        near(&e, "v", want, 1e-10);
    }
}
#[test]
fn vector_options_expand_with_documented_orientation_and_struct_arrays() {
    let mut e = Engine::default();
    series(&mut e, 1, false);
    for n in ["lbqtest", "archtest", "adftest", "pptest", "kpsstest"] {
        run(
            &mut e,
            &format!("[h,p,s,c]={n}(y,Lags=[1;2;4],Alpha=0.05);"),
        );
        check(&mut e, "isequal(size(h),[3 1]) && isequal(size(p),[3 1])");
        run(
            &mut e,
            &format!("[h,p,s,c]={n}(y,Lags=[1;2;4],Alpha=[0.01 0.05 0.1]);"),
        );
        check(&mut e, "isequal(size(h),[1 3]) && isequal(size(s),[1 3])");
        fails(
            &mut e,
            &format!("{n}(y,Lags=[1 2],Alpha=[0.01 0.05 0.1])"),
            "InconsistentOptions",
        );
    }
    run(
        &mut e,
        "[h,p,s,c,r]=adftest(y,Model={'AR','ARD','TS'},Lags=[0 1 4],Test={'t1','t2','F'});",
    );
    check(
        &mut e,
        "isequal(size(r),[1 3]) && r(1).num==128 && r(2).size==126 && r(3).size==123",
    );
    check(
        &mut e,
        "isequal(size(r(3).coeff),[7 1]) && isequal(size(r(3).Cov),[7 7])",
    );
    check(
        &mut e,
        "abs(sum(r(3).res.^2)-r(3).SSE)<1e-10 && abs(r(3).RMSE^2-r(3).MSE)<1e-10",
    );
    run(
        &mut e,
        "[h,p,s,c,r]=pptest(y,Model=[\"AR\";\"ARD\";\"TS\"],Test='t2',Lags=4);",
    );
    check(
        &mut e,
        "isequal(size(r),[3 1]) && r(2).size==127 && numel(r(3).autoCov)==5 && r(3).NWEst>0",
    );
    run(&mut e, "[h,p,s,c,r]=kpsstest(y,Trend=[true false],Lags=1);");
    check(
        &mut e,
        "isequal(size(r),[1 2]) && numel(r(1).coeff)==2 && numel(r(2).coeff)==1",
    );
}
#[test]
fn published_table_points_clamps_and_warning_controls() {
    let mut e = Engine::default();
    series(&mut e, 0, false);
    run(
        &mut e,
        "[h,p,s,c]=kpsstest(y,Trend=true,Alpha=0.05);[h,p,s,d]=kpsstest(y,Trend=false,Alpha=0.01);",
    );
    near(&e, "c", &[0.146], 0.);
    near(&e, "d", &[0.739], 0.);
    run(
        &mut e,
        "[h,p,s,c]=adftest(y(1:101),Model='ARD',Test='F',Lags=0,Alpha=0.05);",
    );
    near(&e, "c", &[4.71], 1e-15);
    let out = e
        .execute("[h,p,s,c]=adftest(y,Model='AR');")
        .unwrap()
        .output
        .join("\n");
    assert!(out.contains("clamped"), "{out}");
    near(&e, "p", &[0.001], 0.);
    run(&mut e, "[msg,id]=lastwarn();");
    check(&mut e, "strcmp(id,'econ:adftest:PValueTooSmall')");
    assert!(
        e.execute("warning('off','econ:adftest:PValueTooSmall');[h,p]=adftest(y);")
            .unwrap()
            .output
            .is_empty()
    );
    assert!(
        e.execute("warning('on','econ:adftest:PValueTooSmall');[h,p]=adftest(y);")
            .unwrap()
            .output
            .join("\n")
            .contains("clamped")
    );
    let out = e.execute("[h,p,s,c]=pptest(y);").unwrap().output.join("\n");
    assert!(out.contains("clamped"));
    let upper = e
        .execute("[h,p,s,c]=kpsstest(y,Trend=true);")
        .unwrap()
        .output
        .join("\n");
    assert!(upper.contains("clamped"));
    near(&e, "p", &[0.1], 0.);
    let upper = e
        .execute("z=y;y=(1:128)'.^2+z;[h,p,s,c]=adftest(y);")
        .unwrap()
        .output
        .join("\n");
    assert!(upper.contains("clamped"));
    near(&e, "p", &[0.999], 0.);
    series(&mut e, 2, false);
    let out = e
        .execute("[h,p,s,c]=kpsstest(y,Trend=false);")
        .unwrap()
        .output
        .join("\n");
    assert!(out.contains("clamped"));
    near(&e, "p", &[0.01], 0.);
}
#[test]
fn missing_data_rules_and_real_lag_positions() {
    let mut e = Engine::default();
    // Direct available-pair reference: mean=3, c0=2.5, c1=2, c2=-1.
    run(&mut e, "[a,l,b]=autocorr([1 2 NaN 4 5],NumLags=2);");
    near(&e, "a", &[1., 0.8, -0.4], 1e-14);
    run(&mut e, "[p,l,b]=parcorr([1 2 NaN 4 5],NumLags=2);");
    near(&e, "p", &[1., 0.8, (-0.4 - 0.64) / 0.36], 2e-14);
    run(
        &mut e,
        "[x,l,b]=crosscorr([1 2 NaN 4 5],[1 2 NaN 4 5],NumLags=2);",
    );
    near(&e, "x", &[-0.4, 0.8, 1., 0.8, -0.4], 1e-14);
    // Unequal lengths zero-pad the shorter series before centering:
    // [1 2 0] (mean 1 -> [0 1 -1]) vs [1 2 3] (mean 2 -> [-1 0 1]), lags -1..1 = [-0.5, -0.5, 0.5].
    run(&mut e, "[xu,lu,bu]=crosscorr([1 2],[1 2 3],NumLags=1);");
    near(&e, "xu", &[-0.5, -0.5, 0.5], 1e-14);
    run(&mut e, "[xu2,lu2]=crosscorr([1 2 3],[1 2],NumLags=1);");
    near(&e, "xu2", &[0.5, -0.5, -0.5], 1e-14);
    fails(
        &mut e,
        "parcorr([1 2 NaN 4 5],Method='ols',NumLags=1)",
        "MissingData",
    );
    fails(&mut e, "archtest([1 2 NaN 4 5])", "MissingData");
    series(&mut e, 1, false);
    for n in ["adftest", "pptest", "kpsstest"] {
        run(
            &mut e,
            &format!(
                "a=y;a(4)=NaN;z=[y(1:3);y(5:end)];[h,p,s,c,r]={n}(a,Lags=1);[h2,p2,s2,c2]={n}(z,Lags=1);"
            ),
        );
        check(&mut e, "r.num==127 && abs(s-s2)<1e-12 && abs(p-p2)<1e-12");
    }
}
#[test]
fn correlogram_no_output_draws_stems_bounds_and_returns_handles() {
    let mut e = Engine::default();
    series(&mut e, 1, false);
    run(&mut e, "[a,l,b]=autocorr(y);");
    assert!(e.figure.is_none());
    for expr in [
        "autocorr(y,NumLags=8)",
        "parcorr(y,NumLags=8)",
        "crosscorr(y,y,NumLags=8)",
    ] {
        run(&mut e, expr);
        let f = e.figure.as_ref().unwrap();
        assert_eq!(f.axes.series.len(), 3);
        assert_eq!(f.axes.series[0].kind, matrust::plot::PlotKind::Stem);
        assert!(!f.axes.hold);
    }
    run(
        &mut e,
        "[a,l,b,h]=autocorr(y,NumLags=4);assert(numel(h)==3 && all(isgraphics(h)));close all;parcorr(y,NumLags=4,NumSTD=0);",
    );
    assert_eq!(e.figure.as_ref().unwrap().axes.series.len(), 1);
    run(&mut e, "close all;f=@autocorr;a=f(y);assert(numel(a)==21);");
    assert!(e.figure.is_none());
}
#[test]
fn lagmatrix_column_order_leads_padding_and_common_time_base() {
    let mut e = Engine::default();
    run(&mut e, "Y=[1 -1;2 -2;3 -3];[z,t]=lagmatrix(Y,[1 0 -1]);");
    let got = data(&e, "z");
    let want = [
        f64::NAN,
        1.,
        2.,
        f64::NAN,
        -1.,
        -2.,
        1.,
        2.,
        3.,
        -1.,
        -2.,
        -3.,
        2.,
        3.,
        f64::NAN,
        -2.,
        -3.,
        f64::NAN,
    ];
    for (a, b) in got.iter().zip(want) {
        assert!((*a == b) || (a.is_nan() && b.is_nan()));
    }
    near(&e, "t", &[1., 2., 3.], 0.);
    check(
        &mut e,
        "isequal(Y,[1 -1;2 -2;3 -3]) && isequal(size(z),[3 6])",
    );
    run(
        &mut e,
        "z=lagmatrix(Y,[1000000000 -1000000000]);assert(all(isnan(z(:))));z=lagmatrix([1 2 3],0);assert(isequal(z,[1 2 3]));",
    );
    run(
        &mut e,
        "[z,t]=lagmatrix(zeros(0,2),[1 -1]);assert(isequal(size(z),[0 4]) && isequal(size(t),[0 1]));",
    );
}
#[test]
fn errors_identifiers_limits_and_shadowing() {
    let mut e = Engine::default();
    series(&mut e, 1, false);
    for n in [
        "autocorr",
        "parcorr",
        "crosscorr",
        "lbqtest",
        "archtest",
        "adftest",
        "pptest",
        "kpsstest",
    ] {
        fails(&mut e, &format!("{n}()"), "NotEnoughInputs");
        let y = if n == "crosscorr" { "y,y" } else { "y" };
        fails(&mut e, &format!("{n}({y},'Bogus',1)"), "UnknownOption");
        fails(
            &mut e,
            &format!("{n}({y},'Lags',-1)"),
            if ["autocorr", "parcorr", "crosscorr"].contains(&n) {
                "UnknownOption"
            } else {
                "InvalidLags"
            },
        );
    }
    for expr in ["adftest(y,Model='BAD')", "pptest(y,Model='BAD')"] {
        fails(&mut e, expr, "InvalidModel");
    }
    for expr in [
        "adftest(y,Test='F')",
        "pptest(y,Test='F')",
        "adftest(y,Test='bad')",
    ] {
        fails(&mut e, expr, "InvalidTest");
    }
    fails(&mut e, "lbqtest(y,Lags=2,DoF=3)", "InvalidLags");
    fails(&mut e, "autocorr(y,NumLags=1.5)", "InvalidLags");
    fails(&mut e, "parcorr(y,Method='bad')", "InvalidMethod");
    fails(&mut e, "autocorr(y,NumLags=2,NumMA=2)", "InvalidOptions");
    fails(
        &mut e,
        "adftest(y,Model='ARD',Test='F',Alpha=0.001)",
        "UnsupportedAlpha",
    );
    fails(&mut e, "lagmatrix(y,[1 NaN])", "InvalidLags");
    fails(&mut e, "lagmatrix(y,1,Shape='full')", "InvalidNumInputs");
    fails(&mut e, "adftest([1 2 3])", "TooFewObservations");
    fails(&mut e, "adftest(y,Lags=100)", "TooFewObservations");
    fails(&mut e, "parcorr(y,NumLags=100)", "WorkLimit");
    fails(&mut e, "autocorr(ones(10,1))", "DegenerateData");
    fails(&mut e, "lbqtest([1 Inf 2])", "InvalidData");
    run(
        &mut e,
        "try;adftest(y,Model='bad');catch ex;id=ex.identifier;end;assert(strcmp(id,'econ:adftest:InvalidModel'));",
    );
    run(
        &mut e,
        "autocorr=[9 8];assert(autocorr(1)==9);clear autocorr;",
    );
}

#[test]
fn computation_obeys_memory_limits_and_cancellation() {
    let mut e = Engine::with_limits(matrust::engine::Limits {
        memory_bytes: 8192,
        ..Default::default()
    });
    series(&mut e, 1, false);
    fails(
        &mut e,
        "adftest(y,Model='TS',Lags=4)",
        "workspace memory budget",
    );
    let mut e = Engine::default();
    e.variables.insert(
        "y".into(),
        Matrix::from_column_vec(
            1_000_000,
            1,
            (0..1_000_000).map(|i| (i as f64 * 0.37).sin()).collect(),
        )
        .unwrap(),
    );
    // Raise the flag before the call: a 1 ms timer thread lost the race on
    // fast machines (autocorr finished first), making this test flaky.
    e.cancellation()
        .store(true, std::sync::atomic::Ordering::Relaxed);
    let result = e.execute("[a,l,b]=autocorr(y,NumLags=20);");
    assert!(result.unwrap_err().to_ascii_lowercase().contains("cancel"));
}
#[test]
#[ignore = "release numerical benchmark; run with RAYON_NUM_THREADS=1 and N"]
fn benchmark_diagnostics_release() {
    let mut e = Engine::default();
    let mut seed = 20261113u64;
    let mut previous = 0.;
    let mut y = Vec::with_capacity(4096);
    for _ in 0..4096 {
        seed ^= seed << 13;
        seed ^= seed >> 7;
        seed ^= seed << 17;
        previous = 0.65 * previous + (seed >> 11) as f64 / (1u64 << 53) as f64 - 0.5;
        y.push(previous);
    }
    e.variables
        .insert("y".into(), Matrix::from_column_vec(4096, 1, y).unwrap());
    let start = std::time::Instant::now();
    for _ in 0..20 {
        run(
            &mut e,
            "[a,l,b]=autocorr(y,NumLags=40);[p,l,b]=parcorr(y,NumLags=12);[h,p,s,c]=lbqtest(y,Lags=20);[h,p,s,c]=archtest(y,Lags=4);[h,p,s,c]=adftest(y,Model='TS',Lags=4);[h,p,s,c]=pptest(y,Model='TS',Lags=4);[h,p,s,c,r]=kpsstest(y,Lags=4);",
        );
    }
    println!(
        "20 diagnostics batches, n=4096, workers={}: {:0.3} ms",
        std::env::var("RAYON_NUM_THREADS").unwrap_or_default(),
        start.elapsed().as_secs_f64() * 1000.
    );
}
