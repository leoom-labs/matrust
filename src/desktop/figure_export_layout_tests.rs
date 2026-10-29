use super::*;
use egui::{Event, Pos2, Rect, Vec2};
use std::sync::atomic::AtomicBool;

fn dialog(format: Format) -> Dialog {
    let mut engine = matrust::Engine::default();
    engine.execute("plot([0 1 2],[0 3 0]);").unwrap();
    Dialog::new(
        1,
        engine.figures().next().unwrap().1,
        None,
        Settings {
            format,
            ..Default::default()
        },
    )
}
fn frame(
    ctx: &egui::Context,
    dialog: &mut Dialog,
    size: Vec2,
    events: Vec<Event>,
) -> (egui::FullOutput, Action) {
    let mut action = Action::None;
    let output = crate::headless::run(
        ctx,
        egui::RawInput {
            screen_rect: Some(Rect::from_min_size(Pos2::ZERO, size)),
            events,
            ..Default::default()
        },
        |root| {
            let ctx = &root.ctx().clone();
            action = dialog.show(ctx);
        },
    );
    (output, action)
}
fn find(shape: &egui::Shape, label: &str) -> Option<Rect> {
    match shape {
        egui::Shape::Text(t) if t.galley.text() == label => {
            Some(Rect::from_min_size(t.pos, t.galley.size()))
        }
        egui::Shape::Vec(shapes) => shapes.iter().find_map(|s| find(s, label)),
        _ => None,
    }
}
fn visible(output: &egui::FullOutput, label: &str, size: Vec2) -> Option<Rect> {
    output.shapes.iter().find_map(|s| {
        find(&s.shape, label).filter(|r| {
            s.clip_rect.expand(0.1).contains_rect(*r)
                && Rect::from_min_size(Pos2::ZERO, size).contains_rect(*r)
        })
    })
}
fn click(ctx: &egui::Context, dialog: &mut Dialog, size: Vec2, pos: Pos2) -> Action {
    let mut action = Action::None;
    for pressed in [true, false] {
        action = frame(
            ctx,
            dialog,
            size,
            vec![
                Event::PointerMoved(pos),
                Event::PointerButton {
                    pos,
                    button: egui::PointerButton::Primary,
                    pressed,
                    modifiers: egui::Modifiers::NONE,
                },
            ],
        )
        .1;
    }
    action
}
fn escape() -> Event {
    Event::Key {
        key: egui::Key::Escape,
        physical_key: None,
        pressed: true,
        repeat: false,
        modifiers: egui::Modifiers::NONE,
    }
}

#[test]
fn export_layout_all_formats_and_long_errors_keep_actions_visible() {
    for format in [
        Format::Png,
        Format::Jpeg,
        Format::Bmp,
        Format::Tiff,
        Format::Svg,
        Format::Pdf,
        Format::Eps,
        Format::Csv,
        Format::MatlabScript,
    ] {
        let ctx = egui::Context::default();
        let mut d = dialog(format);
        let original = d.figure.clone();
        d.error = Some("Destination unavailable: ".repeat(200));
        for size in [
            Vec2::new(1440., 900.),
            Vec2::new(960., 650.),
            Vec2::new(640., 480.),
            Vec2::new(480., 320.),
        ] {
            for _ in 0..4 {
                frame(&ctx, &mut d, size, vec![]);
            }
            let (output, _) = frame(&ctx, &mut d, size, vec![]);
            for label in ["Export figure 1", "Choose file\u{2026}", "Cancel"] {
                assert!(
                    visible(&output, label, size).is_some(),
                    "{format:?}: missing/clipped {label}"
                );
            }
            if format.raster() {
                assert!(visible(&output, "Copy image", size).is_some());
            }
            let rect = ctx
                .memory(|m| m.area_rect(egui::Id::new("figure_export")))
                .unwrap();
            assert!(
                Rect::from_min_size(Pos2::ZERO, size).contains_rect(rect),
                "{format:?}: {rect:?} in {size:?}"
            );
        }
        let size = Vec2::new(480., 320.);
        let (output, _) = frame(&ctx, &mut d, size, vec![]);
        assert!(matches!(
            click(
                &ctx,
                &mut d,
                size,
                visible(&output, "Cancel", size).unwrap().center()
            ),
            Action::Cancel
        ));
        assert!(d.figure.same_snapshot(&original));
    }
}

#[test]
fn export_layout_review_scrolls_to_replacement_and_retains_explicit_consent() {
    let ctx = egui::Context::default();
    let size = Vec2::new(480., 320.);
    let dir = tempfile::tempdir().unwrap();
    let mut d = dialog(Format::Png);
    d.select_all();
    std::fs::write(dir.path().join("figure-1.png"), b"existing").unwrap();
    let plan = export_batch::prepare(
        export_batch::Prepare {
            folder: dir.path().into(),
            figures: (1..=16).map(|i| (i, d.figure.clone())).collect(),
            settings: d.settings,
        },
        &AtomicBool::new(false),
    )
    .unwrap();
    d.review(plan);
    for _ in 0..4 {
        frame(&ctx, &mut d, size, vec![]);
    }
    let (output, _) = frame(&ctx, &mut d, size, vec![]);
    let export = visible(&output, "Export all", size).unwrap().center();
    assert!(matches!(click(&ctx, &mut d, size, export), Action::None));
    let heading = visible(&output, "Export all open figures", size).unwrap();
    let scroll_point = Pos2::new(heading.left() + 10., heading.bottom() + 20.);
    for _ in 0..18 {
        frame(
            &ctx,
            &mut d,
            size,
            vec![
                Event::PointerMoved(scroll_point),
                Event::MouseWheel {
                    phase: egui::TouchPhase::Move,
                    unit: egui::MouseWheelUnit::Point,
                    delta: egui::vec2(0., -120.),
                    modifiers: egui::Modifiers::NONE,
                },
            ],
        );
    }
    for _ in 0..20 {
        frame(&ctx, &mut d, size, vec![]);
    }
    let (output, _) = frame(&ctx, &mut d, size, vec![]);
    let replace = visible(&output, "Replace existing files", size)
        .expect("replacement control must be reachable");
    click(&ctx, &mut d, size, replace.center());
    assert!(d.replace);
    let (output, _) = frame(&ctx, &mut d, size, vec![]);
    assert!(matches!(
        click(
            &ctx,
            &mut d,
            size,
            visible(&output, "Export all", size).unwrap().center()
        ),
        Action::ExportAll
    ));
    assert_eq!(
        std::fs::read(dir.path().join("figure-1.png")).unwrap(),
        b"existing"
    );
    assert_eq!(std::fs::read_dir(dir.path()).unwrap().count(), 1);
}

#[test]
fn export_layout_busy_progress_stop_and_escape_preserve_pending_dialog() {
    let ctx = egui::Context::default();
    let size = Vec2::new(480., 320.);
    let mut d = dialog(Format::Pdf);
    let original = d.figure.clone();
    d.saving = true;
    d.error = Some("Long diagnostic message. ".repeat(200));
    for _ in 0..4 {
        frame(&ctx, &mut d, size, vec![]);
    }
    let (output, _) = frame(&ctx, &mut d, size, vec![]);
    assert!(visible(&output, "Exporting\u{2026}", size).is_some());
    let stop = visible(&output, "Stop", size).unwrap();
    assert!(matches!(
        click(&ctx, &mut d, size, stop.center()),
        Action::Stop
    ));
    assert!(d.saving);
    assert!(matches!(
        frame(&ctx, &mut d, size, vec![escape()]).1,
        Action::Stop
    ));
    assert!(d.figure.same_snapshot(&original));
}

#[test]
fn export_layout_escape_closes_format_popup_before_cancelling_dialog() {
    let ctx = egui::Context::default();
    let size = Vec2::new(640., 480.);
    let mut d = dialog(Format::Png);
    for _ in 0..4 {
        frame(&ctx, &mut d, size, vec![]);
    }
    let (output, _) = frame(&ctx, &mut d, size, vec![]);
    let format = visible(&output, d.settings.format.label(), size).unwrap();
    click(&ctx, &mut d, size, format.center());
    assert!(egui::Popup::is_any_open(&ctx));
    assert!(matches!(
        frame(&ctx, &mut d, size, vec![escape()]).1,
        Action::None
    ));
    assert!(!egui::Popup::is_any_open(&ctx));
    frame(
        &ctx,
        &mut d,
        size,
        vec![Event::Key {
            key: egui::Key::Escape,
            physical_key: None,
            pressed: false,
            repeat: false,
            modifiers: egui::Modifiers::NONE,
        }],
    );
    assert!(matches!(
        frame(&ctx, &mut d, size, vec![escape()]).1,
        Action::Cancel
    ));
    assert_eq!(d.settings.format, Format::Png);
}

#[test]
fn eps_dialog_uses_point_presets_without_raster_copy_or_preview_claim() {
    for size in [Vec2::new(1440., 900.), Vec2::new(640., 480.)] {
        let ctx = egui::Context::default();
        let mut d = dialog(Format::Eps);
        for _ in 0..4 {
            frame(&ctx, &mut d, size, vec![]);
        }
        let (output, _) = frame(&ctx, &mut d, size, vec![]);
        for label in [
            "EPS (color vector)",
            "Width (points)",
            "Height (points)",
            "Letter",
            "Choose file\u{2026}",
            "Cancel",
        ] {
            assert!(
                visible(&output, label, size).is_some(),
                "missing {label} at {size:?}"
            );
        }
        assert!(
            !output
                .shapes
                .iter()
                .any(|s| find(&s.shape, "Copy image").is_some())
        );
        let letter = visible(&output, "Letter", size).unwrap().center();
        click(&ctx, &mut d, size, letter);
        assert_eq!(
            d.settings.vector,
            matrust::plot::Canvas {
                width: 612.,
                height: 792.
            }
        );
        let (output, _) = frame(&ctx, &mut d, size, vec![]);
        assert!(
            visible(
                &output,
                "8.50 \u{d7} 11.00 inches; 72 points per inch",
                size
            )
            .is_some()
        );
        let dir = tempfile::tempdir().unwrap();
        assert!(d.request(dir.path().join("wrong.svg")).is_err());
        let Job::ExportSnapshot(request) = d.request(dir.path().join("correct.eps")).unwrap()
        else {
            panic!("expected real export job")
        };
        assert_eq!(request.vector, Some(d.settings.vector));
        assert!(request.figure.same_snapshot(&d.figure));
        d.saving = true;
        for _ in 0..3 {
            frame(&ctx, &mut d, size, vec![]);
        }
        let (output, _) = frame(&ctx, &mut d, size, vec![]);
        let stop = visible(&output, "Stop", size).unwrap().center();
        assert!(matches!(click(&ctx, &mut d, size, stop), Action::Stop));
        assert!(matches!(
            frame(&ctx, &mut d, size, vec![escape()]).1,
            Action::Stop
        ));
    }
}

fn release_export_action_with_discard(
    ctx: &egui::Context,
    d: &mut Dialog,
    size: Vec2,
    label: &str,
    mut late: impl FnMut(&mut Dialog),
) -> Vec<Action> {
    for _ in 0..4 {
        frame(ctx, d, size, vec![]);
    }
    let (output, _) = frame(ctx, d, size, vec![]);
    let point = visible(&output, label, size)
        .expect("actual visible export action")
        .center();
    let button = |pressed| {
        vec![
            Event::PointerMoved(point),
            Event::PointerButton {
                pos: point,
                button: egui::PointerButton::Primary,
                pressed,
                modifiers: egui::Modifiers::NONE,
            },
        ]
    };
    assert_eq!(frame(ctx, d, size, button(true)).1, Action::None);
    let mut pass = 0;
    let mut admitted = vec![];
    crate::headless::run(
        ctx,
        egui::RawInput {
            screen_rect: Some(Rect::from_min_size(Pos2::ZERO, size)),
            events: button(false),
            ..Default::default()
        },
        |ui| {
            if pass == 0 {
                ui.ctx()
                    .request_discard("export action must wait for final layout");
            }
            let action = d.show(ui.ctx());
            if pass == 0 {
                assert_eq!(
                    action,
                    Action::None,
                    "no picker, clipboard or export may dispatch early"
                );
                late(d);
            } else if action != Action::None {
                admitted.push(action);
            }
            pass += 1;
        },
    );
    assert!(pass >= 2);
    for _ in 0..3 {
        assert_eq!(
            frame(ctx, d, size, vec![]).1,
            Action::None,
            "a completed/cancelled action cannot replay on a later paint"
        );
    }
    admitted
}

#[test]
fn export_real_button_release_waits_for_final_pass_once_for_each_dispatch_kind() {
    for dark in [false, true] {
        for size in [Vec2::new(960., 650.), Vec2::new(640., 480.)] {
            for (label, expected) in [
                ("Choose file\u{2026}", Action::ChoosePath),
                ("Choose folder\u{2026}", Action::ChooseFolder),
                ("Copy image", Action::Copy),
                ("Cancel", Action::Cancel),
                ("Stop", Action::Stop),
                ("Export all", Action::ExportAll),
            ] {
                let ctx = egui::Context::default();
                ctx.set_theme(if dark {
                    egui::Theme::Dark
                } else {
                    egui::Theme::Light
                });
                let mut d = dialog(Format::Png);
                let original = d.figure.clone();
                let dir = tempfile::tempdir().unwrap();
                if matches!(expected, Action::ChooseFolder | Action::ExportAll) {
                    d.select_all();
                }
                if expected == Action::ExportAll {
                    d.review(
                        export_batch::prepare(
                            export_batch::Prepare {
                                folder: dir.path().into(),
                                figures: vec![(1, d.figure.clone())],
                                settings: d.settings,
                            },
                            &AtomicBool::new(false),
                        )
                        .unwrap(),
                    );
                }
                if expected == Action::Stop {
                    d.saving = true;
                }
                assert_eq!(
                    release_export_action_with_discard(&ctx, &mut d, size, label, |_| {}),
                    vec![expected]
                );
                assert!(d.figure.same_snapshot(&original));
                assert_eq!(
                    std::fs::read_dir(dir.path()).unwrap().count(),
                    0,
                    "dialog admission itself never publishes data"
                );
            }
        }
    }
}

#[test]
fn export_retained_action_rejects_changed_settings_job_state_and_review_generation() {
    for change in 0..4 {
        let ctx = egui::Context::default();
        let size = Vec2::new(960., 650.);
        let mut d = dialog(Format::Png);
        let original = d.figure.clone();
        let dir = tempfile::tempdir().unwrap();
        let admitted =
            release_export_action_with_discard(&ctx, &mut d, size, "Choose file\u{2026}", |d| {
                match change {
                    0 => d.settings.format = Format::Eps,
                    1 => d.cancel_action(),
                    2 => d.saving = true,
                    _ => d.review(
                        export_batch::prepare(
                            export_batch::Prepare {
                                folder: dir.path().into(),
                                figures: vec![(1, d.figure.clone())],
                                settings: d.settings,
                            },
                            &AtomicBool::new(false),
                        )
                        .unwrap(),
                    ),
                }
            });
        assert!(admitted.is_empty());
        assert!(d.figure.same_snapshot(&original));
        assert_eq!(std::fs::read_dir(dir.path()).unwrap().count(), 0);
    }
}

#[test]
fn workspace_late_stop_close_replacement_busy_and_quit_retire_picker_before_final_layout() {
    for retire in 0..6 {
        let ctx = egui::Context::default();
        let size = Vec2::new(960., 650.);
        let mut app = Workspace::with_context(ctx.clone(), true);
        app.figure_export = Some(dialog(Format::Png));
        for _ in 0..4 {
            frame(&ctx, app.figure_export.as_mut().unwrap(), size, vec![]);
        }
        let (output, _) = frame(&ctx, app.figure_export.as_mut().unwrap(), size, vec![]);
        let point = visible(&output, "Choose file\u{2026}", size)
            .unwrap()
            .center();
        let button = |pressed| {
            vec![
                Event::PointerMoved(point),
                Event::PointerButton {
                    pos: point,
                    button: egui::PointerButton::Primary,
                    pressed,
                    modifiers: egui::Modifiers::NONE,
                },
            ]
        };
        frame(
            &ctx,
            app.figure_export.as_mut().unwrap(),
            size,
            button(true),
        );
        let mut pass = 0;
        crate::headless::run(
            &ctx,
            egui::RawInput {
                screen_rect: Some(Rect::from_min_size(Pos2::ZERO, size)),
                events: button(false),
                ..Default::default()
            },
            |ui| {
                if pass == 0 {
                    ui.ctx()
                        .request_discard("late export owner/admission change");
                }
                assert_eq!(app.figure_export_action(ui.ctx()), Action::None);
                if pass == 0 {
                    assert!(app.figure_export.as_ref().unwrap().action.is_some());
                    match retire {
                        0 => app.stop_work(),
                        1 => app.figure_export = None,
                        2 => app.figure_export = Some(dialog(Format::Eps)),
                        3 => app.busy = true,
                        4 => app.pending_quit = true,
                        _ => app.allow_close = true,
                    }
                }
                pass += 1;
            },
        );
        assert!(pass >= 2);
        assert!(
            app.figure_export
                .as_ref()
                .is_none_or(|d| d.action.is_none())
        );
        // Removing the late blocker must not replay a retired gesture.
        app.busy = false;
        app.pending_quit = false;
        app.allow_close = false;
        crate::headless::run(
            &ctx,
            egui::RawInput {
                screen_rect: Some(Rect::from_min_size(Pos2::ZERO, size)),
                ..Default::default()
            },
            |ui| assert_eq!(app.figure_export_action(ui.ctx()), Action::None),
        );
        if retire == 2 {
            assert_eq!(
                app.figure_export.as_ref().unwrap().settings.format,
                Format::Eps
            );
        }
    }
}
