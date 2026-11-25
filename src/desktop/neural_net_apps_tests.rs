// Numerical expectations below are analytic constructions; replay comparisons
// explicitly check that the app and its public script execute the same contract.
mod neural_net_apps_tests {
    use super::*;
    use apps::neural_net_apps::{self, Mode, NeuralNet, Problem};
    use apps::{App, AppKind, Host, Snapshot};
    use matrust::{Array, Engine, Matrix};
    use std::sync::atomic::Ordering;

    fn host(mode: Mode) -> Host<NeuralNet> {
        let mut app = NeuralNet::new(mode);
        app.spec.hidden = 3;
        app.spec.epochs = 4;
        Host::new(app)
    }
    fn settle(h: &mut Host<NeuralNet>, ctx: &egui::Context, ws: &Snapshot) {
        let until = Instant::now() + Duration::from_secs(90);
        loop {
            h.refresh(ctx, ws);
            if !h.running() {
                break;
            }
            assert!(Instant::now() < until, "{}", h.status().0);
            std::thread::sleep(Duration::from_millis(2));
        }
    }
    fn start(h: &mut Host<NeuralNet>, ctx: &egui::Context, ws: &Snapshot) {
        h.app.start(ws).unwrap();
        settle(h, ctx, ws);
        assert!(h.app.current.is_some(), "{}", h.status().0);
        assert_ne!(h.status().1, toolkits::AppStatus::Error, "{}", h.status().0);
    }
    fn numbers(e: &Engine, n: &str) -> Vec<f64> {
        let Some(Array::Numeric(m)) = e.array(n) else {
            panic!("missing {n}")
        };
        m.data().to_vec()
    }
    fn close(a: &[f64], b: &[f64]) {
        assert_eq!(a.len(), b.len());
        for (a, b) in a.iter().zip(b) {
            assert!(
                (a - b).abs() < 1e-12 || (a.is_nan() && b.is_nan()),
                "{a} != {b}"
            );
        }
    }
    fn workspace(code: &str) -> Snapshot {
        let mut e = Engine::default();
        e.execute(code).unwrap();
        let mut ws = Snapshot::default();
        ws.sync(
            &["x", "t"]
                .into_iter()
                .map(|n| (n.into(), e.array(n).unwrap()))
                .collect(),
        );
        ws
    }
    #[test]
    fn neural_net_apps_sine_results_equal_fresh_script() {
        let ctx = egui::Context::default();
        let ws = Snapshot::default();
        let mut h = host(Mode::Fitting);
        start(&mut h, &ctx, &ws);
        let script = h.app.script(&ws).unwrap();
        let mut e = Engine::default();
        e.execute(&script).unwrap();
        close(
            &h.app.current.as_ref().unwrap().metrics,
            &numbers(&e, "app_nn_metrics"),
        );
        // Synthetic construction has endpoints sin(+-pi)=0 and midpoint sin(0)=0.
        let targets = numbers(&e, "app_nn_targets");
        assert_eq!(targets.len(), 101);
        for i in [0, 50, 100] {
            assert!(targets[i].abs() < 1e-14);
        }
        assert!(script.contains("plotperform(tr)"));
        assert!(script.contains("plotregression(t,y)"));
    }
    #[test]
    fn neural_net_apps_pattern_confusion_equals_script_and_known_counts() {
        let ctx = egui::Context::default();
        let ws = Snapshot::default();
        let mut h = host(Mode::PatternRecognition);
        start(&mut h, &ctx, &ws);
        let mut e = Engine::default();
        e.execute(&h.app.script(&ws).unwrap()).unwrap();
        e.execute("[rate,counts]=confusion(app_nn_targets,y);")
            .unwrap();
        close(
            &h.app.current.as_ref().unwrap().confusion,
            &numbers(&e, "counts"),
        );
        assert_eq!(numbers(&e, "counts").iter().sum::<f64>(), 200.);
        e.execute("[rate,counts]=confusion([1 1 0 0;0 0 1 1],[0.9 0.2 0.8 0.1;0.1 0.8 0.2 0.9]);")
            .unwrap();
        assert_eq!(numbers(&e, "counts"), vec![1.; 4]);
        assert_eq!(numbers(&e, "rate"), vec![0.5]);
        e.execute("[rate,counts]=confusion([1 1 1 0;0 0 0 1],[1 0 0 0;0 1 1 1]);")
            .unwrap();
        assert_eq!(numbers(&e, "counts"), vec![1., 0., 2., 1.]);
    }
    #[test]
    fn neural_net_apps_rows_columns_and_class_targets() {
        let ctx = egui::Context::default();
        let col = workspace("x=[-3 -2 -1 1 2 3;-2 -1 -3 3 1 2]; t=[1 1 1 0 0 0;0 0 0 1 1 1];");
        let rows = workspace("x=[-3 -2 -1 1 2 3;-2 -1 -3 3 1 2].'; t=[1 1 1 0 0 0;0 0 0 1 1 1].';");
        let mut baseline = host(Mode::PatternRecognition);
        baseline.app.spec.example = false;
        baseline.app.spec.predictors = "x".into();
        baseline.app.spec.responses = "t".into();
        start(&mut baseline, &ctx, &col);
        let expected = baseline.app.current.as_ref().unwrap().metrics.clone();
        let mut row = host(Mode::PatternRecognition);
        row.app.spec = baseline.app.spec.clone();
        row.app.spec.rows = true;
        start(&mut row, &ctx, &rows);
        close(&expected, &row.app.current.as_ref().unwrap().metrics);
        for labels in ["[-4 -4 -4 8 8 8]", "categorical({'a','a','a','b','b','b'})"] {
            let ws = workspace(&format!("x=[-3 -2 -1 1 2 3;-2 -1 -3 3 1 2];t={labels};"));
            let mut h = host(Mode::PatternRecognition);
            h.app.spec = baseline.app.spec.clone();
            h.app.spec.classes = true;
            start(&mut h, &ctx, &ws);
            close(&expected, &h.app.current.as_ref().unwrap().metrics);
        }
    }
    #[test]
    fn neural_net_apps_bad_sizes_keep_last_good_result_and_show_status() {
        let ctx = egui::Context::default();
        let mut h = host(Mode::Fitting);
        start(&mut h, &ctx, &Snapshot::default());
        let previous = h.app.current.as_ref().unwrap().metrics.clone();
        h.app.spec.example = false;
        h.app.spec.predictors = "x".into();
        h.app.spec.responses = "t".into();
        let ws = workspace("x=1:10;t=1:9;");
        h.app.start(&ws).unwrap();
        settle(&mut h, &ctx, &ws);
        assert_eq!(h.status().1, toolkits::AppStatus::Error);
        assert!(
            h.status().0.contains("same sample count"),
            "{}",
            h.status().0
        );
        close(&previous, &h.app.current.as_ref().unwrap().metrics);
    }
    #[test]
    fn neural_net_apps_stop_returns_usable_net_and_replays_checkpoint() {
        let ctx = egui::Context::default();
        let ws = Snapshot::default();
        let mut h = host(Mode::Fitting);
        h.app.spec.epochs = 10000;
        h.app.start(&ws).unwrap();
        h.app.stop.store(true, Ordering::Relaxed);
        settle(&mut h, &ctx, &ws);
        let result = h.app.current.as_ref().expect("stopped result");
        assert_eq!(result.stop, "User stop.");
        let script = h.app.script(&ws).unwrap();
        // Stop snapshot replays without waiting for the originally requested budget.
        let mut e = Engine::default();
        e.execute(&script).unwrap();
        e.execute("check=net(linspace(-pi,pi,101)); stop=tr.stop;")
            .unwrap();
        let Array::Numeric(y) = &result.outputs["y"] else {
            panic!()
        };
        close(y.data(), &numbers(&e, "check"));
        let Some(Array::Character(stop)) = e.array("stop") else {
            panic!()
        };
        assert_eq!(String::from_utf16_lossy(stop.units()), "User stop.");
    }
    #[test]
    fn neural_net_apps_export_validation_and_rollback() {
        let ws = Snapshot::default();
        let mut h = host(Mode::Fitting);
        for bad in ["mrexp_old", "app_nn_run", "x;evil=1", "x%", "for"] {
            h.app.names[0] = bad.into();
            assert!(h.app.script(&ws).is_err(), "accepted {bad}");
        }
        h.app.names = ["net", "tr", "y", "e"].map(str::to_owned);
        h.app.names[1] = "net".into();
        assert!(h.app.script(&ws).is_err());
        h.app.names = ["net", "tr", "y", "e"].map(str::to_owned);
        let ctx = egui::Context::default();
        start(&mut h, &ctx, &ws);
        let Some(toolkits::Action::ExportScript(script)) = h.export(&ws) else {
            panic!()
        };
        let mut e = Engine::default();
        e.execute(&script).unwrap();
        e.execute("z=net(0); saved_epochs=net.trainParam.epochs;")
            .unwrap();
        assert_eq!(numbers(&e, "saved_epochs"), vec![4.]);
        assert!(e.array("app_nn_metrics").is_none());
        assert_eq!(numbers(&e, "z").len(), 1);
        let bad = apps::all_or_nothing("net=3;tr=4;error('forced failure');", &h.app.exported(&ws));
        e.execute("net=17;tr=19;").unwrap();
        assert!(e.execute(&bad).is_err());
        assert_eq!(numbers(&e, "net"), vec![17.]);
        assert_eq!(numbers(&e, "tr"), vec![19.]);
    }
    #[test]
    fn neural_net_apps_commands_open_desktop_and_cli_errors() {
        let ctx = egui::Context::default();
        let mut app = Workspace::with_context(ctx, true);
        let dir = tempfile::tempdir().unwrap();
        for (cmd, kind) in [
            ("nftool", Some(AppKind::NeuralNetFitting)),
            ("nprtool", Some(AppKind::NeuralNetPatternRecognition)),
            ("nctool", Some(AppKind::NeuralNetClustering)),
            ("ntstool", Some(AppKind::NeuralNetTimeSeries)),
            ("nnstart", None),
        ] {
            assert!(
                Engine::default()
                    .execute(cmd)
                    .unwrap_err()
                    .ends_with(&format!("{cmd} requires the desktop"))
            );
            app.submit(worker::Job::ScriptInFolder(
                cmd.into(),
                dir.path().to_owned(),
            ));
            let until = Instant::now() + Duration::from_secs(20);
            while app.busy {
                app.receive();
                assert!(Instant::now() < until);
                std::thread::sleep(Duration::from_millis(2));
            }
            if let Some(kind) = kind {
                assert!(app.toolkits.apps.is_open(kind));
                assert_eq!(AppKind::from_tab(kind.tab()), Some(kind));
                let json = serde_json::to_string(&kind.tab()).unwrap();
                assert_eq!(
                    serde_json::from_str::<matrust::session::ToolTab>(&json).unwrap(),
                    kind.tab()
                );
            } else {
                assert!(app.apps_home);
            }
        }
    }
    fn frame(h: &mut Host<NeuralNet>, ctx: &egui::Context, ws: &Snapshot) {
        let out = ctx.run_ui(
            egui::RawInput {
                screen_rect: Some(egui::Rect::from_min_size(
                    egui::Pos2::ZERO,
                    egui::vec2(1440., 900.),
                )),
                ..Default::default()
            },
            |ctx| {
                egui::CentralPanel::default().show(ctx, |ui| {
                    neural_net_apps::show(h, ui, ws, false, &mut None);
                });
            },
        );
        assert!(!out.shapes.is_empty());
    }
    #[test]
    fn neural_net_apps_headless_empty_trained_error_and_large_data() {
        let ctx = egui::Context::default();
        let mut h = host(Mode::Fitting);
        let ws = Snapshot::default();
        frame(&mut h, &ctx, &ws);
        start(&mut h, &ctx, &ws);
        frame(&mut h, &ctx, &ws);
        h.app.names[0] = "bad;name".into();
        frame(&mut h, &ctx, &ws);
        assert_eq!(h.status().1, toolkits::AppStatus::Error);
        for mode in [Mode::Fitting, Mode::PatternRecognition] {
            for count in [1000, 50000] {
                let mut ws = Snapshot::default();
                let x = (0..count)
                    .map(|i| i as f64 / count as f64)
                    .collect::<Vec<_>>();
                let t = if mode == Mode::Fitting {
                    x.iter().map(|x| x.sin()).collect()
                } else {
                    x.iter()
                        .flat_map(|x| if *x < 0.5 { [1., 0.] } else { [0., 1.] })
                        .collect()
                };
                let target_rows = if mode == Mode::Fitting { 1 } else { 2 };
                ws.sync(
                    &[
                        (
                            "x".into(),
                            Array::Numeric(Matrix::from_column_vec(1, count, x).unwrap()),
                        ),
                        (
                            "t".into(),
                            Array::Numeric(Matrix::from_column_vec(target_rows, count, t).unwrap()),
                        ),
                    ]
                    .into_iter()
                    .collect(),
                );
                let mut h = host(mode);
                h.app.spec.example = false;
                h.app.spec.predictors = "x".into();
                h.app.spec.responses = "t".into();
                h.app.spec.epochs = 0;
                h.app.spec.algorithm = "trainscg".into();
                frame(&mut h, &ctx, &ws);
                start(&mut h, &ctx, &ws);
                frame(&mut h, &ctx, &ws);
            }
        }
    }

    #[test]
    fn neural_net_apps_progress_hook_stops_at_epoch_two_and_restores_scope() {
        use matrust::engine::neural_app_hooks;
        use std::{cell::RefCell, ops::ControlFlow, rc::Rc};
        let reports = Rc::new(RefCell::new(Vec::new()));
        let seen = reports.clone();
        let mut e = Engine::default();
        {
            let _hook = neural_app_hooks::progress(move |r| {
                let epoch = r.epoch;
                assert!(r.perf.is_finite() && r.gradient.is_finite() && r.time >= 0.);
                seen.borrow_mut().push(epoch);
                if epoch == 2 {
                    ControlFlow::Break(())
                } else {
                    ControlFlow::Continue(())
                }
            });
            e.execute("rng(0);x=linspace(-pi,pi,101);t=sin(x);net=fitnet(3);net.trainParam.epochs=10;[net,tr]=train(net,x,t);stop=tr.stop;y=net(x);").unwrap();
        }
        assert_eq!(*reports.borrow(), vec![0, 1, 2]);
        let Some(Array::Character(stop)) = e.array("stop") else {
            panic!()
        };
        assert_eq!(String::from_utf16_lossy(stop.units()), "User stop.");
        assert!(numbers(&e, "y").iter().all(|v| v.is_finite()));
        e.execute("net.trainParam.epochs=0;[net,tr]=train(net,x,t);")
            .unwrap();
        assert_eq!(*reports.borrow(), vec![0, 1, 2]);
    }
    #[test]
    fn neural_net_apps_settings_wait_for_train_and_stale_data_blocks_export() {
        let ctx = egui::Context::default();
        let mut h = host(Mode::Fitting);
        let mut ws = workspace("x=linspace(-pi,pi,31);t=sin(x);");
        h.app.spec.example = false;
        h.app.spec.predictors = "x".into();
        h.app.spec.responses = "t".into();
        start(&mut h, &ctx, &ws);
        let script = h.app.script(&ws).unwrap();
        h.app.spec.hidden = 11;
        settle(&mut h, &ctx, &ws);
        assert_eq!(script, h.app.script(&ws).unwrap());
        h.app.names[0] = "trainedNet".into();
        settle(&mut h, &ctx, &ws);
        assert!(!h.running());
        let Some(toolkits::Action::ExportScript(code)) = h.export(&ws) else {
            panic!()
        };
        let mut e = Engine::default();
        e.execute("x=linspace(-pi,pi,31);t=sin(x);").unwrap();
        e.execute(&code).unwrap();
        e.execute("actual=trainedNet(x);").unwrap();
        let Array::Numeric(expected) = &h.app.current.as_ref().unwrap().outputs["y"] else {
            panic!()
        };
        close(expected.data(), &numbers(&e, "actual"));
        ws.sync(
            &[
                ("x".into(), Array::Numeric(Matrix::scalar(4.))),
                ("t".into(), Array::Numeric(Matrix::scalar(5.))),
            ]
            .into_iter()
            .collect(),
        );
        assert!(h.app.script(&ws).unwrap_err().contains("changed"));
        assert!(h.export(&ws).is_none());
    }
    #[test]
    fn neural_net_apps_all_plot_tabs_render_and_script_saves() {
        for mode in [Mode::Fitting, Mode::PatternRecognition] {
            let ctx = egui::Context::default();
            let ws = Snapshot::default();
            let mut h = host(mode);
            start(&mut h, &ctx, &ws);
            let count = if mode == Mode::Fitting { 4 } else { 5 };
            assert_eq!(h.app.current.as_ref().unwrap().charts.len(), count);
            for view in 0..count {
                h.app.view = view;
                frame(&mut h, &ctx, &ws);
            }
            let Some(toolkits::Action::SaveAppScript(kind, script)) = h.generate(&ws, 0) else {
                panic!()
            };
            assert_eq!(kind, mode.kind());
            matrust::syntax::parse(&script).unwrap();
            let dir = tempfile::tempdir().unwrap();
            let path = dir.path().join("network.m");
            std::fs::write(&path, &script).unwrap();
            assert_eq!(std::fs::read_to_string(path).unwrap(), script);
        }
    }

    fn output(h: &Host<NeuralNet>, name: &str) -> Vec<f64> {
        let Array::Numeric(m) = &h.app.current.as_ref().unwrap().outputs[name] else {
            panic!("missing {name}")
        };
        m.data().to_vec()
    }
    #[test]
    fn neural_net_apps_clustering_hits_replay_and_single_neuron() {
        let ctx = egui::Context::default();
        let ws = Snapshot::default();
        for (rows, cols, hex) in [(3, 4, true), (1, 1, false)] {
            let mut h = host(Mode::Clustering);
            h.app.spec.map_rows = rows;
            h.app.spec.map_cols = cols;
            h.app.spec.hex = hex;
            h.app.spec.seed = 19;
            start(&mut h, &ctx, &ws);
            let mut e = Engine::default();
            e.execute(&h.app.script(&ws).unwrap()).unwrap();
            close(&output(&h, "hits"), &numbers(&e, "hits"));
            close(&output(&h, "clusterIndex"), &numbers(&e, "clusterIndex"));
            // Exactly one winner per sample; a 1x1 map must select neuron 1.
            assert_eq!(output(&h, "hits").iter().sum::<f64>(), 300.);
            if rows == 1 {
                assert_eq!(output(&h, "hits"), vec![300.]);
                assert_eq!(output(&h, "clusterIndex"), vec![1.; 300]);
            }
            assert_eq!(h.app.current.as_ref().unwrap().charts.len(), 5);
            for view in 0..5 {
                h.app.view = view;
                frame(&mut h, &ctx, &ws);
            }
            let Some(toolkits::Action::ExportScript(code)) = h.export(&ws) else {
                panic!()
            };
            e.execute(&code).unwrap();
            close(&output(&h, "hits"), &numbers(&e, "hits"));
        }
    }
    #[test]
    fn neural_net_apps_clustering_rows_and_invalid_map_keep_result() {
        let ctx = egui::Context::default();
        let mut expected = vec![];
        for rows in [false, true] {
            let ws = workspace(if rows {
                "x=[-2 -1 1 2;0 1 0 1].';t=0;"
            } else {
                "x=[-2 -1 1 2;0 1 0 1];t=0;"
            });
            let mut h = host(Mode::Clustering);
            h.app.spec.example = false;
            h.app.spec.predictors = "x".into();
            h.app.spec.rows = rows;
            h.app.spec.map_rows = 2;
            h.app.spec.map_cols = 2;
            start(&mut h, &ctx, &ws);
            if !rows {
                expected = output(&h, "hits");
            } else {
                close(&expected, &output(&h, "hits"));
            }
            h.app.spec.map_rows = 0;
            neural_net_apps::start_training(&mut h, &ctx, &ws);
            frame(&mut h, &ctx, &ws);
            assert_eq!(h.status().1, toolkits::AppStatus::Error);
            assert!(h.status().0.contains("map size"));
            close(&expected, &output(&h, "hits"));
        }
    }
    #[test]
    fn neural_net_apps_time_series_metrics_prediction_replay_all_problems() {
        let ctx = egui::Context::default();
        let ws = Snapshot::default();
        for problem in [Problem::Narx, Problem::Nar, Problem::InputOutput] {
            let mut h = host(Mode::TimeSeries);
            h.app.spec.problem = problem;
            h.app.spec.seed = 17;
            start(&mut h, &ctx, &ws);
            let mut e = Engine::default();
            let script = h.app.script(&ws).unwrap();
            e.execute(&script).unwrap();
            close(
                &h.app.current.as_ref().unwrap().metrics,
                &numbers(&e, "app_nn_metrics"),
            );
            close(
                &output(&h, "app_nn_prediction"),
                &numbers(&e, "app_nn_prediction"),
            );
            assert_eq!(output(&h, "app_nn_prediction").len(), 20);
            // Independent MSE calculation from the exported targets/outputs and
            // known divideblock indices, not a reference generated by MatRust.
            let targets = numbers(&e, "app_nn_targets");
            let y = numbers(&e, "y");
            e.execute("a=tr.trainInd;b=tr.valInd;c=tr.testInd;")
                .unwrap();
            for (k, name) in ["a", "b", "c"].into_iter().enumerate() {
                let indices = numbers(&e, name);
                let mse = indices
                    .iter()
                    .map(|i| {
                        let j = *i as usize - 1;
                        (targets[j] - y[j]).powi(2)
                    })
                    .sum::<f64>()
                    / indices.len() as f64;
                close(&[mse], &[h.app.current.as_ref().unwrap().metrics[k]]);
            }
            assert_eq!(targets.len(), 158);
            // Construction: oscillator is exactly zero before the first step.
            assert!(targets[..29].iter().all(|v| *v == 0.));
            assert!((targets[29] - 0.05).abs() < 1e-14);
            assert_eq!(h.app.current.as_ref().unwrap().charts.len(), 7);
            for view in 0..7 {
                h.app.view = view;
                frame(&mut h, &ctx, &ws);
            }
            let Some(toolkits::Action::ExportScript(code)) = h.export(&ws) else {
                panic!()
            };
            e.execute(&code).unwrap();
            close(&output(&h, "y"), &numbers(&e, "y"));
        }
    }
    #[test]
    fn neural_net_apps_time_series_cells_rows_and_delay_error() {
        let ctx = egui::Context::default();
        let mut expected = vec![];
        for (code, rows) in [
            ("x=1:20;t=sin(x);", false),
            ("x=(1:20).';t=sin(x);", true),
            ("x=con2seq(1:20);t=con2seq(sin(1:20));", false),
            ("x=con2seq(1:20).';t=con2seq(sin(1:20)).';", true),
        ] {
            let ws = workspace(code);
            let mut h = host(Mode::TimeSeries);
            h.app.spec.example = false;
            h.app.spec.predictors = "x".into();
            h.app.spec.responses = "t".into();
            h.app.spec.rows = rows;
            start(&mut h, &ctx, &ws);
            if expected.is_empty() {
                expected = output(&h, "y");
            } else {
                close(&expected, &output(&h, "y"));
            }
            h.app.spec.input_delay = 30;
            h.app.start(&ws).unwrap();
            settle(&mut h, &ctx, &ws);
            assert_eq!(h.status().1, toolkits::AppStatus::Error);
            assert!(h.status().0.contains("maximum delay"), "{}", h.status().0);
            close(&expected, &output(&h, "y"));
            frame(&mut h, &ctx, &ws);
        }
    }
    #[test]
    fn neural_net_apps_dynamic_and_som_stop_replay() {
        for mode in [Mode::Clustering, Mode::TimeSeries] {
            let ctx = egui::Context::default();
            let ws = Snapshot::default();
            let mut h = host(mode);
            h.app.spec.epochs = 10000;
            h.app.start(&ws).unwrap();
            h.app.stop.store(true, Ordering::Relaxed);
            settle(&mut h, &ctx, &ws);
            assert_eq!(
                h.app.current.as_ref().expect("stopped result").stop,
                "User stop."
            );
            let mut e = Engine::default();
            e.execute(&h.app.script(&ws).unwrap()).unwrap();
            let name = h.app.names[2].clone();
            close(&output(&h, &name), &numbers(&e, &name));
            if mode == Mode::TimeSeries {
                close(
                    &output(&h, "app_nn_prediction"),
                    &numbers(&e, "app_nn_prediction"),
                );
            }
        }
    }
    #[test]
    fn neural_net_apps_new_modes_headless_and_20000_steps() {
        let ctx = egui::Context::default();
        for mode in [Mode::Clustering, Mode::TimeSeries] {
            let ws = Snapshot::default();
            let mut h = host(mode);
            frame(&mut h, &ctx, &ws);
            start(&mut h, &ctx, &ws);
            frame(&mut h, &ctx, &ws);
            h.app.names[0] = "bad;name".into();
            frame(&mut h, &ctx, &ws);
            assert_eq!(h.status().1, toolkits::AppStatus::Error);
        }
        let ws = workspace("x=sin((1:20000)*.02);t=cos((1:20000)*.02);");
        let mut h = host(Mode::TimeSeries);
        h.app.spec.example = false;
        h.app.spec.predictors = "x".into();
        h.app.spec.responses = "t".into();
        h.app.spec.epochs = 0;
        h.app.spec.algorithm = "trainscg".into();
        start(&mut h, &ctx, &ws);
        for view in 0..7 {
            h.app.view = view;
            frame(&mut h, &ctx, &ws);
        }
        assert_eq!(output(&h, "y").len(), 19998);
        assert_eq!(AppKind::ALL.len(), 46);
        for mode in [
            Mode::Fitting,
            Mode::PatternRecognition,
            Mode::Clustering,
            Mode::TimeSeries,
        ] {
            assert!(AppKind::ALL.contains(&mode.kind()));
        }
    }

    #[test]
    fn neural_net_apps_time_series_constant_zero_construction_and_trainers() {
        let ctx = egui::Context::default();
        let ws = workspace("x=1:24;t=zeros(1,24);");
        for problem in [Problem::Narx, Problem::Nar, Problem::InputOutput] {
            for algorithm in neural_net_apps::algorithms() {
                let mut h = host(Mode::TimeSeries);
                h.app.spec.example = false;
                h.app.spec.predictors = "x".into();
                h.app.spec.responses = "t".into();
                h.app.spec.problem = problem;
                h.app.spec.algorithm = algorithm.into();
                h.app.spec.epochs = 1;
                start(&mut h, &ctx, &ws);
                close(&output(&h, "y"), &[0.; 22]);
                close(&output(&h, "app_nn_prediction"), &[0.; 20]);
                close(&h.app.current.as_ref().unwrap().metrics[..3], &[0.; 3]);
            }
        }
    }
    #[test]
    #[ignore = "manual release benchmark; changes the process worker pool"]
    fn neural_net_apps_extended_release_benchmark() {
        let workers = std::env::var("NN_APP_WORKERS")
            .unwrap_or_else(|_| "1".into())
            .parse()
            .unwrap();
        matrust::initialize_worker_pool(workers).unwrap();
        for (mode, samples) in [
            (Mode::Clustering, 300),
            (Mode::Clustering, 2000),
            (Mode::TimeSeries, 160),
            (Mode::TimeSeries, 20000),
        ] {
            let ws = workspace(&format!(
                "x=[sin((1:{samples})*.02);cos((1:{samples})*.03)];t=cos((1:{samples})*.02);"
            ));
            let ctx = egui::Context::default();
            let mut h = host(mode);
            h.app.spec.example = false;
            h.app.spec.predictors = "x".into();
            h.app.spec.responses = "t".into();
            h.app.spec.algorithm = "trainscg".into();
            h.app.spec.epochs = 5;
            let started = Instant::now();
            start(&mut h, &ctx, &ws);
            let elapsed = started.elapsed();
            frame(&mut h, &ctx, &ws);
            let started = Instant::now();
            for _ in 0..20 {
                frame(&mut h, &ctx, &ws);
            }
            eprintln!(
                "extended neural benchmark: workers={workers}, mode={mode:?}, samples={samples}, hidden=3, map=10x10, epochs=5, train_and_prepare_ms={:.3}, frame_mean_ms={:.3}",
                elapsed.as_secs_f64() * 1000.,
                started.elapsed().as_secs_f64() * 50.
            );
        }
    }

    #[test]
    fn neural_net_apps_engine_som_hook_regression() {
        use matrust::engine::neural_app_hooks;
        use std::{cell::RefCell, ops::ControlFlow, rc::Rc};
        let epochs = Rc::new(RefCell::new(vec![]));
        let seen = epochs.clone();
        let _hook = neural_app_hooks::progress(move |r| {
            let epoch = r.epoch;
            seen.borrow_mut().push(epoch);
            if epoch == 2 {
                ControlFlow::Break(())
            } else {
                ControlFlow::Continue(())
            }
        });
        let mut e = Engine::default();
        e.execute(
            "n=selforgmap([1 1]);n.trainParam.epochs=4;[n,tr]=train(n,[1 2 3]);stop=tr.stop;",
        )
        .unwrap();
        assert_eq!(*epochs.borrow(), vec![0, 1, 2]);
        let Some(Array::Character(stop)) = e.array("stop") else {
            panic!()
        };
        assert_eq!(String::from_utf16_lossy(stop.units()), "User stop.");
    }
    #[test]
    fn neural_net_apps_engine_long_series_cells_regression() {
        let mut e = Engine::default();
        e.execute("x=con2seq(1:20000);t=con2seq(2*(1:20000));n=narxnet(1:2,1:2,3);[xs,xi,ai,ts]=preparets(n,x,{},t);").unwrap();
        e.execute("assert(isequal(size(xs),[2 19998]));assert(xs{1,1}==3 && xs{2,1}==6 && ts{end}==40000);").unwrap();
    }

    #[test]
    fn neural_net_apps_engine_som_planes_series_budget_regression() {
        for channels in [2, 16] {
            let mut e = Engine::default();
            // Every weight-plane bin is populated by an exact evenly spaced ramp.
            e.execute(&format!("n=configure(selforgmap([10 10]),[zeros({channels},1) ones({channels},1)]);n=setwb(n,repmat(linspace(-1,1,100).',{channels},1));plotsomplanes(n);")).unwrap();
            let figure = e.figure.as_ref().unwrap();
            assert_eq!(figure.all_axes().count(), channels);
            assert!(
                figure
                    .all_axes()
                    .map(|(_, a)| a.series.len())
                    .sum::<usize>()
                    <= 64
            );
            for (_, axes) in figure.all_axes() {
                // Each neuron remains four triangles, regardless of shade quantization.
                assert_eq!(
                    axes.series
                        .iter()
                        .filter(|s| s.kind == matrust::plot::PlotKind::Patch)
                        .map(|s| s.points.len())
                        .sum::<usize>(),
                    1200
                );
            }
        }
    }

    #[test]
    fn neural_net_apps_many_channels_keep_plots_bounded() {
        let ctx = egui::Context::default();
        let ws = workspace("x=repmat(sin(1:40),10,1);t=zeros(32,40);");
        let mut h = host(Mode::TimeSeries);
        h.app.spec.example = false;
        h.app.spec.predictors = "x".into();
        h.app.spec.responses = "t".into();
        h.app.spec.epochs = 0;
        h.app.spec.algorithm = "trainscg".into();
        h.app.spec.horizon = 3;
        start(&mut h, &ctx, &ws);
        close(&output(&h, "y"), &vec![0.; 32 * 38]);
        close(&output(&h, "app_nn_prediction"), &[0.; 96]);
        for view in 0..7 {
            h.app.view = view;
            frame(&mut h, &ctx, &ws);
        }
    }

    /// Run separately in the release-derived gate profile with NN_APP_WORKERS=1
    /// and NN_APP_WORKERS=N. No nested pools; production engine policy is reused.
    #[test]
    #[ignore = "manual release benchmark; changes the process worker pool"]
    fn neural_net_apps_release_benchmark() {
        let workers = std::env::var("NN_APP_WORKERS")
            .unwrap_or_else(|_| "1".into())
            .parse()
            .unwrap();
        matrust::initialize_worker_pool(workers).unwrap();
        for samples in [1000, 50000] {
            let ws = workspace(&format!("x=linspace(-pi,pi,{samples});t=sin(x);"));
            let ctx = egui::Context::default();
            let mut h = host(Mode::Fitting);
            h.app.spec.example = false;
            h.app.spec.predictors = "x".into();
            h.app.spec.responses = "t".into();
            h.app.spec.algorithm = "trainscg".into();
            h.app.spec.epochs = 5;
            let started = Instant::now();
            start(&mut h, &ctx, &ws);
            let elapsed = started.elapsed();
            frame(&mut h, &ctx, &ws);
            let started = Instant::now();
            for _ in 0..20 {
                frame(&mut h, &ctx, &ws);
            }
            eprintln!(
                "neural benchmark: workers={workers}, samples={samples}, hidden=3, epochs=5, train_and_prepare_ms={:.3}, frame_mean_ms={:.3}",
                elapsed.as_secs_f64() * 1000.,
                started.elapsed().as_secs_f64() * 50.
            );
        }
    }

    #[test]
    fn compact_confusion_four_digit_counts_fit_their_cells() {
        let (ctx, _desktop) = redesign_app(session::DesktopTheme::Light);
        let ws = workspace("x=zeros(1,10000); t=repmat(eye(10),1,1000);");
        let mut host = host(Mode::PatternRecognition);
        host.app.spec.example = false;
        host.app.spec.predictors = "x".into();
        host.app.spec.responses = "t".into();
        host.app.spec.epochs = 0;
        start(&mut host, &ctx, &ws);
        let counts = &host.app.current.as_ref().unwrap().confusion;
        assert_eq!(counts.len(), 100);
        assert!(counts.as_chunks::<10>().0.contains(&[1000.; 10]));
        host.app.view = 3;
        let mut render = || {
            app_framework_tests::layout_frame(&ctx, egui::vec2(900., 554.), vec![], |ui| {
                host.show(ui, &ws, false, &mut None)
            })
        };
        render();
        let output = render();
        let is_count = |s: &str| matches!(s, "1000" | "1.0k" | "1k");
        for shape in &output.shapes {
            if let egui::Shape::Text(t) = &shape.shape
                && is_count(t.galley.text())
            {
                assert!(t.galley.job.sections[0].format.font_id.size >= 9.);
            }
        }
        let texts = clear_panes_texts(&output);
        let mut row: Vec<_> = texts
            .iter()
            .filter(|t| is_count(&t.text) && t.rect.left() > apps::COLUMN)
            .collect();
        row.sort_by(|a, b| a.rect.left().total_cmp(&b.rect.left()));
        assert_eq!(row.len(), 10);
        let cell_width = row[1].rect.center().x - row[0].rect.center().x;
        for cell in &row {
            assert!(cell.clip.contains_rect(cell.glyphs));
            assert!(
                cell.rect.width() <= cell_width - 2.,
                "{:?} in {cell_width} pt",
                cell.rect
            );
        }
        for pair in row.windows(2) {
            assert!((pair[0].rect.center().y - pair[1].rect.center().y).abs() < 1.);
            assert!(pair[0].rect.right() + 2. <= pair[1].rect.left());
        }
    }

    #[test]
    fn compact_confusion_labels_align_with_every_class() {
        let (ctx, _app) = redesign_app(session::DesktopTheme::Light);
        let ws = workspace("t=repmat(eye(10),1,8); x=t;");
        let mut h = host(Mode::PatternRecognition);
        h.app.spec.example = false;
        h.app.spec.predictors = "x".into();
        h.app.spec.responses = "t".into();
        start(&mut h, &ctx, &ws);
        let trained = h.app.current.as_ref().unwrap();
        assert_eq!(trained.confusion.len(), 100);
        assert_eq!(trained.charts.len(), 5);
        h.app.view = 3;
        for n in 2..=10 {
            let mut counts = vec![0.; n * n];
            for i in 0..n {
                counts[i * n + i] = (11 + i) as f64;
            }
            h.app.current.as_mut().unwrap().confusion = counts;
            // The app body below the native bars of a 900 x 650 window.
            let mut render = || {
                app_framework_tests::layout_frame(&ctx, egui::vec2(900., 554.), vec![], |ui| {
                    h.show(ui, &ws, false, &mut None)
                })
            };
            render();
            let texts = clear_panes_texts(&render());
            for i in 0..n {
                let count = (11 + i).to_string();
                let cell = texts
                    .iter()
                    .find(|t| {
                        t.text == count
                            && t.rect.left() > apps::COLUMN
                            && t.clip.contains_rect(t.glyphs)
                    })
                    .expect("visible diagonal count");
                let class = (1 + i).to_string();
                let mut ticks = texts.iter().filter(|t| {
                    t.text == class
                        && t.rect.left() > apps::COLUMN
                        && t.clip.contains_rect(t.glyphs)
                });
                assert!(
                    ticks.clone().any(|t| t.rect.top() > cell.rect.bottom()
                        && (t.rect.center().x - cell.rect.center().x).abs() < 2.),
                    "missing target {class} of {n}"
                );
                assert!(
                    ticks.any(|t| t.rect.right() < cell.rect.left()
                        && (t.rect.center().y - cell.rect.center().y).abs() < 2.),
                    "missing prediction {class} of {n}"
                );
            }
        }
    }

    #[test]
    fn confusion_matrix_draws_class_ticks_and_legible_counts() {
        let ws = Snapshot::default();
        let mut h = host(Mode::PatternRecognition);
        let ctx = egui::Context::default();
        start(&mut h, &ctx, &ws);
        h.app.view = 3;
        h.app.current.as_mut().unwrap().confusion = vec![70., 0., 5., 125.];
        for theme in [session::DesktopTheme::Light, session::DesktopTheme::Dark] {
            let (ctx, _app) = redesign_app(theme);
            let mut render = || {
                crate::headless::run(
                    &ctx,
                    egui::RawInput {
                        screen_rect: Some(egui::Rect::from_min_size(
                            egui::Pos2::ZERO,
                            egui::vec2(900., 650.),
                        )),
                        ..Default::default()
                    },
                    |ui| h.app.main_ui(ui),
                )
            };
            render();
            let out = render();
            let texts = clear_panes_texts(&out);
            for class in ["1", "2"] {
                assert_eq!(texts.iter().filter(|t| t.text == class).count(), 2);
            }
            assert!(texts.iter().all(|t| !t.text.contains('.')));
            let palette = tokens::palette(&ctx);
            for (count, fill) in [
                ("125", palette.accent),
                ("0", palette.surface.lerp_to_gamma(palette.accent, 0.1)),
            ] {
                let text = out
                    .shapes
                    .iter()
                    .find_map(|s| match &s.shape {
                        egui::Shape::Text(t) if t.galley.text() == count => Some(t),
                        _ => None,
                    })
                    .expect("cell count");
                assert!(text.galley.job.sections[0].format.font_id.size >= tokens::HEADING);
                assert!(
                    clear_panes_contrast(text.galley.job.sections[0].format.color, fill) >= 4.5
                );
            }
        }
    }
}
