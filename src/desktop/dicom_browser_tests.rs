mod dicom_browser_tests {
    use super::*;
    use apps::{
        App, AppKind, Host, Snapshot,
        dicom_browser::{self, DicomBrowser},
    };
    use matrust::{Array, Engine};
    use std::{
        path::{Path, PathBuf},
        sync::Arc,
        time::{Duration, Instant},
    };

    fn fixtures() -> PathBuf {
        Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/dicom/collection")
    }
    fn host() -> Host<DicomBrowser> {
        let mut host = Host::new(DicomBrowser::default());
        host.app.draft.folder = fixtures().to_string_lossy().into_owned();
        host
    }
    fn frame(
        host: &mut Host<DicomBrowser>,
        ctx: &egui::Context,
        size: [f32; 2],
        events: Vec<egui::Event>,
    ) -> (egui::FullOutput, Option<toolkits::Action>) {
        let mut action = None;
        let output = crate::headless::run(
            ctx,
            egui::RawInput {
                screen_rect: Some(egui::Rect::from_min_size(
                    egui::Pos2::ZERO,
                    egui::vec2(size[0], size[1]),
                )),
                events,
                ..Default::default()
            },
            |ui| {
                dicom_browser::show(host, ui, &Snapshot::default(), false, &mut action);
            },
        );
        (output, action)
    }
    fn idle(host: &mut Host<DicomBrowser>, ctx: &egui::Context) -> egui::FullOutput {
        frame(host, ctx, [1600., 1000.], vec![]).0
    }
    fn wait(host: &mut Host<DicomBrowser>, ctx: &egui::Context) {
        let deadline = Instant::now() + Duration::from_secs(30);
        loop {
            idle(host, ctx);
            if !host.running() {
                break;
            }
            assert!(Instant::now() < deadline, "{}", host.status().0);
            std::thread::sleep(Duration::from_millis(2));
        }
        idle(host, ctx);
    }
    fn scan(host: &mut Host<DicomBrowser>, ctx: &egui::Context) {
        host.app.scan();
        wait(host, ctx);
        assert_eq!(
            host.status().1,
            toolkits::AppStatus::Success,
            "{}",
            host.status().0
        );
    }
    fn select(host: &mut Host<DicomBrowser>, ctx: &egui::Context, row: usize) {
        let uid = host.app.collection.as_ref().unwrap().rows[row].uid.clone();
        host.app.select(uid);
        wait(host, ctx);
        assert_eq!(
            host.status().1,
            toolkits::AppStatus::Success,
            "{}",
            host.status().0
        );
    }
    fn pointer(pos: egui::Pos2, pressed: bool) -> egui::Event {
        egui::Event::PointerButton {
            pos,
            button: egui::PointerButton::Primary,
            pressed,
            modifiers: egui::Modifiers::NONE,
        }
    }
    fn click(
        host: &mut Host<DicomBrowser>,
        ctx: &egui::Context,
        rect: egui::Rect,
    ) -> Option<toolkits::Action> {
        let pos = rect.center();
        frame(
            host,
            ctx,
            [1600., 1000.],
            vec![egui::Event::PointerMoved(pos), pointer(pos, true)],
        );
        frame(host, ctx, [1600., 1000.], vec![pointer(pos, false)]).1
    }
    fn key_down(key: egui::Key) -> egui::Event {
        egui::Event::Key {
            key,
            physical_key: None,
            pressed: true,
            repeat: false,
            modifiers: egui::Modifiers::NONE,
        }
    }
    /// `three_areas` narrow stack: root, CentralPanel child, its frame, then the scroll salt.
    fn stacked_scroll_id(ctx: &egui::Context) -> egui::Id {
        let root = egui::Id::new((ctx.viewport_id(), "__top_ui"));
        let central = root.with(egui::IdSalt::new("child"));
        let framed = central.with(egui::IdSalt::new("child"));
        let salt = egui::Id::new((
            "app_area",
            AppKind::DicomBrowser.label(),
            "compact_scroll".to_owned(),
        ));
        framed.with(egui::IdSalt::new(salt))
    }
    /// Widget rect, including controls egui does not paint while they are clipped.
    fn accesskit_rect(output: &egui::FullOutput, label: &str) -> egui::Rect {
        let update = output
            .platform_output
            .accesskit_update
            .as_ref()
            .unwrap_or_else(|| panic!("no accesskit tree"));
        let bounds = update
            .nodes
            .iter()
            .find_map(|(_, node)| {
                (node.label() == Some(label))
                    .then(|| node.bounds())
                    .flatten()
            })
            .unwrap_or_else(|| panic!("no accesskit node {label}"));
        egui::Rect::from_min_max(
            egui::pos2(bounds.x0 as f32, bounds.y0 as f32),
            egui::pos2(bounds.x1 as f32, bounds.y1 as f32),
        )
    }
    fn text_rect(output: &egui::FullOutput, label: &str) -> egui::Rect {
        fn find(shape: &egui::Shape, label: &str) -> Option<egui::Rect> {
            match shape {
                egui::Shape::Text(t) if t.galley.text() == label => {
                    Some(egui::Rect::from_min_size(t.pos, t.galley.size()))
                }
                egui::Shape::Vec(shapes) => shapes.iter().find_map(|s| find(s, label)),
                _ => None,
            }
        }
        output
            .shapes
            .iter()
            .find_map(|s| find(&s.shape, label))
            .unwrap_or_else(|| panic!("Missing {label}"))
    }
    fn scalar(engine: &Engine, name: &str) -> f64 {
        match engine.array(name).unwrap() {
            Array::Logical(a) => f64::from(a.data()[0]),
            Array::Numeric(a) => a.data()[0],
            _ => panic!("not scalar"),
        }
    }
    fn assert_equal(left: &Engine, right: &Engine, names: &[String]) {
        let mut check = Engine::default();
        for name in names {
            check
                .insert_array("actual", left.array(name).unwrap())
                .unwrap();
            check
                .insert_array("expected", right.array(name).unwrap())
                .unwrap();
            check.execute("same = isequal(actual, expected);").unwrap();
            assert_eq!(scalar(&check, "same"), 1., "{name}");
        }
    }

    #[test]
    fn dicom_browser_pointer_scan_select_frame_export_generate_script() {
        let dir = tempfile::tempdir().unwrap();
        copy_tree(&fixtures(), dir.path());
        // Preserve the pydicom fixture metadata and give each copied file a
        // distinct, exactly known uint16 ramp for frame and window assertions.
        let mut writer = Engine::default();
        for (file, first) in [
            ("study1_series1_b.dcm", 10),
            ("study1_series1_a.dcm", 100),
            ("study1_series1_c.dcm", 1000),
        ] {
            let path = dir.path().join(file).to_string_lossy().replace('\'', "''");
            writer.execute(&format!("m=dicominfo('{path}'); dicomwrite(uint16(reshape({first}:{},4,4)), '{path}', m, 'CreateMode', 'copy');", first+15)).unwrap();
        }
        let ctx = egui::Context::default();
        let mut host = host();
        host.app.draft.folder = dir.path().to_string_lossy().into_owned();
        idle(&mut host, &ctx);
        let rect = host.app.controls["Scan"];
        assert!(click(&mut host, &ctx, rect).is_none());
        wait(&mut host, &ctx);
        assert_eq!(host.app.collection.as_ref().unwrap().rows.len(), 3);
        let uid = host.app.collection.as_ref().unwrap().rows[0].uid.clone();
        let row = host.app.controls[&format!("row{uid}_0")];
        click(&mut host, &ctx, row);
        wait(&mut host, &ctx);
        assert_eq!(
            host.status().1,
            toolkits::AppStatus::Success,
            "{}",
            host.status().0
        );
        assert_eq!(host.app.current.as_ref().unwrap().frames, 3);
        assert!(host.app.current.as_ref().unwrap().mono16);
        assert_eq!(host.app.window, 15.);
        assert_eq!(host.app.level, 17.5);
        let first = host.app.axes.as_ref().unwrap().extras.images[0]
            .data
            .clone();
        let slider = host.app.controls["Frame"];
        // The slider's track occupies the left part of the response, before
        // the editable value and caption. Drag across it with real events.
        let from = slider.left_center() + egui::vec2(8., 0.);
        let to = from + egui::vec2(90., 0.);
        frame(
            &mut host,
            &ctx,
            [1600., 1000.],
            vec![egui::Event::PointerMoved(from), pointer(from, true)],
        );
        frame(
            &mut host,
            &ctx,
            [1600., 1000.],
            vec![egui::Event::PointerMoved(to)],
        );
        frame(&mut host, &ctx, [1600., 1000.], vec![pointer(to, false)]);
        wait(&mut host, &ctx);
        assert!(host.app.shown_frame > 0);
        assert_ne!(
            first.as_ref(),
            host.app.axes.as_ref().unwrap().extras.images[0]
                .data
                .as_ref()
        );
        // isequal treats NaT as unequal even to itself. Compare complete
        // outputs on the dated nonrecursive collection after the three-series UI.
        host.app.draft.recursive = false;
        let rect = host.app.controls["Scan"];
        click(&mut host, &ctx, rect);
        wait(&mut host, &ctx);
        assert_eq!(host.app.collection.as_ref().unwrap().rows.len(), 1);
        let uid = host.app.collection.as_ref().unwrap().rows[0].uid.clone();
        let rect = host.app.controls[&format!("row{uid}_0")];
        click(&mut host, &ctx, rect);
        wait(&mut host, &ctx);
        let output = idle(&mut host, &ctx);
        let action = click(&mut host, &ctx, text_rect(&output, "Export to workspace"));
        let Some(toolkits::Action::ExportScript(export)) = action else {
            panic!("export did not submit")
        };
        let mut exported = Engine::default();
        exported.execute(&export).unwrap();
        let output = idle(&mut host, &ctx);
        let action = click(&mut host, &ctx, text_rect(&output, "Generate script"));
        let Some(toolkits::Action::SaveAppScript(kind, saved)) = action else {
            panic!("script did not submit")
        };
        assert_eq!(kind, AppKind::DicomBrowser);
        assert!(saved.contains(&host.app.script(&Snapshot::default()).unwrap()));
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("series.m");
        let mut desktop = Workspace::with_context(ctx.clone(), true);
        desktop.toolkits.apps.test_save_path = Some(path.clone());
        desktop.queue_app_script_save(kind, saved.clone());
        desktop.finish_app_script_save(&ctx);
        let deadline = Instant::now() + Duration::from_secs(20);
        while desktop.busy {
            desktop.receive();
            assert!(Instant::now() < deadline);
            std::thread::sleep(Duration::from_millis(2));
        }
        assert_eq!(std::fs::read_to_string(&path).unwrap(), saved);
        let mut fresh = Engine::default();
        fresh
            .execute(&std::fs::read_to_string(&path).unwrap())
            .unwrap();
        assert_equal(&exported, &fresh, &host.app.names);
        assert_eq!(
            exported.array(&host.app.names[0]).unwrap().dims().as_ref(),
            &[4, 4, 1, 3]
        );
        let preview = host.app.current.as_ref().unwrap();
        exported
            .insert_array("privateVolume", preview.volume.clone())
            .unwrap();
        exported
            .execute("same = isequal(dicomVolume, privateVolume);")
            .unwrap();
        assert_eq!(scalar(&exported, "same"), 1.);
    }

    #[test]
    fn dicom_browser_all_series_layout_metadata_rgb_and_display_only_window() {
        let ctx = egui::Context::default();
        let mut host = host();
        scan(&mut host, &ctx);
        let oracle: serde_json::Value = serde_json::from_str(include_str!(
            "../../tests/fixtures/dicom/collection/expected.json"
        ))
        .unwrap();
        for row in 0..3 {
            select(&mut host, &ctx, row);
            let preview = host.app.current.as_ref().unwrap();
            let expected = &oracle["rows"][row];
            assert_eq!(
                preview.frames,
                expected["Frames"].as_u64().unwrap() as usize
            );
            assert_eq!(
                preview.volume.dims()[0],
                expected["Rows"].as_u64().unwrap() as usize
            );
            assert_eq!(
                preview.volume.dims()[1],
                expected["Columns"].as_u64().unwrap() as usize
            );
            assert_eq!(
                preview.info.len(),
                expected["Filenames"].as_array().unwrap().len()
            );
            assert!(
                preview.metadata[0]
                    .iter()
                    .any(|(n, _)| n == "TransferSyntaxUID")
            );
            let file_size = preview.metadata[0]
                .iter()
                .find(|(n, _)| n == "FileSize")
                .map(|(_, v)| v.as_str())
                .unwrap();
            assert!(
                !file_size.is_empty() && file_size.bytes().all(|b| b.is_ascii_digit()),
                "FileSize should print as whole integer without decimals, got {file_size:?}"
            );
            let script = host.app.script(&Snapshot::default()).unwrap();
            let mut fresh = Engine::default();
            fresh.execute(&script).unwrap();
            fresh
                .insert_array("privateVolume", preview.volume.clone())
                .unwrap();
            fresh
                .execute("same = isequal(privateVolume, dicomVolume);")
                .unwrap();
            assert_eq!(scalar(&fresh, "same"), 1.);
            if row == 2 {
                assert!(matches!(
                    host.app.axes.as_ref().unwrap().extras.images[0].mapping,
                    matrust::plot::ImageMapping::TrueColor { full_scale: 255. }
                ));
            } else {
                assert!(!preview.mono16);
                host.app.window = 11.;
                host.app.level = -4.;
                idle(&mut host, &ctx);
                assert_eq!(script, host.app.script(&Snapshot::default()).unwrap());
                assert_eq!(
                    host.app.axes.as_ref().unwrap().extras.clim,
                    Some([-9.5, 1.5])
                );
            }
        }
    }

    fn copy_tree(from: &Path, to: &Path) {
        std::fs::create_dir_all(to).unwrap();
        for entry in std::fs::read_dir(from).unwrap() {
            let entry = entry.unwrap();
            let target = to.join(entry.file_name());
            if entry.file_type().unwrap().is_dir() {
                copy_tree(&entry.path(), &target);
            } else {
                std::fs::copy(entry.path(), target).unwrap();
            }
        }
    }
    fn unsupported(path: &Path) {
        let mut bytes = std::fs::read(path).unwrap();
        let uid = b"1.2.840.10008.1.2.1";
        let at = bytes.windows(uid.len()).position(|s| s == uid).unwrap();
        // RLE is indexed by dicomCollection; native PixelData is invalid for
        // that transport and must fail at dicomread, after row selection.
        bytes[at + uid.len() - 1] = b'5';
        std::fs::write(path, bytes).unwrap();
    }

    #[test]
    fn dicom_browser_optional_metadata_fields_are_unified_per_file() {
        let dir = tempfile::tempdir().unwrap();
        copy_tree(&fixtures(), dir.path());
        let path = dir
            .path()
            .join("study1_series1_a.dcm")
            .to_string_lossy()
            .replace('\'', "''");
        Engine::default().execute(&format!(
            "m=dicominfo('{path}'); m=rmfield(m,'PatientSex'); m.PatientBirthDate='19990102'; dicomwrite(uint8(zeros(4)), '{path}', m, 'CreateMode','copy');"
        )).unwrap();
        let ctx = egui::Context::default();
        let mut host = host();
        host.app.draft.folder = dir.path().to_string_lossy().into_owned();
        scan(&mut host, &ctx);
        select(&mut host, &ctx, 0);
        let info = &host.app.current.as_ref().unwrap().info;
        assert_eq!(info.len(), 3);
        let sex = info.field_index("PatientSex").unwrap();
        let birth = info.field_index("PatientBirthDate").unwrap();
        assert!(!info.get(sex, 0).unwrap().is_empty());
        assert!(info.get(sex, 1).unwrap().is_empty());
        assert!(info.get(birth, 0).unwrap().is_empty());
        assert!(!info.get(birth, 1).unwrap().is_empty());
        let script = host.app.script(&Snapshot::default()).unwrap();
        let mut engine = Engine::default();
        engine.execute(&script).unwrap();
        engine
            .insert_array("privateInfo", Array::Struct(info.clone()))
            .unwrap();
        engine
            .execute("same=isequal(privateInfo,dicomInfo);")
            .unwrap();
        assert_eq!(scalar(&engine, "same"), 1.);
    }
    #[test]
    fn dicom_browser_unsupported_read_is_inline_keeps_preview_and_table_usable() {
        let dir = tempfile::tempdir().unwrap();
        copy_tree(&fixtures(), dir.path());
        unsupported(&dir.path().join("sub/study1_series2_multi.dcm"));
        let ctx = egui::Context::default();
        let mut host = host();
        host.app.draft.folder = dir.path().to_string_lossy().into_owned();
        scan(&mut host, &ctx);
        select(&mut host, &ctx, 0);
        let previous = host.app.current.clone().unwrap();
        let table = host.app.collection.clone().unwrap();
        host.app.select(table.rows[1].uid.clone());
        wait(&mut host, &ctx);
        let error = host.app.inline_error.as_ref().unwrap();
        assert!(
            error.contains("dicomread:") && error.contains("RLE"),
            "{error}"
        );
        assert!(Arc::ptr_eq(&previous, host.app.current.as_ref().unwrap()));
        assert!(Arc::ptr_eq(&table, host.app.collection.as_ref().unwrap()));
        assert!(!idle(&mut host, &ctx).shapes.is_empty());
        select(&mut host, &ctx, 2);
        assert!(host.app.inline_error.is_none());
    }

    #[test]
    fn dicom_browser_names_and_failed_export_roll_back_existing_and_absent_outputs() {
        let dir = tempfile::tempdir().unwrap();
        copy_tree(&fixtures(), dir.path());
        let ctx = egui::Context::default();
        let mut host = host();
        host.app.draft.folder = dir.path().to_string_lossy().into_owned();
        scan(&mut host, &ctx);
        select(&mut host, &ctx, 0);
        let good = host.app.names.clone();
        for name in [
            "end",
            "a;error('bad')",
            "mrexp_old",
            "matrust_export_error",
            "dicomread",
            "matrust_dicom_series",
            "matrust_dicom_scan",
            "matrust_dicom_sample",
            "matrust_dicom_clear",
            "1bad",
            "a b",
        ] {
            host.app.names[0] = name.into();
            assert!(host.app.script(&Snapshot::default()).is_err(), "{name}");
            assert!(host.export(&Snapshot::default()).is_none());
            assert!(host.generate(&Snapshot::default(), 0).is_none());
        }
        host.app.names = good.clone();
        host.app.names[1] = good[0].clone();
        assert!(host.app.script(&Snapshot::default()).is_err());
        host.app.names = good;
        let Some(toolkits::Action::ExportScript(script)) = host.export(&Snapshot::default()) else {
            panic!()
        };
        // Fail after all assignments too, to verify restoration of values that
        // a successful series read has already replaced in the workspace.
        let generated = host.app.script(&Snapshot::default()).unwrap();
        let (body, functions) = apps::split_local_functions(&generated);
        let late = apps::all_or_nothing(
            &format!("{body}error('late failure');\n{functions}"),
            &host.app.names,
        );
        for existing in [false, true] {
            let mut engine = Engine::default();
            if existing {
                engine
                    .execute("dicomVolume=19; dicomInfo='keep'; dicomSeries={42};")
                    .unwrap();
            }
            let before = engine.workspace();
            assert!(engine.execute(&late).unwrap_err().contains("late failure"));
            for name in &host.app.names {
                match before.get(name) {
                    Some(old) => assert!(engine.array(name).unwrap().same_snapshot(old)),
                    None => assert!(engine.array(name).is_none()),
                }
            }
        }
        unsupported(&dir.path().join("study1_series1_a.dcm"));
        for existing in [false, true] {
            let mut engine = Engine::default();
            if existing {
                engine
                    .execute("dicomVolume=19; dicomInfo='keep'; dicomSeries={42};")
                    .unwrap();
            }
            let before = engine.workspace();
            assert!(engine.execute(&script).is_err());
            for name in &host.app.names {
                match before.get(name) {
                    Some(old) => assert!(engine.array(name).unwrap().same_snapshot(old)),
                    None => assert!(engine.array(name).is_none()),
                }
            }
        }
    }

    #[test]
    fn dicom_browser_pointer_cancel_scan_preserves_previous_collection() {
        let ctx = egui::Context::default();
        let mut host = host();
        scan(&mut host, &ctx);
        let previous = host.app.collection.clone().unwrap();
        host.app.scan();
        host.app.prepare(&Snapshot::default());
        let request = host.request(&Snapshot::default()).unwrap();
        let release = dicom_browser::hold_scan_for_cancel_test(&mut host, &ctx, request);
        idle(&mut host, &ctx);
        assert!(host.running());
        let rect = host.app.controls["Cancel"];
        click(&mut host, &ctx, rect);
        release.send(()).unwrap();
        wait(&mut host, &ctx);
        assert!(Arc::ptr_eq(
            &previous,
            host.app.collection.as_ref().unwrap()
        ));
        assert!(host.status().0.contains("cancelled"));
        text_rect(
            &idle(&mut host, &ctx),
            "DICOM operation cancelled. Press Scan or select a series to continue.",
        );
        scan(&mut host, &ctx);
        assert_eq!(host.app.collection.as_ref().unwrap().rows.len(), 3);
    }

    #[test]
    fn dicom_browser_headless_empty_scanned_error_narrow_200_rows_and_sort_filter_cache() {
        let ctx = egui::Context::default();
        let mut host = host();
        assert!(!idle(&mut host, &ctx).shapes.is_empty());
        scan(&mut host, &ctx);
        select(&mut host, &ctx, 0);
        let id = egui::Id::new("dicom_preview").with(("matrust_image", 0usize));
        idle(&mut host, &ctx);
        let texture = ctx
            .data(|d| d.get_temp::<(u64, egui::TextureHandle)>(id))
            .unwrap();
        idle(&mut host, &ctx);
        assert_eq!(
            texture.1.id(),
            ctx.data(|d| d.get_temp::<(u64, egui::TextureHandle)>(id))
                .unwrap()
                .1
                .id()
        );
        host.app.filter = "TransferSyntaxUID".into();
        let output = idle(&mut host, &ctx);
        text_rect(&output, "TransferSyntaxUID");
        assert!(
            !frame(&mut host, &ctx, [640., 780.], vec![])
                .0
                .shapes
                .is_empty()
        );
        host.app.inline_error = Some("Unsupported transfer syntax test state".into());
        assert!(!idle(&mut host, &ctx).shapes.is_empty());
        let c = Arc::get_mut(host.app.collection.as_mut().unwrap()).unwrap();
        let sample = c.rows[0].clone();
        c.rows = (0..200)
            .map(|i| {
                let mut row = sample.clone();
                row.uid = format!("1.2.{i}");
                row.frames = 200 - i;
                row.cells[5] = row.frames.to_string();
                row
            })
            .collect();
        host.app.sort_by(5);
        idle(&mut host, &ctx);
        assert!(host.app.laid_out_rows < 20, "table must virtualize rows");
        assert!(host.app.laid_out_rows > 0);
        for column in 0..6 {
            host.app.sort_by(column);
            idle(&mut host, &ctx);
        }
    }

    #[test]
    fn dicom_browser_builtin_desktop_launch_folder_and_tab_roundtrip_cli_thread_scope() {
        for code in ["dicomBrowser", "dicomBrowser('somewhere')"] {
            assert!(
                Engine::default()
                    .execute(code)
                    .unwrap_err()
                    .contains("dicomBrowser requires the desktop")
            );
        }
        let captured = Arc::new(std::sync::Mutex::new(vec![]));
        let sink = captured.clone();
        {
            let _hook = matrust::engine::present_dicom_browser(move |p| {
                sink.lock().unwrap().push(p);
                Ok(())
            });
            let mut e = Engine::default();
            e.set_file_directory(&fixtures()).unwrap();
            e.execute("dicomBrowser; dicomBrowser('sub'); dicomBrowser(\"sub\");")
                .unwrap();
            assert!(e.execute("x=dicomBrowser;").is_err());
            assert!(e.execute("dicomBrowser(2);").is_err());
            assert!(
                std::thread::spawn(|| Engine::default().execute("dicomBrowser").unwrap_err())
                    .join()
                    .unwrap()
                    .contains("requires the desktop")
            );
        }
        assert!(Engine::default().execute("dicomBrowser").is_err());
        let got = captured.lock().unwrap();
        assert_eq!(got.len(), 3);
        assert_eq!(got[0], None);
        assert_eq!(
            got[1].as_ref().map(|p| p.canonicalize().unwrap()),
            Some(fixtures().join("sub").canonicalize().unwrap())
        );
        for code in [
            "dicomBrowser".to_owned(),
            format!("dicomBrowser('{}')", fixtures().to_string_lossy()),
        ] {
            let ctx = egui::Context::default();
            let mut workspace = Workspace::with_context(ctx, true);
            workspace.submit(worker::Job::ScriptInFolder(code.clone(), fixtures()));
            let deadline = Instant::now() + Duration::from_secs(20);
            while workspace.busy {
                workspace.receive();
                assert!(Instant::now() < deadline);
                std::thread::sleep(Duration::from_millis(2));
            }
            assert!(workspace.toolkits.apps.is_open(AppKind::DicomBrowser));
            if code != "dicomBrowser" {
                assert_eq!(
                    workspace.toolkits.apps.dicom.app.draft.folder,
                    fixtures().to_string_lossy()
                );
            }
            let layout = workspace.toolkits.tab_layout().unwrap();
            let mut restored = toolkits::Toolkits::default();
            restored.restore_tab_layout(Some(layout));
            assert!(restored.apps.is_open(AppKind::DicomBrowser));
        }
        assert_eq!(
            AppKind::from_tab(matrust::session::ToolTab::DicomBrowser),
            Some(AppKind::DicomBrowser)
        );
        assert_eq!(
            serde_json::to_string(&AppKind::DicomBrowser.tab()).unwrap(),
            "\"dicom_browser\""
        );
    }

    fn walk_shapes(shape: &egui::Shape, visit: &mut dyn FnMut(&egui::Shape)) {
        visit(shape);
        if let egui::Shape::Vec(shapes) = shape {
            for shape in shapes {
                walk_shapes(shape, visit);
            }
        }
    }
    fn galleys(output: &egui::FullOutput, label: &str) -> Vec<(egui::Rect, bool)> {
        let mut found = Vec::new();
        for clipped in &output.shapes {
            walk_shapes(&clipped.shape, &mut |shape| {
                if let egui::Shape::Text(text) = shape
                    && text.galley.text() == label
                {
                    found.push((
                        egui::Rect::from_min_size(text.pos, text.galley.size()),
                        text.galley.elided,
                    ));
                }
            });
        }
        found
    }
    fn assert_not_elided(output: &egui::FullOutput, label: &str) {
        let found = galleys(output, label);
        assert!(!found.is_empty(), "missing {label}");
        assert!(found.iter().all(|(_, elided)| !elided), "{label} elided");
    }
    fn stroked_rects(output: &egui::FullOutput) -> Vec<egui::Rect> {
        let mut rects = Vec::new();
        for clipped in &output.shapes {
            walk_shapes(&clipped.shape, &mut |shape| {
                if let egui::Shape::Rect(rect) = shape
                    && rect.stroke.width > 0.
                {
                    rects.push(rect.rect);
                }
            });
        }
        rects
    }
    fn smallest_stroked(output: &egui::FullOutput, point: egui::Pos2) -> Option<egui::Rect> {
        stroked_rects(output)
            .into_iter()
            .filter(|rect| rect.contains(point))
            .min_by(|a, b| a.area().total_cmp(&b.area()))
    }
    fn same_rect(left: egui::Rect, right: egui::Rect) -> bool {
        (left.min - right.min).length() < 1.5 && (left.max - right.max).length() < 1.5
    }
    /// The 2 px accent ring `panels::focus_ring` paints on a focused track.
    fn focus_ring_on(output: &egui::FullOutput, target: egui::Rect) -> bool {
        let mut found = false;
        for clipped in &output.shapes {
            walk_shapes(&clipped.shape, &mut |shape| {
                if let egui::Shape::Rect(rect) = shape
                    && rect.stroke.width >= 2.
                    && same_rect(rect.rect, target)
                {
                    found = true;
                }
            });
        }
        found
    }

    #[test]
    fn dicom_browser_layout_900_buttons_flat_header_dates_and_frame() {
        let ctx = egui::Context::default();
        // Themed inactive controls have a 1 px border. The flat header does not.
        crate::theme::configure(&ctx);
        ctx.enable_accesskit();
        let mut host = host();
        scan(&mut host, &ctx);
        select(&mut host, &ctx, 0);
        assert_eq!(host.app.current.as_ref().unwrap().frames, 3);
        let window = egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(900., 800.));
        let narrow = frame(&mut host, &ctx, [900., 800.], vec![]).0;
        for label in ["Export to workspace", "Generate script"] {
            // Clipped buttons are not painted; the accessibility rect is the control.
            let button = accesskit_rect(&narrow, label);
            assert!(
                button.left() >= 280. && button.right() <= 900.,
                "{label} {button:?} is not horizontally inside the window"
            );
        }
        let hover = text_rect(&narrow, "Study date").center();
        let scroll = stacked_scroll_id(&ctx);
        let mut best = egui::scroll_area::State::load(&ctx, scroll)
            .map(|state| state.offset.y)
            .unwrap_or(0.);
        let mut still = 0;
        let mut scrolled = narrow.clone();
        for step in 0..80 {
            scrolled = frame(
                &mut host,
                &ctx,
                [900., 800.],
                vec![
                    egui::Event::PointerMoved(hover),
                    egui::Event::MouseWheel {
                        unit: egui::MouseWheelUnit::Point,
                        delta: egui::vec2(0., -480.),
                        modifiers: egui::Modifiers::NONE,
                        phase: egui::TouchPhase::Move,
                    },
                ],
            )
            .0;
            let offset = egui::scroll_area::State::load(&ctx, scroll)
                .unwrap_or_else(|| panic!("stacked scroll missing {scroll:?}"))
                .offset
                .y;
            if offset > best + 0.25 {
                best = offset;
                still = 0;
            } else {
                still += 1;
            }
            // A wheel that no longer moves the offset means the stack is at its end.
            if still >= 5 && (best > 0. || step >= 15) {
                break;
            }
        }
        assert!(
            still >= 5,
            "stacked scroll did not reach its end (offset {best})"
        );
        for label in ["Export to workspace", "Generate script"] {
            let text = text_rect(&scrolled, label);
            let button = smallest_stroked(&scrolled, text.center())
                .unwrap_or_else(|| panic!("{label} has no button"));
            assert!(
                window.contains_rect(button),
                "{label} {button:?} outside {window:?} after scroll {best}"
            );
        }
        let header = text_rect(&narrow, "Study date");
        assert!(
            stroked_rects(&narrow).iter().all(|rect| {
                !(rect.contains(header.center()) && (16. ..=40.).contains(&rect.height()))
            }),
            "Study date header is a button"
        );
        assert_not_elided(&narrow, "Rows × cols");
        assert_not_elided(&narrow, "Frames");
        let frames = text_rect(&narrow, "Frames");
        assert!(
            frames.right() <= 900.,
            "Frames header at {}",
            frames.right()
        );
        let wide = idle(&mut host, &ctx);
        assert_not_elided(&wide, "2024-01-15 09:30:00");
        text_rect(&wide, "—");
        let name = text_rect(&wide, "Name");
        let filename = text_rect(&wide, "Filename");
        assert!(
            (name.left() - filename.left()).abs() < 1.,
            "Name {} Filename {}",
            name.left(),
            filename.left()
        );
        let value = text_rect(&wide, "Value");
        let dicom = text_rect(&wide, "'DICOM'");
        assert!(
            (value.left() - dicom.left()).abs() < 1.,
            "Value {} 'DICOM' {}",
            value.left(),
            dicom.left()
        );
        text_rect(&wide, "Frame");
        text_rect(&wide, "1 / 3");
        assert!(host.app.controls["Frame"].width() > 80.);
        let track = host.app.controls["Frame"];
        assert!(
            !focus_ring_on(&wide, track),
            "focus ring before the track has focus"
        );
        // The left edge maps to frame 0, so the click only gives the track focus.
        let pos = egui::pos2(track.left() + 2., track.center().y);
        frame(
            &mut host,
            &ctx,
            [1600., 1000.],
            vec![egui::Event::PointerMoved(pos), pointer(pos, true)],
        );
        frame(&mut host, &ctx, [1600., 1000.], vec![pointer(pos, false)]);
        assert_eq!(host.app.frame, 0);
        assert!(!host.app.frame_requested);
        let focused = idle(&mut host, &ctx);
        let track = host.app.controls["Frame"];
        assert!(
            focus_ring_on(&focused, track),
            "focused frame track has no focus ring {track:?}"
        );
        for (key, expected) in [
            (egui::Key::ArrowRight, 1usize),
            (egui::Key::ArrowLeft, 0),
            (egui::Key::End, 2),
            (egui::Key::Home, 0),
        ] {
            frame(&mut host, &ctx, [1600., 1000.], vec![key_down(key)]);
            assert_eq!(host.app.frame, expected, "{key:?}");
            assert!(host.app.frame_requested, "{key:?}");
            host.app.frame_requested = false;
        }
        select(&mut host, &ctx, 2);
        assert_eq!(host.app.current.as_ref().unwrap().frames, 1);
        let single = idle(&mut host, &ctx);
        assert!(
            galleys(&single, "Frame").is_empty(),
            "single frame shows Frame"
        );
        assert!(galleys(&single, "1 / 1").is_empty());
        let long = "Series description ".repeat(12);
        {
            let collection = Arc::get_mut(host.app.collection.as_mut().unwrap()).unwrap();
            collection.rows[0].cells[3].clone_from(&long);
        }
        let wide = idle(&mut host, &ctx);
        let described = galleys(&wide, &long);
        assert!(!described.is_empty());
        assert!(described.iter().all(|(_, elided)| *elided));
        assert_not_elided(&wide, "2024-01-15 09:30:00");
    }

    #[test]
    fn dicom_browser_gradient_preview_has_more_than_one_grey() {
        let folder = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/apps/dicom");
        let ctx = egui::Context::default();
        let mut host = Host::new(DicomBrowser::default());
        host.app.draft.folder = folder.to_string_lossy().into_owned();
        host.app.draft.recursive = false;
        scan(&mut host, &ctx);
        assert_eq!(host.app.collection.as_ref().unwrap().rows.len(), 1);
        select(&mut host, &ctx, 0);
        let pixels = host.app.axes.as_ref().unwrap().extras.images[0]
            .data
            .clone();
        let min = pixels.iter().copied().fold(f64::INFINITY, f64::min);
        let max = pixels.iter().copied().fold(f64::NEG_INFINITY, f64::max);
        assert!(min < max, "{min} {max}");
        assert_eq!(host.app.window, (max - min).max(1.));
        assert_eq!(host.app.level, (min + max) / 2.);
        let clim = host.app.axes.as_ref().unwrap().extras.clim.unwrap();
        assert_eq!(
            clim,
            [
                host.app.level - host.app.window / 2.,
                host.app.level + host.app.window / 2.
            ]
        );
        let id = egui::Id::new("dicom_preview").with(("matrust_image", 0usize));
        ctx.data_mut(|data| {
            data.remove::<(u64, egui::TextureHandle)>(id);
        });
        let mut input = egui::RawInput {
            screen_rect: Some(egui::Rect::from_min_size(
                egui::Pos2::ZERO,
                egui::vec2(1600., 1000.),
            )),
            ..Default::default()
        };
        crate::input::sanitize(&mut input);
        let output = ctx.run_ui(input, |ui| {
            dicom_browser::show(&mut host, ui, &Snapshot::default(), false, &mut None);
        });
        let texture = ctx
            .data(|data| data.get_temp::<(u64, egui::TextureHandle)>(id))
            .unwrap();
        let deltas = output
            .textures_delta
            .set
            .get(&texture.1.id())
            .expect("preview texture uploaded");
        let image = &deltas[0].image;
        assert_eq!(image.size(), [4, 4]);
        let egui::ImageData::Color(pixels) = image;
        let greys: Vec<u8> = pixels
            .pixels
            .iter()
            .map(|pixel| pixel.to_array()[0])
            .collect();
        let mut levels = greys.clone();
        levels.sort_unstable();
        levels.dedup();
        assert!(levels.len() > 1, "{greys:?}");
        assert!(
            greys.iter().any(|grey| (40..220).contains(grey)),
            "{greys:?}"
        );
        assert!(greys.iter().any(|grey| *grey <= 20), "{greys:?}");
        assert!(greys.iter().any(|grey| *grey >= 230), "{greys:?}");
    }

    fn frame_samples(volume: &Array, frame: usize) -> Vec<f64> {
        let dims = volume.dims();
        let rows = dims[0];
        let cols = dims[1];
        let channels = dims.get(2).copied().unwrap_or(1);
        let count = rows * cols * channels;
        let start = count * frame;
        match volume {
            Array::Typed(n) => (0..count).map(|i| n.real(start + i)).collect(),
            Array::Numeric(n) => n.data()[start..start + count].to_vec(),
            other => panic!("volume class {}", other.class()),
        }
    }
    fn info_scalar(info: &matrust::StructArray, name: &str) -> f64 {
        let field = info
            .field_index(name)
            .unwrap_or_else(|| panic!("no {name}"));
        match info.get(field, 0).unwrap().as_ref() {
            Array::Numeric(n) => n.data()[0],
            Array::Typed(n) => n.real(0),
            other => panic!("{name} class {}", other.class()),
        }
    }

    #[cfg(unix)]
    #[test]
    fn dicom_browser_skipped_count_uses_canonical_directory_walk() {
        use std::os::unix::fs::symlink;
        let root = tempfile::tempdir().unwrap();
        let sub = root.path().join("sub");
        std::fs::create_dir(&sub).unwrap();
        std::fs::copy(fixtures().join("study1_series1_a.dcm"), sub.join("one.dcm")).unwrap();
        symlink(&sub, root.path().join("alias")).unwrap();
        let ctx = egui::Context::default();
        let mut host = host();
        host.app.draft.folder = root.path().to_string_lossy().into_owned();
        host.app.draft.recursive = true;
        scan(&mut host, &ctx);
        let collection = host.app.collection.as_ref().unwrap();
        assert_eq!(collection.skipped, Ok(0));
        assert_eq!(collection.rows.len(), 1);
        assert_eq!(collection.rows[0].frames, 1);

        let outside = tempfile::tempdir().unwrap();
        std::fs::write(outside.path().join("bad.dcm"), b"invalid").unwrap();
        std::fs::copy(
            fixtures().join("study1_series1_a.dcm"),
            outside.path().join("two.dcm"),
        )
        .unwrap();
        symlink(outside.path(), root.path().join("outside_alias")).unwrap();
        scan(&mut host, &ctx);
        assert_eq!(host.app.collection.as_ref().unwrap().skipped, Ok(0));
        assert_eq!(host.app.collection.as_ref().unwrap().rows[0].frames, 1);

        symlink(root.path(), sub.join("cycle")).unwrap();
        scan(&mut host, &ctx);
        assert_eq!(host.app.collection.as_ref().unwrap().skipped, Ok(0));
        assert_eq!(host.app.collection.as_ref().unwrap().rows[0].frames, 1);
    }

    #[test]
    fn dicom_browser_skipped_count_includes_non_dicom_and_honors_recursion() {
        let root = tempfile::tempdir().unwrap();
        let sub = root.path().join("sub");
        std::fs::create_dir(&sub).unwrap();
        std::fs::copy(
            fixtures().join("study1_series1_a.dcm"),
            root.path().join("one.dcm"),
        )
        .unwrap();
        std::fs::write(root.path().join("notes.txt"), b"not DICOM").unwrap();
        std::fs::write(root.path().join("bad.dcm"), b"invalid").unwrap();
        std::fs::write(root.path().join("DICOMDIR"), b"ignored index").unwrap();
        std::fs::write(sub.join("notes.txt"), b"nested non-DICOM").unwrap();
        let ctx = egui::Context::default();
        let mut host = host();
        host.app.draft.folder = root.path().to_string_lossy().into_owned();
        host.app.draft.recursive = false;
        scan(&mut host, &ctx);
        assert_eq!(host.app.collection.as_ref().unwrap().skipped, Ok(2));
        host.app.draft.recursive = true;
        scan(&mut host, &ctx);
        assert_eq!(host.app.collection.as_ref().unwrap().skipped, Ok(3));
        text_rect(
            &idle(&mut host, &ctx),
            "3 file(s) in this folder could not be read as DICOM and were skipped",
        );
    }

    #[cfg(unix)]
    #[test]
    fn dicom_browser_skipped_count_includes_unreadable_files() {
        use std::os::unix::fs::PermissionsExt;
        let root = tempfile::tempdir().unwrap();
        let unreadable = root.path().join("unreadable.dcm");
        std::fs::write(&unreadable, b"invalid").unwrap();
        std::fs::set_permissions(&unreadable, std::fs::Permissions::from_mode(0o000)).unwrap();
        // Privileged runners can bypass permissions; invalid content still
        // contributes one skip there. Unprivileged runs exercise File::open failure.
        let ctx = egui::Context::default();
        let mut host = host();
        host.app.draft.folder = root.path().to_string_lossy().into_owned();
        scan(&mut host, &ctx);
        assert_eq!(host.app.collection.as_ref().unwrap().skipped, Ok(1));
    }

    #[cfg(unix)]
    #[test]
    fn dicom_browser_skipped_count_accepts_non_unicode_filenames() {
        use std::os::unix::ffi::OsStringExt;
        let root = tempfile::tempdir().unwrap();
        std::fs::write(
            root.path().join(std::ffi::OsString::from_vec(vec![0xff])),
            b"not DICOM",
        )
        .unwrap();
        let ctx = egui::Context::default();
        let mut host = host();
        host.app.draft.folder = root.path().to_string_lossy().into_owned();
        scan(&mut host, &ctx);
        assert_eq!(host.app.collection.as_ref().unwrap().skipped, Ok(1));
    }

    #[test]
    fn dicom_browser_unknown_skipped_count_label_is_visible_with_reason() {
        assert_eq!(dicom_browser::skipped_label(&Ok(0)), None);
        let unknown = Err("The collection scan did not report a count".into());
        let label = "Skipped-file count is unknown: The collection scan did not report a count";
        assert_eq!(
            dicom_browser::skipped_label(&unknown).as_deref(),
            Some(label)
        );
        let ctx = egui::Context::default();
        let mut host = host();
        scan(&mut host, &ctx);
        Arc::get_mut(host.app.collection.as_mut().unwrap())
            .unwrap()
            .skipped = unknown;
        text_rect(&idle(&mut host, &ctx), label);
    }

    #[test]
    fn dicom_browser_failed_scan_shows_its_reason() {
        let root = tempfile::tempdir().unwrap();
        let ctx = egui::Context::default();
        let mut host = host();
        host.app.draft.folder = root.path().join("missing").to_string_lossy().into_owned();
        host.app.scan();
        wait(&mut host, &ctx);
        let error = host.app.inline_error.clone().expect("visible scan error");
        assert!(
            !error.starts_with("Skipped-file count is unknown"),
            "{error}"
        );
        assert!(error.contains("dicomCollection:"), "{error}");
        text_rect(&idle(&mut host, &ctx), &error);
    }

    #[test]
    fn dicom_browser_use_sample_data_scans_three_series_and_reproduces_script() {
        let _ = std::fs::remove_dir_all(std::env::temp_dir().join("matrust-dicom-sample"));
        let ctx = egui::Context::default();
        let mut host = Host::new(DicomBrowser::default());
        let narrow = frame(&mut host, &ctx, [640., 780.], vec![]).0;
        assert_not_elided(&narrow, "Use sample data");
        let button = host.app.controls["Use sample data"];
        assert!(
            button.width() > 24. && button.height() > 8. && button.right() <= 280.5,
            "{button:?}"
        );
        let wide = idle(&mut host, &ctx);
        assert_not_elided(&wide, "Use sample data");
        let button = host.app.controls["Use sample data"];
        assert!(
            button.width() > 24. && button.right() <= 280.5,
            "{button:?}"
        );
        assert!(click(&mut host, &ctx, button).is_none());
        wait(&mut host, &ctx);
        assert_eq!(
            host.status().1,
            toolkits::AppStatus::Success,
            "{}",
            host.status().0
        );
        let (frames, cells, skipped, folder, sample) = {
            let collection = host.app.collection.as_ref().unwrap();
            (
                collection
                    .rows
                    .iter()
                    .map(|row| row.frames)
                    .collect::<Vec<_>>(),
                collection
                    .rows
                    .iter()
                    .map(|row| row.cells.clone())
                    .collect::<Vec<_>>(),
                collection.skipped.clone(),
                collection.spec.folder.clone(),
                collection.spec.sample,
            )
        };
        assert_eq!(frames, vec![20, 1, 1]);
        assert_eq!(skipped, Ok(1));
        assert!(sample);
        assert!(folder.contains("matrust-dicom-sample"));
        assert_eq!(host.app.draft.folder, folder);
        let expect = [
            ["Sample^Head", "CT", "Head phantom", "128 × 128", "20"],
            ["Sample^Head", "CT", "Scout", "128 × 128", "1"],
            ["Sample^Knee", "MR", "Knee MR", "128 × 128", "1"],
        ];
        for (row, expected) in cells.iter().zip(expect) {
            assert_ne!(row[0], "—", "{row:?}");
            assert!(!row[0].is_empty());
            assert_eq!(
                row[1..].iter().map(String::as_str).collect::<Vec<_>>(),
                expected
            );
        }
        let output = idle(&mut host, &ctx);
        text_rect(
            &output,
            "1 file(s) in this folder could not be read as DICOM and were skipped",
        );
        select(&mut host, &ctx, 0);
        let (first, last, span, intercept) = {
            let preview = host.app.current.as_ref().unwrap();
            assert_eq!(preview.frames, 20);
            assert!(preview.mono16);
            assert_eq!(preview.volume.dims().as_ref(), &[128, 128, 1, 20]);
            let first = frame_samples(&preview.volume, 0);
            let last = frame_samples(&preview.volume, 19);
            let (min, max) = first
                .iter()
                .fold((f64::INFINITY, f64::NEG_INFINITY), |(lo, hi), v| {
                    (lo.min(*v), hi.max(*v))
                });
            (
                first,
                last,
                max - min,
                info_scalar(&preview.info, "RescaleIntercept"),
            )
        };
        assert_ne!(first, last);
        assert!(span >= 500., "{span}");
        assert_eq!(intercept, -1024.);
        let output = idle(&mut host, &ctx);
        let action = click(&mut host, &ctx, text_rect(&output, "Export to workspace"));
        let Some(toolkits::Action::ExportScript(export)) = action else {
            panic!("export did not submit")
        };
        assert!(export.contains("dicomwrite"));
        assert!(!export.contains("matrust_dicom_skipped"));
        assert!(!export.contains("with_dicom_collection_count"));
        assert!(!export.contains("dir(here)"));
        assert!(export.contains("fullfile(tempdir, 'matrust-dicom-sample')"));
        let mut exported = Engine::default();
        exported.execute(&export).unwrap();
        let output = idle(&mut host, &ctx);
        let action = click(&mut host, &ctx, text_rect(&output, "Generate script"));
        let Some(toolkits::Action::SaveAppScript(kind, saved)) = action else {
            panic!("script did not submit")
        };
        assert_eq!(kind, AppKind::DicomBrowser);
        assert!(saved.contains("dicomwrite"));
        assert!(saved.contains("fullfile(tempdir, 'matrust-dicom-sample')"));
        let mut fresh = Engine::default();
        fresh.execute(&saved).unwrap();
        assert_equal(&exported, &fresh, &host.app.names);
    }
}
