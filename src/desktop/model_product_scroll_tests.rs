//! Actual wheel/port interaction; no capture-only scrolling or hidden UI writes.
use super::*;
use matrust::simulation::{Block, Node, ProductInput as P};

fn text_position(shape: &egui::Shape, clip: Rect, label: &str, prefix: bool) -> Option<Pos2> {
    match shape {
        egui::Shape::Text(t)
            if if prefix {
                t.galley.text().starts_with(label)
            } else {
                t.galley.text() == label
            } =>
        {
            let point = t.pos + t.galley.size() * 0.5;
            clip.contains(point).then_some(point)
        }
        egui::Shape::Vec(shapes) => shapes
            .iter()
            .find_map(|s| text_position(s, clip, label, prefix)),
        _ => None,
    }
}
fn visible(output: &egui::FullOutput, label: &str, prefix: bool) -> Option<(Pos2, Rect)> {
    let viewport = Rect::from_min_size(Pos2::ZERO, Vec2::new(960., 650.));
    output.shapes.iter().find_map(|shape| {
        let clip = shape.clip_rect.intersect(viewport);
        text_position(&shape.shape, clip, label, prefix).map(|point| (point, clip))
    })
}
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
            let action = tools.ui(root.ctx(), false);
            assert!(
                action.is_none(),
                "Scrolling and port edits must not submit model execution or document actions"
            );
            if root.ctx().current_pass_index() == 0 {
                root.ctx()
                    .request_discard("Product port scroll edits apply once");
            }
        },
    )
}
fn wheel(
    ctx: &egui::Context,
    tools: &mut super::super::Toolkits,
    point: Pos2,
    amount: f32,
) -> egui::FullOutput {
    frame(
        ctx,
        tools,
        vec![
            egui::Event::PointerMoved(point),
            egui::Event::MouseWheel {
                phase: egui::TouchPhase::Move,
                unit: egui::MouseWheelUnit::Point,
                delta: Vec2::new(0., amount),
                modifiers: egui::Modifiers::NONE,
            },
        ],
    );
    for _ in 0..2 {
        frame(ctx, tools, vec![]);
    }
    frame(ctx, tools, vec![])
}
fn click(ctx: &egui::Context, tools: &mut super::super::Toolkits, point: Pos2) {
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
fn seek(
    ctx: &egui::Context,
    tools: &mut super::super::Toolkits,
    point: Pos2,
    amount: f32,
    label: &str,
    prefix: bool,
) -> egui::FullOutput {
    let mut output = frame(ctx, tools, vec![]);
    for _ in 0..40 {
        if visible(&output, label, prefix).is_some() {
            return output;
        }
        output = wheel(ctx, tools, point, amount);
    }
    let visible_controls: Vec<_> = [
        "Input 1:",
        "Input 32:",
        "Operation 1",
        "Operation 32",
        "Add input",
        "Remove last input",
    ]
    .into_iter()
    .filter_map(|name| visible(&output, name, name.starts_with("Input ")).map(|hit| (name, hit)))
    .collect();
    panic!(
        "Wheel scrolling did not expose {label}; pointer={point:?}, visible_controls={visible_controls:?}"
    );
}
#[test]
fn model_product_32_ports_scroll_compact_disconnect_remove_and_undo() {
    let ctx = egui::Context::default();
    let mut tools = super::super::Toolkits {
        model_open: true,
        ..Default::default()
    };
    let mut operations = vec![P::Multiply; 32];
    operations[31] = P::Divide;
    tools.model = Model {
        steps: 4,
        nodes: vec![
            Node {
                name: "Product 32 ports".into(),
                block: Block::Product { operations },
                inputs: vec![1; 32],
            },
            Node {
                name: "Source".into(),
                block: Block::Constant { value: 2. },
                inputs: vec![],
            },
            Node {
                name: "Output".into(),
                block: Block::Scope,
                inputs: vec![0],
            },
        ],
        ..Model::default()
    };
    tools.selected = 0;
    let original = tools.model.clone();
    assert!(original.validate().is_ok());
    for _ in 0..3 {
        frame(&ctx, &mut tools, vec![]);
    }
    let mut output = frame(&ctx, &mut tools, vec![]);
    // Locate an actually painted clipped input label rather than guessing the
    // nested parameter pane's layout coordinates. If the outer window scrolls,
    // first reveal that label using ordinary wheel input inside the window.
    if visible(&output, "Input 1:", true).is_none() {
        let window = tools.model_document_region().expect("Open model tab").0;
        output = seek(
            &ctx,
            &mut tools,
            Pos2::new(window.right() - 30., window.bottom() - 40.),
            -100.,
            "Input 1:",
            true,
        );
    }
    let (input_point, pane_clip) = visible(&output, "Input 1:", true).expect("First port visible");
    assert!(pane_clip.width() > 10. && pane_clip.height() > 10.);
    // A vertical ScrollArea narrows its paint clip only on Y. Its clip can
    // span BOTH columns, so the clip center can hit the left block list.
    // Use the visible parameter control's X to send real wheel events to
    // the right column, and the clipped Y center to stay clear of its edges.
    let point = Pos2::new(input_point.x, pane_clip.center().y);
    assert!(pane_clip.contains(point));
    output = seek(&ctx, &mut tools, point, -120., "Remove last input", false);
    assert!(visible(&output, "Operation 32", false).is_some());
    assert!(visible(&output, "Divide (/)", false).is_some());
    let add = visible(&output, "Add input", false)
        .expect("Add input visible at end")
        .0;
    let remove = visible(&output, "Remove last input", false)
        .expect("Remove input visible at end")
        .0;
    assert!(tools.model.same_storage(&original));
    assert!(tools.canvas.history.is_empty());
    // Both are disabled: at the maximum port count and with the last port wired.
    click(&ctx, &mut tools, add);
    click(&ctx, &mut tools, remove);
    assert!(tools.model.same_storage(&original));
    assert!(tools.canvas.history.is_empty());
    output = seek(&ctx, &mut tools, point, 100., "Input 32:", true);
    let input = visible(&output, "Input 32:", true).unwrap().0;
    assert!(tools.model.same_storage(&original));
    assert!(tools.canvas.history.is_empty());
    click(&ctx, &mut tools, input);
    output = frame(&ctx, &mut tools, vec![]);
    let disconnected = visible(&output, "Not connected", false)
        .expect("Actual input popup choice")
        .0;
    click(&ctx, &mut tools, disconnected);
    assert_eq!(tools.model.nodes[0].inputs.len(), 32);
    assert_eq!(tools.model.nodes[0].inputs[31], usize::MAX);
    assert_eq!(tools.canvas.history.len(), 1);
    let unwired = tools.model.clone();
    output = seek(&ctx, &mut tools, point, -120., "Remove last input", false);
    assert!(tools.model.same_storage(&unwired));
    assert_eq!(tools.canvas.history.len(), 1);
    click(
        &ctx,
        &mut tools,
        visible(&output, "Remove last input", false).unwrap().0,
    );
    assert_eq!(tools.model.nodes[0].inputs, vec![1; 31]);
    assert!(
        matches!(&tools.model.nodes[0].block,Block::Product{operations} if operations==&vec![P::Multiply;31])
    );
    assert_eq!(tools.canvas.history.len(), 2);
    tools.canvas.travel(
        &mut tools.model,
        &mut tools.selected,
        false,
        &mut String::new(),
    );
    assert!(tools.model.same_storage(&unwired));
    tools.canvas.travel(
        &mut tools.model,
        &mut tools.selected,
        false,
        &mut String::new(),
    );
    assert!(tools.model.same_storage(&original));
    let history = tools.canvas.history.len();
    let cursor = tools.canvas.cursor;
    let output = seek(&ctx, &mut tools, point, 100., "Operation 1", false);
    assert!(visible(&output, "Operation 1", false).is_some());
    assert!(tools.model.same_storage(&original));
    assert_eq!(
        (tools.canvas.history.len(), tools.canvas.cursor),
        (history, cursor)
    );
}
