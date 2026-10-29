//! Exact scaled norms and an independent direct-sum correlation reference.
use matrust::{Engine, Matrix};

fn relative(actual: f64, expected: f64) {
    assert!(actual.is_finite(), "{actual} versus {expected}");
    assert!(
        (actual / expected - 1.).abs() <= 1e-15,
        "{actual:.17e} versus {expected:.17e}"
    );
}

#[test]
fn norms_avoid_intermediate_overflow_and_underflow() {
    let mut engine = Engine::default();
    for scale in [1e200, 1e-200] {
        for (expression, expected) in [
            (format!("norm({scale:e},3)"), scale),
            (format!("norm([{scale:e} {scale:e}])"), scale * 2f64.sqrt()),
            (
                format!("norm([{scale:e};-{scale:e}],2)"),
                scale * 2f64.sqrt(),
            ),
            (
                format!("norm([{scale:e} {scale:e}],3)"),
                scale * 2f64.cbrt(),
            ),
            (
                format!("norm([{scale:e} {scale:e}],4)"),
                scale * 2f64.sqrt().sqrt(),
            ),
            (format!("norm([{scale:e} {scale:e}],-1)"), scale / 2.),
            (
                format!("norm([{scale:e} {scale:e};{scale:e} {scale:e}],'fro')"),
                scale * 2.,
            ),
            (
                format!("norm([{scale:e} {scale:e}],'fro')"),
                scale * 2f64.sqrt(),
            ),
        ] {
            engine.execute(&format!("n={expression};")).unwrap();
            relative(engine.variables["n"].data()[0], expected);
        }
    }
    engine.execute("a=norm([0 0],3);b=norm([0 0],'fro');c=norm([NaN 1],3);d=norm([Inf 1],3);e=norm([3 4],0);f=norm([],3);g=norm([0 2],-3);").unwrap();
    for name in ["a", "b", "f", "g"] {
        assert_eq!(engine.variables[name].data(), &[0.]);
    }
    assert!(engine.variables["c"].data()[0].is_nan());
    for name in ["d", "e"] {
        assert_eq!(engine.variables[name].data(), &[f64::INFINITY]);
    }
}

fn vector(x: &[f64], column: bool) -> Matrix {
    Matrix::from_column_vec(
        if column { x.len() } else { 1 },
        if column { 1 } else { x.len() },
        x.to_vec(),
    )
    .unwrap()
}

// Deliberately different indexing from the implementation: scan every pair
// and bucket by the difference in indices, including implicit zero padding.
fn reference(x: &[f64], y: &[f64], maxlag: usize, scale: &str) -> Vec<f64> {
    let n = x.len().max(y.len());
    let mut sums = vec![0.; 2 * maxlag + 1];
    for (i, a) in x.iter().enumerate() {
        for (j, b) in y.iter().enumerate() {
            let lag = i as isize - j as isize;
            if lag.unsigned_abs() <= maxlag {
                sums[(lag + maxlag as isize) as usize] += a * b;
            }
        }
    }
    let energy =
        (x.iter().map(|v| v * v).sum::<f64>() * y.iter().map(|v| v * v).sum::<f64>()).sqrt();
    for (i, sum) in sums.iter_mut().enumerate() {
        let lag = (i as isize - maxlag as isize).unsigned_abs();
        *sum = match scale {
            "biased" => *sum / n as f64,
            "unbiased" if lag < n => *sum / (n - lag) as f64,
            "coeff" => *sum / energy,
            _ => *sum,
        };
    }
    sums
}

fn check(engine: &Engine, x: &[f64], y: &[f64], maxlag: usize, scale: &str, column: bool) {
    let r = &engine.variables["r"];
    let lags = &engine.variables["lags"];
    let len = 2 * maxlag + 1;
    assert_eq!(
        (r.rows(), r.cols()),
        if column { (len, 1) } else { (1, len) }
    );
    assert_eq!((lags.rows(), lags.cols()), (1, len));
    let expected = reference(x, y, maxlag, scale);
    for (i, (&actual, &expected)) in r.data().iter().zip(&expected).enumerate() {
        if expected.is_nan() {
            assert!(actual.is_nan());
        } else {
            assert!(
                (actual - expected).abs() <= 2e-14 * expected.abs().max(1.),
                "lag index {i}: {actual} != {expected}"
            );
        }
        assert_eq!(lags.data()[i], i as f64 - maxlag as f64);
    }
}

#[test]
fn xcorr_basic_forms_match_direct_sum_with_row_and_column_inputs() {
    for n in [3, 4, 7, 8] {
        let x: Vec<_> = (0..n).map(|i| i as f64 - 2.).collect();
        let y: Vec<_> = (0..n).map(|i| (i % 3) as f64 - 1.).collect();
        for column in [false, true] {
            let mut e = Engine::default();
            e.variables.insert("x".into(), vector(&x, column));
            // The orientation of y must not change that of r.
            e.variables.insert("y".into(), vector(&y, !column));
            for (call, right) in [("xcorr(x)", &x), ("xcorr(x,y)", &y)] {
                e.execute(&format!("[r,lags]={call};")).unwrap();
                check(&e, &x, right, n - 1, "none", column);
            }
            for maxlag in [0, 1, n - 1, n, n + 3] {
                for scale in ["none", "biased", "unbiased", "coeff"] {
                    for (args, right) in [("x", &x), ("x,y", &y)] {
                        e.execute(&format!("[r,lags]=xcorr({args},{maxlag},'{scale}');"))
                            .unwrap();
                        check(&e, &x, right, maxlag, scale, column);
                    }
                }
                for (args, right) in [("x", &x), ("x,y", &y)] {
                    e.execute(&format!("[r,lags]=xcorr({args},{maxlag});"))
                        .unwrap();
                    check(&e, &x, right, maxlag, "none", column);
                }
            }
            for scale in ["biased", "unbiased", "coeff", "none"] {
                for (args, right) in [("x", &x), ("x,y", &y)] {
                    e.execute(&format!(
                        "[r,lags]=xcorr({args},'{scale}'); single=xcorr({args},'{scale}');"
                    ))
                    .unwrap();
                    check(&e, &x, right, n - 1, scale, column);
                    assert_eq!(e.variables["single"].data(), e.variables["r"].data());
                }
            }
        }
    }
}

#[test]
fn xcorr_unequal_lengths_are_zero_padded() {
    for (x, y) in [
        (vec![1., -2., 3.], vec![4., 5.]),
        (vec![4., 5.], vec![1., -2., 3.]),
    ] {
        let mut e = Engine::default();
        e.variables.insert("x".into(), vector(&x, false));
        e.variables.insert("y".into(), vector(&y, true));
        e.execute("[r,lags]=xcorr(x,y);").unwrap();
        check(&e, &x, &y, 2, "none", false);
        for maxlag in [0, 1, 2, 5] {
            e.execute(&format!("[r,lags]=xcorr(x,y,{maxlag},'none');"))
                .unwrap();
            check(&e, &x, &y, maxlag, "none", false);
        }
        for scale in ["biased", "unbiased", "coeff"] {
            assert!(
                e.execute(&format!("xcorr(x,y,'{scale}');"))
                    .unwrap_err()
                    .contains("equal input lengths")
            );
        }
    }
}

#[test]
fn xcorr_zero_coeff_scalar_disambiguation_and_lag_sign() {
    let mut e = Engine::default();
    for call in [
        "xcorr([0 0 0],'coeff')",
        "xcorr([1 2 3],[0 0 0],5,'coeff')",
        "xcorr(zeros(1,720),'coeff')",
    ] {
        e.execute(&format!("r={call};")).unwrap();
        assert!(e.variables["r"].data().iter().all(|v| v.is_nan()));
    }
    e.execute("[r,lags]=xcorr([1 2],3); cross=xcorr([1 2],3,2); imp=xcorr([0 1 0],[1 0 0]);")
        .unwrap();
    check(&e, &[1., 2.], &[1., 2.], 3, "none", false);
    assert_eq!(
        e.variables["cross"].data(),
        reference(&[1., 2.], &[3.], 2, "none")
    );
    assert_eq!(e.variables["imp"].data(), &[0., 0., 0., 1., 0.]);
    // FFTs would spread a single NaN to every lag. Retain direct semantics.
    e.execute("x=ones(1,720); x(360)=NaN; r=xcorr(x);").unwrap();
    let r = e.variables["r"].data();
    assert_eq!(r[0], 1.);
    assert!(r[719].is_nan());
    assert_eq!(r[1438], 1.);
}

#[test]
fn xcorr_rejects_unsupported_inputs_and_bounds_outputs() {
    let mut e = Engine::default();
    for (source, message) in [
        ("xcorr([1 2;3 4])", "matrix input is not supported yet"),
        (
            "xcorr([1 2],[1 2;3 4])",
            "matrix input is not supported yet",
        ),
        ("xcorr([1 i])", "complex input is not supported yet"),
        ("xcorr([])", "nonempty"),
        ("xcorr([1 2],-1)", "nonnegative integer"),
        ("xcorr([1 2],0.5)", "nonnegative integer"),
        ("xcorr([1 2],Inf)", "nonnegative integer"),
        ("xcorr([1 2],NaN)", "nonnegative integer"),
        ("xcorr([1 2],1e100)", "nonnegative integer"),
        ("xcorr([1 2],'unknown')", "scale must be"),
        ("xcorr(ones(1,524289))", "transform limit"),
        ("[r,lags,z]=xcorr([1 2])", "at most two outputs"),
    ] {
        let error = e.execute(source).unwrap_err();
        assert!(error.contains(message), "{source}: {error}");
    }
}

#[test]
fn xcorr_coeff_autocorrelation_has_exact_unit_zero_lag() {
    let mut e = Engine::default();
    for source in [
        "x=[0.1 0.7 0.3]; r=xcorr(x,'coeff');",
        "x=sin(1:1024); r=xcorr(x,'coeff');",
    ] {
        e.execute(source).unwrap();
        assert_eq!(e.variables["r"].data()[e.variables["x"].len() - 1], 1.);
    }
    e.execute("r=xcorr([0.1 0.7 0.3],[0.8 -0.2 0.6],'coeff');")
        .unwrap();
    let expected = reference(&[0.1, 0.7, 0.3], &[0.8, -0.2, 0.6], 2, "coeff");
    for (&a, &b) in e.variables["r"].data().iter().zip(&expected) {
        assert!((a - b).abs() < 1e-15);
    }
}

#[test]
fn xcorr_full_lag_long_inputs_complete() {
    let mut e = Engine::default();
    e.execute("r=xcorr(ones(1,12000));").unwrap();
    for (i, &value) in e.variables["r"].data().iter().enumerate() {
        let expected = (12000 - i.abs_diff(11999)) as f64;
        assert!((value - expected).abs() < 1e-10);
    }
    e.execute("x=randn(1,441000); [r,lags]=xcorr(x);").unwrap();
    let x = e.variables["x"].data();
    let r = e.variables["r"].data();
    assert_eq!(r.len(), 881999);
    assert!(r.iter().all(|v| v.is_finite()));
    let energy: f64 = x.iter().map(|v| v * v).sum();
    // Normwise relative error, including lags whose exact sum is nearly zero.
    for lag in [0, 1, 23, 1000, 440998, 440999] {
        let expected: f64 = x[lag..].iter().zip(x).map(|(a, b)| a * b).sum();
        for i in [440999 + lag, 440999 - lag] {
            assert!((r[i] - expected).abs() <= 1e-12 * energy);
        }
    }
    assert_eq!(e.variables["lags"].data()[0], -440999.);
    assert_eq!(*e.variables["lags"].data().last().unwrap(), 440999.);
}

#[test]
fn xcorr_fft_scales_orientations_and_unequal_support_match_direct_sum() {
    let x: Vec<_> = (0..720).map(|i| 1. + (i as f64 * 0.1).sin()).collect();
    let y: Vec<_> = (0..720).map(|i| 2. + (i as f64 * 0.3).cos()).collect();
    let mut e = Engine::default();
    for column in [false, true] {
        e.variables.insert("x".into(), vector(&x, column));
        e.variables.insert("y".into(), vector(&y, !column));
        for scale in ["none", "biased", "unbiased", "coeff"] {
            e.execute(&format!("[r,lags]=xcorr(x,y,730,'{scale}');"))
                .unwrap();
            let expected = reference(&x, &y, 730, scale);
            let r = &e.variables["r"];
            assert_eq!(
                (r.rows(), r.cols()),
                if column { (1461, 1) } else { (1, 1461) }
            );
            for (&a, &b) in r.data().iter().zip(&expected) {
                assert!((a - b).abs() <= 1e-12 * b.abs().max(1.), "{a} vs {b}");
            }
        }
    }
    for (x, y) in [(&x[..], &y[..350]), (&y[..350], &x[..])] {
        e.variables.insert("x".into(), vector(x, false));
        e.variables.insert("y".into(), vector(y, true));
        e.execute("[r,lags]=xcorr(x,y,730);").unwrap();
        let expected = reference(x, y, 730, "none");
        for (&a, &b) in e.variables["r"].data().iter().zip(&expected) {
            assert!((a - b).abs() <= 1e-12 * b.abs().max(1.));
            if b == 0. {
                assert_eq!(a, 0.);
            }
        }
    }
}

#[test]
fn xcorr_parallel_path_matches_single_worker() {
    for source in [
        "x=sin(1:1024);[r,lags]=xcorr(x,700,'unbiased');",
        "x=sin(1:100000);[r,lags]=xcorr(x,8,'unbiased');",
    ] {
        let mut expected: Option<Matrix> = None;
        for workers in [1, 4] {
            let result = rayon::ThreadPoolBuilder::new()
                .num_threads(workers)
                .build()
                .unwrap()
                .install(|| {
                    let mut e = Engine::default();
                    e.execute(source).unwrap();
                    e.variables["r"].clone()
                });
            if let Some(expected) = &expected {
                assert_eq!(result.data(), expected.data());
            }
            expected = Some(result);
        }
    }
}
