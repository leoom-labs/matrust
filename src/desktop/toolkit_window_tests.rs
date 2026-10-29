use super::*;
use egui::{Event, Pos2, Rect, Vec2};

fn frame(
    ctx: &egui::Context,
    tools: &mut Toolkits,
    size: Vec2,
    events: Vec<Event>,
) -> egui::FullOutput {
    crate::headless::run(
        ctx,
        egui::RawInput {
            screen_rect: Some(Rect::from_min_size(Pos2::ZERO, size)),
            events,
            ..Default::default()
        },
        |root| {
            let ctx = &root.ctx().clone();
            tools.ui(ctx, false);
        },
    )
}

fn find(shape: &egui::Shape, label: &str) -> Option<Rect> {
    match shape {
        egui::Shape::Text(text) if text.galley.text() == label => {
            Some(Rect::from_min_size(text.pos, text.galley.size()))
        }
        egui::Shape::Vec(shapes) => shapes.iter().find_map(|s| find(s, label)),
        _ => None,
    }
}
fn visible(output: &egui::FullOutput, label: &str) -> Option<Rect> {
    output
        .shapes
        .iter()
        .find_map(|s| find(&s.shape, label).filter(|r| s.clip_rect.expand(0.1).contains_rect(*r)))
}

#[test]
fn toolkit_tabs_fill_compact_viewports_and_close_without_edits() {
    for tool in [Tool::Model, Tool::Designer] {
        let ctx = egui::Context::default();
        let mut tools = Toolkits::default();
        tools.open_tool(tool);
        assert_eq!(tools.selected_tool(), Some(tool));
        assert_eq!(tools.tabs().collect::<Vec<_>>(), [tool]);
        let original = tools.model.clone();
        for size in [
            Vec2::new(1440., 900.),
            Vec2::new(960., 650.),
            Vec2::new(640., 480.),
        ] {
            for _ in 0..5 {
                frame(&ctx, &mut tools, size, vec![]);
            }
            let rect = match tool {
                Tool::Designer => tools.designer_region(),
                Tool::Model => tools.model_document_region().map(|(rect, _)| rect),
            }
            .unwrap();
            assert!(
                Rect::from_min_size(Pos2::ZERO, size).contains_rect(rect),
                "Tab {tool:?} out of {size:?}: {rect:?}"
            );
            // Docked content fills the host: no floating window remains.
            assert!(rect.width() > size.x - 40. && rect.height() > size.y - 40.);
            for id in ["Filter Designer", "block_simulation"] {
                assert!(ctx.memory(|m| m.area_rect(egui::Id::new(id))).is_none());
            }
        }
        tools.close_tool(tool);
        assert!(!tools.model_open && !tools.designer_open);
        assert_eq!(tools.selected_tool(), None);
        frame(&ctx, &mut tools, Vec2::new(640., 480.), vec![]);
        assert!(tools.designer_region().is_none() && tools.model_document_region().is_none());
        assert!(tools.model.same_storage(&original));
        assert!(!tools.take_diagram_change());
    }
}

#[test]
fn tool_tabs_open_in_order_select_and_fall_back_on_close() {
    let mut tools = Toolkits::default();
    assert_eq!(tools.tabs().next(), None);
    tools.open_tool(Tool::Model);
    tools.open_tool(Tool::Designer);
    // Opening order, like editor tabs; the newest tab is selected.
    assert_eq!(
        tools.tabs().collect::<Vec<_>>(),
        [Tool::Model, Tool::Designer]
    );
    assert_eq!(tools.selected_tool(), Some(Tool::Designer));
    // Reopening an open tool only selects it.
    tools.open_tool(Tool::Model);
    assert_eq!(
        tools.tabs().collect::<Vec<_>>(),
        [Tool::Model, Tool::Designer]
    );
    assert_eq!(tools.selected_tool(), Some(Tool::Model));
    tools.deselect_tool();
    assert_eq!(tools.selected_tool(), None);
    assert!(tools.model_open && tools.designer_open);
    tools.select_tool(Tool::Model);
    tools.close_tool(Tool::Model);
    assert_eq!(tools.selected_tool(), Some(Tool::Designer));
    tools.open_tool(Tool::Model);
    assert_eq!(
        tools.tabs().collect::<Vec<_>>(),
        [Tool::Designer, Tool::Model]
    );
    tools.close_tool(Tool::Designer);
    assert_eq!(tools.selected_tool(), Some(Tool::Model));
    tools.close_tool(Tool::Model);
    assert_eq!((tools.selected_tool(), tools.tabs().next()), (None, None));
    tools.model.steps = 77;
    assert_eq!(tools.tab_label(Tool::Model), "Block Simulation *");
    assert_eq!(tools.tab_label(Tool::Designer), "Filter Designer");
    tools.select_tool(Tool::Designer);
    assert_eq!(
        tools.selected_tool(),
        None,
        "closed tools cannot be selected"
    );
}

#[test]
fn toolkit_many_input_parameters_scroll_without_growing_window() {
    let ctx = egui::Context::default();
    let size = Vec2::new(960., 650.);
    let mut tools = Toolkits {
        model_open: true,
        ..Default::default()
    };
    tools.model.nodes[2].inputs = vec![0; 32];
    tools.selected = 2;
    let original = tools.model.clone();
    for _ in 0..5 {
        frame(&ctx, &mut tools, size, vec![]);
    }
    let output = frame(&ctx, &mut tools, size, vec![]);
    let first = visible(&output, "Input 1: 1. Signal").expect("first port must be visible");
    assert!(visible(&output, "Input 32: 1. Signal").is_none());
    for _ in 0..12 {
        frame(
            &ctx,
            &mut tools,
            size,
            vec![
                Event::PointerMoved(first.center()),
                Event::MouseWheel {
                    phase: egui::TouchPhase::Move,
                    unit: egui::MouseWheelUnit::Point,
                    delta: egui::vec2(0., -100.),
                    modifiers: egui::Modifiers::NONE,
                },
            ],
        );
    }
    for _ in 0..20 {
        frame(&ctx, &mut tools, size, vec![]);
    }
    let output = frame(&ctx, &mut tools, size, vec![]);
    assert!(
        visible(&output, "Input 32: 1. Signal").is_some(),
        "last port must be reachable by scroll"
    );
    assert!(visible(&output, "Delete selected block").is_some());
    let (rect, _) = tools.model_document_region().unwrap();
    assert!(
        Rect::from_min_size(Pos2::ZERO, size).contains_rect(rect),
        "{rect:?}"
    );
    assert!(tools.model.same_storage(&original));
    assert!(!tools.take_diagram_change());
}

#[test]
fn opening_restoring_and_discarding_positioned_models_reveals_later_rows() {
    use matrust::simulation::{Block, Node, document::ModelFile};
    let model = Model {
        version: 2,
        nodes: vec![
            Node {
                name: "Sine input".into(),
                block: Block::Sine {
                    amplitude: 1.,
                    frequency_hz: 5.,
                    phase: 0.,
                },
                inputs: vec![],
            },
            Node {
                name: "Bias -0.25".into(),
                block: Block::Bias { bias: -0.25 },
                inputs: vec![0],
            },
            Node {
                name: "Absolute value".into(),
                block: Block::Abs,
                inputs: vec![1],
            },
            Node {
                name: "Clamp 0.1 to 0.75".into(),
                block: Block::Saturation {
                    lower: 0.1,
                    upper: 0.75,
                },
                inputs: vec![2],
            },
            Node {
                name: "Output".into(),
                block: Block::Scope,
                inputs: vec![3],
            },
        ],
        positions: vec![
            [30., 60.],
            [220., 60.],
            [410., 60.],
            [410., 220.],
            [220., 220.],
        ],
        ..Model::default()
    };
    model.validate_storage().unwrap();
    for size in [Vec2::new(960., 650.), Vec2::new(1440., 900.)] {
        for mode in [0, 1, 2] {
            let ctx = egui::Context::default();
            let mut tools = Toolkits {
                model_open: true,
                ..Default::default()
            };
            match mode {
                0 => tools.finish_model_read(crate::model_read::Finished {
                    purpose: crate::model_read::Purpose::Confirmed,
                    source: None,
                    imported: false,
                    result: Ok((
                        model.clone(),
                        ModelFile::restored(None, Some(model.clone())),
                    )),
                }),
                1 => tools.restore_model(model.clone(), None, Some(model.clone())),
                _ => {
                    tools.restore_model(model.clone(), None, Some(model.clone()));
                    tools.model.positions[4] = [9000., 9000.];
                    tools.discard_model();
                }
            }
            for _ in 0..8 {
                frame(&ctx, &mut tools, size, vec![]);
            }
            let output = frame(&ctx, &mut tools, size, vec![]);
            // Canvas labels include the block type on a separate line; the list
            // uses only numbered names, so it cannot satisfy these assertions.
            for index in [0, 3, 4] {
                let node = &model.nodes[index];
                let name: String = node.name.chars().take(16).collect();
                let label = format!("{}. {name}\n{}", index + 1, node.block.label());
                assert!(
                    visible(&output, &label).is_some(),
                    "clipped {label} mode={mode} at {size:?}"
                );
            }
            assert!(tools.model.same_storage(&model));
            assert!(!tools.model_dirty());
            assert!(!tools.take_diagram_change());
        }
    }
}

#[test]
fn model_validation_status_paints_theme_colors_without_editing_graph() {
    for theme in [egui::Theme::Light, egui::Theme::Dark] {
        let ctx = egui::Context::default();
        crate::theme::configure(&ctx);
        ctx.set_theme(theme);
        let mut tools = Toolkits {
            model_open: true,
            ..Default::default()
        };
        for invalid in [false, true] {
            if invalid {
                tools.model.nodes[3].inputs[0] = 3;
            }
            let before = serde_json::to_vec(&tools.model).unwrap();
            let label = if invalid {
                tools.model.validate().unwrap_err()
            } else {
                "Model valid \u{b7} ready to simulate".into()
            };
            for _ in 0..4 {
                frame(&ctx, &mut tools, Vec2::new(1440., 900.), vec![]);
            }
            let output = frame(&ctx, &mut tools, Vec2::new(1440., 900.), vec![]);
            let expected = match (theme, invalid) {
                (egui::Theme::Dark, true) => crate::tokens::DARK.error,
                (egui::Theme::Dark, false) => crate::tokens::DARK.success,
                (_, true) => crate::tokens::LIGHT.error,
                (_, false) => crate::tokens::LIGHT.success,
            };
            crate::theme::assert_painted_status(&output, &label, expected);
            assert_eq!(serde_json::to_vec(&tools.model).unwrap(), before);
        }
    }
}
