//! Analytic subset tests; no fabricated MATLAB reference observations.
use matrust::{Array, Engine, Matrix, array::evaluate_polynomial};
use std::{
    collections::BTreeMap,
    sync::atomic::{AtomicBool, Ordering},
};

fn row(values: &[f64]) -> Matrix {
    Matrix::from_column_slice(1, values.len(), values).unwrap()
}
fn evaluate(p: &Matrix, x: &Matrix) -> Matrix {
    evaluate_polynomial(p, x, &AtomicBool::new(false)).unwrap()
}

#[test]
fn independent_polynomial_values_coefficient_orientations_and_query_shapes() {
    for column in [false, true] {
        let p = Matrix::from_column_slice(
            if column { 3 } else { 1 },
            if column { 1 } else { 3 },
            &[3., 2., 1.],
        )
        .unwrap();
        let q = Matrix::from_column_slice(2, 3, &[0., 1., -1., 5., 7., 9.]).unwrap();
        let aliases = (p.clone(), q.clone());
        let result = evaluate(&p, &q);
        assert_eq!((result.rows(), result.cols()), (2, 3));
        assert_eq!(result.data(), &[1., 6., 2., 86., 162., 262.]);
        assert_eq!(p.data().as_ptr(), aliases.0.data().as_ptr());
        assert_eq!(q.data().as_ptr(), aliases.1.data().as_ptr());
        for (rows, cols) in [(0, 0), (1, 0), (0, 1), (0, 7), (7, 0)] {
            let empty = Matrix::from_column_vec(rows, cols, vec![]).unwrap();
            let result = evaluate(&p, &empty);
            assert_eq!(
                (result.rows(), result.cols(), result.len()),
                (rows, cols, 0)
            );
        }
    }
    assert_eq!(
        evaluate(&row(&[0., 0., 2., 3.]), &row(&[-2., 0., 2.])).data(),
        &[-1., 3., 7.]
    );
    assert_eq!(
        evaluate(&row(&[1., -2., 1.]), &row(&[0.5, 1., 1.5])).data(),
        &[0.25, 0., 0.25]
    );
}

#[test]
fn constant_bits_subnormal_inputs_overflow_and_declared_limits() {
    let q = row(&[-f64::MAX, -0., 0., f64::from_bits(1), f64::MAX]);
    for constant in [-0.0f64, f64::from_bits(1), f64::MAX] {
        assert!(
            evaluate(&Matrix::scalar(constant), &q)
                .data()
                .iter()
                .all(|x| x.to_bits() == constant.to_bits())
        );
    }
    let tiny = f64::from_bits(1);
    assert_eq!(
        evaluate(&row(&[1., 0.]), &row(&[tiny])).data()[0].to_bits(),
        tiny.to_bits()
    );
    assert_eq!(
        evaluate(&row(&[f64::MAX, f64::MAX]), &row(&[2.])).data(),
        &[f64::INFINITY]
    );
    let allowed = Matrix::from_column_vec(1, 4096, vec![1.; 4096]).unwrap();
    assert_eq!(evaluate(&allowed, &Matrix::scalar(0.)).data(), &[1.]);
    let excess = Matrix::from_column_vec(4097, 1, vec![1.; 4097]).unwrap();
    let empty = row(&[]);
    assert!(
        evaluate_polynomial(&excess, &empty, &AtomicBool::new(false))
            .unwrap_err()
            .contains("4096")
    );
    let many = Matrix::from_column_vec(1, 16389, vec![0.; 16389]).unwrap();
    assert!(
        evaluate_polynomial(&allowed, &many, &AtomicBool::new(false))
            .unwrap_err()
            .contains("work limit")
    );
    for bad in [
        row(&[]),
        Matrix::from_column_slice(2, 2, &[1.; 4]).unwrap(),
        row(&[1., f64::NAN]),
        row(&[f64::INFINITY]),
    ] {
        assert!(evaluate_polynomial(&bad, &empty, &AtomicBool::new(false)).is_err());
    }
    for bad in [f64::NAN, f64::INFINITY, f64::NEG_INFINITY] {
        assert!(
            evaluate_polynomial(&Matrix::scalar(1.), &row(&[bad]), &AtomicBool::new(false))
                .is_err()
        );
    }
}

#[test]
fn typed_engine_rejection_csl_aliases_and_atomic_destination() {
    let mut e = Engine::default();
    e.execute("p=[3 2 1];p_alias=p;x=[0 5;1 7];x_alias=x;args={p,x};y=polyval(args{:});out=29;")
        .unwrap();
    assert_eq!(e.variables["y"].data(), &[1., 6., 86., 162.]);
    let before = e.workspace();
    for call in [
        "polyval(p)",
        "polyval(p,x,[])",
        "polyval(p,x,[],[0 1])",
        "polyval([],x)",
        "polyval([1 2;3 4],x)",
        "polyval([1 Inf],x)",
        "polyval(p,[NaN])",
        "polyval(logical(p),x)",
        "polyval(p,logical(x))",
        "polyval('abc',x)",
        "polyval(p,char('a'))",
        "polyval({1},x)",
        "polyval(p,{1})",
        "polyval(\"abc\",x)",
    ] {
        assert!(e.execute(&format!("out={call};")).is_err(), "{call}");
        for (name, value) in &before {
            assert!(
                e.array(name).unwrap().same_snapshot(value),
                "{name}: {call}"
            );
        }
    }
    assert!(
        e.array("p")
            .unwrap()
            .same_snapshot(&e.array("p_alias").unwrap())
    );
    assert!(
        e.array("x")
            .unwrap()
            .same_snapshot(&e.array("x_alias").unwrap())
    );
    let cancel = e.cancellation();
    cancel.store(true, Ordering::Relaxed);
    assert!(
        e.execute("out=polyval(p,x);")
            .unwrap_err()
            .contains("cancelled")
    );
    assert!(e.array("out").unwrap().same_snapshot(&before["out"]));
    cancel.store(false, Ordering::Relaxed);
    e.execute("out=polyval(p,x);").unwrap();
    assert_eq!(e.variables["out"].data(), e.variables["y"].data());
}

#[test]
fn user_function_index_precedence_and_multiple_output_preflight() {
    let mut local = Engine::default();
    local
        .execute("[a,b]=polyval(7);function [a,b]=polyval(x);a=x;b=8;end")
        .unwrap();
    assert_eq!(local.variables["a"].data(), &[7.]);
    assert_eq!(local.variables["b"].data(), &[8.]);
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(
        dir.path().join("polyval.m"),
        "function y=polyval(x)\ny=x+1;\nend\n",
    )
    .unwrap();
    let mut file = Engine::default();
    file.set_function_directory(dir.path()).unwrap();
    file.execute("a=polyval(9);").unwrap();
    assert_eq!(file.variables["a"].data(), &[10.]);
    let mut variable = Engine::default();
    variable.execute("polyval=[11 13];a=polyval(2);").unwrap();
    assert_eq!(variable.variables["a"].data(), &[13.]);
    let mut e = Engine::default();
    assert!(e.execute("[a,b]=polyval(figure(9));").is_err());
    assert!(e.figure_ids().is_empty());
    assert!(e.execute("a=polyval(figure(10));").is_err());
    assert_eq!(e.figure_ids(), vec![10]);
    assert!(matrust::session::FUNCTIONS.contains(&"polyval"));
}

#[test]
fn public_one_eight_contexts_large_query_analytic_result_and_shared_inputs() {
    let n = 65537;
    let p = row(&[2., -3., 0., 5.]);
    let x = Matrix::from_column_vec(n, 1, (0..n).map(|i| (i % 5) as f64 - 2.).collect()).unwrap();
    let inputs = BTreeMap::from([
        ("p".into(), Array::Numeric(p.clone())),
        ("x".into(), Array::Numeric(x.clone())),
    ]);
    for workers in [1, 8] {
        rayon::ThreadPoolBuilder::new()
            .num_threads(workers)
            .build()
            .unwrap()
            .install(|| {
                assert_eq!(rayon::current_num_threads(), workers);
                let mut e = Engine::default();
                e.merge_workspace(inputs.clone()).unwrap();
                e.execute("out=polyval(p,x);").unwrap();
                let result = &e.variables["out"];
                assert_eq!((result.rows(), result.cols()), (n, 1));
                for (&query, &got) in x.data().iter().zip(result.data()) {
                    let expected = 2. * query * query * query - 3. * query * query + 5.;
                    assert_eq!(got.to_bits(), expected.to_bits());
                }
                for (name, value) in &inputs {
                    assert!(e.array(name).unwrap().same_snapshot(value));
                }
            });
    }
}

#[test]
fn automatic_dispatch_boundary_and_intermediate_lengths_preserve_all_bits_and_shapes() {
    let serial = rayon::ThreadPoolBuilder::new()
        .num_threads(1)
        .build()
        .unwrap();
    let parallel = rayon::ThreadPoolBuilder::new()
        .num_threads(8)
        .build()
        .unwrap();
    for (count, n, rows) in [
        (4, 262143, 1),
        (4, 262144, 2),
        (4, 262145, 1),
        (5, 262144, 2),
        (8, 262144, 2),
        (12, 262144, 2),
        (17, 262145, 1),
        (17, 1048576, 2),
    ] {
        let mut coefficients = vec![0.; count];
        coefficients[0] = 1.;
        if count == 4 {
            coefficients.copy_from_slice(&[2., -3., 0., 5.]);
        }
        let p = Matrix::from_column_vec(count, 1, coefficients).unwrap();
        let x = Matrix::from_column_vec(
            rows,
            n / rows,
            (0..n).map(|i| (i % 9) as f64 * 0.5 - 2.).collect(),
        )
        .unwrap();
        let aliases = (p.clone(), x.clone());
        let baseline = serial.install(|| evaluate(&p, &x));
        let got = parallel.install(|| {
            let mut engine = Engine::default();
            engine
                .merge_workspace(BTreeMap::from([
                    ("p".into(), Array::Numeric(p.clone())),
                    ("x".into(), Array::Numeric(x.clone())),
                ]))
                .unwrap();
            engine.execute("result=polyval(p,x);").unwrap();
            assert!(
                engine
                    .array("p")
                    .unwrap()
                    .same_snapshot(&Array::Numeric(p.clone()))
            );
            assert!(
                engine
                    .array("x")
                    .unwrap()
                    .same_snapshot(&Array::Numeric(x.clone()))
            );
            engine.variables["result"].clone()
        });
        assert_eq!((got.rows(), got.cols()), (x.rows(), x.cols()));
        for ((&actual, &old), &query) in got.data().iter().zip(baseline.data()).zip(x.data()) {
            assert_eq!(actual.to_bits(), old.to_bits(), "p={count} n={n}");
            // All operands/results here are exactly representable dyadics.
            let analytic = if count == 4 {
                2. * query * query * query - 3. * query * query + 5.
            } else {
                query.powi((count - 1) as i32)
            };
            // Zero sign can differ between power and Horner's trailing +0 terms.
            if analytic == 0. {
                assert_eq!(actual, 0.);
            } else {
                assert_eq!(actual.to_bits(), analytic.to_bits());
            }
        }
        assert_eq!(p.data().as_ptr(), aliases.0.data().as_ptr());
        assert_eq!(x.data().as_ptr(), aliases.1.data().as_ptr());
    }
}

#[test]
fn automatic_large_route_retains_ieee_bits_validation_and_pre_cancel() {
    let serial = rayon::ThreadPoolBuilder::new()
        .num_threads(1)
        .build()
        .unwrap();
    let parallel = rayon::ThreadPoolBuilder::new()
        .num_threads(8)
        .build()
        .unwrap();
    let pattern = [
        -f64::MAX,
        -2.,
        -1.,
        -0.,
        0.,
        f64::from_bits(1),
        0.5,
        1.,
        2.,
        f64::MAX,
    ];
    let x = Matrix::from_column_vec(
        262144,
        1,
        (0..262144).map(|i| pattern[i % pattern.len()]).collect(),
    )
    .unwrap();
    for p in [
        row(&[f64::MAX, -f64::MAX, f64::MAX, f64::MAX]),
        row(&[1., -0., f64::from_bits(1), -0.]),
    ] {
        let aliases = (p.clone(), x.clone());
        let expected = serial.install(|| evaluate(&p, &x));
        let actual = parallel.install(|| evaluate(&p, &x));
        assert_eq!(actual.len(), expected.len());
        assert!(
            actual
                .data()
                .iter()
                .zip(expected.data())
                .all(|(a, b)| a.to_bits() == b.to_bits())
        );
        let cancel = AtomicBool::new(true);
        assert_eq!(
            parallel
                .install(|| evaluate_polynomial(&p, &x, &cancel))
                .unwrap_err(),
            "Calculation cancelled"
        );
        assert_eq!(p.data().as_ptr(), aliases.0.data().as_ptr());
        assert_eq!(x.data().as_ptr(), aliases.1.data().as_ptr());
    }
    let mut invalid = vec![1.; 262144];
    invalid[262143] = f64::NAN;
    let invalid = Matrix::from_column_vec(1, 262144, invalid).unwrap();
    assert!(
        parallel
            .install(|| evaluate_polynomial(&row(&[1.; 4]), &invalid, &AtomicBool::new(false)))
            .unwrap_err()
            .contains("finite")
    );
    // An otherwise eligible shape still rejects excess work before allocation.
    let too_many = Matrix::from_column_vec(1, 4194305, vec![0.; 4194305]).unwrap();
    assert!(
        parallel
            .install(|| evaluate_polynomial(&row(&[1.; 17]), &too_many, &AtomicBool::new(false)))
            .unwrap_err()
            .contains("work limit")
    );
}
