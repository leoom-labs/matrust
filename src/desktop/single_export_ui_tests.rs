use super::*;
use std::sync::atomic::AtomicBool;
fn finish(app: &mut Workspace) {
    let end = std::time::Instant::now() + std::time::Duration::from_secs(10);
    while app.busy && std::time::Instant::now() < end {
        app.receive();
        std::thread::sleep(std::time::Duration::from_millis(1));
    }
    assert!(!app.busy, "{}", app.status);
}
fn frame(
    app: &mut Workspace,
    ctx: &egui::Context,
    events: Vec<egui::Event>,
    discard: bool,
    mut late: impl FnMut(&mut Workspace),
) -> egui::FullOutput {
    let mut pass = 0;
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
        |ui| {
            app.image_view_ui(ui.ctx());
            if discard && pass == 0 {
                ui.ctx()
                    .request_discard("prepared export review final pass");
            }
            app.single_export_ui(ui.ctx());
            if discard && pass == 0 {
                assert!(!app.busy);
                late(app);
            }
            pass += 1;
        },
    )
}
fn click(app: &mut Workspace, ctx: &egui::Context, text: &str, late: impl FnMut(&mut Workspace)) {
    for _ in 0..3 {
        frame(app, ctx, vec![], false, |_| {});
    }
    let output = frame(app, ctx, vec![], false, |_| {});
    let point = output
        .shapes
        .iter()
        .find_map(|s| {
            if let egui::Shape::Text(t) = &s.shape
                && t.galley.text() == text
                && s.clip_rect.contains_rect(t.visual_bounding_rect())
            {
                Some(t.visual_bounding_rect().center())
            } else {
                None
            }
        })
        .expect("fully visible actual review button");
    let event = |pressed| {
        vec![
            egui::Event::PointerMoved(point),
            egui::Event::PointerButton {
                pos: point,
                button: egui::PointerButton::Primary,
                pressed,
                modifiers: egui::Modifiers::NONE,
            },
        ]
    };
    frame(app, ctx, event(true), false, |_| {});
    frame(app, ctx, event(false), true, late);
}
fn start_preview(app: &mut Workspace, path: PathBuf) {
    let v = app.image_view.as_ref().unwrap();
    app.submit(Job::ExportPreview(image_export::Request {
        path,
        source: v.path.clone(),
        pixels: v.pixels.clone(),
    }));
    finish(app);
}
#[test]
fn actual_worker_review_requires_explicit_replace_and_preserves_captured_pixels_and_documents() {
    for dark in [false, true] {
        let ctx = egui::Context::default();
        let mut app = Workspace::with_context(ctx.clone(), true);
        ctx.set_theme(if dark {
            egui::Theme::Dark
        } else {
            egui::Theme::Light
        });
        let dir = tempfile::tempdir().unwrap();
        let source = dir.path().join("source.png");
        image::RgbaImage::from_pixel(64, 32, image::Rgba([17, 83, 149, 255]))
            .save(&source)
            .unwrap();
        app.run("a=1:10;alias=a;plot(a);".into());
        finish(&mut app);
        let figure = app.figure.clone().unwrap();
        let vars = app.variables.clone();
        let docs = serde_json::to_vec(&app.documents).unwrap();
        let history = app.figure_history.clone();
        app.submit(Job::OpenImage(source.clone()));
        finish(&mut app);
        frame(&mut app, &ctx, vec![], false, |_| {});
        let pixels = app.image_view.as_ref().unwrap().pixels.clone();
        let target = dir.path().join("replace.png");
        std::fs::write(&target, b"old content").unwrap();
        start_preview(&mut app, target.clone());
        assert!(app.single_export.reviewing());
        assert!(app.file_action_pending());
        assert_eq!(std::fs::read(&target).unwrap(), b"old content");
        click(&mut app, &ctx, "Replace destination", |_| {});
        finish(&mut app);
        assert!(!app.single_export.pending());
        assert_eq!(
            image::open(&target).unwrap().to_rgba8().get_pixel(20, 10).0,
            [17, 83, 149, 255]
        );
        assert!(Arc::ptr_eq(
            &pixels,
            &app.image_view.as_ref().unwrap().pixels
        ));
        assert!(figure.same_snapshot(app.figure.as_ref().unwrap()));
        assert_eq!(history, app.figure_history);
        assert_eq!(docs, serde_json::to_vec(&app.documents).unwrap());
        for (name, value) in vars {
            assert!(app.variables[&name].same_snapshot(&value));
        }
        assert_eq!(ctx.global_style().visuals.dark_mode, dark);
        let fresh = dir.path().join("fresh.png");
        start_preview(&mut app, fresh.clone());
        assert!(fresh.exists());
        assert!(!app.single_export.pending());
        // The actual figure-dialog request takes the same guarded worker channel.
        app.begin_figure_export();
        app.figure_export.as_mut().unwrap().settings.format = figure_export::Format::Svg;
        let svg = dir.path().join("figure.svg");
        std::fs::write(&svg, b"existing svg").unwrap();
        app.export_to_path(svg.clone());
        finish(&mut app);
        assert!(app.single_export.reviewing());
        assert_eq!(std::fs::read(&svg).unwrap(), b"existing svg");
        click(&mut app, &ctx, "Replace destination", |_| {});
        finish(&mut app);
        assert!(app.figure_export.is_none());
        assert!(std::fs::read_to_string(svg).unwrap().contains("<svg"));
        assert!(figure.same_snapshot(app.figure.as_ref().unwrap()));
        // Stopping an unrelated batch retains its existing dialog lifecycle.
        app.begin_figure_export();
        app.figure_export.as_mut().unwrap().select_all();
        app.figure_export.as_mut().unwrap().saving = true;
        app.cancel_single_export();
        assert!(app.figure_export.as_ref().unwrap().saving);
    }
}
#[test]
fn reviewed_external_changes_cancel_stop_close_and_source_replacement_never_publish() {
    for mode in 0..6 {
        let ctx = egui::Context::default();
        let mut app = Workspace::with_context(ctx.clone(), true);
        let dir = tempfile::tempdir().unwrap();
        let source = dir.path().join("source.png");
        image::RgbaImage::from_pixel(64, 32, image::Rgba([17, 83, 149, 255]))
            .save(&source)
            .unwrap();
        app.submit(Job::OpenImage(source.clone()));
        finish(&mut app);
        frame(&mut app, &ctx, vec![], false, |_| {});
        let target = dir.path().join("existing.png");
        std::fs::write(&target, b"before").unwrap();
        start_preview(&mut app, target.clone());
        assert!(app.single_export.reviewing());
        match mode {
            0 => {
                std::fs::write(&target, b"after!").unwrap();
                click(&mut app, &ctx, "Replace destination", |_| {});
                finish(&mut app);
                assert!(app.status.contains("changed after export preparation"));
            }
            1 => click(&mut app, &ctx, "Cancel", |_| {}),
            2 => click(&mut app, &ctx, "Replace destination", |app| app.stop_work()),
            3 => click(&mut app, &ctx, "Replace destination", |app| {
                app.close_image_preview()
            }),
            4 => {
                app.pending_image =
                    Some(image_load::load(&source, 4096, &AtomicBool::new(false)).unwrap());
                frame(&mut app, &ctx, vec![], false, |_| {});
            }
            _ => click(&mut app, &ctx, "Replace destination", |app| {
                app.pending_quit = true
            }),
        }
        finish(&mut app);
        app.pending_quit = false;
        for _ in 0..3 {
            frame(&mut app, &ctx, vec![], false, |_| {});
        }
        assert!(!app.single_export.pending());
        assert!(!app.busy);
        assert!(!app.image_action);
        assert_eq!(
            std::fs::read(&target).unwrap(),
            if mode == 0 { b"after!" } else { b"before" }
        );
        assert_eq!(
            std::fs::read_dir(dir.path()).unwrap().count(),
            2,
            "owned staging file must be dropped"
        );
    }
}

#[test]
fn dispatched_preview_close_finishes_fresh_save_but_never_approves_late_replacement() {
    // Holding the actual submitted request makes Close/Stop precede worker encoding.
    for (existing, stop, replace_preview) in [
        (false, false, false),
        (true, false, false),
        (false, true, false),
        (true, true, false),
        (false, false, true),
    ] {
        let ctx = egui::Context::default();
        let mut app = Workspace::with_context(ctx.clone(), true);
        let dir = tempfile::tempdir().unwrap();
        let source = dir.path().join("source.png");
        image::RgbaImage::from_pixel(64, 32, image::Rgba([17, 83, 149, 255]))
            .save(&source)
            .unwrap();
        let source_bytes = std::fs::read(&source).unwrap();
        app.run("a=1:10;alias=a;plot(a);".into());
        finish(&mut app);
        let figure = app.figure.clone().unwrap();
        let vars = app.variables.clone();
        let docs = serde_json::to_vec(&app.documents).unwrap();
        let history = app.figure_history.clone();
        app.submit(Job::OpenImage(source.clone()));
        finish(&mut app);
        frame(&mut app, &ctx, vec![], false, |_| {});
        let pixels = app.image_view.as_ref().unwrap().pixels.clone();
        let target = dir.path().join("target.png");
        if existing {
            std::fs::write(&target, b"original destination").unwrap();
        }
        let (send, receive) = std::sync::mpsc::sync_channel(1);
        let real = std::mem::replace(&mut app.worker.send, send);
        app.export_preview_to(target.clone());
        let request = receive.try_recv().unwrap();
        let Job::SingleExport(job) = &request.job else {
            panic!("actual prepared preview route")
        };
        let single_export::Action::Prepare(single_export::Input::Preview(captured)) = &job.action
        else {
            panic!("actual captured preview input")
        };
        assert!(Arc::ptr_eq(&captured.pixels, &pixels));
        assert!(receive.try_recv().is_err());
        assert!(app.busy && app.image_action && !app.image_loading);
        app.close_image_preview();
        assert!(app.single_export.pending() && app.single_export.closed_preview);
        assert!(!app.worker.cancel.load(Ordering::Relaxed));
        if replace_preview {
            app.pending_image =
                Some(image_load::load(&source, 4096, &AtomicBool::new(false)).unwrap());
            frame(&mut app, &ctx, vec![], false, |_| {});
        }
        if stop {
            app.stop_work();
        }
        let status_before_reply = app.status.clone();
        assert_eq!(
            app.worker.cancel.load(Ordering::Relaxed),
            stop || replace_preview
        );
        app.worker.send = real;
        app.worker.send.send(request).unwrap();
        finish(&mut app);
        for _ in 0..3 {
            frame(&mut app, &ctx, vec![], false, |_| {});
        }
        assert!(!app.single_export.pending() && !app.single_export.reviewing());
        assert!(!app.image_action && !app.image_loading && app.pending_image.is_none());
        if replace_preview {
            let viewer = app.image_view.as_ref().unwrap();
            assert!(!Arc::ptr_eq(&pixels, &viewer.pixels));
            assert!(viewer.notice.is_none() && viewer.error.is_none());
        } else {
            assert!(app.image_view.is_none());
        }
        if existing {
            assert_eq!(std::fs::read(&target).unwrap(), b"original destination");
        } else if stop || replace_preview {
            assert!(!target.exists());
        } else {
            assert_eq!(
                image_load::load(&target, 4096, &AtomicBool::new(false))
                    .unwrap()
                    .pixels
                    .pixels,
                pixels.pixels
            );
        }
        if stop || replace_preview {
            assert_eq!(
                app.status, status_before_reply,
                "stale replies cannot replace newer feedback"
            );
        } else if existing {
            assert_eq!(app.status, "Preview closed; replacement cancelled");
        } else {
            assert!(app.status.starts_with("Saved preview to "));
        }
        assert_eq!(std::fs::read(&source).unwrap(), source_bytes);
        assert_eq!(
            std::fs::read_dir(dir.path()).unwrap().count(),
            if target.exists() { 2 } else { 1 }
        );
        assert!(figure.same_snapshot(app.figure.as_ref().unwrap()));
        assert_eq!(history, app.figure_history);
        assert_eq!(docs, serde_json::to_vec(&app.documents).unwrap());
        for (name, value) in vars {
            assert!(app.variables[&name].same_snapshot(&value));
        }
    }
}

#[test]
fn explicitly_approved_preview_replacement_can_finish_after_close_without_reopening() {
    let ctx = egui::Context::default();
    let mut app = Workspace::with_context(ctx.clone(), true);
    let dir = tempfile::tempdir().unwrap();
    let source = dir.path().join("source.png");
    image::RgbaImage::from_pixel(64, 32, image::Rgba([17, 83, 149, 255]))
        .save(&source)
        .unwrap();
    let source_bytes = std::fs::read(&source).unwrap();
    app.submit(Job::OpenImage(source.clone()));
    finish(&mut app);
    frame(&mut app, &ctx, vec![], false, |_| {});
    let target = dir.path().join("target.png");
    std::fs::write(&target, b"original destination").unwrap();
    start_preview(&mut app, target.clone());
    assert!(app.single_export.reviewing());
    let (send, receive) = std::sync::mpsc::sync_channel(1);
    let real = std::mem::replace(&mut app.worker.send, send);
    click(&mut app, &ctx, "Replace destination", |_| {});
    let request = receive.try_recv().unwrap();
    let Job::SingleExport(job) = &request.job else {
        panic!("actual replacement request")
    };
    assert!(matches!(&job.action, single_export::Action::Commit(_)));
    assert!(receive.try_recv().is_err());
    app.close_image_preview();
    assert!(!app.worker.cancel.load(Ordering::Relaxed));
    app.worker.send = real;
    app.worker.send.send(request).unwrap();
    finish(&mut app);
    frame(&mut app, &ctx, vec![], false, |_| {});
    assert!(app.image_view.is_none() && app.pending_image.is_none());
    assert!(!app.single_export.pending() && !app.image_action);
    assert!(app.status.starts_with("Saved preview to "));
    assert_eq!(
        image::open(&target).unwrap().to_rgba8().get_pixel(20, 10).0,
        [17, 83, 149, 255]
    );
    assert_eq!(std::fs::read(&source).unwrap(), source_bytes);
    assert_eq!(std::fs::read_dir(dir.path()).unwrap().count(), 2);
}
