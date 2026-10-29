use super::*;
use crate::figure_geometry::{Geometry, Ordinary};
use matrust::plot::FigureTheme;
use std::sync::atomic::AtomicBool;

fn prepared(title: &str) -> (Figure, Ordinary) {
    let mut engine = matrust::Engine::default();
    engine
        .execute("plot([0 1 2],[0 2 1]);xlabel('Time');ylabel('Amplitude');")
        .unwrap();
    let mut figure = engine.figure.unwrap();
    figure.title = title.into();
    let Geometry::Ordinary(geometry) =
        Geometry::prepare(1, &figure, &AtomicBool::new(false)).unwrap()
    else {
        panic!("ordinary prepared fixture")
    };
    (figure, geometry)
}

fn paint(
    ctx: &egui::Context,
    figure: &Figure,
    geometry: &Ordinary,
    fitted: bool,
    size: egui::Vec2,
    events: Vec<egui::Event>,
) -> (egui::FullOutput, egui::Rect) {
    let mut canvas = egui::Rect::NOTHING;
    let output = crate::headless::run(
        ctx,
        egui::RawInput {
            screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, size)),
            events,
            ..Default::default()
        },
        |root| {
            egui::CentralPanel::default().show(root, |ui| {
                canvas = ui.available_rect_before_wrap();
                let reset =
                    egui_plot::PlotMemory::load(ctx, egui::Id::new("title_scene")).is_none();
                let view = if fitted {
                    show_interactive_prepared_fitted_with_id(
                        ui,
                        egui::Id::new("title_scene"),
                        figure,
                        geometry.view(),
                        reset,
                        None,
                    )
                } else {
                    show_interactive_prepared_with_id(
                        ui,
                        egui::Id::new("title_scene"),
                        figure,
                        geometry.view(),
                        reset,
                        None,
                    )
                };
                assert!(view.bounds.iter().all(|value| value.is_finite()));
            });
        },
    );
    (output, canvas)
}

fn title_shapes(
    shape: &egui::Shape,
    clip: egui::Rect,
    title: &str,
    out: &mut Vec<(egui::Rect, Color32, bool, usize)>,
) {
    match shape {
        egui::Shape::Text(text) if text.galley.text() == title => {
            let ink = text.visual_bounding_rect();
            assert!(
                clip.contains_rect(ink),
                "Title ink must be inside its artwork clip: {ink:?}, {clip:?}"
            );
            out.push((
                ink,
                text.fallback_color,
                text.galley.elided,
                text.galley.rows.len(),
            ));
        }
        egui::Shape::Vec(shapes) => {
            for shape in shapes {
                title_shapes(shape, clip, title, out);
            }
        }
        _ => {}
    }
}

fn contains_background(shape: &egui::Shape, ink: egui::Rect, wanted: Color32) -> bool {
    match shape {
        egui::Shape::Rect(rect) => rect.fill == wanted && rect.rect.contains_rect(ink),
        egui::Shape::Vec(shapes) => shapes.iter().any(|s| contains_background(s, ink, wanted)),
        _ => false,
    }
}

fn check_tick_ink(
    shape: &egui::Shape,
    clip: egui::Rect,
    canvas: egui::Rect,
    title: egui::Rect,
) -> usize {
    match shape {
        egui::Shape::Text(text) if text.galley.text().parse::<f64>().is_ok() => {
            let ink = text.visual_bounding_rect();
            assert!(
                clip.contains_rect(ink) && canvas.contains_rect(ink),
                "Numeric tick ink must fit the actual scene: {:?}, {ink:?}, {clip:?}, {canvas:?}",
                text.galley.text()
            );
            assert!(
                !ink.intersects(title),
                "Tick ink must not collide with title artwork"
            );
            1
        }
        egui::Shape::Vec(shapes) => shapes
            .iter()
            .map(|s| check_tick_ink(s, clip, canvas, title))
            .sum(),
        _ => 0,
    }
}

#[test]
fn ordinary_title_is_single_bounded_resolved_artwork_above_real_plot_in_both_hosts() {
    let title = "Compact coordinates and independent outlines with shared immutable samples";
    let (source, geometry) = prepared(title);
    for host in [egui::Theme::Light, egui::Theme::Dark] {
        for policy in 0..4 {
            let mut figure = source.clone();
            match policy {
                0 => {}
                1 => figure.set_figure_theme(FigureTheme::Light),
                2 => figure.set_figure_theme(FigureTheme::Dark),
                _ => figure.appearance = None,
            }
            let original = figure.clone();
            let pointer = geometry.points[0].as_ptr();
            let ctx = egui::Context::default();
            ctx.set_theme(host);
            for size in [egui::vec2(420., 240.), egui::vec2(720., 420.)] {
                for fitted in [false, true] {
                    for _ in 0..3 {
                        paint(&ctx, &figure, &geometry, fitted, size, vec![]);
                    }
                    let (output, canvas) = paint(&ctx, &figure, &geometry, fitted, size, vec![]);
                    let mut titles = Vec::new();
                    for shape in &output.shapes {
                        title_shapes(&shape.shape, shape.clip_rect, title, &mut titles);
                    }
                    assert_eq!(titles.len(), 1);
                    let (ink, color, elided, rows) = titles[0];
                    assert!(!elided && rows <= 3);
                    let resolved = figure.resolve_appearance(host_theme(&ctx));
                    assert_eq!(color, rgb(resolved.text));
                    assert!(canvas.contains_rect(ink));
                    assert!(output.shapes.iter().any(|shape| contains_background(
                        &shape.shape,
                        ink,
                        rgb(resolved.canvas)
                    )));
                    let memory =
                        egui_plot::PlotMemory::load(&ctx, egui::Id::new("title_scene")).unwrap();
                    let frame = *memory.transform().frame();
                    assert!(ink.bottom() < frame.top());
                    assert!(frame.width() >= 200. && frame.height() >= 80.);
                    assert!(canvas.contains_rect(frame));
                    assert!(figure.same_snapshot(&original));
                    assert!(
                        figure.series[0]
                            .points
                            .same_storage(&original.series[0].points)
                    );
                    assert_eq!(geometry.points[0].as_ptr(), pointer);
                }
            }
        }
    }
}

#[test]
fn title_change_theme_and_fitted_switch_preserve_manual_zoom_and_bounded_long_title() {
    let (mut figure, geometry) = prepared("A title within the figure");
    let ctx = egui::Context::default();
    let size = egui::vec2(420., 240.);
    for _ in 0..3 {
        paint(&ctx, &figure, &geometry, false, size, vec![]);
    }
    let before = egui_plot::PlotMemory::load(&ctx, egui::Id::new("title_scene")).unwrap();
    let pointer = before.transform().frame().center();
    paint(
        &ctx,
        &figure,
        &geometry,
        false,
        size,
        vec![egui::Event::PointerMoved(pointer)],
    );
    paint(
        &ctx,
        &figure,
        &geometry,
        false,
        size,
        vec![egui::Event::PointerMoved(pointer), egui::Event::Zoom(2.)],
    );
    let zoomed = egui_plot::PlotMemory::load(&ctx, egui::Id::new("title_scene")).unwrap();
    let bounds = *zoomed.bounds();
    assert!(bounds.width() < before.bounds().width());
    assert_eq!([zoomed.auto_bounds.x, zoomed.auto_bounds.y], [false; 2]);
    figure.title = "A long ordinary title ".repeat(100);
    let original = figure.clone();
    ctx.set_theme(egui::Theme::Dark);
    let (output, canvas) = paint(
        &ctx,
        &figure,
        &geometry,
        true,
        size,
        vec![egui::Event::PointerGone],
    );
    let mut titles = Vec::new();
    for shape in &output.shapes {
        title_shapes(&shape.shape, shape.clip_rect, &figure.title, &mut titles);
    }
    assert_eq!(titles.len(), 1);
    assert!(titles[0].2 && titles[0].3 <= 3);
    assert!(canvas.contains_rect(titles[0].0));
    let after = egui_plot::PlotMemory::load(&ctx, egui::Id::new("title_scene")).unwrap();
    assert_eq!(*after.bounds(), bounds);
    assert_eq!(after.auto_bounds, zoomed.auto_bounds);
    assert_eq!(after.hidden_items, zoomed.hidden_items);
    assert!(figure.same_snapshot(&original));
    assert!(
        figure.series[0]
            .points
            .same_storage(&original.series[0].points)
    );
}

#[test]
fn fitted_outside_legend_does_not_reintroduce_the_readable_scene_minimum() {
    let mut engine = matrust::Engine::default();
    engine
        .execute("plot([0 1 2],[0 2 1]);title('Signal');legend('Data','Location','eastoutside');")
        .unwrap();
    let figure = engine.figure.unwrap();
    let original = figure.clone();
    let Geometry::Ordinary(geometry) =
        Geometry::prepare(1, &figure, &AtomicBool::new(false)).unwrap()
    else {
        panic!("ordinary fixture")
    };
    let ctx = egui::Context::default();
    let size = egui::vec2(300., 150.);
    for _ in 0..3 {
        paint(&ctx, &figure, &geometry, true, size, vec![]);
    }
    let (output, canvas) = paint(&ctx, &figure, &geometry, true, size, vec![]);
    assert!(canvas.height() < 160.);
    let memory = egui_plot::PlotMemory::load(&ctx, egui::Id::new("title_scene")).unwrap();
    assert!(canvas.contains_rect(*memory.transform().frame()));
    let mut titles = Vec::new();
    for shape in &output.shapes {
        title_shapes(&shape.shape, shape.clip_rect, "Signal", &mut titles);
    }
    assert_eq!(titles.len(), 1);
    assert!(!titles[0].2);
    assert!(figure.same_snapshot(&original));
    assert!(
        figure.series[0]
            .points
            .same_storage(&original.series[0].points)
    );
}

#[test]
fn valid_near_upper_tick_has_complete_ink_without_expanding_manual_data_bounds() {
    for host in [egui::Theme::Light, egui::Theme::Dark] {
        let mut engine = matrust::Engine::default();
        engine.execute("plot([0 1 2],[0 1 0]);axis([0 2 -1 1.01]);title('Valid upper tick');xlabel('Time');ylabel('Amplitude');").unwrap();
        let figure = engine.figure.unwrap();
        let original = figure.clone();
        let Geometry::Ordinary(geometry) =
            Geometry::prepare(1, &figure, &AtomicBool::new(false)).unwrap()
        else {
            panic!("ordinary fixture")
        };
        let ctx = egui::Context::default();
        ctx.set_theme(host);
        for _ in 0..3 {
            paint(
                &ctx,
                &figure,
                &geometry,
                true,
                egui::vec2(680., 280.),
                vec![],
            );
        }
        let (output, canvas) = paint(
            &ctx,
            &figure,
            &geometry,
            true,
            egui::vec2(680., 280.),
            vec![],
        );
        let mut titles = Vec::new();
        for shape in &output.shapes {
            title_shapes(&shape.shape, shape.clip_rect, &figure.title, &mut titles);
        }
        assert_eq!(titles.len(), 1);
        let count: usize = output
            .shapes
            .iter()
            .map(|s| check_tick_ink(&s.shape, s.clip_rect, canvas, titles[0].0))
            .sum();
        assert!(count >= 3);
        let memory = egui_plot::PlotMemory::load(&ctx, egui::Id::new("title_scene")).unwrap();
        assert_eq!(memory.bounds().min(), [0., -1.]);
        assert_eq!(memory.bounds().max(), [2., 1.01]);
        assert_eq!([memory.auto_bounds.x, memory.auto_bounds.y], [false; 2]);
        assert!(figure.same_snapshot(&original));
        assert!(
            figure.series[0]
                .points
                .same_storage(&original.series[0].points)
        );
    }
}

#[test]
fn fixed_and_imported_inner_panels_keep_frame_and_separate_north_legend_from_title() {
    for imported_solved in [false, true] {
        let mut engine = matrust::Engine::default();
        engine.execute("plot([0 1 2],[0 1 0]);axis([0 2 -1 1.01]);title('Fixed axes title');legend('North key','Location','northoutside');").unwrap();
        let mut figure = engine.figure.unwrap();
        figure.placement = Some(matrust::plot::AxesPlacement {
            requested_inner: [0.15, 0.15, 0.7, 0.55],
            requested_outer: [0., 0., 1., 1.],
            constraint: matrust::plot::PositionConstraint::InnerPosition,
            imported_solved,
        });
        let original = figure.clone();
        let Geometry::Ordinary(geometry) =
            Geometry::prepare(1, &figure, &AtomicBool::new(false)).unwrap()
        else {
            panic!("ordinary fixture")
        };
        let ctx = egui::Context::default();
        let size = egui::vec2(720., 420.);
        for _ in 0..4 {
            paint(&ctx, &figure, &geometry, true, size, vec![]);
        }
        let (output, canvas) = paint(&ctx, &figure, &geometry, true, size, vec![]);
        let mut titles = Vec::new();
        for shape in &output.shapes {
            title_shapes(&shape.shape, shape.clip_rect, &figure.title, &mut titles);
        }
        assert_eq!(titles.len(), 1);
        let legend = ctx
            .data(|d| {
                d.get_temp::<Option<egui::Rect>>(egui::Id::new("title_scene").with("legend_rect"))
            })
            .flatten()
            .unwrap();
        assert!(canvas.contains_rect(legend));
        assert!(legend.bottom() < titles[0].0.top());
        let memory = egui_plot::PlotMemory::load(&ctx, egui::Id::new("title_scene")).unwrap();
        let frame = *memory.transform().frame();
        assert!(titles[0].0.bottom() < frame.top());
        let bounds = *memory.bounds();
        let untitled_ctx = egui::Context::default();
        let mut untitled = figure.clone();
        untitled.title.clear();
        for _ in 0..4 {
            paint(&untitled_ctx, &untitled, &geometry, true, size, vec![]);
        }
        let untitled_memory =
            egui_plot::PlotMemory::load(&untitled_ctx, egui::Id::new("title_scene")).unwrap();
        assert_eq!(
            *untitled_memory.transform().frame(),
            frame,
            "Fixed inner panel must not shrink for a title"
        );
        assert_eq!(*untitled_memory.bounds(), bounds);
        // A fixed panel with no top decoration room gets a bounded diagnostic,
        // not clipped artwork or an implicit movement/resize of its data box.
        figure.placement.as_mut().unwrap().requested_inner = [0.15, 0., 0.7, 1.];
        let (unavailable, _) = paint(&ctx, &figure, &geometry, true, size, vec![]);
        let mut diagnostics = Vec::new();
        for shape in &unavailable.shapes {
            title_shapes(
                &shape.shape,
                shape.clip_rect,
                "Axes title needs more room; enlarge the pane or choose Readable size.",
                &mut diagnostics,
            );
        }
        assert_eq!(diagnostics.len(), 1);
        assert_eq!(
            *egui_plot::PlotMemory::load(&ctx, egui::Id::new("title_scene"))
                .unwrap()
                .bounds(),
            bounds
        );
        assert_eq!(
            original.placement.unwrap().requested_inner,
            [0.15, 0.15, 0.7, 0.55]
        );
        assert!(
            figure.series[0]
                .points
                .same_storage(&original.series[0].points)
        );
    }
}
