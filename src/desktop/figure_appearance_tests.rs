//! Actual prepared paint and queued worker contracts; no model recoloring copies.
use super::{
    figure_geometry::{Geometry, scene},
    figure_target::Key,
    worker::{Job, Worker},
};
use eframe::egui;
use matrust::plot::{BaseTheme, Figure, FigureTheme};
use std::{sync::atomic::AtomicBool, time::Duration};

fn color(rgb: [u8; 3]) -> egui::Color32 {
    egui::Color32::from_rgb(rgb[0], rgb[1], rgb[2])
}
fn large_fill(shape: &egui::Shape, wanted: egui::Color32) -> bool {
    match shape {
        egui::Shape::Rect(r) => r.fill == wanted && r.rect.width() > 100. && r.rect.height() > 100.,
        egui::Shape::Vec(v) => v.iter().any(|s| large_fill(s, wanted)),
        _ => false,
    }
}
struct Painted {
    figure: Figure,
    key: Key,
    geometry: Geometry,
    identity: scene::SurfaceIdentity,
}
impl Painted {
    fn new(figure: Figure) -> Self {
        let key = Key {
            number: 1,
            identity: figure.identity.clone(),
        };
        let geometry = Geometry::prepare(1, &figure, &AtomicBool::new(false)).unwrap();
        let identity = scene::SurfaceIdentity::new(key.clone()).unwrap();
        Self {
            figure,
            key,
            geometry,
            identity,
        }
    }
    fn paint(
        &mut self,
        ctx: &egui::Context,
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
                    let outcome = scene::show(
                        ui,
                        scene::Frame {
                            key: &self.key,
                            revision: 1,
                            figure: &self.figure,
                            prepared_source: &self.figure,
                            marker: None,
                            restored_view: None,
                        },
                        &mut self.geometry,
                        false,
                        if explicit {
                            scene::Identity::Explicit(&self.identity)
                        } else {
                            scene::Identity::LegacyDetached
                        },
                    )
                    .unwrap();
                    assert!(outcome.matches(&self.key, 1));
                    assert!(outcome.interaction.is_none());
                });
            },
        )
    }
}
#[test]
fn native_auto_fixed_and_legacy_appearance_share_prepared_routes_without_sample_changes() {
    for subplot in [false, true] {
        let mut engine = matrust::Engine::default();
        engine.execute(if subplot {
            "subplot(1,2,1);plot([0 1 2],[0 1 0]);subplot(1,2,2);stem([0 1 2],[1 0 1]);sgtitle('Shared appearance');"
        } else { "plot([0 1 2],[0 1 0]);" }).unwrap();
        for policy in 0..5 {
            let mut figure = engine.figure.clone().unwrap();
            match policy {
                0 => {}
                1 => {
                    figure.set_figure_theme(FigureTheme::Light);
                    figure.set_canvas_color([17, 129, 241]);
                }
                2 => {
                    figure.set_canvas_color([255; 3]);
                }
                3 => {
                    figure.set_figure_theme(FigureTheme::Dark);
                }
                _ => {
                    figure.appearance = None;
                }
            }
            let original = figure.clone();
            for explicit in [false, true] {
                let ctx = egui::Context::default();
                crate::theme::configure(&ctx);
                let mut painted = Painted::new(figure.clone());
                let retained = painted.geometry.retained_bytes();
                for (theme, host) in [
                    (egui::Theme::Light, BaseTheme::Light),
                    (egui::Theme::Dark, BaseTheme::Dark),
                ] {
                    ctx.set_theme(theme);
                    assert_eq!(
                        ctx.global_style().visuals.dark_mode,
                        host == BaseTheme::Dark
                    );
                    let mut output = painted.paint(&ctx, explicit, vec![]);
                    for _ in 0..3 {
                        output = painted.paint(&ctx, explicit, vec![]);
                    }
                    let resolved = figure.resolve_appearance(host);
                    assert!(
                        output
                            .shapes
                            .iter()
                            .any(|s| large_fill(&s.shape, color(resolved.canvas))),
                        "canvas: subplot={subplot} policy={policy} explicit={explicit} host={host:?}"
                    );
                    assert!(
                        output
                            .shapes
                            .iter()
                            .any(|s| large_fill(&s.shape, color(resolved.axes))),
                        "axes: subplot={subplot} policy={policy} explicit={explicit} host={host:?}"
                    );
                    if subplot {
                        fn title_color(shape: &egui::Shape, expected: egui::Color32) -> bool {
                            match shape {
                                egui::Shape::Text(t) if t.galley.text() == "Shared appearance" => t
                                    .galley
                                    .job
                                    .sections
                                    .iter()
                                    .all(|s| s.format.color == expected),
                                egui::Shape::Vec(v) => v.iter().any(|s| title_color(s, expected)),
                                _ => false,
                            }
                        }
                        assert!(
                            output
                                .shapes
                                .iter()
                                .any(|s| title_color(&s.shape, color(resolved.shared_title)))
                        );
                    }
                    assert_eq!(painted.geometry.retained_bytes(), retained);
                    assert!(painted.figure.same_snapshot(&original));
                    for ((_, a), (_, b)) in painted.figure.all_axes().zip(original.all_axes()) {
                        for (a, b) in a.series.iter().zip(&b.series) {
                            assert!(a.points.same_storage(&b.points));
                        }
                    }
                }
            }
        }
    }
}
#[test]
fn changing_host_repaints_auto_without_resetting_keyed_zoom_or_figure_identity() {
    let ctx = egui::Context::default();
    crate::theme::configure(&ctx);
    ctx.set_theme(egui::Theme::Light);
    let mut engine = matrust::Engine::default();
    engine.execute("plot([0 1 2],[0 1 0]);").unwrap();
    let original = engine.figure.unwrap();
    let mut painted = Painted::new(original.clone());
    for _ in 0..4 {
        painted.paint(&ctx, true, vec![]);
    }
    let id = painted.identity.plot_id();
    let before = egui_plot::PlotMemory::load(&ctx, id).unwrap();
    let pointer = before.transform().frame().center();
    painted.paint(&ctx, true, vec![egui::Event::PointerMoved(pointer)]);
    painted.paint(
        &ctx,
        true,
        vec![egui::Event::PointerMoved(pointer), egui::Event::Zoom(2.)],
    );
    let zoomed = *egui_plot::PlotMemory::load(&ctx, id).unwrap().bounds();
    assert!(zoomed.width() < before.bounds().width());
    ctx.set_theme(egui::Theme::Dark);
    let output = painted.paint(&ctx, true, vec![egui::Event::PointerGone]);
    assert_eq!(
        *egui_plot::PlotMemory::load(&ctx, id).unwrap().bounds(),
        zoomed
    );
    assert!(output.shapes.iter().any(|s| large_fill(
        &s.shape,
        color(original.resolve_appearance(BaseTheme::Dark).canvas)
    )));
    assert!(painted.figure.same_snapshot(&original));
    assert!(
        painted.figure.series[0]
            .points
            .same_storage(&original.series[0].points)
    );
}
#[test]
fn queued_script_and_reviewed_batch_keep_submission_theme_after_desktop_changes() {
    let ctx = egui::Context::default();
    crate::theme::configure(&ctx);
    ctx.set_theme(egui::Theme::Dark);
    let mut worker = Worker::new(ctx.clone());
    let dir = tempfile::tempdir().unwrap();
    let first_path = dir.path().join("captured.svg");
    let filename = first_path
        .to_str()
        .unwrap()
        .replace('\\', "/")
        .replace('\'', "''");
    let request = worker.request(Job::Script(format!(
        "plot([0 1 2],[0 1 0]);saveas(gcf,'{filename}','svg');"
    )));
    assert_eq!(request.theme, BaseTheme::Dark);
    ctx.set_theme(egui::Theme::Light);
    worker.send.send(request).unwrap();
    let first = worker
        .receive
        .recv_timeout(Duration::from_secs(10))
        .unwrap();
    first.result.unwrap();
    let source = first.figure.unwrap();
    assert_eq!(
        std::fs::read_to_string(first_path).unwrap(),
        source.svg_with_theme(BaseTheme::Dark)
    );
    ctx.set_theme(egui::Theme::Dark);
    let request = worker.request(Job::PrepareExportBatch(Box::new(
        super::export_batch::Prepare {
            folder: dir.path().to_path_buf(),
            figures: vec![(1, source.clone())],
            settings: matrust::session::ExportSettings {
                format: matrust::session::ExportFormat::Svg,
                ..Default::default()
            },
        },
    )));
    ctx.set_theme(egui::Theme::Light);
    worker.send.send(request).unwrap();
    let prepared = worker
        .receive
        .recv_timeout(Duration::from_secs(10))
        .unwrap();
    prepared.result.unwrap();
    let plan = prepared.export_plan.unwrap();
    assert_eq!(plan.theme, BaseTheme::Dark);
    let path = plan.items[0].path.clone();
    let canvas = plan.settings.vector;
    let request = worker.request(Job::ExportBatch(Box::new(super::export_batch::Run {
        plan,
        completed: 0,
        replace: false,
        progress: Default::default(),
    })));
    assert_eq!(request.theme, BaseTheme::Light);
    worker.send.send(request).unwrap();
    let exported = worker
        .receive
        .recv_timeout(Duration::from_secs(10))
        .unwrap();
    exported.result.unwrap();
    assert_eq!(exported.export_outcome.unwrap().completed, 1);
    assert_eq!(
        std::fs::read_to_string(path).unwrap(),
        source
            .svg_on_canvas_with_theme(canvas, BaseTheme::Dark)
            .unwrap()
    );
    assert!(exported.figure.as_ref().unwrap().same_snapshot(&source));
    assert!(
        exported.figure.unwrap().series[0]
            .points
            .same_storage(&source.series[0].points)
    );
    assert_eq!(
        worker.theme(),
        BaseTheme::Light,
        "Completion must not restore an older desktop theme"
    );
    worker.shutdown();
}
