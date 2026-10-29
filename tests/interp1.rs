//! Independent analytic contract tests; no fabricated MATLAB fixture.
use matrust::{
    Array, Engine, Matrix,
    array::{InterpolationMethod, interpolate_vectors},
};
use std::{
    collections::BTreeMap,
    sync::atomic::{AtomicBool, Ordering},
};
fn row(data: &[f64]) -> Matrix {
    Matrix::from_column_slice(1, data.len(), data).unwrap()
}
fn run(x: &Matrix, v: &Matrix, q: &Matrix, method: InterpolationMethod, fill: f64) -> Matrix {
    interpolate_vectors(x, v, q, method, fill, &AtomicBool::new(false)).unwrap()
}

#[test]
fn affine_nonuniform_unsorted_pairs_and_query_shapes() {
    for (x, v) in [
        ([4., 0., 1.], [9., 1., 3.]),
        ([0., 1., 4.], [1., 3., 9.]),
        ([4., 1., 0.], [9., 3., 1.]),
    ] {
        for column_x in [false, true] {
            let x = Matrix::from_column_slice(
                if column_x { 3 } else { 1 },
                if column_x { 1 } else { 3 },
                &x,
            )
            .unwrap();
            let v = Matrix::from_column_slice(
                if column_x { 1 } else { 3 },
                if column_x { 3 } else { 1 },
                &v,
            )
            .unwrap();
            let q = Matrix::from_column_slice(2, 3, &[0., 0.5, 1., 2., 3., 4.]).unwrap();
            let output = run(&x, &v, &q, InterpolationMethod::Linear, f64::NAN);
            assert_eq!((output.rows(), output.cols()), (2, 3));
            assert_eq!(output.data(), &[1., 2., 3., 5., 7., 9.]);
            for (rows, cols) in [(0, 0), (1, 0), (0, 1), (0, 3), (3, 0)] {
                let q = Matrix::from_column_vec(rows, cols, vec![]).unwrap();
                let output = run(&x, &v, &q, InterpolationMethod::Nearest, 7.);
                assert_eq!((output.rows(), output.cols()), (rows, cols));
            }
        }
    }
}

#[test]
fn stored_knots_fill_bits_nan_queries_and_nearest_ties() {
    let fill = f64::from_bits(0x7ff800000000005a);
    let x = row(&[4., 0., 1.]);
    let v = row(&[40., -0., 10.]);
    let q = row(&[
        -1.,
        0.,
        0.5,
        1.,
        2.5,
        4.,
        5.,
        f64::NEG_INFINITY,
        f64::INFINITY,
        f64::NAN,
    ]);
    for method in [InterpolationMethod::Linear, InterpolationMethod::Nearest] {
        let out = run(&x, &v, &q, method, fill);
        for i in [0, 6, 7, 8] {
            assert_eq!(out.data()[i].to_bits(), fill.to_bits());
        }
        assert_eq!(out.data()[1].to_bits(), (-0.0f64).to_bits());
        assert_eq!(out.data()[3], 10.);
        assert_eq!(out.data()[5], 40.);
        assert!(out.data()[9].is_nan());
    }
    let nearest = run(&x, &v, &q, InterpolationMethod::Nearest, -7.);
    assert_eq!(nearest.data()[2], 10.);
    assert_eq!(nearest.data()[4], 40.);
    assert!(nearest.data()[9].is_nan());
}

#[test]
fn finite_extremes_subnormal_spans_and_exact_adjacent_knots() {
    let max = f64::MAX;
    let tiny = f64::from_bits(1);
    let x = row(&[-max, max]);
    let q = row(&[-max, -max / 2., 0., max / 2., max]);
    let linear = run(&x, &x, &q, InterpolationMethod::Linear, 0.);
    assert_eq!(linear.data()[0].to_bits(), (-max).to_bits());
    assert_eq!(linear.data()[4].to_bits(), max.to_bits());
    assert_eq!(linear.data()[2], 0.);
    for (got, want) in linear.data().iter().zip(q.data()) {
        assert!(got.is_finite());
        assert!((got / max - want / max).abs() <= 4. * f64::EPSILON);
    }
    let nearest = run(
        &x,
        &row(&[1., 2.]),
        &row(&[-tiny, 0., tiny]),
        InterpolationMethod::Nearest,
        0.,
    );
    assert_eq!(nearest.data(), &[1., 2., 2.]);
    assert_eq!(
        run(
            &row(&[0., 2. * tiny]),
            &row(&[2., 4.]),
            &row(&[tiny]),
            InterpolationMethod::Linear,
            0.
        )
        .data(),
        &[3.]
    );
    let next = f64::from_bits(1.0f64.to_bits() + 1);
    let endpoints = run(
        &row(&[1., next]),
        &row(&[-0., max]),
        &row(&[1., next]),
        InterpolationMethod::Linear,
        0.,
    );
    assert_eq!(endpoints.data()[0].to_bits(), (-0.0f64).to_bits());
    assert_eq!(endpoints.data()[1], max);
    for values in [[max, max], [max / 2., max], [-max, -max / 2.]] {
        let result = run(
            &row(&[0., 1.]),
            &row(&values),
            &row(&[0.25, 0.5, 0.75]),
            InterpolationMethod::Linear,
            0.,
        );
        assert!(
            result
                .data()
                .iter()
                .all(|&v| v.is_finite() && v >= values[0] && v <= values[1])
        );
    }
}

#[test]
fn public_options_aliases_fill_and_invalid_forms_preserve_destinations() {
    let mut e = Engine::default();
    e.execute("x=[4 0 1];v=[9 1 3];alias=v;q=[0 0.5;2 4];a=interp1(x,v,q);method=char('nearest');b=interp1(x,v,q,method,-7);c=interp1(x,v,[-1 5],'linear',-7);args={x,v,q,'linear'};d=interp1(args{:});").unwrap();
    assert_eq!(e.variables["a"].data(), e.variables["d"].data());
    assert_eq!(e.variables["a"].data(), &[1., 5., 2., 9.]);
    assert_eq!(e.variables["c"].data(), &[-7., -7.]);
    assert!(
        e.array("v")
            .unwrap()
            .same_snapshot(&e.array("alias").unwrap())
    );
    e.execute("out=29;").unwrap();
    let before = e.workspace();
    for call in [
        "interp1(x,v,q,\"linear\")",
        "interp1(x,v,q,'linear',[1 2])",
        "interp1(x,v,q,'linear',true)",
        "interp1(logical(x),v,q)",
        "interp1(x,v,char('q'))",
        "interp1({1},v,q)",
        "interp1([0 0],[1 2],[])",
        "interp1([0 -0],[1 2],[])",
        "interp1([0 Inf],[1 2],[])",
        "interp1(0,1,[])",
        "interp1([],[],[])",
        "interp1([0 1],[1 2 3],q)",
        "interp1([0 1;2 3],[1 2;3 4],q)",
    ] {
        assert!(e.execute(&format!("out={call};")).is_err(), "{call}");
        for (name, value) in &before {
            assert!(
                e.array(name).unwrap().same_snapshot(value),
                "{name}: {call}"
            );
        }
    }
    let cancel = e.cancellation();
    cancel.store(true, Ordering::Relaxed);
    assert!(
        e.execute("out=interp1(x,v,q);")
            .unwrap_err()
            .contains("cancelled")
    );
    assert!(e.array("out").unwrap().same_snapshot(&before["out"]));
    cancel.store(false, Ordering::Relaxed);
    e.execute("out=interp1(x,v,q);").unwrap();
}

#[test]
fn function_and_index_precedence_and_existing_argument_effects() {
    let mut local = Engine::default();
    local
        .execute("[a,b]=interp1(7);function [a,b]=interp1(x);a=x;b=8;end")
        .unwrap();
    assert_eq!(local.variables["a"].data(), &[7.]);
    assert_eq!(local.variables["b"].data(), &[8.]);
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(
        dir.path().join("interp1.m"),
        "function y=interp1(x)\ny=x+1;\nend\n",
    )
    .unwrap();
    let mut file = Engine::default();
    file.set_function_directory(dir.path()).unwrap();
    file.execute("a=interp1(9);").unwrap();
    assert_eq!(file.variables["a"].data(), &[10.]);
    let mut variable = Engine::default();
    variable.execute("interp1=[11 13];a=interp1(2);").unwrap();
    assert_eq!(variable.variables["a"].data(), &[13.]);
    let mut e = Engine::default();
    assert!(e.execute("[a,b]=interp1(figure(9));").is_err());
    assert!(e.figure_ids().is_empty());
    assert!(e.execute("a=interp1(figure(10));").is_err());
    assert_eq!(e.figure_ids(), vec![10]);
    assert!(matrust::session::FUNCTIONS.contains(&"interp1"));
}

#[test]
fn actual_one_eight_contexts_unsorted_boundary_and_shared_input() {
    let n = 524_289;
    let x = Matrix::from_column_vec(1, n, (0..n).map(|i| ((i * 2) % n) as f64).collect()).unwrap();
    let v = x.clone();
    let q = row(&[0., 0.5, 4096.25, 524287.5, 524288.]);
    let inputs = BTreeMap::from([
        ("x".into(), Array::Numeric(x.clone())),
        ("v".into(), Array::Numeric(v.clone())),
        ("q".into(), Array::Numeric(q.clone())),
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
                e.execute("out=interp1(x,v,q);near=interp1(x,v,q,'nearest');")
                    .unwrap();
                assert_eq!(e.variables["out"].data(), q.data());
                assert_eq!(
                    e.variables["near"].data(),
                    &[0., 1., 4096., 524288., 524288.]
                );
                for (name, value) in &inputs {
                    assert!(e.array(name).unwrap().same_snapshot(value));
                }
            });
    }
}
