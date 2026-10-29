use matrust::dsp::Filter;
use std::sync::atomic::{AtomicBool, Ordering};

fn same(actual: &[f64], expected: &[f64]) {
    assert_eq!(actual.len(), expected.len());
    for (i, (&a, &b)) in actual.iter().zip(expected).enumerate() {
        assert!(
            a.to_bits() == b.to_bits() || (a.is_nan() && b.is_nan()),
            "sample {i}: {a:?} != {b:?}"
        );
    }
}

#[test]
fn specialized_recurrences_match_sample_kernel_including_nonfinite_and_padding() {
    let cancel = AtomicBool::new(false);
    for (b, a) in [
        (vec![0.2, 0.3], vec![2.]),
        (vec![0.2, 0.3, 0.4], vec![2.]),
        (vec![0.2], vec![2., -0.7]),
        (vec![0.2], vec![2., -0.7, 0.2]),
        (vec![0.2, 0.3], vec![2., -0.7, 0.2]),
        (vec![0.2, 0.3, 0.4], vec![2., -0.7]),
        (vec![0., -0., 0.], vec![1., -0., 0.]),
        (vec![1e150, -1e150, 1e150], vec![1., 0.1, -0.1]),
    ] {
        for n in [0, 1, 2, 31, 1023, 1024, 1025, 4099] {
            for exceptional in [false, true] {
                let mut fast = Filter::new(&b, &a).unwrap();
                fast.set_state(&vec![0.125; fast.state().len()]).unwrap();
                let mut reference = fast.clone();
                let mut actual: Vec<_> =
                    (0..n).map(|i| ((i * 17 % 31) as f64 - 15.) / 16.).collect();
                if exceptional && n > 31 {
                    actual[3] = f64::NAN;
                    actual[11] = f64::INFINITY;
                    actual[17] = f64::NEG_INFINITY;
                    actual[25] = -0.;
                    actual[31] = f64::from_bits(1);
                }
                let expected: Vec<_> = actual.iter().map(|&x| reference.sample(x)).collect();
                fast.process(&mut actual, &cancel).unwrap();
                same(&actual, &expected);
                same(fast.state(), reference.state());
            }
        }
    }
}

#[test]
fn specialized_streaming_and_cancellation_preserve_delays() {
    let cancel = AtomicBool::new(false);
    for (b, a) in [
        (vec![0.2, 0.3], vec![1., -0.5]),
        (vec![0.2, 0.3, 0.2], vec![1., -0.5, 0.2]),
        (vec![1., 2., 3.], vec![1.]),
    ] {
        let mut whole = Filter::new(&b, &a).unwrap();
        let mut chunked = whole.clone();
        let mut source: Vec<_> = (0..4099).map(|i| (i % 19) as f64 / 19.).collect();
        let mut split = source.clone();
        whole.process(&mut source, &cancel).unwrap();
        let mut start = 0;
        for end in [1, 1023, 1024, 1025, 2048, 4099] {
            chunked.process(&mut split[start..end], &cancel).unwrap();
            start = end;
        }
        same(&source, &split);
        same(whole.state(), chunked.state());
        let saved = chunked.state().to_vec();
        let untouched = split.clone();
        cancel.store(true, Ordering::Relaxed);
        assert!(chunked.process(&mut split, &cancel).is_err());
        same(&split, &untouched);
        same(chunked.state(), &saved);
        cancel.store(false, Ordering::Relaxed);
    }
}

#[test]
fn sos_pipeline_preserves_section_order_boundaries_and_exceptional_samples() {
    use matrust::{Matrix, dsp};
    let cancel = AtomicBool::new(false);
    for threads in [1, 8] {
        rayon::ThreadPoolBuilder::new()
            .num_threads(threads)
            .build()
            .unwrap()
            .install(|| {
                for sections in [1, 3, 4, 5, 8, 10, 64] {
                    for samples in [1, 63, 64, 65, 1026, 4101] {
                        for mode in 0..3 {
                            let mut coefficients = Vec::new();
                            for i in 0..sections {
                                coefficients.push(if mode == 1 && i % 5 == 0 {
                                    [0.2, 0.3, 0.2, 1., 0., -0.]
                                } else {
                                    [0.2 + (i % 7) as f64 / 20., 0.3, 0.2, 1., -0.5, 0.2]
                                });
                            }
                            let sos = Matrix::from_column_vec(
                                sections,
                                6,
                                (0..6)
                                    .flat_map(|c| coefficients.iter().map(move |v| v[c]))
                                    .collect(),
                            )
                            .unwrap();
                            let mut source: Vec<_> = (0..samples * 8)
                                .map(|i| ((i % 31) as f64 - 15.) / 16.)
                                .collect();
                            if mode == 2 {
                                let special = [
                                    0.,
                                    -0.,
                                    f64::from_bits(1),
                                    -f64::from_bits(1),
                                    1e300,
                                    -1e300,
                                    f64::NAN,
                                    f64::INFINITY,
                                    f64::NEG_INFINITY,
                                ];
                                for (i, v) in source.iter_mut().enumerate().take(70) {
                                    *v = special[i % special.len()];
                                }
                            }
                            let x = Matrix::from_column_vec(samples, 8, source.clone()).unwrap();
                            // A 1-by-N input is one row signal, matching MATLAB's
                            // first-nonsingleton-dimension rule.
                            let channel_len = if samples == 1 { source.len() } else { samples };
                            for channel in source.chunks_mut(channel_len) {
                                for c in &coefficients {
                                    let mut filter = Filter::new(&c[..3], &c[3..]).unwrap();
                                    for value in channel.iter_mut() {
                                        *value = filter.sample(*value);
                                    }
                                }
                            }
                            let actual = dsp::sosfilt(&sos, &x, &cancel).unwrap();
                            same(actual.data(), &source);
                        }
                    }
                }
            });
    }
}
