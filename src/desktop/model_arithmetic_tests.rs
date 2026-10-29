use super::*;
use matrust::simulation::{Block, Node};
use std::sync::atomic::AtomicBool;

#[test]
fn model_arithmetic_hashes_track_every_parameter_and_clipboard_keeps_behavior() {
    let mut model = Model {
        nodes: vec![
            Node {
                name: "Input".into(),
                block: Block::Constant { value: -3. },
                inputs: vec![],
            },
            Node {
                name: "Abs".into(),
                block: Block::Abs,
                inputs: vec![0],
            },
            Node {
                name: "Bias".into(),
                block: Block::Bias { bias: -0. },
                inputs: vec![1],
            },
            Node {
                name: "Saturation".into(),
                block: Block::Saturation {
                    lower: -0.,
                    upper: 2.,
                },
                inputs: vec![2],
            },
            Node {
                name: "Scope".into(),
                block: Block::Scope,
                inputs: vec![3],
            },
        ],
        steps: 4,
        ..Model::default()
    };
    let before = topology::node_hash(&model.nodes[2]);
    model.nodes[2].block = Block::Bias { bias: 0. };
    assert_ne!(before, topology::node_hash(&model.nodes[2]));
    model.nodes[2].block = Block::Bias { bias: -0. };
    for block in [
        Block::Saturation {
            lower: 0.,
            upper: 2.,
        },
        Block::Saturation {
            lower: -0.,
            upper: 3.,
        },
    ] {
        let before = topology::node_hash(&model.nodes[3]);
        let mut changed = model.nodes[3].clone();
        changed.block = block;
        assert_ne!(before, topology::node_hash(&changed));
    }
    let text = clipboard::Fragment::capture(&model, &[0, 1, 2, 3, 4], 900.)
        .unwrap()
        .encode()
        .unwrap();
    let mut target = Model {
        nodes: vec![],
        steps: 4,
        ..Model::default()
    };
    clipboard::Change::insert(
        &mut target,
        clipboard::Fragment::decode(&text).unwrap(),
        900.,
        0.,
        vec![],
        HISTORY_BYTES,
    )
    .unwrap();
    let cancel = AtomicBool::new(false);
    assert_eq!(
        target.run(&cancel).unwrap().traces[0].values.data(),
        model.run(&cancel).unwrap().traces[0].values.data()
    );
    assert!(
        matches!(target.nodes[2].block, Block::Bias { bias } if bias.to_bits() == (-0_f64).to_bits())
    );
    assert!(
        matches!(target.nodes[3].block, Block::Saturation { lower, upper: 2. } if lower.to_bits() == (-0_f64).to_bits())
    );
}

#[test]
fn model_arithmetic_palette_adds_one_input_blocks_and_shows_parameters() {
    for (kind, label, parameter) in [
        (9, "Abs", None),
        (10, "Bias", Some("Bias")),
        (11, "Saturation", Some("Upper limit")),
    ] {
        let ctx = egui::Context::default();
        let mut tools = super::super::Toolkits {
            model_open: true,
            add_kind: kind,
            ..Default::default()
        };
        tools.model.nodes.clear();
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
                    let ctx = root.ctx().clone();
                    tools.ui(&ctx, false);
                    if ctx.current_pass_index() == 0 {
                        ctx.request_discard("arithmetic palette input applies once");
                    }
                },
            )
        };
        fn find(shape: &egui::Shape, text: &str) -> Option<Pos2> {
            match shape {
                egui::Shape::Text(t) if t.galley.text() == text => {
                    Some(t.pos + t.galley.size() * 0.5)
                }
                egui::Shape::Vec(shapes) => shapes.iter().find_map(|s| find(s, text)),
                _ => None,
            }
        }
        for _ in 0..3 {
            frame(&mut tools, vec![]);
        }
        let output = frame(&mut tools, vec![]);
        let point = output
            .shapes
            .iter()
            .find_map(|s| find(&s.shape, "Add block"))
            .unwrap();
        for pressed in [true, false] {
            frame(
                &mut tools,
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
        assert_eq!(tools.model.nodes.len(), 1);
        assert_eq!(tools.model.nodes[0].block.label(), label);
        assert_eq!(tools.model.nodes[0].inputs, [usize::MAX]);
        assert_eq!(tools.canvas.history.len(), 1);
        assert!(
            tools.take_diagram_change(),
            "insertion schedules model recovery"
        );
        if kind == 11 {
            assert!(matches!(
                tools.model.nodes[0].block,
                Block::Saturation {
                    lower: -0.5,
                    upper: 0.5
                }
            ));
        }

        if let Some(parameter) = parameter {
            assert!(
                frame(&mut tools, vec![])
                    .shapes
                    .iter()
                    .any(|s| find(&s.shape, parameter).is_some())
            );
        }
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
        assert_eq!(tools.model.nodes[0].block.label(), label);
    }
}

#[test]
fn model_arithmetic_cached_readiness_rechecks_new_parameters() {
    let mut model = Model {
        steps: 4,
        nodes: vec![
            Node {
                name: "Input".into(),
                block: Block::Constant { value: 0. },
                inputs: vec![],
            },
            Node {
                name: "Large filter".into(),
                block: Block::Filter {
                    b: vec![0.; 4096],
                    a: vec![1.],
                },
                inputs: vec![0],
            },
            Node {
                name: "Bias".into(),
                block: Block::Bias { bias: 0. },
                inputs: vec![1],
            },
            Node {
                name: "Limit".into(),
                block: Block::Saturation {
                    lower: -0.5,
                    upper: 0.5,
                },
                inputs: vec![2],
            },
            Node {
                name: "Scope".into(),
                block: Block::Scope,
                inputs: vec![3],
            },
        ],
        ..Model::default()
    };
    let mut cache = super::validation::Cache::default();
    // >=4096 coefficients takes the cached-stamp path instead of always validating.
    for _ in 0..3 {
        assert!(cache.status(&model, false).is_ok());
    }
    for invalid in [
        Block::Bias { bias: f64::NAN },
        Block::Bias {
            bias: f64::INFINITY,
        },
    ] {
        model.nodes[2].block = invalid;
        assert_eq!(
            cache.status(&model, false).map_err(str::to_owned),
            model.validate().map(|_| ())
        );
        assert!(cache.status(&model, false).is_err());
        model.nodes[2].block = Block::Bias { bias: -0. };
        assert!(cache.status(&model, false).is_ok());
        assert!(cache.status(&model, false).is_ok());
    }
    for invalid in [
        Block::Saturation {
            lower: f64::NAN,
            upper: 0.5,
        },
        Block::Saturation {
            lower: -0.5,
            upper: f64::NAN,
        },
        Block::Saturation {
            lower: 0.75,
            upper: 0.5,
        },
        Block::Saturation {
            lower: -0.5,
            upper: -0.75,
        },
    ] {
        model.nodes[3].block = invalid;
        assert_eq!(
            cache.status(&model, false).map_err(str::to_owned),
            model.validate().map(|_| ())
        );
        assert!(cache.status(&model, false).is_err());
        model.nodes[3].block = Block::Saturation {
            lower: -0.5,
            upper: 0.5,
        };
        assert!(cache.status(&model, false).is_ok());
        assert!(cache.status(&model, false).is_ok());
    }
}
