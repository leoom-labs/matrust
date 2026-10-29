use matrust::{
    Engine,
    dsp::{self, Band, FirOptions},
};
use std::sync::atomic::AtomicBool;
fn options(window: Option<&[f64]>, scale: bool) -> FirOptions<'_> {
    FirOptions { window, scale }
}
#[test]
fn fir_custom_windows_match_independent_scipy_fixtures() {
    let cancel = AtomicBool::new(false);
    let mut count = 0;
    for line in include_str!("fir_design_oracle.csv").lines().skip(1) {
        let fields: Vec<_> = line.splitn(7, ',').collect();
        let order = fields[0].parse().unwrap();
        let band = Band::parse(fields[1]).unwrap();
        let parse = |s: &str| {
            s.split_whitespace()
                .map(|v| v.parse::<f64>().unwrap())
                .collect::<Vec<_>>()
        };
        let cutoffs = parse(fields[2]);
        let window = if fields[4] == "default" {
            None
        } else {
            Some(parse(fields[4]))
        };
        let expected = parse(fields[6]);
        let actual = dsp::fir1_with_options(
            order,
            &cutoffs,
            band,
            options(window.as_deref(), fields[5] == "true"),
            &cancel,
        )
        .unwrap();
        assert_eq!(actual.len(), expected.len());
        let scale = expected.iter().map(|v| v.abs()).fold(0., f64::max);
        for (i, (actual, expected)) in actual.iter().zip(expected).enumerate() {
            assert!(
                (actual - expected).abs() <= 3e-13 * scale + 1e-315,
                "{} {} {} scale={} i={i}: {actual} != {expected}",
                fields[0],
                fields[1],
                fields[3],
                fields[5]
            );
        }
        count += 1;
    }
    assert_eq!(count, 278);
}
#[test]
fn fir_near_nyquist_bandpass_matches_high_precision_independent_integrals() {
    let cancel = AtomicBool::new(false);
    let mut count = 0;
    for line in include_str!("fir_extreme_oracle.csv").lines().skip(1) {
        let fields: Vec<_> = line.splitn(5, ',').collect();
        let order = fields[0].parse().unwrap();
        let parse = |s: &str| {
            s.split_whitespace()
                .map(|v| v.parse::<f64>().unwrap())
                .collect::<Vec<_>>()
        };
        let cutoffs = parse(fields[1]);
        let window = parse(fields[2]);
        let expected = parse(fields[4]);
        let actual = dsp::fir1_with_options(
            order,
            &cutoffs,
            Band::Bandpass,
            options(Some(&window), fields[3] == "true"),
            &cancel,
        )
        .unwrap();
        assert_eq!(actual.len(), expected.len());
        let bound = 3e-13 * expected.iter().map(|v| v.abs()).fold(0., f64::max);
        for (i, (actual, expected)) in actual.iter().zip(expected).enumerate() {
            assert!(
                (actual - expected).abs() <= bound,
                "order={order} cuts={cutoffs:?} scale={} i={i}: {actual} != {expected}",
                fields[3]
            );
        }
        count += 1;
    }
    assert_eq!(count, 126);
}
#[test]
fn fir_order_promotion_and_custom_window_shape_are_explicit() {
    let cancel = AtomicBool::new(false);
    for band in [Band::Highpass, Band::Bandstop] {
        let cuts = if band == Band::Highpass {
            vec![0.3]
        } else {
            vec![0.2, 0.7]
        };
        assert_eq!(
            dsp::fir1_with_options(7, &cuts, band, options(Some(&[1.; 9]), true), &cancel)
                .unwrap()
                .len(),
            9
        );
        assert!(
            dsp::fir1_with_options(7, &cuts, band, options(Some(&[1.; 8]), true), &cancel).is_err()
        );
    }
    let default = dsp::fir1(8, &[0.3], Band::Lowpass).unwrap();
    assert_eq!(
        default,
        dsp::fir1_with_options(8, &[0.3], Band::Lowpass, options(Some(&[]), true), &cancel)
            .unwrap()
    );
}
#[test]
fn fir_asymmetric_windows_use_magnitude_at_the_reference_frequency() {
    let cancel = AtomicBool::new(false);
    let window: Vec<_> = (0..17).map(|i| 0.2 + i as f64 / 16.).collect();
    let b = dsp::fir1_with_options(
        16,
        &[0.2, 0.7],
        Band::Bandpass,
        options(Some(&window), true),
        &cancel,
    )
    .unwrap();
    let h = dsp::response(&b, &[1.], &[0.45], &cancel).unwrap()[0];
    assert!((h[0].hypot(h[1]) - 1.).abs() < 2e-13);
    assert!(
        b.iter()
            .zip(b.iter().rev())
            .any(|(a, b)| (a - b).abs() > 1e-6)
    );
}
#[test]
fn fir_normalization_is_invariant_to_extreme_common_window_scale() {
    let cancel = AtomicBool::new(false);
    let base: Vec<_> = (0..65).map(|i| 0.4 + i as f64 / 128.).collect();
    let expected = dsp::fir1_with_options(
        64,
        &[0.27],
        Band::Lowpass,
        options(Some(&base), true),
        &cancel,
    )
    .unwrap();
    for factor in [1e-300, 1e300] {
        let window: Vec<_> = base.iter().map(|v| v * factor).collect();
        let actual = dsp::fir1_with_options(
            64,
            &[0.27],
            Band::Lowpass,
            options(Some(&window), true),
            &cancel,
        )
        .unwrap();
        assert!(
            actual
                .iter()
                .zip(&expected)
                .all(|(a, b)| (a - b).abs() < 2e-14)
        );
    }
}
#[test]
fn fir_degenerate_invalid_and_cancelled_designs_fail_without_nonfinite_output() {
    let cancel = AtomicBool::new(false);
    assert!(
        dsp::fir1_with_options(
            4,
            &[0.3],
            Band::Lowpass,
            options(Some(&[0.; 5]), true),
            &cancel
        )
        .is_err()
    );
    assert_eq!(
        dsp::fir1_with_options(
            4,
            &[0.3],
            Band::Lowpass,
            options(Some(&[0.; 5]), false),
            &cancel
        )
        .unwrap(),
        [0.; 5]
    );
    assert!(
        dsp::fir1_with_options(
            4,
            &[0.3],
            Band::Lowpass,
            options(Some(&[f64::NAN; 5]), false),
            &cancel
        )
        .is_err()
    );
    assert!(
        dsp::fir1_with_options(
            4,
            &[0.3],
            Band::Lowpass,
            FirOptions::default(),
            &AtomicBool::new(true)
        )
        .is_err()
    );
    assert!(
        dsp::fir1_with_options(4097, &[0.3], Band::Lowpass, FirOptions::default(), &cancel)
            .is_err()
    );
    // Nonzero windows can still have zero/ill-conditioned DC gain. They must
    // reject scaled designs, while noscale retains the meaningful coefficients.
    for window in [[1., 0., -1.], [1., 0., -1. + f64::EPSILON / 2.]] {
        assert!(
            dsp::fir1_with_options(
                2,
                &[0.5],
                Band::Lowpass,
                options(Some(&window), true),
                &cancel
            )
            .is_err()
        );
        let taps = dsp::fir1_with_options(
            2,
            &[0.5],
            Band::Lowpass,
            options(Some(&window), false),
            &cancel,
        )
        .unwrap();
        assert!(taps[0] > 0. && taps[2] < 0.);
        assert!(taps.iter().all(|v| v.is_finite()));
    }
    // The only retained ideal bandpass tap is an exact cosine zero. Do not
    // normalize a sin/cos library rounding residue into a unit impulse.
    let window = [0., 1., 0., 0., 0.];
    assert!(
        dsp::fir1_with_options(
            4,
            &[0.4, 0.6],
            Band::Bandpass,
            options(Some(&window), true),
            &cancel
        )
        .is_err()
    );
    assert!(
        dsp::fir1_with_options(
            4,
            &[0.4, 0.6],
            Band::Bandpass,
            options(Some(&window), false),
            &cancel
        )
        .unwrap()
        .iter()
        .all(|v| *v == 0.)
    );
    for band in [Band::Lowpass, Band::Highpass] {
        let window = [1., 0., 0., 0., 0.];
        assert!(
            dsp::fir1_with_options(4, &[0.5], band, options(Some(&window), true), &cancel).is_err()
        );
        assert!(
            dsp::fir1_with_options(4, &[0.5], band, options(Some(&window), false), &cancel)
                .unwrap()
                .iter()
                .all(|v| *v == 0.)
        );
    }
}
#[test]
fn fir_script_ordered_options_and_row_column_windows_work() {
    let mut engine = Engine::default();
    engine.execute("w=hann(9);a=fir1(8,0.3,w);b=fir1(8,0.3,'low',w','scale');c=fir1(8,0.3,[]);d=fir1(8,0.3);e=fir1(8,0.3,'NOSCALE');f=fir1(8,0.3,'LOW',w,'noscale');g=fir1(7,0.3,'high',rectwin(9));h=fir1(8,0.3,'',w,'noscale');").unwrap();
    assert_eq!(engine.variables["a"].data(), engine.variables["b"].data());
    assert_eq!(engine.variables["h"].data(), engine.variables["f"].data());
    assert_eq!(engine.variables["c"].data(), engine.variables["d"].data());
    assert_eq!(
        (engine.variables["g"].rows(), engine.variables["g"].cols()),
        (1, 9)
    );
    assert_ne!(engine.variables["e"].data(), engine.variables["d"].data());
    assert_ne!(engine.variables["f"].data(), engine.variables["a"].data());
}
#[test]
fn fir_script_rejects_invalid_option_order_and_unsupported_multiband() {
    for script in [
        "b=fir1(8,0.3,'noscale',hann(9));",
        "b=fir1(8,0.3,hann(9),'low');",
        "b=fir1(8,0.3,'low','high');",
        "b=fir1(8,0.3,[1 2;3 4]);",
        "b=fir1(7,0.3,'high',hann(8));",
        "b=fir1(8,[0.1 0.2 0.3],'DC-0');",
        "b=fir1(8,0.3,'unknown');",
        "b=fir1(8,0.3,1);",
        "b=fir1(8,0.3,hann(9),'noscale','scale');",
        "b=fir1(2.5,0.3);",
        "b=fir1(-1,0.3);",
        "b=fir1(0,0.3);",
        "b=fir1(8,0.3,true);",
        "b=fir1(8,[0.2 0.7],'low');",
        "b=fir1(8,0.3,'bandpass');",
    ] {
        let mut engine = Engine::default();
        engine.execute("b=42;").unwrap();
        assert!(engine.execute(script).is_err(), "{script}");
        assert_eq!(engine.variables["b"].data(), [42.]);
    }
}

#[test]
fn fir_boundaries_invalid_cutoffs_and_extreme_normalization_limits() {
    let cancel = AtomicBool::new(false);
    let weights = vec![1.; 4097];
    for band in [Band::Highpass, Band::Bandstop] {
        let cuts = if band == Band::Highpass {
            vec![0.3]
        } else {
            vec![0.2, 0.7]
        };
        for order in [4095, 4096] {
            assert_eq!(
                dsp::fir1_with_options(order, &cuts, band, options(Some(&weights), true), &cancel)
                    .unwrap()
                    .len(),
                4097
            );
        }
    }
    for value in [f64::NAN, f64::INFINITY, f64::NEG_INFINITY, 0., 1., -0.1] {
        assert!(
            dsp::fir1_with_options(8, &[value], Band::Lowpass, FirOptions::default(), &cancel)
                .is_err()
        );
    }
    for cuts in [[0.2, 0.2], [0.7, 0.2]] {
        assert!(
            dsp::fir1_with_options(8, &cuts, Band::Bandpass, FirOptions::default(), &cancel)
                .is_err()
        );
    }
    for value in [f64::INFINITY, f64::NEG_INFINITY] {
        assert!(
            dsp::fir1_with_options(
                8,
                &[0.3],
                Band::Lowpass,
                options(Some(&[value; 9]), false),
                &cancel
            )
            .is_err()
        );
    }
    let tiny = f64::from_bits(1);
    let weights = dsp::window(dsp::WindowKind::Hamming, 9, false, &cancel).unwrap();
    let weight_sum = weights.data().iter().sum::<f64>();
    for (band, cuts) in [
        (Band::Lowpass, vec![tiny]),
        (Band::Bandpass, vec![tiny, 3. * tiny]),
    ] {
        let actual =
            dsp::fir1_with_options(8, &cuts, band, FirOptions::default(), &cancel).unwrap();
        // sinc(x) rounds to 1 at these tiny arguments; the normalized limit is the window.
        assert!(
            actual
                .iter()
                .zip(weights.data())
                .all(|(a, w)| (a - w / weight_sum).abs() < 3e-15)
        );
    }
    let high = dsp::fir1_with_options(
        8,
        &[1. - f64::EPSILON / 2.],
        Band::Highpass,
        FirOptions::default(),
        &cancel,
    )
    .unwrap();
    for (i, (a, w)) in high.iter().zip(weights.data()).enumerate() {
        let sign = if i.is_multiple_of(2) { 1. } else { -1. };
        assert!((a - sign * w / weight_sum).abs() < 3e-15);
    }
    assert!(
        dsp::fir1_with_options(8, &[tiny], Band::Lowpass, options(None, false), &cancel)
            .unwrap()
            .iter()
            .all(|v| v.is_finite())
    );
}
