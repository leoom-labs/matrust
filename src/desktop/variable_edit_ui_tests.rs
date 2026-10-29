use super::*;
use matrust::{
    Array,
    variable_edit::{Rect, Target},
};
use std::time::{Duration, Instant};

fn finish(app: &mut Workspace) {
    let deadline = Instant::now() + Duration::from_secs(5);
    while app.busy && Instant::now() < deadline {
        app.receive();
        std::thread::sleep(Duration::from_millis(1));
    }
    assert!(!app.busy, "{}", app.status);
}
fn frame(
    app: &mut Workspace,
    ctx: &egui::Context,
    events: Vec<egui::Event>,
    modifiers: egui::Modifiers,
    discard: bool,
) -> egui::FullOutput {
    crate::headless::run(
        ctx,
        egui::RawInput {
            screen_rect: Some(egui::Rect::from_min_size(
                egui::Pos2::ZERO,
                egui::vec2(1100., 760.),
            )),
            events: std::iter::once(egui::Event::ModifiersChanged(modifiers))
                .chain(events)
                .collect(),
            ..Default::default()
        },
        |ctx| {
            if discard && ctx.current_pass_index() == 0 {
                ctx.request_discard("rooted edit final pass");
            }
            app.variable_editor(ctx);
        },
    )
}
fn idle(app: &mut Workspace, ctx: &egui::Context) -> egui::FullOutput {
    for _ in 0..3 {
        frame(app, ctx, vec![], egui::Modifiers::NONE, false);
    }
    frame(app, ctx, vec![], egui::Modifiers::NONE, false)
}
fn find(shape: &egui::Shape, clip: egui::Rect, label: &str) -> Option<egui::Pos2> {
    match shape {
        egui::Shape::Text(t)
            if t.galley.text() == label && clip.contains_rect(t.visual_bounding_rect()) =>
        {
            Some(t.visual_bounding_rect().center())
        }
        egui::Shape::Vec(v) => v.iter().find_map(|s| find(s, clip, label)),
        _ => None,
    }
}
fn click(app: &mut Workspace, ctx: &egui::Context, label: &str, modifiers: egui::Modifiers) {
    let out = idle(app, ctx);
    let pos = out
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
                    modifiers,
                },
            ],
            modifiers,
            false,
        );
    }
}
fn copied(output: egui::FullOutput) -> Option<String> {
    output.platform_output.commands.into_iter().find_map(|c| {
        if let egui::OutputCommand::CopyText(s) = c {
            Some(s)
        } else {
            None
        }
    })
}
fn child(value: &Array, index: usize) -> &Array {
    let Array::Cell(c) = value else { panic!() };
    c.get_linear(index).unwrap()
}

#[test]
fn real_grid_shift_selection_copies_and_pastes_one_atomic_rectangle_with_aliases() {
    for kind in [
        "A=[11 12 13;21 22 23];",
        "A=logical([0 1 0;1 0 1]);",
        "A=complex([11 12 13;21 22 23],[1 2 3;4 5 6]);",
    ] {
        let ctx = egui::Context::default();
        let mut app = Workspace::with_context(ctx.clone(), true);
        app.run(format!("{kind}Alias=A;"));
        finish(&mut app);
        let original = app.variables["A"].clone();
        app.inspect_variable("A".into());
        idle(&mut app, &ctx);
        let pos = app.numeric_editor.cells[&(0, 0)].center();
        for pressed in [true, false] {
            frame(
                &mut app,
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
                egui::Modifiers::NONE,
                false,
            );
        }
        // Extend through a real Shift-click, then a real Shift+arrow event.
        let pos = app.numeric_editor.cells[&(0, 2)].center();
        for pressed in [true, false] {
            frame(
                &mut app,
                &ctx,
                vec![
                    egui::Event::PointerMoved(pos),
                    egui::Event::PointerButton {
                        pos,
                        button: egui::PointerButton::Primary,
                        pressed,
                        modifiers: egui::Modifiers::SHIFT,
                    },
                ],
                egui::Modifiers::SHIFT,
                false,
            );
        }
        frame(
            &mut app,
            &ctx,
            vec![egui::Event::Key {
                key: egui::Key::ArrowDown,
                physical_key: None,
                pressed: true,
                repeat: false,
                modifiers: egui::Modifiers::SHIFT,
            }],
            egui::Modifiers::SHIFT,
            false,
        );
        app.recovery_dirty = false;
        let output = frame(
            &mut app,
            &ctx,
            vec![egui::Event::Copy],
            egui::Modifiers::NONE,
            false,
        );
        assert!(copied(output).is_none());
        finish(&mut app);
        let text = copied(frame(&mut app, &ctx, vec![], egui::Modifiers::NONE, false))
            .expect("Worker clipboard reply");
        assert_eq!(text.lines().count(), 2);
        assert!(text.lines().all(|s| s.split('\t').count() == 3));
        assert!(original.same_snapshot(&app.variables["A"]));
        assert!(original.same_snapshot(&app.variables["Alias"]));
        assert!(!app.recovery_dirty, "Copy must not dirty the workspace");
        let paste = if original.is_complex() {
            "31-1i\t32+2i\t33-3i\n41-4i\t42+5i\t43-6i"
        } else if original.is_logical() {
            "true\tfalse\ttrue\nfalse\ttrue\tfalse"
        } else {
            "31\t32\t33\n41\t42\t43"
        };
        // A real selection remains focused across Copy and drives rectangular Paste.
        click(&mut app, &ctx, "Paste selection", egui::Modifiers::NONE);
        frame(
            &mut app,
            &ctx,
            vec![egui::Event::Paste(paste.into())],
            egui::Modifiers::NONE,
            false,
        );
        finish(&mut app);
        assert!(app.cell_error.is_none(), "{:?}", app.cell_error);
        assert!(app.recovery_dirty);
        let result = &app.variables["A"];
        assert!(!result.same_snapshot(&original));
        assert!(app.variables["Alias"].same_snapshot(&original));
        for (r, line) in paste.lines().enumerate() {
            for (c, field) in line.split('\t').enumerate() {
                let expected =
                    matrust::Scalar::parse_cell(field, result.is_complex(), result.is_logical())
                        .unwrap()
                        .into_complex();
                assert_eq!(result.scalar(r, c).unwrap().into_complex(), expected);
            }
        }
    }
}

#[test]
fn nested_numeric_apply_changes_root_path_and_preserves_other_children_and_aliases() {
    let ctx = egui::Context::default();
    let mut app = Workspace::with_context(ctx.clone(), true);
    app.run("A=[11 12;21 22];C={A,{A,A}};Alias=C;".into());
    finish(&mut app);
    let root = app.variables["C"].clone();
    let leaf = app.variables["A"].clone();
    app.inspect_variable("C".into());
    click(&mut app, &ctx, "Open {1,2}", egui::Modifiers::NONE);
    click(&mut app, &ctx, "Open {1,1}", egui::Modifiers::NONE);
    click(&mut app, &ctx, "21", egui::Modifiers::NONE);
    app.edit_value = "99".into();
    click(&mut app, &ctx, "Apply cell", egui::Modifiers::NONE);
    finish(&mut app);
    assert!(app.cell_error.is_none(), "{:?}", app.cell_error);
    let changed = &app.variables["C"];
    assert_eq!(
        child(child(changed, 1), 0).scalar(1, 0).unwrap(),
        matrust::Scalar::Real(99.)
    );
    assert!(child(changed, 0).same_snapshot(&leaf));
    assert!(child(child(changed, 1), 1).same_snapshot(&leaf));
    assert!(app.variables["Alias"].same_snapshot(&root));
    assert!(app.variables["A"].same_snapshot(&leaf));
    let out = idle(&mut app, &ctx);
    assert!(
        out.shapes
            .iter()
            .any(|s| find(&s.shape, s.clip_rect, "Cell {1,2}").is_some())
    );
}

#[test]
fn stale_worker_revision_rejects_exact_historical_root_rebind_before_literal_parsing() {
    let ctx = egui::Context::default();
    let worker = Worker::new(ctx);
    worker
        .send
        .send(Job::Script("A=[1 2];Alias=A;".into()).into())
        .unwrap();
    let first = worker.receive.recv_timeout(Duration::from_secs(5)).unwrap();
    let original = first.vars["A"].clone();
    let target = Target::new("A".into(), original.clone(), vec![]).unwrap();
    worker
        .send
        .send(Job::Script("clear A;A=Alias;".into()).into())
        .unwrap();
    let rebound = worker.receive.recv_timeout(Duration::from_secs(5)).unwrap();
    assert!(rebound.vars["A"].same_snapshot(&original));
    worker
        .send
        .send(
            Job::RootedVariable(Box::new(variable_edit_job::Request {
                revision: first.variable_revision,
                editor: Default::default(),
                target: target.clone(),
                rectangle: Rect::new(0, 0, 1, 1).unwrap(),
                action: variable_edit_job::Action::Paste("not a literal".into()),
            }))
            .into(),
        )
        .unwrap();
    let rejected = worker.receive.recv_timeout(Duration::from_secs(5)).unwrap();
    assert!(
        rejected
            .result
            .as_ref()
            .err()
            .unwrap()
            .contains("Workspace operation completed")
    );
    assert!(rejected.vars["A"].same_snapshot(&original));
    assert!(rejected.vars["Alias"].same_snapshot(&original));
    worker.cancel.store(true, Ordering::Relaxed);
    worker
        .send
        .send(
            Job::RootedVariable(Box::new(variable_edit_job::Request {
                revision: rejected.variable_revision,
                editor: Default::default(),
                target,
                rectangle: Rect::new(0, 0, 1, 1).unwrap(),
                action: variable_edit_job::Action::Literal("9".into()),
            }))
            .into(),
        )
        .unwrap();
    let stopped = worker.receive.recv_timeout(Duration::from_secs(5)).unwrap();
    assert!(stopped.result.is_err());
    assert!(stopped.vars["A"].same_snapshot(&original));
}

#[test]
fn discarded_paste_submits_once_and_reopened_editor_cannot_publish_late_copy() {
    let ctx = egui::Context::default();
    let mut app = Workspace::with_context(ctx.clone(), true);
    app.run("A=[11 12;21 22];Alias=A;".into());
    finish(&mut app);
    let alias = app.variables["Alias"].clone();
    app.inspect_variable("A".into());
    click(&mut app, &ctx, "11", egui::Modifiers::NONE);
    let (send, receive) = std::sync::mpsc::sync_channel(4);
    let original = std::mem::replace(&mut app.worker.send, send);
    frame(
        &mut app,
        &ctx,
        vec![egui::Event::Paste("51\t52\n61\t62".into())],
        egui::Modifiers::NONE,
        true,
    );
    let job = receive.try_recv().expect("Exactly one final-pass request");
    assert!(matches!(&job.job, Job::RootedVariable(_)));
    assert!(receive.try_recv().is_err());
    app.worker.send = original;
    app.worker.send.send(job).unwrap();
    finish(&mut app);
    assert_eq!(
        app.variables["A"].as_numeric().unwrap().data(),
        &[51., 61., 52., 62.]
    );
    assert!(app.variables["Alias"].same_snapshot(&alias));
    click(&mut app, &ctx, "Copy selection", egui::Modifiers::NONE);
    // Close and explicitly reopen before receiving the worker's result.
    app.editing_variable = None;
    frame(&mut app, &ctx, vec![], egui::Modifiers::NONE, false);
    app.inspect_variable("A".into());
    finish(&mut app);
    assert!(copied(idle(&mut app, &ctx)).is_none());
}

#[test]
fn numeric_grid_is_bounded_and_modal_blocks_real_pointer_and_clipboard_actions() {
    let ctx = egui::Context::default();
    let mut app = Workspace::with_context(ctx.clone(), true);
    app.run("A=ones(10000,16);Alias=A;".into());
    finish(&mut app);
    let original = app.variables["A"].clone();
    app.inspect_variable("A".into());
    let out = idle(&mut app, &ctx);
    assert!(app.numeric_editor.formatted <= 512);
    assert!(out.shapes.len() < 5000);
    app.pending_quit = true;
    app.edit_value = "9".into();
    click(&mut app, &ctx, "Apply cell", egui::Modifiers::NONE);
    frame(
        &mut app,
        &ctx,
        vec![egui::Event::Paste("99".into()), egui::Event::Copy],
        egui::Modifiers::NONE,
        false,
    );
    assert!(!app.busy);
    assert!(original.same_snapshot(&app.variables["A"]));
    assert!(original.same_snapshot(&app.variables["Alias"]));
}

#[test]
fn completed_rebind_refreshes_with_notice_and_late_edit_does_not_retarget_reopened_editor() {
    let ctx = egui::Context::default();
    let mut app = Workspace::with_context(ctx.clone(), true);
    app.run("A=[11 12;21 22];Alias=A;B=77;".into());
    finish(&mut app);
    let original = app.variables["A"].clone();
    app.inspect_variable("A".into());
    idle(&mut app, &ctx);
    app.edit_value = "999".into();
    app.run("clear A;A=Alias;".into());
    finish(&mut app);
    assert!(app.variables["A"].same_snapshot(&original));
    let out = idle(&mut app, &ctx);
    assert!(out.shapes.iter().any(|s|find(&s.shape,s.clip_rect,
        "Workspace operation completed; selection and edit text were refreshed. Review the value before applying.").is_some()));
    assert_eq!(
        app.edit_value,
        matrust::variable_edit::scalar_literal(original.scalar(0, 0).unwrap())
    );
    click(&mut app, &ctx, "22", egui::Modifiers::NONE);
    app.edit_value = "88".into();
    click(&mut app, &ctx, "Apply cell", egui::Modifiers::NONE);
    app.inspect_variable("B".into());
    finish(&mut app);
    idle(&mut app, &ctx);
    assert_eq!(
        app.variables["A"].scalar(1, 1).unwrap(),
        matrust::Scalar::Real(88.)
    );
    assert_eq!(app.editing_variable.as_deref(), Some("B"));
    assert_eq!((app.edit_row, app.edit_col), (1, 1));
    assert_eq!(
        app.edit_value,
        matrust::variable_edit::scalar_literal(matrust::Scalar::Real(77.))
    );
    assert!(app.variables["Alias"].same_snapshot(&original));
}

fn key(key: egui::Key, modifiers: egui::Modifiers) -> egui::Event {
    egui::Event::Key {
        key,
        physical_key: None,
        pressed: true,
        repeat: false,
        modifiers,
    }
}

#[test]
fn ordered_paste_and_navigation_capture_original_coordinates_once_across_discard() {
    for discard in [false, true] {
        for paste_first in [false, true] {
            let ctx = egui::Context::default();
            let mut app = Workspace::with_context(ctx.clone(), true);
            app.run("A=[11 12;21 22;31 32];Alias=A;".into());
            finish(&mut app);
            let original = app.variables["A"].clone();
            app.inspect_variable("A".into());
            click(&mut app, &ctx, "11", egui::Modifiers::NONE);
            let paste = egui::Event::Paste("99".into());
            let arrow = key(egui::Key::ArrowDown, egui::Modifiers::NONE);
            let events = if paste_first {
                vec![paste, arrow]
            } else {
                vec![arrow, paste]
            };
            let (send, receive) = std::sync::mpsc::sync_channel(4);
            let worker_send = std::mem::replace(&mut app.worker.send, send);
            frame(&mut app, &ctx, events, egui::Modifiers::NONE, discard);
            assert_eq!(
                (app.edit_row, app.edit_col),
                (2, 1),
                "Navigation must not replay on discarded passes"
            );
            let job = receive.try_recv().expect("One ordered request");
            assert!(receive.try_recv().is_err());
            let Job::RootedVariable(request) = &job.job else {
                panic!("Wrong worker route")
            };
            assert_eq!(request.rectangle.row(), usize::from(!paste_first));
            assert_eq!(request.rectangle.col(), 0);
            assert!(matches!(&request.action,variable_edit_job::Action::Paste(text) if text=="99"));
            app.worker.send = worker_send;
            app.worker.send.send(job).unwrap();
            finish(&mut app);
            assert_eq!(
                (app.edit_row, app.edit_col),
                (2, 1),
                "Worker adoption must preserve independent cursor movement"
            );
            for row in 0..3 {
                assert_eq!(
                    app.variables["A"].scalar(row, 0).unwrap(),
                    if row == usize::from(!paste_first) {
                        matrust::Scalar::Real(99.)
                    } else {
                        original.scalar(row, 0).unwrap()
                    }
                );
            }
            assert!(app.variables["Alias"].same_snapshot(&original));
        }
    }
}

#[test]
fn first_submit_wins_and_escape_cancels_intents_without_consuming_text_field_escape() {
    let ctx = egui::Context::default();
    let mut app = Workspace::with_context(ctx.clone(), true);
    app.run("A=[11 12;21 22];Alias=A;".into());
    finish(&mut app);
    let original = app.variables["A"].clone();
    app.inspect_variable("A".into());
    click(&mut app, &ctx, "11", egui::Modifiers::NONE);
    let (send, receive) = std::sync::mpsc::sync_channel(4);
    let worker_send = std::mem::replace(&mut app.worker.send, send);
    frame(
        &mut app,
        &ctx,
        vec![
            egui::Event::Paste("51".into()),
            key(egui::Key::ArrowDown, egui::Modifiers::NONE),
            egui::Event::Paste("61".into()),
        ],
        egui::Modifiers::NONE,
        true,
    );
    let job = receive.try_recv().unwrap();
    assert!(receive.try_recv().is_err());
    let Job::RootedVariable(request) = &job.job else {
        panic!()
    };
    assert_eq!(request.rectangle.row(), 0);
    assert!(matches!(&request.action,variable_edit_job::Action::Paste(text) if text=="51"));
    app.worker.send = worker_send;
    app.worker.send.send(job).unwrap();
    finish(&mut app);
    assert_eq!(
        app.variables["A"].scalar(0, 0).unwrap(),
        matrust::Scalar::Real(51.)
    );
    assert_eq!(
        app.variables["A"].scalar(1, 0).unwrap(),
        matrust::Scalar::Real(21.)
    );
    click(&mut app, &ctx, "21", egui::Modifiers::NONE);
    let grid_focus = ctx
        .memory(|memory| memory.focused())
        .expect("Real grid click owns focus");
    app.edit_value = "999".into();
    frame(
        &mut app,
        &ctx,
        vec![
            egui::Event::Paste("77".into()),
            egui::Event::Copy,
            key(egui::Key::Escape, egui::Modifiers::NONE),
        ],
        egui::Modifiers::NONE,
        true,
    );
    assert!(!app.busy);
    assert_eq!(
        app.edit_value,
        matrust::variable_edit::scalar_literal(matrust::Scalar::Real(21.))
    );
    assert_eq!(
        app.variables["A"].scalar(1, 0).unwrap(),
        matrust::Scalar::Real(21.)
    );
    assert!(copied(idle(&mut app, &ctx)).is_none());
    assert!(app.variables["Alias"].same_snapshot(&original));
    // A genuine literal field click owns Escape; the grid must not clear its draft.
    let pos = app.numeric_editor.field_rect.unwrap().center();
    for pressed in [true, false] {
        frame(
            &mut app,
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
            egui::Modifiers::NONE,
            false,
        );
    }
    assert!(ctx.memory(|memory| memory.focused().is_some() && !memory.has_focus(grid_focus)));
    app.edit_value = "123".into();
    frame(
        &mut app,
        &ctx,
        vec![key(egui::Key::Escape, egui::Modifiers::NONE)],
        egui::Modifiers::NONE,
        false,
    );
    assert_eq!(app.edit_value, "123");
    assert!(!app.busy);
}

#[test]
fn focused_grid_escape_discards_pending_clipboard_and_modified_arrows_do_not_navigate() {
    let ctx = egui::Context::default();
    let mut app = Workspace::with_context(ctx.clone(), true);
    app.run("A=[11 12;21 22];Alias=A;".into());
    finish(&mut app);
    let original = app.variables["A"].clone();
    app.inspect_variable("A".into());
    click(&mut app, &ctx, "11", egui::Modifiers::NONE);
    for modifiers in [
        egui::Modifiers::ALT,
        egui::Modifiers::CTRL,
        egui::Modifiers::MAC_CMD,
    ] {
        frame(
            &mut app,
            &ctx,
            vec![key(egui::Key::ArrowDown, modifiers)],
            modifiers,
            true,
        );
        assert_eq!((app.edit_row, app.edit_col), (1, 1));
    }
    // Escape arrives while the real worker request is held in its actual channel.
    click(&mut app, &ctx, "11", egui::Modifiers::NONE);
    let (send, receive) = std::sync::mpsc::sync_channel(4);
    let worker_send = std::mem::replace(&mut app.worker.send, send);
    frame(
        &mut app,
        &ctx,
        vec![egui::Event::Copy],
        egui::Modifiers::NONE,
        false,
    );
    let job = receive.try_recv().unwrap();
    assert!(app.busy);
    assert!(receive.try_recv().is_err());
    frame(
        &mut app,
        &ctx,
        vec![key(egui::Key::Escape, egui::Modifiers::NONE)],
        egui::Modifiers::NONE,
        true,
    );
    app.worker.send = worker_send;
    app.worker.send.send(job).unwrap();
    finish(&mut app);
    assert!(copied(idle(&mut app, &ctx)).is_none());
    assert!(original.same_snapshot(&app.variables["A"]));
    assert!(original.same_snapshot(&app.variables["Alias"]));
}

fn tall_frame(
    app: &mut Workspace,
    ctx: &egui::Context,
    size: egui::Vec2,
    events: Vec<egui::Event>,
    discard: bool,
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
                ctx.request_discard("Tall grid replay");
            }
            let root = app.variables["A"].clone();
            egui::Window::new("Tall numeric editor")
                .fixed_pos([20., 20.])
                .fixed_size([1000., size.y - 80.])
                .show(ctx, |ui| {
                    app.numeric_variable_ui(ui, "A", &root, &root, vec![], true);
                });
        },
    )
}

fn assert_complete_visible_grid(app: &Workspace, output: &egui::FullOutput) {
    let clip = app.numeric_editor.grid_clip.unwrap();
    let cells = &app.numeric_editor.cells;
    let rows: std::collections::BTreeSet<_> = cells
        .iter()
        .filter(|(_, rect)| clip.contains_rect(**rect))
        .map(|((row, _), _)| *row)
        .collect();
    assert!(rows.len() > 2, "Grid must display actual complete rows");
    for pair in rows.iter().zip(rows.iter().skip(1)) {
        assert_eq!(*pair.1, *pair.0 + 1, "No permanently blank virtual rows");
    }
    let first = *rows.first().unwrap();
    let last = *rows.last().unwrap();
    let stride = cells[&(first + 1, 0)].top() - cells[&(first, 0)].top();
    assert!(cells[&(first, 0)].top() - clip.top() < stride + 1.);
    assert!(clip.bottom() - cells[&(last, 0)].bottom() < stride + 1.);
    for row in rows {
        for col in 0..8 {
            let rect = cells[&(row, col)];
            assert!(clip.contains_rect(rect));
            let label = format_scalar_cell(app.variables["A"].scalar(row, col).unwrap());
            assert!(
                output.shapes.iter().any(|shape| find(
                    &shape.shape,
                    shape.clip_rect.intersect(rect),
                    &label
                )
                .is_some()),
                "Missing actual visible cell ({row},{col})"
            );
        }
    }
    assert!(app.numeric_editor.formatted <= 512);
}

#[test]
fn tall_resized_grid_paints_every_visible_cell_and_reaches_last_row_with_bounded_formats() {
    let ctx = egui::Context::default();
    let mut app = Workspace::with_context(ctx.clone(), true);
    app.run("A=reshape(1:80000,10000,8);Alias=A;".into());
    finish(&mut app);
    let original = app.variables["A"].clone();
    app.inspect_variable("A".into());
    for height in [2160., 900., 3000.] {
        let size = egui::vec2(1440., height);
        for _ in 0..3 {
            tall_frame(&mut app, &ctx, size, vec![], false);
        }
        let output = tall_frame(&mut app, &ctx, size, vec![], true);
        assert_complete_visible_grid(&app, &output);
        let clip = app.numeric_editor.grid_clip.unwrap();
        let (&(_, col), rect) = app
            .numeric_editor
            .cells
            .iter()
            .find(|((_, col), r)| *col == 0 && clip.contains_rect(**r))
            .unwrap();
        assert_eq!(col, 0);
        let pos = rect.center();
        for pressed in [true, false] {
            tall_frame(
                &mut app,
                &ctx,
                size,
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
            );
        }
        tall_frame(
            &mut app,
            &ctx,
            size,
            vec![key(egui::Key::End, egui::Modifiers::CTRL)],
            true,
        );
        assert_eq!((app.edit_row, app.edit_col), (10000, 8));
        let output = tall_frame(&mut app, &ctx, size, vec![], false);
        assert_complete_visible_grid(&app, &output);
        assert!(
            app.numeric_editor
                .grid_clip
                .unwrap()
                .contains_rect(app.numeric_editor.cells[&(9999, 7)])
        );
        assert!(original.same_snapshot(&app.variables["A"]));
        assert!(original.same_snapshot(&app.variables["Alias"]));
    }
}

#[test]
fn character_nested_grid_uses_real_selection_paste_worker_and_exact_unit_clipboard() {
    for theme in [egui::Theme::Light, egui::Theme::Dark] {
        let ctx = egui::Context::default();
        let mut app = Workspace::with_context(ctx.clone(), true);
        ctx.set_theme(theme);
        app.run("S=char([65 55296;0 65535]);C={S,S};Alias=C;".into());
        finish(&mut app);
        let original = app.variables["C"].clone();
        let leaf = app.variables["S"].clone();
        app.inspect_variable("C".into());
        click(&mut app, &ctx, "Open {1,1}", egui::Modifiers::NONE);
        idle(&mut app, &ctx);
        for (cell, modifiers) in [
            ((0, 0), egui::Modifiers::NONE),
            ((1, 1), egui::Modifiers::SHIFT),
        ] {
            let pos = app.numeric_editor.cells[&cell].center();
            for pressed in [true, false] {
                frame(
                    &mut app,
                    &ctx,
                    vec![
                        egui::Event::PointerMoved(pos),
                        egui::Event::PointerButton {
                            pos,
                            button: egui::PointerButton::Primary,
                            pressed,
                            modifiers,
                        },
                    ],
                    modifiers,
                    false,
                );
            }
        }
        click(&mut app, &ctx, "Copy selection", egui::Modifiers::NONE);
        finish(&mut app);
        assert_eq!(
            copied(frame(&mut app, &ctx, vec![], egui::Modifiers::NONE, false)).as_deref(),
            Some("\\u0041\t\\uD800\n\\u0000\t\\uFFFF")
        );
        assert!(original.same_snapshot(&app.variables["C"]));
        click(&mut app, &ctx, "Paste selection", egui::Modifiers::NONE);
        frame(
            &mut app,
            &ctx,
            vec![egui::Event::Paste("\\uDFFF\t0x0000\n65535\t66".into())],
            egui::Modifiers::NONE,
            true,
        );
        finish(&mut app);
        assert!(app.cell_error.is_none(), "{:?}", app.cell_error);
        let changed = app.variables["C"].clone();
        let Array::Character(text) = child(&changed, 0) else {
            panic!()
        };
        assert_eq!(text.units(), &[0xdfff, 65535, 0, 66]);
        assert!(child(&changed, 1).same_snapshot(&leaf));
        assert!(original.same_snapshot(&app.variables["Alias"]));
        assert!(leaf.same_snapshot(&app.variables["S"]));
        click(&mut app, &ctx, "Paste selection", egui::Modifiers::NONE);
        frame(
            &mut app,
            &ctx,
            vec![egui::Event::Paste("1\t2\n3\t65536".into())],
            egui::Modifiers::NONE,
            false,
        );
        finish(&mut app);
        assert!(app.cell_error.as_ref().is_some_and(|e| e.contains("65535")));
        assert!(changed.same_snapshot(&app.variables["C"]));
    }
}

#[test]
fn copy_text_row_uses_worker_validation_for_valid_invalid_and_empty_rows() {
    let ctx = egui::Context::default();
    let mut app = Workspace::with_context(ctx.clone(), true);
    for (script, expected) in [
        ("S=char([65 55357 56832]);", Some("A😀")),
        ("S=char([65 0 66]);", None),
        ("S=char([55296]);", None),
        ("S=char(zeros(1,0));", Some("")),
    ] {
        app.run(format!("{script}Alias=S;"));
        finish(&mut app);
        let original = app.variables["S"].clone();
        app.inspect_variable("S".into());
        if original.is_empty() {
            let output = idle(&mut app, &ctx);
            assert!(output.shapes.iter().any(|shape| {
                find(
                    &shape.shape,
                    shape.clip_rect,
                    "Empty character array; no editable code units.",
                )
                .is_some()
            }));
            assert!(
                !output
                    .shapes
                    .iter()
                    .any(|shape| { find(&shape.shape, shape.clip_rect, "Empty matrix").is_some() })
            );
            assert!(app.numeric_editor.cells.is_empty());
            assert_eq!(app.numeric_editor.formatted, 0);
        }
        click(&mut app, &ctx, "Copy text row", egui::Modifiers::NONE);
        finish(&mut app);
        let output = copied(frame(&mut app, &ctx, vec![], egui::Modifiers::NONE, false));
        assert_eq!(output.as_deref(), expected);
        if expected.is_none() {
            assert!(app.cell_error.is_some());
        } else {
            assert!(app.cell_error.is_none(), "{:?}", app.cell_error);
        }
        if script == "S=char([65 0 66]);" {
            let error = app.cell_error.as_ref().unwrap();
            assert!(error.contains("NUL") && error.contains("Copy selection"));
        }
        assert!(original.same_snapshot(&app.variables["S"]));
        assert!(original.same_snapshot(&app.variables["Alias"]));
    }
}

#[test]
fn character_worker_rebind_revision_and_stop_reject_before_publication() {
    let worker = Worker::new(egui::Context::default());
    worker
        .send
        .send(Job::Script("A=char([65 55296]);Alias=A;".into()).into())
        .unwrap();
    let first = worker.receive.recv_timeout(Duration::from_secs(5)).unwrap();
    let original = first.vars["A"].clone();
    let target = Target::new("A".into(), original.clone(), vec![]).unwrap();
    worker
        .send
        .send(Job::Script("clear A;A=Alias;".into()).into())
        .unwrap();
    let rebound = worker.receive.recv_timeout(Duration::from_secs(5)).unwrap();
    assert!(rebound.vars["A"].same_snapshot(&original));
    worker
        .send
        .send(
            Job::RootedVariable(Box::new(variable_edit_job::Request {
                revision: first.variable_revision,
                editor: Default::default(),
                target: target.clone(),
                rectangle: Rect::new(0, 0, 1, 1).unwrap(),
                action: variable_edit_job::Action::Paste("not a unit".into()),
            }))
            .into(),
        )
        .unwrap();
    let rejected = worker.receive.recv_timeout(Duration::from_secs(5)).unwrap();
    assert!(
        rejected
            .result
            .as_ref()
            .err()
            .unwrap()
            .contains("Workspace operation completed")
    );
    assert!(rejected.vars["A"].same_snapshot(&original));
    worker.cancel.store(true, Ordering::Relaxed);
    worker
        .send
        .send(
            Job::RootedVariable(Box::new(variable_edit_job::Request {
                revision: rejected.variable_revision,
                editor: Default::default(),
                target,
                rectangle: Rect::new(0, 0, 1, 1).unwrap(),
                action: variable_edit_job::Action::Literal("\\u0042".into()),
            }))
            .into(),
        )
        .unwrap();
    let stopped = worker.receive.recv_timeout(Duration::from_secs(5)).unwrap();
    assert!(stopped.result.as_ref().err().unwrap().contains("cancelled"));
    assert!(stopped.vars["A"].same_snapshot(&original));
    assert!(stopped.vars["Alias"].same_snapshot(&original));
}

fn edit_error_text_color(
    shape: &egui::Shape,
    clip: egui::Rect,
    label: &str,
    expected: Color32,
) -> usize {
    match shape {
        egui::Shape::Text(text) if text.galley.text() == label => {
            assert!(
                clip.contains_rect(text.visual_bounding_rect()),
                "Error text must be visible"
            );
            assert_eq!(text.opacity_factor, 1.);
            assert!(
                text.override_text_color
                    .is_none_or(|color| color == expected)
            );
            assert!(!text.galley.job.sections.is_empty());
            assert!(
                text.galley
                    .job
                    .sections
                    .iter()
                    .all(|s| s.format.color == expected)
            );
            1
        }
        egui::Shape::Vec(shapes) => shapes
            .iter()
            .map(|s| edit_error_text_color(s, clip, label, expected))
            .sum(),
        _ => 0,
    }
}

#[test]
fn character_keyboard_error_is_visible_in_both_themes_and_keeps_grid_focus() {
    for theme in [egui::Theme::Light, egui::Theme::Dark] {
        let ctx = egui::Context::default();
        let mut app = Workspace::with_context(ctx.clone(), true);
        ctx.set_theme(theme);
        app.run("S=char([65 66 67;68 69 70]);Alias=S;".into());
        finish(&mut app);
        let original = app.variables["S"].clone();
        app.inspect_variable("S".into());
        idle(&mut app, &ctx);
        let pos = app.numeric_editor.cells[&(0, 0)].center();
        for pressed in [true, false] {
            frame(
                &mut app,
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
                egui::Modifiers::NONE,
                false,
            );
        }
        frame(
            &mut app,
            &ctx,
            vec![egui::Event::Paste("65536".into())],
            egui::Modifiers::NONE,
            true,
        );
        assert!(app.busy, "Actual focused-grid paste must reach the worker");
        finish(&mut app);
        let error = app
            .cell_error
            .clone()
            .expect("Invalid UTF16 unit must reject");
        assert!(error.contains("65535"));
        let expected = if theme == egui::Theme::Dark {
            crate::tokens::DARK.error
        } else {
            crate::tokens::LIGHT.error
        };
        if theme == egui::Theme::Dark {
            assert_eq!(ctx.global_style().visuals.error_fg_color, expected);
        }
        let output = idle(&mut app, &ctx);
        assert_eq!(
            output
                .shapes
                .iter()
                .map(|s| edit_error_text_color(&s.shape, s.clip_rect, &error, expected))
                .sum::<usize>(),
            1
        );
        assert!(original.same_snapshot(&app.variables["S"]));
        assert!(original.same_snapshot(&app.variables["Alias"]));
        // No new click or injected focus: the same grid owns subsequent Copy.
        frame(
            &mut app,
            &ctx,
            vec![egui::Event::Copy],
            egui::Modifiers::NONE,
            false,
        );
        assert!(app.busy);
        finish(&mut app);
        assert_eq!(
            copied(frame(&mut app, &ctx, vec![], egui::Modifiers::NONE, false)).as_deref(),
            Some("\\u0041")
        );
        assert!(original.same_snapshot(&app.variables["S"]));
        assert!(original.same_snapshot(&app.variables["Alias"]));
    }
}
