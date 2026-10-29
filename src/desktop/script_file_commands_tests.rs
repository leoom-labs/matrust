#[test]
fn script_file_commands_desktop_instances_use_captured_folders_and_worker_loads() {
    let cwd = std::env::current_dir().unwrap();
    let root = tempfile::tempdir().unwrap();
    let a = root.path().join("a");
    let b = root.path().join("b");
    std::fs::create_dir(&a).unwrap();
    std::fs::create_dir(&b).unwrap();
    let ctx = egui::Context::default();
    let mut first = folder_navigation_app(&ctx, &a);
    let mut second = folder_navigation_app(&egui::Context::default(), &b);
    first.run("A=11;mask=logical([0 1]);save same.mat A mask -v7.3;clear;load same.mat;".into());
    second.run("A=22;save('same.mat','A');clear;load('same.mat');".into());
    finish(&mut first);
    finish(&mut second);
    assert_eq!(first.variables["A"].scalar(0, 0).unwrap().into_real().unwrap(),11.);
    assert_eq!(second.variables["A"].scalar(0, 0).unwrap().into_real().unwrap(),22.);
    assert!(first.variables["mask"].is_logical());
    assert!(a.join("same.mat").is_file() && b.join("same.mat").is_file());
    // A queued script retains the captured directory even if the view changes.
    first.run("save captured.mat A -v7.3;".into());
    first.folder.clone_from(&b);
    finish(&mut first);
    assert!(a.join("captured.mat").exists() && !b.join("captured.mat").exists());
    assert_eq!(std::env::current_dir().unwrap(),cwd);
    assert!(matrust::session::FUNCTIONS.contains(&"save"));
    assert!(matrust::session::FUNCTIONS.contains(&"load"));
}

#[test]
fn command_window_runs_a_folder_script_by_name_in_the_workspace() {
    let root = tempfile::tempdir().unwrap();
    std::fs::write(root.path().join("setx.m"), "x = base + 1;").unwrap();
    let ctx = egui::Context::default();
    let mut app = folder_navigation_app(&ctx, root.path());
    app.run("base = 41;".into());
    finish(&mut app);
    // Exactly what the command window submits for a typed `setx`.
    app.run("setx".into());
    finish(&mut app);
    assert_eq!(app.variables["x"].scalar(0, 0).unwrap().into_real().unwrap(), 42.);
}
