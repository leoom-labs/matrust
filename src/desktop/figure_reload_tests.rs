use super::*;
use egui::{Event, Pos2, Rect, Vec2};
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
        |root| app.ui(root),
    )
}
fn finish(app: &mut Workspace) {
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(3);
    while app.busy && std::time::Instant::now() < deadline {
        app.receive();
        std::thread::sleep(std::time::Duration::from_millis(1));
    }
    assert!(!app.busy, "{}", app.status);
}
fn setup(ctx: &egui::Context, path: &std::path::Path) -> Workspace {
    let mut app = Workspace::with_context(ctx.clone(), true);
    app.run(
        "keep=[1 2 3];figure(1);plot([1 2],[3 4]);title('Saved');figure(2);plot([5 6]);figure(1);"
            .into(),
    );
    finish(&mut app);
    app.submit(Job::SaveFigure(path.to_owned(), None));
    finish(&mut app);
    app
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
fn figure_reload_menu_confirmation_and_success_preserve_other_figures_and_workspace() {
    for ext in ["mfig", "fig"] {
        let ctx = egui::Context::default();
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join(format!("saved.{ext}"));
        let mut app = setup(&ctx, &path);
        // Keep the compact two-sidebar route explicit; fresh Below Files is wider.
        app.layout.workspace_dock = None;
        app.layout.center = None; // Explicit legacy compact-tab fixture.
        app.layout.mode = layout::PaneMode::Split;
        assert!(!app.workspace_on_left());
        let other = app.all_figures[&2].clone();
        let variable = app.variables["keep"].clone();
        let mut external = app.figure.clone().unwrap();
        external.title = "External".into();
        matrust::files::save_figure(&path, &external).unwrap();
        let size = Vec2::new(960., 650.);
        for _ in 0..4 {
            frame(&ctx, &mut app, size, vec![]);
        }
        // Both sidebars leave the center in compact tab mode at this size.
        let output = frame(&ctx, &mut app, size, vec![]);
        click(&ctx, &mut app, size, point(&output, "Plot figure", size));
        assert!(app.compact_figure);
        let output = frame(&ctx, &mut app, size, vec![]);
        click(&ctx, &mut app, size, point(&output, "Menu", size));
        let output = frame(&ctx, &mut app, size, vec![]);
        click(&ctx, &mut app, size, point(&output, "File", size));
        let output = frame(&ctx, &mut app, size, vec![]);
        click(&ctx, &mut app, size, point(&output, "Figures", size));
        let output = frame(&ctx, &mut app, size, vec![]);
        click(
            &ctx,
            &mut app,
            size,
            point(&output, "Reload from disk...", size),
        );
        assert!(app.figure_conflict.as_ref().unwrap().reload_requested);
        assert_eq!(app.figure.as_ref().unwrap().title, "Saved");
        for _ in 0..3 {
            frame(&ctx, &mut app, size, vec![]);
        }
        let output = frame(&ctx, &mut app, size, vec![]);
        click(&ctx, &mut app, size, point(&output, "Reload figure", size));
        finish(&mut app);
        assert!(app.figure_conflict.is_none());
        assert_eq!(app.current_figure, 1);
        assert_eq!(app.figure_ids, vec![1, 2]);
        assert_eq!(app.figure.as_ref().unwrap().title, "External");
        assert!(!app.figure_dirty(1));
        assert!(app.all_figures[&2].same_snapshot(&other));
        assert!(app.variables["keep"].same_snapshot(&variable));
    }
}
#[test]
fn figure_reload_dirty_cancel_retains_edits_views_and_history() {
    let ctx = egui::Context::default();
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("saved.mfig");
    let mut app = setup(&ctx, &path);
    let before = app.figure.clone().unwrap();
    let mut edited = before.clone();
    edited.title = "Local".into();
    edited.xlim = Some([1.1, 1.8]);
    edited.ylim = Some([3.1, 3.8]);
    app.submit(Job::EditFigure(Box::new(worker::FigureEdit {
        id: 1,
        original: before,
        updated: edited,
        view_before: None,
    })));
    finish(&mut app);
    let size = Vec2::new(480., 320.);
    for _ in 0..4 {
        frame(&ctx, &mut app, size, vec![]);
    }
    let original = app.figure.clone().unwrap();
    let history = app.figure_history.clone();
    assert!(history[&1].0 > 0);
    let view = app.view_bounds;
    let disk = std::fs::read(&path).unwrap();
    app.request_figure_reload();
    let size = Vec2::new(480., 320.);
    for _ in 0..4 {
        frame(&ctx, &mut app, size, vec![]);
    }
    let output = frame(&ctx, &mut app, size, vec![]);
    point(&output, "Reload figure 1 from disk?", size);
    point(&output, "Reload figure", size);
    frame(&ctx, &mut app, size, vec![escape(true)]);
    assert!(app.figure_conflict.is_none());
    assert!(app.figure.as_ref().unwrap().same_snapshot(&original));
    assert_eq!(app.figure_history, history);
    assert_eq!(app.view_bounds, view);
    assert_eq!(std::fs::read(path).unwrap(), disk);
    assert!(app.figure_dirty(1));
    frame(&ctx, &mut app, size, vec![escape(false)]);
    app.request_figure_reload();
    for _ in 0..3 {
        frame(&ctx, &mut app, size, vec![]);
    }
    let output = frame(&ctx, &mut app, size, vec![]);
    click(&ctx, &mut app, size, point(&output, "Reload figure", size));
    finish(&mut app);
    assert!(!app.figure_dirty(1));
    assert_eq!(app.figure.as_ref().unwrap().title, "Saved");
    assert_eq!(app.figure_history[&1], (0, 0));
    assert!(app.view_bounds.is_none());
}
#[test]
fn figure_reload_failed_read_keeps_local_figure_and_can_retry() {
    let ctx = egui::Context::default();
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("saved.mfig");
    let mut app = setup(&ctx, &path);
    let saved = std::fs::read(&path).unwrap();
    app.run("title('Local');".into());
    finish(&mut app);
    let original = app.figure.clone().unwrap();
    let size = Vec2::new(640., 480.);
    app.request_figure_reload();
    for broken in [Some(b"not a figure".as_slice()), None] {
        if let Some(bytes) = broken {
            std::fs::write(&path, bytes).unwrap();
        } else {
            std::fs::remove_file(&path).unwrap();
        }
        for _ in 0..4 {
            frame(&ctx, &mut app, size, vec![]);
        }
        let output = frame(&ctx, &mut app, size, vec![]);
        click(&ctx, &mut app, size, point(&output, "Reload figure", size));
        finish(&mut app);
        assert!(app.figure_conflict.as_ref().unwrap().reload_requested);
        assert!(app.figure.as_ref().unwrap().same_snapshot(&original));
        assert!(app.figure_dirty(1));
    }
    std::fs::write(&path, saved).unwrap();
    for _ in 0..3 {
        frame(&ctx, &mut app, size, vec![]);
    }
    let output = frame(&ctx, &mut app, size, vec![]);
    click(&ctx, &mut app, size, point(&output, "Reload figure", size));
    finish(&mut app);
    assert!(app.figure_conflict.is_none());
    assert_eq!(app.figure.as_ref().unwrap().title, "Saved");
    assert!(!app.figure_dirty(1));
    assert_eq!(app.figure_history.get(&1), Some(&(0, 0)));
    assert!(app.view_bounds.is_none());
}
#[test]
fn figure_reload_unavailable_without_source_or_during_other_operations() {
    let ctx = egui::Context::default();
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("saved.mfig");
    let mut app = setup(&ctx, &path);
    assert!(app.can_reload_figure());
    app.busy = true;
    app.request_figure_reload();
    assert!(app.figure_conflict.is_none());
    app.busy = false;
    app.plot_data = Some(plot_data::Dialog::new("keep".into()));
    app.request_figure_reload();
    assert!(app.figure_conflict.is_none());
    app.plot_data = None;
    app.run("figure(2);".into());
    finish(&mut app);
    assert!(!app.can_reload_figure());
    app.request_figure_reload();
    assert!(app.figure_conflict.is_none());
}
