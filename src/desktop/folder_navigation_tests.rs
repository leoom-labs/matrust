fn folder_navigation_app(ctx: &egui::Context, root: &std::path::Path) -> Workspace {
    let mut app = legacy_sample_desktop(ctx.clone(), true);
    assert!(app.change_folder(root.to_owned()));
    finish_folder(&mut app);
    app
}
fn finish_folder(app: &mut Workspace) {
    let deadline = Instant::now() + Duration::from_secs(5);
    while app.folder_browser.loading && Instant::now() < deadline {
        app.receive_folder();
        std::thread::sleep(Duration::from_millis(1));
    }
    assert!(!app.folder_browser.loading, "{}", app.folder_browser.notice);
}
fn folder_navigation_key(
    app: &mut Workspace,
    ctx: &egui::Context,
    modifiers: egui::Modifiers,
    key: egui::Key,
) {
    frame(
        app,
        ctx,
        vec![egui::Event::Key {
            key,
            physical_key: None,
            pressed: true,
            repeat: false,
            modifiers,
        }],
    );
    finish_folder(app);
}
#[test]
fn folder_navigation_buttons_and_keys_preserve_documents_and_workspace() {
    let ctx = egui::Context::default();
    let dir = tempfile::tempdir().unwrap();
    for name in ["a", "b", "c"] {
        std::fs::create_dir(dir.path().join(name)).unwrap();
    }
    let cwd = std::env::current_dir().unwrap();
    let mut app = folder_navigation_app(&ctx, dir.path());
    app.run("keep=[1 2 3];plot(keep);".into());
    finish(&mut app);
    let data = app.variables["keep"].as_numeric().unwrap().data().as_ptr();
    let figure = app.figure.as_ref().unwrap().clone();
    let script = app.documents[0].text.clone();
    for name in ["a", "b", "c"] {
        assert!(app.change_folder(dir.path().join(name)));
        finish_folder(&mut app);
    }
    click_label(&mut app, &ctx, "Back");
    finish_folder(&mut app);
    assert!(app.folder.ends_with("b"));
    folder_navigation_key(&mut app, &ctx, egui::Modifiers::ALT, egui::Key::ArrowLeft);
    assert!(app.folder.ends_with("a"));
    click_label(&mut app, &ctx, "Forward");
    finish_folder(&mut app);
    assert!(app.folder.ends_with("b"));
    folder_navigation_key(&mut app, &ctx, egui::Modifiers::ALT, egui::Key::ArrowRight);
    assert!(app.folder.ends_with("c"));
    assert_eq!(
        data,
        app.variables["keep"].as_numeric().unwrap().data().as_ptr()
    );
    assert!(app.figure.as_ref().unwrap().same_snapshot(&figure));
    assert_eq!(app.documents[0].text, script);
    assert_eq!(std::env::current_dir().unwrap(), cwd);
}
#[test]
fn folder_navigation_address_focus_relative_paths_parent_and_refresh() {
    let ctx = egui::Context::default();
    let dir = tempfile::tempdir().unwrap();
    std::fs::create_dir(dir.path().join("child")).unwrap();
    let mut app = folder_navigation_app(&ctx, dir.path());
    for _ in 0..3 {
        frame(&mut app, &ctx, vec![]);
    }
    folder_navigation_key(&mut app, &ctx, egui::Modifiers::CTRL, egui::Key::L);
    assert!(ctx.memory(|m| m.has_focus(folder_navigation::address_id())));
    let range = egui::text_edit::TextEditState::load(&ctx, folder_navigation::address_id())
        .unwrap()
        .cursor
        .char_range()
        .unwrap()
        .as_sorted_char_range();
    assert_eq!(range.start.0..range.end.0, 0..app.folder_text.chars().count());
    frame(&mut app, &ctx, vec![egui::Event::Text("child".into())]);
    folder_navigation_key(&mut app, &ctx, egui::Modifiers::NONE, egui::Key::Enter);
    assert!(app.folder.ends_with("child"));
    let before = app.folder_navigation.destination(false);
    std::fs::write(app.folder.join("new.m"), "a=1;\n").unwrap();
    folder_navigation_key(&mut app, &ctx, egui::Modifiers::NONE, egui::Key::F5);
    assert!(app.entries.iter().any(|entry| entry.name == "new.m"));
    assert_eq!(app.folder_navigation.destination(false), before);
    app.recovery_dirty = false;
    assert!(app.change_folder(app.folder.clone()));
    finish_folder(&mut app);
    assert!(
        !app.recovery_dirty,
        "Revisiting the same folder must not request another checkpoint"
    );
    folder_navigation_key(&mut app, &ctx, egui::Modifiers::ALT, egui::Key::ArrowUp);
    assert_eq!(app.folder, std::fs::canonicalize(dir.path()).unwrap());
}
#[test]
fn folder_navigation_failures_keep_history_location_and_listing_transactional() {
    let ctx = egui::Context::default();
    let dir = tempfile::tempdir().unwrap();
    let a = dir.path().join("a");
    let b = dir.path().join("b");
    std::fs::create_dir(&a).unwrap();
    std::fs::create_dir(&b).unwrap();
    std::fs::write(b.join("sentinel.m"), "x=1;").unwrap();
    let mut app = folder_navigation_app(&ctx, dir.path());
    app.change_folder(a.clone());
    finish_folder(&mut app);
    app.change_folder(b.clone());
    finish_folder(&mut app);
    let folder = app.folder.clone();
    let previous = app.folder_navigation.destination(false);
    std::fs::remove_dir(&a).unwrap();
    click_label(&mut app, &ctx, "Back");
    finish_folder(&mut app);
    assert_eq!(app.folder, folder);
    assert_eq!(app.folder_navigation.destination(false), previous);
    finish_folder(&mut app);
    assert!(app.entries.iter().any(|e| e.name == "sentinel.m"));
    assert!(app.change_folder(b.join("sentinel.m")));
    finish_folder(&mut app);
    assert!(app.status.contains("Could not open folder"));
    assert_eq!(app.folder, folder);
    assert_eq!(app.folder_navigation.destination(false), previous);
    assert_eq!(
        app.folder_text,
        folder.display().to_string().trim_start_matches(r"\\?\")
    );
    assert!(app.try_open_path(b.join("sentinel.m/invalid")));
    finish(&mut app);
    assert!(app.status.contains("Could not open file"));
}
#[test]
fn folder_navigation_busy_and_modal_guards_prevent_history_mutation() {
    let ctx = egui::Context::default();
    let dir = tempfile::tempdir().unwrap();
    std::fs::create_dir(dir.path().join("a")).unwrap();
    let mut app = folder_navigation_app(&ctx, dir.path());
    app.change_folder(dir.path().join("a"));
    finish_folder(&mut app);
    let before = app.folder.clone();
    let previous = app.folder_navigation.destination(false);
    app.busy = true;
    folder_navigation_key(&mut app, &ctx, egui::Modifiers::ALT, egui::Key::ArrowLeft);
    assert!(!app.change_folder(dir.path().to_owned()));
    app.busy = false;
    app.pending_close = Some(0);
    folder_navigation_key(&mut app, &ctx, egui::Modifiers::ALT, egui::Key::ArrowLeft);
    assert_eq!(app.folder, before);
    assert_eq!(app.folder_navigation.destination(false), previous);
    app.pending_close = None;
}
