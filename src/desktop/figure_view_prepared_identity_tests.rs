use super::*;
use crate::figure_geometry::{Geometry, Ordinary};
use std::sync::atomic::AtomicBool;

fn prepared(id: u32, source: &str) -> (Figure, Ordinary) {
    let mut engine = matrust::Engine::default();
    engine.execute(source).unwrap();
    let figure = engine.figure.unwrap();
    let Geometry::Ordinary(geometry) =
        Geometry::prepare(id, &figure, &AtomicBool::new(false)).unwrap()
    else {
        panic!("ordinary fixture")
    };
    (figure, geometry)
}

fn two_plots(
    ctx: &egui::Context,
    figures: [&Figure; 2],
    geometry: [&Ordinary; 2],
    ids: [egui::Id; 2],
    order: [usize; 2],
    reset: bool,
    events: Vec<egui::Event>,
) -> [View; 2] {
    let mut views = [None, None];
    let output = crate::headless::run(
        ctx,
        egui::RawInput {
            screen_rect: Some(egui::Rect::from_min_size(
                egui::Pos2::ZERO,
                egui::vec2(1200., 620.),
            )),
            events,
            ..Default::default()
        },
        |root| {
            assert_eq!(root.ctx().viewport_id(), egui::ViewportId::ROOT);
            egui::CentralPanel::default().show(root, |ui| {
                let area = ui.available_rect_before_wrap();
                let width = (area.width() - 16.) / 2.;
                for (slot, &index) in order.iter().enumerate() {
                    let rect = egui::Rect::from_min_size(
                        area.min + egui::vec2(slot as f32 * (width + 16.), 0.),
                        egui::vec2(width, area.height()),
                    );
                    let mut child = ui.new_child(egui::UiBuilder::new().max_rect(rect));
                    child.set_clip_rect(rect.intersect(ui.clip_rect()));
                    views[index] = Some(show_interactive_prepared_with_id(
                        &mut child,
                        ids[index],
                        figures[index],
                        geometry[index].view(),
                        reset,
                        None,
                    ));
                }
            });
        },
    );
    assert!(!output.shapes.is_empty());
    views.map(Option::unwrap)
}

#[test]
fn two_prepared_root_plots_keep_independent_pointer_zoom_and_reordered_memory() {
    let (a, ga) = prepared(1, "plot([0 5 10],[0 2 4]);legend('Left');");
    let (b, gb) = prepared(
        2,
        "plot([10 20 30],[100 200 300]);axis([10 30 100 300]);legend('Right');",
    );
    let originals = [a.clone(), b.clone()];
    let ids = [
        egui::Id::new(("tile", 1u32, 7u64)),
        egui::Id::new(("tile", 2u32, 9u64)),
    ];
    let points = [&ga, &gb].map(|g| {
        g.points
            .iter()
            .flatten()
            .map(|p| [p.x.to_bits(), p.y.to_bits()])
            .collect::<Vec<_>>()
    });
    for theme in [egui::Theme::Light, egui::Theme::Dark] {
        let ctx = egui::Context::default();
        ctx.set_theme(theme);
        for pass in 0..4 {
            two_plots(&ctx, [&a, &b], [&ga, &gb], ids, [0, 1], pass == 0, vec![]);
        }
        assert_eq!(
            ctx.global_style().visuals.dark_mode,
            theme == egui::Theme::Dark
        );
        let before_a = egui_plot::PlotMemory::load(&ctx, ids[0]).unwrap();
        let before_b = egui_plot::PlotMemory::load(&ctx, ids[1]).unwrap();
        assert!(
            !before_a
                .transform()
                .frame()
                .intersects(*before_b.transform().frame())
        );
        let pointer = before_a.transform().frame().center();
        two_plots(
            &ctx,
            [&a, &b],
            [&ga, &gb],
            ids,
            [0, 1],
            false,
            vec![egui::Event::PointerMoved(pointer)],
        );
        let views = two_plots(
            &ctx,
            [&a, &b],
            [&ga, &gb],
            ids,
            [0, 1],
            false,
            vec![egui::Event::PointerMoved(pointer), egui::Event::Zoom(2.)],
        );
        let zoomed_a = egui_plot::PlotMemory::load(&ctx, ids[0]).unwrap();
        let untouched_b = egui_plot::PlotMemory::load(&ctx, ids[1]).unwrap();
        assert!(zoomed_a.bounds().width() < before_a.bounds().width());
        assert!(zoomed_a.bounds().height() < before_a.bounds().height());
        assert_eq!(views[0].auto, [false; 2]);
        assert_eq!(untouched_b.bounds(), before_b.bounds());
        assert_eq!(untouched_b.auto_bounds, before_b.auto_bounds);
        assert_eq!(untouched_b.hidden_items, before_b.hidden_items);
        for _ in 0..3 {
            two_plots(
                &ctx,
                [&a, &b],
                [&ga, &gb],
                ids,
                [1, 0],
                false,
                vec![egui::Event::PointerGone],
            );
            assert_eq!(
                egui_plot::PlotMemory::load(&ctx, ids[0]).unwrap().bounds(),
                zoomed_a.bounds()
            );
            assert_eq!(
                egui_plot::PlotMemory::load(&ctx, ids[1]).unwrap().bounds(),
                before_b.bounds()
            );
        }
        assert!(egui_plot::PlotMemory::load(&ctx, egui::Id::new("workspace_figure")).is_none());
        assert!(
            egui_plot::PlotMemory::load(
                &ctx,
                egui::Id::new(("detached_figure", egui::ViewportId::ROOT))
            )
            .is_none()
        );
    }
    for (i, (figure, geometry)) in [(&a, &ga), (&b, &gb)].into_iter().enumerate() {
        assert!(figure.same_snapshot(&originals[i]));
        assert!(
            figure.series[0]
                .points
                .same_storage(&originals[i].series[0].points)
        );
        assert_eq!(
            geometry
                .points
                .iter()
                .flatten()
                .map(|p| [p.x.to_bits(), p.y.to_bits()])
                .collect::<Vec<_>>(),
            points[i]
        );
    }
}

#[test]
fn existing_ordinary_and_prepared_wrappers_keep_their_original_plot_ids() {
    let (figure, geometry) = prepared(1, "plot([1 2 3],[4 5 6]);axis([1 3 4 6]);");
    for prepared_path in [false, true] {
        let ctx = egui::Context::default();
        let expected = if prepared_path {
            egui::Id::new(("detached_figure", egui::ViewportId::ROOT))
        } else {
            egui::Id::new("workspace_figure")
        };
        for pass in 0..3 {
            crate::headless::run(
                &ctx,
                egui::RawInput {
                    screen_rect: Some(egui::Rect::from_min_size(
                        egui::Pos2::ZERO,
                        egui::vec2(800., 550.),
                    )),
                    ..Default::default()
                },
                |root| {
                    egui::CentralPanel::default().show(root, |ui| {
                        let view = if prepared_path {
                            show_interactive_prepared(ui, &figure, geometry.view(), pass == 0, None)
                        } else {
                            show_interactive(
                                ui,
                                &figure,
                                &geometry.points,
                                geometry.transforms,
                                pass == 0,
                                None,
                            )
                        };
                        assert_eq!(view.bounds, [1., 3., 4., 6.]);
                    });
                },
            );
        }
        assert!(egui_plot::PlotMemory::load(&ctx, expected).is_some());
        let other = if prepared_path {
            egui::Id::new("workspace_figure")
        } else {
            egui::Id::new(("detached_figure", egui::ViewportId::ROOT))
        };
        assert!(egui_plot::PlotMemory::load(&ctx, other).is_none());
    }
}
