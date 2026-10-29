use matrust::{Engine, Matrix, dsp};
use std::{
    f64::consts::{PI, TAU},
    sync::atomic::AtomicBool,
};
fn run(name: &str, input: &Matrix, parameter: f64, cancel: &AtomicBool) -> matrust::Result<Matrix> {
    match name {
        "sinc" => dsp::sinc(input, cancel),
        "square" => dsp::square(input, parameter, cancel),
        _ => dsp::sawtooth(input, parameter, cancel),
    }
}
#[test]
fn independent_scipy_numpy_decimal_waveform_fixtures() {
    let cancel = AtomicBool::new(false);
    let mut count = 0;
    for line in include_str!("waveform_oracle.csv").lines().skip(1) {
        let fields: Vec<_> = line.splitn(5, ',').collect();
        let parse = |s: &str| {
            s.split_whitespace()
                .map(|v| v.parse::<f64>().unwrap())
                .collect::<Vec<_>>()
        };
        let values = parse(fields[2]);
        let expected = parse(fields[3]);
        let input = Matrix::from_column_vec(1, values.len(), values).unwrap();
        let actual = run(fields[0], &input, fields[1].parse().unwrap(), &cancel).unwrap();
        assert_eq!((actual.rows(), actual.cols()), (1, expected.len()));
        for (index, (&a, e)) in actual.data().iter().zip(expected).enumerate() {
            if e.is_nan() {
                assert!(a.is_nan());
            } else if fields[0] == "square"
                || (fields[4].starts_with("decimal110 series") && e == 0.)
            {
                assert_eq!(a, e, "case {count} i{index}");
            } else {
                let tolerance = if fields[0] == "sinc" && fields[4].starts_with("decimal110") {
                    4e-15 * e.abs() + 1e-320
                } else {
                    2e-14
                };
                assert!(
                    (a - e).abs() <= tolerance,
                    "case {count} {} i{index}: {a} != {e}",
                    fields[4]
                );
            }
        }
        count += 1;
    }
    assert_eq!(count, 65);
}
#[test]
fn waveform_shapes_empty_inputs_and_immutable_storage() {
    let cancel = AtomicBool::new(false);
    for (rows, cols) in [(0, 0), (0, 3), (2, 0), (1, 1), (1, 6), (6, 1), (2, 3)] {
        let input = Matrix::from_column_vec(
            rows,
            cols,
            (0..rows * cols).map(|i| i as f64 * 0.5).collect(),
        )
        .unwrap();
        let shared = input.clone();
        let before = input.data().to_vec();
        for (name, param) in [("sinc", 0.), ("square", 50.), ("sawtooth", 0.5)] {
            let result = run(name, &input, param, &cancel).unwrap();
            assert_eq!((result.rows(), result.cols()), (rows, cols));
            assert_eq!(input.data(), before);
            assert_eq!(shared.data().as_ptr(), input.data().as_ptr());
        }
    }
}
#[test]
fn sinc_zero_integer_nonfinite_and_even_symmetry() {
    let cancel = AtomicBool::new(false);
    let values = vec![
        -0.,
        0.,
        f64::from_bits(1),
        -f64::from_bits(1),
        1.,
        -1.,
        f64::MAX,
        -f64::MAX,
        f64::INFINITY,
        f64::NEG_INFINITY,
        f64::NAN,
        0.25,
        -0.25,
        1.25,
        -1.25,
    ];
    let result = dsp::sinc(
        &Matrix::from_column_vec(1, values.len(), values).unwrap(),
        &cancel,
    )
    .unwrap();
    assert_eq!(&result.data()[..4], &[1.; 4]);
    for x in &result.data()[4..8] {
        assert_eq!(x.to_bits(), 0f64.to_bits());
    }
    assert!(result.data()[8..11].iter().all(|x| x.is_nan()));
    assert_eq!(result.data()[11].to_bits(), result.data()[12].to_bits());
    assert_eq!(result.data()[13].to_bits(), result.data()[14].to_bits());
    assert!((result.data()[11] - 2f64.sqrt() * 2. / PI).abs() < 4e-16);
}
#[test]
fn waveform_half_open_boundaries_tiny_negative_and_endpoint_parameters() {
    let cancel = AtomicBool::new(false);
    let tiny = f64::from_bits(1);
    let input =
        Matrix::from_column_vec(1, 8, vec![-TAU, -PI, -tiny, -0., 0., PI, TAU, 2. * TAU]).unwrap();
    assert_eq!(
        dsp::square(&input, 50., &cancel).unwrap().data(),
        [1., -1., -1., 1., 1., -1., 1., 1.]
    );
    assert_eq!(dsp::square(&input, 0., &cancel).unwrap().data(), [-1.; 8]);
    assert_eq!(dsp::square(&input, 100., &cancel).unwrap().data(), [1.; 8]);
    assert_eq!(
        dsp::sawtooth(&input, 1., &cancel).unwrap().data(),
        [-1., 0., 1., -1., -1., 0., -1., -1.]
    );
    assert_eq!(
        dsp::sawtooth(&input, 0., &cancel).unwrap().data(),
        [1., 0., -1., 1., 1., 0., 1., 1.]
    );
    assert_eq!(
        dsp::sawtooth(&input, 0.5, &cancel).unwrap().data(),
        [-1., 1., -1., -1., -1., 1., -1., -1.]
    );
    for duty in [25., 50., 75.] {
        let p = TAU * duty / 100.;
        let around = Matrix::from_column_vec(1, 3, vec![p.next_down(), p, p.next_up()]).unwrap();
        assert_eq!(
            dsp::square(&around, duty, &cancel).unwrap().data(),
            [1., -1., -1.]
        );
    }
    for width in [tiny, 1e-300, 1f64.next_down()] {
        let peak = TAU * width;
        let points = Matrix::from_column_vec(
            1,
            5,
            vec![0., peak.next_down(), peak, peak.next_up(), TAU.next_down()],
        )
        .unwrap();
        assert!(
            dsp::sawtooth(&points, width, &cancel)
                .unwrap()
                .data()
                .iter()
                .all(|x| (-1. ..=1.).contains(x))
        );
    }
}
#[test]
fn invalid_waveform_parameters_phase_and_cancellation_reject_atomically() {
    let cancel = AtomicBool::new(false);
    let input = Matrix::scalar(0.);
    let empty = Matrix::from_column_vec(0, 3, vec![]).unwrap();
    for duty in [-1., 100.1, f64::NAN, f64::INFINITY, f64::NEG_INFINITY] {
        assert!(dsp::square(&input, duty, &cancel).is_err());
        assert!(dsp::square(&empty, duty, &cancel).is_err());
    }
    for width in [-0.1, 1.1, f64::NAN, f64::INFINITY, f64::NEG_INFINITY] {
        assert!(dsp::sawtooth(&input, width, &cancel).is_err());
        assert!(dsp::sawtooth(&empty, width, &cancel).is_err());
    }
    for phase in [f64::NAN, f64::INFINITY, f64::NEG_INFINITY] {
        assert!(dsp::square(&Matrix::scalar(phase), 50., &cancel).is_err());
        assert!(dsp::sawtooth(&Matrix::scalar(phase), 1., &cancel).is_err());
    }
    let stopped = AtomicBool::new(true);
    for (name, param) in [("sinc", 0.), ("square", 50.), ("sawtooth", 1.)] {
        assert!(run(name, &input, param, &stopped).is_err());
        assert!(run(name, &empty, param, &stopped).is_err());
    }
    let mut values = vec![0.; 65_537];
    values[65_536] = f64::INFINITY;
    let mixed = Matrix::from_column_vec(1, values.len(), values).unwrap();
    assert!(dsp::square(&mixed, 50., &cancel).is_err());
    assert!(dsp::sawtooth(&mixed, 1., &cancel).is_err());
}
#[test]
fn waveform_serial_parallel_results_match() {
    let cancel = AtomicBool::new(false);
    let input = Matrix::from_column_vec(
        257,
        257,
        (0..257 * 257).map(|i| i as f64 * 0.125 - 4000.).collect(),
    )
    .unwrap();
    for (name, param) in [("sinc", 0.), ("square", 37.), ("sawtooth", 0.37)] {
        let run_pool = |workers| {
            rayon::ThreadPoolBuilder::new()
                .num_threads(workers)
                .build()
                .unwrap()
                .install(|| run(name, &input, param, &cancel).unwrap())
        };
        assert_eq!(run_pool(1).data(), run_pool(8).data());
    }
}
#[test]
fn waveform_script_defaults_and_failed_assignment_preserve_target() {
    let mut engine = Engine::default();
    engine.execute("a=sinc([0 1; -1 0]); b=square([0 pi]); c=sawtooth([0 pi]); d=sawtooth([0 pi],0.5); e=square([0 pi],25); keep=42;").unwrap();
    assert_eq!(engine.variables["a"].data(), [1., 0., 0., 1.]);
    assert_eq!(
        (engine.variables["a"].rows(), engine.variables["a"].cols()),
        (2, 2)
    );
    assert_eq!(engine.variables["b"].data(), [1., -1.]);
    assert_eq!(engine.variables["c"].data(), [-1., 0.]);
    assert_eq!(engine.variables["d"].data(), [-1., 1.]);
    for script in [
        "keep=sinc(1,2);",
        "keep=square(1,50,1);",
        "keep=square([0 Inf]);",
        "keep=sawtooth(0,NaN);",
        "keep=square(0,[1 2]);",
        "keep=sawtooth(0,'single');",
    ] {
        assert!(engine.execute(script).is_err(), "{script}");
        assert_eq!(engine.variables["keep"].data(), [42.]);
    }
}
