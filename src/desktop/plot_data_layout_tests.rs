use super::*;
use egui::{Event, Pos2, Rect, Vec2};

#[test]
fn complex_line_dialog_describes_components_and_opens_matching_script() {
    for size in [Vec2::new(1440., 900.), Vec2::new(960., 650.)] {
        let ctx = egui::Context::default();
        let mut app = Workspace::with_context(ctx.clone(), true);
        let mut engine = matrust::Engine::default();
        engine.execute("Z=complex([1 2],[3 4]);").unwrap();
        app.variables = engine.workspace();
        app.selected = Some("Z".into());
        let original = app.variables["Z"].clone();
        app.begin_plot_data();
        for _ in 0..4 {
            frame(&ctx, &mut app, size, vec![]);
        }
        let output = frame(&ctx, &mut app, size, vec![]);
        point(&output, "Real part of Y", size);
        point(
            &output,
            "Complex Y: imaginary versus real components.",
            size,
        );
        point(&output, "Create plot", size);
        click(&ctx, &mut app, size, point(&output, "Z", size));
        for _ in 0..3 {
            frame(&ctx, &mut app, size, vec![]);
        }
        let output = frame(&ctx, &mut app, size, vec![]);
        point(&output, "Z  (1 x 2, complex double)", size);
        click(
            &ctx,
            &mut app,
            size,
            point(&output, "Z  (1 x 2, complex double)", size),
        );
        for _ in 0..3 {
            frame(&ctx, &mut app, size, vec![]);
        }
        let output = frame(&ctx, &mut app, size, vec![]);
        click(
            &ctx,
            &mut app,
            size,
            point(&output, "Open plot script", size),
        );
        assert!(app.plot_data.is_none());
        assert!(!app.busy);
        let source = &app.documents[app.active].text;
        assert!(source.contains("plot(Z);"));
        assert!(source.contains("xlabel('real(Z)');"));
        assert!(source.contains("ylabel('imag(Z)');"));
        engine.execute(source).unwrap();
        assert_eq!(
            engine.figure.as_ref().unwrap().series[0]
                .points
                .iter()
                .collect::<Vec<_>>()
                .as_slice(),
            &[[1., 3.], [2., 4.]]
        );
        assert!(app.variables["Z"].same_snapshot(&original));
    }
}

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
        |ctx| app.plot_data_ui(ctx),
    )
}
fn setup(ctx: &egui::Context) -> Workspace {
    let mut app = Workspace::with_context(ctx.clone(), true);
    app.variables
        .insert("signal".into(), matrust::Matrix::scalar(3.).into());
    app.begin_plot_data();
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
fn plot_layout_long_errors_keep_actions_visible_and_cancel_preserves_data() {
    let ctx = egui::Context::default();
    let mut app = setup(&ctx);
    let original = app.variables["signal"].clone();
    app.plot_data.as_mut().unwrap().error = Some("A long worker failure. ".repeat(200));
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
            "Plot workspace data",
            "Create plot",
            "Open plot script",
            "Cancel",
        ] {
            point(&output, label, size);
        }
        let rect = ctx
            .memory(|m| m.area_rect(egui::Id::new("plot_workspace_data")))
            .unwrap();
        assert!(
            Rect::from_min_size(Pos2::ZERO, size).contains_rect(rect),
            "{rect:?} in {size:?}"
        );
    }
    let size = Vec2::new(480., 320.);
    let output = frame(&ctx, &mut app, size, vec![]);
    click(&ctx, &mut app, size, point(&output, "Cancel", size));
    assert!(app.plot_data.is_none());
    assert!(app.variables["signal"].same_snapshot(&original));
    assert!(!app.busy);
}

#[test]
fn plot_layout_search_accepts_text_and_popup_escape_preserves_dialog() {
    let ctx = egui::Context::default();
    let mut app = setup(&ctx);
    for i in 0..1000 {
        app.variables.insert(
            format!("data_{i:04}"),
            matrust::Matrix::scalar(i as f64).into(),
        );
    }
    let size = Vec2::new(640., 480.);
    for _ in 0..4 {
        frame(&ctx, &mut app, size, vec![]);
    }
    let output = frame(&ctx, &mut app, size, vec![]);
    click(&ctx, &mut app, size, point(&output, "signal", size));
    for _ in 0..3 {
        frame(&ctx, &mut app, size, vec![]);
    }
    let output = frame(&ctx, &mut app, size, vec![]);
    click(
        &ctx,
        &mut app,
        size,
        point(&output, "Find name or type", size),
    );
    frame(&ctx, &mut app, size, vec![Event::Text("DATA_0999".into())]);
    let output = frame(&ctx, &mut app, size, vec![]);
    point(&output, "1 of 1001 variables", size);
    point(&output, "data_0999  (1 x 1, double)", size);
    assert_eq!(app.plot_data.as_ref().unwrap().y, "signal");
    click(
        &ctx,
        &mut app,
        size,
        point(&output, "data_0999  (1 x 1, double)", size),
    );
    assert_eq!(app.plot_data.as_ref().unwrap().y, "data_0999");
    assert!(app.plot_data.as_ref().unwrap().x_filter.is_empty());
    for _ in 0..3 {
        frame(&ctx, &mut app, size, vec![]);
    }
    let output = frame(&ctx, &mut app, size, vec![]);
    click(&ctx, &mut app, size, point(&output, "data_0999", size));
    for _ in 0..3 {
        frame(&ctx, &mut app, size, vec![]);
    }

    frame(&ctx, &mut app, size, vec![escape(true)]);
    assert!(app.plot_data.is_some());
    frame(&ctx, &mut app, size, vec![escape(false)]);
    frame(&ctx, &mut app, size, vec![escape(true)]);
    assert!(app.plot_data.is_none());
}

#[test]
fn plot_layout_busy_escape_requests_stop_and_stale_selection_is_rejected() {
    let ctx = egui::Context::default();
    let mut app = setup(&ctx);
    let original = app.variables["signal"].clone();
    app.plot_data.as_mut().unwrap().y = "missing".into();
    assert!(
        app.plot_data
            .as_ref()
            .unwrap()
            .request(&app.variables)
            .err()
            .unwrap()
            .contains("no longer available")
    );
    app.plot_data.as_mut().unwrap().creating = true;
    let size = Vec2::new(480., 320.);
    for _ in 0..4 {
        frame(&ctx, &mut app, size, vec![]);
    }
    let output = frame(&ctx, &mut app, size, vec![]);
    point(&output, "Stop", size);
    frame(&ctx, &mut app, size, vec![escape(true)]);
    assert!(app.worker.cancel.load(Ordering::Relaxed));
    assert!(app.plot_data.as_ref().unwrap().creating);
    assert!(app.variables["signal"].same_snapshot(&original));
}

#[test]
fn plot_layout_filter_matches_names_types_and_empty_queries() {
    assert!(matches_query("data_0999", "DATA_09"));
    assert!(matches_query("double", "DOUBLE"));
    assert!(matches_query("signal", ""));
    assert!(!matches_query("signal", "a longer query"));
    assert!(!matches_query("signal", "\u{1f4ca}"));
}
