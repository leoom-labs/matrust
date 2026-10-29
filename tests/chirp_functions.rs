use matrust::{
    Engine, Matrix,
    dsp::{self, ChirpMethod, ChirpOptions, ChirpShape},
};
use std::sync::atomic::AtomicBool;
#[test]
fn independent_scipy_decimal_and_exact_fraction_chirp_fixtures() {
    let cancel = AtomicBool::new(false);
    let mut count = 0;
    for line in include_str!("chirp_oracle.csv").lines().skip(1) {
        let fields: Vec<_> = line.splitn(9, ',').collect();
        let scalar = |i: usize| fields[i].parse::<f64>().unwrap();
        let shape = if fields[1] == "default" {
            ChirpShape::Default
        } else {
            ChirpShape::parse(fields[1]).unwrap()
        };
        let options = ChirpOptions {
            f0: scalar(2),
            t1: scalar(3),
            f1: scalar(4),
            method: ChirpMethod::parse(fields[0]).unwrap(),
            phase_degrees: scalar(5),
            shape,
        };
        let parse = |s: &str| {
            s.split_whitespace()
                .map(|v| v.parse::<f64>().unwrap())
                .collect::<Vec<_>>()
        };
        let input = parse(fields[6]);
        let expected = parse(fields[7]);
        let input = Matrix::from_column_vec(1, input.len(), input).unwrap();
        let actual = dsp::chirp(&input, options, &cancel).unwrap();
        for (index, (&actual, expected)) in actual.data().iter().zip(expected).enumerate() {
            let limit = if fields[8].starts_with("scipy") {
                2e-12
            } else if fields[8].starts_with("fraction") {
                8e-15
            } else {
                4e-13
            };
            assert!(
                (actual - expected).abs() <= limit,
                "row{count} sample{index} {}: {actual} != {expected}",
                fields[8]
            );
        }
        count += 1;
    }
    assert_eq!(count, 115);
}
#[test]
fn chirp_shapes_shared_storage_empty_and_initial_phase() {
    let cancel = AtomicBool::new(false);
    for (rows, cols) in [(0, 0), (0, 4), (3, 0), (1, 1), (1, 6), (6, 1), (2, 3)] {
        let input = Matrix::from_column_vec(rows, cols, vec![0.; rows * cols]).unwrap();
        let shared = input.clone();
        for method in [
            ChirpMethod::Linear,
            ChirpMethod::Quadratic,
            ChirpMethod::Logarithmic,
        ] {
            let options = ChirpOptions {
                phase_degrees: -90.,
                ..ChirpOptions::for_method(method)
            };
            let result = dsp::chirp(&input, options, &cancel).unwrap();
            assert_eq!((result.rows(), result.cols()), (rows, cols));
            assert!(result.data().iter().all(|&x| x == 0.));
            assert_eq!(input.data().as_ptr(), shared.data().as_ptr());
            assert!(input.data().iter().all(|&x| x == 0.));
        }
    }
}
#[test]
fn quadratic_shape_defaults_follow_sweep_direction() {
    let cancel = AtomicBool::new(false);
    let input = Matrix::from_column_vec(1, 4, vec![-0.2, 0.2, 0.7, 1.3]).unwrap();
    for (f0, f1, default_shape, other_shape) in [
        (1., 5., ChirpShape::Concave, ChirpShape::Convex),
        (5., 1., ChirpShape::Convex, ChirpShape::Concave),
    ] {
        let options = ChirpOptions {
            f0,
            f1,
            method: ChirpMethod::Quadratic,
            ..Default::default()
        };
        let default = dsp::chirp(&input, options, &cancel).unwrap();
        let explicit = dsp::chirp(
            &input,
            ChirpOptions {
                shape: default_shape,
                ..options
            },
            &cancel,
        )
        .unwrap();
        assert_eq!(default.data(), explicit.data());
        let other = dsp::chirp(
            &input,
            ChirpOptions {
                shape: other_shape,
                ..options
            },
            &cancel,
        )
        .unwrap();
        assert!(
            other
                .data()
                .iter()
                .zip(default.data())
                .any(|(a, b)| (a - b).abs() > 1e-3)
        );
    }
}
#[test]
fn chirp_constant_tones_handle_exact_huge_products_and_phase_degrees() {
    let cancel = AtomicBool::new(false);
    let huge = Matrix::from_column_vec(1, 3, vec![-f64::MAX, 0., f64::MAX]).unwrap();
    for method in [
        ChirpMethod::Linear,
        ChirpMethod::Quadratic,
        ChirpMethod::Logarithmic,
    ] {
        let options = ChirpOptions {
            f0: f64::MAX,
            f1: f64::MAX,
            method,
            phase_degrees: 180.,
            ..Default::default()
        };
        assert_eq!(
            dsp::chirp(&huge, options, &cancel).unwrap().data(),
            [-1.; 3]
        );
    }
    let frequency = 2f64.powi(51) + 0.5;
    let times = Matrix::from_column_vec(1, 3, vec![0.5, 1.5, 3.5]).unwrap();
    let result = dsp::chirp(
        &times,
        ChirpOptions {
            f0: frequency,
            f1: frequency,
            ..Default::default()
        },
        &cancel,
    )
    .unwrap();
    assert_eq!(result.data(), [0.; 3]);
    let options = ChirpOptions {
        f0: 0.,
        f1: 0.,
        phase_degrees: f64::MAX,
        ..Default::default()
    };
    assert!(
        dsp::chirp(&huge, options, &cancel)
            .unwrap()
            .data()
            .iter()
            .all(|x| x.is_finite())
    );
}
#[test]
fn chirp_scaled_intermediates_and_negative_log_time_remain_supported() {
    let cancel = AtomicBool::new(false);
    let endpoints = ChirpOptions {
        f0: -1e308,
        f1: 1.25e-300,
        t1: 1e300,
        ..Default::default()
    };
    assert!(
        (dsp::chirp(&Matrix::scalar(2e300), endpoints, &cancel)
            .unwrap()
            .data()[0]
            + 1.)
            .abs()
            < 2e-15
    );

    for (t, t1, f1) in [(1e-100, 1e-300, 1e-300), (1e100, 1e300, 1e300)] {
        let result = dsp::chirp(
            &Matrix::scalar(t),
            ChirpOptions {
                f0: 0.,
                f1,
                t1,
                method: ChirpMethod::Quadratic,
                ..Default::default()
            },
            &cancel,
        )
        .unwrap();
        assert!((result.data()[0] + 0.5).abs() < 2e-14);
    }
    let options = ChirpOptions {
        f0: 1.,
        f1: 2.,
        method: ChirpMethod::Logarithmic,
        ..Default::default()
    };
    assert!(
        dsp::chirp(&Matrix::scalar(-f64::MAX), options, &cancel)
            .unwrap()
            .data()[0]
            .is_finite()
    );
    let options = ChirpOptions {
        f0: f64::from_bits(1),
        f1: 1e300,
        method: ChirpMethod::Logarithmic,
        ..Default::default()
    };
    assert!(
        dsp::chirp(&Matrix::scalar(0.52), options, &cancel)
            .unwrap()
            .data()[0]
            .is_finite()
    );
}
#[test]
fn chirp_invalid_parameters_nonfinite_times_and_phase_overflow_are_explicit() {
    let cancel = AtomicBool::new(false);
    let input = Matrix::scalar(0.);
    let empty = Matrix::from_column_vec(0, 3, vec![]).unwrap();
    let base = ChirpOptions::default();
    let invalid = [
        ChirpOptions { t1: 0., ..base },
        ChirpOptions { t1: -1., ..base },
        ChirpOptions {
            t1: f64::INFINITY,
            ..base
        },
        ChirpOptions {
            f0: f64::NAN,
            ..base
        },
        ChirpOptions {
            f1: f64::INFINITY,
            ..base
        },
        ChirpOptions {
            phase_degrees: f64::NEG_INFINITY,
            ..base
        },
        ChirpOptions {
            shape: ChirpShape::Concave,
            ..base
        },
        ChirpOptions {
            method: ChirpMethod::Logarithmic,
            f0: 0.,
            ..base
        },
        ChirpOptions {
            method: ChirpMethod::Logarithmic,
            f0: -1.,
            f1: 1.,
            ..base
        },
    ];
    for options in invalid {
        assert!(dsp::chirp(&input, options, &cancel).is_err());
        assert!(dsp::chirp(&empty, options, &cancel).is_err());
    }
    for time in [f64::NAN, f64::INFINITY, f64::NEG_INFINITY] {
        assert!(dsp::chirp(&Matrix::scalar(time), base, &cancel).is_err());
    }
    for method in [
        ChirpMethod::Linear,
        ChirpMethod::Quadratic,
        ChirpMethod::Logarithmic,
    ] {
        let options = ChirpOptions {
            method,
            f0: 1.,
            f1: 1e300,
            ..base
        };
        let error = dsp::chirp(&Matrix::scalar(f64::MAX), options, &cancel).unwrap_err();
        assert!(error.contains("integrated phase"), "{error}");
    }
    let stopped = AtomicBool::new(true);
    assert!(dsp::chirp(&input, base, &stopped).is_err());
    assert!(dsp::chirp(&empty, base, &stopped).is_err());
    assert!(ChirpMethod::parse("hyperbolic").is_err());
    assert!(ChirpShape::parse("sideways").is_err());
}
#[test]
fn chirp_one_and_eight_workers_match_and_late_errors_discard_output() {
    let cancel = AtomicBool::new(false);
    let input = Matrix::from_column_vec(
        257,
        257,
        (0..257 * 257).map(|i| i as f64 / 65536.).collect(),
    )
    .unwrap();
    for method in [
        ChirpMethod::Linear,
        ChirpMethod::Quadratic,
        ChirpMethod::Logarithmic,
    ] {
        let options = ChirpOptions {
            f0: 1.,
            f1: 17.,
            method,
            ..Default::default()
        };
        let run = |workers| {
            rayon::ThreadPoolBuilder::new()
                .num_threads(workers)
                .build()
                .unwrap()
                .install(|| dsp::chirp(&input, options, &cancel).unwrap())
        };
        assert_eq!(run(1).data(), run(8).data());
    }
    let mut times = vec![0.; 65_537];
    times[65_536] = f64::INFINITY;
    assert!(
        dsp::chirp(
            &Matrix::from_column_vec(1, times.len(), times).unwrap(),
            ChirpOptions::default(),
            &cancel
        )
        .is_err()
    );
}
#[test]
fn chirp_script_defaults_aliases_empty_parameters_and_atomic_failures() {
    let mut engine = Engine::default();
    engine.execute("t=[0 0.01 0.02];a=chirp(t);b=chirp(t,[],[],[],'linear',[],'real');c=chirp(t,0,1,100,'li');d=chirp(t,1,1,7,'q',[],'convex');e=chirp(t,1,1,7,'quadratic',0,'convex','real');f=chirp(t,[],[],[],'lo');g=chirp(t,0.000001,1,100,'logarithmic');h=chirp(t,[],[],[],[]);keep=42;").unwrap();
    for name in ["b", "c", "h"] {
        assert_eq!(engine.variables["a"].data(), engine.variables[name].data());
    }
    assert_eq!(engine.variables["d"].data(), engine.variables["e"].data());
    assert_eq!(engine.variables["f"].data(), engine.variables["g"].data());
    for script in [
        "keep=chirp();",
        "keep=chirp(t,'complex');",
        "keep=chirp(t,1,1,2,'hyperbolic');",
        "keep=chirp(t,1,1,2,'linear',0,'convex');",
        "keep=chirp(t,1,0,2);",
        "keep=chirp([0 Inf]);",
        "keep=chirp(t,[1 2]);",
        "keep=chirp(t,1,1,2,'linear',[0 90]);",
    ] {
        assert!(engine.execute(script).is_err(), "{script}");
        assert_eq!(engine.variables["keep"].data(), [42.]);
    }
}
