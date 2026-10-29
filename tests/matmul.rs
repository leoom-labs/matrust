use matrust::Matrix;

#[test]
fn matrix_kernels_agree_with_compensated_reference_and_preserve_inputs() {
    let serial = rayon::ThreadPoolBuilder::new()
        .num_threads(1)
        .build()
        .unwrap();
    let parallel = rayon::ThreadPoolBuilder::new()
        .num_threads(8)
        .build()
        .unwrap();
    for (rows, inner, cols) in [
        (16, 16, 16),
        (64, 64, 64),
        (129, 257, 65),
        (257, 129, 511),
        (8192, 32, 64),
        (64, 512, 513),
        (8192, 16, 1),
    ] {
        let a = Matrix::from_column_vec(
            rows,
            inner,
            (0..rows * inner)
                .map(|i| ((i * 17 % 101) as f64 - 50.) / 51.)
                .collect(),
        )
        .unwrap();
        let b = Matrix::from_column_vec(
            inner,
            cols,
            (0..inner * cols)
                .map(|i| ((i * 31 % 97) as f64 - 48.) / 49.)
                .collect(),
        )
        .unwrap();
        let a_snapshot = a.data().to_vec();
        let b_snapshot = b.data().to_vec();
        let alias = a.clone();
        let one = serial.install(|| a.mul(&b)).unwrap();
        let eight = parallel.install(|| a.mul(&b)).unwrap();
        assert_eq!((eight.rows(), eight.cols()), (rows, cols));
        for (&x, &y) in one.data().iter().zip(eight.data()) {
            assert!((x - y).abs() <= 32. * f64::EPSILON * inner as f64);
        }
        for sample in 0..32 {
            let row = sample * 257 % rows;
            let col = sample * 131 % cols;
            let mut sum = 0.;
            let mut compensation = 0.;
            let mut magnitude = 0.;
            for k in 0..inner {
                let term = a.get(row, k) * b.get(k, col);
                magnitude += term.abs();
                let corrected = term - compensation;
                let next = sum + corrected;
                compensation = (next - sum) - corrected;
                sum = next;
            }
            let tolerance = 32. * f64::EPSILON * magnitude.max(1.);
            assert!(
                (eight.get(row, col) - sum).abs() <= tolerance,
                "{rows}x{inner}x{cols}: ({row},{col})"
            );
        }
        assert_eq!(a.data(), a_snapshot);
        assert_eq!(b.data(), b_snapshot);
        assert_eq!(a.data().as_ptr(), alias.data().as_ptr());
        assert_ne!(a.data().as_ptr(), eight.data().as_ptr());
    }
}

#[test]
fn matrix_kernels_preserve_nonfinite_propagation_empty_shapes_and_bounds() {
    let pool = rayon::ThreadPoolBuilder::new()
        .num_threads(8)
        .build()
        .unwrap();
    pool.install(|| {
        let zeros = Matrix::filled(256, 256, 0.).unwrap();
        let infinity = Matrix::filled(256, 256, f64::INFINITY).unwrap();
        assert!(
            zeros
                .mul(&infinity)
                .unwrap()
                .data()
                .iter()
                .all(|x| x.is_nan())
        );
        let identity = Matrix::identity(256, 256).unwrap();
        let contaminated = identity.with_value(3, 7, f64::NAN).unwrap();
        let result = contaminated.mul(&identity).unwrap();
        for row in 0..256 {
            for col in 0..256 {
                if row == 3 {
                    assert!(result.get(row, col).is_nan());
                } else {
                    assert_eq!(result.get(row, col), if row == col { 1. } else { 0. });
                }
            }
        }
        for (rows, inner, cols) in [(0, 17, 3), (3, 0, 7), (3, 17, 0)] {
            let result = Matrix::filled(rows, inner, 1.)
                .unwrap()
                .mul(&Matrix::filled(inner, cols, 2.).unwrap())
                .unwrap();
            assert_eq!((result.rows(), result.cols()), (rows, cols));
            assert!(result.data().iter().all(|&x| x == 0.));
        }
        assert!(
            Matrix::filled(65536, 1, 1.)
                .unwrap()
                .mul(&Matrix::filled(1, 257, 1.).unwrap())
                .is_err()
        );
        assert!(
            Matrix::filled(2, 3, 1.)
                .unwrap()
                .mul(&Matrix::filled(2, 4, 1.).unwrap())
                .is_err()
        );
    });
}
