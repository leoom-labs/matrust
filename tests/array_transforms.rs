use matrust::{Engine, Matrix, matrix::MAX_ELEMENTS};
use std::sync::atomic::AtomicBool;

#[test]
fn vector_transform_specializations_cover_offsets_and_chunk_boundaries() {
    let cancel = AtomicBool::new(false);
    let data: Vec<_> = (0..8197)
        .map(|i| if i == 8192 { -0. } else { i as f64 + 1. })
        .collect();
    for (rows, cols) in [(1, data.len()), (data.len(), 1)] {
        let a = Matrix::from_column_slice(rows, cols, &data).unwrap();
        let reversed = a
            .flip_dimension(if rows == 1 { 2 } else { 1 }, &cancel)
            .unwrap();
        for (&actual, &expected) in reversed.data().iter().zip(data.iter().rev()) {
            assert_eq!(actual.to_bits(), expected.to_bits());
        }
        for k in [-9000, -8192, -1, 0, 1, 8192, 9000] {
            for upper in [false, true] {
                let result = a.triangular(k, upper, &cancel).unwrap();
                for (i, &actual) in result.data().iter().enumerate() {
                    let diagonal = if rows == 1 { i as i64 } else { -(i as i64) };
                    let keep = if upper { diagonal >= k } else { diagonal <= k };
                    let expected: f64 = if keep { data[i] } else { 0. };
                    assert_eq!(
                        actual.to_bits(),
                        expected.to_bits(),
                        "{rows}x{cols} k={k} upper={upper} index={i}"
                    );
                }
            }
        }
    }
}

fn flag() -> AtomicBool {
    AtomicBool::new(false)
}
fn matrix(rows: usize, cols: usize, data: &[f64]) -> Matrix {
    Matrix::from_column_slice(rows, cols, data).unwrap()
}
fn expect(actual: &Matrix, shape: (usize, usize), expected: &[f64]) {
    assert_eq!((actual.rows(), actual.cols()), shape);
    assert_eq!(actual.len(), expected.len());
    for (&a, &b) in actual.data().iter().zip(expected) {
        if b.is_nan() {
            assert!(a.is_nan());
        } else {
            assert_eq!(a.to_bits(), b.to_bits());
        }
    }
}

#[test]
fn diagonal_construction_extraction_offsets_and_empty_shapes() {
    let c = flag();
    let row = matrix(1, 3, &[2., -0., 5.]);
    expect(
        &row.diagonal(0, &c).unwrap(),
        (3, 3),
        &[2., 0., 0., 0., -0., 0., 0., 0., 5.],
    );
    let upper = row.diagonal(1, &c).unwrap();
    expect(
        &upper,
        (4, 4),
        &[
            0., 0., 0., 0., 2., 0., 0., 0., 0., -0., 0., 0., 0., 0., 5., 0.,
        ],
    );
    expect(&upper.diagonal(1, &c).unwrap(), (3, 1), row.data());
    let column = row.reshape(3, 1).unwrap();
    let lower = column.diagonal(-2, &c).unwrap();
    expect(&lower.diagonal(-2, &c).unwrap(), (3, 1), row.data());
    let rect = matrix(2, 3, &[1., 2., 3., 4., 5., 6.]);
    expect(&rect.diagonal(0, &c).unwrap(), (2, 1), &[1., 4.]);
    expect(&rect.diagonal(1, &c).unwrap(), (2, 1), &[3., 6.]);
    expect(&rect.diagonal(-1, &c).unwrap(), (1, 1), &[2.]);
    expect(&rect.diagonal(3, &c).unwrap(), (0, 1), &[]);
    expect(&rect.diagonal(-2, &c).unwrap(), (0, 1), &[]);
    expect(&matrix(0, 0, &[]).diagonal(0, &c).unwrap(), (0, 0), &[]);
    expect(&matrix(0, 3, &[]).diagonal(0, &c).unwrap(), (0, 1), &[]);
    expect(&matrix(1, 0, &[]).diagonal(0, &c).unwrap(), (0, 0), &[]);
    assert!(matrix(0, 0, &[]).diagonal(1, &c).is_err());
}

#[test]
fn triangular_masks_rectangles_including_nonfinite_values() {
    let a = matrix(3, 2, &[1., 2., 3., 4., 5., 6.]);
    let c = flag();
    for value in [3., -0., f64::NAN] {
        let scalar = Matrix::scalar(value);
        for k in [-1, 0, 1] {
            for upper in [false, true] {
                let keep = if upper { k <= 0 } else { k >= 0 };
                expect(
                    &scalar.triangular(k, upper, &c).unwrap(),
                    (1, 1),
                    &[if keep { value } else { 0. }],
                );
            }
        }
    }
    expect(
        &a.triangular(0, false, &c).unwrap(),
        (3, 2),
        &[1., 2., 3., 0., 5., 6.],
    );
    expect(
        &a.triangular(-1, false, &c).unwrap(),
        (3, 2),
        &[0., 2., 3., 0., 0., 6.],
    );
    expect(
        &a.triangular(0, true, &c).unwrap(),
        (3, 2),
        &[1., 0., 0., 4., 5., 0.],
    );
    expect(
        &a.triangular(1, true, &c).unwrap(),
        (3, 2),
        &[0., 0., 0., 4., 0., 0.],
    );
    expect(&a.triangular(2, true, &c).unwrap(), (3, 2), &[0.; 6]);
    expect(&a.triangular(-3, false, &c).unwrap(), (3, 2), &[0.; 6]);
    let special = matrix(2, 2, &[-0., f64::NAN, f64::INFINITY, -f64::INFINITY]);
    expect(
        &special.triangular(0, true, &c).unwrap(),
        (2, 2),
        &[-0., 0., f64::INFINITY, -f64::INFINITY],
    );
    expect(
        &special.triangular(0, false, &c).unwrap(),
        (2, 2),
        &[-0., f64::NAN, 0., -f64::INFINITY],
    );
    for shape in [(0, 0), (0, 3), (3, 0)] {
        let empty = matrix(shape.0, shape.1, &[]);
        expect(&empty.triangular(0, true, &c).unwrap(), shape, &[]);
    }
}

#[test]
fn flips_preserve_shapes_bits_and_reshaped_column_major_storage() {
    let c = flag();
    let a = matrix(2, 3, &[1., 2., 3., 4., 5., 6.]);
    expect(
        &a.flip_dimension(1, &c).unwrap(),
        (2, 3),
        &[2., 1., 4., 3., 6., 5.],
    );
    expect(
        &a.flip_dimension(2, &c).unwrap(),
        (2, 3),
        &[5., 6., 3., 4., 1., 2.],
    );
    let b = a.reshape(3, 2).unwrap();
    expect(
        &b.flip_dimension(1, &c).unwrap(),
        (3, 2),
        &[3., 2., 1., 6., 5., 4.],
    );
    let special = matrix(1, 4, &[-0., f64::NAN, f64::INFINITY, 3.]);
    expect(
        &special.flip_dimension(2, &c).unwrap(),
        (1, 4),
        &[3., f64::INFINITY, f64::NAN, -0.],
    );
    for shape in [(0, 0), (0, 3), (3, 0), (1, 0)] {
        expect(
            &matrix(shape.0, shape.1, &[]).flip_dimension(1, &c).unwrap(),
            shape,
            &[],
        );
    }
    assert_eq!(a.first_nonsingleton_dimension(), 1);
    assert_eq!(special.first_nonsingleton_dimension(), 2);
    assert_eq!(matrix(1, 1, &[1.]).first_nonsingleton_dimension(), 1);
}

#[test]
fn cumulative_sum_and_product_have_exact_lane_order_and_orientation() {
    let c = flag();
    let a = matrix(3, 2, &[1., 2., 3., 4., 5., 6.]);
    expect(
        &a.cumulative(1, false, &c).unwrap(),
        (3, 2),
        &[1., 3., 6., 4., 9., 15.],
    );
    expect(
        &a.cumulative(2, false, &c).unwrap(),
        (3, 2),
        &[1., 2., 3., 5., 7., 9.],
    );
    expect(
        &a.cumulative(1, true, &c).unwrap(),
        (3, 2),
        &[1., 2., 6., 4., 20., 120.],
    );
    expect(
        &a.cumulative(2, true, &c).unwrap(),
        (3, 2),
        &[1., 2., 3., 4., 10., 18.],
    );
    let ordered = matrix(1, 5, &[1e16, 1., -1e16, 2., -2.]);
    expect(
        &ordered.cumulative(2, false, &c).unwrap(),
        (1, 5),
        &[1e16, 1e16, 0., 2., 0.],
    );
    let special = matrix(1, 5, &[-0., f64::INFINITY, -f64::INFINITY, 2., 3.]);
    expect(
        &special.cumulative(2, false, &c).unwrap(),
        (1, 5),
        &[-0., f64::INFINITY, f64::NAN, f64::NAN, f64::NAN],
    );
    let product = matrix(1, 5, &[-0., -2., 2., f64::INFINITY, 3.]);
    expect(
        &product.cumulative(2, true, &c).unwrap(),
        (1, 5),
        &[-0., 0., 0., f64::NAN, f64::NAN],
    );
    for shape in [(0, 0), (0, 3), (3, 0)] {
        for dim in [1, 2, 3] {
            expect(
                &matrix(shape.0, shape.1, &[])
                    .cumulative(dim, false, &c)
                    .unwrap(),
                shape,
                &[],
            );
        }
    }
}

#[test]
fn no_op_transforms_share_storage_without_mutating_aliases() {
    let c = flag();
    let a = matrix(2, 3, &[1., 2., 3., 4., 5., 6.]);
    for result in [
        a.flip_dimension(3, &c),
        a.triangular(-2, true, &c),
        a.triangular(2, false, &c),
        a.cumulative(3, false, &c),
    ] {
        assert_eq!(result.unwrap().data().as_ptr(), a.data().as_ptr());
    }
    let row = a.reshape(1, 6).unwrap();
    for result in [row.flip_dimension(1, &c), row.cumulative(1, false, &c)] {
        assert_eq!(result.unwrap().data().as_ptr(), a.data().as_ptr());
    }
    let changed = a.flip_dimension(2, &c).unwrap();
    assert_ne!(changed.data().as_ptr(), a.data().as_ptr());
    expect(&a, (2, 3), &[1., 2., 3., 4., 5., 6.]);
}

#[test]
fn transforms_validate_resource_bounds_and_precancelled_noops() {
    let a = matrix(2, 2, &[1., 2., 3., 4.]);
    let c = flag();
    for dim in [0, MAX_ELEMENTS + 1, usize::MAX] {
        assert!(a.flip_dimension(dim, &c).is_err());
        assert!(a.cumulative(dim, false, &c).is_err());
    }
    for k in [i64::MIN, i64::MAX, MAX_ELEMENTS as i64 + 1] {
        assert!(a.diagonal(k, &c).is_err());
        assert!(a.triangular(k, true, &c).is_err());
    }
    assert!(matrix(1, 4097, &vec![1.; 4097]).diagonal(0, &c).is_err());
    assert!(
        Matrix::scalar(1.)
            .diagonal(MAX_ELEMENTS as i64, &c)
            .is_err()
    );
    let cancelled = AtomicBool::new(true);
    for result in [
        a.flip_dimension(3, &cancelled),
        a.cumulative(3, true, &cancelled),
        a.diagonal(0, &cancelled),
        a.triangular(-2, true, &cancelled),
    ] {
        assert!(result.unwrap_err().contains("cancelled"));
    }
}

#[test]
fn transforms_match_independent_coordinates_across_one_and_eight_workers() {
    let data: Vec<_> = (0..(1025 * 513))
        .map(|i| 1. + (i % 23) as f64 / 100000.)
        .collect();
    let a = matrix(1025, 513, &data);
    let one = rayon::ThreadPoolBuilder::new()
        .num_threads(1)
        .build()
        .unwrap();
    let many = rayon::ThreadPoolBuilder::new()
        .num_threads(8)
        .build()
        .unwrap();
    let c = flag();
    for dim in [1, 2] {
        let left = one.install(|| a.flip_dimension(dim, &c).unwrap());
        let right = many.install(|| a.flip_dimension(dim, &c).unwrap());
        expect(&right, (a.rows(), a.cols()), left.data());
        for col in 0..a.cols() {
            for row in 0..a.rows() {
                let expected = if dim == 1 {
                    a.get(a.rows() - 1 - row, col)
                } else {
                    a.get(row, a.cols() - 1 - col)
                };
                assert_eq!(right.get(row, col), expected);
            }
        }
        for product in [false, true] {
            let left = one.install(|| a.cumulative(dim, product, &c).unwrap());
            let right = many.install(|| a.cumulative(dim, product, &c).unwrap());
            expect(&right, (a.rows(), a.cols()), left.data());
            let (lanes, length) = if dim == 1 {
                (a.cols(), a.rows())
            } else {
                (a.rows(), a.cols())
            };
            for lane in 0..lanes {
                let index = |i| {
                    if dim == 1 {
                        lane * a.rows() + i
                    } else {
                        i * a.rows() + lane
                    }
                };
                let mut expected = a.data()[index(0)];
                for i in 0..length {
                    if i > 0 {
                        expected = if product {
                            expected * a.data()[index(i)]
                        } else {
                            expected + a.data()[index(i)]
                        };
                    }
                    assert_eq!(right.data()[index(i)].to_bits(), expected.to_bits());
                }
            }
        }
    }
    for upper in [false, true] {
        for k in [-7, 0, 5] {
            let left = one.install(|| a.triangular(k, upper, &c).unwrap());
            let right = many.install(|| a.triangular(k, upper, &c).unwrap());
            expect(&right, (a.rows(), a.cols()), left.data());
            for col in 0..a.cols() {
                for row in 0..a.rows() {
                    let d = col as i64 - row as i64;
                    let expected = if if upper { d >= k } else { d <= k } {
                        a.get(row, col)
                    } else {
                        0.
                    };
                    assert_eq!(right.get(row, col), expected);
                }
            }
        }
    }
}

#[test]
fn array_function_scripts_dispatch_defaults_dimensions_offsets_and_shadowing() {
    let mut e = Engine::default();
    e.execute("A=[1 4;2 5;3 6];D=diag([2 3],1);L=tril(A,-1);U=triu(A);F=flip(A);R=fliplr(A);V=flipud([1 2 3]);S=cumsum(A);P=cumprod(A,2);Q=cumsum([1 2 3]);H=flip(A,3);").unwrap();
    expect(
        &e.variables["D"],
        (3, 3),
        &[0., 0., 0., 2., 0., 0., 0., 3., 0.],
    );
    expect(&e.variables["L"], (3, 2), &[0., 2., 3., 0., 0., 6.]);
    expect(&e.variables["U"], (3, 2), &[1., 0., 0., 4., 5., 0.]);
    expect(&e.variables["F"], (3, 2), &[3., 2., 1., 6., 5., 4.]);
    expect(&e.variables["R"], (3, 2), &[4., 5., 6., 1., 2., 3.]);
    expect(&e.variables["V"], (1, 3), &[1., 2., 3.]);
    expect(&e.variables["S"], (3, 2), &[1., 3., 6., 4., 9., 15.]);
    expect(&e.variables["P"], (3, 2), &[1., 2., 3., 4., 10., 18.]);
    expect(&e.variables["Q"], (1, 3), &[1., 3., 6.]);
    assert_eq!(
        e.variables["H"].data().as_ptr(),
        e.variables["A"].data().as_ptr()
    );
    e.execute("flip=[8 9];Z=flip(2);").unwrap();
    expect(&e.variables["Z"], (1, 1), &[9.]);
}

#[test]
fn array_function_scripts_reject_unsupported_options_without_changing_inputs() {
    let mut e = Engine::default();
    e.execute("A=[1 2;3 4];R=42;").unwrap();
    let original = e.variables["A"].data().as_ptr();
    for statement in [
        "R=diag(A,0.5);",
        "R=diag(A,Inf);",
        "R=tril(A,-Inf);",
        "R=flip(A,0);",
        "R=flip(A,-1);",
        "R=cumsum(A,1.5);",
        "R=cumprod(A,'reverse');",
        "R=cumsum(A,1,'omitnan');",
        "R=flipud(A,2);",
        "R=triu(A,0,1);",
        "R=diag(logical(A));",
        "R=flip(logical(A));",
        "R=cumsum(logical(A));",
    ] {
        assert!(e.execute(statement).is_err(), "{statement}");
        expect(&e.variables["R"], (1, 1), &[42.]);
        assert_eq!(e.variables["A"].data().as_ptr(), original);
    }
}
