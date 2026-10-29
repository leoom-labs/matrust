use super::keyboard_tests::{CTRL, press};
use super::organize::Operation;
use super::tests::Harness;
use super::*;
use matrust::simulation::{Block, MAX_LAYOUT_COORDINATE, Node};

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
fn rectangle(h: &mut Harness, start: Pos2, end: Pos2) {
    h.frame(pointer(start, true));
    h.frame(vec![egui::Event::PointerMoved(end)]);
    assert!(
        h.canvas.marquee.is_some(),
        "empty-space drag should start rectangle selection"
    );
    h.frame(pointer(end, false));
}

#[test]
fn model_organize_marquee_replace_add_reverse_empty_and_repeated_passes() {
    for repeat in [false, true] {
        let mut h = Harness::new();
        h.repeat = repeat;
        let original = h.model.clone();
        let origin = h.port(0, None) - Vec2::new(150., 52.);
        rectangle(
            &mut h,
            origin + Vec2::splat(10.),
            origin + Vec2::new(340., 90.),
        );
        assert_eq!(h.canvas.selection.indices(), &[0, 1]);
        h.modifiers = CTRL;
        rectangle(
            &mut h,
            origin + Vec2::new(350., 10.),
            origin + Vec2::new(505., 90.),
        );
        assert_eq!(h.canvas.selection.indices(), &[0, 1, 2]);
        h.modifiers = egui::Modifiers::NONE;
        rectangle(
            &mut h,
            origin + Vec2::new(505., 90.),
            origin + Vec2::new(350., 10.),
        );
        assert_eq!(h.canvas.selection.indices(), &[2]);
        rectangle(
            &mut h,
            origin + Vec2::new(510., 10.),
            origin + Vec2::new(550., 90.),
        );
        assert_eq!(h.selected, usize::MAX);
        assert!(h.canvas.selection.indices().is_empty());
        assert!(h.canvas.history.is_empty());
        assert!(h.model.same_storage(&original));
    }
}

#[test]
fn model_organize_marquee_cancellation_and_block_drag_remain_separate() {
    let mut h = Harness::new();
    let origin = h.port(0, None) - Vec2::new(150., 52.);
    let start = origin + Vec2::splat(10.);
    let end = origin + Vec2::new(340., 90.);
    for cancel in 0..4 {
        h.frame(pointer(start, true));
        h.frame(vec![egui::Event::PointerMoved(end)]);
        assert!(h.canvas.marquee.is_some(), "cancellation case {cancel}");
        match cancel {
            0 => press(&mut h, egui::Key::Escape, egui::Modifiers::NONE),
            1 => {
                h.enabled = false;
                h.frame(vec![]);
            }
            2 => {
                h.model
                    .set_layout(vec![[24., 24.], [194., 24.], [364., 24.]])
                    .unwrap();
                h.frame(vec![]);
            }
            _ => {}
        }
        h.frame(pointer(
            if cancel == 3 {
                Pos2::new(930., 590.)
            } else {
                end
            },
            false,
        ));
        h.enabled = true;
        h.frame(vec![]);
        assert!(h.canvas.marquee.is_none());
        assert_eq!(h.canvas.selection.indices(), &[0]);
    }
    let point = h.port(0, None) - Vec2::new(60., 0.);
    h.frame(pointer(point, true));
    h.frame(vec![egui::Event::PointerMoved(point + Vec2::new(20., 20.))]);
    assert!(h.canvas.moving.is_some());
    assert!(h.canvas.marquee.is_none());
    h.frame(pointer(point + Vec2::new(20., 20.), false));
    assert_eq!(h.canvas.cursor, 1);
}

#[test]
fn model_organize_all_alignments_use_outer_bounds_and_preserve_unselected_blocks() {
    for (op, _) in Operation::ALL.into_iter().take(6) {
        let mut h = Harness::new();
        h.model.nodes[1].inputs = vec![0; 8]; // 152 px tall instead of 56.
        h.model
            .set_layout(vec![[20., 30.], [300., 100.], [600., 70.]])
            .unwrap();
        h.canvas.selection.replace(3, vec![0, 1], &mut h.selected);
        let original = h.model.clone();
        assert!(h.canvas.organize(&mut h.model, 900., op).unwrap());
        let p = &h.model.positions;
        match op {
            Operation::Left => assert_eq!([p[0][0], p[1][0]], [20., 20.]),
            Operation::Right => assert_eq!([p[0][0], p[1][0]], [300., 300.]),
            Operation::CenterX => assert_eq!([p[0][0], p[1][0]], [160., 160.]),
            Operation::Top => assert_eq!([p[0][1], p[1][1]], [30., 30.]),
            Operation::Bottom => assert_eq!([p[0][1], p[1][1]], [196., 100.]),
            Operation::CenterY => assert_eq!([p[0][1], p[1][1]], [113., 65.]),
            _ => unreachable!(),
        }
        assert_eq!(p[2], [600., 70.]);
        assert_eq!(h.canvas.cursor, 1);
        let organized = h.model.clone();
        let mut message = String::new();
        h.canvas
            .travel(&mut h.model, &mut h.selected, false, &mut message);
        assert!(h.model.same_storage(&original));
        h.canvas
            .travel(&mut h.model, &mut h.selected, true, &mut message);
        assert!(h.model.same_storage(&organized));
        let saved = serde_json::to_string(&h.model).unwrap();
        let loaded: Model = serde_json::from_str(&saved).unwrap();
        assert!(loaded.same_storage(&organized));
    }
}

#[test]
fn model_organize_distribution_spatial_order_noop_and_bounds_are_atomic() {
    for op in [Operation::SpaceX, Operation::SpaceY] {
        let mut h = Harness::new();
        h.model
            .set_layout(vec![[500., 500.], [0., 0.], [100., 100.]])
            .unwrap();
        h.canvas.selection.all(3, &mut h.selected);
        assert!(h.canvas.organize(&mut h.model, 900., op).unwrap());
        let axis = usize::from(matches!(op, Operation::SpaceY));
        assert_eq!(h.model.positions[2][axis], 250.);
        assert_eq!(h.model.positions[0], [500., 500.]);
        assert_eq!(h.model.positions[1], [0., 0.]);
        assert!(!h.canvas.organize(&mut h.model, 900., op).unwrap());
        assert_eq!(h.canvas.cursor, 1);
    }
    let mut h = Harness::new();
    h.canvas.selection.all(3, &mut h.selected);
    assert!(
        !h.canvas
            .organize(&mut h.model, 900., Operation::Top)
            .unwrap()
    );
    assert_eq!(h.model.version, 1); // A no-op must not create a stored layout.
    h.model.nodes[1].inputs = vec![0; 32];
    h.model
        .set_layout(vec![[0., 0.], [0., MAX_LAYOUT_COORDINATE], [0., 0.]])
        .unwrap();
    let original = h.model.clone();
    assert!(
        h.canvas
            .organize(&mut h.model, 900., Operation::Bottom)
            .is_err()
    );
    assert!(h.model.same_storage(&original));
    assert!(h.canvas.history.is_empty());
}

#[test]
fn model_organize_menu_undo_focus_repeated_passes_and_512_filter_ownership() {
    for repeat in [false, true] {
        let mut h = Harness::new();
        h.repeat = repeat;
        h.canvas.selection.all(3, &mut h.selected);
        h.button("Align / distribute");
        h.button("Align left");
        assert_eq!(h.model.positions, vec![[24., 24.]; 3]);
        assert_eq!(h.canvas.cursor, 1);
        assert!(
            h.ctx.memory(|m| m.has_focus(Canvas::keyboard_id())),
            "canvas focus after menu, repeat={repeat}"
        );
        assert!(
            !egui::Popup::is_any_open(&h.ctx),
            "menu closed, repeat={repeat}"
        );
        press(&mut h, egui::Key::Z, CTRL);
        assert!(
            h.model.positions.is_empty(),
            "undo after menu, repeat={repeat}, cursor={}",
            h.canvas.cursor
        );
    }
    let mut h = Harness::new();
    h.model.nodes = (0..512)
        .map(|index| Node {
            name: index.to_string(),
            inputs: vec![0],
            block: Block::Filter {
                b: vec![1.; 4097],
                a: vec![1.],
            },
        })
        .collect();
    h.canvas.selection.all(512, &mut h.selected);
    let pointers: Vec<_> = h
        .model
        .nodes
        .iter()
        .map(|node| match &node.block {
            Block::Filter { b, .. } => b.as_ptr(),
            _ => unreachable!(),
        })
        .collect();
    assert!(
        h.canvas
            .organize(&mut h.model, 900., Operation::Left)
            .unwrap()
    );
    assert!(h.canvas.history_bytes < 16 * 1024);
    let mut message = String::new();
    h.canvas
        .travel(&mut h.model, &mut h.selected, false, &mut message);
    for (node, pointer) in h.model.nodes.iter().zip(pointers) {
        if let Block::Filter { b, .. } = &node.block {
            assert_eq!(b.as_ptr(), pointer);
        }
    }
    assert!(h.model.positions.is_empty());
}

#[test]
fn model_organize_scrolled_selection_and_layout_undo_preserve_signal_and_newer_edits() {
    let mut h = Harness::new();
    h.model
        .set_layout(vec![[24., 1000.], [194., 1000.], [364., 1000.]])
        .unwrap();
    h.canvas.reveal(2);
    for _ in 0..24 {
        h.frame(vec![]);
    }
    let point = h.port(0, None);
    rectangle(
        &mut h,
        point - Vec2::new(140., 40.),
        point + Vec2::new(180., 40.),
    );
    assert_eq!(h.canvas.selection.indices(), &[0, 1]);
    let before = h
        .model
        .run(&std::sync::atomic::AtomicBool::new(false))
        .unwrap();
    assert!(
        h.canvas
            .organize(&mut h.model, 900., Operation::Right)
            .unwrap()
    );
    let after = h
        .model
        .run(&std::sync::atomic::AtomicBool::new(false))
        .unwrap();
    assert_eq!(
        before.traces[0].values.data(),
        after.traces[0].values.data()
    );
    h.model.positions[2][0] += 17.;
    let newer = h.model.clone();
    let mut message = String::new();
    h.canvas
        .travel(&mut h.model, &mut h.selected, false, &mut message);
    assert!(h.model.same_storage(&newer));
    assert!(h.canvas.history.is_empty());
}
