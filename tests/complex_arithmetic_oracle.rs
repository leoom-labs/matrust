//! Separate fixture-consumer draft; add only together with the generated40-case
//! JSON after independent saved-reference audit. No fixture is fabricated here.
use matrust::{Array, Complex64, ComplexMatrix, complex_matrix::ComplexOp};
use serde::Deserialize;
use std::{collections::BTreeSet, sync::atomic::AtomicBool};
#[derive(Deserialize)]
struct Case {
    key: String,
    op: String,
    a_shape: [usize; 2],
    b_shape: [usize; 2],
    shape: [usize; 2],
    a: Vec<[String; 2]>,
    b: Vec<[String; 2]>,
    expected: Vec<[String; 2]>,
}
fn number(text: &str) -> f64 {
    assert_eq!(text.len(), 16);
    f64::from_bits(u64::from_str_radix(text, 16).unwrap())
}
fn array(shape: [usize; 2], values: &[[String; 2]], cancel: &AtomicBool) -> Array {
    assert_eq!(values.len(), shape[0] * shape[1]);
    ComplexMatrix::from_column_vec(
        shape[0],
        shape[1],
        values
            .iter()
            .map(|z| Complex64::new(number(&z[0]), number(&z[1])))
            .collect(),
        cancel,
    )
    .unwrap()
    .into()
}
#[test]
fn complex_arithmetic_matches_exact_fraction_component_reference_grid() {
    let cases: Vec<Case> =
        serde_json::from_str(include_str!("fixtures/complex_arithmetic.json")).unwrap();
    let mut keys = BTreeSet::new();
    for op in ["add", "subtract", "multiply", "divide"] {
        for shape in [
            "empty",
            "scalar",
            "row",
            "column",
            "matrix",
            "scalar_left",
            "scalar_right",
        ] {
            keys.insert(format!("{op}:{shape}"));
        }
    }
    for (m, k, n) in [(1, 7, 1), (3, 7, 4), (5, 129, 3), (129, 3, 33)] {
        keys.insert(format!("matmul:{m}:{k}:{n}"));
    }
    for name in [
        "maximum_identity",
        "subnormal_identity",
        "large_rotation",
        "minimum_rotation",
        "mixed_small",
        "large_finite",
        "underflow_tie",
        "tiny_real",
    ] {
        keys.insert(format!("divide:extreme_{name}"));
    }
    assert_eq!(cases.len(), 40);
    let cancel = AtomicBool::new(false);
    let pools = [1, 8].map(|n| {
        rayon::ThreadPoolBuilder::new()
            .num_threads(n)
            .build()
            .unwrap()
    });
    for case in cases {
        assert!(
            keys.remove(&case.key),
            "Unexpected/duplicate key {}",
            case.key
        );
        let op = match case.op.as_str() {
            "add" => ComplexOp::Add,
            "subtract" => ComplexOp::Subtract,
            "multiply" => ComplexOp::Multiply,
            "divide" => ComplexOp::Divide,
            "matmul" => ComplexOp::Matmul,
            _ => panic!("Unknown operator"),
        };
        let a = array(case.a_shape, &case.a, &cancel);
        let b = array(case.b_shape, &case.b, &cancel);
        assert_eq!(case.expected.len(), case.shape[0] * case.shape[1]);
        for pool in &pools {
            let output = pool
                .install(|| ComplexMatrix::arithmetic(&a, &b, op, &cancel))
                .unwrap();
            assert_eq!([output.rows(), output.cols()], case.shape);
            for (index, (value, reference)) in output.data().iter().zip(&case.expected).enumerate()
            {
                for (got, bits) in [(value.re, &reference[0]), (value.im, &reference[1])] {
                    let want = number(bits);
                    assert!(got.is_finite() && want.is_finite());
                    let bound = if case.key.starts_with("divide:extreme_") {
                        2e-13 * want.abs() + 8. * f64::from_bits(1)
                    } else {
                        2e-13 * (1. + want.abs())
                    };
                    assert!(
                        (got - want).abs() <= bound,
                        "{}[{index}]: {got} vs {want}",
                        case.key
                    );
                }
            }
        }
    }
    assert!(keys.is_empty());
}
