//! MATLAB-form filter analysis: freqz, grpdelay, phasez, impz, stepz (outputs, fs, 'whole',
//! frequency vectors, SOS, digitalFilter), bilinear, sos2tf and zplane markers.
//! Expected values: tests/fixtures/signal/filter_analysis_compat.json from
//! tool/refs/signal/filter_analysis_compat.py (SciPy/NumPy); default impz/stepz lengths
//! from GNU Octave signal 1.4.8 (tool/refs/signal/filter_analysis_compat_impz.m).
use matrust::{Array, Engine, plot::MarkerShape};
use serde_json::Value as Json;
use std::fs;

const TOL: f64 = 1e-10;

fn fixture() -> Json {
    serde_json::from_str(
        &fs::read_to_string("tests/fixtures/signal/filter_analysis_compat.json").unwrap(),
    )
    .unwrap()
}

/// Shape and column-major (re, im) data of a numeric workspace variable.
fn numbers(e: &Engine, name: &str) -> ((usize, usize), Vec<(f64, f64)>) {
    match e.workspace().get(name) {
        Some(Array::Numeric(m)) => (
            (m.rows(), m.cols()),
            m.data().iter().map(|&v| (v, 0.)).collect(),
        ),
        Some(Array::Complex(m)) => (
            (m.rows(), m.cols()),
            m.data().iter().map(|v| (v.re, v.im)).collect(),
        ),
        other => panic!("{name}: not numeric: {other:?}"),
    }
}

fn real(e: &Engine, name: &str) -> Vec<f64> {
    numbers(e, name).1.into_iter().map(|v| v.0).collect()
}

fn run(script: &str) -> Engine {
    let mut e = Engine::default();
    e.execute(script)
        .unwrap_or_else(|err| panic!("{script}: {err}"));
    e
}

fn error(script: &str) -> String {
    Engine::default()
        .execute(script)
        .err()
        .unwrap_or_else(|| panic!("{script} should fail"))
}

#[test]
fn outputs_match_scipy_and_octave_references() {
    let refs = fixture();
    let setup = refs["setup"].as_str().unwrap();
    let mut failures = Vec::new();
    let cases = refs["cases"].as_array().unwrap();
    assert!(cases.len() > 80);
    for case in cases {
        let script = case["script"].as_str().unwrap();
        let mut e = Engine::default();
        if let Err(err) = e.execute(&format!("{setup} {script}")) {
            failures.push(format!("{script}: {err}"));
            continue;
        }
        for (name, expected) in case["outputs"].as_object().unwrap() {
            let (shape, actual) = numbers(&e, name);
            let rows = expected["rows"].as_u64().unwrap() as usize;
            let cols = expected["cols"].as_u64().unwrap() as usize;
            if shape != (rows, cols) {
                failures.push(format!(
                    "{script} {name}: size {shape:?}, expected {rows}x{cols}"
                ));
                continue;
            }
            let stride = expected["stride"].as_u64().unwrap() as usize;
            let re = expected["re"].as_array().unwrap();
            let im = expected.get("im").and_then(Json::as_array);
            for (k, value) in actual.iter().step_by(stride).enumerate() {
                let want = (
                    re[k].as_f64().unwrap(),
                    im.map_or(0., |im| im[k].as_f64().unwrap()),
                );
                let scale = want.0.hypot(want.1).max(1.);
                if (value.0 - want.0).hypot(value.1 - want.1) > TOL * scale {
                    failures.push(format!(
                        "{script} {name}({}): {value:?}, expected {want:?}",
                        k * stride + 1
                    ));
                    break;
                }
            }
        }
    }
    assert!(failures.is_empty(), "{}", failures.join("\n"));
}

#[test]
fn default_impulse_lengths_equal_octave_impz() {
    // GNU Octave signal 1.4.8 impz(b, a) / impzlength (filter_analysis_compat_impz.m).
    for (call, length) in [
        ("[0.05 0.2 0.5 0.2 0.05], 1", 5),
        ("1, [1 -0.95]", 193),
        ("1, conv([1 -0.9], [1 -0.9])", 187),
        ("1, [1 0.5 1]", 17),
        ("1, [1 -1.1]", 144),
        ("[0 0 1], [1 -0.95]", 195),
        ("1, [1 -1]", 10),
        (
            "[0.018563010626897178 0.074252042507588711 0.11137806376138307 0.074252042507588711 0.018563010626897178], [1 -1.5703988512281717 1.2756133249832793 -0.4844033683350854 0.076197064610332349]",
            30,
        ),
        (
            "[0.035083422458908969 0.063556749403184482 0.035083422458908969 1 -0.55813690796700388 0.78232220611541203; 1 1.1483925153886769 1 1 -1.0095102382132004 0.39538800204332875]",
            80,
        ),
    ] {
        let e = run(&format!("h = impz({call}); s = stepz({call});"));
        assert_eq!(numbers(&e, "h").0, (length, 1), "impz({call})");
        assert_eq!(numbers(&e, "s").0, (length, 1), "stepz({call})");
    }
    // Poles all at the origin: an FIR filter, length(b).
    assert_eq!(numbers(&run("h = impz([1 2 3], [1 0 0]);"), "h").0, (3, 1));
}

#[test]
fn bilinear_prewarp_matches_the_analog_response_at_fp() {
    let e = run("bs = [0 0 4e4]; as = [1 280 4e4]; fs = 1000; fp = 150; \
         [bz, az] = bilinear(bs, as, fs, fp); \
         s = 2i*pi*fp; ha = polyval(bs, s) / polyval(as, s); \
         hd = freqz(bz, az, [fp 10], fs); \
         [z, p, k] = tf2zp(bs, as); [zd, pd, kd] = bilinear(z, p, k, fs, fp); \
         [bz2, az2] = zp2tf(zd, pd, kd); \
         [bu, au] = bilinear(bs, as, fs); hu = freqz(bu, au, [fp 10], fs);");
    let (ha, hd, hu) = (
        numbers(&e, "ha").1[0],
        numbers(&e, "hd").1[0],
        numbers(&e, "hu").1[0],
    );
    assert!((ha.0 - hd.0).hypot(ha.1 - hd.1) < 1e-12, "{ha:?} vs {hd:?}");
    // Without prewarping the responses differ at fp (frequency warping).
    assert!((ha.0 - hu.0).hypot(ha.1 - hu.1) > 1e-3);
    for (x, y) in [("bz", "bz2"), ("az", "az2")] {
        for (u, v) in real(&e, x).iter().zip(real(&e, y)) {
            assert!((u - v).abs() < 1e-12, "{x} vs {y}");
        }
    }
}

#[test]
fn output_shapes_counts_and_frequency_vectors() {
    let e = run(
        "b = [1 0.5]; a = [1 -0.2]; h = freqz(b, a, 16); [h2, w2] = freqz(b, a, [0 1 2]); \
         [h3, w3] = freqz(b, a, [0; 1]); [g, f] = grpdelay(b, a, [10 20 30], 100); \
         [p, w] = phasez(b, a, 8, 'whole'); h4 = freqz(b, a, [], 50); [h5, t5] = impz(b, a, 6, 2);",
    );
    assert_eq!(numbers(&e, "h").0, (16, 1));
    assert_eq!(numbers(&e, "h2").0, (1, 3));
    assert_eq!(real(&e, "w2"), [0., 1., 2.]);
    assert_eq!(numbers(&e, "h3").0, (2, 1));
    assert_eq!(real(&e, "f"), [10., 20., 30.]);
    assert_eq!(numbers(&e, "f").0, (1, 3));
    assert_eq!(numbers(&e, "p").0, (8, 1));
    let w = real(&e, "w");
    assert!((w[7] - 7. * std::f64::consts::PI / 4.).abs() < 1e-14);
    assert_eq!(numbers(&e, "h4").0, (512, 1));
    assert_eq!(real(&e, "t5"), [0., 0.5, 1., 1.5, 2., 2.5]);
    assert!(matches!(e.workspace().get("h"), Some(Array::Complex(_))));
    for call in [
        "[h, w, x] = freqz([1 1], 1)",
        "[h, t, x] = impz([1 1], 1)",
        "[a, b, c] = bilinear(1, [1 1], 10)",
        "[a, b, c, d] = bilinear([], -1, 1, 10)",
    ] {
        assert!(error(call).contains("output"), "{call}");
    }
}

#[test]
fn digital_filter_forms_match_their_coefficients() {
    let e = run(
        "d = designfilt('lowpassfir', 'FilterOrder', 20, 'CutoffFrequency', 0.4); c = d.Coefficients; \
         [h, f] = freqz(d, 50, 2000); [h2, f2] = freqz(c, 1, 50, 2000); \
         [g, w] = grpdelay(d, 16); [p, w2] = phasez(d, 16, 'whole'); p2 = phasez(c, 1, 16, 'whole'); \
         [i, t] = impz(d); [s, ts] = stepz(d, 30, 1000); s2 = stepz(c, 1, 30); \
         e = designfilt('highpassiir', 'FilterOrder', 6, 'HalfPowerFrequency', 0.4); \
         ie = impz(e, 50); ie2 = sosimpz(e.Coefficients, 50); he = freqz(e, 32); he2 = freqz(e.Coefficients, 32);",
    );
    for (x, y) in [
        ("h", "h2"),
        ("f", "f2"),
        ("p", "p2"),
        ("s", "s2"),
        ("ie", "ie2"),
        ("he", "he2"),
    ] {
        let (a, b) = (numbers(&e, x), numbers(&e, y));
        assert_eq!(a.0, b.0, "{x} vs {y}");
        for (u, v) in a.1.iter().zip(&b.1) {
            assert!((u.0 - v.0).hypot(u.1 - v.1) < 1e-12, "{x} vs {y}");
        }
    }
    assert!(real(&e, "g").iter().all(|g| (g - 10.).abs() < 1e-9));
    assert_eq!(numbers(&e, "i").0, (21, 1));
    assert!((real(&e, "ts")[29] - 0.029).abs() < 1e-15);
}

#[test]
fn bare_calls_plot_in_the_requested_units_without_touching_ans() {
    let mut e = Engine::default();
    e.execute("ans = 7; b = [1 1]; a = [1 -0.5]; freqz(b, a, 64, 1000);")
        .unwrap();
    let figure = e.figure.as_ref().unwrap();
    assert_eq!(figure.xlabel, "Frequency (Hz)");
    assert_eq!(figure.ylabel, "Magnitude (dB)");
    let x: Vec<f64> = figure.series[0].points.iter().map(|p| p[0]).collect();
    assert_eq!(x.len(), 64);
    assert!((x[63] - 500. * 63. / 64.).abs() < 1e-9);
    e.execute("phasez(b, a, 32, 'whole');").unwrap();
    let figure = e.figure.as_ref().unwrap();
    assert_eq!(figure.ylabel, "Phase (rad)");
    assert!((figure.series[0].points.get(31).unwrap()[0] - 2. * 31. / 32.).abs() < 1e-12);
    e.execute("stepz(b, a, [], 100);").unwrap();
    let figure = e.figure.as_ref().unwrap();
    assert_eq!(figure.xlabel, "Time (s)");
    assert_eq!(figure.series[0].points.len(), 14);
    assert!(
        (figure.series[0].points.get(13).unwrap()[1] - (4. - 3. * 0.5f64.powi(13))).abs() < 1e-12
    );
    e.execute("d = designfilt('lowpassiir', 'FilterOrder', 4, 'HalfPowerFrequency', 0.3); grpdelay(d, 20, 50);")
        .unwrap();
    let figure = e.figure.as_ref().unwrap();
    assert_eq!(figure.ylabel, "Group delay (samples)");
    assert_eq!(figure.series[0].points.len(), 20);
    e.execute("impz(d);").unwrap();
    assert_eq!(e.figure.as_ref().unwrap().series[0].points.len(), 30);
    assert_eq!(real(&e, "ans"), [7.]);
    let before = e.figure.clone().unwrap();
    assert!(e.execute("freqz(b, a, 0);").is_err());
    assert!(e.figure.as_ref().unwrap().same_snapshot(&before));
}

#[test]
fn zplane_marks_zeros_poles_and_the_unit_circle_for_every_input_form() {
    let sos = "[1 2 1 1 -0.5 0.3; 1 -1 0.5 1 0.2 0.4]";
    for (script, zeros, poles) in [
        ("zplane([1 1], [1 -0.5 0.25])", 2, 2),
        ("zplane([0.5; -0.5], [0.9i; -0.9i])", 2, 2),
        (&*format!("zplane({sos})"), 4, 4),
    ] {
        let mut e = Engine::default();
        e.execute(script).unwrap();
        let axes = &e.figure.as_ref().unwrap().axes;
        let marked = |shape: MarkerShape| {
            axes.series
                .iter()
                .filter(|s| s.markers && s.effective_marker() == shape)
                .map(|s| s.points.len())
                .sum::<usize>()
        };
        assert_eq!(marked(MarkerShape::Circle), zeros, "{script} zeros");
        assert_eq!(marked(MarkerShape::Cross), poles, "{script} poles");
        // The dotted unit circle: a closed line through radius-1 points.
        assert!(
            axes.series.iter().any(|s| s.points.len() > 300
                && s.points
                    .iter()
                    .all(|p| (p[0].hypot(p[1]) - 1.).abs() < 1e-12)),
            "{script} unit circle"
        );
    }
    // zplane(sos): the roots of each section; poles of [1 -0.5 0.3] and [1 0.2 0.4].
    let mut e = Engine::default();
    e.execute(&format!("zplane({sos})")).unwrap();
    let axes = &e.figure.as_ref().unwrap().axes;
    let poles = axes
        .series
        .iter()
        .find(|s| s.markers && s.effective_marker() == MarkerShape::Cross)
        .unwrap();
    let mut radii: Vec<f64> = poles.points.iter().map(|p| p[0].hypot(p[1])).collect();
    radii.sort_by(f64::total_cmp);
    let expected = [0.4f64.sqrt(), 0.4f64.sqrt(), 0.3f64.sqrt(), 0.3f64.sqrt()];
    let mut expected = expected.to_vec();
    expected.sort_by(f64::total_cmp);
    for (r, x) in radii.iter().zip(&expected) {
        assert!((r - x).abs() < 1e-12, "{radii:?}");
    }
    assert!(error("h = zplane([1 1], 1)").contains("not supported yet"));
}

#[test]
fn bad_shapes_and_arguments_are_rejected() {
    for (script, message) in [
        ("freqz(ones(3), 8)", "vector"),
        ("h = freqz(ones(3, 5))", "vector"),
        ("h = freqz([1 1], 1, ones(2))", "point count"),
        ("h = freqz([1 1], 1, 0)", "integer"),
        ("h = freqz([1 1], 1, 2.5)", "integer"),
        ("h = freqz([1 1], 1, 8, -2)", "fs"),
        ("h = freqz([1 1], 1, [0 1], 'whole')", "whole"),
        ("h = freqz([1 1], 1, 8, 'full')", "option"),
        ("h = freqz([1 1], 1, 8, 10, 20)", "expected"),
        ("h = grpdelay([1 1], [0 1], 8)", "a(1)"),
        ("h = phasez({1}, 1)", "real"),
        ("h = freqz()", "not enough"),
        ("h = impz([1 1], 1, [1 2 3])", "vector of sample indices"),
        ("h = impz(1, [1 -0.9999])", "explicitly"),
        ("h = stepz([1 1], 1, 4, 0)", "fs"),
        ("h = impz([1 1], 1, 4, 10, 1)", "expected"),
        ("h = impz(ones(2, 6), 4, 'x')", "fs"),
        ("b = bilinear(1, [1 1], -5)", "sample rate"),
        ("b = bilinear([1 0 0], [1 1], 10)", "degree"),
        ("b = bilinear(1, [1 1], 10, 6)", "fp"),
        ("b = bilinear(1, [1 1])", "expects"),
        (
            "[z, p, k] = bilinear(eye(2), ones(2, 1), ones(1, 2), 0, 10)",
            "state-space",
        ),
        ("[b, a] = sos2tf(ones(2, 5))", "six columns"),
        ("[b, a] = sos2tf([1 2 1 1 0 0], 'g')", "gain"),
        ("[b, a] = sos2tf([1 2 1 1 0 0], 1, 2)", "optional gain"),
    ] {
        let err = error(script);
        assert!(err.contains(message), "{script}: {err}");
    }
}

#[test]
fn engine_specific_names_keep_their_behaviour() {
    let e = run(
        "sos = butter_sos(4, 0.25); a1 = sosimpz(sos, 40); a2 = impz(sos, 40); \
         g1 = sosgrpdelay(sos, 64); g2 = grpdelay(sos, 64); p1 = sosphasez(sos, 64); p2 = phasez(sos, 64); \
         s1 = sosstepz(sos, 20); s2 = stepz(sos, 20); m1 = sosfreqzmag(sos, 64); m2 = abs(freqz(sos, 64)); \
         k1 = freqzmag([1 1], 1, 8); k2 = abs(freqz([1 1], 1, 8)); \
         i1 = impz(1, [1 -0.5], 16); i2 = 0.5 .^ (0:15)';",
    );
    for (x, y) in [
        ("a1", "a2"),
        ("g1", "g2"),
        ("p1", "p2"),
        ("s1", "s2"),
        ("i1", "i2"),
    ] {
        assert_eq!(real(&e, x), real(&e, y), "{x} vs {y}");
    }
    for (x, y) in [("m1", "m2"), ("k1", "k2")] {
        for (u, v) in real(&e, x).iter().zip(real(&e, y)) {
            assert!((u - v).abs() < 1e-14, "{x} vs {y}");
        }
    }
    // sosfreqz stays plot-only.
    assert!(error("h = sosfreqz(butter_sos(2, 0.3), 8)").contains("does not return a value"));
}
