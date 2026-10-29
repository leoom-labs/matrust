use super::*;

fn preview(size: [usize; 2]) -> image_load::Preview {
    image_load::Preview {
        original: None,
        path: std::env::temp_dir().join("centering-preview.png"),
        original_size: size.map(|n| n as u32),
        format: "PNG",
        file_bytes: 123,
        pixels: egui::ColorImage::filled(size, egui::Color32::RED),
    }
}

fn texture_rect(output: &egui::FullOutput, id: egui::TextureId) -> (egui::Rect, egui::Rect) {
    fn find(shape: &egui::Shape, id: egui::TextureId) -> Option<egui::Rect> {
        match shape {
            // egui paints an unrotated Image as a textured RectShape before
            // tessellation; only rotated images take its raw Mesh branch.
            egui::Shape::Rect(rect) => {
                let brush = rect.brush.as_ref()?;
                if brush.fill_texture_id != id {
                    return None;
                }
                assert!(rect.rect.min.is_finite() && rect.rect.max.is_finite());
                assert!(rect.rect.width() > 0. && rect.rect.height() > 0.);
                assert_eq!(rect.corner_radius, egui::CornerRadius::ZERO);
                assert_eq!(rect.stroke, egui::Stroke::NONE);
                assert_eq!(rect.angle, 0.);
                assert_eq!(rect.blur_width, 0.);
                assert_eq!(
                    brush.uv,
                    egui::Rect::from_min_max(egui::pos2(0., 0.), egui::pos2(1., 1.))
                );
                Some(rect.rect)
            }
            egui::Shape::Mesh(mesh) if mesh.texture_id == id => {
                assert_eq!(mesh.vertices.len(), 4);
                assert_eq!(mesh.indices.len(), 6);
                assert!(mesh.vertices.iter().all(|v| v.pos.is_finite()));
                let uv = mesh.vertices.iter().fold(egui::Rect::NOTHING, |r, v| {
                    r.union(egui::Rect::from_min_max(v.uv, v.uv))
                });
                assert_eq!(
                    uv,
                    egui::Rect::from_min_max(egui::pos2(0., 0.), egui::pos2(1., 1.))
                );
                Some(mesh.calc_bounds())
            }
            egui::Shape::Vec(shapes) => shapes.iter().find_map(|s| find(s, id)),
            _ => None,
        }
    }
    let matches: Vec<_> = output
        .shapes
        .iter()
        .filter_map(|s| find(&s.shape, id).map(|r| (r, s.clip_rect)))
        .collect();
    assert_eq!(matches.len(), 1, "one actual image texture quad");
    matches[0]
}

fn canvas_frame(
    ctx: &egui::Context,
    viewer: &mut Viewer,
    size: egui::Vec2,
) -> (egui::Rect, egui::Rect, egui::Rect) {
    let canvas = egui::Rect::from_min_size(egui::pos2(30., 40.), size);
    let mut visible = egui::Rect::NOTHING;
    let output = crate::headless::run(
        ctx,
        egui::RawInput {
            screen_rect: Some(egui::Rect::from_min_size(
                egui::Pos2::ZERO,
                egui::vec2(1440., 900.),
            )),
            ..Default::default()
        },
        |root| {
            let mut ui = root.new_child(
                egui::UiBuilder::new()
                    .id_salt("preview-test")
                    .max_rect(canvas),
            );
            ui.set_clip_rect(canvas);
            visible = ui.available_rect_before_wrap().intersect(ui.clip_rect());
            show_pixels(&mut ui, viewer);
            if viewer.fit {
                assert!(
                    ui.cursor().top() >= visible.bottom(),
                    "fit reserves the full finite canvas"
                );
            }
        },
    );
    let (image, clip) = texture_rect(&output, viewer.texture.id());
    (image, clip, visible)
}

fn near(a: f32, b: f32) {
    assert!((a - b).abs() <= 0.05, "{a} != {b}");
}

#[test]
fn fitted_texture_is_centered_in_finite_canvas_without_upscaling_or_pixel_changes() {
    for theme in [egui::Theme::Light, egui::Theme::Dark] {
        let ctx = egui::Context::default();
        ctx.set_theme(theme);
        assert_eq!(
            ctx.global_style().visuals.dark_mode,
            theme == egui::Theme::Dark
        );
        for size in [[1024, 1024], [1024, 512], [512, 1024], [32, 16]] {
            let mut viewer = Viewer::new(&ctx, preview(size), None);
            let pixels = viewer.pixels.clone();
            let texture = viewer.texture.id();
            for canvas in [
                egui::vec2(806., 410.),
                egui::vec2(346., 210.),
                egui::vec2(220., 470.),
            ] {
                viewer.navigation.scroll = [17., 29.];
                let (image, clip, visible) = canvas_frame(&ctx, &mut viewer, canvas);
                near(image.center().x, visible.center().x);
                near(image.center().y, visible.center().y);
                assert!(clip.contains_rect(image));
                assert!(image.width() <= size[0] as f32 && image.height() <= size[1] as f32);
                near(
                    image.width() / image.height(),
                    size[0] as f32 / size[1] as f32,
                );
                let expected = (canvas.x / size[0] as f32)
                    .min(canvas.y / size[1] as f32)
                    .min(1.);
                near(image.width(), size[0] as f32 * expected);
                near(image.height(), size[1] as f32 * expected);
                assert_eq!(viewer.navigation.scroll, [0.; 2]);
                assert_eq!(viewer.texture.id(), texture);
                assert!(Arc::ptr_eq(&pixels, &viewer.pixels));
                assert!(
                    viewer
                        .pixels
                        .pixels
                        .iter()
                        .all(|p| *p == egui::Color32::RED)
                );
            }
        }
    }
}

#[test]
fn manual_texture_keeps_one_to_one_zoom_scroll_and_restored_navigation() {
    for theme in [egui::Theme::Light, egui::Theme::Dark] {
        let ctx = egui::Context::default();
        ctx.set_theme(theme);
        let mut viewer = Viewer::new(&ctx, preview([1024, 768]), None);
        viewer.fit = false;
        viewer.zoom = 1.;
        viewer.navigation.scroll = [71., 53.];
        let canvas = egui::vec2(400., 260.);
        let mut last = None;
        for _ in 0..4 {
            last = Some(canvas_frame(&ctx, &mut viewer, canvas));
        }
        let (image, clip, visible) = last.unwrap();
        near(image.width(), 1024.);
        near(image.height(), 768.);
        near(image.min.x, visible.min.x - 71.);
        near(image.min.y, visible.min.y - 53.);
        assert!(!clip.contains_rect(image));
        assert_eq!(viewer.navigation.scroll, [71., 53.]);
        viewer.zoom = 1.25;
        let (zoomed, _, _) = canvas_frame(&ctx, &mut viewer, canvas);
        near(zoomed.width(), 1280.);
        near(zoomed.height(), 960.);
        let saved = viewer.checkpoint();
        let ctx2 = egui::Context::default();
        ctx2.set_theme(theme);
        let mut reopened = Viewer::new(&ctx2, preview([1024, 768]), None);
        reopened.restore_view(saved.clone());
        for _ in 0..4 {
            canvas_frame(&ctx2, &mut reopened, canvas);
        }
        assert_eq!(reopened.checkpoint(), saved);
        let (restored, _, _) = canvas_frame(&ctx2, &mut reopened, canvas);
        assert_eq!(restored, zoomed);
        reopened.fit = true;
        let (fitted, _, area) = canvas_frame(&ctx2, &mut reopened, canvas);
        near(fitted.center().x, area.center().x);
        near(fitted.center().y, area.center().y);
        assert_eq!(reopened.navigation.scroll, [0.; 2]);
        reopened.fit = false;
        reopened.zoom = 1.;
        let (one, _, area) = canvas_frame(&ctx2, &mut reopened, canvas);
        near(one.min.x, area.min.x);
        near(one.min.y, area.min.y);
    }
}

fn window_frame(
    app: &mut Workspace,
    ctx: &egui::Context,
    size: egui::Vec2,
    events: Vec<egui::Event>,
) -> egui::FullOutput {
    crate::headless::run(
        ctx,
        egui::RawInput {
            screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, size)),
            events,
            ..Default::default()
        },
        |_| app.image_view_ui(ctx),
    )
}

fn click(app: &mut Workspace, ctx: &egui::Context, size: egui::Vec2, label: &str) {
    let output = window_frame(app, ctx, size, vec![]);
    let pos = output
        .shapes
        .iter()
        .find_map(|s| match &s.shape {
            egui::Shape::Text(t)
                if t.galley.text() == label
                    && s.clip_rect.contains_rect(t.visual_bounding_rect()) =>
            {
                Some(t.visual_bounding_rect().center())
            }
            _ => None,
        })
        .unwrap_or_else(|| panic!("missing visible control {label}"));
    for pressed in [true, false] {
        window_frame(
            app,
            ctx,
            size,
            vec![
                egui::Event::PointerMoved(pos),
                egui::Event::PointerButton {
                    pos,
                    button: egui::PointerButton::Primary,
                    pressed,
                    modifiers: egui::Modifiers::NONE,
                },
            ],
        );
    }
}

#[test]
fn actual_preview_controls_and_window_restore_keep_centered_fit_and_manual_sizes() {
    for theme in [egui::Theme::Light, egui::Theme::Dark] {
        for size in [egui::vec2(1440., 900.), egui::vec2(960., 650.)] {
            let ctx = egui::Context::default();
            let mut app = Workspace::with_context(ctx.clone(), true);
            ctx.set_theme(theme);
            app.pending_image = Some(preview([1024, 1024]));
            for _ in 0..5 {
                window_frame(&mut app, &ctx, size, vec![]);
            }
            assert_eq!(
                ctx.global_style().visuals.dark_mode,
                theme == egui::Theme::Dark
            );
            let pixels = app.image_view.as_ref().unwrap().pixels.clone();
            let id = app.image_view.as_ref().unwrap().texture.id();
            let before = window_frame(&mut app, &ctx, size, vec![]);
            let fitted = texture_rect(&before, id).0;
            assert!(fitted.width() < 1024.);
            click(&mut app, &ctx, size, "1:1 preview");
            let one = window_frame(&mut app, &ctx, size, vec![]);
            near(texture_rect(&one, id).0.width(), 1024.);
            assert!(!app.image_view.as_ref().unwrap().fit);
            click(&mut app, &ctx, size, "Zoom in");
            let zoom = window_frame(&mut app, &ctx, size, vec![]);
            near(texture_rect(&zoom, id).0.width(), 1280.);
            click(&mut app, &ctx, size, "Fit image");
            for _ in 0..4 {
                window_frame(&mut app, &ctx, size, vec![]);
            }
            let after = window_frame(&mut app, &ctx, size, vec![]);
            assert_eq!(texture_rect(&after, id).0, fitted);
            let viewer = app.image_view.as_ref().unwrap();
            assert!(Arc::ptr_eq(&pixels, &viewer.pixels));
            assert_eq!(viewer.navigation.scroll, [0.; 2]);
            let state = viewer.checkpoint();
            let ctx2 = egui::Context::default();
            let mut restored = Workspace::with_context(ctx2.clone(), true);
            ctx2.set_theme(theme);
            restored.pending_image = Some(preview([1024, 1024]));
            restored.preview_restore = Some(preview_recovery::Restore::new(state));
            for _ in 0..6 {
                window_frame(&mut restored, &ctx2, size, vec![]);
            }
            let out = window_frame(&mut restored, &ctx2, size, vec![]);
            let v = restored.image_view.as_ref().unwrap();
            let rect = texture_rect(&out, v.texture.id()).0;
            // Recovery notice consumes vertical space; horizontal centering survives.
            near(rect.center().x, fitted.center().x);
            assert!(v.fit);
            assert!(!restored.busy);
            assert!(!app.busy);
        }
    }
}
