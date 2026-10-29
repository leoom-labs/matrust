#[test]
fn folder_browser_filter_refresh_and_clear_preserve_open_work() {
    let ctx = egui::Context::default();
    let dir = tempfile::tempdir().unwrap();
    for name in ["alpha.m", "beta.csv", "ALPHABET.txt"] {
        std::fs::write(dir.path().join(name), "x=1;").unwrap();
    }
    let mut app = folder_navigation_app(&ctx, dir.path());
    app.run("keep=[1 2 3];plot(keep);".into());
    finish(&mut app);
    finish_folder(&mut app);
    let original = app.variables["keep"].clone();
    let figure = app.figure.clone().unwrap();
    // The filter is an icon until it is used.
    app.folder_browser.filter_open = true;
    frame(&mut app, &ctx, vec![]);
    ctx.memory_mut(|m| m.request_focus(egui::Id::new("folder_filter")));
    frame(&mut app, &ctx, vec![egui::Event::Text("ALPHA".into())]);
    assert_eq!(app.folder_browser.filter, "ALPHA");
    assert!(app.entries.is_empty());
    app.refresh_folder();
    finish_folder(&mut app);
    assert_eq!(app.entries.len(), 2);
    assert!(app.folder_browser.notice.contains("2 matches"));
    assert!(original.same_snapshot(&app.variables["keep"]));
    assert!(figure.same_snapshot(app.figure.as_ref().unwrap()));
    let output = frame(&mut app, &ctx, vec![]);
    let position = output
        .shapes
        .iter()
        .filter_map(|shape| text_position(&shape.shape, "Clear filename filter"))
        .find(|pos| pos.x < 34. + tokens::LEFT_COLUMN)
        .unwrap();
    pointer_frame(&mut app, &ctx, egui::vec2(1440., 900.), position, true);
    pointer_frame(&mut app, &ctx, egui::vec2(1440., 900.), position, false);
    finish_folder(&mut app);
    assert_eq!(app.entries.len(), 3);
    assert!(app.folder_browser.filter.is_empty());
    std::fs::write(dir.path().join("new.m"), "").unwrap();
    folder_navigation_key(&mut app, &ctx, egui::Modifiers::NONE, egui::Key::F5);
    assert_eq!(app.entries.len(), 4);
}
#[test]
fn folder_browser_stale_results_stop_and_errors_keep_current_location() {
    let ctx = egui::Context::default();
    let dir = tempfile::tempdir().unwrap();
    let a = dir.path().join("a");
    let b = dir.path().join("b");
    std::fs::create_dir(&a).unwrap();
    std::fs::create_dir(&b).unwrap();
    std::fs::write(a.join("old.m"), "").unwrap();
    std::fs::write(b.join("current.m"), "").unwrap();
    let mut app = folder_navigation_app(&ctx, dir.path());
    app.change_folder(a);
    app.change_folder(b.clone());
    finish_folder(&mut app);
    assert_eq!(app.entries.len(), 1);
    assert_eq!(app.entries[0].name, "current.m");
    app.refresh_folder();
    app.folder_browser.stop();
    app.receive_folder();
    assert!(!app.folder_browser.loading);
    assert!(app.folder_browser.notice.contains("stopped"));
    assert_eq!(app.entries[0].name, "current.m");
    std::fs::remove_file(b.join("current.m")).unwrap();
    std::fs::remove_dir(&b).unwrap();
    app.refresh_folder();
    finish_folder(&mut app);
    assert!(app.folder_browser.notice.contains("Could not read folder"));
    assert!(app.folder.ends_with("b"));
    assert_eq!(app.entries[0].name, "current.m");
}
#[test]
fn folder_browser_virtualizes_large_lists_and_opens_visible_script() {
    let ctx = egui::Context::default();
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(dir.path().join("000000.m"), "marker=123;").unwrap();
    let mut app = folder_navigation_app(&ctx, dir.path());
    // Real scanning has separate coverage; UI stress need not create 100k files.
    app.entries = (0..100_000)
        .map(|n| FolderEntry {
            path: dir.path().join(format!("{n:06}.m")),
            name: format!("{n:06}.m"),
            directory: false,
        })
        .collect();
    for _ in 0..3 {
        frame_at(&mut app, &ctx, egui::vec2(960., 650.), vec![]);
    }
    let panel = egui::containers::panel::PanelState::load(&ctx, egui::Id::new("workspace"))
        .unwrap()
        .outer_rect;
    for _ in 0..10 {
        frame_at(&mut app, &ctx, egui::vec2(960., 650.), vec![]);
    }
    assert_eq!(
        panel,
        egui::containers::panel::PanelState::load(&ctx, egui::Id::new("workspace"))
            .unwrap()
            .outer_rect,
        "Filter controls must not expand the sidebar on each frame"
    );
    let output = frame_at(&mut app, &ctx, egui::vec2(960., 650.), vec![]);
    assert!(output.shapes.len() < 1000, "Offscreen rows were rendered");
    assert!(
        output
            .shapes
            .iter()
            .any(|shape| text_position(&shape.shape, "000000.m").is_some())
    );
    assert!(
        !output
            .shapes
            .iter()
            .any(|shape| text_position(&shape.shape, "099999.m").is_some())
    );
    app.open_path(dir.path().join("000000.m"));
    finish(&mut app);
    assert!(app.documents[app.active].text.contains("marker=123"));
    assert!(!app.variables.contains_key("marker"));
}


fn files_key(app: &mut Workspace, ctx: &egui::Context, size: egui::Vec2, key: egui::Key) -> egui::FullOutput {
    frame_at(app, ctx, size, vec![egui::Event::Key {
        key, physical_key: Some(key), pressed: true, repeat: false,
        modifiers: egui::Modifiers::NONE,
    }])
}

#[test]
fn files_selection_keyboard_navigation_opens_script_without_evaluation_in_both_themes() {
    for theme in [session::DesktopTheme::Light, session::DesktopTheme::Dark] {
        for size in [egui::vec2(960., 650.), egui::vec2(1440., 900.)] {
            let ctx = egui::Context::default();
            let dir = tempfile::tempdir().unwrap();
            std::fs::create_dir(dir.path().join("a_folder")).unwrap();
            std::fs::write(dir.path().join("a_folder/inside.txt"), "inside=7;").unwrap();
            std::fs::write(dir.path().join("z_script.m"), "must_not_evaluate=42;").unwrap();
            let mut app = folder_navigation_app(&ctx, dir.path());
            app.layout.theme = Some(theme); app.apply_desktop_theme(&ctx);
            app.set_workspace_dock(&ctx, session::WorkspaceDock::Right);
            app.run("keep=[1 2 3];alias=keep;plot(keep);".into()); finish(&mut app);
            // Worker completion starts another listing refresh, and rows stay disabled while
            // it loads. Drain it so the click below cannot land on a disabled row.
            finish_folder(&mut app);
            let data = app.variables["keep"].clone();
            let figure = app.figure.clone().unwrap();
            let folder = app.folder.clone();
            for _ in 0..3 { frame_at(&mut app, &ctx, size, vec![]); }
            assert_eq!(ctx.global_style().visuals.dark_mode, theme == session::DesktopTheme::Dark);
            let viewport = ctx.read_response(folder_browser::list_id()).unwrap().rect;
            assert!(viewport.is_finite() && ctx.content_rect().contains_rect(viewport));
            assert!(viewport.height() > 150., "Files should use its finite panel: {viewport:?}");
            click_label_at(&mut app, &ctx, size, "a_folder/");
            assert_eq!(app.folder_browser.selected_path(), Some(folder.join("a_folder").as_path()));
            assert_eq!(app.folder, folder, "A single click selects without opening");
            assert!(ctx.memory(|m| m.has_focus(folder_browser::list_id())));
            files_key(&mut app, &ctx, size, egui::Key::Enter); finish_folder(&mut app);
            assert!(app.folder.ends_with("a_folder"));
            files_key(&mut app, &ctx, size, egui::Key::Backspace); finish_folder(&mut app);
            assert_eq!(app.folder, folder);
            files_key(&mut app, &ctx, size, egui::Key::End);
            assert_eq!(app.folder_browser.selected_path(), Some(folder.join("z_script.m").as_path()));
            files_key(&mut app, &ctx, size, egui::Key::Enter); finish(&mut app);
            assert_eq!(app.documents[app.active].text, "must_not_evaluate=42;");
            assert!(!app.variables.contains_key("must_not_evaluate"));
            assert!(data.same_snapshot(&app.variables["keep"]));
            assert!(data.same_snapshot(&app.variables["alias"]));
            assert!(figure.same_snapshot(app.figure.as_ref().unwrap()));
        }
    }
}

#[test]
fn files_keyboard_reveals_virtualized_rows_and_refresh_tracks_paths_not_indices() {
    let ctx = egui::Context::default(); let dir = tempfile::tempdir().unwrap();
    std::fs::write(dir.path().join("middle.m"), "keep=1;").unwrap();
    let mut app = folder_navigation_app(&ctx, dir.path()); let size = egui::vec2(960., 650.);
    for _ in 0..3 { frame_at(&mut app, &ctx, size, vec![]); }
    click_label_at(&mut app, &ctx, size, "middle.m");
    let selected = app.folder_browser.selected_path().unwrap().to_owned();
    std::fs::write(dir.path().join("aaa.m"), "").unwrap();
    app.refresh_folder(); finish_folder(&mut app);
    assert_eq!(app.folder_browser.selected_path(), Some(selected.as_path()));
    files_key(&mut app, &ctx, size, egui::Key::ArrowUp);
    assert!(ctx.memory(|m| m.has_focus(folder_browser::list_id())));
    assert!(app.folder_browser.selected_path().unwrap().ends_with("aaa.m"));
    app.entries = (0..100_000).map(|n| FolderEntry {
        path: dir.path().join(format!("{n:06}.m")), name: format!("{n:06}.m"), directory: false,
    }).collect();
    let output = files_key(&mut app, &ctx, size, egui::Key::End);
    assert!(app.folder_browser.selected_path().unwrap().ends_with("099999.m"));
    assert!(output.shapes.len() < 1000);
    let output = frame_at(&mut app, &ctx, size, vec![]);
    fn visible_ink(shape: &egui::Shape, clip: egui::Rect, viewport: egui::Rect) -> bool {
        match shape {
            egui::Shape::Text(text) if text.galley.text() == "099999.m" => {
                let ink = text.visual_bounding_rect();
                assert!(!text.galley.elided && clip.contains_rect(ink) && viewport.contains_rect(ink),
                    "Selected row glyphs must be visible: {ink:?}, clip={clip:?}, viewport={viewport:?}");
                true
            }
            egui::Shape::Vec(shapes) => shapes.iter().any(|shape| visible_ink(shape, clip, viewport)),
            _ => false,
        }
    }
    let viewport = ctx.read_response(folder_browser::list_id()).unwrap().rect;
    assert!(output.shapes.iter().any(|s| visible_ink(&s.shape, s.clip_rect, viewport)));
    assert!(!output.shapes.iter().any(|s| text_position(&s.shape, "000000.m").is_some()));
    files_key(&mut app, &ctx, size, egui::Key::ArrowDown);
    assert!(ctx.memory(|m| m.has_focus(folder_browser::list_id())));
    files_key(&mut app, &ctx, size, egui::Key::PageUp);
    assert!(!app.folder_browser.selected_path().unwrap().ends_with("099999.m"));
    files_key(&mut app, &ctx, size, egui::Key::Home);
    assert!(app.folder_browser.selected_path().unwrap().ends_with("000000.m"));
    app.refresh_folder(); finish_folder(&mut app);
    assert!(app.folder_browser.selected_path().is_none());
    assert_eq!(app.entries.len(), 2);
}

#[test]
fn files_keyboard_is_scoped_to_list_and_respects_split_panel_and_filter() {
    let ctx = egui::Context::default(); let dir = tempfile::tempdir().unwrap();
    std::fs::write(dir.path().join("alpha.m"), "alpha=1;").unwrap();
    std::fs::write(dir.path().join("beta.m"), "beta=1;").unwrap();
    let mut app = folder_navigation_app(&ctx, dir.path()); let size = egui::vec2(960., 650.);
    app.set_workspace_dock(&ctx, session::WorkspaceDock::BelowFiles);
    for _ in 0..3 { frame_at(&mut app, &ctx, size, vec![]); }
    let viewport = ctx.read_response(folder_browser::list_id()).unwrap().rect;
    let workspace = egui::containers::panel::PanelState::load(&ctx, egui::Id::new("workspace_variables_left")).unwrap().outer_rect;
    assert!(viewport.is_finite() && viewport.bottom() <= workspace.top());
    app.folder_browser.filter_open = true;
    frame_at(&mut app, &ctx, size, vec![]);
    ctx.memory_mut(|m| m.request_focus(egui::Id::new("folder_filter")));
    files_key(&mut app, &ctx, size, egui::Key::End);
    assert!(app.folder_browser.selected_path().is_none());
    files_key(&mut app, &ctx, size, egui::Key::ArrowDown);
    assert!(app.folder_browser.selected_path().unwrap().ends_with("alpha.m"));
    assert!(ctx.memory(|m| m.has_focus(folder_browser::list_id())));
    ctx.memory_mut(|m| m.request_focus(egui::Id::new("command_input")));
    files_key(&mut app, &ctx, size, egui::Key::End);
    assert!(app.folder_browser.selected_path().unwrap().ends_with("alpha.m"));
    // An empty filter folds back into its icon when it loses focus.
    app.folder_browser.filter_open = true;
    frame_at(&mut app, &ctx, size, vec![]);
    ctx.memory_mut(|m| m.request_focus(egui::Id::new("folder_filter")));
    frame_at(&mut app, &ctx, size, vec![egui::Event::Text("no-match".into())]);
    app.refresh_folder(); finish_folder(&mut app);
    assert!(app.entries.is_empty() && app.folder_browser.selected_path().is_none());
    files_key(&mut app, &ctx, size, egui::Key::ArrowDown);
    files_key(&mut app, &ctx, size, egui::Key::Enter);
    assert!(!app.busy && app.open_queue.is_empty());
}

#[test]
fn files_enter_survives_discard_once_and_rejects_late_ownership_changes() {
    for invalidation in 0..6 {
        let ctx = egui::Context::default(); let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("open_me.m"), "not_run=12;").unwrap();
        let mut app = folder_navigation_app(&ctx, dir.path()); let size = egui::vec2(960., 650.);
        for _ in 0..3 { frame_at(&mut app, &ctx, size, vec![]); }
        click_label_at(&mut app, &ctx, size, "open_me.m");
        let count = app.documents.len(); let mut passes = 0;
        crate::headless::run(&ctx, egui::RawInput {
            screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, size)),
            events: vec![egui::Event::Key { key: egui::Key::Enter,
                physical_key: Some(egui::Key::Enter), pressed: true, repeat: false,
                modifiers: egui::Modifiers::NONE }], ..Default::default()
        }, |root| {
            passes += 1;
            if passes == 1 { root.ctx().request_discard("Files open needs final pass"); }
            app.ui(root);
            if passes == 1 {
                assert!(!app.busy && !app.worker.opening && app.open_queue.is_empty());
                match invalidation {
                    1 => app.navigation.left = None,
                    2 => app.refresh_folder(),
                    3 => app.stop_work(),
                    4 => {
                        app.documents[app.active].text.push_str("\n% dirty");
                        app.documents[app.active].changed(); app.request_script_close(app.active);
                    }
                    5 => ctx.memory_mut(|m| m.request_focus(egui::Id::new("command_input"))),
                    _ => {}
                }
            }
        });
        assert!(passes >= 2);
        if invalidation == 0 {
            finish(&mut app);
            assert_eq!(app.documents.len(), count + 1);
            assert_eq!(app.documents[app.active].text, "not_run=12;");
            assert!(!app.variables.contains_key("not_run"));
        } else {
            assert!(!app.busy && !app.worker.opening && app.open_queue.is_empty());
            assert_eq!(app.documents.len(), count);
        }
    }
}


#[test]
fn files_pointer_release_keeps_only_same_frame_authorized_list_focus_after_discard() {
    for invalidation in 0..8 {
        let ctx=egui::Context::default();let dir=tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("selected.m"),"never_evaluate=23;").unwrap();
        let mut app=folder_navigation_app(&ctx,dir.path());let size=egui::vec2(960.,650.);
        if invalidation==7 {app.reveal_editor();}
        let mut position=None;
        for _ in 0..3 {
            position=frame_at(&mut app,&ctx,size,vec![]).shapes.iter()
                .find_map(|shape| text_position(&shape.shape,"selected.m"));
        }
        let position=position.unwrap();let documents=app.documents.len();
        pointer_frame(&mut app,&ctx,size,position,true);
        let mut passes=0;
        crate::headless::run(&ctx,egui::RawInput {
            screen_rect:Some(egui::Rect::from_min_size(egui::Pos2::ZERO,size)),
            events:vec![egui::Event::PointerMoved(position),egui::Event::PointerButton {
                pos:position,button:egui::PointerButton::Primary,pressed:false,modifiers:egui::Modifiers::NONE,
            }],..Default::default()
        },|root| {
            passes+=1;
            if passes==1 {root.ctx().request_discard("Files row release must survive sizing pass");}
            app.ui(root);
            if passes==1 {
                assert!(app.folder_browser.selected_path().unwrap().ends_with("selected.m"));
                assert_eq!(app.documents.len(),documents);
                match invalidation {
                    1=>ctx.memory_mut(|memory|memory.request_focus(egui::Id::new("command_input"))),
                    2=>app.stop_work(),
                    3=>app.refresh_folder(),
                    4=>app.navigation.left=None,
                    5=>{
                        app.documents[app.active].text.push_str("% changed");
                        app.documents[app.active].changed();app.request_script_close(app.active);
                    }
                    6=>ctx.memory_mut(|memory|memory.request_focus(egui::Id::new("folder_filter"))),
                    7=>ctx.memory_mut(|memory|memory.request_focus(editor::id(app.documents[app.active].id))),
                    _=>{}
                }
            }
        });
        assert!(passes>=2);
        assert!(app.open_queue.is_empty() && !app.worker.opening && !app.busy);
        assert_eq!(app.documents.len(),documents);
        if invalidation==0 {
            assert!(ctx.memory(|memory|memory.has_focus(folder_browser::list_id())));
            files_key(&mut app,&ctx,size,egui::Key::Enter);finish(&mut app);
            assert_eq!(app.documents.len(),documents+1);
            assert_eq!(app.documents[app.active].text,"never_evaluate=23;");
            assert!(!app.variables.contains_key("never_evaluate"));
        } else if matches!(invalidation,1|6|7) {
            // The old pointer release may itself surrender that other widget;
            // the Files ticket must still remain retired rather than reclaim it.
            assert!(!ctx.memory(|memory|memory.has_focus(folder_browser::list_id())));
        }
    }
}
