use matrust::{
    Engine,
    dsp::{WindowKind, window},
};
use std::sync::atomic::AtomicBool;

const KINDS: [WindowKind; 3] = [
    WindowKind::BlackmanHarris,
    WindowKind::Nuttall,
    WindowKind::FlatTop,
];

#[test]
fn published_coefficient_identities_and_negative_flat_top_samples() {
    // Independent analytic values at 0, pi/2 and pi: cos takes 0 or +/-1.
    // These are arithmetic identities from published decimal coefficients, not MATLAB observations.
    let expected = [
        [0.00006, 0.21747, 1.0],
        [0.0003628, 0.2269824, 1.0],
        [-0.000421051, -0.05473684, 1.000000003],
    ];
    for (kind, expected) in KINDS.into_iter().zip(expected) {
        let actual = window(kind, 5, false, &AtomicBool::new(false)).unwrap();
        for (index, expected) in expected.into_iter().enumerate() {
            assert!((actual.data()[index] - expected).abs() <= 8. * f64::EPSILON);
        }
    }
    let flat = window(WindowKind::FlatTop, 5, false, &AtomicBool::new(false)).unwrap();
    assert!(flat.data()[0] < 0. && flat.data()[1] < 0.);
}

#[test]
fn symmetry_periodic_prefix_empty_and_singleton_contract() {
    let cancel = AtomicBool::new(false);
    for kind in KINDS {
        for periodic in [false, true] {
            let empty = window(kind, 0, periodic, &cancel).unwrap();
            assert_eq!((empty.rows(), empty.cols()), (0, 1));
            assert_eq!(window(kind, 1, periodic, &cancel).unwrap().data(), &[1.]);
        }
        for n in [2, 3, 4, 5, 16, 17, 4097, 65_537] {
            let symmetric = window(kind, n, false, &cancel).unwrap();
            assert_eq!((symmetric.rows(), symmetric.cols()), (n, 1));
            for (left, right) in symmetric.data().iter().zip(symmetric.data().iter().rev()) {
                assert_eq!(left.to_bits(), right.to_bits());
                assert!(left.is_finite());
            }
            let periodic = window(kind, n, true, &cancel).unwrap();
            let longer = window(kind, n + 1, false, &cancel).unwrap();
            assert_eq!(periodic.data(), &longer.data()[..n]);
        }
    }
}

#[test]
fn public_options_filter_fir_and_shared_assignment() {
    let mut engine = Engine::default();
    engine.execute("a=blackmanharris(9);b=a;p=nuttallwin(8,'periodic');w=flattopwin(9,'symmetric');z=nuttallwin(0);s=flattopwin(1);r=blackmanharris(2.5);y=filter(a,1,[1 0 0 0 0 0 0 0 0]);h=fir1(8,0.3,w);").unwrap();
    assert_eq!(
        engine.variables["a"].data().as_ptr(),
        engine.variables["b"].data().as_ptr()
    );
    assert_eq!(engine.variables["a"].data(), engine.variables["y"].data());
    assert_eq!(engine.variables["r"].rows(), 3);
    assert_eq!(
        (engine.variables["z"].rows(), engine.variables["z"].cols()),
        (0, 1)
    );
    assert_eq!(engine.variables["s"].data(), &[1.]);
    let fir = engine.variables["h"].data();
    assert_eq!(fir.len(), 9);
    assert!(fir.iter().all(|value| value.is_finite()));
    assert!((fir.iter().sum::<f64>() - 1.).abs() < 1e-12);
    for (a, b) in fir.iter().zip(fir.iter().rev()) {
        assert!((a - b).abs() < 1e-14);
    }
    engine.execute("b(1)=7;").unwrap();
    assert_ne!(engine.variables["a"].data()[0], 7.);
}

#[test]
fn invalid_options_lengths_and_cancellation_do_not_publish_output() {
    for name in ["blackmanharris", "nuttallwin", "flattopwin"] {
        for args in [
            "",
            "-1",
            "Inf",
            "NaN",
            "16777217",
            "[2 3]",
            "3+1i",
            "3,'single'",
            "3,'double'",
            "3,'unknown'",
            "3,1",
            "3,'periodic','double'",
        ] {
            let mut engine = Engine::default();
            assert!(
                engine.execute(&format!("a={name}({args});")).is_err(),
                "{name}({args})"
            );
            assert!(!engine.variables.contains_key("a"));
        }
    }
    for kind in KINDS {
        assert!(window(kind, usize::MAX, false, &AtomicBool::new(false)).is_err());
        assert!(window(kind, 0, false, &AtomicBool::new(true)).is_err());
        assert!(window(kind, 65_537, true, &AtomicBool::new(true)).is_err());
    }
}

#[test]
fn actual_one_and_eight_caller_contexts_match_bitwise() {
    for kind in KINDS {
        let run = |workers| {
            rayon::ThreadPoolBuilder::new()
                .num_threads(workers)
                .build()
                .unwrap()
                .install(|| window(kind, 65_537, true, &AtomicBool::new(false)).unwrap())
        };
        let one = run(1);
        let eight = run(8);
        for (one, eight) in one.data().iter().zip(eight.data()) {
            assert_eq!(one.to_bits(), eight.to_bits());
        }
    }
}

#[test]
fn public_threshold_modes_keep_serial_bits_prefix_shape_and_shared_storage() {
    let pools = [1, 8].map(|workers| {
        rayon::ThreadPoolBuilder::new()
            .num_threads(workers)
            .build()
            .unwrap()
    });
    let cancel = AtomicBool::new(false);
    for kind in KINDS {
        for n in [65_535, 65_536, 65_537] {
            for periodic in [false, true] {
                let reference = pools[0].install(|| {
                    assert_eq!(rayon::current_num_threads(), 1);
                    window(kind, n, periodic, &cancel).unwrap()
                });
                let reference_alias = reference.clone();
                assert_eq!(reference.data().as_ptr(), reference_alias.data().as_ptr());
                let prefix_reference = periodic
                    .then(|| pools[0].install(|| window(kind, n + 1, false, &cancel).unwrap()));
                for (pool, workers) in pools.iter().zip([1, 8]) {
                    let actual = pool.install(|| {
                        assert_eq!(rayon::current_num_threads(), workers);
                        window(kind, n, periodic, &cancel).unwrap()
                    });
                    assert_eq!((actual.rows(), actual.cols()), (n, 1));
                    let alias = actual.clone();
                    assert_eq!(actual.data().as_ptr(), alias.data().as_ptr());
                    for (index, (&actual, &expected)) in
                        actual.data().iter().zip(reference.data()).enumerate()
                    {
                        assert_eq!(
                            actual.to_bits(),
                            expected.to_bits(),
                            "{kind:?}, n={n}, periodic={periodic}, workers={workers}, index={index}"
                        );
                        assert!(actual.is_finite());
                        if let Some(longer) = &prefix_reference {
                            assert_eq!(actual.to_bits(), longer.data()[index].to_bits());
                        }
                    }
                }
                assert_eq!(reference.data().as_ptr(), reference_alias.data().as_ptr());
            }
        }
    }
}

#[test]
fn public_threshold_cancelled_calls_retry_without_changing_existing_aliases() {
    use std::sync::atomic::Ordering;
    let pools = [1, 8].map(|workers| {
        rayon::ThreadPoolBuilder::new()
            .num_threads(workers)
            .build()
            .unwrap()
    });
    for (pool, workers) in pools.iter().zip([1, 8]) {
        pool.install(|| {
            assert_eq!(rayon::current_num_threads(), workers);
            for kind in KINDS {
                for periodic in [false, true] {
                    let cancel = AtomicBool::new(false);
                    let existing = window(kind, 65_537, periodic, &cancel).unwrap();
                    let alias = existing.clone();
                    cancel.store(true, Ordering::Relaxed);
                    for n in [65_535, 65_536, 65_537] {
                        assert_eq!(
                            window(kind, n, periodic, &cancel).unwrap_err(),
                            "Calculation cancelled"
                        );
                    }
                    assert_eq!(existing.data().as_ptr(), alias.data().as_ptr());
                    cancel.store(false, Ordering::Relaxed);
                    let retried = window(kind, 65_537, periodic, &cancel).unwrap();
                    assert_eq!((retried.rows(), retried.cols()), (65_537, 1));
                    for (&actual, &expected) in retried.data().iter().zip(existing.data()) {
                        assert_eq!(actual.to_bits(), expected.to_bits());
                    }
                    assert_eq!(existing.data().as_ptr(), alias.data().as_ptr());
                }
            }
        });
    }
}
