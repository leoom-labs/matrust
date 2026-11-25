// Expectations are separate, direct engine calls on the same source data.
mod econ_modeler_tests {
    use super::*;
    use apps::econometric_modeler::{
        self as econ, Command, EconometricModeler, Source, TestKind, Transform,
    };
    use apps::{App, AppKind, Host, Snapshot};
    use matrust::{Array, Engine};

    const DATA: &str = "rng(17); t=(1:120)'; y=3+0.008*t+sin(t*1.71)+0.3*cos(t*0.43)+0.1*randn(size(t)); X=[t y]; T=table(y,'VariableNames',{'Value'}); TT=timetable(seconds(t),y,'VariableNames',{'Value'});";

    fn workspace(code: &str) -> Snapshot {
        let mut e = Engine::default();
        e.execute(code).unwrap();
        let mut ws = Snapshot::default();
        ws.sync(&e.workspace());
        ws
    }
    fn engine(ws: &Snapshot) -> Engine {
        let mut e = Engine::default();
        for (n, a) in ws.variables() {
            e.insert_array(n, a.clone()).unwrap();
        }
        e
    }
    fn numbers(e: &Engine, name: &str) -> Vec<f64> {
        let Some(Array::Numeric(m)) = e.array(name) else {
            panic!("missing {name}");
        };
        m.data().to_vec()
    }
    fn close(a: &[f64], b: &[f64]) {
        assert_eq!(a.len(), b.len());
        for (a, b) in a.iter().zip(b) {
            assert!(
                (a - b).abs() <= 1e-12 || (a.is_nan() && b.is_nan()),
                "{a} != {b}"
            );
        }
    }
    fn wait_workspace(app: &mut Workspace) {
        let until = Instant::now() + Duration::from_secs(30);
        while app.busy {
            app.receive();
            assert!(Instant::now() < until, "{}", app.status);
            std::thread::sleep(Duration::from_millis(2));
        }
    }
    fn settle(h: &mut Host<EconometricModeler>, ctx: &egui::Context, ws: &Snapshot) {
        let until = Instant::now() + Duration::from_secs(60);
        loop {
            h.refresh(ctx, ws);
            if !h.running() {
                break;
            }
            assert!(Instant::now() < until, "{}", h.status().0);
            std::thread::sleep(Duration::from_millis(2));
        }
    }
    fn run(h: &mut Host<EconometricModeler>, ctx: &egui::Context, ws: &Snapshot, cmd: Command) {
        econ::start_work(h, ctx, ws, cmd);
        settle(h, ctx, ws);
        assert_ne!(h.status().1, toolkits::AppStatus::Error, "{}", h.status().0);
    }
    fn loaded(ctx: &egui::Context, ws: &Snapshot, source: &str) -> Host<EconometricModeler> {
        let mut h = Host::new(EconometricModeler::default());
        run(
            &mut h,
            ctx,
            ws,
            Command::Import(Source::parse(source, ws).unwrap()),
        );
        h
    }
    #[test]
    fn econ_modeler_selectors_transforms_and_fresh_script_match_direct_engine() {
        let ctx = egui::Context::default();
        let ws = workspace(DATA);
        for source in ["y", "X(:,2)", "T.Value", "TT.Value"] {
            let mut h = loaded(&ctx, &ws, source);
            let mut e = engine(&ws);
            e.execute("expected=y(:);").unwrap();
            close(
                h.app.current.as_ref().unwrap().series[0].values.data(),
                &numbers(&e, "expected"),
            );
            for (transform, expression) in [
                (Transform::Log, "log(y(:))"),
                (Transform::Difference, "diff(y(:))"),
                (Transform::Seasonal(12), "y(13:end)-y(1:end-12)"),
                (Transform::Detrend, "detrend(y(:),1)"),
            ] {
                h.app.selected = 0;
                run(&mut h, &ctx, &ws, Command::Transform(transform));
                e.execute(&format!("expected={expression};")).unwrap();
                let view = h.app.current.as_ref().unwrap();
                close(
                    view.series.last().unwrap().values.data(),
                    &numbers(&e, "expected"),
                );
            }
            // Verify a chained transform too.
            h.app.selected = 1;
            run(&mut h, &ctx, &ws, Command::Transform(Transform::Difference));
            e.execute("expected=diff(log(y(:)));").unwrap();
            close(
                h.app
                    .current
                    .as_ref()
                    .unwrap()
                    .series
                    .last()
                    .unwrap()
                    .values
                    .data(),
                &numbers(&e, "expected"),
            );
            let script = h.app.script(&ws).unwrap();
            let mut fresh = engine(&ws);
            fresh.execute(&script).unwrap();
            for (i, s) in h.app.current.as_ref().unwrap().series.iter().enumerate() {
                fresh
                    .execute(&format!("actual=econ_results.Series{{{}}};", i + 1))
                    .unwrap();
                close(s.values.data(), &numbers(&fresh, "actual"));
            }
            assert!(fresh.array("app_econ_total").is_none());
        }
    }
    #[test]
    fn econ_modeler_numeric_classes_and_multicolumn_table_variables() {
        let ctx = egui::Context::default();
        let ws = workspace(&format!(
            "{DATA} Ys=single(X); Yi=int32(X); Ysparse=sparse(y); T2=table(Ys,'VariableNames',{{'Values'}});"
        ));
        for source in ["Ys(:,2)", "Yi(:,2)", "Ysparse", "T2.Values(:,2)"] {
            let mut h = loaded(&ctx, &ws, source);
            run(&mut h, &ctx, &ws, Command::Transform(Transform::Difference));
            let mut e = engine(&ws);
            e.execute(&format!("v={source}; expected=diff(full(double(v(:))));"))
                .unwrap();
            close(
                h.app.current.as_ref().unwrap().series[1].values.data(),
                &numbers(&e, "expected"),
            );
        }
    }
    #[test]
    fn econ_modeler_all_tests_and_correlograms_equal_engine_and_replay() {
        let ctx = egui::Context::default();
        let ws = workspace(DATA);
        let mut h = loaded(&ctx, &ws, "y");
        h.app.num_lags = "12".into();
        run(&mut h, &ctx, &ws, Command::Diagnostics);
        let mut e = engine(&ws);
        e.execute("[a,al,ab]=autocorr(y,'NumLags',12); [p,pl,pb]=parcorr(y,'NumLags',12);")
            .unwrap();
        let d = h
            .app
            .current
            .as_ref()
            .unwrap()
            .diagnostics
            .as_ref()
            .unwrap();
        for (actual, key) in [
            (&d.acf, "a"),
            (&d.acf_lags, "al"),
            (&d.acf_bounds, "ab"),
            (&d.pacf, "p"),
            (&d.pacf_lags, "pl"),
            (&d.pacf_bounds, "pb"),
        ] {
            close(actual, &numbers(&e, key));
        }
        for kind in TestKind::ALL {
            h.app.test_kind = kind;
            for lags in ["1", "3"] {
                h.app.test_lags = lags.into();
                h.app.dof = if kind == TestKind::LjungBox {
                    "1".into()
                } else {
                    String::new()
                };
                // This is a separate engine call, not derived from the app output.
                let direct = match kind {
                    TestKind::Adf => {
                        format!("adftest(y,'Lags',{lags},'Alpha',0.05,'Model','ARD','Test','t1')")
                    }
                    TestKind::Kpss => format!("kpsstest(y,'Lags',{lags},'Alpha',0.05,'Trend',1)"),
                    TestKind::Pp => {
                        format!("pptest(y,'Lags',{lags},'Alpha',0.05,'Model','ARD','Test','t1')")
                    }
                    TestKind::LjungBox => format!("lbqtest(y,'Lags',{lags},'Alpha',0.05,'DoF',1)"),
                    TestKind::Arch => format!("archtest(y,'Lags',{lags},'Alpha',0.05)"),
                };
                e.execute(&format!("[h,p,s,c]={direct}; expected=[double(h) p s c];"))
                    .unwrap();
                run(&mut h, &ctx, &ws, Command::Test);
                close(
                    h.app.current.as_ref().unwrap().tests.last().unwrap(),
                    &numbers(&e, "expected"),
                );
            }
        }
        assert_eq!(h.app.plan.tests.len(), 10);
        let mut fresh = engine(&ws);
        fresh.execute(&h.app.script(&ws).unwrap()).unwrap();
        for (i, row) in h.app.current.as_ref().unwrap().tests.iter().enumerate() {
            fresh.execute(&format!("r=econ_results.Tests{{{}}}; actual=[r.H r.PValue r.Statistic r.CriticalValue];",i+1)).unwrap();
            close(row, &numbers(&fresh, "actual"));
        }
        fresh
            .execute("a=econ_results.Diagnostics.ACF; p=econ_results.Diagnostics.PACF;")
            .unwrap();
        let d = h
            .app
            .current
            .as_ref()
            .unwrap()
            .diagnostics
            .as_ref()
            .unwrap();
        close(&d.acf, &numbers(&fresh, "a"));
        close(&d.pacf, &numbers(&fresh, "p"));
    }
    #[test]
    fn econ_modeler_test_options_and_shadowing_source_names() {
        let ctx = egui::Context::default();
        let ws = workspace(&format!("{DATA} isnumeric=y; r=y;"));
        let mut h = loaded(&ctx, &ws, "isnumeric");
        run(&mut h, &ctx, &ws, Command::Transform(Transform::Difference));
        run(
            &mut h,
            &ctx,
            &ws,
            Command::Import(Source::parse("r", &ws).unwrap()),
        );
        let mut e = engine(&ws);
        e.execute("expected=diff(y(:));").unwrap();
        close(
            h.app.current.as_ref().unwrap().series[1].values.data(),
            &numbers(&e, "expected"),
        );
        for (kind, model, statistic, trend, alpha, direct) in [
            (
                TestKind::Adf,
                "AR",
                "t1",
                false,
                "0.01",
                "adftest(y,'Lags',2,'Alpha',0.01,'Model','AR','Test','t1')",
            ),
            (
                TestKind::Adf,
                "ARD",
                "F",
                false,
                "0.1",
                "adftest(y,'Lags',2,'Alpha',0.1,'Model','ARD','Test','F')",
            ),
            (
                TestKind::Adf,
                "TS",
                "t2",
                false,
                "0.05",
                "adftest(y,'Lags',2,'Alpha',0.05,'Model','TS','Test','t2')",
            ),
            (
                TestKind::Pp,
                "AR",
                "t2",
                false,
                "0.01",
                "pptest(y,'Lags',2,'Alpha',0.01,'Model','AR','Test','t2')",
            ),
            (
                TestKind::Pp,
                "TS",
                "t1",
                false,
                "0.05",
                "pptest(y,'Lags',2,'Alpha',0.05,'Model','TS','Test','t1')",
            ),
            (
                TestKind::Kpss,
                "AR",
                "t1",
                false,
                "0.1",
                "kpsstest(y,'Lags',2,'Alpha',0.1,'Trend',0)",
            ),
            (
                TestKind::LjungBox,
                "AR",
                "t1",
                false,
                "0.1",
                "lbqtest(y,'Lags',2,'Alpha',0.1)",
            ),
            (
                TestKind::Arch,
                "AR",
                "t1",
                false,
                "0.1",
                "archtest(y,'Lags',2,'Alpha',0.1)",
            ),
        ] {
            h.app.test_kind = kind;
            h.app.model = model.into();
            h.app.statistic = statistic.into();
            h.app.trend = trend;
            h.app.alpha = alpha.into();
            h.app.test_lags = "2".into();
            e.execute(&format!("[h,p,s,c]={direct}; expected=[double(h) p s c];"))
                .unwrap();
            run(&mut h, &ctx, &ws, Command::Test);
            close(
                h.app.current.as_ref().unwrap().tests.last().unwrap(),
                &numbers(&e, "expected"),
            );
        }
        let mut fresh = engine(&ws);
        fresh.execute(&h.app.script(&ws).unwrap()).unwrap();
        for (i, row) in h.app.current.as_ref().unwrap().tests.iter().enumerate() {
            fresh.execute(&format!("v=econ_results.Tests{{{}}}; actual=[v.H v.PValue v.Statistic v.CriticalValue];",i+1)).unwrap();
            close(row, &numbers(&fresh, "actual"));
        }
    }
    #[test]
    fn econ_modeler_invalid_text_engine_errors_and_missing_values_keep_last_good() {
        let ctx = egui::Context::default();
        let ws = workspace(DATA);
        let mut h = loaded(&ctx, &ws, "y");
        run(&mut h, &ctx, &ws, Command::Test);
        let before = h.app.current.as_ref().unwrap().tests.clone();
        for bad in [
            "2; evil=1",
            "1%comment",
            "[1 2]",
            "0/0",
            "abc",
            "-1",
            "99999",
        ] {
            h.app.test_lags = bad.into();
            assert!(h.app.test_spec().is_err(), "{bad}");
            econ::start_work(&mut h, &ctx, &ws, Command::Test);
            assert_eq!(h.app.current.as_ref().unwrap().tests, before);
            assert_eq!(h.status().1, toolkits::AppStatus::Error);
        }
        h.app.test_lags = "120".into();
        econ::start_work(&mut h, &ctx, &ws, Command::Test);
        settle(&mut h, &ctx, &ws);
        assert_eq!(h.status().1, toolkits::AppStatus::Error);
        assert_eq!(h.app.current.as_ref().unwrap().tests, before);
        assert_eq!(h.app.plan.tests.len(), 1);
        for bad in [
            "y;evil=1",
            "sin(y)",
            "y(1:10)",
            "X(:,end)",
            "T",
            "T.NoSuch",
            "X",
            "X(:,3)",
            "y%",
            "rand(100000000)",
            "TT.Properties.RowTimes",
        ] {
            assert!(Source::parse(bad, &ws).is_err(), "{bad}");
        }
        let ws = workspace("y=[1;NaN;2;4;NaN;3;5;6];");
        let mut h = loaded(&ctx, &ws, "y");
        assert_eq!(h.app.current.as_ref().unwrap().series[0].nan_count, 2);
        run(&mut h, &ctx, &ws, Command::Transform(Transform::Difference));
        let mut e = engine(&ws);
        e.execute("expected=diff(y(:));").unwrap();
        close(
            h.app.current.as_ref().unwrap().series[1].values.data(),
            &numbers(&e, "expected"),
        );
        h.app.selected = 0;
        h.app.test_kind = TestKind::Arch;
        econ::start_work(&mut h, &ctx, &ws, Command::Test);
        settle(&mut h, &ctx, &ws);
        assert_eq!(h.status().1, toolkits::AppStatus::Error);
        assert_eq!(h.app.current.as_ref().unwrap().series.len(), 2);
    }
    #[test]
    fn econ_modeler_checked_export_names_and_atomic_rollback() {
        let ctx = egui::Context::default();
        let ws = workspace(DATA);
        let mut h = loaded(&ctx, &ws, "y");
        run(&mut h, &ctx, &ws, Command::Test);
        for bad in [
            "for",
            "mrexp_old",
            "mrexp_had",
            "matrust_export_error",
            "mrexp_complete",
            "app_econ_s0",
            "x;evil=1",
            "x%",
            "y",
        ] {
            h.app.name = bad.into();
            assert!(h.export(&ws).is_none(), "{bad}");
        }
        h.app.name = "results".into();
        let Some(toolkits::Action::ExportScript(code)) = h.export(&ws) else {
            panic!()
        };
        let mut e = engine(&ws);
        e.execute(&code).unwrap();
        e.execute("r=results.Tests{1}; actual=[r.H r.PValue r.Statistic r.CriticalValue];")
            .unwrap();
        close(
            &h.app.current.as_ref().unwrap().tests[0],
            &numbers(&e, "actual"),
        );
        e.execute("results=17; y=1;").unwrap();
        assert!(e.execute(&code).is_err());
        assert_eq!(numbers(&e, "results"), vec![17.]);
        e.execute("clear results;").unwrap();
        assert!(e.execute(&code).is_err());
        assert!(e.array("results").is_none());
        let forced = apps::all_or_nothing(
            "results=3; error('forced after assignment');",
            &["results".into()],
        );
        e.execute("results=23;").unwrap();
        assert!(e.execute(&forced).is_err());
        assert_eq!(numbers(&e, "results"), vec![23.]);
        let mut changed = workspace("y=(1:120)';");
        // Different snapshot, even when its independently assigned version matches.
        changed.sync(
            &[("y".into(), Array::Numeric(matrust::Matrix::scalar(3.)))]
                .into_iter()
                .collect(),
        );
        assert!(h.app.script(&changed).is_err());
    }
    #[test]
    fn econ_modeler_limits_cancel_and_scope_restore() {
        let ctx = egui::Context::default();
        let ws = workspace(DATA);
        let mut h = loaded(&ctx, &ws, "y");
        let mut oversized = h.app.plan.clone();
        oversized.series.resize(33, oversized.series[0].clone());
        assert!(econ::build_script(&oversized, "out").is_err());
        h.app.num_lags = "257".into();
        assert!(h.app.start(&ws, Command::Diagnostics).is_err());
        // The desktop export transaction also rejects a pre-cancelled publish.
        let mut e = engine(&ws);
        e.execute("out=19;").unwrap();
        let code = apps::all_or_nothing(
            &econ::build_script(&h.app.plan, "out").unwrap(),
            &["out".into()],
        );
        e.cancellation()
            .store(true, std::sync::atomic::Ordering::Relaxed);
        assert!(apps::execute_export(&mut e, &code).is_err());
        e.cancellation()
            .store(false, std::sync::atomic::Ordering::Relaxed);
        assert_eq!(numbers(&e, "out"), vec![19.]);
        let count = std::sync::Arc::new(std::sync::atomic::AtomicUsize::new(0));
        {
            let outer = count.clone();
            let _guard = matrust::engine::econ_app_hooks::present(move || {
                outer.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
                Ok(())
            });
            Engine::default().execute("econometricModeler").unwrap();
            {
                let inner = count.clone();
                let _guard = matrust::engine::econ_app_hooks::present(move || {
                    inner.fetch_add(10, std::sync::atomic::Ordering::Relaxed);
                    Ok(())
                });
                Engine::default().execute("econometricModeler").unwrap();
            }
            Engine::default().execute("econometricModeler").unwrap();
        }
        assert_eq!(count.load(std::sync::atomic::Ordering::Relaxed), 12);
        assert!(
            Engine::default()
                .execute("econometricModeler")
                .unwrap_err()
                .contains("requires the desktop")
        );
    }
    fn frame(
        h: &mut Host<EconometricModeler>,
        ctx: &egui::Context,
        ws: &Snapshot,
        events: Vec<egui::Event>,
        size: egui::Vec2,
    ) -> (egui::FullOutput, Option<toolkits::Action>) {
        let mut action = None;
        let output = ctx.run_ui(
            egui::RawInput {
                screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, size)),
                events,
                ..Default::default()
            },
            |ctx| {
                egui::CentralPanel::default()
                    .show(ctx, |ui| econ::show(h, ui, ws, false, &mut action));
            },
        );
        assert!(!output.shapes.is_empty());
        (output, action)
    }
    fn click(
        h: &mut Host<EconometricModeler>,
        ctx: &egui::Context,
        ws: &Snapshot,
        label: &str,
    ) -> Option<toolkits::Action> {
        let size = egui::vec2(1440., 1400.);
        let (out, _) = frame(h, ctx, ws, vec![], size);
        let mut positions = vec![];
        for s in &out.shapes {
            tool_tab_texts(&s.shape, label, &mut positions);
        }
        let pos = *positions
            .first()
            .unwrap_or_else(|| panic!("Missing button {label}"));
        let mut action = None;
        for pressed in [true, false] {
            let (_, next) = frame(
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
                size,
            );
            if next.is_some() {
                action = next;
            }
        }
        action
    }
    #[test]
    fn empty_load_series_shows_the_field_prompt() {
        let mut h = Host::new(EconometricModeler::default());
        let ctx = egui::Context::default();
        let ws = Snapshot::default();
        click(&mut h, &ctx, &ws, "Load series");
        assert_eq!(h.status().0, "Enter a variable name or expression");
        assert!(!h.running());
    }

    #[test]
    fn econ_modeler_real_pointer_clicks_load_transform_diagnostics_test_export_and_script() {
        let ctx = egui::Context::default();
        let ws = workspace(DATA);
        let mut h = Host::new(EconometricModeler::default());
        h.app.source_text = "X(:,2)".into();
        click(&mut h, &ctx, &ws, "Load series");
        settle(&mut h, &ctx, &ws);
        assert_eq!(
            h.app
                .current
                .as_ref()
                .expect("Load button submitted")
                .series
                .len(),
            1
        );
        click(&mut h, &ctx, &ws, "Add transform");
        settle(&mut h, &ctx, &ws);
        assert_eq!(
            h.app
                .current
                .as_ref()
                .expect("Transform submitted")
                .series
                .len(),
            2
        );
        h.app.num_lags = "8".into();
        click(&mut h, &ctx, &ws, "Update diagnostics");
        settle(&mut h, &ctx, &ws);
        assert_eq!(
            h.app
                .current
                .as_ref()
                .unwrap()
                .diagnostics
                .as_ref()
                .expect("Diagnostics submitted")
                .acf
                .len(),
            9
        );
        click(&mut h, &ctx, &ws, "Run test");
        settle(&mut h, &ctx, &ws);
        assert_eq!(
            h.app.current.as_ref().expect("Test submitted").tests.len(),
            1
        );
        let Some(toolkits::Action::ExportScript(code)) =
            click(&mut h, &ctx, &ws, "Export to workspace")
        else {
            panic!("Export button did nothing: {}", h.status().0)
        };
        let mut e = engine(&ws);
        e.execute(&code).unwrap();
        assert!(e.array("econ_results").is_some());
        let dir = tempfile::tempdir().unwrap();
        let mut desktop = Workspace::with_context(ctx.clone(), true);
        desktop.submit(worker::Job::ScriptInFolder(
            DATA.into(),
            dir.path().to_owned(),
        ));
        wait_workspace(&mut desktop);
        desktop.submit(worker::Job::ExportAppScript(code, dir.path().to_owned()));
        wait_workspace(&mut desktop);
        assert!(
            desktop.variables.contains_key("econ_results"),
            "{}",
            desktop.status
        );
        let Some(toolkits::Action::SaveAppScript(kind, code)) =
            click(&mut h, &ctx, &ws, "Generate script")
        else {
            panic!("Generate script button did nothing")
        };
        assert_eq!(kind, AppKind::EconometricModeler);
        assert!(code.ends_with(&h.app.script(&ws).unwrap()));
        let path = dir.path().join("diagnostics.m");
        desktop.toolkits.apps.test_save_path = Some(path.clone());
        desktop.queue_app_script_save(kind, code.clone());
        desktop.finish_app_script_save(&ctx);
        wait_workspace(&mut desktop);
        assert_eq!(std::fs::read_to_string(&path).unwrap(), code);
        let mut fresh = engine(&ws);
        fresh
            .execute(&std::fs::read_to_string(path).unwrap())
            .unwrap();
        fresh.execute("n=numel(econ_results.Tests);").unwrap();
        assert_eq!(numbers(&fresh, "n"), vec![1.]);
        h.app.test_lags = "1;bad=1".into();
        let (out, _) = frame(&mut h, &ctx, &ws, vec![], egui::vec2(1440., 1400.));
        fn has_inline_error(shape: &egui::epaint::Shape) -> bool {
            match shape {
                egui::epaint::Shape::Text(t) => t.galley.text().contains("Test lags:"),
                egui::epaint::Shape::Vec(v) => v.iter().any(has_inline_error),
                _ => false,
            }
        }
        assert!(
            out.shapes.iter().any(|s| has_inline_error(&s.shape)),
            "Missing inline validation"
        );
        click(&mut h, &ctx, &ws, "Run test");
        assert_eq!(h.status().1, toolkits::AppStatus::Error);
        assert_eq!(h.app.current.as_ref().unwrap().tests.len(), 1);
    }
    #[test]
    fn econ_modeler_headless_empty_eight_tests_error_and_narrow() {
        let ctx = egui::Context::default();
        let ws = workspace(DATA);
        let mut empty = Host::new(EconometricModeler::default());
        frame(
            &mut empty,
            &ctx,
            &Snapshot::default(),
            vec![],
            egui::vec2(1440., 900.),
        );
        let mut h = loaded(&ctx, &ws, "y");
        for i in 0..8 {
            h.app.test_kind = TestKind::ALL[i % 5];
            run(&mut h, &ctx, &ws, Command::Test);
        }
        frame(&mut h, &ctx, &ws, vec![], egui::vec2(1440., 900.));
        h.app.test_lags = "bad(".into();
        econ::start_work(&mut h, &ctx, &ws, Command::Test);
        frame(&mut h, &ctx, &ws, vec![], egui::vec2(1440., 900.));
        frame(&mut h, &ctx, &ws, vec![], egui::vec2(640., 720.));
        assert_eq!(h.app.current.as_ref().unwrap().tests.len(), 8);
    }
    #[test]
    fn econ_modeler_command_opens_desktop_and_cli_is_explicit() {
        for command in ["econometricModeler", "econometricModeler()"] {
            assert!(
                Engine::default()
                    .execute(command)
                    .unwrap_err()
                    .ends_with("econometricModeler requires the desktop")
            );
        }
        for command in ["econometricModeler(1)", "x=econometricModeler()"] {
            assert!(
                Engine::default()
                    .execute(command)
                    .unwrap_err()
                    .contains("expected no inputs or outputs")
            );
        }
        let ctx = egui::Context::default();
        let mut app = Workspace::with_context(ctx, true);
        let dir = tempfile::tempdir().unwrap();
        app.submit(worker::Job::ScriptInFolder(
            "econometricModeler".into(),
            dir.path().to_owned(),
        ));
        wait_workspace(&mut app);
        assert!(app.toolkits.apps.is_open(AppKind::EconometricModeler));
        assert_eq!(
            AppKind::from_tab(AppKind::EconometricModeler.tab()),
            Some(AppKind::EconometricModeler)
        );
        let json = serde_json::to_string(&AppKind::EconometricModeler.tab()).unwrap();
        assert_eq!(
            serde_json::from_str::<matrust::session::ToolTab>(&json).unwrap(),
            AppKind::EconometricModeler.tab()
        );
    }

    const MODEL_DATA: &str = "rng(73); y=filter(1,[1 -0.35],randn(160,1));";

    #[test]
    #[ignore = "manual release benchmark; configures the process worker pool"]
    fn econ_modeler_release_benchmark() {
        let workers = std::env::var("MATRUST_BENCH_WORKERS")
            .unwrap_or_else(|_| "1".into())
            .parse::<usize>()
            .unwrap();
        matrust::initialize_worker_pool(workers).unwrap();
        let ws = workspace(MODEL_DATA);
        let plan = econ::Plan {
            series: vec![econ::SeriesSpec {
                source: Source::parse("y", &ws).unwrap(),
                transforms: vec![],
            }],
            models: [
                econ::ModelKind::Arima,
                econ::ModelKind::Garch,
                econ::ModelKind::Egarch,
                econ::ModelKind::Gjr,
            ]
            .into_iter()
            .map(|kind| econ::ModelSpec {
                kind,
                horizon: Some(24),
                ..Default::default()
            })
            .collect(),
            ..Default::default()
        };
        let code = econ::build_script(&plan, "econ_results").unwrap();
        let mut samples = Vec::new();
        for repetition in 0..4 {
            let mut e = engine(&ws);
            let start = Instant::now();
            e.execute(&code).unwrap();
            if repetition > 0 {
                samples.push(start.elapsed().as_secs_f64());
            }
        }
        samples.sort_by(f64::total_cmp);
        println!(
            "workers={workers} observations=160 models=4 horizon=24 warmups=1 repetitions=3 median_s={:.6}",
            samples[1]
        );
    }

    fn assert_fit(v: &econ::ModelView, e: &mut Engine, record: &str, mean: bool) {
        e.execute(&format!("s={record}.Summary; a=[s.LogLikelihood s.AIC s.BIC]; p=[s.Table.Value s.Table.StandardError s.Table.TStatistic s.Table.PValue];")).unwrap();
        close(&[v.log_likelihood, v.aic, v.bic], &numbers(e, "a"));
        if mean {
            e.execute("p=[p; [s.VarianceTable.Value s.VarianceTable.StandardError s.VarianceTable.TStatistic s.VarianceTable.PValue]];").unwrap();
        }
        let expected = numbers(e, "p");
        for (i, p) in v.parameters.iter().enumerate() {
            close(
                &p.values,
                &std::array::from_fn::<_, 4, _>(|j| expected[j * v.parameters.len() + i]),
            );
        }
        for (values, key) in [
            (&v.forecast, "Forecast"),
            (&v.mse, "YMSE"),
            (&v.residuals, "Residuals"),
            (&v.lower, "Lower"),
            (&v.upper, "Upper"),
            (&v.acf, "ACF"),
            (&v.ljung_box, "LjungBox"),
            (&v.histogram_y, "HistogramY"),
            (&v.normal, "Normal"),
            (&v.qq_y, "QQY"),
        ] {
            e.execute(&format!("actual={record}.{key};")).unwrap();
            close(values, &numbers(e, "actual"));
        }
    }
    #[test]
    fn econ_modeler_each_model_fresh_script_and_independent_estimate_forecast() {
        let ctx = egui::Context::default();
        let ws = workspace(MODEL_DATA);
        for (kind, constructor) in [
            (econ::ModelKind::Arima, "arima(1,0,0)"),
            (econ::ModelKind::Garch, "garch(1,1)"),
            (econ::ModelKind::Egarch, "egarch(1,1)"),
            (econ::ModelKind::Gjr, "gjr(1,1)"),
        ] {
            for student in [false, true] {
                let mut h = loaded(&ctx, &ws, "y");
                h.app.controls.kind = kind;
                h.app.controls.student = student;
                run(&mut h, &ctx, &ws, Command::Estimate);
                h.app.controls.horizon = "7".into();
                run(&mut h, &ctx, &ws, Command::Forecast);
                let v = &h.app.current.as_ref().unwrap().models[0];
                let mut direct = engine(&ws);
                direct.execute(&format!("template={constructor}; template.Distribution='{}'; [m,c,ll]=estimate(template,y,'Display','off'); s=summarize(m); expected=[s.LogLikelihood s.AIC s.BIC]; params=[s.Table.Value s.Table.StandardError s.Table.TStatistic s.Table.PValue];",if student {"t"} else {"Gaussian"})).unwrap();
                close(
                    &[v.log_likelihood, v.aic, v.bic],
                    &numbers(&direct, "expected"),
                );
                if kind.mean() {
                    direct.execute("params=[params; [s.VarianceTable.Value s.VarianceTable.StandardError s.VarianceTable.TStatistic s.VarianceTable.PValue]]; [f,mse]=forecast(m,7,y); [r,variance]=infer(m,y); lower=f-1.96*sqrt(mse); upper=f+1.96*sqrt(mse);").unwrap();
                    close(&v.mse, &numbers(&direct, "mse"));
                    close(&v.lower, &numbers(&direct, "lower"));
                    close(&v.upper, &numbers(&direct, "upper"));
                } else {
                    direct.execute("f=forecast(m,7,y); variance=infer(m,y); r=(y-m.Offset)./sqrt(variance);").unwrap();
                    assert!(v.mse.is_empty());
                }
                close(&v.forecast, &numbers(&direct, "f"));
                close(&v.residuals, &numbers(&direct, "r"));
                close(&v.variance, &numbers(&direct, "variance"));
                let params = numbers(&direct, "params");
                for (i, p) in v.parameters.iter().enumerate() {
                    close(
                        &p.values,
                        &std::array::from_fn::<_, 4, _>(|j| params[j * v.parameters.len() + i]),
                    );
                }
                direct.execute("[ac,al,ab]=autocorr(r,'NumLags',10); [h,p,s,c]=lbqtest(r,'Lags',10); lb=[double(h) p s c]; [hy,edges]=histcounts(r,20,'Normalization','pdf'); hx=(edges(1:end-1)+edges(2:end))/2; normal=normpdf(hx,mean(r),max(std(r),eps)); qx=norminv(((1:numel(r))'-0.5)/numel(r)); qy=sort(r);").unwrap();
                for (actual, key) in [
                    (&v.acf, "ac"),
                    (&v.acf_bounds, "ab"),
                    (&v.ljung_box, "lb"),
                    (&v.histogram_x, "hx"),
                    (&v.histogram_y, "hy"),
                    (&v.normal, "normal"),
                    (&v.qq_x, "qx"),
                    (&v.qq_y, "qy"),
                ] {
                    close(actual, &numbers(&direct, key));
                }
                let script = h.app.script(&ws).unwrap();
                let mut fresh = engine(&ws);
                fresh.execute(&script).unwrap();
                assert_fit(v, &mut fresh, "econ_results.Models{1}", kind.mean());
                fresh.execute("object=econ_models{1}; summary=summarize(object); actual=summary.LogLikelihood;").unwrap();
                close(&[v.log_likelihood], &numbers(&fresh, "actual"));
                assert!(fresh.array("app_econ_fit").is_none());
            }
        }
    }
    #[test]
    fn econ_modeler_seasonal_differenced_constant_off_and_combined_script_options() {
        let ctx = egui::Context::default();
        let ws = workspace(MODEL_DATA);
        let mut h = loaded(&ctx, &ws, "y");
        h.app.controls.p = "0".into();
        h.app.controls.q = "1".into();
        h.app.controls.d = "1".into();
        h.app.controls.sar = "[4]".into();
        h.app.controls.sma = "[4]".into();
        h.app.controls.period = "4".into();
        h.app.controls.constant = false;
        run(&mut h, &ctx, &ws, Command::Estimate);
        run(&mut h, &ctx, &ws, Command::Forecast);
        let mut direct = engine(&ws);
        direct.execute("[m,c,l]=estimate(arima('D',1,'MALags',1,'SARLags',4,'SMALags',4,'Seasonality',4,'Constant',0),y,'Display','off'); s=summarize(m); expected=[s.LogLikelihood s.AIC s.BIC]; [f,mse]=forecast(m,12,y);").unwrap();
        let v = &h.app.current.as_ref().unwrap().models[0];
        close(
            &[v.log_likelihood, v.aic, v.bic],
            &numbers(&direct, "expected"),
        );
        close(&v.forecast, &numbers(&direct, "f"));
        close(&v.mse, &numbers(&direct, "mse"));
        for (kind, variance) in [
            (econ::ModelKind::ArimaGarch, "garch"),
            (econ::ModelKind::ArimaEgarch, "egarch"),
            (econ::ModelKind::ArimaGjr, "gjr"),
        ] {
            h.app.controls.kind = kind;
            let mut plan = h.app.plan.clone();
            let mut spec = h.app.controls.spec(0, 1).unwrap();
            spec.horizon = Some(9);
            plan.models = vec![spec];
            let script = econ::build_script(&plan, "out").unwrap();
            assert!(script.contains(&format!("'Variance',{variance}(1,1")));
            assert!(script.contains("forecast(app_econ_fit,9,"));
            matrust::syntax::parse(&script).unwrap();
            // Joint estimation is in the engine (5d2): the composite script runs.
            engine(&ws)
                .execute(&script)
                .unwrap_or_else(|e| panic!("{variance}: {e}"));
        }
    }
    #[test]
    fn econ_modeler_models_sort_rename_delete_checked_multi_export_and_rollback() {
        let ctx = egui::Context::default();
        let ws = workspace(MODEL_DATA);
        let mut h = loaded(&ctx, &ws, "y");
        for p in [0, 1, 2] {
            h.app.controls.p = p.to_string();
            run(&mut h, &ctx, &ws, Command::Estimate);
        }
        run(&mut h, &ctx, &ws, Command::Forecast);
        let mut direct = engine(&ws);
        direct.execute("scores=zeros(3,2); for j=1:3, f=estimate(arima(j-1,0,0),y,'Display','off'); s=summarize(f); scores(j,:)=[s.AIC s.BIC]; end; [a,ai]=sort(scores(:,1)); [b,bi]=sort(scores(:,2));").unwrap();
        for (bic, key) in [(false, "ai"), (true, "bi")] {
            assert_eq!(
                h.app.model_order(bic),
                numbers(&direct, key)
                    .iter()
                    .map(|x| *x as usize - 1)
                    .collect::<Vec<_>>()
            );
        }
        h.app.selected_model = 1;
        h.app.rename_model("Candidate").unwrap();
        assert_eq!(h.app.plan.models[1].name, "Candidate");
        assert!(h.app.rename_model("for").is_err());
        let duplicate = h.app.plan.models[0].name.clone();
        assert!(h.app.rename_model(&duplicate).is_err());
        h.app.plan.models[0].selected = false;
        let code = h.app.script(&ws).unwrap();
        let mut fresh = engine(&ws);
        fresh.execute(&code).unwrap();
        fresh
            .execute("n=numel(econ_models); nf=numel(econ_forecasts{2}.Forecast);")
            .unwrap();
        assert_eq!(numbers(&fresh, "n"), vec![2.]);
        assert_eq!(numbers(&fresh, "nf"), vec![12.]);
        assert_fit(
            &h.app.current.as_ref().unwrap().models[1],
            &mut fresh,
            "econ_results.Models{1}",
            true,
        );
        for bad in [
            "for",
            "mrexp_old",
            "app_econ_fit",
            "y",
            "econ_results",
            "x;evil=1",
        ] {
            h.app.model_names[0] = bad.into();
            assert!(h.export(&ws).is_none(), "{bad}");
        }
        h.app.model_names[0] = "econ_models".into();
        h.app.model_names[1] = "econ_models".into();
        assert!(h.export(&ws).is_none());
        h.app.model_names[1] = "econ_residuals".into();
        let Some(toolkits::Action::ExportScript(code)) = h.export(&ws) else {
            panic!()
        };
        fresh
            .execute("econ_results=17; econ_models=18; econ_residuals=19; econ_forecasts=20; y=1;")
            .unwrap();
        assert!(fresh.execute(&code).is_err());
        for (key, old) in [
            ("econ_results", 17.),
            ("econ_models", 18.),
            ("econ_residuals", 19.),
            ("econ_forecasts", 20.),
        ] {
            assert_eq!(numbers(&fresh, key), vec![old]);
        }
        let names = h.app.exported(&ws);
        let forced = apps::all_or_nothing(
            "econ_results=1; econ_models=2; econ_residuals=3; econ_forecasts=4; error('after outputs');",
            &names,
        );
        assert!(fresh.execute(&forced).is_err());
        assert_eq!(numbers(&fresh, "econ_models"), vec![18.]);
        assert_eq!(numbers(&fresh, "econ_forecasts"), vec![20.]);
        fresh
            .cancellation()
            .store(true, std::sync::atomic::Ordering::Relaxed);
        assert!(apps::execute_export(&mut fresh, &code).is_err());
        fresh
            .cancellation()
            .store(false, std::sync::atomic::Ordering::Relaxed);
        assert_eq!(numbers(&fresh, "econ_forecasts"), vec![20.]);
        h.app.delete_model().unwrap();
        assert_eq!(h.app.plan.models.len(), 2);
        assert_eq!(h.app.current.as_ref().unwrap().models.len(), 2);
        let count = h.app.plan.models.len();
        for bad in ["1;evil=1", "a", "99", "-1", "1%bad"] {
            h.app.controls.p = bad.into();
            assert!(h.app.start(&ws, Command::Estimate).is_err());
            assert_eq!(h.app.plan.models.len(), count);
        }
        for bad in ["[1 1]", "[2 1]", "rand(2)", "[1];evil=1", "[257]", "[1:3]"] {
            assert!(econ::parse_lags(bad).is_err(), "{bad}");
        }
        for m in &mut h.app.plan.models {
            m.selected = false;
        }
        assert!(h.app.script(&ws).is_err());
    }
    #[test]
    fn econ_modeler_real_pointer_estimate_forecast_export_generate_and_stop() {
        let ctx = egui::Context::default();
        let ws = workspace(MODEL_DATA);
        let mut h = loaded(&ctx, &ws, "y");
        click(&mut h, &ctx, &ws, "Models");
        click(&mut h, &ctx, &ws, "Estimate");
        settle(&mut h, &ctx, &ws);
        assert_eq!(
            h.app
                .current
                .as_ref()
                .expect("Estimate submitted")
                .models
                .len(),
            1
        );
        h.app.controls.horizon = "5".into();
        click(&mut h, &ctx, &ws, "Forecast");
        settle(&mut h, &ctx, &ws);
        assert_eq!(h.app.current.as_ref().unwrap().models[0].forecast.len(), 5);
        let Some(toolkits::Action::ExportScript(code)) =
            click(&mut h, &ctx, &ws, "Export to workspace")
        else {
            panic!("{}", h.status().0)
        };
        let mut fresh = engine(&ws);
        fresh.execute(&code).unwrap();
        fresh
            .execute("s=summarize(econ_models{1}); actual=s.LogLikelihood;")
            .unwrap();
        close(
            &[h.app.current.as_ref().unwrap().models[0].log_likelihood],
            &numbers(&fresh, "actual"),
        );
        let dir = tempfile::tempdir().unwrap();
        let mut desktop = Workspace::with_context(ctx.clone(), true);
        desktop.submit(worker::Job::ScriptInFolder(
            MODEL_DATA.into(),
            dir.path().to_owned(),
        ));
        wait_workspace(&mut desktop);
        desktop.submit(worker::Job::ExportAppScript(code, dir.path().to_owned()));
        wait_workspace(&mut desktop);
        assert!(
            desktop.variables.contains_key("econ_models"),
            "{}",
            desktop.status
        );
        assert!(desktop.variables.contains_key("econ_forecasts"));
        let Some(toolkits::Action::SaveAppScript(kind, code)) =
            click(&mut h, &ctx, &ws, "Generate script")
        else {
            panic!("Generate script did nothing")
        };
        assert_eq!(kind, AppKind::EconometricModeler);
        assert!(code.ends_with(&h.app.script(&ws).unwrap()));
        let path = dir.path().join("models.m");
        desktop.toolkits.apps.test_save_path = Some(path.clone());
        desktop.queue_app_script_save(kind, code.clone());
        desktop.finish_app_script_save(&ctx);
        wait_workspace(&mut desktop);
        assert_eq!(std::fs::read_to_string(path).unwrap(), code);
        let mut replay = engine(&ws);
        replay.execute(&code).unwrap();
        assert_fit(
            &h.app.current.as_ref().unwrap().models[0],
            &mut replay,
            "econ_results.Models{1}",
            true,
        );
        // A long second fit makes Stop observable without relying on sleep timing.
        let ws = workspace("rng(73); y=randn(60000,1);");
        let mut h = loaded(&ctx, &ws, "y");
        h.app.controls.p = "0".into();
        run(&mut h, &ctx, &ws, Command::Estimate);
        let before = h.app.current.as_ref().unwrap().models[0].log_likelihood;
        h.app.view = econ::View::Models;
        h.app.controls.p = "16".into();
        h.app.controls.q = "16".into();
        click(&mut h, &ctx, &ws, "Estimate");
        assert!(h.running());
        click(&mut h, &ctx, &ws, "Stop");
        settle(&mut h, &ctx, &ws);
        assert_eq!(h.app.plan.models.len(), 1);
        assert_eq!(
            h.app.current.as_ref().unwrap().models[0].log_likelihood,
            before
        );
        assert_eq!(h.status().1, toolkits::AppStatus::Error);
        for _ in 0..3 {
            frame(&mut h, &ctx, &ws, vec![], egui::vec2(1440., 1400.));
        }
        assert_eq!(h.app.plan.models.len(), 1);
    }
    #[test]
    fn econ_modeler_estimation_error_retains_models_and_recovers() {
        let ctx = egui::Context::default();
        let ws = workspace("y=[1;-1;0.5;-0.7;1.2;-0.4];");
        let mut h = loaded(&ctx, &ws, "y");
        h.app.controls.p = "0".into();
        run(&mut h, &ctx, &ws, Command::Estimate);
        let before = h.app.current.as_ref().unwrap().models[0].log_likelihood;
        h.app.controls.p = "16".into();
        econ::start_work(&mut h, &ctx, &ws, Command::Estimate);
        settle(&mut h, &ctx, &ws);
        assert_eq!(h.status().1, toolkits::AppStatus::Error);
        assert_eq!(h.app.plan.models.len(), 1);
        assert_eq!(
            h.app.current.as_ref().unwrap().models[0].log_likelihood,
            before
        );
        h.app.controls.p = "0".into();
        run(&mut h, &ctx, &ws, Command::Estimate);
        assert_eq!(h.app.plan.models.len(), 2);
    }
    #[test]
    fn econ_modeler_headless_twelve_models_eight_tests_forecast_and_narrow() {
        let ctx = egui::Context::default();
        let ws = workspace(MODEL_DATA);
        let mut h = loaded(&ctx, &ws, "y");
        h.app.controls.p = "0".into();
        for _ in 0..12 {
            run(&mut h, &ctx, &ws, Command::Estimate);
        }
        for _ in 0..8 {
            run(&mut h, &ctx, &ws, Command::Test);
        }
        run(&mut h, &ctx, &ws, Command::Forecast);
        for view in [econ::View::Models, econ::View::Diagnostics] {
            h.app.view = view;
            for size in [egui::vec2(1440., 900.), egui::vec2(640., 720.)] {
                frame(&mut h, &ctx, &ws, vec![], size);
            }
        }
        assert_eq!(h.app.current.as_ref().unwrap().models.len(), 12);
        assert_eq!(h.app.current.as_ref().unwrap().tests.len(), 8);
        assert_eq!(
            h.app.current.as_ref().unwrap().models[11].forecast.len(),
            12
        );
    }
}
