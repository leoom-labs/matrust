//! Exact binary identities and explicit subset contracts; no vendor runtime credit.
use matrust::{
    Engine, Matrix,
    array::{nextpow2, pow2, scale_pow2},
};
use std::sync::atomic::{AtomicBool, Ordering};

fn power(exponent: i32) -> f64 {
    // Independent reference: place the sole significand bit, without a log or
    // the implementation's exponent extraction/scaling kernels.
    assert!((-1074..=1023).contains(&exponent));
    f64::from_bits(if exponent < -1022 {
        1u64 << (exponent + 1074)
    } else {
        ((exponent + 1023) as u64) << 52
    })
}

#[test]
fn every_binary_power_and_both_adjacent_magnitudes_have_exact_exponents() {
    let cancel = AtomicBool::new(false);
    for exponent in -1074..=1023 {
        let value = power(exponent);
        let below = f64::from_bits(value.to_bits() - 1);
        let above = f64::from_bits(value.to_bits() + 1);
        let lower = match exponent {
            -1074 => 0,
            -1073 => -1074,
            _ => exponent,
        };
        for sign in [1., -1.] {
            let input =
                Matrix::from_column_slice(1, 3, &[sign * below, sign * value, sign * above])
                    .unwrap();
            let result = nextpow2(&input, &cancel).unwrap();
            assert_eq!(
                result.data(),
                &[
                    f64::from(lower),
                    f64::from(exponent),
                    f64::from(exponent + 1)
                ]
            );
        }
    }
    let input = Matrix::from_column_slice(
        1,
        6,
        &[
            0.,
            -0.,
            f64::MAX,
            f64::INFINITY,
            f64::NEG_INFINITY,
            f64::NAN,
        ],
    )
    .unwrap();
    let result = nextpow2(&input, &cancel).unwrap();
    assert_eq!(result.data()[0].to_bits(), 0);
    assert_eq!(result.data()[1].to_bits(), 0);
    assert_eq!(&result.data()[2..5], &[1024., f64::INFINITY, f64::INFINITY]);
    assert!(result.data()[5].is_nan());
}

#[test]
fn unary_integer_powers_ieee_and_fractional_identity() {
    let cancel = AtomicBool::new(false);
    let input = Matrix::from_column_vec(1, 2098, (-1074..=1023).map(f64::from).collect()).unwrap();
    let result = pow2(&input, &cancel).unwrap();
    for (&got, exponent) in result.data().iter().zip(-1074..=1023) {
        assert_eq!(got.to_bits(), power(exponent).to_bits(), "{exponent}");
    }
    let input = Matrix::from_column_slice(
        1,
        8,
        &[
            -1075.,
            1024.,
            0.,
            -0.,
            f64::NEG_INFINITY,
            f64::INFINITY,
            f64::NAN,
            0.5,
        ],
    )
    .unwrap();
    let result = pow2(&input, &cancel).unwrap();
    for (&got, expected) in
        result.data()[..6]
            .iter()
            .zip([0.0f64, f64::INFINITY, 1., 1., 0., f64::INFINITY])
    {
        assert_eq!(got.to_bits(), expected.to_bits());
    }
    assert!(result.data()[6].is_nan());
    let root = result.data()[7];
    assert!(root > 1. && root < 2.);
    assert!((root * root - 2.).abs() <= 2. * f64::EPSILON);
}

#[test]
fn scaling_truncation_extreme_intermediates_rounding_and_saturated_exponents() {
    let cancel = AtomicBool::new(false);
    let cases = [
        (0.5, 1024., power(1023)),
        (power(-1074), 1074., 1.),
        (f64::MAX, -1023., f64::from_bits(0x3fff_ffff_ffff_ffff)),
        (1., 2.9, 4.),
        (1., -2.9, 0.25),
        (1., -0.9, 1.),
        (power(-1074), -1., 0.),
        (-power(-1074), -1., -0.),
        (f64::from_bits(3), -1., f64::from_bits(2)),
        (power(-1074), f64::MAX, f64::INFINITY),
        (-power(-1074), f64::MAX, f64::NEG_INFINITY),
        (f64::MAX, -f64::MAX, 0.),
        (-f64::MAX, -f64::MAX, -0.),
        (0., f64::MAX, 0.),
        (-0., f64::MAX, -0.),
        (-0., -f64::MAX, -0.),
        (f64::INFINITY, -f64::MAX, f64::INFINITY),
        (f64::NEG_INFINITY, f64::MAX, f64::NEG_INFINITY),
        (f64::MAX, 1., f64::INFINITY),
    ];
    for (f, e, expected) in cases {
        let result = scale_pow2(&Matrix::scalar(f), &Matrix::scalar(e), &cancel).unwrap();
        assert_eq!(
            result.data()[0].to_bits(),
            expected.to_bits(),
            "F={f:?}, E={e:?}"
        );
    }
    assert!(
        scale_pow2(
            &Matrix::scalar(f64::NAN),
            &Matrix::scalar(f64::MAX),
            &cancel
        )
        .unwrap()
        .data()[0]
            .is_nan()
    );
    for bad in [f64::NAN, f64::INFINITY, f64::NEG_INFINITY] {
        assert!(
            scale_pow2(&Matrix::scalar(0.), &Matrix::scalar(bad), &cancel)
                .unwrap_err()
                .contains("finite exponents")
        );
    }
}

#[test]
fn shapes_scalar_expansion_empties_and_shared_storage_are_explicit() {
    let cancel = AtomicBool::new(false);
    for (rows, cols) in [
        (1, 6),
        (6, 1),
        (2, 3),
        (0, 0),
        (1, 0),
        (0, 1),
        (0, 3),
        (4, 0),
    ] {
        let input = Matrix::from_column_vec(rows, cols, vec![2.; rows * cols]).unwrap();
        let alias = input.clone();
        for output in [
            nextpow2(&input, &cancel).unwrap(),
            pow2(&input, &cancel).unwrap(),
            scale_pow2(&input, &Matrix::scalar(1.), &cancel).unwrap(),
            scale_pow2(&Matrix::scalar(1.), &input, &cancel).unwrap(),
        ] {
            assert_eq!((output.rows(), output.cols()), (rows, cols));
        }
        assert_eq!(input.data().as_ptr(), alias.data().as_ptr());
    }
    let row = Matrix::from_column_slice(1, 2, &[1., 2.]).unwrap();
    let col = Matrix::from_column_slice(2, 1, &[1., 2.]).unwrap();
    assert!(scale_pow2(&row, &col, &cancel).is_err());
    let empty = Matrix::from_column_vec(0, 3, vec![]).unwrap();
    let other = Matrix::from_column_vec(3, 0, vec![]).unwrap();
    assert!(scale_pow2(&empty, &other, &cancel).is_err());
    assert!(scale_pow2(&empty, &Matrix::scalar(f64::NAN), &cancel).is_err());
    assert_eq!(scale_pow2(&empty, &empty, &cancel).unwrap().cols(), 3);
}

#[test]
fn typed_calls_csl_precedence_failure_and_cancellation_preserve_workspace() {
    for name in ["nextpow2", "pow2"] {
        let mut engine = Engine::default();
        engine
            .execute(&format!(
                "x=[1 2;4 8];alias=x;args={{x}};out={name}(args{{:}});saved=out;"
            ))
            .unwrap();
        let before = engine.workspace();
        for args in [
            "",
            "logical(x)",
            "complex(x,0)",
            "logical([])",
            "complex([],[])",
            "'abc'",
            "char([65])",
            "{x}",
            "x,x,x",
        ] {
            assert!(engine.execute(&format!("out={name}({args});")).is_err());
            for (key, value) in &before {
                assert!(engine.array(key).unwrap().same_snapshot(value));
            }
        }
        let cancel = engine.cancellation();
        cancel.store(true, Ordering::Relaxed);
        assert!(
            engine
                .execute(&format!("out={name}(x);"))
                .unwrap_err()
                .contains("cancelled")
        );
        cancel.store(false, Ordering::Relaxed);
        assert!(engine.array("out").unwrap().same_snapshot(&before["out"]));
        engine.execute(&format!("out={name}(x);")).unwrap();
        assert!(
            engine
                .array("x")
                .unwrap()
                .same_snapshot(&engine.array("alias").unwrap())
        );
        let mut local = Engine::default();
        local
            .execute(&format!(
                "[a,b]={name}(7);function [a,b]={name}(x);a=x;b=8;end"
            ))
            .unwrap();
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
        let mut indexed = Engine::default();
        indexed
            .execute(&format!("{name}=[11 13];a={name}(2);"))
            .unwrap();
        assert_eq!(indexed.variables["a"].data(), &[13.]);
        let mut builtin = Engine::default();
        assert!(
            builtin
                .execute(&format!("[a,b]={name}(figure(9));"))
                .is_err()
        );
        assert!(builtin.figure_ids().is_empty());
        assert!(builtin.execute(&format!("a={name}(figure(10));")).is_err());
        assert_eq!(builtin.figure_ids(), vec![10]);
        assert!(matrust::session::FUNCTIONS.contains(&name));
        assert!(matrust::engine::HELP.contains(name));
    }
    let mut engine = Engine::default();
    engine
        .execute("args={[-1 2],[2.9 -2.9]};out=pow2(args{:});")
        .unwrap();
    assert_eq!(engine.variables["out"].data(), &[-4., 0.5]);
    assert!(engine.execute("out=nextpow2(1,2);").is_err());
    assert!(engine.execute("out=pow2(1,logical(1));").is_err());
    assert!(engine.execute("out=pow2(1,Inf);").is_err());
    assert_eq!(engine.variables["out"].data(), &[-4., 0.5]);
}

#[test]
fn serial_one_eight_contexts_chunk_boundaries_and_pre_cancel() {
    let pools = [1, 8].map(|n| {
        rayon::ThreadPoolBuilder::new()
            .num_threads(n)
            .build()
            .unwrap()
    });
    for len in [4095, 4096, 4097, 65537] {
        let input = Matrix::from_column_vec(
            len,
            1,
            (0..len).map(|i| power((i % 16) as i32 - 8)).collect(),
        )
        .unwrap();
        for (pool, workers) in pools.iter().zip([1, 8]) {
            pool.install(|| {
                assert_eq!(rayon::current_num_threads(), workers);
                let cancel = AtomicBool::new(false);
                let result = nextpow2(&input, &cancel).unwrap();
                for (i, &got) in result.data().iter().enumerate() {
                    assert_eq!(got, (i % 16) as f64 - 8.);
                }
                let rebuilt = pow2(&result, &cancel).unwrap();
                let scaled = scale_pow2(&input, &Matrix::scalar(0.), &cancel).unwrap();
                for ((&got, &same), &expected) in
                    rebuilt.data().iter().zip(scaled.data()).zip(input.data())
                {
                    assert_eq!(got.to_bits(), expected.to_bits());
                    assert_eq!(same.to_bits(), expected.to_bits());
                }
                cancel.store(true, Ordering::Relaxed);
                assert_eq!(
                    nextpow2(&input, &cancel).unwrap_err(),
                    "Calculation cancelled"
                );
                assert_eq!(pow2(&input, &cancel).unwrap_err(), "Calculation cancelled");
                assert_eq!(
                    scale_pow2(&input, &input, &cancel).unwrap_err(),
                    "Calculation cancelled"
                );
            });
        }
    }
}
