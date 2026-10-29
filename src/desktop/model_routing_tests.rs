use super::*;
use matrust::simulation::{Block, Node, ProductInput as P, SwitchCriteria as C};
use parameters::{Edit, Properties};
use std::sync::atomic::AtomicBool;

fn find(shape: &egui::Shape, text: &str) -> Option<Pos2> {
    match shape {
        egui::Shape::Text(t) if t.galley.text() == text => Some(t.pos + t.galley.size() * 0.5),
        egui::Shape::Vec(shapes) => shapes.iter().find_map(|s| find(s, text)),
        _ => None,
    }
}
fn frame(
    ctx: &egui::Context,
    tools: &mut super::super::Toolkits,
    events: Vec<egui::Event>,
) -> egui::FullOutput {
    crate::headless::run(
        ctx,
        egui::RawInput {
            screen_rect: Some(Rect::from_min_size(Pos2::ZERO, Vec2::new(1440., 900.))),
            events,
            ..Default::default()
        },
        |root| {
            tools.ui(root.ctx(), false);
            if root.ctx().current_pass_index() == 0 {
                root.ctx()
                    .request_discard("routing edits must apply once per frame");
            }
        },
    )
}
fn click(ctx: &egui::Context, tools: &mut super::super::Toolkits, text: &str) {
    let output = frame(ctx, tools, vec![]);
    let point = output
        .shapes
        .iter()
        .find_map(|s| find(&s.shape, text))
        .unwrap_or_else(|| panic!("Missing {text}"));
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
#[test]
fn model_routing_palette_default_ports_and_undo_redo() {
    for (kind, label, ports) in [
        (12, "Product", 2),
        (13, "Unary Minus", 1),
        (14, "Switch", 3),
    ] {
        let ctx = egui::Context::default();
        let mut tools = super::super::Toolkits {
            model_open: true,
            add_kind: kind,
            ..Default::default()
        };
        tools.model.nodes.clear();
        tools.selected = 0;
        for _ in 0..3 {
            frame(&ctx, &mut tools, vec![]);
        }
        click(&ctx, &mut tools, "Add block");
        assert_eq!(tools.model.nodes.len(), 1);
        assert_eq!(tools.model.nodes[0].block.label(), label);
        assert_eq!(tools.model.nodes[0].inputs, vec![usize::MAX; ports]);
        assert_eq!(tools.canvas.history.len(), 1);
        assert!(tools.take_diagram_change());
        let added = tools.model.clone();
        tools.canvas.travel(
            &mut tools.model,
            &mut tools.selected,
            false,
            &mut String::new(),
        );
        assert!(tools.model.nodes.is_empty());
        tools.canvas.travel(
            &mut tools.model,
            &mut tools.selected,
            true,
            &mut String::new(),
        );
        assert!(tools.model.same_storage(&added));
    }
}
#[test]
fn model_product_port_controls_never_drop_connected_wire_and_history_is_atomic() {
    let ctx = egui::Context::default();
    let mut tools = super::super::Toolkits {
        model_open: true,
        ..Default::default()
    };
    tools.model.nodes = vec![
        Node {
            name: "Source".into(),
            block: Block::Constant { value: 2. },
            inputs: vec![],
        },
        Node {
            name: "Product".into(),
            block: Block::Product {
                operations: vec![P::Multiply; 2],
            },
            inputs: vec![0, 0],
        },
    ];
    tools.selected = 1;
    for _ in 0..3 {
        frame(&ctx, &mut tools, vec![]);
    }
    let original = tools.model.clone();
    click(&ctx, &mut tools, "Remove last input");
    assert!(tools.model.same_storage(&original));
    assert!(tools.canvas.history.is_empty());
    click(&ctx, &mut tools, "Add input");
    assert_eq!(tools.model.nodes[1].inputs, [0, 0, usize::MAX]);
    assert_eq!(tools.canvas.history.len(), 1);
    let grown = tools.model.clone();
    tools.canvas.travel(
        &mut tools.model,
        &mut tools.selected,
        false,
        &mut String::new(),
    );
    assert!(tools.model.same_storage(&original));
    tools.canvas.travel(
        &mut tools.model,
        &mut tools.selected,
        true,
        &mut String::new(),
    );
    assert!(tools.model.same_storage(&grown));
    click(&ctx, &mut tools, "Remove last input");
    assert!(tools.model.same_storage(&original));
    tools.canvas.travel(
        &mut tools.model,
        &mut tools.selected,
        false,
        &mut String::new(),
    );
    assert!(tools.model.same_storage(&grown));
}
#[test]
fn model_routing_parameter_history_hashes_and_stale_guard() {
    for (before, after, inputs, new_inputs) in [
        (
            Block::Product {
                operations: vec![P::Multiply; 2],
            },
            Block::Product {
                operations: vec![P::Multiply, P::Divide, P::Multiply],
            },
            vec![0, 1],
            vec![0, 1, usize::MAX],
        ),
        (
            Block::Switch {
                criteria: C::Greater,
                threshold: -0.,
            },
            Block::Switch {
                criteria: C::Nonzero,
                threshold: 0.,
            },
            vec![0, 1, 2],
            vec![0, 1, 2],
        ),
    ] {
        let mut m = Model {
            nodes: vec![Node {
                name: "Block".into(),
                block: before,
                inputs,
            }],
            ..Model::default()
        };
        let original = m.clone();
        let snapshot = Properties::capture(&m.nodes[0]);
        let hash = topology::node_hash(&m.nodes[0]);
        m.nodes[0].block = after;
        m.nodes[0].inputs = new_inputs;
        assert_ne!(hash, topology::node_hash(&m.nodes[0]));
        let changed = m.clone();
        let edit = Edit::node(&m, 0, snapshot).unwrap();
        assert!(edit.heap_bytes() < 4096);
        assert!(edit.apply(&mut m, false));
        assert!(m.same_storage(&original));
        assert!(edit.apply(&mut m, true));
        assert!(m.same_storage(&changed));
        m.nodes[0].name = "External edit".into();
        let stale = m.clone();
        assert!(!edit.apply(&mut m, false));
        assert!(m.same_storage(&stale));
    }
    let a = Node {
        name: "S".into(),
        block: Block::Switch {
            criteria: C::Greater,
            threshold: -0.,
        },
        inputs: vec![0, 0, 0],
    };
    let mut b = a.clone();
    b.block = Block::Switch {
        criteria: C::Greater,
        threshold: 0.,
    };
    assert_ne!(topology::node_hash(&a), topology::node_hash(&b));
}
#[test]
fn model_routing_clipboard_and_cached_validation_cover_new_parameters() {
    let mut m = Model {
        steps: 4,
        nodes: vec![
            Node {
                name: "C".into(),
                block: Block::Constant { value: 2. },
                inputs: vec![],
            },
            Node {
                name: "P".into(),
                block: Block::Product {
                    operations: vec![P::Multiply, P::Divide],
                },
                inputs: vec![0, 0],
            },
            Node {
                name: "N".into(),
                block: Block::UnaryMinus,
                inputs: vec![1],
            },
            Node {
                name: "S".into(),
                block: Block::Switch {
                    criteria: C::GreaterEqual,
                    threshold: -0.,
                },
                inputs: vec![2, 0, 1],
            },
            Node {
                name: "F".into(),
                block: Block::Filter {
                    b: vec![0.; 4096],
                    a: vec![1.],
                },
                inputs: vec![3],
            },
            Node {
                name: "O".into(),
                block: Block::Scope,
                inputs: vec![3],
            },
        ],
        ..Model::default()
    };
    let text = clipboard::Fragment::capture(&m, &[0, 1, 2, 3, 4, 5], 900.)
        .unwrap()
        .encode()
        .unwrap();
    let mut pasted = Model {
        nodes: vec![],
        steps: 4,
        ..Model::default()
    };
    clipboard::Change::insert(
        &mut pasted,
        clipboard::Fragment::decode(&text).unwrap(),
        900.,
        0.,
        vec![],
        HISTORY_BYTES,
    )
    .unwrap();
    let cancel = AtomicBool::new(false);
    assert_eq!(
        m.run(&cancel).unwrap().traces[0].values.data(),
        pasted.run(&cancel).unwrap().traces[0].values.data()
    );
    let mut cache = validation::Cache::default();
    assert!(cache.status(&m, false).is_ok());
    m.nodes[3].block = Block::Switch {
        criteria: C::Greater,
        threshold: f64::NAN,
    };
    assert!(cache.status(&m, false).is_err());
    m.nodes[3].block = Block::Switch {
        criteria: C::Nonzero,
        threshold: 0.,
    };
    assert!(cache.status(&m, false).is_ok());
    m.nodes[1].block = Block::Product {
        operations: vec![P::Multiply; 33],
    };
    assert!(cache.status(&m, false).is_err());
    assert!(clipboard::Fragment::capture(&m, &[1], 900.).is_err());
}

#[test]
fn model_switch_threshold_drag_coalesces_once_across_discarded_passes() {
    let mut results = Vec::new();
    for repeat in [false, true] {
        let ctx = egui::Context::default();
        let mut tools = super::super::Toolkits {
            model_open: true,
            ..Default::default()
        };
        tools.model.nodes = vec![Node {
            name: "Switch".into(),
            block: Block::Switch {
                criteria: C::Greater,
                threshold: 123.45,
            },
            inputs: vec![usize::MAX; 3],
        }];
        tools.selected = 0;
        let frame = |tools: &mut super::super::Toolkits, events| {
            crate::headless::run(
                &ctx,
                egui::RawInput {
                    screen_rect: Some(Rect::from_min_size(Pos2::ZERO, Vec2::new(1440., 900.))),
                    events,
                    ..Default::default()
                },
                |root| {
                    tools.ui(root.ctx(), false);
                    if repeat && root.ctx().current_pass_index() == 0 {
                        root.ctx()
                            .request_discard("threshold interaction must coalesce");
                    }
                },
            )
        };
        for _ in 0..3 {
            frame(&mut tools, vec![]);
        }
        let output = frame(&mut tools, vec![]);
        let p = output
            .shapes
            .iter()
            .find_map(|s| find(&s.shape, "123.45"))
            .expect("Threshold value visible");
        let pointer = |p, pressed| {
            vec![
                egui::Event::PointerMoved(p),
                egui::Event::PointerButton {
                    pos: p,
                    button: egui::PointerButton::Primary,
                    pressed,
                    modifiers: egui::Modifiers::NONE,
                },
            ]
        };
        frame(&mut tools, pointer(p, true));
        for dx in [20., 40., 60.] {
            frame(
                &mut tools,
                vec![egui::Event::PointerMoved(p + Vec2::new(dx, 0.))],
            );
        }
        frame(&mut tools, pointer(p + Vec2::new(60., 0.), false));
        let Block::Switch { threshold, .. } = tools.model.nodes[0].block else {
            panic!()
        };
        assert!(threshold > 123.45);
        results.push(threshold.to_bits());
        assert_eq!(tools.canvas.history.len(), 1);
        tools.canvas.travel(
            &mut tools.model,
            &mut tools.selected,
            false,
            &mut String::new(),
        );
        assert!(matches!(
            tools.model.nodes[0].block,
            Block::Switch {
                threshold: 123.45,
                ..
            }
        ));
    }
    assert_eq!(results[0], results[1]);
}
