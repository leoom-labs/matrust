//! Tests for items 2-6: UnconditionalVariance, seasonal arima Description,
//! short-series parcorr, crosscorr with NaN, and lagmatrix with Inf/-Inf.
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

fn near(got: &[f64], want: &[f64], tol: f64) {
    assert_eq!(got.len(), want.len());
    for (i, (&a, &b)) in got.iter().zip(want).enumerate() {
        if b.is_nan() {
            assert!(a.is_nan(), "idx {i}: expected NaN, got {a}");
        } else {
            assert!(
                (a - b).abs() <= tol,
                "idx {i}: {a:0.17e} != {b:0.17e} (diff {})",
                (a - b).abs()
            );
        }
    }
}

#[test]
fn unconditional_variance_formulas_nan_display_and_readonly() {
    let mut e = Engine::default();

    // GARCH: c / (1 - sum(ARCH) - sum(GARCH))
    run(
        &mut e,
        "mg=garch('Constant',0.04,'GARCH',{0.7 0.1},'ARCH',0.1);",
    );
    let want_g = 0.04 / (1.0 - 0.1 - 0.8);
    check(
        &mut e,
        &format!("abs(mg.UnconditionalVariance - {want_g}) < 1e-14"),
    );

    // GJR: c / (1 - sum(ARCH) - sum(GARCH) - 0.5 * sum(Leverage))
    run(
        &mut e,
        "mj=gjr('Constant',0.05,'GARCH',0.7,'ARCH',0.1,'Leverage',0.1);",
    );
    let want_j = 0.05 / (1.0 - 0.1 - 0.7 - 0.05);
    check(
        &mut e,
        &format!("abs(mj.UnconditionalVariance - {want_j}) < 1e-14"),
    );

    // EGARCH: exp(c / (1 - sum(GARCH)))
    run(
        &mut e,
        "me=egarch('Constant',-0.2,'GARCH',0.9,'ARCH',0.15,'Leverage',-0.05);",
    );
    let want_e = (-0.2_f64 / (1.0 - 0.9)).exp();
    check(
        &mut e,
        &format!("abs(me.UnconditionalVariance - {want_e}) < 1e-14"),
    );

    // Unspecified (NaN) parameter -> NaN UnconditionalVariance.
    run(&mut e, "ug=garch(1,1);uj=gjr(1,1);ue=egarch(1,1);");
    check(
        &mut e,
        "isnan(ug.UnconditionalVariance) && isnan(uj.UnconditionalVariance) && isnan(ue.UnconditionalVariance)",
    );

    // Displayed in property list.
    let out = e.execute("mg").unwrap().output.join("\n");
    assert!(out.contains("UnconditionalVariance:"), "{out}");

    // Read-only error on assignment.
    for stmt in [
        "mg.UnconditionalVariance=1;",
        "mj.UnconditionalVariance=1;",
        "me.UnconditionalVariance=1;",
    ] {
        let err = e.execute(stmt).unwrap_err();
        assert!(err.contains("read-only"), "{stmt}: {err}");
    }
}

#[test]
fn arima_default_description_includes_seasonality() {
    let mut e = Engine::default();
    // MATLAB arima doc example:
    // Mdl = arima('Constant',0,'D',1,'Seasonality',12,'MALags',1,'SMALags',12)
    run(
        &mut e,
        "m1=arima('Constant',0,'D',1,'Seasonality',12,'MALags',1,'SMALags',12);",
    );
    check(
        &mut e,
        "strcmp(m1.Description,'ARIMA(0,1,1) Model Seasonally Integrated with Seasonal MA(12) (Gaussian Distribution)')",
    );

    run(
        &mut e,
        "m2=arima('ARLags',1,'D',1,'MALags',1,'Seasonality',12,'SARLags',12,'SMALags',12);",
    );
    check(
        &mut e,
        "strcmp(m2.Description,'ARIMA(1,1,1) Model Seasonally Integrated with Seasonal AR(12) and MA(12) (Gaussian Distribution)')",
    );
}

#[test]
fn parcorr_short_series_default_numlags_and_statsmodels_match() {
    let mut e = Engine::default();
    e.variables.insert(
        "y".into(),
        Matrix::from_column_vec(30, 1, reference::SHORT_SERIES.to_vec()).unwrap(),
    );
    // Default NumLags = min(20, T-1) = 20, default Method = 'ols'.
    run(&mut e, "[p,pl]=parcorr(y);");
    let p = data(&e, "p");
    assert_eq!(p.len(), 21);
    near(&p[..15], &reference::SHORT_PACF_OLS, 1e-12);

    run(&mut e, "[w,wl]=parcorr(y,Method='yule-walker');");
    let w = data(&e, "w");
    assert_eq!(w.len(), 21);
    near(&w[..15], &reference::SHORT_PACF_YW, 1e-12);
}

#[test]
fn crosscorr_with_nan_matches_numpy_reference() {
    let mut e = Engine::default();
    e.variables.insert(
        "nx".into(),
        Matrix::from_column_vec(12, 1, reference::NAN_X.to_vec()).unwrap(),
    );
    e.variables.insert(
        "ny".into(),
        Matrix::from_column_vec(12, 1, reference::NAN_Y.to_vec()).unwrap(),
    );
    e.variables.insert(
        "clean".into(),
        Matrix::from_column_vec(12, 1, reference::SHORT_SERIES[12..24].to_vec()).unwrap(),
    );
    run(&mut e, "x1=crosscorr(nx,clean,NumLags=4);");
    near(&data(&e, "x1"), &reference::XCF_NAN_X, 1e-12);
    run(&mut e, "x2=crosscorr(clean,ny,NumLags=4);");
    near(&data(&e, "x2"), &reference::XCF_NAN_Y, 1e-12);
    run(&mut e, "x3=crosscorr(nx,ny,NumLags=4);");
    near(&data(&e, "x3"), &reference::XCF_NAN_BOTH, 1e-12);
}

#[test]
fn lagmatrix_accepts_inf_and_neg_inf() {
    let mut e = Engine::default();
    run(&mut e, "y=[1;Inf;-Inf;4];z=lagmatrix(y,[0 1 -1]);");
    let z = data(&e, "z");
    // 4 rows x 3 cols (column-major):
    // lag 0: [1, Inf, -Inf, 4]
    // lag 1: [NaN, 1, Inf, -Inf]
    // lag -1: [Inf, -Inf, 4, NaN]
    assert_eq!(z[0], 1.0);
    assert_eq!(z[1], f64::INFINITY);
    assert_eq!(z[2], f64::NEG_INFINITY);
    assert_eq!(z[3], 4.0);
    assert!(z[4].is_nan());
    assert_eq!(z[5], 1.0);
    assert_eq!(z[6], f64::INFINITY);
    assert_eq!(z[7], f64::NEG_INFINITY);
    assert_eq!(z[8], f64::INFINITY);
    assert_eq!(z[9], f64::NEG_INFINITY);
    assert_eq!(z[10], 4.0);
    assert!(z[11].is_nan());

    // Still rejects Inf/NaN in lags and text/complex in Y.
    assert!(e.execute("lagmatrix(y,Inf)").is_err());
    assert!(e.execute("lagmatrix('abc',1)").is_err());
}
