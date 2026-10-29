use super::keyboard_tests::{CTRL, press};
use super::tests::Harness;
use super::*;
use matrust::simulation::{Block, Node};

fn point(h: &mut Harness, index: usize) -> Pos2 {
    h.frame(vec![]);
    h.canvas
        .blocks
        .iter()
        .find(|(node, _)| *node == index)
        .unwrap()
        .1
        .center()
}
fn pointer(point: Pos2, pressed: bool) -> Vec<egui::Event> {
    vec![
        egui::Event::PointerMoved(point),
        egui::Event::PointerButton {
            pos: point,
            button: egui::PointerButton::Primary,
            pressed,
            modifiers: egui::Modifiers::NONE,
        },
    ]
}
fn settle(h: &mut Harness) {
    for _ in 0..30 {
        h.frame(vec![]);
    }
}

#[test]
fn model_view_fit_all_and_selection_are_transient_and_reveal_remote_blocks() {
    for repeat in [false, true] {
        let mut h = Harness::new();
        h.repeat = repeat;
        h.model
            .set_layout(vec![[10., 10.], [9000., 5000.], [1_000_000., 1_000_000.]])
            .unwrap();
        let original = h.model.clone();
        h.frame(vec![]);
        h.button("View");
        h.button("Fit diagram");
        settle(&mut h);
        assert!(h.canvas.view.zoom < 0.001);
        assert_eq!(h.canvas.blocks.len(), 3);
        assert!(h.canvas.ports.is_empty());
        let p = point(&mut h, 2);
        h.click(p);
        assert_eq!(h.selected, 2);
        h.button("View");
        h.button("Fit selection");
        settle(&mut h);
        assert_eq!(h.canvas.view.zoom, 1.);
        h.port(2, None);
        assert!(h.model.same_storage(&original));
        assert!(h.canvas.history.is_empty());
        assert!(!h.canvas.take_changed());
    }
}

#[test]
fn model_view_zoomed_drag_nudge_and_undo_use_logical_coordinates() {
    for zoom in [0.5, 0.7, 2.] {
        let mut h = Harness::new();
        h.canvas.view.zoom = zoom;
        h.model
            .set_layout(vec![[24., 24.], [194., 24.], [364., 24.]])
            .unwrap();
        if zoom == 0.7 {
            h.model.positions[1] = [
                f32::from_bits(1_000_000_f32.to_bits() - 1),
                f32::from_bits(1_000_000_f32.to_bits() - 3),
            ];
        }
        let original = h.model.clone();
        settle(&mut h);
        point(&mut h, 0); // Refresh actual laid-out block and viewport bounds.
        let block = h
            .canvas
            .blocks
            .iter()
            .find(|(index, _)| *index == 0)
            .unwrap()
            .1;
        let viewport = h.canvas.view.viewport.shrink(1.);
        let delta = Vec2::splat(16. * zoom);
        // The block center can be visible while center+delta is clipped at2x.
        // Keep both pointer endpoints in the real viewport without changing
        // the required logical displacement, zoom or exact undo/nudge checks.
        let starts = block
            .shrink(9. * zoom)
            .intersect(viewport)
            .intersect(viewport.translate(-delta));
        assert!(
            starts.is_positive(),
            "No visible drag start at zoom={zoom}: block={block:?} viewport={viewport:?}"
        );
        let p = starts.center();
        assert!(viewport.contains(p) && viewport.contains(p + delta));
        h.frame(pointer(p, true));
        h.frame(vec![egui::Event::PointerMoved(p + delta)]);
        assert_eq!(
            h.canvas
                .moving
                .as_ref()
                .expect("Visible drag must start")
                .positions[0],
            [40., 40.]
        );
        assert!(
            h.canvas.view.viewport.contains(p + delta),
            "Canvas moved during block drag"
        );
        h.frame(pointer(p + delta, false));
        assert_eq!(h.model.positions[0], [40., 40.]);
        assert_eq!(&h.model.positions[1..], &original.positions[1..]);
        assert_eq!(h.canvas.cursor, 1);
        press(&mut h, egui::Key::Z, CTRL);
        assert!(h.model.same_storage(&original));
        press(&mut h, egui::Key::ArrowRight, CTRL);
        assert_eq!(h.model.positions[0], [32., 24.]);
        assert_eq!(h.canvas.view.zoom, zoom);
    }
}

#[test]
fn model_view_zoomed_ports_and_marquee_keep_correct_targets() {
    for zoom in [0.5, 1.25] {
        let mut h = Harness::new();
        h.canvas.view.zoom = zoom;
        settle(&mut h);
        let source = h.port(1, None);
        let target = h.port(2, Some(0));
        h.click(source);
        h.click(target);
        assert_eq!(h.model.nodes[2].inputs, [1]);
        assert_eq!(h.canvas.cursor, 1);
        let first = h.canvas.blocks.iter().find(|(i, _)| *i == 0).unwrap().1;
        let second = h.canvas.blocks.iter().find(|(i, _)| *i == 1).unwrap().1;
        let start = first.min - Vec2::splat(10. * zoom);
        let end = second.max + Vec2::splat(10. * zoom);
        h.frame(pointer(start, true));
        h.frame(vec![egui::Event::PointerMoved(end)]);
        h.frame(pointer(end, false));
        assert_eq!(h.canvas.selection.indices(), &[0, 1]);
        assert_eq!(h.canvas.cursor, 1);
    }
}

#[test]
fn model_view_zoom_menu_cancels_drag_and_preserves_clipboard_layout() {
    let mut h = Harness::new();
    let original = h.model.clone();
    h.button("View");
    h.button("Zoom in");
    settle(&mut h);
    assert_eq!(h.canvas.view.zoom, 1.25);
    h.button("View");
    h.button("Zoom out");
    assert_eq!(h.canvas.view.zoom, 1.);
    let p = point(&mut h, 0);
    h.frame(pointer(p, true));
    h.frame(vec![egui::Event::PointerMoved(p + Vec2::splat(20.))]);
    assert!(h.canvas.moving.is_some());
    // A view request cancels uncommitted geometry; the toolbar routes this too.
    h.canvas.cancel_connection();
    h.canvas.view.request = Some(view::Command::Actual);
    h.frame(pointer(p + Vec2::splat(20.), false));
    assert!(h.model.same_storage(&original));
    h.canvas.view.zoom = 0.5;
    h.canvas.selection.all(3, &mut h.selected);
    h.button("Edit blocks");
    h.button("Duplicate blocks (Ctrl+D)");
    assert_eq!(
        &h.model.positions[3..],
        &[[56., 56.], [226., 56.], [396., 56.]]
    );
    assert_eq!(h.canvas.view.zoom, 0.5);
}

#[test]
fn model_view_dense_overview_is_bounded_and_view_math_roundtrips() {
    let mut h = Harness::new();
    h.model.nodes = (0..512)
        .map(|index| Node {
            name: index.to_string(),
            block: Block::Sum,
            inputs: vec![0; 32],
        })
        .collect();
    h.frame(vec![]);
    h.button("View");
    h.button("Fit diagram");
    settle(&mut h);
    assert_eq!(h.canvas.blocks.len(), 512);
    assert!(h.canvas.ports.is_empty());
    let output = h.frame(vec![]);
    assert!(
        output.shapes.len() < 5000,
        "overview should aggregate coincident wires and omit port widgets"
    );
    assert!(h.canvas.history.is_empty());
    for zoom in [0.0001, 0.5, 1., 4.] {
        let view = view::View {
            zoom,
            ..Default::default()
        };
        let rect = Rect::from_min_size(Pos2::new(100., 200.), Vec2::new(126., 56.));
        let origin = Pos2::new(12., 34.);
        let restored = view.model_rect(view.screen_rect(rect, origin), origin);
        assert!((restored.min - rect.min).length() < 0.05);
        assert!((restored.max - rect.max).length() < 0.05);
    }
}

#[test]
fn model_view_zoomed_keyboard_wiring_reveals_last_of_32_inputs() {
    let mut h = Harness::new();
    h.model.nodes[2].block = Block::Sum;
    h.model.nodes[2].inputs = vec![0; 32];
    h.canvas.view.zoom = 2.;
    settle(&mut h);
    let p = point(&mut h, 1);
    h.click(p);
    press(&mut h, egui::Key::C, egui::Modifiers::NONE);
    press(&mut h, egui::Key::End, egui::Modifiers::NONE);
    for _ in 0..31 {
        press(&mut h, egui::Key::PageDown, egui::Modifiers::NONE);
    }
    settle(&mut h);
    h.port(2, Some(31));
    press(&mut h, egui::Key::Enter, egui::Modifiers::NONE);
    assert_eq!(h.model.nodes[2].inputs[31], 1);
    assert_eq!(&h.model.nodes[2].inputs[..31], &[0; 31]);
    assert_eq!(h.canvas.cursor, 1);
    press(&mut h, egui::Key::Z, CTRL);
    assert_eq!(h.model.nodes[2].inputs, vec![0; 32]);
    assert_eq!(h.canvas.view.zoom, 2.);
}

#[test]
#[ignore = "manual release frame benchmark; run without concurrent builds or captures"]
fn model_view_benchmark_cpu_frames() {
    use std::{hint::black_box, time::Instant};
    let workers = rayon::current_num_threads();
    println!(
        "\nworkers,blocks,mode,frames,repetition,milliseconds_per_frame,visible_blocks,shapes"
    );
    for count in [16, 128, 512] {
        for mode in ["actual", "overview"] {
            let mut h = Harness::new();
            h.model.nodes = (0..count)
                .map(|index| Node {
                    name: format!("Block{index}"),
                    block: Block::Sum,
                    inputs: vec![0; 32],
                })
                .collect();
            h.model.nodes[0].block = Block::Constant { value: 1. };
            h.model.nodes[0].inputs.clear();
            let original = h.model.clone();
            if mode == "overview" {
                h.canvas.view.request = Some(view::Command::All);
            }
            settle(&mut h);
            for repetition in 0..12 {
                let start = Instant::now();
                let mut shapes = 0;
                for _ in 0..100 {
                    let output = black_box(h.frame(vec![]));
                    shapes = output.shapes.len();
                }
                let milliseconds = start.elapsed().as_secs_f64() * 10.;
                assert!(h.model.same_storage(&original));
                assert!(h.canvas.history.is_empty());
                let visible = h.canvas.blocks.len();
                if mode == "overview" {
                    assert_eq!(visible, count);
                }
                if repetition > 0 {
                    println!(
                        "{workers},{count},{mode},100,{repetition},{milliseconds:.9},{visible},{shapes}"
                    );
                }
            }
        }
    }
}

#[test]
fn positioned_model_initial_view_fits_once_without_mutating_geometry_or_history() {
    for repeat in [false, true] {
        let mut h = Harness::new();
        h.repeat = repeat;
        h.model
            .set_layout(vec![[30., 60.], [220., 220.], [410., 220.]])
            .unwrap();
        let original = h.model.clone();
        h.canvas.view.zoom = 2.;
        h.canvas.reveal(2);
        h.canvas.reset_for_model(&h.model, h.selected);
        settle(&mut h);
        assert!(h.canvas.view.zoom > 0. && h.canvas.view.zoom < 1.);
        assert!(h.canvas.view.request.is_none());
        assert_eq!(h.canvas.blocks.len(), h.model.nodes.len());
        for (_, block) in &h.canvas.blocks {
            assert!(
                h.canvas.view.viewport.expand(1.).contains_rect(*block),
                "initial fit clipped {block:?} outside {:?}",
                h.canvas.view.viewport
            );
        }
        assert!(h.model.same_storage(&original));
        assert!(h.canvas.history.is_empty());
        assert!(!h.canvas.take_changed());
        h.canvas.view.request = Some(view::Command::In);
        h.frame(vec![]);
        let chosen = h.canvas.view.zoom;
        settle(&mut h);
        assert_eq!(
            h.canvas.view.zoom, chosen,
            "ordinary frames must retain user zoom"
        );
        h.model.nodes[0].name = "Edited after open".into();
        settle(&mut h);
        assert_eq!(
            h.canvas.view.zoom, chosen,
            "editing must not trigger another fit"
        );
    }
}

#[test]
fn initial_view_keeps_auto_layout_actual_size_and_never_magnifies_small_positioned_models() {
    let mut h = Harness::new();
    h.canvas.view.zoom = 0.5;
    h.canvas.reset_for_model(&h.model, 2);
    settle(&mut h);
    assert_eq!(h.canvas.view.zoom, 1.);
    assert!(h.canvas.view.request.is_none());
    h.model
        .set_layout(vec![[24., 24.], [194., 24.], [364., 24.]])
        .unwrap();
    h.canvas.reset_for_model(&h.model, 0);
    settle(&mut h);
    assert_eq!(
        h.canvas.view.zoom, 1.,
        "initial fit may shrink but never magnify"
    );
    assert!(!h.canvas.take_changed());
}

#[test]
fn model_view_zoomed_drag_release_outside_canvas_cancels_preview() {
    let mut h = Harness::new();
    h.canvas.view.zoom = 2.;
    h.model
        .set_layout(vec![[24., 24.], [194., 24.], [364., 24.]])
        .unwrap();
    settle(&mut h);
    let original = h.model.clone();
    let block = h
        .canvas
        .blocks
        .iter()
        .find(|(index, _)| *index == 0)
        .unwrap()
        .1;
    let viewport = h.canvas.view.viewport.shrink(1.);
    let delta = Vec2::splat(16.);
    let starts = block
        .shrink(18.)
        .intersect(viewport)
        .intersect(viewport.translate(-delta));
    assert!(starts.is_positive());
    let start = starts.center();
    h.frame(pointer(start, true));
    h.frame(vec![egui::Event::PointerMoved(start + delta)]);
    assert!(h.canvas.moving.as_ref().is_some_and(|m| m.changed()));
    assert!(h.model.same_storage(&original));
    let outside = h.canvas.view.viewport.max + Vec2::splat(20.);
    assert!(!h.canvas.view.viewport.contains(outside));
    h.frame(pointer(outside, false));
    assert!(h.canvas.moving.is_none());
    assert!(h.model.same_storage(&original));
    assert!(h.canvas.history.is_empty());
}
