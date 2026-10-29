const SLX_CHAIN: &[u8] = include_bytes!("../../tests/fixtures/slx/slx-inherited-rate-admission-1526/chain.slx");

fn slx_model_frame(app: &mut Workspace, ctx: &egui::Context, events: Vec<egui::Event>) -> egui::FullOutput {
    crate::headless::run(ctx, egui::RawInput { screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(1100.,780.))), events, ..Default::default() }, |root| {
        let _ = app.toolkits.ui(root.ctx(), app.busy);
        if let Some(theme)=app.layout.theme {
            assert_eq!(root.ctx().global_style().visuals.dark_mode,theme==session::DesktopTheme::Dark);
        }
    })
}
fn slx_model_click(app:&mut Workspace,ctx:&egui::Context,label:&str) {
    fn text(shape:&egui::Shape,label:&str,clip:egui::Rect)->Option<egui::Pos2>{
        match shape {
            egui::Shape::Text(t) if t.galley.text()==label => {
                let ink=t.visual_bounding_rect();
                clip.contains_rect(ink).then_some(ink.center())
            }
            egui::Shape::Vec(v)=>v.iter().find_map(|s|text(s,label,clip)), _=>None,
        }
    }
    let mut hit=None;
    for _ in 0..8 {
        let frame=slx_model_frame(app,ctx,vec![]);
        hit=frame.shapes.iter().find_map(|s|text(&s.shape,label,s.clip_rect));
        if hit.is_some(){break;}
    }
    let pos=hit.unwrap_or_else(||panic!("visible native button {label}"));
    for pressed in [true,false] {
        slx_model_frame(app,ctx,vec![egui::Event::PointerMoved(pos),egui::Event::PointerButton {pos,button:egui::PointerButton::Primary,pressed,modifiers:egui::Modifiers::NONE}]);
    }
}

#[test]
fn slx_actual_worker_unsaved_save_as_preserves_source_workspace_and_recovery_model(){
    let dir=tempfile::tempdir().unwrap();let path=dir.path().join("source.slx");std::fs::write(&path,SLX_CHAIN).unwrap();
    let ctx=egui::Context::default();let mut app=legacy_sample_desktop(ctx,true);
    app.run("data=[1 2 3];plot(data);".into());finish(&mut app);
    let data=app.variables["data"].clone();let figure=app.figure.as_ref().unwrap().clone();
    app.open_path(path.clone());finish(&mut app);
    assert_eq!(app.toolkits.model.steps,5);assert!(app.toolkits.model_dirty());
    let (file,baseline)=app.toolkits.model_metadata();assert!(file.is_none() && baseline.is_none());
    assert!(data.same_snapshot(&app.variables["data"]));assert!(figure.same_snapshot(app.figure.as_ref().unwrap()));
    let imported=app.toolkits.model.clone();
    // Recovery stores native data; a source removed after import is not reinterpreted.
    std::fs::remove_file(&path).unwrap();
    app.toolkits.restore_model(imported.clone(),file,baseline);
    assert!(app.toolkits.model.same_storage(&imported));assert!(app.toolkits.model_dirty());
    let output=dir.path().join("saved.mrsim");assert!(!app.toolkits.save_model_path(&output,None));app.advance_model_read();finish(&mut app);
    assert!(matrust::simulation::Model::load(&output).unwrap().same_storage(&imported));assert!(!app.toolkits.model_dirty());
    assert!(!path.exists());std::fs::write(&path,SLX_CHAIN).unwrap();
    let before=std::fs::read(&path).unwrap();assert!(!app.toolkits.save_model_path(&path,None));app.advance_model_read();finish(&mut app);
    assert_eq!(std::fs::read(path).unwrap(),before);assert!(app.status.contains(".mrsim"));
}

#[test]
fn slx_native_dirty_open_cancel_and_discard_use_actual_visible_buttons_in_both_themes(){
    let dir=tempfile::tempdir().unwrap();let path=dir.path().join("source.slx");std::fs::write(&path,SLX_CHAIN).unwrap();
    for dark in [false,true] {
        let ctx=egui::Context::default();
        let mut app=legacy_sample_desktop(ctx.clone(),true);
        app.layout.theme=Some(if dark{session::DesktopTheme::Dark}else{session::DesktopTheme::Light});
        app.apply_desktop_theme(&ctx);
        assert_eq!(ctx.global_style().visuals.dark_mode,dark);
        app.toolkits.model.steps=123;
        app.open_path(path.clone());finish(&mut app);
        assert_eq!(app.toolkits.model.steps,123);assert!(app.toolkits.model_file_modal_open());
        slx_model_click(&mut app,&ctx,"Cancel");assert!(!app.toolkits.model_file_modal_open());assert_eq!(app.toolkits.model.steps,123);
        app.open_path(path.clone());finish(&mut app);slx_model_click(&mut app,&ctx,"Discard changes");
        app.advance_model_read();finish(&mut app);
        assert_eq!(app.toolkits.model.steps,5);assert!(app.toolkits.model_dirty());assert!(app.toolkits.model_metadata().0.is_none());
        assert_eq!(std::fs::read(&path).unwrap(),SLX_CHAIN);
    }
}

#[test]
fn slx_stop_after_worker_completion_and_invalid_retry_never_publish_partial_model(){
    let dir=tempfile::tempdir().unwrap();let path=dir.path().join("source.slx");std::fs::write(&path,SLX_CHAIN).unwrap();
    let ctx=egui::Context::default();let mut app=legacy_sample_desktop(ctx,true);let before=app.toolkits.model.clone();
    assert!(app.toolkits.request_open(path.clone()));app.advance_model_read();
    let reply=app.worker.receive.recv_timeout(Duration::from_secs(3)).unwrap();assert!(reply.model_read.as_ref().unwrap().result.is_ok());
    app.stop_work();app.busy=false;app.receive_model_read(reply.model_read.unwrap());
    assert!(app.toolkits.model.same_storage(&before));assert!(app.status.contains("cancelled"));
    std::fs::write(&path,b"not a ZIP").unwrap();app.open_path(path.clone());finish(&mut app);
    assert!(app.toolkits.model.same_storage(&before));assert!(app.status.contains("SLX"));
    std::fs::write(&path,SLX_CHAIN).unwrap();app.open_path(path);finish(&mut app);assert_eq!(app.toolkits.model.steps,5);
}
