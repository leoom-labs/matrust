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
                egui::Stroke::new(1., Color32::GRAY)
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
