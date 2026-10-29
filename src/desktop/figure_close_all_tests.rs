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
fn setup(ctx: &egui::Context, root: &std::path::Path, count: u32, dirty: bool) -> Workspace {
    let mut app = Workspace::with_context(ctx.clone(), true);
    app.run("keep=[1 2 3];".into());
    finish(&mut app);
    for id in 1..=count {
        app.run(format!(
            "figure({id});plot([1 2],[3 4]);title('Saved {id}');"
        ));
        finish(&mut app);
        app.submit(Job::SaveFigure(root.join(format!("{id}.mfig")), None));
        finish(&mut app);
        if dirty {
            app.run(format!("title('Local {id}');"));
            finish(&mut app);
        }
    }
    app.documents[0].text = "Unsaved script retained".into();
    app.documents[0].changed();
    app
}
fn activate(ctx: &egui::Context, app: &mut Workspace, size: Vec2, label: &str) {
    for _ in 0..3 {
        frame(ctx, app, size, vec![]);
    }
    let output = frame(ctx, app, size, vec![]);
    click(ctx, app, size, point(&output, label, size));
}
fn complete(ctx: &egui::Context, app: &mut Workspace) {
    for _ in 0..40 {
        finish(app);
        if app.figure_close.is_none() {
            break;
        }
        frame(ctx, app, Vec2::new(960., 650.), vec![]);
    }
    finish(app);
    assert!(app.figure_close.is_none(), "{}", app.status);
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
        // Prefer the modal/menu layer over the now-visible short-window toolstrip.
        .rev()
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
fn figure_close_all_menu_cancel_then_discard_keeps_scripts_and_workspace() {
    let ctx = egui::Context::default();
    let dir = tempfile::tempdir().unwrap();
    let mut app = setup(&ctx, dir.path(), 16, true);
    // Exercise the actual compact tabs with the legacy Right placement.
    app.layout.workspace_dock = None;
    app.layout.center = None; // Explicit legacy compact-tab fixture.
    app.layout.mode = layout::PaneMode::Split;
    assert!(!app.workspace_on_left());
    let figures = app.all_figures.clone();
    let variable = app.variables["keep"].clone();
    let size = Vec2::new(960., 650.);
    activate(&ctx, &mut app, size, "Plot figure");
    assert!(app.compact_figure);
    activate(&ctx, &mut app, size, "Menu");
    activate(&ctx, &mut app, size, "File");
    activate(&ctx, &mut app, size, "Figures");
    activate(&ctx, &mut app, size, "Close all figures");
    assert!(matches!(
        app.figure_close.as_ref().unwrap().action,
        Closing::All
    ));
    let compact = Vec2::new(480., 320.);
    activate(&ctx, &mut app, compact, "Cancel");
    assert!(app.figure_close.is_none());
    for (id, figure) in &figures {
        assert!(app.all_figures[id].same_snapshot(figure));
    }
    app.request_close_all_figures();
    activate(&ctx, &mut app, compact, "Discard changes");
    finish(&mut app);
    assert!(app.figure_ids.is_empty() && app.all_figures.is_empty() && app.figure.is_none());
    assert!(app.variables["keep"].same_snapshot(&variable));
    assert_eq!(app.documents[0].text, "Unsaved script retained");
    assert!(app.documents[0].dirty);
    assert!(!app.pending_quit && !app.allow_close);
    for id in 1..=16 {
        assert_eq!(
            matrust::files::load_figure(&dir.path().join(format!("{id}.mfig")))
                .unwrap()
                .title,
            format!("Saved {id}")
        );
    }
}
#[test]
fn figure_close_all_saves_every_modified_figure_before_closing() {
    let ctx = egui::Context::default();
    let dir = tempfile::tempdir().unwrap();
    let mut app = setup(&ctx, dir.path(), 3, true);
    app.request_close_all_figures();
    activate(&ctx, &mut app, Vec2::new(960., 650.), "Save all");
    assert_eq!(app.figure_ids.len(), 3);
    complete(&ctx, &mut app);
    assert!(app.figure_ids.is_empty());
    assert!(app.figure_documents.is_empty() && app.figure_views.is_empty());
    assert!(!app.pending_quit && !app.allow_close);
    assert!(app.documents[0].dirty);
    for id in 1..=3 {
        assert_eq!(
            matrust::files::load_figure(&dir.path().join(format!("{id}.mfig")))
                .unwrap()
                .title,
            format!("Local {id}")
        );
    }
}
#[test]
fn figure_close_all_conflict_cancel_preserves_open_figures_and_completed_saves() {
    let ctx = egui::Context::default();
    let dir = tempfile::tempdir().unwrap();
    let mut app = setup(&ctx, dir.path(), 3, true);
    let mut external = app.all_figures[&2].clone();
    external.title = "External".into();
    matrust::files::save_figure(&dir.path().join("2.mfig"), &external).unwrap();
    app.request_close_all_figures();
    activate(&ctx, &mut app, Vec2::new(960., 650.), "Save all");
    for _ in 0..10 {
        finish(&mut app);
        if app.figure_conflict.is_some() {
            break;
        }
        frame(&ctx, &mut app, Vec2::new(960., 650.), vec![]);
    }
    assert!(app.figure_conflict.is_some());
    assert_eq!(app.figure_ids.len(), 3);
    activate(&ctx, &mut app, Vec2::new(480., 320.), "Cancel");
    assert!(app.figure_close.is_none());
    assert!(!app.figure_dirty(1));
    assert!(app.figure_dirty(2) && app.figure_dirty(3));
    for (id, title) in [(1, "Local 1"), (2, "External"), (3, "Saved 3")] {
        assert_eq!(
            matrust::files::load_figure(&dir.path().join(format!("{id}.mfig")))
                .unwrap()
                .title,
            title
        );
    }
}
#[test]
fn figure_close_all_failed_save_stays_open_and_retry_finishes() {
    let ctx = egui::Context::default();
    let dir = tempfile::tempdir().unwrap();
    let sub = dir.path().join("folder");
    std::fs::create_dir(&sub).unwrap();
    let mut app = setup(&ctx, &sub, 1, true);
    std::fs::remove_file(sub.join("1.mfig")).unwrap();
    std::fs::remove_dir(&sub).unwrap();
    app.request_close_all_figures();
    let size = Vec2::new(640., 480.);
    activate(&ctx, &mut app, size, "Save");
    finish(&mut app);
    assert!(app.figure_conflict.is_some());
    activate(&ctx, &mut app, size, "Overwrite figure");
    finish(&mut app);
    assert!(app.figure_conflict.is_some() && app.figure_close.is_some());
    assert_eq!(app.figure_ids, vec![1]);
    assert!(app.figure_dirty(1));
    std::fs::create_dir(&sub).unwrap();
    activate(&ctx, &mut app, size, "Overwrite figure");
    complete(&ctx, &mut app);
    assert!(app.figure_ids.is_empty());
    assert_eq!(
        matrust::files::load_figure(&sub.join("1.mfig"))
            .unwrap()
            .title,
        "Local 1"
    );
}
#[test]
fn figure_close_all_busy_cancel_and_quit_guard_preserve_figures() {
    let ctx = egui::Context::default();
    let dir = tempfile::tempdir().unwrap();
    let mut app = setup(&ctx, dir.path(), 16, true);
    app.request_close_all_figures();
    app.busy = true;
    app.status = "Long operation status. ".repeat(300);
    let size = Vec2::new(480., 320.);
    app.request_quit(&ctx);
    assert!(!app.pending_quit && !app.allow_close);
    activate(&ctx, &mut app, size, "Cancel closing figures");
    assert!(app.worker.cancel.load(Ordering::Relaxed));
    assert!(app.figure_close.is_none());
    assert_eq!(app.figure_ids.len(), 16);
    app.busy = false;
    app.worker.cancel.store(false, Ordering::Relaxed);
    app.request_close_all_figures();
    app.busy = true;
    for _ in 0..3 {
        frame(&ctx, &mut app, size, vec![]);
    }
    frame(&ctx, &mut app, size, vec![escape(true)]);
    assert!(app.figure_close.is_none() && app.worker.cancel.load(Ordering::Relaxed));
    assert_eq!(app.figure_ids.len(), 16);
}
#[test]
fn figure_close_all_clean_file_menu_closes_without_prompt_and_guards_other_work() {
    let ctx = egui::Context::default();
    let dir = tempfile::tempdir().unwrap();
    let mut app = setup(&ctx, dir.path(), 2, false);
    app.busy = true;
    app.request_close_all_figures();
    assert!(app.figure_close.is_none());
    app.busy = false;
    let size = Vec2::new(960., 650.);
    activate(&ctx, &mut app, size, "Menu");
    activate(&ctx, &mut app, size, "File");
    activate(&ctx, &mut app, size, "Figures");
    activate(&ctx, &mut app, size, "Close all figures");
    finish(&mut app);
    assert!(app.figure_close.is_none() && app.figure_ids.is_empty());
    assert!(!app.can_close_all_figures());
    assert!(!app.pending_quit && !app.allow_close);
}
