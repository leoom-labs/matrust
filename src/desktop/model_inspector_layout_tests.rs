//! Initial selected controls and routed graph must coexist without outer scrolling.
use super::*;
use matrust::simulation::{Block, Node, ProductInput, SwitchCriteria};

fn visible_text(shape: &egui::Shape, clip: Rect, label: &str) -> Option<Rect> {
    match shape {
        egui::Shape::Text(text) if text.galley.text() == label => {
            let rect = Rect::from_min_size(text.pos, text.galley.size());
            clip.contains_rect(rect).then_some(rect)
        }
        egui::Shape::Vec(shapes) => shapes.iter().find_map(|s| visible_text(s, clip, label)),
        _ => None,
    }
}
fn visible(output: &egui::FullOutput, size: Vec2, label: &str) -> Rect {
    let viewport = Rect::from_min_size(Pos2::ZERO, size);
    output
        .shapes
        .iter()
        .find_map(|s| visible_text(&s.shape, s.clip_rect.intersect(viewport), label))
        .unwrap_or_else(|| panic!("Fully visible control missing: {label}"))
}
fn frame(
    ctx: &egui::Context,
    tools: &mut super::super::Toolkits,
    size: Vec2,
    events: Vec<egui::Event>,
) -> egui::FullOutput {
    crate::headless::run(
        ctx,
        egui::RawInput {
            screen_rect: Some(Rect::from_min_size(Pos2::ZERO, size)),
            events,
            ..Default::default()
        },
        |root| {
            // Same content exclusion as the desktop toolbar/folder rows and tab
            // strip: the tab hosts the model in the desktop's central panel.
            egui::Panel::top("toolbar")
                .exact_size(100.)
                .show(root, |_| {});
            let mut pass = tools.begin_pass(root.ctx());
            egui::CentralPanel::default().show(root, |ui| {
                tools.tab_ui(ui, super::super::Tool::Model, false, &mut pass);
            });
            assert!(tools.finish_pass(root.ctx(), pass).is_none());
            if root.ctx().current_pass_index() == 0 {
                root.ctx()
                    .request_discard("Layout and interaction apply once per frame");
            }
        },
    )
}
fn settle(ctx: &egui::Context, tools: &mut super::super::Toolkits, size: Vec2) -> egui::FullOutput {
    for _ in 0..6 {
        frame(ctx, tools, size, vec![]);
    }
    frame(ctx, tools, size, vec![])
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
fn setup(block: Block) -> super::super::Toolkits {
    let inputs = if matches!(block, Block::Switch { .. }) {
        vec![1, 2, 3]
    } else {
        vec![1, 2]
    };
    let mut tools = super::super::Toolkits {
        model_open: true,
        ..Default::default()
    };
    tools.model = Model {
        version: 2,
        nodes: vec![
            Node {
                name: "Selected routing block".into(),
                block,
                inputs,
            },
            Node {
                name: "Top source".into(),
                block: Block::Constant { value: 6. },
                inputs: vec![],
            },
            Node {
                name: "Middle source".into(),
                block: Block::Constant { value: 2. },
                inputs: vec![],
            },
            Node {
                name: "Bottom source".into(),
                block: Block::Constant { value: -2. },
                inputs: vec![],
            },
            Node {
                name: "Output".into(),
                block: Block::Scope,
                inputs: vec![0],
            },
        ],
        positions: vec![[260., 100.], [0., 0.], [0., 100.], [0., 210.], [480., 100.]],
        ..Model::default()
    };
    tools.selected = 0;
    tools.canvas.reset_for_model(&tools.model, 0);
    tools
}

#[test]
fn selected_routing_controls_and_graph_fit_are_visible_wide_and_compact() {
    for size in [Vec2::new(1440., 900.), Vec2::new(960., 650.)] {
        for switch in [false, true] {
            let block = if switch {
                Block::Switch {
                    criteria: SwitchCriteria::GreaterEqual,
                    threshold: 123.45,
                }
            } else {
                Block::Product {
                    operations: vec![ProductInput::Multiply, ProductInput::Divide],
                }
            };
            let mut tools = setup(block);
            let original = tools.model.clone();
            let ctx = egui::Context::default();
            let output = settle(&ctx, &mut tools, size);
            for label in [
                "Sample time (s)",
                "New model",
                "Save model",
                "Add block",
                "Delete selected block",
            ] {
                visible(&output, size, label);
            }
            if switch {
                visible(&output, size, "u2 >= Threshold");
                visible(&output, size, "Threshold");
                visible(&output, size, "123.45");
            } else {
                for label in [
                    "Operation 1",
                    "Operation 2",
                    "Add input",
                    "Remove last input",
                ] {
                    visible(&output, size, label);
                }
            }
            assert!(tools.canvas.view.viewport.height() >= 240.);
            assert!(
                tools.canvas.view.zoom >= 0.7,
                "Initial labels must be readable, not just present"
            );
            for (index, node) in tools.model.nodes.iter().enumerate() {
                let name: String = node.name.chars().take(16).collect();
                let label = format!("{}. {name}\n{}", index + 1, node.block.label());
                let rect = visible(&output, size, &label);
                assert!(
                    tools.canvas.view.viewport.contains_rect(rect),
                    "Node text escaped canvas"
                );
            }
            assert!(tools.canvas.view.viewport.width() >= 400.);
            assert_eq!(tools.canvas.blocks.len(), tools.model.nodes.len());
            // Routing extends <=20 logical units beyond nodes. Fit reserves24,
            // so actual orthogonal lanes/ports remain inside the viewport too.
            for (_, rect) in &tools.canvas.blocks {
                assert!(
                    tools
                        .canvas
                        .view
                        .viewport
                        .expand(1.)
                        .contains_rect(rect.expand(20. * tools.canvas.view.zoom)),
                    "Fit clipped routing envelope: {rect:?}"
                );
            }
            assert!(tools.model.same_storage(&original));
            assert!(tools.canvas.history.is_empty());
            let zoom = tools.canvas.view.zoom;
            settle(&ctx, &mut tools, size);
            assert_eq!(
                tools.canvas.view.zoom, zoom,
                "Ordinary frames must not refit"
            );
        }
    }
}

#[test]
fn compact_product_add_and_switch_threshold_are_interactive_without_scroll() {
    let size = Vec2::new(960., 650.);
    let ctx = egui::Context::default();
    let mut tools = setup(Block::Product {
        operations: vec![ProductInput::Multiply, ProductInput::Divide],
    });
    let original = tools.model.clone();
    let output = settle(&ctx, &mut tools, size);
    let point = visible(&output, size, "Add input").center();
    for pressed in [true, false] {
        frame(&ctx, &mut tools, size, pointer(point, pressed));
    }
    assert_eq!(tools.model.nodes[0].inputs, vec![1, 2, usize::MAX]);
    assert_eq!(tools.canvas.history.len(), 1);
    tools.canvas.travel(
        &mut tools.model,
        &mut tools.selected,
        false,
        &mut String::new(),
    );
    assert!(tools.model.same_storage(&original));

    let ctx = egui::Context::default();
    let mut tools = setup(Block::Switch {
        criteria: SwitchCriteria::GreaterEqual,
        threshold: 123.45,
    });
    let original = tools.model.clone();
    let output = settle(&ctx, &mut tools, size);
    let point = visible(&output, size, "123.45").center();
    frame(&ctx, &mut tools, size, pointer(point, true));
    for dx in [20., 40., 60.] {
        frame(
            &ctx,
            &mut tools,
            size,
            vec![egui::Event::PointerMoved(point + Vec2::new(dx, 0.))],
        );
    }
    frame(
        &ctx,
        &mut tools,
        size,
        pointer(point + Vec2::new(60., 0.), false),
    );
    assert!(
        matches!(tools.model.nodes[0].block, Block::Switch { threshold, .. } if threshold > 123.45)
    );
    assert_eq!(tools.canvas.history.len(), 1);
    tools.canvas.travel(
        &mut tools.model,
        &mut tools.selected,
        false,
        &mut String::new(),
    );
    assert!(tools.model.same_storage(&original));
}

#[test]
fn padded_model_coordinates_roundtrip_and_pointer_zoom_stay_anchored() {
    let size = Vec2::new(960., 650.);
    let ctx = egui::Context::default();
    let mut tools = setup(Block::Switch {
        criteria: SwitchCriteria::Greater,
        threshold: 0.,
    });
    settle(&ctx, &mut tools, size);
    // Fit can leave a narrow graph whose horizontal offset is clamped at zero.
    // Test anchoring with real overflow in both dimensions, away from bounds.
    tools.canvas.view.zoom = 2.;
    tools.canvas.view.requested_offset = Some(Vec2::new(120., 120.));
    settle(&ctx, &mut tools, size);
    assert!(tools.canvas.view.offset.x > 100. && tools.canvas.view.offset.y > 100.);
    let original = tools.model.clone();
    let viewport = tools.canvas.view.viewport;
    let pointer = viewport.center();
    let before = tools
        .canvas
        .view
        .model_point(pointer, tools.canvas.view.origin);
    frame(
        &ctx,
        &mut tools,
        size,
        vec![
            egui::Event::PointerMoved(pointer),
            egui::Event::MouseWheel {
                phase: egui::TouchPhase::Move,
                unit: egui::MouseWheelUnit::Point,
                delta: Vec2::new(0., 50.),
                modifiers: egui::Modifiers::CTRL,
            },
        ],
    );
    settle(&ctx, &mut tools, size);
    let after = tools
        .canvas
        .view
        .model_point(pointer, tools.canvas.view.origin);
    assert!(
        (before - after).length() < 3.,
        "Padded origin drifted during pointer zoom: {before:?} -> {after:?}"
    );
    assert!(tools.model.same_storage(&original));
    assert!(tools.canvas.history.is_empty());
}

#[test]
fn high_zoom_bottom_and_right_wire_lanes_remain_scrollable() {
    let size = Vec2::new(960., 650.);
    let ctx = egui::Context::default();
    let mut tools = setup(Block::Product {
        operations: vec![ProductInput::Multiply, ProductInput::Divide],
    });
    // Port0 routes backwards from the rightmost source; port1 routes below the
    // lowest source. Both extend past node bounds in the actual routing code.
    tools.model.positions[1] = [700., 0.];
    tools.model.nodes[0].inputs = vec![1, 3];
    tools.canvas.reset_for_model(&tools.model, 0);
    settle(&ctx, &mut tools, size);
    let original = tools.model.clone();
    tools.canvas.view.zoom = 4.;
    tools.canvas.view.requested_offset = Some(Vec2::splat(1_000_000.));
    settle(&ctx, &mut tools, size);
    let origin = tools.canvas.view.origin;
    let viewport = tools.canvas.view.viewport;
    // Native source block width126 and lowest source bottom210+56. Routing
    // uses right+12 for port0, bottom+17 for port1, in logical model units.
    let right_lane = origin.x + (700. + 126. + 12.) * 4.;
    let bottom_lane = origin.y + (210. + 56. + 17.) * 4.;
    assert!(
        right_lane <= viewport.right() - 1.,
        "Right wire lane beyond scroll extent"
    );
    assert!(
        bottom_lane <= viewport.bottom() - 1.,
        "Bottom wire lane beyond scroll extent"
    );
    assert!(tools.canvas.view.offset.x > 0. && tools.canvas.view.offset.y > 0.);
    assert!(tools.model.same_storage(&original));
    assert!(tools.canvas.history.is_empty());
}

#[test]
fn padded_middle_pan_has_no_press_jump_and_uses_real_scroll_offset() {
    let size = Vec2::new(960., 650.);
    let ctx = egui::Context::default();
    let mut tools = setup(Block::Switch {
        criteria: SwitchCriteria::Greater,
        threshold: 0.,
    });
    settle(&ctx, &mut tools, size);
    // A transient zoom/scroll seed exercises a nonzero offset larger than the
    // padding. No graph/model state is modified by this view-only arrangement.
    tools.canvas.view.zoom = 2.;
    tools.canvas.view.requested_offset = Some(Vec2::new(120., 120.));
    settle(&ctx, &mut tools, size);
    let before = tools.canvas.view.offset;
    assert!(before.x > view::CONTENT_PAD * tools.canvas.view.zoom);
    assert!(before.y > view::CONTENT_PAD * tools.canvas.view.zoom);
    let original = tools.model.clone();
    let point = tools.canvas.view.viewport.center();
    let middle = |pos, pressed| {
        vec![
            egui::Event::PointerMoved(pos),
            egui::Event::PointerButton {
                pos,
                button: egui::PointerButton::Middle,
                pressed,
                modifiers: egui::Modifiers::NONE,
            },
        ]
    };
    frame(&ctx, &mut tools, size, middle(point, true));
    frame(&ctx, &mut tools, size, vec![]);
    assert!(
        (tools.canvas.view.offset - before).length() < 1.,
        "Middle press must not subtract content padding"
    );
    let delta = Vec2::new(-20., -15.);
    frame(
        &ctx,
        &mut tools,
        size,
        vec![egui::Event::PointerMoved(point + delta)],
    );
    frame(&ctx, &mut tools, size, middle(point + delta, false));
    settle(&ctx, &mut tools, size);
    assert!((tools.canvas.view.offset - (before - delta)).length() < 1.);
    assert!(tools.canvas.view.pan.is_none());
    assert!(tools.model.same_storage(&original));
    assert!(tools.canvas.history.is_empty());
}

#[test]
fn model_split_tabs_keep_graph_geometry_and_search_selection_available() {
    for size in [Vec2::new(1440., 900.), Vec2::new(960., 650.)] {
        let ctx = egui::Context::default();
        let mut tools = setup(Block::Switch {
            criteria: SwitchCriteria::GreaterEqual,
            threshold: 0.5,
        });
        let original = tools.model.clone();
        let output = settle(&ctx, &mut tools, size);
        let graph = tools.canvas.view.viewport;
        let blocks = visible(&output, size, "Blocks").center();
        for pressed in [true, false] {
            frame(&ctx, &mut tools, size, pointer(blocks, pressed));
        }
        let output = settle(&ctx, &mut tools, size);
        assert!(tools.model_blocks_tab);
        assert_eq!(tools.canvas.view.viewport, graph);
        let search = visible(&output, size, "Find blocks by name or type").center();
        for pressed in [true, false] {
            frame(&ctx, &mut tools, size, pointer(search, pressed));
        }
        frame(
            &ctx,
            &mut tools,
            size,
            vec![egui::Event::Text("Bottom".into())],
        );
        let output = settle(&ctx, &mut tools, size);
        let item = visible(&output, size, "4. Bottom source").center();
        for pressed in [true, false] {
            frame(&ctx, &mut tools, size, pointer(item, pressed));
        }
        assert_eq!(tools.selected, 3);
        assert!(
            tools.model_blocks_tab,
            "Selection must not hide the list during navigation"
        );
        let output = settle(&ctx, &mut tools, size);
        visible(&output, size, "4. Bottom source");
        let parameters = visible(&output, size, "Parameters").center();
        for pressed in [true, false] {
            frame(&ctx, &mut tools, size, pointer(parameters, pressed));
        }
        let output = settle(&ctx, &mut tools, size);
        assert!(!tools.model_blocks_tab);
        visible(&output, size, "Value");
        assert_eq!(tools.canvas.view.viewport, graph);
        assert!(tools.model.same_storage(&original));
        assert!(tools.canvas.history.is_empty());
    }
}

#[test]
fn model_split_wire_gesture_does_not_rewrap_toolbar_or_move_canvas() {
    let size = Vec2::new(960., 650.);
    let ctx = egui::Context::default();
    let mut tools = setup(Block::Switch {
        criteria: SwitchCriteria::GreaterEqual,
        threshold: 0.5,
    });
    let original = tools.model.clone();
    let output = settle(&ctx, &mut tools, size);
    visible(&output, size, "Cancel gesture");
    visible(&output, size, "View");
    let before = tools.canvas.view.viewport;
    let port = tools
        .canvas
        .ports
        .iter()
        .find(|(index, input, _)| *index == 1 && input.is_none())
        .unwrap()
        .2;
    frame(&ctx, &mut tools, size, pointer(port, true));
    assert_eq!(tools.canvas.view.viewport, before);
    frame(
        &ctx,
        &mut tools,
        size,
        vec![egui::Event::PointerMoved(port + Vec2::new(15., 0.))],
    );
    assert_eq!(tools.canvas.source, Some(1));
    assert!(tools.canvas.dragging);
    assert_eq!(
        tools.canvas.view.viewport, before,
        "Starting a wire shifted the diagram"
    );
    frame(
        &ctx,
        &mut tools,
        size,
        pointer(port + Vec2::new(15., 0.), false),
    );
    frame(
        &ctx,
        &mut tools,
        size,
        vec![egui::Event::Key {
            key: egui::Key::Escape,
            physical_key: None,
            pressed: true,
            repeat: false,
            modifiers: egui::Modifiers::NONE,
        }],
    );
    settle(&ctx, &mut tools, size);
    assert!(tools.canvas.source.is_none());
    assert_eq!(tools.canvas.view.viewport, before);
    assert!(tools.model.same_storage(&original));
    assert!(tools.canvas.history.is_empty());
}

#[test]
fn model_split_zoom_label_digit_changes_do_not_change_viewport() {
    let size = Vec2::new(960., 650.);
    let ctx = egui::Context::default();
    assert_eq!(
        ctx.global_style().spacing.scroll.allocated_width(),
        0.,
        "This geometry test uses default floating scrollbars"
    );
    let mut tools = setup(Block::Switch {
        criteria: SwitchCriteria::GreaterEqual,
        threshold: 0.5,
    });
    settle(&ctx, &mut tools, size);
    let before = tools.canvas.view.viewport;
    for zoom in [0.09, 0.1, 0.99, 1., 4., 0.0001] {
        tools.canvas.view.zoom = zoom;
        settle(&ctx, &mut tools, size);
        assert_eq!(
            tools.canvas.view.viewport, before,
            "Zoom label changed layout at {zoom}"
        );
    }
}

#[test]
fn model_split_parameters_keep_coefficient_editor_reachable() {
    let size = Vec2::new(960., 650.);
    let ctx = egui::Context::default();
    let mut tools = setup(Block::Filter {
        b: vec![0.5, 0.5],
        a: vec![1.],
    });
    tools.model.nodes[0].inputs = vec![1];
    let original = tools.model.clone();
    let output = settle(&ctx, &mut tools, size);
    let edit = visible(&output, size, "Edit coefficients\u{2026}").center();
    for pressed in [true, false] {
        frame(&ctx, &mut tools, size, pointer(edit, pressed));
    }
    assert!(tools.coefficient_editor.is_some());
    assert!(tools.model.same_storage(&original));
    assert!(tools.canvas.history.is_empty());
}

#[test]
fn model_split_long_input_names_keep_selector_and_popup_inside_inspector() {
    let size = Vec2::new(960., 650.);
    let ctx = egui::Context::default();
    let mut tools = setup(Block::Switch {
        criteria: SwitchCriteria::GreaterEqual,
        threshold: 0.5,
    });
    tools.model.nodes[1].name = "W".repeat(128);
    let original = tools.model.clone();
    let output = settle(&ctx, &mut tools, size);
    let graph = tools.canvas.view.viewport;
    fn input_rect(shape: &egui::Shape, clip: Rect) -> Option<Rect> {
        match shape {
            egui::Shape::Text(text) if text.galley.text().starts_with("Input 1: 2. W") => {
                let rect = Rect::from_min_size(text.pos, text.galley.size());
                assert!(
                    clip.contains_rect(rect),
                    "Selected input text extends beyond inspector"
                );
                Some(rect)
            }
            egui::Shape::Vec(shapes) => shapes.iter().find_map(|s| input_rect(s, clip)),
            _ => None,
        }
    }
    let text = output
        .shapes
        .iter()
        .find_map(|s| input_rect(&s.shape, s.clip_rect))
        .expect("Long input selector label");
    assert!(text.width() <= 320.);
    assert!(text.left() > graph.right());
    for pressed in [true, false] {
        frame(&ctx, &mut tools, size, pointer(text.center(), pressed));
    }
    let output = settle(&ctx, &mut tools, size);
    let choice = visible(&output, size, "3. Middle source").center();
    assert!(
        tools.model.same_storage(&original),
        "Opening popup changed input"
    );
    assert_eq!(tools.canvas.view.viewport, graph);
    for pressed in [true, false] {
        frame(&ctx, &mut tools, size, pointer(choice, pressed));
    }
    assert_eq!(tools.model.nodes[0].inputs[0], 2);
    assert_eq!(tools.canvas.history.len(), 1);
    tools.canvas.travel(
        &mut tools.model,
        &mut tools.selected,
        false,
        &mut String::new(),
    );
    assert!(tools.model.same_storage(&original));
}
