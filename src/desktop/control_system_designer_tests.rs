// Control System Designer app (apps/control_system_designer.rs): the script
// reproduces what the app shows, element factors, the locus gain drag,
// requirement regions, both architectures, discrete loops, undo/redo,
// errors, export without clobbering, the saved script and a rendered tab.
mod control_system_designer_tests {
    use super::*;
    use apps::control_system_designer::{
        Architecture, Compensator, ControlSystemDesigner, Design, Element, ElementKind, Handle,
        Limit, Requirements, Source, dragged_gain, locus_gain, natural, overshoot_damping,
    };
    use apps::{App, AppKind, Host, Snapshot};
    use matrust::Complex64 as Z;
    use std::f64::consts::PI;
    use toolkits::Tool;

    /// One private run, synchronously, delivered to the app.
    fn design(host: &mut Host<ControlSystemDesigner>, ws: &Snapshot) -> Result<(), String> {
        host.app.prepare(ws);
        let request = host.request(ws)?;
        let outputs = apps::run(&mut matrust::Engine::default(), request)?;
        host.app.receive(&outputs)
    }
    fn shown(host: &Host<ControlSystemDesigner>) -> &Design {
        host.app.current.as_deref().expect("a design")
    }
    fn values(engine: &matrust::Engine, name: &str) -> Vec<f64> {
        match engine.array(name) {
            Some(matrust::Array::Numeric(m)) => m.data().to_vec(),
            other => panic!("{name}: {other:?}"),
        }
    }
    fn value(engine: &matrust::Engine, name: &str) -> f64 {
        values(engine, name)[0]
    }
    fn run(setup: &str, script: &str, after: &str) -> matrust::Engine {
        let mut engine = matrust::Engine::default();
        engine.execute(setup).unwrap();
        engine
            .execute(script)
            .unwrap_or_else(|e| panic!("{script}\n{e}"));
        engine
            .execute(after)
            .unwrap_or_else(|e| panic!("{after}\n{e}"));
        engine
    }
    /// Complex values from `re`/`im` outputs, sorted for comparison.
    fn sorted(mut points: Vec<[f64; 2]>) -> Vec<[f64; 2]> {
        points.sort_by(|a, b| a[0].total_cmp(&b[0]).then(a[1].total_cmp(&b[1])));
        points
    }
    fn complex_of(engine: &matrust::Engine, re: &str, im: &str) -> Vec<[f64; 2]> {
        sorted(
            values(engine, re)
                .into_iter()
                .zip(values(engine, im))
                .map(|(x, y)| [x, y])
                .collect(),
        )
    }
    fn close(a: f64, b: f64, tol: f64) -> bool {
        (a - b).abs() <= tol * (1. + b.abs())
    }
    fn assert_points(a: &[[f64; 2]], b: &[[f64; 2]], tol: f64) {
        assert_eq!(a.len(), b.len(), "{a:?} vs {b:?}");
        for (p, q) in a.iter().zip(b) {
            assert!(
                close(p[0], q[0], tol) && close(p[1], q[1], tol),
                "{a:?} vs {b:?}"
            );
        }
    }
    /// G = 1/(s(s+1)(s+5)) (the default plant), C = 60 (s + 1.5)/(s + 15).
    fn lead_design() -> Host<ControlSystemDesigner> {
        let mut host = Host::new(ControlSystemDesigner::default());
        host.app.compensator = Compensator {
            gain: 60.,
            elements: vec![Element::Lead {
                zero: -1.5,
                pole: -15.,
            }],
        };
        host
    }

    /// (element, expected zeros, expected poles).
type ElementCase = (Element, Vec<[f64; 2]>, Vec<[f64; 2]>);

#[test]
    fn control_system_designer_script_reproduces_poles_margins_and_stepinfo() {
        let ws = Snapshot::default();
        let mut host = lead_design();
        design(&mut host, &ws).unwrap();
        let d = shown(&host).clone();
        let script = host.app.script(&ws).unwrap();
        assert!(
            script.contains(
                "G = tf([1], [1 6 5 0]);\nC = zpk([], [], 60);\n\
                 C = C*zpk(-1.5, -15, 1);   % lead, zero at -1.5, pole at -15\n\
                 T = feedback(C*G, 1);\n"
            ),
            "{script}"
        );
        // Only the plots that are on: root locus, Bode with margins, step.
        assert!(script.contains("figure; rlocus(C*G)\nfigure; margin(C*G)\nfigure; step(T)\n"));
        assert!(!script.contains("nichols") && !script.contains("feedback(C, G)"));
        let engine = run(
            "",
            &script,
            "p_ = pole(T); pr_ = real(p_); pi_ = imag(p_);\n\
             [gm_, pm_, wcg_, wcp_] = margin(C*G);\n\
             S_ = stepinfo(T); rise_ = S_.RiseTime; settle_ = S_.SettlingTime;\n\
             over_ = S_.Overshoot; peak_ = S_.Peak;\n\
             q_ = roots([1 21 95 135 90]); qr_ = real(q_); qi_ = imag(q_);",
        );
        // Closed-loop poles: those of the script's T, and the roots of
        // s(s+1)(s+5)(s+15) + 60(s+1.5) = s^4 + 21s^3 + 95s^2 + 135s + 90.
        assert_points(
            &sorted(d.poles.clone()),
            &complex_of(&engine, "pr_", "pi_"),
            1e-12,
        );
        assert_points(
            &sorted(d.poles.clone()),
            &complex_of(&engine, "qr_", "qi_"),
            1e-8,
        );
        assert_points(
            &sorted(d.closed.clone()),
            &complex_of(&engine, "qr_", "qi_"),
            1e-8,
        );
        for (shown, name) in [
            (d.gain_margin, "gm_"),
            (d.phase_margin, "pm_"),
            (d.phase_crossover, "wcg_"),
            (d.gain_crossover, "wcp_"),
            (d.rise, "rise_"),
            (d.settling, "settle_"),
            (d.overshoot, "over_"),
            (d.peak, "peak_"),
        ] {
            assert_eq!(shown, value(&engine, name), "{name}");
        }
        assert!(d.stable);
        // The Output column shows those numbers.
        let rows = host.app.results();
        assert_eq!(rows[0], ("Gain", "60".to_owned()));
        assert!(rows.contains(&("Gain margin", d.gain_margin_text())));
        assert!(rows.contains(&("Phase margin", d.phase_margin_text())));
        assert!(rows.contains(&("Settling time", format!("{} s", apps::short(d.settling)))));
        assert_eq!(host.app.compensator.formula(0.), "60 (s + 1.5) / (s + 15)");
        let summary = host.app.summary().unwrap();
        assert!(summary.starts_with("Gain 60 · phase margin"), "{summary}");
    }

    #[test]
    fn control_system_designer_elements_map_to_zpk_factors() {
        let s3 = 3_f64.sqrt();
        let pair = |re: f64, im: f64| vec![[re, -im], [re, im]];
        // (element, zeros, poles) in the s-plane, from s^2 + 2 zeta wn s + wn^2.
        let continuous: Vec<ElementCase> = vec![
            (Element::RealPole(-3.), vec![], vec![[-3., 0.]]),
            (Element::RealZero(-2.), vec![[-2., 0.]], vec![]),
            (
                Element::ComplexPoles { wn: 2., zeta: 0.5 },
                vec![],
                pair(-1., s3),
            ),
            (
                Element::ComplexZeros { wn: 4., zeta: 0.25 },
                pair(-1., 15_f64.sqrt()),
                vec![],
            ),
            (Element::Integrator, vec![], vec![[0., 0.]]),
            (Element::Differentiator, vec![[0., 0.]], vec![]),
            (
                Element::Lead {
                    zero: -1.,
                    pole: -10.,
                },
                vec![[-1., 0.]],
                vec![[-10., 0.]],
            ),
            (
                Element::Lag {
                    zero: -0.1,
                    pole: -0.01,
                },
                vec![[-0.1, 0.]],
                vec![[-0.01, 0.]],
            ),
            (
                Element::Notch {
                    wn: 5.,
                    depth: 0.1,
                    width: 0.5,
                },
                pair(-0.5, 5. * 0.99_f64.sqrt()),
                pair(-2.5, 5. * 0.75_f64.sqrt()),
            ),
        ];
        let kinds: Vec<ElementKind> = continuous.iter().map(|(e, ..)| e.kind()).collect();
        assert_eq!(kinds, ElementKind::ALL, "every element type is covered");
        let ts = 0.1;
        let to_z = |p: &[f64; 2]| {
            let z = (Z::new(p[0], p[1]) * ts).exp();
            [z.re, z.im]
        };
        for (element, zeros, poles) in &continuous {
            let factor = element.factor(0.);
            let engine = run(
                "",
                &format!("F_ = {factor};"),
                "[z_, p_, k_] = zpkdata(F_, 'v'); zr_ = real(z_); zi_ = imag(z_);\n\
                 pr_ = real(p_); pi_ = imag(p_); ts_ = F_.Ts;",
            );
            assert_points(
                &complex_of(&engine, "zr_", "zi_"),
                &sorted(zeros.clone()),
                1e-14,
            );
            assert_points(
                &complex_of(&engine, "pr_", "pi_"),
                &sorted(poles.clone()),
                1e-14,
            );
            assert_eq!(value(&engine, "k_"), 1., "{factor}");
            assert_eq!(value(&engine, "ts_"), 0., "{factor}");
            // Discrete: pairs, the integrator and the differentiator sit at
            // z = e^(s Ts); real locations are already z-plane values.
            let factor = element.factor(ts);
            let engine = run(
                "",
                &format!("F_ = {factor};"),
                "[z_, p_, k_] = zpkdata(F_, 'v'); zr_ = real(z_); zi_ = imag(z_);\n\
                 pr_ = real(p_); pi_ = imag(p_); ts_ = F_.Ts;",
            );
            let mapped = |roots: &Vec<[f64; 2]>| -> Vec<[f64; 2]> {
                sorted(match element {
                    Element::RealPole(_)
                    | Element::RealZero(_)
                    | Element::Lead { .. }
                    | Element::Lag { .. } => roots.clone(),
                    _ => roots.iter().map(to_z).collect(),
                })
            };
            assert_points(&complex_of(&engine, "zr_", "zi_"), &mapped(zeros), 1e-14);
            assert_points(&complex_of(&engine, "pr_", "pi_"), &mapped(poles), 1e-14);
            assert_eq!(value(&engine, "ts_"), ts, "{factor}");
        }
        // The script multiplies the factors onto the gain, and the formula
        // shows the same roots.
        let c = Compensator {
            gain: 4.,
            elements: vec![
                Element::Integrator,
                Element::ComplexPoles { wn: 2., zeta: 0.5 },
                Element::RealZero(-2.),
            ],
        };
        assert_eq!(
            c.script("K", 0.),
            "K = zpk([], [], 4);\n\
             K = K*zpk([], 0, 1);   % integrator\n\
             K = K*zpk([], [-1+1.7320508075688772i, -1-1.7320508075688772i], 1);   \
             % complex pole pair, wn 2 rad/s, damping 0.5\n\
             K = K*zpk(-2, [], 1);   % real zero at -2\n"
        );
        assert_eq!(c.formula(0.), "4 (s + 2) / (s (s^2 + 2 s + 4))");
        assert_eq!(c.formula(0.1).split(' ').next(), Some("4"));
        assert!(c.formula(0.1).contains("(z - 1)"), "{}", c.formula(0.1));
        let engine = run(
            "",
            &c.script("K", 0.),
            "[z_, p_, k_] = zpkdata(K, 'v'); n_ = numel(p_);",
        );
        assert_eq!((value(&engine, "k_"), value(&engine, "n_")), (4., 3.));
        // Dragging a pair's root sets its frequency and damping; a real
        // root follows the real part; the Bode corner moves along w.
        let mut c = c;
        let pair_handle = Handle {
            element: 1,
            zero: false,
        };
        c.move_root(pair_handle, [-1., -1.], 0.);
        let Element::ComplexPoles { wn, zeta } = c.elements[1] else {
            panic!()
        };
        assert!(close(wn, 2_f64.sqrt(), 1e-15) && close(zeta, 0.5_f64.sqrt(), 1e-15));
        let zero_handle = Handle {
            element: 2,
            zero: true,
        };
        c.move_root(zero_handle, [-7., 0.3], 0.);
        assert_eq!(c.elements[2], Element::RealZero(-7.));
        assert_eq!(c.corner(zero_handle, 0.), Some(7.));
        c.move_corner(zero_handle, 20., 0.);
        assert_eq!(c.elements[2], Element::RealZero(-20.));
        c.move_corner(pair_handle, 3., 0.);
        assert!(matches!(c.elements[1], Element::ComplexPoles { wn, .. } if wn == 3.));
        assert_eq!(c.handles().len(), 2, "the integrator does not move");
        // Discrete: a z-plane root's corner is |ln z|/Ts.
        let mut d = Compensator {
            gain: 1.,
            elements: vec![Element::RealPole(0.5)],
        };
        let h = Handle {
            element: 0,
            zero: false,
        };
        assert!(close(d.corner(h, 0.1).unwrap(), 2_f64.ln() / 0.1, 1e-15));
        d.move_corner(h, 10., 0.1);
        assert!(close(
            match d.elements[0] {
                Element::RealPole(x) => x,
                _ => panic!(),
            },
            (-1_f64).exp(),
            1e-15
        ));
        // A pair dragged in the z-plane: wn and damping of s = ln(z)/Ts.
        let s = Z::new(-2., 3.);
        let z = (s * 0.1).exp();
        let (wn, zeta) = natural(z, 0.1);
        assert!(close(wn, 13_f64.sqrt(), 1e-12) && close(zeta, 2. / 13_f64.sqrt(), 1e-12));
    }

    #[test]
    fn control_system_designer_dragging_a_closed_loop_pole_sets_the_rlocus_gain() {
        let ws = Snapshot::default();
        let mut host = lead_design();
        design(&mut host, &ws).unwrap();
        let d = shown(&host).clone();
        let script = host.app.script(&ws).unwrap();
        let engine = run(
            "",
            &script,
            "[r_, k_] = rlocus(C*G); rr_ = real(r_); ri_ = imag(r_); n_ = size(r_, 1);",
        );
        let (rr, ri, k) = (
            values(&engine, "rr_"),
            values(&engine, "ri_"),
            values(&engine, "k_"),
        );
        let n = value(&engine, "n_") as usize;
        // The app draws rlocus's own branches and gains.
        assert_eq!(d.locus.gains, k);
        assert_eq!(d.locus.branches.len(), n);
        for (b, branch) in d.locus.branches.iter().enumerate() {
            for (j, point) in branch.iter().enumerate() {
                assert_eq!(*point, [rr[b + j * n], ri[b + j * n]]);
            }
        }
        // A point of the upper complex branch, well past the breakaway.
        let j = k.len() * 3 / 4;
        let b = (0..n)
            .max_by(|x, y| ri[x + j * n].total_cmp(&ri[y + j * n]))
            .unwrap();
        assert!(ri[b + j * n] > 0.5, "a complex branch at column {j}");
        let at = |j: usize| [rr[b + j * n], ri[b + j * n]];
        let scale = [1., 1.];
        let (gain, point) = locus_gain(&d.locus, at(j), scale).unwrap();
        assert!(close(gain, k[j], 1e-12), "{gain} vs {}", k[j]);
        assert_points(&[point], &[at(j)], 1e-12);
        // Between two locus points the gain interpolates along the segment;
        // a point just off the locus picks the nearest one.
        let middle = [
            0.5 * (at(j)[0] + at(j + 1)[0]),
            0.5 * (at(j)[1] + at(j + 1)[1]),
        ];
        let (gain, _) = locus_gain(&d.locus, middle, scale).unwrap();
        assert!(close(gain, 0.5 * (k[j] + k[j + 1]), 1e-12));
        let off = [at(j)[0] + 1e-6, at(j)[1] + 1e-6];
        assert!(close(
            locus_gain(&d.locus, off, scale).unwrap().0,
            k[j],
            1e-3
        ));
        // C's gain becomes 60 * k(j); the new closed-loop poles are rlocus's
        // column j.
        let new_gain = dragged_gain(&d.locus, d.compensator.gain, at(j), scale).unwrap();
        assert!(close(new_gain, 60. * k[j], 1e-12));
        host.app.compensator.gain = new_gain;
        design(&mut host, &ws).unwrap();
        let column: Vec<[f64; 2]> = (0..n).map(|b| [rr[b + j * n], ri[b + j * n]]).collect();
        assert_points(&sorted(shown(&host).closed.clone()), &sorted(column), 1e-7);
        // The gain never reaches zero, even at an open-loop pole.
        let pole = d.plant_poles[0];
        assert!(dragged_gain(&d.locus, 60., pole, scale).unwrap() > 0.);
    }

    #[test]
    fn control_system_designer_requirement_regions_and_checks() {
        // Overshoot 5 % needs damping 0.6901 (-ln 0.05 / sqrt(pi^2 + ln^2 0.05)).
        assert!((overshoot_damping(5.) - 0.690_106_730_559_821_7).abs() < 1e-12);
        let on = |value| Limit { on: true, value };
        let mut r = Requirements::default();
        assert!(!r.pole_limits() && !r.forbidden([5., 5.], 0.));
        let s = |re: f64, im: f64| [re, im];
        r.damping = on(0.5);
        assert!(!r.forbidden(s(-1., 1.), 0.)); // damping 0.707
        assert!(r.forbidden(s(-1., 3.), 0.)); // damping 0.316
        assert!(!r.forbidden(s(-4., 0.), 0.));
        assert!(r.forbidden(s(0.5, 0.), 0.));
        r.damping.on = false;
        r.overshoot = on(5.);
        assert!(!r.forbidden(s(-1., 1.), 0.)); // 0.707 >= 0.690
        assert!(r.forbidden(s(-0.6, 0.8), 0.)); // 0.6 < 0.690
        r.overshoot.on = false;
        r.settling = on(2.); // Re(s) <= -4/2
        assert!(!r.forbidden(s(-2.5, 7.), 0.));
        assert!(r.forbidden(s(-1.5, 0.), 0.));
        r.settling.on = false;
        r.frequency = on(2.);
        assert!(r.forbidden(s(-1., 1.), 0.)); // |s| = 1.41
        assert!(!r.forbidden(s(-3., 0.), 0.));
        // Discrete: the same verdicts at z = e^(s Ts); z = 0 is deadbeat.
        r = Requirements {
            settling: on(3.),
            damping: on(0.4),
            frequency: on(0.5),
            ..Requirements::default()
        };
        assert!(r.pole_limits());
        let ts = 0.1;
        for p in [
            s(-1., 1.),
            s(-2., 1.),
            s(-2., 6.),
            s(-0.3, 0.1),
            s(-5., 0.),
            s(0.2, 0.),
        ] {
            let z = (Z::new(p[0], p[1]) * ts).exp();
            assert_eq!(r.forbidden(p, 0.), r.forbidden([z.re, z.im], ts), "{p:?}");
        }
        assert!(!r.forbidden([0., 0.], ts));
        // Pass and fail from the design's stepinfo, damp and margin.
        let ws = Snapshot::default();
        let mut host = lead_design();
        design(&mut host, &ws).unwrap();
        let d = shown(&host).clone();
        let gm_db = 20. * d.gain_margin.log10();
        host.app.requirements = Requirements {
            settling: on(d.settling + 0.1),
            overshoot: on(d.overshoot - 0.5),
            damping: on(0.1),
            frequency: on(100.),
            gain_margin: on(gm_db - 1.),
            phase_margin: on(d.phase_margin + 1.),
        };
        let checks = host.app.requirements.checks(&d);
        let verdicts: Vec<bool> = checks.iter().map(|c| c.pass).collect();
        assert_eq!(
            verdicts,
            [true, false, true, false, true, false],
            "{checks:?}"
        );
        assert!(checks[0].label.starts_with("Settling time ≤"));
        let summary = host.app.summary().unwrap();
        assert!(summary.ends_with("3 requirements not met"), "{summary}");
        // Requirements are display only: the script and its run stay.
        assert_eq!(host.app.script(&ws).unwrap(), {
            let fresh = lead_design();
            fresh.app.script(&ws).unwrap()
        });
    }

    #[test]
    fn control_system_designer_c_in_the_feedback_path() {
        let ws = Snapshot::default();
        let mut host = lead_design();
        host.app.architecture = Architecture::Feedback;
        host.app.views.effort = true;
        design(&mut host, &ws).unwrap();
        let script = host.app.script(&ws).unwrap();
        assert!(script.contains("T = feedback(G, C);\n"), "{script}");
        assert!(
            script.contains("figure; step(feedback(1, C*G))\n"),
            "{script}"
        );
        // Same loop poles as C in the forward path; T(0) = 1/C(0) since G
        // has an integrator: C(0) = 60 * 1.5 / 15 = 6.
        let engine = run(
            "",
            &script,
            "dc_ = dcgain(T); q_ = roots([1 21 95 135 90]); qr_ = real(q_); qi_ = imag(q_);",
        );
        assert!(close(value(&engine, "dc_"), 1. / 6., 1e-12));
        let d = shown(&host);
        assert_points(
            &sorted(d.poles.clone()),
            &complex_of(&engine, "qr_", "qi_"),
            1e-8,
        );
        let last = d.step.last().unwrap()[1];
        assert!(
            (last - 1. / 6.).abs() < 1e-3,
            "step settles at T(0): {last}"
        );
        assert!(d.effort.is_some());
        // A sensor and a prefilter join the loop: T = F*G/(1 + C*G*H).
        host.app.h = Source::typed("1", "0.1 1");
        host.app.f = Source::typed("2", "1");
        let script = host.app.script(&ws).unwrap();
        assert!(
            script.contains("H = tf([1], [0.1 1]);\nF = tf([2], [1]);\n"),
            "{script}"
        );
        assert!(script.contains("T = F*feedback(G, C*H);\n"), "{script}");
        assert!(script.contains("figure; rlocus(C*G*H)\n"), "{script}");
        assert!(
            script.contains("figure; step(F*feedback(1, C*G*H))\n"),
            "{script}"
        );
        design(&mut host, &ws).unwrap();
        let engine = run("", &script, "dc_ = dcgain(T);");
        assert!(close(value(&engine, "dc_"), 2. / 6., 1e-12));
        // C in the forward path with the same H and F.
        host.app.architecture = Architecture::Forward;
        let script = host.app.script(&ws).unwrap();
        assert!(script.contains("T = F*feedback(C*G, H);\n"), "{script}");
        assert!(
            script.contains("figure; step(F*feedback(C, G*H))\n"),
            "{script}"
        );
        design(&mut host, &ws).unwrap();
        assert_eq!(
            host.app.exported(&ws),
            ["C", "T", "G", "H", "F"],
            "typed models are exported with C and T"
        );
    }

    #[test]
    fn control_system_designer_discrete_loop() {
        let mut engine = matrust::Engine::default();
        engine
            .execute("Gd = c2d(zpk([], [0 -1 -5], 1), 0.1); Hc = tf(1, [1 1]);")
            .unwrap();
        let mut ws = Snapshot::default();
        ws.sync(&engine.workspace());
        let mut host = Host::new(ControlSystemDesigner::default());
        host.app.g = Source::variable("Gd");
        host.app.compensator = Compensator {
            gain: 5.,
            elements: vec![Element::Lead {
                zero: 0.9,
                pole: 0.5,
            }],
        };
        assert_eq!(host.app.inputs(), ["Gd"]);
        design(&mut host, &ws).unwrap();
        assert_eq!(host.app.ts(), 0.1);
        let script = host.app.script(&ws).unwrap();
        assert!(
            script.contains(
                "% G is the workspace variable Gd.\nC = zpk([], [], 5, 0.1);\n\
                 C = C*zpk(0.9, 0.5, 1, 0.1);"
            ),
            "{script}"
        );
        let d = shown(&host).clone();
        assert_eq!(d.ts, 0.1);
        let engine = run(
            "Gd = c2d(zpk([], [0 -1 -5], 1), 0.1);",
            &script,
            "p_ = pole(T); pr_ = real(p_); pi_ = imag(p_);\n\
             [gm_, pm_, wcg_, wcp_] = margin(C*Gd); S_ = stepinfo(T); settle_ = S_.SettlingTime;\n\
             a_ = abs(p_);",
        );
        assert_points(
            &sorted(d.poles.clone()),
            &complex_of(&engine, "pr_", "pi_"),
            1e-12,
        );
        assert_eq!(d.gain_margin, value(&engine, "gm_"));
        assert_eq!(d.phase_margin, value(&engine, "pm_"));
        assert_eq!(d.settling, value(&engine, "settle_"));
        assert!(d.stable && values(&engine, "a_").iter().all(|a| *a < 1.));
        // The discrete Bode stops at the Nyquist frequency pi/Ts.
        let last = d.magnitude.last().unwrap()[0];
        assert!(10_f64.powf(last) <= PI / 0.1 + 1e-9, "{last}");
        // Typed models take the sample time; real roots of C move with it
        // (s = -10 becomes z = e^-1), pairs keep their frequency.
        let mut host = Host::new(ControlSystemDesigner::default());
        host.app.compensator.elements = vec![
            Element::RealPole(-10.),
            Element::RealPole(-20.),
            Element::ComplexZeros { wn: 2., zeta: 0.5 },
        ];
        host.app.h = Source::typed("1", "0.5 1");
        let empty = Snapshot::default();
        design(&mut host, &empty).unwrap();
        host.app.sample_time = "0.1".into();
        design(&mut host, &empty).unwrap();
        assert_eq!(
            host.app.compensator.elements,
            [
                Element::RealPole((-1_f64).exp()),
                Element::RealPole((-2_f64).exp()),
                Element::ComplexZeros { wn: 2., zeta: 0.5 }
            ]
        );
        let script = host.app.script(&empty).unwrap();
        assert!(
            script.contains("G = tf([1], [1 6 5 0], 0.1);\nH = tf([1], [0.5 1], 0.1);\n"),
            "{script}"
        );
        assert_eq!(shown(&host).ts, 0.1);
        // New elements are placed in the z-plane.
        host.app.add_element(ElementKind::Integrator, None);
        host.app
            .add_element(ElementKind::RealPole, Some([0.25, 0.4]));
        let (_, poles) = host.app.compensator.elements[3].roots(0.1);
        assert_eq!(poles, [Z::new(1., 0.)]);
        assert_eq!(host.app.compensator.elements[4], Element::RealPole(0.25));
        // A continuous sensor in a discrete loop: the engine's error.
        let mut host = Host::new(ControlSystemDesigner::default());
        host.app.g = Source::variable("Gd");
        host.app.h = Source::variable("Hc");
        let error = design(&mut host, &ws).unwrap_err();
        assert!(error.contains("sampling intervals must agree"), "{error}");
    }

    #[test]
    fn control_system_designer_undo_and_redo_compensator_edits() {
        let ws = Snapshot::default();
        let mut host = Host::new(ControlSystemDesigner::default());
        let app = &mut host.app;
        app.prepare(&ws);
        assert!(!app.can_undo() && !app.can_redo());
        app.compensator.gain = 5.;
        app.prepare(&ws);
        app.add_element(ElementKind::Lead, None);
        app.prepare(&ws);
        app.compensator.move_root(
            Handle {
                element: 0,
                zero: false,
            },
            [-20., 0.],
            0.,
        );
        app.prepare(&ws);
        let lead = |pole| Element::Lead { zero: -1., pole };
        assert_eq!(app.compensator.elements, [lead(-20.)]);
        assert!(app.undo());
        assert_eq!(app.compensator.elements, [lead(-10.)]);
        assert!(app.undo());
        assert_eq!(
            (app.compensator.gain, app.compensator.elements.len()),
            (5., 0)
        );
        assert!(app.undo());
        assert_eq!(app.compensator, Compensator::default());
        assert!(!app.undo() && app.can_redo());
        assert!(app.redo() && app.redo());
        assert_eq!(app.compensator.elements, [lead(-10.)]);
        // The script follows the restored compensator.
        assert!(app.script(&ws).unwrap().contains("C = C*zpk(-1, -10, 1);"));
        // A new edit after an undo drops the redo steps.
        assert!(app.can_redo());
        app.compensator.gain = 7.;
        app.prepare(&ws);
        assert!(!app.can_redo() && !app.redo());
        assert!(app.undo());
        assert_eq!(app.compensator.gain, 5.);
    }

    #[test]
    fn control_system_designer_errors_show_and_keep_the_last_design() {
        let mut engine = matrust::Engine::default();
        engine.execute("x = 5;").unwrap();
        let mut ws = Snapshot::default();
        ws.sync(&engine.workspace());
        let mut host = lead_design();
        design(&mut host, &ws).unwrap();
        let good = shown(&host).clone();
        // An improper C never reaches the engine.
        host.app.compensator.elements = vec![Element::Differentiator, Element::RealZero(-3.)];
        let error = host.app.script(&ws).unwrap_err();
        assert!(error.contains("improper"), "{error}");
        assert!(error.contains("2 zeros and 0 poles"), "{error}");
        host.app.compensator.elements = vec![Element::ComplexPoles { wn: 0., zeta: 0.5 }];
        let error = host.app.script(&ws).unwrap_err();
        assert!(error.contains("natural frequency"), "{error}");
        host.app.compensator = good.compensator.clone();
        host.app.compensator.gain = 0.;
        assert!(host.app.script(&ws).unwrap_err().contains("gain"));
        host.app.compensator.gain = 60.;
        // A variable that is not an LTI model, a missing one, bad coefficients.
        host.app.g = Source {
            use_variable: true,
            variable: "x".into(),
            ..host.app.g.clone()
        };
        let error = host.app.script(&ws).unwrap_err();
        assert!(error.contains("not a tf, zpk or ss model"), "{error}");
        host.app.g.variable = "gone".into();
        assert!(
            host.request(&ws)
                .err()
                .unwrap()
                .contains("gone does not exist")
        );
        host.app.g = Source::typed("1", "1 x");
        let error = host.app.script(&ws).unwrap_err();
        assert!(error.starts_with("Plant G denominator"), "{error}");
        // Output names must differ from the loop's models.
        host.app.g = Source::typed("1", "1 6 5 0");
        host.app.c_name = "G".into();
        host.app.c_edited = true;
        assert!(
            host.app
                .script(&ws)
                .unwrap_err()
                .contains("compensator name G")
        );
        host.app.c_edited = false;
        // An engine error (a typed sensor with a different sample time is
        // impossible, so a workspace one): the status line carries it and
        // the plots keep the last good design.
        engine.execute("Hd = c2d(tf(1, [1 1]), 0.2);").unwrap();
        ws.sync(&engine.workspace());
        host.app.h = Source {
            use_variable: true,
            variable: "Hd".into(),
            ..host.app.h.clone()
        };
        let ctx = egui::Context::default();
        let deadline = Instant::now() + Duration::from_secs(20);
        loop {
            host.refresh(&ctx, &ws);
            if !host.running() {
                break;
            }
            assert!(Instant::now() < deadline);
            std::thread::sleep(Duration::from_millis(2));
        }
        let (text, status) = host.status();
        assert_eq!(status, toolkits::AppStatus::Error);
        assert!(text.contains("sampling intervals must agree"), "{text}");
        assert_eq!(shown(&host).compensator, good.compensator);
        assert_eq!(shown(&host).poles, good.poles);
        // The improper C through the host: a script error in the status line.
        host.app.h = Source::typed("1", "1");
        host.app.compensator.elements = vec![Element::Differentiator];
        host.refresh(&ctx, &ws);
        let (text, status) = host.status();
        assert_eq!(status, toolkits::AppStatus::Error);
        assert!(text.contains("improper"), "{text}");
    }

    fn open_designer(
        app: &mut Workspace,
        ctx: &egui::Context,
        size: egui::Vec2,
    ) -> egui::FullOutput {
        app.toolkits
            .open_tool(Tool::App(AppKind::ControlSystemDesigner));
        settle(app, ctx, size)
    }
    /// Frames until the app's run finished and a design is shown.
    fn settle(app: &mut Workspace, ctx: &egui::Context, size: egui::Vec2) -> egui::FullOutput {
        let deadline = Instant::now() + Duration::from_secs(30);
        loop {
            let output = frame_at(app, ctx, size, vec![]);
            let host = &app.toolkits.apps.control;
            if host.app.current.is_some() && !host.running() {
                return frame_at(app, ctx, size, vec![]);
            }
            assert!(Instant::now() < deadline, "{:?}", host.status());
            drop(output);
            std::thread::sleep(Duration::from_millis(5));
        }
    }
    fn painted(output: &egui::FullOutput, label: &str) -> bool {
        output
            .shapes
            .iter()
            .any(|s| text_position(&s.shape, label).is_some())
    }

    #[test]
    fn control_system_designer_tab_renders_editors_and_output() {
        let size = egui::vec2(1440., 900.);
        let ctx = egui::Context::default();
        let mut app = Workspace::with_context(ctx.clone(), true);
        let output = open_designer(&mut app, &ctx, size);
        assert_eq!(
            app.toolkits.selected_tool(),
            Some(Tool::App(AppKind::ControlSystemDesigner))
        );
        for label in [
            "INPUTS",
            "DESIGN",
            "OUTPUT",
            "Root locus",
            "Bode",
            "Nichols",
            "Step",
            "r to u",
            "Root locus of C·G·H",
            "Compensator C",
            "C = 1",
            "Add element",
            "Undo",
            "Design requirements",
            "Closed-loop poles",
            "Gain margin",
            "Export C and T",
            "Generate script",
        ] {
            assert!(painted(&output, label), "{label} not painted");
        }
        let (text, status) = app
            .toolkits
            .app_status(Tool::App(AppKind::ControlSystemDesigner), false)
            .unwrap();
        assert_eq!(status, toolkits::AppStatus::Success);
        assert!(text.starts_with("Gain 1 · phase margin"), "{text}");
        // The Nichols editor with ngrid, the r to u step and a shaded
        // requirement region all render.
        app.toolkits.apps.control.app.requirements.damping.on = true;
        click_label_at(&mut app, &ctx, size, "Nichols");
        click_label_at(&mut app, &ctx, size, "r to u");
        let output = settle(&mut app, &ctx, size);
        let control = &app.toolkits.apps.control.app;
        assert!(control.views.nichols && control.views.effort);
        let nichols = control.current.as_ref().unwrap().nichols.as_ref().unwrap();
        assert!(nichols.grid.len() > 10, "ngrid contours");
        for label in ["Nichols of C·G·H with ngrid", "Step r to u (plant input)"] {
            assert!(painted(&output, label), "{label} not painted");
        }
        // A narrow host stacks OUTPUT under the main view and still renders.
        let narrow = egui::vec2(900., 700.);
        let mut output = frame_at(&mut app, &ctx, narrow, vec![]);
        let design = clear_panes_texts(&output)
            .into_iter()
            .find(|t| t.text == "DESIGN")
            .unwrap();
        let visible_output = |frame: &egui::FullOutput| {
            clear_panes_texts(frame)
                .into_iter()
                .any(|t| t.text == "OUTPUT" && t.clip.contains_rect(t.glyphs))
        };
        // With all five plots enabled the Output section is below the fold.
        for _ in 0..30 {
            if visible_output(&output) {
                break;
            }
            output = frame_at(
                &mut app,
                &ctx,
                narrow,
                vec![
                    egui::Event::PointerMoved(design.clip.center()),
                    egui::Event::MouseWheel {
                        phase: egui::TouchPhase::Move,
                        unit: egui::MouseWheelUnit::Point,
                        delta: egui::vec2(0., -80.),
                        modifiers: egui::Modifiers::NONE,
                    },
                ],
            );
        }
        assert!(visible_output(&output), "stacked Output must be reachable");
        // Closing the tab keeps the inputs for the next open.
        app.close_tool_tab(Tool::App(AppKind::ControlSystemDesigner));
        assert!(app.toolkits.tabs().next().is_none());
        assert!(app.toolkits.apps.control.app.views.nichols);
    }

    #[test]
    fn control_system_designer_export_creates_c_and_t_without_clobbering() {
        let size = egui::vec2(1440., 900.);
        let ctx = egui::Context::default();
        let mut app = Workspace::with_context(ctx.clone(), true);
        app.run("C = 7; T = 'keep'; G = 3;".into());
        finish(&mut app);
        open_designer(&mut app, &ctx, size);
        app.toolkits.apps.control.app.compensator = Compensator {
            gain: 60.,
            elements: vec![Element::Lead {
                zero: -1.5,
                pole: -15.,
            }],
        };
        let output = settle(&mut app, &ctx, size);
        assert!(painted(&output, "60 (s + 1.5) / (s + 15)"));
        let poles = shown(&app.toolkits.apps.control).poles.clone();
        click_label_at(&mut app, &ctx, size, "Export C and T");
        finish(&mut app);
        frame_at(&mut app, &ctx, size, vec![]);
        assert!(
            matches!(&app.variables["C"], matrust::Array::Numeric(m) if m.data() == [7.]),
            "C is kept"
        );
        assert!(matches!(&app.variables["G"], matrust::Array::Numeric(m) if m.data() == [3.]));
        assert!(matches!(&app.variables["T"], matrust::Array::Character(_)));
        assert_eq!(
            matrust::engine::lti_class(&app.variables["C1"]),
            Some("zpk"),
            "{}",
            app.status
        );
        assert!(matrust::engine::lti_class(&app.variables["T1"]).is_some());
        assert_eq!(matrust::engine::lti_class(&app.variables["G1"]), Some("tf"));
        assert!(app.toolkits.apps.control.message.contains("C1, T1"));
        // The exported closed loop has the poles the app shows.
        let mut engine = matrust::Engine::default();
        engine.merge_workspace(app.variables.clone()).unwrap();
        engine
            .execute("p_ = pole(T1); pr_ = real(p_); pi_ = imag(p_);")
            .unwrap();
        assert_points(&sorted(poles), &complex_of(&engine, "pr_", "pi_"), 1e-12);
        // Taken names move on: C2, T2 and G2 next.
        frame_at(&mut app, &ctx, size, vec![]);
        let ws = &app.toolkits.apps.workspace;
        let script = app.toolkits.apps.control.app.script(ws).unwrap();
        assert!(
            script.contains("G2 = tf(")
                && script.contains("C2 = zpk(")
                && script.contains("T2 = feedback(C2*G2, 1);"),
            "{script}"
        );
    }

    #[test]
    fn control_system_designer_generate_script_saves_header_and_the_same_script() {
        let size = egui::vec2(1440., 900.);
        let ctx = egui::Context::default();
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("design.m");
        let mut app = Workspace::with_context(ctx.clone(), true);
        open_designer(&mut app, &ctx, size);
        app.toolkits.apps.control.app.compensator.elements = vec![Element::Lead {
            zero: -1.5,
            pole: -15.,
        }];
        app.toolkits.apps.control.app.views.root_locus = false;
        settle(&mut app, &ctx, size);
        let script = {
            let ws = &app.toolkits.apps.workspace;
            app.toolkits.apps.control.app.script(ws).unwrap()
        };
        assert!(!script.contains("rlocus") && script.contains("margin(C*G)"));
        app.toolkits.apps.test_save_path = Some(path.clone());
        click_label_at(&mut app, &ctx, size, "Generate script");
        finish(&mut app);
        frame_at(&mut app, &ctx, size, vec![]);
        let saved = std::fs::read_to_string(&path).unwrap();
        assert!(
            saved.starts_with("% Control System Designer (MatRust app), generated "),
            "{saved}"
        );
        assert!(saved.ends_with(&script), "{saved}");
        assert!(
            app.toolkits.apps.control.message.starts_with("Saved"),
            "{}",
            app.toolkits.apps.control.message
        );
        // The saved file runs and gives the closed loop the app shows.
        let poles = shown(&app.toolkits.apps.control).poles.clone();
        let engine = run("", &saved, "p_ = pole(T); pr_ = real(p_); pi_ = imag(p_);");
        assert_points(&sorted(poles), &complex_of(&engine, "pr_", "pi_"), 1e-12);
    }

    #[test]
    fn wide_short_design_shows_every_editor_and_both_step_responses() {
        let (ctx, mut desktop) = redesign_app(session::DesktopTheme::Light);
        let designer = &mut desktop.toolkits.apps.control;
        designer.app.views.nichols = true;
        designer.app.views.effort = true;
        let ws = apps::Snapshot::default();
        let deadline = Instant::now() + Duration::from_secs(30);
        // Native tabs, command bar and status take 96 points of the 650-point window.
        let render = |designer: &mut Host<ControlSystemDesigner>| {
            app_framework_tests::layout_frame(&ctx, egui::vec2(1600., 554.), vec![], |ui| {
                designer.show(ui, &ws, false, &mut None)
            })
        };
        loop {
            render(designer);
            if !designer.running() {
                break;
            }
            assert!(Instant::now() < deadline);
            std::thread::sleep(Duration::from_millis(2));
        }
        let out = render(designer);
        let texts = clear_panes_texts(&out);
        for (axis, expected) in [
            ("Real", 1),
            ("Magnitude (dB)", 2),
            ("Phase (deg)", 2),
            ("Output y", 1),
            ("Input u", 1),
            ("Time (s)", 2),
        ] {
            let visible = texts
                .iter()
                .filter(|t| t.text == axis && t.clip.contains_rect(t.glyphs))
                .count();
            assert_eq!(visible, expected, "visible {axis} axes");
        }
        for (axis, caption) in [("Output y", "Step r to y"), ("Input u", "Step r to u")] {
            let axis = texts.iter().find(|t| t.text == axis).unwrap().glyphs;
            let caption = texts
                .iter()
                .find(|t| t.text.starts_with(caption))
                .unwrap()
                .glyphs;
            assert!(
                caption.bottom() < axis.top(),
                "response title overlaps its caption"
            );
        }
    }

    #[test]
    fn compact_design_separates_locus_and_bode_axes() {
        let (ctx, mut desktop) = redesign_app(session::DesktopTheme::Light);
        let designer = &mut desktop.toolkits.apps.control;
        let ws = apps::Snapshot::default();
        designer.refresh(&ctx, &ws);
        let until = Instant::now() + Duration::from_secs(30);
        while designer.running() {
            assert!(Instant::now() < until);
            std::thread::sleep(Duration::from_millis(2));
            designer.refresh(&ctx, &ws);
        }
        let mut render = || {
            crate::headless::run(
                &ctx,
                egui::RawInput {
                    screen_rect: Some(egui::Rect::from_min_size(
                        egui::Pos2::ZERO,
                        egui::vec2(900., 554.),
                    )),
                    ..Default::default()
                },
                |ui| designer.show(ui, &ws, false, &mut None),
            )
        };
        render();
        let out = render();
        let texts = clear_panes_texts(&out);
        let ink = |label: &str| texts.iter().find(|t| t.text == label).unwrap().glyphs;
        let magnitude = ink("Magnitude (dB)");
        let phase = ink("Phase (deg)");
        assert!(magnitude.bottom() < phase.top());
        let frames: Vec<_> = out
            .shapes
            .iter()
            .map(|s| s.clip_rect)
            .filter(|r| r.width() > 100. && r.width() < 320. && r.height() > 48. && r.left() > 280.)
            .collect();
        let locus = frames
            .iter()
            .find(|r| r.right() <= magnitude.left())
            .expect("root locus frame");
        for axis in [magnitude, phase] {
            let bode = frames
                .iter()
                .find(|r| r.left() >= axis.right() && r.y_range().contains(axis.center().y))
                .expect("Bode frame");
            assert!(!locus.intersects(*bode));
            assert!(axis.top() >= bode.top() && axis.bottom() <= bode.bottom());
        }
    }
}
