use super::keyboard_tests::{CTRL, focus, press};
use super::tests::Harness;
use super::*;
use matrust::simulation::{Block, Node};

fn copied(output: egui::FullOutput) -> Option<String> {
    output
        .platform_output
        .commands
        .into_iter()
        .find_map(|command| match command {
            egui::OutputCommand::CopyText(text) => Some(text),
            _ => None,
        })
}
fn context(h: &mut Harness, index: usize) {
    let point = h.port(index, None) - Vec2::new(60., 0.);
    for pressed in [true, false] {
        h.frame(vec![
            egui::Event::PointerMoved(point),
            egui::Event::PointerButton {
                pos: point,
                button: egui::PointerButton::Secondary,
                pressed,
                modifiers: egui::Modifiers::NONE,
            },
        ]);
    }
}

#[test]
fn model_cut_native_event_is_one_undoable_removal_and_repeat_cannot_cut_neighbors() {
    for repeat in [false, true] {
        let mut h = Harness::new();
        h.repeat = repeat;
        h.model.nodes[1].block = Block::Filter {
            b: vec![1., 2., 3.],
            a: vec![1.],
        };
        h.model.nodes[1].inputs = vec![0];
        h.model.nodes[2].inputs = vec![1];
        focus(&mut h);
        h.canvas.selection.replace(3, vec![1, 2], &mut h.selected);
        let original = h.model.clone();
        let pointer = match &h.model.nodes[1].block {
            Block::Filter { b, .. } => b.as_ptr(),
            _ => unreachable!(),
        };
        let text =
            copied(h.frame(vec![egui::Event::Cut])).expect("cut publishes clipboard payload");
        assert_eq!(h.model.nodes.len(), 1);
        assert_eq!(h.canvas.cursor, 1);
        assert_eq!(h.selected, usize::MAX);
        assert!(h.canvas.selection.indices().is_empty());
        assert!(h.canvas.take_changed());
        assert!(copied(h.frame(vec![egui::Event::Cut])).is_none());
        assert_eq!(h.model.nodes.len(), 1);
        assert_eq!(h.canvas.cursor, 1);
        press(&mut h, egui::Key::Z, CTRL);
        assert!(h.model.same_storage(&original));
        assert_eq!(h.canvas.selection.indices(), &[1, 2]);
        if let Block::Filter { b, .. } = &h.model.nodes[1].block {
            assert_eq!(b.as_ptr(), pointer);
        }
        let mut target = Model::default();
        target.nodes.clear();
        clipboard::Change::insert(
            &mut target,
            clipboard::Fragment::decode(&text).unwrap(),
            900.,
            32.,
            vec![],
            HISTORY_BYTES,
        )
        .unwrap();
        assert_eq!(target.nodes[0].inputs, [usize::MAX]);
        assert_eq!(target.nodes[1].inputs, [0]);
    }
}

#[test]
fn model_cut_context_menu_preserves_group_and_targets_unselected_block() {
    for repeat in [false, true] {
        let mut h = Harness::new();
        h.repeat = repeat;
        h.canvas.selection.replace(3, vec![0, 1], &mut h.selected);
        context(&mut h, 1);
        assert_eq!(h.canvas.selection.indices(), &[0, 1]);
        assert_eq!(h.selected, 1);
        h.button("Duplicate blocks (Ctrl+D)");
        assert_eq!(h.model.nodes.len(), 5);
        assert_eq!(h.canvas.selection.indices(), &[3, 4]);
        assert_eq!(h.canvas.cursor, 1);
        assert_eq!(&h.model.positions[3..], &[[56., 56.], [226., 56.]]);
        press(&mut h, egui::Key::Z, CTRL);
        assert_eq!(h.model.nodes.len(), 3);
        context(&mut h, 2);
        assert_eq!(h.canvas.selection.indices(), &[2]);
        h.button("Cut blocks (Ctrl+X)");
        assert_eq!(h.model.nodes.len(), 2);
        assert_eq!(h.canvas.cursor, 1);
        assert!(
            h.ctx
                .memory(|memory| memory.has_focus(Canvas::keyboard_id()))
        );
        press(&mut h, egui::Key::Z, CTRL);
        assert_eq!(h.model.nodes.len(), 3);
        assert_eq!(h.canvas.selection.indices(), &[2]);
        context(&mut h, 1);
        h.button("Delete blocks (Delete)");
        assert_eq!(h.model.nodes.len(), 2);
        press(&mut h, egui::Key::Z, CTRL);
        assert_eq!(h.model.nodes.len(), 3);
    }
}

#[test]
fn model_cut_encoding_and_memory_failure_publish_no_clipboard_and_preserve_model() {
    let mut h = Harness::new();
    focus(&mut h);
    h.model.nodes[0].block = Block::Constant { value: f64::NAN };
    let original = h.model.clone();
    assert!(copied(h.frame(vec![egui::Event::Cut])).is_none());
    assert!(h.model.same_storage(&original));
    assert!(h.canvas.history.is_empty());
    h.model.nodes = (0..70)
        .map(|index| Node {
            name: index.to_string(),
            inputs: vec![0],
            block: Block::Filter {
                b: vec![1.; 4097],
                a: vec![1.; 4097],
            },
        })
        .collect();
    h.canvas.selection.all(70, &mut h.selected);
    let original = h.model.clone();
    assert!(copied(h.frame(vec![egui::Event::Cut])).is_none());
    assert!(h.model.same_storage(&original));
    assert!(h.canvas.history.is_empty());
    // The clipboard fits, but survivor-wire snapshots push deletion history
    // over its limit. The prepared payload must not replace the clipboard.
    h.model.nodes = (0..512)
        .map(|index| Node {
            name: index.to_string(),
            inputs: vec![0; 32],
            block: if index < 62 {
                Block::Filter {
                    b: vec![1.; 4097],
                    a: vec![1.; 4097],
                }
            } else {
                Block::Sum
            },
        })
        .collect();
    h.canvas
        .selection
        .replace(512, (0..62).collect(), &mut h.selected);
    assert!(
        clipboard::Fragment::capture(&h.model, h.canvas.selection.indices(), 900.)
            .unwrap()
            .encode()
            .is_ok()
    );
    let original = h.model.clone();
    assert!(copied(h.frame(vec![egui::Event::Cut])).is_none());
    assert!(h.model.same_storage(&original));
    assert!(h.canvas.history.is_empty());
}

#[test]
fn model_cut_shortcut_menu_and_focus_guards_preserve_other_editors() {
    let mut h = Harness::new();
    let original = h.model.clone();
    assert!(copied(h.frame(vec![egui::Event::Cut])).is_none());
    assert!(h.model.same_storage(&original));
    focus(&mut h);
    h.keyboard_allowed = false;
    assert!(copied(h.frame(vec![egui::Event::Cut])).is_none());
    assert!(h.model.same_storage(&original));
    h.keyboard_allowed = true;
    h.enabled = false;
    assert!(copied(h.frame(vec![egui::Event::Cut])).is_none());
    assert!(h.model.same_storage(&original));
    h.enabled = true;
    h.frame(vec![]);
    focus(&mut h);
    press(&mut h, egui::Key::X, CTRL);
    assert_eq!(h.model.nodes.len(), 2);
    press(&mut h, egui::Key::Z, CTRL);
    assert!(h.model.same_storage(&original));
    h.button("Edit blocks");
    h.button("Cut blocks (Ctrl+X)");
    assert_eq!(h.model.nodes.len(), 2);
    press(&mut h, egui::Key::Z, CTRL);
    assert!(h.model.same_storage(&original));
}
