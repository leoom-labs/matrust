//! References: tool/refs/signal/timedomain.py, SciPy. Every numeric tolerance is
//! explicit. Hand-derived tests cover conventions where SciPy differs.
use matrust::{
    Engine, Matrix,
    dsp::{self, timedomain as td},
};
use serde_json::Value;
use std::sync::atomic::{AtomicBool, Ordering};
fn fixture() -> Value {
    serde_json::from_str(include_str!("fixtures/signal/timedomain.json")).unwrap()
}
fn values(v: &Value) -> Vec<f64> {
    v.as_array()
        .unwrap()
        .iter()
        .map(|x| x.as_f64().unwrap())
        .collect()
}
fn row(x: &[f64]) -> Matrix {
    Matrix::from_column_slice(1, x.len(), x).unwrap()
}
fn close(actual: &[f64], expected: &[f64], tolerance: f64) {
    assert_eq!(actual.len(), expected.len());
    for (i, (&a, &b)) in actual.iter().zip(expected).enumerate() {
        assert!(
            a.is_finite() && (a - b).abs() <= tolerance,
            "sample {i}: {a} != {b}, tolerance {tolerance}"
        );
    }
}
#[test]
fn scipy_peaks_prominence_width_and_all_options() {
    let c = AtomicBool::new(false);
    let f = fixture();
    let p = &f["peaks"];
    let y = row(&values(&p["input"]));
    let base = td::findpeaks(&y, None, &td::PeakOptions::default(), &c).unwrap();
    close(&base.heights, &values(&p["height"]), 0.);
    close(&base.locations, &values(&p["location"]), 0.);
    close(&base.widths, &values(&p["width"]), 2e-14);
    close(&base.prominences, &values(&p["prominence"]), 0.);
    let cases = [
        ("'MinPeakHeight',3", vec![4.]),
        ("'SortStr','none'", vec![1., 3., 2., 4., 2.]),
        ("'WidthReference','halfprom'", vec![1., 3., 2., 4., 2.]),
        ("'MinPeakProminence',2.5", vec![3., 4.]),
        ("'Threshold',2.5", vec![3., 4.]),
        ("'MinPeakDistance',5", vec![1., 4.]),
        ("'NPeaks',2", vec![1., 3.]),
        ("'SortStr','ascend'", vec![1., 2., 2., 3., 4.]),
        ("'SortStr','descend','NPeaks',2", vec![4., 3.]),
        ("'WidthReference','halfheight'", vec![1., 3., 2., 4., 2.]),
        ("'MinPeakWidth',1.1", vec![4.]),
        ("'MaxPeakWidth',1.1", vec![1., 3., 2., 2.]),
    ];
    for (options, expected) in cases {
        let mut e = Engine::default();
        e.variables.insert("x".into(), y.clone());
        e.execute(&format!("[p,l,w,q]=findpeaks(x,{options});"))
            .unwrap();
        close(e.variables["p"].data(), &expected, 0.);
    }
    let x: Vec<_> = (0..y.len()).map(|j| 2. * j as f64).collect();
    let scaled = td::findpeaks(&y, Some(&x), &td::PeakOptions::default(), &c).unwrap();
    close(
        &scaled.widths,
        &base.widths.iter().map(|v| v * 2.).collect::<Vec<_>>(),
        2e-14,
    );
    close(
        &scaled.locations,
        &base
            .locations
            .iter()
            .map(|v| 2. * (v - 1.))
            .collect::<Vec<_>>(),
        0.,
    );
    let plateau = td::findpeaks(
        &row(&[0., 2., 2., 2., 0.]),
        None,
        &td::PeakOptions::default(),
        &c,
    )
    .unwrap();
    close(&plateau.locations, &[2.], 0.);
    close(&plateau.widths, &[3.], 0.);
    // Equal peaks do not truncate one another's prominence search.
    let equal = td::findpeaks(
        &row(&[0., 3., 1., 3., 0.]),
        None,
        &td::PeakOptions::default(),
        &c,
    )
    .unwrap();
    close(&equal.prominences, &[3., 3.], 0.);
    // Nonzero baseline distinguishes half height from half prominence.
    let yy = row(&[2., 4., 2.]);
    let half = td::PeakOptions {
        half_height: true,
        ..Default::default()
    };
    close(
        &td::findpeaks(&yy, None, &half, &c).unwrap().widths,
        &[2.],
        0.,
    );
    close(
        &td::findpeaks(&yy, None, &td::PeakOptions::default(), &c)
            .unwrap()
            .widths,
        &[1.],
        0.,
    );
}
#[test]
fn scipy_rate_conversion_and_orientation() {
    let f = fixture();
    let x = row(&values(&f["input"]));
    let c = AtomicBool::new(false);
    for (p, q, key) in [(3, 2, "resample_3_2"), (2, 3, "resample_2_3")] {
        let y = td::resample(&x, p, q, 10, 5., &c).unwrap();
        close(y.data(), &values(&f[key]), 3e-14);
        assert_eq!(y.rows(), 1);
        let col = Matrix::from_column_slice(x.len(), 1, x.data()).unwrap();
        let z = td::resample(&col, p, q, 10, 5., &c).unwrap();
        assert_eq!(z.cols(), 1);
        close(z.data(), y.data(), 0.);
        close(
            td::resample(&x, 2 * p, 2 * q, 10, 5., &c).unwrap().data(),
            y.data(),
            0.,
        );
    }
    close(
        td::resample(&x, 3, 2, 6, 3., &c).unwrap().data(),
        &values(&f["resample_custom"]),
        3e-14,
    );
    close(
        td::decimate(&x, 4, None, false, &c).unwrap().data(),
        &values(&f["decimate4"]),
        3e-12,
    );
    close(
        td::decimate(&x, 4, Some(30), true, &c).unwrap().data(),
        &values(&f["decimate4_fir"]),
        3e-14,
    );
    close(
        td::interp(&x, 4, 4, 0.5, &c).unwrap().data(),
        &values(&f["interp4"]),
        3e-14,
    );
}
#[test]
fn interp_is_cardinal_and_unit_rate_is_identity() {
    let c = AtomicBool::new(false);
    let mut engine = Engine::default();
    engine.execute("x=1:8; y=interp(x,1);").unwrap();
    assert_eq!(engine.variables["x"].data(), engine.variables["y"].data());
    for alpha in [0.25, 0.5, 1.] {
        let x = Matrix::from_column_vec(4, 2, (1..=8).map(f64::from).collect()).unwrap();
        assert_eq!(td::interp(&x, 1, 4, alpha, &c).unwrap().data(), x.data());
    }
    // Cardinal sinc samples are delta[k], independently of the fixture design.
    // Measure relative pointwise error away from zeros; use an absolute bound
    // at sine zero crossings, where a relative error is undefined.
    for frequency in [0.1, 0.3, 0.45] {
        let x: Vec<_> = (0..512)
            .map(|j| (std::f64::consts::PI * frequency * j as f64).sin())
            .collect();
        let y = td::interp(&row(&x), 4, 4, 0.5, &c).unwrap();
        for (j, &expected) in x.iter().enumerate().take(480).skip(32) {
            let tolerance = if expected.abs() < 1e-8 {
                1e-14
            } else {
                1e-6 * expected.abs()
            };
            assert!(
                (y.data()[4 * j] - expected).abs() <= tolerance,
                "frequency={frequency}, sample={j}"
            );
        }
    }
}

#[test]
fn remez_ordinary_and_high_order_designs_meet_hand_checked_specifications() {
    let c = AtomicBool::new(false);
    for (orders, bands) in [
        (&[8, 10, 50][..], &[0., 0.3, 0.4, 0.6, 0.7, 1.][..]),
        (&[18, 30][..], &[0., 0.25, 0.3, 0.6, 0.65, 1.][..]),
    ] {
        for &order in orders {
            let h = td::firpm(
                order,
                bands,
                &[0., 0., 1., 1., 0., 0.],
                None,
                td::RemezKind::Symmetric,
                &c,
            )
            .unwrap();
            close(&h, &h.iter().rev().copied().collect::<Vec<_>>(), 0.);
            for band in 0..3 {
                let f: Vec<_> = (0..=1000)
                    .map(|j| {
                        bands[2 * band] + (bands[2 * band + 1] - bands[2 * band]) * j as f64 / 1000.
                    })
                    .collect();
                let response = dsp::response(&h, &[1.], &f, &c).unwrap();
                let desired = if band == 1 { 1. } else { 0. };
                // A constant 1/2 is feasible with error 1/2. The minimax FIR
                // must improve on it; this also detects a degenerate zero FIR.
                assert!(
                    response
                        .iter()
                        .all(|z| (z[0].hypot(z[1]) - desired).abs() < 0.5)
                );
            }
        }
    }
    for order in [160, 164, 176, 192, 208, 224, 240, 252] {
        eprintln!("Hand-checked lowpass order {order}");
        let h = td::firpm(
            order,
            &[0., 0.2, 0.3, 1.],
            &[1., 1., 0., 0.],
            None,
            td::RemezKind::Symmetric,
            &c,
        )
        .unwrap();
        for (lo, hi, desired) in [(0., 0.2, 1.), (0.3, 1., 0.)] {
            let f: Vec<_> = (0..=2048)
                .map(|j| lo + (hi - lo) * j as f64 / 2048.)
                .collect();
            let response = dsp::response(&h, &[1.], &f, &c).unwrap();
            let error = response
                .iter()
                .map(|z| (z[0].hypot(z[1]) - desired).abs())
                .fold(0., f64::max);
            assert!(
                error < 1e-6,
                "order={order}, band={lo}..{hi}, error={error}"
            );
        }
    }
}

#[test]
fn differentiator_uses_endpoint_amplitudes() {
    let mut engine = Engine::default();
    engine
        .execute("b=firpm(21,[0 1],[0 pi],'differentiator');")
        .unwrap();
    let c = AtomicBool::new(false);
    let response = dsp::response(engine.variables["b"].data(), &[1.], &[0.25, 0.5], &c).unwrap();
    // An ideal discrete-time differentiator has magnitude omega (radians/sample).
    for (z, f) in response.iter().zip([0.25, 0.5]) {
        let actual = z[0].hypot(z[1]);
        let expected = std::f64::consts::PI * f;
        assert!(
            (actual - expected).abs() < 0.01 * expected,
            "f={f}, magnitude={actual}, expected={expected}"
        );
    }
}

#[test]
fn peak_height_boundary_is_strict_including_plateaus() {
    let mut engine = Engine::default();
    engine
        .execute("[p,l]=findpeaks([0 3 0 3 3 0 4 0],'MinPeakHeight',3);")
        .unwrap();
    close(engine.variables["p"].data(), &[4.], 0.);
    close(engine.variables["l"].data(), &[7.], 0.);
    engine
        .execute("p=findpeaks([-3 -2 -3 -1 -3],'MinPeakHeight',-2);")
        .unwrap();
    close(engine.variables["p"].data(), &[-1.], 0.);
}

#[test]
fn common_windows_and_default_bandpass_accept_the_full_sample_limit() {
    let c = AtomicBool::new(false);
    let n = 1_048_576;
    let x = Matrix::filled(n, 1, 1.).unwrap();
    let y = td::medfilt1(&x, 101, false, &c).unwrap();
    assert_eq!(y.rows(), n);
    // With odd-window zero padding, each endpoint still has 51 ones/50 zeros.
    assert!(y.data().iter().all(|&v| v == 1.));
    assert!(
        td::medfilt1(&x, 4097, false, &c)
            .unwrap_err()
            .contains("work limit")
    );
    let mut impulse = vec![0.; n];
    impulse[n / 2] = 1.;
    let x = Matrix::from_column_vec(n, 1, impulse).unwrap();
    let (y, h) =
        td::convenience_filter(&x, &[0.1, 0.2], None, dsp::Band::Bandpass, 0.85, 60., &c).unwrap();
    let begin = n / 2 - h.len() / 2;
    close(&y.data()[begin..begin + h.len()], &h, 0.);
    assert!(
        y.data()[..begin]
            .iter()
            .chain(&y.data()[begin + h.len()..])
            .all(|&v| v == 0.)
    );
}

#[test]
fn fir2_accepts_final_window_in_all_three_positions_and_reports_syntax() {
    let mut engine = Engine::default();
    engine.execute("w=hann(17); a=fir2(16,[0 1],[1 0],w); b=fir2(16,[0 1],[1 0],512,w); c=fir2(16,[0 1],[1 0],512,0,w); d=fir2(16,[0 1],[1 0],512,w');").unwrap();
    for name in ["b", "c", "d"] {
        close(
            engine.variables[name].data(),
            engine.variables["a"].data(),
            0.,
        );
    }
    for source in [
        "fir2(16,[0 1],[1 0],512,'wrong')",
        "fir2(16,[0 1],[1 0],512,[1 2;3 4])",
        "fir2(16,[0 1],[1 0],512,w,0)",
        "fir2(16,[0 1],[1 0],512,0,'wrong')",
        "fir2(16,[0 1],[1 0],512,0.5)",
    ] {
        let error = engine.execute(source).unwrap_err();
        assert!(error.contains("fir2 syntax:"), "{source}: {error}");
        assert!(!error.contains("Expected a scalar"));
    }
}

#[test]
fn median_explicit_zeropad_matches_default() {
    let mut engine = Engine::default();
    engine.execute("x=[5 9 2]; a=medfilt1(x,4); b=medfilt1(x,4,'zeropad'); c=medfilt1(x,'zeropad'); d=medfilt1(x); t=medfilt1(x,4,'truncate');").unwrap();
    close(engine.variables["a"].data(), &[2.5, 3.5, 3.5], 0.);
    close(
        engine.variables["b"].data(),
        engine.variables["a"].data(),
        0.,
    );
    close(
        engine.variables["c"].data(),
        engine.variables["d"].data(),
        0.,
    );
    close(engine.variables["t"].data(), &[7., 5., 5.], 0.);
}
#[test]
fn scipy_hilbert_and_envelopes() {
    let f = fixture();
    let x = row(&values(&f["input"]));
    let c = AtomicBool::new(false);
    let z = td::hilbert(&x, &c).unwrap();
    close(
        &z.data().iter().map(|v| v.re).collect::<Vec<_>>(),
        &values(&f["hilbert_real"]),
        3e-14,
    );
    close(
        &z.data().iter().map(|v| v.im).collect::<Vec<_>>(),
        &values(&f["hilbert_imag"]),
        3e-14,
    );
    for n in [31, 32, 127, 128] {
        let cosine: Vec<_> = (0..n)
            .map(|j| (2. * std::f64::consts::PI * 3. * j as f64 / n as f64).cos())
            .collect();
        let columns =
            Matrix::from_column_vec(n, 2, cosine.iter().chain(&cosine).copied().collect()).unwrap();
        let analytic = td::hilbert(&columns, &c).unwrap();
        for (j, v) in analytic.data().iter().enumerate() {
            let angle = 2. * std::f64::consts::PI * 3. * (j % n) as f64 / n as f64;
            assert!((v.re - angle.cos()).abs() < 1e-13 && (v.im - angle.sin()).abs() < 1e-13);
        }
    }
    let (up, lo) = td::envelope(&x, 3, td::EnvelopeKind::Analytic, &c).unwrap();
    close(up.data(), &values(&f["envelope_analytic"]), 3e-14);
    let mean = x.data().iter().sum::<f64>() / x.len() as f64;
    close(
        &up.data()
            .iter()
            .zip(lo.data())
            .map(|(a, b)| a + b)
            .collect::<Vec<_>>(),
        &vec![2. * mean; x.len()],
        3e-14,
    );
    let (up, lo) = td::envelope(&x, 5, td::EnvelopeKind::Rms, &c).unwrap();
    close(up.data(), &values(&f["envelope_rms"]), 3e-14);
    close(
        lo.data(),
        &up.data().iter().map(|v| -v).collect::<Vec<_>>(),
        0.,
    );
    let (up, lo) = td::envelope(
        &row(&[0., 2., 0., -2., 0., 2., 0.]),
        1,
        td::EnvelopeKind::Peak,
        &c,
    )
    .unwrap();
    close(up.data(), &[0., 2., 2., 2., 2., 2., 0.], 0.);
    close(
        lo.data(),
        &[0., -2. / 3., -4. / 3., -2., -4. / 3., -2. / 3., 0.],
        5e-16,
    );
}
#[test]
fn scipy_medians_and_savitzky_golay() {
    let f = fixture();
    let x = row(&values(&f["input"]));
    let c = AtomicBool::new(false);
    for (n, truncate, key) in [
        (3, false, "median3"),
        (4, false, "median4"),
        (4, true, "median4_truncate"),
    ] {
        close(
            td::medfilt1(&x, n, truncate, &c).unwrap().data(),
            &values(&f[key]),
            1e-15,
        );
    }
    let (b, g) = td::sgolay(3, 7, &c).unwrap();
    for j in 0..7 {
        close(
            &(0..7).map(|k| b.get(j, k)).collect::<Vec<_>>(),
            &values(&f["sgolay_b"][j]),
            1e-12,
        );
    }
    for k in 0..4 {
        close(
            &g.data()[7 * k..7 * (k + 1)],
            &values(&f["sgolay_g"][k]),
            1e-12,
        );
    }
    for i in 0..7 {
        for j in 0..7 {
            assert!((b.get(i, j) - b.get(j, i)).abs() < 1e-12);
        }
    }
    close(
        td::sgolayfilt(&x, 3, 7, &c).unwrap().data(),
        &values(&f["sgolayfilt"]),
        1e-12,
    );
    let polynomial: Vec<_> = (0..20).map(|j| (j as f64).powi(3)).collect();
    close(
        td::sgolayfilt(&row(&polynomial), 3, 7, &c).unwrap().data(),
        &polynomial,
        2e-10,
    );
}
#[test]
fn kaiser_filters_meet_stopband_and_struct_contract() {
    let c = AtomicBool::new(false);
    for (kind, edges, pass, stop) in [
        (dsp::Band::Lowpass, vec![0.3], 0.1, 0.5),
        (dsp::Band::Highpass, vec![0.6], 0.9, 0.4),
        (dsp::Band::Bandpass, vec![0.3, 0.6], 0.45, 0.1),
        (dsp::Band::Bandstop, vec![0.3, 0.6], 0.1, 0.45),
    ] {
        let (_, h) =
            td::convenience_filter(&row(&[1., 0., 0.]), &edges, None, kind, 0.75, 60., &c).unwrap();
        let response = dsp::response(&h, &[1.], &[pass, stop], &c).unwrap();
        assert!((response[0][0].hypot(response[0][1]) - 1.).abs() < 0.002);
        assert!(response[1][0].hypot(response[1][1]) < 0.0012);
        for (a, b) in h.iter().zip(h.iter().rev()) {
            assert!((a - b).abs() < 1e-14);
        }
    }
    let mut e = Engine::default();
    e.execute("[y,d]=lowpass([0 1 0],30,200,'Steepness',0.75,'StopbandAttenuation',60); n=d.order; b=d.coefficients; t=d.type;").unwrap();
    assert_eq!(
        e.variables["n"].data()[0] as usize + 1,
        e.variables["b"].len()
    );
}
#[test]
fn scipy_frequency_sampling_and_iterative_remez() {
    let c = AtomicBool::new(false);
    let f = fixture();
    for (n, ff, mm, key) in [
        (32, vec![0., 0.3, 0.5, 1.], vec![1., 1., 0., 0.], "fir2"),
        (
            32,
            vec![0., 0.4, 0.4, 1.],
            vec![1., 1., 0., 0.],
            "fir2_repeat",
        ),
        (31, vec![0., 0.3, 0.5, 1.], vec![1., 1., 0., 0.], "fir2_odd"),
    ] {
        close(
            &td::fir2(n, &ff, &mm, None, 0, None, &c).unwrap(),
            &values(&f[key]),
            2e-14,
        );
    }
    let window: Vec<_> = (0..17)
        .map(|j| 0.5 - 0.5 * (2. * std::f64::consts::PI * j as f64 / 16.).cos())
        .collect();
    close(
        &td::fir2(
            16,
            &[0., 0.4, 0.4, 1.],
            &[1., 1., 0., 0.],
            Some(256),
            4,
            Some(&window),
            &c,
        )
        .unwrap(),
        &values(&f["fir2_lap_window"]),
        2e-14,
    );
    let mut failures = Vec::new();
    for case in f["firpm"].as_array().unwrap() {
        let kind = match case["kind"].as_str().unwrap() {
            "hilbert" => td::RemezKind::Hilbert,
            "differentiator" => td::RemezKind::Differentiator,
            _ => td::RemezKind::Symmetric,
        };
        let result = td::firpm(
            case["order"].as_u64().unwrap() as usize,
            &values(&case["bands"]),
            &values(&case["amplitudes"]),
            Some(&values(&case["weights"])),
            kind,
            &c,
        );
        match result {
            Ok(h) => {
                let expected = values(&case["coefficients"]);
                assert_eq!(h.len(), expected.len());
                let error = h
                    .iter()
                    .zip(&expected)
                    .map(|(a, b)| (a - b).abs())
                    .fold(0., f64::max);
                eprintln!("Remez {}: max tap error {error:e}", case["name"]);
                if h.iter().any(|v| !v.is_finite()) || error > 1e-6 {
                    failures.push(format!("{}: tap error {error:e} > 1e-6", case["name"]));
                }
            }
            Err(error) => failures.push(format!("{}: {error}", case["name"])),
        }
    }
    assert!(failures.is_empty(), "{}", failures.join("\n"));
}
#[test]
fn engine_dispatch_help_examples_and_rejections() {
    let mut e = Engine::default();
    e.execute("[p,l,w,q]=findpeaks([0 2 0 1 0]); [u,v]=envelope([0 1 0 -1],3,'rms'); [b,g]=sgolay(2,5); z=hilbert([1 0 -1 0]);").unwrap();
    close(e.variables["l"].data(), &[2., 4.], 0.);
    assert_eq!(e.complex_variables["z"].cols(), 4);
    e.execute("findpeaks([0 2 0 1 0]);").unwrap();
    assert_eq!(e.figure.as_ref().unwrap().series_count(), 2);
    for name in [
        "findpeaks",
        "resample",
        "decimate",
        "interp",
        "hilbert",
        "envelope",
        "medfilt1",
        "sgolayfilt",
        "sgolay",
        "lowpass",
        "highpass",
        "bandpass",
        "bandstop",
        "fir2",
        "firpm",
    ] {
        assert!(matrust::session::FUNCTIONS.contains(&name));
        let page = matrust::help::page(name).unwrap();
        for line in page
            .lines()
            .filter_map(|line| line.strip_prefix("Example: "))
        {
            e.execute(line)
                .unwrap_or_else(|err| panic!("{name}: {line}: {err}"));
        }
    }
    for source in [
        "findpeaks([0 1 0],'bad',2)",
        "findpeaks([0 1 0],[1 1 2])",
        "findpeaks([0 NaN 0])",
        "resample([1 2],0,2)",
        "resample([1 2],100000,1)",
        "interp([1 2],4,4,0)",
        "decimate([1 2],4)",
        "medfilt1([1 2],0)",
        "sgolay(5,5)",
        "sgolay(2,4)",
        "lowpass([1 2],0.2,'Steepness',1)",
        "fir2(4,[0 1],[1 0],2)",
        "firpm(5,[0 1],[1 1])",
    ] {
        assert!(e.execute(source).is_err(), "accepted {source}");
    }
}
#[test]
fn empty_singleton_cancellation_and_bounds() {
    let c = AtomicBool::new(false);
    let empty = Matrix::filled(0, 0, 0.).unwrap();
    assert!(td::resample(&empty, 3, 2, 10, 5., &c).unwrap().is_empty());
    assert!(td::hilbert(&empty, &c).unwrap().is_empty());
    close(
        td::medfilt1(&row(&[2.]), 4, true, &c).unwrap().data(),
        &[2.],
        0.,
    );
    c.store(true, Ordering::Relaxed);
    assert!(td::resample(&row(&[1.]), 1, 1, 10, 5., &c).is_err());
    assert!(td::findpeaks(&row(&[0., 1., 0.]), None, &td::PeakOptions::default(), &c).is_err());
    assert!(td::hilbert(&row(&[1.]), &c).is_err());
    assert!(td::sgolay(2, 5, &c).is_err());
    assert!(
        td::firpm(
            8,
            &[0., 0.3, 0.5, 1.],
            &[1., 1., 0., 0.],
            None,
            td::RemezKind::Symmetric,
            &c
        )
        .is_err()
    );
}

#[test]
fn dense_repeating_peaks_have_bounded_searches_and_deterministic_ties() {
    let c = AtomicBool::new(false);
    // Every equal peak shares the same distant bases: naive scans are quadratic.
    let mut signal = vec![1.5; 32769];
    signal[0] = 0.;
    signal[32768] = 0.;
    for j in (1..32768).step_by(2) {
        signal[j] = 2.;
    }
    let p = td::findpeaks(&row(&signal), None, &td::PeakOptions::default(), &c).unwrap();
    assert_eq!(p.heights.len(), 16384);
    assert!(p.prominences.iter().all(|v| *v == 2.));
    assert!(p.widths.iter().all(|v| *v == 32767.));
    let o = td::PeakOptions {
        min_distance: 3.,
        ..Default::default()
    };
    let p = td::findpeaks(&row(&[0., 2., 0., 2., 0., 2., 0.]), None, &o, &c).unwrap();
    close(&p.locations, &[2., 6.], 0.);
}
#[test]
fn shared_pool_resampling_is_bitwise_equal_to_serial() {
    let c = AtomicBool::new(false);
    let x = Matrix::from_column_vec(
        22003,
        3,
        (0..66009).map(|j| (j as f64 * 0.13).sin()).collect(),
    )
    .unwrap();
    let serial = td::resample(&x, 3, 2, 10, 5., &c).unwrap();
    let pool = rayon::ThreadPoolBuilder::new()
        .num_threads(4)
        .build()
        .unwrap();
    let parallel = pool.install(|| td::resample(&x, 3, 2, 10, 5., &c)).unwrap();
    assert_eq!(serial.data(), parallel.data());
}
#[test]
fn full_stopband_response_and_delay_compensation() {
    let c = AtomicBool::new(false);
    let mut impulse = vec![0.; 513];
    impulse[256] = 1.;
    let input = row(&impulse);
    for (kind, edges) in [
        (dsp::Band::Lowpass, vec![0.3]),
        (dsp::Band::Highpass, vec![0.6]),
        (dsp::Band::Bandpass, vec![0.3, 0.6]),
        (dsp::Band::Bandstop, vec![0.3, 0.6]),
    ] {
        let (y, h) = td::convenience_filter(&input, &edges, None, kind, 0.75, 60., &c).unwrap();
        let width = match kind {
            dsp::Band::Lowpass => 1. - edges[0],
            dsp::Band::Highpass => edges[0],
            dsp::Band::Bandpass => edges[0].min(1. - edges[1]),
            dsp::Band::Bandstop => (edges[1] - edges[0]) / 2.,
        } * 0.25;
        let f: Vec<_> = (0..=2048)
            .map(|j| j as f64 / 2048.)
            .filter(|&f| match kind {
                dsp::Band::Lowpass => f >= edges[0] + width,
                dsp::Band::Highpass => f <= edges[0] - width,
                dsp::Band::Bandpass => f <= edges[0] - width || f >= edges[1] + width,
                dsp::Band::Bandstop => f >= edges[0] + width && f <= edges[1] - width,
            })
            .collect();
        let hz = dsp::response(&h, &[1.], &f, &c).unwrap();
        let max = hz.iter().map(|v| v[0].hypot(v[1])).fold(0., f64::max);
        // Kaiser's empirical 60-dB formula permits a first-lobe overshoot.
        assert!(max < 0.0012, "{:?} stopband max {max}", kind);
        let half = h.len() / 2;
        close(&y.data()[256 - half..=256 + half], &h, 1e-15);
        let (_, physical) = td::convenience_filter(
            &input,
            &edges.iter().map(|v| v * 100.).collect::<Vec<_>>(),
            Some(200.),
            kind,
            0.75,
            60.,
            &c,
        )
        .unwrap();
        close(&physical, &h, 1e-14);
    }
}

#[test]
fn analytic_constant_scalar_and_nyquist_signals_remain_complex() {
    let c = AtomicBool::new(false);
    for x in [vec![3.], vec![2.; 16], vec![1., -1., 1., -1.]] {
        let z = td::hilbert(&row(&x), &c).unwrap();
        close(
            &z.data().iter().map(|v| v.re).collect::<Vec<_>>(),
            &x,
            1e-14,
        );
        close(
            &z.data().iter().map(|v| v.im).collect::<Vec<_>>(),
            &vec![0.; x.len()],
            1e-14,
        );
    }
    let (up, lo) = td::envelope(&row(&[2.; 16]), 3, td::EnvelopeKind::Analytic, &c).unwrap();
    close(up.data(), &[2.; 16], 1e-14);
    close(lo.data(), &[2.; 16], 1e-14);
    close(
        &td::firpm(8, &[0., 1.], &[1., 1.], None, td::RemezKind::Symmetric, &c).unwrap(),
        &[0., 0., 0., 0., 1., 0., 0., 0., 0.],
        0.,
    );
}

#[test]
fn peak_plateau_threshold_and_nonzero_baseline_options() {
    let mut e = Engine::default();
    e.execute("[p,l,w,q]=findpeaks([0 2 2 2 0],'Threshold',1); [a,b,c,d]=findpeaks([2 4 2],'WidthReference','halfheight'); [v,u,z,t]=findpeaks([2 4 2],'WidthReference','halfprom'); [f,g,h,i]=findpeaks([0 1 0],'NPeaks',0);").unwrap();
    close(e.variables["l"].data(), &[2.], 0.);
    close(e.variables["w"].data(), &[3.], 0.);
    close(e.variables["c"].data(), &[2.], 0.);
    close(e.variables["z"].data(), &[1.], 0.);
    assert!(e.variables["f"].is_empty());
}

#[test]
fn other_independent_kernels_match_serial_bit_for_bit() {
    let c = AtomicBool::new(false);
    let x = Matrix::from_column_vec(
        22003,
        3,
        (0..66009).map(|j| (j as f64 * 0.13).sin()).collect(),
    )
    .unwrap();
    let pool = rayon::ThreadPoolBuilder::new()
        .num_threads(4)
        .build()
        .unwrap();
    let median = td::medfilt1(&x, 9, true, &c).unwrap();
    let smooth = td::sgolayfilt(&x, 3, 17, &c).unwrap();
    let env = td::envelope(&x, 17, td::EnvelopeKind::Rms, &c).unwrap();
    let dec = td::decimate(&x, 4, None, false, &c).unwrap();
    pool.install(|| {
        assert_eq!(median.data(), td::medfilt1(&x, 9, true, &c).unwrap().data());
        assert_eq!(smooth.data(), td::sgolayfilt(&x, 3, 17, &c).unwrap().data());
        let p = td::envelope(&x, 17, td::EnvelopeKind::Rms, &c).unwrap();
        assert_eq!(env.0.data(), p.0.data());
        assert_eq!(env.1.data(), p.1.data());
        assert_eq!(
            dec.data(),
            td::decimate(&x, 4, None, false, &c).unwrap().data()
        );
    });
}
