#[test]
fn folder_resolution_commits_only_received_success_and_cancel_keeps_history() {
    let ctx = egui::Context::default();
    let root = tempfile::tempdir().unwrap();
    for name in ["a", "b"] {
        std::fs::create_dir(root.path().join(name)).unwrap();
    }
    let mut app = folder_navigation_app(&ctx, root.path());
    let previous = app.folder.clone();
    let history = app.folder_navigation.destination(false);
    assert!(app.change_folder(root.path().join("a")));
    assert_eq!(
        app.folder, previous,
        "Submission must not mutate committed location"
    );
    assert!(app.folder_browser.navigating());
    app.queue_open_paths([root.path().join("b")]);
    app.stop_work();
    assert!(app.open_queue.is_empty());
    assert!(app.worker.cancel.load(Ordering::Relaxed));
    app.receive_folder();
    assert_eq!(app.folder, previous);
    assert_eq!(app.folder_navigation.destination(false), history);
    assert!(app.change_folder(root.path().join("missing")));
    finish_folder(&mut app);
    assert_eq!(app.folder, previous);
    assert!(app.status.contains("Could not open folder"));
    assert!(app.change_folder(root.path().join("a")));
    assert!(app.change_folder(root.path().join("b")));
    finish_folder(&mut app);
    assert_eq!(app.folder, root.path().join("b").canonicalize().unwrap());
    assert_eq!(
        app.folder_navigation.destination(false).unwrap().1,
        previous
    );
    assert!(!app.folder_browser.navigating());
}
#[test]
fn folder_resolution_workspaces_isolate_relative_files_and_function_paths_without_host_cwd() {
    let cwd = std::env::current_dir().unwrap();
    let root = tempfile::tempdir().unwrap();
    let a = root.path().join("a");
    let b = root.path().join("b");
    for (folder, value) in [(&a, 11), (&b, 22)] {
        std::fs::create_dir(folder).unwrap();
        std::fs::write(
            folder.join("local_value.m"),
            format!("function y=local_value()\ny={value};\nend\n"),
        )
        .unwrap();
    }
    let mut first = folder_navigation_app(&egui::Context::default(), &a);
    let mut second = folder_navigation_app(&egui::Context::default(), &b);
    first.run("answer=local_value();plot([1 2],[1 2]);".into());
    second.run("answer=local_value();".into());
    finish(&mut first);
    finish(&mut second);
    assert_eq!(
        first.variables["answer"]
            .as_numeric()
            .unwrap()
            .as_scalar()
            .unwrap(),
        11.
    );
    assert_eq!(
        second.variables["answer"]
            .as_numeric()
            .unwrap()
            .as_scalar()
            .unwrap(),
        22.
    );
    for app in [&mut first, &mut second] {
        app.submit(Job::ExportVariable("values.csv".into(), "answer".into()));
        finish(app);
        app.submit(Job::SaveWorkspace("workspace.mat".into()));
        finish(app);
        app.open_path("local_value.m".into());
        finish(app);
        assert_eq!(
            app.documents[app.active].path.as_ref().unwrap(),
            &app.folder.join("local_value.m")
        );
    }
    assert_ne!(
        std::fs::read(a.join("values.csv")).unwrap(),
        std::fs::read(b.join("values.csv")).unwrap()
    );
    assert!(a.join("workspace.mat").is_file() && b.join("workspace.mat").is_file());
    first.submit(Job::SaveFigure("plot.mfig".into(), None));
    finish(&mut first);
    assert!(a.join("plot.mfig").is_file() && !b.join("plot.mfig").exists());
    assert_eq!(std::env::current_dir().unwrap(), cwd);
}
#[test]
fn folder_resolution_queued_directories_wait_for_each_commit() {
    let ctx = egui::Context::default();
    let root = tempfile::tempdir().unwrap();
    let paths: Vec<_> = ["a", "b", "c"]
        .into_iter()
        .map(|name| {
            let path = root.path().join(name);
            std::fs::create_dir(&path).unwrap();
            path
        })
        .collect();
    let mut app = folder_navigation_app(&ctx, root.path());
    app.queue_open_paths(paths.clone());
    let deadline = Instant::now() + Duration::from_secs(5);
    while (app.busy || !app.open_queue.is_empty() || app.folder_browser.navigating())
        && Instant::now() < deadline
    {
        frame(&mut app, &ctx, vec![]);
        std::thread::sleep(Duration::from_millis(1));
    }
    assert!(app.open_queue.is_empty() && !app.folder_browser.navigating());
    assert_eq!(app.folder, paths[2].canonicalize().unwrap());
    app.step_folder(false);
    finish_folder(&mut app);
    assert_eq!(app.folder, paths[1].canonicalize().unwrap());
    app.step_folder(false);
    finish_folder(&mut app);
    assert_eq!(app.folder, paths[0].canonicalize().unwrap());
}
#[test]
fn folder_resolution_export_request_keeps_its_folder_after_navigation_changes() {
    let ctx = egui::Context::default();
    let root = tempfile::tempdir().unwrap();
    let other = root.path().join("other");
    std::fs::create_dir(&other).unwrap();
    let mut app = folder_navigation_app(&ctx, root.path());
    app.run("answer=5;".into());
    finish(&mut app);
    let frozen = Job::ExportVariable("frozen.csv".into(), "answer".into())
        .in_folder(&app.folder)
        .unwrap();
    app.change_folder(other.clone());
    finish_folder(&mut app);
    app.submit(frozen);
    finish(&mut app);
    assert!(root.path().join("frozen.csv").exists());
    assert!(!other.join("frozen.csv").exists());
}
