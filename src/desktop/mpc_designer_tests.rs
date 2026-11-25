// Expected controller properties come from fresh engine script execution.
mod mpc_designer_tests {
    use super::*;
    use apps::{
        App, AppKind, Host, Snapshot,
        mpc_designer::{self as mpc, Command, MpcDesigner},
    };
    use matrust::{Array, Engine, engine::mpc_app_hooks};
    const PLANT: &str = "plant=ss([-1 0;0 -2],[1 0;0 1],[1 0;0 1],[0 0;0 0]); plant.InputName={'valve','heater'}; plant.OutputName={'level','temperature'};";
    fn workspace(code: &str) -> (Engine, Snapshot) {
        let mut e = Engine::default();
        e.execute(code).unwrap();
        let mut ws = Snapshot::default();
        ws.sync(&e.workspace());
        (e, ws)
    }
    fn wait_workspace(app: &mut Workspace) {
        let until = Instant::now() + Duration::from_secs(30);
        while app.busy {
            app.receive();
            assert!(Instant::now() < until, "{}", app.status);
            std::thread::sleep(Duration::from_millis(2));
        }
    }
    fn settle(h: &mut Host<MpcDesigner>, ctx: &egui::Context, ws: &Snapshot) {
        let until = Instant::now() + Duration::from_secs(30);
        loop {
            h.refresh(ctx, ws);
            if !h.running() {
                break;
            }
            assert!(Instant::now() < until, "{}", h.status().0);
            std::thread::sleep(Duration::from_millis(2));
        }
    }
    fn run(h: &mut Host<MpcDesigner>, ctx: &egui::Context, ws: &Snapshot, cmd: Command) {
        mpc::start_work(h, ctx, ws, cmd);
        settle(h, ctx, ws);
        assert_ne!(h.status().1, toolkits::AppStatus::Error, "{}", h.status().0);
    }
    fn loaded(ws: &Snapshot) -> (egui::Context, Host<MpcDesigner>) {
        let ctx = egui::Context::default();
        let mut h = Host::new(MpcDesigner::default());
        h.app.source_text = "plant".into();
        run(&mut h, &ctx, ws, Command::Create);
        (ctx, h)
    }
    fn replay(h: &Host<MpcDesigner>, ws: &Snapshot) -> Engine {
        let mut fresh = Engine::default();
        for (n, a) in ws
            .variables()
            .filter(|(_, a)| matrust::engine::lti_class(a).is_some())
        {
            fresh.insert_array(n, a.clone()).unwrap();
        }
        for s in &h.app.catalogue {
            s.install(&mut fresh).unwrap();
        }
        fresh.execute(&h.app.script(ws).unwrap()).unwrap();
        fresh
    }
    fn truth(e: &Engine, name: &str) {
        match e.array(name).unwrap() {
            Array::Logical(m) => assert_eq!(m.data(), &[1]),
            a => panic!("Expected true, got {a:?}"),
        }
    }
    fn same_properties(h: &Host<MpcDesigner>, ws: &Snapshot) {
        assert_eq!(h.app.export_code(ws).unwrap(), h.app.script(ws).unwrap());
        let mut fresh = replay(h, ws);
        let mut actual = h.app.controllers[h.app.selected].value.clone();
        actual.name = "actual".into();
        actual.install(&mut fresh).unwrap();
        let expected = &h.app.controllers[h.app.selected].spec.name;
        for p in [
            "Ts",
            "PredictionHorizon",
            "ControlHorizon",
            "Weights",
            "MV",
            "OV",
            "DV",
            "Model",
            "Optimizer",
            "Notes",
            "UserData",
        ] {
            fresh
                .execute(&format!("ok=isequal(actual.{p},{expected}.{p});"))
                .unwrap();
            truth(&fresh, "ok");
        }
        let plant = matrust::engine::mpc_prediction_ss(&fresh, expected).unwrap();
        assert_eq!(
            h.app.controllers[h.app.selected].plant_size,
            [plant.0.rows(), plant.1.cols(), plant.2.rows()]
        );
        let model = matrust::engine::mpc_prediction_model(&mut fresh, expected).unwrap();
        assert_eq!(
            h.app.controllers[h.app.selected].size,
            [model.a.rows(), model.bu.cols(), model.c.rows()]
        );
    }

    fn numeric(e: &Engine, name: &str) -> matrust::Matrix {
        let Array::Numeric(m) = e.array(name).unwrap() else {
            panic!("{name} is not numeric")
        };
        m
    }
    fn equal_matrix(a: &matrust::Matrix, b: &matrust::Matrix) {
        assert_eq!((a.rows(), a.cols()), (b.rows(), b.cols()));
        for (x, y) in a.data().iter().zip(b.data()) {
            assert!((x - y).abs() <= 1e-12, "{x} != {y}");
        }
    }
    #[test]
    fn mpc_designer_consolidated_import_preserves_supplied_estimator() {
        let (source_engine, ws) =
            workspace("source=mpc(ss(.8,.2,1,0,.1)); setEstimator(source,[0;0],[0;0]);");
        let ctx = egui::Context::default();
        let mut h = Host::new(MpcDesigner::default());
        let source = mpc_app_hooks::snapshot(&source_engine, "source").unwrap();
        h.app.catalogue.push(source);
        h.app.source_text = "source".into();
        run(&mut h, &ctx, &ws, Command::Create);
        for apply in [false, true] {
            if apply {
                run(&mut h, &ctx, &ws, Command::Apply);
            }
            let mut independent = replay(&h, &ws);
            let mut actual = h.app.controllers[0].value.clone();
            actual.name = "actual".into();
            actual.install(&mut independent).unwrap();
            independent.execute("[ls,ms]=getEstimator(source); [la,ma]=getEstimator(actual); o=mpcsimopt; o.PlantInitialState=1; [ys,ts,us]=sim(source,3,0,o); [ya,ta,ua]=sim(actual,3,0,o);").unwrap();
            equal_matrix(&numeric(&independent, "la"), &numeric(&independent, "ls"));
            equal_matrix(&numeric(&independent, "ma"), &numeric(&independent, "ms"));
            equal_matrix(&numeric(&independent, "ya"), &numeric(&independent, "ys"));
            equal_matrix(&numeric(&independent, "ua"), &numeric(&independent, "us"));
            assert_eq!(numeric(&independent, "us").data(), &[0., 0., 0.]);
            for (a, b) in numeric(&independent, "ys")
                .data()
                .iter()
                .zip([1., 0.8, 0.64])
            {
                assert!((a - b).abs() < 1e-12);
            }
            let n = &h.app.controllers[0].spec.name;
            independent
                .execute(&format!(
                    "[lr,mr]=getEstimator({n}); [yr,tr,ur]=sim({n},3,0,o);"
                ))
                .unwrap();
            equal_matrix(&numeric(&independent, "lr"), &numeric(&independent, "ls"));
            equal_matrix(&numeric(&independent, "mr"), &numeric(&independent, "ms"));
            equal_matrix(&numeric(&independent, "yr"), &numeric(&independent, "ys"));
            equal_matrix(&numeric(&independent, "ur"), &numeric(&independent, "us"));
        }
        h.app.scenario.samples = "3".into();
        h.app.scenario.reference = "0".into();
        h.app.scenario.initial = "1".into();
        run(&mut h, &ctx, &ws, Command::Run);
        let e = replay(&h, &ws);
        let mut original = Engine::default();
        mpc_app_hooks::snapshot(&source_engine, "source")
            .unwrap()
            .install(&mut original)
            .unwrap();
        original
            .execute("o=mpcsimopt;o.PlantInitialState=1;[ys,ts,us]=sim(source,3,0,o);")
            .unwrap();
        let trajectory = &h.app.result.as_ref().unwrap().trajectories[0];
        equal_matrix(&trajectory.y, &numeric(&original, "ys"));
        equal_matrix(&trajectory.u, &numeric(&original, "us"));
        equal_matrix(&numeric(&e, "scenario_y"), &numeric(&original, "ys"));
        equal_matrix(&numeric(&e, "scenario_u"), &numeric(&original, "us"));
        assert_eq!(h.app.export_code(&ws).unwrap(), h.app.script(&ws).unwrap());
    }
    #[test]
    fn mpc_designer_ecr_positive_finite_keeps_accepted_controller() {
        let (_, ws) = workspace("plant=ss(.8,.2,1,0,.1);");
        let (ctx, mut h) = loaded(&ws);
        let before = h.app.controllers[0].value.clone();
        for invalid in ["0", "-1", "NaN", "Inf"] {
            h.app.editor.as_mut().unwrap().ecr = invalid.into();
            mpc::start_work(&mut h, &ctx, &ws, Command::Apply);
            settle(&mut h, &ctx, &ws);
            assert_eq!(
                h.status().1,
                toolkits::AppStatus::Error,
                "accepted {invalid}"
            );
            let mut e = Engine::default();
            let mut original = before.clone();
            original.name = "original".into();
            original.install(&mut e).unwrap();
            let mut actual = h.app.controllers[0].value.clone();
            actual.name = "actual".into();
            actual.install(&mut e).unwrap();
            e.execute("ok=isequal(original,actual);").unwrap();
            truth(&e, "ok");
        }
    }
    #[test]
    fn mpc_designer_import_and_settings_use_physical_output_indices() {
        let (e, ws) = workspace(
            "p=ss(.8,.2,[2;1],[0;0],.1);p.OutputName={'first','second'};p=setmpcsignals(p,'MO',2,'UO',1);source=mpc(p,.1,1,1);source.OV(1).Units='m';source.OV(2).Units='K';source.OV(1).Max=.1;source.OV(1).MaxECR=0;",
        );
        let ctx = egui::Context::default();
        let mut h = Host::new(MpcDesigner::default());
        h.app
            .catalogue
            .push(mpc_app_hooks::snapshot(&e, "source").unwrap());
        h.app.source_text = "source".into();
        run(&mut h, &ctx, &ws, Command::Create);
        let rows = &h.app.controllers[0].spec.ov;
        assert_eq!(
            (
                rows[0].channel,
                rows[0].name.as_str(),
                rows[0].units.as_str()
            ),
            (0, "first", "m")
        );
        assert_eq!(
            (
                rows[1].channel,
                rows[1].name.as_str(),
                rows[1].units.as_str()
            ),
            (1, "second", "K")
        );
        run(&mut h, &ctx, &ws, Command::Apply);
        h.app.scenario.samples = "2".into();
        h.app.scenario.reference = "[2 1]".into();
        run(&mut h, &ctx, &ws, Command::Run);
        let data = &h.app.result.as_ref().unwrap().trajectories[0];
        assert!((data.u.get(0, 0) - 0.25).abs() < 1e-6);
        assert!(data.y.get(1, 0) <= 0.1 + 1e-7);
        scenario_replay(&h, &ws);
        let (out, _) = frame(&mut h, &ctx, &ws, vec![], egui::vec2(1440., 1000.));
        position(&out, "Output 1 · first [m] · unmeasured");
        h.app.output_channel = 1;
        let (out, _) = frame(&mut h, &ctx, &ws, vec![], egui::vec2(1440., 1000.));
        position(&out, "Output 2 · second [K]");
    }
    #[test]
    fn mpc_designer_custom_estimator_import_and_noop_apply_remain_custom() {
        let (e, ws) = workspace(
            "source=mpc(ss(.8,.2,1,0,.1));source.Weights.MVRate=.3;setEstimator(source,'custom');",
        );
        let ctx = egui::Context::default();
        let mut h = Host::new(MpcDesigner::default());
        h.app
            .catalogue
            .push(mpc_app_hooks::snapshot(&e, "source").unwrap());
        h.app.source_text = "source".into();
        run(&mut h, &ctx, &ws, Command::Create);
        run(&mut h, &ctx, &ws, Command::Apply);
        let mut fresh = replay(&h, &ws);
        let mut actual = h.app.controllers[0].value.clone();
        actual.name = "actual".into();
        actual.install(&mut fresh).unwrap();
        fresh.execute("[ls,ms]=getEstimator(source);[la,ma]=getEstimator(actual);o=mpcsimopt;o.PlantInitialState=1;[ys,ts,us]=sim(source,3,0,o);[ya,ta,ua]=sim(actual,3,0,o);").unwrap();
        assert!(numeric(&fresh, "ls").is_empty());
        equal_matrix(&numeric(&fresh, "la"), &numeric(&fresh, "ls"));
        equal_matrix(&numeric(&fresh, "ma"), &numeric(&fresh, "ms"));
        equal_matrix(&numeric(&fresh, "ya"), &numeric(&fresh, "ys"));
        equal_matrix(&numeric(&fresh, "ua"), &numeric(&fresh, "us"));
        let n = &h.app.controllers[0].spec.name;
        fresh
            .execute(&format!(
                "[lr,mr]=getEstimator({n});[yr,tr,ur]=sim({n},3,0,o);"
            ))
            .unwrap();
        equal_matrix(&numeric(&fresh, "lr"), &numeric(&fresh, "ls"));
        equal_matrix(&numeric(&fresh, "mr"), &numeric(&fresh, "ms"));
        equal_matrix(&numeric(&fresh, "yr"), &numeric(&fresh, "ys"));
        equal_matrix(&numeric(&fresh, "ur"), &numeric(&fresh, "us"));
    }
    #[test]
    fn mpc_designer_scalar_30_sample_scenario_matches_fresh_engine() {
        let (_, ws) = workspace("plant=ss(.8,.2,1,0,.1);");
        let (ctx, mut h) = loaded(&ws);
        h.app.editor.as_mut().unwrap().mv[0].bounds[1] = "0.4".into();
        run(&mut h, &ctx, &ws, Command::Apply);
        h.app.scenario.reference = "1".into();
        run(&mut h, &ctx, &ws, Command::Run);
        let result = h.app.result.as_ref().unwrap();
        let data = &result.trajectories[0];
        assert_eq!((data.t.rows(), data.t.cols()), (30, 1));
        assert!((data.t.get(29, 0) - 2.9).abs() < 1e-12);
        assert!(data.u.data().iter().all(|v| *v <= 0.4 + 1e-12));
        let e = replay(&h, &ws);
        for (actual, name) in [
            (&data.y, "scenario_y"),
            (&data.t, "scenario_t"),
            (&data.u, "scenario_u"),
        ] {
            equal_matrix(actual, &numeric(&e, name));
        }
        // A second independent Engine executes explicit public sim calls, with
        // the accepted controller installed, rather than using app trajectories.
        let mut independent = Engine::default();
        result.run.controllers[0]
            .value
            .install(&mut independent)
            .unwrap();
        independent
            .execute("[iy,it,iu]=sim(controller,30,1);")
            .unwrap();
        equal_matrix(&data.y, &numeric(&independent, "iy"));
        equal_matrix(&data.t, &numeric(&independent, "it"));
        equal_matrix(&data.u, &numeric(&independent, "iu"));
    }

    fn scenario_replay(h: &Host<MpcDesigner>, ws: &Snapshot) {
        let e = replay(h, ws);
        let result = h.app.result.as_ref().unwrap();
        for (i, data) in result.trajectories.iter().enumerate() {
            let suffix = if i == 0 {
                String::new()
            } else {
                (i + 1).to_string()
            };
            for (actual, name) in [(&data.y, "y"), (&data.t, "t"), (&data.u, "u")] {
                equal_matrix(actual, &numeric(&e, &format!("scenario_{name}{suffix}")));
            }
        }
    }
    #[test]
    fn mpc_designer_compare_matches_independent_engines_and_self_is_exact() {
        let (_, ws) = workspace("plant=ss(.8,.2,1,0,.1);");
        let (ctx, mut h) = loaded(&ws);
        h.app.editor.as_mut().unwrap().mv[0].bounds[1] = ".8".into();
        run(&mut h, &ctx, &ws, Command::Apply);
        let mut second = h.app.controllers[0].clone();
        second.spec.name = "alternative".into();
        h.app.controllers.push(second);
        h.app.selected = 1;
        h.app.editor = Some(h.app.controllers[1].spec.clone());
        h.app.editor.as_mut().unwrap().weights[2] = "4".into();
        run(&mut h, &ctx, &ws, Command::Apply);
        h.app.selected = 0;
        h.app.editor = Some(h.app.controllers[0].spec.clone());
        h.app.scenario.reference = "1".into();
        h.app.compare = Some(1);
        run(&mut h, &ctx, &ws, Command::Run);
        scenario_replay(&h, &ws);
        let result = h.app.result.as_ref().unwrap();
        assert!(
            result.trajectories[0]
                .u
                .data()
                .iter()
                .zip(result.trajectories[1].u.data())
                .any(|(a, b)| (a - b).abs() > 1e-4)
        );
        for (i, weight) in [".1", "4"].iter().enumerate() {
            let mut independent = Engine::default();
            independent.execute(&format!("p=ss(.8,.2,1,0,.1); c=mpc(p,.1,10,2); c.MV.Max=.8; c.Weights.MVRate={weight}; [y,t,u]=sim(c,30,1);")).unwrap();
            equal_matrix(&result.trajectories[i].y, &numeric(&independent, "y"));
            equal_matrix(&result.trajectories[i].t, &numeric(&independent, "t"));
            equal_matrix(&result.trajectories[i].u, &numeric(&independent, "u"));
        }
        h.app.compare = Some(0);
        run(&mut h, &ctx, &ws, Command::Run);
        let data = &h.app.result.as_ref().unwrap().trajectories;
        assert_eq!(data[0].y.data(), data[1].y.data());
        assert_eq!(data[0].u.data(), data[1].u.data());
        scenario_replay(&h, &ws);
    }
    #[test]
    fn mpc_designer_physical_reordered_outputs_md_lookahead_and_labels() {
        let (_, ws) = workspace(
            "plant=ss([.8 0;0 .7],[.2 .1;.1 .3],[1 0;1 1],[0 0;0 0],.1); plant.OutputName={'unmeasured level','measured temperature'}; plant=setmpcsignals(plant,'MV',1,'MD',2,'MO',2,'UO',1);",
        );
        let (ctx, mut h) = loaded(&ws);
        assert_eq!(h.app.controllers[0].spec.ov[0].channel, 0);
        h.app.editor.as_mut().unwrap().ov[0].units = "m".into();
        h.app.editor.as_mut().unwrap().ov[1].units = "K".into();
        run(&mut h, &ctx, &ws, Command::Apply);
        h.app.scenario.samples = "5".into();
        h.app.scenario.reference = "[2 3;4 5]".into();
        h.app.scenario.disturbance = "[.1;.2]".into();
        h.app.scenario.initial = "[.2;.4]".into();
        h.app.scenario.reference_preview = true;
        h.app.scenario.disturbance_preview = true;
        run(&mut h, &ctx, &ws, Command::Run);
        scenario_replay(&h, &ws);
        let result = h.app.result.as_ref().unwrap();
        assert_eq!(result.reference.get(0, 0), 2.);
        assert_eq!(result.reference.get(0, 1), 3.);
        assert!((result.trajectories[0].y.get(0, 0) - 0.2).abs() < 1e-12);
        assert!((result.trajectories[0].y.get(0, 1) - 0.6).abs() < 1e-12);
        let mut independent = Engine::default();
        result.run.controllers[0]
            .value
            .install(&mut independent)
            .unwrap();
        independent.execute("o=mpcsimopt; o.RefLookAhead='on'; o.MDLookAhead='on'; o.PlantInitialState=[.2;.4]; [y,t,u]=sim(controller,5,[2 3;4 5],[.1;.2],o);").unwrap();
        equal_matrix(&result.trajectories[0].y, &numeric(&independent, "y"));
        equal_matrix(&result.trajectories[0].u, &numeric(&independent, "u"));
        let (out, _) = frame(&mut h, &ctx, &ws, vec![], egui::vec2(1440., 1000.));
        position(&out, "Output 1 · unmeasured level [m] · unmeasured");
        h.app.output_channel = 1;
        let (out, _) = frame(&mut h, &ctx, &ws, vec![], egui::vec2(1440., 1000.));
        position(&out, "Output 2 · measured temperature [K]");
        h.app.view = 2;
        frame(&mut h, &ctx, &ws, vec![], egui::vec2(1440., 1000.));
    }
    #[test]
    fn mpc_designer_scenario_errors_reset_draft_and_stale_source() {
        let (mut e, ws) = workspace("plant=ss(.8,.2,1,0,.1);");
        let (ctx, mut h) = loaded(&ws);
        run(&mut h, &ctx, &ws, Command::Run); // empty signals are nominal defaults
        let good = h.app.result.as_ref().unwrap().trajectories[0].y.clone();
        let original = h.app.scenario.clone();
        for (field, value) in [
            (0, "0"),
            (0, "-1"),
            (0, "1.5"),
            (0, "10001"),
            (0, "Inf"),
            (0, "[2 3]"),
            (1, "[1 2]"),
            (1, "[1;2 3]"),
            (1, "sin(1)"),
            (1, "1;evil=1"),
            (1, "NaN"),
            (1, "Inf"),
            (2, "1"),
            (3, "[1 2]"),
            (3, "[1 2;3 4]"),
        ] {
            h.app.scenario = original.clone();
            let scenario = &mut h.app.scenario;
            match field {
                0 => scenario.samples = value.into(),
                1 => scenario.reference = value.into(),
                2 => scenario.disturbance = value.into(),
                _ => scenario.initial = value.into(),
            }
            assert!(
                h.app.start(&ws, Command::Run).is_err(),
                "accepted {field}={value}"
            );
            assert_eq!(
                h.app.result.as_ref().unwrap().trajectories[0].y.data(),
                good.data()
            );
        }
        for reference in [
            " ".repeat(32769),
            "1".repeat(32769),
            format!("[{}]", vec!["0"; 10001].join(" ")),
        ] {
            h.app.scenario = original.clone();
            h.app.scenario.reference = reference;
            assert!(h.app.start(&ws, Command::Run).is_err());
        }
        h.app.scenario = original;
        h.app.scenario.reference = "1".into();
        h.app.scenario.initial = "2".into();
        run(&mut h, &ctx, &ws, Command::Run);
        let first = h.app.result.as_ref().unwrap().trajectories[0].clone();
        assert_eq!(first.y.get(0, 0), 2.);
        run(&mut h, &ctx, &ws, Command::Run);
        assert_eq!(
            first.y.data(),
            h.app.result.as_ref().unwrap().trajectories[0].y.data()
        );
        assert_eq!(
            first.u.data(),
            h.app.result.as_ref().unwrap().trajectories[0].u.data()
        );
        let code = h.app.script(&ws).unwrap();
        h.app.editor.as_mut().unwrap().weights[2] = "20".into();
        h.app.scenario.reference = "3".into();
        assert_eq!(h.app.script(&ws).unwrap(), code);
        scenario_replay(&h, &ws);
        // Running an accepted controller also preserves a separate unapplied draft.
        run(&mut h, &ctx, &ws, Command::Run);
        assert_eq!(h.app.editor.as_ref().unwrap().weights[2], "20");
        let displayed_code = h.app.script(&ws).unwrap();
        run(&mut h, &ctx, &ws, Command::Apply);
        assert_eq!(h.app.script(&ws).unwrap(), displayed_code);
        scenario_replay(&h, &ws);
        e.execute("plant=ss(.7,.2,1,0,.1);").unwrap();
        let mut changed = Snapshot::default();
        changed.sync(&e.workspace());
        assert!(h.app.start(&changed, Command::Run).is_err());
        assert!(h.app.script(&changed).is_err());
    }
    #[test]
    fn mpc_designer_compare_rejects_incompatible_ts_and_physical_roles() {
        for second in [
            "plant2=ss(.8,.2,1,0,.2);",
            "plant2=ss(.8,[.2 .1],1,[0 0],.1); plant2=setmpcsignals(plant2,'MV',1,'MD',2);",
        ] {
            let (_, ws) = workspace(&format!("plant=ss(.8,.2,1,0,.1); {second}"));
            let (ctx, mut h) = loaded(&ws);
            h.app.source_text = "plant2".into();
            run(&mut h, &ctx, &ws, Command::Create);
            h.app.compare = Some(0);
            let error = h.app.start(&ws, Command::Run).unwrap_err();
            assert!(
                error.contains("sample time") || error.contains("physical"),
                "{error}"
            );
        }
        let (_, ws) = workspace(
            "plant=ss(.8,[.2 .1],1,[0 0],.1); plant=setmpcsignals(plant,'MV',1,'MD',2); plant2=setmpcsignals(plant,'MV',2,'MD',1);",
        );
        let (ctx, mut h) = loaded(&ws);
        h.app.source_text = "plant2".into();
        run(&mut h, &ctx, &ws, Command::Create);
        h.app.compare = Some(0);
        assert!(
            h.app
                .start(&ws, Command::Run)
                .unwrap_err()
                .contains("roles")
        );
    }
    #[test]
    fn mpc_designer_scenario_export_names_rollback_and_cancellation_are_atomic() {
        let (mut e, ws) = workspace("plant=ss(.8,.2,1,0,.1);");
        let (ctx, mut h) = loaded(&ws);
        h.app.scenario.reference = "1".into();
        run(&mut h, &ctx, &ws, Command::Run);
        for prefix in ["app_mpc_bad", "mrexp_old", "for", "x;evil=1", "controller"] {
            h.app.scenario_name = prefix.into();
            // A controller prefix is legal unless a resulting name collides.
            if prefix != "controller" {
                assert!(h.app.script(&ws).is_err(), "{prefix}");
            }
        }
        h.app.scenario_name = "scenario".into();
        let Some(toolkits::Action::ExportScript(code)) = h.export(&ws) else {
            panic!()
        };
        apps::execute_export(&mut e, &code).unwrap();
        equal_matrix(
            &numeric(&e, "scenario_y"),
            &h.app.result.as_ref().unwrap().trajectories[0].y,
        );
        e.execute("controller=42; scenario_y=43; clear plant;")
            .unwrap();
        assert!(apps::execute_export(&mut e, &code).is_err());
        e.execute("ok=isequal(controller,42)&&isequal(scenario_y,43);")
            .unwrap();
        truth(&e, "ok");
        let cancel = e.cancellation();
        let signal = cancel.clone();
        let thread = std::thread::spawn(move || {
            std::thread::sleep(Duration::from_millis(10));
            signal.store(true, std::sync::atomic::Ordering::Relaxed);
        });
        let cancelled = apps::all_or_nothing(
            "controller=1; scenario_y=2; while true, end",
            &["controller".into(), "scenario_y".into()],
        );
        assert!(apps::execute_export(&mut e, &cancelled).is_err());
        thread.join().unwrap();
        cancel.store(false, std::sync::atomic::Ordering::Relaxed);
        e.execute("ok=isequal(controller,42)&&isequal(scenario_y,43);")
            .unwrap();
        truth(&e, "ok");
        let mut colliding = h.app.controllers[0].spec.clone();
        colliding.name = "scenario_y".into();
        assert!(
            mpc::build_scenario_script(&[colliding], &h.app.scenario, "scenario")
                .unwrap_err()
                .contains("collides")
        );
        h.app.controller_only = true;
        assert!(!h.app.script(&ws).unwrap().contains("= sim("));
    }
    #[test]
    fn mpc_designer_pointer_scenario_compare_stop_export_and_script_replay() {
        let (mut e, ws) = workspace("plant=ss(.8,.2,1,0,.1);");
        let (ctx, mut h) = loaded(&ws);
        click(&mut h, &ctx, &ws, "Duplicate");
        h.app.editor.as_mut().unwrap().weights[2] = "2".into();
        click(&mut h, &ctx, &ws, "Apply controller");
        settle_frames(&mut h, &ctx, &ws);
        h.app.scenario.reference = "1".into();
        click(&mut h, &ctx, &ws, "Compare: None");
        click(&mut h, &ctx, &ws, "Compare with controller");
        assert_eq!(h.app.compare, Some(0));
        click(&mut h, &ctx, &ws, "Run scenario");
        settle_frames(&mut h, &ctx, &ws);
        assert_ne!(h.status().1, toolkits::AppStatus::Error, "{}", h.status().0);
        assert_eq!(h.app.result.as_ref().unwrap().trajectories.len(), 2);
        let good = h.app.result.as_ref().unwrap().trajectories[0].y.clone();
        h.app.scenario.samples = "10000".into();
        click(&mut h, &ctx, &ws, "Run scenario");
        assert!(h.running());
        let (out, _) = frame(&mut h, &ctx, &ws, vec![], egui::vec2(1440., 2400.));
        position(
            &out,
            "Calculation running; last completed results remain visible.",
        );
        for size in [egui::vec2(480., 640.), egui::vec2(320., 640.)] {
            frame(&mut h, &ctx, &ws, vec![], size);
        }
        click(&mut h, &ctx, &ws, "Stop");
        settle_frames(&mut h, &ctx, &ws);
        assert_eq!(
            good.data(),
            h.app.result.as_ref().unwrap().trajectories[0].y.data()
        );
        assert_eq!(h.status().1, toolkits::AppStatus::Error);
        h.app.scenario.samples = "30".into();
        click(&mut h, &ctx, &ws, "Run scenario");
        settle_frames(&mut h, &ctx, &ws);
        let Some(toolkits::Action::ExportScript(code)) =
            click(&mut h, &ctx, &ws, "Export to workspace")
        else {
            panic!("export")
        };
        apps::execute_export(&mut e, &code).unwrap();
        equal_matrix(
            &numeric(&e, "scenario_y"),
            &h.app.result.as_ref().unwrap().trajectories[0].y,
        );
        let Some(toolkits::Action::SaveAppScript(_, code)) =
            click(&mut h, &ctx, &ws, "Generate script")
        else {
            panic!("generate")
        };
        let mut fresh = Engine::default();
        fresh.execute("plant=ss(.8,.2,1,0,.1);").unwrap();
        fresh.execute(&code).unwrap();
        for (i, data) in h
            .app
            .result
            .as_ref()
            .unwrap()
            .trajectories
            .iter()
            .enumerate()
        {
            let suffix = if i == 0 {
                String::new()
            } else {
                (i + 1).to_string()
            };
            for (m, n) in [(&data.y, "y"), (&data.t, "t"), (&data.u, "u")] {
                equal_matrix(m, &numeric(&fresh, &format!("scenario_{n}{suffix}")));
            }
        }
    }
    #[test]
    fn mpc_designer_headless_narrow_actions_and_many_long_labels() {
        let (_, ws) = workspace(
            "plant=ss(.8,.2,ones(24,1),zeros(24,1),.1); plant=setmpcsignals(plant,'MO',1,'UO',[2 3 4 5 6 7 8 9 10 11 12 13 14 15 16 17 18 19 20 21 22 23 24]);",
        );
        let (ctx, mut h) = loaded(&ws);
        h.app.editor.as_mut().unwrap().ov[23].name =
            "a_very_long_channel_name_with_a_distinguishable_physical_index".into();
        run(&mut h, &ctx, &ws, Command::Apply);
        run(&mut h, &ctx, &ws, Command::Run);
        h.app.output_channel = 23;
        for i in 1..16 {
            let mut c = h.app.controllers[0].clone();
            c.spec.name = format!("controller_with_a_long_shared_prefix_and_distinct_suffix_{i}");
            h.app.controllers.push(c);
        }
        for dark in [false, true] {
            ctx.set_visuals(if dark {
                egui::Visuals::dark()
            } else {
                egui::Visuals::light()
            });
            for size in [
                egui::vec2(1440., 1000.),
                egui::vec2(480., 640.),
                egui::vec2(320., 640.),
            ] {
                for view in 0..4 {
                    h.app.view = view;
                    frame(&mut h, &ctx, &ws, vec![], size);
                }
                h.app.view = 0;
                h.app.error = Some("Scenario validation error; previous data retained".into());
                frame(&mut h, &ctx, &ws, vec![], size);
                h.app.error = None;
            }
        }
        h.app.view = 0;
        let size = egui::vec2(480., 640.);
        let (out, _) = frame(&mut h, &ctx, &ws, vec![], size);
        for label in [
            "Run scenario",
            "Stop",
            "Export to workspace",
            "Generate script",
        ] {
            let p = position(&out, label);
            assert!(
                p.x > 0. && p.x < size.x && p.y > 0. && p.y < size.y,
                "{label}: {p:?}"
            );
        }
        let pos = position(&out, "Run scenario");
        for pressed in [true, false] {
            frame(
                &mut h,
                &ctx,
                &ws,
                vec![
                    egui::Event::PointerMoved(pos),
                    egui::Event::PointerButton {
                        pos,
                        button: egui::PointerButton::Primary,
                        pressed,
                        modifiers: egui::Modifiers::NONE,
                    },
                ],
                size,
            );
        }
        settle_frames(&mut h, &ctx, &ws);
        assert!(h.app.result.is_some());
    }
    #[test]
    fn mpc_designer_controller_properties_equal_generated_script_in_fresh_engine() {
        let (_, ws) = workspace(PLANT);
        let (ctx, mut h) = loaded(&ws);
        assert_eq!(h.app.editor.as_ref().unwrap().mv[0].name, "valve");
        let s = h.app.editor.as_mut().unwrap();
        s.ts = "0.2".into();
        s.prediction = "12".into();
        s.control = "[2 3 7]".into();
        s.weights = ["[2 3]", "[0.2 0.4]", "[0.1 0.3]"].map(str::to_owned);
        s.ecr = "500".into();
        s.mv[0].bounds = ["-2", "3", "-0.4", "0.5", "0.3", "1"].map(str::to_owned);
        s.mv[1].bounds = ["0", "5", "-1", "1", "0", "0"].map(str::to_owned);
        s.ov[0].bounds[0] = "-4".into();
        s.ov[0].bounds[1] = "8".into();
        s.ov[0].bounds[4] = "0".into();
        run(&mut h, &ctx, &ws, Command::Apply);
        same_properties(&h, &ws);
        let mut e = replay(&h, &ws);
        e.execute("expected=mpc(setmpcsignals(plant,'MV',[1 2],'MO',[1 2]),0.2,12,[2 3 7]); expected.Weights.OV=[2 3]; expected.Weights.MV=[0.2 0.4]; expected.Weights.MVRate=[0.1 0.3]; expected.Weights.ECR=500; expected.MV(1).Min=-2; expected.MV(1).Max=3; expected.MV(1).RateMin=-0.4; expected.MV(1).RateMax=0.5; expected.MV(1).MinECR=0.3; expected.MV(1).MaxECR=1; expected.MV(2).Min=0; expected.MV(2).Max=5; expected.MV(2).RateMin=-1; expected.MV(2).RateMax=1; expected.OV(1).Min=-4; expected.OV(1).Max=8; expected.OV(1).MinECR=0; ok=isequal(expected,controller);").unwrap();
        truth(&e, "ok");
    }
    #[test]
    fn mpc_designer_tf_zpk_discrete_sources_and_disturbance_roles_replay() {
        for code in [
            "plant=tf(1,[1 1]);",
            "plant=zpk([],[-1],2);",
            "plant=ss(0.9,1,1,0,0.25);",
        ] {
            let (_, ws) = workspace(code);
            let (_, h) = loaded(&ws);
            same_properties(&h, &ws);
        }
        let (_, ws) = workspace(
            "plant=ss([-1 0;0 -2],[1 0 1;0 1 0],[1 1;0 1],[0 0 0;0 0 0]); plant=setmpcsignals(plant,'MV',1,'MD',2,'UD',3,'MO',1,'UO',2);",
        );
        let (ctx, mut h) = loaded(&ws);
        assert_eq!(h.app.editor.as_ref().unwrap().inputs, vec![0, 1, 2]);
        assert_eq!(h.app.editor.as_ref().unwrap().outputs, vec![0, 1]);
        h.app.editor.as_mut().unwrap().control = "[2;3;5]".into();
        run(&mut h, &ctx, &ws, Command::Apply);
        same_properties(&h, &ws);
        let mut fresh = replay(&h, &ws);
        fresh.execute("expected=mpc(plant,1,10,[2 3 5]); ok=isequal(expected.MV,controller.MV)&&isequal(expected.OV,controller.OV)&&isequal(expected.ControlHorizon,controller.ControlHorizon);").unwrap();
        truth(&fresh, "ok");
    }
    #[test]
    fn mpc_designer_invalid_literals_blocking_weights_constraints_keep_last_good() {
        let (_, ws) = workspace(PLANT);
        let (ctx, mut h) = loaded(&ws);
        let good = h.app.controllers[0].summary.clone();
        let original = h.app.editor.clone().unwrap();
        for (kind, value) in [
            (0, "[2 2]"),
            (0, "[0 10]"),
            (0, "[2 2;3 3]"),
            (0, "11"),
            (1, "[1 -2]"),
            (1, "[1 2 3]"),
            (2, "8; evil=1"),
            (2, "NaN"),
            (2, "Inf"),
            (3, "-1"),
            (4, "0"),
            (4, "10001"),
            (5, "0"),
            (5, "-0.1"),
        ] {
            h.app.editor = Some(original.clone());
            let s = h.app.editor.as_mut().unwrap();
            match kind {
                0 => s.control = value.into(),
                1 => s.weights[0] = value.into(),
                2 => s.mv[0].bounds[0] = value.into(),
                3 => s.ecr = value.into(),
                4 => s.prediction = value.into(),
                _ => s.ts = value.into(),
            }
            mpc::start_work(&mut h, &ctx, &ws, Command::Apply);
            settle(&mut h, &ctx, &ws);
            assert_eq!(h.status().1, toolkits::AppStatus::Error, "accepted {value}");
            assert_eq!(h.app.controllers[0].summary, good);
            let error = h.app.error.clone().expect("field error");
            let (out, _) = frame(&mut h, &ctx, &ws, vec![], egui::vec2(1440., 2400.));
            let mut messages = vec![];
            for shape in &out.shapes {
                tool_tab_texts(&shape.shape, &error, &mut messages);
            }
            assert!(!messages.is_empty(), "Missing inline error: {error}");
        }
        h.app.editor = Some(original);
        h.app.editor.as_mut().unwrap().control = "[2 3 5]".into();
        run(&mut h, &ctx, &ws, Command::Apply);
        same_properties(&h, &ws);
        h.app.editor.as_mut().unwrap().mv[0].bounds[0] = "5".into();
        h.app.editor.as_mut().unwrap().mv[0].bounds[1] = "2".into();
        mpc::start_work(&mut h, &ctx, &ws, Command::Apply);
        assert_eq!(h.status().1, toolkits::AppStatus::Error);
    }
    #[test]
    fn mpc_designer_engine_rejection_discrete_ts_and_stale_source_keep_result() {
        let (mut e, ws) = workspace("plant=ss(0.9,1,1,0,0.1);");
        let (ctx, mut h) = loaded(&ws);
        let previous = h.app.controllers[0].summary.clone();
        h.app.editor.as_mut().unwrap().ts = "0.2".into();
        mpc::start_work(&mut h, &ctx, &ws, Command::Apply);
        settle(&mut h, &ctx, &ws);
        assert_eq!(h.status().1, toolkits::AppStatus::Error);
        assert_eq!(h.app.controllers[0].summary, previous);
        e.execute("plant=ss(0.8,1,1,0,0.1);").unwrap();
        let mut changed = Snapshot::default();
        changed.sync(&e.workspace());
        assert!(h.app.script(&changed).is_err());
    }
    #[test]
    fn mpc_designer_import_roles_and_multiple_controllers_export_atomicity() {
        let (mut e, ws) = workspace(&format!(
            "{PLANT} plant.C=[1 1;0 1]; saved=mpc(plant,0.25,8,[2 2 4]); saved.MV(1).Min=4; saved.MV(1).Max=7; saved.Model.Nominal.U=[5;0]; saved.Optimizer.MaxIterations=90;"
        ));
        let ctx = egui::Context::default();
        let mut h = Host::new(MpcDesigner::default());
        h.app.catalogue = mpc_app_hooks::catalogue(&e);
        h.app.source_text = "saved".into();
        run(&mut h, &ctx, &ws, Command::Create);
        same_properties(&h, &ws);
        let s = h.app.editor.as_mut().unwrap();
        s.mv[0].bounds[0] = "8".into();
        s.mv[0].bounds[1] = "9".into();
        s.control = "4".into();
        run(&mut h, &ctx, &ws, Command::Apply);
        same_properties(&h, &ws);
        h.app.source_text = "plant".into();
        run(&mut h, &ctx, &ws, Command::Create);
        let s = h.app.editor.as_mut().unwrap();
        s.inputs[1] = 1;
        s.outputs[1] = 1;
        s.sync_roles();
        run(&mut h, &ctx, &ws, Command::Apply);
        assert_eq!(h.app.controllers[1].spec.mv.len(), 1);
        same_properties(&h, &ws);
        h.app.export_all = true;
        let Some(toolkits::Action::ExportScript(code)) = h.export(&ws) else {
            panic!()
        };
        apps::execute_export(&mut e, &code).unwrap();
        assert!(e.array("controller").is_some() && e.array("controller1").is_some());
        e.execute("controller=42; controller1=43; clear plant;")
            .unwrap();
        assert!(apps::execute_export(&mut e, &code).is_err());
        e.execute("ok=isequal(controller,42)&&isequal(controller1,43);")
            .unwrap();
        truth(&e, "ok");
        for name in [
            "mrexp_old",
            "mrexp_had",
            "mrexp_complete",
            "matrust_export_error",
            "app_mpc_bad",
            "for",
            "a;bad=1",
        ] {
            assert!(mpc::checked_name(name).is_err(), "{name}");
        }
        h.app.controllers[1].spec.name = "saved".into();
        assert!(h.app.script(&ws).is_err());
    }
    fn frame(
        h: &mut Host<MpcDesigner>,
        ctx: &egui::Context,
        ws: &Snapshot,
        events: Vec<egui::Event>,
        size: egui::Vec2,
    ) -> (egui::FullOutput, Option<toolkits::Action>) {
        let mut action = None;
        let out = ctx.run_ui(
            egui::RawInput {
                screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, size)),
                events,
                ..Default::default()
            },
            |ctx| {
                egui::CentralPanel::default()
                    .show(ctx, |ui| mpc::show(h, ui, ws, false, &mut action));
            },
        );
        assert!(!out.shapes.is_empty());
        (out, action)
    }
    fn pointer(
        h: &mut Host<MpcDesigner>,
        ctx: &egui::Context,
        ws: &Snapshot,
        pos: egui::Pos2,
    ) -> Option<toolkits::Action> {
        let mut action = None;
        for pressed in [true, false] {
            let (_, a) = frame(
                h,
                ctx,
                ws,
                vec![
                    egui::Event::PointerMoved(pos),
                    egui::Event::PointerButton {
                        pos,
                        button: egui::PointerButton::Primary,
                        pressed,
                        modifiers: egui::Modifiers::NONE,
                    },
                ],
                egui::vec2(1440., 2400.),
            );
            if a.is_some() {
                action = a;
            }
        }
        action
    }
    fn position(out: &egui::FullOutput, label: &str) -> egui::Pos2 {
        let mut found = vec![];
        for s in &out.shapes {
            tool_tab_texts(&s.shape, label, &mut found);
        }
        *found.first().unwrap_or_else(|| panic!("Missing {label}"))
    }
    fn click(
        h: &mut Host<MpcDesigner>,
        ctx: &egui::Context,
        ws: &Snapshot,
        label: &str,
    ) -> Option<toolkits::Action> {
        let (out, _) = frame(h, ctx, ws, vec![], egui::vec2(1440., 2400.));
        pointer(h, ctx, ws, position(&out, label))
    }
    fn settle_frames(h: &mut Host<MpcDesigner>, ctx: &egui::Context, ws: &Snapshot) {
        let until = Instant::now() + Duration::from_secs(30);
        loop {
            frame(h, ctx, ws, vec![], egui::vec2(1440., 2400.));
            if !h.running() {
                break;
            }
            assert!(Instant::now() < until);
            std::thread::sleep(Duration::from_millis(2));
        }
    }
    #[test]
    fn mpc_designer_real_pointer_create_constraint_apply_export_generate_list_and_stop() {
        let (mut e, ws) = workspace(PLANT);
        let ctx = egui::Context::default();
        let mut h = Host::new(MpcDesigner::default());
        h.app.source_text = "plant".into();
        frame(&mut h, &ctx, &ws, vec![], egui::vec2(1440., 2400.));
        click(&mut h, &ctx, &ws, "Create controller");
        settle_frames(&mut h, &ctx, &ws);
        assert_eq!(h.app.controllers.len(), 1);
        assert_eq!(h.app.controllers[0].spec.name, "controller");
        // Locate an actual constraint TextEdit by its displayed literal, focus it
        // with pointer input, select all and replace it with keyboard text input.
        let (out, _) = frame(&mut h, &ctx, &ws, vec![], egui::vec2(1440., 2400.));
        let mut positions = vec![];
        for s in &out.shapes {
            tool_tab_texts(&s.shape, "[-Inf]", &mut positions);
        }
        pointer(
            &mut h,
            &ctx,
            &ws,
            *positions.first().expect("constraint cell"),
        );
        frame(
            &mut h,
            &ctx,
            &ws,
            vec![
                egui::Event::Key {
                    key: egui::Key::A,
                    physical_key: None,
                    pressed: true,
                    repeat: false,
                    modifiers: egui::Modifiers::COMMAND,
                },
                egui::Event::Text("-3".into()),
            ],
            egui::vec2(1440., 2400.),
        );
        assert_eq!(h.app.editor.as_ref().unwrap().mv[0].bounds[0], "-3");
        click(&mut h, &ctx, &ws, "Apply controller");
        settle_frames(&mut h, &ctx, &ws);
        assert_ne!(h.status().1, toolkits::AppStatus::Error, "{}", h.status().0);
        same_properties(&h, &ws);
        let Some(toolkits::Action::ExportScript(code)) =
            click(&mut h, &ctx, &ws, "Export to workspace")
        else {
            panic!("Export button did nothing: {}", h.status().0)
        };
        apps::execute_export(&mut e, &code).unwrap();
        assert!(
            e.array("controller").is_some(),
            "names={:?}; workspace={:?}; code={code}",
            h.app.exported(&ws),
            e.workspace().keys().collect::<Vec<_>>()
        );
        e.execute("ok=isequal(controller.MV(1).Min,-3);").unwrap();
        truth(&e, "ok");
        let Some(toolkits::Action::SaveAppScript(AppKind::MpcDesigner, code)) =
            click(&mut h, &ctx, &ws, "Generate script")
        else {
            panic!("Generate button did nothing")
        };
        assert!(code.ends_with(&h.app.script(&ws).unwrap()));
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("controller.m");
        let mut desktop = Workspace::with_context(ctx.clone(), true);
        desktop.submit(worker::Job::ScriptInFolder(
            PLANT.into(),
            dir.path().to_owned(),
        ));
        wait_workspace(&mut desktop);
        let Some(toolkits::Action::ExportScript(export_code)) = h.export(&ws) else {
            panic!()
        };
        desktop.submit(worker::Job::ExportAppScript(
            export_code,
            dir.path().to_owned(),
        ));
        wait_workspace(&mut desktop);
        assert!(desktop.variables.contains_key("controller"));
        desktop.toolkits.apps.test_save_path = Some(path.clone());
        desktop.queue_app_script_save(AppKind::MpcDesigner, code.clone());
        desktop.finish_app_script_save(&ctx);
        wait_workspace(&mut desktop);
        assert_eq!(std::fs::read_to_string(&path).unwrap(), code);
        let mut fresh = Engine::default();
        fresh.execute(PLANT).unwrap();
        fresh
            .execute(&std::fs::read_to_string(path).unwrap())
            .unwrap();
        assert!(fresh.array("controller").is_some());
        click(&mut h, &ctx, &ws, "Duplicate");
        assert_eq!(h.app.controllers.len(), 2);
        h.app.rename = "second".into();
        click(&mut h, &ctx, &ws, "Rename");
        assert_eq!(h.app.controllers[1].spec.name, "second");
        click(&mut h, &ctx, &ws, "Delete");
        assert_eq!(h.app.controllers.len(), 1);
        // Hold a real Runner task until cancellation, making Stop deterministic
        // without depending on the duration of a tiny controller construction.
        mpc::hold_runner_for_stop_test(&mut h, &ctx);
        assert!(h.running());
        click(&mut h, &ctx, &ws, "Stop");
        settle_frames(&mut h, &ctx, &ws);
        assert_eq!(h.app.controllers.len(), 1);
        assert_eq!(h.status().1, toolkits::AppStatus::Error);
    }
    #[test]
    fn mpc_designer_launch_hook_cli_and_desktop_forms_and_headless_states() {
        assert!(
            Engine::default()
                .execute("mpcDesigner")
                .unwrap_err()
                .contains("mpcDesigner requires the desktop")
        );
        for call in ["mpcDesigner", "mpcDesigner(plant)", "mpcDesigner(saved)"] {
            let mut cli = Engine::default();
            let error = cli
                .execute(&format!("{PLANT} saved=mpc(plant,0.2); {call};"))
                .unwrap_err();
            assert!(
                error.contains("mpcDesigner requires the desktop"),
                "{call}: {error}"
            );
            let ctx = egui::Context::default();
            let mut app = Workspace::with_context(ctx.clone(), true);
            app.run(format!("{PLANT} saved=mpc(plant,0.2); {call};"));
            let until = Instant::now() + Duration::from_secs(30);
            while app.busy {
                app.receive();
                assert!(Instant::now() < until, "{}", app.status);
                std::thread::sleep(Duration::from_millis(2));
            }
            assert!(
                app.toolkits.apps.is_open(AppKind::MpcDesigner),
                "{call}: {}; console={:?}",
                app.status,
                app.console
            );
            app.sync_app_workspace();
            let ws = &app.toolkits.apps.workspace;
            let h = &mut app.toolkits.apps.mpc;
            frame(h, &ctx, ws, vec![], egui::vec2(1440., 1000.));
            settle_frames(h, &ctx, ws);
            assert_eq!(
                h.app.controllers.len(),
                usize::from(call != "mpcDesigner"),
                "{call}: {}",
                h.status().0
            );
        }
        let (_, ws) = workspace(PLANT);
        let (ctx, mut h) = loaded(&ws);
        assert_eq!(
            h.app.controllers[0].spec.mv.len() + h.app.controllers[0].spec.ov.len(),
            4
        );
        frame(&mut h, &ctx, &ws, vec![], egui::vec2(1440., 1000.));
        h.app.error = Some("Invalid controller draft".into());
        frame(&mut h, &ctx, &ws, vec![], egui::vec2(480., 640.));
        let mut empty = Host::new(MpcDesigner::default());
        frame(
            &mut empty,
            &ctx,
            &Snapshot::default(),
            vec![],
            egui::vec2(1440., 900.),
        );
        assert_eq!(
            AppKind::from_tab(AppKind::MpcDesigner.tab()),
            Some(AppKind::MpcDesigner)
        );
        let json = serde_json::to_string(&AppKind::MpcDesigner.tab()).unwrap();
        assert_eq!(
            serde_json::from_str::<matrust::session::ToolTab>(&json).unwrap(),
            AppKind::MpcDesigner.tab()
        );
    }
}
