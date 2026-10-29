mod global_action_search_cases {
    use super::*;
    const WINDOWS_CTRL: egui::Modifiers = egui::Modifiers {
        ctrl: true, command: true, alt: false, shift: false, mac_cmd: false,
    };

    fn setup(ctx: &egui::Context, size: egui::Vec2) -> Workspace {
        let mut app = Workspace::with_context(ctx.clone(), true);
        app.new_document("search_run_value=71;\n");
        app.run("shared=[1 2 3]; alias=shared; plot(shared);".into()); finish(&mut app);
        app.reveal_editor(); app.selected = Some("shared".into());
        for _ in 0..3 { frame_at(&mut app, ctx, size, vec![]); }
        app
    }
    fn open(app: &mut Workspace, ctx: &egui::Context, size: egui::Vec2) {
        // A zoom change is committed by egui on a subsequent frame. Choose
        // the responsive launcher label from the settled requested viewport.
        for _ in 0..3 { frame_at(app, ctx, size, vec![]); }
        let label = if ctx.content_rect().width() < 900. { "Search" } else { "Search functions and actions" };
        click_label_at(app, ctx, size, label);
        for _ in 0..2 { frame_at(app, ctx, size, vec![]); }
        assert!(app.global_search.is_open());
        assert!(ctx.memory(|m| m.has_focus(global_search::query_id())));
    }
    fn query(app: &mut Workspace, ctx: &egui::Context, size: egui::Vec2, text: &str) {
        let point = ctx.read_response(global_search::query_id()).unwrap().rect.center();
        pointer_frame(app, ctx, size, point, true); pointer_frame(app, ctx, size, point, false);
        command_first_key(app, ctx, size, egui::Key::A, WINDOWS_CTRL);
        frame_at(app, ctx, size, vec![egui::Event::Text(text.into())]);
        frame_at(app, ctx, size, vec![]);
    }
    /// A click on the status bar, outside the palette card, closes it.
    fn click_outside(app: &mut Workspace, ctx: &egui::Context, size: egui::Vec2) {
        let point = egui::pos2(size.x * 0.5, size.y - 4.);
        pointer_frame(app, ctx, size, point, true); pointer_frame(app, ctx, size, point, false);
        for _ in 0..2 { frame_at(app, ctx, size, vec![]); }
    }
    fn assert_ink(output: &egui::FullOutput, viewport: egui::Rect, label: &str) {
        fn visit(shape: &egui::Shape, clip: egui::Rect, viewport: egui::Rect, label: &str, count: &mut usize) {
            match shape {
                egui::Shape::Text(text) if text.galley.text() == label => {
                    assert!(!text.galley.elided, "elided {label}");
                    let ink = text.visual_bounding_rect();
                    assert!(clip.contains_rect(ink) && viewport.contains_rect(ink), "clipped {label}: {ink:?} / {clip:?}");
                    *count += 1;
                }
                egui::Shape::Vec(shapes) => for shape in shapes { visit(shape, clip, viewport, label, count); },
                _ => {}
            }
        }
        let mut count = 0;
        for shape in &output.shapes { visit(&shape.shape, shape.clip_rect, viewport, label, &mut count); }
        assert!(count > 0, "missing visible {label}");
    }

    #[test]
    fn global_search_finite_real_query_settings_and_close_in_both_sizes_themes_and_scales() {
        for theme in [session::DesktopTheme::Light, session::DesktopTheme::Dark] {
            for size in [egui::vec2(960., 650.), egui::vec2(1440., 900.)] {
                for scale in [75, 100, 150] {
                    let ctx = egui::Context::default(); let mut app = setup(&ctx, size);
                    app.layout.theme = Some(theme); app.apply_desktop_theme(&ctx); app.set_desktop_scale(&ctx, scale);
                    frame_at(&mut app, &ctx, size, vec![]);
                    let document = app.documents[app.active].clone(); let figure = app.figure.clone().unwrap();
                    let shared = app.variables["shared"].clone();
                    open(&mut app, &ctx, size); query(&mut app, &ctx, size, "aPpEaRaNcE");
                    let output = frame_at(&mut app, &ctx, size, vec![]);
                    for label in ["Appearance: Light", "Appearance: Dark", "Appearance: Use system setting", "Up/Down to select, Enter to run, Esc to close"] {
                        assert_ink(&output, ctx.content_rect(), label);
                    }
                    command_first_key(&mut app, &ctx, size, egui::Key::ArrowDown, egui::Modifiers::NONE);
                    assert!(ctx.memory(|m| m.has_focus(global_search::list_id())), "first query-to-list arrow owns focus");
                    command_first_key(&mut app, &ctx, size, egui::Key::Enter, egui::Modifiers::NONE);
                    assert!(!app.global_search.is_open()); assert_eq!(app.layout.theme, Some(session::DesktopTheme::Dark));
                    assert!(ctx.global_style().visuals.dark_mode);
                    assert!(app.documents[app.active].same_snapshot(&document)); assert!(app.figure.as_ref().unwrap().same_snapshot(&figure));
                    assert!(app.variables["shared"].same_snapshot(&shared)); assert!(app.variables["shared"].same_snapshot(&app.variables["alias"]));
                    open(&mut app, &ctx, size); query(&mut app, &ctx, size, "Desktop scale: 100%");
                    command_first_key(&mut app, &ctx, size, egui::Key::Enter, egui::Modifiers::NONE);
                    assert_eq!(app.layout.ui_scale_percent(), 100); assert!(!app.global_search.is_open());
                    open(&mut app, &ctx, size);
                    command_first_key(&mut app, &ctx, size, egui::Key::Escape, egui::Modifiers::NONE);
                    assert!(!app.global_search.is_open());
                }
            }
        }
    }

    #[test]
    fn global_search_tab_list_navigation_help_and_bounded_unmatched_query_keep_editor_text() {
        let ctx = egui::Context::default(); let size = egui::vec2(960., 650.); let mut app = setup(&ctx, size);
        let document = app.documents[app.active].clone(); open(&mut app, &ctx, size);
        let count = app.documents.len();
        for modifiers in [egui::Modifiers::SHIFT, egui::Modifiers::ALT, WINDOWS_CTRL] {
            let point = ctx.read_response(global_search::query_id()).unwrap().rect.center();
            pointer_frame(&mut app, &ctx, size, point, true); pointer_frame(&mut app, &ctx, size, point, false);
            assert!(ctx.memory(|m| m.has_focus(global_search::query_id())));
            command_first_key(&mut app, &ctx, size, egui::Key::Enter, modifiers);
            assert!(app.global_search.is_open()); assert_eq!(app.documents.len(), count); assert!(!app.busy);
        }
        // TextEdit may surrender its own focus on a modified Return; pointer focus is explicit.
        let query_point = ctx.read_response(global_search::query_id()).unwrap().rect.center();
        pointer_frame(&mut app, &ctx, size, query_point, true); pointer_frame(&mut app, &ctx, size, query_point, false);
        command_first_key(&mut app, &ctx, size, egui::Key::Tab, egui::Modifiers::NONE);
        assert!(ctx.memory(|m| m.has_focus(global_search::list_id())));
        command_first_key(&mut app, &ctx, size, egui::Key::End, egui::Modifiers::NONE);
        assert!(ctx.memory(|m| m.has_focus(global_search::list_id())), "End retains list ownership");
        let output = frame_at(&mut app, &ctx, size, vec![]);
        assert_ink(&output, ctx.content_rect(), "Help: MatRust syntax and supported commands");
        command_first_key(&mut app, &ctx, size, egui::Key::Home, egui::Modifiers::NONE);
        let output = frame_at(&mut app, &ctx, size, vec![]); assert_ink(&output, ctx.content_rect(), "New script");
        command_first_key(&mut app, &ctx, size, egui::Key::End, egui::Modifiers::NONE);
        command_first_key(&mut app, &ctx, size, egui::Key::Enter, egui::Modifiers::NONE);
        assert!(!app.global_search.is_open()); assert!(app.console.iter().any(|line| line == matrust::engine::HELP));
        assert!(app.central_command()); assert!(app.documents.iter().find(|d| d.id == document.id).unwrap().same_snapshot(&document));
        open(&mut app, &ctx, size); query(&mut app, &ctx, size, &"💠".repeat(300));
        let output = frame_at(&mut app, &ctx, size, vec![]); assert_ink(&output, ctx.content_rect(), &format!("No matches for '{}…'", "💠".repeat(24)));
        let state = egui::text_edit::TextEditState::load(&ctx, global_search::query_id()).unwrap();
        assert!(state.cursor.char_range().unwrap().primary.index.0 <= 128);
        command_first_key(&mut app, &ctx, size, egui::Key::Enter, egui::Modifiers::NONE);
        assert!(!app.busy); assert!(app.global_search.is_open());
        click_outside(&mut app, &ctx, size); assert!(!app.global_search.is_open());
    }

    #[test]
    fn global_search_reuses_real_run_and_async_save_routes_with_unchanged_shared_arrays() {
        let dir = tempfile::tempdir().unwrap(); let path = dir.path().join("actual.m");
        let ctx = egui::Context::default(); let size = egui::vec2(1440., 900.); let mut app = setup(&ctx, size);
        assert!(!app.save_script_path(app.active, &path, None)); assert!(app.script_io.pending()); finish(&mut app);
        let shared = app.variables["shared"].clone(); let id = app.documents[app.active].id;
        open(&mut app, &ctx, size); query(&mut app, &ctx, size, "Run script");
        command_first_key(&mut app, &ctx, size, egui::Key::Enter, egui::Modifiers::NONE); finish(&mut app);
        assert_eq!(app.variables["search_run_value"].as_numeric().unwrap().get(0, 0), 71.);
        assert_eq!(app.documents[app.active].id, id); assert!(app.variables["shared"].same_snapshot(&shared));
        app.documents[app.active].text.push_str("% saved through action search\n"); app.documents[app.active].changed();
        let expected = app.documents[app.active].text.clone();
        frame_at(&mut app, &ctx, size, vec![]); open(&mut app, &ctx, size); query(&mut app, &ctx, size, "Save script");
        // One press frame: receive() has already run, so the newly queued save is still pending.
        frame_at(&mut app, &ctx, size, vec![egui::Event::Key {
            key: egui::Key::Enter, physical_key: Some(egui::Key::Enter), pressed: true,
            repeat: false, modifiers: egui::Modifiers::NONE,
        }]);
        assert!(!app.global_search.is_open()); assert!(app.script_io.pending()); finish(&mut app);
        assert_eq!(std::fs::read(&path).unwrap(), expected.as_bytes()); assert!(!app.documents[app.active].dirty);
        assert_eq!(app.documents[app.active].id, id); assert!(app.variables["shared"].same_snapshot(&shared));
    }

    #[test]
    fn global_search_discarded_enter_dispatches_once_and_rejects_late_document_busy_modal_or_focus() {
        for change in 0..6 {
            let ctx = egui::Context::default(); let size = egui::vec2(960., 650.); let mut app = setup(&ctx, size);
            open(&mut app, &ctx, size); query(&mut app, &ctx, size, "New script");
            let before = app.documents.len(); let original = app.documents[app.active].clone(); let mut passes = 0;
            crate::headless::run(&ctx, egui::RawInput {
                screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, size)),
                events: vec![egui::Event::Key { key: egui::Key::Enter, physical_key: Some(egui::Key::Enter), pressed: true, repeat: false, modifiers: egui::Modifiers::NONE }],
                ..Default::default()
            }, |root| {
                passes += 1; if passes == 1 { root.ctx().request_discard("action must wait for final target guard"); }
                app.ui(root);
                if passes == 1 {
                    assert_eq!(app.documents.len(), before);
                    match change {
                        1 => app.new_document("late_document=1;"),
                        2 => app.busy = true,
                        3 => {
                            // The palette correctly blocks request_script_close. Model a
                            // foreign confirmation that becomes owned between layout passes,
                            // then let the real existing Close modal render on the retry.
                            assert!(app.documents[app.active].dirty);
                            app.pending_close = Some(app.active);
                            assert!(app.file_action_pending_without_global_search());
                        },
                        4 => root.ctx().memory_mut(|m| m.request_focus(egui::Id::new("unrelated_focus_owner"))),
                        5 => app.stop_work(),
                        _ => {}
                    }
                }
            });
            assert!(passes >= 2);
            assert_eq!(app.documents.len(), before + usize::from(change <= 1), "discard change={change}, status={}, pending_close={:?}", app.status, app.pending_close);
            assert!(app.documents.iter().find(|d| d.id == original.id).unwrap().same_snapshot(&original));
            if change == 0 { assert!(!app.global_search.is_open()); }
            if change == 2 {
                // Escape still works after availability changes; it does not refocus a script.
                command_first_key(&mut app, &ctx, size, egui::Key::Escape, egui::Modifiers::NONE);
                assert!(!app.global_search.is_open()); app.busy = false;
            }
        }
    }

    #[test]
    fn global_search_rejects_replaced_named_variable_and_reused_figure_number() {
        for figure in [false, true] {
            let ctx = egui::Context::default(); let size = egui::vec2(1440., 900.); let mut app = setup(&ctx, size);
            open(&mut app, &ctx, size); query(&mut app, &ctx, size, if figure { "Figure properties" } else { "Inspect variable" });
            let original = app.figure.as_ref().unwrap().identity.clone(); let old = app.variables["shared"].clone();
            app.run(if figure { "close(1); figure(1); plot([8 9]);" } else { "shared=[8 9];" }.into()); finish(&mut app);
            if figure { assert!(!original.same_object(&app.figure.as_ref().unwrap().identity)); }
            else { assert!(!old.same_snapshot(&app.variables["shared"])); }
            command_first_key(&mut app, &ctx, size, egui::Key::Enter, egui::Modifiers::NONE);
            assert!(app.global_search.is_open()); assert!(app.figure_editor.is_none()); assert!(app.editing_variable.is_none());
            click_outside(&mut app, &ctx, size); assert!(!app.global_search.is_open());
        }
    }

    #[test]
    fn global_search_saved_script_and_figure_reload_keep_real_confirmations_and_workspace() {
        let dir = tempfile::tempdir().unwrap(); let script = dir.path().join("reload.m"); let figure_path = dir.path().join("reload.mfig");
        let ctx = egui::Context::default(); let size = egui::vec2(1440., 900.); let mut app = setup(&ctx, size);
        assert!(!app.save_script_path(app.active, &script, None)); finish(&mut app);
        let shared = app.variables["shared"].clone();
        std::fs::write(&script, "reload_is_not_run=99;\n").unwrap();
        app.select_document_layout(layout::PaneMode::Editor); frame_at(&mut app, &ctx, size, vec![]);
        let point = ctx.read_response(editor::id(app.documents[app.active].id)).unwrap().interact_rect.center();
        pointer_frame(&mut app, &ctx, size, point, true); pointer_frame(&mut app, &ctx, size, point, false);
        assert!(app.can_reload_script());
        open(&mut app, &ctx, size); query(&mut app, &ctx, size, "Reload script");
        assert!(app.file_action_pending_without_eps()); assert!(!app.file_action_pending_without_global_search());
        assert!(!app.can_reload_script(), "ordinary admission still blocks the palette");
        command_first_key(&mut app, &ctx, size, egui::Key::Enter, egui::Modifiers::NONE);
        assert!(!app.global_search.is_open()); assert!(app.script_reload.is_some());
        assert_eq!(app.documents[app.active].text, "search_run_value=71;\n");
        click_label_at(&mut app, &ctx, size, "Reload script"); finish(&mut app);
        assert!(app.script_io.deferred_reload.is_some(), "worker reply waits for final-pass publication");
        assert_eq!(app.documents[app.active].text, "search_run_value=71;\n");
        frame_at(&mut app, &ctx, size, vec![]);
        assert!(app.script_io.deferred_reload.is_none());
        assert_eq!(app.documents[app.active].text, "reload_is_not_run=99;\n"); assert!(!app.documents[app.active].dirty);
        assert!(!app.variables.contains_key("reload_is_not_run")); assert!(app.variables["shared"].same_snapshot(&shared));
        app.submit(Job::SaveFigureById(app.current_figure, figure_path.clone(), None)); finish(&mut app);
        let saved_title = app.figure.as_ref().unwrap().title.clone();
        app.run("title('Changed after save');".into()); finish(&mut app);
        assert_ne!(app.figure.as_ref().unwrap().title, saved_title);
        for _ in 0..3 { frame_at(&mut app, &ctx, size, vec![]); }
        assert!(app.can_reload_figure()); open(&mut app, &ctx, size); query(&mut app, &ctx, size, "Reload figure");
        assert!(!app.can_reload_figure());
        command_first_key(&mut app, &ctx, size, egui::Key::Enter, egui::Modifiers::NONE);
        assert!(!app.global_search.is_open()); assert!(app.figure_conflict.as_ref().is_some_and(|c| c.reload_requested));
        assert_eq!(app.figure.as_ref().unwrap().title, "Changed after save");
        click_label_at(&mut app, &ctx, size, "Reload figure"); finish(&mut app);
        assert_eq!(app.figure.as_ref().unwrap().title, saved_title);
        assert!(app.variables["shared"].same_snapshot(&shared)); assert!(app.variables["shared"].same_snapshot(&app.variables["alias"]));
    }

    #[test]
    fn global_search_discarded_pointer_release_keeps_first_owner_and_rejects_late_focus() {
        for late_focus in [false, true] {
            let ctx = egui::Context::default(); let size = egui::vec2(960., 650.); let mut app = setup(&ctx, size);
            open(&mut app, &ctx, size); query(&mut app, &ctx, size, "New script");
            let output = frame_at(&mut app, &ctx, size, vec![]);
            let point = output.shapes.iter().rev().find_map(|s| text_position(&s.shape, "New script")).unwrap();
            pointer_frame(&mut app, &ctx, size, point, true); let before = app.documents.len(); let mut passes = 0;
            crate::headless::run(&ctx, egui::RawInput {
                screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, size)),
                events: vec![egui::Event::PointerMoved(point), egui::Event::PointerButton {
                    pos: point, button: egui::PointerButton::Primary, pressed: false, modifiers: egui::Modifiers::NONE,
                }], ..Default::default()
            }, |root| {
                passes += 1; if passes == 1 { root.ctx().request_discard("pointer result waits for final owner"); }
                app.ui(root);
                if passes == 1 {
                    assert_eq!(app.documents.len(), before);
                    assert!(root.ctx().memory(|m| m.has_focus(global_search::list_id())));
                    if late_focus { root.ctx().memory_mut(|m| m.request_focus(egui::Id::new("command_input"))); }
                }
            });
            assert!(passes >= 2); assert_eq!(app.documents.len(), before + usize::from(!late_focus));
            if late_focus {
                assert!(app.global_search.is_open());
                assert!(!ctx.memory(|m| m.has_focus(global_search::query_id()) || m.has_focus(global_search::list_id())), "retry must not reclaim palette focus");
            } else { assert!(!app.global_search.is_open()); }
        }
    }

    #[test]
    fn global_search_theme_words_find_the_appearance_settings() {
        for query in ["theme", "Theme", "dark", "light", "appearance", "mode", "dark mode", "the"] {
            let labels = global_search::matching_labels(query);
            for label in ["Appearance: Light", "Appearance: Dark", "Appearance: Use system setting"] {
                assert!(labels.contains(&label), "{query:?} misses {label}: {labels:?}");
            }
        }
        assert!(!global_search::matching_labels("theme").contains(&"New script"));
        assert_eq!(global_search::matching_labels("zzz_no_entry"), Vec::<&str>::new());
        assert_eq!(global_search::no_matches(" zzz "), "No matches for 'zzz'");
        // Enter runs the first row: the named theme must come first.
        assert_eq!(global_search::matching_labels("dark")[0], "Appearance: Dark");
        assert_eq!(global_search::matching_labels("light")[0], "Appearance: Light");
    }

    #[test]
    fn global_search_card_has_no_title_or_close_button_and_keeps_the_toolbar_undimmed() {
        let ctx = egui::Context::default(); let size = egui::vec2(1440., 900.); let mut app = setup(&ctx, size);
        let before = frame_at(&mut app, &ctx, size, vec![]);
        let tab_alpha = |output: &egui::FullOutput| output.shapes.iter().find_map(|s| match &s.shape {
            egui::Shape::Text(t) if t.galley.text() == "Plots" => Some(t.opacity_factor),
            _ => None,
        });
        let closed = tab_alpha(&before).expect("Plots tab");
        open(&mut app, &ctx, size);
        let output = frame_at(&mut app, &ctx, size, vec![]);
        assert_ink(&output, ctx.content_rect(), "Search actions and settings");
        assert_ink(&output, ctx.content_rect(), "Up/Down to select, Enter to run, Esc to close");
        assert_ink(&output, ctx.content_rect(), "Ctrl+N");
        for gone in ["Close search", "Arrow keys select; Enter opens. Escape closes."] {
            assert!(!output.shapes.iter().any(|s| text_position(&s.shape, gone).is_some()), "{gone}");
        }
        assert_eq!(tab_alpha(&output), Some(closed), "toolbar dimmed behind the palette");
        // One 560 px card under the launcher; the query field spans it.
        let query = ctx.read_response(global_search::query_id()).unwrap().rect;
        let launcher = app.global_search.anchor_rect().unwrap();
        assert!(query.top() >= launcher.bottom(), "{query:?} / {launcher:?}");
        let inner = 560. - global_search::card_frame(&ctx).total_margin().sum().x;
        assert!((query.width() - inner).abs() < 1.5, "{query:?}");
    }

    #[test]
    fn global_search_leaves_tabs_and_ribbon_live_and_a_click_closes_it_and_acts() {
        let ctx = egui::Context::default(); let size = egui::vec2(1440., 900.); let mut app = setup(&ctx, size);
        open(&mut app, &ctx, size);
        let documents = app.documents.len();
        click_label_at(&mut app, &ctx, size, "New script");
        for _ in 0..2 { frame_at(&mut app, &ctx, size, vec![]); }
        assert!(!app.global_search.is_open(), "the click outside closes the palette");
        assert_eq!(app.documents.len(), documents + 1, "the ribbon action ran");
        open(&mut app, &ctx, size);
        click_label_at(&mut app, &ctx, size, "Plots");
        for _ in 0..2 { frame_at(&mut app, &ctx, size, vec![]); }
        assert!(!app.global_search.is_open());
        assert_eq!(app.layout.toolstrip, session::ToolstripTab::Plots);
    }
}
