use super::*;
use std::{
    sync::atomic::AtomicBool,
    time::{Duration, Instant},
};

struct Scene {
    key: Key,
    revision: u64,
    figure: Figure,
    geometry: Geometry,
    identity: SurfaceIdentity,
}
impl Scene {
    fn new(number: u32, revision: u64, figure: &Figure) -> Self {
        let key = Key {
            number,
            identity: figure.identity.clone(),
        };
        let identity = SurfaceIdentity::new(key.clone()).unwrap();
        let geometry = Geometry::prepare(number, figure, &AtomicBool::new(false)).unwrap();
        Self {
            key,
            revision,
            figure: figure.clone(),
            geometry,
            identity,
        }
    }
}
fn finish(app: &mut crate::Workspace) {
    let deadline = Instant::now() + Duration::from_secs(5);
    while app.busy && Instant::now() < deadline {
        app.receive();
        std::thread::sleep(Duration::from_millis(1));
    }
    assert!(!app.busy, "{}", app.status);
}
fn pair(
    ctx: &egui::Context,
    scenes: &mut [Scene; 2],
    order: [usize; 2],
    reset: bool,
    events: Vec<egui::Event>,
) -> (egui::FullOutput, [Outcome; 2]) {
    let mut result = [None, None];
    let output = crate::headless::run(
        ctx,
        egui::RawInput {
            screen_rect: Some(egui::Rect::from_min_size(
                egui::Pos2::ZERO,
                egui::vec2(1600., 800.),
            )),
            events,
            ..Default::default()
        },
        |root| {
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
                    let scene = &mut scenes[index];
                    result[index] = Some(
                        show(
                            &mut child,
                            Frame {
                                key: &scene.key,
                                revision: scene.revision,
                                figure: &scene.figure,
                                prepared_source: &scene.figure,
                                marker: None,
                                restored_view: None,
                            },
                            &mut scene.geometry,
                            reset,
                            Identity::Explicit(&scene.identity),
                        )
                        .unwrap(),
                    );
                }
            });
        },
    );
    (output, result.map(Option::unwrap))
}
fn position(shape: &egui::epaint::Shape, text: &str) -> Option<egui::Pos2> {
    match shape {
        egui::epaint::Shape::Text(t) if t.galley.text() == text => {
            Some(t.pos + t.galley.size() * 0.5)
        }
        egui::epaint::Shape::Vec(v) => v.iter().find_map(|s| position(s, text)),
        _ => None,
    }
}

#[test]
fn two_keyed_figures_paint_and_zoom_without_selecting_or_mutating_workspace() {
    for theme in [egui::Theme::Light, egui::Theme::Dark] {
        let ctx = egui::Context::default();
        let mut app = crate::Workspace::with_context(ctx.clone(), true);
        app.run("keep=[1 2 3];alias=keep;figure(1);plot([0 5 10],[0 2 4]);legend('First');figure(2);semilogx([1 10 100],[2 4 8]);legend('Second');axis([1 100 2 8]);figure(1);".into());
        finish(&mut app);
        ctx.set_theme(theme);
        assert_eq!(app.current_figure, 1);
        let originals = app.all_figures.clone();
        let variables = app.variables.clone();
        let documents = app.figure_documents.clone();
        let history = app.figure_history.clone();
        let current = app.figure.clone().unwrap();
        let mut scenes = [1, 2].map(|id| {
            Scene::new(
                id,
                app.figure_documents[&id].revision,
                &app.all_figures[&id],
            )
        });
        for pass in 0..4 {
            pair(&ctx, &mut scenes, [0, 1], pass == 0, vec![]);
        }
        let a = egui_plot::PlotMemory::load(&ctx, scenes[0].identity.plot_id()).unwrap();
        let b = egui_plot::PlotMemory::load(&ctx, scenes[1].identity.plot_id()).unwrap();
        assert!(!a.transform().frame().intersects(*b.transform().frame()));
        let pointer = b.transform().frame().center();
        pair(
            &ctx,
            &mut scenes,
            [0, 1],
            false,
            vec![egui::Event::PointerMoved(pointer)],
        );
        let (_, outcomes) = pair(
            &ctx,
            &mut scenes,
            [0, 1],
            false,
            vec![egui::Event::PointerMoved(pointer), egui::Event::Zoom(2.)],
        );
        assert!(outcomes[1].matches(&scenes[1].key, scenes[1].revision));
        assert!(!outcomes[1].matches(&scenes[0].key, scenes[0].revision));
        assert_eq!(outcomes[1].view.unwrap().auto, [false; 2]);
        let zoomed = egui_plot::PlotMemory::load(&ctx, scenes[1].identity.plot_id()).unwrap();
        assert!(zoomed.bounds().width() < b.bounds().width());
        assert_eq!(
            egui_plot::PlotMemory::load(&ctx, scenes[0].identity.plot_id())
                .unwrap()
                .bounds(),
            a.bounds()
        );
        for _ in 0..3 {
            pair(
                &ctx,
                &mut scenes,
                [1, 0],
                false,
                vec![egui::Event::PointerGone],
            );
            assert_eq!(
                egui_plot::PlotMemory::load(&ctx, scenes[1].identity.plot_id())
                    .unwrap()
                    .bounds(),
                zoomed.bounds()
            );
        }
        assert_eq!(app.current_figure, 1);
        assert!(app.figure.as_ref().unwrap().same_snapshot(&current));
        assert_eq!(app.figure_documents, documents);
        assert_eq!(app.figure_history, history);
        for (id, source) in &originals {
            assert!(app.all_figures[id].same_snapshot(source));
            assert!(
                app.all_figures[id].series[0]
                    .points
                    .same_storage(&source.series[0].points)
            );
        }
        for (name, source) in &variables {
            assert!(app.variables[name].same_snapshot(source));
        }
        assert!(egui_plot::PlotMemory::load(&ctx, egui::Id::new("workspace_figure")).is_none());
        assert!(scenes[0].identity.forget(&ctx, &scenes[1].figure).is_err());
        assert!(egui_plot::PlotMemory::load(&ctx, scenes[0].identity.plot_id()).is_some());
        scenes[0].identity.forget(&ctx, &scenes[0].figure).unwrap();
        assert!(egui_plot::PlotMemory::load(&ctx, scenes[0].identity.plot_id()).is_none());
        assert_eq!(
            egui_plot::PlotMemory::load(&ctx, scenes[1].identity.plot_id())
                .unwrap()
                .bounds(),
            zoomed.bounds()
        );
    }
}

#[test]
fn subplot_selection_returns_only_its_keyed_edit_and_other_scene_memory_survives() {
    let mut engine = matrust::Engine::default();
    engine.execute("figure(1);plot([0 1 2],[0 1 0]);axis([0 2 -1 2]);figure(2);subplot(1,2,1);plot([0 1],[2 3]);axis([0 1 2 3]);subplot(1,2,2);plot([0 1],[4 5]);axis([0 1 4 5]);figure(1);").unwrap();
    let figures: std::collections::BTreeMap<_, _> =
        engine.figures().map(|(id, f)| (id, f.clone())).collect();
    let active = engine.figure.clone().unwrap();
    for theme in [egui::Theme::Light, egui::Theme::Dark] {
        let ctx = egui::Context::default();
        ctx.set_theme(theme);
        let mut scenes = [
            Scene::new(1, 10, &figures[&1]),
            Scene::new(2, 20, &figures[&2]),
        ];
        let mut output = None;
        for pass in 0..4 {
            output = Some(pair(&ctx, &mut scenes, [0, 1], pass == 0, vec![]).0);
        }
        let positions: Vec<_> = figures[&2].all_axes().map(|(p, _)| p).collect();
        for position in &positions {
            let id = scenes[1].identity.plot_id().with(("axes", *position));
            assert!(egui_plot::PlotMemory::load(&ctx, id).is_some());
            assert!(
                egui_plot::PlotMemory::load(&ctx, egui::Id::new(("subplot", 2u32, *position)))
                    .is_none()
            );
        }
        let before = egui_plot::PlotMemory::load(&ctx, scenes[0].identity.plot_id()).unwrap();
        let click = output
            .unwrap()
            .shapes
            .iter()
            .find_map(|s| position(&s.shape, "Axes 1"))
            .unwrap();
        let mut selection = None;
        for pressed in [true, false] {
            let (_, outcomes) = pair(
                &ctx,
                &mut scenes,
                [0, 1],
                false,
                vec![
                    egui::Event::PointerMoved(click),
                    egui::Event::PointerButton {
                        pos: click,
                        button: egui::PointerButton::Primary,
                        pressed,
                        modifiers: egui::Modifiers::NONE,
                    },
                ],
            );
            if !pressed {
                selection = Some(outcomes.into_iter().nth(1).unwrap());
            }
        }
        let selection = selection.unwrap();
        assert!(selection.matches(&scenes[1].key, 20));
        let Some(Interaction::Edit(edit, undoable)) = selection.interaction else {
            panic!("actual axes selection must return its typed edit")
        };
        assert_eq!(edit.id, 2);
        assert!(!undoable);
        assert!(edit.original.same_snapshot(&figures[&2]));
        assert_ne!(
            edit.updated.active_position(),
            edit.original.active_position()
        );
        for (p, original) in edit.original.all_axes() {
            let next = edit.updated.all_axes().find(|(q, _)| *q == p).unwrap().1;
            assert!(
                original.series[0]
                    .points
                    .same_storage(&next.series[0].points)
            );
        }
        assert_eq!(
            egui_plot::PlotMemory::load(&ctx, scenes[0].identity.plot_id())
                .unwrap()
                .bounds(),
            before.bounds()
        );
        assert!(engine.figure.as_ref().unwrap().same_snapshot(&active));
        for (id, live) in engine.figures() {
            assert!(live.same_snapshot(&figures[&id]));
        }
        scenes[1].identity.forget(&ctx, &scenes[1].figure).unwrap();
        for position in positions {
            assert!(
                egui_plot::PlotMemory::load(
                    &ctx,
                    scenes[1].identity.plot_id().with(("axes", position))
                )
                .is_none()
            );
        }
        assert!(egui_plot::PlotMemory::load(&ctx, scenes[0].identity.plot_id()).is_some());
    }
}

#[test]
fn stale_sources_reused_numbers_and_revisions_cannot_authorize_scene_results() {
    let mut engine = matrust::Engine::default();
    engine.execute("plot([1 2],[3 4]);").unwrap();
    let original = engine.figure.clone().unwrap();
    let mut a = Scene::new(1, 7, &original);
    let old_id = a.identity.plot_id();
    let ctx = egui::Context::default();
    let mut shown = None;
    crate::headless::run(
        &ctx,
        egui::RawInput {
            screen_rect: Some(egui::Rect::from_min_size(
                egui::Pos2::ZERO,
                egui::vec2(800., 600.),
            )),
            ..Default::default()
        },
        |root| {
            egui::CentralPanel::default().show(root, |ui| {
                shown = Some(
                    show(
                        ui,
                        Frame {
                            key: &a.key,
                            revision: 7,
                            figure: &a.figure,
                            prepared_source: &a.figure,
                            marker: None,
                            restored_view: None,
                        },
                        &mut a.geometry,
                        true,
                        Identity::Explicit(&a.identity),
                    )
                    .unwrap(),
                );
            });
        },
    );
    let shown = shown.unwrap();
    assert!(shown.matches(&a.key, 7));
    assert!(!shown.matches(&a.key, 8));
    engine
        .execute("close(1);figure(1);plot([1 2],[3 4]);")
        .unwrap();
    let replacement = engine.figure.unwrap();
    let key = Key {
        number: 1,
        identity: replacement.identity.clone(),
    };
    let fresh_id = SurfaceIdentity::new(key.clone()).unwrap();
    assert_ne!(fresh_id.plot_id(), old_id);
    assert!(!shown.matches(&key, 7));
    assert!(!a.identity.matches(&key));
    let mut changed = original.clone();
    changed.title = "new title".into();
    for (key, figure) in [(&a.key, &changed), (&key, &replacement)] {
        crate::headless::run(
            &ctx,
            egui::RawInput {
                screen_rect: Some(egui::Rect::from_min_size(
                    egui::Pos2::ZERO,
                    egui::vec2(800., 600.),
                )),
                ..Default::default()
            },
            |root| {
                egui::CentralPanel::default().show(root, |ui| {
                    assert!(
                        show(
                            ui,
                            Frame {
                                key,
                                revision: 8,
                                figure,
                                prepared_source: &original,
                                marker: None,
                                restored_view: None
                            },
                            &mut a.geometry,
                            false,
                            Identity::Explicit(&a.identity)
                        )
                        .is_err()
                    );
                });
            },
        );
    }
    assert!(
        original.series[0]
            .points
            .same_storage(&a.figure.series[0].points)
    );
    assert!(egui_plot::PlotMemory::load(&ctx, fresh_id.plot_id()).is_none());
}
