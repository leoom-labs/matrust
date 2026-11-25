// Expected calls and line counts follow this fixture's construction, not engine output.
mod profiler_app_tests {
    use super::*;
    use apps::profiler_app::{Profiler, Sort, decode, validate_code};
    use apps::{App, AppKind, Host, Snapshot};
    use matrust::{Array, CharacterRow, Matrix, StructArray};
    use std::sync::{Arc, atomic::AtomicBool};

    const SOURCE: &str = "function y=main()\ny=0;\nfor k=1:3\ny=y+sub(k);\nend\nend\nfunction y=sub(x)\n% UTF-8 source: café λ\na=x+1;\nb=a*2;\nc=b-1;\ny=sum([c x]);\nend\n";

    fn fixture() -> (tempfile::TempDir, Snapshot, Host<Profiler>) {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("main.m"), SOURCE).unwrap();
        let mut ws = Snapshot::default();
        ws.set_folder(Some(dir.path().to_owned()));
        let mut host = Host::new(Profiler::default());
        host.set_folder(Some(dir.path()));
        host.app.code = "answer = main();".into();
        (dir, ws, host)
    }
    fn settle(host: &mut Host<Profiler>, ctx: &egui::Context, ws: &Snapshot) {
        let until = Instant::now() + Duration::from_secs(20);
        loop {
            host.refresh(ctx, ws);
            if !host.running() {
                break;
            }
            assert!(Instant::now() < until, "{}", host.status().0);
            std::thread::sleep(Duration::from_millis(2));
        }
    }
    fn signatures(host: &Host<Profiler>) -> Vec<(String, usize)> {
        let mut rows: Vec<_> = host
            .app
            .current
            .as_ref()
            .unwrap()
            .functions
            .iter()
            .map(|f| (f.name.clone(), f.calls))
            .collect();
        rows.sort();
        rows
    }
    fn expected() -> Vec<(String, usize)> {
        vec![
            ("main".into(), 1),
            ("main>sub".into(), 3),
            ("sum".into(), 3),
        ]
    }
    fn row(host: &Host<Profiler>, name: &str) -> usize {
        host.app
            .current
            .as_ref()
            .unwrap()
            .functions
            .iter()
            .position(|f| f.name == name)
            .unwrap()
    }
    fn good(host: &Host<Profiler>) {
        assert_eq!(
            host.status().1,
            toolkits::AppStatus::Success,
            "{}",
            host.status().0
        );
    }

    #[test]
    fn profiler_app_function_graph_exact_calls_lines_sort_navigation_and_history() {
        let (_dir, ws, mut host) = fixture();
        let ctx = egui::Context::default();
        host.app.history = true;
        host.app.run();
        settle(&mut host, &ctx, &ws);
        good(&host);
        assert_eq!(signatures(&host), expected());
        let main = row(&host, "main");
        let sub = row(&host, "main>sub");
        let sum = row(&host, "sum");
        let profile = host.app.current.as_ref().unwrap();
        let f = &profile.functions;
        assert_eq!(
            f[main]
                .lines
                .values()
                .map(|l| (l.number, l.calls))
                .collect::<Vec<_>>(),
            [(2, 1), (3, 1), (4, 3)]
        );
        assert_eq!(
            f[sub]
                .lines
                .values()
                .map(|l| (l.number, l.calls))
                .collect::<Vec<_>>(),
            [(9, 3), (10, 3), (11, 3), (12, 3)]
        );
        assert_eq!(f[sub].source[7], "% UTF-8 source: café λ");
        assert!(Arc::ptr_eq(&f[main].source, &f[sub].source));
        assert_eq!(f[sub].hottest.len(), 4);
        assert!(f[sub].hottest.iter().all(|n| (9..=12).contains(n)));
        assert_eq!(f[main].children[0].index, sub);
        assert_eq!(f[main].children[0].calls, 3);
        assert_eq!(f[sub].parents[0].index, main);
        assert_eq!(f[sub].parents[0].calls, 3);
        assert_eq!(f[sub].children[0].index, sum);
        assert!(f[sum].lines.is_empty() && f[sum].source.is_empty());
        assert!(
            f.iter()
                .all(|f| f.self_time >= 0. && f.self_time <= f.total)
        );
        assert!(profile.total >= f[main].total && profile.total.is_finite());
        let Array::Struct(p) = &profile.info else {
            panic!()
        };
        let Array::Numeric(h) = p
            .get(p.field_index("FunctionHistory").unwrap(), 0)
            .unwrap()
            .as_ref()
        else {
            panic!()
        };
        assert_eq!((h.rows(), h.cols()), (2, 14)); // main + 3 sub + 3 sum, each enter/exit.
        host.app.sort_by(Sort::Calls);
        assert_eq!(
            host.app
                .order
                .iter()
                .map(|i| host.app.current.as_ref().unwrap().functions[*i].calls)
                .collect::<Vec<_>>(),
            [3, 3, 1]
        );
        host.app.sort_by(Sort::Name);
        assert_eq!(
            host.app
                .order
                .iter()
                .map(|i| host.app.current.as_ref().unwrap().functions[*i]
                    .name
                    .as_str())
                .collect::<Vec<_>>(),
            ["main", "main>sub", "sum"]
        );
        host.app.sort_by(Sort::Name);
        assert_eq!(host.app.order, [sum, sub, main]);
        host.app.navigate(main);
        host.app.navigate(sub);
        assert_eq!(host.app.selected, Some(sub));
        host.app.back();
        assert_eq!(host.app.selected, Some(main));
        host.app.navigate(sum);
        host.app.back();
        assert_eq!(host.app.selected, Some(main));
        host.app.back();
        assert_eq!(host.app.selected, None);
        for sort in [Sort::Type, Sort::Total, Sort::SelfTime] {
            host.app.sort_by(sort);
            assert_eq!(host.app.order.len(), 3);
        }
        assert_eq!(host.app.results()[1].1, "3");
    }

    #[test]
    fn profiler_app_saved_script_and_export_reproduce_function_table() {
        let (dir, ws, mut host) = fixture();
        host.app.run();
        settle(&mut host, &egui::Context::default(), &ws);
        good(&host);
        let script = host.app.script(&ws).unwrap();
        assert!(
            script.contains("profile on -nohistory;")
                && script.contains("profile off;")
                && script.contains("profile_info = profile('info');")
        );
        let Some(toolkits::Action::SaveAppScript(AppKind::Profiler, saved)) = host.generate(&ws, 0)
        else {
            panic!()
        };
        assert!(saved.ends_with(&script));
        let mut fresh = matrust::Engine::default();
        fresh.set_file_directory(dir.path()).unwrap();
        fresh.execute(&saved).unwrap();
        let result = decode(&fresh.array("profile_info").unwrap(), None).unwrap();
        let mut signature: Vec<_> = result
            .functions
            .iter()
            .map(|f| (f.name.clone(), f.calls))
            .collect();
        signature.sort();
        assert_eq!(signature, expected());
        host.app.name = "timings".into();
        let Some(toolkits::Action::ExportScript(export)) = host.export(&ws) else {
            panic!()
        };
        fresh.execute("timings=42;").unwrap();
        fresh.execute(&export).unwrap();
        let result = decode(&fresh.array("timings").unwrap(), None).unwrap();
        assert_eq!(result.functions.len(), 3);
        // Importing current results preserves exact timings and has a standalone script.
        let info = host.app.current.as_ref().unwrap().info.clone();
        host.app.use_snapshot(info.clone()).unwrap();
        let script = host.app.script(&ws).unwrap();
        let mut fresh = matrust::Engine::default();
        fresh.execute(&script).unwrap();
        let restored = decode(&fresh.array("timings").unwrap(), None).unwrap();
        let Some(Array::Numeric(elapsed)) = fresh.array("app_profiler_elapsed") else {
            panic!()
        };
        assert_eq!(
            elapsed.as_scalar().unwrap(),
            host.app.current.as_ref().unwrap().total
        );
        assert!(
            restored.info.same_snapshot(&info)
                || restored
                    .functions
                    .iter()
                    .zip(&host.app.current.as_ref().unwrap().functions)
                    .all(|(a, b)| a.total == b.total && a.calls == b.calls && a.name == b.name)
        );
    }

    #[test]
    fn profiler_app_export_preserves_workspace_and_displayed_snapshot() {
        for code in ["x = x + 1;", "x = x + 1; sum(x);"] {
            let mut engine = matrust::Engine::default();
            engine.execute("x=9; keep=7; app_profiler_elapsed=123; app_profiler_v0=456; app_profiler_snapshot=789; app_profiler_export_snapshot=321;").unwrap();
            let before = engine.workspace();
            let mut ws = Snapshot::default();
            ws.sync(&before);
            let mut host = Host::new(Profiler::default());
            host.app.code = code.into();
            host.app.name = "displayed_profile".into();
            host.app.run();
            settle(&mut host, &egui::Context::default(), &ws);
            good(&host);
            let shown = host.app.current.as_ref().unwrap().info.clone();
            let Some(toolkits::Action::ExportScript(export)) = host.export(&ws) else {
                panic!("snapshot export")
            };
            apps::execute_export(&mut engine, &export).unwrap();
            assert_eq!(
                engine.variables["x"].data(),
                &[9.],
                "Export must not rerun x = x + 1"
            );
            let after = engine.workspace();
            assert_eq!(after.len(), before.len() + 1);
            for (name, value) in &before {
                assert!(value.same_snapshot(&after[name]), "Export changed {name}");
            }
            let difference = matrust::engine::compare::array_diff(
                "profile",
                &shown,
                &engine.array("displayed_profile").unwrap(),
                &AtomicBool::new(false),
            )
            .unwrap();
            assert_eq!(
                difference.status,
                matrust::engine::compare::DiffStatus::Same,
                "{difference:?}"
            );
            let script = host.app.script(&ws).unwrap();
            assert!(script.contains(&format!("profile on -nohistory;\n{code}")));
            assert!(
                script.contains("profile off;")
                    && script.contains("displayed_profile = profile('info');")
            );
        }
    }

    #[test]
    fn profiler_app_total_run_time_includes_code_without_function_calls() {
        let mut host = Host::new(Profiler::default());
        host.app.code = "x=0; for k=1:1000, x=x+1; end".into();
        host.app.run();
        settle(&mut host, &egui::Context::default(), &Snapshot::default());
        good(&host);
        let profile = host.app.current.as_ref().unwrap();
        assert!(profile.functions.is_empty());
        assert!(profile.total > 0.);
    }

    #[test]
    fn profiler_app_parser_and_reserved_export_names() {
        for code in [
            "sum(1:3)",
            "a=1; b=2; sum([a b]);",
            "sum(1:3) % trailing comment",
            "for k=1:3\nsum(k);\nend",
        ] {
            validate_code(code).unwrap();
        }
        for code in [
            "",
            "% only comment",
            "if true",
            "a=(1;",
            "end; profile off",
            "function y=f()\ny=1;\nend",
        ] {
            assert!(validate_code(code).is_err(), "{code}");
        }
        assert!(validate_code(&"a".repeat(16385)).is_err());
        let ws = Snapshot::default();
        let mut host = Host::new(Profiler::default());
        host.app.run();
        for name in [
            "x; disp(1)",
            "a b",
            "1a",
            "end",
            "mrexp_old",
            "mrexp_had",
            "mrexp_complete",
            "matrust_export_error",
            "app_profiler_sources",
            "profile",
            "profsave",
        ] {
            host.app.name = name.into();
            assert!(host.app.script(&ws).is_err(), "{name}");
            assert!(host.export(&ws).is_none());
        }
        host.app.name = "timings_ok".into();
        assert!(host.export(&ws).is_none(), "no displayed profile yet");
        settle(&mut host, &egui::Context::default(), &ws);
        good(&host);
        assert!(host.export(&ws).is_some());
    }

    #[test]
    fn profiler_app_error_status_keeps_last_good_snapshot_for_export() {
        let (dir, ws, mut host) = fixture();
        let ctx = egui::Context::default();
        host.app.run();
        settle(&mut host, &ctx, &ws);
        good(&host);
        let total = host.app.current.as_ref().unwrap().total;
        host.app.code = "error('profile fixture failed');".into();
        host.app.run();
        settle(&mut host, &ctx, &ws);
        assert_eq!(host.status().1, toolkits::AppStatus::Error);
        assert!(host.status().0.contains("profile fixture failed"));
        assert_eq!(signatures(&host), expected());
        assert_eq!(host.app.current.as_ref().unwrap().total, total);
        let Some(toolkits::Action::ExportScript(export)) = host.export(&ws) else {
            panic!()
        };
        let mut engine = matrust::Engine::default();
        engine.set_file_directory(dir.path()).unwrap();
        engine.execute("profile_info=73;").unwrap();
        apps::execute_export(&mut engine, &export).unwrap();
        let difference = matrust::engine::compare::array_diff(
            "profile",
            &host.app.current.as_ref().unwrap().info,
            &engine.array("profile_info").unwrap(),
            &AtomicBool::new(false),
        )
        .unwrap();
        assert_eq!(
            difference.status,
            matrust::engine::compare::DiffStatus::Same
        );
        assert!(
            engine.execute(&host.app.script(&ws).unwrap()).is_err(),
            "Generate script still runs the entered code"
        );
        host.app.code = "for".into();
        host.app.run();
        settle(&mut host, &ctx, &ws);
        assert_eq!(host.status().1, toolkits::AppStatus::Error);
        assert_eq!(signatures(&host), expected());
    }

    #[test]
    fn profiler_app_stop_cancels_runner_and_rerun_works() {
        let (_dir, ws, mut host) = fixture();
        let ctx = egui::Context::default();
        host.app.run();
        settle(&mut host, &ctx, &ws);
        host.app.code = "pause(60);".into();
        host.app.run();
        let start = Instant::now();
        host.refresh(&ctx, &ws);
        assert!(host.running());
        host.app.stop();
        settle(&mut host, &ctx, &ws);
        assert!(start.elapsed() < Duration::from_secs(5));
        assert_eq!(signatures(&host), expected());
        host.app.code = "answer=main();".into();
        host.app.run();
        settle(&mut host, &ctx, &ws);
        good(&host);
        assert_eq!(signatures(&host), expected());
    }

    #[test]
    fn profiler_app_html_report_uses_chosen_folder_and_literal_path() {
        let (_dir, ws, mut host) = fixture();
        host.app.run();
        settle(&mut host, &egui::Context::default(), &ws);
        let output = tempfile::tempdir().unwrap();
        let folder = output.path().join("report's folder");
        let request = host.app.html_request(folder.to_str().unwrap(), 1).unwrap();
        let mut runner = apps::Runner::default();
        let ctx = egui::Context::default();
        runner.submit(&ctx, request);
        let until = Instant::now() + Duration::from_secs(20);
        while runner.result().is_none() {
            assert!(Instant::now() < until);
            runner.poll(&ctx);
            std::thread::sleep(Duration::from_millis(2));
        }
        runner.result().unwrap().as_ref().unwrap();
        let index = std::fs::read_to_string(folder.join("file0.html")).unwrap();
        assert!(index.contains("main") && index.contains("sum"));
        assert!(host.app.html_request("bad\nfolder", 2).is_err());
        assert!(host.app.html_request("", 2).is_err());
    }

    #[test]
    fn profiler_app_profile_viewer_opens_desktop_and_cli_keeps_text() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("main.m"), SOURCE).unwrap();
        // Viewer is called indirectly from a script file, not detected by text matching.
        std::fs::write(dir.path().join("show_profile.m"), "profile viewer;").unwrap();
        let ctx = egui::Context::default();
        let mut desktop = Workspace::with_context(ctx.clone(), true);
        desktop.submit(worker::Job::ScriptInFolder(
            "profile on; main(); profile off; show_profile;".into(),
            dir.path().to_owned(),
        ));
        let until = Instant::now() + Duration::from_secs(20);
        while desktop.busy {
            assert!(Instant::now() < until);
            desktop.receive();
            std::thread::sleep(Duration::from_millis(2));
        }
        assert_eq!(
            desktop.toolkits.selected_tool(),
            Some(toolkits::Tool::App(AppKind::Profiler))
        );
        assert!(desktop.toolkits.apps.is_open(AppKind::Profiler));
        assert_eq!(signatures(&desktop.toolkits.apps.profiler), expected());
        let layout = desktop.toolkits.tab_layout().unwrap();
        assert_eq!(layout.selected, Some(matrust::session::ToolTab::Profiler));
        let text = serde_json::to_string(&layout).unwrap();
        let restored: matrust::session::ToolTabs = serde_json::from_str(&text).unwrap();
        assert!(restored.open.contains(matrust::session::ToolTab::Profiler));
        let mut cli = matrust::Engine::default();
        let execution = cli
            .execute("profile on; sum(1:3); profile viewer;")
            .unwrap();
        assert!(
            execution
                .output
                .iter()
                .any(|s| s.contains("Function\tCalls") && s.contains("sum\t1"))
        );
        // The same worker path supports the Inputs column's Use current / Clear actions.
        desktop.submit(worker::Job::ScriptInFolder(
            "profile clear; profile viewer;".into(),
            dir.path().to_owned(),
        ));
        while desktop.busy {
            assert!(Instant::now() < until);
            desktop.receive();
            std::thread::sleep(Duration::from_millis(2));
        }
        assert!(
            desktop
                .toolkits
                .apps
                .profiler
                .app
                .current
                .as_ref()
                .unwrap()
                .functions
                .is_empty()
        );
    }

    #[test]
    fn profiler_app_viewer_event_captures_at_call_and_consumes_once() {
        let mut engine = matrust::Engine::default();
        engine
            .execute("profile on; sum(1:3); profile viewer; profile clear; profile resume;")
            .unwrap();
        let info = engine.take_profiler_viewer().unwrap().unwrap();
        let profile = decode(&info, None).unwrap();
        assert_eq!(profile.functions.len(), 1);
        assert_eq!(
            (
                profile.functions[0].name.as_str(),
                profile.functions[0].calls
            ),
            ("sum", 1)
        );
        assert!(engine.take_profiler_viewer().is_none());
        engine.execute("status = profile('status');").unwrap();
        let Array::Struct(status) = engine.array("status").unwrap() else {
            panic!()
        };
        let Array::Character(value) = status
            .get(status.field_index("ProfilerStatus").unwrap(), 0)
            .unwrap()
            .as_ref()
        else {
            panic!()
        };
        assert_eq!(value.to_utf8(&AtomicBool::new(false)).unwrap(), "on");
    }

    #[test]
    fn profiler_app_viewer_keeps_focus_after_profiled_plot() {
        let folder = tempfile::tempdir().unwrap();
        let mut desktop = Workspace::with_context(egui::Context::default(), true);
        desktop.layout.center = Some(Default::default());
        desktop.submit(worker::Job::ScriptInFolder(
            "profile on; plot([1 2],[3 4]); profile viewer;".into(),
            folder.path().to_owned(),
        ));
        let until = Instant::now() + Duration::from_secs(20);
        while desktop.busy {
            assert!(Instant::now() < until);
            desktop.receive();
            std::thread::sleep(Duration::from_millis(2));
        }
        assert!(desktop.figure.is_some());
        assert_eq!(
            desktop.toolkits.selected_tool(),
            Some(toolkits::Tool::App(AppKind::Profiler))
        );
        assert!(
            desktop
                .toolkits
                .apps
                .profiler
                .app
                .current
                .as_ref()
                .unwrap()
                .functions
                .iter()
                .any(|f| f.name == "plot" && f.calls == 1)
        );
    }

    #[test]
    fn profiler_app_script_can_clear_its_workspace() {
        let mut host = Host::new(Profiler::default());
        host.app.code = "clear; sum(1:3);".into();
        host.app.run();
        settle(&mut host, &egui::Context::default(), &Snapshot::default());
        good(&host);
        assert!(
            host.app
                .current
                .as_ref()
                .unwrap()
                .functions
                .iter()
                .any(|f| f.name == "sum" && f.calls == 1)
        );
    }

    fn chars(value: &str) -> Array {
        CharacterRow::from_text(value, &AtomicBool::new(false))
            .unwrap()
            .into()
    }
    fn structure(fields: &[&str], rows: Vec<Vec<Array>>) -> Array {
        StructArray::from_slots(
            rows.len(),
            1,
            fields.iter().map(|s| (*s).into()).collect(),
            rows.into_iter().flatten().map(Arc::new).collect(),
            &AtomicBool::new(false),
        )
        .unwrap()
        .into()
    }
    fn synthetic(count: usize) -> Array {
        let edges = structure(&["Index", "NumCalls", "TotalTime"], vec![]);
        let rows = (0..count)
            .map(|i| {
                vec![
                    chars(&format!("function_{i:04}")),
                    chars("Built-in"),
                    chars(""),
                    Matrix::scalar((i % 7 + 1) as f64).into(),
                    Matrix::scalar(i as f64 / 1000.).into(),
                    edges.clone(),
                    edges.clone(),
                    Matrix::from_column_vec(0, 3, vec![]).unwrap().into(),
                ]
            })
            .collect();
        structure(
            &["FunctionTable"],
            vec![vec![structure(
                &[
                    "FunctionName",
                    "Type",
                    "FileName",
                    "NumCalls",
                    "TotalTime",
                    "Parents",
                    "Children",
                    "ExecutedLines",
                ],
                rows,
            )]],
        )
    }
    #[test]
    fn profiler_app_five_slowest_lines_use_fixture_times() {
        let info = synthetic(1);
        let Array::Struct(p) = &info else { panic!() };
        let Array::Struct(table) = p.get(0, 0).unwrap().as_ref() else {
            panic!()
        };
        let mut values = Vec::new();
        for (i, name) in table.field_names().iter().enumerate() {
            values.push(if name == "ExecutedLines" {
                Matrix::from_column_vec(
                    6,
                    3,
                    vec![
                        2., 3., 4., 5., 6., 7., 3., 3., 3., 3., 3., 3., 4., 3., 6., 2., 5., 1.,
                    ],
                )
                .unwrap()
                .into()
            } else if name == "TotalTime" {
                Matrix::scalar(30.).into()
            } else {
                table.get(i, 0).unwrap().as_ref().clone()
            });
        }
        let fields: Vec<_> = table.field_names().iter().map(String::as_str).collect();
        let info = structure(
            &["FunctionTable"],
            vec![vec![structure(&fields, vec![values])]],
        );
        let profile = decode(&info, None).unwrap();
        assert_eq!(profile.functions[0].hottest, [4, 6, 2, 3, 5]);
        assert_eq!(profile.functions[0].lines[&7].calls, 3);
    }

    fn frame(
        host: &mut Host<Profiler>,
        ctx: &egui::Context,
        ws: &Snapshot,
        size: egui::Vec2,
        events: Vec<egui::Event>,
    ) -> egui::FullOutput {
        ctx.run_ui(
            egui::RawInput {
                screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, size)),
                events,
                ..Default::default()
            },
            |ctx| {
                egui::CentralPanel::default().show(ctx, |ui| {
                    apps::profiler_app::show(host, ui, ws, false, &mut None);
                });
            },
        )
    }
    fn strings(output: &egui::FullOutput) -> Vec<String> {
        fn visit(s: &egui::epaint::Shape, strings: &mut Vec<String>) {
            match s {
                egui::epaint::Shape::Text(t) => strings.push(t.galley.text().to_owned()),
                egui::epaint::Shape::Vec(v) => {
                    for s in v {
                        visit(s, strings)
                    }
                }
                _ => (),
            }
        }
        let mut result = Vec::new();
        for s in &output.shapes {
            visit(&s.shape, &mut result)
        }
        result
    }
    fn click(host: &mut Host<Profiler>, ctx: &egui::Context, ws: &Snapshot, label: &str) {
        let size = egui::vec2(1440., 900.);
        let output = frame(host, ctx, ws, size, vec![]);
        let position = output
            .shapes
            .iter()
            .find_map(|s| super::text_position(&s.shape, label))
            .unwrap_or_else(|| panic!("missing {label}"));
        for pressed in [true, false] {
            frame(
                host,
                ctx,
                ws,
                size,
                vec![
                    egui::Event::PointerMoved(position),
                    egui::Event::PointerButton {
                        pos: position,
                        button: egui::PointerButton::Primary,
                        pressed,
                        modifiers: egui::Modifiers::NONE,
                    },
                ],
            );
        }
    }

    #[test]
    fn profiler_app_rendered_row_child_back_and_dim_source_lines() {
        let (_dir, ws, mut host) = fixture();
        let ctx = egui::Context::default();
        host.app.run();
        settle(&mut host, &ctx, &ws);
        good(&host);
        host.app.sort_by(Sort::Name);
        click(&mut host, &ctx, &ws, "main");
        assert_eq!(host.app.selected, Some(row(&host, "main")));
        click(&mut host, &ctx, &ws, "main>sub");
        assert_eq!(host.app.selected, Some(row(&host, "main>sub")));
        let output = frame(&mut host, &ctx, &ws, egui::vec2(1440., 900.), vec![]);
        let labels = strings(&output);
        assert!(labels.iter().any(|s| s.contains("café λ")));
        assert!(
            labels
                .iter()
                .any(|s| s.starts_with('•') && s.contains("a=x+1;"))
        );
        fn dim(shape: &egui::epaint::Shape, color: egui::Color32) -> bool {
            match shape {
                egui::epaint::Shape::Text(t) if t.galley.text().contains("café λ") => {
                    t.galley.job.sections[0].format.color == color
                }
                egui::epaint::Shape::Vec(v) => v.iter().any(|s| dim(s, color)),
                _ => false,
            }
        }
        assert!(
            output
                .shapes
                .iter()
                .any(|s| dim(&s.shape, crate::tokens::palette(&ctx).text_disabled))
        );
        click(&mut host, &ctx, &ws, "Back");
        assert_eq!(host.app.selected, Some(row(&host, "main")));
        click(&mut host, &ctx, &ws, "Back");
        assert_eq!(host.app.selected, None);
    }

    #[test]
    fn profiler_app_headless_frames_empty_three_and_five_hundred_functions() {
        let ctx = egui::Context::default();
        let ws = Snapshot::default();
        let mut host = Host::new(Profiler::default());
        let output = frame(&mut host, &ctx, &ws, egui::vec2(1440., 900.), vec![]);
        assert!(
            strings(&output)
                .iter()
                .any(|s| s.contains("Run and Profile"))
        );
        for count in [3, 500] {
            host.app.use_snapshot(synthetic(count)).unwrap();
            settle(&mut host, &ctx, &ws);
            good(&host);
            let output = frame(&mut host, &ctx, &ws, egui::vec2(1440., 900.), vec![]);
            let labels = strings(&output);
            assert!(labels.iter().any(|s| s.contains("Function")));
            assert_eq!(host.app.order.len(), count);
            assert!(
                labels.iter().filter(|s| s.starts_with("function_")).count() < 50,
                "table must virtualize rows"
            );
            host.app.navigate(0);
            let output = frame(&mut host, &ctx, &ws, egui::vec2(1440., 900.), vec![]);
            assert!(
                strings(&output)
                    .iter()
                    .any(|s| s.contains("no source listing"))
            );
            host.app.back();
            let output = frame(&mut host, &ctx, &ws, egui::vec2(800., 700.), vec![]);
            assert!(!output.shapes.is_empty());
        }
    }
}
