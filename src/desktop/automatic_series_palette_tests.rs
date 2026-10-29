//! Real prepared/native paints and existing export transactions, not OS acceptance.
use super::{
    figure_geometry::{Geometry, scene},
    figure_target::Key,
    worker::{Job, Worker},
};
use eframe::egui;
use matrust::{
    Engine,
    plot::{BaseTheme, Figure},
};
use std::{sync::atomic::AtomicBool, time::Duration};

fn paint(
    ctx: &egui::Context,
    figure: &Figure,
    geometry: &mut Geometry,
    key: &Key,
    identity: &scene::SurfaceIdentity,
    explicit: bool,
    events: Vec<egui::Event>,
) -> egui::FullOutput {
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
        |root| {
            egui::CentralPanel::default().show(root, |ui| {
                let result = scene::show(
                    ui,
                    scene::Frame {
                        key,
                        revision: 1,
                        figure,
                        prepared_source: figure,
                        marker: None,
                        restored_view: None,
                    },
                    geometry,
                    false,
                    if explicit {
                        scene::Identity::Explicit(identity)
                    } else {
                        scene::Identity::LegacyDetached
                    },
                )
                .unwrap();
                assert!(result.matches(key, 1));
            });
        },
    )
}
fn colored_strokes(shape: &egui::Shape, color: egui::Color32, region: egui::Rect) -> usize {
    match shape {
        egui::Shape::Vec(shapes) => shapes
            .iter()
            .map(|s| colored_strokes(s, color, region))
            .sum(),
        egui::Shape::LineSegment { stroke, .. }
            if stroke.color == color && region.contains_rect(shape.visual_bounding_rect()) =>
        {
            1
        }
        egui::Shape::Path(p)
            if p.stroke.color == egui::epaint::ColorMode::Solid(color)
                && region.contains_rect(shape.visual_bounding_rect()) =>
        {
            1
        }
        _ => 0,
    }
}
#[test]
fn native_data_and_legend_resolve_same_palette_without_changing_zoom_or_samples() {
    let mut engine = Engine::default();
    engine.execute("plot([0 1 2],[0 1 0]);hold on;plot([0 1 2],[2 1 2],'Color',[17 113 190]/255);legend('automatic','manual');").unwrap();
    let figure = engine.figure.unwrap();
    let original = figure.clone();
    for explicit in [false, true] {
        let ctx = egui::Context::default();
        crate::theme::configure(&ctx);
        let key = Key {
            number: 1,
            identity: figure.identity.clone(),
        };
        let identity = scene::SurfaceIdentity::new(key.clone()).unwrap();
        let mut geometry = Geometry::prepare(1, &figure, &AtomicBool::new(false)).unwrap();
        let retained = geometry.retained_bytes();
        for (theme, host, wanted) in [
            (egui::Theme::Light, BaseTheme::Light, [17, 113, 190]),
            (egui::Theme::Dark, BaseTheme::Dark, [86, 180, 233]),
        ] {
            ctx.set_theme(theme);
            let mut output = paint(
                &ctx,
                &figure,
                &mut geometry,
                &key,
                &identity,
                explicit,
                vec![],
            );
            for _ in 0..3 {
                output = paint(
                    &ctx,
                    &figure,
                    &mut geometry,
                    &key,
                    &identity,
                    explicit,
                    vec![],
                );
            }
            assert_eq!(
                figure
                    .resolve_appearance(host)
                    .series_color(&figure.series[0]),
                wanted
            );
            let color = egui::Color32::from_rgb(wanted[0], wanted[1], wanted[2]);
            assert!(
                output
                    .shapes
                    .iter()
                    .map(|s| colored_strokes(&s.shape, color, egui::Rect::EVERYTHING))
                    .sum::<usize>()
                    >= 2,
                "data and legend route explicit={explicit} host={host:?}"
            );
            let plot_id = if explicit {
                identity.plot_id()
            } else {
                egui::Id::new("figure_plot")
            };
            // Explicit identity is the tiled/surface route; ordinary detached uses its historical ID.
            if explicit {
                let memory = egui_plot::PlotMemory::load(&ctx, plot_id).unwrap();
                let frame = *memory.transform().frame();
                let legend = ctx
                    .data(|d| d.get_temp::<Option<egui::Rect>>(plot_id.with("legend_rect")))
                    .flatten()
                    .unwrap();
                assert!(
                    output
                        .shapes
                        .iter()
                        .map(|s| colored_strokes(&s.shape, color, legend))
                        .sum::<usize>()
                        > 0,
                    "legend {host:?}"
                );
                assert!(
                    output
                        .shapes
                        .iter()
                        .map(|s| colored_strokes(&s.shape, color, frame))
                        .sum::<usize>()
                        > 0,
                    "data {host:?}"
                );
                let before = *memory.bounds();
                if host == BaseTheme::Light {
                    let pointer = frame.center();
                    paint(
                        &ctx,
                        &figure,
                        &mut geometry,
                        &key,
                        &identity,
                        true,
                        vec![egui::Event::PointerMoved(pointer)],
                    );
                    paint(
                        &ctx,
                        &figure,
                        &mut geometry,
                        &key,
                        &identity,
                        true,
                        vec![egui::Event::PointerMoved(pointer), egui::Event::Zoom(2.)],
                    );
                    let zoom = *egui_plot::PlotMemory::load(&ctx, plot_id).unwrap().bounds();
                    assert!(zoom.width() < before.width());
                    ctx.set_theme(egui::Theme::Dark);
                    paint(
                        &ctx,
                        &figure,
                        &mut geometry,
                        &key,
                        &identity,
                        true,
                        vec![egui::Event::PointerGone],
                    );
                    assert_eq!(
                        *egui_plot::PlotMemory::load(&ctx, plot_id).unwrap().bounds(),
                        zoom
                    );
                }
            }
            assert_eq!(geometry.retained_bytes(), retained);
            assert!(figure.same_snapshot(&original));
            assert!(
                figure.series[0]
                    .points
                    .same_storage(&original.series[0].points)
            );
            assert_eq!(
                figure
                    .resolve_appearance(host)
                    .series_color(&figure.series[1]),
                [17, 113, 190]
            );
        }
    }
}

fn replay_rgb(path: &std::path::Path) -> [u8; 3] {
    let mut engine = Engine::default();
    engine
        .execute_document(&std::fs::read_to_string(path).unwrap(), path)
        .unwrap();
    let f = engine.figure.unwrap();
    assert!(f.series[0].explicit_color && f.series[0].auto_palette.is_none());
    f.series[0].color
}
#[test]
fn worker_single_review_and_batch_keep_captured_dark_script_rgb_after_host_changes() {
    let ctx = egui::Context::default();
    crate::theme::configure(&ctx);
    let mut worker = Worker::new(ctx.clone());
    worker
        .send
        .send(worker.request(Job::Script("plot([0 1],[2 3]);".into())))
        .unwrap();
    let first = worker
        .receive
        .recv_timeout(Duration::from_secs(10))
        .unwrap();
    first.result.unwrap();
    let figure = first.figure.unwrap();
    let original = figure.clone();
    let dir = tempfile::tempdir().unwrap();
    let direct = dir.path().join("direct.m");
    ctx.set_theme(egui::Theme::Dark);
    let request = worker.request(Job::ExportSnapshot(Box::new(
        super::worker::ExportRequest {
            path: direct.clone(),
            figure: figure.clone(),
            vector: None,
            width: 960,
            height: 560,
        },
    )));
    assert_eq!(request.theme, BaseTheme::Dark);
    ctx.set_theme(egui::Theme::Light);
    worker.send.send(request).unwrap();
    worker
        .receive
        .recv_timeout(Duration::from_secs(10))
        .unwrap()
        .result
        .unwrap();
    assert_eq!(replay_rgb(&direct), [86, 180, 233]);

    let review = dir.path().join("review.m");
    std::fs::write(&review, b"prior").unwrap();
    let plan = super::single_export::prepare(
        super::single_export::Input::Figure(Box::new(super::worker::ExportRequest {
            path: review.clone(),
            figure: figure.clone(),
            vector: None,
            width: 960,
            height: 560,
        })),
        BaseTheme::Dark,
        &AtomicBool::new(false),
    )
    .unwrap();
    assert!(plan.replaces());
    assert_eq!(std::fs::read(&review).unwrap(), b"prior");
    let result = super::single_export::perform(
        super::single_export::Request {
            ticket: 1,
            action: super::single_export::Action::Commit(plan),
        },
        BaseTheme::Light,
        &AtomicBool::new(false),
    );
    result.result.unwrap();
    assert_eq!(replay_rgb(&review), [86, 180, 233]);

    ctx.set_theme(egui::Theme::Dark);
    let request = worker.request(Job::PrepareExportBatch(Box::new(
        super::export_batch::Prepare {
            folder: dir.path().to_path_buf(),
            figures: vec![(1, figure.clone())],
            settings: matrust::session::ExportSettings {
                format: matrust::session::ExportFormat::MatlabScript,
                ..Default::default()
            },
        },
    )));
    ctx.set_theme(egui::Theme::Light);
    worker.send.send(request).unwrap();
    let finished = worker
        .receive
        .recv_timeout(Duration::from_secs(10))
        .unwrap();
    finished.result.unwrap();
    let plan = finished.export_plan.unwrap();
    assert_eq!(plan.theme, BaseTheme::Dark);
    let output = plan.items[0].path.clone();
    worker
        .send
        .send(
            worker.request(Job::ExportBatch(Box::new(super::export_batch::Run {
                plan,
                completed: 0,
                replace: false,
                progress: Default::default(),
            }))),
        )
        .unwrap();
    let finished = worker
        .receive
        .recv_timeout(Duration::from_secs(10))
        .unwrap();
    finished.result.unwrap();
    assert_eq!(finished.export_outcome.unwrap().completed, 1);
    assert_eq!(replay_rgb(&output), [86, 180, 233]);
    assert!(figure.same_snapshot(&original));
    assert!(
        figure.series[0]
            .points
            .same_storage(&original.series[0].points)
    );
    worker.shutdown();
}

#[test]
fn subplot_palettes_reach_legacy_explicit_and_fitted_prepared_views() {
    let mut engine = Engine::default();
    engine.execute("subplot(1,2,1);plot([0 1],[0 1]);legend('Left automatic');subplot(1,2,2);plot([0 1],[1 0]);legend('Right automatic');").unwrap();
    let figure = engine.figure.unwrap();
    let original = figure.clone();
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
        for (theme, wanted) in [
            (egui::Theme::Light, [17, 113, 190]),
            (egui::Theme::Dark, [86, 180, 233]),
        ] {
            ctx.set_theme(theme);
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
                            let identity = match route {
                                0 => scene::Identity::LegacyDetached,
                                1 => scene::Identity::Explicit(&identity),
                                _ => scene::Identity::ExplicitFitted(&identity),
                            };
                            let result = scene::show(
                                ui,
                                scene::Frame {
                                    key: &key,
                                    revision: 1,
                                    figure: &figure,
                                    prepared_source: &figure,
                                    marker: None,
                                    restored_view: None,
                                },
                                &mut geometry,
                                false,
                                identity,
                            )
                            .unwrap();
                            assert!(result.matches(&key, 1));
                        });
                    },
                ));
            }
            let color = egui::Color32::from_rgb(wanted[0], wanted[1], wanted[2]);
            assert!(
                output
                    .unwrap()
                    .shapes
                    .iter()
                    .map(|s| colored_strokes(&s.shape, color, egui::Rect::EVERYTHING))
                    .sum::<usize>()
                    >= 4,
                "two plotted series and two legends: route={route}, theme={theme:?}"
            );
            if route != 0 {
                for (position, _) in figure.all_axes() {
                    let id = identity.plot_id().with(("axes", position));
                    assert!(
                        egui_plot::PlotMemory::load(&ctx, id).is_some(),
                        "independent subplot memory {position:?}"
                    );
                }
            }
            assert_eq!(geometry.retained_bytes(), retained);
            assert!(figure.same_snapshot(&original));
            for ((p, a), (q, b)) in figure.all_axes().zip(original.all_axes()) {
                assert_eq!(p, q);
                assert!(a.series[0].points.same_storage(&b.series[0].points));
            }
        }
    }
}
