use matrust::{Engine, Matrix, dsp};
use std::sync::atomic::AtomicBool;

fn matrix(r: usize, c: usize, data: &[f64]) -> Matrix {
    Matrix::from_column_slice(r, c, data).unwrap()
}
fn assert_matrix(x: &Matrix, r: usize, c: usize, data: &[f64]) {
    assert_eq!((x.rows(), x.cols()), (r, c));
    assert_eq!(
        x.data().iter().map(|v| v.to_bits()).collect::<Vec<_>>(),
        data.iter().map(|v| v.to_bits()).collect::<Vec<_>>()
    );
}

#[test]
fn sample_rate_phase_examples_and_payload_bits() {
    let cancel = AtomicBool::new(false);
    let row = matrix(1, 4, &[1., 2., 3., 4.]);
    assert_matrix(
        &dsp::upsample(&row, 3, 2, &cancel).unwrap(),
        1,
        12,
        &[0., 0., 1., 0., 0., 2., 0., 0., 3., 0., 0., 4.],
    );
    assert_matrix(&dsp::downsample(&row, 3, 1, &cancel).unwrap(), 1, 1, &[2.]);
    let x = matrix(3, 2, &[1., 3., 5., 2., 4., 6.]);
    assert_matrix(
        &dsp::upsample(&x, 2, 0, &cancel).unwrap(),
        6,
        2,
        &[1., 0., 3., 0., 5., 0., 2., 0., 4., 0., 6., 0.],
    );
    assert_matrix(
        &dsp::downsample(&x, 2, 0, &cancel).unwrap(),
        2,
        2,
        &[1., 5., 2., 6.],
    );
    let bits = [
        f64::from_bits(0x7ff8000000000011),
        -0.,
        f64::INFINITY,
        f64::NEG_INFINITY,
    ];
    let x = matrix(4, 1, &bits);
    assert_matrix(
        &dsp::downsample(&dsp::upsample(&x, 3, 1, &cancel).unwrap(), 3, 1, &cancel).unwrap(),
        4,
        1,
        &bits,
    );
    assert_eq!(
        dsp::upsample(&x, 1, 0, &cancel).unwrap().data().as_ptr(),
        x.data().as_ptr()
    );
}

#[test]
fn empty_rate_shapes_and_rejected_invalid_or_excessive_requests() {
    let cancel = AtomicBool::new(false);
    for (r, c, ur, uc, dr, dc) in [
        (0, 0, 0, 0, 0, 0),
        (1, 0, 1, 0, 1, 0),
        (0, 1, 0, 1, 0, 1),
        (0, 3, 0, 3, 0, 3),
        (3, 0, 6, 0, 1, 0),
    ] {
        let x = matrix(r, c, &[]);
        assert_matrix(&dsp::upsample(&x, 2, 1, &cancel).unwrap(), ur, uc, &[]);
        assert_matrix(&dsp::downsample(&x, 2, 1, &cancel).unwrap(), dr, dc, &[]);
    }
    let x = matrix(2, 1, &[1., 2.]);
    for (n, phase) in [(0, 0), (2, 2), (usize::MAX, 0)] {
        assert!(dsp::upsample(&x, n, phase, &cancel).is_err());
    }
    assert!(dsp::upsample(&x, matrust::matrix::MAX_ELEMENTS, 0, &cancel).is_err());
    assert!(dsp::upfirdn(&matrix(0, 1, &[]), &x, 1, 1, &cancel).is_err());
    assert!(dsp::upfirdn(&x, &matrix(1, 1, &[f64::NAN]), 1, 1, &cancel).is_err());
    assert!(
        dsp::upfirdn(
            &matrix(1, 1, &[f64::MAX]),
            &matrix(1, 1, &[2.]),
            1,
            1,
            &cancel
        )
        .unwrap_err()
        .contains("overflow")
    );
    assert!(
        dsp::upfirdn(
            &matrix(2, 2, &[1.; 4]),
            &matrix(2, 3, &[1.; 6]),
            1,
            1,
            &cancel
        )
        .is_err()
    );
    assert!(dsp::downsample(&x, 2, 0, &AtomicBool::new(true)).is_err());
}

#[test]
fn polyphase_exact_fixtures_matrix_pairing_and_support_tail() {
    let cancel = AtomicBool::new(false);
    let x = matrix(1, 3, &[1., 2., 3.]);
    let h = matrix(1, 3, &[1., 1., 1.]);
    assert_matrix(
        &dsp::upfirdn(&x, &h, 1, 1, &cancel).unwrap(),
        1,
        5,
        &[1., 3., 6., 5., 3.],
    );
    assert_matrix(
        &dsp::upfirdn(&x, &h, 3, 2, &cancel).unwrap(),
        1,
        5,
        &[1., 1., 2., 3., 3.],
    );
    // Unlike upsample, upfirdn does not retain trailing inserted zeros after last input.
    assert_matrix(
        &dsp::upfirdn(&x, &Matrix::scalar(1.), 3, 1, &cancel).unwrap(),
        1,
        7,
        &[1., 0., 0., 2., 0., 0., 3.],
    );
    let bank = matrix(2, 2, &[1., 2., 2., -1.]);
    assert_matrix(
        &dsp::upfirdn(&matrix(1, 2, &[1., 2.]), &bank, 1, 1, &cancel).unwrap(),
        3,
        2,
        &[1., 4., 4., 2., 3., -2.],
    );
    let channels = matrix(2, 2, &[1., 2., 3., 4.]);
    assert_matrix(
        &dsp::upfirdn(&channels, &bank, 1, 1, &cancel).unwrap(),
        3,
        2,
        &[1., 4., 4., 6., 5., -4.],
    );
    assert_matrix(
        &dsp::upfirdn(&channels, &matrix(1, 2, &[1., 2.]), 1, 1, &cancel).unwrap(),
        3,
        2,
        &[1., 4., 4., 3., 10., 8.],
    );
    assert_matrix(
        &dsp::upfirdn(
            &x,
            &Matrix::scalar(1.),
            matrust::matrix::MAX_ELEMENTS,
            matrust::matrix::MAX_ELEMENTS,
            &cancel,
        )
        .unwrap(),
        1,
        3,
        &[1., 2., 3.],
    );
    // Compensated summation retains a unit residual otherwise rounded away.
    assert_matrix(
        &dsp::upfirdn(&matrix(1, 3, &[1e16, 1., -1e16]), &h, 1, 1, &cancel).unwrap(),
        1,
        5,
        &[1e16, 1e16, 1., -1e16, -1e16],
    );
}

#[test]
fn one_and_eight_workers_have_identical_rate_and_fir_results() {
    // The downsample result is exactly 512K elements, crossing all three
    // provisional scheduling predicates in this test.
    let x = matrix(
        65536,
        16,
        &(0..65536 * 16)
            .map(|i| (i % 31) as f64 / 8.)
            .collect::<Vec<_>>(),
    );
    let h = matrix(
        1,
        17,
        &(0..17).map(|i| (i as f64 - 8.) / 128.).collect::<Vec<_>>(),
    );
    let mut expected = None;
    for workers in [1, 8] {
        let pool = rayon::ThreadPoolBuilder::new()
            .num_threads(workers)
            .build()
            .unwrap();
        let values = pool.install(|| {
            let cancel = AtomicBool::new(false);
            [
                dsp::upsample(&x, 2, 1, &cancel).unwrap(),
                dsp::downsample(&x, 2, 1, &cancel).unwrap(),
                dsp::upfirdn(&x, &h, 3, 2, &cancel).unwrap(),
            ]
            .map(|m| m.data().iter().map(|v| v.to_bits()).collect::<Vec<_>>())
        });
        if let Some(old) = &expected {
            assert_eq!(&values, old);
        } else {
            expected = Some(values);
        }
    }
}

#[test]
fn script_defaults_filter_bank_and_rejections_retain_existing_target() {
    let mut engine = Engine::default();
    engine.execute("x=[1 2 3]; h=[1 1 1]; u=upsample(x,3,2); d=downsample(u,3,2); y=upfirdn(x,h); z=upfirdn(x,h,3,2);").unwrap();
    assert_matrix(&engine.variables["d"], 1, 3, &[1., 2., 3.]);
    assert_matrix(&engine.variables["y"], 1, 5, &[1., 3., 6., 5., 3.]);
    assert_matrix(&engine.variables["z"], 1, 5, &[1., 1., 2., 3., 3.]);
    engine.execute("target=99;").unwrap();
    for expression in [
        "upsample(x)",
        "upsample(x,0)",
        "upsample(x,2,2)",
        "downsample(x,2,-1)",
        "upsample(x,Inf)",
        "downsample(x,1.5)",
        "upfirdn(x,h,[])",
        "upfirdn(x,h,2,NaN)",
        "upfirdn(x,h,1,1,1)",
        "upfirdn([],h)",
        "upfirdn(x,[1 NaN])",
        "upsample(x>0,2)",
    ] {
        assert!(
            engine.execute(&format!("target={expression};")).is_err(),
            "{expression}"
        );
        assert_eq!(engine.variables["target"].data(), &[99.]);
    }
}
