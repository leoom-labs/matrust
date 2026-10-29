use super::*;
use matrust::{
    Array,
    variable_edit::cell_replace::{Binding, CellTarget},
};
use std::time::{Duration, Instant};
fn finish(app: &mut Workspace) {
    let end = Instant::now() + Duration::from_secs(5);
    while app.busy && Instant::now() < end {
        app.receive();
        std::thread::sleep(Duration::from_millis(1));
    }
    assert!(!app.busy, "{}", app.status);
}
fn frame(
    app: &mut Workspace,
    ctx: &egui::Context,
    events: Vec<egui::Event>,
    discard: bool,
) -> egui::FullOutput {
    frame_size(app, ctx, events, discard, egui::vec2(1200., 900.))
}
fn frame_size(
    app: &mut Workspace,
    ctx: &egui::Context,
    events: Vec<egui::Event>,
    discard: bool,
    size: egui::Vec2,
) -> egui::FullOutput {
    crate::headless::run(
        ctx,
        egui::RawInput {
            screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, size)),
            events,
            ..Default::default()
        },
        |ctx| {
            if discard && ctx.current_pass_index() == 0 {
                ctx.request_discard("cell replacement final pass");
            }
            app.variable_editor(ctx);
        },
    )
}
fn idle(app: &mut Workspace, ctx: &egui::Context) -> egui::FullOutput {
    for _ in 0..3 {
        frame(app, ctx, vec![], false);
    }
    frame(app, ctx, vec![], false)
}
fn find(shape: &egui::Shape, clip: egui::Rect, label: &str) -> Option<egui::Pos2> {
    match shape {
        egui::Shape::Text(t)
            if t.galley.text() == label && clip.contains_rect(t.visual_bounding_rect()) =>
        {
            Some(t.visual_bounding_rect().center())
        }
        egui::Shape::Vec(items) => items.iter().find_map(|s| find(s, clip, label)),
        _ => None,
    }
}
fn click(app: &mut Workspace, ctx: &egui::Context, label: &str, discard_release: bool) {
    let output = idle(app, ctx);
    let pos = output
        .shapes
        .iter()
        .rev()
        .find_map(|s| find(&s.shape, s.clip_rect, label))
        .unwrap_or_else(|| panic!("Missing visible {label}"));
    for pressed in [true, false] {
        frame(
            app,
            ctx,
            vec![
                egui::Event::PointerMoved(pos),
                egui::Event::PointerButton {
                    pos,
                    button: egui::PointerButton::Primary,
                    pressed,
                    modifiers: egui::Modifiers::NONE,
                },
            ],
            discard_release && !pressed,
        );
    }
}
fn choose(app: &mut Workspace, ctx: &egui::Context, name: &str) {
    click(app, ctx, "Workspace variable name", false);
    frame(app, ctx, vec![egui::Event::Text(name.into())], false);
    click(app, ctx, "Select value", false);
}
fn leaf<'a>(mut value: &'a Array, path: &[usize]) -> &'a Array {
    for &i in path {
        let Array::Cell(c) = value else { panic!() };
        value = c.get_linear(i).unwrap();
    }
    value
}
#[test]
fn actual_nested_replace_picker_submits_once_and_preserves_exact_character_and_aliases() {
    let ctx = egui::Context::default();
    let mut app = Workspace::with_context(ctx.clone(), true);
    app.run("A=[1 2];D=cell(1,2);D{1}=A;D{2}=A;C=cell(1,2);C{1}=A;C{2}=D;Alias=C;S=reshape(char([0 55296 65 56320 65535 66]),2,3);".into());
    finish(&mut app);
    let root = app.variables["C"].clone();
    let source = app.variables["S"].clone();
    app.inspect_variable("C".into());
    click(&mut app, &ctx, "Open {1,2}", false);
    click(&mut app, &ctx, "Replace {1,1}", false);
    choose(&mut app, &ctx, "S");
    let (send, receive) = std::sync::mpsc::sync_channel(4);
    let real = std::mem::replace(&mut app.worker.send, send);
    app.recovery_dirty = false;
    click(&mut app, &ctx, "Replace contents", true);
    let job = receive.try_recv().unwrap();
    assert!(receive.try_recv().is_err());
    let Job::ReplaceCell(request) = &job.job else {
        panic!("actual replacement route")
    };
    assert_eq!(request.target.path(), [1, 0]);
    assert_eq!(request.source.name(), "S");
    assert!(request.source.expected().same_snapshot(&source));
    app.worker.send = real;
    app.worker.send.send(job).unwrap();
    finish(&mut app);
    let after = &app.variables["C"];
    assert!(leaf(after, &[1, 0]).same_snapshot(&source));
    assert!(leaf(after, &[1, 1]).same_snapshot(leaf(&root, &[1, 1])));
    assert!(leaf(after, &[0]).same_snapshot(leaf(&root, &[0])));
    assert!(app.variables["Alias"].same_snapshot(&root));
    assert!(app.variables["S"].same_snapshot(&source));
    assert!(
        app.recovery_dirty,
        "Actual replacement participates in existing checkpoint publication"
    );
    assert!(app.typed_inspector.path.is_empty());
    let output = idle(&mut app, &ctx);
    assert!(output.shapes.iter().any(|s| {
        find(
            &s.shape,
            s.clip_rect,
            "Cell contents replaced; returned to the root value.",
        )
        .is_some()
    }));
}
#[test]
fn picker_cancel_busy_modal_and_unrelated_job_never_publish_a_replacement() {
    let ctx = egui::Context::default();
    let mut app = Workspace::with_context(ctx.clone(), true);
    app.run("C=cell(1,1);C{1}=3;Alias=C;S=logical([1 0]);".into());
    finish(&mut app);
    let root = app.variables["C"].clone();
    app.inspect_variable("C".into());
    click(&mut app, &ctx, "Replace {1,1}", false);
    choose(&mut app, &ctx, "S");
    app.pending_close = Some(0);
    click(&mut app, &ctx, "Replace contents", false);
    assert!(!app.busy);
    app.pending_close = None;
    click(&mut app, &ctx, "Cancel replacement", false);
    assert!(!app.busy);
    assert!(app.variables["C"].same_snapshot(&root));
    click(&mut app, &ctx, "Replace {1,1}", false);
    choose(&mut app, &ctx, "S");
    app.run("clear S;S=[8 9];".into());
    finish(&mut app);
    let output = idle(&mut app, &ctx);
    assert!(output.shapes.iter().any(|s| {
        find(
            &s.shape,
            s.clip_rect,
            "Workspace operation completed; select the replacement values again.",
        )
        .is_some()
    }));
    assert!(app.variables["C"].same_snapshot(&root));
    assert!(app.variables["Alias"].same_snapshot(&root));
}
#[test]
fn real_worker_rejects_rebound_source_revision_then_stop_and_retry_share_graphics() {
    let ctx = egui::Context::default();
    let worker = Worker::new(ctx);
    worker
        .send
        .send(Job::Script("C=cell(1,1);C{1}=5;Alias=C;S=figure;Keep=S;".into()).into())
        .unwrap();
    let first = worker.receive.recv_timeout(Duration::from_secs(5)).unwrap();
    let root = first.vars["C"].clone();
    let source = first.vars["S"].clone();
    let target = CellTarget::new(Binding::new("C".into(), root.clone()).unwrap(), vec![0]).unwrap();
    let binding = Binding::new("S".into(), source.clone()).unwrap();
    worker
        .send
        .send(Job::Script("clear S;S=Keep;".into()).into())
        .unwrap();
    let rebound = worker.receive.recv_timeout(Duration::from_secs(5)).unwrap();
    assert!(rebound.vars["S"].same_snapshot(&source));
    let request = |revision| cell_replace_job::Request {
        revision,
        editor: Default::default(),
        target: target.clone(),
        source: binding.clone(),
    };
    worker
        .send
        .send(Job::ReplaceCell(Box::new(request(first.variable_revision))).into())
        .unwrap();
    let stale = worker.receive.recv_timeout(Duration::from_secs(5)).unwrap();
    assert!(
        stale
            .result
            .as_ref()
            .unwrap_err()
            .contains("Workspace operation completed")
    );
    assert!(stale.vars["C"].same_snapshot(&root));
    worker.cancel.store(true, Ordering::Relaxed);
    worker
        .send
        .send(Job::ReplaceCell(Box::new(request(stale.variable_revision))).into())
        .unwrap();
    let stopped = worker.receive.recv_timeout(Duration::from_secs(5)).unwrap();
    assert!(stopped.result.as_ref().unwrap_err().contains("cancelled"));
    assert!(stopped.vars["C"].same_snapshot(&root));
    worker.cancel.store(false, Ordering::Relaxed);
    worker
        .send
        .send(Job::ReplaceCell(Box::new(request(stopped.variable_revision))).into())
        .unwrap();
    let changed = worker.receive.recv_timeout(Duration::from_secs(5)).unwrap();
    assert!(changed.result.is_ok());
    assert!(
        changed
            .cell_replacement
            .as_ref()
            .unwrap()
            .result
            .as_ref()
            .unwrap()
            .same_snapshot(&changed.vars["C"])
    );
    assert!(leaf(&changed.vars["C"], &[0]).same_snapshot(&source));
    assert!(changed.vars["Alias"].same_snapshot(&root));
    assert_eq!(changed.figures.len(), rebound.figures.len());
    for (id, figure) in &rebound.figures {
        assert!(figure.same_snapshot(&changed.figures[id]));
    }
}

#[test]
fn closing_and_reopening_does_not_adopt_an_old_picker_reply() {
    let ctx = egui::Context::default();
    let mut app = Workspace::with_context(ctx.clone(), true);
    app.run("C=cell(1,1);C{1}=1;S=[7 8];".into());
    finish(&mut app);
    let source = app.variables["S"].clone();
    app.inspect_variable("C".into());
    click(&mut app, &ctx, "Replace {1,1}", false);
    choose(&mut app, &ctx, "S");
    let (send, receive) = std::sync::mpsc::sync_channel(4);
    let real = std::mem::replace(&mut app.worker.send, send);
    click(&mut app, &ctx, "Replace contents", false);
    let job = receive.try_recv().unwrap();
    // This job was already admitted: closing invalidates UI adoption, not an
    // already accepted transaction. Stop is the explicit cooperative cancel.
    app.editing_variable = None;
    idle(&mut app, &ctx);
    app.inspect_variable("C".into());
    idle(&mut app, &ctx);
    app.worker.send = real;
    app.worker.send.send(job).unwrap();
    finish(&mut app);
    assert!(leaf(&app.variables["C"], &[0]).same_snapshot(&source));
    let output = idle(&mut app, &ctx);
    assert!(!output.shapes.iter().any(|s| {
        find(
            &s.shape,
            s.clip_rect,
            "Cell contents replaced; returned to the root value.",
        )
        .is_some()
    }));
    assert_eq!(app.editing_variable.as_deref(), Some("C"));
}

#[test]
fn compact_long_name_nested_replacement_keeps_actual_controls_reachable() {
    let size = egui::vec2(960., 650.);
    let target_name = format!("Target_{}", "t".repeat(50));
    let source_name = format!("Source_{}", "s".repeat(50));
    for dark in [false, true] {
        let ctx = egui::Context::default();
        let mut app = Workspace::with_context(ctx.clone(), true);
        app.layout.theme = Some(if dark {
            session::DesktopTheme::Dark
        } else {
            session::DesktopTheme::Light
        });
        app.apply_desktop_theme(&ctx);
        assert_eq!(ctx.global_style().visuals.dark_mode, dark);
        app.run(format!("A=[1 2];Inner=cell(1,2);Inner{{1}}=A;Inner{{2}}=A;Middle=cell(1,2);Middle{{1}}=Inner;Middle{{2}}=A;{target_name}=cell(1,2);{target_name}{{1}}=A;{target_name}{{2}}=Middle;Alias={target_name};{source_name}=reshape(char([0 55296 65 56320 65535 66]),2,3);"));
        finish(&mut app);
        let before = app.variables[&target_name].clone();
        let source = app.variables[&source_name].clone();
        app.inspect_variable(target_name.clone());
        let paint = |app: &mut Workspace| {
            for _ in 0..3 {
                frame_size(app, &ctx, vec![], false, size);
            }
            frame_size(app, &ctx, vec![], false, size)
        };
        let pointer = |app: &mut Workspace, label: &str| {
            let output = paint(app);
            let screen = egui::Rect::from_min_size(egui::Pos2::ZERO, size);
            let pos = output
                .shapes
                .iter()
                .rev()
                .find_map(|s| find(&s.shape, s.clip_rect.intersect(screen), label))
                .unwrap_or_else(|| {
                    panic!("Control not fully painted in compact viewport: {label}; dark={dark}")
                });
            for pressed in [true, false] {
                frame_size(
                    app,
                    &ctx,
                    vec![
                        egui::Event::PointerMoved(pos),
                        egui::Event::PointerButton {
                            pos,
                            button: egui::PointerButton::Primary,
                            pressed,
                            modifiers: egui::Modifiers::NONE,
                        },
                    ],
                    false,
                    size,
                );
            }
        };
        pointer(&mut app, "Open {1,2}");
        pointer(&mut app, "Open {1,1}");
        pointer(&mut app, "Replace {1,2}");
        pointer(&mut app, "Workspace variable name");
        frame_size(
            &mut app,
            &ctx,
            vec![egui::Event::Text(source_name.clone())],
            false,
            size,
        );
        pointer(&mut app, "Select value");
        let output = paint(&mut app);
        let screen = egui::Rect::from_min_size(egui::Pos2::ZERO, size);
        for label in [
            format!("Replace contents of {target_name}{{2}}{{1}}{{2}}"),
            format!("Selected {source_name}: 2 x 3 | char"),
            "Cancel replacement".into(),
        ] {
            assert!(
                output
                    .shapes
                    .iter()
                    .any(|s| find(&s.shape, s.clip_rect.intersect(screen), &label).is_some()),
                "{label}"
            );
        }
        pointer(&mut app, "Replace contents");
        finish(&mut app);
        let after = &app.variables[&target_name];
        assert!(leaf(after, &[1, 0, 1]).same_snapshot(&source));
        assert!(leaf(after, &[1, 0, 0]).same_snapshot(leaf(&before, &[1, 0, 0])));
        assert!(leaf(after, &[0]).same_snapshot(leaf(&before, &[0])));
        assert!(app.variables["Alias"].same_snapshot(&before));
        assert!(app.variables[&source_name].same_snapshot(&source));
        assert_eq!(app.editing_variable.as_deref(), Some(target_name.as_str()));
        assert!(app.typed_inspector.path.is_empty());
        let output = paint(&mut app);
        assert!(output.shapes.iter().any(|s| {
            find(
                &s.shape,
                s.clip_rect.intersect(screen),
                "Cell contents replaced; returned to the root value.",
            )
            .is_some()
        }));
    }
}
