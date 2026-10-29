#[test]
fn native_shell_empty_editor_and_docked_panels_fit_wide_and_compact() {
    for size in [egui::vec2(1440.,900.),egui::vec2(960.,650.)] {
        let ctx=egui::Context::default();let mut app=legacy_sample_desktop(ctx.clone(),true);
        assert!(app.figure.is_none());
        let mut output=frame_at(&mut app,&ctx,size,vec![]);
        for _ in 0..3 {output=frame_at(&mut app,&ctx,size,vec![]);}
        let panel=|name|egui::containers::panel::PanelState::load(&ctx,egui::Id::new(name)).unwrap().outer_rect;
        let left=panel("workspace");let lower=panel("workspace_variables_left");let console=panel("console");let toolbar=panel("toolbar");
        assert!(app.workspace_on_left());
        assert!(left.contains_rect(lower) && lower.top()>left.top()+100. && lower.height()>100.);
        assert!(console.left()>=left.right()-1. && console.width()>size.x*0.5);
        assert!(egui::containers::panel::PanelState::load(&ctx,egui::Id::new("workspace_variables")).is_none());
        assert!(toolbar.height()<=tokens::TOP_BAR + panels::STRIP_TILE + 3. * tokens::S,"{toolbar:?}");
        assert!(egui::containers::panel::PanelState::load(&ctx,egui::Id::new("editor_split")).is_none());
        let find=|label|output.shapes.iter().rev().find_map(|s|text_position(&s.shape,label));
        assert!(find("Editor").is_some());assert!(find("Workspace").is_some());assert!(find("Files").is_some());
        assert!(find("Figure").is_none());assert!(find("Damped oscillation").is_none());
        assert_eq!(app.layout,legacy_sample_layout());
    }
}

#[test]
fn native_ribbon_actions_and_collapsed_examples_keep_real_workflows() {
    let ctx=egui::Context::default();let mut app=legacy_sample_desktop(ctx.clone(),true);
    let count=app.documents.len();click_label(&mut app,&ctx,"New script");assert_eq!(app.documents.len(),count+1);
    app.documents[app.active].text="A=[1 2];plot(A,A);".into();
    click_label(&mut app,&ctx,"Run");finish(&mut app);
    assert_eq!(app.variables["A"].as_numeric().unwrap().data(),&[1.,2.]);
    let original=app.variables["A"].clone();let figure=app.figure.clone().unwrap();
    for _ in 0..3 {frame_at(&mut app,&ctx,egui::vec2(1440.,900.),vec![]);}
    assert!(egui::containers::panel::PanelState::load(&ctx,egui::Id::new("editor_split")).is_some());
    click_label(&mut app,&ctx,"Apps");click_label(&mut app,&ctx,"Examples");
    click_label(&mut app,&ctx,"Damped oscillation");
    assert_eq!(app.documents.len(),count+2);assert!(original.same_snapshot(&app.variables["A"]));assert!(figure.same_snapshot(app.figure.as_ref().unwrap()));
}

#[test]
fn right_workspace_width_is_bounded_and_optional_in_session_preferences() {
    let mut layout=session::DesktopLayout {workspace_width:Some(240.),..Default::default()};
    let json=serde_json::to_string(&layout).unwrap();assert_eq!(serde_json::from_str::<session::DesktopLayout>(&json).unwrap(),layout);
    layout.workspace_width=Some(f32::INFINITY);layout.normalize();assert_eq!(layout.workspace_width,None);
    let old:session::DesktopLayout=serde_json::from_str("{}").unwrap();assert_eq!(old.workspace_width,None);
}

#[test]
fn right_workspace_drag_persists_independently_and_viewport_clamps_without_rewriting() {
    fn panel(ctx: &egui::Context, name: &str) -> egui::Rect {
        egui::containers::panel::PanelState::load(ctx, egui::Id::new(name))
            .unwrap()
            .outer_rect
    }
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("session.json");
    let size = egui::vec2(1440., 900.);
    let compact = egui::vec2(960., 650.);
    let ctx = egui::Context::default();
    let mut app = legacy_sample_desktop(ctx.clone(), true);
    choose_workspace_position(&mut app, &ctx, size, "Right");
    app.run("A=[1 2];plot(A,A);".into());
    finish(&mut app);
    app.layout.sidebar_width = Some(230.);
    app.layout.console_height = Some(160.);
    app.layout.editor_fraction = Some(0.55);
    app.layout.workspace_width = Some(220.);
    app.recovery = Some(recovery::Recovery::open(path.clone()).unwrap());
    for _ in 0..3 { frame_at(&mut app, &ctx, size, vec![]); }
    let before = app.layout;
    let samples = app.variables["A"].clone();
    let figure = app.figure.clone().unwrap();
    app.recovery_dirty = false;
    // Keep automatic autosave out of the gesture assertions on loaded hosts.
    app.recovery_due = Instant::now() + Duration::from_secs(3600);

    let visible = panel(&ctx, "workspace_variables");
    let handle = ctx.read_response(egui::Id::new("workspace_variables").with("__resize"))
        .expect("The visible right panel must have a real resize handle").rect;
    assert!((handle.center().x - visible.left()).abs() < 1., "{handle:?} vs {visible:?}");
    let from = handle.center();
    let to = from - egui::vec2(80., 0.);
    frame_at(&mut app, &ctx, size, vec![egui::Event::PointerMoved(from)]);
    pointer_frame(&mut app, &ctx, size, from, true);
    frame_at(&mut app, &ctx, size, vec![egui::Event::PointerMoved(to)]);
    pointer_frame(&mut app, &ctx, size, to, false);
    for _ in 0..3 { frame_at(&mut app, &ctx, size, vec![]); }
    let width = app.layout.workspace_width.expect("Actual right-pane drag must capture preference");
    assert!(width > visible.width() + 60., "{width} vs {visible:?}");
    assert!((panel(&ctx, "workspace_variables").width() - width).abs() < 2.);
    let mut expected = before;
    expected.workspace_width = Some(width);
    assert_eq!(app.layout, expected, "Only the right Workspace preference may change");
    assert!(app.recovery_dirty);
    assert!(samples.same_snapshot(&app.variables["A"]));
    assert!(figure.same_snapshot(app.figure.as_ref().unwrap()));

    app.recovery_due = Instant::now();
    frame_at(&mut app, &ctx, size, vec![]);
    assert!(!app.recovery_dirty, "Resize must enter the actual recovery queue");
    app.recovery.as_ref().unwrap().flush(app.snapshot()).unwrap();
    let saved = Session::load(&path).unwrap();
    assert_eq!(saved.layout, expected);
    let new_ctx = egui::Context::default();
    let mut reopened = legacy_sample_desktop(new_ctx.clone(), true);
    reopened.restore_session(saved);
    for _ in 0..3 { frame_at(&mut reopened, &new_ctx, size, vec![]); }
    assert_eq!(reopened.layout, expected);
    assert!((panel(&new_ctx, "workspace_variables").width() - width).abs() < 2.);

    // Same-context viewport changes must clamp display size, not saved intent.
    for _ in 0..3 { frame_at(&mut reopened, &new_ctx, compact, vec![]); }
    let small = panel(&new_ctx, "workspace_variables");
    let maximum = (compact.x * 0.25).max(workspace_tools::minimum_dock_width(&new_ctx));
    assert!(small.width() <= maximum + 1., "{small:?}");
    assert!(small.width() < width - 30.);
    assert_eq!(reopened.layout, expected);
    for _ in 0..3 { frame_at(&mut reopened, &new_ctx, size, vec![]); }
    assert!((panel(&new_ctx, "workspace_variables").width() - width).abs() < 2.);
    assert_eq!(reopened.layout, expected);
    assert_eq!(Session::load(&path).unwrap().layout, expected);
}
