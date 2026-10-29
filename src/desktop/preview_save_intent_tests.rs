use super::*;

struct Picker;
impl Picker {
    fn install(choice: Option<PathBuf>) -> Self {
        PREVIEW_PICKER.with(|slot| {
            assert!(slot.borrow().is_none());
            *slot.borrow_mut() = Some((choice, 0));
        });
        Self
    }
    fn calls(&self) -> usize {
        PREVIEW_PICKER.with(|slot| slot.borrow().as_ref().unwrap().1)
    }
}
impl Drop for Picker {
    fn drop(&mut self) {
        PREVIEW_PICKER.with(|slot| *slot.borrow_mut() = None);
    }
}
fn finish(app: &mut Workspace) {
    let until = std::time::Instant::now() + std::time::Duration::from_secs(5);
    while app.busy && std::time::Instant::now() < until {
        app.receive();
        std::thread::sleep(std::time::Duration::from_millis(1));
    }
    assert!(!app.busy, "{}", app.status);
}
fn frame(app: &mut Workspace, ctx: &egui::Context, events: Vec<egui::Event>) -> egui::FullOutput {
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
            app.finish_document_action(ui.ctx());
            app.finish_preview_save(ui.ctx());
        },
    )
}
fn button(app: &mut Workspace, ctx: &egui::Context) -> egui::Pos2 {
    for _ in 0..3 {
        frame(app, ctx, vec![]);
    }
    frame(app, ctx, vec![])
        .shapes
        .iter()
        .find_map(|shape| {
            if let egui::Shape::Text(text) = &shape.shape
                && text.galley.text() == "Save preview..."
                && shape.clip_rect.contains_rect(text.visual_bounding_rect())
            {
                Some(text.visual_bounding_rect().center())
            } else {
                None
            }
        })
        .expect("actual fully visible preview save button")
}
fn release(
    app: &mut Workspace,
    ctx: &egui::Context,
    picker: &Picker,
    point: egui::Pos2,
    mut late: impl FnMut(&mut Workspace),
) {
    frame(
        app,
        ctx,
        vec![
            egui::Event::PointerMoved(point),
            egui::Event::PointerButton {
                pos: point,
                button: egui::PointerButton::Primary,
                pressed: true,
                modifiers: egui::Modifiers::NONE,
            },
        ],
    );
    let mut passes = 0;
    crate::headless::run(
        ctx,
        egui::RawInput {
            screen_rect: Some(egui::Rect::from_min_size(
                egui::Pos2::ZERO,
                egui::vec2(960., 650.),
            )),
            events: vec![
                egui::Event::PointerMoved(point),
                egui::Event::PointerButton {
                    pos: point,
                    button: egui::PointerButton::Primary,
                    pressed: false,
                    modifiers: egui::Modifiers::NONE,
                },
            ],
            ..Default::default()
        },
        |ui| {
            passes += 1;
            if passes == 1 {
                ui.ctx()
                    .request_discard("Preview save waits for final authority");
            }
            app.image_view_ui(ui.ctx());
            app.finish_document_action(ui.ctx());
            if passes == 1 {
                assert!(
                    app.image_view.as_ref().unwrap().save_intent.is_some(),
                    "real release retains one intent"
                );
                app.finish_preview_save(ui.ctx());
                assert_eq!(picker.calls(), 0);
                assert!(!app.busy);
                late(app);
            } else {
                app.finish_preview_save(ui.ctx());
            }
        },
    );
    assert!(passes >= 2);
}
fn setup(theme: session::DesktopTheme, source: &std::path::Path) -> (egui::Context, Workspace) {
    let ctx = egui::Context::default();
    let mut app = Workspace::with_context(ctx.clone(), true);
    app.layout.theme = Some(theme);
    app.apply_desktop_theme(&ctx);
    image::RgbaImage::from_pixel(64, 32, image::Rgba([41, 83, 127, 255]))
        .save(source)
        .unwrap();
    app.run("a=[1 2 3];alias=a;plot(a);".into());
    finish(&mut app);
    app.submit(Job::OpenImage(source.to_owned()));
    finish(&mut app);
    frame(&mut app, &ctx, vec![]);
    assert_eq!(
        ctx.global_style().visuals.dark_mode,
        theme == session::DesktopTheme::Dark
    );
    (ctx, app)
}
#[test]
fn preview_real_release_discard_opens_once_then_cancel_or_worker_save_preserves_source() {
    for theme in [session::DesktopTheme::Light, session::DesktopTheme::Dark] {
        for chosen in [false, true] {
            let dir = tempfile::tempdir().unwrap();
            let source = dir.path().join("original.png");
            let destination = dir.path().join("preview.png");
            let (ctx, mut app) = setup(theme, &source);
            let original = std::fs::read(&source).unwrap();
            let figure = app.figure.clone().unwrap();
            let documents = serde_json::to_vec(&app.documents).unwrap();
            let pixels = app.image_view.as_ref().unwrap().pixels.clone();
            let point = button(&mut app, &ctx);
            let picker = Picker::install(chosen.then(|| destination.clone()));
            release(&mut app, &ctx, &picker, point, |_| {});
            assert_eq!(picker.calls(), 1);
            if chosen {
                assert!(app.busy);
                finish(&mut app);
                let saved = image::open(&destination).unwrap().into_rgba8();
                assert_eq!(saved.dimensions(), (64, 32));
                assert!(saved.pixels().all(|pixel| pixel.0 == [41, 83, 127, 255]));
            } else {
                assert!(!app.busy && !destination.exists());
            }
            for _ in 0..3 {
                frame(&mut app, &ctx, vec![]);
            }
            assert_eq!(picker.calls(), 1, "no later frame replay");
            assert_eq!(std::fs::read(&source).unwrap(), original);
            assert_eq!(serde_json::to_vec(&app.documents).unwrap(), documents);
            assert!(app.figure.as_ref().unwrap().same_snapshot(&figure));
            assert!(app.variables["a"].same_snapshot(&app.variables["alias"]));
            assert!(Arc::ptr_eq(
                &pixels,
                &app.image_view.as_ref().unwrap().pixels
            ));
        }
    }
}
#[test]
fn preview_release_rejects_late_busy_modal_replace_close_stop_format_and_authority() {
    for case in 0..9 {
        let dir = tempfile::tempdir().unwrap();
        let source = dir.path().join("original.png");
        let destination = dir.path().join("must-not-exist.png");
        let (ctx, mut app) = setup(session::DesktopTheme::Dark, &source);
        let point = button(&mut app, &ctx);
        let picker = Picker::install(Some(destination.clone()));
        release(&mut app, &ctx, &picker, point, |app| match case {
            0 => app.busy = true,
            1 => app.pending_close = Some(0),
            2 | 3 => {
                app.pending_image = Some(image_load::Preview {
                    original: None,
                    path: if case == 2 {
                        source.clone()
                    } else {
                        dir.path().join("other.png")
                    },
                    original_size: [64, 32],
                    format: "PNG",
                    file_bytes: 1,
                    pixels: egui::ColorImage::filled([64, 32], egui::Color32::BLUE),
                })
            }
            4 => app.close_image_preview(),
            5 => app.stop_work(),
            6 => app.image_view.as_mut().unwrap().export_format = image_export::Format::Jpeg,
            7 => app.document_save.command_selected(),
            8 => app.pending_quit = true,
            _ => unreachable!(),
        });
        assert_eq!(picker.calls(), 0, "case {case}");
        assert!(!destination.exists());
        app.busy = false;
        app.pending_close = None;
        app.pending_quit = false;
        for _ in 0..3 {
            frame(&mut app, &ctx, vec![]);
        }
        assert_eq!(picker.calls(), 0, "retired case {case} cannot replay");
    }
}

#[test]
fn preview_button_and_generic_save_share_one_final_picker_even_when_cancelled() {
    let dir = tempfile::tempdir().unwrap();
    let source = dir.path().join("original.png");
    let (ctx, mut app) = setup(session::DesktopTheme::Light, &source);
    let point = button(&mut app, &ctx);
    let picker = Picker::install(None);
    frame(
        &mut app,
        &ctx,
        vec![
            egui::Event::PointerMoved(point),
            egui::Event::PointerButton {
                pos: point,
                button: egui::PointerButton::Primary,
                pressed: true,
                modifiers: egui::Modifiers::NONE,
            },
        ],
    );
    let mut passes = 0;
    crate::headless::run(
        &ctx,
        egui::RawInput {
            screen_rect: Some(egui::Rect::from_min_size(
                egui::Pos2::ZERO,
                egui::vec2(960., 650.),
            )),
            events: vec![
                egui::Event::PointerMoved(point),
                egui::Event::PointerButton {
                    pos: point,
                    button: egui::PointerButton::Primary,
                    pressed: false,
                    modifiers: egui::Modifiers::NONE,
                },
            ],
            ..Default::default()
        },
        |ui| {
            passes += 1;
            if passes == 1 {
                ui.ctx()
                    .request_discard("both save routes retain final intent");
            }
            app.image_view_ui(ui.ctx());
            if passes == 1 {
                assert!(app.image_view.as_ref().unwrap().save_intent.is_some());
                // The same queue used by File/Save and Ctrl+S, in actual root order.
                app.document_save.queue(
                    ui.ctx().cumulative_frame_nr(),
                    false,
                    document_save::Pane::Preview,
                );
            }
            app.finish_document_action(ui.ctx());
            app.finish_preview_save(ui.ctx());
            if passes == 1 {
                assert_eq!(picker.calls(), 0);
            }
        },
    );
    assert!(passes >= 2);
    assert_eq!(
        picker.calls(),
        1,
        "Cancel cannot leave a second button picker queued"
    );
    assert!(!app.busy);
    assert!(app.image_view.as_ref().unwrap().save_intent.is_none());
    for _ in 0..3 {
        frame(&mut app, &ctx, vec![]);
    }
    assert_eq!(picker.calls(), 1);
}
