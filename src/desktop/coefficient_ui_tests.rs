use super::super::model_canvas::Canvas;
use super::*;
use egui::{Event, Pos2, Rect, Vec2};

struct Harness {
    ctx: egui::Context,
    model: Model,
    canvas: Canvas,
    editor: Editor,
    closed: bool,
}
impl Harness {
    fn new(sos: bool) -> Self {
        let mut model = Model::default();
        model.nodes[3].block = if sos {
            Block::SosFilter {
                sections: vec![[1., 0., 0., 1., -0.5, 0.]; 64],
            }
        } else {
            Block::Filter {
                b: vec![0.000244140625; 4097],
                a: vec![1.],
            }
        };
        let editor = Editor::new(3, &model.nodes[3].block).unwrap();
        Self {
            ctx: Default::default(),
            model,
            canvas: Default::default(),
            editor,
            closed: false,
        }
    }
    fn frame(&mut self, size: Vec2, events: Vec<Event>) -> egui::FullOutput {
        crate::headless::run(
            &self.ctx,
            egui::RawInput {
                screen_rect: Some(Rect::from_min_size(Pos2::ZERO, size)),
                events,
                ..Default::default()
            },
            |root| {
                let ctx = &root.ctx().clone();
                self.closed |= self.editor.ui(ctx, &mut self.model, &mut self.canvas);
            },
        )
    }
    fn click(&mut self, size: Vec2, pos: Pos2) {
        for pressed in [true, false] {
            self.frame(
                size,
                vec![
                    Event::PointerMoved(pos),
                    Event::PointerButton {
                        pos,
                        button: egui::PointerButton::Primary,
                        pressed,
                        modifiers: egui::Modifiers::NONE,
                    },
                ],
            );
        }
    }
}
fn find(shape: &egui::Shape, label: &str) -> Option<Rect> {
    match shape {
        egui::Shape::Text(t) if t.galley.text() == label => {
            Some(Rect::from_min_size(t.pos, t.galley.size()))
        }
        egui::Shape::Vec(shapes) => shapes.iter().find_map(|s| find(s, label)),
        _ => None,
    }
}
fn point(output: &egui::FullOutput, label: &str, size: Vec2) -> Pos2 {
    let (rect, clip) = output
        .shapes
        .iter()
        .find_map(|s| find(&s.shape, label).map(|r| (r, s.clip_rect)))
        .unwrap_or_else(|| panic!("Missing {label}"));
    assert!(
        clip.expand(0.1).contains_rect(rect),
        "Clipped {label}: {rect:?} in {clip:?}"
    );
    assert!(
        Rect::from_min_size(Pos2::ZERO, size).contains_rect(rect),
        "Outside {label}"
    );
    rect.center()
}

#[test]
fn coefficient_ui_large_fir_and_sos_keep_actions_visible_after_resize_and_cancel() {
    for sos in [false, true] {
        let mut h = Harness::new(sos);
        let original = h.model.clone();
        for size in [
            Vec2::new(1440., 900.),
            Vec2::new(960., 650.),
            Vec2::new(640., 480.),
            Vec2::new(480., 320.),
        ] {
            for _ in 0..4 {
                h.frame(size, vec![]);
            }
            let output = h.frame(size, vec![]);
            for label in ["Filter coefficients", "Apply coefficients", "Cancel"] {
                point(&output, label, size);
            }
            let rect = h
                .ctx
                .memory(|m| m.area_rect(egui::Id::new("filter_coefficients")))
                .unwrap();
            assert!(
                Rect::from_min_size(Pos2::ZERO, size).contains_rect(rect),
                "{rect:?} in {size:?}"
            );
        }
        let size = Vec2::new(480., 320.);
        let output = h.frame(size, vec![]);
        h.click(size, point(&output, "Cancel", size));
        assert!(h.closed);
        assert!(h.model.same_storage(&original));
        assert!(!h.canvas.take_changed());
    }
}

#[test]
fn coefficient_ui_invalid_apply_preserves_model_and_successful_correction_closes() {
    let mut h = Harness::new(false);
    let original = h.model.clone();
    h.editor.numerator = "0.25 0.5 0.25".into();
    h.editor.denominator = "0".into();
    let size = Vec2::new(480., 320.);
    for _ in 0..4 {
        h.frame(size, vec![]);
    }
    let output = h.frame(size, vec![]);
    h.click(size, point(&output, "Apply coefficients", size));
    assert!(!h.closed && !h.editor.message.is_empty());
    assert!(h.model.same_storage(&original));
    assert!(!h.canvas.take_changed());
    for _ in 0..4 {
        h.frame(size, vec![]);
    }
    let output = h.frame(size, vec![]);
    point(&output, &h.editor.message, size);
    point(&output, "Cancel", size);
    h.editor.denominator = "1".into();
    h.click(size, point(&output, "Apply coefficients", size));
    assert!(h.closed);
    assert!(
        matches!(&h.model.nodes[3].block,Block::Filter{b,a} if b==&[0.25,0.5,0.25] && a==&[1.])
    );
    assert!(h.canvas.take_changed());
}

#[test]
fn coefficient_ui_escape_cancels_focused_text_and_backdrop_preserves_draft() {
    let mut h = Harness::new(false);
    let original = h.model.clone();
    h.editor.numerator = "1 2 3".into();
    let size = Vec2::new(640., 480.);
    for _ in 0..4 {
        h.frame(size, vec![]);
    }
    h.click(size, Pos2::new(2., 2.));
    assert!(!h.closed);
    let output = h.frame(size, vec![]);
    h.click(size, point(&output, "1 2 3", size));
    h.frame(size, vec![Event::Text(" 4".into())]);
    assert_ne!(h.editor.numerator, "1 2 3");
    h.frame(
        size,
        vec![Event::Key {
            key: egui::Key::Escape,
            physical_key: None,
            pressed: true,
            repeat: false,
            modifiers: egui::Modifiers::NONE,
        }],
    );
    assert!(h.closed);
    assert!(h.model.same_storage(&original));
    assert!(!h.canvas.take_changed());
}

#[test]
fn coefficient_ui_full_toolkit_modal_blocks_underlying_delete_and_cancels_cleanly() {
    let ctx = egui::Context::default();
    let size = Vec2::new(960., 650.);
    let mut tools = super::super::Toolkits {
        model_open: true,
        ..Default::default()
    };
    let original = tools.model.clone();
    tools.open_coefficients(usize::MAX);
    assert!(tools.coefficient_editor.is_none());
    tools.open_coefficients(0); // A sine block has no coefficient editor.
    assert!(tools.coefficient_editor.is_none());
    let frame = |tools: &mut super::super::Toolkits, events| {
        crate::headless::run(
            &ctx,
            egui::RawInput {
                screen_rect: Some(Rect::from_min_size(Pos2::ZERO, size)),
                events,
                ..Default::default()
            },
            |root| {
                let ctx = &root.ctx().clone();
                tools.ui(ctx, false);
                if ctx.current_pass_index() == 0 {
                    ctx.request_discard("modal actions once per frame");
                }
            },
        )
    };
    for _ in 0..4 {
        frame(&mut tools, vec![]);
    }
    let output = frame(&mut tools, vec![]);
    let delete = point(&output, "Delete selected block", size);
    tools.open_coefficients(3);
    assert!(tools.modal_open());
    for _ in 0..4 {
        frame(&mut tools, vec![]);
    }
    for pressed in [true, false] {
        frame(
            &mut tools,
            vec![
                Event::PointerMoved(delete),
                Event::PointerButton {
                    pos: delete,
                    button: egui::PointerButton::Primary,
                    pressed,
                    modifiers: egui::Modifiers::NONE,
                },
            ],
        );
    }
    assert!(tools.model.same_storage(&original));
    assert!(tools.coefficient_editor.is_some());
    let output = frame(&mut tools, vec![]);
    let cancel = point(&output, "Cancel", size);
    for pressed in [true, false] {
        frame(
            &mut tools,
            vec![
                Event::PointerMoved(cancel),
                Event::PointerButton {
                    pos: cancel,
                    button: egui::PointerButton::Primary,
                    pressed,
                    modifiers: egui::Modifiers::NONE,
                },
            ],
        );
    }
    assert!(tools.coefficient_editor.is_none());
    assert!(!tools.modal_open() && tools.model_open);
    assert!(tools.model.same_storage(&original));
    assert!(!tools.take_diagram_change());
}
