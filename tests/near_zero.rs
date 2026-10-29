//! Structural/IEEE and elementary analytic checks; independent accuracy corpus is pending.
use matrust::{
    Array, Engine, Matrix,
    array::{expm1, log1p},
};
use std::{
    collections::BTreeMap,
    sync::atomic::{AtomicBool, Ordering},
};

fn apply(name: &str, input: &Matrix) -> Matrix {
    let cancel = AtomicBool::new(false);
    match name {
        "log1p" => log1p(input, &cancel).unwrap(),
        "expm1" => expm1(input, &cancel).unwrap(),
        _ => unreachable!(),
    }
}

#[test]
fn tiny_values_signed_zeros_and_shapes_preserve_bits_and_shared_input() {
    // For these |x| <= 2^-100, the quadratic correction is below half an ULP
    // of x, including subnormals. Rounded log1p(x) and expm1(x) equal x.
    let tiny = f64::from_bits((1023 - 100) << 52);
    let values = [0., -0., tiny, -tiny, f64::from_bits(1), -f64::from_bits(1)];
    for name in ["log1p", "expm1"] {
        let scalar = apply(name, &Matrix::scalar(-0.));
        assert_eq!((scalar.rows(), scalar.cols()), (1, 1));
        assert_eq!(scalar.data()[0].to_bits(), (-0.0f64).to_bits());
        for (rows, cols) in [(1, 6), (6, 1), (2, 3)] {
            let input = Matrix::from_column_slice(rows, cols, &values).unwrap();
            let alias = input.clone();
            let result = apply(name, &input);
            assert_eq!((result.rows(), result.cols()), (rows, cols));
            for (&got, &expected) in result.data().iter().zip(&values) {
                assert_eq!(got.to_bits(), expected.to_bits(), "{name}");
            }
            assert_eq!(input.data().as_ptr(), alias.data().as_ptr());
            assert_ne!(result.data().as_ptr(), input.data().as_ptr());
        }
        for (rows, cols) in [(0, 0), (1, 0), (0, 1), (0, 3), (4, 0)] {
            let input = Matrix::from_column_vec(rows, cols, vec![]).unwrap();
            let result = apply(name, &input);
            assert_eq!(
                (result.rows(), result.cols(), result.len()),
                (rows, cols, 0)
            );
        }
    }
    assert_eq!((1. + tiny).to_bits(), 1.0f64.to_bits());
}

#[test]
fn ieee_domain_neighbors_and_elementary_bounds_are_explicit() {
    let log_input = Matrix::from_column_slice(
        1,
        5,
        &[
            -1.,
            f64::from_bits((-1.0f64).to_bits() - 1),
            1.,
            f64::INFINITY,
            f64::NAN,
        ],
    )
    .unwrap();
    let output = apply("log1p", &log_input);
    assert_eq!(output.data()[0], f64::NEG_INFINITY);
    assert!(output.data()[1].is_finite() && output.data()[1] < -30.);
    assert!(output.data()[2] > 0.69 && output.data()[2] < 0.70);
    assert_eq!(output.data()[3], f64::INFINITY);
    assert!(output.data()[4].is_nan());
    for bad in [
        f64::from_bits((-1.0f64).to_bits() + 1),
        -2.,
        f64::NEG_INFINITY,
    ] {
        let input = Matrix::from_column_slice(1, 3, &[0., bad, 1.]).unwrap();
        let alias = input.clone();
        assert!(
            log1p(&input, &AtomicBool::new(false))
                .unwrap_err()
                .contains("below -1")
        );
        assert_eq!(input.data().as_ptr(), alias.data().as_ptr());
        assert_eq!(input.data()[1].to_bits(), bad.to_bits());
    }
    let exp_input = Matrix::from_column_slice(
        1,
        7,
        &[
            f64::NEG_INFINITY,
            -f64::MAX,
            -1.,
            1.,
            f64::MAX,
            f64::INFINITY,
            f64::NAN,
        ],
    )
    .unwrap();
    let output = apply("expm1", &exp_input);
    assert_eq!(&output.data()[..2], &[-1., -1.]);
    assert!(output.data()[2] > -0.64 && output.data()[2] < -0.63);
    assert!(output.data()[3] > 1.71 && output.data()[3] < 1.72);
    assert_eq!(&output.data()[4..6], &[f64::INFINITY, f64::INFINITY]);
    assert!(output.data()[6].is_nan());
}

#[test]
fn typed_admission_csl_domain_and_cancellation_preserve_destination_and_aliases() {
    for name in ["log1p", "expm1"] {
        let mut engine = Engine::default();
        engine.execute(&format!("x=[0 0;0 0];alias=x;args={{x}};y={name}(args{{:}});empty_complex=complex([],[]);empty_logical=logical([]);out=29;")).unwrap();
        assert_eq!(engine.variables["y"].data(), &[0.; 4]);
        assert_eq!(
            (engine.variables["y"].rows(), engine.variables["y"].cols()),
            (2, 2)
        );
        let before = engine.workspace();
        for args in [
            "",
            "x,x",
            "x,'extra'",
            "logical(x)",
            "complex(x,0)",
            "empty_complex",
            "empty_logical",
            "'abc'",
            "char([65])",
            "\"abc\"",
            "{x}",
        ] {
            assert!(
                engine.execute(&format!("out={name}({args});")).is_err(),
                "{name}({args})"
            );
            for (key, value) in &before {
                assert!(engine.array(key).unwrap().same_snapshot(value), "{key}");
            }
        }
        if name == "log1p" {
            assert!(engine.execute("out=log1p([0 -2]);").is_err());
            assert!(engine.array("out").unwrap().same_snapshot(&before["out"]));
        }
        let cancel = engine.cancellation();
        cancel.store(true, Ordering::Relaxed);
        assert!(
            engine
                .execute(&format!("out={name}(x);"))
                .unwrap_err()
                .contains("cancelled")
        );
        assert!(engine.array("out").unwrap().same_snapshot(&before["out"]));
        cancel.store(false, Ordering::Relaxed);
        engine.execute(&format!("out={name}(x);")).unwrap();
        assert_eq!(engine.variables["out"].data(), &[0.; 4]);
        assert!(
            engine
                .array("x")
                .unwrap()
                .same_snapshot(&engine.array("alias").unwrap())
        );
    }
}

#[test]
fn functions_and_indexed_variables_keep_precedence_and_argument_evaluation_policy() {
    for name in ["log1p", "expm1"] {
        let mut local = Engine::default();
        local
            .execute(&format!(
                "[a,b]={name}(7);function [a,b]={name}(x);a=x;b=8;end"
            ))
            .unwrap();
        assert_eq!(local.variables["a"].data(), &[7.]);
        assert_eq!(local.variables["b"].data(), &[8.]);
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(
            dir.path().join(format!("{name}.m")),
            format!("function y={name}(x)\ny=x+1;\nend\n"),
        )
        .unwrap();
        let mut file = Engine::default();
        file.set_function_directory(dir.path()).unwrap();
        file.execute(&format!("a={name}(9);")).unwrap();
        assert_eq!(file.variables["a"].data(), &[10.]);
        let mut variable = Engine::default();
        variable
            .execute(&format!("{name}=[11 13];a={name}(2);"))
            .unwrap();
        assert_eq!(variable.variables["a"].data(), &[13.]);
        let mut builtin = Engine::default();
        assert!(
            builtin
                .execute(&format!("[a,b]={name}(figure(9));"))
                .is_err()
        );
        assert!(builtin.figure_ids().is_empty());
        assert!(builtin.execute(&format!("a={name}(figure(10));")).is_err());
        assert_eq!(builtin.figure_ids(), vec![10]);
        assert!(
            builtin
                .execute(&format!("a={name}(0,figure(11));"))
                .is_err()
        );
        assert_eq!(builtin.figure_ids(), vec![10, 11]);
        assert!(matrust::session::FUNCTIONS.contains(&name));
        assert!(matrust::engine::HELP.contains(name));
    }
}

#[test]
fn public_one_eight_pool_contexts_chunk_boundaries_and_pre_cancel() {
    let pools = [1, 8].map(|workers| {
        rayon::ThreadPoolBuilder::new()
            .num_threads(workers)
            .build()
            .unwrap()
    });
    for n in [4095, 4096, 4097, 65537] {
        let tiny = f64::from_bits((1023 - 100) << 52);
        let pattern = [0., -0., tiny, -tiny];
        let input =
            Matrix::from_column_vec(n, 1, (0..n).map(|i| pattern[i % 4]).collect()).unwrap();
        for (pool, workers) in pools.iter().zip([1, 8]) {
            pool.install(|| {
                assert_eq!(rayon::current_num_threads(), workers);
                for name in ["log1p", "expm1"] {
                    let mut engine = Engine::default();
                    engine
                        .merge_workspace(BTreeMap::from([(
                            "x".into(),
                            Array::Numeric(input.clone()),
                        )]))
                        .unwrap();
                    engine.execute(&format!("out={name}(x);")).unwrap();
                    let output = &engine.variables["out"];
                    assert_eq!((output.rows(), output.cols()), (n, 1));
                    for (&got, &expected) in output.data().iter().zip(input.data()) {
                        assert_eq!(got.to_bits(), expected.to_bits());
                    }
                    assert!(
                        engine
                            .array("x")
                            .unwrap()
                            .same_snapshot(&Array::Numeric(input.clone()))
                    );
                }
                let cancel = AtomicBool::new(true);
                assert_eq!(log1p(&input, &cancel).unwrap_err(), "Calculation cancelled");
                assert_eq!(expm1(&input, &cancel).unwrap_err(), "Calculation cancelled");
            });
        }
    }
}
