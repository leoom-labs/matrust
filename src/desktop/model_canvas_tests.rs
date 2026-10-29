use super::*;
use matrust::simulation::{Block, Node};
use std::sync::atomic::AtomicBool;

fn model() -> Model {
    Model {
        nodes: vec![
            Node {
                name: "First".into(),
                block: Block::Constant { value: 2. },
                inputs: vec![],
            },
            Node {
                name: "Second".into(),
                block: Block::Constant { value: 7. },
                inputs: vec![],
            },
            Node {
                name: "Output".into(),
                block: Block::Scope,
                inputs: vec![0],
            },
        ],
        steps: 4,
        ..Model::default()
    }
}

pub(super) struct Harness {
    pub(super) ctx: egui::Context,
    pub(super) canvas: Canvas,
    pub(super) model: Model,
    pub(super) selected: usize,
    message: String,
    pub(super) enabled: bool,
    pub(super) repeat: bool,
    pub(super) keyboard_allowed: bool,
    shift: bool,
    pub(super) modifiers: egui::Modifiers,
}
impl Harness {
    pub(super) fn new() -> Self {
        let mut value = Self {
            ctx: Default::default(),
            canvas: Default::default(),
            model: model(),
            selected: 0,
            message: String::new(),
            enabled: true,
            repeat: false,
            keyboard_allowed: true,
            shift: false,
            modifiers: egui::Modifiers::NONE,
        };
        for _ in 0..3 {
            value.frame(vec![]);
        }
        value
    }
    pub(super) fn frame(&mut self, events: Vec<egui::Event>) -> egui::FullOutput {
        crate::headless::run(
            &self.ctx,
            egui::RawInput {
                screen_rect: Some(Rect::from_min_size(Pos2::ZERO, Vec2::new(940., 600.))),
                events: std::iter::once(egui::Event::ModifiersChanged(egui::Modifiers {
                    shift: self.shift || self.modifiers.shift,
                    ..self.modifiers
                }))
                .chain(events)
                .collect(),
                ..Default::default()
            },
            |root| {
                let ctx = &root.ctx().clone();
                egui::CentralPanel::default().show(root, |ui| {
                    ui.add_enabled_ui(self.enabled, |ui| {
                        self.canvas.show(
                            ui,
                            &mut self.model,
                            &mut self.selected,
                            &mut self.message,
                            self.keyboard_allowed,
                            0.,
                        );
                    });
                });
                if self.repeat && ctx.current_pass_index() == 0 {
                    ctx.request_discard("connection input must apply only once");
                }
            },
        )
    }
    pub(super) fn port(&mut self, index: usize, port: Option<usize>) -> Pos2 {
        self.frame(vec![]);
        self.canvas
            .ports
            .iter()
            .find(|(i, p, _)| *i == index && *p == port)
            .unwrap()
            .2
    }
    fn pointer(&mut self, position: Pos2, pressed: bool, button: egui::PointerButton) {
        self.frame(vec![
            egui::Event::PointerMoved(position),
            egui::Event::PointerButton {
                pos: position,
                button,
                pressed,
                modifiers: egui::Modifiers {
                    shift: self.shift || self.modifiers.shift,
                    ..self.modifiers
                },
            },
        ]);
    }
    pub(super) fn click(&mut self, position: Pos2) {
        self.pointer(position, true, egui::PointerButton::Primary);
        self.pointer(position, false, egui::PointerButton::Primary);
    }
    pub(super) fn button(&mut self, label: &str) {
        fn find(shape: &egui::Shape, label: &str) -> Option<Pos2> {
            match shape {
                egui::Shape::Text(text) if text.galley.text() == label => {
                    Some(text.pos + text.galley.size() * 0.5)
                }
                egui::Shape::Vec(shapes) => shapes.iter().find_map(|shape| find(shape, label)),
                _ => None,
            }
        }
        let output = self.frame(vec![]);
        let position = output
            .shapes
            .iter()
            .find_map(|shape| find(&shape.shape, label))
            .unwrap();
        self.click(position);
    }
}

#[test]
fn model_canvas_click_connects_real_signal_and_undo_redo_preserve_saved_model() {
    let mut h = Harness::new();
    let output = h.port(1, None);
    h.click(output);
    assert_eq!(h.canvas.source, Some(1));
    let input = h.port(2, Some(0));
    h.click(input);
    assert_eq!(h.model.nodes[2].inputs, [1]);
    assert_eq!(h.selected, 2);
    assert_eq!(
        h.model.run(&AtomicBool::new(false)).unwrap().traces[0]
            .values
            .data(),
        &[7.; 4]
    );
    h.button("Undo diagram");
    assert_eq!(h.model.nodes[2].inputs, [0]);
    h.button("Redo diagram");
    assert_eq!(h.model.nodes[2].inputs, [1]);
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("wired.mrsim");
    h.model.save(&path).unwrap();
    assert!(Model::load(&path).unwrap().same_storage(&h.model));
}

#[test]
fn model_canvas_drag_connects_and_context_menu_disconnect_can_be_undone() {
    let mut h = Harness::new();
    let output = h.port(1, None);
    let input = h.port(2, Some(0));
    h.pointer(output, true, egui::PointerButton::Primary);
    h.frame(vec![egui::Event::PointerMoved(output + Vec2::new(24., 0.))]);
    let live_input = h.port(2, Some(0));
    assert!(
        (live_input - input).length() < 0.5,
        "Starting a wire must not move its target port"
    );
    assert!(h.canvas.view.viewport.contains(input));
    h.frame(vec![egui::Event::PointerMoved(input)]);
    h.pointer(input, false, egui::PointerButton::Primary);
    assert_eq!(h.model.nodes[2].inputs, [1]);
    let input = h.port(2, Some(0));
    h.pointer(input, true, egui::PointerButton::Secondary);
    h.pointer(input, false, egui::PointerButton::Secondary);
    h.button("Disconnect input");
    assert_eq!(h.model.nodes[2].inputs, [usize::MAX]);
    assert!(h.model.validate().is_err());
    h.button("Undo diagram");
    assert_eq!(h.model.nodes[2].inputs, [1]);
}

#[test]
fn model_canvas_escape_disabled_and_repeated_layout_do_not_apply_extra_connections() {
    let mut h = Harness::new();
    h.repeat = true;
    let output = h.port(1, None);
    h.click(output);
    assert_eq!(h.canvas.source, Some(1));
    h.frame(vec![egui::Event::Key {
        key: egui::Key::Escape,
        physical_key: None,
        pressed: true,
        repeat: false,
        modifiers: egui::Modifiers::NONE,
    }]);
    assert_eq!(h.canvas.source, None);
    assert_eq!(h.model.nodes[2].inputs, [0]);
    let output = h.port(1, None);
    h.click(output);
    h.enabled = false;
    let input = h.port(2, Some(0));
    h.click(input);
    assert_eq!(h.model.nodes[2].inputs, [0]);
    assert_eq!(h.canvas.source, None);
    h.enabled = true;
    let output = h.port(1, None);
    h.click(output);
    let input = h.port(2, Some(0));
    h.click(input);
    assert_eq!(h.canvas.history.len(), 1);
    h.button("Undo diagram");
    assert_eq!(h.canvas.cursor, 0);
    assert_eq!(h.model.nodes[2].inputs, [0]);
}

#[test]
fn model_canvas_history_is_bounded_rejects_stale_inputs_and_preserves_feedback_validation() {
    let mut model = model();
    let mut canvas = Canvas::default();
    for i in 0..300 {
        canvas.connect(&mut model, 2, 0, i % 2);
    }
    assert_eq!(canvas.history.len(), HISTORY_LIMIT);
    assert!(!canvas.connect(&mut model, 2, 0, 512));
    assert!(!canvas.connect(&mut model, 2, 32, 0));
    model.nodes[2].inputs[0] = usize::MAX;
    let mut message = String::new();
    canvas.travel(&mut model, &mut 0, false, &mut message);
    assert_eq!(model.nodes[2].inputs, [usize::MAX]);
    assert!(canvas.history.is_empty());
    assert!(canvas.connect(&mut model, 2, 0, 2));
    assert!(model.validate().is_err());
    model.nodes[2].block = Block::UnitDelay { initial: 3. };
    model.nodes.push(Node {
        name: "Delayed scope".into(),
        block: Block::Scope,
        inputs: vec![2],
    });
    assert_eq!(
        model.run(&AtomicBool::new(false)).unwrap().traces[0]
            .values
            .data(),
        &[3.; 4]
    );
}

#[test]
fn model_canvas_large_diagram_only_renders_visible_blocks_and_ports() {
    let mut h = Harness::new();
    for inputs in [1, 32] {
        h.model.nodes = (0..512)
            .map(|i| Node {
                name: format!("Block {i}"),
                block: Block::Sum,
                inputs: vec![0; inputs],
            })
            .collect();
        let output = h.frame(vec![]);
        assert!(h.canvas.ports.len() < 100, "{}", h.canvas.ports.len());
        assert!(output.shapes.len() < 800, "{}", output.shapes.len());
        assert_eq!(h.model.nodes.len(), 512);
    }
}

#[test]
fn model_canvas_distinguishes_sum_ports_and_supports_signal_fanout() {
    let mut h = Harness::new();
    h.model.nodes[2] = Node {
        name: "Add".into(),
        block: Block::Sum,
        inputs: vec![0, 0],
    };
    h.model.nodes.push(Node {
        name: "Result".into(),
        block: Block::Scope,
        inputs: vec![2],
    });
    for (port, expected) in [(1, 9.), (0, 14.)] {
        let output = h.port(1, None);
        h.click(output);
        let input = h.port(2, Some(port));
        h.click(input);
        assert_eq!(h.model.nodes[2].inputs[port], 1);
        let simulation = h.model.run(&AtomicBool::new(false)).unwrap();
        assert_eq!(simulation.traces[0].values.data(), &[expected; 4]);
    }
    assert_eq!(h.model.nodes[2].inputs, [1, 1]);
}

#[test]
fn model_canvas_file_changes_clear_history_and_topology_edits_preserve_prior_wiring() {
    let mut tools = super::super::Toolkits::default();
    let original = tools.model.clone();
    assert!(!tools.model_dirty());
    assert!(tools.canvas.connect(&mut tools.model, 4, 0, 1));
    assert!(tools.model_dirty());
    tools.canvas.source = Some(1);
    tools.canvas.dragging = true;
    let context = egui::Context::default();
    let _ = crate::headless::run(&context, egui::RawInput::default(), |context| {
        tools.ui(context, false);
    });
    assert_eq!(tools.canvas.source, None);
    assert!(!tools.canvas.dragging);
    assert_eq!(tools.canvas.history.len(), 1);
    assert_eq!(tools.model.nodes[4].inputs, [1]);
    tools.restore_model(original.clone(), None, Some(original));
    assert!(!tools.model_dirty());
    assert!(tools.canvas.history.is_empty());
    tools.canvas.connect(&mut tools.model, 4, 0, 0);
    tools.add_block(Block::Gain { gain: 2. });
    assert_eq!(tools.canvas.history.len(), 2);
    tools.canvas.connect(&mut tools.model, 5, 0, 1);
    tools.canvas.source = Some(1);
    tools.discard_model();
    assert_eq!(tools.model.nodes.len(), 5);
    assert!(!tools.model_dirty());
    assert!(tools.canvas.history.is_empty());
    assert_eq!(tools.canvas.source, None);
}

#[test]
fn model_layout_drag_preview_commit_snapping_and_diagram_history() {
    for (shift, delta, expected) in [
        (false, Vec2::new(33., 65.), [56., 88.]),
        (true, Vec2::new(33., 65.), [57., 89.]),
    ] {
        let mut h = Harness::new();
        h.shift = shift;
        h.repeat = true;
        let original = h.model.clone();
        let body = h.port(0, None) - Vec2::new(63., 0.);
        h.pointer(body, true, egui::PointerButton::Primary);
        h.frame(vec![egui::Event::PointerMoved(body + delta * 0.5)]);
        assert!(h.canvas.moving.is_some());
        assert!(h.model.same_storage(&original));
        h.frame(vec![egui::Event::PointerMoved(body + delta)]);
        h.pointer(body + delta, false, egui::PointerButton::Primary);
        assert_eq!(h.model.version, 2);
        assert_eq!(h.model.positions[0], expected);
        assert_eq!(h.canvas.history.len(), 1);
        assert_eq!(
            h.model.run(&AtomicBool::new(false)).unwrap().traces[0]
                .values
                .data(),
            &[2.; 4]
        );
        h.button("Undo diagram");
        assert!(h.model.same_storage(&original));
        h.button("Redo diagram");
        assert_eq!(h.model.positions[0], expected);
        let source = h.port(1, None);
        h.click(source);
        let target = h.port(2, Some(0));
        h.click(target);
        assert_eq!(h.model.nodes[2].inputs, [1]);
        h.button("Undo diagram");
        assert_eq!(h.model.nodes[2].inputs, [0]);
        assert_eq!(h.model.positions[0], expected);
        h.button("Auto arrange");
        assert!(h.model.same_storage(&original));
        h.button("Undo diagram");
        assert_eq!(h.model.positions[0], expected);
        h.model.positions[0][0] += 1.;
        let externally_changed = h.model.clone();
        h.button("Undo diagram");
        assert!(h.model.same_storage(&externally_changed));
        assert!(h.canvas.history.is_empty());
    }
}

#[test]
fn model_layout_escape_disable_and_stale_layout_cancel_moves() {
    let mut h = Harness::new();
    for mode in 0..3 {
        let body = h.port(0, None) - Vec2::new(63., 0.);
        h.pointer(body, true, egui::PointerButton::Primary);
        h.frame(vec![egui::Event::PointerMoved(body + Vec2::new(16., 16.))]);
        assert!(h.canvas.moving.is_some());
        match mode {
            0 => {
                h.frame(vec![egui::Event::Key {
                    key: egui::Key::Escape,
                    physical_key: None,
                    pressed: true,
                    repeat: false,
                    modifiers: egui::Modifiers::NONE,
                }]);
            }
            1 => {
                h.enabled = false;
                h.frame(vec![]);
            }
            _ => {
                h.model
                    .set_layout(vec![[24., 40.], [194., 40.], [364., 40.]])
                    .unwrap();
                h.frame(vec![]);
            }
        }
        assert!(h.canvas.moving.is_none());
        h.pointer(
            body + Vec2::new(32., 32.),
            false,
            egui::PointerButton::Primary,
        );
        assert_eq!(
            h.model.positions.first().copied(),
            if mode == 2 { Some([24., 40.]) } else { None }
        );
        assert!(h.canvas.history.is_empty());
        h.enabled = true;
    }
    let mut h = Harness::new();
    let original = h.model.clone();
    let body = h.port(0, None) - Vec2::new(63., 0.);
    h.pointer(body, true, egui::PointerButton::Primary);
    h.frame(vec![egui::Event::PointerMoved(body + Vec2::new(16., 16.))]);
    assert!(h.canvas.moving.is_some());
    h.pointer(Pos2::new(1000., 800.), false, egui::PointerButton::Primary);
    assert!(h.canvas.moving.is_none());
    assert!(h.model.same_storage(&original));
    assert!(h.canvas.history.is_empty());
}

#[test]
fn model_layout_topology_maintenance_and_reveal_preserve_custom_positions() {
    let mut tools = super::super::Toolkits::default();
    tools
        .model
        .set_layout((0..5).map(|i| [24. + i as f32 * 170., 24.]).collect())
        .unwrap();
    tools.selected = 1;
    tools.delete_block();
    assert_eq!(
        tools.model.positions,
        vec![[24., 24.], [364., 24.], [534., 24.], [704., 24.]]
    );
    assert_eq!(tools.model.nodes[1].inputs, [0, usize::MAX]);
    tools.add_block(Block::Gain { gain: 2. });
    assert_eq!(tools.model.positions.len(), tools.model.nodes.len());
    assert!(tools.model.validate_storage().is_ok());
    tools.canvas.source = Some(0);
    tools.canvas.moving = Some(layout::Moving::new(
        &tools.model,
        tools.model.positions.clone(),
        0,
        Pos2::ZERO,
    ));
    let ctx = egui::Context::default();
    let _ = crate::headless::run(&ctx, egui::RawInput::default(), |root| {
        let ctx = &root.ctx().clone();
        tools.ui(ctx, false);
    });
    assert!(tools.canvas.moving.is_none());
    let mut h = Harness::new();
    h.model
        .set_layout(vec![[10000., 10000.], [10400., 10000.], [10800., 10000.]])
        .unwrap();
    let before = h.model.clone();
    h.canvas.reveal(0);
    // Default egui scroll animation lasts up to 0.3 seconds (18 frames).
    for _ in 0..24 {
        h.frame(vec![]);
    }
    assert!(
        h.canvas
            .ports
            .iter()
            .any(|(node, port, _)| *node == 0 && port.is_none())
    );
    assert!(h.model.same_storage(&before));
}
