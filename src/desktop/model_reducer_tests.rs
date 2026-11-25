// Model Reducer: SciPy references (tool/refs/model_reducer) and the real app host.
mod model_reducer_tests {
    use super::*;
    use apps::model_reducer::{Method, ModelReducer, View};
    use apps::{App, AppKind, Host, Snapshot};
    use toolkits::Tool;

    fn reference() -> serde_json::Value {
        serde_json::from_str(include_str!(
            "../../tests/fixtures/apps/model_reducer/reference.json"
        ))
        .unwrap()
    }
    fn list(v: &serde_json::Value) -> Vec<f64> {
        v.as_array()
            .unwrap()
            .iter()
            .map(|x| x.as_f64().unwrap())
            .collect()
    }
    fn matrix(v: &[f64]) -> String {
        let parts: Vec<String> = v.iter().map(|x| format!("{x:?}")).collect();
        format!("[{}]", parts.join(" "))
    }
    /// `name = ss(...)` from a reference system (column-major lists).
    fn define(name: &str, sys: &serde_json::Value) -> String {
        let (n, p, m) = (
            sys["n"].as_u64().unwrap(),
            sys["p"].as_u64().unwrap(),
            sys["m"].as_u64().unwrap(),
        );
        format!(
            "{name} = ss(reshape({}, {n}, {n}), reshape({}, {n}, {m}), reshape({}, {p}, {n}), reshape({}, {p}, {m}));\n",
            matrix(&list(&sys["a"])),
            matrix(&list(&sys["b"])),
            matrix(&list(&sys["c"])),
            matrix(&list(&sys["d"])),
        )
    }
    fn workspace(code: &str) -> Snapshot {
        let mut engine = matrust::Engine::default();
        engine.execute(code).unwrap();
        let mut ws = Snapshot::default();
        ws.sync(&engine.workspace());
        ws
    }
    fn engine_with(ws: &Snapshot) -> matrust::Engine {
        let mut engine = matrust::Engine::default();
        for (name, a) in ws.variables() {
            engine.insert_array(name, a.clone()).unwrap();
        }
        engine
    }
    /// The model query, as the Inputs column runs it.
    fn read_model(host: &mut Host<ModelReducer>, ws: &Snapshot) {
        host.app.prepare(ws);
        let request = host.app.info_request(ws).expect("a model");
        let key = request.key;
        let result = apps::run(&mut matrust::Engine::default(), request);
        host.app.receive_info(key, &result);
        host.app.prepare(ws);
    }
    fn compute(host: &mut Host<ModelReducer>, ws: &Snapshot) -> Result<apps::Outputs, String> {
        host.app.prepare(ws);
        let outputs = apps::run(&mut matrust::Engine::default(), host.request(ws)?)?;
        host.app.receive(&outputs)?;
        Ok(outputs)
    }
    fn set_balanced(host: &mut Host<ModelReducer>, k: usize, order: usize, match_dc: bool) {
        let row = &mut host.app.rows[k];
        row.method = Method::Balanced;
        row.order = order.to_string();
        row.order_edited = true;
        row.match_dc = match_dc;
    }
    fn values(engine: &mut matrust::Engine, expr: &str) -> Vec<f64> {
        engine.execute(&format!("mrt_v__ = {expr};")).unwrap();
        match engine.array("mrt_v__").unwrap() {
            matrust::Array::Numeric(m) => m.data().to_vec(),
            a => panic!("{expr}: {}", a.class()),
        }
    }
    fn close(actual: f64, expected: f64, tolerance: f64, what: &str) {
        assert!(
            (actual - expected).abs() <= tolerance * expected.abs().max(1.),
            "{what}: {actual:.17} != {expected:.17}"
        );
    }
    fn export(host: &mut Host<ModelReducer>, ws: &Snapshot) -> String {
        match host.export(ws).unwrap() {
            toolkits::Action::ExportScript(s) => s,
            _ => panic!("wrong export action"),
        }
    }

    #[test]
    fn model_reducer_hsvd_of_balanced_system_matches_scipy() {
        let r = reference();
        let ws = workspace(&define("G", &r["balanced4"]));
        let mut host = Host::new(ModelReducer::default());
        read_model(&mut host, &ws);
        assert_eq!(host.app.model, "G");
        let info = host.app.info().unwrap().clone();
        assert_eq!((info.outputs, info.inputs, info.states), (1, 1, 4));
        assert_eq!(info.ts, 0.);
        assert_eq!(info.unstable(), 0);
        let expected = list(&r["balanced4"]["hsv"]);
        assert_eq!(info.hsv.len(), 4);
        for (a, b) in info.hsv.iter().zip(&expected) {
            close(*a, *b, 1e-10, "hsv");
        }
        // The rows follow the suggested order: half the stable states.
        assert_eq!(host.app.rows[0].order, "2");
        compute(&mut host, &ws).unwrap();
        let c = host.app.current.as_ref().unwrap();
        assert_eq!(c.original.order, 4);
        assert_eq!(c.reduced[0].order, 2);
        // The generated script's hsvd agrees too.
        let mut engine = engine_with(&ws);
        engine.execute(&host.app.script(&ws).unwrap()).unwrap();
        for (a, b) in values(&mut engine, "mr_hsv").iter().zip(&expected) {
            close(*a, *b, 1e-10, "script hsv");
        }
    }

    #[test]
    fn model_reducer_matchdc_dc_gain_and_error_bound_on_dense_grid() {
        let r = reference();
        let sys = &r["balanced4"];
        let hsv = list(&sys["hsv"]);
        let dc = list(&sys["dcgain"])[0];
        let ws = workspace(&define("G", sys));
        let mut host = Host::new(ModelReducer::default());
        read_model(&mut host, &ws);
        let cases = [(2, true), (2, false), (1, true), (3, false)];
        set_balanced(&mut host, 0, 2, true);
        for (k, (order, match_dc)) in cases.iter().enumerate().skip(1) {
            host.app.add(Method::Balanced, &ws).unwrap();
            set_balanced(&mut host, k, *order, *match_dc);
        }
        assert!(host.app.add(Method::Balanced, &ws).is_err(), "at most 4 rows");
        compute(&mut host, &ws).unwrap();
        let mut engine = engine_with(&ws);
        engine.execute(&export(&mut host, &ws)).unwrap();
        engine
            .execute("mrt_w = logspace(-5, 5, 6000)'; mrt_h = freqresp(G, mrt_w);")
            .unwrap();
        let shown = host.app.current.clone().unwrap();
        for ((order, match_dc), (row, result)) in cases
            .iter()
            .zip(host.app.rows.iter().zip(&shown.reduced))
        {
            let name = &row.name;
            assert_eq!(result.order, *order, "{name}");
            if *match_dc {
                // Singular perturbation keeps the DC gain.
                let reduced = values(&mut engine, &format!("dcgain({name})"))[0];
                close(reduced, dc, 1e-10, "reduced DC gain");
                assert!(result.dc_difference <= 1e-10, "{}", result.dc_difference);
            }
            // Glover's bound for both elimination methods on a dense grid.
            let bound = 2. * hsv[*order..].iter().sum::<f64>();
            let error = values(
                &mut engine,
                &format!("max(abs(freqresp({name}, mrt_w) - mrt_h))"),
            )[0];
            assert!(error <= bound * (1. + 1e-9), "{name}: {error} > {bound}");
            // ... and the app's error on its own Bode grid.
            assert!(
                result.error_db <= 20. * bound.log10() + 1e-9,
                "{name}: {} dB",
                result.error_db
            );
            // Truncation keeps the dominant states: the error is at least
            // the first discarded Hankel singular value somewhere.
            assert!(error >= hsv[*order] * (1. - 1e-6), "{name}: {error}");
        }
    }

    #[test]
    fn model_reducer_unstable_modes_marked_and_always_kept() {
        let r = reference();
        let sys = &r["unstable"];
        let ws = workspace(&define("G", sys));
        let mut host = Host::new(ModelReducer::default());
        read_model(&mut host, &ws);
        let info = host.app.info().unwrap().clone();
        assert_eq!(info.unstable(), 1);
        assert!(info.hsv[0].is_infinite());
        for (a, b) in info.hsv[1..].iter().zip(list(&sys["stable_hsv"])) {
            close(*a, b, 1e-9, "stable hsv");
        }
        // Below the unstable count is refused before any run.
        set_balanced(&mut host, 0, 0, true);
        let e = host.request(&ws).err().unwrap();
        assert!(e.contains("at least 1"), "{e}");
        // A click on the first bar keeps it (and the unstable one).
        host.app.click_bar(0.3, &info);
        assert_eq!(host.app.rows[0].order, "1");
        host.app.click_bar(2.6, &info);
        assert_eq!(host.app.rows[0].order, "3");
        let pole = list(&sys["unstable_poles"])[0];
        for order in [1, 2] {
            set_balanced(&mut host, 0, order, order == 2);
            compute(&mut host, &ws).unwrap();
            let reduced = &host.app.current.as_ref().unwrap().reduced[0];
            assert_eq!(reduced.order, order);
            assert!(
                reduced
                    .poles
                    .iter()
                    .any(|[re, im]| (re - pole).abs() <= 1e-10 && im.abs() <= 1e-10),
                "{:?}",
                reduced.poles
            );
            assert!(
                reduced
                    .poles
                    .iter()
                    .filter(|[re, _]| *re > 0.)
                    .count()
                    == 1
            );
        }
    }

    #[test]
    fn model_reducer_mimo_two_by_two() {
        let r = reference();
        let sys = &r["mimo"];
        let ws = workspace(&define("P", sys));
        let mut host = Host::new(ModelReducer::default());
        read_model(&mut host, &ws);
        let info = host.app.info().unwrap().clone();
        assert_eq!((info.outputs, info.inputs, info.states), (2, 2, 5));
        for (a, b) in info.hsv.iter().zip(list(&sys["hsv"])) {
            close(*a, b, 1e-10, "MIMO hsv");
        }
        set_balanced(&mut host, 0, 3, true);
        host.app.add(Method::Modes, &ws).unwrap();
        host.app.rows[1].eliminated = [4, 5].into();
        host.app.output = 1;
        host.app.input = 1;
        compute(&mut host, &ws).unwrap();
        let c = host.app.current.clone().unwrap();
        assert_eq!(c.channel, (1, 1));
        assert_eq!(c.reduced[0].order, 3);
        assert_eq!(c.reduced[1].order, 3);
        assert!(c.reduced[0].dc_difference <= 1e-10);
        assert!(c.reduced[1].dc_difference <= 1e-10);
        let mut engine = engine_with(&ws);
        engine.execute(&export(&mut host, &ws)).unwrap();
        let dc = list(&sys["dcgain"]);
        for name in ["rsys", "rsys1"] {
            for (a, b) in values(&mut engine, &format!("dcgain({name})"))
                .iter()
                .zip(&dc)
            {
                close(*a, *b, 1e-10, name);
            }
        }
        // Output 2 <- input 2 lives on the second input page of step,
        // and at (2,2) on every frequency page of Bode.
        engine
            .execute("[~,mrt_t]=step(P); [~,~,mrt_w]=bode(P);")
            .unwrap();
        for (name, result) in [
            ("P", &c.original),
            ("rsys", &c.reduced[0]),
            ("rsys1", &c.reduced[1]),
        ] {
            engine
                .execute(&format!(
                    "mrt_y=step({name},mrt_t); [mrt_m,mrt_p]=bode({name},mrt_w);"
                ))
                .unwrap();
            let nt = result.step.len();
            let step = values(&mut engine, "mrt_y");
            assert_eq!(step.len(), 4 * nt);
            let magnitude = values(&mut engine, "mrt_m");
            let phase = values(&mut engine, "mrt_p");
            let magnitude_db: Vec<_> = magnitude
                .iter()
                .skip(3)
                .step_by(4)
                .map(|m| 20. * m.log10())
                .collect();
            let phase: Vec<_> = phase.into_iter().skip(3).step_by(4).collect();
            for (curve, expected) in [
                (&result.step, &step[3 * nt..]),
                (&result.magnitude, magnitude_db.as_slice()),
                (&result.phase, phase.as_slice()),
            ] {
                assert_eq!(curve.len(), expected.len(), "{name}");
                for ([_, actual], &want) in curve.iter().zip(expected) {
                    close(*actual, want, 1e-12, name);
                }
            }
        }
        assert_eq!(c.original.poles.len(), 5);
    }

    #[test]
    fn model_reducer_generated_script_in_fresh_engine_gives_exported_models() {
        let r = reference();
        let setup = define("G", &r["balanced4"]);
        let ws = workspace(&format!("{setup}mr_reduced = 1; mr_hsv = 2;"));
        let mut host = Host::new(ModelReducer::default());
        read_model(&mut host, &ws);
        set_balanced(&mut host, 0, 2, true);
        host.app.add(Method::Modes, &ws).unwrap();
        host.app.rows[1].eliminated = [3, 4].into();
        host.app.rows[1].match_dc = false;
        host.app.add(Method::PoleZero, &ws).unwrap();
        host.app.rows[2].tolerance = "1e-6".into();
        host.app.add(Method::Balanced, &ws).unwrap();
        set_balanced(&mut host, 3, 1, false);
        host.app.rows[3].export = false;
        host.app.rows[0].name = "balanced".into();
        let outputs = compute(&mut host, &ws).unwrap();
        let script = host.app.script(&ws).unwrap();
        for call in [
            "opts = balredOptions('StateProjection', 'MatchDC')",
            "balred(G, 2, opts)",
            "modred(ss(G), [3 4], 'Truncate')",
            "minreal(G, 1e-6)",
            "opts = balredOptions('StateProjection', 'Truncate')",
            "balred(G, 1, opts)",
            "figure; step(",
            "figure; bode(",
            "figure; pzmap(",
        ] {
            assert!(script.contains(call), "{call} missing from\n{script}");
        }
        assert!(
            !script.contains("StateElimMethod"),
            "app script must not emit the old balred name/value form:\n{script}"
        );
        let saved = match host.generate(&ws, 42).unwrap() {
            toolkits::Action::SaveAppScript(AppKind::ModelReducer, text) => text,
            _ => panic!("wrong save action"),
        };
        assert!(saved.ends_with(&script));
        let mut fresh = matrust::Engine::default();
        fresh.execute(&setup).unwrap();
        fresh.execute(&saved).unwrap();
        let mut exported = engine_with(&ws);
        exported.execute(&export(&mut host, &ws)).unwrap();
        let names = host.app.exported(&ws);
        assert_eq!(names, ["balanced", "rsys1", "rsys2"]);
        assert!(!fresh.contains_array("rsys3") && !exported.contains_array("rsys3"));
        for name in &names {
            for field in ["A", "B", "C", "D"] {
                let expr = format!("{name}.{field}");
                let a = values(&mut fresh, &expr);
                let b = values(&mut exported, &expr);
                assert_eq!(a, b, "{expr}");
            }
        }
        assert_eq!(values(&mut fresh, "size(balanced.A, 1)"), [2.]);
        assert_eq!(values(&mut fresh, "size(rsys1.A, 1)"), [2.]);
        // Workspace names the script avoided survive.
        assert_eq!(values(&mut exported, "mr_hsv"), [2.]);
        fresh.execute(&host.app.probe(&ws).0).unwrap();
        for (name, array) in outputs {
            let expected = match array {
                matrust::Array::Numeric(m) => m.data().to_vec(),
                _ => unreachable!(),
            };
            let actual = values(&mut fresh, &name);
            assert_eq!(actual.len(), expected.len(), "{name}");
            for (a, b) in actual.iter().zip(expected) {
                assert!(*a == b || a.is_nan() && b.is_nan(), "{name}");
            }
        }
    }

    #[test]
    fn model_reducer_export_names_checked_and_rollback() {
        let r = reference();
        let ws = workspace(&format!("{}rsys = 7;", define("G", &r["balanced4"])));
        let mut host = Host::new(ModelReducer::default());
        read_model(&mut host, &ws);
        host.app.add(Method::PoleZero, &ws).unwrap();
        assert_eq!(host.app.rows[1].name, "rsys1");
        for name in [
            "mrexp_old",
            "mrexp_had",
            "matrust_export_error",
            "mrexp_complete",
            "1bad",
            "for",
            "x);bad=1;%",
            "",
            "bode",
            "step",
        ] {
            host.app.rows[0].name = name.into();
            assert!(host.app.script(&ws).is_err(), "{name}");
        }
        host.app.rows[0].name = host.app.rows[1].name.clone();
        assert!(host.app.script(&ws).unwrap_err().contains("duplicate"));
        host.app.rows[0].name = "rsys".into();
        compute(&mut host, &ws).unwrap();
        // A changed model at export time fails; both names roll back.
        let code = export(&mut host, &ws);
        let mut engine = engine_with(&ws);
        engine.execute("G = 5;").unwrap();
        assert!(engine.execute(&code).is_err());
        assert_eq!(values(&mut engine, "rsys"), [7.]);
        assert!(!engine.contains_array(&host.app.rows[1].name));
        // A model that is not in the workspace is refused.
        host.app.model = "missing".into();
        assert!(host.request(&ws).is_err());
    }

    #[test]
    fn model_reducer_fields_parse_as_one_literal_and_last_result_stays() {
        let r = reference();
        let ws = workspace(&define("G", &r["balanced4"]));
        let mut host = Host::new(ModelReducer::default());
        read_model(&mut host, &ws);
        compute(&mut host, &ws).unwrap();
        let good = host.app.current.as_ref().unwrap().reduced[0].step.clone();
        for bad in [
            "2); x=1; %",
            "2\nerror('boom')",
            "1.5",
            "-1",
            "Inf",
            "NaN",
            "5",
            "1+1",
            "",
        ] {
            host.app.rows[0].order = bad.into();
            assert!(host.request(&ws).is_err(), "order {bad}");
        }
        host.app.rows[0].order = "2".into();
        host.app.rows[0].method = Method::PoleZero;
        for bad in ["1e-6); evil; %", "-1", "0", "eps", "1e-6\nerror('x')"] {
            host.app.rows[0].tolerance = bad.into();
            assert!(host.request(&ws).is_err(), "tolerance {bad}");
        }
        host.app.rows[0].tolerance = String::new();
        assert!(host.app.script(&ws).unwrap().contains("minreal(G);"));
        host.app.rows[0].method = Method::Modes;
        host.app.rows[0].eliminated = [5].into();
        assert!(host.request(&ws).err().unwrap().contains("State 5"));
        // An error reaches the status line and leaves the last curves.
        let ctx = egui::Context::default();
        host.refresh(&ctx, &ws);
        assert_eq!(host.status().1, toolkits::AppStatus::Error);
        assert_eq!(host.app.current.as_ref().unwrap().reduced[0].step, good);
        // Stop holds the run until an input changes.
        host.app.rows[0].eliminated.clear();
        host.app.halt();
        host.app.prepare(&ws);
        assert!(host.app.script(&ws).unwrap_err().contains("stopped"));
        host.app.rows[0].match_dc = false;
        host.app.prepare(&ws);
        assert!(host.app.script(&ws).is_ok());
        host.app.halt();
        host.app.resume();
        assert!(host.app.script(&ws).is_ok());
    }

    #[test]
    fn model_reducer_discrete_and_tf_models() {
        let ws = workspace(
            "H = tf([1 3], [1 6 11 6]); Gd = c2d(ss(tf([1 2], [1 4 5 2])), 0.1);",
        );
        for (model, states) in [("H", 3), ("Gd", 3)] {
            let mut host = Host::new(ModelReducer::default());
            host.app.model = model.into();
            read_model(&mut host, &ws);
            assert_eq!(host.app.info().unwrap().states, states);
            set_balanced(&mut host, 0, 1, true);
            compute(&mut host, &ws).unwrap_or_else(|e| panic!("{model}: {e}"));
            let c = host.app.current.as_ref().unwrap();
            assert!(c.reduced[0].dc_difference <= 1e-9, "{model}");
            assert_eq!(c.reduced[0].order, 1);
        }
    }

    fn open_reducer(app: &mut Workspace, ctx: &egui::Context, size: egui::Vec2) {
        app.toolkits.open_tool(Tool::App(AppKind::ModelReducer));
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(60);
        loop {
            frame_at(app, ctx, size, vec![]);
            let host = &app.toolkits.apps.reducer;
            if host.app.current.is_some() && !host.running() {
                return;
            }
            assert!(
                std::time::Instant::now() < deadline,
                "{:?}",
                host.status()
            );
            std::thread::sleep(std::time::Duration::from_millis(5));
        }
    }

    #[test]
    fn model_reducer_tool_tab_and_headless_frames_0_1_50_states() {
        let kind = AppKind::ModelReducer;
        let tab = kind.tab();
        assert_eq!(AppKind::from_tab(tab), Some(kind));
        assert_eq!(serde_json::to_string(&tab).unwrap(), "\"model_reducer\"");
        let decoded: matrust::session::ToolTab =
            serde_json::from_str("\"model_reducer\"").unwrap();
        assert_eq!(decoded, tab);
        assert!(kind.is_built() && AppKind::ALL.contains(&kind));
        // No model: the frame renders and asks for one.
        for width in [820., 1600.] {
            let ctx = egui::Context::default();
            let mut desktop = Workspace::with_context(ctx.clone(), true);
            desktop.toolkits.open_tool(Tool::App(kind));
            let output = frame_at(&mut desktop, &ctx, egui::vec2(width, 950.), vec![]);
            assert!(!output.shapes.is_empty());
            let (text, status) = desktop.toolkits.apps.reducer.status();
            assert_eq!(status, toolkits::AppStatus::Error);
            assert!(text.contains("choose"), "{text}");
            desktop.close_tool_tab(Tool::App(kind));
        }
        let models = [
            ("one", "one = ss(-2, 1, 3, 0);", 1),
            (
                "big",
                "big = ss(-diag(1:50) + diag(0.5*ones(49,1), 1), ones(50,1), (1:50)/50, 0);",
                50,
            ),
        ];
        for (name, code, states) in models {
            let mut engine = matrust::Engine::default();
            engine.execute(code).unwrap();
            for width in [820., 1600.] {
                let ctx = egui::Context::default();
                let mut desktop = Workspace::with_context(ctx.clone(), true);
                desktop
                    .variables
                    .insert(name.into(), engine.array(name).unwrap());
                let size = egui::vec2(width, 950.);
                open_reducer(&mut desktop, &ctx, size);
                let host = &desktop.toolkits.apps.reducer;
                assert_eq!(host.app.model, name);
                assert_eq!(host.app.info().unwrap().states, states);
                assert_eq!(host.status().1, toolkits::AppStatus::Success);
                let c = host.app.current.as_ref().unwrap();
                assert_eq!(c.original.order, states);
                // Balanced truncation also drops numerically zero HSVs.
                assert!((1..=states.div_ceil(2)).contains(&c.reduced[0].order));
                for view in View::ALL {
                    desktop.toolkits.apps.reducer.app.view = view;
                    let output = frame_at(&mut desktop, &ctx, size, vec![]);
                    assert!(!output.shapes.is_empty());
                }
                assert_eq!(desktop.toolkits.selected_tool(), Some(Tool::App(kind)));
                desktop.close_tool_tab(Tool::App(kind));
            }
        }
    }
}
