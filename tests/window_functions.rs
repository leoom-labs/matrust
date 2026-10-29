use matrust::{
    Engine,
    dsp::{WindowKind, window, window_length},
};
use std::sync::atomic::AtomicBool;

#[test]
fn independent_scipy_window_fixtures() {
    let cancel = AtomicBool::new(false);
    let mut cases = 0;
    for line in include_str!("dsp_windows_oracle.csv").lines().skip(1) {
        let fields: Vec<_> = line.splitn(4, ',').collect();
        let kind = WindowKind::from_name(fields[0]).unwrap();
        let n = fields[1].parse::<usize>().unwrap();
        let periodic = fields[2] == "true";
        let expected: Vec<f64> = fields[3]
            .split_whitespace()
            .map(|x| x.parse().unwrap())
            .collect();
        let actual = window(kind, n, periodic, &cancel).unwrap();
        assert_eq!((actual.rows(), actual.cols()), (n, 1));
        assert_eq!(actual.len(), expected.len());
        for (index, (actual, expected)) in actual.data().iter().zip(expected).enumerate() {
            assert!(
                (actual - expected).abs() < 2e-15,
                "{} n={n} periodic={periodic} i={index}: {actual} != {expected}",
                fields[0]
            );
        }
        cases += 1;
    }
    assert_eq!(cases, 81);
}

#[test]
fn windows_preserve_symmetry_periodic_prefix_and_singletons() {
    let cancel = AtomicBool::new(false);
    for name in [
        "hann", "hamming", "blackman", "bartlett", "triang", "rectwin",
    ] {
        let kind = WindowKind::from_name(name).unwrap();
        assert_eq!(window(kind, 1, false, &cancel).unwrap().data(), [1.]);
        for n in [2, 3, 16, 17, 65_536, 65_537] {
            let result = window(kind, n, false, &cancel).unwrap();
            assert!(
                result
                    .data()
                    .iter()
                    .zip(result.data().iter().rev())
                    .all(|(a, b)| a.to_bits() == b.to_bits())
            );
            if kind.supports_periodic() {
                let periodic = window(kind, n, true, &cancel).unwrap();
                let longer = window(kind, n + 1, false, &cancel).unwrap();
                assert_eq!(periodic.data(), &longer.data()[..n]);
            }
        }
    }
}

#[test]
fn windows_reject_unbounded_and_cancelled_work() {
    let cancel = AtomicBool::new(false);
    for n in [
        f64::NAN,
        f64::INFINITY,
        f64::NEG_INFINITY,
        -1.,
        -0.1,
        16_777_217.,
    ] {
        assert!(window_length(n).is_err(), "{n}");
    }
    assert_eq!(window_length(2.49).unwrap(), 2);
    assert_eq!(window_length(2.5).unwrap(), 3);
    assert_eq!(window_length(0.4).unwrap(), 0);
    assert!(window(WindowKind::Hann, usize::MAX, false, &cancel).is_err());
    assert!(window(WindowKind::Bartlett, 10, true, &cancel).is_err());
    assert!(window(WindowKind::Hann, 10, false, &AtomicBool::new(true)).is_err());
}

#[test]
fn windows_one_and_eight_worker_results_match_exactly() {
    let cancel = AtomicBool::new(false);
    for kind in [WindowKind::Hann, WindowKind::Hamming, WindowKind::Blackman] {
        let run = |threads| {
            rayon::ThreadPoolBuilder::new()
                .num_threads(threads)
                .build()
                .unwrap()
                .install(|| window(kind, 262_145, true, &cancel).unwrap())
        };
        assert_eq!(run(1).data(), run(8).data());
    }
}

#[test]
fn window_script_options_rounding_empty_and_filter_workflow() {
    let mut engine = Engine::default();
    engine.execute("a=hann(5);b=hann(4,'periodic');c=hamming(5,'symmetric');d=blackman(5);e=bartlett(5);f=triang(4);g=rectwin(2.5);z=hann(0);x=[1 0 0 0 0];y=filter(a,1,x);").unwrap();
    assert_eq!(engine.variables["g"].rows(), 3);
    assert_eq!(
        (engine.variables["z"].rows(), engine.variables["z"].cols()),
        (0, 1)
    );
    assert_eq!(engine.variables["y"].data(), engine.variables["a"].data());
    assert_eq!(engine.variables["e"].data(), [0., 0.5, 1., 0.5, 0.]);
    assert_eq!(engine.variables["f"].data(), [0.25, 0.75, 0.75, 0.25]);
}

#[test]
fn unsupported_window_options_do_not_silently_change_precision_or_sampling() {
    for script in [
        "a=hann(5,'single');",
        "a=blackman(5,'unknown');",
        "a=bartlett(5,'periodic');",
        "a=triang(5,'single');",
        "a=rectwin(5,1);",
        "a=hamming([2 3]);",
        "a=hann(-1);",
        "a=hann(Inf);",
        "a=hann(5,1);",
        "a=hann(5,'periodic',1);",
    ] {
        let mut engine = Engine::default();
        assert!(engine.execute(script).is_err(), "{script}");
        assert!(!engine.variables.contains_key("a"));
    }
}
