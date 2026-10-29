use super::*;
use matrust::simulation::{Block, Node};
use std::sync::atomic::AtomicBool;

fn gain() -> Node {
    Node {
        name: "Added gain".into(),
        block: Block::Gain { gain: 2. },
        inputs: vec![usize::MAX],
    }
}
fn travel(canvas: &mut Canvas, model: &mut Model, selected: &mut usize, redo: bool) {
    canvas.travel(model, selected, redo, &mut String::new());
}
fn coefficients(model: &Model, index: usize) -> *const f64 {
    match &model.nodes[index].block {
        Block::Filter { b, .. } => b.as_ptr(),
        _ => panic!("filter expected"),
    }
}

#[test]
fn model_topology_delete_restores_fanout_positions_coefficients_and_samples() {
    let mut model = Model::default();
    model.nodes.push(Node {
        name: "Second scope".into(),
        block: Block::Scope,
        inputs: vec![3],
    });
    model.nodes.push(Node {
        name: "Feedback".into(),
        block: Block::UnitDelay { initial: 1. },
        inputs: vec![6],
    });
    model
        .set_layout((0..7).map(|i| [i as f32 * 100., 24.]).collect())
        .unwrap();
    let before = model.clone();
    let samples = model.run(&AtomicBool::new(false)).unwrap();
    let pointer = coefficients(&model, 3);
    let mut canvas = Canvas::default();
    let mut selected = canvas.delete(&mut model, 3).unwrap();
    assert_eq!(model.nodes[3].inputs, [usize::MAX]);
    assert_eq!(model.nodes[4].inputs, [usize::MAX]);
    assert_eq!(model.nodes[5].inputs, [5]);
    assert_eq!(model.positions[3], [400., 24.]);
    for _ in 0..3 {
        travel(&mut canvas, &mut model, &mut selected, false);
        assert_eq!(selected, 3);
        assert!(model.same_storage(&before));
        assert_eq!(coefficients(&model, 3), pointer);
        let restored = model.run(&AtomicBool::new(false)).unwrap();
        for (a, b) in samples.traces.iter().zip(&restored.traces) {
            assert_eq!(
                bytemuck::cast_slice::<f64, u64>(a.values.data()),
                bytemuck::cast_slice::<f64, u64>(b.values.data())
            );
        }
        travel(&mut canvas, &mut model, &mut selected, true);
        assert_eq!(model.nodes.len(), 6);
    }
}

#[test]
fn model_topology_add_delete_buttons_share_wiring_history_and_branch_redo() {
    let mut h = super::tests::Harness::new();
    let original = h.model.clone();
    let source = h.port(1, None);
    h.click(source);
    let target = h.port(2, Some(0));
    h.click(target);
    h.selected = h.canvas.insert(&mut h.model, gain()).unwrap();
    assert_eq!(h.selected, 3);
    h.button("Undo diagram");
    assert_eq!(h.model.nodes.len(), 3);
    assert_eq!(h.model.nodes[2].inputs, [1]);
    h.button("Undo diagram");
    assert!(h.model.same_storage(&original));
    h.button("Redo diagram");
    h.button("Redo diagram");
    assert_eq!(h.model.nodes.len(), 4);
    assert_eq!(h.selected, 3);
    h.canvas.delete(&mut h.model, 1).unwrap();
    assert_eq!(h.model.nodes[1].inputs, [usize::MAX]);
    h.button("Undo diagram");
    assert_eq!(h.model.nodes[2].inputs, [1]);
    h.button("Undo diagram");
    let source = h.port(0, None);
    h.click(source);
    let target = h.port(2, Some(0));
    h.click(target);
    assert_eq!(h.canvas.cursor, h.canvas.history.len());
    assert!(h.model.same_storage(&original));
}

#[test]
fn model_topology_last_block_and_invalid_draft_edges_roundtrip_exactly() {
    for positioned in [false, true] {
        let mut model = Model {
            nodes: vec![Node {
                name: "Delay".into(),
                block: Block::UnitDelay { initial: -0. },
                inputs: vec![0],
            }],
            ..Model::default()
        };
        if positioned {
            model.set_layout(vec![[0., 999_999.]]).unwrap();
        }
        let original = model.clone();
        let mut canvas = Canvas::default();
        let mut selected = canvas.delete(&mut model, 0).unwrap();
        assert_eq!(model.version, 1);
        assert!(model.positions.is_empty() && model.nodes.is_empty());
        travel(&mut canvas, &mut model, &mut selected, false);
        assert!(model.same_storage(&original));
        travel(&mut canvas, &mut model, &mut selected, true);
        canvas.insert(&mut model, gain()).unwrap();
        assert_eq!(model.nodes.len(), 1);
    }
    let mut model = Model::default();
    model.nodes[4].inputs = vec![usize::MAX - 1];
    let original = model.clone();
    let mut change = topology::Change::delete(&mut model, 1).unwrap();
    assert_eq!(model.nodes[3].inputs, [usize::MAX - 2]);
    assert!(change.apply(&mut model, false));
    assert!(model.same_storage(&original));
}

#[test]
fn model_topology_stale_edits_are_retained_and_unrelated_parameters_survive() {
    for case in 0..5 {
        let mut model = Model::default();
        let mut canvas = Canvas::default();
        let mut selected = canvas.insert(&mut model, gain()).unwrap();
        match case {
            0 => model.nodes[selected].block = Block::Gain { gain: 3. },
            1 => model.nodes[0].name = "Renamed outside history".into(),
            2 => model.nodes[4].inputs[0] = 0,
            3 => model.set_layout(vec![[24., 24.]; 6]).unwrap(),
            _ => model.nodes[selected].name.reserve(4096),
        }
        let newer = model.clone();
        travel(&mut canvas, &mut model, &mut selected, false);
        assert!(model.same_storage(&newer));
        assert!(canvas.history.is_empty());
        assert_eq!(canvas.history_bytes, 0);
    }
    let mut model = Model::default();
    let mut change = topology::Change::delete(&mut model, 3).unwrap();
    model.nodes[0].block = Block::Sine {
        amplitude: 7.,
        frequency_hz: 10.,
        phase: 0.,
    };
    model.steps = 17;
    assert!(change.apply(&mut model, false));
    assert!(matches!(
        model.nodes[0].block,
        Block::Sine { amplitude: 7., .. }
    ));
    assert_eq!(model.steps, 17);
}

#[test]
fn model_topology_history_is_bounded_by_memory_and_drops_redo_storage() {
    let mut model = Model::default();
    let mut canvas = Canvas::default();
    for _ in 0..90 {
        let index = canvas
            .insert(
                &mut model,
                Node {
                    name: "Large filter".into(),
                    block: Block::Filter {
                        b: vec![0.; 4097],
                        a: vec![1.; 4097],
                    },
                    inputs: vec![usize::MAX],
                },
            )
            .unwrap();
        canvas.delete(&mut model, index).unwrap();
        assert!(canvas.history_bytes <= HISTORY_BYTES);
        assert!(canvas.history.len() <= HISTORY_LIMIT);
    }
    assert!(canvas.history.len() < HISTORY_LIMIT);
    assert_eq!(
        canvas.history_bytes,
        canvas
            .history
            .iter()
            .map(Change::retained_bytes)
            .sum::<usize>()
    );
    travel(&mut canvas, &mut model, &mut 0, false);
    canvas.insert(&mut model, gain()).unwrap();
    assert_eq!(canvas.cursor, canvas.history.len());
    assert_eq!(
        canvas.history_bytes,
        canvas
            .history
            .iter()
            .map(Change::retained_bytes)
            .sum::<usize>()
    );
    canvas.reset();
    assert_eq!(canvas.history_bytes, 0);
}

#[test]
fn model_topology_toolkit_dirty_state_and_file_reset_follow_undo() {
    let mut tools = super::super::Toolkits::default();
    let original = tools.model.clone();
    tools.add_block(Block::Gain { gain: 2. });
    assert!(tools.model_dirty());
    tools.selected = 3;
    tools.coefficient_editor =
        super::super::coefficients::Editor::new(3, &tools.model.nodes[3].block);
    tools.delete_block();
    assert!(tools.coefficient_editor.is_none());
    travel(
        &mut tools.canvas,
        &mut tools.model,
        &mut tools.selected,
        false,
    );
    assert!(tools.model_dirty());
    travel(
        &mut tools.canvas,
        &mut tools.model,
        &mut tools.selected,
        false,
    );
    assert!(!tools.model_dirty());
    assert!(tools.model.same_storage(&original));
    tools.restore_model(original.clone(), None, Some(original));
    assert!(tools.canvas.history.is_empty());
    assert_eq!(tools.canvas.history_bytes, 0);
}

#[test]
fn model_topology_native_controls_apply_once_during_repeated_layout_passes() {
    let ctx = egui::Context::default();
    let mut tools = super::super::Toolkits {
        model_open: true,
        selected: 0,
        ..Default::default()
    };
    tools.model.nodes = (0..3)
        .map(|i| Node {
            name: format!("Constant {i}"),
            block: Block::Constant { value: 1. },
            inputs: vec![],
        })
        .collect();
    fn frame(
        ctx: &egui::Context,
        tools: &mut super::super::Toolkits,
        events: Vec<egui::Event>,
    ) -> egui::FullOutput {
        crate::headless::run(
            ctx,
            egui::RawInput {
                screen_rect: Some(Rect::from_min_size(Pos2::ZERO, Vec2::new(960., 650.))),
                events,
                ..Default::default()
            },
            |root| {
                let ctx = &root.ctx().clone();
                tools.ui(ctx, false);
                if ctx.current_pass_index() == 0 {
                    ctx.request_discard("one input, multiple layout passes");
                }
            },
        )
    }
    fn button(ctx: &egui::Context, tools: &mut super::super::Toolkits, label: &str) {
        fn find(shape: &egui::Shape, label: &str) -> Option<Pos2> {
            match shape {
                egui::Shape::Text(t) if t.galley.text() == label => {
                    Some(t.pos + t.galley.size() * 0.5)
                }
                egui::Shape::Vec(shapes) => shapes.iter().find_map(|s| find(s, label)),
                _ => None,
            }
        }
        let output = frame(ctx, tools, vec![]);
        let point = output
            .shapes
            .iter()
            .find_map(|s| find(&s.shape, label))
            .unwrap();
        for pressed in [true, false] {
            frame(
                ctx,
                tools,
                vec![
                    egui::Event::PointerMoved(point),
                    egui::Event::PointerButton {
                        pos: point,
                        button: egui::PointerButton::Primary,
                        pressed,
                        modifiers: egui::Modifiers::NONE,
                    },
                ],
            );
        }
    }
    for _ in 0..3 {
        frame(&ctx, &mut tools, vec![]);
    }
    button(&ctx, &mut tools, "Add block");
    assert_eq!(tools.model.nodes.len(), 4);
    assert_eq!(tools.canvas.history.len(), 1);
    button(&ctx, &mut tools, "Delete selected block");
    assert_eq!(tools.model.nodes.len(), 3);
    assert_eq!(tools.canvas.history.len(), 2);
    button(&ctx, &mut tools, "Undo diagram");
    assert_eq!(tools.model.nodes.len(), 4);
    button(&ctx, &mut tools, "Redo diagram");
    assert_eq!(tools.model.nodes.len(), 3);
}
