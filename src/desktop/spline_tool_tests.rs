// Independent references: tool/refs/spline_tool/reference.py (SciPy/NumPy).
mod spline_tool_tests {
    use super::*;
    use apps::spline_tool::{
        Ends, MAX_GRID, MAX_KNOTS, MAX_SITES, MAX_SPLINES, Method, SplineTool, View, parse_knots,
    };
    use apps::{App, AppKind, Host, Snapshot};
    use serde_json::Value;
    use toolkits::Tool;

    fn reference() -> Value {
        serde_json::from_str(include_str!(
            "../../tests/fixtures/apps/spline_tool/reference.json"
        ))
        .unwrap()
    }
    fn numbers(v: &Value) -> Vec<f64> {
        v.as_array()
            .unwrap()
            .iter()
            .map(|v| v.as_f64().unwrap())
            .collect()
    }
    fn row(v: &Value) -> String {
        numbers(v)
            .into_iter()
            .map(apps::number)
            .collect::<Vec<_>>()
            .join(" ")
    }
    fn workspace(code: &str) -> Snapshot {
        let mut e = matrust::Engine::default();
        e.execute(code).unwrap();
        let mut ws = Snapshot::default();
        ws.sync(&e.workspace());
        ws
    }
    fn fixture() -> Snapshot {
        let r = reference();
        workspace(&format!(
            "x=[{}]; y=[{}]; w=[{}];",
            row(&r["x"]),
            row(&r["y"]),
            row(&r["weights"])
        ))
    }
    fn host() -> Host<SplineTool> {
        let mut h = Host::new(SplineTool::default());
        h.app.fits[0].grid_count = 41;
        h
    }
    fn compute(h: &mut Host<SplineTool>, ws: &Snapshot) -> Result<matrust::Engine, String> {
        h.app.prepare(ws);
        let mut e = matrust::Engine::default();
        let output = apps::run(&mut e, h.request(ws)?)?;
        h.app.receive(&output)?;
        Ok(e)
    }
    fn values(e: &matrust::Engine, name: &str) -> Vec<f64> {
        match e.array(name).unwrap() {
            matrust::Array::Numeric(m) => m.data().to_vec(),
            _ => panic!("non-numeric {name}"),
        }
    }
    fn close(a: &[f64], b: &[f64], tol: f64) {
        assert_eq!(a.len(), b.len());
        for (i, (a, b)) in a.iter().zip(b).enumerate() {
            assert!(
                (a - b).abs() <= tol * b.abs().max(1.),
                "at {i}: {a:.16e} != {b:.16e}, tolerance {tol}"
            );
        }
    }
    fn check_case(h: &Host<SplineTool>, name: &str, tol: f64) {
        let r = reference();
        let expected = &r["cases"][name];
        let c = &h.app.current[0];
        close(&c.values, &numbers(&expected["values"]), tol);
        close(&c.first, &numbers(&expected["first"]), tol * 10.);
        close(&c.second, &numbers(&expected["second"]), tol * 20.);
        close(&c.residuals, &numbers(&expected["residuals"]), tol);
        close(
            &c.metrics,
            &[
                expected["residual_norm"].as_f64().unwrap(),
                expected["roughness"].as_f64().unwrap(),
            ],
            tol * 30.,
        );
    }

    #[test]
    fn spline_tool_csapi_through_points_not_a_knot_and_end_conditions_scipy() {
        let ws = fixture();
        for (ends, name) in [
            (Ends::NotAKnot, "not_a_knot"),
            (Ends::Natural, "natural"),
            (Ends::Clamped, "clamped"),
            (Ends::Second, "second"),
        ] {
            let mut h = host();
            let f = &mut h.app.fits[0];
            f.ends = ends;
            (f.left, f.right) = if ends == Ends::Second {
                ("0.3".into(), "-0.2".into())
            } else {
                ("0.25".into(), "-0.5".into())
            };
            compute(&mut h, &ws).unwrap();
            check_case(&h, name, 1e-12);
            assert!(h.app.current[0].residuals.iter().all(|v| v.abs() <= 1e-12));
        }
        let ws = workspace("x=[0 1 2 3 4]; y=[1 0 -1 0 1];");
        let mut h = host();
        h.app.fits[0].ends = Ends::Periodic;
        let mut e = compute(&mut h, &ws).unwrap();
        e.execute("d=fnval(fnder(spline1),[0 4]); dd=fnval(fnder(spline1,2),[0 4]);")
            .unwrap();
        let d = values(&e, "d");
        let dd = values(&e, "dd");
        close(&d[..1], &d[1..], 1e-12);
        close(&dd[..1], &dd[1..], 1e-12);
    }

    #[test]
    fn spline_tool_csaps_endpoints_weighted_smoothing_and_spaps_scipy() {
        let ws = fixture();
        for (p, name) in [("0.35", "p_0.35"), ("0.9", "p_0.9"), ("1", "p_1.0")] {
            let mut h = host();
            let f = &mut h.app.fits[0];
            f.method = Method::Smooth;
            f.weights = "w".into();
            f.p = p.into();
            compute(&mut h, &ws).unwrap();
            check_case(&h, name, 3e-11);
            if p == "1" {
                assert!(h.app.current[0].residuals.iter().all(|v| v.abs() <= 1e-12));
            }
        }
        let mut h = host();
        h.app.fits[0].method = Method::Smooth;
        h.app.fits[0].p = "0".into();
        h.app.fits[0].weights = "w".into();
        compute(&mut h, &ws).unwrap();
        close(
            &h.app.current[0].values,
            &numbers(&reference()["cases"]["line"]["values"]),
            1e-12,
        );
        assert!(h.app.current[0].second.iter().all(|v| v.abs() < 1e-12));
        close(&h.app.current[0].metrics[1..], &[0.], 1e-12);
        let mut h = host();
        h.app.fits[0].method = Method::Tolerance;
        h.app.fits[0].tolerance = apps::number(reference()["tolerance"].as_f64().unwrap());
        compute(&mut h, &ws).unwrap();
        check_case(&h, "tolerance", 5e-8);
    }

    #[test]
    fn spline_tool_spap2_given_knots_weighted_and_pchip_scipy() {
        let ws = fixture();
        for (weighted, name) in [(false, "lsq"), (true, "weighted_lsq")] {
            let mut h = host();
            let f = &mut h.app.fits[0];
            f.method = Method::LeastSquares;
            f.knots = "[1 2.5]".into();
            if weighted {
                f.weights = "w".into();
            }
            compute(&mut h, &ws).unwrap();
            check_case(&h, name, 2e-11);
            close(&h.app.current[0].knots, &numbers(&reference()["knots"]), 0.);
        }
        let mut h = host();
        h.app.fits[0].method = Method::Pchip;
        compute(&mut h, &ws).unwrap();
        check_case(&h, "pchip", 2e-11);
        for order in 2..=8 {
            let mut h = host();
            h.app.fits[0].method = Method::LeastSquares;
            h.app.fits[0].order = order.to_string();
            compute(&mut h, &ws).unwrap();
            check_case(&h, &format!("order_{order}"), 2e-9);
        }
    }

    #[test]
    fn spline_tool_fnder_fnint_analytic_cubic_pp_and_b_form() {
        let r = reference();
        let ws = workspace(&format!(
            "x=[{}]; y=[{}];",
            row(&r["x"]),
            row(&r["cubic"]["y"])
        ));
        for method in [Method::Cubic, Method::LeastSquares] {
            let mut h = host();
            h.app.fits[0].method = method;
            let mut e = compute(&mut h, &ws).unwrap();
            close(
                &h.app.current[0].values,
                &numbers(&r["cubic"]["values"]),
                2e-12,
            );
            close(
                &h.app.current[0].first,
                &numbers(&r["cubic"]["first"]),
                2e-12,
            );
            close(
                &h.app.current[0].second,
                &numbers(&r["cubic"]["second"]),
                2e-12,
            );
            e.execute("integral=fnval(fnint(spline1),linspace(0,4,41));")
                .unwrap();
            close(
                &values(&e, "integral"),
                &numbers(&r["cubic"]["integral"]),
                2e-12,
            );
        }
    }

    #[test]
    fn spline_tool_script_preview_atomic_export_save_replay_exact_multirow() {
        let ws = workspace(
            "x=[0 .3 .9 1.5 2 2.8 3.4 4]; y=[1 1.8 -.2 .4 2.1 1.2 .5 1.3; 2 1 0 3 2 4 1 2]; st_spline1=8; st_roughness1=9;",
        );
        let mut h = host();
        for method in [
            Method::Smooth,
            Method::Tolerance,
            Method::LeastSquares,
            Method::Pchip,
        ] {
            h.app.add(&ws).unwrap();
            h.app.fits[h.app.selected].method = method;
        }
        assert_eq!(h.app.fits.len(), MAX_SPLINES);
        assert!(h.app.add(&ws).is_err());
        let preview = compute(&mut h, &ws).unwrap();
        let pchip = h
            .app
            .current
            .iter()
            .find(|c| c.method == Method::Pchip)
            .unwrap();
        let expected = &reference()["multi_pchip"];
        close(&pchip.values, &numbers(&expected["values"]), 2e-12);
        close(&pchip.first, &numbers(&expected["first"]), 2e-12);
        close(&pchip.second, &numbers(&expected["second"]), 2e-12);
        let script = h.app.script(&ws).unwrap();
        assert!(
            script.contains("figure; plot(")
                && script.contains("spap2(")
                && script.contains("csapi(")
        );
        let saved = match h.generate(&ws, 42).unwrap() {
            toolkits::Action::SaveAppScript(AppKind::SplineTool, s) => s,
            _ => panic!("save action"),
        };
        assert!(saved.ends_with(&script));
        let export = match h.export(&ws).unwrap() {
            toolkits::Action::ExportScript(s) => s,
            _ => panic!("export action"),
        };
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("spline_tool.m");
        std::fs::write(&path, &saved).unwrap();
        let from_disk = std::fs::read_to_string(&path).unwrap();
        for text in [&from_disk, &export] {
            let mut fresh = matrust::Engine::default();
            for (name, a) in ws.variables() {
                fresh.insert_array(name, a.clone()).unwrap();
            }
            fresh.execute(text).unwrap();
            for f in &h.app.fits {
                assert_eq!(
                    values(&fresh, &f.values_name),
                    values(&preview, &f.values_name)
                );
                fresh
                    .execute(&format!("check=fnval({},linspace(0,4,41));", f.spline_name))
                    .unwrap();
                assert_eq!(values(&fresh, "check"), values(&preview, &f.values_name));
                assert_eq!(
                    fresh.array(&f.values_name).unwrap().dims().as_ref(),
                    &[2, 41]
                );
            }
        }
        // Atomic rollback on a later construction failure, including pre-existing names.
        let mut fresh = matrust::Engine::default();
        fresh
            .execute("x=[0 1 2]; y=[1 2 1]; spline1=73; spline_values1=74;")
            .unwrap();
        assert!(fresh.execute(&export).is_err());
        assert_eq!(values(&fresh, "spline1"), [73.]);
        assert_eq!(values(&fresh, "spline_values1"), [74.]);
        for f in h.app.fits.iter().skip(1) {
            assert!(fresh.array(&f.spline_name).is_none());
        }
    }

    #[test]
    fn spline_tool_names_expression_parser_limits_and_last_good() {
        let ws = fixture();
        let mut h = host();
        compute(&mut h, &ws).unwrap();
        let before = h.app.current[0].values.clone();
        for name in [
            "bad name",
            "1name",
            "end",
            "x); hacked=1; %",
            "mrexp_old",
            "mrexp_had",
            "matrust_export_error",
            "mrexp_complete",
        ] {
            h.app.fits[0].spline_name = name.into();
            assert!(h.request(&ws).is_err(), "{name}");
        }
        h.app.fits[0].spline_name = "spline1".into();
        h.app.fits[0].values_name = "spline1".into();
        assert!(h.request(&ws).is_err());
        h.app.fits[0].values_name = "spline_values1".into();
        for bad in [
            "0.5); hacked=1; %",
            "0.5\nerror('bad')",
            "NaN",
            "Inf",
            "1+1",
            "-1",
            "1.1",
        ] {
            h.app.fits[0].method = Method::Smooth;
            h.app.fits[0].p = bad.into();
            assert!(h.request(&ws).is_err(), "{bad}");
        }
        h.app.fits[0].p = "0.9".into();
        for bad in ["x); hacked=1; %", "[1 2]"] {
            h.app.fits[0].x = bad.into();
            assert!(h.request(&ws).is_err());
        }
        h.app.fits[0].x = "x".into();
        h.app.fits[0].grid_start = "0); hacked=1; %".into();
        assert!(h.request(&ws).is_err());
        h.app.fits[0].grid_start.clear();
        h.app.fits[0].grid_end = "5".into();
        assert!(compute(&mut h, &ws).is_err());
        h.app.fits[0].grid_end.clear();
        h.app.fits[0].grid_count = MAX_GRID + 1;
        assert!(h.request(&ws).is_err());
        h.app.fits[0].grid_count = 41;
        h.app.fits[0].method = Method::LeastSquares;
        for bad in [
            "[1 1]",
            "[2 1]",
            "[0]",
            "[4]",
            "[1]; hacked=1",
            "linspace(0,4,10000000)",
        ] {
            h.app.fits[0].knots = bad.into();
            assert!(h.request(&ws).is_err(), "{bad}");
        }
        h.app.fits[0].knots = "[]".into();
        h.app.fits[0].order = "100".into();
        assert!(h.request(&ws).is_err());
        let too_many = format!(
            "[{}]",
            (1..=MAX_KNOTS + 1)
                .map(|v| v.to_string())
                .collect::<Vec<_>>()
                .join(" ")
        );
        assert!(parse_knots(&too_many).is_err());
        // Real worker error and validation error keep the old view.
        h.app.fits[0].order = "8".into();
        h.app.fits[0].knots = "[1 2 3]".into();
        assert!(compute(&mut h, &ws).is_err());
        assert_eq!(h.app.current[0].values, before);
        assert!(h.app.receive(&apps::Outputs::new()).is_err());
        assert_eq!(h.app.current[0].values, before);
        h.app.fits[0].p = "NaN".into();
        h.app.fits[0].method = Method::Smooth;
        h.refresh(&egui::Context::default(), &ws);
        assert_eq!(h.status().1, toolkits::AppStatus::Error);
        assert_eq!(h.app.current[0].values, before);
    }

    #[test]
    fn spline_tool_knots_add_remove_drag_sorted_outside_refused() {
        let ws = fixture();
        let mut h = host();
        h.app.fits[0].method = Method::LeastSquares;
        h.app.add_knot(2.5, (0., 4.)).unwrap();
        h.app.add_knot(1., (0., 4.)).unwrap();
        assert_eq!(parse_knots(&h.app.fits[0].knots).unwrap(), [1., 2.5]);
        h.app.move_knot(0, 3., (0., 4.)).unwrap();
        assert_eq!(parse_knots(&h.app.fits[0].knots).unwrap(), [2.5, 3.]);
        let before = h.app.fits[0].knots.clone();
        for v in [-1., 0., 4., 5., f64::NAN] {
            assert!(h.app.move_knot(0, v, (0., 4.)).is_err());
            assert_eq!(h.app.fits[0].knots, before);
        }
        assert!(h.app.move_knot(0, 3., (0., 4.)).is_err());
        h.app.remove_knot(1).unwrap();
        compute(&mut h, &ws).unwrap();
        assert_eq!(parse_knots(&h.app.fits[0].knots).unwrap(), [2.5]);
    }

    #[test]
    fn spline_tool_headless_pointer_drag_changes_knot_and_refuses_outside() {
        let ws = fixture();
        let mut h = host();
        h.app.fits[0].method = Method::LeastSquares;
        h.app.fits[0].knots = "[1 2.5]".into();
        compute(&mut h, &ws).unwrap();
        let ctx = egui::Context::default();
        crate::theme::configure(&ctx);
        let frame = |app: &mut SplineTool, events: Vec<egui::Event>| {
            crate::headless::run(
                &ctx,
                egui::RawInput {
                    screen_rect: Some(egui::Rect::from_min_size(
                        egui::Pos2::ZERO,
                        egui::vec2(800., 600.),
                    )),
                    events,
                    ..Default::default()
                },
                |ui| app.main_ui(ui),
            )
        };
        frame(&mut h.app, vec![]);
        frame(&mut h.app, vec![]);
        let transform =
            egui_plot::PlotMemory::load(&ctx, egui::Id::new(("spline_tool_plot", "main")))
                .unwrap()
                .transform();
        let at = |x| transform.position_from_point(&egui_plot::PlotPoint::new(x, 1.));
        let start = at(1.);
        frame(
            &mut h.app,
            vec![
                egui::Event::PointerMoved(start),
                egui::Event::PointerButton {
                    pos: start,
                    button: egui::PointerButton::Primary,
                    pressed: true,
                    modifiers: egui::Modifiers::NONE,
                },
            ],
        );
        // Cross egui's drag threshold while still near the knot, then drag.
        frame(
            &mut h.app,
            vec![egui::Event::PointerMoved(start + egui::vec2(8., 0.))],
        );
        frame(&mut h.app, vec![egui::Event::PointerMoved(at(1.5))]);
        close(
            &parse_knots(&h.app.fits[0].knots).unwrap(),
            &[1.5, 2.5],
            1e-5,
        );
        let before = h.app.fits[0].knots.clone();
        frame(&mut h.app, vec![egui::Event::PointerMoved(at(-0.5))]);
        assert_eq!(h.app.fits[0].knots, before);
        assert!(h.app.script(&ws).unwrap_err().contains("strictly inside"));
        frame(
            &mut h.app,
            vec![egui::Event::PointerButton {
                pos: at(-0.5),
                button: egui::PointerButton::Primary,
                pressed: false,
                modifiers: egui::Modifiers::NONE,
            }],
        );
        h.app.run();
        assert!(h.app.script(&ws).is_ok());
    }

    #[test]
    fn spline_tool_input_shapes_nonfinite_weights_and_bounds() {
        for code in [
            "x=[0 0 1]; y=[1 2 3];",
            "x=[2 1 0]; y=[1 2 3];",
            "x=[0 1 2]; y=[1 2];",
            "x=[0 1 NaN]; y=[1 2 3];",
            "x=[0 1 2]; y=[1 Inf 3];",
            "x=0; y=1;",
            "x=[0 1; 2 3]; y=[1 2 3 4];",
        ] {
            assert!(host().request(&workspace(code)).is_err(), "{code}");
        }
        for weights in ["[1 -1 1]", "[0 0 0]", "[1 0 0]", "[1 2]", "[1 NaN 1]"] {
            let ws = workspace(&format!("x=[0 1 2]; y=[1 2 3]; w={weights};"));
            let mut h = host();
            h.app.fits[0].method = Method::Smooth;
            h.app.fits[0].weights = "w".into();
            assert!(h.request(&ws).is_err());
        }
        let ws = workspace(&format!("x=linspace(0,1,{}); y=x;", MAX_SITES + 1));
        assert!(host().request(&ws).is_err());
        let ws = workspace("x=[0 1 2]; y=ones(9,3);");
        assert!(host().request(&ws).is_err());
    }

    fn wait(h: &mut Host<SplineTool>, ctx: &egui::Context, ws: &Snapshot) {
        let until = Instant::now() + Duration::from_secs(30);
        loop {
            h.refresh(ctx, ws);
            if !h.running() {
                break;
            }
            assert!(Instant::now() < until, "runner timed out");
            std::thread::sleep(Duration::from_millis(5));
        }
    }
    #[test]
    fn spline_tool_runner_stop_resume_and_input_versions() {
        let ws = fixture();
        let ctx = egui::Context::default();
        let mut h = host();
        wait(&mut h, &ctx, &ws);
        assert!(!h.app.current.is_empty());
        let before = h.app.current[0].values.clone();
        h.app.fits[0].p = "0.8".into();
        h.app.fits[0].method = Method::Smooth;
        h.refresh(&ctx, &ws);
        h.app.stop(&ws);
        wait(&mut h, &ctx, &ws);
        assert!(h.status().0.contains("stopped"));
        assert_eq!(h.app.current[0].values, before);
        h.app.run();
        wait(&mut h, &ctx, &ws);
        assert_eq!(h.status().1, toolkits::AppStatus::Success);
        h.app.stop(&ws);
        wait(&mut h, &ctx, &ws);
        let changed = workspace("x=[0 1 2 3]; y=[1 2 0 1];");
        // Sync through the same Snapshot to preserve monotonic versions.
        let mut ws = ws;
        let map = changed
            .variables()
            .map(|(n, a)| (n.to_owned(), a.clone()))
            .collect();
        ws.sync(&map);
        wait(&mut h, &ctx, &ws);
        assert_eq!(h.status().1, toolkits::AppStatus::Success);
        assert_eq!(h.app.current[0].x.len(), 4);
    }

    #[test]
    fn spline_tool_tool_tab_restore_and_headless_frames_empty_three_ten_thousand() {
        let kind = AppKind::SplineTool;
        assert_eq!(AppKind::from_tab(kind.tab()), Some(kind));
        assert_eq!(
            serde_json::to_string(&kind.tab()).unwrap(),
            "\"spline_tool\""
        );
        let decoded: matrust::session::ToolTab = serde_json::from_str("\"spline_tool\"").unwrap();
        assert_eq!(decoded, kind.tab());
        for n in [0, 3, 10_000] {
            let ws = if n == 0 {
                Snapshot::default()
            } else {
                workspace(&format!("x=linspace(0,1,{n}); y=sin(x);"))
            };
            for width in [820., 1600.] {
                let ctx = egui::Context::default();
                let mut desktop = Workspace::with_context(ctx.clone(), true);
                desktop.toolkits.open_tool(Tool::App(kind));
                let vars = ws
                    .variables()
                    .map(|(n, a)| (n.to_owned(), a.clone()))
                    .collect();
                desktop.variables = vars;
                desktop.toolkits.apps.workspace.sync(&desktop.variables);
                if n > 0 {
                    compute(&mut desktop.toolkits.apps.spline, &ws).unwrap();
                }
                for view in [View::Spline, View::First, View::Second, View::Residuals] {
                    desktop.toolkits.apps.spline.app.view = view;
                    let output = frame_at(&mut desktop, &ctx, egui::vec2(width, 950.), vec![]);
                    assert!(!output.shapes.is_empty());
                    assert_eq!(desktop.toolkits.selected_tool(), Some(Tool::App(kind)));
                }
                desktop.close_tool_tab(Tool::App(kind));
            }
        }
    }

    #[test]
    fn spline_tool_release_workload_1_and_n_workers() {
        let ws = workspace("x=linspace(0,10,10000); y=[sin(x);cos(x)];");
        let n = std::thread::available_parallelism()
            .map_or(1, |n| n.get())
            .min(8);
        for workers in [1, n] {
            let pool = rayon::ThreadPoolBuilder::new()
                .num_threads(workers)
                .build()
                .unwrap();
            pool.install(|| {
                for method in Method::ALL {
                    let mut h = host();
                    h.app.fits[0].method = method;
                    let start = Instant::now();
                    compute(&mut h, &ws).unwrap();
                    let elapsed = start.elapsed();
                    assert_eq!(h.app.current[0].x.len(), 10_000);
                    eprintln!(
                        "spline_tool workload method={method:?} workers={workers} sites=10000 components=2 grid=41 elapsed={elapsed:?}"
                    );
                }
            });
        }
    }
}
