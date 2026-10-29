use super::keyboard_tests::{CTRL, press};
use super::tests::Harness;
use super::*;

fn pointer(pos: Pos2, button: egui::PointerButton, pressed: bool) -> Vec<egui::Event> {
    vec![
        egui::Event::PointerMoved(pos),
        egui::Event::PointerButton {
            pos,
            button,
            pressed,
            modifiers: egui::Modifiers::NONE,
        },
    ]
}
fn wheel(pos: Pos2, delta: f32, modifiers: egui::Modifiers) -> Vec<egui::Event> {
    vec![
        egui::Event::PointerMoved(pos),
        egui::Event::MouseWheel {
            phase: egui::TouchPhase::Move,
            unit: egui::MouseWheelUnit::Line,
            delta: Vec2::new(0., delta),
            modifiers,
        },
    ]
}
fn large(h: &mut Harness) {
    h.model
        .set_layout(vec![[400., 300.], [900., 700.], [1500., 1200.]])
        .unwrap();
    for _ in 0..3 {
        h.frame(vec![]);
    }
    h.canvas.view.requested_offset = Some(Vec2::new(250., 200.));
    for _ in 0..3 {
        h.frame(vec![]);
    }
}

#[test]
fn model_navigation_control_wheel_anchors_pointer_once_and_keeps_model_exact() {
    for repeat in [false, true] {
        let mut h = Harness::new();
        h.repeat = repeat;
        large(&mut h);
        let original = h.model.clone();
        let p = h.canvas.view.viewport.center();
        let before = h.canvas.view.model_point(p, h.canvas.view.origin);
        h.frame(wheel(p, 1., CTRL));
        h.frame(vec![]);
        assert_eq!(h.canvas.view.zoom, 1.25);
        let after = h.canvas.view.model_point(p, h.canvas.view.origin);
        assert!(
            (before - after).length() < 1.,
            "pointer anchor changed: {before:?} -> {after:?}"
        );
        h.frame(wheel(p, -1., CTRL));
        h.frame(vec![]);
        assert_eq!(h.canvas.view.zoom, 1.);
        assert!(h.model.same_storage(&original));
        assert!(h.canvas.history.is_empty());
        assert!(!h.canvas.take_changed());
    }
}

#[test]
fn model_navigation_middle_drag_pans_over_blocks_and_escape_stops_it() {
    for repeat in [false, true] {
        let mut h = Harness::new();
        h.repeat = repeat;
        large(&mut h);
        let original = h.model.clone();
        let block = h.canvas.blocks.iter().find(|(i, _)| *i == 0).unwrap().1;
        let p = block.center();
        let old = h.canvas.view.offset;
        h.frame(pointer(p, egui::PointerButton::Middle, true));
        h.frame(vec![egui::Event::PointerMoved(p - Vec2::new(40., 30.))]);
        assert!((h.canvas.view.offset - (old + Vec2::new(40., 30.))).length() < 1.);
        assert!(
            h.canvas.moving.is_none() && h.canvas.marquee.is_none() && h.canvas.source.is_none()
        );
        press(&mut h, egui::Key::Escape, egui::Modifiers::NONE);
        let stopped = h.canvas.view.offset;
        h.frame(vec![egui::Event::PointerMoved(p - Vec2::splat(90.))]);
        assert_eq!(h.canvas.view.offset, stopped);
        h.frame(pointer(p, egui::PointerButton::Middle, false));
        assert!(h.canvas.view.pan.is_none());
        assert!(h.model.same_storage(&original));
        assert!(h.canvas.history.is_empty());
    }
}

#[test]
fn model_navigation_wheel_and_pan_guards_limits_and_ordinary_scroll() {
    let mut h = Harness::new();
    large(&mut h);
    let p = h.canvas.view.viewport.center();
    h.frame(wheel(Pos2::new(920., 590.), 1., CTRL));
    assert_eq!(h.canvas.view.zoom, 1.);
    h.keyboard_allowed = false;
    h.frame(wheel(p, 1., CTRL));
    h.frame(pointer(p, egui::PointerButton::Middle, true));
    assert_eq!(h.canvas.view.zoom, 1.);
    assert!(h.canvas.view.pan.is_none());
    h.frame(pointer(p, egui::PointerButton::Middle, false));
    h.keyboard_allowed = true;
    h.enabled = false;
    h.frame(wheel(p, 1., CTRL));
    assert_eq!(h.canvas.view.zoom, 1.);
    h.enabled = true;
    h.frame(vec![]);
    h.frame(wheel(p, 10000., CTRL));
    assert_eq!(h.canvas.view.zoom, 4.);
    h.frame(wheel(p, f32::NAN, CTRL));
    assert_eq!(h.canvas.view.zoom, 4.);
    h.frame(wheel(p, -1., egui::Modifiers::NONE));
    h.frame(vec![]);
    assert_eq!(h.canvas.view.zoom, 4.);
    assert!(h.canvas.view.offset.is_finite());
}

#[test]
fn model_navigation_empty_canvas_context_paste_requests_native_clipboard_and_focus() {
    for repeat in [false, true] {
        let mut h = Harness::new();
        h.repeat = repeat;
        let text = clipboard::Fragment::capture(&h.model, &[0], 900.)
            .unwrap()
            .encode()
            .unwrap();
        h.model.nodes.clear();
        h.canvas.reset();
        h.selected = usize::MAX;
        for _ in 0..3 {
            h.frame(vec![]);
        }
        let p = h.canvas.view.viewport.center();
        h.frame(pointer(p, egui::PointerButton::Secondary, true));
        h.frame(pointer(p, egui::PointerButton::Secondary, false));
        h.button("Paste blocks (Ctrl+V)");
        assert!(
            h.ctx
                .memory(|memory| memory.has_focus(Canvas::keyboard_id()))
        );
        h.frame(vec![egui::Event::Paste(text)]);
        assert_eq!(h.model.nodes.len(), 1);
        assert_eq!(h.canvas.cursor, 1);
        press(&mut h, egui::Key::Z, CTRL);
        assert!(h.model.nodes.is_empty());
    }
}

#[test]
fn model_navigation_point_wheel_and_final_pan_release_do_not_move_blocks() {
    let mut h = Harness::new();
    large(&mut h);
    let original = h.model.clone();
    let p = h.canvas.view.viewport.center();
    h.frame(vec![
        egui::Event::PointerMoved(p),
        egui::Event::MouseWheel {
            phase: egui::TouchPhase::Move,
            unit: egui::MouseWheelUnit::Point,
            delta: Vec2::new(0., 100.),
            modifiers: CTRL,
        },
    ]);
    h.frame(vec![]);
    assert_eq!(h.canvas.view.zoom, 1.25);
    let old = h.canvas.view.offset;
    let p = h.canvas.view.viewport.center();
    h.frame(pointer(p, egui::PointerButton::Middle, true));
    h.frame(pointer(
        p - Vec2::new(70., 60.),
        egui::PointerButton::Middle,
        false,
    ));
    assert!((h.canvas.view.offset - (old + Vec2::new(70., 60.))).length() < 1.);
    assert!(h.canvas.view.pan.is_none());
    h.frame(pointer(p, egui::PointerButton::Middle, true));
    h.frame(pointer(p, egui::PointerButton::Primary, true));
    h.frame(vec![egui::Event::PointerMoved(p - Vec2::splat(20.))]);
    assert!(h.canvas.moving.is_none() && h.canvas.marquee.is_none() && h.canvas.source.is_none());
    h.frame(pointer(
        p - Vec2::splat(20.),
        egui::PointerButton::Primary,
        false,
    ));
    h.frame(pointer(
        p - Vec2::splat(20.),
        egui::PointerButton::Middle,
        false,
    ));
    assert!(h.model.same_storage(&original));
    assert!(h.canvas.history.is_empty());
}
