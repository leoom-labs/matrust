// Actual menu clicks use a consistent viewport and verify both accessibility
// and active-document retention; painted text behind a window is not sufficient.
fn model_document_click_label_at(app: &mut Workspace, ctx: &egui::Context, size: egui::Vec2, label: &str) {
    let mut position = None;
    for _ in 0..3 {
        position = frame_at(app, ctx, size, vec![]).shapes.iter().rev()
            .find_map(|shape| text_position(&shape.shape, label));
    }
    let point = position.unwrap_or_else(|| panic!("Missing {label}"));
    if label == "File" {
        let (tab, _) = app.toolkits.model_document_region().unwrap();
        assert!(!tab.contains(point), "File must be outside the model tab");
    }
    for pressed in [true, false] { pointer_frame(app, ctx, size, point, pressed); }
    if label == "File" {
        assert!(egui::Popup::is_any_open(ctx), "Actual File click opens its menu");
        assert_eq!(app.generic_save_target(), document_save::Pane::Model);
    }
}

fn model_document_app(ctx: &egui::Context, root: &std::path::Path) -> (Workspace, PathBuf, PathBuf) {
    let mut app = figure_save_all_app(ctx, root, 1);
    let script = named_dirty_scripts(&mut app, root, 1).remove(0);
    let path = root.join("active.mrsim");
    matrust::simulation::Model::default().save(&path).unwrap();
    assert!(app.toolkits.request_open(path.clone()));
    app.advance_model_read();
    finish(&mut app);
    app.toolkits.model.steps = 321;
    app.show_plot_pane();
    // The opened model's tab is shown; Figure stays the active document until a press.
    app.toolkits.select_tool(toolkits::Tool::Model);
    (app, path, script)
}

fn model_document_canvas(app: &mut Workspace, ctx: &egui::Context, size: egui::Vec2) -> egui::Pos2 {
    for _ in 0..3 { frame_at(app, ctx, size, vec![]); }
    let response = ctx.read_response(egui::Id::new("model_diagram_keyboard")).unwrap();
    let (window, layer) = app.toolkits.model_document_region().unwrap();
    let visible = response.interact_rect.intersect(window).intersect(ctx.content_rect());
    assert!(visible.is_finite() && visible.is_positive());
    assert_eq!(response.layer_id, layer);
    let point = visible.center();
    assert_eq!(ctx.layer_id_at(point), Some(layer));
    point
}

#[test]
fn model_document_canvas_press_and_save_in_same_frame_leave_hidden_documents_untouched() {
    for size in [egui::vec2(1440., 900.), egui::vec2(960., 650.)] {
        let ctx = egui::Context::default();
        let dir = tempfile::tempdir().unwrap();
        let (mut app, path, script) = model_document_app(&ctx, dir.path());
        let figure_bytes = std::fs::read(dir.path().join("1.mfig")).unwrap();
        let values = app.variables["keep"].clone();
        let points = app.figure.as_ref().unwrap().series[0].points.clone();
        let position = model_document_canvas(&mut app, &ctx, size);
        frame_at(&mut app, &ctx, size, vec![
            egui::Event::PointerMoved(position),
            egui::Event::PointerButton { pos: position, button: egui::PointerButton::Primary,
                pressed: true, modifiers: egui::Modifiers::CTRL },
            egui::Event::Key { key: egui::Key::S, physical_key: None, pressed: true,
                repeat: false, modifiers: egui::Modifiers::CTRL },
        ]);
        finish(&mut app);
        assert_eq!(app.generic_save_target(), document_save::Pane::Model);
        assert_eq!(matrust::simulation::Model::load(&path).unwrap().steps, 321);
        assert!(!app.toolkits.model_dirty());
        assert_eq!(std::fs::read_to_string(&script).unwrap(), "old=1;\n");
        assert_eq!(std::fs::read(dir.path().join("1.mfig")).unwrap(), figure_bytes);
        assert!(app.documents[0].dirty && app.figure_dirty(1));
        assert!(values.same_snapshot(&app.variables["keep"]));
        assert!((points).same_storage(&app.figure.as_ref().unwrap().series[0].points));
        assert!(!app.variables.contains_key("sim_t"));
        pointer_frame(&mut app, &ctx, size, position, false);
    }
}

#[test]
fn model_document_close_key_and_menu_hide_view_and_preserve_unsaved_model() {
    for menu in [false, true] {
        let ctx = egui::Context::default();
        let dir = tempfile::tempdir().unwrap();
        let (mut app, path, script) = model_document_app(&ctx, dir.path());
        let size = egui::vec2(1440., 900.);
        let position = model_document_canvas(&mut app, &ctx, size);
        pointer_frame(&mut app, &ctx, size, position, true);
        pointer_frame(&mut app, &ctx, size, position, false);
        let model = serde_json::to_string(&app.toolkits.model).unwrap();
        let metadata = serde_json::to_string(&app.toolkits.model_metadata()).unwrap();
        let original = std::fs::read(&path).unwrap();
        assert_eq!(app.generic_save_target(), document_save::Pane::Model);
        if menu {
            model_document_click_label_at(&mut app, &ctx, size, "Menu");
            model_document_click_label_at(&mut app, &ctx, size, "File");
            model_document_click_label_at(&mut app, &ctx, size, "Close model view");
        } else {
            document_close_key(&mut app, &ctx, size);
        }
        assert!(!app.toolkits.model_open && app.toolkits.model_dirty());
        assert_eq!(serde_json::to_string(&app.toolkits.model).unwrap(), model);
        assert_eq!(serde_json::to_string(&app.toolkits.model_metadata()).unwrap(), metadata);
        assert_eq!(std::fs::read(&path).unwrap(), original);
        assert_eq!(std::fs::read_to_string(&script).unwrap(), "old=1;\n");
        assert_eq!(app.generic_save_target(), document_save::Pane::Figure);
        assert!(app.figure_close.is_none() && app.pending_close.is_none());
        assert!(app.documents[0].dirty && app.figure_dirty(1) && !app.allow_close);
        model_document_click_label_at(&mut app, &ctx, size, "Menu");
        model_document_click_label_at(&mut app, &ctx, size, "Tools");
        model_document_click_label_at(&mut app, &ctx, size, "Block Simulation");
        assert!(app.toolkits.model_open && app.toolkits.model_dirty());
        assert_eq!(app.generic_save_target(), document_save::Pane::Model);
        assert_eq!(serde_json::to_string(&app.toolkits.model).unwrap(), model);
    }
}

#[test]
fn model_document_menu_and_save_as_shortcuts_route_model_once() {
    let ctx = egui::Context::default();
    let dir = tempfile::tempdir().unwrap();
    let (mut app, path, _) = model_document_app(&ctx, dir.path());
    let size = egui::vec2(1440., 900.);
    app.show_model_document();
    model_document_click_label_at(&mut app, &ctx, size, "Menu");
    model_document_click_label_at(&mut app, &ctx, size, "File");
    model_document_click_label_at(&mut app, &ctx, size, "Save model");
    finish(&mut app);
    assert_eq!(matrust::simulation::Model::load(&path).unwrap().steps, 321);
    app.document_save.intercepted = Some(Vec::new());
    document_save_key(&mut app, &ctx, size, true);
    assert_eq!(app.generic_save_target(), document_save::Pane::Model);
    model_document_click_label_at(&mut app, &ctx, size, "Menu");
    model_document_click_label_at(&mut app, &ctx, size, "File");
    model_document_click_label_at(&mut app, &ctx, size, "Save model as…");
    assert_eq!(app.document_save.intercepted.as_deref(), Some(&[
        document_save::Request::Model { save_as: true },
        document_save::Request::Model { save_as: true },
    ][..]));
    // A real layout retry cannot publish another save or fall through to close.
    let mut passes = 0;
    let _ = crate::headless::run(&ctx, egui::RawInput {
        screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, size)),
        events: vec![egui::Event::ModifiersChanged(egui::Modifiers::CTRL),
            egui::Event::Key { key: egui::Key::S, physical_key: None,
                pressed: true, repeat: false, modifiers: egui::Modifiers::CTRL },
            egui::Event::Key { key: egui::Key::W, physical_key: None,
                pressed: true, repeat: false, modifiers: egui::Modifiers::CTRL }],
        ..Default::default()
    }, |root| {
        passes += 1;
        app.ui(root);
        if passes == 1 { root.ctx().request_discard("Model document action executes once"); }
    });
    assert!(passes >= 2);
    assert_eq!(app.document_save.intercepted.as_ref().unwrap().len(), 3);
    assert!(app.toolkits.model_open);
}

#[test]
fn model_document_conflict_cancel_and_failed_save_keep_view_and_other_documents() {
    let ctx = egui::Context::default();
    let dir = tempfile::tempdir().unwrap();
    let (mut app, path, script) = model_document_app(&ctx, dir.path());
    app.show_model_document();
    let external = matrust::simulation::Model { steps: 654, ..Default::default() };
    external.save(&path).unwrap();
    document_save_key(&mut app, &ctx, egui::vec2(1440., 900.), false);
    finish(&mut app);
    assert!(app.toolkits.model_file_modal_open());
    document_close_key(&mut app, &ctx, egui::vec2(1440., 900.));
    assert!(app.toolkits.model_open);
    click_label(&mut app, &ctx, "Cancel");
    assert!(app.toolkits.model_dirty() && app.toolkits.model_open);
    assert_eq!(matrust::simulation::Model::load(&path).unwrap().steps, 654);
    let failed_path = dir.path().join("folder.mrsim");
    std::fs::create_dir(&failed_path).unwrap();
    app.toolkits.restore_model(app.toolkits.model.clone(), Some(failed_path.clone()), Some(external));
    document_save_key(&mut app, &ctx, egui::vec2(1440., 900.), false);
    finish(&mut app);
    assert!(failed_path.is_dir() && app.toolkits.model_dirty() && app.toolkits.model_open);
    assert!(!app.toolkits.model_file_modal_open());
    assert_eq!(app.toolkits.model.steps, 321);
    assert_eq!(std::fs::read_to_string(&script).unwrap(), "old=1;\n");
    assert!(app.documents[0].dirty && app.figure_dirty(1));
}

#[test]
fn model_document_stop_after_committed_save_does_not_close_the_view() {
    let ctx = egui::Context::default();
    let dir = tempfile::tempdir().unwrap();
    let (mut app, path, script) = model_document_app(&ctx, dir.path());
    app.show_model_document();
    document_save_key(&mut app, &ctx, egui::vec2(1440., 900.), false);
    let reply = app.worker.receive.recv_timeout(Duration::from_secs(3)).unwrap();
    assert_eq!(matrust::simulation::Model::load(&path).unwrap().steps, 321);
    app.stop_work();
    app.busy = false;
    app.receive_model_write(reply.model_write.unwrap());
    frame(&mut app, &ctx, vec![]);
    assert!(!app.toolkits.model_dirty() && app.toolkits.model_open);
    assert!(app.toolkits.message.contains("subsequent actions cancelled"));
    assert_eq!(std::fs::read_to_string(&script).unwrap(), "old=1;\n");
    assert!(app.pending_close.is_none() && app.figure_close.is_none() && !app.allow_close);
}

#[test]
fn model_document_late_direct_and_preflight_open_preserve_intervening_script_target() {
    for preflight in [false, true] {
        let ctx = egui::Context::default();
        let dir = tempfile::tempdir().unwrap();
        let mut app = figure_save_all_app(&ctx, dir.path(), 1);
        named_dirty_scripts(&mut app, dir.path(), 1);
        let path = dir.path().join("late.mrsim");
        matrust::simulation::Model { steps: 456, ..Default::default() }.save(&path).unwrap();
        if preflight {
            assert!(app.try_open_path(path));
        } else {
            assert!(app.toolkits.request_open(path));
            app.advance_model_read();
        }
        app.reveal_editor();
        finish(&mut app);
        frame(&mut app, &ctx, vec![]);
        assert_eq!(app.toolkits.model.steps, 456);
        assert!(app.toolkits.model_open);
        assert_eq!(app.generic_save_target(), document_save::Pane::Script);
        assert!(app.documents[0].dirty && app.figure_dirty(1));
    }
}

#[test]
fn model_document_completion_frame_text_and_cancelled_open_do_not_steal_target() {
    let ctx = egui::Context::default();
    let dir = tempfile::tempdir().unwrap();
    let mut app = figure_save_all_app(&ctx, dir.path(), 1);
    named_dirty_scripts(&mut app, dir.path(), 1);
    app.reveal_editor();
    frame(&mut app, &ctx, vec![]);
    ctx.memory_mut(|memory| memory.request_focus(editor::id(app.documents[0].id)));
    frame(&mut app, &ctx, vec![]);
    let path = dir.path().join("late.mrsim");
    matrust::simulation::Model { steps: 456, ..Default::default() }.save(&path).unwrap();
    assert!(app.toolkits.request_open(path.clone()));
    app.advance_model_read();
    let reply = app.worker.receive.recv_timeout(Duration::from_secs(3)).unwrap();
    let (send, receive) = std::sync::mpsc::sync_channel(1);
    assert!(send.send(reply).is_ok());
    let original = std::mem::replace(&mut app.worker.receive, receive);
    frame(&mut app, &ctx, vec![egui::Event::Text("typed".into())]);
    app.worker.receive = original;
    assert_eq!(app.generic_save_target(), document_save::Pane::Script);
    assert!(app.documents[0].text.contains("typed"));
    app.toolkits.hide_model_view();
    assert!(app.toolkits.request_open(path));
    app.advance_model_read();
    let reply = app.worker.receive.recv_timeout(Duration::from_secs(3)).unwrap();
    app.stop_work();
    app.busy = false;
    app.receive_model_read(reply.model_read.unwrap());
    assert!(!app.toolkits.model_open);
    assert_eq!(app.generic_save_target(), document_save::Pane::Script);
}

#[test]
fn model_document_preview_close_restores_model_and_hidden_model_falls_back_to_figure() {
    let ctx = egui::Context::default();
    let dir = tempfile::tempdir().unwrap();
    let (mut app, _, _) = model_document_app(&ctx, dir.path());
    app.show_model_document();
    let source = dir.path().join("preview.png");
    std::fs::copy(std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/preview/tiff-preview/reference-1.png"), &source).unwrap();
    app.submit(Job::OpenImage(source));
    finish(&mut app);
    frame(&mut app, &ctx, vec![]);
    assert_eq!(app.generic_save_target(), document_save::Pane::Preview);
    document_close_key(&mut app, &ctx, egui::vec2(1440., 900.));
    assert!(app.image_view.is_none() && app.toolkits.model_open);
    assert_eq!(app.generic_save_target(), document_save::Pane::Model);
    document_close_key(&mut app, &ctx, egui::vec2(1440., 900.));
    assert!(!app.toolkits.model_open);
    assert_eq!(app.generic_save_target(), document_save::Pane::Figure);
}

#[test]
fn model_document_keyboard_input_activates_model_but_menu_navigation_does_not() {
    let ctx = egui::Context::default();
    let dir = tempfile::tempdir().unwrap();
    let (mut app, _, _) = model_document_app(&ctx, dir.path());
    let size = egui::vec2(1440., 900.);
    model_document_canvas(&mut app, &ctx, size);
    let id = egui::Id::new("model_diagram_keyboard");
    ctx.memory_mut(|memory| memory.request_focus(id));
    frame(&mut app, &ctx, vec![]);
    // The model tab stays shown and focused while Figure is the active document.
    app.document_save.activate(document_save::Pane::Figure);
    frame(&mut app, &ctx, vec![]);
    assert_eq!(app.generic_save_target(), document_save::Pane::Figure);
    assert_eq!(ctx.memory(|memory| memory.focused()), Some(id));
    frame(&mut app, &ctx, vec![egui::Event::Key {
        key: egui::Key::ArrowRight, physical_key: None, pressed: true,
        repeat: false, modifiers: egui::Modifiers::NONE,
    }]);
    assert_eq!(app.generic_save_target(), document_save::Pane::Model);
    app.show_plot_pane();
    click_label(&mut app, &ctx, "Menu");
    click_label(&mut app, &ctx, "File");
    for key in [egui::Key::ArrowDown, egui::Key::Escape] {
        frame(&mut app, &ctx, vec![egui::Event::Key {
            key, physical_key: None, pressed: true, repeat: false,
            modifiers: egui::Modifiers::NONE,
        }]);
    }
    app.document_save.intercepted = Some(Vec::new());
    document_save_key(&mut app, &ctx, size, false);
    assert_eq!(app.document_save.intercepted.as_deref(), Some(&[
        document_save::Request::Figure { figure: 1, save_as: false },
    ][..]));
    assert!(app.toolkits.model_dirty() && app.toolkits.model_open);
}

#[test]
fn model_document_documents_tab_takes_save_back_from_retained_model_focus() {
    // Docked tools never overlap the figure; leaving the model tab is the analog
    // of pressing the exposed figure beside the former floating model window.
    let ctx = egui::Context::default();
    let dir = tempfile::tempdir().unwrap();
    let (mut app, _, _) = model_document_app(&ctx, dir.path());
    let size = egui::vec2(1440., 900.);
    model_document_canvas(&mut app, &ctx, size);
    ctx.memory_mut(|memory| memory.request_focus(egui::Id::new("model_diagram_keyboard")));
    frame(&mut app, &ctx, vec![]);
    assert_eq!(app.generic_save_target(), document_save::Pane::Model);
    assert_eq!(ctx.memory(|memory| memory.focused()),
        Some(egui::Id::new("model_diagram_keyboard")));
    // Legacy layout: one documents tab sits before the tool tabs.
    click_label_at(&mut app, &ctx, size, "Editor and figure");
    assert!(app.toolkits.selected_tool().is_none() && app.toolkits.model_open);
    frame_at(&mut app, &ctx, size, vec![]);
    assert!(app.toolkits.model_document_region().is_none());
    assert_ne!(ctx.memory(|memory| memory.focused()),
        Some(egui::Id::new("model_diagram_keyboard")));
    app.document_save.intercepted = Some(Vec::new());
    document_save_key(&mut app, &ctx, size, false);
    assert_eq!(app.document_save.intercepted.as_deref(), Some(&[
        document_save::Request::Figure { figure: 1, save_as: false },
    ][..]));
    assert!(app.toolkits.model_open && app.toolkits.model_dirty());
}

#[test]
fn model_and_designer_tabs_leave_actual_toolbar_file_menu_accessible() {
    for size in [egui::vec2(1440.,900.),egui::vec2(960.,650.),egui::vec2(640.,480.)] {
        for designer in [false,true] {
            let ctx=egui::Context::default();
            let dir=tempfile::tempdir().unwrap();
            let (mut app,path,script)=model_document_app(&ctx,dir.path());
            let model_bytes=std::fs::read(&path).unwrap();
            if designer { app.toolkits.hide_model_view(); app.toolkits.open_tool(toolkits::Tool::Designer); app.show_plot_pane(); app.select_tool_tab(toolkits::Tool::Designer); }
            else { app.show_model_document(); }
            let expected=if designer {document_save::Pane::Figure} else {document_save::Pane::Model};
            for (size,collapsed) in [(size,false),(size,true),(egui::vec2(960.,650.),true),(size,false)] {
                app.layout.toolstrip_collapsed=collapsed;
                click_label_at(&mut app,&ctx,size,"Menu");
                let point=document_save_heading(&mut app,&ctx,size,"File");
                let tab=if designer {app.toolkits.designer_region()} else {app.toolkits.model_document_region().map(|(rect,_)|rect)}.unwrap();
                assert!(!tab.contains(point) && ctx.content_rect().contains_rect(tab));
                assert!(ctx.memory(|m|m.area_rect(egui::Id::new(if designer {"Filter Designer"} else {"block_simulation"}))).is_none());
                assert_eq!(app.generic_save_target(),expected);
                for pressed in [true,false] {pointer_frame(&mut app,&ctx,size,point,pressed);}
                assert!(egui::Popup::is_any_open(&ctx));
                assert_eq!(app.generic_save_target(),expected);
                let command=if designer {"Save figure"} else {"Save model"};
                document_save_heading(&mut app,&ctx,size,command);
                assert_eq!(app.generic_save_target(),expected);
                frame_at(&mut app,&ctx,size,vec![egui::Event::Key {key:egui::Key::Escape,physical_key:None,pressed:true,repeat:false,modifiers:egui::Modifiers::NONE}]);
            }
            assert_eq!(std::fs::read(&path).unwrap(),model_bytes);
            assert_eq!(std::fs::read_to_string(script).unwrap(),"old=1;\n");
            assert!(app.documents[0].dirty && app.figure_dirty(1));
        }
    }
}
