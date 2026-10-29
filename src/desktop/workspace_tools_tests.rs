use matrust::Array;
#[test]
fn workspace_metadata_rename_moves_real_and_logical_storage_without_copying() {
    use workspace_edit::{Action, Request};
    let mut engine = matrust::Engine::default();
    engine
        .execute("source=[1 2 3];alias=source;mask=logical([0 1]);plot(source);")
        .unwrap();
    let original = engine.array("source").unwrap();
    let mask = engine.array("mask").unwrap();
    let figure = engine.figure.clone().unwrap();
    for (from, to) in [("source", "renamed"), ("mask", "flag")] {
        workspace_edit::perform(
            &mut engine,
            Request {
                targets: BTreeMap::from([(from.into(), engine_value(from, &original, &mask))]),
                action: Action::Rename(to.into()),
            },
        )
        .unwrap();
    }
    assert!(original.same_snapshot(&engine.array("renamed").unwrap()));
    assert!(original.same_snapshot(&engine.array("alias").unwrap()));
    assert!(mask.same_snapshot(&engine.array("flag").unwrap()));
    assert!(engine.array("source").is_none() && engine.array("mask").is_none());
    assert!(figure.same_snapshot(engine.figure.as_ref().unwrap()));
    fn engine_value(name: &str, original: &Array, mask: &Array) -> Array {
        if name == "source" {
            original.clone()
        } else {
            mask.clone()
        }
    }
}

#[test]
fn workspace_metadata_rejects_collisions_keywords_stale_snapshots_and_cancel_atomically() {
    use workspace_edit::{Action, Request};
    let mut engine = matrust::Engine::default();
    engine.execute("a=[1 2];b=logical([1 0]);").unwrap();
    let a = engine.array("a").unwrap();
    let before = engine.workspace();
    for name in ["b", "if", "1bad", "bad name", "", "while", "a"] {
        assert!(
            workspace_edit::perform(
                &mut engine,
                Request {
                    targets: BTreeMap::from([("a".into(), a.clone())]),
                    action: Action::Rename(name.into())
                }
            )
            .is_err()
        );
    }
    engine.execute("b=logical([0 0]);").unwrap();
    assert!(
        workspace_edit::perform(
            &mut engine,
            Request {
                targets: before,
                action: Action::Remove { all: true }
            }
        )
        .is_err()
    );
    assert!(a.same_snapshot(&engine.array("a").unwrap()));
    let older = engine.workspace();
    engine.execute("new_variable=7;").unwrap();
    assert!(
        workspace_edit::perform(
            &mut engine,
            Request {
                targets: older,
                action: Action::Remove { all: true }
            }
        )
        .is_err()
    );
    let targets = engine.workspace();
    engine.cancellation().store(true, Ordering::Relaxed);
    assert!(
        workspace_edit::perform(
            &mut engine,
            Request {
                targets,
                action: Action::Remove { all: true }
            }
        )
        .is_err()
    );
    assert_eq!(engine.workspace().len(), 3);
}

#[test]
fn workspace_rename_ui_updates_selection_and_open_inspector() {
    let ctx = egui::Context::default();
    let mut app = legacy_sample_desktop(ctx.clone(), true);
    app.run("source=[1 2 3];plot(source);".into());
    finish(&mut app);
    let original = app.variables["source"].clone();
    let figure = app.figure.clone().unwrap();
    app.inspect_variable("source".into());
    let output = frame(&mut app, &ctx, vec![]);
    let menu = output
        .shapes
        .iter()
        .filter_map(|s| text_position(&s.shape, "Menu"))
        .find(|p| p.y < 80.)
        .unwrap();
    assert!(!ctx.memory(|m| m.area_rect(egui::Id::new("variable_editor"))).unwrap().contains(menu),
        "The initial inspector must not cover the top-bar menu");
    for pressed in [true, false] {
        pointer_frame(&mut app, &ctx, egui::vec2(1440., 900.), menu, pressed);
    }
    click_label(&mut app, &ctx, "Workspace");
    click_label(&mut app, &ctx, "Rename variable...");
    script_action_key(
        &mut app,
        &ctx,
        egui::Modifiers::CTRL | egui::Modifiers::COMMAND,
        egui::Key::A,
    );
    frame(&mut app, &ctx, vec![egui::Event::Text("renamed".into())]);
    click_label(&mut app, &ctx, "Rename");
    finish(&mut app);
    assert!(app.workspace_tools.dialog.is_none(), "{}", app.status);
    assert_eq!(app.selected.as_deref(), Some("renamed"));
    assert_eq!(app.editing_variable.as_deref(), Some("renamed"));
    assert!(original.same_snapshot(&app.variables["renamed"]));
    assert!(figure.same_snapshot(app.figure.as_ref().unwrap()));
    assert!(app.recovery_dirty);
}

#[test]
fn workspace_multiselection_delete_confirmation_and_clear_keep_figures_and_scripts() {
    let ctx = egui::Context::default();
    let mut app = legacy_sample_desktop(ctx.clone(), true);
    app.run("a=[1 2];b=logical([1 0]);c=9;plot(a);".into());
    finish(&mut app);
    let figure = app.figure.clone().unwrap();
    let script = app.documents[app.active].text.clone();
    let names = ["a", "b", "c"];
    app.workspace_tools
        .select("a", &names, egui::Modifiers::NONE);
    app.workspace_tools
        .select("c", &names, egui::Modifiers::CTRL);
    assert_eq!(app.workspace_targets(false).len(), 2);
    app.begin_workspace_action(false, false);
    app.request_quit(&ctx);
    assert!(!app.pending_quit && app.figure_close.is_none());
    click_label(&mut app, &ctx, "Cancel");
    assert_eq!(app.variables.len(), 3);
    app.workspace_tools
        .select("b", &names, egui::Modifiers::SHIFT);
    assert_eq!(
        app.workspace_targets(false)
            .keys()
            .cloned()
            .collect::<Vec<_>>(),
        ["b", "c"]
    );
    app.begin_workspace_action(false, false);
    click_label(&mut app, &ctx, "Delete variables");
    finish(&mut app);
    assert_eq!(app.variables.keys().cloned().collect::<Vec<_>>(), ["a"]);
    app.begin_workspace_action(false, true);
    click_label(&mut app, &ctx, "Delete variables");
    finish(&mut app);
    assert!(app.variables.is_empty());
    assert!(figure.same_snapshot(app.figure.as_ref().unwrap()));
    assert_eq!(app.documents[app.active].text, script);
}

#[test]
fn workspace_filter_virtualizes_ten_thousand_rows_and_keeps_sidebar_stable() {
    fn panel(ctx: &egui::Context, name: &str) -> egui::Rect {
        egui::containers::panel::PanelState::load(ctx, egui::Id::new(name))
            .unwrap()
            .outer_rect
    }
    let ctx = egui::Context::default();
    let mut app = legacy_sample_desktop(ctx.clone(), true);
    // Keep this existing width-stability regression on the supported right dock.
    app.set_workspace_dock(&ctx, session::WorkspaceDock::Right);
    let shared = Array::from(matrust::Matrix::scalar(1.));
    app.variables = (0..10000)
        .map(|i| (format!("variable{i:05}"), shared.clone()))
        .collect();
    let first = frame(&mut app, &ctx, vec![]);
    assert!(first.shapes.len() < 1500, "{} shapes", first.shapes.len());
    let width = panel(&ctx, "workspace_variables").width();
    for _ in 0..6 {
        frame(&mut app, &ctx, vec![]);
    }
    assert!((panel(&ctx, "workspace_variables").width() - width).abs() < 1.);
    app.workspace_tools.filter = "VARIABLE09999".into();
    let output = frame(&mut app, &ctx, vec![]);
    assert!(
        output
            .shapes
            .iter()
            .any(|s| text_position(&s.shape, "1 of 10000 variables").is_some())
    );
    assert_eq!(app.variables.len(), 10000);
}

#[test]
fn workspace_mouse_and_keyboard_selection_require_delete_confirmation() {
    fn click_row(app: &mut Workspace, ctx: &egui::Context, name: &str, modifiers: egui::Modifiers) {
        let mut position = None;
        for _ in 0..3 {
            let output = frame(app, ctx, vec![]);
            position = output
                .shapes
                .iter()
                .find_map(|s| text_position(&s.shape, name));
        }
        let position = position.unwrap();
        for pressed in [true, false] {
            let _ = crate::headless::run(ctx,
                egui::RawInput {
                    screen_rect: Some(egui::Rect::from_min_size(
                        egui::Pos2::ZERO,
                        egui::vec2(1440., 900.),
                    )),
                    events: vec![egui::Event::ModifiersChanged(modifiers),
                        egui::Event::PointerMoved(position),
                        egui::Event::PointerButton {
                            pos: position,
                            button: egui::PointerButton::Primary,
                            pressed,
                            modifiers,
                        },
                    ],
                    ..Default::default()
                },
                |root| app.ui(root),
            );
        }
    }
    let ctx = egui::Context::default();
    let mut app = legacy_sample_desktop(ctx.clone(), true);
    app.run("a=1;b=2;c=3;".into());
    finish(&mut app);
    click_row(&mut app, &ctx, "a", egui::Modifiers::NONE);
    click_row(&mut app, &ctx, "c", egui::Modifiers::CTRL);
    assert_eq!(app.workspace_selection_count(), 2);
    script_action_key(&mut app, &ctx, egui::Modifiers::NONE, egui::Key::Delete);
    assert!(app.workspace_tools.dialog.is_some());
    assert_eq!(app.variables.len(), 3);
    click_label(&mut app, &ctx, "Cancel");
    click_row(&mut app, &ctx, "a", egui::Modifiers::NONE);
    script_action_key(&mut app, &ctx, egui::Modifiers::NONE, egui::Key::F2);
    assert!(app.workspace_tools.dialog.is_some());
    click_label(&mut app, &ctx, "Cancel");
    click_row(&mut app, &ctx, "a", egui::Modifiers::NONE);
    script_action_key(&mut app, &ctx, egui::Modifiers::CTRL, egui::Key::A);
    assert_eq!(app.workspace_selection_count(), 3);
    assert_eq!(app.variables.len(), 3);
}

#[test]
fn workspace_long_names_stay_within_compact_sidebar() {
    let ctx = egui::Context::default();
    let mut app = legacy_sample_desktop(ctx.clone(), true);
    let name = "long_variable_name_".repeat(3);
    app.variables
        .insert(name, Array::from(matrust::Matrix::scalar(1.)));
    let mut widths = Vec::new();
    for _ in 0..8 {
        frame_at(&mut app, &ctx, egui::vec2(960., 650.), vec![]);
        widths.push(
            egui::containers::panel::PanelState::load(&ctx, egui::Id::new("workspace"))
                .unwrap()
                .outer_rect
                .width(),
        );
    }
    let maximum = 240_f32.max(workspace_tools::minimum_dock_width(&ctx));
    assert!(widths.iter().all(|width| *width <= maximum));
    assert!((widths[2] - widths[7]).abs() < 1.);
}

#[test]
fn workspace_keyboard_navigation_scrolls_virtual_rows_and_extends_selection() {
    let ctx = egui::Context::default();
    let mut app = legacy_sample_desktop(ctx.clone(), true);
    let shared = Array::from(matrust::Matrix::scalar(1.));
    app.variables = (0..10000)
        .map(|i| (format!("v{i:05}"), shared.clone()))
        .collect();
    click_label(&mut app, &ctx, "v00000");
    frame(&mut app, &ctx, vec![]);
    script_action_key(&mut app, &ctx, egui::Modifiers::NONE, egui::Key::End);
    assert_eq!(app.selected.as_deref(), Some("v09999"));
    let output = frame(&mut app, &ctx, vec![]);
    assert!(output.shapes.len() < 1500);
    assert!(
        output
            .shapes
            .iter()
            .any(|s| text_position(&s.shape, "v09999").is_some())
    );
    script_action_key(&mut app, &ctx, egui::Modifiers::SHIFT, egui::Key::ArrowUp);
    assert_eq!(app.selected.as_deref(), Some("v09998"));
    assert_eq!(app.workspace_selection_count(), 2);
    script_action_key(&mut app, &ctx, egui::Modifiers::SHIFT, egui::Key::ArrowUp);
    assert_eq!(app.workspace_selection_count(), 3);
    script_action_key(&mut app, &ctx, egui::Modifiers::SHIFT, egui::Key::ArrowDown);
    assert_eq!(app.workspace_selection_count(), 2);
    script_action_key(&mut app, &ctx, egui::Modifiers::NONE, egui::Key::Home);
    assert_eq!(app.selected.as_deref(), Some("v00000"));
    script_action_key(&mut app, &ctx, egui::Modifiers::NONE, egui::Key::PageDown);
    let page = app.selected.clone().unwrap();
    assert!(
        // One page: the taller Workspace card shows more than ten rows.
        page.as_str() > "v00000" && page.as_str() < "v00030",
        "{page}"
    );
    script_action_key(&mut app, &ctx, egui::Modifiers::NONE, egui::Key::PageUp);
    assert_eq!(app.selected.as_deref(), Some("v00000"));
    script_action_key(&mut app, &ctx, egui::Modifiers::NONE, egui::Key::ArrowUp);
    assert_eq!(app.selected.as_deref(), Some("v00000"));
    assert!(shared.same_snapshot(&app.variables["v09999"]));
}

#[test]
fn workspace_keyboard_cursor_can_move_without_selection_then_inspect() {
    let ctx = egui::Context::default();
    let mut app = legacy_sample_desktop(ctx.clone(), true);
    app.run("a=1;b=2;c=3;".into());
    finish(&mut app);
    click_label(&mut app, &ctx, "a");
    frame(&mut app, &ctx, vec![]);
    script_action_key(&mut app, &ctx, egui::Modifiers::CTRL, egui::Key::ArrowDown);
    assert_eq!(app.selected.as_deref(), Some("a"));
    assert_eq!(app.workspace_selection_count(), 1);
    script_action_key(&mut app, &ctx, egui::Modifiers::NONE, egui::Key::Enter);
    assert_eq!(app.editing_variable.as_deref(), Some("b"));
    assert_eq!(app.selected.as_deref(), Some("b"));
    assert_eq!(app.variables.len(), 3);
}

#[test]
fn workspace_keyboard_navigation_respects_filter_and_other_focus() {
    let ctx = egui::Context::default();
    let mut app = legacy_sample_desktop(ctx.clone(), true);
    app.run("alpha=1;beta=2;gamma=3;".into());
    finish(&mut app);
    app.workspace_tools.filter = "a".into();
    click_label(&mut app, &ctx, "alpha");
    frame(&mut app, &ctx, vec![]);
    app.workspace_tools.filter = "alpha".into();
    script_action_key(&mut app, &ctx, egui::Modifiers::NONE, egui::Key::End);
    assert_eq!(app.selected.as_deref(), Some("alpha"));
    let output = frame(&mut app, &ctx, vec![]);
    assert!(
        output
            .shapes
            .iter()
            .any(|s| text_position(&s.shape, "1 of 3 variables").is_some())
    );
    ctx.memory_mut(|m| m.request_focus(egui::Id::new("other_control")));
    script_action_key(&mut app, &ctx, egui::Modifiers::NONE, egui::Key::Enter);
    assert!(app.editing_variable.is_none());
    app.workspace_tools.filter = "no_match".into();
    script_action_key(&mut app, &ctx, egui::Modifiers::NONE, egui::Key::End);
    assert_eq!(app.variables.len(), 3);
    assert_eq!(app.selected.as_deref(), Some("alpha"));
}

#[test]
fn workspace_empty_filter_navigation_preserves_selection_after_cursor_move() {
    let ctx = egui::Context::default();
    let mut app = legacy_sample_desktop(ctx.clone(), true);
    app.run("a=1;b=2;c=3;".into());
    finish(&mut app);
    click_label(&mut app, &ctx, "a");
    frame(&mut app, &ctx, vec![]);
    script_action_key(&mut app, &ctx, egui::Modifiers::CTRL, egui::Key::ArrowDown);
    assert_eq!(app.selected.as_deref(), Some("a"));
    app.workspace_tools.filter = "no_match".into();
    script_action_key(&mut app, &ctx, egui::Modifiers::NONE, egui::Key::End);
    assert_eq!(app.selected.as_deref(), Some("a"));
    assert_eq!(
        app.workspace_targets(false)
            .keys()
            .cloned()
            .collect::<Vec<_>>(),
        ["a"]
    );
    assert_eq!(app.variables.len(), 3);
}


#[test]
fn workspace_rename_invalid_keyboard_text_paints_readable_status_without_mutation() {
    for selected_theme in [matrust::session::DesktopTheme::Light, matrust::session::DesktopTheme::Dark] {
        let ctx = egui::Context::default();
        let mut app = legacy_sample_desktop(ctx.clone(), true);
        app.layout.theme = Some(selected_theme);
        app.apply_desktop_theme(&ctx);
        app.run("source=[1 2];alias=source;".into());
        finish(&mut app);
        let original = app.variables["source"].clone();
        app.selected = Some("source".into());
        app.begin_workspace_action(true, false);
        for _ in 0..3 { frame(&mut app, &ctx, vec![]); }
        script_action_key(&mut app, &ctx, egui::Modifiers::CTRL | egui::Modifiers::COMMAND, egui::Key::A);
        frame(&mut app, &ctx, vec![egui::Event::Text("1bad".into())]);
        click_label(&mut app, &ctx, "Rename");
        assert!(!app.busy, "Invalid name must reject before submitting a mutation");
        let error = app.workspace_tools.dialog.as_ref().unwrap().error.clone().expect("Actual validation error");
        let output = frame(&mut app, &ctx, vec![]);
        let expected = if selected_theme == matrust::session::DesktopTheme::Dark { crate::tokens::DARK.error } else { crate::tokens::LIGHT.error };
        crate::theme::assert_painted_status(&output, &error, expected);
        assert!(original.same_snapshot(&app.variables["source"]));
        assert!(original.same_snapshot(&app.variables["alias"]));
        assert!(!app.variables.contains_key("1bad"));
    }
}

// Real footer gestures use an intercepted picker only; worker CSV publication is real.
fn csv_footer_position(app: &mut Workspace, ctx: &egui::Context) -> egui::Pos2 {
    // The variable actions live in the Workspace card's overflow menu.
    click_label(app, ctx, "Workspace actions");
    let mut output = frame(app, ctx, vec![]);
    for _ in 0..2 { output = frame(app, ctx, vec![]); }
    output.shapes.iter().rev()
        .find_map(|shape| text_position(&shape.shape, "Export variable CSV…"))
        .expect("The Workspace actions menu must list the CSV export")
}

fn csv_footer_release(
    app: &mut Workspace,
    ctx: &egui::Context,
    position: egui::Pos2,
    mut between_passes: impl FnMut(&mut Workspace, &egui::Context),
) {
    let mut passes = 0;
    crate::headless::run(ctx, egui::RawInput {
        screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(1440., 900.))),
        events: vec![egui::Event::PointerMoved(position), egui::Event::PointerButton {
            pos: position, button: egui::PointerButton::Primary, pressed: false,
            modifiers: egui::Modifiers::NONE,
        }],
        ..Default::default()
    }, |root| {
        passes += 1;
        if passes == 1 { root.ctx().request_discard("CSV picker waits for the final pass"); }
        app.ui(root);
        if passes == 1 {
            assert_eq!(app.workspace_tools.csv_picker_calls, 0);
            assert!(!app.busy, "The discarded release must not enqueue a worker job");
            between_passes(app, ctx);
        }
    });
    assert!(passes >= 2, "Exercise an actual discarded release and final retry");
}

#[test]
fn workspace_csv_footer_discard_exports_real_and_logical_with_shared_aliases() {
    for dock in [matrust::session::WorkspaceDock::BelowFiles, matrust::session::WorkspaceDock::Right] {
        for (script, expected) in [
            ("source=[1 2;3 4];alias=source;", "1,2\n3,4\n"),
            ("source=logical([1 0;0 1]);alias=source;", "1,0\n0,1\n"),
        ] {
            let ctx = egui::Context::default();
            let mut app = legacy_sample_desktop(ctx.clone(), true);
            let dir = tempfile::tempdir().unwrap();
            app.folder = dir.path().to_path_buf();
            app.run(script.into()); finish(&mut app);
            app.set_workspace_dock(&ctx, dock);
            app.selected = Some("source".into());
            let original = app.variables["source"].clone();
            app.workspace_tools.csv_picker = Some(|app| Some(app.folder.join("selected.csv")));
            let position = csv_footer_position(&mut app, &ctx);
            pointer_frame(&mut app, &ctx, egui::vec2(1440., 900.), position, true);
            csv_footer_release(&mut app, &ctx, position, |_, _| {});
            assert_eq!(app.workspace_tools.csv_picker_calls, 1);
            assert!(app.busy, "Final release must dispatch the actual CSV worker job");
            finish(&mut app);
            assert_eq!(std::fs::read_to_string(dir.path().join("selected.csv")).unwrap(), expected);
            assert!(original.same_snapshot(&app.variables["source"]));
            assert!(original.same_snapshot(&app.variables["alias"]));
            frame(&mut app, &ctx, vec![]);
            assert_eq!(app.workspace_tools.csv_picker_calls, 1);
        }
    }
}

#[test]
fn workspace_csv_cancelled_picker_consumes_the_release_once() {
    for dock in [matrust::session::WorkspaceDock::BelowFiles, matrust::session::WorkspaceDock::Right] {
        let ctx = egui::Context::default();
        let mut app = legacy_sample_desktop(ctx.clone(), true);
        app.run("source=[1 2];alias=source;".into()); finish(&mut app);
        app.set_workspace_dock(&ctx, dock);
        app.selected = Some("source".into());
        let original = app.variables["source"].clone();
        app.workspace_tools.csv_picker = Some(|_| None);
        let position = csv_footer_position(&mut app, &ctx);
        pointer_frame(&mut app, &ctx, egui::vec2(1440., 900.), position, true);
        csv_footer_release(&mut app, &ctx, position, |_, _| {});
        assert_eq!(app.workspace_tools.csv_picker_calls, 1);
        assert!(!app.busy);
        frame(&mut app, &ctx, vec![]);
        assert_eq!(app.workspace_tools.csv_picker_calls, 1);
        assert!(original.same_snapshot(&app.variables["source"]));
        assert!(original.same_snapshot(&app.variables["alias"]));
        // A new real release is a new action, even after a cancelled picker.
        click_label(&mut app, &ctx, "Workspace actions");
        click_label(&mut app, &ctx, "Export variable CSV…");
        assert_eq!(app.workspace_tools.csv_picker_calls, 2);
        assert!(!app.busy);
        // A later layout request may discard a pass after the picker returned.
        let position = csv_footer_position(&mut app, &ctx);
        pointer_frame(&mut app, &ctx, egui::vec2(1440., 900.), position, true);
        let mut passes = 0;
        crate::headless::run(&ctx, egui::RawInput {
            screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(1440., 900.))),
            events: vec![egui::Event::PointerMoved(position), egui::Event::PointerButton {
                pos: position, button: egui::PointerButton::Primary, pressed: false,
                modifiers: egui::Modifiers::NONE,
            }], ..Default::default()
        }, |root| {
            passes += 1;
            app.ui(root);
            if passes == 1 { root.ctx().request_discard("later layout retry after picker Cancel"); }
        });
        assert!(passes >= 2);
        assert_eq!(app.workspace_tools.csv_picker_calls, 3, "The consumed gesture must not reopen");
        assert!(!app.busy);
    }
}

fn invalidate_csv_action(app: &mut Workspace, case: usize) {
    match case {
        0 => app.selected = Some("other".into()),
        1 => { app.variables.insert("source".into(), app.variables["other"].clone()); }
        2 => app.pending_quit = true,
        3 => app.busy = true,
        4 => app.stop_work(),
        5 => app.begin_workspace_action(true, false),
        6 => app.navigation.workspace = false,
        7 => app.folder.push("different"),
        8 => app.allow_close = true,
        9 => app.new_document(""),
        _ => unreachable!(),
    }
}

#[test]
fn workspace_csv_rejects_changed_selection_owner_modal_stop_and_busy_before_picker() {
    for case in 0..11 {
        let ctx = egui::Context::default();
        let mut app = legacy_sample_desktop(ctx.clone(), true);
        app.run("source=[1 2];other=[3 4];alias=source;".into()); finish(&mut app);
        app.selected = Some("source".into());
        let alias = app.variables["alias"].clone();
        app.workspace_tools.csv_picker = Some(|_| panic!("A stale CSV action opened the picker"));
        let position = csv_footer_position(&mut app, &ctx);
        pointer_frame(&mut app, &ctx, egui::vec2(1440., 900.), position, true);
        csv_footer_release(&mut app, &ctx, position, |app, ctx| {
            match case {
                9 => ctx.memory_mut(|memory| memory.request_focus(egui::Id::new("late_csv_foreign_owner"))),
                10 => { app.new_document(""); }
                _ => invalidate_csv_action(app, case),
            }
        });
        assert_eq!(app.workspace_tools.csv_picker_calls, 0, "case {case}");
        assert!(alias.same_snapshot(&app.variables["alias"]));
        // Release the simulated busy flag; no retained intent may replay next frame.
        if case == 3 { app.busy = false; }
        frame(&mut app, &ctx, vec![]);
        assert_eq!(app.workspace_tools.csv_picker_calls, 0, "later frame case {case}");
    }
}

#[test]
fn workspace_csv_revalidates_after_picker_before_name_only_worker_admission() {
    for case in 0..10 {
        let ctx = egui::Context::default();
        let mut app = legacy_sample_desktop(ctx.clone(), true);
        let dir = tempfile::tempdir().unwrap();
        app.folder = dir.path().to_path_buf();
        app.run("source=[1 2];other=[3 4];alias=source;".into()); finish(&mut app);
        app.selected = Some("source".into());
        let alias = app.variables["alias"].clone();
        app.workspace_tools.csv_picker = Some(|app| {
            let path = app.folder.join("must-not-exist.csv");
            let case: usize = app.status.strip_prefix("CSV test case ").unwrap().parse().unwrap();
            invalidate_csv_action(app, case);
            Some(path)
        });
        let position = csv_footer_position(&mut app, &ctx);
        pointer_frame(&mut app, &ctx, egui::vec2(1440., 900.), position, true);
        app.status = format!("CSV test case {case}");
        pointer_frame(&mut app, &ctx, egui::vec2(1440., 900.), position, false);
        assert_eq!(app.workspace_tools.csv_picker_calls, 1, "case {case}");
        assert_eq!(app.busy, case == 3, "No new worker admission in case {case}");
        assert!(!dir.path().join("must-not-exist.csv").exists());
        assert!(alias.same_snapshot(&app.variables["alias"]));
    }
}

#[test]
fn workspace_csv_availability_matches_real_logical_writer_types_without_payload_work() {
    let ctx = egui::Context::default();
    let mut app = legacy_sample_desktop(ctx.clone(), true);
    app.run("real_value=[1 2];mask=logical([1 0]);z=complex([1 2],[3 4]);text='hi';cells={1,2};h=plot([1 2]);empty_real=[];".into());
    finish(&mut app);
    app.workspace_tools.csv_picker = Some(|_| panic!("Unsupported CSV type opened a picker"));
    for name in ["real_value", "mask", "empty_real"] {
        app.selected = Some(name.into());
        frame(&mut app, &ctx, vec![]);
        assert!(app.can_export_workspace_csv(&ctx), "{name}");
    }
    for name in ["z", "text", "cells", "h"] {
        app.selected = Some(name.into());
        let original = app.variables[name].clone();
        let position = csv_footer_position(&mut app, &ctx);
        assert!(!app.can_export_workspace_csv(&ctx), "{name}");
        for pressed in [true, false] {
            pointer_frame(&mut app, &ctx, egui::vec2(1440., 900.), position, pressed);
        }
        assert_eq!(app.workspace_tools.csv_picker_calls, 0);
        assert!(!app.busy);
        assert!(original.same_snapshot(&app.variables[name]));
    }
}
