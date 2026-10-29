use super::clipboard::{Change as Paste, Fragment};
use super::keyboard_tests::{CTRL, focus, key, press};
use super::tests::Harness;
use super::*;
use matrust::simulation::{Block, MAX_LAYOUT_COORDINATE, Node};
use std::sync::atomic::AtomicBool;

fn copied(output: egui::FullOutput) -> String {
    output
        .platform_output
        .commands
        .into_iter()
        .find_map(|command| match command {
            egui::OutputCommand::CopyText(text) => Some(text),
            _ => None,
        })
        .expect("native clipboard output")
}
fn insert(model: &mut Model, indices: &[usize]) -> Paste {
    let fragment = Fragment::capture(model, indices, 900.).unwrap();
    Paste::insert(
        model,
        fragment,
        900.,
        32.,
        indices.to_vec(),
        HISTORY_BYTES - size_of::<Change>(),
    )
    .unwrap()
}

#[test]
fn model_clipboard_roundtrip_internal_fanout_feedback_external_inputs_and_exact_parameters() {
    let mut h = Harness::new();
    h.model.nodes[0].name = "Signal Unicode \u{03b1}".into();
    h.model.nodes[0].block = Block::Filter {
        b: vec![-0., f64::MIN_POSITIVE, 1.2345678901234567],
        a: vec![1.],
    };
    h.model.nodes[0].inputs = vec![0];
    h.model.nodes[2].inputs = vec![0, 1, 2, 999, usize::MAX];
    let text = Fragment::capture(&h.model, &[0, 2], 900.)
        .unwrap()
        .encode()
        .unwrap();
    let decoded = Fragment::decode(&text).unwrap();
    let mut target = Model::default();
    target.nodes.clear();
    Paste::insert(&mut target, decoded, 900., 32., vec![], HISTORY_BYTES).unwrap();
    assert_eq!(target.nodes[0].inputs, [0]);
    assert_eq!(
        target.nodes[1].inputs,
        [0, usize::MAX, 1, usize::MAX, usize::MAX]
    );
    assert!(target.nodes[0].name.starts_with("Signal Unicode \u{03b1}"));
    if let Block::Filter { b, .. } = &target.nodes[0].block {
        assert_eq!(
            b.iter().map(|v| v.to_bits()).collect::<Vec<_>>(),
            vec![
                (-0_f64).to_bits(),
                f64::MIN_POSITIVE.to_bits(),
                1.2345678901234567_f64.to_bits()
            ]
        );
    } else {
        panic!("filter preserved");
    }
    assert!(Fragment::decode("ordinary clipboard text").is_err());
    assert!(Fragment::decode(&text.replace("MATRUST_BLOCKS_V1", "MATRUST_BLOCKS_V2")).is_err());
    assert!(
        Fragment::decode("MATRUST_BLOCKS_V1\n{\"nodes\":[],\"positions\":[],\"unknown\":1}")
            .is_err()
    );
    assert!(Fragment::decode(&"x".repeat(16 * 1024 * 1024 + 1)).is_err());
    h.model.nodes[0].block = Block::Constant { value: f64::NAN };
    assert!(
        Fragment::capture(&h.model, &[0], 900.)
            .unwrap()
            .encode()
            .is_err()
    );
}

#[test]
fn model_clipboard_paste_undo_redo_moves_owned_buffers_and_preserves_actual_signal() {
    let mut h = Harness::new();
    h.model.nodes[1].block = Block::Filter {
        b: vec![1.; 33],
        a: vec![1.],
    };
    h.model.nodes[1].inputs = vec![0];
    h.model.nodes[2].inputs = vec![1];
    let original = h.model.clone();
    let original_pointer = match &h.model.nodes[1].block {
        Block::Filter { b, .. } => b.as_ptr(),
        _ => unreachable!(),
    };
    let expected = h.model.run(&AtomicBool::new(false)).unwrap();
    let mut change = insert(&mut h.model, &[0, 1, 2]);
    let duplicate_pointer = match &h.model.nodes[4].block {
        Block::Filter { b, .. } => b.as_ptr(),
        _ => unreachable!(),
    };
    assert_ne!(original_pointer, duplicate_pointer);
    assert_eq!(h.model.nodes[4].inputs, [3]);
    assert_eq!(h.model.nodes[5].inputs, [4]);
    let result = h.model.run(&AtomicBool::new(false)).unwrap();
    assert_eq!(result.traces.len(), 2);
    for trace in result.traces {
        assert_eq!(trace.values.data(), expected.traces[0].values.data());
    }
    let pasted = h.model.clone();
    for _ in 0..3 {
        assert!(change.apply(&mut h.model, false));
        assert!(h.model.same_storage(&original));
        assert!(change.apply(&mut h.model, true));
        assert!(h.model.same_storage(&pasted));
        if let Block::Filter { b, .. } = &h.model.nodes[4].block {
            assert_eq!(b.as_ptr(), duplicate_pointer);
        }
        if let Block::Filter { b, .. } = &h.model.nodes[1].block {
            assert_eq!(b.as_ptr(), original_pointer);
        }
    }
    let saved = serde_json::to_string(&h.model).unwrap();
    assert!(
        serde_json::from_str::<Model>(&saved)
            .unwrap()
            .same_storage(&pasted)
    );
}

#[test]
fn model_clipboard_invalid_draft_wires_stay_invalid_and_late_stale_edits_are_atomic() {
    let mut h = Harness::new();
    h.model.nodes[2].inputs = vec![3, 500, usize::MAX];
    let original = h.model.clone();
    let mut change = insert(&mut h.model, &[0, 1]);
    assert_eq!(h.model.nodes[2].inputs, [5, 502, usize::MAX]);
    h.model.nodes[0].block = Block::Constant { value: 99. };
    assert!(change.apply(&mut h.model, false));
    assert_eq!(h.model.nodes[2].inputs, original.nodes[2].inputs);
    assert!(matches!(
        h.model.nodes[0].block,
        Block::Constant { value: 99. }
    ));
    assert!(change.apply(&mut h.model, true));
    h.model.nodes[4].block = Block::Constant { value: 88. };
    let newer = h.model.clone();
    assert!(!change.apply(&mut h.model, false));
    assert!(h.model.same_storage(&newer));
    h.model.nodes[2].inputs = vec![usize::MAX - 1];
    let newer = h.model.clone();
    let fragment = Fragment::capture(&h.model, &[0, 1], 900.).unwrap();
    assert!(Paste::insert(&mut h.model, fragment, 900., 32., vec![], HISTORY_BYTES).is_err());
    assert!(h.model.same_storage(&newer));
}

#[test]
fn model_clipboard_native_events_duplicate_keys_focus_and_repeated_passes() {
    for repeat in [false, true] {
        let mut h = Harness::new();
        h.repeat = repeat;
        focus(&mut h);
        press(&mut h, egui::Key::A, CTRL);
        let original = h.model.clone();
        let text = copied(h.frame(vec![egui::Event::Copy]));
        assert!(h.model.same_storage(&original));
        h.frame(vec![egui::Event::Paste(text.clone())]);
        assert_eq!(h.model.nodes.len(), 6);
        assert_eq!(h.canvas.selection.indices(), &[3, 4, 5]);
        assert_eq!(h.canvas.cursor, 1);
        press(&mut h, egui::Key::Z, CTRL);
        assert!(h.model.same_storage(&original));
        assert_eq!(h.canvas.selection.indices(), &[0, 1, 2]);
        press(&mut h, egui::Key::Y, CTRL);
        assert_eq!(h.canvas.selection.indices(), &[3, 4, 5]);
        h.frame(vec![egui::Event::Paste(text)]);
        assert_eq!(h.model.positions[6], [88., 88.]);
        h.frame(vec![key(egui::Key::D, CTRL, false)]);
        h.frame(vec![key(egui::Key::D, CTRL, true)]);
        assert_eq!(h.model.nodes.len(), 12);
        assert_eq!(h.canvas.cursor, 3);
        let unchanged = h.model.clone();
        h.frame(vec![egui::Event::Paste("foreign text".into())]);
        assert!(h.model.same_storage(&unchanged));
        h.enabled = false;
        h.frame(vec![egui::Event::Paste(copied_for(&original))]);
        assert!(h.model.same_storage(&unchanged));
        h.enabled = true;
        h.keyboard_allowed = false;
        h.frame(vec![egui::Event::Paste(copied_for(&original))]);
        assert!(h.model.same_storage(&unchanged));
    }
}
fn copied_for(model: &Model) -> String {
    Fragment::capture(model, &[0], 900.)
        .unwrap()
        .encode()
        .unwrap()
}

#[test]
fn model_clipboard_menu_width_focus_and_cross_model_paste() {
    for repeat in [false, true] {
        let mut h = Harness::new();
        h.repeat = repeat;
        h.canvas.selection.all(3, &mut h.selected);
        h.button("Edit blocks");
        h.button("Duplicate blocks (Ctrl+D)");
        assert_eq!(
            h.model.positions,
            vec![
                [24., 24.],
                [194., 24.],
                [364., 24.],
                [56., 56.],
                [226., 56.],
                [396., 56.]
            ]
        );
        assert!(h.ctx.memory(|m| m.has_focus(Canvas::keyboard_id())));
        press(&mut h, egui::Key::Z, CTRL);
        assert_eq!(h.model.nodes.len(), 3);
        let text = copied(h.frame(vec![egui::Event::Copy]));
        h.canvas.reset();
        h.model = Model::default();
        h.model.nodes.clear();
        h.selected = usize::MAX;
        h.frame(vec![]);
        h.button("Edit blocks");
        h.button("Paste blocks (Ctrl+V)");
        assert!(h.ctx.memory(|m| m.has_focus(Canvas::keyboard_id())));
        h.frame(vec![egui::Event::Paste(text)]);
        assert_eq!(h.model.nodes.len(), 3);
        assert_eq!(h.canvas.cursor, 1);
        press(&mut h, egui::Key::Z, CTRL);
        assert!(h.model.nodes.is_empty() && h.model.positions.is_empty());
    }
}

#[test]
fn model_clipboard_storage_node_limits_and_boundaries_preserve_existing_model() {
    let mut h = Harness::new();
    let original = h.model.clone();
    let fragment = Fragment::capture(&h.model, &[0, 1], 900.).unwrap();
    assert!(Paste::insert(&mut h.model, fragment, 900., 32., vec![], 1).is_err());
    assert!(h.model.same_storage(&original));
    h.model.nodes = (0..512)
        .map(|i| Node {
            name: i.to_string(),
            inputs: vec![],
            block: Block::Constant { value: 1. },
        })
        .collect();
    let fragment = Fragment::capture(&h.model, &[0], 900.).unwrap();
    assert!(Paste::insert(&mut h.model, fragment, 900., 32., vec![], HISTORY_BYTES).is_err());
    assert_eq!(h.model.nodes.len(), 512);
    h.model.nodes.truncate(70);
    for node in &mut h.model.nodes {
        node.block = Block::Filter {
            b: vec![1.; 4097],
            a: vec![1.; 4097],
        };
    }
    assert!(Fragment::capture(&h.model, &(0..70).collect::<Vec<_>>(), 900.).is_err());
    let mut h = Harness::new();
    h.model
        .set_layout(vec![
            [MAX_LAYOUT_COORDINATE, 10.],
            [MAX_LAYOUT_COORDINATE - 100., 50.],
            [0., 0.],
        ])
        .unwrap();
    insert(&mut h.model, &[0, 1]);
    assert_eq!(
        &h.model.positions[3..],
        &[
            [MAX_LAYOUT_COORDINATE, 42.],
            [MAX_LAYOUT_COORDINATE - 100., 82.]
        ]
    );
    assert_eq!(h.model.positions[3][0] - h.model.positions[4][0], 100.);
}
