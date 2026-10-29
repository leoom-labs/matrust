use matrust::{
    Array, Engine, LogicalMatrix, Matrix,
    array::{MissingPlacement as M, SortOrder as O},
    matrix::MAX_ELEMENTS,
};
use std::sync::atomic::{AtomicBool, Ordering};

fn numeric(rows: usize, cols: usize, data: &[f64]) -> Array {
    Matrix::from_column_slice(rows, cols, data).unwrap().into()
}
fn expect(a: &Array, shape: (usize, usize), expected: &[f64]) {
    assert_eq!((a.rows(), a.cols()), shape);
    assert_eq!(a.len(), expected.len());
    for (i, value) in expected.iter().enumerate() {
        assert_eq!(
            a.scalar(i % shape.0, i / shape.0)
                .unwrap()
                .into_real()
                .unwrap()
                .to_bits(),
            value.to_bits()
        );
    }
}
fn index(i: Option<Matrix>, expected: &[f64]) {
    assert_eq!(i.unwrap().data(), expected);
}

#[test]
fn sort_numeric_dimensions_and_stable_nonfinite_ties_preserve_bits() {
    let c = AtomicBool::new(false);
    let a = numeric(2, 3, &[3., 8., 1., 7., 2., 9.]);
    let (b, i) = a.sort_dimension(2, O::Ascend, M::Auto, true, &c).unwrap();
    expect(&b, (2, 3), &[1., 7., 2., 8., 3., 9.]);
    index(i, &[2., 2., 3., 1., 1., 3.]);
    let (b, i) = a.sort_dimension(1, O::Descend, M::Auto, true, &c).unwrap();
    expect(&b, (2, 3), &[8., 3., 7., 1., 9., 2.]);
    index(i, &[2., 1., 2., 1., 2., 1.]);
    let n1 = f64::from_bits(0x7ff8_0000_0000_0001);
    let n2 = f64::from_bits(0xfff8_0000_0000_0100);
    let a = numeric(
        1,
        8,
        &[n1, -0., 2., 0., n2, f64::NEG_INFINITY, 2., f64::INFINITY],
    );
    for (order, missing, positions) in [
        (O::Ascend, M::Auto, [5, 1, 3, 2, 6, 7, 0, 4]),
        (O::Descend, M::Auto, [0, 4, 7, 2, 6, 1, 3, 5]),
        (O::Ascend, M::First, [0, 4, 5, 1, 3, 2, 6, 7]),
        (O::Descend, M::Last, [7, 2, 6, 1, 3, 5, 0, 4]),
    ] {
        let (b, i) = a.sort_dimension(2, order, missing, true, &c).unwrap();
        expect(
            &b,
            (1, 8),
            &positions.map(|p| a.scalar(0, p).unwrap().into_real().unwrap()),
        );
        index(i, &positions.map(|p| p as f64 + 1.));
    }
}

#[test]
fn sort_logical_rectangular_rows_and_columns_return_stable_double_indices() {
    let c = AtomicBool::new(false);
    let a: Array = LogicalMatrix::from_column_vec(2, 4, vec![1, 0, 0, 1, 1, 0, 0, 1], &c)
        .unwrap()
        .into();
    for (order, values, indices) in [
        (
            O::Ascend,
            [0., 0., 0., 0., 1., 1., 1., 1.],
            [2., 1., 4., 3., 1., 2., 3., 4.],
        ),
        (
            O::Descend,
            [1., 1., 1., 1., 0., 0., 0., 0.],
            [1., 2., 3., 4., 2., 1., 4., 3.],
        ),
    ] {
        let (b, i) = a.sort_dimension(2, order, M::First, true, &c).unwrap();
        assert!(b.is_logical());
        expect(&b, (2, 4), &values);
        index(i, &indices);
        let (without, i) = a.sort_dimension(2, order, M::Last, false, &c).unwrap();
        expect(&without, (2, 4), &values);
        assert!(i.is_none());
    }
    let (b, i) = a.sort_dimension(1, O::Ascend, M::Auto, true, &c).unwrap();
    expect(&b, (2, 4), &[0., 1., 0., 1., 0., 1., 0., 1.]);
    index(i, &[2., 1., 1., 2., 2., 1., 1., 2.]);
}

#[test]
fn sort_empty_scalar_higher_dimensions_share_input_and_validate_before_work() {
    let c = AtomicBool::new(false);
    for (r, col) in [(0, 0), (0, 7), (7, 0), (1, 0), (0, 1), (1, 1)] {
        let a: Array = Matrix::filled(r, col, -0.).unwrap().into();
        for dim in [1, 2, 3, MAX_ELEMENTS] {
            let (b, i) = a
                .sort_dimension(dim, O::Descend, M::Auto, true, &c)
                .unwrap();
            assert!(b.same_snapshot(&a));
            let i = i.unwrap();
            assert_eq!((i.rows(), i.cols()), (r, col));
            assert!(i.data().iter().all(|v| *v == 1.));
        }
    }
    let a = numeric(2, 2, &[4., 1., 3., 2.]);
    let (b, i) = a.sort_dimension(3, O::Ascend, M::Auto, true, &c).unwrap();
    assert!(b.same_snapshot(&a));
    index(i, &[1.; 4]);
    for dim in [0, MAX_ELEMENTS + 1, usize::MAX] {
        assert!(
            a.sort_dimension(dim, O::Ascend, M::Auto, false, &c)
                .is_err()
        );
    }
    c.store(true, Ordering::Relaxed);
    assert!(
        a.sort_dimension(1, O::Ascend, M::Auto, false, &c)
            .unwrap_err()
            .contains("cancelled")
    );
}

// Independent reference uses one standard stable sort per lane, without the
// candidate's index tie-break, chunk merging, permutation layout or gather.
fn reference(a: &Array, dim: usize, descending: bool) -> (Vec<f64>, Vec<f64>) {
    let (r, c) = (a.rows(), a.cols());
    let mut output = vec![0.; a.len()];
    let mut indices = output.clone();
    let (lanes, n) = if dim == 1 { (c, r) } else { (r, c) };
    for lane in 0..lanes {
        let mut pairs: Vec<_> = (0..n)
            .map(|at| {
                (
                    if dim == 1 {
                        a.scalar(at, lane).unwrap().into_real().unwrap()
                    } else {
                        a.scalar(lane, at).unwrap().into_real().unwrap()
                    },
                    at,
                )
            })
            .collect();
        pairs.sort_by(|a, b| {
            let key = |x: f64| if x.is_nan() { f64::INFINITY } else { x };
            let cmp = match (a.0.is_nan(), b.0.is_nan()) {
                (true, false) => std::cmp::Ordering::Greater,
                (false, true) => std::cmp::Ordering::Less,
                _ => key(a.0).partial_cmp(&key(b.0)).unwrap(),
            };
            if descending { cmp.reverse() } else { cmp }
        });
        for (at, (value, original)) in pairs.into_iter().enumerate() {
            let out = if dim == 1 {
                lane * r + at
            } else {
                lane + at * r
            };
            output[out] = value;
            indices[out] = original as f64 + 1.;
        }
    }
    (output, indices)
}

#[test]
fn sort_run_tails_and_parallel_vectors_lanes_match_independent_stable_reference() {
    let c = AtomicBool::new(false);
    let pools = [1, 8].map(|n| {
        rayon::ThreadPoolBuilder::new()
            .num_threads(n)
            .build()
            .unwrap()
    });
    for (r, col, dim) in [
        (8191, 1, 1),
        (8192, 1, 1),
        (8193, 1, 1),
        (16385, 1, 1),
        (524289, 1, 1),
        (8193, 64, 1),
        (64, 8193, 2),
    ] {
        let a: Array = Matrix::from_column_vec(
            r,
            col,
            (0..r * col)
                .map(|i| match i % 257 {
                    0 => f64::from_bits(0x7ff8_0000_0000_0000 + (i % 17) as u64),
                    1 => -0.,
                    2 => 0.,
                    _ => ((i * 7919) % 997) as f64 - 499.,
                })
                .collect(),
        )
        .unwrap()
        .into();
        for descending in [false, true] {
            let (expected, positions) = reference(&a, dim, descending);
            for pool in &pools {
                let (b, i) = pool
                    .install(|| {
                        a.sort_dimension(
                            dim,
                            if descending { O::Descend } else { O::Ascend },
                            M::Auto,
                            true,
                            &c,
                        )
                    })
                    .unwrap();
                expect(&b, (r, col), &expected);
                index(i, &positions);
            }
        }
    }
}

#[test]
fn sort_logical_parallel_gather_and_bucket_indices_match_stable_reference() {
    let c = AtomicBool::new(false);
    let pools = [1, 8].map(|n| {
        rayon::ThreadPoolBuilder::new()
            .num_threads(n)
            .build()
            .unwrap()
    });
    for (r, col, dim) in [(524289, 1, 1), (8193, 64, 1), (64, 8193, 2)] {
        let a: Array = LogicalMatrix::from_column_vec(
            r,
            col,
            (0..r * col).map(|i| u8::from(i % 7 < 3)).collect(),
            &c,
        )
        .unwrap()
        .into();
        for descending in [false, true] {
            let (expected, positions) = reference(&a, dim, descending);
            for pool in &pools {
                for want_indices in [false, true] {
                    let (b, i) = pool
                        .install(|| {
                            a.sort_dimension(
                                dim,
                                if descending { O::Descend } else { O::Ascend },
                                M::Auto,
                                want_indices,
                                &c,
                            )
                        })
                        .unwrap();
                    assert!(b.is_logical());
                    expect(&b, (r, col), &expected);
                    if want_indices {
                        index(i, &positions);
                    } else {
                        assert!(i.is_none());
                    }
                }
            }
        }
    }
}

#[test]
fn sort_script_forms_output_types_ignored_index_and_atomic_errors() {
    let mut e = Engine::default();
    e.execute("A=[3 1 2;8 7 9]; [B,I]=sort(A,2); [C,~]=sort(A,2,'descend'); L=logical([1 0 1;0 1 0]); [S,J]=sort(L,2); V=sort([3 1 2]); D=sort([3;1;2]); R=7; K=8;").unwrap();
    assert_eq!(e.variables["B"].data(), &[1., 7., 2., 8., 3., 9.]);
    assert_eq!(e.variables["I"].data(), &[2., 2., 3., 1., 1., 3.]);
    assert_eq!(e.variables["C"].data(), &[3., 9., 2., 8., 1., 7.]);
    assert_eq!(e.logical_variables["S"].data(), &[0, 0, 1, 0, 1, 1]);
    assert_eq!(e.variables["J"].data(), &[2., 1., 1., 3., 3., 2.]);
    assert_eq!((e.variables["V"].rows(), e.variables["V"].cols()), (1, 3));
    assert_eq!((e.variables["D"].rows(), e.variables["D"].cols()), (3, 1));
    e.variables.insert(
        "N".into(),
        Matrix::from_column_slice(1, 3, &[f64::NAN, 2., 1.]).unwrap(),
    );
    e.execute("[Q,P]=sort(N,'MissingPlacement','first');")
        .unwrap();
    assert_eq!(e.variables["P"].data(), &[1., 3., 2.]);
    for bad in [
        "sort()",
        "sort(A,0)",
        "sort(A,1.5)",
        "sort(A,[])",
        "sort(A,logical(1))",
        "sort(A,'abs')",
        "sort(A,'ComparisonMethod','real')",
        "sort(A,'MissingPlacement','wrong')",
        "sort(A,'MissingPlacement','first','MissingPlacement','last')",
    ] {
        assert!(e.execute(&format!("[R,K]={bad};")).is_err(), "{bad}");
        assert_eq!(e.variables["R"].data(), &[7.]);
        assert_eq!(e.variables["K"].data(), &[8.]);
    }
    assert!(e.execute("[R,K,Z]=sort(A);").is_err());
    assert!(matrust::session::FUNCTIONS.contains(&"sort"));
}

#[test]
fn sort_preserves_variable_and_user_function_shadowing() {
    let mut e = Engine::default();
    e.execute("sort=[11 22]; R=sort(2);").unwrap();
    assert_eq!(e.variables["R"].data(), &[22.]);
    assert!(
        e.execute("[B,I]=sort(1);")
            .unwrap_err()
            .contains("Array indexing")
    );
    e.execute("clear; [B,I]=sort(9); function [y,i]=sort(x);y=x+1;i=77;end")
        .unwrap();
    assert_eq!(e.variables["B"].data(), &[10.]);
    assert_eq!(e.variables["I"].data(), &[77.]);
}
