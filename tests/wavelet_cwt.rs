//! Continuous wavelet transform builtins: cwt, icwt, cwtfilterbank (wt, freqz,
//! wavelets, scales), wsst and iwsst. Expected values come from the NumPy
//! implementation of the published analytic Morse (Lilly & Olhede), Morlet and
//! bump wavelets and of the Daubechies-Lu-Wu synchrosqueezing in
//! tool/refs/wavelet_cwt/wavelet_cwt.py (it prints the constants in `refs`),
//! and from closed forms written out in the tests. Never MATLAB.
use matrust::{Array, Complex64, Engine};
use refs::*;

const SIG256: &str = "t = (0:255)/256; x = cos(2*pi*20*t) + 0.5*sin(2*pi*45*t + 0.3) + 0.25*t;";
const Z256: &str = "t = (0:255)/256; z = exp(1i*2*pi*20*t) + 0.5*exp(-1i*2*pi*45*t);";
const PACKETS: &str = "t = (0:1023)/1024; x = cos(2*pi*40*t).*exp(-((t-0.5)/0.15).^2) \
                       + 0.5*sin(2*pi*90*t).*exp(-((t-0.4)/0.1).^2);";
const TONES: &str = "t = (0:1023)/1024; x = cos(2*pi*40*t) + 0.5*sin(2*pi*90*t);";

fn run(e: &mut Engine, code: &str) -> String {
    e.execute(code)
        .unwrap_or_else(|s| panic!("{code}: {s}"))
        .output
        .join("")
}
fn complex_values(e: &mut Engine, code: &str) -> Vec<Complex64> {
    run(e, &format!("zz={code};"));
    match e.workspace().get("zz").unwrap() {
        Array::Complex(c) => c.data().to_vec(),
        Array::Numeric(m) => m.data().iter().map(|v| Complex64::new(*v, 0.)).collect(),
        a => panic!("{code}: got {}", a.class()),
    }
}
fn values(e: &mut Engine, code: &str) -> Vec<f64> {
    run(e, &format!("zz={code};"));
    match e.workspace().get("zz").unwrap() {
        Array::Numeric(m) => m.data().to_vec(),
        a => panic!("{code}: got {}", a.class()),
    }
}
fn scalar(e: &mut Engine, code: &str) -> f64 {
    match values(e, code).as_slice() {
        [v] => *v,
        v => panic!("{code}: {} values", v.len()),
    }
}
fn dims(e: &mut Engine, name: &str) -> Vec<usize> {
    e.workspace().get(name).unwrap().dims().to_vec()
}
fn error(code: &str) -> String {
    Engine::default()
        .execute(code)
        .err()
        .unwrap_or_else(|| panic!("{code}: expected an error"))
}
/// |got - want| <= tol * max(1, |want|) element by element.
fn all_close(got: &[f64], want: &[f64], tol: f64, what: &str) {
    assert_eq!(got.len(), want.len(), "{what}: lengths");
    for (i, (g, w)) in got.iter().zip(want).enumerate() {
        assert!(
            (g - w).abs() <= tol * w.abs().max(1.),
            "{what}[{i}]: got {g:.17e}, want {w:.17e}"
        );
    }
}
/// Every `step`-th element in column-major order (real parts, then imaginary
/// parts) and the sum of |v|^2, as the reference script.
fn probe(v: &[Complex64], step: usize) -> Vec<f64> {
    let mut p: Vec<f64> = v.iter().step_by(step).map(|c| c.re).collect();
    p.extend(v.iter().step_by(step).map(|c| c.im));
    p.push(v.iter().map(|c| c.norm_sqr()).sum());
    p
}
/// [count, first, second, last, sum] of a vector, as the reference script.
fn summary(v: &[f64]) -> Vec<f64> {
    vec![v.len() as f64, v[0], v[1], v[v.len() - 1], v.iter().sum()]
}

#[test]
fn wavelet_constants_match_the_published_closed_forms() {
    // CONST = [peak frequency, time width, Nyquist cutoff frequency, admissibility].
    let cases: [(&str, &[f64; 4], f64, f64); 5] = [
        ("", &CONST_MORSE, 10., ICWT_ONES[0]),
        (", 'amor'", &CONST_AMOR, 10., ICWT_ONES[1]),
        (
            ", 'bump', 'VoicesPerOctave', 16",
            &CONST_BUMP,
            16.,
            ICWT_ONES[2],
        ),
        (
            ", 'TimeBandwidth', 30",
            &CONST_MORSE_TB30,
            10.,
            ICWT_ONES[3],
        ),
        (
            ", 'WaveletParameters', [2 20]",
            &CONST_MORSE_2_20,
            10.,
            ICWT_ONES[4],
        ),
    ];
    let pi = std::f64::consts::PI;
    for (options, c, nv, ones) in cases {
        let mut e = Engine::default();
        run(
            &mut e,
            &format!("x = cos(0.3*(1:1000)); [wt, f, coi] = cwt(x{options});"),
        );
        // The smallest scale s0 = wc/pi has the centre frequency w0/(2 pi s0).
        let f = values(&mut e, "f");
        all_close(&[f[0]], &[c[0] / (2. * c[2])], 1e-10, options);
        // One time width from the nearer edge: w0 sigma_t/(2 pi d) at d = N/2.
        let coi = values(&mut e, "coi");
        all_close(&[coi[499]], &[c[0] * c[1] / (pi * 1000.)], 1e-10, options);
        // Two time widths of the largest wavelet fit the signal.
        let last = *f.last().unwrap();
        let bound = c[0] * c[1] / (pi * 1000.);
        assert!(last >= bound * (1. - 1e-12) && last < bound * 2f64.powf(1. / nv));
        // icwt of ones: 2 log(2) rows/(NV C).
        let got = values(&mut e, &format!("icwt(ones(5, 4){options})"));
        all_close(&got, &[ones; 4], 1e-10, options);
        let want = 2. * std::f64::consts::LN_2 * 5. / (nv * c[3]);
        all_close(&[got[0]], &[want], 1e-10, options);
    }
    // Morse closed forms for the default (gamma = 3, beta = 20): peak
    // (beta/gamma)^(1/gamma) and admissibility 2 e^(b/g) w0^(-b) Gamma(b/g)/g.
    let w0 = (20f64 / 3.).powf(1. / 3.);
    all_close(&[CONST_MORSE[0]], &[w0], 1e-13, "morse peak");
    // Gamma(20/3) = 389.0349262461801 (mpmath), not recomputed here.
    let c = 2. * (20f64 / 3.).exp() * w0.powf(-20.) * 389.0349262461801 / 3.;
    all_close(&[CONST_MORSE[3]], &[c], 1e-10, "morse admissibility");
    all_close(
        &[CONST_AMOR[2]],
        &[6. + (2. * 10f64.ln()).sqrt()],
        1e-13,
        "amor cutoff",
    );
}

#[test]
fn scales_and_frequencies_follow_the_formulas() {
    let cases: [(&str, &[f64; 5], f64); 12] = [
        ("x", &GRID_MORSE_1000, 10.),
        ("x, 'amor'", &GRID_AMOR_1000, 10.),
        ("x, 'bump'", &GRID_BUMP_1000, 10.),
        ("x, 1000", &GRID_MORSE_FS, 10.),
        ("x, 'VoicesPerOctave', 16", &GRID_MORSE_NV16, 16.),
        (
            "x, 1000, 'FrequencyLimits', [20 200]",
            &GRID_MORSE_LIMITS,
            10.,
        ),
        (
            "x, 1000, 'FrequencyLimits', [0.001 2000]",
            &GRID_MORSE_CLIPPED,
            10.,
        ),
        (
            "x, 'amor', 1000, 'VoicesPerOctave', 12, 'FrequencyLimits', [10 80]",
            &GRID_AMOR_LIMITS,
            12.,
        ),
        ("x, 'TimeBandwidth', 30", &GRID_MORSE_TB30, 10.),
        ("x, 'WaveletParameters', [2 20]", &GRID_MORSE_2_20, 10.),
        ("x(1:255)'", &GRID_MORSE_255, 10.),
        ("[x x(1:24)]", &GRID_MORSE_1024, 10.),
    ];
    let mut e = Engine::default();
    run(&mut e, "x = cos(0.3*(1:1000)) + 0.01*(1:1000);");
    for (args, want, nv) in cases {
        run(&mut e, &format!("[wt, f] = cwt({args});"));
        let f = values(&mut e, "f");
        all_close(&summary(&f), want, 1e-10, args);
        // Geometric: NV voices per octave, from high to low frequency.
        for pair in f.windows(2) {
            let ratio = pair[0] / pair[1];
            assert!(
                (ratio - 2f64.powf(1. / nv)).abs() < 1e-12,
                "{args}: {ratio}"
            );
        }
        let n = scalar(
            &mut e,
            &format!("numel({})", args.split(',').next().unwrap()),
        );
        assert_eq!(dims(&mut e, "f"), [f.len(), 1], "{args}");
        assert_eq!(dims(&mut e, "wt"), [f.len(), n as usize], "{args}");
    }
    // 71 scales for 1000 samples: floor(10 log2(smax/s0)) + 1 with
    // smax = N/(2 sigma_t) and s0 = wc/pi.
    let (w0, sigma, wc) = (CONST_MORSE[0], CONST_MORSE[1], CONST_MORSE[2]);
    let pi = std::f64::consts::PI;
    let count = (10. * ((1000. / (2. * sigma)) / (wc / pi)).log2()).floor() + 1.;
    assert_eq!(count, 71.);
    assert_eq!(GRID_MORSE_1000[0], 71.);
    // scales(fb): s0 2^(k/NV) in samples, a row; frequencies are w0/(2 pi s) fs.
    run(
        &mut e,
        "fb = cwtfilterbank; s = scales(fb); cf = centerFrequencies(fb); cp = centerPeriods(fb);",
    );
    let s = values(&mut e, "s");
    all_close(&summary(&s), &SCALES_MORSE_1024, 1e-10, "scales");
    all_close(&[s[0]], &[wc / pi], 1e-10, "smallest scale");
    assert_eq!(dims(&mut e, "s"), [1, 71]);
    assert_eq!(dims(&mut e, "cf"), [71, 1]);
    let cf = values(&mut e, "cf");
    let cp = values(&mut e, "cp");
    for k in 0..71 {
        let want = w0 / (2. * pi * s[k]);
        assert!((cf[k] - want).abs() <= 1e-12 * want, "cf[{k}]");
        assert!((cp[k] * cf[k] - 1.).abs() < 1e-12, "cp[{k}]");
    }
    all_close(&summary(&cf), &GRID_MORSE_1024, 1e-10, "centerFrequencies");
    run(
        &mut e,
        "fb = cwtfilterbank('SignalLength', 1000, 'Wavelet', 'amor', 'SamplingFrequency', 1000, \
         'VoicesPerOctave', 12, 'FrequencyLimits', [10 80]); s = scales(fb);",
    );
    all_close(
        &summary(&values(&mut e, "s")),
        &SCALES_AMOR_LIMITS,
        1e-10,
        "amor scales",
    );
}

#[test]
fn cwt_matches_the_numpy_reference() {
    let cases: [(&str, &[f64]); 5] = [
        ("x", &CWT_MORSE),
        ("x, 'amor'", &CWT_AMOR),
        ("x', 'bump'", &CWT_BUMP),
        (
            "x, 256, 'VoicesPerOctave', 12, 'FrequencyLimits', [8 100], 'TimeBandwidth', 40",
            &CWT_MORSE_OPTS,
        ),
        ("x, 'ExtendSignal', false", &CWT_MORSE_PERIODIC),
    ];
    let mut e = Engine::default();
    run(&mut e, SIG256);
    for (args, want) in cases {
        let wt = complex_values(&mut e, &format!("cwt({args})"));
        all_close(&probe(&wt, 97), want, 1e-10, args);
    }
    // One output, 'morse' named, and the options in another order agree.
    let a = complex_values(&mut e, "cwt(x, 'morse', 'VoicesPerOctave', 10)");
    all_close(&probe(&a, 97), &CWT_MORSE, 1e-10, "named morse");
    // The sampling frequency changes f and coi, not the coefficients.
    let b = complex_values(&mut e, "cwt(x, \"morse\", 512)");
    assert_eq!(a, b);
}

#[test]
fn pure_tone_gives_a_ridge_at_its_frequency() {
    let mut e = Engine::default();
    run(&mut e, "t = (0:999)/1000; x = 3*cos(2*pi*100*t + 0.4);");
    for (args, nv) in [
        ("x, 1000", 10.),
        ("x, 'amor', 1000", 10.),
        ("x, 'bump', 1000", 10.),
        ("x, 1000, 'VoicesPerOctave', 24", 24.),
        ("x, 'bump', 1000, 'VoicesPerOctave', 48", 48.),
    ] {
        run(
            &mut e,
            &format!("[wt, f] = cwt({args}); [m, k] = max(abs(wt(:, 500)));"),
        );
        let ridge = scalar(&mut e, "f(k)");
        // Within one voice of the tone; the L1 norm gives its amplitude.
        assert!((ridge / 100.).log2().abs() <= 1. / nv, "{args}: {ridge}");
        let m = scalar(&mut e, "m");
        assert!((m - 3.).abs() < 0.05, "{args}: {m}");
        // The ridge is there at every time away from the edges.
        run(&mut e, "[~, kk] = max(abs(wt(:, 200:800)));");
        assert_eq!(scalar(&mut e, "double(all(kk == k))"), 1., "{args}");
    }
}

#[test]
fn cone_of_influence_is_one_time_width_from_each_edge() {
    let cases: [(&str, &[f64; 7], usize); 4] = [
        ("x, 1000", &COI_MORSE, 1000),
        ("x, 'amor', 1000", &COI_AMOR, 1000),
        ("x, 'bump', 1000", &COI_BUMP, 1000),
        ("x(1:255), 1000", &COI_MORSE_ODD, 255),
    ];
    let mut e = Engine::default();
    run(&mut e, "x = cos(0.3*(1:1000));");
    for (args, want, n) in cases {
        run(&mut e, &format!("[wt, f, coi] = cwt({args});"));
        assert_eq!(dims(&mut e, "coi"), [n, 1], "{args}");
        let c = values(&mut e, "coi");
        let got = [
            c[0],
            c[1],
            c[10],
            c[n / 2 - 1],
            c[n / 2],
            c[n - 1],
            c.iter().sum(),
        ];
        all_close(&got, want, 1e-10, args);
        // Symmetric, never above the highest frequency, falling towards the middle.
        let fmax = scalar(&mut e, "f(1)");
        for i in 0..n {
            assert_eq!(c[i], c[n - 1 - i], "{args}: symmetry at {i}");
            assert!(c[i] <= fmax);
        }
        assert!(c[..n / 2].windows(2).all(|w| w[1] <= w[0]));
    }
    // Closed form: w0 sigma_t fs/(2 pi d), d = distance to the nearer edge in samples.
    let k = CONST_MORSE[0] * CONST_MORSE[1] * 1000. / (2. * std::f64::consts::PI);
    all_close(
        &[COI_MORSE[2], COI_MORSE[3]],
        &[k / 11., k / 500.],
        1e-12,
        "closed form",
    );
    // Without a sampling frequency the cone is in cycles per sample.
    run(
        &mut e,
        "[~, ~, coi1] = cwt(x); [~, ~, coi2] = cwt(x, 1000);",
    );
    let ratio = values(&mut e, "coi2./coi1");
    assert!(ratio.iter().all(|r| (r - 1000.).abs() < 1e-9));
}

#[test]
fn complex_signal_gives_the_analytic_and_anti_analytic_parts() {
    let mut e = Engine::default();
    run(&mut e, Z256);
    run(&mut e, "[wz, f, coi] = cwt(z);");
    let na = values(&mut e, "f").len();
    assert_eq!(dims(&mut e, "wz"), [na, 256, 2]);
    let wz = complex_values(&mut e, "wz");
    all_close(&probe(&wz, 197), &CWT_COMPLEX, 1e-10, "complex cwt");
    // Page 1 holds the counterclockwise component (amplitude 1 at 20/256),
    // page 2 the clockwise one (amplitude 0.5 at 45/256).
    run(
        &mut e,
        "[m1, k1] = max(abs(wz(:, 128, 1))); [m2, k2] = max(abs(wz(:, 128, 2)));",
    );
    let (m1, m2) = (scalar(&mut e, "m1"), scalar(&mut e, "m2"));
    assert!(
        (m1 - 1.).abs() < 0.02 && (m2 - 0.5).abs() < 0.01,
        "{m1} {m2}"
    );
    assert!((scalar(&mut e, "f(k1)") * 256. / 20.).log2().abs() <= 0.1);
    assert!((scalar(&mut e, "f(k2)") * 256. / 45.).log2().abs() <= 0.1);
    // A real signal as a complex one: page 1 is half the real transform and
    // page 2 its conjugate.
    run(&mut e, SIG256);
    run(
        &mut e,
        "w = cwt(x); wc = cwt(complex(x, 0)); d1 = max(max(abs(wc(:, :, 1) - w/2))); \
         d2 = max(max(abs(wc(:, :, 2) - conj(w)/2)));",
    );
    assert!(scalar(&mut e, "d1") < 1e-13);
    assert!(scalar(&mut e, "d2") < 1e-13);
    // The inverse of a 3-D transform is complex.
    let zr = complex_values(&mut e, "icwt(wz)");
    all_close(&probe(&zr, 16), &ICWT_COMPLEX, 1e-10, "complex icwt");
    assert_eq!(dims(&mut e, "zz"), [1, 256]);
}

#[test]
fn icwt_reconstructs_the_signal() {
    let mut e = Engine::default();
    run(&mut e, SIG256);
    run(&mut e, "[wt, f] = cwt(x); xr = icwt(wt);");
    assert_eq!(dims(&mut e, "xr"), [1, 256]);
    let xr = complex_values(&mut e, "xr");
    all_close(&probe(&xr, 16), &ICWT_MORSE, 1e-10, "icwt");
    let xm = values(&mut e, "icwt(wt, 'SignalMean', 0.75)");
    let picked: Vec<f64> = xm.iter().step_by(64).copied().collect();
    all_close(&picked, &ICWT_MORSE_MEAN, 1e-10, "SignalMean");
    let xf = values(&mut e, "icwt(wt, f, [0.05 0.12])");
    let picked: Vec<f64> = xf.iter().step_by(16).copied().collect();
    all_close(&picked, &ICWT_MORSE_RANGE, 1e-10, "frequency range");
    let same = values(
        &mut e,
        "icwt(wt, 'morse', f, [0.05 0.12], 'VoicesPerOctave', 10)",
    );
    assert_eq!(xf, same);
    // A column signal also reconstructs as a row.
    run(&mut e, "xc = icwt(cwt(x'));");
    assert_eq!(dims(&mut e, "xc"), [1, 256]);

    // Relative error of the (approximate) inverse for content inside the
    // analysed band: the reference script reports 1.9e-7 (morse), 3.7e-6
    // (amor), 4e-2 (bump at the default 10 voices) and 2.9e-4 (bump at 48).
    run(&mut e, PACKETS);
    for (cwt_args, icwt_args, bound) in [
        ("x", "", 1e-5),
        ("x, 'amor'", ", 'amor'", 1e-4),
        ("x, 'bump'", ", 'bump'", 5e-2),
        (
            "x, 'bump', 'VoicesPerOctave', 48",
            ", 'bump', 'VoicesPerOctave', 48",
            1e-2,
        ),
        ("x, 'TimeBandwidth', 30", ", 'TimeBandwidth', 30", 1e-2),
        (
            "x, 2000, 'VoicesPerOctave', 16",
            ", 'VoicesPerOctave', 16",
            1e-2,
        ),
    ] {
        let err = scalar(
            &mut e,
            &format!("norm(icwt(cwt({cwt_args}){icwt_args}) - x)/norm(x)"),
        );
        assert!(err < bound, "{cwt_args}: {err}");
    }
    // The filter bank carries the wavelet and the voices.
    run(
        &mut e,
        "[wt, ~, ~, fb] = cwt(x, 'amor', 'VoicesPerOctave', 20); \
         xa = icwt(wt, 'AnalysisFilterBank', fb); xb = icwt(wt, 'amor', 'VoicesPerOctave', 20);",
    );
    assert_eq!(values(&mut e, "xa"), values(&mut e, "xb"));
    assert!(scalar(&mut e, "norm(xa - x)/norm(x)") < 1e-4);
    // The mean is not kept by the transform: SignalMean restores it.
    run(&mut e, "y = x + 2;");
    let err = scalar(
        &mut e,
        "norm(icwt(cwt(y), 'SignalMean', mean(y)) - y)/norm(y)",
    );
    assert!(err < 1e-2, "{err}");
}

#[test]
fn filter_bank_object_and_methods() {
    let mut e = Engine::default();
    let shown = run(&mut e, "fb = cwtfilterbank");
    assert!(shown.contains("cwtfilterbank with properties"), "{shown}");
    for property in [
        "VoicesPerOctave: 10",
        "Wavelet: 'Morse'",
        "SamplingFrequency: 1",
        "SignalLength: 1024",
        "TimeBandwidth: 60",
        "Boundary: 'reflection'",
    ] {
        assert!(shown.contains(property), "{property} in {shown}");
    }
    assert!(!shown.contains("MatRust"), "{shown}");
    assert_eq!(run(&mut e, "disp(class(fb))").trim(), "cwtfilterbank");
    assert_eq!(scalar(&mut e, "double(isa(fb, 'cwtfilterbank'))"), 1.);
    assert_eq!(scalar(&mut e, "double(isstruct(fb))"), 0.);
    assert_eq!(scalar(&mut e, "fb.SignalLength"), 1024.);
    assert!(values(&mut e, "fb.FrequencyLimits").is_empty());
    let message = e.execute("fb.VoicesPerOctave = 12;").unwrap_err();
    assert!(message.contains("read-only"), "{message}");

    // wt(fb, x) is cwt(x); cwt returns the bank it used and accepts one.
    run(&mut e, SIG256);
    run(
        &mut e,
        "fb = cwtfilterbank('SignalLength', 256, 'Wavelet', 'amor', 'VoicesPerOctave', 12, \
         'SamplingFrequency', 256, 'FrequencyLimits', [8 100]); \
         [w1, f1, c1] = wt(fb, x); \
         [w2, f2, c2, fb2] = cwt(x, 'amor', 256, 'VoicesPerOctave', 12, 'FrequencyLimits', [8 100]); \
         [w3, f3, c3] = cwt(x, 'FilterBank', fb); w4 = wt(fb2, x');",
    );
    let w1 = complex_values(&mut e, "w1");
    assert_eq!(w1, complex_values(&mut e, "w2"));
    assert_eq!(w1, complex_values(&mut e, "w3"));
    assert_eq!(w1, complex_values(&mut e, "w4"));
    assert_eq!(values(&mut e, "f1"), values(&mut e, "f2"));
    assert_eq!(values(&mut e, "c1"), values(&mut e, "c3"));
    assert_eq!(run(&mut e, "disp(fb2.Wavelet)").trim(), "amor");
    assert_eq!(values(&mut e, "fb2.FrequencyLimits"), [8., 100.]);
    assert_eq!(scalar(&mut e, "fb2.VoicesPerOctave"), 12.);
    assert_eq!(scalar(&mut e, "fb2.SamplingFrequency"), 256.);
    run(
        &mut e,
        "[~, ~, ~, fb3] = cwt(x, 'WaveletParameters', [2 20], 'ExtendSignal', false);",
    );
    assert_eq!(values(&mut e, "fb3.WaveletParameters"), [2., 20.]);
    assert_eq!(scalar(&mut e, "fb3.TimeBandwidth"), 20.);
    assert_eq!(run(&mut e, "disp(fb3.Boundary)").trim(), "periodic");
    assert_eq!(run(&mut e, "disp(fb3.Wavelet)").trim(), "Morse");

    // Frequency responses: Psi(s w) on the DFT grid of the transform.
    run(
        &mut e,
        "fp = cwtfilterbank('SignalLength', 64, 'Boundary', 'periodic'); [hp, wp] = freqz(fp); \
         fr = cwtfilterbank('SignalLength', 64, 'SamplingFrequency', 10); [hr, wr] = freqz(fr); \
         [h2, w2] = freqz(fr, 'FrequencyRange', 'twosided'); s = scales(fp);",
    );
    let na = values(&mut e, "s").len();
    assert_eq!(dims(&mut e, "hp"), [na, 33]);
    assert_eq!(dims(&mut e, "wp"), [1, 33]);
    assert_eq!(dims(&mut e, "hr"), [na, 65]);
    assert_eq!(dims(&mut e, "h2"), [na, 128]);
    all_close(
        &probe(&complex_values(&mut e, "hp"), 13),
        &FREQZ_PERIODIC,
        1e-10,
        "freqz",
    );
    all_close(
        &probe(&complex_values(&mut e, "hr"), 29),
        &FREQZ_REFLECTED,
        1e-10,
        "freqz reflected",
    );
    let wp = values(&mut e, "wp");
    assert!(
        wp.iter()
            .enumerate()
            .all(|(k, w)| (w - k as f64 / 64.).abs() < 1e-15)
    );
    let wr = values(&mut e, "wr");
    assert!(
        wr.iter()
            .enumerate()
            .all(|(k, w)| (w - k as f64 * 10. / 128.).abs() < 1e-13)
    );
    assert_eq!(scalar(&mut e, "max(max(abs(h2(:, 1:65) - hr)))"), 0.);
    assert_eq!(scalar(&mut e, "max(max(abs(h2(:, 66:128))))"), 0.);
    // Closed form (Lilly & Olhede): 2 (w/w0)^beta exp(w0^gamma - w^gamma), peak 2.
    let (s, hp) = (values(&mut e, "s"), values(&mut e, "hp"));
    let w0 = CONST_MORSE[0];
    for k in 1..33 {
        for (j, sj) in s.iter().enumerate() {
            let w = sj * 2. * std::f64::consts::PI * k as f64 / 64.;
            let want = 2. * (w / w0).powf(20.) * (w0.powi(3) - w.powi(3)).exp();
            assert!((hp[j + na * k] - want).abs() < 1e-12, "psi({j}, {k})");
        }
    }
    assert!(scalar(&mut e, "max(hp(:))") <= 2.);
    // A periodic tone on a DFT bin: wt(j, t) = Psi(s_j w1) exp(i w1 t)/2 * 2/2... the
    // analytic filter keeps the positive-frequency half A/2 and doubles it.
    run(
        &mut e,
        "tt = 0:63; y = 1.5*cos(2*pi*5*tt/64 + 0.7); wy = wt(fp, y); \
         want = hp(:, 6) * (0.75*exp(1i*(2*pi*5*tt/64 + 0.7))); d = max(max(abs(wy - want)));",
    );
    assert!(scalar(&mut e, "d") < 1e-13);

    // Time-domain wavelets: inverse DFT of the responses, time zero at column N/2 + 1.
    run(
        &mut e,
        "[psi, tp] = wavelets(fp); fbb = cwtfilterbank('SignalLength', 64, 'Wavelet', 'bump'); psib = wavelets(fbb);",
    );
    assert_eq!(dims(&mut e, "psi"), [na, 64]);
    assert_eq!(dims(&mut e, "tp"), [1, 64]);
    all_close(
        &probe(&complex_values(&mut e, "psi"), 37),
        &WAVELETS_MORSE,
        1e-10,
        "wavelets",
    );
    all_close(
        &probe(&complex_values(&mut e, "psib"), 37),
        &WAVELETS_BUMP,
        1e-10,
        "bump wavelets",
    );
    let tp = values(&mut e, "tp");
    assert!(tp.iter().enumerate().all(|(i, t)| *t == i as f64 - 32.));
    // The wavelet peaks at time zero where it equals the mean of its response.
    run(&mut e, "[~, kk] = max(abs(psi(1, :)));");
    let peak = scalar(&mut e, "tp(kk)");
    assert_eq!(peak, 0.);
    assert!(scalar(&mut e, "abs(psi(1, 33) - sum(hp(1, :))/64)") < 1e-14);
    run(&mut e, "[~, tr] = wavelets(fr);");
    let tr = values(&mut e, "tr");
    assert!((tr[0] + 3.2).abs() < 1e-15 && (tr[63] - 3.1).abs() < 1e-15);
}

#[test]
fn wsst_sharpens_and_iwsst_inverts() {
    let mut e = Engine::default();
    // Frequencies: log-spaced from fs/N to fs/2, NV (floor(log2 N) - 1) of them.
    run(
        &mut e,
        "t = (0:1023)/1024; x = cos(2*pi*40*t); [sst, f] = wsst(x, 1024);",
    );
    assert_eq!(dims(&mut e, "sst"), [288, 1024]);
    assert_eq!(dims(&mut e, "f"), [1, 288]);
    let f = values(&mut e, "f");
    for (l, v) in f.iter().enumerate() {
        let want = 2f64.powf(9. * l as f64 / 287.);
        assert!((v - want).abs() <= 1e-12 * want, "f[{l}] = {v}");
    }
    assert_eq!(f[287], 512.);
    // The tone is squeezed into the bin nearest to 40 Hz (within one voice),
    // at every time.
    run(&mut e, "[m, k] = max(abs(sst)); kk = unique(k);");
    let k = values(&mut e, "kk");
    assert_eq!(k.len(), 1);
    let ridge = f[k[0] as usize - 1];
    assert!((ridge / 40.).log2().abs() <= 1. / 32., "{ridge}");
    assert!(
        (ridge / 40.).log2().abs() <= 0.5 * 9. / 287. + 1e-12,
        "{ridge}"
    );
    // All the energy is on the ridge: amplitude A gives A C/2 there.
    let c_amor = CONST_AMOR[3];
    let m = values(&mut e, "m");
    assert!(m.iter().all(|v| (v - c_amor / 2.).abs() < 1e-6), "{}", m[0]);
    assert!(scalar(&mut e, "sum(sum(abs(sst) > 1e-9)) - 1024") == 0.);
    let cwt_rows = scalar(&mut e, "sum(abs(cwt(x, 'amor')) > 0.01)*ones(1024, 1)/1024");
    assert!(
        cwt_rows > 10.,
        "the cwt spreads the tone over {cwt_rows} rows"
    );

    let wsst_probe = |e: &mut Engine, call: &str| -> Vec<f64> {
        run(
            e,
            &format!("sst = {call}; s = sum(sst, 1); m = max(abs(sst), [], 1);"),
        );
        values(
            e,
            "[real(s(1:64:end)) imag(s(1:64:end)) m(1:64:end) sum(abs(sst(:)).^2)]",
        )
    };
    all_close(
        &wsst_probe(&mut e, "wsst(x)"),
        &WSST_TONE_AMOR,
        1e-10,
        "tone",
    );
    run(&mut e, TONES);
    all_close(
        &wsst_probe(&mut e, "wsst(x)"),
        &WSST_TONES_AMOR,
        1e-10,
        "tones",
    );
    all_close(
        &wsst_probe(&mut e, "wsst(x', 'bump')"),
        &WSST_TONES_BUMP,
        1e-10,
        "bump",
    );
    run(&mut e, "xt = x;");
    run(&mut e, SIG256);
    all_close(
        &wsst_probe(
            &mut e,
            "wsst(x, 'VoicesPerOctave', 16, 'ExtendSignal', true)",
        ),
        &WSST_SIG256_NV16_EXT,
        1e-10,
        "options",
    );
    assert_eq!(dims(&mut e, "sst"), [16 * 7, 256]);

    // Inverse: relative error below 1e-6 for the default wavelet (the
    // reference script reports about 2e-7), a column vector.
    for signal in [
        "t = (0:1023)/1024; x = cos(2*pi*40*t);",
        TONES,
        PACKETS,
        "t = (0:1023)/1024; x = 5 + 1e-3*cos(2*pi*60*t + 1);",
    ] {
        run(&mut e, signal);
        run(&mut e, "xr = iwsst(wsst(x)); xm = x - mean(x);");
        assert_eq!(dims(&mut e, "xr"), [1024, 1]);
        let err = scalar(&mut e, "norm(xr' - xm)/norm(xm)");
        assert!(err < 1e-6, "{signal}: {err}");
    }
    run(&mut e, TONES);
    let err = scalar(
        &mut e,
        "norm(iwsst(wsst(x, 1024, 'amor'), 'amor')' - x)/norm(x)",
    );
    assert!(err < 1e-6, "{err}");
    // The bump wavelet is sampled coarsely by the scales: 1.3e-3 with 32
    // voices and 1.7e-4 with 48 (reference script). The bounds stay at 2e-3
    // and 3e-4; amor above stays under 1e-6.
    let err = scalar(&mut e, "norm(iwsst(wsst(x, 'bump'), 'bump')' - x)/norm(x)");
    assert!(err < 2e-3, "{err}");
    let err = scalar(
        &mut e,
        "norm(iwsst(wsst(x, 'bump', 'VoicesPerOctave', 48), 'bump')' - x)/norm(x)",
    );
    assert!(err < 3e-4, "{err}");

    // Frequency range and ridge reconstructions.
    run(
        &mut e,
        "[sst, f] = wsst(x, 1024); xh = iwsst(sst, f, [60 130]);",
    );
    let xh: Vec<f64> = values(&mut e, "xh").iter().step_by(64).copied().collect();
    all_close(&xh, &IWSST_RANGE, 1e-10, "frequency range");
    let err = scalar(
        &mut e,
        "norm(xh' - 0.5*sin(2*pi*90*t))/norm(0.5*sin(2*pi*90*t))",
    );
    // The scales between the two tones mix them (reference script: 3.4e-2).
    assert!(err < 5e-2, "{err}");
    run(
        &mut e,
        "[~, ir] = max(abs(sst)); x1 = iwsst(sst, ir'); x2 = iwsst(sst, ir, 'NumFrequencyBins', 4); \
         x3 = iwsst(sst, [ir' ir'], 'amor');",
    );
    assert_eq!(dims(&mut e, "x1"), [1024, 1]);
    assert_eq!(dims(&mut e, "x3"), [1024, 2]);
    let pick = |v: Vec<f64>| -> Vec<f64> { v.iter().step_by(64).copied().collect() };
    all_close(&pick(values(&mut e, "x1")), &IWSST_RIDGE, 1e-10, "ridge");
    all_close(
        &pick(values(&mut e, "x2")),
        &IWSST_RIDGE_4,
        1e-10,
        "ridge, 4 bins",
    );
    assert_eq!(values(&mut e, "x3(:, 2)"), values(&mut e, "x1"));
    // 2 real(sum of the rows)/C.
    all_close(
        &values(&mut e, "[iwsst(ones(3, 2)); iwsst(ones(3, 2), 'bump')]"),
        &[IWSST_ONES[0], IWSST_ONES[0], IWSST_ONES[1], IWSST_ONES[1]],
        1e-10,
        "ones",
    );
    // A constant signal has no coefficients above the threshold.
    assert_eq!(scalar(&mut e, "max(max(abs(wsst(3*ones(1, 64)))))"), 0.);
}

#[test]
fn errors_name_the_function_and_the_argument() {
    for (code, part) in [
        ("cwt()", "cwt: not enough input arguments"),
        (
            "cwt([1 2 3])",
            "cwt: the signal must be a vector with at least 4 samples",
        ),
        (
            "cwt(ones(4, 4))",
            "cwt: the signal must be a vector with at least 4 samples",
        ),
        (
            "cwt([1 2 NaN 4 5])",
            "cwt: the signal must contain finite values",
        ),
        (
            "cwt('abcdef')",
            "cwt: the signal must be a real or complex double vector",
        ),
        ("cwt(1:64, 'haar')", "cwt: unknown wavelet or option 'haar'"),
        (
            "cwt(1:64, 'morse', 'Voices', 12)",
            "cwt: unknown option 'Voices'",
        ),
        (
            "cwt(1:64, 0)",
            "cwt: the sampling frequency must be a positive scalar",
        ),
        (
            "cwt(1:64, [1 2])",
            "cwt: the sampling frequency must be a positive scalar",
        ),
        (
            "cwt(1:64, 'VoicesPerOctave', 2.5)",
            "cwt: VoicesPerOctave must be an integer between 1 and 48",
        ),
        (
            "cwt(1:64, 'VoicesPerOctave', 64)",
            "cwt: VoicesPerOctave must be an integer between 1 and 48",
        ),
        (
            "cwt(1:64, 'VoicesPerOctave')",
            "cwt: option 'VoicesPerOctave' needs a value",
        ),
        (
            "cwt(1:64, 'TimeBandwidth', 2)",
            "cwt: TimeBandwidth must be a scalar between 3 and 120",
        ),
        (
            "cwt(1:64, 'amor', 'TimeBandwidth', 30)",
            "cwt: TimeBandwidth and WaveletParameters apply only to the Morse wavelet",
        ),
        (
            "cwt(1:64, 'TimeBandwidth', 30, 'WaveletParameters', [3 30])",
            "cwt: specify TimeBandwidth or WaveletParameters, not both",
        ),
        (
            "cwt(1:64, 'WaveletParameters', [3 200])",
            "cwt: WaveletParameters must be [gamma, P2]",
        ),
        (
            "cwt(1:64, 'FrequencyLimits', [0.2 0.1])",
            "cwt: FrequencyLimits must be two positive, strictly increasing frequencies",
        ),
        (
            "cwt(1:64, 'FrequencyLimits', [0.2 0.201])",
            "cwt: FrequencyLimits must cover at least 1/VoicesPerOctave octaves",
        ),
        (
            "cwt(1:64, 'FrequencyLimits', [2 3])",
            "cwt: FrequencyLimits must cover at least 1/VoicesPerOctave octaves",
        ),
        (
            "cwt(1:64, 'ExtendSignal', 2)",
            "cwt: ExtendSignal must be true or false",
        ),
        (
            "cwt(1:64, 'FilterBank', 3)",
            "cwt: expected a cwtfilterbank object",
        ),
        (
            "cwt(1:64, 'FilterBank', cwtfilterbank)",
            "cwt: the signal length must equal the SignalLength of the filter bank (1024)",
        ),
        (
            "cwt(1:1024, 'amor', 'FilterBank', cwtfilterbank)",
            "cwt: FilterBank cannot be combined",
        ),
        ("cwt(1:64, seconds(1))", "cwt: duration"),
        (
            "[a, b, c, d, s] = cwt(1:64)",
            "cwt: the scaling coefficients output is not supported yet",
        ),
        (
            "[a, b, c, d, s, g] = cwt(1:64)",
            "cwt: too many output arguments",
        ),
        ("icwt()", "icwt: not enough input arguments"),
        (
            "icwt('abc')",
            "icwt: the coefficients must be a matrix or a 3-D array with two pages",
        ),
        (
            "icwt(ones(2, 3, 3))",
            "icwt: the coefficients must be a matrix or a 3-D array with two pages",
        ),
        (
            "icwt(ones(3, 8), 'haar')",
            "icwt: unknown wavelet or option 'haar'",
        ),
        (
            "icwt(ones(3, 8), 'SignalMean', [1 2])",
            "icwt: SignalMean must be a finite scalar",
        ),
        (
            "icwt(ones(3, 8), 'Mean', 1)",
            "icwt: unknown wavelet or option 'Mean'",
        ),
        (
            "icwt(ones(3, 8), 'amor', 'Mean', 1)",
            "icwt: unknown option 'Mean'",
        ),
        (
            "icwt(ones(3, 8), [1 2 3])",
            "icwt: a frequency vector needs a frequency range",
        ),
        (
            "icwt(ones(3, 8), [1 2], [1 2])",
            "icwt: the frequency vector must have one element per row of the coefficients",
        ),
        (
            "icwt(ones(3, 8), [3 2 1], [2 1])",
            "icwt: the frequency range must be two increasing values",
        ),
        (
            "icwt(ones(3, 8), [3 2 1], [5 6])",
            "icwt: no frequency lies inside the frequency range",
        ),
        (
            "[a, b] = icwt(ones(3, 8))",
            "icwt: too many output arguments",
        ),
        (
            "cwtfilterbank('SignalLength', 3)",
            "cwtfilterbank: SignalLength must be an integer between 4 and",
        ),
        (
            "cwtfilterbank('Wavelet', 'db4')",
            "cwtfilterbank: unknown wavelet 'db4'",
        ),
        (
            "cwtfilterbank('Boundary', 'zero')",
            "cwtfilterbank: Boundary must be 'reflection' or 'periodic'",
        ),
        (
            "cwtfilterbank('Length', 64)",
            "cwtfilterbank: unknown option 'Length'",
        ),
        (
            "cwtfilterbank(64)",
            "cwtfilterbank: expected an option name, got a numeric argument",
        ),
        (
            "cwtfilterbank('SamplingPeriod', seconds(1))",
            "cwtfilterbank: duration",
        ),
        (
            "fb = cwtfilterbank; wt(fb, 1:64)",
            "wt: the signal length must equal the SignalLength of the filter bank (1024)",
        ),
        ("fb = cwtfilterbank; wt(fb)", "wt: expected wt(fb, x)"),
        (
            "fb = cwtfilterbank; scales(fb, 2)",
            "scales: too many input arguments",
        ),
        (
            "fb = cwtfilterbank; [a, b] = scales(fb)",
            "scales: too many output arguments",
        ),
        (
            "fb = cwtfilterbank; freqz(fb, 'FrequencyRange', 'half')",
            "freqz: FrequencyRange must be 'onesided' or 'twosided'",
        ),
        (
            "fb = cwtfilterbank; freqz(fb, 'IncludeLowpass', true)",
            "freqz: the lowpass (scaling) filter is not supported yet",
        ),
        (
            "fb = cwtfilterbank; freqz(fb, 'Points', 8)",
            "freqz: unknown option 'Points'",
        ),
        ("wsst()", "wsst: not enough input arguments"),
        (
            "wsst([1 2 3])",
            "wsst: the signal must be a vector with at least 4 samples",
        ),
        ("wsst(exp(1i*(1:64)))", "wsst: the signal must be real"),
        (
            "wsst(1:64, 'morse')",
            "wsst: unknown wavelet 'morse' (expected 'amor' or 'bump')",
        ),
        (
            "wsst(1:64, 'haar')",
            "wsst: unknown wavelet 'haar' (expected 'amor' or 'bump')",
        ),
        (
            "wsst(1:64, -1)",
            "wsst: the sampling frequency must be a positive scalar",
        ),
        (
            "wsst(1:64, 'VoicesPerOctave', 11)",
            "wsst: VoicesPerOctave must be an even integer between 10 and 48",
        ),
        ("wsst(1:64, 'Voices', 12)", "wsst: unknown option 'Voices'"),
        ("[a, b, c] = wsst(1:64)", "wsst: too many output arguments"),
        ("iwsst()", "iwsst: not enough input arguments"),
        (
            "iwsst(ones(4, 8), 'morse')",
            "iwsst: unknown wavelet 'morse' (expected 'amor' or 'bump')",
        ),
        (
            "iwsst(ones(4, 8), 1:3)",
            "iwsst: the ridge indices need one row per column of the transform (8)",
        ),
        (
            "iwsst(ones(4, 8), 5*ones(8, 1))",
            "iwsst: the ridge indices must be integers between 1 and 4",
        ),
        (
            "iwsst(ones(4, 8), 'NumFrequencyBins', 2)",
            "iwsst: NumFrequencyBins needs ridge indices",
        ),
        (
            "iwsst(ones(4, 8), ones(8, 1), 'NumFrequencyBins', 0)",
            "iwsst: NumFrequencyBins must be a positive integer",
        ),
        (
            "iwsst(ones(4, 8), [1 2 3], [1 2])",
            "iwsst: the frequency vector must have one element per row of the coefficients",
        ),
        (
            "iwsst(ones(4, 8), 1, 2, 3)",
            "iwsst: too many input arguments",
        ),
    ] {
        let message = error(code);
        assert!(message.contains(part), "{code}: {message}");
    }
    // Names that are only filter bank methods stay undefined for other values,
    // and freqz of a filter keeps its usual meaning.
    assert!(error("wt(1:8)").contains("wt"));
    assert!(error("scales(3)").contains("scales"));
    let mut e = Engine::default();
    run(&mut e, "[h, w] = freqz([1 1], 1, 8);");
    assert_eq!(scalar(&mut e, "numel(h)"), 8.);
    // A variable named like a method or a builtin shadows it.
    run(
        &mut e,
        "wt = cwt(cos(1:64)); a = wt(2, 3); scales = [4 5 6]; b = scales(2);",
    );
    assert_eq!(scalar(&mut e, "b"), 5.);
}

#[test]
fn cwt_size_limits_are_refused_before_allocation() {
    let mut e = Engine::default();
    // 2^22 samples fit the input (32 MiB). At 48 voices the coefficient
    // matrix does not fit the statement budget, so these calls must refuse
    // before allocating it.
    run(&mut e, "x = zeros(1, 2^22);");
    for call in [
        "cwt(x, 'VoicesPerOctave', 48)",
        "wsst(x, 'VoicesPerOctave', 48)",
        "cwtfilterbank('SignalLength', 2^22, 'VoicesPerOctave', 48)",
        "wt(cwtfilterbank('SignalLength', 2^22, 'VoicesPerOctave', 48), x)",
    ] {
        let start = std::time::Instant::now();
        let err = match e.execute(call) {
            Ok(_) => panic!("{call}: expected a limit error"),
            Err(err) => err,
        };
        let ms = start.elapsed().as_millis();
        assert!(ms < 1000, "{call} took {ms} ms: {err}");
        assert!(
            err.contains("evaluation budget")
                || err.contains("memory limit")
                || err.contains("element limit")
                || err.contains("exceeds"),
            "{call}: {err}"
        );
        assert!(!err.contains("Workspace"), "{call}: refused late: {err}");
        assert!(
            !err.contains("Cannot allocate"),
            "{call}: allocated before the check: {err}"
        );
    }
    // A scalar consumer prevents workspace accounting from being the check.
    // Disable the evaluation budget so only memory admission can refuse.
    let mut e = Engine::with_limits(matrust::engine::Limits {
        evaluation_units: None,
        memory_bytes: 1024 * 1024,
        ..matrust::engine::Limits::browser()
    });
    let err = e
        .execute("n = numel(cwtfilterbank('SignalLength', 4096, 'VoicesPerOctave', 48));")
        .unwrap_err();
    assert!(err.contains("memory limit"), "{err}");
    assert!(!err.contains("Workspace"), "refused late: {err}");
    run(&mut e, "fb = cwtfilterbank('SignalLength', 64);");
}

#[test]
fn review_b7_tiny_sampling_frequency_has_a_scale_grid() {
    let mut e = Engine::default();
    let started = std::time::Instant::now();
    run(
        &mut e,
        "[wt, f] = cwt(zeros(1,128), 1e-323); \
         fb = cwtfilterbank('SignalLength', 128, 'SamplingFrequency', 1e-323); \
         s = scales(fb);",
    );
    assert!(
        started.elapsed().as_millis() < 1000,
        "took {:?}",
        started.elapsed()
    );
    let wt = dims(&mut e, "wt");
    assert_eq!(wt.get(1).copied().unwrap_or(0), 128, "{wt:?}");
    assert!(wt.first().copied().unwrap_or(0) >= 1, "{wt:?}");
    let f = values(&mut e, "f");
    assert!(!f.is_empty());
    assert!(f.iter().all(|x| x.is_finite()), "{f:?}");
    let s = values(&mut e, "s");
    assert_eq!(s.len(), f.len());
    assert!(s.iter().all(|x| x.is_finite() && *x > 0.), "{s:?}");
    // Same cast, via FrequencyLimits, when the hertz bounds are not representable.
    let err = error("cwt(zeros(1,128), 1e-323, 'FrequencyLimits', [1 2])");
    assert!(
        err.contains("representable") || err.contains("FrequencyLimits"),
        "{err}"
    );
    // PeriodLimits is rejected before any scale cast.
    let err = error("cwt(zeros(1,128), 1e-323, 'PeriodLimits', [1 2])");
    assert!(err.contains("PeriodLimits"), "{err}");
}

#[test]
fn help_examples_run() {
    let text = include_str!("../assets/help/functions.md");
    let mut ran = 0;
    for name in ["cwt", "icwt", "cwtfilterbank", "wsst", "iwsst"] {
        let start = text
            .find(&format!("\n## {name}\n"))
            .unwrap_or_else(|| panic!("no help page for {name}"));
        let body = &text[start + 1..];
        let body = &body[..body[3..].find("\n## ").map_or(body.len(), |i| i + 3)];
        assert!(body.contains("Differences from MATLAB:"), "{name}");
        for line in body.lines().filter_map(|l| l.strip_prefix("Example:")) {
            let code = line.split("   %").next().unwrap().trim();
            Engine::default()
                .execute(code)
                .unwrap_or_else(|s| panic!("{name}: {code}: {s}"));
            ran += 1;
        }
    }
    assert!(ran >= 8, "{ran} examples");
    // The values quoted in the examples.
    let mut e = Engine::default();
    run(
        &mut e,
        "x = cos(2*pi*100*(0:999)/1000); [wt, f] = cwt(x, 'amor', 1000, 'VoicesPerOctave', 12, \
         'FrequencyLimits', [20 300]); [~, k] = max(abs(wt(:, 500)));",
    );
    assert!((scalar(&mut e, "f(k)") / 100.).log2().abs() < 1. / 12.);
    run(&mut e, TONES);
    assert!(scalar(&mut e, "max(abs(iwsst(wsst(x))' - x))") < 1e-6);
}

#[test]
fn review_b7_iwsst_admits_ridge_work() {
    // Ridge reconstruction adds about `bins` rows at every sample. 20 columns
    // by 10 ridges by 20 bins is 4000 additions, over a 16 KiB budget (2048
    // doubles). The same call with 2 bins (400 additions) fits.
    let mut e = Engine::with_limits(matrust::engine::Limits {
        evaluation_units: None,
        memory_bytes: 16 * 1024,
        ..matrust::engine::Limits::browser()
    });
    let started = std::time::Instant::now();
    let err = e
        .execute("n = numel(iwsst(zeros(4, 20), ones(20, 10), 'NumFrequencyBins', 20));")
        .unwrap_err();
    assert!(started.elapsed().as_millis() < 1000, "{err}");
    assert!(err.contains("memory limit"), "{err}");
    assert!(!err.contains("Workspace"), "{err}");
    run(
        &mut e,
        "y = iwsst(zeros(4, 20), ones(20, 10), 'NumFrequencyBins', 2);",
    );
    assert_eq!(scalar(&mut e, "numel(y)"), 200.);
    assert_eq!(scalar(&mut e, "max(abs(y(:)))"), 0.);
}

#[rustfmt::skip]
#[allow(clippy::approx_constant, clippy::excessive_precision)]
mod refs {
// Generated by tool/refs/wavelet_cwt/wavelet_cwt.py; do not edit.
pub const CONST_MORSE: [f64; 4] = [1.8820720577620569, 5.500773536942527, 2.1676641111362525, 0.6553434149791638];
pub const CONST_AMOR: [f64; 4] = [6.0, 1.4142135623730951, 8.145966026289347, 0.8610267803506784];
pub const CONST_BUMP: [f64; 4] = [5.0, 5.847705277601328, 5.500993494605388, 0.2903187792185874];
pub const CONST_MORSE_TB30: [f64; 4] = [1.4938015821857216, 3.907349808686821, 1.8138715542244208, 0.9383932593846159];
pub const CONST_MORSE_2_20: [f64; 4] = [2.23606797749979, 3.2036163775859636, 2.8484683340333454, 1.1398130619077886];
pub const GRID_MORSE_1000: [f64; 5] = [71.0, 0.4341244679221789, 0.4050524510047554, 0.0033915974056420227, 6.435407606501865];
pub const GRID_AMOR_1000: [f64; 5] = [71.0, 0.36828044584499214, 0.3436178061112623, 0.002877190983164001, 5.459343938525972];
pub const GRID_BUMP_1000: [f64; 5] = [57.0, 0.45446336238202306, 0.4240293105471752, 0.00936981252843013, 6.655829913821919];
pub const GRID_MORSE_FS: [f64; 5] = [71.0, 434.12446792217895, 405.0524510047554, 3.391597405642023, 6435.407606501865];
pub const GRID_MORSE_NV16: [f64; 5] = [113.0, 0.4341244679221789, 0.41571901471380124, 0.0033915974056420227, 10.162971810538918];
pub const GRID_MORSE_LIMITS: [f64; 5] = [34.0, 199.99999999999997, 186.6065983073615, 20.30630990890589, 2703.622979039498];
pub const GRID_MORSE_CLIPPED: [f64; 5] = [71.0, 434.12446792217895, 405.0524510047554, 3.391597405642023, 6435.407606501865];
pub const GRID_AMOR_LIMITS: [f64; 5] = [37.0, 80.0, 75.50994501453548, 10.0, 1257.200762157404];
pub const SCALES_AMOR_LIMITS: [f64; 5] = [37.0, 11.93662073189215, 12.646409136803774, 95.4929658551372, 1500.6728681718678];
pub const GRID_MORSE_TB30: [f64; 5] = [78.0, 0.4117715994571745, 0.3841964872714236, 0.0019802746259601598, 6.121281916365712];
pub const GRID_MORSE_2_20: [f64; 5] = [75.0, 0.3925035695119673, 0.36621877965062605, 0.002323922510639665, 5.828769749425307];
pub const GRID_MORSE_255: [f64; 5] = [51.0, 0.4341244679221789, 0.4050524510047554, 0.01356638962256809, 6.293645012677146];
pub const GRID_MORSE_1024: [f64; 5] = [71.0, 0.4341244679221789, 0.4050524510047554, 0.0033915974056420227, 6.435407606501865];
pub const SCALES_MORSE_1024: [f64; 5] = [71.0, 0.6899889165004683, 0.7395118101493722, 88.31858131205995, 1309.2237640771937];
pub const COI_MORSE: [f64; 7] = [434.12446792217895, 434.12446792217895, 149.79159981347547, 3.2954151958964606, 3.2954151958964606, 434.12446792217895, 18948.325835955187];
pub const COI_AMOR: [f64; 7] = [368.28044584499213, 368.28044584499213, 122.77040674869632, 2.7009489484713187, 2.7009489484713187, 368.28044584499213, 15605.012236590112];
pub const COI_BUMP: [f64; 7] = [454.463362382023, 454.463362382023, 423.04145485218885, 9.306912006748155, 9.306912006748155, 454.463362382023, 45049.82737769813];
pub const COI_MORSE_ODD: [f64; 7] = [434.12446792217895, 434.12446792217895, 149.79159981347547, 12.974075574395512, 12.872715608970548, 434.12446792217895, 14454.755056572956];
pub const CWT_MORSE: [f64; 271] = [-0.010211586605851685, 0.020787374235425798, 0.02731794637840003, 0.01599193505319939, -0.06610682302562669, -0.1628493119930444, 0.09833551258254772, 0.13555669634715073, -0.2754010167036357, 0.0010824411794684342, -9.892956370410979e-06, -0.010459047841062727, -0.026473757268272717, 0.016431586471408424, -0.0057171683724896675, 0.20023004579266854, -0.21876372537796726, -0.02615374248840904, 0.06572443914501397, 0.009526630035939015, -4.388259926221607e-05, -0.011142593549250915, 0.0181113672365732, -0.015013831800358299, 0.0179895881026874, -0.062265982358689756, 0.3192543476150652, 0.01345139621053111, 0.37673284617570535, -0.03414170405920678, 6.334730657249459e-05, 0.007053785326805117, -8.623393373707255e-06, 0.0059817625988502045, -0.002712065348050199, 0.015885081400294907, -0.3548431117221661, -0.03554948466530509, -0.41626764642221153, 0.027866928954680813, 0.00028135753090857517, 0.01169616313689908, -0.010211576679908277, 0.001339233069358169, 0.000266296078486034, -0.0016831801251557233, 0.2874744719853718, 0.08728522386526286, 0.07241184163079858, 0.10241287517750944, -0.0016144016316557426, 1.767208518435399e-06, 0.00023048753511918822, -0.0038529260465672444, -0.00019683504000682538, 7.164814114425362e-05, -0.15206589708776716, -0.18972422789817003, 0.06868500820528697, -0.29454560182710354, 0.002508903978186599, 4.965715431227402e-06, 0.00699335326399865, 0.0009859222311443105, 0.00020963087958332846, -6.163729145788899e-07, 0.04129734141410735, 0.3570063301716422, -0.019911974177729056, 0.19635426613735257, 0.006024552910395612, -4.444113657299218e-05, -0.0013148009924910712, 0.003676528724563949, 0.00036435508914890935, -1.142759547767914e-05, -1.527572825177348e-09, -0.5633723376468167, -0.011149389850137493, 0.2546233560435186, -0.034761859729098135, 0.00010439314645840765, 0.007555549188101064, -0.006475037070219028, -0.0012069469858233246, -0.00033261602710867976, -0.0027025969028182687, 0.7167743268493959, 0.03489403507089412, -0.43395332601391806, 0.05047527748778078, 0.00013349098445069273, -0.009329920470510478, 0.01021157667990882, -0.006373959803218309, -0.002997075292619329, 0.00014865020318839873, -0.700259521982726, -0.08946611886816613, 0.14824364038629073, 0.05561190818040143, -0.001542575471496067, 3.835230566966198e-06, -0.012687851559294099, 0.015818161978122615, -0.012900000462848425, -0.0021958063963231646, 0.49390356446112826, 0.20436788706404801, 0.042591165489134025, -0.2882701113667688, 0.003679135677477337, -9.546330218970265e-07, 0.005229241506259664, -0.020640090347378223, 0.0016903710284846235, 0.009469253996931376, -0.22155502288923445, -0.40860623440172295, -0.02698956294088935, 0.30716813592016745, 0.0019143719857913554, -3.661351290855948e-05, 0.01003969045561702, 0.024430074750003822, 0.033115446322189296, 0.00530131371930014, -0.04276955667447031, 0.7062880869751306, 0.06966298207266658, 0.10956676349503969, -0.0897653503429103, -0.02791743093184062, -0.015305791382367881, -0.02453665889100568, -0.027806292593248665, 0.003555267842683442, 0.017090995189894216, 0.047415778547983754, 0.0565268323085663, -0.8969840904724684, -0.11731215510743788, 0.07901499548674674, 0.12108613759997025, -0.004700539415538695, -1.6523780265971928e-05, 0.015409238380534132, -0.005674685944273378, -0.03321244745165913, 0.04633777956339099, 0.39095288223230507, 0.33756184648383547, 0.01129258957984959, -0.4270037146683553, 0.009631517103063428, 7.939148058638547e-06, -0.010334039956205224, -0.010240057084078684, 0.01757000510732482, 0.003083849160424676, -0.12481916254331912, -0.597549068664993, -0.004195507754742742, 0.32859516269965666, 0.007470686338111371, -0.00014097941630707476, -0.01080680063516093, 0.015211666997684318, -0.011658839187746267, -0.005338228268028259, 0.022765091343280114, 0.8566663243409154, 0.007188517251444912, 0.12366817048472765, -0.07655871858454599, 0.0004261136990916607, 0.004582909853653911, -0.0017991765046850212, 0.006994609686367621, 0.0018101293132071138, -0.0020894824027945787, -0.9476763036857978, -0.026477467562111577, -0.2546991954284229, 0.13272286460621757, 3.078754807507231e-05, -4.973247402798556e-06, -0.009055061651088473, -0.0013345164561700852, -0.0005267508848989654, 7.84306498207571e-05, 0.7644868902121662, 0.07858633271938213, 0.07716101247853388, 0.027379732891105922, -0.0041343421394261426, 1.211058603129573e-05, 0.0020561780866834474, -0.0009331627458305426, 8.373818687873707e-05, -1.082210737257529e-06, -0.4192989439299238, -0.19082385905798493, 0.0061425679547829105, -0.3848150224126848, 0.012133746049938452, -7.916806158448023e-06, -0.001076654928711249, -0.001278129402883279, -0.00034156616492045567, 4.8897615024822e-06, 0.143392021132287, 0.37643336121026283, -0.007759586221881449, 0.43011287212507165, -0.0036160831161129274, -0.00011392649785785499, 0.004633415272994713, 0.002149028146775754, -0.0026522965537034344, -0.00018268258647558049, -0.027476962572441973, -0.5882414880945772, 0.010685005370223072, -0.012906636520048712, -0.06395298028378145, 0.0004928631132693234, 0.00813476883953511, -0.0017991765046851153, 0.005342811578004027, -0.0017070161672777594, 0.0027219713390385708, 0.7002595138775665, -0.017795183395329116, -0.21991603447117175, 0.15814781130111938, -0.0004771748472466622, -9.763134478133342e-06, 0.00754097335694593, 0.0013195676638811118, 0.0029201994998782387, -0.006365138183461784, -0.6031332931084636, 0.02014061423851804, 0.09723553159725391, -0.06639052423256066, -0.0031386757017699235, 1.9075609179223358e-05, -0.01578275553270454, -0.008669848025463785, 0.02757920055969649, -0.029181781405480226, 0.36963145292513977, 0.0034747406765235217, -0.007035202660974938, -0.30381663408983994, 0.013411122403452509, -2.4875162784240936e-05, 0.013355650002896844, 0.007332134804783054, -0.01809048949605645, -0.05826995978199949, -0.15038533038507312, 0.007005899316741254, 0.04233197860922225, 0.37451553726093956, -0.017112381610392545, -0.0032837086371094774, 0.005054544339872888, 0.004836501368374405, 1062.179734094213];
pub const ICWT_MORSE: [f64; 33] = [1.0931758750305274, -0.41857990113389676, -1.448948641943238, 0.05371565112257459, 1.4832823954220735, 0.31836474302556333, -1.2362825001560565, -0.4992951134320022, 0.8522788090188009, 0.3863211706339209, -0.5547286119502366, -0.045659891091695604, 0.5163235809652638, -0.3263893422372221, -0.7585608773774214, 0.5334703432782548, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 159.867285593136];
pub const ICWT_MORSE_MEAN: [f64; 4] = [1.8431758750305274, 2.2332823954220737, 1.602278809018801, 1.2663235809652638];
pub const ICWT_MORSE_RANGE: [f64; 16] = [1.1287038093588666, 0.03773865941648618, -0.9897749054403977, -0.0017859354534877005, 0.9954680061181334, 3.925225084899335e-05, -0.9958052686925565, -3.311711165045373e-05, 0.9957835903866914, 2.5632589695874428e-05, -0.995768087525691, -3.9471615515802635e-05, 0.9960627237850729, 0.002447995293164826, -0.9991345571386975, -0.03847987309355329];
pub const CWT_AMOR: [f64; 277] = [-0.04582054119653281, 0.023804676159304174, 0.03666338212945164, 0.004976531840695935, -0.8452677517601981, -0.03462940290734904, -0.06370985259921016, 0.020368908702604976, 0.0061359406700190414, -0.019829708667345604, -0.02909202437257699, -0.05851313925768323, -0.12581488133591912, 0.2977835693329173, -0.08502767137391778, -0.00353410908123974, -0.022698977523849136, 0.025128362199869792, -0.022528854134937056, -0.09431596866365743, 0.07365488764226641, -0.22902789415168043, 0.01270448643343018, -0.01297794538299206, 0.020623586718457546, 0.000722739880679245, 0.0008508263296698945, 0.3808027430987931, 0.39960891387604863, -0.018517757499180117, -0.006642045212255508, 0.01338660915598387, 0.0037708520686560955, 0.0006815946926583895, 0.8718298278576542, 0.05863642262237099, -0.2663277093863838, 0.006094386993016548, 0.010306366519260177, 0.0021577197126661832, 0.0005266551017387729, 0.44965479262192254, 0.24307831182901352, 0.3162505362606136, 0.01001010564496285, 0.008216065768073743, -0.00100689365595127, 0.00045046407674233674, 0.0158237884185222, 0.666923026217325, 0.0878021170920689, -0.20198097262028025, 0.0021098859456899267, -0.003936238800253423, -0.00043138597059749957, 4.033868876659914e-06, 0.5464700713023263, -0.03622331109430134, 0.09421098281177219, 0.01558949973297524, -0.008122844686850038, -0.0014593066873398906, 7.184775743024728e-06, 0.04816207367577846, -0.19208340958350997, 0.2465786654540752, -0.11035619394384982, 0.0010121674165734452, 0.0005899638034540166, 7.318100638657931e-06, 8.834706961864696e-05, -0.36918059973524986, -0.10724549330077678, -0.11592585115737218, 0.01194103634603741, 0.004869664664870572, 0.0004788545677439968, -1.4134715892819828e-07, -0.10797463119499372, -0.45579623667720415, 0.416180081819785, -0.04026523238100267, -0.0013032934666653891, -0.003188547212856162, -0.00033194584401833433, -0.0012130438769545862, -0.9885574169407276, -0.03957531053245179, -0.21002166706756076, 0.006775629879729681, 0.004528126657354523, -0.002036137422166367, 0.00011388655712568277, -0.4843591239955559, -0.13809212890459346, 0.42205995319002476, -0.0032648415189675695, -0.009810661015097382, 0.009598375974834756, 0.0016576608582083976, -0.016215710159481894, -0.33016421793263934, 0.06629193206765627, -0.19218638498052876, 0.002957753691068321, -0.014291793459304546, 0.01117528216412013, 0.0030418755838326457, -0.22436672595153181, 0.10941123937865005, 0.24231823540846986, 0.009399912521894278, 0.009792776210892461, -0.02022798473107626, 0.017361626249494607, -0.010239905214124656, 0.4678957834094868, 0.22821418559993834, -0.12383112196663074, 0.008361177272939823, 0.019979446303362496, -0.02203029046241931, 0.019648577620587332, 0.7341353976302788, 0.08459983405278841, 0.012510776197467613, 0.009947647474726295, 0.003503834463973931, 0.028231450096760487, 0.04978060371400897, 0.10464758209237107, 0.4385831527210406, 0.41846847495366396, -0.057226423216368325, -0.012844272779213083, -0.024673759141291118, -0.04661621028091417, -0.11172649929537233, -0.07101176736698157, 0.003713916330045866, 0.024314031936777156, 0.08449908342841563, -0.3626851625168379, -0.09602421781647716, -0.4693573595366163, 0.011671059589656491, 0.016848070134132878, 0.024992512457195984, -0.0412339080202634, 0.03334662247958781, -0.6378572020385758, 0.018060240619188095, 0.09310642139310008, 0.012032090857838008, -0.0026734100999299642, -0.018439397534955826, -0.011372068627576684, -0.9623999633560013, -0.05102927041205677, -0.3305271434998039, 0.0004133936571877601, -0.010403938087729221, 0.004047549252069405, -0.013011528833309716, -0.22888638353393656, -0.25445333370925216, -0.15626536026574986, 0.07250368338681945, -0.00888924506851009, 0.007612215642303436, -0.007888733672258267, -0.0018703335096613219, -0.46600268913874476, 0.08081802628110205, -0.13515581332349605, -0.003020851564663249, 0.005503060784749368, -0.006721804230939478, 7.074771407979899e-05, -0.1862530384325664, 0.16253445097421593, -0.3763391007467426, 0.04126132687025017, 0.0037718268569257047, -0.006265453153300473, -0.0004747810636797506, -0.004800398331480549, 0.5473295792688004, 0.1480995143828252, 0.004051288520466614, -0.0029857875164124724, -0.00433213319022848, -0.000811210098957343, -6.284783815677173e-07, 0.5464700689680675, 0.17331597443006427, -0.47103341966299817, 0.01741089568262755, -0.0007641810881308203, 0.000504285609236331, 2.9401611912083665e-06, 0.058685653516923474, 0.6332138475837227, 0.1227826047966559, 0.061451341499319495, 0.005985775036164629, 0.0035112767076083194, -5.543330495363761e-05, 0.00013221642857196153, 0.8912808083135605, 0.03067564674846279, -0.3874854576140621, 0.004310962411200176, -0.004162780935625038, -0.0005639202334794581, 3.7500106558727204e-06, 0.20200632438366278, 0.044885047875676334, -0.035610871196139926, 0.0627475589667673, 0.003945879793914233, -0.002388910809823885, 9.144147939528125e-05, 0.0018150502750066027, -2.6283107079638768e-09, 0.007793497022140432, -0.21335394457075252, -0.0009684986613562052, 0.006366393391541267, 0.002676994555322771, 5.82841829795744e-05, -0.04770521101367424, -0.25809440393846517, -0.26682171905988417, 0.042286532017856336, -0.0024225078035650825, 0.0010750078822971982, 0.0023835340339634515, -0.003277612970640416, -0.7970869404587401, 0.11479126214785233, -0.059585940813647514, -0.002152487085963791, -0.0005580596005316731, -0.004176510142420786, 0.0007109381633262494, -0.7395680045029028, -0.13564271454830823, -0.41764455686800517, 0.0215125522876072, -0.011449977324048144, -0.0021075352220181467, -0.003642282607450572, -0.07212464194477988, -0.4679725179772927, 0.1975475338221763, 0.024178973170246417, 0.008889034383387083, -0.009614932464000305, -0.023251882311723492, 0.03067135080348326, -0.6032388359756442, 0.037863430537421125, -0.4016994935646612, 0.00789479589344469, 0.01896341960854673, 0.01905797757708898, -0.0301735597417907, -0.1971227185702199, 0.23508241372250277, 0.054540485098660677, 0.06214805933073238, 0.006645557139748404, 0.014541443708795107, 0.02432993565104197, 0.017883431692520223, 1382.5646431121859];
pub const CWT_BUMP: [f64; 197] = [-0.0014261081624937525, -0.007536506643518224, 0.030842448807526526, -0.027148111391399863, -0.03890048240203098, 0.00755628119440381, 0.11467303148411183, 0.34680105196616196, 0.0030988262312039412, -0.0037947001501427253, 0.0038053120516429034, -0.04067338102132495, -0.030856585920077343, -0.00022073853650353902, 0.40411182008757146, 0.0007448906783353472, -0.013743100042403424, 0.005965908693236101, 0.0006494439091908321, 0.016171485194141525, -0.010056394485088719, -9.059060856886767e-05, -0.9151859806984497, -0.004955093546018538, -0.00807324594117601, 0.0006115940340221147, -0.0003730663783896614, -0.01147045020094393, -0.05161581789250702, 4.3093490212906755e-05, 0.1830655167958064, -0.0018115279177645888, 0.013339006870391057, 0.001579889056661877, -1.1803311774354577e-05, 0.008701260394274754, -0.20447341356347493, -1.0267597058585054e-05, 0.0044684855474726335, -0.0002539325086118608, 0.00034405971293574025, 0.0012044359307245313, 6.393825178129145e-05, -0.019339281570857812, 0.4115261425859754, -0.009477336355936634, -0.0022527731331623882, -9.824951471782714e-05, 0.0020270063582993603, -0.0010944933843492067, -3.162448657399104e-05, -0.6742678521995762, -0.0021455073201428814, -0.0017769427518411558, -0.0005082171041439354, -2.0800771214681686e-05, 0.003126090856030369, -0.0027177073906748737, 2.310563233276576e-05, 0.9171389939728296, 0.001109456526908924, -0.006189743972826954, 0.0004194698650495761, -5.3602239439009506e-05, 0.009482138219632178, -0.05780540006850843, 8.504142626856594e-06, -0.12046838728916504, 0.0007112121423408504, 0.019911627947338032, 0.0020226103121855914, -0.00020632531423511296, 0.030768018424581833, -0.3319422516722293, -4.553087345581431e-05, 0.002256843033445304, 0.00013295658847574705, 0.005181727557867512, -0.0038161636755670836, -0.00023946007133737472, 0.05055239952662364, 0.3771367673513169, 0.017723623484615276, -0.008072131218718172, -0.00034556778188609853, -0.00793855862255232, 0.016336865876740013, -0.0013307625179889361, 0.7446427134013607, -0.0026775414014049356, -0.010083579628989859, -0.007847351508245748, 0.020879794666783493, 0.012390731147199479, 0.11823784667307977, -0.0129135480565097, -0.591076508060891, -0.052329047277575105, -0.007976372029227168, -0.04281552527211945, -0.0597677208408307, 0.027724904999887928, 0.034122081105844124, 0.0047980176380955095, 0.09755047432280507, -0.17978706021066715, -0.020184249777551407, -0.01138612448465468, 0.001367583211437576, -0.0042058301442618636, 0.004875671062275772, 0.0006618406111558406, -0.8368795443148029, -0.05101243299619718, -0.01897347132029211, -0.003934084853088705, -0.0016590808021063107, -0.031301419528428895, 0.015095705380678968, -7.387453091551036e-05, 0.2564602674116892, 0.004611530107625697, -0.021249740990070955, -0.0017279633405525748, 0.00013223144886401908, 0.020131947828151378, -0.08572818520812833, -6.0998027022089066e-06, 0.1063135009880284, -0.0003075193114878114, -0.01117395234144887, 0.0021356171110611297, -0.00018868641405152592, 0.010471432832597148, 0.43675864313780377, -4.581995651563731e-06, 0.0007553730107853223, 0.0004557148001509846, 0.006721260341645277, 7.93124487744615e-05, 9.915894028516737e-07, -0.0004933205002005884, 0.029417513638165096, 0.009973623893084559, -0.0015122462206530313, -0.00019790587783628322, 0.0014131599195288003, 0.0020598852047964793, 6.362469586055166e-06, 0.5488310468248417, -0.004181311459173191, -0.009143301910535428, -0.00025286155164065254, -5.0720429359428886e-05, -0.0023474681065442844, 0.0043163358997617175, -1.1643528421010956e-05, 0.17051776476821873, -0.0020550992978661297, 0.010233687337025526, 0.0014462206177046036, 0.00017705083186509674, -0.00030601130103344147, -0.09111188450617227, 2.4825730112065156e-05, -0.21744591167799576, -0.0014021758589492627, -0.002344343907958455, 0.0033716877927829746, -0.0001252986448141929, 0.010653498493714397, 0.3583070528511512, -1.8217119753665334e-06, -0.01352471971141836, 0.0029878687753381286, -0.025696054114905168, 0.00045947778540698323, -9.606327708769666e-05, 0.06711311220913327, 0.13340848833945118, -0.007709943260301605, 0.00798156771865873, 0.005738189881921201, -0.038052654834448835, 0.029288260160141986, 0.0008644383762794596, -0.09937637699782578, -0.09706156798014696, -0.023581355968832783, -0.0013114862080346886, -0.001471598956434601, -0.05191510984612455, 0.041342625127454825, 0.0033791879449960698, -0.7585320223654147, -0.1251089222296604, 542.6755966141205];
pub const CWT_MORSE_OPTS: [f64; 235] = [-0.029552313218725458, 0.27876178158277776, -0.1250565834087045, -0.9567235038770707, -0.08625097974586522, -0.00019113396506403635, 0.19686967722417129, -0.012143657319858467, -0.5286946884565071, 0.053625146367486616, -0.0013554934065094718, 0.3156203294530214, 0.0543766125459036, 0.3530703890873951, -0.020092268794474937, -0.004205852556737293, 0.3892291077315061, 0.08544137013402561, 0.6782898794817024, -0.0026711123587685403, -0.009607170233911664, 0.3560174723523646, 2.344539226663278e-05, 0.3756604591550656, 0.008314522155615782, -0.017751818324583737, 0.2034354788987418, -0.2294725228656168, 0.02254178238963859, -0.002431921578390286, -0.026379000692276982, 0.0014734879745275792, -0.4379015025275748, -0.0626453556204113, -0.001803931800872038, -0.028305285713698686, -0.13720939697053672, -0.2955842206651865, -0.024564044039842158, 0.00039339490373614954, -0.009814505796707649, -0.15319801460724392, 0.31063255955258556, -0.0025930812979815293, 0.0003916162743795729, 0.0465748047965924, -0.09058269945765819, 0.9127077411702963, 0.00016159653074646155, 0.000270800433656451, 0.1511478392084212, -0.04222743532313649, 0.8304227185727945, 3.546089539078115e-05, -0.0005695319432366642, 0.28914537905185694, -0.05962505592610441, 0.08938415359031518, 1.216194522162516e-06, -0.0030530189938043142, 0.4090896641606303, -0.11059090430609908, -0.5002544799385312, -3.0657185645837334e-07, -0.008504171529320984, 0.43912696752048086, -0.08789622754927376, -0.44999846364657703, -2.728193069575339e-06, -0.018072525016438863, 0.33838740045124077, 0.11360910681295718, -0.12752063235964067, 0.00011921911832258262, -0.031117349633158396, 0.1464604617466186, 0.4272560065721923, 0.025715728450574946, -0.0007584617019163981, -0.042370261691488796, -0.03230010540540397, 0.5213640438270025, 0.022803838587962158, 0.002701779430366496, -0.03897784109358595, -0.1157484896334661, 0.0795626678217467, 0.00437062004802991, 0.0005451101125991394, -0.0008402900098772662, -0.10770003095624736, -0.6744220572775437, 0.0016452179209173103, 0.0006719078957910295, 0.09070011987694421, -0.05805961183704951, -0.993720094307375, -0.01313914248719755, 0.0002761058129340275, 0.23402084496710993, 0.013046065404515448, -0.5013545194512087, 0.025526241773759763, -0.0015930927346705104, 0.38809935441374604, 0.10261917550221952, 0.22653906693746653, -0.03255262279196362, -0.0065456882935930084, 0.4748912885669394, 0.1872491864394795, 0.3441408831525973, 0.04545595226906667, -0.018930031886587574, 0.21580692090148218, 0.11016668559762628, 0.017142655832077358, -0.05448923704460492, 0.06874729534013557, -0.16860082922960576, 0.03457340424978293, 0.04235917676449205, -0.0013549401343351654, 0.11408312866749645, 0.0672788325671966, 0.8803134900670033, -0.030810774484441573, -0.0022909656617875144, 0.03271211532597021, 0.021365785659699287, 0.8561903075567213, 0.02836122168234733, -0.0029966897590286462, -0.12654963193268587, -0.06863857382580534, 0.20309233986190633, -0.017736771929774277, -0.0019652514839362503, -0.3199857212787409, -0.1860860233194097, -0.250963455165055, 0.003098292374876624, 0.003885898169571451, -0.45465632355886854, -0.18831516896594694, -0.22846218840947996, 0.003605151044283995, 0.019345653053046072, -0.45048854504905067, 0.08710447770069497, -0.06263394936299319, -0.0011993036900052102, 0.04934851709303073, -0.31519652634992484, 0.5529992010407195, 0.002417979474472477, -0.0010465615168909253, 0.09445132304165633, -0.14747018733758718, 0.749933338031728, 0.003881280459851368, -0.0005507828870279446, 0.14359507426920545, -0.04758985957008793, 0.2768668663866277, 0.000532881325846039, -0.0013233230573572244, 0.16863971783788914, -0.031568935507231596, -0.5548707209932554, 1.4683836904474585e-05, -0.002602340158489695, 0.13017892473346937, -0.03489836704254819, -0.907532541354798, -6.592526210355386e-07, -0.004163959400372983, 0.0025334277745022032, 0.023473058862157087, -0.5002544799386536, -8.356636550978168e-08, -0.00487964586180603, -0.19107612876520189, 0.16458287304598818, 0.04432098019240274, 8.928600653359412e-06, -0.0018813391625335891, -0.3676064077154724, 0.27428559206638153, 0.1908481139500916, -2.5930351928432398e-05, 0.010087289888565538, -0.4309497154477784, 0.12960707907595734, 0.0847735884261927, -0.00028629228670760715, 0.03797132431562124, -0.3526318792817882, -0.3483643062436933, 0.009457052028582488, 0.001650803552582919, 0.08661797316027707, -0.19943116805000363, -0.8078133312186115, -0.002034508450351113, -0.0004023313446521257, 0.1510242397303438, -0.06298323857992007, -0.6744239374022485, -0.003061367004898238, -0.0011700422773940641, 0.20766373845374742, 0.021785053040717376, 0.09780064451198447, -0.0005588322047048381, -0.0026506140207132184, 0.21434188637153198, 0.06131535871374742, 0.7559716717717461, 0.02053142027856238, -0.004912045795624723, 0.13028863830689758, 0.0333883147968653, 0.6399429621405857, -0.04274025927404655, -0.0073017772818620924, -0.04380156220900715, -0.13263136287533292, 0.06118880335908711, 0.046771275844630394, 0.009915447383366974, -0.2088756481223744, -0.2834825434171347, -0.01375536630182047, 1564.6111840380186];
pub const CWT_MORSE_PERIODIC: [f64; 271] = [-0.031052855621226817, -0.004301213590838435, -0.013552038149053144, -0.02419810637502308, -0.017929313788824175, -0.2065440032587868, 0.11684066616572909, 0.09675203666509247, -0.27164802384762826, 0.0010862976338604903, -1.1007583966457701e-05, -0.021563869524860398, 0.003367374559359809, 0.019727213385417924, -0.015497125349320549, 0.1669268203336113, -0.22347433302188288, -0.02599479919206911, 0.0657244282558947, 0.009526630035941687, -5.518633485982258e-05, 0.014669436439250652, 0.011706715957204373, -0.012350138611474238, 0.0012269018211044365, -0.06685240900911993, 0.31936293376035463, 0.013451417792548534, 0.3767328461757052, -0.03414170405920682, 6.491723131037431e-05, 0.016116593734793898, -0.015474151371342714, 0.008521736047067562, 0.0015629979923367028, 0.015494579623684647, -0.3548432074623143, -0.035549484663683475, -0.41626764642221215, 0.02786692895468068, 0.0002812758649166169, -0.0012848209170713363, 0.0004469339278624241, -0.004578470473270554, -0.0006725635634727408, -0.00166577311470055, 0.2874744653471424, 0.08728522386526308, 0.07241184163079852, 0.10241287517750944, -0.0016144001810094921, 0.00023456488749514816, 0.007462557512206133, -8.507424433296107e-05, 0.0001739705780152744, 7.354454253619396e-05, -0.1520658971447749, -0.18972422789817006, 0.06868500820528706, -0.29454560182710354, 0.002508903971145767, -8.139591343970189e-05, 0.007498809758134177, 0.0024971572063237878, 1.634228490171412e-05, -9.474039301647641e-07, 0.04129734141393358, 0.3570063301716422, -0.01991197417772897, 0.19635426613735318, 0.006024552910402583, -2.6071107828581474e-05, -0.010383078775841584, -0.0007835386097402707, -0.00011503022925164058, -1.8285831424359744e-06, -1.464361210415377e-10, -0.5633723376468168, -0.011149389850137503, 0.25462335604351893, -0.034761859729098184, 0.00010244757568068094, 0.0002967938573399118, -0.0010851463717624016, -0.0018723324922042374, -0.00014028994984719746, -0.0027052599511508462, 0.7167743268206365, 0.03489403507089422, -0.43395332601391867, 0.05047527748778094, 0.0001335754990259355, 0.00548608289174402, -0.00044693392786188894, 0.0024693214431754373, -0.0012917787993719934, 0.0004599801087834686, -0.700261041237253, -0.08946611878081595, 0.14824364038629062, 0.05561190818040138, -0.0015425765586320056, -0.00021977602241526938, 0.006870542137402633, 0.0034562803768891765, -0.0004878176478025451, -0.002034519362769386, 0.4942784227565936, 0.20436815286183024, 0.042591165308927655, -0.28827011136676833, 0.0036791356784689148, 6.794441066742526e-05, -0.020201804034748475, -0.009720641791284769, 0.015469025474191416, -0.008310887704070192, -0.22934031776378583, -0.4054115448607512, -0.026554603218872413, 0.3071614633287235, 0.0019143731469417487, -3.588313911165428e-05, 0.020919899375019278, 0.00836960606489242, -0.009567409585718126, -0.02145643804128093, 0.05470659723109734, 0.6832020211715725, 0.02509500558834714, 0.10880839696792009, -0.03892148502390709, -0.005944879722442049, 0.008030645910677245, 0.004940996783610004, 0.00793242431074383, 0.025686121676697073, 0.022229875962535423, 0.008945396817926662, -0.01751663117565341, -0.7568954555496249, -0.14840120125883177, 0.058642113780295796, 0.1192442319572207, -0.004707889049484593, -0.0003465671276381081, -0.012930198473351775, -0.023545407659157677, 0.007068795101557557, 0.0018632837656277232, 0.3947447168106445, 0.3374091977191213, 0.011298216906912238, -0.4270036981196905, 0.009631517103075148, 4.4669832849747815e-05, -0.018419199515469966, 0.015973108242776354, -0.007094853044079962, 0.006789162805022723, -0.12527870106439573, -0.5975679684395225, -0.004195474460636256, 0.3285951626996564, 0.007470686338111356, -0.00014349242317815192, 0.011324426784146975, 0.002735003646198234, 0.002000757951335324, -0.001955415918213723, 0.023012117170576978, 0.8566674030091818, 0.007188517253741309, 0.12366817048472795, -0.07655871858454598, 0.00042619088814320575, 0.014989113633779836, -0.012364690665742487, 0.0023724579467530814, 0.0004917527789916267, -0.002052844702552109, -0.9476762994155666, -0.026477467562111567, -0.2546991954284228, 0.13272286460621752, 3.078676560674475e-05, -9.872916877750996e-05, -0.0007333296224930636, -0.002929074671080444, -0.00021521799835389627, 7.531955189964568e-05, 0.7644868902444285, 0.07858633271938217, 0.0771610124785339, 0.02737973289110609, -0.004134342137887674, 2.4972972197593647e-05, 0.005920407021964108, 0.0006639288391704521, 7.341406171114829e-05, -8.958426827880214e-07, -0.41929894392924577, -0.190823859057985, 0.006142567954782892, -0.3848150224126852, 0.012133746049939099, -6.084536223020689e-06, 0.008909371475927765, -0.0015313564794744495, -0.00033296258209350796, 4.225157621500983e-06, 0.14339201933705428, 0.3764333612102628, -0.007759586221881559, 0.4301128721250721, -0.0036160831161128736, -0.00011467195915797622, -0.01105483288976359, 0.007003110255254993, 8.220620879190003e-05, 6.861117557590592e-05, -0.027477712634453297, -0.5882414879086707, 0.010685005370223006, -0.012906636520049319, -0.0639529802837813, 0.0004929293694344053, 0.012991759914053802, -0.01236469066574257, 0.005500517564878096, 0.0009087877849923423, 0.0026908938779388394, 0.700260696484705, -0.01779518331142549, -0.2199160344711718, 0.15814781130111955, -0.00047717662593923117, -0.000261412703966229, 0.015885828707272544, -0.012170470352046443, 0.006715855872796623, -0.0005834335517673987, -0.6026052312377234, 0.020131333974448783, 0.0972355287692023, -0.06639052423256159, -0.0031386756899914286, 0.00017195773053696245, -0.009620426945172317, 0.0170188507984378, 0.0017914000818740725, -0.00613167972472314, 0.35184829397273065, 0.0012206429110957328, -0.007344328356399497, -0.3038183408246695, 0.01341111959417239, -8.301357784861306e-05, -0.014323848067676379, -0.022919772638504712, -0.02136764208369242, -0.004768593278585698, -0.11109839445924502, -0.05111695844846175, 0.00448206964025535, 0.4754428821869865, -0.024169155628439486, -0.01260846049266692, 0.024520280490157725, 0.025613854227619942, 1061.4112920937944];
pub const CWT_COMPLEX: [f64; 267] = [-0.00550255860411152, -0.0006534103099698341, -0.04314818417850085, 0.06962314264636739, 0.4084016039822273, -0.004323064289786932, -5.362569438678577e-08, -7.989569920206711e-07, -0.0026775881359730387, 0.00974063557409901, 0.02731425049190344, -0.1941545854339393, -0.011047000836458824, 2.3720433928453577e-06, 1.1138096882518447e-09, 0.011838462314569502, -0.0003124988690472854, 0.005006160689172849, -0.32564174996442846, -0.02647854249472161, 3.7289807795388985e-05, -1.681943017153853e-09, -0.00654090120277823, 0.005356267711660593, -0.0012854295499775065, -0.027608982612940633, 0.15491225047288293, 0.0003143255486915131, -4.918641493580067e-08, 6.872745216453099e-08, -0.0016696920389535506, 0.00029146025401711393, -8.214650682040149e-05, 0.8177619304120873, 0.00044300365177990693, -7.333375595108826e-07, -1.660153453192381e-09, -0.001605111191396547, -0.0005562980867823079, 4.363235542487139e-05, 0.7644868908429769, -0.02020716621471797, -5.8030998015684565e-06, 9.011478583741822e-10, -0.010438356453168011, -0.0012972218477014744, -0.0018792645840938886, 0.0910145802679197, -0.19651346577605977, 1.284759846925665e-05, 1.2198906252335051e-08, 2.522995529731882e-07, -0.016177424256215808, 0.017361020936481966, 0.012678839262970623, -0.6267535024168966, 0.0010253613011441814, 7.772171820366602e-08, 2.6184054060921476e-07, 0.009541747021557283, 0.032516092311292266, -0.020538155743423755, -0.42461780884946976, 0.008413295597830864, -0.030844158675561867, -0.01228369867843949, -0.010035880975849001, 0.02377282360471503, 0.05768803814739315, -0.17669184279530936, -0.003989822564414283, 0.25456673832569293, 0.0032476922983778337, 0.008052655533813256, -0.02354214632126056, 0.0330288235603732, 0.02558176033840545, 0.0006826970429667442, 0.2238610699972355, -0.026464046138555214, -5.1937602902807625e-06, 0.013032701509367469, -0.003015361534932647, 0.0015176095433271368, -1.9941388296196294e-07, -0.013462791833152809, -0.10635044537213374, 0.0001427460446166949, 0.0010746320066626217, 0.0016484218862148298, 0.0002616537259026021, 5.728223784196018e-09, -0.00012630382711341197, 0.3405849977368045, 0.0005811204768021351, -0.005469978804990378, -0.0019499487757907862, -4.28727499290772e-05, 4.9309306637823624e-11, 1.035503435775631e-08, 0.2586448921778032, -0.01275515784331312, -6.36234430138138e-08, 0.0037936253583554487, -0.00040019511320388724, -3.515763677833044e-07, -5.551115123125783e-17, -0.08560747551715893, -0.0255564397451045, 4.505408931601381e-05, 0.000524943100500584, 0.003882218812830378, -0.00063778209589502, 5.2461166244466284e-08, -0.0016124201670923285, 0.2830779577882744, 7.324626221503185e-10, -0.009157836112916271, 0.01213156373912137, 0.009993862957811236, 0.0025274924032474325, 1.1679984212420447e-05, 0.13332049424854062, -0.004834590704289593, 0.010275762842502367, -0.012091250364604232, -0.03117078471101215, 0.0908689714454706, -0.011834373115410857, -0.27101682406726646, 0.004940511448028453, 0.002848003132813795, -0.002956618953265306, -0.007480875450507779, 0.025219777547094018, 0.017683573430986633, -0.098949764664767, 0.5340022790771426, -0.0015360422887711915, -7.075186377880627e-07, -7.002057259889804e-07, -0.01944881331779041, 0.02620906421457729, 0.00880309950395807, 0.9683295532363281, -0.007381292654684671, -1.5849487235083948e-06, 1.1406674826962369e-09, 0.006348943014807304, -0.014654440362650038, -0.0027900966491951904, 0.2672989244669101, -0.08728805662031669, 3.6727255003365933e-06, 3.1465849335832186e-09, 0.011327012809163513, 0.005973922929991985, -0.00024100774760505081, -4.609310781548496e-08, -0.3739912560693739, 0.00031432554869195717, 2.037368027410502e-08, 1.0186496221422775e-07, 0.002405167621336195, -9.847266895596476e-05, -6.744143724110853e-05, -0.43710285836187934, 0.00449789155885227, -2.224555166602249e-07, 2.3955563762119545e-09, -0.007308160393554446, -0.0016803858003820514, -1.3336336407797334e-05, 0.1520658972879173, 0.03024216140408747, -8.684952602672213e-06, -1.8550623027708407e-10, 0.0030007589951562097, -0.008481467250039955, -0.0022553234954899803, 0.11079557743215757, 0.05961170897002312, -0.0001304438563419219, 6.520450668116851e-09, 2.7036979365282735e-06, -0.003788330995985196, -0.005559498096424266, 0.005093049402490354, -0.2597690882682382, -0.0010267243264207582, 1.8763656239179083e-07, -7.722485185420469e-07, 0.017471460715020214, -0.006800758557892381, -0.06437838637699836, -0.7156039493571227, -0.08091340754642964, 0.02455859903927783, -0.005216096457722158, -0.01530622932526932, 0.022165927536659573, -0.0330514934988995, 0.1711927362827134, 0.08434383218462833, -0.42804920358093335, 0.003583284925255662, -0.013896797516139382, -0.0044784390699760535, -0.00526847838638429, -0.014853024402236355, -7.596594433337267e-05, 0.14175845813806917, 0.02282804319063407, -1.0437332584513635e-05, 0.008945695379031714, 0.013193731781455907, -0.002274795223343502, 1.5266571006256058e-06, 0.02014849183906265, -0.12958839263379593, -5.9137991978890624e-05, -0.010912049823485336, -0.005357070861316757, 0.00015629350710749513, 3.802189344636453e-10, -8.89531912648088e-05, -0.26579514099702095, 0.0015064981985178148, -0.007020344004288177, 0.0007033835398526038, -2.3910062450286858e-05, 6.471647523431656e-11, -1.3962148014134979e-08, 0.3487422661223728, 0.004563868452607743, -3.3549308130521024e-06, 0.0026298212180258106, 0.0002784465477295967, -4.823668335706809e-07, 4.829470157119431e-15, 0.060291586703241945, -0.07736044725697654, -1.1701130033665284e-06, 0.010577155701466539, 0.003932459461964061, -8.537020238068525e-05, -4.3981289571393134e-07, -0.0024131573139128766, -0.08587075969741279, 0.0005106206685741145, -0.009172439149073976, 0.011326150397560806, -0.009667452410622925, 0.0022377059821839795, -8.261848732520755e-06, 0.48175836226956315, -0.00011868254024089296, 0.011940615990682277, 0.020580574270901954, -0.025249913858246785, 0.010943822126471069, -0.06482220984224443, 0.06990048033067042, -0.03219676209707734, 0.0003854861161530427, -0.02310537518501531, 1061.935356725683];
pub const ICWT_COMPLEX: [f64; 33] = [1.4626832050630698, 0.1635443330249341, -1.360313934534701, -0.4558387051643032, 1.005257504077615, 0.46141236518679085, -0.649192962215557, -0.1928526388332628, 0.5000448196387837, -0.18972293687175237, -0.643701560997588, 0.46216460749652594, 0.9944221210567048, -0.46767332290799646, -1.3455582168043871, 0.22034503509792563, -0.0831300272643524, 1.4027921069084088, 0.33936966449563694, -1.1781585035205036, -0.4887738139167442, 0.8074660164150258, 0.34766951888131137, -0.5412631328744741, 9.366727647495098e-05, 0.5414933301732715, -0.34766758029006983, -0.8081115986721777, 0.48808029319583607, 1.1789303971427043, -0.33674373730336804, -1.4002742879461971, 319.64835186523135];
pub const ICWT_ONES: [f64; 5] = [1.0576854283062926, 0.8050239509131654, 1.4922113857601584, 0.7386531964376009, 0.6081235631742754];
pub const FREQZ_PERIODIC: [f64; 159] = [0.0, 0.0, 0.0, 1.3683878712318637e-21, 8.975632973744733e-14, 1.3970322316986568e-18, 8.788544407821414e-11, 0.0022473079533356956, 2.8159733865965063e-10, 0.005975838834487592, 8.788544407821382e-11, 0.0022473079533356956, 7.605493828104382e-12, 0.00026215003676426714, 0.842655504433085, 1.2957902096041089e-05, 1.9964034773299801, 3.3440081822354193e-07, 0.7402817876814956, 3.778516720631833e-15, 0.05873216370725369, 8.362955997245175e-06, 0.0015769587380813042, 0.1406927104722303, 2.0890607363063192e-05, 1.98063821423616, 3.7878112194969856e-35, 0.5388810951943783, 3.446456191861196e-13, 0.01797160131379722, 0.0007791196761711914, 0.00020078433586606285, 0.9967835526236634, 1.4354657733745008e-54, 1.2717269185195696, 5.008045257922918e-20, 0.06284438659924302, 6.07133732332699e-06, 6.760674540980902e-175, 0.3798448443099671, 3.5734230381531165e-69, 1.7073269707680956, 7.79238658857714e-25, 0.11447207311333511, 2.427847451006327e-07, 4.710022175098647e-201, 0.19835356363969728, 2.097239922706363e-77, 1.854500993388171, 3.448013616289652e-27, 0.0, 6.766284031660055e-08, 3.2495816358753473e-211, 0.16635510582688767, 1.645653680546548e-79, 1.8604591099245638, 2.638473944067141e-27, 0.0, 1.0378915597015829e-07, 7.986268055787556e-208, 0.21069310104416802, 1.13329472716713e-76, 1.7671635632443723, 1.0636183383375545e-25, 0.0, 5.497200088723801e-07, 1.7079467168152013e-194, 0.3467024124423137, 2.2100796668323054e-70, 0.0, 5.811918793030464e-23, 0.0, 6.071337323327098e-06, 6.760674540980902e-175, 0.6289333682343294, 4.6070004710497535e-62, 0.0, 1.2542225098662246e-19, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 333.29588336806546];
pub const FREQZ_REFLECTED: [f64; 141] = [0.0, 0.0, 3.5549362936918163e-16, 2.2258980682363558e-11, 4.317528515006022e-09, 7.928835388966176e-08, 4.061941198888607e-07, 9.394676193918452e-07, 1.2639773335063266e-06, 1.1466898619697862e-06, 7.694766058052907e-07, 4.061941198888607e-07, 1.761286060741259e-07, 6.475812417839016e-08, 2.0687565707048004e-08, 5.8540645437905054e-09, 3.778516720631833e-15, 2.7666827185011352e-11, 4.290262523963306e-08, 1.4751591879724096e-05, 0.0012714376929894972, 0.032269582852359376, 0.28678403684755305, 1.0564558685825967, 1.8830060019143378, 1.8604591099245638, 1.1442667989198572, 0.4825347017646536, 0.15098045043124428, 0.03735222962768326, 0.007685475172819493, 0.0013685468029883542, 7.213908913424854e-157, 1.2387232459465518e-111, 1.5166268441933996e-78, 1.6396600594309494e-54, 3.4105985353864547e-37, 7.79238658857714e-25, 4.0116793653998946e-16, 4.3927339583077e-10, 5.381411656882358e-06, 0.0025043361526994943, 0.10850627585388774, 0.8426555044330865, 1.889806078312106, 1.7302645476019891, 0.830916681201382, 0.0, 0.0, 0.0, 3.2248443388499574e-226, 1.098366713242237e-155, 3.411536742070096e-106, 1.307438408195689e-71, 1.4230132439323618e-47, 5.505398019894735e-31, 1.1877572757167274e-19, 5.112678192494926e-12, 5.497200088723801e-07, 0.0008778658736915238, 0.07308505523672307, 0.7671854438424938, 1.8882711881398622, 0.0, 0.0, 0.0, 0.0, 8.289926533505022e-301, 2.4448340273449345e-204, 8.887827420334959e-138, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 667.3760625644745];
pub const WAVELETS_MORSE: [f64; 109] = [0.00651008602816823, -3.821964966732594e-11, -6.440831316484319e-07, -0.000879146207038474, -0.0001720521671521244, -0.007606756659199595, -1.1045875039292219e-08, 3.084487666282082e-05, 0.0034448279542203027, 0.015315111013720474, -0.0007177531614250606, -6.930585891255978e-07, 0.0007281016168638904, -0.012163003422748701, -0.028967893262154893, 0.029557320559185505, -0.00016051617823434512, 0.0029694699380054517, -0.014791795275626594, 0.04665876692957241, -0.030995117907235758, -0.014661105559548422, -0.07766421835341111, 0.03755003263565668, -0.06026649954179569, 0.009906578603386944, -0.19887300677727934, 0.17221455899104926, 0.05020952666731703, 0.007988995743861992, 0.005996790557960828, 0.04015055400808569, -0.013903197624905234, -0.030513451657795405, 0.05820298189685333, -0.0027237760291851307, -0.028610414300847813, -0.0008809271247783466, -0.009072798231778364, 0.0189399624812326, -0.014211882817546082, 0.026403964505401722, 1.6684694078734907e-06, 0.00033322441155163654, 0.004082474279539722, 0.01702789271768361, -0.018230695646594228, 2.937805689014498e-05, 8.506077492330399e-07, 0.0004893812817816645, -0.0005892379239190483, 0.004103379512856048, 0.0003909017660900056, 1.1442170182261041e-10, 0.0, -3.74950070991531e-12, 1.9309311409462793e-06, 0.0008001195330077336, -0.009593610990750491, -0.0033177763677241217, -8.03250244096887e-09, 3.1909899907450745e-05, -0.0016940683549192606, 0.005640694025519021, -0.02274890649825838, -1.4630071758383971e-06, 0.000472007487224002, -0.005343456875154747, 0.0021274964013825465, 0.012609403616959024, 5.895800241675561e-05, 0.011227481068999089, 0.03625434108824316, -0.003759937906454013, 0.022362802013490835, 0.018810847460817987, -0.001899851396729723, 0.0764336014951226, -0.026424213785852406, -0.044284047158207954, -0.1399957692075729, 0.0, 0.10065498271199695, 0.07322580088012648, 0.0482767239651727, 0.07028670614841713, -0.06927329711876466, 0.06192251186931273, -0.0009059676752003835, -0.044300038301511725, -0.007539132362970128, -0.0020051279332695573, -0.009950638891302557, -0.02002883717165479, 0.028838531516595465, -0.009152171156077436, -1.4989858426392444e-06, 0.0003408747466319851, -0.004713891317115805, -0.0017963154317406534, 0.018632950271348047, -2.12602285699047e-05, -1.052763199286405e-06, -2.270804387971559e-05, -0.004974483435991406, -0.013675305651872102, -5.540041133285378e-05, -4.0747918927941384e-11, 5.207748177626023];
pub const WAVELETS_BUMP: [f64; 61] = [-0.008383397997454717, -0.004842679916223415, 0.005844946773252863, -0.00044239297238376704, -0.003794106208123524, -0.023643605619453882, -0.012263621891325668, 0.005888695562600282, 0.010117746718676202, 0.04282458774860635, 0.04346263743610457, -0.024684902247460363, -0.06068505011973713, -0.018778086201826, -0.07346173621804228, 0.057223006350580724, -0.02667526090559255, -0.014775869456031848, -0.0678142239471254, -0.01963260951982152, -0.019427267808658392, -0.04115618436331083, -0.03298866105168069, 0.00823296640726535, -0.0009609860879787723, 0.0035664380421789495, -0.016785442108968873, 0.0023366411916563336, 0.0008830247503629138, 0.0034963125752459495, 0.0, 0.0005797141482562218, 0.008093223339657965, -0.008762162835035496, 0.020018038124372688, 0.007112566578841392, 0.002709021082011231, 0.008003642934844808, -0.0337012825948354, 0.012466518659778516, -0.002546010590990324, -0.0366855714580343, -0.06361064416156592, -0.08550971993190785, 0.011365769142129743, 0.0, 0.03915296897919843, 0.10228369844337251, -0.03922991454924411, -0.05846743067705008, -0.04545186321827818, -0.014229910004835249, 0.015721697536204247, 0.006544420111199503, -7.84521739547496e-05, 0.003573740631142436, 0.006931267547011066, -0.01505875212314913, 0.010871022180276616, 0.000710370620960632, 2.212417841094825];
pub const WSST_TONE_AMOR: [f64; 49] = [0.4305134838070796, -0.43051348380707893, 0.4305134838070791, -0.43051348380707916, 0.4305134838070791, -0.430513483807079, 0.4305134838070792, -0.4305134838070791, 0.4305134838070791, -0.43051348380707893, 0.4305134838070791, -0.4305134838070792, 0.430513483807079, -0.4305134838070792, 0.4305134838070791, -0.4305134838070788, -1.0471502140121312e-15, 1.2924545144357169e-16, -4.532007251392499e-16, 7.31405657540397e-16, -1.0838710624613072e-15, 1.39791648890242e-15, -1.5421027395370474e-15, 1.7235594533983712e-15, -2.0824667181162667e-15, 1.9397242591177197e-15, -2.9469794307234084e-15, 2.7206654592764143e-15, -3.1136798481362035e-15, 3.5055040320067792e-15, -3.335725385339384e-15, 4.282755356449315e-15, 0.4305134838070796, 0.43051348380707893, 0.4305134838070791, 0.43051348380707916, 0.4305134838070791, 0.430513483807079, 0.4305134838070792, 0.4305134838070791, 0.4305134838070791, 0.43051348380707893, 0.4305134838070791, 0.4305134838070792, 0.430513483807079, 0.4305134838070792, 0.4305134838070791, 0.4305134838070788, 189.79006437346106];
pub const WSST_TONES_AMOR: [f64; 49] = [0.43051348380707843, -0.5827229402536974, 0.645770161435324, -0.5827229402536969, 0.4305134838070792, -0.278304027360462, 0.21525680617883405, -0.27830402736046106, 0.43051348380707805, -0.5827229402537002, 0.6457701614353237, -0.5827229402536974, 0.43051348380707444, -0.27830402736046267, 0.21525680617883436, -0.2783040273604637, -0.2152566776282478, 0.15220945644661876, -9.240092913305239e-16, -0.1522094564466175, 0.2152566776282451, -0.15220945644661635, -8.369941255135893e-16, 0.15220945644661882, -0.21525667762824865, 0.1522094564466191, -4.676940585103289e-15, -0.1522094564466166, 0.21525667762823697, -0.15220945644661765, -2.8862280825973647e-15, 0.15220945644662554, 0.41816545585557235, 0.41271212750378705, 0.4091181513741883, 0.41271212750378694, 0.4181654558555727, 0.404828809816858, 0.4048212419059567, 0.4048288098168581, 0.4181654558555726, 0.41271212750378833, 0.40911815137418794, 0.41271212750378516, 0.4181654558555711, 0.4048288098168585, 0.40482124190595636, 0.4048288098168602, 205.6657477778804];
pub const IWSST_RANGE: [f64; 16] = [0.01626183509667336, -0.36640559995025007, 0.5114398961329765, -0.3664055999502491, 0.016261835096674906, 0.33388192975689684, -0.4774184393933177, 0.33388192975689884, 0.01626183509667257, -0.36640559995025146, 0.5114398961329771, -0.3664055999502559, 0.01626183509666821, 0.333881929756897, -0.47741843939331585, 0.33388192975690034];
pub const IWSST_RIDGE: [f64; 16] = [0.983738382391891, -0.9871479794549493, 0.9832788844519926, -0.987147979454949, 0.9837383823918915, -0.9803287853288328, 0.9744954255422913, -0.9803287853288327, 0.9837383823918909, -0.9871479794549542, 0.9832788844519916, -0.9871479794549434, 0.983738382391887, -0.9803287853288345, 0.9744954255422905, -0.9803287853288404];
pub const IWSST_RIDGE_4: [f64; 16] = [0.9803277536809324, -0.9720074325319273, 0.972293158936155, -0.9720074325319271, 0.9803277536809328, -0.9706278234115003, 0.9649249283618315, -0.9706278234115003, 0.9803277536809325, -0.9720074325319311, 0.9722931589361541, -0.9720074325319222, 0.9803277536809287, -0.9706278234115018, 0.9649249283618307, -0.9706278234115071];
pub const WSST_TONES_BUMP: [f64; 49] = [0.14498870414375964, -0.1963914622203496, 0.21768318175905627, -0.19639146222034973, 0.14498870414376, -0.0935859460671703, 0.07229422652846361, -0.09358594606717008, 0.14498870414375953, -0.1963914622203494, 0.21768318175905627, -0.19639146222034987, 0.14498870414376028, -0.09358594606717055, 0.07229422652846342, -0.09358594606716963, -0.07269447761529699, 0.0514027580765899, -2.994481483754419e-16, -0.05140275807658924, 0.07269447761529572, -0.051402758076589214, -3.349081937920532e-16, 0.05140275807659019, -0.07269447761529685, 0.05140275807659061, -1.3092125421145331e-15, -0.05140275807658834, 0.07269447761529517, -0.051402758076588825, -8.300716414320567e-16, 0.0514027580765908, 0.14498870414375997, 0.14498870414375994, 0.14498870414375994, 0.14498870414375994, 0.1449887041437599, 0.14498870414375992, 0.14498870414375997, 0.14498870414375994, 0.14498870414375983, 0.14498870414375986, 0.1449887041437599, 0.14498870414375986, 0.14498870414375994, 0.14498870414375994, 0.1449887041437598, 0.14498870414375983, 26.937560478768695];
pub const WSST_SIG256_NV16_EXT: [f64; 13] = [0.47622733198307265, 0.6366059152283673, 0.36685989501231486, 0.22455185236113592, 0.016549875459925124, 0.06072266844799546, 0.20737915027269593, -0.066583800020538, 0.33718798325306315, 0.41419393943324245, 0.41033730871076846, 0.3971756442397372, 47.16581887988309];
pub const IWSST_ONES: [f64; 2] = [6.968424370675584, 20.666937275464594];
// icwt relative errors: packets morse 1.89e-07, amor 3.68e-06, bump(48 voices) 0.000287; sig256 morse 0.173; complex 0.0306
// iwsst relative errors: tone amor 2.17e-07, tones amor 1.98e-07, packets amor 2.32e-07, tones bump 0.00127, tones bump(48 voices) 0.000167
// PyWavelets not installed: shape check skipped
}
