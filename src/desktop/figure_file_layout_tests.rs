use super::*;
use egui::{Event, Pos2, Rect, Vec2};
use matrust::files::figure_version::DiskVersion;
fn frame(
    ctx: &egui::Context,
    app: &mut Workspace,
    size: Vec2,
    events: Vec<Event>,
) -> egui::FullOutput {
    crate::headless::run(
        ctx,
        egui::RawInput {
            screen_rect: Some(Rect::from_min_size(Pos2::ZERO, size)),
            events,
            ..Default::default()
        },
        |root| {
            let ctx = &root.ctx().clone();
            app.figure_close_ui(ctx);
            app.figure_conflict_ui(ctx);
        },
    )
}
fn setup(ctx: &egui::Context, count: u32) -> Workspace {
    let mut app = Workspace::with_context(ctx.clone(), true);
    app.run(
        (1..=count)
            .map(|id| format!("figure({id});plot([1 2],[3 4]);"))
            .collect(),
    );
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(3);
    while app.busy && std::time::Instant::now() < deadline {
        app.receive();
        std::thread::sleep(std::time::Duration::from_millis(1));
    }
    assert!(!app.busy, "{}", app.status);
    app.status = "A long file operation failure with context. ".repeat(300);
    app
}
fn conflict(app: &mut Workspace, observed: DiskVersion) {
    app.figure_conflict = Some(worker::FigureConflict {
        reload_requested: false,
        id: 1,
        path: std::path::PathBuf::from(format!("C:/{}figure.mfig", "long-folder/".repeat(120))),
        view: None,
        observed,
    });
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
fn point(output: &egui::FullOutput, label: &str, size: Vec2) -> Pos2 {
    let (rect, clip) = output
        .shapes
        .iter()
        .find_map(|s| find(&s.shape, label).map(|r| (r, s.clip_rect)))
        .unwrap_or_else(|| panic!("Missing {label}"));
    assert!(
        clip.expand(0.1).contains_rect(rect),
        "Clipped {label}: {rect:?} in {clip:?}"
    );
    assert!(
        Rect::from_min_size(Pos2::ZERO, size).contains_rect(rect),
        "Outside {label}: {rect:?}"
    );
    rect.center()
}
fn click(ctx: &egui::Context, app: &mut Workspace, size: Vec2, pos: Pos2) {
    for pressed in [true, false] {
        frame(
            ctx,
            app,
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
        );
    }
}
fn escape(pressed: bool) -> Event {
    Event::Key {
        key: egui::Key::Escape,
        physical_key: None,
        pressed,
        repeat: false,
        modifiers: egui::Modifiers::NONE,
    }
}

#[test]
fn figure_file_layout_conflicts_keep_decisions_visible_and_cancel_preserves_figure() {
    for observed in [DiskVersion::Missing, DiskVersion::Unknown] {
        let ctx = egui::Context::default();
        let mut app = setup(&ctx, 1);
        let original = app.figure.clone().unwrap();
        conflict(&mut app, observed);
        for size in [
            Vec2::new(1440., 900.),
            Vec2::new(960., 650.),
            Vec2::new(640., 480.),
            Vec2::new(480., 320.),
        ] {
            for _ in 0..4 {
                frame(&ctx, &mut app, size, vec![]);
            }
            let output = frame(&ctx, &mut app, size, vec![]);
            for label in [
                "Figure 1 changed on disk",
                "Reload figure",
                "Overwrite figure",
                "Save figure as...",
                "Cancel",
            ] {
                point(&output, label, size);
            }
            let rect = ctx
                .memory(|m| m.area_rect(egui::Id::new("figure_disk_conflict")))
                .unwrap();
            assert!(
                Rect::from_min_size(Pos2::ZERO, size).contains_rect(rect),
                "{rect:?} in {size:?}"
            );
        }
        let size = Vec2::new(480., 320.);
        let output = frame(&ctx, &mut app, size, vec![]);
        click(&ctx, &mut app, size, point(&output, "Cancel", size));
        assert!(app.figure_conflict.is_none());
        assert!(app.figure_dirty(1));
        assert!(app.figure.as_ref().unwrap().same_snapshot(&original));
    }
}
#[test]
fn figure_file_layout_sixteen_unsaved_figures_keep_close_actions_visible() {
    let ctx = egui::Context::default();
    let mut app = setup(&ctx, 16);
    app.figure_close = Some(CloseFlow {
        action: Closing::Application,
        saving: false,
    });
    for doc in app.figure_documents.values_mut() {
        doc.path = Some(std::path::PathBuf::from(
            "long-folder/".repeat(120) + "figure.mfig",
        ));
    }
    let originals = app.all_figures.clone();
    for size in [
        Vec2::new(1440., 900.),
        Vec2::new(960., 650.),
        Vec2::new(640., 480.),
        Vec2::new(480., 320.),
    ] {
        for _ in 0..4 {
            frame(&ctx, &mut app, size, vec![]);
        }
        let output = frame(&ctx, &mut app, size, vec![]);
        for label in ["Unsaved figures", "Save all", "Discard changes", "Cancel"] {
            point(&output, label, size);
        }
        let rect = ctx
            .memory(|m| m.area_rect(egui::Id::new("unsaved_figures")))
            .unwrap();
        assert!(
            Rect::from_min_size(Pos2::ZERO, size).contains_rect(rect),
            "{rect:?} in {size:?}"
        );
    }
    frame(&ctx, &mut app, Vec2::new(480., 320.), vec![escape(true)]);
    assert!(app.figure_close.is_none() && !app.allow_close);
    assert_eq!(app.all_figures.len(), 16);
    for (id, original) in originals {
        assert!(app.all_figures[&id].same_snapshot(&original));
    }
}
#[test]
fn figure_file_layout_busy_conflict_escape_requests_stop_and_retains_decision() {
    let ctx = egui::Context::default();
    let mut app = setup(&ctx, 1);
    conflict(&mut app, DiskVersion::Unknown);
    app.busy = true;
    let size = Vec2::new(480., 320.);
    for _ in 0..4 {
        frame(&ctx, &mut app, size, vec![]);
    }
    let output = frame(&ctx, &mut app, size, vec![]);
    point(&output, "Stop file operation", size);
    frame(&ctx, &mut app, size, vec![escape(true)]);
    assert!(app.worker.cancel.load(Ordering::Relaxed));
    assert!(app.figure_conflict.is_some());
    assert!(app.figure_dirty(1));
}
#[test]
fn figure_file_layout_save_all_cancel_stops_remaining_and_keeps_figures() {
    let ctx = egui::Context::default();
    let mut app = setup(&ctx, 16);
    app.figure_close = Some(CloseFlow {
        action: Closing::SaveOnly,
        saving: true,
    });
    app.busy = true;
    let size = Vec2::new(480., 320.);
    for _ in 0..4 {
        frame(&ctx, &mut app, size, vec![]);
    }
    let output = frame(&ctx, &mut app, size, vec![]);
    click(
        &ctx,
        &mut app,
        size,
        point(&output, "Cancel remaining figure saves", size),
    );
    assert!(app.figure_close.is_none());
    assert!(app.worker.cancel.load(Ordering::Relaxed));
    assert_eq!(app.all_figures.len(), 16);
    assert!(!app.allow_close);
}

fn finish(app: &mut Workspace) {
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(3);
    while app.busy && std::time::Instant::now() < deadline {
        app.receive();
        std::thread::sleep(std::time::Duration::from_millis(1));
    }
    assert!(!app.busy, "{}", app.status);
}
#[test]
fn figure_file_layout_compact_conflict_choices_use_real_worker_and_files() {
    for action in ["Reload figure", "Overwrite figure", "Cancel"] {
        let ctx = egui::Context::default();
        let mut app = setup(&ctx, 1);
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("saved.mfig");
        app.submit(Job::SaveFigure(path.clone(), None));
        finish(&mut app);
        let mut external = app.figure.clone().unwrap();
        external.title = "External".into();
        matrust::files::save_figure(&path, &external).unwrap();
        app.run("title('Local');".into());
        finish(&mut app);
        app.submit(Job::SaveFigure(path.clone(), None));
        finish(&mut app);
        assert!(app.figure_conflict.is_some());
        let original = app.figure.clone().unwrap();
        app.status = "File changed on disk. ".repeat(300);
        let size = Vec2::new(480., 320.);
        for _ in 0..4 {
            frame(&ctx, &mut app, size, vec![]);
        }
        let output = frame(&ctx, &mut app, size, vec![]);
        click(&ctx, &mut app, size, point(&output, action, size));
        finish(&mut app);
        assert!(app.figure_conflict.is_none(), "{}", app.status);
        let disk = matrust::files::load_figure(&path).unwrap();
        match action {
            "Reload figure" => {
                assert_eq!(app.figure.as_ref().unwrap().title, "External");
                assert_eq!(disk.title, "External");
                assert!(!app.figure_dirty(1));
            }
            "Overwrite figure" => {
                assert_eq!(app.figure.as_ref().unwrap().title, "Local");
                assert_eq!(disk.title, "Local");
                assert!(!app.figure_dirty(1));
            }
            _ => {
                assert!(app.figure.as_ref().unwrap().same_snapshot(&original));
                assert_eq!(disk.title, "External");
                assert!(app.figure_dirty(1));
            }
        }
    }
}
