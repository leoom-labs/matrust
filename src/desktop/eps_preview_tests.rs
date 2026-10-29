use super::*;
#[test]
fn renderer_basename_is_visible_and_hover_keeps_full_readable_path() {
    for theme in [egui::Theme::Light, egui::Theme::Dark] {
        for size in [egui::vec2(960., 650.), egui::vec2(1440., 900.)] {
            let ctx = egui::Context::default();
            let mut app = Workspace::with_context(ctx.clone(), true);
            ctx.set_theme(theme);
            ctx.all_styles_mut(|style| style.interaction.tooltip_delay = 0.);
            let source = std::env::temp_dir().join("active.eps");
            let renderer = std::env::temp_dir()
                .join("long-user-local-application-directory")
                .join("another-long-portable-installation-directory")
                .join("Ghostscript-version-directory")
                .join("bin")
                .join("gswin64c.exe");
            let full = crate::path_label::display(&renderer).to_string();
            app.eps_preview.renderer = Some(renderer.clone());
            app.eps_preview.pending = Some(source.clone());
            let mut pointer = None;
            let mut hovered = false;
            for pass in 0..8 {
                let output = crate::headless::run(
                    &ctx,
                    egui::RawInput {
                        screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, size)),
                        time: Some(pass as f64),
                        events: pointer.map(egui::Event::PointerMoved).into_iter().collect(),
                        ..Default::default()
                    },
                    |_| app.eps_preview_ui(&ctx),
                );
                if pass >= 2 {
                    pointer = Some(
                        output
                            .shapes
                            .iter()
                            .find_map(|s| ink(&s.shape, s.clip_rect, "Renderer: gswin64c.exe"))
                            .expect("renderer executable basename must be fully visible"),
                    );
                }
                hovered |= output
                    .shapes
                    .iter()
                    .any(|s| ink(&s.shape, s.clip_rect, &full).is_some());
            }
            assert_eq!(
                ctx.global_style().visuals.dark_mode,
                theme == egui::Theme::Dark
            );
            assert!(
                hovered,
                "full readable renderer path must remain available on hover"
            );
            assert_eq!(app.eps_preview.renderer.as_ref(), Some(&renderer));
            assert_eq!(app.snapshot().eps_renderer.as_ref(), Some(&renderer));
            assert_eq!(app.eps_preview.pending.as_ref(), Some(&source));
            assert!(!app.busy, "reading renderer identity must not launch it");
        }
    }
}

fn frame(
    app: &mut Workspace,
    ctx: &egui::Context,
    events: Vec<egui::Event>,
    discard: bool,
) -> egui::FullOutput {
    crate::headless::run(
        ctx,
        egui::RawInput {
            screen_rect: Some(egui::Rect::from_min_size(
                egui::Pos2::ZERO,
                egui::vec2(960., 650.),
            )),
            events,
            ..Default::default()
        },
        |root| {
            if discard && root.current_pass_index() == 0 {
                root.request_discard("EPS pointer final pass");
            }
            app.eps_preview_ui(ctx);
            app.preview_recovery_ui(ctx, app.file_action_pending());
            app.image_view_ui(ctx);
        },
    )
}
fn finish(app: &mut Workspace) {
    let deadline = Instant::now() + Duration::from_secs(5);
    while app.busy && Instant::now() < deadline {
        app.receive();
        std::thread::sleep(Duration::from_millis(1));
    }
    assert!(!app.busy, "{}", app.status);
}
fn ink(shape: &egui::Shape, clip: egui::Rect, label: &str) -> Option<egui::Pos2> {
    match shape {
        egui::Shape::Text(text)
            if text.galley.text() == label && clip.contains_rect(text.visual_bounding_rect()) =>
        {
            Some(text.visual_bounding_rect().center())
        }
        egui::Shape::Vec(shapes) => shapes.iter().find_map(|s| ink(s, clip, label)),
        _ => None,
    }
}
fn click(app: &mut Workspace, ctx: &egui::Context, label: &str, discard: bool) {
    for _ in 0..3 {
        frame(app, ctx, vec![], false);
    }
    let output = frame(app, ctx, vec![], false);
    let pos = output
        .shapes
        .iter()
        .rev()
        .find_map(|s| ink(&s.shape, s.clip_rect, label))
        .unwrap_or_else(|| panic!("Missing fully visible {label}"));
    for pressed in [true, false] {
        frame(
            app,
            ctx,
            vec![
                egui::Event::PointerMoved(pos),
                egui::Event::PointerButton {
                    pos,
                    button: egui::PointerButton::Primary,
                    pressed,
                    modifiers: egui::Modifiers::NONE,
                },
            ],
            discard && !pressed,
        );
    }
}
#[test]
fn eps_open_configuration_discarded_pointer_and_failed_worker_preserve_documents() {
    for theme in [egui::Theme::Light, egui::Theme::Dark] {
        let ctx = egui::Context::default();
        let mut app = Workspace::with_context(ctx.clone(), true);
        ctx.set_theme(theme);
        app.run("keep=[1 2 3];plot(keep);".into());
        finish(&mut app);
        let value = app.variables["keep"].clone();
        let figure = app.figure.as_ref().unwrap().clone();
        let model = app.toolkits.model.clone();
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("bare.eps");
        std::fs::write(
            &path,
            b"%!PS-Adobe-3.0 EPSF-3.0\n%%BoundingBox: 0 0 20 20\n0 0 20 20 rectfill showpage\n",
        )
        .unwrap();
        assert!(app.try_open_path(path.clone()));
        finish(&mut app);
        assert!(app.eps_preview.pending.is_some());
        assert!(!app.busy);
        click(&mut app, &ctx, "Open preview", true); // Unconfigured is genuinely disabled.
        assert!(!app.busy);
        assert!(app.eps_preview.pending.is_some());
        app.eps_preview.renderer = Some(directory.path().join("missing-gs.exe"));
        app.pending_quit = true;
        click(&mut app, &ctx, "Open preview", true);
        assert!(!app.busy);
        assert!(app.eps_preview.pending.is_some());
        app.pending_quit = false;
        click(&mut app, &ctx, "Open preview", true);
        assert!(
            app.busy,
            "Actual final-pass pointer release must submit the worker"
        );
        assert!(app.eps_preview.pending.is_none());
        finish(&mut app);
        assert!(
            app.eps_preview
                .error
                .as_ref()
                .unwrap()
                .contains("unavailable")
        );
        assert!(app.eps_preview.pending.is_some());
        assert!(app.variables["keep"].same_snapshot(&value));
        assert!(app.figure.as_ref().unwrap().same_snapshot(&figure));
        assert!(app.toolkits.model.same_storage(&model));
        click(&mut app, &ctx, "Cancel", true);
        assert!(app.eps_preview.pending.is_none());
        assert!(!app.busy);
        assert_eq!(app.snapshot().eps_renderer, app.eps_preview.renderer);
    }
}
#[test]
fn eps_recovery_never_interprets_without_actual_reopen_pointer() {
    let ctx = egui::Context::default();
    let mut app = Workspace::with_context(ctx.clone(), true);
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("saved.eps");
    std::fs::write(&path, b"%!PS\nshowpage\n").unwrap();
    let mut saved = app.snapshot();
    saved.eps_renderer = Some(directory.path().join("missing-gs.exe"));
    saved.preview = Some(matrust::session::PreviewState {
        path,
        page: 0,
        fit: true,
        zoom: 1.,
        format: Default::default(),
        navigation: Default::default(),
    });
    app.restore_session(saved);
    for _ in 0..6 {
        frame(&mut app, &ctx, vec![], true);
        assert!(!app.busy);
    }
    assert!(!app.preview_restore.as_ref().unwrap().started);
    click(&mut app, &ctx, "Reopen EPS", true);
    assert!(app.busy);
    finish(&mut app);
    assert!(app.preview_restore.as_ref().unwrap().error.is_some());
    assert!(app.eps_preview.pending.is_some());
    app.close_image_preview();
    assert!(app.preview_restore.is_none());
    assert!(app.eps_preview.pending.is_none());
}
#[test]
fn cancelled_eps_result_cannot_replace_an_existing_read_only_image() {
    let ctx = egui::Context::default();
    let mut app = Workspace::with_context(ctx.clone(), true);
    let directory = tempfile::tempdir().unwrap();
    let png = directory.path().join("keep.png");
    image::save_buffer_with_format(
        &png,
        &[20, 40, 60, 255],
        1,
        1,
        image::ColorType::Rgba8,
        image::ImageFormat::Png,
    )
    .unwrap();
    app.submit(Job::OpenImage(png));
    finish(&mut app);
    frame(&mut app, &ctx, vec![], false);
    let pixels = app.image_view.as_ref().unwrap().pixels.clone();
    let path = directory.path().join("source.eps");
    std::fs::write(&path, b"%!PS\nshowpage\n").unwrap();
    app.eps_preview.renderer = Some(directory.path().join("missing.exe"));
    app.begin_eps_preview(path);
    assert!(app.busy);
    // The actual cooperative worker Stop flag must suffice: no extra selection,
    // focus or result-ignore injection is used to make cancellation pass.
    app.worker.cancel.store(true, Ordering::Relaxed);
    finish(&mut app);
    assert!(app.pending_image.is_none());
    assert!(Arc::ptr_eq(
        &pixels,
        &app.image_view.as_ref().unwrap().pixels
    ));
    assert!(app.eps_preview.pending.is_none());
    let path = directory.path().join("source.eps");
    app.begin_eps_preview(path);
    assert!(app.busy);
    app.close_image_preview();
    finish(&mut app);
    assert!(app.image_view.is_none());
    assert!(app.pending_image.is_none());
    assert!(app.eps_preview.pending.is_none());
}

#[test]
fn eps_source_basename_is_visible_in_settings_and_dormant_recovery_without_rendering() {
    for theme in [egui::Theme::Light, egui::Theme::Dark] {
        for size in [egui::vec2(960., 650.), egui::vec2(1440., 900.)] {
            for dormant in [false, true] {
                let ctx = egui::Context::default();
                let mut app = Workspace::with_context(ctx.clone(), true);
                ctx.set_theme(theme);
                ctx.all_styles_mut(|style| style.interaction.tooltip_delay = 0.);
                let source = std::env::temp_dir()
                    .join("long-project-directory-with-independent-preview-captures")
                    .join("another-long-directory-for-the-original-read-only-source")
                    .join("waveform-overlay.eps");
                let renderer = std::env::temp_dir().join("missing-test-gs.exe");
                let full = crate::path_label::display(&source).to_string();
                app.eps_preview.renderer = Some(renderer.clone());
                if dormant {
                    app.preview_restore = Some(preview_recovery::Restore::new(
                        matrust::session::PreviewState {
                            path: source.clone(),
                            page: 0,
                            fit: true,
                            zoom: 1.,
                            format: Default::default(),
                            navigation: Default::default(),
                        },
                    ));
                } else {
                    app.eps_preview.pending = Some(source.clone());
                }
                let saved = app.snapshot();
                let documents = serde_json::to_value(&app.documents).unwrap();
                let mut pointer = None;
                let mut hovered = false;
                for pass in 0..8 {
                    let output = crate::headless::run(
                        &ctx,
                        egui::RawInput {
                            screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, size)),
                            time: Some(pass as f64),
                            events: pointer.map(egui::Event::PointerMoved).into_iter().collect(),
                            ..Default::default()
                        },
                        |_| {
                            if dormant {
                                app.eps_recovery_ui(&ctx, false);
                            } else {
                                app.eps_preview_ui(&ctx);
                            }
                        },
                    );
                    if pass >= 2 {
                        pointer = Some(output.shapes.iter().find_map(|s|
                            ink(&s.shape, s.clip_rect, "File: waveform-overlay.eps")
                        ).expect("the complete source basename must be painted inside the dialog clip"));
                        let controls: &[&str] = if dormant {
                            &["Reopen EPS", "Forget preview"]
                        } else {
                            &["Choose Ghostscript...", "Open preview", "Cancel"]
                        };
                        for label in controls {
                            assert!(
                                output
                                    .shapes
                                    .iter()
                                    .any(|s| ink(&s.shape, s.clip_rect, label).is_some()),
                                "missing fully visible control {label}"
                            );
                        }
                    }
                    hovered |= output
                        .shapes
                        .iter()
                        .any(|s| ink(&s.shape, s.clip_rect, &full).is_some());
                    assert!(
                        !app.busy,
                        "reading source identity must not submit a renderer job"
                    );
                    assert!(app.eps_preview.loading.is_none());
                    assert!(app.pending_image.is_none() && app.image_view.is_none());
                }
                assert!(
                    hovered,
                    "the complete readable path remains available on actual hover"
                );
                assert_eq!(
                    ctx.global_style().visuals.dark_mode,
                    theme == egui::Theme::Dark
                );
                assert_eq!(app.eps_preview.renderer.as_ref(), Some(&renderer));
                assert_eq!(app.snapshot().eps_renderer, saved.eps_renderer);
                assert_eq!(app.snapshot().preview, saved.preview);
                assert_eq!(serde_json::to_value(&app.documents).unwrap(), documents);
                if dormant {
                    assert!(app.eps_preview.pending.is_none());
                    let restore = app.preview_restore.as_ref().unwrap();
                    assert_eq!(restore.state.path, source);
                    assert!(!restore.started && restore.error.is_none());
                } else {
                    assert_eq!(app.eps_preview.pending.as_ref(), Some(&source));
                    assert!(app.preview_restore.is_none());
                }
            }
        }
    }
}
