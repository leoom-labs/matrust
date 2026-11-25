// SciPy spectral-window fixtures and integration through the real app host.
mod window_designer_tests {
    use super::*;
    use apps::window_designer::{Kind, MAX_LENGTH, MAX_WINDOWS, WindowDesigner};
    use apps::{App, AppKind, Host, Snapshot};
    use toolkits::Tool;

    fn workspace(code: &str) -> Snapshot {
        let mut engine = matrust::Engine::default();
        engine.execute(code).unwrap();
        let mut ws = Snapshot::default();
        ws.sync(&engine.workspace());
        ws
    }
    fn compute(host: &mut Host<WindowDesigner>, ws: &Snapshot) -> Result<apps::Outputs, String> {
        host.app.prepare(ws);
        let outputs = apps::run(&mut matrust::Engine::default(), host.request(ws)?)?;
        host.app.receive(&outputs)?;
        Ok(outputs)
    }
    fn values(engine: &matrust::Engine, name: &str) -> Vec<f64> {
        match engine.array(name).unwrap() {
            matrust::Array::Numeric(m) => m.data().to_vec(),
            a => panic!("{name}: {}", a.class()),
        }
    }
    fn close(actual: f64, expected: f64, tolerance: f64) {
        assert!(
            (actual - expected).abs() <= tolerance,
            "{actual:.15} != {expected:.15} (tolerance {tolerance})"
        );
    }

    #[test]
    fn window_designer_scipy_all_families_and_periodic_measurements() {
        let reference: serde_json::Value = serde_json::from_str(include_str!(
            "../../tests/fixtures/apps/window_designer/reference.json"
        ))
        .unwrap();
        let ws = Snapshot::default();
        for case in reference["cases"].as_array().unwrap() {
            let name = case["kind"].as_str().unwrap();
            let kind = Kind::ALL.into_iter().find(|k| k.label() == name).unwrap();
            let mut host = Host::new(WindowDesigner::default());
            host.app.windows[0].set_kind(kind);
            host.app.windows[0].periodic = case["periodic"].as_bool().unwrap();
            compute(&mut host, &ws).unwrap_or_else(|e| panic!("{name}: {e}"));
            let c = &host.app.current[0];
            assert_eq!(c.samples.len(), 64);
            for (a, b) in c.samples.iter().zip(case["samples"].as_array().unwrap()) {
                close(*a, b.as_f64().unwrap(), 2e-12);
            }
            // Independent reference FFT is 16x denser. Width interpolation,
            // leakage integration and sampled sidelobe peaks have finite error.
            for (k, tolerance) in [0.0002, 0.015, 4e-7, 2e-11, 2e-12].into_iter().enumerate() {
                let expected = case["measurements"][k].as_f64().unwrap();
                let actual = c.measurements[k];
                assert!(
                    (actual - expected).abs() <= tolerance,
                    "{name} metric {k}: {actual} != {expected}"
                );
            }
            assert!(
                c.db.iter()
                    .all(|v| v.is_finite() && *v >= -150. && *v <= 1e-12)
            );
            close(c.frequency[0], 0., 0.);
            close(*c.frequency.last().unwrap(), 1., 0.);
        }
    }

    #[test]
    fn window_designer_generated_script_preview_export_and_save_agree() {
        let ws = Snapshot::default();
        let mut host = Host::new(WindowDesigner::default());
        for kind in [
            Kind::Hann,
            Kind::Rectangular,
            Kind::Blackman,
            Kind::Kaiser,
            Kind::Chebyshev,
            Kind::FlatTop,
        ] {
            host.app.add(kind, &ws).unwrap();
        }
        let outputs = compute(&mut host, &ws).unwrap();
        let script = host.app.script(&ws).unwrap();
        assert!(script.contains("fft(w, nfft)") && script.contains("subplot(1,2,2)"));
        let saved = match host.generate(&ws, 42).unwrap() {
            toolkits::Action::SaveAppScript(AppKind::WindowDesigner, text) => text,
            _ => panic!("wrong save action"),
        };
        assert!(saved.ends_with(&script));
        let mut fresh = matrust::Engine::default();
        fresh.execute(&saved).unwrap();
        for c in &host.app.current {
            let a = fresh.array(&c.name).unwrap();
            assert_eq!(a.dims().as_ref(), &[64, 1]);
            assert_eq!(values(&fresh, &c.name), c.samples);
        }
        // These aliases only read the saved script's vectors and metrics.
        fresh.execute(&host.app.probe(&ws).0).unwrap();
        for (name, array) in outputs {
            let expected = match array {
                matrust::Array::Numeric(m) => m.data().to_vec(),
                _ => unreachable!(),
            };
            let actual = values(&fresh, &name);
            assert_eq!(actual.len(), expected.len());
            for (a, b) in actual.iter().zip(expected) {
                assert!(a == &b || a.is_nan() && b.is_nan(), "{name}");
            }
        }
        let export = match host.export(&ws).unwrap() {
            toolkits::Action::ExportScript(s) => s,
            _ => panic!("wrong export"),
        };
        let mut exported = matrust::Engine::default();
        exported.execute(&export).unwrap();
        for c in &host.app.current {
            assert_eq!(values(&exported, &c.name), c.samples);
        }
    }

    #[test]
    fn window_designer_singleton_two_samples_and_zero_window() {
        let ws = Snapshot::default();
        for kind in Kind::ALL.into_iter().filter(|k| *k != Kind::User) {
            for n in [1, 2] {
                let mut host = Host::new(WindowDesigner::default());
                host.app.windows[0].set_kind(kind);
                host.app.windows[0].length = n.to_string();
                compute(&mut host, &ws).unwrap_or_else(|e| panic!("{kind:?}({n}): {e}"));
                let c = &host.app.current[0];
                assert_eq!(c.samples.len(), n);
                assert!(c.db.iter().all(|v| v.is_finite()));
                if n == 1 {
                    assert_eq!(c.samples, [1.]);
                    assert_eq!(c.measurements[0], 0.);
                    assert!(c.measurements[1].is_nan() && c.measurements[2].is_nan());
                    assert_eq!(&c.measurements[3..], [1., 1.]);
                }
                if n == 2 && matches!(kind, Kind::Hann | Kind::Bartlett) {
                    assert_eq!(c.samples, [0., 0.]);
                    assert!(c.measurements[..4].iter().all(|v| v.is_nan()));
                    assert_eq!(c.measurements[4], 0.);
                    assert!(c.db.iter().all(|v| *v == -150.));
                }
            }
        }
    }

    #[test]
    fn window_designer_parser_ranges_and_last_good_result() {
        let ws = Snapshot::default();
        let mut host = Host::new(WindowDesigner::default());
        compute(&mut host, &ws).unwrap();
        let previous = host.app.current[0].samples.clone();
        for bad in [
            "0",
            "-1",
            "1.2",
            "4097",
            "Inf",
            "NaN",
            "2^6",
            "64); intrusion=1; %",
            "64\nerror('boom')",
        ] {
            host.app.windows[0].length = bad.into();
            assert!(host.request(&ws).is_err(), "{bad}");
            assert_eq!(host.app.current[0].samples, previous);
        }
        host.app.windows[0].length = "64".into();
        for (kind, bad) in [
            (Kind::Kaiser, "-1"),
            (Kind::Kaiser, "101"),
            (Kind::Chebyshev, "0"),
            (Kind::Chebyshev, "301"),
            (Kind::Gaussian, "0"),
            (Kind::Gaussian, "101"),
            (Kind::Tukey, "-0.1"),
            (Kind::Tukey, "1.1"),
            (Kind::Taylor, "0"),
            (Kind::Taylor, "2.5"),
            (Kind::Taylor, "1e200"),
            (Kind::Taylor, "NaN"),
        ] {
            host.app.windows[0].set_kind(kind);
            host.app.windows[0].parameter = bad.into();
            let e = host.request(&ws).err().expect("bad parameter");
            if kind == Kind::Taylor {
                assert!(e.contains("nbar"), "{e}");
            }
        }
        host.app.windows[0].set_kind(Kind::Taylor);
        for bad in ["0", "30", "-301", "-Inf", "-30); error('bad'); %"] {
            host.app.windows[0].sidelobe = bad.into();
            assert!(host.request(&ws).is_err());
        }
        host.app.windows[0].sidelobe = "-30".into();
        host.app.windows[0].parameter = "64".into();
        compute(&mut host, &ws).unwrap();
        // A worker error reaches the status line and leaves the last plot.
        let ctx = egui::Context::default();
        host.app.windows[0].parameter = "NaN".into();
        host.refresh(&ctx, &ws);
        assert_eq!(host.status().1, toolkits::AppStatus::Error);
        assert!(host.status().0.contains("nbar"));
        assert_eq!(host.app.current[0].samples.len(), 64);
    }

    #[test]
    fn window_designer_user_vectors_names_clobber_and_atomic_export() {
        let ws = workspace(
            "x = [1 2 3 2 1]; y = [2 1]; wd_samples = 99; wd_metrics1 = 98; wd_measure = 97; bad = [1 2;3 4]; z = [1 NaN];",
        );
        let mut host = Host::new(WindowDesigner::default());
        host.app.windows[0].set_kind(Kind::User);
        host.app.windows[0].source = "x".into();
        host.app.windows[0].name = "y".into();
        host.app.add(Kind::User, &ws).unwrap();
        host.app.windows[1].source = "y".into();
        host.app.windows[1].name = "x".into();
        compute(&mut host, &ws).unwrap();
        assert_eq!(host.app.current[0].samples, [1., 2., 3., 2., 1.]);
        close(host.app.current[0].measurements[3], 5. * 19. / 81., 1e-12);
        close(host.app.current[0].measurements[4], 9. / 5., 1e-12);
        assert_eq!(host.app.current[1].samples, [2., 1.]);
        let code = match host.export(&ws).unwrap() {
            toolkits::Action::ExportScript(s) => s,
            _ => unreachable!(),
        };
        let mut engine = matrust::Engine::default();
        for (name, a) in ws.variables() {
            engine.insert_array(name, a.clone()).unwrap();
        }
        engine.execute(&code).unwrap();
        assert_eq!(values(&engine, "x"), [2., 1.]);
        assert_eq!(values(&engine, "y"), [1., 2., 3., 2., 1.]);
        for name in [
            "mrexp_old",
            "mrexp_had",
            "matrust_export_error",
            "mrexp_complete",
            "1bad",
            "for",
            "x);bad=1;%",
        ] {
            host.app.windows[0].name = name.into();
            assert!(host.app.script(&ws).is_err(), "{name}");
        }
        host.app.windows[0].name = "x".into();
        assert!(host.app.script(&ws).unwrap_err().contains("duplicate"));
        host.app.windows[0].name = "result".into();
        for source in ["missing", "bad", "z", "x);error('bad');%"] {
            host.app.windows[0].source = source.into();
            assert!(host.request(&ws).is_err(), "{source}");
        }
        // The same snapshot builds a script, but a changed source at export
        // causes a worker error. The framework rolls both output names back.
        host.app.windows[0].source = "x".into();
        let code = match host.export(&ws).unwrap() {
            toolkits::Action::ExportScript(s) => s,
            _ => unreachable!(),
        };
        engine.execute("x = [NaN 1]; result = 123;").unwrap();
        assert!(engine.execute(&code).is_err());
        assert_eq!(values(&engine, "result"), [123.]);
        assert!(values(&engine, "x")[0].is_nan());
        let huge = workspace(&format!("x = ones({},1);", MAX_LENGTH + 1));
        assert!(host.request(&huge).is_err());
    }

    #[test]
    fn window_designer_user_impulse_zero_and_signed_formula_values() {
        let ws = workspace("impulse = [0 1 0]; zero = zeros(1,4); signed = [1 -1];");
        let mut host = Host::new(WindowDesigner::default());
        host.app.windows[0].set_kind(Kind::User);
        for (source, gain, bandwidth) in [
            ("impulse", 1. / 3., 3.),
            ("zero", 0., f64::NAN),
            ("signed", 0., f64::INFINITY),
        ] {
            host.app.windows[0].source = source.into();
            compute(&mut host, &ws).unwrap();
            let c = &host.app.current[0];
            close(c.measurements[4], gain, 1e-15);
            assert!(
                c.measurements[3] == bandwidth || c.measurements[3].is_nan() && bandwidth.is_nan()
            );
            // An impulse has a flat unit spectrum: no sidelobe or -3 dB crossing.
            if source == "impulse" {
                close(c.measurements[0], 0., 1e-12);
                assert!(c.measurements[1].is_nan() && c.measurements[2].is_nan());
                assert!(c.db.iter().all(|v| v.abs() < 1e-12));
            }
        }
    }

    #[test]
    fn window_designer_list_operations_and_tool_tab_headless_0_1_8() {
        let ws = Snapshot::default();
        let mut host = Host::new(WindowDesigner::default());
        host.app.windows[0].set_kind(Kind::Kaiser);
        host.app.windows[0].parameter = "5".into();
        host.app.duplicate(&ws).unwrap();
        assert_eq!(host.app.windows[1].parameter, "5");
        assert_ne!(host.app.windows[0].name, host.app.windows[1].name);
        host.app.remove();
        assert_eq!(host.app.windows.len(), 1);
        host.app.windows[0].shown = false;
        assert!(host.request(&ws).is_err());
        host.app.windows[0].shown = true;
        for _ in 1..MAX_WINDOWS {
            host.app.add(Kind::Hann, &ws).unwrap();
        }
        assert!(host.app.add(Kind::Hann, &ws).is_err());
        host.app.windows[3].shown = false;
        compute(&mut host, &ws).unwrap();
        assert_eq!(host.app.current.len(), 7);
        assert_eq!(host.app.exported(&ws).len(), 7);

        let kind = AppKind::WindowDesigner;
        let tab = kind.tab();
        assert_eq!(AppKind::from_tab(tab), Some(kind));
        assert_eq!(serde_json::to_string(&tab).unwrap(), "\"window_designer\"");
        let decoded: matrust::session::ToolTab =
            serde_json::from_str("\"window_designer\"").unwrap();
        assert_eq!(decoded, tab);
        for count in [0, 1, 8] {
            for width in [820., 1600.] {
                let ctx = egui::Context::default();
                let mut desktop = Workspace::with_context(ctx.clone(), true);
                desktop.toolkits.open_tool(Tool::App(kind));
                desktop.toolkits.apps.window.app.windows.clear();
                for _ in 0..count {
                    desktop
                        .toolkits
                        .apps
                        .window
                        .app
                        .add(Kind::Hann, &ws)
                        .unwrap();
                }
                if count > 0 {
                    compute(&mut desktop.toolkits.apps.window, &ws).unwrap();
                }
                let output = frame_at(&mut desktop, &ctx, egui::vec2(width, 950.), vec![]);
                assert!(!output.shapes.is_empty());
                assert_eq!(desktop.toolkits.selected_tool(), Some(Tool::App(kind)));
                desktop.close_tool_tab(Tool::App(kind));
            }
        }
    }

    #[test]
    fn window_designer_release_workload_1_and_n_workers() {
        // Gate profile is release optimisation without LTO. Use isolated
        // pools, sequentially: no nested pool and no new parallel app path.
        let workers = std::thread::available_parallelism()
            .map_or(1, |n| n.get())
            .min(8);
        for count in [1, workers] {
            let pool = rayon::ThreadPoolBuilder::new()
                .num_threads(count)
                .build()
                .unwrap();
            let elapsed = pool.install(|| {
                let ws = Snapshot::default();
                let mut host = Host::new(WindowDesigner::default());
                for _ in 1..MAX_WINDOWS {
                    host.app.add(Kind::Hann, &ws).unwrap();
                }
                for w in &mut host.app.windows {
                    w.length = MAX_LENGTH.to_string();
                }
                let start = Instant::now();
                compute(&mut host, &ws).unwrap();
                assert_eq!(host.app.current.len(), MAX_WINDOWS);
                start.elapsed()
            });
            eprintln!(
                "window_designer workload workers={count} windows=8 N=4096 FFT=131072 elapsed={elapsed:?}"
            );
        }
    }
}
