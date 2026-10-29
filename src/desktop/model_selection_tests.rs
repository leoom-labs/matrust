use super::keyboard_tests::{CTRL, focus, key, press};
use super::tests::Harness;
use super::*;
use matrust::simulation::{Block, MAX_LAYOUT_COORDINATE, Node};
use std::sync::atomic::AtomicBool;

fn click(h: &mut Harness, index: usize, modifiers: egui::Modifiers) {
    let point = h.port(index, None) - Vec2::new(60., 0.);
    h.modifiers = modifiers;
    h.click(point);
    h.modifiers = egui::Modifiers::NONE;
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

#[test]
fn model_selection_pointer_toggle_ranges_empty_and_keyboard_select_all() {
    for repeat in [false, true] {
        let mut h = Harness::new();
        h.repeat = repeat;
        click(&mut h, 0, egui::Modifiers::NONE);
        click(&mut h, 1, CTRL);
        assert_eq!(h.canvas.selection.indices(), &[0, 1]);
        click(&mut h, 2, egui::Modifiers::SHIFT);
        assert_eq!(h.canvas.selection.indices(), &[1, 2]);
        click(&mut h, 2, CTRL);
        click(&mut h, 1, CTRL);
        assert!(h.canvas.selection.indices().is_empty());
        assert_eq!(h.selected, usize::MAX);
        press(&mut h, egui::Key::Delete, egui::Modifiers::NONE);
        assert_eq!(h.model.nodes.len(), 3);
        press(&mut h, egui::Key::A, CTRL);
        assert_eq!(h.canvas.selection.indices(), &[0, 1, 2]);
        press(&mut h, egui::Key::Home, egui::Modifiers::NONE);
        press(&mut h, egui::Key::ArrowDown, egui::Modifiers::SHIFT);
        assert_eq!(h.canvas.selection.indices(), &[0, 1]);
        assert!(h.canvas.history.is_empty());
    }
}

#[test]
fn model_selection_group_drag_nudge_cancel_and_boundary_preserve_spacing() {
    let mut h = Harness::new();
    let original = h.model.clone();
    click(&mut h, 0, egui::Modifiers::NONE);
    click(&mut h, 1, CTRL);
    let point = h.port(0, None) - Vec2::new(60., 0.);
    h.frame(pointer(point, true));
    h.frame(vec![egui::Event::PointerMoved(point + Vec2::new(17., 19.))]);
    h.frame(pointer(point + Vec2::new(17., 19.), false));
    assert_eq!(h.canvas.selection.indices(), &[0, 1]);
    assert_eq!(&h.model.positions[..2], &[[40., 40.], [210., 40.]]);
    assert_eq!(h.canvas.cursor, 1);
    assert_eq!(
        h.model.run(&AtomicBool::new(false)).unwrap().traces[0]
            .values
            .data(),
        &[2.; 4]
    );
    press(&mut h, egui::Key::Z, CTRL);
    assert!(h.model.same_storage(&original));
    for held in [false, true, true] {
        h.frame(vec![key(egui::Key::ArrowRight, CTRL, held)]);
    }
    assert_eq!(&h.model.positions[..2], &[[48., 24.], [218., 24.]]);
    assert_eq!(h.canvas.cursor, 1);
    press(&mut h, egui::Key::Z, CTRL);
    assert!(h.model.same_storage(&original));
    let point = h.port(0, None) - Vec2::new(60., 0.);
    h.frame(pointer(point, true));
    h.frame(vec![egui::Event::PointerMoved(point + Vec2::new(30., 30.))]);
    press(&mut h, egui::Key::Escape, egui::Modifiers::NONE);
    h.frame(pointer(point + Vec2::new(30., 30.), false));
    assert!(h.model.same_storage(&original));
    h.canvas.reset();
    h.model
        .set_layout(vec![[0., 0.], [MAX_LAYOUT_COORDINATE, 10.], [40., 40.]])
        .unwrap();
    h.frame(vec![]);
    focus(&mut h);
    press(&mut h, egui::Key::ArrowDown, egui::Modifiers::SHIFT);
    let boundary = h.model.clone();
    // Release the earlier held arrow before a fresh movement interaction.
    let mut release = key(egui::Key::ArrowRight, CTRL, false);
    if let egui::Event::Key { pressed, .. } = &mut release {
        *pressed = false;
    }
    h.frame(vec![release]);
    press(&mut h, egui::Key::ArrowRight, CTRL);
    assert!(h.model.same_storage(&boundary));
    assert!(h.canvas.history.is_empty());
    press(&mut h, egui::Key::ArrowDown, CTRL);
    assert_eq!(
        &h.model.positions[..2],
        &[[0., 8.], [MAX_LAYOUT_COORDINATE, 18.]]
    );
}

#[test]
fn model_selection_delete_all_is_one_transaction_and_undo_restores_selection() {
    for repeat in [false, true] {
        let mut h = Harness::new();
        h.repeat = repeat;
        h.model
            .set_layout(vec![[24., 24.], [194., 24.], [364., 24.]])
            .unwrap();
        h.frame(vec![]);
        focus(&mut h);
        let original = h.model.clone();
        press(&mut h, egui::Key::A, CTRL);
        press(&mut h, egui::Key::Delete, egui::Modifiers::NONE);
        assert!(h.model.nodes.is_empty() && h.model.positions.is_empty());
        assert_eq!(h.model.version, 1);
        assert_eq!(h.canvas.cursor, 1);
        press(&mut h, egui::Key::Z, CTRL);
        assert!(h.model.same_storage(&original));
        assert_eq!(h.canvas.selection.indices(), &[0, 1, 2]);
        press(&mut h, egui::Key::Y, CTRL);
        assert!(h.model.nodes.is_empty());
    }
}

fn graph() -> Model {
    let mut model = Model::default();
    model.nodes[0].block = Block::Constant { value: 1. };
    model.nodes[0].inputs.clear();
    model.nodes[1].block = Block::Filter {
        b: vec![0.25; 4097],
        a: vec![1.],
    };
    model.nodes[1].inputs = vec![0];
    model.nodes[2].block = Block::Sum;
    model.nodes[2].inputs = vec![1, 1, 2, 999, usize::MAX];
    model.nodes[3].inputs = vec![2];
    model
        .set_layout((0..5).map(|i| [i as f32 * 100., 24.]).collect())
        .unwrap();
    model
}
fn address(model: &Model, index: usize) -> *const f64 {
    match &model.nodes[index].block {
        Block::Filter { b, .. } => b.as_ptr(),
        _ => unreachable!(),
    }
}

#[test]
fn model_selection_bulk_delete_restores_fanout_invalid_edges_layout_and_buffers() {
    let mut model = graph();
    let original = model.clone();
    let pointers = [address(&model, 1), address(&model, 3)];
    let mut change = group_delete::Change::delete(&mut model, &[1, 3], HISTORY_BYTES).unwrap();
    assert_eq!(
        model.nodes[1].inputs,
        [usize::MAX, usize::MAX, 1, 997, usize::MAX]
    );
    assert_eq!(model.nodes[2].inputs, [usize::MAX]);
    assert!(change.retained_bytes() <= HISTORY_BYTES);
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("group.mrsim");
    model.save(&path).unwrap();
    assert!(Model::load(&path).unwrap().same_storage(&model));
    for _ in 0..3 {
        assert!(change.apply(&mut model, false));
        assert!(model.same_storage(&original));
        assert_eq!([address(&model, 1), address(&model, 3)], pointers);
        assert!(change.apply(&mut model, true));
    }
}

#[test]
fn model_selection_bulk_stale_redo_is_atomic_and_unrelated_parameters_survive() {
    let mut model = graph();
    let mut change = group_delete::Change::delete(&mut model, &[1, 3], HISTORY_BYTES).unwrap();
    model.nodes[0].block = Block::Constant { value: 9. };
    assert!(change.apply(&mut model, false));
    assert!(matches!(
        model.nodes[0].block,
        Block::Constant { value: 9. }
    ));
    // Edit the last removed node: no earlier selected node may be removed first.
    if let Block::Filter { b, .. } = &mut model.nodes[3].block {
        b[0] = 0.875;
    }
    let newer = model.clone();
    assert!(!change.apply(&mut model, true));
    assert!(model.same_storage(&newer));
    let mut model = graph();
    let mut change = group_delete::Change::delete(&mut model, &[1, 3], HISTORY_BYTES).unwrap();
    model.nodes[1].name = "Newer surviving node".into();
    let newer = model.clone();
    assert!(!change.apply(&mut model, false));
    assert!(model.same_storage(&newer));
}

#[test]
fn model_selection_bulk_bounds_and_memory_reject_without_mutating_model() {
    let mut model = graph();
    let original = model.clone();
    for indices in [&[][..], &[1, 1], &[3, 1], &[5]] {
        assert!(group_delete::Change::delete(&mut model, indices, HISTORY_BYTES).is_err());
        assert!(model.same_storage(&original));
    }
    assert!(group_delete::Change::delete(&mut model, &[1, 3], 16).is_err());
    assert!(model.same_storage(&original));
    model.positions.clear();
    model.version = 1;
    model.nodes = (0..70)
        .map(|i| Node {
            name: format!("Filter{i}"),
            block: Block::Filter {
                b: vec![0.; 4097],
                a: vec![1.; 4097],
            },
            inputs: vec![0],
        })
        .collect();
    let original = model.clone();
    let pointers: Vec<_> = (0..70).map(|index| address(&model, index)).collect();
    assert!(
        group_delete::Change::delete(&mut model, &(0..70).collect::<Vec<_>>(), HISTORY_BYTES)
            .is_err()
    );
    assert!(model.same_storage(&original));
    assert_eq!(
        (0..70)
            .map(|index| address(&model, index))
            .collect::<Vec<_>>(),
        pointers
    );
}

#[test]
fn model_selection_bulk_handles_512_dense_nodes_with_bounded_history() {
    let mut model = Model {
        steps: 1,
        nodes: (0..512)
            .map(|index| Node {
                name: format!("Node{index}"),
                block: Block::Sum,
                inputs: vec![0; 32],
            })
            .collect(),
        ..Model::default()
    };
    model.nodes[0].block = Block::Constant { value: 1. };
    model.nodes[0].inputs.clear();
    model.nodes[511].block = Block::Scope;
    model.nodes[511].inputs = vec![510];
    let original = model.clone();
    let mut change =
        group_delete::Change::delete(&mut model, &(0..512).collect::<Vec<_>>(), HISTORY_BYTES)
            .unwrap();
    assert!(model.nodes.is_empty());
    assert!(change.retained_bytes() < HISTORY_BYTES);
    assert!(change.apply(&mut model, false));
    assert!(model.same_storage(&original));
    assert_eq!(
        model.run(&AtomicBool::new(false)).unwrap().traces[0]
            .values
            .data(),
        &[32.]
    );
}

#[test]
fn model_selection_full_window_group_delete_and_keyboard_undo_fit_both_viewports() {
    for size in [Vec2::new(1440., 900.), Vec2::new(960., 650.)] {
        let ctx = egui::Context::default();
        let mut tools = super::super::Toolkits {
            model_open: true,
            ..Default::default()
        };
        let original = tools.model.clone();
        let frame = |tools: &mut super::super::Toolkits, events: Vec<egui::Event>, modifiers| {
            crate::headless::run(
                &ctx,
                egui::RawInput {
                    screen_rect: Some(Rect::from_min_size(Pos2::ZERO, size)),
                    events: std::iter::once(egui::Event::ModifiersChanged(modifiers))
                        .chain(events)
                        .collect(),
                    ..Default::default()
                },
                |root| {
                    let ctx = &root.ctx().clone();
                    tools.ui(ctx, false);
                    if ctx.current_pass_index() == 0 {
                        ctx.request_discard("group actions once per input frame");
                    }
                },
            )
        };
        fn find(shape: &egui::Shape, label: &str) -> Option<Rect> {
            match shape {
                egui::Shape::Text(text) if text.galley.text() == label => {
                    Some(Rect::from_min_size(text.pos, text.galley.size()))
                }
                egui::Shape::Vec(shapes) => shapes.iter().find_map(|shape| find(shape, label)),
                _ => None,
            }
        }
        let point = |output: &egui::FullOutput, label: &str| {
            let (rect, clip) = output
                .shapes
                .iter()
                .find_map(|shape| find(&shape.shape, label).map(|rect| (rect, shape.clip_rect)))
                .unwrap_or_else(|| panic!("Missing {label}"));
            assert!(
                clip.expand(0.1).contains_rect(rect),
                "Clipped {label}: {rect:?} in {clip:?}"
            );
            assert!(
                Rect::from_min_size(Pos2::ZERO, size).contains_rect(rect),
                "Outside viewport: {label}"
            );
            rect.center()
        };
        for _ in 0..3 {
            frame(&mut tools, vec![], egui::Modifiers::NONE);
        }
        let blocks_tab = point(&frame(&mut tools, vec![], egui::Modifiers::NONE), "Blocks");
        for pressed in [true, false] {
            frame(
                &mut tools,
                pointer(blocks_tab, pressed),
                egui::Modifiers::NONE,
            );
        }
        for (label, modifiers) in [("1. Signal", egui::Modifiers::NONE), ("2. Noise", CTRL)] {
            let p = point(&frame(&mut tools, vec![], egui::Modifiers::NONE), label);
            for pressed in [true, false] {
                frame(
                    &mut tools,
                    vec![
                        egui::Event::PointerMoved(p),
                        egui::Event::PointerButton {
                            pos: p,
                            button: egui::PointerButton::Primary,
                            pressed,
                            modifiers,
                        },
                    ],
                    modifiers,
                );
            }
        }
        assert_eq!(tools.canvas.selection.indices(), &[0, 1]);
        let parameters_tab = point(
            &frame(&mut tools, vec![], egui::Modifiers::NONE),
            "Parameters",
        );
        for pressed in [true, false] {
            frame(
                &mut tools,
                pointer(parameters_tab, pressed),
                egui::Modifiers::NONE,
            );
        }
        let output = frame(&mut tools, vec![], egui::Modifiers::NONE);
        point(
            &output,
            "2 blocks selected; parameters affect the active block.",
        );
        let p = point(&output, "Delete selected blocks");
        for pressed in [true, false] {
            frame(&mut tools, pointer(p, pressed), egui::Modifiers::NONE);
        }
        assert_eq!(tools.model.nodes.len(), 3);
        assert_eq!(tools.canvas.cursor, 1);
        frame(&mut tools, vec![key(egui::Key::Z, CTRL, false)], CTRL);
        assert!(tools.model.same_storage(&original));
        assert_eq!(tools.canvas.selection.indices(), &[0, 1]);
    }
}
