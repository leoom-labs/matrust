use matrust::{
    LogicalMatrix as L, Matrix as M,
    logical::{ArrayRef as A, BooleanOp as B, Comparison as C, Predicate as P},
};
use std::sync::atomic::{AtomicBool, Ordering};

fn cancel() -> AtomicBool {
    AtomicBool::new(false)
}
fn numeric(rows: usize, cols: usize, values: &[f64]) -> M {
    M::from_column_slice(rows, cols, values).unwrap()
}
fn logical(rows: usize, cols: usize, values: &[u8]) -> L {
    L::from_column_vec(rows, cols, values.to_vec(), &cancel()).unwrap()
}

#[test]
fn logical_storage_is_canonical_bounded_shared_and_column_major() {
    let values = vec![0, 1, 1, 0, 1, 0];
    let original = values.as_ptr();
    let a = L::from_column_vec(2, 3, values, &cancel()).unwrap();
    assert_eq!(
        a.data().as_ptr(),
        original,
        "constructor must take ownership"
    );
    assert!(!a.get(0, 0));
    assert!(a.get(1, 0));
    assert!(a.get(0, 1));
    let b = a.reshape(3, 2).unwrap();
    assert_eq!(a.data().as_ptr(), b.data().as_ptr());
    assert_eq!(a.data().as_ptr(), a.clone().data().as_ptr());
    assert!(a.reshape(1, 5).is_err());
    assert!(a.as_scalar().is_err());
    assert!(L::scalar(true).as_scalar().unwrap());
    assert!(L::from_column_vec(1, 1, vec![2], &cancel()).is_err());
    assert!(L::from_column_vec(1, 1, vec![255], &cancel()).is_err());
    assert!(L::from_column_vec(1, 2, vec![0], &cancel()).is_err());
    for (r, c) in [
        (usize::MAX, 2),
        (0, usize::MAX),
        (1, matrust::matrix::MAX_ELEMENTS + 1),
    ] {
        assert!(L::filled(r, c, false, &cancel()).is_err());
    }
    assert_eq!(std::mem::size_of_val(a.data()), 6);
}

#[test]
fn numeric_conversion_preserves_shape_and_handles_nonfinite_values() {
    let values = [0., -0., 1., -2., f64::INFINITY, f64::NEG_INFINITY];
    let a = numeric(2, 3, &values);
    let b = L::from_numeric(&a, &cancel()).unwrap();
    assert_eq!((b.rows(), b.cols()), (2, 3));
    assert_eq!(b.data(), &[0, 0, 1, 1, 1, 1]);
    assert_eq!(
        b.to_numeric(&cancel()).unwrap().data(),
        &[0., 0., 1., 1., 1., 1.]
    );
    assert_eq!(a.data()[1].to_bits(), (-0.0_f64).to_bits());
    for nan in [f64::NAN, f64::from_bits(0xfff8_0000_0000_0001)] {
        assert!(
            L::from_numeric(&numeric(1, 3, &[0., nan, 1.]), &cancel())
                .unwrap_err()
                .contains("NaN")
        );
    }
    for (r, c) in [(0, 0), (0, 3), (2, 0)] {
        let empty = L::from_numeric(&numeric(r, c, &[]), &cancel()).unwrap();
        let roundtrip = empty.to_numeric(&cancel()).unwrap();
        assert_eq!((roundtrip.rows(), roundtrip.cols()), (r, c));
        assert!(empty.is_empty());
    }
}

#[test]
fn six_relations_follow_ieee_nan_infinity_signed_zero_rules() {
    let a = numeric(
        2,
        3,
        &[1., 2., f64::NAN, f64::INFINITY, f64::NEG_INFINITY, -0.],
    );
    let b = numeric(
        2,
        3,
        &[1., 3., f64::NAN, f64::INFINITY, f64::NEG_INFINITY, 0.],
    );
    let cases = [
        (C::Equal, [1, 0, 0, 1, 1, 1]),
        (C::NotEqual, [0, 1, 1, 0, 0, 0]),
        (C::Less, [0, 1, 0, 0, 0, 0]),
        (C::LessEqual, [1, 1, 0, 1, 1, 1]),
        (C::Greater, [0, 0, 0, 0, 0, 0]),
        (C::GreaterEqual, [1, 0, 0, 1, 1, 1]),
    ];
    for (op, expected) in cases {
        let result = L::compare(A::Numeric(&a), A::Numeric(&b), op, &cancel()).unwrap();
        assert_eq!(result.data(), &expected, "{op:?}");
        assert_eq!((result.rows(), result.cols()), (2, 3));
    }
}

#[test]
fn mixed_comparisons_expand_scalars_without_converting_masks() {
    let a = logical(1, 4, &[0, 1, 0, 1]);
    let b = numeric(1, 4, &[0., 2., f64::NAN, -1.]);
    assert_eq!(
        L::compare(A::Logical(&a), A::Numeric(&b), C::Less, &cancel())
            .unwrap()
            .data(),
        &[0, 1, 0, 0]
    );
    assert_eq!(
        L::compare(A::Numeric(&b), A::Logical(&a), C::Less, &cancel())
            .unwrap()
            .data(),
        &[0, 0, 0, 1]
    );
    let scalar = M::scalar(0.5);
    assert_eq!(
        L::compare(A::Logical(&a), A::Numeric(&scalar), C::Greater, &cancel())
            .unwrap()
            .data(),
        a.data()
    );
    assert_eq!(
        L::compare(A::Numeric(&scalar), A::Logical(&a), C::Less, &cancel())
            .unwrap()
            .data(),
        a.data()
    );
    let t = L::scalar(true);
    assert_eq!(
        L::compare(A::Logical(&t), A::Logical(&a), C::Equal, &cancel())
            .unwrap()
            .data(),
        a.data()
    );
    let s = L::compare(A::Logical(&t), A::Numeric(&scalar), C::Greater, &cancel()).unwrap();
    assert_eq!((s.rows(), s.cols(), s.as_scalar().unwrap()), (1, 1, true));
    let empty = logical(0, 3, &[]);
    for (left, right) in [
        (A::Logical(&empty), A::Numeric(&scalar)),
        (A::Numeric(&scalar), A::Logical(&empty)),
    ] {
        let out = L::compare(left, right, C::Equal, &cancel()).unwrap();
        assert_eq!((out.rows(), out.cols()), (0, 3));
    }
    // Same count is not enough: nonscalar row/column expansion is not implemented.
    let column = a.reshape(4, 1).unwrap();
    assert!(L::compare(A::Logical(&a), A::Logical(&column), C::Equal, &cancel()).is_err());
    assert!(
        L::compare(
            A::Logical(&empty),
            A::Numeric(&numeric(0, 1, &[])),
            C::Equal,
            &cancel()
        )
        .is_err()
    );
}

#[test]
fn predicates_classify_finite_subnormal_infinite_and_nan_values() {
    let a = numeric(
        1,
        7,
        &[
            0.,
            -0.,
            f64::from_bits(1),
            f64::MAX,
            f64::INFINITY,
            f64::NEG_INFINITY,
            f64::NAN,
        ],
    );
    for (op, expected) in [
        (P::IsNan, [0, 0, 0, 0, 0, 0, 1]),
        (P::IsInfinite, [0, 0, 0, 0, 1, 1, 0]),
        (P::IsFinite, [1, 1, 1, 1, 0, 0, 0]),
    ] {
        assert_eq!(
            L::predicate(A::Numeric(&a), op, &cancel()).unwrap().data(),
            &expected
        );
    }
    let b = logical(2, 1, &[0, 1]);
    for op in [P::IsNan, P::IsInfinite, P::IsFinite] {
        let out = L::predicate(A::Logical(&b), op, &cancel()).unwrap();
        assert_eq!(out.data(), &[u8::from(matches!(op, P::IsFinite)); 2]);
        assert_eq!((out.rows(), out.cols()), (2, 1));
    }
}

#[test]
fn boolean_truth_tables_scalar_identities_and_aliases() {
    let a = logical(2, 2, &[0, 0, 1, 1]);
    let b = logical(2, 2, &[0, 1, 0, 1]);
    for (op, expected) in [
        (B::And, [0, 0, 0, 1]),
        (B::Or, [0, 1, 1, 1]),
        (B::Xor, [0, 1, 1, 0]),
    ] {
        assert_eq!(a.combine(&b, op, &cancel()).unwrap().data(), &expected);
    }
    let no = a.not(&cancel()).unwrap();
    assert_eq!(no.data(), &[1, 1, 0, 0]);
    assert_eq!(a.data(), &[0, 0, 1, 1]);
    for (op, value) in [(B::And, true), (B::Or, false), (B::Xor, false)] {
        let s = L::scalar(value);
        for result in [a.combine(&s, op, &cancel()), s.combine(&a, op, &cancel())] {
            assert_eq!(result.unwrap().data().as_ptr(), a.data().as_ptr());
        }
    }
    assert_eq!(
        a.combine(&L::scalar(true), B::Xor, &cancel())
            .unwrap()
            .data(),
        no.data()
    );
    assert_eq!(
        a.combine(&L::scalar(false), B::And, &cancel())
            .unwrap()
            .data(),
        &[0; 4]
    );
    assert_eq!(
        a.combine(&L::scalar(true), B::Or, &cancel())
            .unwrap()
            .data(),
        &[1; 4]
    );
    assert!(
        a.combine(&b.reshape(4, 1).unwrap(), B::And, &cancel())
            .is_err()
    );
    let empty = logical(3, 0, &[]);
    for op in [B::And, B::Or, B::Xor] {
        for value in [false, true] {
            let out = empty.combine(&L::scalar(value), op, &cancel()).unwrap();
            assert_eq!((out.rows(), out.cols()), (3, 0));
        }
    }
}

#[test]
fn logical_relations_cover_both_scalar_directions_and_empty_arrays() {
    let flag = cancel();
    let a = logical(1, 4, &[0, 0, 1, 1]);
    let b = logical(1, 4, &[0, 1, 0, 1]);
    for (op, expected) in [
        (C::Equal, [1, 0, 0, 1]),
        (C::NotEqual, [0, 1, 1, 0]),
        (C::Less, [0, 1, 0, 0]),
        (C::LessEqual, [1, 1, 0, 1]),
        (C::Greater, [0, 0, 1, 0]),
        (C::GreaterEqual, [1, 0, 1, 1]),
    ] {
        assert_eq!(
            L::compare(A::Logical(&a), A::Logical(&b), op, &flag)
                .unwrap()
                .data(),
            &expected
        );
        for (i, &expected_value) in expected.iter().enumerate() {
            let lhs = L::scalar(a.data()[i] != 0);
            let rhs = L::scalar(b.data()[i] != 0);
            assert_eq!(
                L::compare(A::Logical(&lhs), A::Logical(&b), op, &flag)
                    .unwrap()
                    .data()[i],
                expected_value
            );
            assert_eq!(
                L::compare(A::Logical(&a), A::Logical(&rhs), op, &flag)
                    .unwrap()
                    .data()[i],
                expected_value
            );
        }
        let empty = logical(0, 4, &[]);
        let scalar = L::scalar(true);
        for (left, right) in [
            (A::Logical(&empty), A::Logical(&scalar)),
            (A::Logical(&scalar), A::Logical(&empty)),
        ] {
            let result = L::compare(left, right, op, &flag).unwrap();
            assert_eq!((result.rows(), result.cols()), (0, 4));
        }
    }
}

#[test]
fn large_parallel_and_tail_results_match_independent_scalar_reference() {
    for len in [
        8191, 8192, 8193, 524287, 524288, 524289, 1_048_579, 4_194_303, 4_194_304, 4_194_305,
    ] {
        let data: Vec<_> = (0..len).map(|i| ((i * 17) % 37) as f64 - 18.).collect();
        let a = M::from_column_vec(len, 1, data).unwrap();
        for threads in [1, 8] {
            rayon::ThreadPoolBuilder::new()
                .num_threads(threads)
                .build()
                .unwrap()
                .install(|| {
                    let flag = cancel();
                    let positive = L::compare(
                        A::Numeric(&a),
                        A::Numeric(&M::scalar(0.)),
                        C::Greater,
                        &flag,
                    )
                    .unwrap();
                    let nonzero = L::from_numeric(&a, &flag).unwrap();
                    let negative = nonzero
                        .combine(&positive.not(&flag).unwrap(), B::And, &flag)
                        .unwrap();
                    let ordered =
                        L::compare(A::Logical(&positive), A::Logical(&nonzero), C::Less, &flag)
                            .unwrap();
                    let doubles = negative.to_numeric(&flag).unwrap();
                    for (i, &value) in a.data().iter().enumerate() {
                        assert_eq!(positive.data()[i], u8::from(value > 0.));
                        assert_eq!(negative.data()[i], u8::from(value < 0.));
                        assert_eq!(ordered.data()[i], u8::from(value < 0.));
                        assert_eq!(doubles.data()[i], if value < 0. { 1. } else { 0. });
                    }
                    let mut invalid = a.data().to_vec();
                    invalid[len - 1] = f64::NAN;
                    assert!(
                        L::from_numeric(&M::from_column_vec(len, 1, invalid).unwrap(), &flag)
                            .is_err()
                    );
                });
        }
    }
}

#[test]
fn cancellation_applies_to_empty_scalar_shared_and_allocating_paths() {
    let a = logical(2, 2, &[0, 1, 1, 0]);
    let n = numeric(2, 2, &[0., 1., 2., 3.]);
    let flag = cancel();
    flag.store(true, Ordering::Relaxed);
    assert!(L::filled(0, 0, false, &flag).is_err());
    assert!(L::from_column_vec(0, 0, vec![], &flag).is_err());
    assert!(L::from_numeric(&n, &flag).is_err());
    assert!(a.to_numeric(&flag).is_err());
    assert!(a.not(&flag).is_err());
    assert!(a.combine(&L::scalar(true), B::And, &flag).is_err());
    assert!(a.combine(&a, B::Or, &flag).is_err());
    assert!(L::compare(A::Numeric(&n), A::Logical(&a), C::Less, &flag).is_err());
    assert!(L::predicate(A::Numeric(&n), P::IsFinite, &flag).is_err());
    assert_eq!(a.data(), &[0, 1, 1, 0]);
}
