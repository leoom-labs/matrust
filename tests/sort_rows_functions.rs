use matrust::{
    Array, Engine, LogicalMatrix, Matrix,
    array::{MissingPlacement as M, SortOrder as O},
};
use std::{cmp::Ordering, sync::atomic::AtomicBool};

fn check(a: &Array, columns: Option<&[i32]>, direction: Option<O>, missing: M, indices: &[usize]) {
    let cancel = AtomicBool::new(false);
    for want in [false, true] {
        let (b, i) = a
            .sort_rows(columns, direction, missing, want, &cancel)
            .unwrap();
        assert_eq!((b.rows(), b.cols()), (a.rows(), a.cols()));
        assert_eq!(b.is_logical(), a.is_logical());
        for col in 0..a.cols() {
            for (row, &old) in indices.iter().enumerate() {
                assert_eq!(
                    b.scalar(row, col).unwrap().into_real().unwrap().to_bits(),
                    a.scalar(old, col).unwrap().into_real().unwrap().to_bits()
                );
            }
        }
        if want {
            let i = i.unwrap();
            assert_eq!((i.rows(), i.cols()), (a.rows(), 1));
            assert_eq!(
                i.data(),
                indices.iter().map(|&i| i as f64 + 1.).collect::<Vec<_>>()
            );
        } else {
            assert!(i.is_none());
        }
    }
}

// Independent direct lexicographic stable-sort reference. It does not use
// candidate scalar passes, ranks, index tie-breaks or bounded merging.
fn reference(a: &Array, columns: Option<&[i32]>, direction: Option<O>, missing: M) -> Vec<usize> {
    let columns: Vec<_> = columns.map_or_else(
        || (1..=a.cols()).map(|c| c as i32).collect(),
        |c| c.to_vec(),
    );
    let mut order: Vec<_> = (0..a.rows()).collect();
    order.sort_by(|&r, &s| {
        for &col in &columns {
            let d = direction.unwrap_or(if col < 0 { O::Descend } else { O::Ascend });
            let first = missing == M::First || missing == M::Auto && d == O::Descend;
            let (x, y) = (
                a.scalar(r, col.unsigned_abs() as usize - 1)
                    .unwrap()
                    .into_real()
                    .unwrap(),
                a.scalar(s, col.unsigned_abs() as usize - 1)
                    .unwrap()
                    .into_real()
                    .unwrap(),
            );
            let cmp = match (x.is_nan(), y.is_nan()) {
                (true, true) => Ordering::Equal,
                (true, false) => {
                    if first {
                        Ordering::Less
                    } else {
                        Ordering::Greater
                    }
                }
                (false, true) => {
                    if first {
                        Ordering::Greater
                    } else {
                        Ordering::Less
                    }
                }
                _ => {
                    let c = x.partial_cmp(&y).unwrap();
                    if d == O::Descend { c.reverse() } else { c }
                }
            };
            if cmp != Ordering::Equal {
                return cmp;
            }
        }
        Ordering::Equal
    });
    order
}

#[test]
fn sortrows_priority_sign_override_and_stability() {
    let a: Array = Matrix::from_column_slice(
        5,
        3,
        &[2., 1., 1., 2., 1., 0., 3., 2., 0., 2., 9., 8., 7., 6., 5.],
    )
    .unwrap()
    .into();
    check(&a, None, None, M::Auto, &[4, 2, 1, 3, 0]);
    check(&a, Some(&[1, -2]), None, M::Auto, &[1, 2, 4, 0, 3]);
    check(
        &a,
        Some(&[-1, 2]),
        Some(O::Ascend),
        M::Auto,
        &[2, 4, 1, 0, 3],
    );
    check(&a, Some(&[2, 2, -1]), None, M::Auto, &[0, 3, 2, 4, 1]);
}

#[test]
fn sortrows_nonfinite_and_signed_zero_ties_keep_original_bits() {
    let n1 = f64::from_bits(0x7ff8000000000001);
    let n2 = f64::from_bits(0xfff8000000000017);
    let a: Array = Matrix::from_column_slice(
        9,
        2,
        &[
            n1,
            -0.,
            0.,
            n2,
            2.,
            f64::INFINITY,
            f64::NEG_INFINITY,
            2.,
            n1,
            1.,
            3.,
            3.,
            1.,
            0.,
            0.,
            0.,
            0.,
            0.,
        ],
    )
    .unwrap()
    .into();
    for columns in [None, Some(&[1][..]), Some(&[-1, 2][..])] {
        for direction in [None, Some(O::Ascend), Some(O::Descend)] {
            for missing in [M::Auto, M::First, M::Last] {
                check(
                    &a,
                    columns,
                    direction,
                    missing,
                    &reference(&a, columns, direction, missing),
                );
            }
        }
    }
}

#[test]
fn sortrows_empty_single_row_logical_shapes_and_invalid_keys() {
    let cancel = AtomicBool::new(false);
    for (r, c) in [(0, 0), (0, 3), (7, 0), (1, 0), (0, 1), (1, 1), (1, 7)] {
        let a: Array = Matrix::filled(r, c, -0.).unwrap().into();
        let (b, i) = a.sort_rows(None, None, M::Auto, true, &cancel).unwrap();
        assert!(a.same_snapshot(&b));
        let i = i.unwrap();
        assert_eq!((i.rows(), i.cols()), (r, 1));
        assert_eq!(i.data(), (1..=r).map(|v| v as f64).collect::<Vec<_>>());
        assert!(
            a.sort_rows(Some(&[]), None, M::Auto, false, &cancel)
                .is_err()
        );
    }
    let a: Array =
        LogicalMatrix::from_column_vec(5, 2, vec![1, 0, 1, 0, 0, 0, 1, 0, 0, 1], &cancel)
            .unwrap()
            .into();
    check(&a, None, None, M::Auto, &[3, 1, 4, 0, 2]);
    check(&a, Some(&[-2, 1]), None, M::Last, &[1, 4, 3, 0, 2]);
    for columns in [&[0][..], &[3], &[i32::MIN]] {
        assert!(
            a.sort_rows(Some(columns), None, M::Auto, false, &cancel)
                .is_err()
        );
    }
    assert!(
        a.sort_rows(None, None, M::Auto, false, &AtomicBool::new(true))
            .unwrap_err()
            .contains("cancelled")
    );
}

#[test]
fn sortrows_run_merge_tails_and_parallel_results_match_independent_reference() {
    let pools = [1, 8].map(|n| {
        rayon::ThreadPoolBuilder::new()
            .num_threads(n)
            .build()
            .unwrap()
    });
    for (r, c) in [
        (8191, 3),
        (8192, 3),
        (8193, 3),
        (16385, 2),
        (524289, 2),
        (2, 8193),
    ] {
        let a: Array =
            Matrix::from_column_vec(r, c, (0..r * c).map(|i| ((i * 37) % 101) as f64).collect())
                .unwrap()
                .into();
        let alias = a.clone();
        let columns = [-1, c as i32];
        let expected = reference(&a, Some(&columns), None, M::Auto);
        for pool in &pools {
            pool.install(|| check(&a, Some(&columns), None, M::Auto, &expected));
        }
        assert!(a.same_snapshot(&alias));
    }
}

#[test]
fn sortrows_script_forms_errors_and_shadowing_are_transactional() {
    let mut e = Engine::default();
    e.execute("A=[2 0;1 3;1 2;2 0];[B,I]=sortrows(A);[C,~]=sortrows(A,[1 -2]);[D,J]=sortrows(A,[-1 2],'ascend');[L,K]=sortrows(logical(A));R=7;S=8;").unwrap();
    assert_eq!(e.variables["I"].data(), &[3., 2., 1., 4.]);
    assert_eq!(e.variables["J"].data(), &[3., 2., 1., 4.]);
    assert!(e.logical_variables.contains_key("L"));
    for bad in [
        "sortrows()",
        "sortrows(A,[])",
        "sortrows(A,0)",
        "sortrows(A,1.5)",
        "sortrows(A,3)",
        "sortrows(A,logical(1))",
        "sortrows(A,[1 2;1 2])",
        "sortrows(A,'abs')",
        "sortrows(A,'ComparisonMethod','real')",
        "sortrows(A,'MissingPlacement','bad')",
        "sortrows(A,'MissingPlacement','first','MissingPlacement','last')",
    ] {
        assert!(e.execute(&format!("[R,S]={bad};")).is_err(), "{bad}");
        assert_eq!(e.variables["R"].data(), &[7.]);
        assert_eq!(e.variables["S"].data(), &[8.]);
    }
    assert!(e.execute("[R,S,T]=sortrows(A);").is_err());
    assert!(matrust::session::FUNCTIONS.contains(&"sortrows"));
    e.execute("sortrows=[11 22];R=sortrows(2);").unwrap();
    assert_eq!(e.variables["R"].data(), &[22.]);
    assert!(
        e.execute("[R,S]=sortrows(1);")
            .unwrap_err()
            .contains("Array indexing")
    );
    // Use a fresh engine: variable-specific clear syntax is intentionally not assumed.
    let mut e = Engine::default();
    e.execute("[B,I]=sortrows(9);function [b,i]=sortrows(a);b=a+1;i=77;end")
        .unwrap();
    assert_eq!(e.variables["B"].data(), &[10.]);
    assert_eq!(e.variables["I"].data(), &[77.]);
}

#[test]
fn sortrows_stable_scalar_passes_preserve_payload_ties_across_run_boundary() {
    let rows = 8193;
    let nan = f64::from_bits(0x7ff8000000000037);
    let numeric: Array = Matrix::from_column_vec(
        rows,
        3,
        (0..rows * 3)
            .map(|i| match i / rows {
                0 => match i % 5 {
                    0 => nan,
                    1 => -0.,
                    2 => 0.,
                    _ => 1.,
                },
                1 => ((i % rows) % 7) as f64,
                _ => ((i % rows) % 3) as f64,
            })
            .collect(),
    )
    .unwrap()
    .into();
    let cancel = AtomicBool::new(false);
    let logical: Array = LogicalMatrix::from_column_vec(
        rows,
        3,
        (0..rows * 3)
            .map(|i| ((i * 17 + i / rows) % 3 == 0) as u8)
            .collect(),
        &cancel,
    )
    .unwrap()
    .into();
    for a in [numeric, logical] {
        let alias = a.clone();
        for columns in [None, Some(&[2, -1, 2, 3][..])] {
            for direction in [None, Some(O::Descend)] {
                for missing in [M::Auto, M::First, M::Last] {
                    check(
                        &a,
                        columns,
                        direction,
                        missing,
                        &reference(&a, columns, direction, missing),
                    );
                }
            }
        }
        assert!(a.same_snapshot(&alias));
    }
}

#[test]
fn sortrows_two_rows_preserve_typed_bits_priority_and_missing_semantics() {
    let nan_a = f64::from_bits(0x7ff8000000000011);
    let nan_b = f64::from_bits(0x7ff8000000000037);
    let cancel = AtomicBool::new(false);
    let cases: Vec<Array> = vec![
        Matrix::from_column_vec(2, 4, vec![2., 1., 0., 9., 8., 3., -1., 7.])
            .unwrap()
            .into(),
        Matrix::from_column_vec(
            2,
            4,
            vec![-0., 0., nan_a, nan_b, f64::INFINITY, f64::INFINITY, 2., 1.],
        )
        .unwrap()
        .into(),
        Matrix::from_column_vec(
            2,
            4,
            vec![
                nan_a,
                4.,
                nan_b,
                nan_a,
                f64::NEG_INFINITY,
                f64::INFINITY,
                -0.,
                0.,
            ],
        )
        .unwrap()
        .into(),
        Matrix::from_column_vec(
            2,
            4,
            vec![-0., 0., nan_a, nan_b, 3., 3., f64::INFINITY, f64::INFINITY],
        )
        .unwrap()
        .into(),
        LogicalMatrix::from_column_vec(2, 4, vec![1, 1, 0, 1, 1, 0, 0, 0], &cancel)
            .unwrap()
            .into(),
    ];
    for a in cases {
        let alias = a.clone();
        for columns in [
            None,
            Some(&[1, 2, 3, 4][..]),
            Some(&[4, 3, 2, 1][..]),
            Some(&[-2, 1, -4, 2][..]),
            Some(&[2, -2, 1][..]),
        ] {
            for direction in [None, Some(O::Ascend), Some(O::Descend)] {
                for missing in [M::Auto, M::First, M::Last] {
                    check(
                        &a,
                        columns,
                        direction,
                        missing,
                        &reference(&a, columns, direction, missing),
                    );
                }
            }
        }
        assert!(a.same_snapshot(&alias));
    }
}

#[test]
fn sortrows_two_rows_validate_all_keys_before_early_exit_and_cancel_without_mutation() {
    let a: Array = Matrix::from_column_vec(2, 2, vec![2., 1., 0., 0.])
        .unwrap()
        .into();
    let alias = a.clone();
    let cancel = AtomicBool::new(false);
    for columns in [&[1, 0][..], &[1, 3][..], &[1, i32::MIN][..]] {
        assert!(
            a.sort_rows(Some(columns), None, M::Auto, true, &cancel)
                .is_err()
        );
        assert!(a.same_snapshot(&alias));
    }
    cancel.store(true, std::sync::atomic::Ordering::Relaxed);
    assert!(
        a.sort_rows(None, None, M::Auto, true, &cancel)
            .unwrap_err()
            .contains("cancelled")
    );
    assert!(a.same_snapshot(&alias));
    cancel.store(false, std::sync::atomic::Ordering::Relaxed);
    check(&a, None, None, M::Auto, &[1, 0]);
    let empty: Array = Matrix::from_column_vec(2, 0, vec![]).unwrap().into();
    check(&empty, None, None, M::Auto, &[0, 1]);
}
