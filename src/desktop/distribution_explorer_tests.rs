mod distribution_explorer_tests {
    use super::*;
    use apps::distribution_explorer::{DistributionExplorer, Family, Function, Mode};
    use apps::{App, AppKind, Host, Snapshot};
    use toolkits::{Action, Tool};

    #[derive(serde::Deserialize)]
    struct Reference {
        family: String,
        parameters: Vec<f64>,
        x: f64,
        p: f64,
        stats: Vec<f64>,
    }
    fn references() -> Vec<Reference> {
        serde_json::from_str(include_str!(
            "../../tests/fixtures/distribution_explorer/expected.json"
        ))
        .unwrap()
    }
    fn close(a: f64, b: f64) {
        assert!(
            (a - b).abs() < 2e-8 * b.abs().max(1.),
            "got {a:.15}, want {b:.15}"
        );
    }
    fn same_values(a: &[f64], b: &[f64]) {
        assert_eq!(a.len(), b.len());
        for (a, b) in a.iter().zip(b) {
            assert!(a == b || (a.is_nan() && b.is_nan()), "{a} != {b}");
        }
    }
    fn values(engine: &matrust::Engine, name: &str) -> Vec<f64> {
        match engine.array(name).unwrap() {
            matrust::Array::Numeric(m) => m.data().to_vec(),
            a => panic!("{name}: {}", a.class()),
        }
    }
    fn compute(host: &mut Host<DistributionExplorer>) -> Result<(), String> {
        let req = host.request(&Snapshot::default())?;
        let out = apps::run(&mut matrust::Engine::default(), req)?;
        host.app.receive(&out)
    }
    fn configured(r: &Reference) -> Host<DistributionExplorer> {
        let mut app = DistributionExplorer::default();
        app.set_family(
            Family::ALL
                .into_iter()
                .find(|f| f.name() == r.family)
                .unwrap(),
        );
        for (c, v) in app.parameters.iter_mut().zip(&r.parameters) {
            c.value = apps::number(*v);
        }
        app.probe_x = apps::number(r.x);
        app.probe_p = apps::number(r.p);
        Host::new(app)
    }
    #[test]
    fn distribution_explorer_all_families_scipy_and_fresh_generated_script() {
        let refs = references();
        assert_eq!(refs.len(), Family::ALL.len());
        for r in refs {
            let mut host = configured(&r);
            compute(&mut host).unwrap_or_else(|e| panic!("{}: {e}", r.family));
            let shown = host.app.shown.as_ref().unwrap().clone();
            for (a, b) in shown.stats.iter().zip(&r.stats) {
                close(*a, *b);
            }
            let mut engine = matrust::Engine::default();
            let Action::SaveAppScript(kind, saved) =
                host.generate(&Snapshot::default(), 0).unwrap()
            else {
                panic!()
            };
            assert_eq!(kind, AppKind::DistributionExplorer);
            engine
                .execute(&saved)
                .unwrap_or_else(|e| panic!("{}: {e}\n{saved}", r.family));
            assert_eq!(shown.stats, values(&engine, "app_stats"));
            let xs = values(&engine, "app_grid");
            let ys = values(&engine, "app_curve");
            assert_eq!(
                shown.curve,
                xs.into_iter()
                    .zip(ys)
                    .map(|(x, y)| [x, y])
                    .collect::<Vec<_>>()
            );
            for function in [Function::Pdf, Function::Cdf] {
                host.app.function = function;
                compute(&mut host).unwrap();
                render(&mut host, egui::vec2(1440., 900.));
            }
        }
    }
    #[test]
    fn distribution_explorer_requested_normal_exponential_and_discrete_values() {
        let mut host = configured(&references()[0]);
        compute(&mut host).unwrap();
        close(
            host.app.shown.as_ref().unwrap().stats[4],
            0.7978845608028654,
        );
        close(host.app.shown.as_ref().unwrap().stats[6], 2.979981992270027);
        host.app.probe_x = "2.5".into();
        compute(&mut host).unwrap();
        close(
            host.app.shown.as_ref().unwrap().stats[5],
            0.8413447460685429,
        );
        for (name, index, want) in [
            ("Exponential", 0, 3.),
            ("Exponential", 1, 9.),
            ("Exponential", 5, 0.6321205588285577),
            ("Binomial", 4, 0.266827932),
            ("Binomial", 0, 3.),
            ("Poisson", 4, 0.14652511110987343),
        ] {
            let r = references().into_iter().find(|r| r.family == name).unwrap();
            let mut host = configured(&r);
            compute(&mut host).unwrap();
            close(host.app.shown.as_ref().unwrap().stats[index], want);
        }
    }
    #[test]
    fn distribution_explorer_defaults_all_render_and_samples_reproduce_export() {
        for family in Family::ALL {
            let mut host = Host::new(DistributionExplorer::default());
            host.app.set_family(family);
            compute(&mut host).unwrap_or_else(|e| panic!("{family:?}: {e}"));
            host.app.mode = Mode::Samples;
            host.app.n = "32".into();
            host.app.seed = "123".into();
            host.app.bins = "8".into();
            host.app.object = "distribution".into();
            host.app.samples = "draws".into();
            compute(&mut host).unwrap_or_else(|e| panic!("{family:?}: {e}"));
            render(&mut host, egui::vec2(1440., 900.));
            let mut engine = matrust::Engine::default();
            let code = host.app.script(&Snapshot::default()).unwrap();
            engine.execute(&code).unwrap();
            let expected = values(&engine, "draws");
            assert_eq!(expected.len(), 32);
            same_values(
                &values(&engine, "app_stats"),
                &host.app.shown.as_ref().unwrap().stats,
            );
            let Action::ExportScript(export) = host.export(&Snapshot::default()).unwrap() else {
                panic!()
            };
            let mut exported = matrust::Engine::default();
            apps::execute_export(&mut exported, &export).unwrap();
            assert_eq!(expected, values(&exported, "draws"));
            let Action::SaveAppScript(_, saved) = host.generate(&Snapshot::default(), 0).unwrap()
            else {
                panic!()
            };
            let mut fresh = matrust::Engine::default();
            fresh.execute(&saved).unwrap();
            assert_eq!(expected, values(&fresh, "draws"));
            let shown = host.app.shown.as_ref().unwrap();
            let area: f64 = shown
                .edges
                .windows(2)
                .zip(&shown.density)
                .map(|(e, y)| (e[1] - e[0]) * y)
                .sum();
            close(area, 1.);
        }
    }
    #[test]
    fn distribution_explorer_invalid_input_keeps_last_plot_and_checks_names() {
        let ctx = egui::Context::default();
        let ws = Snapshot::default();
        let mut host = Host::new(DistributionExplorer::default());
        compute(&mut host).unwrap();
        let good = host.app.shown.as_ref().unwrap().curve.clone();
        for invalid in [
            "0",
            "-1",
            "1); injected=1; %",
            "1; 2",
            "NaN",
            "Inf",
            "ones(1,100000000)",
        ] {
            host.app.parameters[1].value = invalid.into();
            host.refresh(&ctx, &ws);
            assert_eq!(host.status().1, toolkits::AppStatus::Error);
            assert_eq!(host.app.shown.as_ref().unwrap().curve, good);
        }
        host.app.set_family(Family::Binomial);
        for (i, v) in [(0, "2.5"), (1, "1.1")] {
            let old = host.app.parameters[i].value.clone();
            host.app.parameters[i].value = v.into();
            assert!(host.app.script(&ws).is_err());
            host.app.parameters[i].value = old;
        }
        for name in [
            "1bad",
            "mrexp_old",
            "mrexp_had",
            "mrexp_complete",
            "matrust_export_error",
            "app_grid",
            "pdf",
            "rng",
            "for",
            "x; clear",
        ] {
            host.app.object = name.into();
            assert!(host.app.script(&ws).is_err(), "{name}");
        }
        host.app.object = "pd".into();
        host.app.mode = Mode::Samples;
        host.app.samples = "pd".into();
        assert!(host.app.script(&ws).is_err());
        host.app.samples = "x".into();
        for (field, v) in [
            (0, "0"),
            (0, "1000001"),
            (0, "2.5"),
            (1, "-1"),
            (1, "4294967296"),
            (2, "1025"),
            (2, "0"),
        ] {
            let f = match field {
                0 => &mut host.app.n,
                1 => &mut host.app.seed,
                _ => &mut host.app.bins,
            };
            let old = std::mem::replace(f, v.into());
            assert!(host.app.script(&ws).is_err());
            *match field {
                0 => &mut host.app.n,
                1 => &mut host.app.seed,
                _ => &mut host.app.bins,
            } = old;
        }
        host.app.set_family(Family::Uniform);
        host.app.parameters[0].value = "2".into();
        assert!(host.app.script(&ws).is_err());
    }
    fn render(host: &mut Host<DistributionExplorer>, size: egui::Vec2) {
        let ctx = egui::Context::default();
        let out = crate::headless::run(
            &ctx,
            egui::RawInput {
                screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, size)),
                ..Default::default()
            },
            |ui| {
                egui::CentralPanel::default().show(ui, |ui| {
                    apps::three_areas(
                        ui,
                        AppKind::DistributionExplorer,
                        "DISTRIBUTION",
                        |ui, area| match area {
                            apps::Area::Inputs => host.app.inputs_ui(ui, &Snapshot::default()),
                            apps::Area::Main => host.app.main_ui(ui),
                            apps::Area::Output => apps::result_block(ui, &host.app.results()),
                        },
                    );
                });
            },
        );
        assert!(!out.shapes.is_empty());
    }
    fn wait(host: &mut Host<DistributionExplorer>, ctx: &egui::Context) {
        let start = Instant::now();
        loop {
            host.refresh(ctx, &Snapshot::default());
            if !host.running() {
                break;
            }
            assert!(
                start.elapsed() < Duration::from_secs(30),
                "{:?}",
                host.status()
            );
            std::thread::sleep(Duration::from_millis(2));
        }
        assert_ne!(
            host.status().1,
            toolkits::AppStatus::Error,
            "{:?}",
            host.status()
        );
    }
    #[test]
    fn distribution_explorer_runner_sample_limits_stop_and_coalesced_changes() {
        let ctx = egui::Context::default();
        let mut host = Host::new(DistributionExplorer::default());
        host.app.mode = Mode::Samples;
        for n in [1, 1_000_000] {
            host.app.n = n.to_string();
            let start = Instant::now();
            host.refresh(&ctx, &Snapshot::default());
            let submit_time = start.elapsed();
            assert!(submit_time < Duration::from_secs(1));
            let frame_start = Instant::now();
            render(&mut host, egui::vec2(1440., 900.));
            let frame_time = frame_start.elapsed();
            assert!(frame_time < Duration::from_secs(1));
            if n == 1 {
                wait(&mut host, &ctx);
            }
            host.app.stop();
            host.refresh(&ctx, &Snapshot::default());
            let start = Instant::now();
            while host.running() {
                host.refresh(&ctx, &Snapshot::default());
                assert!(start.elapsed() < Duration::from_secs(5));
                std::thread::sleep(Duration::from_millis(2));
            }
            let stop_time = start.elapsed();
            host.app.resume();
            let compute_start = Instant::now();
            wait(&mut host, &ctx);
            assert_eq!(host.app.shown.as_ref().unwrap().stats.len(), 6);
            eprintln!(
                "distribution_explorer n={n} workers={} submit={submit_time:?} frame={frame_time:?} stop={stop_time:?} complete={:?}",
                std::env::var("RAYON_NUM_THREADS").unwrap_or_else(|_| "default".into()),
                compute_start.elapsed()
            );
        }
        host.app.mode = Mode::Explore;
        for mu in 1..40 {
            host.app.parameters[0].value = mu.to_string();
            host.refresh(&ctx, &Snapshot::default());
        }
        wait(&mut host, &ctx);
        close(host.app.shown.as_ref().unwrap().stats[0], 39.);
    }
    #[test]
    fn distribution_explorer_probe_line_drags_and_invalid_probability_retains_curve() {
        let mut host = Host::new(DistributionExplorer::default());
        compute(&mut host).unwrap();
        let ctx = egui::Context::default();
        let frame = |host: &mut Host<DistributionExplorer>, events| {
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
                |ui| {
                    egui::CentralPanel::default().show(ui, |ui| host.app.main_ui(ui));
                },
            )
        };
        for _ in 0..3 {
            frame(&mut host, vec![]);
        }
        let start = host.app.probe_screen.unwrap();
        frame(
            &mut host,
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
        let end = start + egui::vec2(64., 0.);
        frame(&mut host, vec![egui::Event::PointerMoved(end)]);
        frame(
            &mut host,
            vec![egui::Event::PointerButton {
                pos: end,
                button: egui::PointerButton::Primary,
                pressed: false,
                modifiers: egui::Modifiers::NONE,
            }],
        );
        assert!(host.app.probe_x.parse::<f64>().unwrap() > 0.1);
        compute(&mut host).unwrap();
        let good = host.app.shown.as_ref().unwrap().curve.clone();
        host.app.function = Function::Cdf;
        for invalid in ["-0.01", "1.01", "0.5); clear; %"] {
            host.app.probe_p = invalid.into();
            host.refresh(&ctx, &Snapshot::default());
            assert_eq!(host.status().1, toolkits::AppStatus::Error);
            assert_eq!(host.app.shown.as_ref().unwrap().curve, good);
        }
        host.app.probe_p = "0.975".into();
        host.app.parameters[0].lower = 5.;
        host.app.parameters[0].upper = -5.;
        assert!(host.app.script(&Snapshot::default()).is_err());
    }
    #[test]
    fn distribution_explorer_tool_tab_headless_both_modes_and_truncation_capability() {
        let ctx = egui::Context::default();
        let mut workspace = Workspace::with_context(ctx.clone(), true);
        workspace
            .toolkits
            .open_tool(Tool::App(AppKind::DistributionExplorer));
        assert_eq!(
            AppKind::built()
                .filter(|k| *k == AppKind::DistributionExplorer)
                .count(),
            1
        );
        let tab = AppKind::DistributionExplorer.tab();
        assert_eq!(AppKind::from_tab(tab), Some(AppKind::DistributionExplorer));
        let serialized = serde_json::to_string(&tab).unwrap();
        assert_eq!(
            serde_json::from_str::<matrust::session::ToolTab>(&serialized).unwrap(),
            tab
        );
        for mode in [Mode::Explore, Mode::Samples] {
            workspace.toolkits.apps.explorer.app.mode = mode;
            let start = Instant::now();
            loop {
                let out = frame_at(&mut workspace, &ctx, egui::vec2(1440., 900.), vec![]);
                if workspace
                    .toolkits
                    .apps
                    .explorer
                    .app
                    .shown
                    .as_ref()
                    .is_some_and(|s| s.mode == mode)
                    && !workspace.toolkits.apps.explorer.running()
                {
                    assert!(!out.shapes.is_empty());
                    break;
                }
                assert!(
                    start.elapsed() < Duration::from_secs(30),
                    "{:?}",
                    workspace.toolkits.apps.explorer.status()
                );
                std::thread::sleep(Duration::from_millis(2));
            }
        }
        // The capability message is deliberate: no family supports truncate in this engine.
        assert!(!matrust::session::FUNCTIONS.contains(&"truncate"));
        workspace.close_tool_tab(Tool::App(AppKind::DistributionExplorer));
        assert!(
            !workspace
                .toolkits
                .apps
                .is_open(AppKind::DistributionExplorer)
        );
    }
}
