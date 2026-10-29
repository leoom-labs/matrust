fn preview_rect(ctx: &egui::Context) -> egui::Rect {
    ctx.memory(|m| m.area_rect(egui::Id::new("plot_image_preview_window")).unwrap())
}

#[test]
fn preview_navigation_scroll_drag_resize_and_restart_preserve_view_without_copying_pixels() {
    let ctx = egui::Context::default();
    let mut app = legacy_sample_desktop(ctx.clone(),true);
    let source = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/preview/tiff-preview/reference-1.png");
    app.submit(Job::OpenImage(source)); finish(&mut app);
    for _ in 0..3 { frame(&mut app,&ctx,vec![]); }
    let view=app.image_view.as_mut().unwrap(); view.fit=false; view.zoom=4.;
    let pixels=view.pixels.clone();
    frame(&mut app,&ctx,vec![]);
    let rect=preview_rect(&ctx);
    let mouse=rect.max-egui::vec2(80.,80.);
    frame(&mut app,&ctx,vec![egui::Event::PointerMoved(mouse),egui::Event::MouseWheel {
                    phase: egui::TouchPhase::Move,
        unit:egui::MouseWheelUnit::Point,delta:egui::vec2(-80.,-120.),modifiers:egui::Modifiers::NONE,
    }]);
    for _ in 0..40 { frame(&mut app,&ctx,vec![]); }
    let scroll=app.image_view.as_ref().unwrap().navigation.scroll;
    assert!(scroll[0]>20. && scroll[1]>20.,"scroll={scroll:?}");
    let from=preview_rect(&ctx).min+egui::vec2(150.,16.);
    let to=from+egui::vec2(40.,20.);
    pointer_frame(&mut app,&ctx,egui::vec2(1440.,900.),from,true);
    frame(&mut app,&ctx,vec![egui::Event::PointerMoved(to)]);
    pointer_frame(&mut app,&ctx,egui::vec2(1440.,900.),to,false);
    frame(&mut app,&ctx,vec![]);
    assert!((preview_rect(&ctx).min.x-rect.min.x-40.).abs()<2.);
    let corner=preview_rect(&ctx).max-egui::vec2(3.,3.);
    pointer_frame(&mut app,&ctx,egui::vec2(1440.,900.),corner,true);
    frame(&mut app,&ctx,vec![egui::Event::PointerMoved(corner+egui::vec2(50.,20.))]);
    pointer_frame(&mut app,&ctx,egui::vec2(1440.,900.),corner+egui::vec2(50.,20.),false);
    for _ in 0..3 { frame(&mut app,&ctx,vec![]); }
    assert!(preview_rect(&ctx).width()>rect.width()+20.);
    assert!(Arc::ptr_eq(&pixels,&app.image_view.as_ref().unwrap().pixels));
    let expected=app.image_view.as_ref().unwrap().navigation;
    let dir=tempfile::tempdir().unwrap(); let file=dir.path().join("session.json");
    app.recovery=Some(recovery::Recovery::open(file.clone()).unwrap());
    eframe::App::on_exit(&mut app,None); drop(app);
    let ctx2=egui::Context::default();
    let mut restored=legacy_sample_desktop(ctx2.clone(),true);
    restored.restore_session(Session::load(&file).unwrap());
    frame(&mut restored,&ctx2,vec![]); finish(&mut restored);
    for _ in 0..4 { frame(&mut restored,&ctx2,vec![]); }
    let actual=restored.image_view.as_ref().unwrap().navigation;
    restored.recovery_dirty=false;
    for _ in 0..3 { frame(&mut restored,&ctx2,vec![]); }
    assert!(!restored.recovery_dirty,"An unchanged preview must not request repeated checkpoints");
    for (a,b) in actual.scroll.into_iter().zip(expected.scroll) { assert!((a-b).abs()<2.,"{actual:?} vs {expected:?}"); }
    let (a,b)=(actual.window.unwrap(),expected.window.unwrap());
    for (a,b) in a.position.into_iter().chain(a.size).zip(b.position.into_iter().chain(b.size)) {
        assert!((a-b).abs()<2.,"window {actual:?} vs {expected:?}");
    }
    for _ in 0..4 { frame_at(&mut restored,&ctx2,egui::vec2(960.,650.),vec![]); }
    let rect=preview_rect(&ctx2);
    assert!(rect.left()>=0. && rect.top()>=0. && rect.right()<=960. && rect.bottom()<=650.,"{rect:?}");
    click_label_at(&mut restored,&ctx2,egui::vec2(960.,650.),"Fit image");
    assert_eq!(restored.image_view.as_ref().unwrap().navigation.scroll,[0.;2]);
}

#[test]
fn preview_navigation_reload_retains_offsets_but_page_and_source_changes_reset_them() {
    let ctx=egui::Context::default(); let mut app=legacy_sample_desktop(ctx.clone(),true);
    let source=std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/preview/tiff-preview/pages.tiff");
    app.submit(Job::OpenPagedImage(source,0)); finish(&mut app);
    for _ in 0..3 { frame(&mut app,&ctx,vec![]); }
    let view=app.image_view.as_mut().unwrap(); view.fit=false; view.zoom=8.; view.navigation.scroll=[120.,160.];
    frame(&mut app,&ctx,vec![]);
    click_label(&mut app,&ctx,"Reload image"); finish(&mut app); frame(&mut app,&ctx,vec![]);
    assert_eq!(app.image_view.as_ref().unwrap().navigation.scroll,[120.,160.]);
    click_label(&mut app,&ctx,"Next page"); finish(&mut app); frame(&mut app,&ctx,vec![]);
    assert_eq!(app.image_view.as_ref().unwrap().navigation.scroll,[0.;2]);
    app.image_view.as_mut().unwrap().navigation.scroll=[32768.;2];
    frame(&mut app,&ctx,vec![]);
    assert!(app.image_view.as_ref().unwrap().navigation.scroll.iter().all(|v| *v<32768.));
    app.submit(Job::OpenImage(std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/preview/tiff-preview/reference-1.png")));
    finish(&mut app); frame(&mut app,&ctx,vec![]);
    assert_eq!(app.image_view.as_ref().unwrap().navigation.scroll,[0.;2]);
}
