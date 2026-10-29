use matrust::{Engine, dsp};
use std::sync::atomic::AtomicBool;

#[test]
fn independent_scipy_kaiser_windows() {
    let cancel = AtomicBool::new(false);
    let mut rows = 0;
    for line in include_str!("kaiser_oracle.csv").lines().skip(1) {
        let f: Vec<_> = line.splitn(5, ',').collect();
        let n = f[0].parse::<usize>().unwrap();
        let beta = f[1].parse().unwrap();
        let expected: Vec<f64> = f[3]
            .split_whitespace()
            .map(|v| v.parse().unwrap())
            .collect();
        let actual = dsp::kaiser(n, beta, &cancel).unwrap();
        assert_eq!((actual.rows(), actual.cols()), (n, 1));
        assert_eq!(actual.len(), expected.len());
        for (index, (a, b)) in actual.data().iter().zip(expected).enumerate() {
            assert!(
                (a - b).abs() <= 3e-13 * b.abs() + f64::from_bits(8),
                "n={n}, beta={beta}, i={index}: {a} != {b}"
            );
        }
        rows += 1;
    }
    assert_eq!(rows, 100);
}
#[test]
fn kaiser_mirrored_chunks_shapes_and_negative_beta_are_exact() {
    let cancel = AtomicBool::new(false);
    for n in [0, 1, 2, 3, 8, 8191, 8192, 8193, 16385] {
        for beta in [0., 0.5, 8.6, 50., 700., f64::MAX] {
            let output = dsp::kaiser(n, beta, &cancel).unwrap();
            assert_eq!((output.rows(), output.cols()), (n, 1));
            assert!(
                output
                    .data()
                    .iter()
                    .all(|x| x.is_finite() && (0. ..=1.).contains(x))
            );
            assert!(
                output
                    .data()
                    .iter()
                    .zip(output.data().iter().rev())
                    .all(|(a, b)| a.to_bits() == b.to_bits())
            );
            assert_eq!(
                output.data(),
                dsp::kaiser(n, -beta, &cancel).unwrap().data()
            );
            if !n.is_multiple_of(2) {
                assert_eq!(output.data()[n / 2], 1.);
            }
            if beta == 0. {
                assert!(output.data().iter().all(|&x| x == 1.));
            }
        }
    }
}
#[test]
fn kaiser_extreme_beta_and_subnormal_endpoints_are_supported() {
    let cancel = AtomicBool::new(false);
    assert_eq!(
        dsp::kaiser(3, f64::MAX, &cancel).unwrap().data(),
        [0., 1., 0.]
    );
    assert_eq!(dsp::kaiser(4, f64::MAX, &cancel).unwrap().data(), [0.; 4]);
    assert_eq!(
        dsp::kaiser(2, f64::from_bits(1), &cancel).unwrap().data(),
        [1.; 2]
    );
    // exp(-749) alone rounds to zero, but 1/I0(749) is representable.
    let endpoint = dsp::kaiser(2, 749., &cancel).unwrap().data()[0];
    assert!(endpoint > 0. && endpoint < f64::MIN_POSITIVE);
}
#[test]
fn kaiser_validation_bounds_and_cancellation_do_not_allocate_invalid_outputs() {
    let cancel = AtomicBool::new(false);
    for beta in [f64::NAN, f64::INFINITY, f64::NEG_INFINITY] {
        for n in [0, 1, 8] {
            assert!(dsp::kaiser(n, beta, &cancel).is_err());
        }
    }
    assert!(dsp::kaiser(usize::MAX, 0.5, &cancel).is_err());
    let stopped = AtomicBool::new(true);
    for n in [0, 1, 65_537] {
        assert!(dsp::kaiser(n, 0.5, &stopped).is_err());
    }
}
#[test]
fn kaiser_one_and_eight_workers_preserve_mirrored_output_bits() {
    let cancel = AtomicBool::new(false);
    let pools = [1, 8].map(|workers| {
        rayon::ThreadPoolBuilder::new()
            .num_threads(workers)
            .build()
            .unwrap()
    });
    for n in [65_536, 65_537, 73_729] {
        for beta in [0., 0.5, 50., 749., 1e14, f64::MAX] {
            let one = pools[0].install(|| dsp::kaiser(n, beta, &cancel).unwrap());
            let eight = pools[1].install(|| dsp::kaiser(n, beta, &cancel).unwrap());
            assert!(
                one.data()
                    .iter()
                    .zip(eight.data())
                    .all(|(a, b)| a.to_bits() == b.to_bits())
            );
        }
    }
}
#[test]
fn kaiser_script_defaults_rounding_filter_workflow_and_atomic_errors() {
    let mut engine = Engine::default();
    engine.execute("a=kaiser(9);b=kaiser(9,[]);c=kaiser(9,0.5,'double');d=kaiser(2.5,0);z=kaiser(0);w=kaiser(9,8.6);bfilter=fir1(8,0.3,w);response=filter(bfilter,1,[1 0 0 0 0 0 0 0 0]);keep=42;").unwrap();
    assert_eq!(engine.variables["a"].data(), engine.variables["b"].data());
    assert_eq!(engine.variables["a"].data(), engine.variables["c"].data());
    assert_eq!(engine.variables["d"].data(), [1.; 3]);
    assert_eq!(
        (engine.variables["z"].rows(), engine.variables["z"].cols()),
        (0, 1)
    );
    assert_eq!(
        engine.variables["response"].data(),
        engine.variables["bfilter"].data()
    );
    for script in [
        "keep=kaiser();",
        "keep=kaiser(-1);",
        "keep=kaiser(Inf);",
        "keep=kaiser([1 2]);",
        "keep=kaiser(3,[1 2]);",
        "keep=kaiser(0,NaN);",
        "keep=kaiser(3,Inf);",
        "keep=kaiser(3,1,'single');",
        "keep=kaiser(3,1,'periodic');",
        "keep=kaiser(16777217);",
    ] {
        assert!(engine.execute(script).is_err(), "{script}");
        assert_eq!(engine.variables["keep"].data(), [42.]);
    }
}
