// Data Statistics: generated script, independent oracle, exact overlay ownership,
// workspace inputs, no-clobber outputs, last-good errors and a headless app tab.
mod data_statistics_tests {
    use super::*;
    use apps::data_statistics::{DataStatistics, FIELDS, MAX_FIGURE_POINTS};
    use apps::{App, AppKind, Host, Snapshot};
    use toolkits::Tool;

    const SETUP: &str =
        "figure(7); h = plot([1 2 2 NaN 5 8 10], [3 NaN 3 7 9 11 15], 'DisplayName', 'Measured');";
    // NumPy/SciPy, tool/refs/data_statistics/generate.py (NaNs removed per axis).
    const X: [f64; 7] = [1., 10., 4.666666666666667, 3.5, 2., 3.669695718539436, 9.];
    const Y: [f64; 7] = [3., 15., 8., 8., 3., 4.69041575982343, 12.];

    fn snapshot(engine: &matrust::Engine) -> Snapshot {
        let mut ws = Snapshot::default();
        ws.sync(&engine.workspace());
        ws
    }
    fn figures(engine: &matrust::Engine) -> BTreeMap<u32, Figure> {
        engine.figures().map(|(n, f)| (n, f.clone())).collect()
    }
    fn host() -> (matrust::Engine, Host<DataStatistics>, Snapshot) {
        let mut e = matrust::Engine::default();
        e.execute(SETUP).unwrap();
        let mut host = Host::new(DataStatistics::default());
        host.app.sync_figures(&figures(&e));
        let ws = snapshot(&e);
        (e, host, ws)
    }
    fn compute(host: &mut Host<DataStatistics>, ws: &Snapshot) -> Result<(), String> {
        host.app.prepare(ws);
        let request = host.request(ws)?;
        let outputs = apps::run(&mut matrust::Engine::default(), request)?;
        host.app.receive(&outputs)
    }
    fn close(actual: &[f64], expected: &[f64]) {
        for (actual, expected) in actual.iter().zip(expected) {
            assert!((actual - expected).abs() < 1e-12, "{actual} != {expected}");
        }
    }
    fn struct_values(e: &mut matrust::Engine, name: &str) -> Vec<f64> {
        let fields = FIELDS.map(|f| format!("{name}.{f}")).join(" ");
        e.execute(&format!("observed = [{fields}];")).unwrap();
        e.variables["observed"].data().to_vec()
    }
    fn source_figure(e: &matrust::Engine) -> &Figure {
        e.figures().find(|(n, _)| *n == 7).unwrap().1
    }

    #[test]
    fn data_statistics_generated_script_matches_table_and_numpy_scipy() {
        let (_, mut host, ws) = host();
        compute(&mut host, &ws).unwrap();
        let shown = host.app.current.as_ref().unwrap();
        close(&shown.x, &X);
        close(&shown.y, &Y);
        assert_eq!((shown.count, shown.ignored), (7, [1, 1]));
        let script = host.app.script(&ws).unwrap();
        assert!(script.contains("get(source, 'XData')"));
        assert!(script.contains("'omitnan'"));
        let mut fresh = matrust::Engine::default();
        fresh
            .execute(&script)
            .unwrap_or_else(|e| panic!("{e}\n{script}"));
        close(&struct_values(&mut fresh, "xstats"), &shown.x);
        close(&struct_values(&mut fresh, "ystats"), &shown.y);
        assert_eq!(host.app.results()[0], ("Number of points", "7".into()));
        let Some(toolkits::Action::SaveAppScript(kind, saved)) = host.generate(&ws, 0) else {
            panic!("save action")
        };
        assert_eq!(kind, AppKind::DataStatistics);
        assert_eq!(saved, apps::saved_script(kind, 0, &script));
    }

    #[test]
    fn data_statistics_graphics_delete_dispatch_is_atomic_and_keeps_other_figures() {
        let mut e = matrust::Engine::default();
        e.execute("figure(1); removed=line([1 2],[3 4]); kept=line([1 2],[5 6]); legend('show'); figure(2); other=line([1 2],[7 8]); f=gcf;").unwrap();
        let before: Vec<_> = e.figures().map(|(n, f)| (n, f.clone())).collect();
        // Invalid output requests and mixed targets must publish no removals.
        assert!(e.execute("bad = delete(removed);").is_err());
        assert!(e.execute("delete([removed f]);").is_err());
        for (n, f) in &before {
            assert!(f.same_snapshot(e.figures().find(|(m, _)| m == n).unwrap().1));
        }
        e.execute("delete(removed); gone=isgraphics(removed); alive=isgraphics(kept);")
            .unwrap();
        assert_eq!(e.logical_variables["gone"].data(), &[0]);
        assert_eq!(e.logical_variables["alive"].data(), &[1]);
        assert_eq!(e.current_figure, 2);
        assert!(
            before
                .iter()
                .find(|(n, _)| *n == 2)
                .unwrap()
                .1
                .same_snapshot(e.figures().find(|(n, _)| *n == 2).unwrap().1)
        );
        e.execute("delete([kept other]);").unwrap();
        assert!(e.figures().all(|(_, f)| f.series.is_empty()));
        // The string overload still deletes files.
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("delete-probe.txt");
        std::fs::write(&path, "temporary").unwrap();
        e.execute(&format!(
            "delete('{}');",
            path.to_string_lossy().replace('\'', "''")
        ))
        .unwrap();
        assert!(!path.exists());
    }

    #[test]
    fn data_statistics_tick_apply_and_untick_remove_only_owned_lines() {
        let (mut e, mut host, ws) = host();
        e.execute("userline = line([1 10],[8 8], 'DisplayName', 'y mean');")
            .unwrap();
        let user = source_figure(&e).series[1].identity.clone();
        let original = source_figure(&e).series[0].identity.clone();
        host.app.set_show(1, 2, true); // y mean
        host.app.set_show(0, 5, true); // x mean +/- std
        compute(&mut host, &ws).unwrap();
        let preview = host.app.current.as_ref().unwrap();
        assert_eq!(preview.overlays.len(), 3);
        let script = host.app.apply_script(&ws).unwrap();
        e.execute(&script)
            .unwrap_or_else(|e| panic!("{e}\n{script}"));
        let f = source_figure(&e);
        assert_eq!(f.series.len(), 5);
        assert!(f.series[0].identity.same_object(&original));
        assert!(f.series[1].identity.same_object(&user));
        assert_eq!(f.series[2].label, "x std -");
        close(&f.series[2].points.get(0).unwrap()[..1], &[X[2] - X[5]]);
        assert_eq!(f.series[4].label, "y mean");
        assert_eq!(f.series[4].points.get(0).unwrap()[1], 8.);
        host.app.set_show(1, 2, false);
        host.app.set_show(0, 5, false);
        e.execute(&host.app.apply_script(&ws).unwrap()).unwrap();
        let f = source_figure(&e);
        assert_eq!(f.series.len(), 2);
        assert!(f.series[0].identity.same_object(&original));
        assert!(f.series[1].identity.same_object(&user));
        assert!(e.array("xstats").is_none(), "Apply does not export structs");
        // Every row and both axes; repeated Apply replaces only owned handles.
        host.app.show = [[true; 7]; 2];
        compute(&mut host, &ws).unwrap();
        assert_eq!(host.app.current.as_ref().unwrap().overlays.len(), 16);
        let all = host.app.apply_script(&ws).unwrap();
        e.execute(&all).unwrap();
        e.execute(&all).unwrap();
        let f = source_figure(&e);
        assert_eq!(f.series.len(), 18);
        for (axis, expected) in [X, Y].into_iter().enumerate() {
            let levels = [
                expected[0],
                expected[1],
                expected[2],
                expected[3],
                expected[4],
                expected[2] - expected[5],
                expected[2] + expected[5],
                expected[6],
            ];
            for (line, value) in f.series[2 + axis * 8..2 + (axis + 1) * 8]
                .iter()
                .zip(levels)
            {
                close(
                    &[
                        line.points.get(0).unwrap()[axis],
                        line.points.get(1).unwrap()[axis],
                    ],
                    &[value, value],
                );
            }
        }
        assert!(f.series[1].identity.same_object(&user));
        e.execute("owned_tag = matrust_data_statistics.tag;")
            .unwrap();
        assert!(matches!(
            e.array("owned_tag"),
            Some(matrust::Array::Character(_))
        ));
    }

    #[test]
    fn data_statistics_workspace_inputs_names_and_no_clobber() {
        let mut e = matrust::Engine::default();
        e.execute("a = [1;2;2;NaN;5;8;10]; b = [3 NaN 3 7 9 11 15]; xstats = 91; ystats = 92;")
            .unwrap();
        let ws = snapshot(&e);
        let mut host = Host::new(DataStatistics::default());
        host.app.workspace = true;
        host.app.x_variable = "a".into();
        host.app.y_variable = "b".into();
        assert_eq!(host.app.names(&ws).unwrap(), ["xstats1", "ystats1"]);
        compute(&mut host, &ws).unwrap();
        close(&host.app.current.as_ref().unwrap().x, &X);
        let Some(toolkits::Action::ExportScript(export)) = host.export(&ws) else {
            panic!("export action")
        };
        e.execute(&export).unwrap();
        close(&struct_values(&mut e, "xstats1"), &X);
        close(&struct_values(&mut e, "ystats1"), &Y);
        assert_eq!(e.variables["xstats"].data(), &[91.]);
        assert_eq!(e.variables["ystats"].data(), &[92.]);
        // A stale export cannot replace a variable created since the snapshot.
        let mut stale = matrust::Engine::default();
        stale.execute("a=[1 2]; b=[3 4]; xstats1=123;").unwrap();
        assert!(stale.execute(&export).is_err());
        assert_eq!(stale.variables["xstats1"].data(), &[123.]);
        host.app.x_edited = true;
        host.app.x_name = "a".into();
        assert!(host.app.script(&ws).unwrap_err().contains("already exists"));
        host.app.x_name = "end".into();
        assert!(host.app.script(&ws).is_err());
        host.app.x_name = "same".into();
        host.app.y_edited = true;
        host.app.y_name = "same".into();
        assert!(host.app.script(&ws).is_err());
        host.app.x_name = "horizontal".into();
        host.app.y_name = "vertical".into();
        compute(&mut host, &ws).unwrap();
        let script = host.app.script(&ws).unwrap();
        let mut fresh = matrust::Engine::default();
        fresh
            .execute("a=[1;2;2;NaN;5;8;10]; b=[3 NaN 3 7 9 11 15];")
            .unwrap();
        fresh.execute(&script).unwrap();
        close(&struct_values(&mut fresh, "horizontal"), &X);
    }

    #[test]
    fn data_statistics_figure_refresh_duplicate_labels_and_empty_error_keep_last_good() {
        let (mut e, mut host, ws) = host();
        compute(&mut host, &ws).unwrap();
        let before = host.request(&ws).unwrap().key;
        e.execute("set(h, 'YData', [4 4 4 4 4 4 4]);").unwrap();
        host.app.sync_figures(&figures(&e));
        assert_ne!(before, host.request(&ws).unwrap().key);
        compute(&mut host, &ws).unwrap();
        assert_eq!(host.app.current.as_ref().unwrap().y[2], 4.);
        // Selecting a second axes with the same DisplayName must not read the first.
        e.execute("f2=figure(8); a1=axes('Parent',f2,'Position',[.05 .1 .4 .8]); plot(a1,[1 2],[4 4],'DisplayName','Measured'); a2=axes('Parent',f2,'Position',[.55 .1 .4 .8]); h2=plot(a2,[1 2],[12 14],'DisplayName','Measured');").unwrap();
        host.app.sync_figures(&figures(&e));
        host.app.selected = host
            .app
            .sources
            .iter()
            .position(|s| s.points.get(0).unwrap()[1] == 12.)
            .unwrap();
        compute(&mut host, &ws).unwrap();
        assert_eq!(host.app.current.as_ref().unwrap().y[2], 13.);
        e.execute("set(h2, 'XData', [], 'YData', []);").unwrap();
        host.app.sync_figures(&figures(&e));
        let empty_error = compute(&mut host, &ws).unwrap_err();
        assert!(empty_error.contains("nonempty"), "{empty_error}");
        assert_eq!(host.app.current.as_ref().unwrap().y[2], 13.);
        // The Host reports errors on the status line and preserves the chart.
        let ctx = egui::Context::default();
        host.refresh(&ctx, &ws);
        let deadline = Instant::now() + Duration::from_secs(20);
        while host.running() {
            host.refresh(&ctx, &ws);
            assert!(Instant::now() < deadline);
            std::thread::sleep(Duration::from_millis(2));
        }
        assert_eq!(host.status().1, toolkits::AppStatus::Error);
        assert_eq!(host.app.current.as_ref().unwrap().y[2], 13.);
    }

    #[test]
    fn data_statistics_bounds_and_invalid_workspace_vectors() {
        let mut e = matrust::Engine::default();
        e.execute("a=[1 2]; b=[1 2 3]; bad=[1 2;3 4]; allnan=[NaN NaN];")
            .unwrap();
        let ws = snapshot(&e);
        let mut host = Host::new(DataStatistics::default());
        host.app.workspace = true;
        host.app.x_variable = "a".into();
        host.app.y_variable = "b".into();
        assert!(
            compute(&mut host, &ws)
                .unwrap_err()
                .contains("equal lengths")
        );
        host.app.y_variable = "bad".into();
        assert!(host.request(&ws).is_err());
        host.app.y_variable = "allnan".into();
        assert!(compute(&mut host, &ws).unwrap_err().contains("non-NaN"));
        e.execute(&format!("figure(1); plot(1:{});", MAX_FIGURE_POINTS + 1))
            .unwrap();
        host.app.workspace = false;
        host.app.sync_figures(&figures(&e));
        assert!(host.request(&ws).err().unwrap().contains("at most"));
    }

    #[test]
    fn data_statistics_tab_renders_three_areas_and_restores_session_tab() {
        let size = egui::vec2(1440., 1000.);
        let ctx = egui::Context::default();
        let mut app = Workspace::with_context(ctx.clone(), true);
        app.run(SETUP.into());
        finish(&mut app);
        app.toolkits.apps.statistics.app.set_show(1, 2, true);
        app.toolkits.open_tool(Tool::App(AppKind::DataStatistics));
        let deadline = Instant::now() + Duration::from_secs(30);
        loop {
            let output = frame_at(&mut app, &ctx, size, vec![]);
            let host = &app.toolkits.apps.statistics;
            if host.app.current.is_some() && !host.running() {
                drop(output);
                break;
            }
            assert!(Instant::now() < deadline, "{:?}", host.status());
            std::thread::sleep(Duration::from_millis(5));
        }
        let output = frame_at(&mut app, &ctx, size, vec![]);
        for label in [
            "INPUTS",
            "STATISTICS",
            "OUTPUT",
            "Apply to figure",
            "Export to workspace",
            "Generate script",
            "Number of points",
            "NaNs ignored (X)",
            "X statistics struct",
            "show on plot",
        ] {
            assert!(
                output
                    .shapes
                    .iter()
                    .any(|s| text_position(&s.shape, label).is_some()),
                "missing {label}"
            );
        }
        assert_eq!(
            app.toolkits.selected_tool(),
            Some(Tool::App(AppKind::DataStatistics))
        );
        let tab = AppKind::DataStatistics.tab();
        assert_eq!(apps::AppKind::from_tab(tab), Some(AppKind::DataStatistics));
        let json = serde_json::to_string(&tab).unwrap();
        assert_eq!(
            serde_json::from_str::<matrust::session::ToolTab>(&json).unwrap(),
            tab
        );
        // The actual button uses the same apply builder and the workspace worker.
        click_label_at(&mut app, &ctx, size, "Apply to figure");
        let deadline = Instant::now() + Duration::from_secs(30);
        loop {
            frame_at(&mut app, &ctx, size, vec![]);
            if !app.busy && app.all_figures.get(&7).is_some_and(|f| f.series.len() == 2) {
                break;
            }
            assert!(Instant::now() < deadline, "Apply: {}", app.status);
            std::thread::sleep(Duration::from_millis(5));
        }
        // Editing the source refreshes both the table and an applied overlay.
        app.run("set(h, 'YData', [4 4 4 4 4 4 4]);".into());
        finish(&mut app);
        loop {
            frame_at(&mut app, &ctx, size, vec![]);
            if !app.busy
                && app.all_figures.get(&7).is_some_and(|f| {
                    f.series.len() == 2 && f.series[1].points.get(0).unwrap()[1] == 4.
                })
            {
                break;
            }
            assert!(
                Instant::now() < deadline,
                "Refresh: {}; host {:?}; shown {:?}; message {}",
                app.status,
                app.toolkits.apps.statistics.status(),
                app.toolkits
                    .apps
                    .statistics
                    .app
                    .current
                    .as_ref()
                    .map(|r| (r.y[2], r.overlays.len())),
                app.toolkits.apps.statistics.message
            );
            std::thread::sleep(Duration::from_millis(5));
        }
        assert_eq!(
            app.toolkits.apps.statistics.app.current.as_ref().unwrap().y[2],
            4.
        );
        app.toolkits.apps.statistics.app.set_show(1, 2, false);
        loop {
            frame_at(&mut app, &ctx, size, vec![]);
            if !app.busy && app.all_figures.get(&7).is_some_and(|f| f.series.len() == 1) {
                break;
            }
            assert!(
                Instant::now() < deadline,
                "Untick: {}; host {:?}; shown {:?}; flags {:?}; message {}",
                app.status,
                app.toolkits.apps.statistics.status(),
                app.toolkits
                    .apps
                    .statistics
                    .app
                    .current
                    .as_ref()
                    .map(|r| r.overlays.len()),
                app.toolkits.apps.statistics.app.show,
                app.toolkits.apps.statistics.message
            );
            std::thread::sleep(Duration::from_millis(5));
        }
        app.close_tool_tab(Tool::App(AppKind::DataStatistics));
    }

    #[test]
    fn data_statistics_free_text_inputs_cannot_escape_generated_script() {
        let (_, mut host, ws) = host();
        host.app.workspace = true;
        for payload in ["x); clear victim; disp(1", "x; injected=123", "x % comment"] {
            host.app.x_variable = payload.into();
            host.app.y_variable = payload.into();
            assert!(host.app.script(&ws).is_err(), "accepted data {payload}");
            host.app.workspace = false;
            host.app.x_edited = true;
            host.app.x_name = payload.into();
            assert!(host.app.script(&ws).is_err(), "accepted name {payload}");
            host.app.workspace = true;
        }
    }
}
