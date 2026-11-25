//! prctile, quantile, zscore, skewness, kurtosis, mad, normalize, rescale,
//! smoothdata and bounds.
//!
//! Expected values: tests/fixtures/stats/gaps.json from tool/refs/stats/gaps.py
//! (NumPy 'hazen' percentiles, SciPy skew/kurtosis/median_abs_deviation, and
//! NumPy implementations of the documented definitions), plus cases computed
//! by hand from the definitions where NumPy's default differs (percentile
//! method, excess kurtosis). No MATLAB evidence. GNU Octave was not available
//! where the fixture was generated.
//!
//! Tolerances: cases marked "exact" (order statistics, bounds, constant data)
//! compare bit for bit apart from NaN payloads; every other value must agree
//! to 1e-12 relative, with an absolute floor of 1e-12 for values below 1 in
//! magnitude (z-scores and centred data near 0).
use matrust::{Array, Engine, Matrix};
use serde_json::Value as Json;

fn fixture() -> Json {
    serde_json::from_str(include_str!("fixtures/stats/gaps.json")).unwrap()
}

fn number(j: &Json) -> f64 {
    match j {
        Json::Null => f64::NAN,
        Json::String(s) if s == "Inf" => f64::INFINITY,
        Json::String(s) if s == "-Inf" => f64::NEG_INFINITY,
        _ => j.as_f64().unwrap(),
    }
}

fn matrix(e: &Engine, name: &str) -> Matrix {
    if let Some(m) = e.variables.get(name) {
        return m.clone();
    }
    match e.array(name) {
        Some(Array::Numeric(m)) => m.clone(),
        other => panic!("missing numeric {name}: {other:?}"),
    }
}

fn run(source: &str) -> Engine {
    let mut e = Engine::default();
    if let Err(err) = e.execute(source) {
        panic!("{source}\n{err}");
    }
    e
}

fn error(source: &str) -> String {
    Engine::default()
        .execute(source)
        .expect_err(&format!("{source} should fail"))
}

fn same(actual: f64, expected: f64, exact: bool) -> bool {
    if expected.is_nan() {
        return actual.is_nan();
    }
    if exact || expected.is_infinite() {
        return actual == expected;
    }
    (actual - expected).abs() <= 1e-12 * expected.abs().max(1.)
}

fn check(
    e: &Engine,
    name: &str,
    rows: usize,
    cols: usize,
    expected: &[f64],
    exact: bool,
    code: &str,
) {
    let m = matrix(e, name);
    assert_eq!((m.rows(), m.cols()), (rows, cols), "{name} shape in {code}");
    for (k, (&a, &b)) in m.data().iter().zip(expected).enumerate() {
        assert!(
            same(a, b, exact),
            "{name}[{k}] = {a:e}, expected {b:e} in {code}"
        );
    }
}

/// Every fixture case whose code contains `filter`.
fn replay(filter: &str) {
    let fixture = fixture();
    let mut ran = 0;
    for case in fixture["cases"].as_array().unwrap() {
        let code = case["code"].as_str().unwrap();
        if !code.contains(filter) {
            continue;
        }
        ran += 1;
        let e = run(code);
        let exact = case["tol"] == "exact";
        for (name, out) in case["outputs"].as_object().unwrap() {
            let data: Vec<f64> = out["data"].as_array().unwrap().iter().map(number).collect();
            let rows = out["rows"].as_u64().unwrap() as usize;
            let cols = out["cols"].as_u64().unwrap() as usize;
            check(&e, name, rows, cols, &data, exact, code);
        }
    }
    assert!(ran >= 5, "only {ran} fixture cases for {filter}");
}

fn values(source: &str, name: &str) -> Vec<f64> {
    matrix(&run(source), name).data().to_vec()
}

#[test]
fn prctile_and_quantile_match_the_midpoint_rule_fixture() {
    replay("prctile(");
    replay("quantile(");
}

#[test]
fn prctile_by_hand_from_the_midpoint_definition() {
    // Sorted [1 2 3 6 10]: the k-th value is the 100(k-0.5)/5 = 10, 30, 50,
    // 70, 90th percentile. 40 lies halfway between 2 and 3; 75 is a quarter of
    // the way from 6 to 10; 5 and 95 lie outside and take the extremes.
    // NumPy's default ('linear') would give 2.6 for p = 40 instead of 2.5.
    let y = values("y = prctile([6 3 2 10 1], [5 10 40 50 75 95]);", "y");
    assert_eq!(y, [1., 1., 2.5, 3., 7., 10.]);
    // quantile of four values: positions 0.125, 0.375, 0.625, 0.875.
    let q = values("q = quantile([4 1 3 2], [0.125 0.25 0.5 0.875 1]);", "q");
    assert_eq!(q, [1., 1.5, 2.5, 4., 4.]);
    // quantile(X, N) uses the probabilities (1:N)/(N+1).
    assert_eq!(values("q = quantile(1:10, 3);", "q"), [3., 5.5, 8.]);
}

#[test]
fn prctile_shapes_follow_p_for_vectors_and_dim_for_matrices() {
    let e = run(
        "a = prctile(1:5, [10; 50]); b = prctile((1:5)', [10 50]); c = prctile(magic(4), [25 75]); d = prctile(magic(4), 50, 2);",
    );
    let shape = |n: &str| {
        let m = matrix(&e, n);
        (m.rows(), m.cols())
    };
    assert_eq!(shape("a"), (2, 1));
    assert_eq!(shape("b"), (1, 2));
    assert_eq!(shape("c"), (2, 4));
    assert_eq!(shape("d"), (4, 1));
}

#[test]
fn zscore_matches_the_fixture_and_handles_zero_deviation() {
    replay("zscore(");
}

#[test]
fn skewness_and_kurtosis_match_scipy() {
    replay("skewness(");
    replay("kurtosis(");
}

#[test]
fn kurtosis_is_not_the_excess_kurtosis() {
    // By hand: [1 2 3 4 5] has m2 = 2, m4 = 6.8, so k = 6.8 / 4 = 1.7
    // (SciPy's default, the excess kurtosis, would report -1.3).
    let k = values("k = kurtosis([1 2 3 4 5]);", "k")[0];
    assert!((k - 1.7).abs() <= 1e-15, "{k}");
    // Bias-corrected: (n-1)/((n-2)(n-3)) ((n+1)k - 3(n-1)) + 3 = 4/6 (10.2 - 12) + 3 = 1.8.
    let k0 = values("k = kurtosis([1 2 3 4 5], 0);", "k")[0];
    assert!((k0 - 1.8).abs() <= 1e-14, "{k0}");
    // Symmetric data has zero skewness.
    assert_eq!(values("s = skewness([1 2 3 4 5]);", "s"), [0.]);
}

#[test]
fn mad_matches_the_fixture() {
    replay("mad(");
}

#[test]
fn normalize_matches_every_method_in_the_fixture() {
    replay("normalize(");
}

#[test]
fn rescale_matches_the_fixture() {
    replay("rescale(");
}

#[test]
fn bounds_matches_min_and_max() {
    replay("bounds(");
}

#[test]
fn smoothdata_matches_every_method_in_the_fixture() {
    for method in [
        "'movmean'",
        "'movmedian'",
        "'gaussian'",
        "'lowess'",
        "'loess'",
        "'rlowess'",
        "'rloess'",
        "'sgolay'",
        "smoothdata(S",
        "smoothdata(SM",
    ] {
        replay(method);
    }
}

#[test]
fn smoothdata_interior_sgolay_is_the_classical_filter() {
    // Interior Savitzky-Golay (window 5, degree 2) smoothing coefficients are
    // (-3 12 17 12 -3)/35 (Savitzky & Golay 1964, table I).
    let y = values("y = smoothdata([0 0 0 1 0 0 0], 'sgolay', 5);", "y");
    for (a, b) in y[2..5].iter().zip([12. / 35., 17. / 35., 12. / 35.]) {
        assert!((a - b).abs() <= 1e-14, "{y:?}");
    }
    // A quadratic is reproduced exactly by loess and sgolay, ends included.
    let e = run("x = (1:9).^2; a = smoothdata(x, 'loess', 5); b = smoothdata(x, 'sgolay', 5);");
    for name in ["a", "b"] {
        for (k, v) in matrix(&e, name).data().iter().enumerate() {
            let x = ((k + 1) * (k + 1)) as f64;
            assert!((v - x).abs() <= 1e-11 * x, "{name}: {v} vs {x}");
        }
    }
}

#[test]
fn robust_smoothing_resists_an_outlier() {
    // Zig-zag data so that residuals are not exactly zero (a zero median
    // residual ends the robustness passes, as in Cleveland's procedure).
    // Reference (tool/refs/stats/gaps.py definitions): 17.1513 and 3.9013.
    let e = run(
        "x = [1 3 2 4 3 50 4 6 5 7 6]; a = smoothdata(x, 'lowess', 5); b = smoothdata(x, 'rlowess', 5);",
    );
    let (a, b) = (matrix(&e, "a"), matrix(&e, "b"));
    assert!(
        (a.data()[5] - 17.151284407684557).abs() < 1e-10,
        "{:?}",
        a.data()
    );
    assert!(
        (b.data()[5] - 3.901320627943428).abs() < 1e-10,
        "{:?}",
        b.data()
    );
}

#[test]
fn help_examples_give_the_values_they_state() {
    let cases: &[(&str, &[f64])] = &[
        ("y = prctile([6 3 2 10 1], [5 50 95]);", &[1., 3., 10.]),
        (
            "y = quantile([1 2 3 4], [0.25 0.5 0.75]);",
            &[1.5, 2.5, 3.5],
        ),
        ("y = zscore([1 2 3]);", &[-1., 0., 1.]),
        ("y = mad([1 2 3 4 100]);", &[31.2]),
        ("y = mad([1 2 3 4 100], 1);", &[1.]),
        ("y = normalize([2 4 6], 'range', [-1 1]);", &[-1., 0., 1.]),
        (
            "y = normalize([1 2 3; 4 6 8], 2, 'center', 'median');",
            &[-1., -2., 0., 0., 1., 2.],
        ),
        ("y = rescale([1 2 3 5]);", &[0., 0.25, 0.5, 1.]),
        (
            "y = smoothdata([1 5 2 8 3], 'movmean', 3);",
            &[3., 8. / 3., 5., 13. / 3., 5.5],
        ),
    ];
    for (code, expected) in cases {
        let y = values(code, "y");
        assert_eq!(y.len(), expected.len(), "{code}");
        for (a, b) in y.iter().zip(*expected) {
            assert!((a - b).abs() <= 1e-12 * b.abs().max(1.), "{code}: {y:?}");
        }
    }
    let e = run(
        "[z, mu, sigma] = zscore([2 4 4 4 5 5 7 9], 1); [b, w] = smoothdata([1 5 2 8 3 9 4], 'movmedian', 3); [s, l] = bounds([3 NaN 1 7]);",
    );
    assert_eq!(matrix(&e, "mu").data(), [5.]);
    assert_eq!(matrix(&e, "sigma").data(), [2.]);
    assert_eq!(matrix(&e, "w").data(), [3.]);
    assert_eq!(matrix(&e, "s").data(), [1.]);
    assert_eq!(matrix(&e, "l").data(), [7.]);
    for (code, expected) in [
        ("y = skewness([1 2 3 10]);", 1.0182),
        ("y = skewness([1 2 3 10], 0);", 1.7636),
        ("y = kurtosis([1 2 3 10]);", 2.2304),
        ("y = kurtosis([1 2 3 4 10], 0);", 6.1520),
    ] {
        let y = values(code, "y")[0];
        assert!((y - expected).abs() < 5e-5, "{code}: {y}");
    }
}

#[test]
fn logical_input_is_treated_as_double() {
    assert_eq!(values("y = prctile([true false true], 50);", "y"), [1.]);
    assert_eq!(values("y = rescale([true false]);", "y"), [1., 0.]);
}

#[test]
fn invalid_arguments_are_rejected_explicitly() {
    for (code, message) in [
        ("prctile([1 2 3], 101)", "between 0 and 100"),
        ("prctile([1 2 3], [])", "nonempty vector"),
        ("quantile([1 2 3], -0.5)", "between 0 and 1"),
        ("prctile([1 2], 50, 0)", "positive integer"),
        ("prctile([1 2], 50, 'all')", "not supported yet"),
        ("zscore([1 2 3], 2)", "flag must be 0, 1 or []"),
        ("skewness([1 2 3], 0.5)", "flag must be 0, 1 or []"),
        ("mad([1 2 3], 2)", "flag must be 0, 1 or []"),
        ("normalize([1 2 3], 'unknown')", "Use normalize"),
        ("normalize([1 2 3], 'range', [0 1 2])", "two-element"),
        ("normalize([1 2 3], 'norm', -1)", "positive number"),
        (
            "normalize([1 2; 3 4], 'center', [1 2 3])",
            "one value per vector",
        ),
        ("normalize([1 2 3], 'DataVariables', 1)", "tables"),
        ("rescale([1 2 3], 1)", "Use rescale"),
        ("rescale([1 2 3], 0, 1, 'InputMin', [1 2])", "compatible"),
        (
            "smoothdata([1 2 3], 'sgolay', 3, 'Degree', 3)",
            "less than the window",
        ),
        ("smoothdata([1 2 3], 'movmean', 2.5)", "non-integer windows"),
        (
            "smoothdata([1 2 3], 'movmean', 'Degree', 1)",
            "only to the 'sgolay'",
        ),
        (
            "smoothdata([1 2 3], 'SamplePoints', [1 2 3])",
            "not supported yet",
        ),
        (
            "smoothdata([1 2 3], 'movmean', 3, 'SmoothingFactor', 0.5)",
            "cannot be combined",
        ),
        (
            "smoothdata([1 2 3], 'SmoothingFactor', 2)",
            "between 0 and 1",
        ),
        ("smoothdata([1 2 3], 'magic')", "Use smoothdata"),
        ("bounds([1 2 3], 'all')", "not supported yet"),
        ("bounds([1 2 3], 1, 2)", "Use [S, L] = bounds"),
        ("[a, b] = mad([1 2 3])", "too many output"),
        ("[a, b, c, d] = zscore([1 2 3])", "too many output"),
        ("skewness([1+2i 3])", "complex"),
        ("prctile({1}, 50)", "real numeric"),
        ("prctile(ones(2,2,2), 50)", "N-D arrays"),
    ] {
        let err = error(code);
        assert!(err.contains(message), "{code}: {err}");
    }
}

#[test]
fn every_new_name_is_registered() {
    for name in [
        "prctile",
        "quantile",
        "zscore",
        "skewness",
        "kurtosis",
        "mad",
        "normalize",
        "rescale",
        "smoothdata",
        "bounds",
    ] {
        assert!(matrust::session::FUNCTIONS.contains(&name), "{name}");
        let e = run(&format!("t = exist('{name}');"));
        assert_eq!(matrix(&e, "t").data(), [5.], "{name}");
    }
}

#[test]
fn large_inputs_stay_fast_and_bounded() {
    // 200k samples: the interior coefficient reuse keeps loess linear in n.
    let started = std::time::Instant::now();
    let e = run(
        "x = sin((1:200000)/500) + mod(1:200000, 7)/10; a = smoothdata(x, 'loess', 21); p = prctile(x, [1 50 99]); s = skewness(x);",
    );
    assert_eq!(matrix(&e, "a").cols(), 200000);
    assert_eq!(matrix(&e, "p").cols(), 3);
    assert!(started.elapsed().as_secs() < 30);
    let err = error("x = 1:100000; y = smoothdata(x, 'rloess', 100000);");
    assert!(err.contains("too large"), "{err}");
}

// ------------------------------------------------ fix round 1 regressions

#[test]
fn rescale_with_nan_input_bounds_gives_nan_without_panicking() {
    // Both bounds NaN once reached f64::clamp(NaN, NaN), which panics.
    let y = values(
        r#"y = rescale([1 2], "InputMin", NaN, "InputMax", NaN);"#,
        "y",
    );
    assert!(y.len() == 2 && y.iter().all(|v| v.is_nan()), "{y:?}");
    // One NaN bound, and NaN bounds for one column only.
    let y = values("y = rescale([1 2], 'InputMin', NaN);", "y");
    assert!(y.iter().all(|v| v.is_nan()), "{y:?}");
    let y = values(
        "y = rescale([1 2; 3 4], 'InputMin', [NaN 0], 'InputMax', [NaN 8]);",
        "y",
    );
    assert!(y[0].is_nan() && y[1].is_nan(), "{y:?}");
    assert_eq!(y[2..], [0.25, 0.5]);
    replay("rescale(");
}

/// Whether every element of the logical row `c` set by `source` is true.
fn holds(source: &str) -> bool {
    values(&format!("{source} t = double(all(c));"), "t") == [1.]
}

#[test]
fn every_output_count_follows_one_class_rule() {
    // bounds keeps the class like min and max, with one output or two.
    for class in ["int32", "uint8", "single", "int64"] {
        let code = format!(
            "x = {class}([3 1 2]); [s, l] = bounds(x); s1 = bounds(x); c = [isa(s, '{class}') isa(l, '{class}') isa(s1, '{class}') s == 1 l == 3 s1 == 1];"
        );
        assert!(holds(&code), "{code}");
    }
    let code = "x = int32([4 1; 2 8]); [s, l] = bounds(x, 2); c = [isa(s, 'int32') isequal(s, int32([1; 2])) isequal(l, int32([4; 8]))];";
    assert!(holds(code), "{code}");
    // Integer values beyond 2^53 are selected exactly.
    let code = "x = [intmax('int64') intmin('int64') int64(0)]; [s, l] = bounds(x); c = [isequal(s, intmin('int64')) isequal(l, intmax('int64'))];";
    assert!(holds(code), "{code}");
    // Single NaN handling as for double.
    let code = "[s, l] = bounds(single([3 NaN 1]), 'includenan'); [a, b] = bounds(single([3 NaN 1])); c = [isa(s, 'single') isnan(s) isnan(l) a == 1 b == 3];";
    assert!(holds(code), "{code}");
    // zscore: single in, single out for every output; 1 and 3 outputs agree.
    let code = "x = single([1 2 4]); z1 = zscore(x); [z, mu, sigma] = zscore(x); c = [isa(z1, 'single') isa(z, 'single') isa(mu, 'single') isa(sigma, 'single') isequal(z, z1) abs(double(mu) - 7/3) < 1e-6];";
    assert!(holds(code), "{code}");
    // smoothdata: single stays single, the window output is double; integer
    // input gives double.
    let code = "[b, w] = smoothdata(single([1 5 2 8 3]), 'movmean', 3); b1 = smoothdata(single([1 5 2 8 3]), 'movmean', 3); d = smoothdata(int16([1 5 2 8 3]), 'movmean', 3); [d2, w2] = smoothdata(int16([1 5 2 8 3]), 'movmean', 3); c = [isa(b, 'single') isa(b1, 'single') isa(w, 'double') isa(d, 'double') isa(d2, 'double') isequal(d, d2) d(1) == 3];";
    assert!(holds(code), "{code}");
    // The other single/double functions and rescale.
    let code = "x = single([1 2 3 10]); c = [isa(prctile(x, 50), 'single') isa(quantile(x, 0.5), 'single') isa(skewness(x), 'single') isa(kurtosis(x), 'single') isa(mad(x), 'single') isa(normalize(x), 'single') isa(rescale(x), 'single') isa(rescale(int8([1 2 3])), 'double') isequal(rescale(uint8([0 5 10])), [0 0.5 1])];";
    assert!(holds(code), "{code}");
    // Integer input to the single/double-only functions is rejected the same
    // way for every output count.
    for code in [
        "zscore(int32([1 2 3]))",
        "[z, mu, sigma] = zscore(int32([1 2 3]))",
        "[z, mu] = zscore(uint8([1 2 3]))",
        "prctile(int32([1 2 3]), 50)",
        "quantile(int8([1 2 3]), 0.5)",
        "skewness(int16([1 2 3]))",
        "kurtosis(int32([1 2 3 4]))",
        "mad(uint16([1 2 3]))",
        "normalize(int32([1 2 3]))",
    ] {
        let err = error(code);
        assert!(
            err.contains("requires single or double input"),
            "{code}: {err}"
        );
        assert!(!err.contains("min requires"), "{code}: {err}");
    }
    // Typed option values are plain numbers.
    assert_eq!(values("y = prctile([6 3 2 10 1], int32(50));", "y"), [3.]);
}

#[test]
fn robust_includenan_keeps_the_weight_of_samples_with_a_nan_fit() {
    // Samples next to a NaN have a NaN fit under 'includenan'; they used to get
    // robustness weight 0 (NaN residual), which changed the fits of the
    // NaN-free windows around them. Expected values: tool/refs/stats/gaps.py.
    replay("smoothdata([1.0 3.0 2.0 4.0 3.0 NaN");
    replay("'includenan');");
}

/// Smallest k with removed(k) >= factor, straight from the definition in the
/// help page (naive DFT, every k): an independent check of the O(n) scan.
fn window_by_definition(x: &[f64], factor: f64) -> usize {
    let n = x.len();
    let present: Vec<f64> = x.iter().copied().filter(|v| !v.is_nan()).collect();
    let m = present.iter().sum::<f64>() / present.len() as f64;
    let y: Vec<f64> = x
        .iter()
        .map(|v| if v.is_nan() { 0. } else { v - m })
        .collect();
    let pi = std::f64::consts::PI;
    let power: Vec<f64> = (0..n)
        .map(|j| {
            let (mut re, mut im) = (0., 0.);
            for (t, v) in y.iter().enumerate() {
                let a = -2. * pi * (j * t % n) as f64 / n as f64;
                re += v * a.cos();
                im += v * a.sin();
            }
            re * re + im * im
        })
        .collect();
    let total: f64 = power[1..].iter().sum();
    if n <= 1 || total.is_nan() || total <= 0. {
        return 1;
    }
    for k in 1..=n {
        let kf = k as f64;
        let mut removed = 0.;
        for (j, p) in power.iter().enumerate().skip(1) {
            let f = j as f64 / n as f64;
            let h = (pi * f * kf).sin() / (kf * (pi * f).sin());
            removed += (1. - h * h) * p;
        }
        if removed / total >= factor {
            return k;
        }
    }
    n
}

#[test]
fn default_window_is_the_smallest_qualifying_window() {
    // Alternating data: every even k removes all the energy, odd k only
    // 1 - 1/k^2. Bisection on [1, 6] found 4; the smallest window is 2.
    let e = run(r#"[y, w] = smoothdata([1 -1 1 -1 1 -1], "SmoothingFactor", 0.9);"#);
    assert_eq!(matrix(&e, "w").data(), [2.]);
    assert_eq!(matrix(&e, "y").data(), [1., 0., 0., 0., 0., 0.]);
    // The O(n) scan against the definition, on varied data and factors.
    let data = [
        "[1 -1 1 -1 1 -1 1]",
        "[1 -1 1 -1 1 -1]",
        "[0 0 1 0 0 0 0 0 1 0 0]",
        "sin((1:40) / 3) + mod((1:40) * 7, 5) / 4",
        "cos((1:33) * 2.5) + (1:33) / 20",
        "[3 NaN 1 4 1 5 NaN 9 2 6 5 3 5]",
        "mod((1:64) .^ 2, 17)",
    ];
    for source in data {
        let x = values(&format!("x = {source};"), "x");
        for factor in [0.05, 0.25, 0.5, 0.8, 0.9, 0.97] {
            let code = format!("[y, w] = smoothdata({source}, 'SmoothingFactor', {factor});");
            let w = values(&code, "w")[0];
            assert_eq!(w, window_by_definition(&x, factor) as f64, "{code}");
        }
    }
    replay("'SmoothingFactor'");
    // Large input: the scan is linear in n.
    let started = std::time::Instant::now();
    let e = run("[y, w] = smoothdata(sin((1:400000) / 3000) + mod(1:400000, 3) / 50);");
    let w = matrix(&e, "w").data()[0];
    assert!(w > 1. && w < 400000., "{w}");
    assert!(started.elapsed().as_secs() < 20);
}

// ------------------------------------------------------------------- rms

#[test]
fn rms_matches_numpy_along_dims_vecdim_all_and_nanflags() {
    // tests/fixtures/stats/rms.json from tool/refs/stats/rms.py (NumPy
    // sqrt(mean(abs(x)**2)), nanmean for 'omitnan').
    let fixture: Json = serde_json::from_str(include_str!("fixtures/stats/rms.json")).unwrap();
    let cases = fixture["cases"].as_array().unwrap();
    assert!(cases.len() >= 20);
    for case in cases {
        let code = case["code"].as_str().unwrap();
        let dims: Vec<usize> = case["dims"]
            .as_array()
            .unwrap()
            .iter()
            .map(|d| d.as_u64().unwrap() as usize)
            .collect();
        let expected: Vec<f64> = case["values"]
            .as_array()
            .unwrap()
            .iter()
            .map(number)
            .collect();
        check(&run(code), "y", dims[0], dims[1], &expected, false, code);
    }
    // A sine of amplitude a has rms a/sqrt(2) over whole periods.
    let y = values("y = rms(3*sin(2*pi*(0:99)/20));", "y")[0];
    assert!((y - 3. / 2_f64.sqrt()).abs() < 1e-12, "{y}");
    // Single data gives a single result; integer data is an error.
    let e = run(
        "c = double(strcmp(class(rms(single([3 4]))), 'single')); v = double(rms(single([3 4])));",
    );
    assert_eq!(matrix(&e, "c").data(), [1.]);
    assert_eq!(matrix(&e, "v").data(), [f64::from(12.5_f32.sqrt())]);
    for (code, message) in [
        ("rms(int8([1 2]))", "single or double"),
        ("rms([1 2], 'native')", "unknown option 'native'"),
        ("rms([1 2], 1, 'omitnan', 2)", "Use rms(X)"),
        ("rms({1, 2})", "numeric, logical or char"),
        ("rms([1 2], 0)", "Dimension argument"),
    ] {
        let err = error(code);
        assert!(err.contains(message), "{code}: {err}");
    }
    // A program file in MATLAB: exist gives 2; registered for help and completion.
    assert!(matrust::session::FUNCTIONS.contains(&"rms"));
    assert_eq!(values("t = exist('rms');", "t"), [2.]);
}
