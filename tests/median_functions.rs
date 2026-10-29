use matrust::{
    Engine, Matrix,
    matrix::{MAX_ELEMENTS, MedianAxis as A},
};
use std::sync::atomic::{AtomicBool, Ordering};
fn input(r: usize, c: usize, x: &[f64]) -> Matrix {
    Matrix::from_column_slice(r, c, x).unwrap()
}
fn bits(x: &[f64]) -> Vec<u64> {
    x.iter().map(|v| v.to_bits()).collect()
}
fn expect(value: Matrix, shape: (usize, usize), data: &[f64]) {
    assert_eq!((value.rows(), value.cols()), shape);
    assert_eq!(value.len(), data.len());
    for (&actual, &expected) in value.data().iter().zip(data) {
        if expected.is_nan() {
            assert!(actual.is_nan());
        } else {
            assert_eq!(actual.to_bits(), expected.to_bits());
        }
    }
}

#[test]
fn median_shapes_default_all_and_missing_policies() {
    let cancel = AtomicBool::new(false);
    let m = input(3, 2, &[1., 4., 2., 8., 3., 6.]);
    expect(
        m.median(A::Default, false, &cancel).unwrap(),
        (1, 2),
        &[2., 6.],
    );
    expect(
        m.median(A::Dimension(2), false, &cancel).unwrap(),
        (3, 1),
        &[4.5, 3.5, 4.],
    );
    expect(m.median(A::All, false, &cancel).unwrap(), (1, 1), &[3.5]);
    let m = input(3, 2, &[1., f64::NAN, 3., f64::NAN, f64::NAN, f64::NAN]);
    expect(
        m.median(A::Default, false, &cancel).unwrap(),
        (1, 2),
        &[f64::NAN, f64::NAN],
    );
    expect(
        m.median(A::Default, true, &cancel).unwrap(),
        (1, 2),
        &[2., f64::NAN],
    );
    expect(m.median(A::All, true, &cancel).unwrap(), (1, 1), &[2.]);
    expect(
        input(1, 4, &[4., 1., 3., 2.])
            .median(A::Default, false, &cancel)
            .unwrap(),
        (1, 1),
        &[2.5],
    );
}

#[test]
fn median_empty_and_identity_dimensions_preserve_shapes_and_shared_storage() {
    let cancel = AtomicBool::new(false);
    for (rows, cols) in [(0, 0), (0, 3), (3, 0), (1, 0), (0, 1)] {
        let m = Matrix::filled(rows, cols, 0.).unwrap();
        let default_shape = if (rows == 0 && cols == 0) || rows == 1 {
            (1, 1)
        } else {
            (1, cols)
        };
        let result = m.median(A::Default, false, &cancel).unwrap();
        assert_eq!((result.rows(), result.cols()), default_shape);
        assert!(result.data().iter().all(|v| v.is_nan()));
        expect(
            m.median(A::All, true, &cancel).unwrap(),
            (1, 1),
            &[f64::NAN],
        );
        for (axis, shape) in [(A::Dimension(1), (1, cols)), (A::Dimension(2), (rows, 1))] {
            let result = m.median(axis, true, &cancel).unwrap();
            assert_eq!((result.rows(), result.cols()), shape);
            assert!(result.data().iter().all(|v| v.is_nan()));
        }
        let shared = m.median(A::Dimension(3), true, &cancel).unwrap();
        assert_eq!((shared.rows(), shared.cols()), (rows, cols));
        assert_eq!(shared.data().as_ptr(), m.data().as_ptr());
    }
    let m = input(
        1,
        3,
        &[-0., f64::from_bits(0xfff8_0000_0000_0011), f64::INFINITY],
    );
    for axis in [A::Dimension(1), A::Dimension(3), A::Dimension(MAX_ELEMENTS)] {
        let shared = m.median(axis, true, &cancel).unwrap();
        assert_eq!(shared.data().as_ptr(), m.data().as_ptr());
        assert_eq!(bits(shared.data()), bits(m.data()));
    }
}

#[test]
fn median_even_midpoints_handle_extremes_subnormals_and_signed_zero() {
    let cancel = AtomicBool::new(false);
    let tiny = f64::from_bits(1);
    for (a, b, expected) in [
        (f64::MAX, f64::MAX, f64::MAX),
        (-f64::MAX, f64::MAX, 0.),
        (f64::MAX.next_down(), f64::MAX, f64::MAX.next_down()),
        (tiny, tiny, tiny),
        (0., tiny, 0.),
        (tiny, 2. * tiny, 2. * tiny),
        (-tiny, 0., -0.),
        (-0., -0., -0.),
        (-0., 0., 0.),
        (f64::NEG_INFINITY, f64::NEG_INFINITY, f64::NEG_INFINITY),
        (f64::INFINITY, 2., f64::INFINITY),
        (f64::NEG_INFINITY, f64::INFINITY, f64::NAN),
    ] {
        expect(
            input(1, 2, &[a, b]).median(A::All, false, &cancel).unwrap(),
            (1, 1),
            &[expected],
        );
    }
    expect(
        input(1, 3, &[0., -0., -0.])
            .median(A::All, false, &cancel)
            .unwrap(),
        (1, 1),
        &[-0.],
    );
}

#[test]
fn median_radix_and_selection_boundaries_match_independent_full_sort_and_preserve_input() {
    let cancel = AtomicBool::new(false);
    for length in [8191, 8192, 8193, 16384, 16385] {
        for missing in [false, true] {
            let data: Vec<_> = (0..length)
                .map(|i| match i % 31 {
                    0 => f64::NEG_INFINITY,
                    1 => f64::INFINITY,
                    2 => -0.,
                    3 => 0.,
                    4 if missing => f64::NAN,
                    _ => ((i * 173) % 4093) as f64 - 2046.,
                })
                .collect();
            let m = Matrix::from_column_vec(length, 1, data.clone()).unwrap();
            let pointer = m.data().as_ptr();
            let mut ordered: Vec<_> = data.iter().copied().filter(|v| !v.is_nan()).collect();
            ordered.sort_by(f64::total_cmp);
            let n = ordered.len();
            // Central values in this fixture are small integral/half-integral;
            // direct addition is an independent exact midpoint reference here.
            let reference = if n.is_multiple_of(2) {
                (ordered[n / 2 - 1] + ordered[n / 2]) / 2.
            } else {
                ordered[n / 2]
            };
            expect(
                m.median(A::Default, true, &cancel).unwrap(),
                (1, 1),
                &[reference],
            );
            if missing {
                assert!(m.median(A::All, false, &cancel).unwrap().data()[0].is_nan());
            }
            assert_eq!(m.data().as_ptr(), pointer);
            assert_eq!(bits(m.data()), bits(&data));
        }
    }
    let values = [
        f64::from_bits(1),
        f64::MAX,
        f64::NEG_INFINITY,
        f64::INFINITY,
        -0.,
    ];
    for value in values {
        let m = Matrix::filled(8193, 1, value).unwrap();
        expect(m.median(A::All, false, &cancel).unwrap(), (1, 1), &[value]);
    }
}

#[test]
fn median_one_eight_worker_parity_crosses_parallel_threshold_in_both_layouts() {
    let pools = [1, 8].map(|n| {
        rayon::ThreadPoolBuilder::new()
            .num_threads(n)
            .build()
            .unwrap()
    });
    let cancel = AtomicBool::new(false);
    for (rows, cols, axis) in [
        (8192, 65, A::Dimension(1)),
        (8193, 65, A::Dimension(1)),
        (65, 8193, A::Dimension(2)),
    ] {
        assert!(rows * cols > 512 * 1024);
        let m = Matrix::from_column_vec(
            rows,
            cols,
            (0..rows * cols)
                .map(|i| match i % 47 {
                    0 => f64::NAN,
                    1 => -0.,
                    2 => f64::INFINITY,
                    3 => f64::NEG_INFINITY,
                    _ => ((i * 7) % 127) as f64,
                })
                .collect(),
        )
        .unwrap();
        for omit in [false, true] {
            let a = pools[0].install(|| m.median(axis, omit, &cancel)).unwrap();
            let b = pools[1].install(|| m.median(axis, omit, &cancel)).unwrap();
            assert_eq!((a.rows(), a.cols()), (b.rows(), b.cols()));
            assert_eq!(bits(a.data()), bits(b.data()));
        }
    }
}

#[test]
fn median_script_options_rejections_and_precancel_are_atomic() {
    let mut e = Engine::default();
    e.execute("A=[1 NaN 3; 4 2 6]; M=median(A,2,'omitmissing'); S=median(A,'all','omitnan');")
        .unwrap();
    expect(e.variables["M"].clone(), (2, 1), &[2., 4.]);
    expect(e.variables["S"].clone(), (1, 1), &[3.]);
    e.execute("M=99;").unwrap();
    for expression in [
        "median(A,0)",
        "median(A,-1)",
        "median(A,1.5)",
        "median(A,NaN)",
        "median(A,Inf)",
        "median(A,[])",
        "median(A,[1 2])",
        "median(A,'native')",
        "median(A,'omitnan',2)",
        "median(A,'Weights',[1 1])",
        "median(A,'omitnan','omitnan')",
        "median(A>0)",
    ] {
        assert!(
            e.execute(&format!("M={expression};")).is_err(),
            "{expression}"
        );
        expect(e.variables["M"].clone(), (1, 1), &[99.]);
    }
    assert!(e.execute("[X,Y]=median(A);").is_err());
    let m = input(1, 3, &[3., 1., 2.]);
    let cancel = AtomicBool::new(true);
    assert!(
        m.median(A::All, true, &cancel)
            .unwrap_err()
            .contains("cancelled")
    );
    cancel.store(false, Ordering::Relaxed);
    for axis in [A::Dimension(0), A::Dimension(MAX_ELEMENTS + 1)] {
        assert!(m.median(axis, true, &cancel).is_err());
    }
}
