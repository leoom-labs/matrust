use super::tests::Harness;
use super::*;
use matrust::simulation::Block;

pub(super) fn key(key: egui::Key, modifiers: egui::Modifiers, repeat: bool) -> egui::Event {
    egui::Event::Key {
        key,
        physical_key: None,
        pressed: true,
        repeat,
        modifiers,
    }
}
pub(super) fn press(h: &mut Harness, keycode: egui::Key, modifiers: egui::Modifiers) {
    h.frame(vec![key(keycode, modifiers, false)]);
    let mut release = key(keycode, modifiers, false);
    if let egui::Event::Key { pressed, .. } = &mut release {
        *pressed = false;
    }
    h.frame(vec![release]);
}
pub(super) fn focus(h: &mut Harness) {
    let p = h.port(0, None) - Vec2::new(60., 0.);
    h.click(p);
    assert!(h.ctx.memory(|m| m.has_focus(Canvas::keyboard_id())));
}
pub(super) const CTRL: egui::Modifiers = egui::Modifiers {
    ctrl: true,
    command: true,
    ..egui::Modifiers::NONE
};

#[test]
fn model_keyboard_real_focus_selection_delete_and_history_apply_once_per_frame() {
    for repeat in [false, true] {
        let mut h = Harness::new();
        h.repeat = repeat;
        let original = h.model.clone();
        focus(&mut h);
        press(&mut h, egui::Key::ArrowRight, egui::Modifiers::NONE);
        assert_eq!(h.selected, 1);
        press(&mut h, egui::Key::End, egui::Modifiers::NONE);
        assert_eq!(h.selected, 2);
        press(&mut h, egui::Key::Home, egui::Modifiers::NONE);
        assert_eq!(h.selected, 0);
        h.frame(vec![key(egui::Key::Delete, egui::Modifiers::NONE, false)]);
        assert_eq!(h.model.nodes.len(), 2);
        assert_eq!(h.canvas.cursor, 1);
        h.frame(vec![key(egui::Key::Delete, egui::Modifiers::NONE, true)]);
        assert_eq!(h.model.nodes.len(), 2);
        let mut release = key(egui::Key::Delete, egui::Modifiers::NONE, false);
        if let egui::Event::Key { pressed, .. } = &mut release {
            *pressed = false;
        }
        h.frame(vec![release]);
        press(&mut h, egui::Key::Z, CTRL);
        assert!(h.model.same_storage(&original));
        assert_eq!(h.canvas.cursor, 0);
        press(&mut h, egui::Key::Z, CTRL | egui::Modifiers::SHIFT);
        assert_eq!(h.model.nodes.len(), 2);
        press(&mut h, egui::Key::Z, CTRL);
        press(&mut h, egui::Key::Y, CTRL);
        assert_eq!(h.model.nodes.len(), 2);
    }
}

#[test]
fn model_keyboard_ignores_unfocused_disabled_modal_popup_and_modified_delete() {
    let mut h = Harness::new();
    let original = h.model.clone();
    press(&mut h, egui::Key::Delete, egui::Modifiers::NONE);
    assert!(h.model.same_storage(&original));
    focus(&mut h);
    h.keyboard_allowed = false;
    press(&mut h, egui::Key::Delete, egui::Modifiers::NONE);
    h.keyboard_allowed = true;
    h.enabled = false;
    press(&mut h, egui::Key::Delete, egui::Modifiers::NONE);
    h.enabled = true;
    focus(&mut h);
    egui::Popup::open_id(&h.ctx, egui::Id::new("test menu"));
    press(&mut h, egui::Key::Delete, egui::Modifiers::NONE);
    egui::Popup::close_all(&h.ctx);
    for modifiers in [CTRL, egui::Modifiers::SHIFT, egui::Modifiers::ALT] {
        press(&mut h, egui::Key::Delete, modifiers);
    }
    assert!(h.model.same_storage(&original));
    assert!(h.canvas.history.is_empty());
}

#[test]
fn model_keyboard_escape_cancels_gestures_then_releases_focus_and_bounds_selection() {
    let mut h = Harness::new();
    let original = h.model.clone();
    let output = h.port(0, None);
    h.click(output);
    assert_eq!(h.canvas.source, Some(0));
    press(&mut h, egui::Key::Escape, egui::Modifiers::NONE);
    assert!(h.canvas.source.is_none());
    assert!(h.ctx.memory(|m| m.has_focus(Canvas::keyboard_id())));
    press(&mut h, egui::Key::Escape, egui::Modifiers::NONE);
    assert!(!h.ctx.memory(|m| m.has_focus(Canvas::keyboard_id())));
    press(&mut h, egui::Key::Delete, egui::Modifiers::NONE);
    assert!(h.model.same_storage(&original));
    focus(&mut h);
    for _ in 0..8 {
        press(&mut h, egui::Key::ArrowDown, egui::Modifiers::NONE);
    }
    assert_eq!(h.selected, 2);
    for _ in 0..8 {
        press(&mut h, egui::Key::ArrowUp, egui::Modifiers::NONE);
    }
    assert_eq!(h.selected, 0);
    assert!(h.model.same_storage(&original));
    for _ in 0..3 {
        press(&mut h, egui::Key::Delete, egui::Modifiers::NONE);
    }
    assert!(h.model.nodes.is_empty());
    for keycode in [egui::Key::End, egui::Key::ArrowDown, egui::Key::Delete] {
        press(&mut h, keycode, egui::Modifiers::NONE);
    }
    assert_eq!(h.canvas.cursor, 3);
    for _ in 0..3 {
        press(&mut h, egui::Key::Z, CTRL);
    }
    assert!(h.model.same_storage(&original));
}

#[test]
fn model_keyboard_routes_parameter_coefficient_and_wire_history_to_original_signal() {
    let mut h = Harness::new();
    h.model = Model::default();
    h.model.steps = 16;
    h.canvas.reset();
    h.frame(vec![]);
    let original = h.model.clone();
    let before = parameters::Properties::capture(&h.model.nodes[0]);
    if let Block::Sine { amplitude, .. } = &mut h.model.nodes[0].block {
        *amplitude = 3.;
    }
    h.canvas
        .record_edit(parameters::Edit::node(&h.model, 0, before).unwrap(), None);
    h.canvas
        .replace_coefficients(
            &mut h.model,
            3,
            Block::Filter {
                b: vec![0.5, 0.5],
                a: vec![1.],
            },
        )
        .unwrap();
    assert!(h.canvas.connect(&mut h.model, 4, 0, 0));
    let edited = h.model.clone();
    focus(&mut h);
    for _ in 0..3 {
        press(&mut h, egui::Key::Z, CTRL);
    }
    assert!(h.model.same_storage(&original));
    let cancel = std::sync::atomic::AtomicBool::new(false);
    assert_eq!(
        h.model.run(&cancel).unwrap().traces[0].values.data(),
        original.run(&cancel).unwrap().traces[0].values.data()
    );
    for _ in 0..3 {
        press(&mut h, egui::Key::Y, CTRL);
    }
    assert!(h.model.same_storage(&edited));
}

#[test]
fn model_keyboard_text_edit_keeps_delete_and_undo_out_of_diagram_history() {
    let mut h = Harness::new();
    assert!(h.canvas.connect(&mut h.model, 2, 0, 1));
    let original = h.model.clone();
    let mut text = "editable parameter".to_owned();
    let text_id = egui::Id::new("test parameter field");
    let mut frame = |h: &mut Harness, events: Vec<egui::Event>, request_focus: bool| {
        let _ = crate::headless::run(
            &h.ctx,
            egui::RawInput {
                screen_rect: Some(Rect::from_min_size(Pos2::ZERO, Vec2::new(940., 600.))),
                events,
                ..Default::default()
            },
            |root| {
                egui::CentralPanel::default().show(root, |ui| {
                    let response = ui.add(egui::TextEdit::singleline(&mut text).id(text_id));
                    if request_focus {
                        response.request_focus();
                    }
                    h.canvas.show(
                        ui,
                        &mut h.model,
                        &mut h.selected,
                        &mut String::new(),
                        true,
                        0.,
                    );
                });
            },
        );
        text.is_empty()
    };
    frame(&mut h, vec![], true);
    frame(&mut h, vec![key(egui::Key::A, CTRL, false)], false);
    assert!(frame(
        &mut h,
        vec![key(egui::Key::Delete, egui::Modifiers::NONE, false)],
        false,
    ));
    frame(&mut h, vec![key(egui::Key::Z, CTRL, false)], false);
    assert!(h.model.same_storage(&original));
    assert_eq!(h.canvas.cursor, 1);
    assert!(h.ctx.memory(|m| m.has_focus(text_id)));
    // The text field consumed Delete rather than deleting a model node.
    // Undo may restore its text depending on its own coalescing interval.
}
