fn choose_workspace_position(app: &mut Workspace,ctx:&egui::Context,size:egui::Vec2,label:&str) {
    click_label_at(app,ctx,size,"Menu");click_label_at(app,ctx,size,"Layout");click_label_at(app,ctx,size,"Workspace position");click_label_at(app,ctx,size,label);
    if egui::Popup::is_any_open(ctx) {
        frame_at(app,ctx,size,vec![egui::Event::Key {key:egui::Key::Escape,physical_key:None,pressed:true,repeat:false,modifiers:egui::Modifiers::NONE}]);
        frame_at(app,ctx,size,vec![egui::Event::Key {key:egui::Key::Escape,physical_key:None,pressed:false,repeat:false,modifiers:egui::Modifiers::NONE}]);
    }
    for _ in 0..4 {frame_at(app,ctx,size,vec![]);}
}
fn workspace_dock_rect(ctx:&egui::Context,id:&str)->egui::Rect {
    egui::containers::panel::PanelState::load(ctx,egui::Id::new(id)).unwrap().outer_rect
}

#[test]
fn workspace_dock_actual_menu_changes_geometry_and_retains_active_figure_and_aliases() {
    for size in [egui::vec2(1440.,900.),egui::vec2(960.,650.)] {
        let ctx=egui::Context::default();let mut app=legacy_sample_desktop(ctx.clone(),true);
        app.run("kept=[1 2;3 4];plot([0 1],[1 0]);".into());finish(&mut app);app.show_plot_pane();
        app.layout.sidebar_width=Some(230.);app.layout.workspace_width=Some(220.);
        choose_workspace_position(&mut app,&ctx,size,"Right");
        let data=app.variables["kept"].clone();let figure=app.figure.clone().unwrap();let doc=app.documents[app.active].id;
        for _ in 0..4 {frame_at(&mut app,&ctx,size,vec![]);}
        let original=app.layout;let narrow=workspace_dock_rect(&ctx,"console").width();
        assert_eq!(app.generic_save_target(),document_save::Pane::Figure);
        choose_workspace_position(&mut app,&ctx,size,"Below Files");
        assert!(app.workspace_on_left());assert_eq!(app.generic_save_target(),document_save::Pane::Figure);
        assert_eq!(app.layout.workspace_width,original.workspace_width);assert_eq!(app.layout.sidebar_width,original.sidebar_width);
        let column=workspace_dock_rect(&ctx,"workspace");let lower=workspace_dock_rect(&ctx,"workspace_variables_left");
        assert!(column.contains_rect(lower));assert!(lower.height()>100. && lower.top()>column.top()+100.);
        assert!(workspace_dock_rect(&ctx,"console").width()>narrow+140.);
        assert!(egui::containers::panel::PanelState::load(&ctx,egui::Id::new("workspace_variables")).is_none());
        let output=frame_at(&mut app,&ctx,size,vec![]);
        let files=output.shapes.iter().find_map(|s|text_position(&s.shape,"Files").filter(|p|column.contains(*p))).unwrap();
        let workspace=output.shapes.iter().find_map(|s|text_position(&s.shape,"Workspace").filter(|p|column.contains(*p))).unwrap();
        assert!(files.y<lower.top());assert!(lower.contains(workspace));
        choose_workspace_position(&mut app,&ctx,size,"Right");
        assert!(!app.workspace_on_left());assert_eq!(app.layout,original);assert_eq!(app.generic_save_target(),document_save::Pane::Figure);
        assert!((workspace_dock_rect(&ctx,"workspace_variables").width()-workspace_tools::minimum_dock_width(&ctx)).abs()<2.);
        assert_eq!(app.documents[app.active].id,doc);assert!(data.same_snapshot(&app.variables["kept"]));assert!(figure.same_snapshot(app.figure.as_ref().unwrap()));
    }
}

#[test]
fn left_workspace_real_splitter_persists_fraction_without_rewriting_either_width() {
    let ctx=egui::Context::default();let mut app=legacy_sample_desktop(ctx.clone(),true);let size=egui::vec2(1440.,900.);
    app.run("kept=[1 2;3 4];".into());finish(&mut app);let data=app.variables["kept"].clone();
    app.layout.sidebar_width=Some(270.);app.layout.workspace_width=Some(250.);
    choose_workspace_position(&mut app,&ctx,size,"Below Files");
    let dir=tempfile::tempdir().unwrap();let path=dir.path().join("dock.json");
    app.recovery=Some(recovery::Recovery::open(path.clone()).unwrap());app.recovery_dirty=false;app.recovery_due=Instant::now()+Duration::from_secs(3600);
    let before=app.layout;let lower=workspace_dock_rect(&ctx,"workspace_variables_left");
    let handle=ctx.read_response(egui::Id::new("workspace_variables_left").with("__resize")).unwrap().rect;
    assert!((handle.center().y-lower.top()).abs()<1.);
    // The card follows its rows until the divider is dragged: drag it up.
    let from=handle.center();let to=from-egui::vec2(0.,70.);
    frame_at(&mut app,&ctx,size,vec![egui::Event::PointerMoved(from)]);pointer_frame(&mut app,&ctx,size,from,true);
    frame_at(&mut app,&ctx,size,vec![egui::Event::PointerMoved(to)]);pointer_frame(&mut app,&ctx,size,to,false);
    for _ in 0..3 {frame_at(&mut app,&ctx,size,vec![]);}
    let fraction=app.layout.left_workspace_fraction.expect("Actual splitter must record a preference");
    assert!(fraction>0.2 && fraction<0.8);assert!(workspace_dock_rect(&ctx,"workspace_variables_left").height()>lower.height()+50.);
    let mut expected=before;expected.left_workspace_fraction=Some(fraction);assert_eq!(app.layout,expected);assert!(app.recovery_dirty);
    assert!(data.same_snapshot(&app.variables["kept"]));
    app.recovery_due=Instant::now();frame_at(&mut app,&ctx,size,vec![]);assert!(!app.recovery_dirty);
    app.recovery.as_ref().unwrap().flush(app.snapshot()).unwrap();let saved=Session::load(&path).unwrap();assert_eq!(saved.layout,expected);
    let ctx2=egui::Context::default();let mut restored=legacy_sample_desktop(ctx2.clone(),true);restored.restore_session(saved);restored.start_session_recovery(path.clone());finish(&mut restored);
    for viewport in [size,egui::vec2(960.,650.),size] {
        for _ in 0..4 {frame_at(&mut restored,&ctx2,viewport,vec![]);}
        assert_eq!(restored.layout,expected);
        let column=workspace_dock_rect(&ctx2,"workspace");let lower=workspace_dock_rect(&ctx2,"workspace_variables_left");assert!(column.contains_rect(lower));
        assert_eq!(restored.variables["kept"].as_numeric().unwrap().data(),&[1.,3.,2.,4.]);
    }
    assert_eq!(Session::load(&path).unwrap().layout,expected);
}

#[test]
fn left_workspace_inspector_and_history_reuse_real_selection_and_modal_guards() {
    let ctx=egui::Context::default();let mut app=legacy_sample_desktop(ctx.clone(),true);let size=egui::vec2(960.,650.);
    app.run("kept=[1 2;3 4];".into());finish(&mut app);let data=app.variables["kept"].clone();let doc=app.documents[app.active].id;
    app.command="kept + 1".into();app.history.push("kept + 2".into());
    for dock in ["Below Files","Right"] {
        choose_workspace_position(&mut app,&ctx,size,dock);
        // Click the actual name field in the shared typed row; selection still uses its real Button.
        click_label_at(&mut app,&ctx,size,"kept");assert_eq!(app.selected.as_deref(),Some("kept"));
        click_label_at(&mut app,&ctx,size,"Workspace actions");click_label_at(&mut app,&ctx,size,"Inspect / edit…");assert_eq!(app.editing_variable.as_deref(),Some("kept"));
        assert!(data.same_snapshot(&app.variables["kept"]));app.editing_variable=None;
    }
    choose_workspace_position(&mut app,&ctx,size,"Below Files");
    click_label_at(&mut app,&ctx,size,"History");frame_at(&mut app,&ctx,size,vec![]);
    click_label_at(&mut app,&ctx,size,"kept + 2");assert_eq!(app.command,"kept + 2");
    app.documents[app.active].text="dirty=1;".into();app.documents[app.active].changed();app.request_script_close(app.active);assert!(app.file_action_pending());
    let layout=app.layout;app.set_workspace_dock(&ctx,session::WorkspaceDock::Right);assert_eq!(app.layout,layout,"Fresh modal guard must reject even a stale caller");
    click_label_at(&mut app,&ctx,size,"Cancel");assert!(!app.file_action_pending());assert_eq!(app.layout,layout);
    assert_eq!(app.documents[app.active].id,doc);assert!(data.same_snapshot(&app.variables["kept"]));assert_eq!(app.command,"kept + 2");
}

#[test]
fn left_workspace_splitter_extremes_survive_real_session_reopen() {
    for size in [egui::vec2(1440.,900.),egui::vec2(960.,650.)] {
        for grow in [false,true] {
            let ctx=egui::Context::default();let mut app=legacy_sample_desktop(ctx.clone(),true);
            app.run("kept=[1 2;3 4];".into());finish(&mut app);let data=app.variables["kept"].clone();
            // Start from a split with room to move both ways.
            app.layout.left_workspace_fraction=Some(0.4);
            choose_workspace_position(&mut app,&ctx,size,"Below Files");
            let before=workspace_dock_rect(&ctx,"workspace_variables_left");let column=workspace_dock_rect(&ctx,"workspace");
            let from=ctx.read_response(egui::Id::new("workspace_variables_left").with("__resize")).unwrap().rect.center();
            let to=egui::pos2(from.x,if grow {column.top()+1.} else {column.bottom()-1.});
            frame_at(&mut app,&ctx,size,vec![egui::Event::PointerMoved(from)]);pointer_frame(&mut app,&ctx,size,from,true);
            frame_at(&mut app,&ctx,size,vec![egui::Event::PointerMoved(to)]);pointer_frame(&mut app,&ctx,size,to,false);
            for _ in 0..3 {frame_at(&mut app,&ctx,size,vec![]);}
            let height=workspace_dock_rect(&ctx,"workspace_variables_left").height();
            assert!((height-before.height()).abs()>column.height()*0.1);
            assert_eq!(height>before.height(),grow);
            let saved_fraction=app.layout.left_workspace_fraction.unwrap();assert!((0.2..=0.8).contains(&saved_fraction));
            let dir=tempfile::tempdir().unwrap();let path=dir.path().join("endpoint.json");
            matrust::session::SessionStore::new(&path).save(app.snapshot()).unwrap();
            let saved=Session::load(&path).unwrap();assert_eq!(saved.layout,app.layout);
            let newctx=egui::Context::default();let mut restored=legacy_sample_desktop(newctx.clone(),true);
            restored.restore_session(saved);restored.start_session_recovery(path);finish(&mut restored);
            for _ in 0..4 {frame_at(&mut restored,&newctx,size,vec![]);}
            assert_eq!(restored.layout,app.layout);
            assert!((workspace_dock_rect(&newctx,"workspace_variables_left").height()-height).abs()<2.,"Actual endpoint height must survive reopening");
            assert!(data.same_snapshot(&app.variables["kept"]));assert_eq!(restored.variables["kept"].as_numeric().unwrap().data(),&[1.,3.,2.,4.]);
        }
    }
}

#[test]
fn fresh_left_and_explicit_right_survive_actual_typed_workspace_recovery() {
    for right in [false,true] {
        let ctx=egui::Context::default();let mut app=legacy_sample_desktop(ctx.clone(),true);
        let size=egui::vec2(960.,650.);
        assert!(app.workspace_on_left(), "Isolation must not substitute the legacy visual default");
        if right {choose_workspace_position(&mut app,&ctx,size,"Right");}
        app.run("A=[1 2;3 4];AA=A;Z=complex([1 2],[3 4]);C={'text',Z};plot([1 2],[3 4]);".into());finish(&mut app);
        let data=app.variables["A"].clone();let figure=app.figure.clone().unwrap();
        let document=app.documents[app.active].id;
        click_label_at(&mut app,&ctx,size,"A");assert_eq!(app.selected.as_deref(),Some("A"));
        assert!(data.same_snapshot(&app.variables["A"]));assert!(figure.same_snapshot(app.figure.as_ref().unwrap()));
        assert_eq!(app.documents[app.active].id,document);
        let expected=app.layout;
        let dir=tempfile::tempdir().unwrap();let path=dir.path().join("fresh.json");
        app.recovery=Some(recovery::Recovery::open(path.clone()).unwrap());
        app.recovery_dirty=true;app.recovery_due=Instant::now();
        frame_at(&mut app,&ctx,size,vec![]);assert!(!app.recovery_dirty);
        app.recovery.as_ref().unwrap().flush(app.snapshot()).unwrap();
        let saved=Session::load(&path).unwrap();assert_eq!(saved.layout,expected);
        let newctx=egui::Context::default();let mut restored=legacy_sample_desktop(newctx.clone(),true);
        assert!(restored.workspace_on_left());restored.restore_session(saved);restored.start_session_recovery(path);finish(&mut restored);
        for _ in 0..4 {frame_at(&mut restored,&newctx,size,vec![]);}
        assert_eq!(restored.layout,expected);assert_eq!(restored.workspace_on_left(),!right);
        let panel=if right {"workspace_variables"} else {"workspace_variables_left"};
        assert!(workspace_dock_rect(&newctx,panel).height()>100.);
        assert!(restored.variables["A"].same_snapshot(&restored.variables["AA"]));
        assert_eq!(restored.variables["A"].as_numeric().unwrap().data(),&[1.,3.,2.,4.]);
        assert!(matches!(restored.variables["Z"],Array::Complex(_)));
        assert!(matches!(restored.variables["C"],Array::Cell(_)));
        assert_eq!(restored.figure.as_ref().unwrap().svg(),figure.svg());
        assert_eq!(Session::load(&dir.path().join("fresh.json")).unwrap().layout,expected);
    }
}

#[test]
fn valid_legacy_sessions_override_fresh_left_in_real_recovery_geometry() {
    for size in [egui::vec2(1440.,900.),egui::vec2(960.,650.)] {
        for case in 0..5 {
            let ctx=egui::Context::default();let mut app=legacy_sample_desktop(ctx.clone(),true);
            assert!(app.workspace_on_left());
            app.documents[0].text="unsaved_legacy_work=42;".into();app.documents[0].changed();
            let mut value=serde_json::to_value(app.snapshot()).unwrap();
            if case==0 {value.as_object_mut().unwrap().remove("layout");}
            else {
                value["layout"]=serde_json::json!({"sidebar_width":230,"workspace_width":220,"theme":"dark"});
                match case {
                    1=>{},
                    2=>value["layout"]["workspace_dock"]=serde_json::json!("right"),
                    3=>value["layout"]["workspace_dock"]=serde_json::json!({"future":true}),
                    4=>value["layout"]["workspace_dock"]=serde_json::json!("below_files"),
                    _=>unreachable!(),
                }
            }
            let dir=tempfile::tempdir().unwrap();let path=dir.path().join("legacy.json");
            std::fs::write(&path,serde_json::to_vec(&value).unwrap()).unwrap();
            let saved=Session::load(&path).unwrap();let expected=saved.layout;
            app.restore_session(saved);
            for _ in 0..4 {frame_at(&mut app,&ctx,size,vec![]);}
            assert_eq!(app.layout,expected);assert_eq!(app.workspace_on_left(),case==4);
            let panel=if case==4 {"workspace_variables_left"} else {"workspace_variables"};
            assert!(workspace_dock_rect(&ctx,panel).height()>100.);
            assert_eq!(app.documents[0].text,"unsaved_legacy_work=42;");assert!(app.documents[0].dirty);
            if case!=0 {assert_eq!(app.layout.workspace_width,Some(220.));assert_eq!(app.layout.sidebar_width,Some(230.));assert_eq!(app.layout.theme,Some(session::DesktopTheme::Dark));}
        }
    }
}

#[test]
fn real_reset_menu_selects_fresh_left_preserves_theme_and_checkpoints_it() {
    for size in [egui::vec2(1440.,900.),egui::vec2(960.,650.)] {
        let ctx=egui::Context::default();let mut app=legacy_sample_desktop(ctx.clone(),true);
        app.run("A=[1 2];AA=A;plot(A,A);".into());finish(&mut app);
        choose_workspace_position(&mut app,&ctx,size,"Right");
        app.layout.sidebar_width=Some(240.);app.layout.workspace_width=Some(230.);
        app.layout.left_workspace_fraction=Some(0.7);app.layout.console_height=Some(150.);
        app.layout.theme=Some(session::DesktopTheme::Dark);app.set_desktop_scale(&ctx,125);
        let data=app.variables["A"].clone();let figure=app.figure.clone().unwrap();let doc=app.documents[app.active].id;
        let dir=tempfile::tempdir().unwrap();let path=dir.path().join("reset.json");
        app.recovery=Some(recovery::Recovery::open(path.clone()).unwrap());
        app.recovery_due=Instant::now()+Duration::from_secs(3600);app.recovery_dirty=false;
        scale_click(&mut app,&ctx,size,"Menu");
        scale_click(&mut app,&ctx,size,"Layout");scale_click(&mut app,&ctx,size,"Reset layout");
        for _ in 0..4 {scale_frame(&mut app,&ctx,size,vec![]);}
        let expected=session::DesktopLayout {theme:Some(session::DesktopTheme::Dark),center:Some(session::CenterLayout {command:true,editor_revealed:true}),..session::DesktopLayout::fresh_desktop()};
        assert_eq!(app.layout,expected);assert_eq!(ctx.zoom_factor(),1.);assert!(app.recovery_dirty);
        let column=workspace_dock_rect(&ctx,"workspace");let lower=workspace_dock_rect(&ctx,"workspace_variables_left");
        assert!(column.contains_rect(lower)&&lower.top()>column.top()+100.&&lower.height()>100.);
        assert!(egui::containers::panel::PanelState::load(&ctx,egui::Id::new("workspace_variables")).is_none());
        assert_eq!(app.documents[app.active].id,doc);assert!(data.same_snapshot(&app.variables["A"]));
        assert!(figure.same_snapshot(app.figure.as_ref().unwrap()));
        app.recovery_due=Instant::now();frame_at(&mut app,&ctx,size,vec![]);assert!(!app.recovery_dirty);
        app.recovery.as_ref().unwrap().flush(app.snapshot()).unwrap();
        let saved=Session::load(&path).unwrap();assert_eq!(saved.layout,expected);
        let newctx=egui::Context::default();let mut restored=legacy_sample_desktop(newctx.clone(),true);
        restored.restore_session(saved);restored.start_session_recovery(path);finish(&mut restored);
        for _ in 0..4 {frame_at(&mut restored,&newctx,size,vec![]);}
        assert_eq!(restored.layout,expected);assert!(restored.workspace_on_left());
        assert!(workspace_dock_rect(&newctx,"workspace").contains_rect(workspace_dock_rect(&newctx,"workspace_variables_left")));
        assert!(restored.variables["A"].same_snapshot(&restored.variables["AA"]));
        assert_eq!(restored.figure.as_ref().unwrap().svg(),figure.svg());
    }
}

include!("workspace_value_column_tests.rs");

#[test]
fn selected_value_uses_the_engines_display_format_and_caps_lines() {
    let mut engine = matrust::Engine::default();
    engine.execute("m=magic(4); x=0.8147; t=(1:30)'*ones(1,12);").unwrap();
    let vars = engine.workspace();
    let short = matrust::display_format::DisplayFormat::default();
    let text = workspace_dock::selected_value_display(&vars["m"], &short);
    assert_eq!(text.lines().next().unwrap().split_whitespace().collect::<Vec<_>>(), ["16", "2", "3", "13"]);
    assert!(!text.contains("16.0"), "{text}");
    assert_eq!(workspace_dock::selected_value_display(&vars["x"], &short), "    0.8147");
    let tall = workspace_dock::selected_value_display(&vars["t"], &short);
    assert_eq!(tall.lines().count(), 20, "{tall}");
    assert_eq!(tall.lines().last(), Some("... (30 x 12)"));
    // The panel follows `format long` set in the Command Window.
    let ctx = egui::Context::default();
    let mut app = Workspace::with_context(ctx.clone(), true);
    app.run("format long; y=pi;".into());
    finish(&mut app);
    assert!(app.display_format.long);
    let y = app.variables["y"].clone();
    assert!(app.selected_value_text(&y).contains("3.141592653589793"));
    app.run("format short;".into());
    finish(&mut app);
    assert_eq!(&*app.selected_value_text(&y), "    3.1416");
}

#[test]
fn selected_value_cache_is_dropped_and_the_line_cap_holds_for_block_path() {
    let mut engine = matrust::Engine::default();
    engine.execute("big=ones(100001,1);").unwrap();
    let vars = engine.workspace();
    let compact = matrust::display_format::DisplayFormat { compact: true, ..Default::default() };
    let text = workspace_dock::selected_value_display(&vars["big"], &compact);
    assert!(text.lines().count() <= 20, "{}", text.lines().count());
    assert_eq!(text.lines().last(), Some("... (100001 x 1)"));
    let ctx = egui::Context::default();
    let mut app = Workspace::with_context(ctx.clone(), true);
    app.run("y=magic(4);".into());
    finish(&mut app);
    app.selected = Some("y".into());
    let y = app.variables["y"].clone();
    app.selected_value_text(&y);
    assert!(app.selected_display.is_some());
    app.run("clear y".into());
    finish(&mut app);
    for _ in 0..3 { frame_at(&mut app, &ctx, egui::vec2(1440., 900.), vec![]); }
    assert!(app.selected_display.is_none(), "cache kept the cleared array alive");
}
