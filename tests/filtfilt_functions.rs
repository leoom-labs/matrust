use matrust::{Engine, Matrix, dsp};
use std::sync::atomic::AtomicBool;
fn row(x: &[f64]) -> Matrix {
    Matrix::from_column_slice(1, x.len(), x).unwrap()
}
#[test]
fn independent_scipy_full_padding_fixtures() {
    let cancel = AtomicBool::new(false);
    let mut count = 0;
    for line in include_str!("filtfilt_oracle.csv").lines().skip(1) {
        let f: Vec<_> = line.split(',').collect();
        assert_eq!(f.len(), 8);
        let parse = |s: &str| {
            s.split_whitespace()
                .map(|v| v.parse::<f64>().unwrap())
                .collect::<Vec<_>>()
        };
        let rows = f[1].parse::<usize>().unwrap();
        let cols = f[2].parse::<usize>().unwrap();
        let b = parse(f[3]);
        let a = parse(f[4]);
        let input = parse(f[5]);
        let expected = parse(f[6]);
        let x = Matrix::from_column_slice(rows, cols, &input).unwrap();
        let actual = dsp::filtfilt(&b, &a, &x, &cancel).unwrap();
        assert_eq!((actual.rows(), actual.cols()), (rows, cols));
        assert_eq!(actual.len(), expected.len());
        let tolerance = f[7].parse::<f64>().unwrap();
        for (index, (&actual, expected)) in actual.data().iter().zip(expected).enumerate() {
            assert!(
                (actual - expected).abs() <= tolerance * (1. + expected.abs()),
                "{} sample{index}: {actual} != {expected}",
                f[0]
            );
        }
        assert_eq!(x.data(), input);
        count += 1;
    }
    // Generator fixes this count; missing fixtures must not silently pass.
    assert_eq!(count, 144);
}
#[test]
fn filtfilt_gain_constants_linear_ramps_and_empty_shapes() {
    let c = AtomicBool::new(false);
    let x = row(&[1., -2., 0., 3.]);
    assert_eq!(
        dsp::filtfilt(&[6.], &[2.], &x, &c).unwrap().data(),
        &[9., -18., 0., 27.]
    );
    for (b, a) in [
        (vec![0.25, 0.5, 0.25], vec![1.]),
        (vec![0.5], vec![1., -0.5]),
    ] {
        let x = row(&[7.; 40]);
        let y = dsp::filtfilt(&b, &a, &x, &c).unwrap();
        assert!(y.data().iter().all(|v| (v - 7.).abs() < 1e-12));
    }
    let x = row(&(0..40).map(|i| i as f64).collect::<Vec<_>>());
    let y = dsp::filtfilt(&[0.25, 0.5, 0.25], &[1.], &x, &c).unwrap();
    assert_eq!(y.data(), x.data());
    for (rows, cols) in [(0, 0), (0, 7), (1, 0), (8, 0)] {
        let x = Matrix::from_column_vec(rows, cols, vec![]).unwrap();
        let y = dsp::filtfilt(&[1.], &[1.], &x, &c).unwrap();
        assert_eq!((y.rows(), y.cols()), (rows, cols));
    }
    let huge = row(&[f64::MAX; 4]);
    // FMA reflection avoids spurious overflow from an intermediate 2*MAX.
    assert_eq!(
        dsp::filtfilt(&[1., 0.], &[1.], &huge, &c).unwrap().data(),
        huge.data()
    );
}
#[test]
fn filtfilt_rejects_short_nonfinite_singular_overflow_and_bounded_work() {
    let c = AtomicBool::new(false);
    let x = row(&[1.; 32]);
    for (b, a) in [
        (vec![], vec![1.]),
        (vec![1.], vec![]),
        (vec![1.], vec![0.]),
        (vec![f64::NAN], vec![1.]),
        (vec![1.], vec![f64::INFINITY]),
        (vec![1.], vec![1., -1.]),
        (vec![f64::MAX], vec![f64::MIN_POSITIVE]),
    ] {
        assert!(dsp::filtfilt(&b, &a, &x, &c).is_err());
    }
    for bad in [f64::NAN, f64::INFINITY, f64::NEG_INFINITY] {
        assert!(dsp::filtfilt(&[1.], &[1.], &row(&[bad, 0.]), &c).is_err());
    }
    assert!(dsp::filtfilt(&[1.], &[1.], &row(&[1.]), &c).is_err());
    assert!(dsp::filtfilt(&[0.5, 0.5], &[1.], &row(&[1.; 3]), &c).is_err());
    assert!(dsp::filtfilt(&[0.5, 0.5], &[1.], &row(&[1.; 4]), &c).is_ok());
    assert!(
        dsp::filtfilt(
            &[1.],
            &[1., -2.],
            &row(&(0..2048)
                .map(|i| if i == 0 { 1e300 } else { 0. })
                .collect::<Vec<_>>()),
            &c
        )
        .is_err()
    );
    assert!(dsp::filtfilt(&[1., 0.], &[1.], &row(&[f64::MAX, -f64::MAX, 0., 0.]), &c).is_err());
    assert!(dsp::filtfilt(&[f64::MAX, f64::MAX], &[1.], &row(&[2.; 8]), &c).is_err());
    let a: Vec<_> = (0..258)
        .map(|i| {
            if i == 0 {
                1.
            } else if i == 1 {
                -0.5
            } else {
                0.
            }
        })
        .collect();
    assert!(dsp::filtfilt(&[1.], &a, &row(&vec![1.; 1000]), &c).is_err());
    let b = vec![1. / 4097.; 4097];
    assert!(dsp::filtfilt(&b, &[1.], &row(&vec![1.; 40000]), &c).is_err());
}
#[test]
fn filtfilt_workers_are_bitwise_equal_and_inputs_unchanged() {
    let c = AtomicBool::new(false);
    // 32768 samples * 8 columns * two order-two passes exceeds the current
    // one-million-work threshold; the eight-worker call enters parallel code.
    let x = Matrix::from_column_vec(
        32768,
        8,
        (0..262144).map(|i| (i as f64 / 11.).sin()).collect(),
    )
    .unwrap();
    let alias = x.clone();
    let before = x.data().to_vec();
    let mut outputs = Vec::new();
    for workers in [1, 8] {
        let pool = rayon::ThreadPoolBuilder::new()
            .num_threads(workers)
            .build()
            .unwrap();
        outputs.push(
            pool.install(|| dsp::filtfilt(&[0.2, 0.1, 0.2], &[1., -0.4, 0.1], &x, &c).unwrap()),
        );
    }
    assert!(
        outputs[0]
            .data()
            .iter()
            .zip(outputs[1].data())
            .all(|(a, b)| a.to_bits() == b.to_bits())
    );
    assert_eq!(x.data(), before);
    assert_eq!(alias.data(), before);
    let cancelled = AtomicBool::new(true);
    assert!(
        dsp::filtfilt(&[1.], &[1.], &x, &cancelled)
            .unwrap_err()
            .contains("cancelled")
    );
    assert_eq!(x.data(), before);
    assert_eq!(alias.data(), before);
}
#[test]
fn filtfilt_script_workflow_and_failed_assignment_are_atomic() {
    let mut e = Engine::default();
    e.execute("b=fir1(8,0.3);x=sin(1:64);y=filtfilt(b,1,x);held=y;")
        .unwrap();
    let expected = e.variables["y"].data().to_vec();
    // Six-element column coefficients remain transfer-function inputs.
    e.execute("q=filtfilt([1;0;0;1;0;0],1,x);").unwrap();
    assert_eq!((e.variables["q"].rows(), e.variables["q"].cols()), (1, 64));
    let column_expected = dsp::filtfilt(
        &[1., 0., 0., 1., 0., 0.],
        &[1.],
        &e.variables["x"],
        &AtomicBool::new(false),
    )
    .unwrap();
    assert_eq!(e.variables["q"].data(), column_expected.data());
    for script in [
        "y=filtfilt(1,[1 -1],x);",
        "y=filtfilt(b,1,x,'gust');",
        "y=filtfilt([1 2;3 4],1,x);",
        "y=filtfilt([],1,x);",
    ] {
        assert!(e.execute(script).is_err());
        assert_eq!(e.variables["y"].data(), expected);
    }
    assert_eq!(e.variables["held"].data(), expected);
}
#[test]
#[allow(clippy::excessive_precision)]
fn filtfilt_one_row_sos_follows_matlab_rule() {
    let mut e = Engine::default();
    e.execute("x=ones(1,50);s=[0.067455273889071868 0.13491054777814374 0.067455273889071868 1 -1.1429805025399014 0.41280159809618883];y=filtfilt(s,1,x);")
        .unwrap();
    let y = e.variables["y"].data().to_vec();
    assert!((y[0] - 1.).abs() < 1e-12 && (y[24] - 1.).abs() < 1e-12);
    // 4th element not 1: stays an FIR numerator.
    e.execute("f=[1 2 3 2 1 1];u=filtfilt(f,1,x);v=filtfilt([1;2;3;2;1;1],1,x);")
        .unwrap();
    assert_eq!(e.variables["u"].data(), e.variables["v"].data());
    // numel(a)==3: stays a transfer function.
    let sos_row = [
        0.067455273889071868,
        0.13491054777814374,
        0.067455273889071868,
        1.,
        -1.1429805025399014,
        0.41280159809618883,
    ];
    let direct = dsp::filtfilt(
        &sos_row,
        &[1., 2., 3.],
        &e.variables["x"],
        &AtomicBool::new(false),
    );
    let via_engine = e.execute("t=filtfilt(s,[1 2 3],x);");
    match direct {
        Ok(d) => {
            via_engine.unwrap();
            assert_eq!(e.variables["t"].data(), d.data());
        }
        Err(_) => assert!(via_engine.is_err()),
    }
}

#[test]
fn filtfilt_empty_six_column_b_is_an_error() {
    let mut e = Engine::default();
    assert!(e.execute("y=filtfilt(zeros(0,6),1,ones(1,50));").is_err());
}
