use matrust::Matrix;

#[test]
fn vector_trig_matches_scalar_across_domain_and_quadrant_boundaries() {
    let mut values: Vec<_> = (0..1_000_001)
        .map(|i| -8192. + 16384. * i as f64 / 1_000_000.)
        .collect();
    for quadrant in -5215..=5215 {
        let x = quadrant as f64 * std::f64::consts::FRAC_PI_2;
        values.extend([x.next_down(), x, x.next_up()]);
    }
    let input = Matrix::from_column_vec(values.len(), 1, values).unwrap();
    for (output, reference) in [
        (input.sin(), libm::sin as fn(f64) -> f64),
        (input.cos(), libm::cos),
    ] {
        let mut maximum = 0_f64;
        for (&x, &actual) in input.data().iter().zip(output.data()) {
            let error = (actual - reference(x)).abs();
            maximum = maximum.max(error);
            assert!(
                error <= 1e-15,
                "x={x}, actual={actual}, reference={}, error={error}",
                reference(x)
            );
        }
        eprintln!("maximum absolute error: {maximum:e}");
    }
}

#[test]
fn trig_preserves_special_values_shapes_tails_and_parallel_results() {
    let special = [
        0.,
        -0.,
        f64::from_bits(1),
        -f64::from_bits(1),
        8192.,
        8192_f64.next_up(),
        -8192.,
        (-8192_f64).next_down(),
        1e20,
        -1e100,
        f64::MAX,
        -f64::MAX,
        f64::INFINITY,
        f64::NEG_INFINITY,
        f64::NAN,
    ];
    let input = Matrix::from_column_vec(3, 20, special.repeat(4)).unwrap();
    for (output, scalar) in [
        (input.sin(), libm::sin as fn(f64) -> f64),
        (input.cos(), libm::cos),
    ] {
        assert_eq!((output.rows(), output.cols()), (3, 20));
        for (&x, &actual) in input.data().iter().zip(output.data()) {
            let expected = scalar(x);
            if expected.is_nan() {
                assert!(actual.is_nan());
            } else if x == 0. || x.abs() > 8192. {
                assert_eq!(actual.to_bits(), expected.to_bits());
            } else {
                assert!((actual - expected).abs() <= 1e-15);
            }
        }
    }
    let serial = rayon::ThreadPoolBuilder::new()
        .num_threads(1)
        .build()
        .unwrap();
    let parallel = rayon::ThreadPoolBuilder::new()
        .num_threads(8)
        .build()
        .unwrap();
    for len in [0, 1, 31, 32, 33, 8191, 8192, 8193, 65535, 65536, 65537] {
        let input =
            Matrix::from_column_vec(1, len, (0..len).map(|i| i as f64 * 0.01).collect()).unwrap();
        for cosine in [false, true] {
            let run = || if cosine { input.cos() } else { input.sin() };
            let a = serial.install(run);
            let b = parallel.install(run);
            assert_eq!(a.data(), b.data());
            assert_eq!((a.rows(), a.cols()), (1, len));
            for (&x, &actual) in input.data().iter().zip(a.data()) {
                let expected = if cosine { x.cos() } else { x.sin() };
                assert!((actual - expected).abs() <= 1e-15);
            }
        }
    }
}

#[test]
fn large_angle_reduction_matches_matlab_reference_for_scalar_and_vector_inputs() {
    // Reference sin/cos(1e20); the Windows GNU CRT sine returned 0.9477 here.
    for len in [1, 33, 65536] {
        let input = Matrix::filled(1, len, 1e20).unwrap();
        for &value in input.sin().data() {
            assert!((value - -0.6452512852657808).abs() < 1e-15);
        }
        for &value in input.cos().data() {
            assert!((value - 0.7639704044417283).abs() < 1e-15);
        }
    }
}
