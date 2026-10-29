fn assert_v73_typed_equal(expected: &matrust::Array, actual: &matrust::Array) {
    assert_eq!((expected.rows(), expected.cols()), (actual.rows(), actual.cols()));
    assert_eq!(expected.is_complex(), actual.is_complex());
    assert_eq!(expected.is_logical(), actual.is_logical());
    for col in 0..expected.cols() {
        for row in 0..expected.rows() {
            let a = expected.scalar(row, col).unwrap().into_complex();
            let b = actual.scalar(row, col).unwrap().into_complex();
            assert_eq!((a.re.to_bits(), a.im.to_bits()), (b.re.to_bits(), b.im.to_bits()));
        }
    }
}

#[test]
fn workspace_v73_save_worker_preserves_workspace_and_reopens_typed_data() {
    let ctx = egui::Context::default();
    let mut app = legacy_sample_desktop(ctx.clone(), true);
    app.run("A=[1 2 3;4 5 6];mask=logical([0 1 0]);Z=complex([1 -0 3;4 5 6],[7 -0 9;10 11 12]);ZeroImag=complex([1 2 3]);E=complex(zeros(0,3));Chunk=complex(ones(20000,1),ones(20000,1)*2);Alias=Chunk;".into());
    finish(&mut app);
    assert!(app.variables["Chunk"].same_snapshot(&app.variables["Alias"]));
    let before = app.variables.clone();
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("saved.mat");
    let job = Job::SaveWorkspaceV73("saved.mat".into()).in_folder(dir.path()).unwrap();
    app.submit(job);
    finish(&mut app);
    assert!(path.exists());
    assert!(app.console.iter().any(|line| line.contains("Saved v7.3 workspace")));
    for (name, value) in &before {
        assert!(value.same_snapshot(&app.variables[name]));
    }
    {
        let file = hdf5::File::open(&path).unwrap();
        let dataset = file.dataset("Chunk").unwrap();
        assert_eq!(dataset.shape(), vec![1, 20000]);
        let chunk = dataset.chunk().expect("Large complex arrays use bounded chunks");
        assert!(chunk.iter().product::<usize>() * 16 <= 256 * 1024);
        assert!(dataset.filters().contains(&hdf5::filters::Filter::Shuffle));
        assert!(dataset.filters().contains(&hdf5::filters::Filter::Deflate(1)));
    }
    app.run("clear;".into());
    finish(&mut app);
    app.open_path(path);
    finish(&mut app);
    click_label(&mut app, &ctx, "Select supported");
    click_label(&mut app, &ctx, "Load selected");
    finish(&mut app);
    assert_eq!(app.variables.len(), before.len());
    for (name, value) in &before {
        assert_v73_typed_equal(value, &app.variables[name]);
    }
}

#[test]
fn workspace_v73_save_formats_are_visible_in_wide_and_compact_toolstrips() {
    for size in [egui::vec2(1440., 900.), egui::vec2(960., 650.)] {
        let ctx = egui::Context::default();
        let mut app = legacy_sample_desktop(ctx.clone(), true);
        app.layout.toolstrip_collapsed = false;
        if size.x < 1100. {
            click_label_at(&mut app, &ctx, size, "Home");
        }
        click_label_at(&mut app, &ctx, size, "Save workspace");
        let output = frame_at(&mut app, &ctx, size, vec![]);
        for label in ["MATLAB v7 (Level-5)", "MATLAB v7.3 (HDF5)"] {
            assert!(output.shapes.iter().any(|s| text_position(&s.shape, label).is_some()), "{label} at {size:?}");
        }
        assert!(!app.busy, "Opening format choices must not start a save");
    }
}

#[test]
fn complex_workspace_v73_menu_saves_and_imports_in_wide_and_compact_views() {
    for size in [egui::vec2(1440., 900.), egui::vec2(960., 650.)] {
        let ctx = egui::Context::default();
        let mut app = legacy_sample_desktop(ctx.clone(), true);
        let dir = tempfile::tempdir().unwrap();
        app.folder = dir.path().to_owned();
        app.run("R=[1 2];L=logical([0 1]);Z=complex([3 -0],[5 -0]);E=complex(zeros(4,0));".into());
        finish(&mut app);
        let before = app.variables.clone();
        app.layout.toolstrip_collapsed = false;
        if size.x < 1100. { click_label_at(&mut app, &ctx, size, "Home"); }
        {
            let choice = toolstrip::workspace_save_choice_for_test(Some("menu.mat".into()));
            click_label_at(&mut app, &ctx, size, "Save workspace");
            click_label_at(&mut app, &ctx, size, "MATLAB v7.3 (HDF5)");
            assert_eq!(choice.calls(), 1, "One gesture must invoke one picker");
        }
        finish(&mut app);
        let path = dir.path().join("menu.mat");
        assert!(path.exists());
        assert!(app.console.iter().any(|line| line.contains("Saved v7.3 workspace")));
        for (name, value) in &before { assert!(value.same_snapshot(&app.variables[name])); }
        app.run("clear;keep=99;".into());
        finish(&mut app);
        let keep = app.variables["keep"].clone();
        app.open_path(path);
        finish(&mut app);
        click_label_at(&mut app, &ctx, size, "Select supported");
        click_label_at(&mut app, &ctx, size, "Load selected");
        finish(&mut app);
        assert!(keep.same_snapshot(&app.variables["keep"]));
        assert_eq!(app.variables.len(), before.len() + 1);
        for (name, value) in &before { assert_v73_typed_equal(value, &app.variables[name]); }
    }
}

#[test]
fn selected_typed_v73_save_retains_unselected_complex_values() {
    let ctx = egui::Context::default();
    let mut app = legacy_sample_desktop(ctx.clone(), true);
    let dir = tempfile::tempdir().unwrap();
    app.folder = dir.path().to_owned();
    app.run("R=[1 2];L=logical([0 1]);Z=complex([3 4],[5 6]);E=complex(zeros(0,3));Other=Z;".into());
    finish(&mut app);
    let before = app.variables.clone();
    // Selected real/logical export remains supported even with complex values elsewhere.
    app.run("save('real.mat','R','L','-v7.3');save('selected.mat','R','L','Z','E','-v7.3');".into());
    finish(&mut app);
    for (file, expected) in [("real.mat", vec!["L", "R"]), ("selected.mat", vec!["E", "L", "R", "Z"])] {
        let catalog = matrust::files::inspect_workspace(&dir.path().join(file), &std::sync::atomic::AtomicBool::new(false)).unwrap();
        let mut names = catalog.variables().map(|v| v.name.as_str()).collect::<Vec<_>>();
        names.sort_unstable();
        assert_eq!(names, expected);
    }
    for (name, value) in &before { assert!(value.same_snapshot(&app.variables[name])); }
    app.run("clear R L Z E;".into());
    finish(&mut app);
    app.open_path(dir.path().join("selected.mat"));
    finish(&mut app);
    click_label(&mut app, &ctx, "Select supported");
    click_label(&mut app, &ctx, "Load selected");
    finish(&mut app);
    assert!(before["Other"].same_snapshot(&app.variables["Other"]));
    for name in ["R", "L", "Z", "E"] { assert_v73_typed_equal(&before[name], &app.variables[name]); }
}

#[test]
fn complex_workspace_v73_stop_before_worker_io_preserves_destination_and_retry() {
    let ctx = egui::Context::default();
    let mut app = legacy_sample_desktop(ctx.clone(), true);
    let dir = tempfile::tempdir().unwrap();
    app.folder = dir.path().to_owned();
    app.run("Z=complex([1 2],[3 4]);Alias=Z;".into());
    finish(&mut app);
    let before = app.variables.clone();
    let path = dir.path().join("existing.mat");
    std::fs::write(&path, b"destination remains intact").unwrap();
    let (send, receive) = std::sync::mpsc::sync_channel(1);
    let original = std::mem::replace(&mut app.worker.send, send);
    app.layout.toolstrip_collapsed = false;
    {
        let choice = toolstrip::workspace_save_choice_for_test(Some("existing.mat".into()));
        click_label(&mut app, &ctx, "Save workspace");
        click_label(&mut app, &ctx, "MATLAB v7.3 (HDF5)");
        assert_eq!(choice.calls(), 1);
    }
    assert!(app.busy);
    let job = receive.try_recv().expect("Actual accepted menu job");
    assert!(receive.try_recv().is_err());
    app.worker.send = original;
    app.stop_work();
    assert!(app.worker.cancel.load(Ordering::Relaxed));
    let console_start = app.console.len();
    assert!(app.worker.send.send(job).is_ok());
    finish(&mut app);
    assert_eq!(std::fs::read(&path).unwrap(), b"destination remains intact");
    assert!(app.console[console_start..].iter().any(|line| line.to_lowercase().contains("cancel")));
    assert!(!app.console[console_start..].iter().any(|line| line.contains("Saved v7.3 workspace")));
    for (name, value) in &before { assert!(value.same_snapshot(&app.variables[name])); }
    app.submit(Job::SaveWorkspaceV73(path.clone()));
    finish(&mut app);
    let catalog = matrust::files::inspect_workspace(&path, &std::sync::atomic::AtomicBool::new(false)).unwrap();
    assert_eq!(catalog.variables().count(), 2);
    for (name, value) in &before { assert!(value.same_snapshot(&app.variables[name])); }
}
