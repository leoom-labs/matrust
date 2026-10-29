use super::keyboard_tests::{CTRL, focus, key, press};
use super::tests::Harness;
use super::*;
use matrust::simulation::{Block, MAX_LAYOUT_COORDINATE, Node};
use std::sync::atomic::AtomicBool;

fn release(h: &mut Harness, code: egui::Key) {
    let mut event = key(code, CTRL, false);
    if let egui::Event::Key { pressed, .. } = &mut event {
        *pressed = false;
    }
    h.frame(vec![event]);
}

#[test]
fn model_keyboard_edit_nudge_groups_holds_and_roundtrips_saved_layout_and_signal() {
    for repeat in [false, true] {
        let mut h = Harness::new();
        h.model = Model::default();
        h.model.steps = 16;
        h.repeat = repeat;
        h.frame(vec![]);
        let original = h.model.clone();
        let coefficient_pointer = match &h.model.nodes[3].block {
            Block::Filter { b, .. } => b.as_ptr(),
            _ => unreachable!(),
        };
        focus(&mut h);
        for held in [false, true, true, true] {
            h.frame(vec![key(egui::Key::ArrowRight, CTRL, held)]);
        }
        release(&mut h, egui::Key::ArrowRight);
        assert_eq!(h.model.positions[0], [56., 24.]);
        assert_eq!(h.canvas.cursor, 1);
        assert!(h.canvas.history_bytes <= HISTORY_BYTES);
        assert!(
            matches!(&h.model.nodes[3].block, Block::Filter { b, .. } if b.as_ptr() == coefficient_pointer)
        );
        let moved = h.model.clone();
        let cancel = AtomicBool::new(false);
        assert_eq!(
            h.model.run(&cancel).unwrap().traces[0].values.data(),
            original.run(&cancel).unwrap().traces[0].values.data()
        );
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("keyboard.mrsim");
        h.model.save(&path).unwrap();
        assert!(Model::load(&path).unwrap().same_storage(&moved));
        press(&mut h, egui::Key::Z, CTRL);
        assert!(h.model.same_storage(&original));
        assert_eq!(h.model.version, 1);
        press(&mut h, egui::Key::Y, CTRL);
        assert!(h.model.same_storage(&moved));
        press(&mut h, egui::Key::ArrowDown, CTRL | egui::Modifiers::SHIFT);
        assert_eq!(h.model.positions[0], [56., 25.]);
        assert_eq!(h.canvas.cursor, 2);
        press(&mut h, egui::Key::Z, CTRL);
        assert!(h.model.same_storage(&moved));
    }
}

#[test]
fn model_keyboard_edit_nudge_bounds_stale_layout_and_distinct_interactions() {
    let mut h = Harness::new();
    h.model.set_layout(vec![[0., 0.]; 3]).unwrap();
    h.frame(vec![]);
    // Overlapping custom blocks: explicitly focus the canvas and select node zero.
    h.canvas.focus(&h.ctx);
    h.selected = 0;
    h.frame(vec![]);
    press(&mut h, egui::Key::ArrowLeft, CTRL);
    assert!(h.canvas.history.is_empty());
    press(&mut h, egui::Key::ArrowRight, CTRL);
    press(&mut h, egui::Key::ArrowRight, CTRL);
    assert_eq!(h.canvas.cursor, 2);
    h.frame(vec![key(egui::Key::ArrowDown, CTRL, false)]);
    let mut external = h.model.positions.clone();
    external[1] = [500., 400.];
    h.model.set_layout(external).unwrap();
    let before_repeat = h.model.clone();
    h.frame(vec![key(egui::Key::ArrowDown, CTRL, true)]);
    release(&mut h, egui::Key::ArrowDown);
    assert_eq!(h.canvas.cursor, 4);
    press(&mut h, egui::Key::Z, CTRL);
    assert!(h.model.same_storage(&before_repeat));
    press(&mut h, egui::Key::Z, CTRL);
    assert!(h.model.same_storage(&before_repeat));
    assert!(h.canvas.history.is_empty());
    let mut edge = h.model.positions.clone();
    edge[0] = [MAX_LAYOUT_COORDINATE - 1., MAX_LAYOUT_COORDINATE];
    h.model.set_layout(edge).unwrap();
    h.canvas.focus(&h.ctx);
    press(&mut h, egui::Key::ArrowRight, CTRL);
    assert_eq!(
        h.model.positions[0],
        [MAX_LAYOUT_COORDINATE, MAX_LAYOUT_COORDINATE]
    );
    press(&mut h, egui::Key::ArrowRight, CTRL);
    press(&mut h, egui::Key::ArrowDown, CTRL);
    assert_eq!(h.canvas.cursor, 1);
}

#[test]
fn model_keyboard_edit_connections_disconnect_and_undo_restore_actual_signal() {
    for repeat in [false, true] {
        let mut h = Harness::new();
        h.repeat = repeat;
        let original = h.model.clone();
        focus(&mut h);
        press(&mut h, egui::Key::ArrowDown, egui::Modifiers::NONE);
        press(&mut h, egui::Key::C, egui::Modifiers::NONE);
        assert!(h.canvas.keyboard_wiring);
        press(&mut h, egui::Key::Enter, egui::Modifiers::NONE);
        assert!(h.canvas.keyboard_wiring); // Source has no input; keep choosing a target.
        press(&mut h, egui::Key::End, egui::Modifiers::NONE);
        assert_eq!(h.canvas.source, Some(1));
        press(&mut h, egui::Key::Enter, egui::Modifiers::NONE);
        assert!(!h.canvas.keyboard_wiring);
        assert_eq!(h.model.nodes[2].inputs, [1]);
        assert_eq!(
            h.model.run(&AtomicBool::new(false)).unwrap().traces[0]
                .values
                .data(),
            &[7.; 4]
        );
        press(&mut h, egui::Key::Backspace, egui::Modifiers::NONE);
        assert_eq!(h.model.nodes[2].inputs, [usize::MAX]);
        assert!(h.model.validate().is_err());
        assert_eq!(h.canvas.cursor, 2);
        press(&mut h, egui::Key::Backspace, egui::Modifiers::NONE);
        assert_eq!(h.canvas.cursor, 2);
        for _ in 0..2 {
            press(&mut h, egui::Key::Z, CTRL);
        }
        assert!(h.model.same_storage(&original));
        press(&mut h, egui::Key::C, egui::Modifiers::NONE);
        press(&mut h, egui::Key::Escape, egui::Modifiers::NONE);
        assert!(h.canvas.source.is_none() && !h.canvas.keyboard_wiring);
    }
}

#[test]
fn model_keyboard_edit_all_32_inputs_can_be_revealed_and_connected() {
    let mut h = Harness::new();
    h.model.nodes[2] = Node {
        name: "Mix".into(),
        block: Block::Sum,
        inputs: vec![0; 32],
    };
    h.model.nodes.push(Node {
        name: "Scope".into(),
        block: Block::Scope,
        inputs: vec![2],
    });
    h.frame(vec![]);
    focus(&mut h);
    press(&mut h, egui::Key::ArrowDown, egui::Modifiers::NONE);
    press(&mut h, egui::Key::C, egui::Modifiers::NONE);
    press(&mut h, egui::Key::ArrowDown, egui::Modifiers::NONE);
    for _ in 0..40 {
        press(&mut h, egui::Key::PageDown, egui::Modifiers::NONE);
    }
    assert_eq!(h.canvas.keyboard_port, 31);
    h.port(2, Some(31)); // Target is actually in the rendered viewport.
    press(&mut h, egui::Key::Enter, egui::Modifiers::NONE);
    assert_eq!(h.model.nodes[2].inputs[31], 1);
    assert!(
        h.model.nodes[2].inputs[..31]
            .iter()
            .all(|source| *source == 0)
    );
    assert_eq!(
        h.model.run(&AtomicBool::new(false)).unwrap().traces[0]
            .values
            .data(),
        &[69.; 4]
    );
    for _ in 0..40 {
        press(&mut h, egui::Key::PageUp, egui::Modifiers::NONE);
    }
    assert_eq!(h.canvas.keyboard_port, 0);
    h.port(2, Some(0));
}

#[test]
fn model_keyboard_edit_dialog_guard_ends_nudge_groups_and_disabled_wiring() {
    let mut h = Harness::new();
    focus(&mut h);
    h.frame(vec![key(egui::Key::ArrowRight, CTRL, false)]);
    let before = h.model.clone();
    h.keyboard_allowed = false;
    h.frame(vec![key(egui::Key::ArrowRight, CTRL, true)]);
    press(&mut h, egui::Key::C, egui::Modifiers::NONE);
    assert!(h.model.same_storage(&before));
    assert!(h.canvas.source.is_none());
    h.keyboard_allowed = true;
    h.frame(vec![key(egui::Key::ArrowRight, CTRL, true)]);
    release(&mut h, egui::Key::ArrowRight);
    assert_eq!(h.canvas.cursor, 2);
    press(&mut h, egui::Key::C, egui::Modifiers::NONE);
    assert!(h.canvas.keyboard_wiring);
    h.enabled = false;
    h.frame(vec![]);
    assert!(h.canvas.source.is_none() && !h.canvas.keyboard_wiring);
}
