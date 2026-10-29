fn named_layout_ink(shape: &egui::Shape, label: &str) -> Option<egui::Rect> {
    match shape {
        egui::Shape::Text(text) if text.galley.text() == label => Some(text.visual_bounding_rect()),
        egui::Shape::Vec(shapes) => shapes.iter().rev().find_map(|shape| named_layout_ink(shape,label)),
        _ => None,
    }
}
fn named_layout_click(app: &mut Workspace, ctx: &egui::Context, size: egui::Vec2,
                      label: &str, discard: bool, late_focus: Option<egui::Id>) {
    let mut position=None;
    for _ in 0..3 {
        let out=frame_at(app,ctx,size,vec![]);
        position=out.shapes.iter().rev().find_map(|shape| {
            let rect=named_layout_ink(&shape.shape,label)?;
            shape.clip_rect.contains_rect(rect).then_some(rect.center())
        });
    }
    let pos=position.unwrap_or_else(||panic!("Missing fully visible {label} at {size:?}"));
    pointer_frame(app,ctx,size,pos,true);
    let mut pass=0;
    crate::headless::run(ctx,egui::RawInput {
        screen_rect:Some(egui::Rect::from_min_size(egui::Pos2::ZERO,size)),
        events:vec![egui::Event::PointerMoved(pos),egui::Event::PointerButton {
            pos,button:egui::PointerButton::Primary,pressed:false,modifiers:egui::Modifiers::NONE }],
        ..Default::default()
    },|root| {
        if discard && pass==0 { ctx.request_discard("named layout release must survive only this frame"); }
        app.ui(root);
        if discard && pass==0 && let Some(focus)=late_focus { ctx.memory_mut(|memory|memory.request_focus(focus)); }
        pass+=1;
    });
    if discard { assert!(pass>1); }
}
fn named_layout_type(app: &mut Workspace, ctx: &egui::Context, size: egui::Vec2, text: &str) {
    for _ in 0..3 { frame_at(app,ctx,size,vec![]); }
    ctx.memory_mut(|m|m.request_focus(egui::Id::new("named_layout_name")));
    frame_at(app,ctx,size,vec![egui::Event::Key {
        key:egui::Key::A,physical_key:None,pressed:true,repeat:false,modifiers:egui::Modifiers { ctrl:true,command:true,..egui::Modifiers::NONE } }]);
    frame_at(app,ctx,size,vec![egui::Event::Text(text.to_owned())]);
}
fn named_layout_open_action(app: &mut Workspace, ctx: &egui::Context, size: egui::Vec2,
                          action: &str, name: Option<&str>) {
    named_layout_click(app,ctx,size,"Menu",false,None);
    named_layout_click(app,ctx,size,"Layout",false,None);
    named_layout_click(app,ctx,size,action,false,None);
    if let Some(name)=name { named_layout_click(app,ctx,size,name,false,None); }
}

#[test]
fn named_layout_actual_menus_save_replace_restore_rename_delete_and_reset_preserve_work() {
    for dark in [false,true] { for size in [egui::vec2(960.,650.),egui::vec2(1440.,900.)] {
        let ctx=egui::Context::default();let mut app=Workspace::with_context(ctx.clone(),true);
        app.layout.theme=Some(if dark {session::DesktopTheme::Dark}else{session::DesktopTheme::Light});
        app.apply_desktop_theme(&ctx);
        app.run("A=[-0 2 3];B=A;plot(A,[3 2 4]);".into());finish(&mut app);
        app.documents[0].text="unsaved = 123;".into();app.documents[0].changed();app.reveal_editor();
        app.layout.workspace_dock=None;app.layout.sidebar_width=Some(300.);app.layout.workspace_width=Some(220.);
        app.layout.console_height=Some(170.);app.layout.editor_fraction=Some(0.6);
        app.select_document_layout(layout::PaneMode::Figure);
        for _ in 0..4 { frame_at(&mut app,&ctx,size,vec![]); }
        let plot_id=egui::Id::new("workspace_figure");
        let memory=egui_plot::PlotMemory::load(&ctx,plot_id).unwrap();
        let before=*memory.bounds();let pointer=memory.transform().frame().center();
        frame_at(&mut app,&ctx,size,vec![egui::Event::PointerMoved(pointer)]);
        frame_at(&mut app,&ctx,size,vec![egui::Event::PointerMoved(pointer),egui::Event::Zoom(2.)]);
        let zoomed=*egui_plot::PlotMemory::load(&ctx,plot_id).unwrap().bounds();assert!(zoomed.width()<before.width());
        let docs=serde_json::to_vec(&app.documents).unwrap();let active=app.active;
        let figure=app.figure.clone().unwrap();let variables=app.variables.clone();
        let model=app.toolkits.model.clone();let history=app.history.entries.clone();
        named_layout_open_action(&mut app,&ctx,size,"Save layout...",None);
        named_layout_type(&mut app,&ctx,size,"Analysis");
        named_layout_click(&mut app,&ctx,size,"Save layout",true,None);
        assert!(!app.named_layout.pending());assert_eq!(app.layout_presets.entries().len(),1);
        assert_eq!(ctx.global_style().visuals.dark_mode,dark);
        let saved=app.layout_presets.get("Analysis").unwrap().clone();
        app.reset_layout(&ctx);assert_eq!(app.layout_presets.get("Analysis"),Some(&saved));
        named_layout_open_action(&mut app,&ctx,size,"Restore layout",Some("Analysis"));
        assert_eq!(app.layout.sidebar_width,Some(300.));assert_eq!(app.layout.workspace_width,Some(220.));
        assert_eq!(app.navigation.left,Some(navigation::Left::Files));assert!(app.navigation.workspace);
        for _ in 0..3 { frame_at(&mut app,&ctx,size,vec![]); }
        assert_eq!(*egui_plot::PlotMemory::load(&ctx,plot_id).unwrap().bounds(),zoomed);
        named_layout_open_action(&mut app,&ctx,size,"Rename layout",Some("Analysis"));
        named_layout_type(&mut app,&ctx,size,"Review");
        frame_at(&mut app,&ctx,size,vec![egui::Event::Key { key:egui::Key::Enter,physical_key:None,pressed:true,repeat:false,modifiers:egui::Modifiers::NONE }]);
        assert!(app.layout_presets.get("Analysis").is_none());
        assert_eq!(app.layout_presets.get("Review").unwrap().layout(),saved.layout());
        app.layout.sidebar_width=Some(325.);
        named_layout_open_action(&mut app,&ctx,size,"Save layout...",None);
        named_layout_type(&mut app,&ctx,size,"Review");named_layout_click(&mut app,&ctx,size,"Replace layout",false,None);
        assert_eq!(app.layout_presets.entries().len(),1);assert_eq!(app.layout_presets.get("Review").unwrap().layout().sidebar_width,Some(325.));
        named_layout_open_action(&mut app,&ctx,size,"Delete layout",Some("Review"));assert!(app.layout_presets.is_empty());
        assert_eq!(serde_json::to_vec(&app.documents).unwrap(),docs);assert_eq!(app.active,active);
        assert_eq!(app.history.entries,history);assert!(app.toolkits.model.same_storage(&model));
        assert!(figure.same_snapshot(app.figure.as_ref().unwrap()));
        assert!(figure.series[0].points.same_storage(&app.figure.as_ref().unwrap().series[0].points));
        for (name,value) in &variables { assert!(value.same_snapshot(&app.variables[name])); }
        assert_eq!(app.variables["A"].as_numeric().unwrap().data().as_ptr(),app.variables["B"].as_numeric().unwrap().data().as_ptr());
    }}
}

#[test]
fn named_layout_split_is_reusable_after_document_reorder_close_and_never_creates_documents() {
    let ctx=egui::Context::default();let mut app=Workspace::with_context(ctx.clone(),true);
    let mut first=Document::new();first.text="first_dirty = 1;".into();first.changed();
    let mut second=Document::new();second.text="second_dirty = 2;".into();second.changed();
    app.documents=vec![first,second];app.active=1;app.reveal_editor();app.set_named_editor_split(true);
    app.layout.figure_tiling=Some(session::FigureTiling::Four);app.layout.figure_tile_size=Some(session::FigureTileSize::Readable);
    app.layout_presets.save("Two editors",app.layout,true,false,false).unwrap();
    let preset=app.layout_presets.get("Two editors").unwrap().clone();
    assert!(preset.layout().editor_columns.is_none());
    let target=app.documents[app.active].id;app.documents.swap(0,1);app.active=0;
    let before=serde_json::to_vec(&app.documents).unwrap();app.collapse_editor_columns();
    app.restore_named_layout(&ctx,&preset);
    assert_eq!(app.documents[app.active].id,target);assert_eq!(app.documents.len(),2);
    assert!(app.editor_columns_snapshot().is_some());assert_eq!(serde_json::to_vec(&app.documents).unwrap(),before);
    assert_eq!(app.layout.figure_tiling,Some(session::FigureTiling::Four));assert_eq!(app.layout.figure_tile_size,Some(session::FigureTileSize::Readable));
    app.documents.remove(1);let remaining=app.documents[0].id;app.restore_named_layout(&ctx,&preset);
    assert_eq!(app.documents.len(),1);assert_eq!(app.documents[app.active].id,remaining);
    assert!(app.editor_columns_snapshot().is_none());assert!(app.status.contains("second script"));
    let fresh=session::DesktopLayout::fresh_desktop();let mut presets=session::LayoutPresets::default();
    presets.save("Command",fresh,false,false,false).unwrap();
    app.documents[0]=Document::new();app.reveal_editor();let id=app.documents[0].id;
    app.restore_named_layout(&ctx,presets.get("Command").unwrap());
    assert_eq!(app.documents[0].id,id);assert!(!app.has_startup_placeholder());assert!(app.central_command());
}

#[test]
fn named_layout_discard_late_focus_cancel_and_close_do_not_commit_a_stale_dialog() {
    let size=egui::vec2(960.,650.);let ctx=egui::Context::default();let mut app=Workspace::with_context(ctx.clone(),true);
    for focus in [egui::Id::new("command_input"),editor::id(app.documents[0].id)] {
        named_layout_open_action(&mut app,&ctx,size,"Save layout...",None);
        named_layout_type(&mut app,&ctx,size,"Do not save");
        named_layout_click(&mut app,&ctx,size,"Save layout",true,Some(focus));
        assert!(app.layout_presets.is_empty());assert!(app.named_layout.pending());assert!(!app.busy);
        named_layout_click(&mut app,&ctx,size,"Cancel",true,None);assert!(!app.named_layout.pending());
    }
    named_layout_open_action(&mut app,&ctx,size,"Save layout...",None);
    named_layout_type(&mut app,&ctx,size,"Cancelled");
    frame_at(&mut app,&ctx,size,vec![egui::Event::Key { key:egui::Key::Escape,physical_key:None,pressed:true,repeat:false,modifiers:egui::Modifiers::NONE }]);
    assert!(!app.named_layout.pending());assert!(app.layout_presets.is_empty());
    named_layout_open_action(&mut app,&ctx,size,"Save layout...",None);
    let output=crate::headless::run(&ctx,egui::RawInput {
        screen_rect:Some(egui::Rect::from_min_size(egui::Pos2::ZERO,size)),
        viewports:std::iter::once((egui::ViewportId::ROOT,egui::ViewportInfo { events:vec![egui::ViewportEvent::Close],..Default::default() })).collect(),
        ..Default::default()
    },|root|app.ui(root));
    assert!(output.viewport_output[&egui::ViewportId::ROOT].commands.iter().any(|command| matches!(command,egui::ViewportCommand::CancelClose)));
    assert!(app.named_layout.pending());assert!(!app.pending_quit && !app.allow_close);
    app.stop_work();assert!(!app.named_layout.pending());assert!(app.layout_presets.is_empty());
}
