use matrust::{Engine, Matrix, dsp};
use std::sync::atomic::AtomicBool;

fn sos(rows: &[[f64; 6]]) -> Matrix {
    Matrix::from_column_vec(
        rows.len(),
        6,
        (0..6)
            .flat_map(|c| rows.iter().map(move |r| r[c]))
            .collect(),
    )
    .unwrap()
}
fn row(values: &[f64]) -> Matrix {
    Matrix::from_column_slice(1, values.len(), values).unwrap()
}
fn identity() -> Matrix {
    sos(&[[1., 0., 0., 1., 0., 0.]; 2])
}

#[test]
// 192 SciPy references plus 24 Decimal80/120 high-order references; all original
// tolerances retained. See research/sos-filtfilt/high-precision-reference-attempt-1.
fn independent_scipy_and_decimal_sos_full_padding_fixtures() {
    let cancel = AtomicBool::new(false);
    let parse = |v: &str| {
        v.split_whitespace()
            .map(|x| x.parse::<f64>().unwrap())
            .collect::<Vec<_>>()
    };
    let mut count = 0;
    for line in include_str!("sos_filtfilt_oracle.csv").lines().skip(1) {
        let f: Vec<_> = line.split(',').collect();
        assert_eq!(f.len(), 10);
        let rows = f[1].parse::<usize>().unwrap();
        let cols = f[2].parse::<usize>().unwrap();
        let sections = f[3].parse::<usize>().unwrap();
        let coefficients = Matrix::from_column_vec(sections, 6, parse(f[4])).unwrap();
        let values = parse(f[6]);
        let input = Matrix::from_column_slice(rows, cols, &values).unwrap();
        let expected = parse(f[7]);
        let actual = dsp::filtfilt_sos(&coefficients, &parse(f[5]), &input, &cancel).unwrap();
        assert_eq!((actual.rows(), actual.cols()), (rows, cols));
        assert_eq!(actual.len(), expected.len());
        let tolerance = f[8].parse::<f64>().unwrap();
        for (index, (&a, b)) in actual.data().iter().zip(expected).enumerate() {
            assert!(
                (a - b).abs() <= tolerance * (1. + b.abs()),
                "{} sample{index}: {a} != {b}",
                f[0]
            );
        }
        assert_eq!(input.data(), values);
        count += 1;
    }
    assert_eq!(count, 216);
}

#[test]
fn sos_gain_distribution_zero_negative_extreme_and_zero_order_scalar() {
    let cancel = AtomicBool::new(false);
    for (gains, x, expected) in [
        (vec![1.], 3., 3.),
        (vec![0.], 3., 0.),
        (vec![-4.], 3., 48.),
        (vec![2., -3., 4.], 1., 576.),
        (vec![1e300], 1e-300, 1e300),
        (vec![1e-300], 1e300, 1e-300),
    ] {
        // Zero effective order permits a scalar SOS signal; existing TF
        // historical min-one-padding behavior is deliberately unchanged.
        let actual = dsp::filtfilt_sos(&identity(), &gains, &row(&[x]), &cancel).unwrap();
        if expected == 0. {
            assert_eq!(actual.data()[0], 0.);
        } else {
            assert!(
                (actual.data()[0] / expected - 1.).abs() < 3e-13,
                "{gains:?}"
            );
        }
    }
    let scaled_a0 = sos(&[[2., 0., 0., 2., 0., 0.], [4., 0., 0., 4., 0., 0.]]);
    assert_eq!(
        dsp::filtfilt_sos(&scaled_a0, &[1.], &row(&[f64::MAX]), &cancel)
            .unwrap()
            .data(),
        &[f64::MAX]
    );
}

#[test]
fn sos_padding_boundaries_empty_shapes_and_invalid_arguments() {
    let cancel = AtomicBool::new(false);
    let first_order = sos(&[[0.5, 0., 0., 1., -0.5, 0.]; 2]);
    assert!(dsp::filtfilt_sos(&first_order, &[1.], &row(&[1.; 6]), &cancel).is_err());
    assert!(dsp::filtfilt_sos(&first_order, &[1.], &row(&[1.; 7]), &cancel).is_ok());
    for (rows, cols) in [(0, 0), (0, 5), (3, 0)] {
        let empty = Matrix::from_column_vec(rows, cols, vec![]).unwrap();
        let result = dsp::filtfilt_sos(&first_order, &[1.], &empty, &cancel).unwrap();
        assert_eq!((result.rows(), result.cols()), (rows, cols));
    }
    let input = row(&[1.; 32]);
    for gains in [
        vec![],
        vec![1., 1.],
        vec![1.; 4],
        vec![f64::NAN],
        vec![f64::INFINITY],
    ] {
        assert!(dsp::filtfilt_sos(&identity(), &gains, &input, &cancel).is_err());
    }
    for coefficients in [
        sos(&[[1., 0., 0., 1., 0., 0.]; 65]),
        sos(&[[1., 0., 0., 0., 0., 0.]; 2]),
        sos(&[[1., 0., 0., 1., -1., 0.]; 2]),
        sos(&[[f64::NAN, 0., 0., 1., 0., 0.]; 2]),
    ] {
        assert!(dsp::filtfilt_sos(&coefficients, &[1.], &input, &cancel).is_err());
    }
    for bad in [f64::NAN, f64::INFINITY, f64::NEG_INFINITY] {
        assert!(dsp::filtfilt_sos(&identity(), &[1.], &row(&[bad]), &cancel).is_err());
    }
    assert!(dsp::filtfilt_sos(&identity(), &[f64::MAX], &row(&[1.]), &cancel).is_err());
}

#[test]
fn sos_workers_are_bit_identical_and_shared_inputs_retained() {
    let cancel = AtomicBool::new(false);
    let coefficients = dsp::butter_sos(8, 0.2, dsp::Band::Lowpass).unwrap();
    // (2*8192+3*24)*8*4*3 > 1 million: exercises candidate channel parallelism.
    let data: Vec<_> = (0..8192 * 8).map(|i| (i as f64 * 0.01).sin()).collect();
    let x = Matrix::from_column_slice(8192, 8, &data).unwrap();
    let alias = x.clone();
    let mut outputs = Vec::new();
    for workers in [1, 8] {
        let pool = rayon::ThreadPoolBuilder::new()
            .num_threads(workers)
            .build()
            .unwrap();
        outputs
            .push(pool.install(|| dsp::filtfilt_sos(&coefficients, &[1.], &x, &cancel).unwrap()));
    }
    assert!(
        outputs[0]
            .data()
            .iter()
            .zip(outputs[1].data())
            .all(|(a, b)| a.to_bits() == b.to_bits())
    );
    assert_eq!(x.data().as_ptr(), alias.data().as_ptr());
    assert_eq!(alias.data(), data);
    assert!(
        dsp::filtfilt_sos(&coefficients, &[1.], &x, &AtomicBool::new(true))
            .unwrap_err()
            .contains("cancelled")
    );
}

#[test]
fn sos_script_dispatch_single_row_tf_and_failed_assignment() {
    let mut engine = Engine::default();
    engine
        .execute("s=butter_sos(4,0.2,'low');x=sin(1:64);y=filtfilt(s,1,x);held=y;")
        .unwrap();
    // MATLAB rule: a 1x6 row with 4th element 1 and numel(a)<=2 is one SOS
    // section; this one is the identity section.
    engine.execute("q=filtfilt([1 0 0 1 0 0],1,x);").unwrap();
    assert_eq!(engine.variables["q"].data(), engine.variables["x"].data());
    for script in [
        "y=filtfilt(s,[1 2],x);",
        "y=filtfilt(s,1,x,'gust');",
        "y=filtfilt(s,Inf,x);",
    ] {
        assert!(engine.execute(script).is_err());
        assert_eq!(
            engine.variables["y"].data(),
            engine.variables["held"].data()
        );
    }
}
