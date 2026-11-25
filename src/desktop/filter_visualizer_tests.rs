// Filter Visualization: every view against SciPy fixtures
// (tool/refs/filter_visualizer/reference.py), through the real app host.
mod filter_visualizer_tests {
    use super::*;
    use apps::filter_visualizer::{
        Curve, DEFAULT_POINTS, FilterVisualizer, Form, MAX_FILTERS, Part, View,
    };
    use apps::{App, AppKind, Host, Snapshot};
    use serde_json::Value;
    use toolkits::Tool;

    fn reference() -> Value {
        serde_json::from_str(include_str!(
            "../../tests/fixtures/apps/filter_visualizer/reference.json"
        ))
        .unwrap()
    }
    fn numbers(v: &Value) -> Vec<f64> {
        v.as_array()
            .unwrap()
            .iter()
            .map(|x| x.as_f64().unwrap_or(f64::NAN))
            .collect()
    }
    fn complex(v: &Value) -> Vec<[f64; 2]> {
        v.as_array()
            .unwrap()
            .iter()
            .map(|p| [p[0].as_f64().unwrap(), p[1].as_f64().unwrap()])
            .collect()
    }
    fn row(values: &[f64]) -> String {
        values
            .iter()
            .map(|v| apps::number(*v))
            .collect::<Vec<_>>()
            .join(" ")
    }
    /// The reference coefficients as workspace variables b, a, fir, sos, g.
    fn engine(reference: &Value) -> matrust::Engine {
        let sos: Vec<String> = reference["sos"]["sos"]
            .as_array()
            .unwrap()
            .iter()
            .map(|r| row(&numbers(r)))
            .collect();
        let mut engine = matrust::Engine::default();
        engine
            .execute(&format!(
                "b = [{}]; a = [{}]; fir = [{}]; sos = [{}]; g = {};",
                row(&numbers(&reference["iir"]["b"])),
                row(&numbers(&reference["iir"]["a"])),
                row(&numbers(&reference["fir"]["b"])),
                sos.join("; "),
                apps::number(reference["sos"]["g"].as_f64().unwrap()),
            ))
            .unwrap();
        engine
    }
    fn snapshot(engine: &matrust::Engine) -> Snapshot {
        let mut ws = Snapshot::default();
        ws.sync(&engine.workspace());
        ws
    }
    fn workspace(code: &str) -> Snapshot {
        let mut engine = matrust::Engine::default();
        engine.execute(code).unwrap();
        snapshot(&engine)
    }
    /// The three reference filters: iir (b, a), fir (b) and sos (sos, g).
    fn three(ws: &Snapshot) -> Host<FilterVisualizer> {
        let mut host = Host::new(FilterVisualizer::default());
        let f = host.app.add(Form::Tf, ws).unwrap();
        (f.name, f.b, f.a) = ("iir".into(), "b".into(), "a".into());
        let f = host.app.add(Form::Fir, ws).unwrap();
        (f.name, f.b) = ("fir".into(), "fir".into());
        let f = host.app.add(Form::Sos, ws).unwrap();
        (f.name, f.sos, f.gain) = ("sos".into(), "sos".into(), "g".into());
        host.app.points = "128".into();
        host
    }
    fn one(ws: &Snapshot, b: &str, a: &str) -> Host<FilterVisualizer> {
        let mut host = Host::new(FilterVisualizer::default());
        let f = host.app.add(Form::Tf, ws).unwrap();
        (f.name, f.b, f.a) = ("lp".into(), b.into(), a.into());
        host
    }
    /// The private run in a fresh Engine, which is kept for its variables.
    fn compute(
        host: &mut Host<FilterVisualizer>,
        ws: &Snapshot,
    ) -> Result<matrust::Engine, String> {
        host.app.prepare(ws);
        let mut engine = matrust::Engine::default();
        let outputs = apps::run(&mut engine, host.request(ws)?)?;
        host.app.receive(&outputs)?;
        Ok(engine)
    }
    fn curves(host: &Host<FilterVisualizer>) -> &[Curve] {
        &host.app.current.as_ref().expect("a computed view").curves
    }
    /// Real and imaginary parts of a real or complex variable.
    fn parts(engine: &matrust::Engine, name: &str) -> Vec<[f64; 2]> {
        match engine.array(name).unwrap_or_else(|| panic!("no {name}")) {
            matrust::Array::Numeric(m) => m.data().iter().map(|v| [*v, 0.]).collect(),
            matrust::Array::Complex(m) => m.data().iter().map(|z| [z.re, z.im]).collect(),
            a => panic!("{name}: {}", a.class()),
        }
    }
    fn bits(engine: &matrust::Engine, name: &str) -> Vec<[u64; 2]> {
        parts(engine, name)
            .iter()
            .map(|[re, im]| [re.to_bits(), im.to_bits()])
            .collect()
    }
    fn close(actual: f64, expected: f64, tolerance: f64, what: &str) {
        assert!(
            (actual - expected).abs() <= tolerance * expected.abs().max(1.),
            "{what}: {actual:.15e} != {expected:.15e} (tolerance {tolerance})"
        );
    }
    fn all_close(actual: &[f64], expected: &[f64], tolerance: f64, what: &str) {
        assert_eq!(actual.len(), expected.len(), "{what}: length");
        for (k, (a, e)) in actual.iter().zip(expected).enumerate() {
            if e.is_nan() {
                // SciPy has no value here (a zero of the response).
                continue;
            }
            close(*a, *e, tolerance, &format!("{what}[{k}]"));
        }
    }
    /// Every expected root has its own computed root within `tolerance`.
    fn same_roots(actual: &[[f64; 2]], expected: &[[f64; 2]], tolerance: f64, what: &str) {
        assert_eq!(actual.len(), expected.len(), "{what}: count");
        let mut left: Vec<[f64; 2]> = actual.to_vec();
        for e in expected {
            let (at, distance) = left
                .iter()
                .map(|a| (a[0] - e[0]).hypot(a[1] - e[1]))
                .enumerate()
                .min_by(|a, b| a.1.total_cmp(&b.1))
                .unwrap();
            assert!(distance <= tolerance, "{what}: {e:?} is {distance} away");
            left.swap_remove(at);
        }
    }
    const CASES: [&str; 3] = ["iir", "fir", "sos"];

    #[test]
    fn filter_visualizer_frequency_views_match_scipy() {
        let reference = reference();
        let ws = snapshot(&engine(&reference));
        let mut host = three(&ws);
        for (range, whole) in [("half", false), ("whole", true)] {
            host.app.whole = whole;
            // Magnitude, linear then dB, over normalised frequency.
            host.app.view = View::Magnitude;
            host.app.db = false;
            compute(&mut host, &ws).unwrap();
            let shown = host.app.current.clone().unwrap();
            assert!(!shown.hz && shown.curves.len() == 3);
            for (c, case) in shown.curves.iter().zip(CASES) {
                let r = &reference[case][range];
                let w: Vec<f64> = numbers(&r["w"])
                    .iter()
                    .map(|v| v / std::f64::consts::PI)
                    .collect();
                all_close(&c.x, &w, 1e-14, &format!("{case} {range} axis"));
                let h = complex(&r["h"]);
                let magnitude: Vec<f64> = h.iter().map(|[re, im]| re.hypot(*im)).collect();
                all_close(&c.y, &magnitude, 1e-11, &format!("{case} {range} |h|"));
                assert!(c.y2.is_empty());
            }
            host.app.db = true;
            compute(&mut host, &ws).unwrap();
            for (c, case) in curves(&host).iter().zip(CASES) {
                for (k, [re, im]) in complex(&reference[case][range]["h"]).iter().enumerate() {
                    let magnitude = re.hypot(*im);
                    // Deep in a stopband two correct evaluations differ in dB.
                    if magnitude > 1e-5 {
                        close(
                            c.y[k],
                            20. * magnitude.log10(),
                            1e-6,
                            &format!("{case} dB[{k}]"),
                        );
                    }
                }
            }
            // Group delay in samples: exact values, each with the bound a
            // double evaluation can keep (loose only beside a zero on the
            // unit circle); null where the numerator vanishes.
            host.app.view = View::GroupDelay;
            compute(&mut host, &ws).unwrap();
            for (c, case) in curves(&host).iter().zip(CASES) {
                let r = &reference[case][range];
                let expected = numbers(&r["group_delay"]);
                let tolerance = numbers(&r["group_delay_tolerance"]);
                assert_eq!(c.y.len(), expected.len());
                let mut tight = 0;
                for (k, (e, t)) in expected.iter().zip(&tolerance).enumerate() {
                    if e.is_nan() {
                        assert!(c.y[k].is_nan(), "{case} {range}: undefined at {k}");
                        continue;
                    }
                    assert!(
                        (c.y[k] - e).abs() <= *t,
                        "{case} {range} group delay[{k}]: {} != {e} (tolerance {t})",
                        c.y[k]
                    );
                    tight += usize::from(*t < 1e-7);
                }
                assert!(
                    tight >= 120,
                    "{case} {range}: {tight} well-conditioned points"
                );
            }
        }
        // Phase and phase delay: where the response has no zero inside the
        // range the two unwrappings must agree point for point (the
        // Butterworth zero sits at pi, outside the half range).
        host.app.whole = false;
        host.app.view = View::Phase;
        (host.app.unwrapped, host.app.degrees) = (true, false);
        compute(&mut host, &ws).unwrap();
        for (c, case) in curves(&host).iter().zip(CASES) {
            let expected = numbers(&reference[case]["half"]["phase"]);
            all_close(&c.y, &expected, 1e-8, &format!("{case} unwrapped phase"));
        }
        host.app.degrees = true;
        compute(&mut host, &ws).unwrap();
        for (c, case) in curves(&host).iter().zip(CASES) {
            let expected: Vec<f64> = numbers(&reference[case]["half"]["phase"])
                .iter()
                .map(|v| v.to_degrees())
                .collect();
            all_close(&c.y, &expected, 1e-8, &format!("{case} phase in degrees"));
        }
        (host.app.unwrapped, host.app.degrees) = (false, false);
        compute(&mut host, &ws).unwrap();
        for (c, case) in curves(&host).iter().zip(CASES) {
            for (k, [re, im]) in complex(&reference[case]["half"]["h"]).iter().enumerate() {
                // -pi and pi are the same wrapped phase.
                let difference = c.y[k] - im.atan2(*re);
                assert!(
                    difference.sin().abs() < 1e-7 && difference.cos() > 0.,
                    "{case} wrapped phase[{k}]"
                );
                assert!(c.y[k].abs() <= std::f64::consts::PI + 1e-12);
            }
        }
        host.app.view = View::PhaseDelay;
        compute(&mut host, &ws).unwrap();
        for (c, case) in curves(&host).iter().zip(CASES) {
            let expected = numbers(&reference[case]["half"]["phase_delay"]);
            assert!(
                expected[0].is_nan() && !c.y[0].is_finite(),
                "{case}: undefined at 0"
            );
            all_close(&c.y, &expected, 1e-8, &format!("{case} phase delay"));
        }
        // Magnitude + phase holds both plots.
        host.app.view = View::MagnitudePhase;
        (host.app.unwrapped, host.app.degrees, host.app.db) = (true, false, false);
        compute(&mut host, &ws).unwrap();
        for (c, case) in curves(&host).iter().zip(CASES) {
            let r = &reference[case]["half"];
            let magnitude: Vec<f64> = complex(&r["h"])
                .iter()
                .map(|[re, im]| re.hypot(*im))
                .collect();
            all_close(&c.y, &magnitude, 1e-11, &format!("{case} pair magnitude"));
            all_close(
                &c.y2,
                &numbers(&r["phase"]),
                1e-8,
                &format!("{case} pair phase"),
            );
        }
        // With a sample rate on every filter the axis is in Hz.
        host.app.view = View::Magnitude;
        for f in &mut host.app.filters {
            f.fs = "1000".into();
        }
        compute(&mut host, &ws).unwrap();
        assert!(host.app.current.as_ref().unwrap().hz);
        for (c, case) in curves(&host).iter().zip(CASES) {
            let r = &reference[case]["hz"];
            all_close(&c.x, &numbers(&r["f"]), 1e-13, &format!("{case} Hz axis"));
            let magnitude: Vec<f64> = complex(&r["h"])
                .iter()
                .map(|[re, im]| re.hypot(*im))
                .collect();
            all_close(&c.y, &magnitude, 1e-11, &format!("{case} Hz magnitude"));
        }
        // One filter without a rate: every filter is drawn normalised.
        host.app.filters[1].fs.clear();
        compute(&mut host, &ws).unwrap();
        let shown = host.app.current.as_ref().unwrap();
        assert!(!shown.hz);
        close(
            *shown.curves[0].x.last().unwrap(),
            127. / 128.,
            1e-14,
            "normalised",
        );
        assert!(
            host.app
                .script(&ws)
                .unwrap()
                .contains("sample rate not used")
        );
    }

    #[test]
    fn filter_visualizer_default_grid_is_8192_points() {
        let reference = reference();
        let ws = snapshot(&engine(&reference));
        let mut host = one(&ws, "b", "a");
        assert_eq!(host.app.points, DEFAULT_POINTS.to_string());
        host.app.db = false;
        let run = compute(&mut host, &ws).unwrap();
        let c = &curves(&host)[0];
        assert_eq!(c.x.len(), 8192);
        let default = &reference["default"];
        let h = complex(&default["h"]);
        let w = numbers(&default["w"]);
        let exported = parts(&run, "h_lp");
        for (k, index) in default["indices"].as_array().unwrap().iter().enumerate() {
            let index = index.as_u64().unwrap() as usize;
            close(
                c.x[index],
                w[k] / std::f64::consts::PI,
                1e-14,
                "default axis",
            );
            close(
                c.y[index],
                h[k][0].hypot(h[k][1]),
                1e-11,
                "default magnitude",
            );
            close(exported[index][0], h[k][0], 1e-11, "default real part");
            close(exported[index][1], h[k][1], 1e-11, "default imaginary part");
            close(parts(&run, "w_lp")[index][0], w[k], 1e-14, "default w");
        }
    }

    #[test]
    fn filter_visualizer_time_and_pole_zero_views_match_scipy() {
        let reference = reference();
        let ws = snapshot(&engine(&reference));
        let mut host = three(&ws);
        host.app.length = reference["time"].as_u64().unwrap().to_string();
        for (view, key) in [(View::Impulse, "impulse"), (View::Step, "step")] {
            host.app.view = view;
            compute(&mut host, &ws).unwrap();
            for (c, case) in curves(&host).iter().zip(CASES) {
                all_close(
                    &c.y,
                    &numbers(&reference[case][key]),
                    1e-12,
                    &format!("{case} {key}"),
                );
                let samples: Vec<f64> = (0..40).map(f64::from).collect();
                assert_eq!(c.x, samples, "{case} {key} samples");
            }
        }
        // Automatic length: an FIR response is its taps. Seconds with a rate.
        host.app.length.clear();
        host.app.view = View::Impulse;
        for f in &mut host.app.filters {
            f.fs = "1000".into();
        }
        compute(&mut host, &ws).unwrap();
        let fir = &curves(&host)[1];
        assert_eq!(fir.y.len(), 31);
        all_close(
            &fir.y,
            &numbers(&reference["fir"]["impulse"])[..31],
            1e-12,
            "fir taps",
        );
        all_close(
            &fir.y,
            &numbers(&reference["fir"]["b"]),
            1e-15,
            "fir taps are b",
        );
        close(fir.x[30], 0.03, 1e-15, "seconds");
        for f in &mut host.app.filters {
            f.fs.clear();
        }

        host.app.view = View::PoleZero;
        let run = compute(&mut host, &ws).unwrap();
        // A repeated root is ill-conditioned in b, a (both SciPy and the
        // engine scatter the four Butterworth zeros about 3e-4 around -1).
        let tolerances = [
            ("iir", 2e-3, 1e-9),
            ("fir", 1e-6, 1e-12),
            ("sos", 1e-10, 1e-10),
        ];
        for (c, (case, zero_tolerance, pole_tolerance)) in curves(&host).iter().zip(tolerances) {
            let r = &reference[case];
            same_roots(
                &c.zeros,
                &complex(&r["zeros"]),
                zero_tolerance,
                &format!("{case} zeros"),
            );
            same_roots(
                &c.poles,
                &complex(&r["poles"]),
                pole_tolerance,
                &format!("{case} poles"),
            );
            close(
                parts(&run, &format!("k_{case}"))[0][0],
                r["gain"].as_f64().unwrap(),
                1e-12,
                &format!("{case} gain"),
            );
            assert_eq!(r["stable"], Value::Bool(true));
            // Multiplicities: only places holding more than one root are marked.
            for (marks, key) in [(&c.zero_marks, "zero_marks"), (&c.pole_marks, "pole_marks")] {
                let expected: Vec<([f64; 2], usize)> = r[key]
                    .as_array()
                    .unwrap()
                    .iter()
                    .map(|m| {
                        let at = [m[0][0].as_f64().unwrap(), m[0][1].as_f64().unwrap()];
                        (at, m[1].as_u64().unwrap() as usize)
                    })
                    .filter(|(_, count)| *count > 1)
                    .collect();
                assert_eq!(marks.len(), expected.len(), "{case} {key}");
                for ((at, count), (expected_at, expected_count)) in marks.iter().zip(&expected) {
                    assert_eq!(count, expected_count, "{case} {key}");
                    assert!(
                        (at[0] - expected_at[0]).hypot(at[1] - expected_at[1]) < 1e-6,
                        "{case} {key} at {at:?}"
                    );
                }
            }
        }
        // Four zeros at -1, thirty poles at the origin, nothing repeated in sos.
        let shown = curves(&host);
        assert_eq!(shown[0].zero_marks.len(), 1);
        assert_eq!(shown[0].zero_marks[0].1, 4);
        assert_eq!(shown[1].pole_marks, [([0., 0.], 30)]);
        assert!(shown[2].zero_marks.is_empty() && shown[2].pole_marks.is_empty());
        let script = host.app.script(&ws).unwrap();
        assert!(script.contains("fv_marks(z_iir); fv_marks(p_iir);"));
        assert!(script.contains("'DisplayName', 'Unit circle'") && script.contains("axis equal"));
    }

    #[test]
    fn filter_visualizer_flags_order_and_form() {
        let reference = reference();
        let ws = snapshot(&engine(&reference));
        let mut host = three(&ws);
        compute(&mut host, &ws).unwrap();
        for (c, case) in curves(&host).iter().zip(CASES) {
            let r = &reference[case];
            let flag = |key: &str| f64::from(u8::from(r[key].as_bool().unwrap()));
            assert_eq!(c.info.order, r["order"].as_f64().unwrap(), "{case} order");
            assert_eq!(c.info.stable, flag("stable"), "{case} stable");
            assert_eq!(
                c.info.minimum_phase,
                flag("minimum_phase"),
                "{case} minimum phase"
            );
            assert_eq!(
                c.info.linear_phase,
                flag("linear_phase"),
                "{case} linear phase"
            );
            assert_eq!(c.info.fir, r["fir"].as_bool().unwrap(), "{case} fir");
            assert_eq!(
                c.info.sections as u64,
                r["sections"].as_u64().unwrap(),
                "{case}"
            );
        }
        assert_eq!(curves(&host)[0].info.form(), "IIR, direct form");
        assert_eq!(curves(&host)[1].info.form(), "FIR, 31 taps");
        assert_eq!(curves(&host)[2].info.form(), "IIR, 6 sections");
        // Known properties are not computed again for another view, only
        // when a filter's coefficients change.
        let before: Vec<_> = curves(&host).iter().map(|c| c.info).collect();
        host.app.view = View::GroupDelay;
        assert!(!host.request(&ws).ok().unwrap().code.contains("roots("));
        compute(&mut host, &ws).unwrap();
        let after: Vec<_> = curves(&host).iter().map(|c| c.info).collect();
        assert_eq!(after, before);
        let mut changed = engine(&reference);
        changed.execute("b = [1 2]; a = 1;").unwrap();
        let changed = snapshot(&changed);
        assert_eq!(
            host.request(&changed)
                .ok()
                .unwrap()
                .code
                .matches("roots(")
                .count(),
            2
        );
        compute(&mut host, &changed).unwrap();
        let info = curves(&host)[0].info;
        assert_eq!((info.order, info.minimum_phase, info.fir), (1., 0., true));
        assert_eq!(curves(&host)[1].info, before[1]);
        compute(&mut host, &ws).unwrap();
        host.app.view = View::Magnitude;
        host.app.selected = 1;
        assert_eq!(
            host.app.results(),
            [
                ("Filter", "fir".to_owned()),
                ("Order", "30".to_owned()),
                ("Stable", "Yes".to_owned()),
                ("Minimum phase", "No".to_owned()),
                ("Linear phase", "Yes".to_owned()),
                ("Form", "FIR, 31 taps".to_owned()),
            ]
        );
        // Textbook cases: zeros of 1 + 2 z^-1 and 1 + 0.5 z^-1 are -2 and
        // -0.5; 1 - 1.5 z^-1 has its pole at 1.5; [1 0 -1] is antisymmetric;
        // (z + 2)^2 has a repeated zero outside the unit circle; one section
        // is taken as its b, a.
        let ws = workspace(
            "one = 1; out = [1 2]; in = [1 0.5]; unstable = [1 -1.5]; anti = [1 0 -1]; \
             twice = [1 4 4]; delayed = [0 0 1 0.5 0]; row = [1 2 1 1 -0.5 0.25]; \
             long = [1 zeros(1, 1198) 0.5];",
        );
        for (b, a, order, stable, minimum, linear, fir) in [
            ("out", "one", 1., 1., 0., 0., true),
            ("in", "one", 1., 1., 1., 0., true),
            ("one", "unstable", 1., 0., 1., 0., false),
            ("anti", "one", 2., 1., 1., 1., true),
            ("twice", "one", 2., 1., 0., 0., true),
            ("delayed", "one", 3., 1., 1., 0., true),
            ("in", "in", 1., 1., 1., 0., false),
        ] {
            let mut host = one(&ws, b, a);
            compute(&mut host, &ws).unwrap_or_else(|e| panic!("{b}, {a}: {e}"));
            let info = curves(&host)[0].info;
            assert_eq!(
                (
                    info.order,
                    info.stable,
                    info.minimum_phase,
                    info.linear_phase,
                    info.fir
                ),
                (order, stable, minimum, linear, fir),
                "{b}, {a}"
            );
        }
        let mut host = Host::new(FilterVisualizer::default());
        let f = host.app.add(Form::Sos, &ws).unwrap();
        f.sos = "row".into();
        compute(&mut host, &ws).unwrap();
        let info = curves(&host)[0].info;
        assert_eq!((info.order, info.stable, info.minimum_phase), (2., 1., 1.));
        assert_eq!(info.form(), "IIR, 1 section");
        assert!(
            host.app
                .script(&ws)
                .unwrap()
                .contains("freqz(row(1, 1:3), row(1, 4:6), 8192)")
        );
        // 1200 coefficients are beyond the engine's root finder: the view
        // still draws and the two root flags say so.
        let mut host = Host::new(FilterVisualizer::default());
        let f = host.app.add(Form::Fir, &ws).unwrap();
        f.b = "long".into();
        host.app.points = "64".into();
        compute(&mut host, &ws).unwrap();
        let info = curves(&host)[0].info;
        assert!(info.stable.is_nan() && info.minimum_phase.is_nan());
        assert_eq!(
            (info.order, info.linear_phase, info.taps),
            (1199., 0., 1200)
        );
        assert!(
            host.app
                .results()
                .contains(&("Minimum phase", "n/a".to_owned()))
        );
    }

    #[test]
    fn filter_visualizer_generated_script_export_and_save_agree() {
        let reference = reference();
        let source = engine(&reference);
        let ws = snapshot(&source);
        let mut host = three(&ws);
        host.app.filters[0].fs = "1000".into();
        host.app.filters[1].fs = "1000".into();
        host.app.filters[2].fs = "8000".into();
        for (view, calls, plots) in [
            (
                View::Magnitude,
                vec!["freqz(b, a, 128, 1000)", "freqz(sos, 128, 8000)"],
                "plot(",
            ),
            (
                View::Phase,
                vec!["phasez(fir, 1, 128, 1000)", "angle(g)"],
                "plot(",
            ),
            (
                View::MagnitudePhase,
                vec!["freqz(b, a, 128, 1000)", "phasez(sos, 128, 8000)"],
                "subplot(2, 1, 2)",
            ),
            (
                View::GroupDelay,
                vec!["grpdelay(b, a, 128, 1000)", "grpdelay(sos, 128, 8000)"],
                "plot(",
            ),
            (
                View::PhaseDelay,
                vec!["phasez(b, a, 128, 1000)", "./ (2*pi*w_sos/8000)"],
                "plot(",
            ),
            (
                View::Impulse,
                vec!["impz(b, a, [], 1000)", "impz(sos, [], 8000)"],
                "stem(",
            ),
            (
                View::Step,
                vec!["stepz(fir, 1, [], 1000)", "s_sos = g * s_sos"],
                "stem(",
            ),
            (
                View::PoleZero,
                vec!["tf2zp(", "sos2zp(sos)", "k_sos = g * k_sos"],
                "plot(real(z_fir)",
            ),
        ] {
            host.app.view = view;
            let private = compute(&mut host, &ws).unwrap_or_else(|e| panic!("{view:?}: {e}"));
            let script = host.app.script(&ws).unwrap();
            for call in calls {
                assert!(script.contains(call), "{view:?}: no {call} in\n{script}");
            }
            assert!(
                script.contains(plots) && script.contains("figure;"),
                "{view:?}"
            );
            assert!(script.contains("legend('show')") && script.contains("'DisplayName', 'fir"));
            let names = host.app.exported(&ws);
            let bases: Vec<&str> = match view {
                View::Magnitude => vec!["h", "w"],
                View::Phase => vec!["phi", "w"],
                View::MagnitudePhase => vec!["h", "phi", "w"],
                View::GroupDelay => vec!["gd", "w"],
                View::PhaseDelay => vec!["pd", "w"],
                View::Impulse => vec!["h", "t"],
                View::Step => vec!["s", "t"],
                View::PoleZero => vec!["z", "p", "k"],
            };
            let expected: Vec<String> = CASES
                .iter()
                .flat_map(|f| bases.iter().map(move |base| format!("{base}_{f}")))
                .collect();
            assert_eq!(names, expected, "{view:?}");
            // Generate script: the header, then the same text.
            let saved = match host.generate(&ws, 42).unwrap() {
                toolkits::Action::SaveAppScript(AppKind::FilterVisualizer, text) => text,
                _ => panic!("wrong save action"),
            };
            assert!(saved.starts_with("% Filter Visualization (MatRust app)"));
            assert!(saved.ends_with(&script));
            let mut fresh = engine(&reference);
            fresh
                .execute(&saved)
                .unwrap_or_else(|e| panic!("{view:?}: {e}"));
            // Export: the same script in the workspace, all or nothing.
            let export = match host.export(&ws).unwrap() {
                toolkits::Action::ExportScript(s) => s,
                _ => panic!("wrong export"),
            };
            let mut exported = engine(&reference);
            exported
                .execute(&export)
                .unwrap_or_else(|e| panic!("{view:?}: {e}"));
            for name in &names {
                let value = bits(&private, name);
                assert!(
                    !value.is_empty() || view == View::PoleZero,
                    "{view:?} {name}"
                );
                assert_eq!(bits(&fresh, name), value, "{view:?} {name} saved");
                assert_eq!(bits(&exported, name), value, "{view:?} {name} exported");
            }
            // Nothing but the exported names is left behind.
            let mut left: Vec<String> = exported.workspace().into_keys().collect();
            left.retain(|n| !source.contains_array(n));
            left.sort();
            let mut sorted = names.clone();
            sorted.sort();
            assert_eq!(left, sorted, "{view:?}");
        }
        // The drawn arrays are the exported ones: magnitude in dB of h, over w.
        host.app.view = View::Magnitude;
        host.app.db = true;
        let private = compute(&mut host, &ws).unwrap();
        for c in curves(&host) {
            let h = parts(&private, &format!("h_{}", c.name));
            let w = parts(&private, &format!("w_{}", c.name));
            for k in 0..128 {
                assert_eq!(c.x[k], w[k][0], "Hz axis is w");
                let db = 20. * h[k][0].hypot(h[k][1]).log10();
                assert!(
                    (c.y[k] - db).abs() <= 1e-9 * db.abs().max(1.),
                    "{} dB",
                    c.name
                );
            }
        }
        close(
            *curves(&host)[2].x.last().unwrap(),
            4000. * 127. / 128.,
            1e-14,
            "sos Hz",
        );
        // A log axis is the same data drawn with semilogx.
        host.app.log = true;
        assert!(
            host.app
                .script(&ws)
                .unwrap()
                .contains("semilogx(w_iir, 20*log10(abs(h_iir))")
        );
        compute(&mut host, &ws).unwrap();
        assert!(host.app.current.as_ref().unwrap().log);
    }

    #[test]
    fn filter_visualizer_export_names_are_checked() {
        let reference = reference();
        let mut source = engine(&reference);
        source
            .execute("h_lp = 5; keep = 7; plot = [1 2 3];")
            .unwrap();
        let ws = snapshot(&source);
        let mut host = one(&ws, "b", "a");
        host.app.points = "16".into();
        compute(&mut host, &ws).unwrap();
        assert_eq!(host.app.exported(&ws), ["h_lp", "w_lp"]);
        let response = Part::ALL.iter().position(|p| *p == Part::Response).unwrap();
        for bad in [
            "",
            "1bad",
            "for",
            "h);bad=1;%",
            "h\nerror('x')",
            "h h",
            "a'",
            "n".repeat(61).as_str(),
        ] {
            host.app.names[response] = bad.into();
            assert!(host.app.script(&ws).is_err(), "{bad}");
            assert!(
                host.request(&ws).is_err() && host.app.exported(&ws).is_empty(),
                "{bad}"
            );
        }
        // Reserved by the export transaction, once the suffix is on.
        for (base, filter) in [
            ("mrexp", "old"),
            ("mrexp", "had"),
            ("mrexp", "complete"),
            ("matrust_export", "error"),
        ] {
            host.app.names[response] = base.into();
            host.app.filters[0].name = filter.into();
            assert!(
                host.app.script(&ws).unwrap_err().contains("reserved"),
                "{base}_{filter}"
            );
        }
        host.app.filters[0].name = "lp".into();
        host.app.names[response] = "w".into();
        assert!(
            host.app
                .script(&ws)
                .unwrap_err()
                .contains("duplicate export name w_lp")
        );
        host.app.names[response] = "h".into();
        // A filter may not be exported over its own coefficients.
        let ws2 = workspace("b_lp = [1 2 1]; a = 1;");
        let mut own = one(&ws2, "b_lp", "a");
        own.app.names[response] = "b".into();
        assert!(own.app.script(&ws2).unwrap_err().contains("own input"));
        own.app.names[response] = "h".into();
        compute(&mut own, &ws2).unwrap();
        // Filter names are identifiers too, and unique among shown filters.
        for bad in [
            "",
            "my filter",
            "a'b",
            "1st",
            "x);y=1;%",
            "n".repeat(41).as_str(),
        ] {
            host.app.filters[0].name = bad.into();
            assert!(host.app.script(&ws).is_err(), "{bad}");
        }
        host.app.filters[0].name = "lp".into();
        let f = host.app.add(Form::Fir, &ws).unwrap();
        (f.name, f.b) = ("lp".into(), "fir".into());
        assert!(
            host.app
                .script(&ws)
                .unwrap_err()
                .contains("two shown filters")
        );
        host.app.filters[1].shown = false;
        host.app.script(&ws).unwrap();
        // A variable named like a function the script calls is refused.
        host.app.filters[0].b = "plot".into();
        assert!(
            host.app
                .script(&ws)
                .unwrap_err()
                .contains("hides a function")
        );
        host.app.filters[0].b = "b".into();

        // Export replaces h_lp and leaves the rest; a failure restores it.
        let export = match host.export(&ws).unwrap() {
            toolkits::Action::ExportScript(s) => s,
            _ => panic!("wrong export"),
        };
        let mut engine = engine(&reference);
        engine.execute("h_lp = 5; keep = 7;").unwrap();
        engine.execute(&export).unwrap();
        assert_eq!(parts(&engine, "h_lp").len(), 16);
        assert_eq!(parts(&engine, "keep"), [[7., 0.]]);
        engine.execute("h_lp = 5; w_lp = 6; a = [0 1];").unwrap();
        let error = engine.execute(&export).unwrap_err();
        assert!(!error.is_empty());
        assert_eq!(parts(&engine, "h_lp"), [[5., 0.]]);
        assert_eq!(parts(&engine, "w_lp"), [[6., 0.]]);
        for name in ["mrexp_had", "mrexp_old"] {
            assert!(!engine.contains_array(name), "left {name}");
        }
    }

    #[test]
    fn filter_visualizer_bad_inputs_error_cleanly_and_keep_the_last_result() {
        let ws = workspace(
            "b = [1 2 1]/4; a = [1 -0.5]; e = []; z = zeros(1, 0); m = [1 2; 3 4]; n = [1 NaN]; \
             c = [1 2i]; s = single([1 2]); a0 = [0 1]; sos = [1 2 1 1 -0.5 0.2; 1 0 0 1 0.1 0]; \
             bad5 = ones(2, 5); zero_a0 = [1 2 1 0 1 1; 1 0 0 1 0 0]; g = 2; gv = [1 2]; blow = [1 -10];",
        );
        let mut host = one(&ws, "b", "a");
        host.app.points = "32".into();
        compute(&mut host, &ws).unwrap();
        let before = curves(&host)[0].y.clone();
        // A zero-length b is refused before any script exists.
        for (b, wanted) in [
            ("e", "nonempty"),
            ("z", "nonempty"),
            ("m", "vector"),
            ("n", "finite"),
            ("c", "real"),
            ("s", "real vector"),
            ("missing", "not a workspace variable"),
            ("", "choose Numerator b"),
            ("b);x=1;%", "Numerator b"),
        ] {
            host.app.filters[0].b = b.into();
            let error = host
                .request(&ws)
                .err()
                .unwrap_or_else(|| panic!("{b} accepted"));
            assert!(error.contains(wanted), "{b}: {error}");
            assert!(error.starts_with("Filter Visualization: lp: "), "{error}");
            assert_eq!(curves(&host)[0].y, before, "last good result kept");
        }
        host.app.filters[0].b = "b".into();
        host.app.filters[0].a = "a0".into();
        assert!(host.request(&ws).err().unwrap().contains("nonzero"));
        host.app.filters[0].a = "a".into();
        // And the engine refuses it too when a saved script meets an empty b.
        let script = host.app.script(&ws).unwrap();
        let mut engine = matrust::Engine::default();
        engine.execute("b = []; a = [1 -0.5];").unwrap();
        let error = engine.execute(&script).unwrap_err();
        assert!(
            error.contains("freqz") && error.contains("nonempty"),
            "{error}"
        );
        engine.execute("b = zeros(1, 0);").unwrap();
        assert!(engine.execute(&script).is_err());

        for fs in ["0", "-1", "abc", "Inf", "1000); x=1; %", "1e13", "2*500"] {
            host.app.filters[0].fs = fs.into();
            assert!(host.request(&ws).is_err(), "fs {fs}");
        }
        host.app.filters[0].fs.clear();
        for points in ["", "1", "65537", "1.5", "2^10", "NaN", "128); x=1; %"] {
            host.app.points = points.into();
            assert!(host.request(&ws).is_err(), "points {points}");
        }
        host.app.points = "32".into();
        host.app.view = View::Impulse;
        for length in ["0", "65537", "2.5", "ten", "5\nerror('x')"] {
            host.app.length = length.into();
            assert!(host.request(&ws).is_err(), "length {length}");
        }
        host.app.length.clear();
        host.app.view = View::Magnitude;

        let f = &mut host.app.filters[0];
        f.form = Form::Sos;
        for (sos, gain) in [
            ("bad5", ""),
            ("b", ""),
            ("zero_a0", ""),
            ("missing", ""),
            ("sos", "gv"),
            ("sos", "missing"),
            ("sos", "Inf"),
            ("sos", "2*g"),
            ("sos", "g); x=1; %"),
        ] {
            let f = &mut host.app.filters[0];
            (f.sos, f.gain) = (sos.into(), gain.into());
            assert!(host.request(&ws).is_err(), "{sos}, {gain}");
        }
        // Gain: blank and 1 are no gain, a number or a workspace scalar scale.
        for (gain, factor, text) in [
            ("", 1., None),
            ("1", 1., None),
            ("-2.5", -2.5, Some("-2.5 * h_lp")),
            ("g", 2., Some("g * h_lp")),
        ] {
            let f = &mut host.app.filters[0];
            (f.sos, f.gain) = ("sos".into(), gain.into());
            host.app.db = false;
            let run = compute(&mut host, &ws).unwrap();
            let script = host.app.script(&ws).unwrap();
            assert_eq!(script.contains(" * h_lp"), text.is_some(), "{gain}");
            if let Some(text) = text {
                assert!(script.contains(text));
            }
            // DC gain of the sections: (4 / 0.7) * (1 / 1.1).
            close(
                parts(&run, "h_lp")[0][0],
                factor * 4. / 0.7 / 1.1,
                1e-13,
                "sections at 0",
            );
            assert_eq!(host.app.inputs().contains(&"g".to_owned()), gain == "g");
        }
        // A negative gain turns the phase by pi.
        host.app.filters[0].gain = "-2.5".into();
        host.app.view = View::Phase;
        (host.app.unwrapped, host.app.degrees) = (true, false);
        compute(&mut host, &ws).unwrap();
        close(
            curves(&host)[0].y[0],
            std::f64::consts::PI,
            1e-15,
            "phase of a negative gain",
        );

        // A failure inside the run reaches the status line; the plot stays.
        let mut host = one(&ws, "b", "blow");
        host.app.points = "32".into();
        compute(&mut host, &ws).unwrap();
        let before = curves(&host)[0].y.clone();
        host.app.view = View::Impulse;
        host.app.length = "400".into();
        let ctx = egui::Context::default();
        let start = Instant::now();
        loop {
            host.refresh(&ctx, &ws);
            if !host.running() || start.elapsed() > Duration::from_secs(60) {
                break;
            }
            std::thread::sleep(Duration::from_millis(5));
        }
        host.refresh(&ctx, &ws);
        let (text, status) = host.status();
        assert_eq!(status, toolkits::AppStatus::Error, "{text}");
        assert!(text.contains("nonfinite"), "{text}");
        assert_eq!(host.app.current.as_ref().unwrap().view, View::Magnitude);
        assert_eq!(curves(&host)[0].y, before);
    }

    #[test]
    fn filter_visualizer_digital_filter_is_read_in_the_workspace() {
        let reference = reference();
        let mut engine = matrust::Engine::default();
        engine
            .execute(
                "d = designfilt('lowpassiir', 'FilterOrder', 4, 'HalfPowerFrequency', 0.3); \
                 f = designfilt('lowpassfir', 'FilterOrder', 30, 'CutoffFrequency', 0.3); \
                 plain.Coefficients = [1 2 1]; d_sos = 99;",
            )
            .unwrap();
        let mut ws = snapshot(&engine);
        let mut host = Host::new(FilterVisualizer::default());
        let filter = host.app.add(Form::Object, &ws).unwrap();
        filter.name = "lp".into();
        // No private Engine can read the object, so the app says what to do.
        assert!(
            host.request(&ws)
                .err()
                .unwrap()
                .contains("choose digitalFilter")
        );
        host.app.filters[0].object = "d".into();
        assert!(
            host.request(&ws)
                .err()
                .unwrap()
                .contains("Read coefficients")
        );
        // Reading runs in the workspace and leaves one plain copy; an
        // existing d_sos is not touched.
        let script = host.app.filters[0].read_object(&ws).unwrap();
        apps::execute_export(&mut engine, &script).unwrap();
        ws = snapshot(&engine);
        assert_eq!(parts(&engine, "d_sos"), [[99., 0.]]);
        assert!(engine.contains_array("d_sos1") && !engine.contains_array("d_b"));
        host.app.points = "128".into();
        host.app.db = false;
        compute(&mut host, &ws).unwrap();
        let f = &host.app.filters[0];
        assert_eq!(
            (f.form, f.sos.as_str(), f.gain.as_str()),
            (Form::Sos, "d_sos1", "")
        );
        // The design is butter(4, 0.3): SciPy's response of that filter.
        let c = &curves(&host)[0];
        let expected: Vec<f64> = complex(&reference["iir"]["half"]["h"])
            .iter()
            .map(|[re, im]| re.hypot(*im))
            .collect();
        all_close(&c.y, &expected, 1e-10, "digitalFilter magnitude");
        assert_eq!((c.info.order, c.info.sections, c.info.fir), (4., 2, false));

        let filter = host.app.add(Form::Object, &ws).unwrap();
        (filter.name, filter.object) = ("fir".into(), "f".into());
        let script = filter.read_object(&ws).unwrap();
        apps::execute_export(&mut engine, &script).unwrap();
        ws = snapshot(&engine);
        compute(&mut host, &ws).unwrap();
        let f = &host.app.filters[1];
        assert_eq!((f.form, f.b.as_str()), (Form::Fir, "f_b"));
        let info = curves(&host)[1].info;
        assert_eq!(
            (info.order, info.linear_phase, info.fir, info.taps),
            (30., 1., true, 31)
        );

        // Anything else is refused in the workspace, which stays as it was.
        let filter = host.app.add(Form::Object, &ws).unwrap();
        filter.object = "plain".into();
        let script = filter.read_object(&ws).unwrap();
        let before = engine.workspace().len();
        let error = apps::execute_export(&mut engine, &script).unwrap_err();
        assert!(error.contains("not a digitalFilter"), "{error}");
        assert_eq!(engine.workspace().len(), before);
        host.app.prepare(&snapshot(&engine));
        assert_eq!(host.app.filters[2].form, Form::Object);
        filter_missing(&mut host, &ws);
    }
    fn filter_missing(host: &mut Host<FilterVisualizer>, ws: &Snapshot) {
        host.app.filters[2].object = "gone".into();
        assert!(host.app.filters[2].read_object(ws).is_err());
        host.app.selected = 2;
        assert!(host.app.read_selected_object(ws).is_err());
    }

    #[test]
    fn filter_visualizer_list_stop_hover_and_tool_tab_headless_0_1_5() {
        let reference = reference();
        let source = engine(&reference);
        let ws = snapshot(&source);
        let mut host = three(&ws);
        assert_eq!(host.app.selected, 2);
        // Reorder and remove.
        host.app.reorder(-1);
        let order: Vec<&str> = host.app.filters.iter().map(|f| f.name.as_str()).collect();
        assert_eq!(
            (order.as_slice(), host.app.selected),
            (["iir", "sos", "fir"].as_slice(), 1)
        );
        host.app.reorder(1);
        host.app.reorder(1);
        host.app.selected = 0;
        host.app.reorder(-1);
        let order: Vec<&str> = host.app.filters.iter().map(|f| f.name.as_str()).collect();
        assert_eq!(order, ["iir", "fir", "sos"]);
        // A hidden filter is not computed, drawn or exported.
        host.app.filters[1].shown = false;
        compute(&mut host, &ws).unwrap();
        let names: Vec<&str> = curves(&host).iter().map(|c| c.name.as_str()).collect();
        assert_eq!(names, ["iir", "sos"]);
        assert_eq!(host.app.exported(&ws), ["h_iir", "w_iir", "h_sos", "w_sos"]);
        assert_eq!(host.app.inputs(), ["a", "b", "g", "sos"]);
        assert!(!host.app.script(&ws).unwrap().contains("fir"));
        host.app.selected = 1;
        assert_eq!(host.app.results()[1].0, "Properties");
        host.app.remove();
        assert_eq!(host.app.filters.len(), 2);
        for f in &mut host.app.filters {
            f.shown = false;
        }
        assert!(host.request(&ws).err().unwrap().contains("add a filter"));
        for f in &mut host.app.filters {
            f.shown = true;
        }
        while host.app.filters.len() < MAX_FILTERS {
            let f = host.app.add(Form::Fir, &ws).unwrap();
            f.b = "fir".into();
        }
        assert!(host.app.add(Form::Fir, &ws).is_err());
        let names: std::collections::BTreeSet<&str> =
            host.app.filters.iter().map(|f| f.name.as_str()).collect();
        assert_eq!(names.len(), MAX_FILTERS);
        compute(&mut host, &ws).unwrap();
        assert_eq!(curves(&host).len(), MAX_FILTERS);

        // Stop holds the current script until an input changes or Run.
        let mut host = three(&ws);
        compute(&mut host, &ws).unwrap();
        host.app.stop(&ws);
        assert!(host.app.is_stopped(&ws));
        assert!(host.request(&ws).err().unwrap().contains("stopped"));
        assert!(
            host.app.script(&ws).is_ok(),
            "the script itself is unchanged"
        );
        host.app.run();
        host.request(&ws).unwrap();
        host.app.stop(&ws);
        host.app.points = "64".into();
        host.app.prepare(&ws);
        assert!(!host.app.is_stopped(&ws));
        host.request(&ws).unwrap();
        host.app.points = "128".into();

        // Hover readout: the frequency and every filter's value there.
        host.app.db = true;
        compute(&mut host, &ws).unwrap();
        let shown = host.app.current.clone().unwrap();
        let at = 40;
        let w = numbers(&reference["iir"]["half"]["w"])[at] / std::f64::consts::PI;
        let text = shown.readout(w + 1e-4).unwrap();
        assert!(
            text.starts_with(&format!("f = {} (Nyquist = 1)", apps::short(w + 1e-4))),
            "{text}"
        );
        for case in CASES {
            let [re, im] = complex(&reference[case]["half"]["h"])[at];
            let db = apps::short(20. * re.hypot(im).log10());
            assert!(
                text.contains(&format!("{case}: {db} dB")),
                "{case} {db} in {text}"
            );
        }
        assert!(shown.readout(f64::NAN).is_none());
        host.app.view = View::MagnitudePhase;
        compute(&mut host, &ws).unwrap();
        let text = host.app.current.as_ref().unwrap().readout(0.).unwrap();
        assert_eq!(text.matches(" dB, ").count(), 3, "{text}");
        assert_eq!(text.matches('\u{b0}').count(), 3, "{text}");
        host.app.view = View::Impulse;
        compute(&mut host, &ws).unwrap();
        let text = host.app.current.as_ref().unwrap().readout(2.2).unwrap();
        let second = apps::short(numbers(&reference["iir"]["impulse"])[2]);
        assert!(
            text.starts_with("n = 2.2") && text.contains(&format!("iir: {second}")),
            "{text}"
        );
        host.app.view = View::PoleZero;
        compute(&mut host, &ws).unwrap();
        assert!(host.app.current.as_ref().unwrap().readout(0.5).is_none());

        // The tool tab, and frames with no filter, one and five.
        let kind = AppKind::FilterVisualizer;
        let tab = kind.tab();
        assert_eq!(AppKind::from_tab(tab), Some(kind));
        assert_eq!(
            (kind.label(), kind.is_built()),
            ("Filter Visualization", true)
        );
        assert_eq!(
            serde_json::to_string(&tab).unwrap(),
            "\"filter_visualizer\""
        );
        let decoded: matrust::session::ToolTab =
            serde_json::from_str("\"filter_visualizer\"").unwrap();
        assert_eq!(decoded, tab);
        for count in [0, 1, 5] {
            for width in [820., 1600.] {
                let ctx = egui::Context::default();
                let mut desktop = Workspace::with_context(ctx.clone(), true);
                desktop.variables = source.workspace();
                desktop.toolkits.open_tool(Tool::App(kind));
                let app = &mut desktop.toolkits.apps.filter;
                app.app.points = "64".into();
                for k in 0..count {
                    let form = [Form::Tf, Form::Fir, Form::Sos][k % 3];
                    let f = app.app.add(form, &ws).unwrap();
                    (f.b, f.a, f.sos) = (
                        if form == Form::Fir { "fir" } else { "b" }.into(),
                        "a".into(),
                        "sos".into(),
                    );
                }
                for view in View::ALL {
                    let app = &mut desktop.toolkits.apps.filter;
                    app.app.view = view;
                    if count > 0 {
                        compute(app, &ws).unwrap();
                        assert_eq!(curves(app).len(), count);
                    }
                    let output = frame_at(&mut desktop, &ctx, egui::vec2(width, 950.), vec![]);
                    assert!(!output.shapes.is_empty());
                    assert_eq!(desktop.toolkits.selected_tool(), Some(Tool::App(kind)));
                    let mut found = Vec::new();
                    for label in ["INPUTS", "RESPONSE", "OUTPUT", view.label()] {
                        output
                            .shapes
                            .iter()
                            .for_each(|s| tool_tab_texts(&s.shape, label, &mut found));
                        assert!(!found.is_empty(), "{label} at {width} with {count}");
                        found.clear();
                    }
                }
                desktop.close_tool_tab(Tool::App(kind));
            }
        }
    }

    #[test]
    fn filter_visualizer_release_workload_1_and_n_workers() {
        // Gate profile is release optimisation without LTO. Eight 512-tap
        // FIR filters at the largest grid, in isolated pools one after the
        // other: the app adds no parallel path of its own.
        let workers = std::thread::available_parallelism()
            .map_or(1, |n| n.get())
            .min(8);
        let ws = workspace("b = fir1(511, 0.25);");
        for count in [1, workers] {
            let pool = rayon::ThreadPoolBuilder::new()
                .num_threads(count)
                .build()
                .unwrap();
            let elapsed = pool.install(|| {
                let mut host = Host::new(FilterVisualizer::default());
                for _ in 0..MAX_FILTERS {
                    host.app.add(Form::Fir, &ws).unwrap().b = "b".into();
                }
                host.app.points = "65536".into();
                let start = Instant::now();
                compute(&mut host, &ws).unwrap();
                assert_eq!(curves(&host).len(), MAX_FILTERS);
                assert_eq!(curves(&host)[0].x.len(), 65536);
                start.elapsed()
            });
            eprintln!(
                "filter_visualizer workload workers={count} filters=8 taps=512 points=65536 elapsed={elapsed:?}"
            );
        }
    }
}
