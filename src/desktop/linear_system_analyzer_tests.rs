// Linear System Analyzer app (apps/linear_system_analyzer.rs): the script
// reproduces the numbers the app shows, every plot type's call, MIMO I/O
// pairs, expression systems, export without clobbering, errors, the saved
// script and a rendered four-plot tab. Expected values are closed forms:
// 1/(s^2+0.6s+1) has zeta 0.3 and wn 1; 1/(s+1)^3 has its phase crossover at
// sqrt(3) where the gain is 1/8; diag(1/(s+1), 1/(s+2)) has singular values
// 1/sqrt(1+w^2) and 1/sqrt(4+w^2).
mod linear_system_analyzer_tests {
    use super::*;
    use apps::linear_system_analyzer::{
        Analysis, Char, Curves, LinearSystemAnalyzer, PlotType, SystemResult, check_expression,
        identifiers,
    };
    use apps::{App, AppKind, Host, Snapshot};
    use toolkits::Tool;

    type Lsa = Host<LinearSystemAnalyzer>;

    const SETUP: &str = "G = tf(1, [1 0.6 1]); L = tf(1, [1 3 3 1]); \
                         P = ss([-1 0; 0 -2], eye(2), eye(2), zeros(2)); \
                         D = tf(0.5, [1 -0.5], 0.1); S1 = ss(-1, 1, 1, 0); \
                         t = (0:0.05:5)'; u = sin(t);";

    fn workspace(code: &str) -> Snapshot {
        let mut engine = matrust::Engine::default();
        engine.execute(code).unwrap();
        let mut ws = Snapshot::default();
        ws.sync(&engine.workspace());
        ws
    }
    fn host(ws: &Snapshot, shown: &[&str]) -> Lsa {
        let mut host = Host::new(LinearSystemAnalyzer::default());
        host.app.prepare(ws);
        show(&mut host, shown);
        host
    }
    fn show(host: &mut Lsa, shown: &[&str]) {
        for s in &mut host.app.systems {
            s.shown = shown.contains(&s.name.as_str());
        }
    }
    /// One plot of the given type with every characteristic switched on.
    fn single(host: &mut Lsa, kind: PlotType) {
        host.app.count = 1;
        host.app.active = 0;
        host.app.plots[0].kind = kind;
        for c in kind.characteristics() {
            host.app.plots[0].set(*c, true);
        }
    }
    /// The systems query and one private run, synchronously.
    fn analyze(host: &mut Lsa, ws: &Snapshot) -> Result<(), String> {
        host.app.prepare(ws);
        if let Some((request, names)) = host.app.shape_request_for(ws) {
            let key = request.key;
            let result = apps::run(&mut matrust::Engine::default(), request);
            host.app.receive_shapes(key, &names, &result);
        }
        let request = host.request(ws)?;
        let outputs = apps::run(&mut matrust::Engine::default(), request)?;
        host.app.receive(&outputs)
    }
    fn analysis(host: &Lsa) -> &Analysis {
        host.app.current.as_ref().expect("a result")
    }
    fn system<'a>(host: &'a Lsa, label: &str) -> &'a SystemResult {
        analysis(host)
            .systems
            .iter()
            .find(|s| s.label == label)
            .unwrap_or_else(|| panic!("no system {label}"))
    }
    /// A fresh Engine that ran SETUP and then `script`.
    fn run_script(script: &str) -> matrust::Engine {
        let mut engine = matrust::Engine::default();
        engine.execute(SETUP).unwrap();
        engine
            .execute(script)
            .unwrap_or_else(|e| panic!("{script}\n{e}"));
        engine
    }
    fn values(engine: &mut matrust::Engine, expr: &str) -> Vec<f64> {
        engine
            .execute(&format!("lsa_v__ = double({expr});"))
            .unwrap_or_else(|e| panic!("{expr}: {e}"));
        match engine.array("lsa_v__") {
            Some(matrust::Array::Numeric(m)) => m.data().to_vec(),
            other => panic!("{expr}: {other:?}"),
        }
    }
    fn close(a: &[f64], b: &[f64]) {
        assert_eq!(a.len(), b.len());
        for (x, y) in a.iter().zip(b) {
            assert!((x - y).abs() <= 1e-12 * (1. + y.abs()), "{x} vs {y}");
        }
    }
    fn row<'a>(rows: &'a [(&'static str, String)], label: &str) -> &'a str {
        &rows
            .iter()
            .find(|(l, _)| *l == label)
            .unwrap_or_else(|| panic!("no row {label}: {rows:?}"))
            .1
    }

    #[test]
    fn linear_system_analyzer_generated_script_reproduces_the_numbers_shown() {
        let ws = workspace(SETUP);
        // stepinfo of 1/(s^2+0.6s+1): the script prints it, the app shows it.
        let mut host = host(&ws, &["G"]);
        single(&mut host, PlotType::Step);
        analyze(&mut host, &ws).unwrap();
        let script = host.app.script(&ws).unwrap();
        assert!(script.contains("\nstep(G)\n"), "{script}");
        assert!(script.contains("\nstepinfo(G)\n"), "{script}");
        let shown = system(&host, "G").stepinfo.unwrap();
        let mut engine = run_script(&script);
        for (field, value) in [
            ("RiseTime", shown.rise),
            ("SettlingTime", shown.settling),
            ("Overshoot", shown.overshoot),
            ("Peak", shown.peak),
            ("PeakTime", shown.peak_time),
        ] {
            assert_eq!(values(&mut engine, &format!("ans.{field}")), [value], "{field}");
        }
        // zeta = 0.3: overshoot 100*exp(-pi*zeta/sqrt(1-zeta^2)) = 37.23 %.
        let zeta: f64 = 0.3;
        let overshoot = 100. * (-std::f64::consts::PI * zeta / (1. - zeta * zeta).sqrt()).exp();
        assert!((shown.overshoot - overshoot).abs() < 0.05, "{shown:?}");
        let g = system(&host, "G");
        assert_eq!(g.dc, [1.]);
        assert!(g.stable);
        assert!(g.zeta.iter().all(|z| (z - 0.3).abs() < 1e-12), "{:?}", g.zeta);
        let rows = host.app.table(g);
        assert_eq!(row(&rows, "Overshoot"), format!("{} %", apps::short(shown.overshoot)));
        assert_eq!(row(&rows, "Poles"), "-0.3 ± 0.95394i");
        // allmargin of 1/(s+1)^3: gain margin 8 at sqrt(3), no gain crossover.
        let mut host = Host::new(LinearSystemAnalyzer::default());
        host.app.prepare(&ws);
        show(&mut host, &["L"]);
        single(&mut host, PlotType::Bode);
        analyze(&mut host, &ws).unwrap();
        let script = host.app.script(&ws).unwrap();
        assert!(script.contains("\nbode(L)\n") && script.contains("\nallmargin(L)\n"));
        let margins = system(&host, "L").margins.clone().unwrap();
        let mut engine = run_script(&script);
        assert_eq!(values(&mut engine, "ans.GainMargin"), margins.gain);
        assert_eq!(values(&mut engine, "ans.GMFrequency"), margins.gain_w);
        assert_eq!(values(&mut engine, "ans.PhaseMargin"), margins.phase);
        assert!((margins.gain[0] - 8.).abs() < 1e-9, "{margins:?}");
        assert!((margins.gain_w[0] - 3_f64.sqrt()).abs() < 1e-9, "{margins:?}");
        assert!(margins.phase.is_empty());
        let rows = host.app.table(system(&host, "L"));
        assert_eq!(row(&rows, "Gain margin"), "18.062 dB at 1.7321 rad/s");
        // sigma of a 2x2 ss: the peak the app marks is the script's.
        let mut host = Host::new(LinearSystemAnalyzer::default());
        host.app.prepare(&ws);
        show(&mut host, &["P"]);
        single(&mut host, PlotType::Sigma);
        analyze(&mut host, &ws).unwrap();
        let script = host.app.script(&ws).unwrap();
        assert!(script.contains("\nsigma(P)\n") && script.contains("\nmax(max(sigma(P)))\n"));
        let Curves::Sigma { w, sv, count, peak } = &analysis(&host).tiles[0].curves[0].1 else {
            panic!("sigma curves")
        };
        assert_eq!(*count, 2);
        for (k, w) in w.iter().enumerate() {
            let expected = [1. / (1. + w * w).sqrt(), 1. / (4. + w * w).sqrt()];
            for r in 0..2 {
                assert!((sv[r + 2 * k] - expected[r]).abs() < 1e-12 * expected[r]);
            }
        }
        let [_, peak] = peak.unwrap();
        let mut engine = run_script(&script);
        assert_eq!(values(&mut engine, "ans"), [peak]);
        assert_eq!(values(&mut engine, "sigma(P)"), *sv);
        // A discrete tf: y(k) = 1 - 0.5^k, DC gain 1, Bode up to pi/Ts.
        let mut host = Host::new(LinearSystemAnalyzer::default());
        host.app.prepare(&ws);
        show(&mut host, &["D"]);
        host.app.count = 2;
        host.app.plots[0].set(Char::Peak, true);
        analyze(&mut host, &ws).unwrap();
        let script = host.app.script(&ws).unwrap();
        let d = system(&host, "D");
        assert_eq!((d.dc.as_slice(), d.shape.ts), ([1.].as_slice(), 0.1));
        let info = d.stepinfo.unwrap();
        let mut engine = run_script(&script);
        assert_eq!(values(&mut engine, "ans.SettlingTime"), [info.settling]);
        let Curves::Time { t, y, .. } = &analysis(&host).tiles[0].curves[0].1 else {
            panic!("step curves")
        };
        for (k, (t, y)) in t.iter().zip(y).enumerate().take(20) {
            assert!((t - 0.1 * k as f64).abs() < 1e-12);
            assert!((y - (1. - 0.5_f64.powi(k as i32))).abs() < 1e-12, "{k}: {y}");
        }
        let Curves::Frequency { w, .. } = &analysis(&host).tiles[1].curves[0].1 else {
            panic!("bode curves")
        };
        assert!((w.last().unwrap() - std::f64::consts::PI / 0.1).abs() < 1e-9);
        // The export line is the active plot's data: [y, t] = step(D).
        assert!(script.ends_with("[y, t1] = step(D);\n"), "{script}");
        assert_eq!(values(&mut engine, "y"), *y);
    }

    #[test]
    fn linear_system_analyzer_every_plot_type_appears_in_the_script() {
        let ws = workspace(SETUP);
        let mut host = host(&ws, &["G", "S1"]);
        host.app.plots[0].x0 = "1".into();
        host.app.plots[0].ngrid = true;
        host.app.plots[0].u = "u".into();
        host.app.plots[0].t = "t".into();
        for (kind, call, systems) in [
            (PlotType::Step, "step(G, S1)\n", 2),
            (PlotType::Impulse, "impulse(G, S1)\n", 2),
            (PlotType::Initial, "initial(S1, [1])\n", 1),
            (PlotType::Bode, "bode(G, S1)\n", 2),
            (
                PlotType::BodeMagnitude,
                "% One input and one output: the singular value is the Bode magnitude.\n\
                 sigma(G, S1)\ntitle('Bode magnitude')\nylabel('Magnitude (dB)')\n",
                2,
            ),
            (PlotType::Nyquist, "nyquist(G, S1)\n", 2),
            (PlotType::Nichols, "nichols(G, S1)\nngrid\n", 2),
            (PlotType::Sigma, "sigma(G, S1)\n", 2),
            (PlotType::PoleZero, "pzmap(G, S1)\n", 2),
            (PlotType::IoPoleZero, "pzmap(G, S1)\ntitle('I/O pole-zero map')\n", 2),
            (PlotType::Lsim, "lsim(G, S1, u, t)\n", 2),
        ] {
            single(&mut host, kind);
            analyze(&mut host, &ws).unwrap_or_else(|e| panic!("{kind:?}: {e}"));
            let script = host.app.script(&ws).unwrap();
            assert!(script.contains(&format!("figure\n{call}")), "{kind:?}\n{script}");
            assert_eq!(analysis(&host).tiles[0].kind, kind);
            assert_eq!(analysis(&host).tiles[0].curves.len(), systems, "{kind:?}");
            run_script(&script);
        }
        // Only non-default options: a final time, a frequency range in Hz,
        // a sigma type.
        host.app.time_auto = false;
        host.app.final_time = "5".into();
        host.app.freq_auto = false;
        host.app.hz = true;
        host.app.wmin = "0.1".into();
        host.app.wmax = "10".into();
        host.app.plots = [
            PlotType::Step,
            PlotType::Bode,
            PlotType::Sigma,
            PlotType::Nichols,
        ]
        .map(apps::linear_system_analyzer::PlotSpec::new)
        .to_vec();
        host.app.plots[2].sigma_type = 3;
        host.app.count = 4;
        analyze(&mut host, &ws).unwrap();
        let script = host.app.script(&ws).unwrap();
        let w = "logspace(log10(2*pi*0.1), log10(2*pi*10), 1000)";
        for line in [
            "% The app shows frequency in Hz; the plots below use rad/s.\n".to_owned(),
            "subplot(2, 2, 1)\nstep(G, S1, 5)\n".to_owned(),
            format!("subplot(2, 2, 2)\nbode(G, S1, {w})\n"),
            format!("subplot(2, 2, 3)\nsigma(G, S1, {w}, 3)\n"),
            format!("subplot(2, 2, 4)\nnichols(G, S1, {w})\n"),
        ] {
            assert!(script.contains(&line), "{line}\n{script}");
        }
        let engine = run_script(&script);
        // Step, two Bode panels, sigma, Nichols: Bode stays in its cell.
        assert_eq!(engine.figure.as_ref().unwrap().all_axes().count(), 5);
        // Three plots: the third spans the lower row.
        host.app.count = 3;
        analyze(&mut host, &ws).unwrap();
        let script = host.app.script(&ws).unwrap();
        assert!(script.contains("subplot(2, 2, [3 4])\nsigma("), "{script}");
        // The default layout: Step and Bode side by side, three axes.
        let mut host = host_default(&ws);
        analyze(&mut host, &ws).unwrap();
        let script = host.app.script(&ws).unwrap();
        assert!(
            script.contains("figure\nsubplot(1, 2, 1)\nstep(D)\nsubplot(1, 2, 2)\nbode(D)\n"),
            "{script}"
        );
        let engine = run_script(&script);
        assert_eq!(engine.figure.as_ref().unwrap().all_axes().count(), 3);
    }
    fn host_default(ws: &Snapshot) -> Lsa {
        let mut host = Host::new(LinearSystemAnalyzer::default());
        host.app.prepare(ws);
        host
    }

    #[test]
    fn linear_system_analyzer_mimo_io_pair_selection() {
        let setup = "P = ss([-1 0; 0 -2], [1 0; 1 1], eye(2), zeros(2));";
        let ws = workspace(setup);
        let mut host = host(&ws, &["P"]);
        assert_eq!(host.app.systems.len(), 1, "the first model is ticked");
        single(&mut host, PlotType::Step);
        analyze(&mut host, &ws).unwrap();
        let script = host.app.script(&ws).unwrap();
        assert!(script.contains("figure\nstep(P)\n"), "{script}");
        assert!(!script.contains("stepinfo"), "MIMO characteristics need one pair");
        let Curves::Time { y: all, traces, .. } = analysis(&host).tiles[0].curves[0].1.clone()
        else {
            panic!("step")
        };
        assert_eq!(traces, 4);
        let mut engine = matrust::Engine::default();
        engine.execute(setup).unwrap();
        assert_eq!(values(&mut engine, "step(P)"), all);
        // Output 2, input 1: a channel variable, then the same calls.
        host.app.io = Some((2, 1));
        analyze(&mut host, &ws).unwrap();
        let script = host.app.script(&ws).unwrap();
        assert!(
            script.contains("P_2_1 = ss(P.A, P.B(:, 1), P.C(2, :), P.D(2, 1), P.Ts);\n"),
            "{script}"
        );
        assert!(script.contains("figure\nstep(P_2_1)\n"), "{script}");
        assert!(script.contains("\nstepinfo(P_2_1)\n"), "{script}");
        let Curves::Time { y, t, traces, .. } = &analysis(&host).tiles[0].curves[0].1 else {
            panic!("step")
        };
        assert_eq!(*traces, 1);
        // The channel's response is column 2 (output 2, input 1) of P's.
        close(y, &all[t.len()..2 * t.len()]);
        let channel = system(&host, "P (output 2, input 1)");
        assert!(channel.stepinfo.is_some() && channel.shape.siso());
        assert!(host.app.exported(&ws).contains(&"P_2_1".to_owned()));
        let mut engine = matrust::Engine::default();
        engine.execute(setup).unwrap();
        engine.execute(&script).unwrap();
        close(&values(&mut engine, "y"), y);
        // A pair the model does not have.
        host.app.io = Some((3, 1));
        host.app.prepare(&ws);
        let error = host.app.script(&ws).unwrap_err();
        assert!(error.contains("P has 2 outputs and 2 inputs"), "{error}");
        // All pairs again: Bode magnitude and I/O pole-zero draw a panel per channel.
        host.app.io = None;
        single(&mut host, PlotType::BodeMagnitude);
        analyze(&mut host, &ws).unwrap();
        let script = host.app.script(&ws).unwrap();
        assert!(
            script.contains(
                "subplot(2, 2, 3)\nsigma(ss(P.A, P.B(:, 1), P.C(2, :), P.D(2, 1), P.Ts))\n\
                 title('Bode magnitude: output 2, input 1')\n"
            ),
            "{script}"
        );
        let mut engine = matrust::Engine::default();
        engine.execute(setup).unwrap();
        engine.execute(&script).unwrap();
        assert_eq!(engine.figure.as_ref().unwrap().all_axes().count(), 4);
        single(&mut host, PlotType::IoPoleZero);
        analyze(&mut host, &ws).unwrap();
        let Curves::PoleZero { channels } = &analysis(&host).tiles[0].curves[0].1 else {
            panic!("pzmap")
        };
        assert_eq!(channels.len(), 4);
        // pole() of an ss channel is eig(A); the zero cancels the mode the
        // channel does not see: 1/(s+1) has a zero at -2, 1/(s+2) at -1,
        // and the zero channel (1, 2) has none.
        assert!(channels.iter().all(|[p, _]| p.len() == 2), "{channels:?}");
        let zeros: Vec<Vec<[f64; 2]>> = channels.iter().map(|[_, z]| z.clone()).collect();
        assert_eq!(
            zeros,
            [vec![[-2., 0.]], vec![], vec![[-1., 0.]], vec![[-1., 0.]]]
        );
        // A plain pole-zero map of a MIMO model asks for a pair.
        single(&mut host, PlotType::PoleZero);
        host.app.prepare(&ws);
        let error = host.app.script(&ws).unwrap_err();
        assert!(error.contains("choose one I/O pair"), "{error}");
    }

    #[test]
    fn linear_system_analyzer_expression_system_is_defined_shown_and_exported() {
        let setup = "G = tf(1, [1 1]); C = tf([2 1], [1 0]);";
        let ws = workspace(setup);
        let mut host = host(&ws, &[]);
        host.app.expression = "feedback(G*C, 1)".into();
        host.app.add_expression(&ws).unwrap();
        assert!(host.app.expression.is_empty());
        let sys = host.app.systems.last().unwrap().clone();
        assert_eq!((sys.name.as_str(), sys.shown), ("sys1", true));
        single(&mut host, PlotType::Step);
        analyze(&mut host, &ws).unwrap();
        assert_eq!(host.app.inputs(), ["G", "C"]);
        let script = host.app.script(&ws).unwrap();
        assert!(script.contains("\nsys1 = feedback(G*C, 1);\nfigure\nstep(sys1)\n"), "{script}");
        // feedback(G*C, 1) = (2s + 1)/(s^2 + 3s + 1): DC gain 1, stable.
        let s = system(&host, "sys1");
        assert!((s.dc[0] - 1.).abs() < 1e-12 && s.stable, "{s:?}");
        assert_eq!(host.app.exported(&ws), ["sys1", "y", "t"]);
        // Export: the same script, all or nothing, in the workspace.
        let Some(toolkits::Action::ExportScript(export)) = host.export(&ws) else {
            panic!("export action")
        };
        let mut engine = matrust::Engine::default();
        engine.execute(setup).unwrap();
        engine.execute(&export).unwrap();
        assert_eq!(matrust::engine::lti_class(&engine.array("sys1").unwrap()), Some("tf"));
        assert!(engine.array("y").is_some() && engine.array("t").is_some());
        // Typed expressions: one expression, no statements or comments.
        for bad in ["G; clear all", "x = G", "G % note", "feedback(G, 1", "G, C", "G ..."] {
            assert!(check_expression(bad).is_err(), "{bad}");
        }
        for good in ["feedback(G*C, 1)", "[G; C]", "G == C", "series(G, C)'", "tf('s')"] {
            assert!(check_expression(good).is_ok(), "{good}");
        }
        assert_eq!(
            identifiers("feedback(G*C, 1e-3) + s.Ts + 'G2' + 2i"),
            ["feedback", "G", "C", "s"]
        );
        // Removing it leaves the workspace models.
        host.app.remove(host.app.systems.len() - 1);
        assert!(host.app.systems.iter().all(|s| s.expression.is_none()));
    }

    #[test]
    fn linear_system_analyzer_errors_name_the_problem_and_keep_the_last_result() {
        let setup = "G = tf(1, [1 0.6 1]); N = ss(-1, [1 1], 1, [0 0]);";
        let ws = workspace(setup);
        let mut host = host(&ws, &["G"]);
        single(&mut host, PlotType::Step);
        analyze(&mut host, &ws).unwrap();
        let good = analysis(&host).systems[0].poles.clone();
        // An expression that does not parse names itself.
        host.app.expression = "feedback(G*, 1)".into();
        host.app.add_expression(&ws).unwrap();
        let error = analyze(&mut host, &ws).unwrap_err();
        assert!(error.contains("sys1 = feedback(G*, 1)"), "{error}");
        assert!(host.app.system_error("sys1").is_some());
        assert_eq!(analysis(&host).systems[0].poles, good, "the plots keep the last result");
        host.app.remove(host.app.systems.len() - 1);
        // An expression that is not a model.
        host.app.expression = "[1 2 3]".into();
        host.app.add_expression(&ws).unwrap();
        let error = analyze(&mut host, &ws).unwrap_err();
        assert!(error.contains("is a double, not a tf, zpk or ss model"), "{error}");
        host.app.remove(host.app.systems.len() - 1);
        // A typed statement never reaches the list.
        host.app.expression = "G; clear all".into();
        assert!(host.app.add_expression(&ws).is_err());
        assert!(host.app.expression_error.is_some());
        assert_eq!(host.app.systems.len(), 2);
        // sigma's inverse types need a square model: the engine's message.
        show(&mut host, &["N"]);
        single(&mut host, PlotType::Sigma);
        host.app.plots[0].sigma_type = 1;
        let error = analyze(&mut host, &ws).unwrap_err();
        assert!(error.contains("require a square system"), "{error}");
        assert_eq!(analysis(&host).systems[0].poles, good);
        // Nothing shown, and a bad final time.
        show(&mut host, &[]);
        host.app.prepare(&ws);
        assert!(host.app.script(&ws).unwrap_err().contains("tick a system"));
        show(&mut host, &["G"]);
        host.app.plots[0].kind = PlotType::Step;
        host.app.time_auto = false;
        host.app.final_time = "-1".into();
        let error = host.app.script(&ws).unwrap_err();
        assert!(error.contains("final time must be a positive number"), "{error}");
        host.app.time_auto = true;
        // Through the threaded runner, the status line carries the message.
        show(&mut host, &["N"]);
        host.app.plots[0].kind = PlotType::Sigma;
        let ctx = egui::Context::default();
        let deadline = Instant::now() + Duration::from_secs(30);
        loop {
            host.refresh(&ctx, &ws);
            // The systems query runs from the Inputs column.
            if let Some((request, names)) = host.app.shape_request_for(&ws) {
                let key = request.key;
                let result = apps::run(&mut matrust::Engine::default(), request);
                host.app.receive_shapes(key, &names, &result);
            }
            if !host.running() && host.status().0.contains("require a square system") {
                break;
            }
            assert!(Instant::now() < deadline, "{:?}", host.status());
            std::thread::sleep(Duration::from_millis(2));
        }
        let (text, _) = host.status();
        assert!(text.contains("require a square system"), "{text}");
        assert_eq!(analysis(&host).systems[0].poles, good);
    }

    fn open_lsa(
        app: &mut Workspace,
        ctx: &egui::Context,
        size: egui::Vec2,
        plots: usize,
    ) -> egui::FullOutput {
        app.toolkits
            .open_tool(Tool::App(AppKind::LinearSystemAnalyzer));
        let deadline = Instant::now() + Duration::from_secs(60);
        loop {
            let output = frame_at(app, ctx, size, vec![]);
            let host = &app.toolkits.apps.lsa;
            if host.app.current.as_ref().is_some_and(|a| a.tiles.len() == plots) && !host.running() {
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
    fn linear_system_analyzer_tab_renders_four_plots() {
        let size = egui::vec2(1440., 900.);
        let ctx = egui::Context::default();
        let mut app = Workspace::with_context(ctx.clone(), true);
        app.run("G = tf(1, [1 0.6 1]); H = zpk([], [-1 -2], 2);".into());
        finish(&mut app);
        open_lsa(&mut app, &ctx, size, 2);
        assert_eq!(
            app.toolkits.selected_tool(),
            Some(Tool::App(AppKind::LinearSystemAnalyzer))
        );
        // The layout picker: four plots (Step, Bode, Impulse, Pole-zero).
        click_label_at(&mut app, &ctx, size, "4 plots");
        app.toolkits.apps.lsa.app.systems[1].shown = true;
        let output = open_lsa(&mut app, &ctx, size, 4);
        for label in [
            "INPUTS",
            "PLOTS",
            "OUTPUT",
            "Plot 1",
            "Plot 4",
            "Step",
            "Bode",
            "Impulse",
            "Pole-zero",
            "Systems",
            "Add expression",
            "Frequency range",
            "Poles",
            "Damping",
            "Export to workspace",
            "Generate script",
        ] {
            assert!(painted(&output, label), "{label} not painted");
        }
        let host = &app.toolkits.apps.lsa;
        assert_eq!(analysis(host).systems.len(), 2);
        let (text, status) = app
            .toolkits
            .app_status(Tool::App(AppKind::LinearSystemAnalyzer), false)
            .unwrap();
        assert_eq!(status, toolkits::AppStatus::Success, "{text}");
        assert!(text.starts_with("G, H · Step, Bode, Impulse, Pole-zero"), "{text}");
        // Characteristics markers render with their values.
        app.toolkits.apps.lsa.app.plots[0].set(Char::Peak, true);
        let output = frame_at(&mut app, &ctx, size, vec![]);
        let peak = system(&app.toolkits.apps.lsa, "G").stepinfo.unwrap().peak;
        let label = format!("  peak {} at", apps::short(peak));
        assert!(
            output.shapes.iter().any(|s| {
                let mut found = false;
                fn visit(shape: &egui::epaint::Shape, prefix: &str, found: &mut bool) {
                    match shape {
                        egui::epaint::Shape::Text(t) => *found |= t.galley.text().starts_with(prefix),
                        egui::epaint::Shape::Vec(v) => v.iter().for_each(|s| visit(s, prefix, found)),
                        _ => {}
                    }
                }
                visit(&s.shape, &label, &mut found);
                found
            }),
            "{label}"
        );
        // A narrow host stacks OUTPUT under the plots and still renders.
        let output = frame_at(&mut app, &ctx, egui::vec2(900., 700.), vec![]);
        assert!(painted(&output, "OUTPUT"));
        app.close_tool_tab(Tool::App(AppKind::LinearSystemAnalyzer));
        assert!(app.toolkits.tabs().next().is_none());
    }

    #[test]
    fn linear_system_analyzer_export_names_do_not_clobber() {
        let size = egui::vec2(1440., 900.);
        let ctx = egui::Context::default();
        let mut app = Workspace::with_context(ctx.clone(), true);
        app.run("G = tf(1, [1 0.6 1]); y = 7; t = 1; sys1 = 3;".into());
        finish(&mut app);
        open_lsa(&mut app, &ctx, size, 2);
        {
            let apps = &mut app.toolkits.apps;
            apps.lsa.app.expression = "2*G".into();
            apps.lsa.app.add_expression(&apps.workspace).unwrap();
        }
        let name = app.toolkits.apps.lsa.app.systems.last().unwrap().name.clone();
        assert_eq!(name, "sys2", "sys1 is a workspace variable");
        let deadline = Instant::now() + Duration::from_secs(60);
        while app.toolkits.apps.lsa.app.current.as_ref().unwrap().systems.len() != 2
            || app.toolkits.apps.lsa.running()
        {
            frame_at(&mut app, &ctx, size, vec![]);
            assert!(Instant::now() < deadline, "{:?}", app.toolkits.apps.lsa.status());
            std::thread::sleep(Duration::from_millis(5));
        }
        let ws = &app.toolkits.apps.workspace;
        assert_eq!(app.toolkits.apps.lsa.app.exported(ws), ["sys2", "y1", "t1"]);
        click_label_at(&mut app, &ctx, size, "Export to workspace");
        finish(&mut app);
        frame_at(&mut app, &ctx, size, vec![]);
        for (name, value) in [("y", 7.), ("t", 1.), ("sys1", 3.)] {
            assert!(
                matches!(&app.variables[name], matrust::Array::Numeric(m) if m.data() == [value]),
                "{name} is kept"
            );
        }
        assert_eq!(matrust::engine::lti_class(&app.variables["sys2"]), Some("tf"));
        assert!(app.variables.contains_key("y1") && app.variables.contains_key("t1"));
        assert!(app.toolkits.apps.lsa.message.contains("y1"), "{}", app.toolkits.apps.lsa.message);
        // The exported system is a workspace model now: the expression
        // system moves to the next free name and the next outputs too (the
        // export's figure took the main area: back to the app's tab).
        app.select_tool_tab(Tool::App(AppKind::LinearSystemAnalyzer));
        let deadline = Instant::now() + Duration::from_secs(60);
        loop {
            frame_at(&mut app, &ctx, size, vec![]);
            let ws = &app.toolkits.apps.workspace;
            let script = app.toolkits.apps.lsa.app.script(ws);
            if script.is_ok() {
                break;
            }
            assert!(
                Instant::now() < deadline,
                "{:?} {script:?} {:?}",
                app.toolkits.apps.lsa.status(),
                app.toolkits.apps.lsa.app.systems
            );
            std::thread::sleep(Duration::from_millis(5));
        }
        let lsa = &app.toolkits.apps.lsa.app;
        assert!(lsa.systems.iter().any(|s| s.name == "sys2" && s.expression.is_none()));
        assert!(lsa.systems.iter().any(|s| s.name == "sys3" && s.expression.is_some()));
        let ws = &app.toolkits.apps.workspace;
        let script = lsa.script(ws).unwrap();
        assert!(script.contains("sys3 = 2*G;") && script.contains("[y2, t2] = step(G);"), "{script}");
    }

    #[test]
    fn linear_system_analyzer_generate_script_saves_header_and_the_same_script() {
        let size = egui::vec2(1440., 900.);
        let ctx = egui::Context::default();
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("analysis.m");
        let mut app = Workspace::with_context(ctx.clone(), true);
        app.run("G = tf(1, [1 0.6 1]);".into());
        finish(&mut app);
        open_lsa(&mut app, &ctx, size, 2);
        let script = {
            let ws = &app.toolkits.apps.workspace;
            app.toolkits.apps.lsa.app.script(ws).unwrap()
        };
        app.toolkits.apps.test_save_path = Some(path.clone());
        click_label_at(&mut app, &ctx, size, "Generate script");
        finish(&mut app);
        frame_at(&mut app, &ctx, size, vec![]);
        let saved = std::fs::read_to_string(&path).unwrap();
        assert!(
            saved.starts_with("% Linear System Analyzer (MatRust app), generated "),
            "{saved}"
        );
        assert!(saved.ends_with(&script), "{saved}");
        assert!(
            app.toolkits.apps.lsa.message.starts_with("Saved"),
            "{}",
            app.toolkits.apps.lsa.message
        );
        let mut engine = matrust::Engine::default();
        engine.execute("G = tf(1, [1 0.6 1]);").unwrap();
        engine.execute(&saved).unwrap();
        assert!(engine.array("y").is_some());
    }
}
