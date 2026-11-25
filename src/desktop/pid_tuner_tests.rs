// PID Tuner app (apps/pid_tuner.rs): the script reproduces what the app
// shows, the sliders map to wc and the phase margin, workspace plants,
// errors, export without clobbering, the saved script and a rendered tab.
mod pid_tuner_tests {
    use super::*;
    use apps::pid_tuner::{DEFAULT_MARGIN, Design, FACTOR, Kind, MARGIN, PidTuner};
    use apps::{App, AppKind, Host, Snapshot};
    use toolkits::Tool;

    /// One private run, synchronously, delivered to the app.
    fn tune(host: &mut Host<PidTuner>, ws: &Snapshot) -> Result<(), String> {
        host.app.prepare(ws);
        let request = host.request(ws)?;
        let outputs = apps::run(&mut matrust::Engine::default(), request)?;
        host.app.receive(&outputs)
    }
    fn design(host: &Host<PidTuner>) -> &Design {
        host.app.current.as_ref().expect("a design")
    }
    fn number(engine: &matrust::Engine, name: &str) -> f64 {
        match engine.array(name) {
            Some(matrust::Array::Numeric(m)) => m.data()[0],
            other => panic!("{name}: {other:?}"),
        }
    }
    /// Runs `script` in a fresh Engine (with `setup` first) and returns C's gains.
    fn gains_of(setup: &str, script: &str, controller: &str) -> [f64; 4] {
        let mut engine = matrust::Engine::default();
        engine.execute(setup).unwrap();
        engine
            .execute(script)
            .unwrap_or_else(|e| panic!("{script}\n{e}"));
        engine
            .execute(&format!(
                "kp_={controller}.Kp; ki_={controller}.Ki; kd_={controller}.Kd; tf_={controller}.Tf;"
            ))
            .unwrap();
        ["kp_", "ki_", "kd_", "tf_"].map(|n| number(&engine, n))
    }

    #[test]
    fn pid_tuner_generated_script_reproduces_the_gains_shown() {
        let ws = Snapshot::default();
        let mut host = Host::new(PidTuner::default());
        for kind in [Kind::P, Kind::PI, Kind::PID, Kind::PIDF, Kind::I, Kind::PDF] {
            host.app.kind = kind;
            tune(&mut host, &ws).unwrap_or_else(|e| panic!("{kind:?}: {e}"));
            let shown = design(&host).gains;
            let script = host.app.script(&ws).unwrap();
            assert!(script.contains(&format!("'{}'", kind.name())), "{script}");
            assert_eq!(gains_of("", &script, "C"), shown, "{kind:?}");
            // The Output block shows those gains.
            let rows = host.app.results();
            assert_eq!(rows[0], ("Kp", apps::short(shown[0])));
            assert_eq!(rows[2], ("Kd", apps::short(shown[2])));
            assert!(design(&host).stable);
        }
        // P and PI have no derivative; PID has all three.
        host.app.kind = Kind::PI;
        tune(&mut host, &ws).unwrap();
        let [kp, ki, kd, tf] = design(&host).gains;
        assert!(kp > 0. && ki > 0. && kd == 0. && tf == 0.);
        // A slider-moved design reproduces too, explicit wc and margin included.
        host.app.kind = Kind::PID;
        tune(&mut host, &ws).unwrap();
        host.app.factor = 1.7;
        host.app.margin = 50.;
        tune(&mut host, &ws).unwrap();
        let script = host.app.script(&ws).unwrap();
        assert!(
            script.contains("pidtuneOptions('PhaseMargin', 50)"),
            "{script}"
        );
        assert_eq!(gains_of("", &script, "C"), design(&host).gains);
    }

    #[test]
    fn pid_tuner_sliders_map_to_crossover_and_phase_margin() {
        assert_eq!((*FACTOR.start(), *FACTOR.end()), (0.1, 10.));
        assert_eq!((*MARGIN.start(), *MARGIN.end()), (30., 90.));
        let ws = Snapshot::default();
        let mut host = Host::new(PidTuner::default());
        tune(&mut host, &ws).unwrap();
        // Initial PID design for 1/(s+1)^3: the plant phase is -150 degrees
        // at wc0, 3*atan(wc0) = 150, wc0 = tan(50).
        let wc0 = host.app.wc0.unwrap();
        assert!((wc0 - 50_f64.to_radians().tan()).abs() < 1e-9);
        assert!((design(&host).phase_margin - 60.).abs() < 1e-3);
        // Response time: wc = wc0 * factor.
        host.app.factor = 2.;
        assert_eq!(host.app.crossover(), Some(2. * wc0));
        let script = host.app.script(&ws).unwrap();
        assert!(
            script.contains(&format!("pidtune(G, 'PID', {});", apps::number(2. * wc0))),
            "{script}"
        );
        tune(&mut host, &ws).unwrap();
        assert_eq!(design(&host).wc, 2. * wc0);
        assert_eq!(host.app.wc0, Some(wc0), "the base stays the initial design");
        // Robustness: the target phase margin at that crossover. At 2*wc0
        // the plant lags 201 degrees, so a PID reaches margins below 68.
        host.app.margin = 45.;
        tune(&mut host, &ws).unwrap();
        assert!((design(&host).phase_margin - 45.).abs() < 1e-3);
        assert!((design(&host).gain_crossover - 2. * wc0).abs() < 1e-6 * wc0);
        // An unreachable target is pidtune's error; the design stays.
        host.app.margin = 85.;
        let error = tune(&mut host, &ws).unwrap_err();
        assert!(error.contains("85 degree phase margin"), "{error}");
        host.app.margin = 45.;
        // Reset: the initial call again; the replaced design stays to compare.
        let moved = design(&host).clone();
        host.app.reset();
        assert_eq!((host.app.factor, host.app.margin), (1., DEFAULT_MARGIN));
        assert!(
            host.app
                .script(&ws)
                .unwrap()
                .contains("C = pidtune(G, 'PID');")
        );
        assert!(
            host.app
                .previous
                .as_ref()
                .is_some_and(|p| p.same_as(&moved))
        );
        // P has one gain: only the crossover moves, no margin option.
        host.app.kind = Kind::P;
        host.app.prepare(&ws);
        assert_eq!(host.app.wc0, None, "a new type starts a new initial design");
        tune(&mut host, &ws).unwrap();
        host.app.factor = 0.5;
        host.app.margin = 45.;
        let script = host.app.script(&ws).unwrap();
        assert!(!script.contains("pidtuneOptions"), "{script}");
        // A new plant resets the sliders.
        host.app.denominator = "1 2 1".into();
        host.app.prepare(&ws);
        assert_eq!((host.app.factor, host.app.margin), (1., DEFAULT_MARGIN));
        assert!(host.app.previous.is_none());
    }

    #[test]
    fn pid_tuner_plant_from_a_workspace_tf_variable() {
        let mut engine = matrust::Engine::default();
        engine
            .execute("plant = tf(2, [1 4 5 2]); C = 7; note = 'x';")
            .unwrap();
        let mut ws = Snapshot::default();
        ws.sync(&engine.workspace());
        let mut host = Host::new(PidTuner::default());
        host.app.use_variable = true;
        host.app.variable = "plant".into();
        assert_eq!(host.app.inputs(), ["plant"]);
        let script = host.app.script(&ws).unwrap();
        // C is taken: the controller becomes C1; the plant is not redefined.
        assert_eq!(
            script,
            "% The plant is the workspace variable plant.\n\
             C1 = pidtune(plant, 'PID');\nstep(feedback(C1*plant, 1))\n"
        );
        tune(&mut host, &ws).unwrap();
        let shown = design(&host).gains;
        assert_eq!(
            gains_of("plant = tf(2, [1 4 5 2]); C = 7;", &script, "C1"),
            shown
        );
        assert_eq!(host.app.exported(&ws), ["C1"]);
        // zpk and ss plants work the same way (PI: a first-order plant never
        // lags the 150 degrees a PID design starts from).
        engine
            .execute("pz = zpk([], [-1 -2 -3], 6); ps = ss(-2, 1, 3, 0);")
            .unwrap();
        ws.sync(&engine.workspace());
        host.app.kind = Kind::PI;
        for name in ["pz", "ps"] {
            host.app.variable = name.into();
            tune(&mut host, &ws).unwrap_or_else(|e| panic!("{name}: {e}"));
        }
    }

    #[test]
    fn pid_tuner_errors_show_the_engine_message_and_keep_the_last_design() {
        let mut engine = matrust::Engine::default();
        engine.execute("x = 5;").unwrap();
        let mut ws = Snapshot::default();
        ws.sync(&engine.workspace());
        let mut host = Host::new(PidTuner::default());
        tune(&mut host, &ws).unwrap();
        let good = design(&host).clone();
        // A variable that is not an LTI model: the engine's error.
        host.app.use_variable = true;
        host.app.variable = "x".into();
        let error = tune(&mut host, &ws).unwrap_err();
        assert!(error.contains("LTI"), "{error}");
        assert!(
            design(&host).same_as(&good),
            "the plots keep the last good design"
        );
        // A missing variable never reaches the engine.
        host.app.variable = "gone".into();
        let error = host.request(&ws).err().unwrap();
        assert!(error.contains("gone does not exist"), "{error}");
        // An unstable result from pidtune.
        host.app.use_variable = false;
        host.app.denominator = "1 0 -1".into();
        host.app.kind = Kind::P;
        let error = tune(&mut host, &ws).unwrap_err();
        assert!(error.contains("unstable"), "{error}");
        // Typed coefficients that are not numbers.
        host.app.denominator = "1 x".into();
        let error = host.app.script(&ws).unwrap_err();
        assert!(error.starts_with("Denominator"), "{error}");
        // Through the threaded runner, the status line carries the message.
        let ctx = egui::Context::default();
        host.app.denominator = "1 0 -1".into();
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
        assert!(text.contains("unstable"), "{text}");
        assert!(design(&host).same_as(&good));
    }

    fn open_pid_tuner(
        app: &mut Workspace,
        ctx: &egui::Context,
        size: egui::Vec2,
    ) -> egui::FullOutput {
        app.toolkits.open_tool(Tool::App(AppKind::PidTuner));
        let deadline = Instant::now() + Duration::from_secs(30);
        loop {
            let output = frame_at(app, ctx, size, vec![]);
            let host = &app.toolkits.apps.pid;
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
    fn pid_tuner_tab_renders_three_areas_in_the_main_window() {
        let size = egui::vec2(1440., 900.);
        let ctx = egui::Context::default();
        let mut app = Workspace::with_context(ctx.clone(), true);
        let output = open_pid_tuner(&mut app, &ctx, size);
        assert_eq!(
            app.toolkits.selected_tool(),
            Some(Tool::App(AppKind::PidTuner))
        );
        for label in [
            "INPUTS",
            "RESPONSE",
            "OUTPUT",
            "Step (reference)",
            "Bode (open loop)",
            "Export to workspace",
            "Generate script",
            "Response time",
            "Robustness",
            "Kp",
        ] {
            assert!(painted(&output, label), "{label} not painted");
        }
        let kp = apps::short(design(&app.toolkits.apps.pid).gains[0]);
        assert!(painted(&output, &kp), "Kp value {kp}");
        // The status line names the design.
        let (text, status) = app
            .toolkits
            .app_status(Tool::App(AppKind::PidTuner), false)
            .unwrap();
        assert_eq!(status, toolkits::AppStatus::Success);
        assert!(text.starts_with("PID controller"), "{text}");
        // Every main view renders.
        for view in [
            "Bode (open loop)",
            "Controller effort",
            "Step (disturbance)",
        ] {
            click_label_at(&mut app, &ctx, size, view);
            let output = frame_at(&mut app, &ctx, size, vec![]);
            assert!(painted(&output, view));
        }
        assert_eq!(
            app.toolkits.apps.pid.app.view,
            apps::pid_tuner::View::Disturbance
        );
        // A narrow host stacks OUTPUT under the main view and still renders.
        let narrow = egui::vec2(900., 700.);
        let output = frame_at(&mut app, &ctx, narrow, vec![]);
        assert!(painted(&output, "OUTPUT"));
        // Closing the tab keeps the inputs for the next open.
        app.close_tool_tab(Tool::App(AppKind::PidTuner));
        assert!(app.toolkits.tabs().next().is_none());
        assert!(app.toolkits.apps.pid.app.current.is_some());
    }

    #[test]
    fn pid_tuner_export_creates_c_without_clobbering() {
        let size = egui::vec2(1440., 900.);
        let ctx = egui::Context::default();
        let mut app = Workspace::with_context(ctx.clone(), true);
        app.run("C = 7;".into());
        finish(&mut app);
        open_pid_tuner(&mut app, &ctx, size);
        let shown = design(&app.toolkits.apps.pid).gains;
        click_label_at(&mut app, &ctx, size, "Export to workspace");
        finish(&mut app);
        frame_at(&mut app, &ctx, size, vec![]);
        assert!(
            matches!(&app.variables["C"], matrust::Array::Numeric(m) if m.data() == [7.]),
            "C is kept"
        );
        assert_eq!(
            matrust::engine::lti_class(&app.variables["C1"]),
            Some("pid"),
            "{}",
            app.status
        );
        assert_eq!(matrust::engine::lti_class(&app.variables["G"]), Some("tf"));
        assert!(app.toolkits.apps.pid.message.contains("C1"));
        // The exported controller has the gains the app shows.
        let mut engine = matrust::Engine::default();
        engine.merge_workspace(app.variables.clone()).unwrap();
        engine
            .execute("kp_=C1.Kp; ki_=C1.Ki; kd_=C1.Kd; tf_=C1.Tf;")
            .unwrap();
        assert_eq!(
            ["kp_", "ki_", "kd_", "tf_"].map(|n| number(&engine, n)),
            shown
        );
        // C and C1 are taken now: the next default name is C2, the plant G1.
        frame_at(&mut app, &ctx, size, vec![]);
        let ws = &app.toolkits.apps.workspace;
        let script = app.toolkits.apps.pid.app.script(ws).unwrap();
        assert!(
            script.starts_with("G1 = tf(") && script.contains("C2 = pidtune(G1"),
            "{script}"
        );
    }

    #[test]
    fn pid_tuner_generate_script_saves_header_and_the_same_script() {
        let size = egui::vec2(1440., 900.);
        let ctx = egui::Context::default();
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("tuned.m");
        let mut app = Workspace::with_context(ctx.clone(), true);
        open_pid_tuner(&mut app, &ctx, size);
        let script = {
            let ws = &app.toolkits.apps.workspace;
            app.toolkits.apps.pid.app.script(ws).unwrap()
        };
        app.toolkits.apps.test_save_path = Some(path.clone());
        click_label_at(&mut app, &ctx, size, "Generate script");
        finish(&mut app);
        frame_at(&mut app, &ctx, size, vec![]);
        let saved = std::fs::read_to_string(&path).unwrap();
        assert!(
            saved.starts_with("% PID Tuner (MatRust app), generated "),
            "{saved}"
        );
        assert!(saved.ends_with(&script), "{saved}");
        let header_lines = saved.lines().take_while(|l| l.starts_with('%')).count();
        assert_eq!(
            saved
                .lines()
                .skip(header_lines)
                .collect::<Vec<_>>()
                .join("\n")
                + "\n",
            script
        );
        assert!(
            app.toolkits.apps.pid.message.starts_with("Saved"),
            "{}",
            app.toolkits.apps.pid.message
        );
        // The Filter Designer's save state is untouched.
        assert!(app.toolkits.message.is_empty());
        // The saved file runs and reproduces the gains.
        let shown = design(&app.toolkits.apps.pid).gains;
        assert_eq!(gains_of("", &saved, "C"), shown);
    }
}
