#[test]
fn desktop_disables_window_animation_for_light_dark_and_system_theme_selection() {
    for preference in [egui::ThemePreference::Light,egui::ThemePreference::Dark,egui::ThemePreference::System] {
        let ctx=egui::Context::default();
        ctx.set_theme(preference);
        let _app=legacy_sample_desktop(ctx.clone(),true);
        assert_eq!(ctx.options(|options|options.theme_preference),preference);
        assert_eq!(ctx.global_style().visuals.dark_mode,ctx.theme()==egui::Theme::Dark);
        assert_eq!(ctx.global_style().visuals.window_fill.a(),255);
        for theme in [egui::Theme::Light,egui::Theme::Dark] {
            assert_eq!(ctx.style_of(theme).animation_time,0.0);
        }
        // Later OS updates select the actual matching palette in System mode
        // without re-enabling animation on either stored style.
        for theme in [egui::Theme::Dark,egui::Theme::Light] {
            let _=crate::headless::run(&ctx,egui::RawInput {
                system_theme:Some(theme),..Default::default()
            },|_|{});
            assert_eq!(ctx.global_style().animation_time,0.0);
            let expected=match preference {
                egui::ThemePreference::Light=>egui::Theme::Light,
                egui::ThemePreference::Dark=>egui::Theme::Dark,
                egui::ThemePreference::System=>theme,
            };
            assert_eq!(ctx.theme(),expected);
            assert_eq!(ctx.global_style().visuals.dark_mode,expected==egui::Theme::Dark);
        }
    }
}

#[test]
fn desktop_window_body_is_opaque_within_three_early_frames_on_dark_selection() {
    fn opaque_body(shape:&egui::Shape,fill:Color32)->bool {
        match shape {
            egui::Shape::Rect(rect)=>rect.rect.width()>250. && rect.rect.height()>150.
                && rect.fill==fill && rect.fill.a()==255,
            egui::Shape::Vec(shapes)=>shapes.iter().any(|shape|opaque_body(shape,fill)),
            _=>false,
        }
    }
    let ctx=egui::Context::default();
    ctx.set_theme(egui::Theme::Dark);
    let _app=legacy_sample_desktop(ctx.clone(),true);
    let fill=ctx.style_of(egui::Theme::Dark).visuals.window_fill;
    assert_ne!(fill,Color32::WHITE);
    assert!(ctx.global_style().visuals.dark_mode);
    let mut last=None;
    for frame in 0..3 {
        last=Some(crate::headless::run(&ctx,egui::RawInput {
            screen_rect:Some(egui::Rect::from_min_size(egui::Pos2::ZERO,egui::vec2(640.,480.))),
            time:Some(f64::from(frame)/60.),..Default::default()
        },|root| {
            egui::Window::new("Window opacity regression")
                .fixed_pos(egui::pos2(20.,20.))
                .fixed_size(egui::vec2(300.,220.))
                .collapsible(false)
                .show(root.ctx(),|ui|{
                    // Window's Resize follows actual content extent; fixed_size
                    // alone does not reserve blank body height in egui 0.36.
                    ui.set_min_size(egui::vec2(280.,180.));
                    ui.label("Opaque body over existing plot content");
                });
        }));
    }
    let output=last.unwrap();
    fn rectangles(shape:&egui::Shape,output:&mut Vec<(egui::Rect,Color32)>) {
        match shape {
            egui::Shape::Rect(rect)=>output.push((rect.rect,rect.fill)),
            egui::Shape::Vec(shapes)=>for shape in shapes {rectangles(shape,output);},
            _=>{},
        }
    }
    let mut bodies=Vec::new();
    for shape in &output.shapes {rectangles(&shape.shape,&mut bodies);}
    assert!(bodies.iter().any(|(rect,_)|rect.width()>250. && rect.height()>150.),
        "Probe must render a reserved body before checking opacity: {bodies:?}");
    assert!(output.shapes.iter().any(|shape|opaque_body(&shape.shape,fill)),
        "Window fill must be opaque before a three-ready-frame screenshot: {bodies:?}");
}
