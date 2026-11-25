use super::*;
use std::sync::atomic::AtomicBool;

fn db(c: &Coefficients, f: f64) -> f64 {
    let h = c.response(&[f], &AtomicBool::new(false)).unwrap()[0];
    20. * h[0].hypot(h[1]).max(1e-300).log10()
}
fn run(spec: &Spec) -> Coefficients {
    let mut engine = matrust::Engine::default();
    design(spec, &mut engine).unwrap_or_else(|e| {
        panic!(
            "{}: {e}\n{}",
            spec.title(),
            spec.script().unwrap_or_default()
        )
    })
}
fn spec(response: Response, method: Method) -> Spec {
    let mut s = Spec {
        fs: 1000.,
        ..Spec::default()
    };
    s.reset_frequencies();
    s.set_response(response);
    s.set_method(method);
    s
}
/// Worst passband loss and worst stopband level against the mask, in dB.
fn mask_margin(spec: &Spec, c: &Coefficients) -> (f64, f64) {
    let (mut pass, mut stop) = (0f64, -400f64);
    for band in spec.mask() {
        for i in 0..=200 {
            let f = band.f0 + (band.f1 - band.f0) * i as f64 / 200.;
            let level = db(c, f.clamp(0., 1.));
            if band.pass {
                pass = pass.min(level);
            } else {
                stop = stop.max(level);
            }
        }
    }
    (pass, stop)
}

#[test]
fn each_method_shows_only_the_fields_it_uses() {
    use Field::*;
    let mut s = spec(Response::Lowpass, Method::Butterworth);
    s.minimum = false;
    assert_eq!(s.fields(), [Order, Fs, Fc, Structure]);
    s.minimum = true;
    assert_eq!(s.fields(), [Fs, Fpass, Fstop, Apass, Astop, Structure]);
    s.set_method(Method::Chebyshev2);
    s.minimum = false;
    assert_eq!(s.fields(), [Order, Fs, Fstop, Astop, Structure]);
    s.set_method(Method::Window);
    assert_eq!(s.fields(), [Order, Fs, Fc, Window]);
    s.window = KAISER;
    assert_eq!(s.fields(), [Order, Fs, Fc, Window, Beta]);
    s.set_method(Method::LeastSquares);
    s.minimum = true; // no minimum order for firls: the order stays
    assert_eq!(s.fields(), [Order, Fs, Fpass, Fstop, Wpass, Wstop]);
    s.set_units(Units::Normalized);
    assert_eq!(s.fields(), [Order, Fpass, Fstop, Wpass, Wstop]);
    let notch = spec(Response::Notch, Method::NotchIir);
    assert_eq!(notch.method, Method::NotchIir);
    assert_eq!(notch.fields(), [Fs, F0, Bandwidth, Structure]);
    let multi = spec(Response::Multiband, Method::FrequencySampling);
    assert_eq!(multi.fields(), [Order, Fs, Freqs, Amps]);
    let hilbert = spec(Response::Hilbert, Method::Equiripple);
    assert_eq!(hilbert.fields(), [Order, Fs, Fpass]);
}

#[test]
fn response_change_keeps_a_valid_method() {
    let mut s = spec(Response::Lowpass, Method::Bessel);
    s.set_response(Response::Highpass);
    assert_eq!(s.method, Method::Butterworth); // Bessel is lowpass only
    s.set_method(Method::Elliptic);
    s.set_response(Response::Bandpass);
    assert_eq!(s.method, Method::Elliptic);
    s.set_response(Response::Hilbert);
    assert_eq!(s.method, Method::Equiripple);
    for r in RESPONSES {
        let mut s = Spec::default();
        s.set_response(r);
        assert!(r.methods().contains(&s.method));
        for &m in r.methods() {
            s.set_method(m);
            for minimum in [false, true] {
                s.minimum = minimum;
                assert!(s.problems().is_empty(), "{}: {:?}", s.title(), s.problems());
            }
        }
    }
}

#[test]
fn invalid_specs_name_the_field_in_plain_words() {
    let mut s = spec(Response::Lowpass, Method::Elliptic);
    s.fpass[0] = 300.;
    s.fstop[0] = 200.;
    assert_eq!(
        s.problems(),
        [(Field::Fstop, "Fpass must be below Fstop".to_owned())]
    );
    s.fpass[0] = 600.;
    assert_eq!(
        s.problems(),
        [(Field::Fpass, "Must lie between 0 and 500 Hz".to_owned())]
    );
    assert_eq!(s.script(), Err("Must lie between 0 and 500 Hz".to_owned()));
    let mut s = spec(Response::Bandpass, Method::Butterworth);
    s.fstop = [400., 300.];
    assert_eq!(s.problems()[0].0, Field::Fstop);
    s.fs = 0.;
    assert_eq!(
        s.problems(),
        [(
            Field::Fs,
            "Sample rate must be a positive number".to_owned()
        )]
    );
    let mut s = spec(Response::Highpass, Method::Window);
    s.minimum = false;
    s.order = 31;
    assert_eq!(s.problems()[0].0, Field::Order);
    let mut s = spec(Response::Lowpass, Method::Elliptic);
    s.astop = 0.5;
    assert_eq!(
        s.problems(),
        [(Field::Astop, "Astop must be larger than Apass".to_owned())]
    );
    let mut s = spec(Response::Multiband, Method::Equiripple);
    s.freqs = "0 100 150".into();
    assert_eq!(s.problems()[0].0, Field::Freqs);
    s.freqs = "0 100 150 500".into();
    s.amps = "1 1 0".into();
    assert_eq!(s.problems()[0].0, Field::Amps);
    s.amps = "1 x 0 0".into();
    assert_eq!(
        s.problems(),
        [(Field::Amps, "'x' is not a number".to_owned())]
    );
}

#[test]
fn units_switch_keeps_the_same_filter() {
    let mut s = spec(Response::Bandpass, Method::Elliptic);
    let before = (s.fpass, s.fstop);
    s.set_units(Units::Normalized);
    assert!((s.fpass[0] - before.0[0] / 500.).abs() < 1e-15);
    let normalized = run(&s);
    s.set_units(Units::Hz);
    assert!((s.fpass[1] - before.0[1]).abs() < 1e-9 && (s.fstop[0] - before.1[0]).abs() < 1e-9);
    let hz = run(&s);
    let (Coefficients::Sos { sos: a, .. }, Coefficients::Sos { sos: b, .. }) = (&normalized, &hz)
    else {
        panic!("expected sections");
    };
    assert_eq!(a.len(), b.len());
    assert!(
        a.iter()
            .flatten()
            .zip(b.iter().flatten())
            .all(|(x, y)| (x - y).abs() < 1e-9)
    );
}

#[test]
fn script_is_plain_matlab() {
    let mut s = spec(Response::Bandpass, Method::Elliptic);
    s.fpass = [100., 200.];
    s.fstop = [80., 230.];
    s.apass = 1.;
    s.astop = 60.;
    assert_eq!(
        s.script().unwrap(),
        "% Elliptic bandpass, minimum order (MatRust Filter Designer)\n\
         Fs = 1000;  % sample rate (Hz)\n\
         Fpass = [100 200];  Fstop = [80 230];  % Hz\n\
         Apass = 1;  Astop = 60;  % dB\n\
         [n, Wn] = ellipord(Fpass/(Fs/2), Fstop/(Fs/2), Apass, Astop);\n\
         [z, p, k] = ellip(n, Apass, Astop, Wn, 'bandpass');\n\
         [sos, g] = zp2sos(z, p, k);\n"
    );
    s.structure = Structure::Direct;
    assert!(s.script().unwrap().ends_with("[b, a] = zp2tf(z, p, k);\n"));
    assert_eq!(s.outputs(), ["b", "a"]);
}

/// Owner check, part 2: an elliptic bandpass at minimum order meets its mask.
#[test]
fn iir_minimum_order_meets_the_mask() {
    for method in [
        Method::Butterworth,
        Method::Chebyshev1,
        Method::Chebyshev2,
        Method::Elliptic,
    ] {
        for response in [
            Response::Lowpass,
            Response::Highpass,
            Response::Bandpass,
            Response::Bandstop,
        ] {
            let mut s = spec(response, method);
            s.minimum = true;
            s.apass = 1.;
            s.astop = 40.;
            let c = run(&s);
            let (pass, stop) = mask_margin(&s, &c);
            assert!(pass >= -1. - 1e-6, "{}: passband {pass}", s.title());
            assert!(stop <= -40. + 1e-6, "{}: stopband {stop}", s.title());
        }
    }
    let mut s = spec(Response::Bandpass, Method::Elliptic);
    s.minimum = true;
    let c = run(&s);
    assert_eq!(c.order(), 14); // SciPy ellipord([.35 .55],[.3 .6],1,80) = 7, doubled
}

#[test]
fn specified_order_iir_and_window_fir_designs() {
    let mut s = spec(Response::Lowpass, Method::Butterworth);
    s.minimum = false;
    s.order = 6;
    s.fc = [100., 0.];
    let c = run(&s);
    assert_eq!(c.order(), 6);
    assert!((db(&c, 0.2) + 3.0103).abs() < 1e-3); // −3 dB at Fc = 100 Hz of 500
    s.structure = Structure::Direct;
    let Coefficients::Tf { b, a } = run(&s) else {
        panic!()
    };
    assert_eq!((b.len(), a.len()), (7, 7));
    let mut s = spec(Response::Bandstop, Method::Window);
    s.minimum = false;
    s.order = 40;
    s.fc = [100., 200.];
    for window in [0, KAISER, 6] {
        s.window = window;
        let c = run(&s);
        assert_eq!(c.order(), 40);
        assert!(db(&c, 0.3) < -20., "window {window}");
        assert!(db(&c, 0.) > -0.5);
    }
}

#[test]
fn fir_equiripple_and_frequency_sampling_designs() {
    let mut s = spec(Response::Lowpass, Method::Equiripple);
    s.minimum = false;
    s.order = 60;
    s.fpass = [100., 0.];
    s.fstop = [150., 0.];
    let c = run(&s);
    let (pass, stop) = (db(&c, 0.1), db(&c, 0.5));
    assert!(pass.abs() < 0.5 && stop < -40., "{pass} {stop}");
    s.set_method(Method::FrequencySampling);
    let c = run(&s);
    assert!(db(&c, 0.1).abs() < 0.5 && db(&c, 0.5) < -30.);
    let mut s = spec(Response::Multiband, Method::Equiripple);
    s.order = 80;
    let c = run(&s);
    assert!(db(&c, 0.1).abs() < 0.5 && db(&c, 0.35) < -30.);
    assert!((db(&c, 0.8) - 20. * 0.5f64.log10()).abs() < 0.5);
    let mut s = spec(Response::Arbitrary, Method::FrequencySampling);
    s.order = 60;
    let c = run(&s);
    assert!((db(&c, 0.5) - 20. * 0.5f64.log10()).abs() < 0.5);
    let mut s = spec(Response::Hilbert, Method::Equiripple);
    s.order = 30;
    let c = run(&s);
    assert!(db(&c, 0.5).abs() < 0.5); // SciPy remez: |H(0.5)| = 0.9575
    let mut s = spec(Response::Differentiator, Method::Equiripple);
    s.order = 31;
    let c = run(&s);
    let h = c.response(&[0.25], &AtomicBool::new(false)).unwrap()[0];
    assert!((h[0].hypot(h[1]) - 0.25 * std::f64::consts::PI).abs() < 1e-2);
}

// The builtins below land with the engine tasks of this step
// (firls, kaiserord, firpmord, iirnotch, bilinear).

/// Owner check, part 1: a 50 Hz notch at Fs = 1 kHz.
#[test]
fn notch_50_hz_at_1_khz() {
    let mut s = spec(Response::Notch, Method::NotchIir);
    s.f0 = 50.;
    s.bandwidth = 5.;
    let c = run(&s);
    assert_eq!(c.order(), 2);
    assert!(db(&c, 0.1) < -100., "notch depth {}", db(&c, 0.1));
    assert!(db(&c, 0.).abs() < 1e-9 && db(&c, 1.).abs() < 1e-9);
    // Half-power edges of the bilinear notch (Orfanidis; Octave iirnotch):
    // 47.5604 and 52.5604 Hz, exactly `bandwidth` apart, not centred on f0.
    for edge in [47.560393667818, 52.560393667818] {
        assert!(
            (db(&c, edge / 500.) + 3.0103).abs() < 1e-3,
            "{edge}: {}",
            db(&c, edge / 500.)
        );
    }
}

#[test]
fn fir_minimum_order_and_least_squares() {
    for method in [Method::Window, Method::Equiripple] {
        for response in [
            Response::Lowpass,
            Response::Highpass,
            Response::Bandpass,
            Response::Bandstop,
        ] {
            let mut s = spec(response, method);
            s.minimum = true;
            s.apass = 1.;
            s.astop = 50.;
            let c = run(&s);
            let (pass, stop) = mask_margin(&s, &c);
            // Kaiser and Herrmann order formulas are estimates: allow 3 dB.
            assert!(pass >= -4. && stop <= -47., "{}: {pass} {stop}", s.title());
        }
    }
    let mut s = spec(Response::Lowpass, Method::LeastSquares);
    s.order = 60;
    let c = run(&s);
    assert!(db(&c, 0.2).abs() < 0.5 && db(&c, 0.7) < -40.);
}

#[test]
fn bessel_lowpass_has_its_order_and_unit_dc_gain() {
    let mut s = spec(Response::Lowpass, Method::Bessel);
    s.minimum = false;
    s.order = 4;
    s.fc = [100., 0.];
    let c = run(&s);
    assert_eq!(c.order(), 4);
    assert!(db(&c, 0.).abs() < 1e-9);
    assert!(db(&c, 0.9) < -20.);
}

#[test]
fn every_combination_designs() {
    for r in RESPONSES {
        for &m in r.methods() {
            for minimum in [false, true] {
                let mut s = spec(r, m);
                s.minimum = minimum;
                let c = run(&s);
                let f: Vec<f64> = (0..64).map(|i| i as f64 / 63.).collect();
                let h = c.response(&f, &AtomicBool::new(false)).unwrap();
                assert!(h.iter().flatten().all(|v| v.is_finite()), "{}", s.title());
            }
        }
    }
}

#[test]
fn filter_type_switches_between_iir_and_fir_families() {
    let mut s = spec(Response::Bandpass, Method::Elliptic);
    assert_eq!(s.families(), (true, true));
    s.set_family(false);
    assert_eq!(s.method, Method::Window);
    s.set_method(Method::LeastSquares);
    s.set_family(false);
    assert_eq!(s.method, Method::LeastSquares);
    s.set_family(true);
    assert_eq!(s.method, Method::Butterworth);
    s.set_response(Response::Hilbert);
    assert_eq!(s.families(), (false, true));
    s.set_family(true);
    assert!(!s.method.iir());
    s.set_response(Response::Notch);
    assert_eq!(s.families(), (true, false));
}
