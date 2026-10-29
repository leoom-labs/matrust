use matrust::{Engine, Matrix};

#[test]
fn randomized_inputs_do_not_panic() {
    let alphabet = b"abc0123456789<>=~&|[]():;,'+-*/^.% \n";
    let mut state = 0x12345678_u64;
    for len in 0..96 {
        for _ in 0..30 {
            let input: String = (0..len)
                .map(|_| {
                    state = state.wrapping_mul(6364136223846793005).wrapping_add(1);
                    alphabet[(state >> 32) as usize % alphabet.len()] as char
                })
                .collect();
            // Arbitrary identifiers are undefined, so generated expressions cannot
            // invoke large allocation functions. The property under test is parsing.
            let _ = Engine::default().execute(&input);
        }
    }
}

#[test]
fn products_match_independent_reference_for_rectangular_shapes() {
    for (m, k, n) in [(1, 4, 3), (3, 5, 7), (7, 3, 2), (5, 0, 2), (0, 4, 3)] {
        let a = Matrix::from_column_slice(
            m,
            k,
            &(0..m * k)
                .map(|i| (i as f64 - 5.0) / 7.0)
                .collect::<Vec<_>>(),
        )
        .unwrap();
        let b = Matrix::from_column_slice(
            k,
            n,
            &(0..k * n)
                .map(|i| (i as f64 + 2.0) / 11.0)
                .collect::<Vec<_>>(),
        )
        .unwrap();
        let c = a.mul(&b).unwrap();
        assert_eq!((c.rows(), c.cols()), (m, n));
        for r in 0..m {
            for col in 0..n {
                let reference: f64 = (0..k).map(|j| a.get(r, j) * b.get(j, col)).sum();
                assert!((c.get(r, col) - reference).abs() < 1e-11);
            }
        }
    }
}

#[test]
fn shared_assignment_does_not_copy_storage() {
    let mut engine = Engine::default();
    engine.execute("A=ones(128); B=A;").unwrap();
    assert_eq!(
        engine.variables["A"].data().as_ptr(),
        engine.variables["B"].data().as_ptr()
    );
    engine.execute("B=B+1;").unwrap();
    assert_eq!(engine.variables["A"].get(0, 0), 1.0);
    assert_eq!(engine.variables["B"].get(0, 0), 2.0);
}

#[test]
fn long_transpose_chain_is_rejected() {
    assert!(
        Engine::default()
            .execute(&format!("1{}", "'".repeat(20_000)))
            .is_err()
    );
}

#[test]
fn display_sampling_preserves_a_narrow_peak_and_bounds_geometry() {
    let x = Matrix::from_column_slice(1, 10000, &(0..10000).map(|i| i as f64).collect::<Vec<_>>())
        .unwrap();
    let mut data = vec![0.0; 10000];
    data[5013] = 100.0;
    let y = Matrix::from_column_slice(1, 10000, &data).unwrap();
    let figure = matrust::plot::Figure::new(&x, &y).unwrap();
    let display = figure.series[0].display_points(512);
    assert!(display.len() <= 512);
    assert!(display.contains(&[5013.0, 100.0]));
    assert_eq!(display.first(), Some(&[0.0, 0.0]));
    assert_eq!(display.last(), Some(&[9999.0, 0.0]));
    assert_eq!(figure.series[0].points.len(), 10000);
    let mut swapped = matrust::plot::Figure::new(&y, &x).unwrap();
    for kind in [
        matrust::plot::PlotKind::Line,
        matrust::plot::PlotKind::Scatter,
    ] {
        swapped.series[0].kind = kind;
        let display = swapped.series[0].display_points(512);
        assert!(display.len() <= 512);
        assert!(display.contains(&[100., 5013.]));
        assert!(display.contains(&[0., 9999.]));
    }
}
