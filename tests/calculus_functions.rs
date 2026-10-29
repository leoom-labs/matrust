use matrust::{Engine, Matrix, matrix::MAX_ELEMENTS};
use std::sync::atomic::{AtomicBool, Ordering};

fn matrix(rows: usize, cols: usize, data: &[f64]) -> Matrix {
    Matrix::from_column_slice(rows, cols, data).unwrap()
}
fn expect(a: &Matrix, shape: (usize, usize), values: &[f64]) {
    assert_eq!((a.rows(), a.cols()), shape);
    assert_eq!(a.len(), values.len());
    for (&a, &b) in a.data().iter().zip(values) {
        if b.is_nan() {
            assert!(a.is_nan());
        } else {
            assert_eq!(a.to_bits(), b.to_bits());
        }
    }
}

#[test]
fn differences_use_current_r2025a_fixed_axis_and_column_major_shapes() {
    let cancel = AtomicBool::new(false);
    let a = matrix(3, 3, &[1., 7., 17., 3., 11., 19., 5., 13., 23.]);
    expect(
        &a.difference(1, 1, &cancel).unwrap(),
        (2, 3),
        &[6., 10., 8., 8., 8., 10.],
    );
    expect(&a.difference(2, 1, &cancel).unwrap(), (1, 3), &[4., 0., 2.]);
    expect(
        &a.difference(1, 2, &cancel).unwrap(),
        (3, 2),
        &[2., 4., 2., 2., 2., 4.],
    );
    expect(
        &a.difference(2, 2, &cancel).unwrap(),
        (3, 1),
        &[0., -2., 2.],
    );
    expect(&a.difference(3, 1, &cancel).unwrap(), (0, 3), &[]);
    expect(
        &a.difference(MAX_ELEMENTS, 2, &cancel).unwrap(),
        (3, 0),
        &[],
    );
    let mut e = Engine::default();
    e.execute("A=[1 1 1;5 5 5]; B=diff(A,2); C=diff(A,[],2); R=diff([0 5 15 30 50 75 105],2);")
        .unwrap();
    expect(&e.variables["B"], (0, 3), &[]); // Reference default spills axes; this intentionally does not.
    expect(&e.variables["C"], (2, 2), &[0.; 4]);
    expect(&e.variables["R"], (1, 5), &[5.; 5]);
    assert!(matrust::engine::HELP.contains("R2025a+"));
    for name in ["diff", "trapz", "cumtrapz"] {
        assert!(matrust::session::FUNCTIONS.contains(&name));
    }
}

#[test]
fn calculus_empty_and_singleton_shapes_are_explicit() {
    let c = AtomicBool::new(false);
    for (r, col) in [(0, 0), (0, 3), (3, 0), (1, 0), (0, 1), (1, 1)] {
        let a = Matrix::filled(r, col, f64::NAN).unwrap();
        for dim in [1, 2] {
            let expected = if dim == 1 {
                (r.saturating_sub(1), col)
            } else {
                (r, col.saturating_sub(1))
            };
            let result = a.difference(1, dim, &c).unwrap();
            assert_eq!((result.rows(), result.cols()), expected);
            assert_eq!(result.len(), 0);
            let reduced = if dim == 1 { (1, col) } else { (r, 1) };
            let result = a.trapezoid(None, Some(dim), false, &c).unwrap();
            expect(&result, reduced, &vec![0.; reduced.0 * reduced.1]);
            let result = a.trapezoid(None, Some(dim), true, &c).unwrap();
            expect(&result, (r, col), &vec![0.; r * col]);
        }
        for cum in [false, true] {
            let result = a.trapezoid(None, Some(MAX_ELEMENTS), cum, &c).unwrap();
            expect(&result, (r, col), &vec![0.; r * col]);
        }
    }
    let empty = matrix(0, 0, &[]);
    expect(
        &empty.trapezoid(None, None, false, &c).unwrap(),
        (1, 1),
        &[0.],
    );
    expect(&empty.trapezoid(None, None, true, &c).unwrap(), (0, 0), &[]);
    // Empty coordinate arrays are not admitted by this bounded vector subset.
    // This rejection is explicit even when Y's selected integration axis is empty.
    for cumulative in [false, true] {
        assert!(
            empty
                .trapezoid(Some(&empty), Some(1), cumulative, &c)
                .is_err()
        );
        assert!(
            matrix(0, 2, &[])
                .trapezoid(Some(&empty), Some(1), cumulative, &c)
                .is_err()
        );
    }
    expect(
        &matrix(1, 3, &[2., 4., 8.])
            .trapezoid(None, None, true, &c)
            .unwrap(),
        (1, 3),
        &[0., 3., 9.],
    );
}

#[test]
fn trapezoids_cover_both_axes_spacing_direction_and_scalar_ambiguity() {
    let c = AtomicBool::new(false);
    let a = matrix(3, 2, &[1., 4., 9., 2., 8., 18.]);
    expect(
        &a.trapezoid(None, None, false, &c).unwrap(),
        (1, 2),
        &[9., 18.],
    );
    expect(
        &a.trapezoid(None, None, true, &c).unwrap(),
        (3, 2),
        &[0., 2.5, 9., 0., 5., 18.],
    );
    expect(
        &a.trapezoid(None, Some(2), false, &c).unwrap(),
        (3, 1),
        &[1.5, 6., 13.5],
    );
    expect(
        &a.trapezoid(None, Some(2), true, &c).unwrap(),
        (3, 2),
        &[0., 0., 0., 1.5, 6., 13.5],
    );
    for x in [matrix(1, 3, &[3., 1., 1.]), matrix(3, 1, &[3., 1., 1.])] {
        expect(
            &a.trapezoid(Some(&x), None, false, &c).unwrap(),
            (1, 2),
            &[-5., -10.],
        );
        expect(
            &a.trapezoid(Some(&x), None, true, &c).unwrap(),
            (3, 2),
            &[0., -5., -5., 0., -10., -10.],
        );
    }
    let mut e = Engine::default();
    e.execute("A=[1 2;4 8;9 18]; T=trapz(0.5,A); C=cumtrapz(0.5,A); Z=trapz(2,3); S=trapz(2,3,1); N=cumtrapz(-2,[1 4 9]);").unwrap();
    expect(&e.variables["T"], (1, 2), &[4.5, 9.]);
    expect(&e.variables["C"], (3, 2), &[0., 1.25, 4.5, 0., 2.5, 9.]);
    expect(&e.variables["Z"], (1, 1), &[0.]);
    expect(&e.variables["S"], (1, 1), &[0.]);
    expect(&e.variables["N"], (1, 3), &[-0., -5., -18.]);
}

#[test]
fn calculus_special_values_and_finite_half_sum_overflow_are_defined() {
    let c = AtomicBool::new(false);
    for (rows, cols, axis) in [(1, 2, 2), (2, 1, 1), (2, 2, 2)] {
        let zero = Matrix::filled(rows, cols, -0.).unwrap();
        let result = zero.trapezoid(None, Some(axis), true, &c).unwrap();
        let first_interval = if axis == 1 { 1 } else { rows };
        assert_eq!(result.data()[0].to_bits(), 0f64.to_bits());
        assert_eq!(result.data()[first_interval].to_bits(), (-0f64).to_bits());
    }
    let a = matrix(1, 4, &[1., f64::INFINITY, f64::INFINITY, f64::NAN]);
    expect(
        &a.difference(1, 2, &c).unwrap(),
        (1, 3),
        &[f64::INFINITY, f64::NAN, f64::NAN],
    );
    expect(
        &a.trapezoid(None, None, true, &c).unwrap(),
        (1, 4),
        &[0., f64::INFINITY, f64::INFINITY, f64::NAN],
    );
    let a = matrix(1, 2, &[f64::MAX, f64::MAX]);
    expect(
        &a.trapezoid(None, None, false, &c).unwrap(),
        (1, 1),
        &[f64::MAX],
    );
    let coordinates = matrix(1, 2, &[2., 2.]);
    expect(
        &matrix(1, 2, &[f64::INFINITY, 1.])
            .trapezoid(Some(&coordinates), None, false, &c)
            .unwrap(),
        (1, 1),
        &[f64::NAN],
    );
    let tiny = f64::from_bits(1);
    // Coordinate weighting preserves tiny representable areas in every loop shape.
    for (rows, cols, axis, data) in [
        (2, 1, 1, vec![tiny, 0.]),
        (1, 2, 2, vec![tiny, 0.]),
        (2, 2, 2, vec![tiny, -tiny, 0., -0.]),
    ] {
        let a = matrix(rows, cols, &data);
        for (end, sign) in [(2., 1.), (-2., -1.)] {
            let x = matrix(1, 2, &[0., end]);
            let reduced = a.trapezoid(Some(&x), Some(axis), false, &c).unwrap();
            let cumulative = a.trapezoid(Some(&x), Some(axis), true, &c).unwrap();
            assert_eq!(reduced.data()[0].to_bits(), (sign * tiny).to_bits());
            let first = if axis == 1 { 1 } else { rows };
            assert_eq!(cumulative.data()[first].to_bits(), (sign * tiny).to_bits());
            if rows == 2 && axis == 2 {
                assert_eq!(reduced.data()[1].to_bits(), (-sign * tiny).to_bits());
                assert_eq!(
                    cumulative.data()[first + 1].to_bits(),
                    (-sign * tiny).to_bits()
                );
            }
        }
    }
    // Scalar spacing deliberately scales completed unit-spacing sums.
    let tiny_pair = matrix(1, 2, &[tiny, 0.]);
    expect(
        &tiny_pair
            .trapezoid(Some(&Matrix::scalar(2.)), Some(2), false, &c)
            .unwrap(),
        (1, 1),
        &[0.],
    );
    let nan_spacing = Matrix::scalar(f64::NAN);
    expect(
        &Matrix::scalar(3.)
            .trapezoid(Some(&nan_spacing), Some(1), true, &c)
            .unwrap(),
        (1, 1),
        &[f64::NAN],
    );
}

#[test]
fn calculus_bounds_cancellation_and_failed_assignments_preserve_inputs() {
    let cancel = AtomicBool::new(false);
    let a = Matrix::filled(32768, 2, 1.).unwrap();
    let pointer = a.data().as_ptr();
    assert!(
        a.difference(16384, 1, &cancel)
            .unwrap_err()
            .contains("work limit")
    );
    expect(
        &a.difference(MAX_ELEMENTS, 1, &cancel).unwrap(),
        (0, 2),
        &[],
    );
    for (n, dim) in [(0, 1), (MAX_ELEMENTS + 1, 1), (1, 0), (1, 3)] {
        assert!(a.difference(n, dim, &cancel).is_err());
    }
    for dim in [0, MAX_ELEMENTS + 1] {
        assert!(a.trapezoid(None, Some(dim), false, &cancel).is_err());
    }
    for x in [
        matrix(2, 2, &[0.; 4]),
        matrix(1, 2, &[0., 1.]),
        matrix(0, 0, &[]),
    ] {
        assert!(a.trapezoid(Some(&x), Some(1), false, &cancel).is_err());
    }
    cancel.store(true, Ordering::Relaxed);
    assert!(a.difference(MAX_ELEMENTS, 1, &cancel).is_err());
    assert!(a.trapezoid(None, Some(3), true, &cancel).is_err());
    assert_eq!(pointer, a.data().as_ptr());
    let mut e = Engine::default();
    e.execute("A=[1 2;3 4];keep=A;").unwrap();
    let before = e.variables["keep"].data().as_ptr();
    for source in [
        "keep=diff(A,0);",
        "keep=diff(A,1,3);",
        "keep=trapz([1 2 3],A);",
        "keep=cumtrapz(A,0);",
        "keep=trapz('bad',A);",
        "keep=diff(true);",
    ] {
        assert!(e.execute(source).is_err(), "{source}");
        assert_eq!(e.variables["keep"].data().as_ptr(), before);
    }
}

#[test]
fn calculus_worker_counts_keep_bitwise_results_across_parallel_shapes() {
    let one = rayon::ThreadPoolBuilder::new()
        .num_threads(1)
        .build()
        .unwrap();
    let many = rayon::ThreadPoolBuilder::new()
        .num_threads(4)
        .build()
        .unwrap();
    let c = AtomicBool::new(false);
    for (rows, cols) in [(1024, 513), (524289, 2), (3, 174763), (1, 524289)] {
        let data = (0..rows * cols)
            .map(|i| ((i % 101) as f64 - 50.) / 7.)
            .collect();
        let a = Matrix::from_column_vec(rows, cols, data).unwrap();
        for dim in [1, 2] {
            let expected = one.install(|| a.difference(2, dim, &c).unwrap());
            let actual = many.install(|| a.difference(2, dim, &c).unwrap());
            expect(&actual, (expected.rows(), expected.cols()), expected.data());
            for cumulative in [false, true] {
                let expected =
                    one.install(|| a.trapezoid(None, Some(dim), cumulative, &c).unwrap());
                let actual = many.install(|| a.trapezoid(None, Some(dim), cumulative, &c).unwrap());
                expect(&actual, (expected.rows(), expected.cols()), expected.data());
            }
        }
    }
}
