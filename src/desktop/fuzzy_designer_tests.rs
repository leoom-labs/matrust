// App numbers are checked against independent engine evalfis/gensurf calls on
// the FIS built by the exported script. No MATLAB process or app-only oracle.
mod fuzzy_designer_tests {
    use super::*;
    use apps::{
        App, AppKind, Host, Snapshot,
        fuzzy_designer::{self, FuzzyDesigner, View},
    };
    use matrust::{Array, Engine, engine::fuzzy_app_hooks};

    fn settle(h: &mut Host<FuzzyDesigner>) {
        let ctx = egui::Context::default();
        let ws = Snapshot::default();
        let until = Instant::now() + Duration::from_secs(30);
        let mut idle = 0;
        while idle < 2 {
            h.refresh(&ctx, &ws);
            if h.running() {
                idle = 0;
            } else {
                idle += 1;
            }
            assert!(Instant::now() < until, "{}", h.status().0);
            std::thread::sleep(Duration::from_millis(2));
        }
    }
    fn good(h: &Host<FuzzyDesigner>) {
        assert_eq!(
            h.status().1,
            toolkits::AppStatus::Success,
            "{}",
            h.status().0
        );
    }
    fn nums(e: &Engine, name: &str) -> Vec<f64> {
        let Some(Array::Numeric(m)) = e.array(name) else {
            panic!("missing {name}")
        };
        m.data().to_vec()
    }
    fn close(a: &[f64], b: &[f64]) {
        assert_eq!(a.len(), b.len());
        for (a, b) in a.iter().zip(b) {
            assert!((a - b).abs() < 1e-12, "{a} != {b}");
        }
    }
    fn setup(sugeno: bool) -> Host<FuzzyDesigner> {
        let mut h = Host::new(FuzzyDesigner::default());
        h.app.import(fuzzy_designer::empty(sugeno)).unwrap();
        settle(&mut h);
        good(&h);
        for name in ["drive", "load"] {
            h.app.add_variable(false, name, "[0 1]").unwrap();
            settle(&mut h);
            good(&h);
        }
        h.app.add_variable(true, "command", "[0 1]").unwrap();
        settle(&mut h);
        good(&h);
        for i in 0..2 {
            h.app.output_variable = false;
            h.app.variable = i;
            for (name, p) in [("low", "[0 0 1]"), ("high", "[0 1 1]")] {
                h.app.add_member(name, "trimf", p, false).unwrap();
                settle(&mut h);
                good(&h);
            }
        }
        h.app.output_variable = true;
        h.app.variable = 0;
        if sugeno {
            h.app
                .add_member("low", "linear", "[0.2 0.3 0.1]", false)
                .unwrap();
            settle(&mut h);
            good(&h);
            h.app
                .add_member("high", "linear", "[0.4 -0.1 0.6]", false)
                .unwrap();
            settle(&mut h);
            good(&h);
        } else {
            for (name, p) in [("low", "[0 0 1]"), ("high", "[0 1 1]")] {
                h.app.add_member(name, "trimf", p, false).unwrap();
                settle(&mut h);
                good(&h);
            }
        }
        h.app
            .set_rule(
                "If drive is low and load is low then command is low (0.7)",
                None,
            )
            .unwrap();
        settle(&mut h);
        good(&h);
        h.app
            .set_rule(
                "If drive is high or load is high then command is high (1)",
                None,
            )
            .unwrap();
        settle(&mut h);
        good(&h);
        h
    }
    fn export(h: &mut Host<FuzzyDesigner>, e: &mut Engine) {
        let Some(toolkits::Action::ExportScript(code)) = h.export(&Snapshot::default()) else {
            panic!()
        };
        apps::execute_export(e, &code).unwrap();
    }
    #[test]
    fn fuzzy_designer_mamdani_and_sugeno_linear_replay_export_and_grid() {
        for sugeno in [false, true] {
            let mut h = setup(sugeno);
            h.app.input_values = vec![0.25, 0.75];
            settle(&mut h);
            good(&h);
            let mut e = Engine::default();
            export(&mut h, &mut e);
            let grid = "[0 0;0.25 0.75;0.5 0.5;0.75 0.25;1 1]";
            e.execute(&format!("expected=evalfis(fuzzySystem,{grid});"))
                .unwrap();
            let dir = tempfile::tempdir().unwrap();
            let path = dir.path().join("exported.fis");
            e.execute(&format!("writefis(fuzzySystem,'{}');", path.display()))
                .unwrap();
            let mut fresh = Engine::default();
            fresh
                .execute(&format!("exported=readfis('{}');", path.display()))
                .unwrap();
            let Some(toolkits::Action::SaveAppScript(AppKind::FuzzyLogicDesigner, code)) =
                h.generate(&Snapshot::default(), 0)
            else {
                panic!()
            };
            assert!(
                code.contains("addInput") && code.contains("addMF") && code.contains("addRule")
            );
            fresh.execute(&code).unwrap();
            fresh
                .execute(&format!(
                    "same=isequal(exported,fuzzySystem);actual=evalfis(fuzzySystem,{grid});"
                ))
                .unwrap();
            assert!(matches!(fresh.array("same"),Some(Array::Logical(m)) if m.data()==[1]));
            close(&nums(&fresh, "actual"), &nums(&e, "expected"));
            e.execute("[y,im,ro,ag,fr]=evalfis(fuzzySystem,[0.25 0.75]);")
                .unwrap();
            let r = h.app.current.as_ref().unwrap();
            close(&r.values, &nums(&e, "y"));
            close(&r.firing, &nums(&e, "fr"));
            close(&r.aggregate, &nums(&e, "ag"));
            assert_eq!(r.rules.len(), 2);
            assert!(r.rules[0].contains("If"));
        }
    }
    #[test]
    fn fuzzy_designer_fixture_open_export_and_roundtrip() {
        let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("tests/fixtures/apps/fuzzy/handmade.fis");
        let mut h = Host::new(FuzzyDesigner::default());
        h.app.open_file(path.to_str().unwrap()).unwrap();
        settle(&mut h);
        good(&h);
        assert_eq!(h.app.system.name, "workshop's controller");
        assert_eq!(h.app.system.rules.len(), 2);
        let mut e = Engine::default();
        export(&mut h, &mut e);
        e.execute(&format!(
            "original=readfis('{}');same=isequal(original,fuzzySystem);",
            path.display()
        ))
        .unwrap();
        assert!(matches!(e.array("same"),Some(Array::Logical(m)) if m.data()==[1]));
        assert_eq!(
            fuzzy_app_hooks::snapshot(&e, "original").unwrap(),
            h.app.system
        );
    }
    #[test]
    fn fuzzy_designer_invalid_fields_keep_last_good_and_rule_edit() {
        let mut h = setup(false);
        let original = h.app.system.clone();
        let y = h.app.current.as_ref().unwrap().values.clone();
        h.app.add_member("broken", "gaussmf", "[1]", false).unwrap();
        settle(&mut h);
        assert_eq!(h.status().1, toolkits::AppStatus::Error);
        assert_eq!(h.app.system, original);
        close(&h.app.current.as_ref().unwrap().values, &y);
        h.app
            .set_rule("If drive is missing then command is high", Some(0))
            .unwrap();
        settle(&mut h);
        assert_eq!(h.status().1, toolkits::AppStatus::Error);
        assert_eq!(h.app.system, original);
        assert!(
            h.app
                .add_member("x", "trimf", "[0 1];evil=1", false)
                .is_err()
        );
        assert!(h.app.error.is_some());
        assert!(h.app.add_variable(false, "bad", "[0 Inf]").is_err());
        h.app
            .set_rule("If drive is not low then command is high (0.5)", Some(0))
            .unwrap();
        settle(&mut h);
        good(&h);
        assert_eq!(h.app.system.rules[0].antecedent, [-1., 0.]);
        assert_eq!(h.app.system.rules[0].weight, 0.5);
    }
    #[test]
    fn fuzzy_designer_remove_variable_matches_engine_and_adjusts_linear() {
        for sugeno in [false, true] {
            let mut h = setup(sugeno);
            let mut e = Engine::default();
            export(&mut h, &mut e);
            e.execute("fuzzySystem=removeInput(fuzzySystem,'drive');")
                .unwrap();
            let expected = fuzzy_app_hooks::snapshot(&e, "fuzzySystem").unwrap();
            h.app.remove_variable(false, 0).unwrap();
            settle(&mut h);
            good(&h);
            assert_eq!(h.app.system, expected);
            assert_eq!(h.app.input_values.len(), 1);
        }
    }
    #[test]
    fn fuzzy_designer_surface_matches_gensurf_two_inputs_and_curve() {
        let mut h = setup(false);
        h.app.view = View::Surface;
        h.app.grid = 9;
        h.app.input_values = vec![0.2, 0.7];
        settle(&mut h);
        good(&h);
        let mut e = Engine::default();
        export(&mut h, &mut e);
        e.execute("[x,y,z]=gensurf(fuzzySystem,gensurfOptions('NumGridPoints',9,'ReferenceInputs',[0.2 0.7]));").unwrap();
        let r = h.app.current.as_ref().unwrap();
        close(&r.surface_x, &nums(&e, "x"));
        close(&r.surface_y, &nums(&e, "y"));
        close(&r.surface_z, &nums(&e, "z"));
        h.app.remove_variable(false, 1).unwrap();
        settle(&mut h);
        good(&h);
        export(&mut h, &mut e);
        e.execute("[x,y,z]=gensurf(fuzzySystem,gensurfOptions('NumGridPoints',9));")
            .unwrap();
        let r = h.app.current.as_ref().unwrap();
        assert!(r.surface_y.is_empty());
        close(&r.surface_x, &nums(&e, "x"));
        close(&r.surface_z, &nums(&e, "z"));
    }
    #[test]
    fn fuzzy_designer_names_and_atomic_export_rollback() {
        let mut h = setup(false);
        for bad in [
            "x;evil=1",
            "a(1)",
            "end",
            "mrexp_old",
            "mrexp_had",
            "matrust_export_error",
            "mrexp_complete",
            "app_fuzzy_y",
        ] {
            h.app.name = bad.into();
            assert!(h.app.script(&Snapshot::default()).is_err(), "{bad}");
            assert!(h.export(&Snapshot::default()).is_none());
        }
        h.app.name = "fuzzySystem".into();
        let mut e = Engine::default();
        e.execute("fuzzySystem=17;").unwrap();
        let code = apps::all_or_nothing(
            "fuzzySystem=mamfis; error('forced failure');",
            &h.app.exported(&Snapshot::default()),
        );
        assert!(e.execute(&code).is_err());
        assert_eq!(nums(&e, "fuzzySystem"), [17.]);
        export(&mut h, &mut e);
        assert_eq!(
            fuzzy_app_hooks::snapshot(&e, "fuzzySystem").unwrap(),
            h.app.system
        );
    }
    #[test]
    fn fuzzy_designer_commands_open_desktop_and_workspace_picker() {
        let ctx = egui::Context::default();
        let mut app = Workspace::with_context(ctx, true);
        let dir = tempfile::tempdir().unwrap();
        let fixture = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("tests/fixtures/apps/fuzzy/handmade.fis");
        for cmd in [
            "fuzzyLogicDesigner".into(),
            "f=sugfis('NumInputs',2,'NumOutputs',1);fuzzyLogicDesigner(f)".into(),
            format!("fuzzyLogicDesigner('{}')", fixture.display()),
        ] {
            let error = Engine::default().execute(&cmd).unwrap_err();
            assert!(
                error.ends_with("fuzzyLogicDesigner requires the desktop"),
                "{cmd}: {error}"
            );
            app.submit(worker::Job::ScriptInFolder(cmd, dir.path().to_owned()));
            let until = Instant::now() + Duration::from_secs(20);
            while app.busy {
                app.receive();
                assert!(Instant::now() < until);
                std::thread::sleep(Duration::from_millis(2));
            }
            assert!(app.toolkits.apps.is_open(AppKind::FuzzyLogicDesigner));
            settle(&mut app.toolkits.apps.fuzzy);
            good(&app.toolkits.apps.fuzzy);
        }
        assert!(
            app.toolkits
                .apps
                .fuzzy
                .app
                .catalogue
                .iter()
                .any(|(name, s)| name == "f" && s.sugeno)
        );
        assert_eq!(
            app.toolkits.selected_tool(),
            Some(toolkits::Tool::App(AppKind::FuzzyLogicDesigner))
        );
        let tab = AppKind::FuzzyLogicDesigner.tab();
        assert_eq!(AppKind::from_tab(tab), Some(AppKind::FuzzyLogicDesigner));
        assert_eq!(
            serde_json::from_str::<matrust::session::ToolTab>(
                &serde_json::to_string(&tab).unwrap()
            )
            .unwrap(),
            tab
        );
    }
    fn frame(h: &mut Host<FuzzyDesigner>, width: f32) {
        let ctx = egui::Context::default();
        let ws = Snapshot::default();
        let out = ctx.run_ui(
            egui::RawInput {
                screen_rect: Some(egui::Rect::from_min_size(
                    egui::Pos2::ZERO,
                    egui::vec2(width, 900.),
                )),
                ..Default::default()
            },
            |ctx| {
                egui::CentralPanel::default()
                    .show(ctx, |ui| fuzzy_designer::show(h, ui, &ws, false, &mut None));
            },
        );
        assert!(!out.shapes.is_empty());
    }
    #[test]
    fn fuzzy_designer_headless_empty_27_rules_error_and_12_mfs() {
        let mut h = Host::new(FuzzyDesigner::default());
        settle(&mut h);
        good(&h);
        frame(&mut h, 1440.);
        frame(&mut h, 800.);
        let mut e = Engine::default();
        e.execute("f=mamfis('NumInputs',3,'NumOutputs',1);")
            .unwrap();
        h.app
            .import(fuzzy_app_hooks::snapshot(&e, "f").unwrap())
            .unwrap();
        settle(&mut h);
        good(&h);
        assert_eq!(h.app.system.rules.len(), 27);
        for v in [
            View::Memberships,
            View::Inference,
            View::Surface,
            View::System,
        ] {
            h.app.view = v;
            settle(&mut h);
            frame(&mut h, 1440.);
        }
        h.app.set_rule("nonsense", None).unwrap();
        settle(&mut h);
        frame(&mut h, 1440.);
        e.execute("f=mamfis('NumInputs',1,'NumInputMFs',12,'NumOutputs',1,'AddRules','none');")
            .unwrap();
        h.app
            .import(fuzzy_app_hooks::snapshot(&e, "f").unwrap())
            .unwrap();
        h.app.view = View::Memberships;
        settle(&mut h);
        good(&h);
        frame(&mut h, 1440.);
        assert_eq!(h.app.current.as_ref().unwrap().mf_y.len(), 12 * 181);
    }
    #[test]
    fn fuzzy_designer_save_fis_and_warning_diagnostics() {
        let mut h = setup(false);
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("controller's design.fis");
        let request = h.app.save_request(&path, 1).unwrap();
        apps::run(&mut Engine::default(), request).unwrap();
        let mut reopened = Host::new(FuzzyDesigner::default());
        reopened.app.open_file(path.to_str().unwrap()).unwrap();
        settle(&mut reopened);
        good(&reopened);
        assert_eq!(reopened.app.system, h.app.system);
        h.app.input_values = vec![2., 2.];
        settle(&mut h);
        good(&h);
        assert!(
            h.app
                .current
                .as_ref()
                .unwrap()
                .warning
                .to_lowercase()
                .contains("range")
        );
        let mut e = Engine::default();
        e.execute("f=mamfis('NumInputs',1,'NumOutputs',1,'AddRules','none');")
            .unwrap();
        h.app
            .import(fuzzy_app_hooks::snapshot(&e, "f").unwrap())
            .unwrap();
        settle(&mut h);
        good(&h);
        assert!(
            h.app
                .current
                .as_ref()
                .unwrap()
                .warning
                .to_lowercase()
                .contains("no rule")
        );
    }
    #[test]
    fn fuzzy_designer_large_surface_is_background_cancellable_and_keeps_view() {
        let mut h = setup(false);
        let old = h.app.current.as_ref().unwrap().values.clone();
        h.app.view = View::Surface;
        h.app.grid = 201;
        let ctx = egui::Context::default();
        let ws = Snapshot::default();
        let start = Instant::now();
        h.refresh(&ctx, &ws);
        assert!(h.running());
        assert!(start.elapsed() < Duration::from_secs(1));
        fuzzy_designer::stop(&mut h);
        let until = Instant::now() + Duration::from_secs(5);
        while h.running() {
            h.refresh(&ctx, &ws);
            assert!(Instant::now() < until);
            std::thread::sleep(Duration::from_millis(2));
        }
        for _ in 0..3 {
            h.refresh(&ctx, &ws);
            assert!(!h.running());
        }
        close(&h.app.current.as_ref().unwrap().values, &old);
        assert!(h.status().0.contains("stopped"));
    }
    #[test]
    fn fuzzy_designer_hook_is_scoped_and_rejects_outputs_and_nonfis() {
        use std::sync::{Arc, Mutex};
        let events = Arc::new(Mutex::new(Vec::new()));
        let slot = events.clone();
        let guard = fuzzy_app_hooks::present(move |s| {
            slot.lock().unwrap().push(s);
            Ok(())
        });
        let mut e = Engine::default();
        e.execute("fuzzyLogicDesigner(); f=sugfis; fuzzyLogicDesigner(f);")
            .unwrap();
        assert_eq!(events.lock().unwrap().len(), 2);
        assert!(e.execute("fuzzyLogicDesigner(17)").is_err());
        assert!(e.execute("x=fuzzyLogicDesigner()").is_err());
        drop(guard);
        assert!(
            e.execute("fuzzyLogicDesigner")
                .unwrap_err()
                .ends_with("fuzzyLogicDesigner requires the desktop")
        );
    }

    #[test]
    fn fuzzy_designer_three_input_surface_uses_reference_and_selected_axes() {
        let mut engine = Engine::default();
        engine
            .execute("f=mamfis('NumInputs',3,'NumOutputs',1);")
            .unwrap();
        let mut h = Host::new(FuzzyDesigner::default());
        h.app
            .import(fuzzy_app_hooks::snapshot(&engine, "f").unwrap())
            .unwrap();
        settle(&mut h);
        good(&h);
        h.app.view = View::Surface;
        h.app.surface_inputs = [0, 2];
        h.app.grid = 5;
        h.app.input_values = vec![0.2, 0.4, 0.7];
        settle(&mut h);
        good(&h);
        export(&mut h, &mut engine);
        engine.execute("[x,y,z]=gensurf(fuzzySystem,gensurfOptions('InputIndex',[1 3],'NumGridPoints',5,'ReferenceInputs',[0.2 0.4 0.7]));").unwrap();
        let r = h.app.current.as_ref().unwrap();
        close(&r.surface_x, &nums(&engine, "x"));
        close(&r.surface_y, &nums(&engine, "y"));
        close(&r.surface_z, &nums(&engine, "z"));
    }
    #[test]
    fn fuzzy_designer_cancelled_export_restores_fis_object() {
        let mut h = setup(false);
        let mut engine = Engine::default();
        export(&mut h, &mut engine);
        engine.execute("original=fuzzySystem;").unwrap();
        let code = apps::all_or_nothing(
            "fuzzySystem=sugfis;pause(60);",
            &h.app.exported(&Snapshot::default()),
        );
        let cancel = engine.cancellation();
        let thread = std::thread::spawn(move || {
            let result = apps::execute_export(&mut engine, &code);
            (engine, result)
        });
        std::thread::sleep(Duration::from_millis(20));
        cancel.store(true, std::sync::atomic::Ordering::Relaxed);
        let (mut engine, result) = thread.join().unwrap();
        assert!(result.is_err());
        cancel.store(false, std::sync::atomic::Ordering::Relaxed);
        engine
            .execute("same=isequal(original,fuzzySystem);")
            .unwrap();
        assert!(matches!(engine.array("same"),Some(Array::Logical(m)) if m.data()==[1]));
    }

    #[test]
    fn fuzzy_designer_all_builtin_memberships_and_replacement_match_plotmf() {
        let mut h = Host::new(FuzzyDesigner::default());
        h.app.add_variable(false, "sensor", "[0 1]").unwrap();
        settle(&mut h);
        good(&h);
        for (kind, p) in [
            ("trimf", "[0 0.5 1]"),
            ("trapmf", "[0 0.2 0.8 1]"),
            ("gaussmf", "[0.2 0.5]"),
            ("gauss2mf", "[0.2 0.2 0.2 0.8]"),
            ("gbellmf", "[0.3 2 0.5]"),
            ("sigmf", "[5 0.5]"),
            ("dsigmf", "[5 0.2 5 0.8]"),
            ("psigmf", "[5 0.2 -5 0.8]"),
            ("zmf", "[0 1]"),
            ("smf", "[0 1]"),
            ("pimf", "[0 0.2 0.8 1]"),
            ("linsmf", "[0 1]"),
            ("linzmf", "[0 1]"),
        ] {
            h.app.add_member(kind, kind, p, false).unwrap();
            settle(&mut h);
            good(&h);
        }
        h.app.member = 0;
        h.app
            .add_member("replaced", "gaussmf", "[0.15 0.4]", true)
            .unwrap();
        settle(&mut h);
        good(&h);
        assert_eq!(h.app.system.inputs[0].members.len(), 13);
        assert_eq!(h.app.system.inputs[0].members[0].kind, "gaussmf");
        let mut e = Engine::default();
        export(&mut h, &mut e);
        e.execute("[x,y]=plotmf(fuzzySystem,'input',1);").unwrap();
        let r = h.app.current.as_ref().unwrap();
        close(&r.mf_x, &nums(&e, "x"));
        close(&r.mf_y, &nums(&e, "y"));
    }
}
