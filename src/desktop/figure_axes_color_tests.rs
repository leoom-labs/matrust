use super::*;
use crate::figure_geometry::{Geometry, scene};
use crate::figure_target::Key;
use matrust::plot::{AxesColors, AxesGridStyle, ColorMode, FigureAppearance, FigureTheme};
use std::sync::atomic::AtomicBool;

fn explicit_colors() -> AxesColors {
    AxesColors {
        legend: None,
        background: Some([0.2, 0.4, 0.6]),
        x: Some([1., 0., 0.]),
        z: None,
        y: Some([0., 0., 1.]),
        title: Some([0., 1., 0.]),
        xlabel: Some([1., 0., 1.]),
        ylabel: Some([0., 1., 1.]),
        grid: Some(AxesGridStyle {
            color: [0.5; 3],
            alpha: 0.5,
            x_mode: ColorMode::Manual,
            y_mode: ColorMode::Manual,
            color_mode: ColorMode::Auto,
            alpha_mode: ColorMode::Manual,
        }),
    }
}

fn walk(shape: &egui::Shape, f: &mut impl FnMut(&egui::Shape)) {
    f(shape);
    if let egui::Shape::Vec(children) = shape {
        for child in children {
            walk(child, f);
        }
    }
}

#[test]
fn explicit_axes_colors_reach_all_prepared_scene_routes_without_changing_zoom_or_storage() {
    for (subplot, logarithmic) in [(false, false), (true, false), (false, true)] {
        let mut engine = matrust::Engine::default();
        engine.execute(if subplot {
            "subplot(1,2,1);plot([0 1 2],[0 2 1]);title('First');subplot(1,2,2);plot([0 1 2],[2 0 1]);title('Second');"
        } else if logarithmic { "plot([1 10 100],[1 100 10]);title('First');" }
        else { "plot([0 1 2],[0 2 1]);title('First');" }).unwrap();
        let mut figure = engine.figure.unwrap();
        for axes in figure.all_axes_mut() {
            axes.colors = Some(explicit_colors());
            axes.xgrid = true;
            axes.ygrid = true;
            axes.xlabel = "X ink".into();
            axes.ylabel = "Y ink".into();
            let limits = if logarithmic { [1., 100.] } else { [0., 4.] };
            axes.xlim = Some(limits);
            axes.ylim = Some(limits);
            if logarithmic {
                axes.xscale = AxisScale::Log;
                axes.yscale = AxisScale::Log;
            }
        }
        let before = figure.clone();
        let key = Key {
            number: 1,
            identity: figure.identity.clone(),
        };
        for route in 0..3 {
            let ctx = egui::Context::default();
            crate::theme::configure(&ctx);
            let identity = scene::SurfaceIdentity::new(key.clone()).unwrap();
            let mut geometry = Geometry::prepare(1, &figure, &AtomicBool::new(false)).unwrap();
            let retained = geometry.retained_bytes();
            let paint = |geometry: &mut Geometry, events| {
                let event_trace = format!("{events:?}");
                crate::headless::run(
                    &ctx,
                    egui::RawInput {
                        screen_rect: Some(egui::Rect::from_min_size(
                            egui::Pos2::ZERO,
                            egui::vec2(960., 650.),
                        )),
                        events,
                        ..Default::default()
                    },
                    |root| {
                        egui::CentralPanel::default().show(root, |ui| {
                            let outcome = scene::show(
                                ui,
                                scene::Frame {
                                    key: &key,
                                    revision: 1,
                                    figure: &figure,
                                    prepared_source: &figure,
                                    marker: None,
                                    restored_view: None,
                                },
                                geometry,
                                false,
                                match route {
                                    0 => scene::Identity::LegacyDetached,
                                    1 => scene::Identity::Explicit(&identity),
                                    _ => scene::Identity::ExplicitFitted(&identity),
                                },
                            )
                            .unwrap();
                            assert!(outcome.matches(&key, 1));
                            let trace = match &outcome.interaction {
                                Some(scene::Interaction::Edit(edit, undoable)) => format!("edit undoable={undoable}, original={:?}, updated={:?}", edit.original, edit.updated),
                                Some(scene::Interaction::Pick(_, _)) => "sample pick".into(),
                                None => "none".into(),
                            };
                            assert!(outcome.interaction.is_none(), "route={route}, subplot={subplot}, log={logarithmic}, dark={}, events={event_trace}, interaction={trace}", ctx.global_style().visuals.dark_mode);
                        });
                    },
                )
            };
            for theme in [egui::Theme::Light, egui::Theme::Dark] {
                ctx.set_theme(theme);
                assert_eq!(
                    ctx.global_style().visuals.dark_mode,
                    theme == egui::Theme::Dark
                );
                for _ in 0..3 {
                    paint(&mut geometry, vec![]);
                }
                let output = paint(&mut geometry, vec![]);
                let mut background = 0;
                let mut title = 0;
                let mut xlabel = 0;
                let mut ylabel = 0;
                let mut red_vertical = 0;
                let mut blue_horizontal = 0;
                let mut red_ticks = 0;
                let mut blue_ticks = 0;
                for clipped in &output.shapes {
                    walk(&clipped.shape, &mut |shape| match shape {
                        egui::Shape::Rect(rect)
                            if rect.fill == rgb([51, 102, 153]) && rect.rect.width() > 50. =>
                        {
                            background += 1
                        }
                        egui::Shape::Text(text) => {
                            let expected = match text.galley.text() {
                                "First" | "Second" => {
                                    title += 1;
                                    Some(rgb([0, 255, 0]))
                                }
                                "X ink" => {
                                    xlabel += 1;
                                    Some(rgb([255, 0, 255]))
                                }
                                "Y ink" => {
                                    ylabel += 1;
                                    Some(rgb([0, 255, 255]))
                                }
                                _ => None,
                            };
                            if let Some(color) = expected {
                                assert!(
                                    text.galley
                                        .job
                                        .sections
                                        .iter()
                                        .all(|s| s.format.color == color)
                                );
                                assert!(
                                    clipped.clip_rect.contains_rect(text.visual_bounding_rect())
                                );
                            }
                            if text.galley.text().parse::<f64>().is_ok() {
                                if text
                                    .galley
                                    .job
                                    .sections
                                    .iter()
                                    .all(|s| s.format.color == rgb([255, 0, 0]))
                                {
                                    red_ticks += 1;
                                }
                                if text
                                    .galley
                                    .job
                                    .sections
                                    .iter()
                                    .all(|s| s.format.color == rgb([0, 0, 255]))
                                {
                                    blue_ticks += 1;
                                }
                            }
                        }
                        egui::Shape::LineSegment { points, stroke }
                            if stroke.color.a() > 0 && stroke.color.a() <= 128 =>
                        {
                            if points[0].x == points[1].x
                                && stroke.color.r() > 0
                                && stroke.color.g() == 0
                                && stroke.color.b() == 0
                            {
                                red_vertical += 1;
                            }
                            if points[0].y == points[1].y
                                && stroke.color.b() > 0
                                && stroke.color.g() == 0
                                && stroke.color.r() == 0
                            {
                                blue_horizontal += 1;
                            }
                        }
                        _ => {}
                    });
                }
                let count = if subplot { 2 } else { 1 };
                assert!(
                    background >= count && title >= count && xlabel == count && ylabel == count,
                    "route={route} subplot={subplot} theme={theme:?}: {background}/{title}/{xlabel}/{ylabel}"
                );
                assert!(red_vertical > 0 && blue_horizontal > 0);
                assert!(red_ticks > 0 && blue_ticks > 0);
                assert_eq!(geometry.retained_bytes(), retained);
                assert!(figure.same_snapshot(&before));
                for ((_, a), (_, b)) in figure.all_axes().zip(before.all_axes()) {
                    assert!(a.series[0].points.same_storage(&b.series[0].points));
                }
            }
            if !subplot && route != 0 {
                let id = identity.plot_id();
                let memory = egui_plot::PlotMemory::load(&ctx, id).unwrap();
                let pointer = memory.transform().frame().center();
                let before_bounds = *memory.bounds();
                paint(&mut geometry, vec![egui::Event::PointerMoved(pointer)]);
                paint(
                    &mut geometry,
                    vec![egui::Event::PointerMoved(pointer), egui::Event::Zoom(2.)],
                );
                let zoomed = *egui_plot::PlotMemory::load(&ctx, id).unwrap().bounds();
                assert!(zoomed.width() < before_bounds.width());
                ctx.set_theme(egui::Theme::Light);
                paint(&mut geometry, vec![egui::Event::PointerGone]);
                assert_eq!(
                    *egui_plot::PlotMemory::load(&ctx, id).unwrap().bounds(),
                    zoomed
                );
                assert!(figure.same_snapshot(&before));
                assert!(
                    figure.series[0]
                        .points
                        .same_storage(&before.series[0].points)
                );
            }
        }
    }
}

#[test]
fn fixed_legacy_and_auto_grid_intent_is_consistent_and_zero_alpha_decoration_is_noninteractive() {
    let mut figure = Figure {
        axes: matrust::plot::Axes {
            xgrid: true,
            ygrid: true,
            ..Default::default()
        },
        ..Default::default()
    };
    for (appearance, modern) in [
        (None, false),
        (
            Some(FigureAppearance {
                theme: FigureTheme::Light,
                canvas_mode: ColorMode::Manual,
                series_palette: None,
            }),
            false,
        ),
        (Some(FigureAppearance::default()), true),
    ] {
        figure.appearance = appearance;
        let resolved = figure.resolve_appearance(BaseTheme::Light);
        let colors = figure.resolve_colors(resolved);
        assert_eq!(colors.grid_alpha, if modern { 0.15 } else { 1. });
        assert_eq!(
            axes_style::Decoration::new(
                &figure.axes,
                figure.axis_transforms(),
                colors,
                resolved,
                egui::Stroke::new(1., Color32::GRAY),
                [None, None],
            )
            .is_some(),
            modern
        );
    }
    let mut colors = explicit_colors();
    colors.grid.as_mut().unwrap().alpha = 0.;
    figure.colors = Some(colors);
    let resolved = figure.resolve_appearance(BaseTheme::Light);
    let decoration = axes_style::Decoration::new(
        &figure.axes,
        figure.axis_transforms(),
        figure.resolve_colors(resolved),
        resolved,
        egui::Stroke::new(1., Color32::GRAY),
        [None, None],
    )
    .unwrap();
    assert_eq!(decoration.bounds(), PlotBounds::NOTHING);
    assert!(decoration.name().is_empty() && !decoration.allow_hover());
    assert!(matches!(decoration.geometry(), PlotGeometry::None));
}

#[test]
fn single_ruler_label_and_ticks_keep_color_across_host_themes_and_manual_zoom() {
    for direction in 0..2 {
        for mode in [ColorMode::Auto, ColorMode::Manual] {
            for explicit_label in [false, true] {
                let mut engine = matrust::Engine::default();
                engine.execute("plot([0 1 2],[0 2 1]);axis([0 4 0 4]);grid off;xlabel('X only');ylabel('Y only');").unwrap();
                let mut figure = engine.figure.unwrap();
                let mut intent = AxesColors::default();
                let ruler = if direction == 0 {
                    [1., 0., 0.]
                } else {
                    [0., 0., 1.]
                };
                let expected_ruler = if direction == 0 {
                    rgb([255, 0, 0])
                } else {
                    rgb([0, 0, 255])
                };
                if direction == 0 {
                    intent.x = Some(ruler);
                } else {
                    intent.y = Some(ruler);
                }
                if explicit_label {
                    if direction == 0 {
                        intent.xlabel = Some([0., 1., 0.]);
                    } else {
                        intent.ylabel = Some([0., 1., 0.]);
                    }
                }
                let mut grid = AxesGridStyle::from_appearance(
                    figure.resolve_appearance(BaseTheme::Light),
                    true,
                );
                if direction == 0 {
                    grid.x_mode = mode;
                } else {
                    grid.y_mode = mode;
                }
                intent.grid = Some(grid);
                figure.colors = Some(intent);
                let original = figure.clone();
                let key = Key {
                    number: 1,
                    identity: figure.identity.clone(),
                };
                let identity = scene::SurfaceIdentity::new(key.clone()).unwrap();
                let mut geometry = Geometry::prepare(1, &figure, &AtomicBool::new(false)).unwrap();
                let ctx = egui::Context::default();
                crate::theme::configure(&ctx);
                let paint = |geometry: &mut Geometry, events| {
                    crate::headless::run(
                        &ctx,
                        egui::RawInput {
                            screen_rect: Some(egui::Rect::from_min_size(
                                egui::Pos2::ZERO,
                                egui::vec2(960., 650.),
                            )),
                            events,
                            ..Default::default()
                        },
                        |root| {
                            egui::CentralPanel::default().show(root, |ui| {
                                scene::show(
                                    ui,
                                    scene::Frame {
                                        key: &key,
                                        revision: 1,
                                        figure: &figure,
                                        prepared_source: &figure,
                                        marker: None,
                                        restored_view: None,
                                    },
                                    geometry,
                                    false,
                                    scene::Identity::Explicit(&identity),
                                )
                                .unwrap();
                            });
                        },
                    )
                };
                for theme in [egui::Theme::Light, egui::Theme::Dark] {
                    ctx.set_theme(theme);
                    assert_eq!(
                        ctx.global_style().visuals.dark_mode,
                        theme == egui::Theme::Dark
                    );
                    for _ in 0..3 {
                        paint(&mut geometry, vec![]);
                    }
                    let output = paint(&mut geometry, vec![]);
                    let frame = *egui_plot::PlotMemory::load(&ctx, identity.plot_id())
                        .unwrap()
                        .transform()
                        .frame();
                    let mut label_count = 0;
                    let mut ticks = 0;
                    let mut edges = 0;
                    for clipped in &output.shapes {
                        walk(&clipped.shape, &mut |shape| match shape {
                            egui::Shape::Text(text)
                                if text.galley.text()
                                    == if direction == 0 { "X only" } else { "Y only" } =>
                            {
                                label_count += 1;
                                let expected = if explicit_label {
                                    rgb([0, 255, 0])
                                } else {
                                    expected_ruler
                                };
                                assert!(
                                    text.galley
                                        .job
                                        .sections
                                        .iter()
                                        .all(|s| s.format.color == expected)
                                );
                                assert!(
                                    clipped.clip_rect.contains_rect(text.visual_bounding_rect())
                                );
                            }
                            egui::Shape::Text(text)
                                if text.galley.text().parse::<f64>().is_ok()
                                    && text
                                        .galley
                                        .job
                                        .sections
                                        .iter()
                                        .all(|s| s.format.color == expected_ruler) =>
                            {
                                ticks += 1
                            }
                            egui::Shape::LineSegment { points, stroke }
                                if stroke.width > 0. && stroke.color == expected_ruler =>
                            {
                                let span = if direction == 0 {
                                    (points[1].x - points[0].x).abs()
                                } else {
                                    (points[1].y - points[0].y).abs()
                                };
                                if span > frame.size()[direction] - 4. {
                                    edges += 1;
                                }
                            }
                            _ => {}
                        });
                    }
                    assert_eq!(label_count, 1);
                    assert!(
                        ticks > 0 && edges >= 2,
                        "direction={direction} mode={mode:?} theme={theme:?}: {ticks} tick labels, {edges} edges"
                    );
                }
                let id = identity.plot_id();
                let memory = egui_plot::PlotMemory::load(&ctx, id).unwrap();
                let pointer = memory.transform().frame().center();
                let before = *memory.bounds();
                paint(&mut geometry, vec![egui::Event::PointerMoved(pointer)]);
                paint(
                    &mut geometry,
                    vec![egui::Event::PointerMoved(pointer), egui::Event::Zoom(2.)],
                );
                let zoomed = *egui_plot::PlotMemory::load(&ctx, id).unwrap().bounds();
                assert!(zoomed.width() < before.width());
                ctx.set_theme(egui::Theme::Light);
                paint(&mut geometry, vec![egui::Event::PointerGone]);
                assert_eq!(
                    *egui_plot::PlotMemory::load(&ctx, id).unwrap().bounds(),
                    zoomed
                );
                assert!(figure.same_snapshot(&original));
                assert!(
                    figure.series[0]
                        .points
                        .same_storage(&original.series[0].points)
                );
            }
        }
    }
}

#[test]
fn explicit_legend_rgb_reaches_native_and_svg_without_changing_prepared_zoom_or_storage() {
    let mut engine = matrust::Engine::default();
    engine.execute("h=plot([0 1 2],[2 3 1],[0 1 2],[4 1 5]);legend(h,{'Legend first','Legend second'},'AutoUpdate','off','Color',[0.2 0.4 0.6],'TextColor',[1 0 1],'EdgeColor',[0 1 0]);").unwrap();
    let figure = engine.figure.unwrap();
    let before = figure.clone();
    let ctx = egui::Context::default();
    crate::theme::configure(&ctx);
    let key = Key {
        number: 1,
        identity: figure.identity.clone(),
    };
    let identity = scene::SurfaceIdentity::new(key.clone()).unwrap();
    let mut geometry = Geometry::prepare(1, &figure, &AtomicBool::new(false)).unwrap();
    let retained = geometry.retained_bytes();
    let paint = |geometry: &mut Geometry, events| {
        crate::headless::run(
            &ctx,
            egui::RawInput {
                screen_rect: Some(egui::Rect::from_min_size(
                    egui::Pos2::ZERO,
                    egui::vec2(960., 650.),
                )),
                events,
                ..Default::default()
            },
            |root| {
                egui::CentralPanel::default().show(root, |ui| {
                    let outcome = scene::show(
                        ui,
                        scene::Frame {
                            key: &key,
                            revision: 1,
                            figure: &figure,
                            prepared_source: &figure,
                            marker: None,
                            restored_view: None,
                        },
                        geometry,
                        false,
                        scene::Identity::Explicit(&identity),
                    )
                    .unwrap();
                    assert!(outcome.matches(&key, 1));
                    assert!(outcome.interaction.is_none());
                });
            },
        )
    };
    let mut zoom = None;
    for theme in [egui::Theme::Light, egui::Theme::Dark] {
        ctx.set_theme(theme);
        for _ in 0..3 {
            paint(&mut geometry, vec![]);
        }
        let output = paint(&mut geometry, vec![]);
        let mut labels = 0;
        let mut boxes = 0;
        for clipped in &output.shapes {
            walk(&clipped.shape, &mut |shape| match shape {
                egui::Shape::Text(text)
                    if matches!(text.galley.text(), "Legend first" | "Legend second") =>
                {
                    labels += 1;
                    assert!(clipped.clip_rect.contains_rect(text.visual_bounding_rect()));
                    for section in &text.galley.job.sections {
                        let color = text.override_text_color.unwrap_or_else(|| {
                            if section.format.color == egui::Color32::PLACEHOLDER {
                                text.fallback_color
                            } else {
                                section.format.color
                            }
                        });
                        assert_eq!(color, rgb([255, 0, 255]));
                    }
                }
                egui::Shape::Rect(rect) if rect.fill == rgb([51, 102, 153]) => {
                    assert_eq!(rect.stroke.color, rgb([0, 255, 0]));
                    boxes += 1;
                }
                _ => {}
            });
        }
        assert_eq!(labels, 2);
        assert!(boxes > 0);
        let host = if theme == egui::Theme::Dark {
            matrust::plot::BaseTheme::Dark
        } else {
            matrust::plot::BaseTheme::Light
        };
        let svg = figure.svg_with_theme(host);
        assert!(svg.contains("fill=\"#ff00ff\">Legend first</text>"));
        assert!(svg.contains("fill=\"#336699\""));
        assert!(svg.contains("stroke=\"#00ff00\""));
        let memory = egui_plot::PlotMemory::load(&ctx, identity.plot_id()).unwrap();
        if let Some(bounds) = zoom {
            assert_eq!(*memory.bounds(), bounds);
        } else {
            let pointer = memory.transform().frame().center();
            let width = memory.bounds().width();
            paint(&mut geometry, vec![egui::Event::PointerMoved(pointer)]);
            paint(
                &mut geometry,
                vec![egui::Event::PointerMoved(pointer), egui::Event::Zoom(2.)],
            );
            let bounds = *egui_plot::PlotMemory::load(&ctx, identity.plot_id())
                .unwrap()
                .bounds();
            assert!(bounds.width() < width);
            zoom = Some(bounds);
            paint(&mut geometry, vec![egui::Event::PointerGone]);
        }
        assert_eq!(geometry.retained_bytes(), retained);
        assert!(figure.same_snapshot(&before));
        for (a, b) in figure.series.iter().zip(&before.series) {
            assert!(a.points.same_storage(&b.points));
        }
    }
}

#[test]
fn box_off_drops_the_top_and_right_frame_and_minor_grid_paints_dotted_lines() {
    let mut engine = matrust::Engine::default();
    engine
        .execute("plot([0 1 2],[0 2 1]);axis([0 4 0 4]);")
        .unwrap();
    let source = engine.figure.unwrap();
    let key = Key {
        number: 1,
        identity: source.identity.clone(),
    };
    // (frame rects with a stroke, long frame-coloured edge segments,
    //  vertical and horizontal minor dashes at MATLAB's MinorGridAlpha 0.25)
    let paint = |figure: &Figure| -> (usize, usize, usize, usize) {
        let ctx = egui::Context::default();
        crate::theme::configure(&ctx);
        ctx.set_theme(egui::Theme::Light);
        let mut edge = egui::Stroke::NONE;
        let mut geometry = Geometry::prepare(1, figure, &AtomicBool::new(false)).unwrap();
        let mut output = None;
        for _ in 0..4 {
            output = Some(crate::headless::run(
                &ctx,
                egui::RawInput {
                    screen_rect: Some(egui::Rect::from_min_size(
                        egui::Pos2::ZERO,
                        egui::vec2(960., 650.),
                    )),
                    ..Default::default()
                },
                |root| {
                    egui::CentralPanel::default().show(root, |ui| {
                        // The plot frame stroke comes from the figure style.
                        ui.scope(|ui| {
                            crate::theme::apply_figure_style(ui, egui::Theme::Light);
                            edge = ui.visuals().widgets.noninteractive.bg_stroke;
                        });
                        scene::show(
                            ui,
                            scene::Frame {
                                key: &key,
                                revision: 1,
                                figure,
                                prepared_source: figure,
                                marker: None,
                                restored_view: None,
                            },
                            &mut geometry,
                            false,
                            scene::Identity::LegacyDetached,
                        )
                        .unwrap();
                    });
                },
            ));
        }
        let minor_alpha = (matrust::plot::MINOR_GRID_ALPHA * 255.).round() as u8;
        let mut counts = (0, 0, 0, 0);
        for clipped in &output.unwrap().shapes {
            walk(&clipped.shape, &mut |shape| match shape {
                egui::Shape::Rect(rect)
                    if rect.stroke.width > 0.
                        && rect.stroke.color == edge.color
                        && rect.rect.width() > 200. =>
                {
                    counts.0 += 1
                }
                egui::Shape::LineSegment { points, stroke } => {
                    let length = (points[1] - points[0]).length();
                    if *stroke == edge && length > 200. {
                        counts.1 += 1;
                    }
                    if stroke.color.a() == minor_alpha {
                        counts.2 += usize::from(points[0].x == points[1].x);
                        counts.3 += usize::from(points[0].y == points[1].y);
                    }
                }
                _ => {}
            });
        }
        counts
    };
    // Historical frame: egui's stroked plot rect, no separate edges, no minor lines.
    assert_eq!(paint(&source), (1, 0, 0, 0));
    let mut open = source.clone();
    open.axes.box_outline = false;
    // Box off: no stroked frame; the bottom and left axis lines only.
    assert_eq!(paint(&open), (0, 2, 0, 0));
    let mut minor = source.clone();
    minor.axes.minor_grid = [true, false, false];
    let (frame, edges, vertical, horizontal) = paint(&minor);
    assert_eq!((frame, edges, horizontal), (1, 0, 0));
    assert!(vertical > 0, "no dotted x minor grid");
}

/// Composites tessellated egui meshes at one point, in paint order, over
/// white: vertex colours only (no text at the sampled interior points).
fn composite_at(primitives: &[egui::ClippedPrimitive], at: egui::Pos2) -> [u8; 3] {
    let mut rgb = [1_f32; 3];
    for primitive in primitives {
        let egui::epaint::Primitive::Mesh(mesh) = &primitive.primitive else {
            continue;
        };
        if !primitive.clip_rect.contains(at) {
            continue;
        }
        for tri in mesh.indices.as_chunks::<3>().0 {
            let [a, b, c] = [0, 1, 2].map(|k| mesh.vertices[tri[k] as usize]);
            let cross = |p: egui::Pos2, q: egui::Pos2, r: egui::Pos2| {
                (q.x - p.x) * (r.y - p.y) - (q.y - p.y) * (r.x - p.x)
            };
            let area = cross(a.pos, b.pos, c.pos);
            if area.abs() < 1e-6 {
                continue;
            }
            let wa = cross(b.pos, c.pos, at) / area;
            let wb = cross(c.pos, a.pos, at) / area;
            let wc = 1. - wa - wb;
            if wa < 0. || wb < 0. || wc < 0. {
                continue;
            }
            // Premultiplied vertex colours, blended "over".
            let channel = |k: usize| {
                (wa * f32::from(a.color.to_array()[k])
                    + wb * f32::from(b.color.to_array()[k])
                    + wc * f32::from(c.color.to_array()[k]))
                    / 255.
            };
            let alpha = channel(3);
            for (k, value) in rgb.iter_mut().enumerate() {
                *value = channel(k) + *value * (1. - alpha);
            }
        }
    }
    rgb.map(|v| (v.clamp(0., 1.) * 255.).round() as u8)
}

/// Axes `Layer` in an actual headless desktop frame: opaque blue data over
/// the whole box, covering both the explicit decoration (`explicit_grid = true`:
/// red grid, green ticks) and the legacy egui-native grid path
/// (`explicit_grid = false`: `figure.appearance = None`, egui's internal grid
/// at `bottom` and `Decoration`'s Layer-top replacement at `Top`).
#[test]
fn corpus_axes_layer_orders_grid_and_inward_ticks_in_the_desktop_frame() {
    for explicit_grid in [true, false] {
        let mut samples = Vec::new();
        let mut tick_rgb = [0; 3];
        let mut grid_lines = Vec::new();
        for layer in ["bottom", "Top"] {
            let mut engine = matrust::Engine::default();
            engine
                .execute(&format!(
                    "fill([0 10 10 0],[0 0 10 10],[0 0 1],'EdgeColor','none'); axis([0 10 0 10]); grid on;
                     set(gca,'TickDir','in','TickLength',[0.05 0.05],'LineWidth',2,'Layer','{layer}');{}",
                    if explicit_grid {
                        "set(gca,'GridColor',[1 0 0],'GridAlpha',1,'XColor',[0 0.8 0],'YColor',[0 0.8 0]);"
                    } else {
                        ""
                    }
                ))
                .unwrap();
            let mut figure = engine.figure.unwrap();
            if !explicit_grid {
                figure.appearance = None;
            }
            let (axes, points) = crate::graphics_frames::display_points(&figure);
            let ctx = egui::Context::default();
            crate::theme::configure(&ctx);
            ctx.set_theme(egui::Theme::Light);
            let mut out = None;
            for pass in 0..2 {
                out = Some(crate::headless::run(
                    &ctx,
                    egui::RawInput {
                        screen_rect: Some(egui::Rect::from_min_size(
                            egui::Pos2::ZERO,
                            egui::vec2(640., 480.),
                        )),
                        ..Default::default()
                    },
                    |root| {
                        egui::CentralPanel::default().show(root, |ui| {
                            let _ = show_interactive(ui, &figure, &points, axes, pass == 0, None);
                        });
                    },
                ));
            }
            let out = out.unwrap();
            // Locate the grid and ticks from the painted segments only; the
            // colours below come from compositing the whole frame.
            let mut segments = Vec::new();
            for clipped in &out.shapes {
                walk(&clipped.shape, &mut |shape| {
                    if let egui::Shape::LineSegment { points, stroke } = shape {
                        segments.push((*points, *stroke));
                    }
                });
            }
            let tick_width = 2. * 4. / 3.;
            let ticks: Vec<_> = segments
                .iter()
                .filter(|(p, s)| {
                    (s.width - tick_width).abs() < 1e-3
                        && p[0].x == p[1].x
                        && (p[0].y - p[1].y).abs() > 10.
                        && (p[0].y - p[1].y).abs() < 100.
                })
                .copied()
                .collect();
            let grid_color = |c: Color32| {
                if explicit_grid {
                    c.r() > 0 && c.g() == 0 && c.b() == 0
                } else {
                    c.a() > 0 && c.r() == c.g() && c.g() == c.b()
                }
            };
            let grid = |vertical: bool| {
                let mut v: Vec<f32> = segments
                    .iter()
                    .filter(|(p, s)| {
                        grid_color(s.color)
                            && (s.width - 1.).abs() < 1e-3
                            && if vertical {
                                p[0].x == p[1].x && (p[0].y - p[1].y).abs() > 100.
                            } else {
                                p[0].y == p[1].y && (p[0].x - p[1].x).abs() > 100.
                            }
                    })
                    .map(|(p, _)| if vertical { p[0].x } else { p[0].y })
                    .collect();
                v.sort_by(f32::total_cmp);
                v.dedup();
                v
            };
            let (columns, rows) = (grid(true), grid(false));
            // The same grid lines and colours in both modes (egui's own grid
            // for 'bottom', its replacement after the children for 'top').
            let mut lines: Vec<_> = segments
                .iter()
                .filter(|(_, s)| grid_color(s.color) && (s.width - 1.).abs() < 1e-3)
                .map(|(p, s)| format!("{p:?} {:?}", s.color))
                .collect();
            lines.sort();
            grid_lines.push(lines);
            assert!(
                columns.len() >= 4 && rows.len() >= 4,
                "explicit {explicit_grid} {layer}: {columns:?} {rows:?} {:?}",
                segments.iter().map(|(_, s)| *s).collect::<Vec<_>>()
            );
            // An x tick whose interior column (not the box border) also
            // carries a grid line.
            let inner = &columns[1..columns.len() - 1];
            let (tick, stroke) = ticks
                .iter()
                .find(|(p, _)| inner.iter().any(|x| (x - p[0].x).abs() < 0.01))
                .unwrap_or_else(|| panic!("{ticks:?} {columns:?}"));
            assert_eq!(stroke.color.a(), 255);
            tick_rgb = [stroke.color.r(), stroke.color.g(), stroke.color.b()];
            let x = tick[0].x;
            let near_edge = egui::pos2(x, (tick[0].y + tick[1].y) / 2.);
            let k = columns.iter().position(|c| (c - x).abs() < 0.01).unwrap();
            let next = columns[k + 1];
            let mid_y = (rows[rows.len() / 2 - 1] + rows[rows.len() / 2]) / 2.;
            let primitives = ctx.tessellate(out.shapes.clone(), out.pixels_per_point);
            samples.push([
                composite_at(&primitives, egui::pos2(x, mid_y)),
                composite_at(&primitives, near_edge),
                composite_at(&primitives, egui::pos2((x + next) / 2., mid_y)),
            ]);
        }
        assert_eq!(
            grid_lines[0], grid_lines[1],
            "explicit grid {explicit_grid}"
        );
        let blue = [0, 0, 255];
        assert_eq!(samples[0], [blue; 3], "explicit grid {explicit_grid}");
        let [interior, edge, off] = samples[1];
        assert_eq!(off, blue, "explicit grid {explicit_grid}");
        assert_eq!(edge, tick_rgb, "explicit grid {explicit_grid}");
        if explicit_grid {
            assert_eq!(tick_rgb, [0, 204, 0]);
            // Density-faded red over blue.
            assert!(interior[0] > 100 && interior[1] == 0, "{interior:?}");
        } else {
            // egui's own faint grey grid colour over blue.
            assert!(
                interior[0] > 0 && interior[0] == interior[1],
                "{interior:?}"
            );
        }
        assert!(interior != blue && interior != tick_rgb, "{interior:?}");
    }
}

/// Legacy appearance + manual `xticks([0.3 0.7])`: `Layer 'top'` uses the same
/// custom spacer as egui's internal grid at `'bottom'`, so the grid crossing
/// pixels sit at the manual tick positions in both modes.
#[test]
fn corpus_axes_layer_legacy_manual_ticks_keep_spacer_positions() {
    let mut grid_lines = Vec::new();
    let mut samples = Vec::new();
    for layer in ["bottom", "Top"] {
        let mut engine = matrust::Engine::default();
        engine
            .execute(&format!(
                "plot([0 1],[0 0.05]); axis([0 1 0 1]); grid on; xticks([0.3 0.7]);
                 set(gca,'Layer','{layer}');"
            ))
            .unwrap();
        let mut figure = engine.figure.unwrap();
        figure.appearance = None;
        let (axes, points) = crate::graphics_frames::display_points(&figure);
        let ctx = egui::Context::default();
        crate::theme::configure(&ctx);
        ctx.set_theme(egui::Theme::Light);
        let mut out = None;
        for pass in 0..2 {
            out = Some(crate::headless::run(
                &ctx,
                egui::RawInput {
                    screen_rect: Some(egui::Rect::from_min_size(
                        egui::Pos2::ZERO,
                        egui::vec2(640., 480.),
                    )),
                    ..Default::default()
                },
                |root| {
                    egui::CentralPanel::default().show(root, |ui| {
                        let _ = show_interactive(ui, &figure, &points, axes, pass == 0, None);
                    });
                },
            ));
        }
        let out = out.unwrap();
        let mut segments = Vec::new();
        for clipped in &out.shapes {
            walk(&clipped.shape, &mut |shape| {
                if let egui::Shape::LineSegment { points, stroke } = shape {
                    segments.push((*points, *stroke));
                }
            });
        }
        let is_grey_grid = |c: Color32, w: f32| {
            c.a() > 0 && c.r() == c.g() && c.g() == c.b() && (w - 1.).abs() < 1e-3
        };
        let mut columns: Vec<f32> = segments
            .iter()
            .filter(|(p, s)| {
                is_grey_grid(s.color, s.width) && p[0].x == p[1].x && (p[0].y - p[1].y).abs() > 100.
            })
            .map(|(p, _)| p[0].x)
            .collect();
        columns.sort_by(f32::total_cmp);
        columns.dedup();
        let mut rows: Vec<f32> = segments
            .iter()
            .filter(|(p, s)| {
                is_grey_grid(s.color, s.width) && p[0].y == p[1].y && (p[0].x - p[1].x).abs() > 100.
            })
            .map(|(p, _)| p[0].y)
            .collect();
        rows.sort_by(f32::total_cmp);
        rows.dedup();
        assert_eq!(columns.len(), 2, "{layer}: {columns:?}");
        assert!(rows.len() >= 3, "{layer}: {rows:?}");
        let mut lines: Vec<_> = segments
            .iter()
            .filter(|(_, s)| is_grey_grid(s.color, s.width))
            .map(|(p, s)| format!("{p:?} {:?}", s.color))
            .collect();
        lines.sort();
        grid_lines.push(lines);
        // Sample between two horizontal grid rows in the clear upper half.
        let y_clear = (rows[0] + rows[1]) / 2.;
        let primitives = ctx.tessellate(out.shapes.clone(), out.pixels_per_point);
        samples.push([
            composite_at(&primitives, egui::pos2(columns[0], y_clear)),
            composite_at(&primitives, egui::pos2(columns[1], y_clear)),
            composite_at(
                &primitives,
                egui::pos2((columns[0] + columns[1]) / 2., y_clear),
            ),
        ]);
    }
    assert_eq!(grid_lines[0], grid_lines[1]);
    assert_eq!(samples[0], samples[1]);
    let [at_03, at_07, at_05] = samples[1];
    assert_eq!(at_03, at_07);
    assert_ne!(
        at_03, at_05,
        "grid line must show at manual ticks 0.3 and 0.7"
    );
}

/// Axes `Layer 'top'` draws grid lines and inward ticks above axes overlays
/// such as an opaque filled `rectangle` in a headless desktop frame.
#[test]
fn corpus_axes_layer_top_draws_grid_and_ticks_over_rectangle() {
    let mut samples = Vec::new();
    for layer in ["bottom", "Top"] {
        let mut engine = matrust::Engine::default();
        engine
            .execute(&format!(
                "rectangle('Position',[0 0 10 10],'FaceColor',[0 0 1],'EdgeColor','none');
                 axis([0 10 0 10]); grid on;
                 set(gca,'GridColor',[1 0 0],'GridAlpha',1,'XColor',[0 0.8 0],'YColor',[0 0.8 0], ...
                     'TickDir','in','TickLength',[0.05 0.05],'LineWidth',2,'Layer','{layer}');"
            ))
            .unwrap();
        let figure = engine.figure.unwrap();
        let (axes, points) = crate::graphics_frames::display_points(&figure);
        let ctx = egui::Context::default();
        crate::theme::configure(&ctx);
        ctx.set_theme(egui::Theme::Light);
        let mut out = None;
        for pass in 0..2 {
            out = Some(crate::headless::run(
                &ctx,
                egui::RawInput {
                    screen_rect: Some(egui::Rect::from_min_size(
                        egui::Pos2::ZERO,
                        egui::vec2(640., 480.),
                    )),
                    ..Default::default()
                },
                |root| {
                    egui::CentralPanel::default().show(root, |ui| {
                        let _ = show_interactive(ui, &figure, &points, axes, pass == 0, None);
                    });
                },
            ));
        }
        let out = out.unwrap();
        let mut segments = Vec::new();
        for clipped in &out.shapes {
            walk(&clipped.shape, &mut |shape| {
                if let egui::Shape::LineSegment { points, stroke } = shape {
                    segments.push((*points, *stroke));
                }
            });
        }
        let tick_width = 2. * 4. / 3.;
        let ticks: Vec<_> = segments
            .iter()
            .filter(|(p, s)| {
                (s.width - tick_width).abs() < 1e-3
                    && p[0].x == p[1].x
                    && (p[0].y - p[1].y).abs() > 10.
                    && (p[0].y - p[1].y).abs() < 100.
            })
            .copied()
            .collect();
        let is_red_grid =
            |c: Color32, w: f32| c.r() > 0 && c.g() == 0 && c.b() == 0 && (w - 1.).abs() < 1e-3;
        let grid = |vertical: bool| {
            let mut v: Vec<f32> = segments
                .iter()
                .filter(|(p, s)| {
                    is_red_grid(s.color, s.width)
                        && if vertical {
                            p[0].x == p[1].x && (p[0].y - p[1].y).abs() > 100.
                        } else {
                            p[0].y == p[1].y && (p[0].x - p[1].x).abs() > 100.
                        }
                })
                .map(|(p, _)| if vertical { p[0].x } else { p[0].y })
                .collect();
            v.sort_by(f32::total_cmp);
            v.dedup();
            v
        };
        let (columns, rows) = (grid(true), grid(false));
        assert!(
            columns.len() >= 4 && rows.len() >= 4,
            "{layer}: {columns:?} {rows:?}"
        );
        let inner = &columns[1..columns.len() - 1];
        let (tick, _) = ticks
            .iter()
            .find(|(p, _)| inner.iter().any(|x| (x - p[0].x).abs() < 0.01))
            .unwrap_or_else(|| panic!("{ticks:?} {columns:?}"));
        let x = tick[0].x;
        let near_edge = egui::pos2(x, (tick[0].y + tick[1].y) / 2.);
        let k = columns.iter().position(|c| (c - x).abs() < 0.01).unwrap();
        let next = columns[k + 1];
        let mid_y = (rows[rows.len() / 2 - 1] + rows[rows.len() / 2]) / 2.;
        let primitives = ctx.tessellate(out.shapes.clone(), out.pixels_per_point);
        samples.push([
            composite_at(&primitives, egui::pos2(x, mid_y)),
            composite_at(&primitives, near_edge),
            composite_at(&primitives, egui::pos2((x + next) / 2., mid_y)),
        ]);
    }
    let blue = [0, 0, 255];
    assert_eq!(samples[0], [blue; 3]);
    let [interior, edge, off] = samples[1];
    assert_eq!(off, blue);
    assert_eq!(edge, [0, 204, 0]);
    assert!(interior[0] > 100 && interior[1] == 0, "{interior:?}");
}

fn desktop_segments(script: &str) -> Vec<([egui::Pos2; 2], egui::Stroke)> {
    let mut engine = matrust::Engine::default();
    engine.execute(script).unwrap();
    let figure = engine.figure.unwrap();
    let (axes, points) = crate::graphics_frames::display_points(&figure);
    let ctx = egui::Context::default();
    crate::theme::configure(&ctx);
    ctx.set_theme(egui::Theme::Light);
    let mut out = None;
    for pass in 0..2 {
        out = Some(crate::headless::run(
            &ctx,
            egui::RawInput {
                screen_rect: Some(egui::Rect::from_min_size(
                    egui::Pos2::ZERO,
                    egui::vec2(640., 480.),
                )),
                ..Default::default()
            },
            |root| {
                egui::CentralPanel::default().show(root, |ui| {
                    let _ = show_interactive(ui, &figure, &points, axes, pass == 0, None);
                });
            },
        ));
    }
    let out = out.unwrap();
    let mut segments = Vec::new();
    for clipped in &out.shapes {
        walk(&clipped.shape, &mut |shape| {
            if let egui::Shape::LineSegment { points, stroke } = shape {
                segments.push((*points, *stroke));
            }
        });
    }
    segments
}

/// Modern appearance, both layers: manual `xticks` keep one full-strength x
/// grid line per tick. The fade must not use label precision.
#[test]
fn corpus_axes_layer_explicit_xticks_draw_full_strength_grid_lines() {
    let full = Color32::from_rgb(255, 0, 0);
    let cases = [
        (
            "plot([0 1],[0 1]); axis([0 1 0 1]); grid on; xticks(0:0.25:1);",
            5,
        ),
        (
            "plot([0 30],[0 1]); xlim([0 30]); grid on; xticks([0 10 20 30]);",
            4,
        ),
    ];
    for (script, count) in cases {
        let mut positions = Vec::new();
        for layer in ["bottom", "Top"] {
            let segments = desktop_segments(&format!(
                "{script} set(gca,'GridColor',[1 0 0],'GridAlpha',1,'Layer','{layer}');"
            ));
            let mut columns: Vec<f32> = segments
                .iter()
                .filter(|(p, s)| {
                    s.color == full
                        && (s.width - 1.).abs() < 1e-3
                        && p[0].x == p[1].x
                        && (p[0].y - p[1].y).abs() > 100.
                })
                .map(|(p, _)| p[0].x)
                .collect();
            columns.sort_by(f32::total_cmp);
            columns.dedup();
            assert_eq!(columns.len(), count, "{layer}: {columns:?}");
            positions.push(columns);
        }
        assert_eq!(positions[0], positions[1], "{script}");
    }
}

/// Layer 'top' grid must not cover the yyaxis right axis line or its ticks.
#[test]
fn corpus_axes_layer_top_keeps_the_yyaxis_right_axis_colour() {
    let mut engine = matrust::Engine::default();
    engine
        .execute(
            "yyaxis left; plot([0 1],[0 1]); ylim([0 1]); \
             yyaxis right; plot([0 1],[0 1]); ylim([0 1]); \
             xlim([0 1]); grid on; xticks([0 1]); yticks(0:0.2:1); \
             set(gca,'Layer','top','GridColor',[0 0 1],'GridAlpha',1,'YColor',[1 0 0]);",
        )
        .unwrap();
    let figure = engine.figure.unwrap();
    let (axes, points) = crate::graphics_frames::display_points(&figure);
    let ctx = egui::Context::default();
    crate::theme::configure(&ctx);
    ctx.set_theme(egui::Theme::Light);
    let mut out = None;
    for pass in 0..2 {
        out = Some(crate::headless::run(
            &ctx,
            egui::RawInput {
                screen_rect: Some(egui::Rect::from_min_size(
                    egui::Pos2::ZERO,
                    egui::vec2(640., 480.),
                )),
                ..Default::default()
            },
            |root| {
                egui::CentralPanel::default().show(root, |ui| {
                    let _ = show_interactive(ui, &figure, &points, axes, pass == 0, None);
                });
            },
        ));
    }
    let out = out.unwrap();
    let mut segments = Vec::new();
    for (order, clipped) in out.shapes.iter().enumerate() {
        walk(&clipped.shape, &mut |shape| {
            if let egui::Shape::LineSegment { points, stroke } = shape {
                segments.push((order, *points, *stroke));
            }
        });
    }
    let red = Color32::from_rgb(255, 0, 0);
    let blue = Color32::from_rgb(0, 0, 255);
    let axis = segments
        .iter()
        .find(|(_, p, s)| s.color == red && p[0].x == p[1].x && (p[0].y - p[1].y).abs() > 100.)
        .unwrap_or_else(|| panic!("no right axis line"));
    let last_blue_grid = segments
        .iter()
        .filter(|(_, p, s)| {
            s.color == blue && ((p[0].x - p[1].x).abs() > 100. || (p[0].y - p[1].y).abs() > 100.)
        })
        .map(|(order, _, _)| *order)
        .max()
        .expect("no blue grid lines");
    assert!(
        axis.0 > last_blue_grid,
        "right axis shape ({}) must follow the Layer-top grid ({last_blue_grid})",
        axis.0
    );
    let mut rows: Vec<f32> = segments
        .iter()
        .filter(|(_, p, s)| s.color == blue && p[0].y == p[1].y && (p[0].x - p[1].x).abs() > 100.)
        .map(|(_, p, _)| p[0].y)
        .collect();
    rows.sort_by(f32::total_cmp);
    rows.dedup();
    assert!(rows.len() >= 2, "{rows:?}");
    let mid_y = (rows[0] + rows[1]) / 2.;
    let tick = segments
        .iter()
        .find(|(_, p, s)| {
            s.color == red
                && (p[0].y - p[1].y).abs() < 0.01
                && (p[0].x - p[1].x).abs() > 2.
                && (p[0].x - p[1].x).abs() < 8.
                && (p[0].y - rows[0]).abs() > 1.
                && (p[0].y - rows[rows.len() - 1]).abs() > 1.
        })
        .unwrap_or_else(|| panic!("no right axis tick"));
    // egui's tessellator defaults to `round_line_segments_to_pixels = true`,
    // which snaps a 1 px stroke at an integer coordinate (e.g. 552.0 or 368.0)
    // to the pixel center (+0.5 at 1x DPI). Sampling at the unrounded integer
    // hits the half-alpha AA feather edge instead of the rendered pixel center.
    let mut axis_x = axis.1[0].x;
    axis.2
        .round_center_to_pixel(out.pixels_per_point, &mut axis_x);
    let mut tick_y = tick.1[0].y;
    tick.2
        .round_center_to_pixel(out.pixels_per_point, &mut tick_y);
    let primitives = ctx.tessellate(out.shapes.clone(), out.pixels_per_point);
    let on_axis = composite_at(&primitives, egui::pos2(axis_x, mid_y));
    let on_tick = composite_at(
        &primitives,
        egui::pos2((tick.1[0].x + tick.1[1].x) / 2., tick_y),
    );
    assert_eq!(on_axis, [255, 0, 0], "right axis line covered");
    assert_eq!(on_tick, [255, 0, 0], "right axis tick covered");
}

/// Under Layer 'top', the desktop legend box stays above the grid lines.
#[test]
fn corpus_axes_layer_top_keeps_desktop_legend_above_grid() {
    let mut engine = matrust::Engine::default();
    engine
        .execute(
            "plot([0 10],[0 1],'DisplayName','s1'); axis([0 10 0 10]); \
             grid on; xticks(0:1:10); yticks(0:1:10); \
             set(gca,'Layer','top','GridColor',[1 0 0],'GridAlpha',1); \
             legend('Location','north','Color',[0.2 0.4 0.6],'EdgeColor',[0 1 0]);",
        )
        .unwrap();
    let figure = engine.figure.unwrap();
    let (axes, points) = crate::graphics_frames::display_points(&figure);
    let ctx = egui::Context::default();
    crate::theme::configure(&ctx);
    ctx.set_theme(egui::Theme::Light);
    let mut out = None;
    for pass in 0..3 {
        out = Some(crate::headless::run(
            &ctx,
            egui::RawInput {
                screen_rect: Some(egui::Rect::from_min_size(
                    egui::Pos2::ZERO,
                    egui::vec2(640., 480.),
                )),
                ..Default::default()
            },
            |root| {
                egui::CentralPanel::default().show(root, |ui| {
                    let _ = show_interactive(ui, &figure, &points, axes, pass == 0, None);
                });
            },
        ));
    }
    let out = out.unwrap();
    let legend_fill = rgb([51, 102, 153]);
    let red = Color32::from_rgb(255, 0, 0);
    let mut legend_rect = None;
    let mut grid_cols = Vec::new();
    let mut grid_rows = Vec::new();
    for clipped in &out.shapes {
        walk(&clipped.shape, &mut |shape| match shape {
            egui::Shape::Rect(rect) if rect.fill == legend_fill && rect.rect.width() > 40. => {
                legend_rect = Some(rect.rect);
            }
            egui::Shape::LineSegment { points, stroke }
                if stroke.color == red && (stroke.width - 1.).abs() < 1e-3 =>
            {
                if points[0].x == points[1].x && (points[0].y - points[1].y).abs() > 100. {
                    grid_cols.push((points[0].x, *stroke));
                } else if points[0].y == points[1].y && (points[0].x - points[1].x).abs() > 100. {
                    grid_rows.push(points[0].y);
                }
            }
            _ => {}
        });
    }
    let legend_rect = legend_rect.expect("legend box not rendered");
    grid_rows.sort_by(f32::total_cmp);
    grid_rows.dedup();
    // Pick a vertical red grid line that crosses the right side of the legend
    // box (clear of the legend text/symbol on the left).
    let &(raw_x, stroke) = grid_cols
        .iter()
        .filter(|(x, _)| *x > legend_rect.left() + 4. && *x < legend_rect.right() - 4.)
        .max_by(|a, b| a.0.total_cmp(&b.0))
        .unwrap_or_else(|| {
            panic!("no vertical grid line crosses legend {legend_rect:?}: {grid_cols:?}")
        });
    let mut gx = raw_x;
    stroke.round_center_to_pixel(out.pixels_per_point, &mut gx);
    // Sample inside the legend box (bottom padding strip, clear of text) and
    // outside the legend box on the same grid line.
    let y_in_legend = legend_rect.bottom() - 3.;
    let y_below_legend = grid_rows
        .windows(2)
        .map(|w| (w[0] + w[1]) / 2.)
        .find(|y| *y > legend_rect.bottom() + 10.)
        .expect("no clear y below legend");
    let primitives = ctx.tessellate(out.shapes.clone(), out.pixels_per_point);
    assert_eq!(
        composite_at(&primitives, egui::pos2(gx, y_in_legend)),
        [51, 102, 153],
        "Layer-top grid must not cover the legend background"
    );
    assert_eq!(
        composite_at(&primitives, egui::pos2(gx, y_below_legend)),
        [255, 0, 0],
        "grid line outside the legend must still be visible"
    );
}

/// Deferred right-axis repaint under Layer 'top' uses `ui.clip_rect()`, so
/// tick labels and the right ylabel outside the plot frame remain unclipped.
#[test]
fn corpus_axes_layer_top_yyaxis_right_axis_labels_stay_unclipped() {
    let mut engine = matrust::Engine::default();
    engine
        .execute(
            "yyaxis left; plot([0 1],[0 1]); ylim([0 1]); \
             yyaxis right; plot([0 1],[0 10]); ylim([0 10]); ylabel('Right Y'); \
             xlim([0 1]); grid on; \
             set(gca,'Layer','top','GridColor',[0 0 1],'GridAlpha',1,'YColor',[1 0 0]);",
        )
        .unwrap();
    let figure = engine.figure.unwrap();
    let (axes, points) = crate::graphics_frames::display_points(&figure);
    let ctx = egui::Context::default();
    crate::theme::configure(&ctx);
    ctx.set_theme(egui::Theme::Light);
    let mut out = None;
    for pass in 0..2 {
        out = Some(crate::headless::run(
            &ctx,
            egui::RawInput {
                screen_rect: Some(egui::Rect::from_min_size(
                    egui::Pos2::ZERO,
                    egui::vec2(640., 480.),
                )),
                ..Default::default()
            },
            |root| {
                egui::CentralPanel::default().show(root, |ui| {
                    let _ = show_interactive(ui, &figure, &points, axes, pass == 0, None);
                });
            },
        ));
    }
    let out = out.unwrap();
    let red = Color32::from_rgb(255, 0, 0);
    let mut axis_x = None;
    let mut tick_labels = 0usize;
    let mut right_ylabel = 0usize;
    for clipped in &out.shapes {
        walk(&clipped.shape, &mut |shape| {
            if let egui::Shape::LineSegment { points, stroke } = shape
                && stroke.color == red
                && points[0].x == points[1].x
                && (points[0].y - points[1].y).abs() > 100.
            {
                axis_x = Some(points[0].x);
            }
        });
    }
    let axis_x = axis_x.expect("no right axis line");
    for clipped in &out.shapes {
        walk(&clipped.shape, &mut |shape| {
            if let egui::Shape::Text(text) = shape {
                let is_red = text.galley.job.sections.iter().all(|s| {
                    let color = text.override_text_color.unwrap_or_else(|| {
                        if s.format.color == Color32::PLACEHOLDER {
                            text.fallback_color
                        } else {
                            s.format.color
                        }
                    });
                    color == red
                });
                if !is_red {
                    return;
                }
                let bounds = text.visual_bounding_rect();
                if text.galley.text() == "Right Y" {
                    right_ylabel += 1;
                    assert!(
                        bounds.min.x > axis_x,
                        "right ylabel should sit outside the plot frame: {bounds:?} vs {axis_x}"
                    );
                    assert!(
                        clipped.clip_rect.contains_rect(bounds),
                        "right ylabel clipped: clip {:?} vs {bounds:?}",
                        clipped.clip_rect
                    );
                } else if text.galley.text().parse::<f64>().is_ok() {
                    tick_labels += 1;
                    assert!(
                        bounds.min.x > axis_x,
                        "right tick label {:?} should sit outside the plot frame: {bounds:?} vs {axis_x}",
                        text.galley.text()
                    );
                    assert!(
                        clipped.clip_rect.contains_rect(bounds),
                        "right tick label {:?} clipped: clip {:?} vs {bounds:?}",
                        text.galley.text(),
                        clipped.clip_rect
                    );
                }
            }
        });
    }
    assert!(
        tick_labels >= 3,
        "expected unclipped right-axis tick labels, found {tick_labels}"
    );
    assert_eq!(right_ylabel, 1, "expected unclipped right ylabel");
}

/// Layer 'top': an inward x tick at xlim max lies on `frame.right()` and must
/// not repaint the bottom of the yyaxis right axis. SVG draws the right axis
/// after the top ticks.
#[test]
fn corpus_axes_layer_top_keeps_right_axis_colour_at_xlim_max_tick() {
    let mut engine = matrust::Engine::default();
    engine
        .execute(
            "yyaxis left; plot([0 1],[0 1]); ylim([0 1]); \
             yyaxis right; plot([0 1],[0 1]); ylim([0 1]); \
             xlim([0 1]); grid on; xticks([0 1]); yticks(0:0.2:1); \
             set(gca,'Layer','top','TickDir','in','GridColor',[0 0 1],'GridAlpha',1,\
             'XColor',[0 1 0],'YColor',[1 0 0]);",
        )
        .unwrap();
    let figure = engine.figure.unwrap();
    let (axes, points) = crate::graphics_frames::display_points(&figure);
    let ctx = egui::Context::default();
    crate::theme::configure(&ctx);
    ctx.set_theme(egui::Theme::Light);
    let mut out = None;
    for pass in 0..2 {
        out = Some(crate::headless::run(
            &ctx,
            egui::RawInput {
                screen_rect: Some(egui::Rect::from_min_size(
                    egui::Pos2::ZERO,
                    egui::vec2(640., 480.),
                )),
                ..Default::default()
            },
            |root| {
                egui::CentralPanel::default().show(root, |ui| {
                    let _ = show_interactive(ui, &figure, &points, axes, pass == 0, None);
                });
            },
        ));
    }
    let out = out.unwrap();
    let mut segments = Vec::new();
    for (order, clipped) in out.shapes.iter().enumerate() {
        walk(&clipped.shape, &mut |shape| {
            if let egui::Shape::LineSegment { points, stroke } = shape {
                segments.push((order, *points, *stroke));
            }
        });
    }
    let red = Color32::from_rgb(255, 0, 0);
    let axis = segments
        .iter()
        .find(|(_, p, s)| s.color == red && p[0].x == p[1].x && (p[0].y - p[1].y).abs() > 100.)
        .expect("no right axis line");
    let bottom = axis.1[0].y.max(axis.1[1].y);
    let axis_x_raw = axis.1[0].x;
    let xtick = segments
        .iter()
        .find(|(_, p, s)| {
            s.color != red
                && (p[0].x - p[1].x).abs() < 0.01
                && (p[0].x - axis_x_raw).abs() < 0.75
                && (p[0].y - p[1].y).abs() > 2.
                && (p[0].y - p[1].y).abs() < 30.
                && p[0].y.min(p[1].y) < bottom - 2.
                && p[0].y.max(p[1].y) > bottom - 1.
        })
        .expect("no x tick at xlim max on the right axis");
    assert!(
        xtick.0 < axis.0,
        "right axis shape ({}) must follow the xlim-max x tick ({})",
        axis.0,
        xtick.0
    );
    assert_ne!(
        xtick.2.color, red,
        "x tick colour must differ from the right axis"
    );
    let mut axis_x = axis_x_raw;
    axis.2
        .round_center_to_pixel(out.pixels_per_point, &mut axis_x);
    // Two pixels above the frame bottom, inside the inward tick (~5 px).
    let sample = egui::pos2(axis_x, bottom - 2.);
    let tick_lo = xtick.1[0].y.min(xtick.1[1].y);
    let tick_hi = xtick.1[0].y.max(xtick.1[1].y);
    assert!(
        sample.y > tick_lo && sample.y < tick_hi,
        "sample {sample:?} must sit inside the x tick {tick_lo}..{tick_hi}"
    );
    let primitives = ctx.tessellate(out.shapes.clone(), out.pixels_per_point);
    assert_eq!(
        composite_at(&primitives, sample),
        [255, 0, 0],
        "right axis line covered by the xlim-max x tick at {sample:?}"
    );
}
