mod editor_columns_cases {
    use super::*;

    // egui-winit maps a Windows Ctrl gesture to both fields; TextEdit uses command.
    const WINDOWS_CTRL: egui::Modifiers = egui::Modifiers {
        alt: false,
        ctrl: true,
        shift: false,
        mac_cmd: false,
        command: true,
    };

    fn save_fixture(app: &mut Workspace, index: usize, path: &std::path::Path) {
        let document = app.documents[index].id;
        let expected = app.documents[index].text.clone();
        assert!(!app.save_script_path(index, path, None));
        assert!(app.script_io.pending(), "save must be queued, not rejected");
        finish(app);
        assert!(!app.script_io.pending());
        let saved = &app.documents[index];
        assert_eq!(saved.id, document);
        assert!(matrust::document::same_path(
            saved.path.as_deref().expect("published path"),
            path
        ));
        assert_eq!(saved.text, expected);
        assert!(
            !saved.dirty,
            "successful publication must acknowledge this version"
        );
        assert_eq!(std::fs::read(path).unwrap(), expected.as_bytes());
    }

    fn setup(ctx: &egui::Context, size: egui::Vec2) -> Workspace {
        let mut app = Workspace::with_context(ctx.clone(), true);
        app.new_document("left_value=11;\n");
        app.new_document("right_value=29;\n");
        app.activate_script_index(0);
        app.select_document_layout(layout::PaneMode::Editor);
        for _ in 0..3 {
            frame_at(&mut app, ctx, size, vec![]);
        }
        click_label_at(&mut app, ctx, size, "Editor actions");
        click_label_at(&mut app, ctx, size, "Side by side");
        for _ in 0..3 {
            frame_at(&mut app, ctx, size, vec![]);
        }
        assert_eq!(
            app.snapshot().layout.editor_columns,
            Some(session::EditorColumns { left: 0, right: 1 })
        );
        app
    }

    fn point(app: &Workspace, ctx: &egui::Context, index: usize) -> egui::Pos2 {
        let response = ctx
            .read_response(editor::id(app.documents[index].id))
            .unwrap();
        let visible = response.interact_rect.intersect(ctx.content_rect());
        assert!(
            visible.width() > 40. && visible.height() > 30.,
            "editor {index}: {visible:?}"
        );
        visible.center()
    }

    fn focus(app: &mut Workspace, ctx: &egui::Context, size: egui::Vec2, index: usize) {
        let position = point(app, ctx, index);
        pointer_frame(app, ctx, size, position, true);
        pointer_frame(app, ctx, size, position, false);
        assert_eq!(app.active, index);
        assert!(ctx.memory(|m| m.has_focus(editor::id(app.documents[index].id))));
    }

    fn cursor(ctx: &egui::Context, document: u64) -> std::ops::Range<usize> {
        let range = egui::text_edit::TextEditState::load(ctx, editor::id(document))
            .unwrap()
            .cursor
            .char_range()
            .unwrap()
            .as_sorted_char_range();
        range.start.0..range.end.0
    }

    fn click_tab(app: &mut Workspace, ctx: &egui::Context, size: egui::Vec2, title: &str) {
        fn positions(shape: &egui::epaint::Shape, title: &str, found: &mut Vec<egui::Pos2>) {
            match shape {
                egui::epaint::Shape::Text(text) if text.galley.text() == title => {
                    found.push(text.pos + text.galley.size() * 0.5)
                }
                egui::epaint::Shape::Vec(shapes) => {
                    for shape in shapes {
                        positions(shape, title, found);
                    }
                }
                _ => {}
            }
        }
        let output = frame_at(app, ctx, size, vec![]);
        let mut found = Vec::new();
        for shape in &output.shapes {
            positions(&shape.shape, title, &mut found);
        }
        // The shared tab strip is above the optional matching per-column heading.
        let point = found
            .into_iter()
            .min_by(|a, b| a.y.total_cmp(&b.y))
            .expect("script tab");
        pointer_frame(app, ctx, size, point, true);
        pointer_frame(app, ctx, size, point, false);
    }

    #[test]
    fn real_two_editors_keep_independent_text_cursor_search_and_undo_in_both_themes_and_sizes() {
        fn assert_column_title_colors(app: &mut Workspace, ctx: &egui::Context, size: egui::Vec2) {
            fn headings<'a>(
                shape: &'a egui::Shape,
                clip: egui::Rect,
                title: &str,
                rect: egui::Rect,
                found: &mut Vec<(&'a egui::epaint::TextShape, egui::Rect)>,
                rejected: &mut Vec<(egui::Rect, egui::Rect)>,
            ) {
                match shape {
                    egui::Shape::Text(text) if text.galley.text() == title => {
                        let ink = text.visual_bounding_rect();
                        // Identify the actual Label by its recorded response, not
                        // another same-named tab or the peer's Untitled heading.
                        if rect.contains(text.pos + text.galley.size() * 0.5) {
                            found.push((text, clip));
                        } else if rejected.len() < 8 {
                            rejected.push((ink, clip));
                        }
                    }
                    egui::Shape::Vec(shapes) => {
                        for shape in shapes {
                            headings(shape, clip, title, rect, found, rejected);
                        }
                    }
                    _ => {}
                }
            }
            let output = frame_at(app, ctx, size, vec![]);
            let observed = app.editor_heading_observations();
            for index in 0..2 {
                let observation = observed
                    .iter()
                    .flatten()
                    .find(|heading| heading.document == app.documents[index].id)
                    .expect("actual column Label response");
                let mut found = Vec::new();
                let mut rejected = Vec::new();
                for shape in &output.shapes {
                    headings(
                        &shape.shape,
                        shape.clip_rect,
                        &app.documents[index].title(),
                        observation.rect,
                        &mut found,
                        &mut rejected,
                    );
                }
                let trace = format!(
                    "size={size:?}, viewport={:?}, active={}, document={}, observation={observation:?}, rejected={rejected:?}",
                    ctx.content_rect(),
                    app.active,
                    app.documents[index].id
                );
                assert_eq!(
                    observation.active,
                    index == app.active,
                    "settled active-at-paint changed; {trace}"
                );
                // Context::run_ui advances the frame counter once after all passes.
                assert_eq!(
                    observation.frame + 1,
                    ctx.cumulative_frame_nr(),
                    "heading belongs to returned frame; {trace}"
                );
                assert_eq!(found.len(), 1, "exactly one actual column heading; {trace}");
                let (text, clip) = found[0];
                let ink = text.visual_bounding_rect();
                assert!(
                    clip.contains_rect(ink),
                    "column heading ink: {ink:?}, clip: {clip:?}; {trace}"
                );
                assert!(
                    observation.clip.contains_rect(ink),
                    "column child clip: {trace}, ink={ink:?}"
                );
                assert!(!text.galley.elided, "full column title: {trace}");
                assert_eq!(text.opacity_factor, 1., "{trace}");
                let expected = if index == app.active {
                    crate::tokens::palette(ctx).accent
                } else {
                    ctx.global_style().visuals.text_color()
                };
                // Match epaint's effective glyph ink, including its cached
                // placeholder/fallback and explicit draw-time color override.
                let mut glyphs = 0;
                for row in &text.galley.rows {
                    for vertex in &row.visuals.mesh.vertices[row.visuals.glyph_vertex_range.clone()]
                    {
                        let color = text.override_text_color.unwrap_or_else(|| {
                            if vertex.color == Color32::PLACEHOLDER {
                                text.fallback_color
                            } else {
                                vertex.color
                            }
                        });
                        assert_eq!(color, expected, "painted column heading {index}; {trace}");
                        glyphs += 1;
                    }
                }
                assert!(glyphs > 0, "nonempty painted column heading");
            }
        }
        for theme in [session::DesktopTheme::Light, session::DesktopTheme::Dark] {
            for size in [egui::vec2(960., 650.), egui::vec2(1440., 900.)] {
                let ctx = egui::Context::default();
                let mut app = setup(&ctx, size);
                app.layout.theme = Some(theme);
                app.apply_desktop_theme(&ctx);
                frame_at(&mut app, &ctx, size, vec![]);
                assert_eq!(
                    ctx.global_style().visuals.dark_mode,
                    theme == session::DesktopTheme::Dark
                );
                let ids = [app.documents[0].id, app.documents[1].id];
                let rects = ids.map(|id| ctx.read_response(editor::id(id)).unwrap().interact_rect);
                assert!(
                    rects[0].right() <= rects[1].left(),
                    "two distinct finite columns: {rects:?}"
                );
                focus(&mut app, &ctx, size, 0);
                assert_column_title_colors(&mut app, &ctx, size);
                command_first_key(&mut app, &ctx, size, egui::Key::End, WINDOWS_CTRL);
                frame_at(
                    &mut app,
                    &ctx,
                    size,
                    vec![egui::Event::Text("% left edit".into())],
                );
                let left_cursor = cursor(&ctx, ids[0]);
                let left_text = app.documents[0].text.clone();
                focus(&mut app, &ctx, size, 1);
                assert_column_title_colors(&mut app, &ctx, size);
                command_first_key(&mut app, &ctx, size, egui::Key::End, WINDOWS_CTRL);
                frame_at(
                    &mut app,
                    &ctx,
                    size,
                    vec![egui::Event::Text("% right edit".into())],
                );
                assert_eq!(app.documents[0].text, left_text);
                assert_eq!(cursor(&ctx, ids[0]), left_cursor);
                command_first_key(&mut app, &ctx, size, egui::Key::Z, WINDOWS_CTRL);
                assert_eq!(app.documents[1].text, "right_value=29;\n");
                assert_eq!(app.documents[0].text, left_text);
                // Each actual search field retains its own query across activation.
                command_first_key(&mut app, &ctx, size, egui::Key::F, WINDOWS_CTRL);
                frame_at(
                    &mut app,
                    &ctx,
                    size,
                    vec![egui::Event::Text("right_value".into())],
                );
                let right_search = ctx.memory(|m| m.focused()).unwrap();
                focus(&mut app, &ctx, size, 0);
                command_first_key(&mut app, &ctx, size, egui::Key::F, WINDOWS_CTRL);
                frame_at(
                    &mut app,
                    &ctx,
                    size,
                    vec![egui::Event::Text("left_value".into())],
                );
                assert_ne!(ctx.memory(|m| m.focused()).unwrap(), right_search);
                let output = frame_at(&mut app, &ctx, size, vec![]);
                for query in ["left_value", "right_value"] {
                    assert!(
                        output
                            .shapes
                            .iter()
                            .any(|s| text_position(&s.shape, query).is_some()),
                        "query {query}"
                    );
                }
                assert!(app.documents.iter().all(|doc| doc.dirty));
                assert_eq!([app.documents[0].id, app.documents[1].id], ids);
            }
        }
    }

    #[test]
    fn same_frame_pointer_save_and_real_run_selection_target_the_clicked_document() {
        let ctx = egui::Context::default();
        let size = egui::vec2(960., 650.);
        let mut app = setup(&ctx, size);
        let dir = tempfile::tempdir().unwrap();
        for index in 0..2 {
            let path = dir.path().join(format!("script{index}.m"));
            save_fixture(&mut app, index, &path);
            app.documents[index].text.push_str("% retained edit\n");
            app.documents[index].changed();
        }
        for _ in 0..3 {
            frame_at(&mut app, &ctx, size, vec![]);
        }
        let right = point(&app, &ctx, 1);
        frame_at(
            &mut app,
            &ctx,
            size,
            vec![
                egui::Event::PointerMoved(right),
                egui::Event::PointerButton {
                    pos: right,
                    button: egui::PointerButton::Primary,
                    pressed: true,
                    modifiers: WINDOWS_CTRL,
                },
                egui::Event::Key {
                    key: egui::Key::S,
                    physical_key: None,
                    pressed: true,
                    repeat: false,
                    modifiers: WINDOWS_CTRL,
                },
            ],
        );
        finish(&mut app);
        pointer_frame(&mut app, &ctx, size, right, false);
        assert_eq!(app.active, 1);
        assert!(!app.documents[1].dirty && app.documents[0].dirty);
        assert_eq!(
            std::fs::read_to_string(dir.path().join("script1.m")).unwrap(),
            app.documents[1].text
        );
        assert_eq!(
            std::fs::read_to_string(dir.path().join("script0.m")).unwrap(),
            "left_value=11;\n"
        );
        command_first_key(&mut app, &ctx, size, egui::Key::Enter, WINDOWS_CTRL);
        finish(&mut app);
        assert_eq!(
            app.variables["right_value"].as_numeric().unwrap().get(0, 0),
            29.
        );
        assert!(!app.variables.contains_key("left_value"));
        let left = point(&app, &ctx, 0);
        frame_at(
            &mut app,
            &ctx,
            size,
            vec![
                egui::Event::PointerMoved(left),
                egui::Event::PointerButton {
                    pos: left,
                    button: egui::PointerButton::Primary,
                    pressed: true,
                    modifiers: WINDOWS_CTRL,
                },
                egui::Event::Key {
                    key: egui::Key::Enter,
                    physical_key: None,
                    pressed: true,
                    repeat: false,
                    modifiers: WINDOWS_CTRL,
                },
            ],
        );
        finish(&mut app);
        pointer_frame(&mut app, &ctx, size, left, false);
        assert_eq!(app.active, 0);
        assert_eq!(
            app.variables["left_value"].as_numeric().unwrap().get(0, 0),
            11.
        );
        focus(&mut app, &ctx, size, 0);
        command_first_key(&mut app, &ctx, size, egui::Key::A, WINDOWS_CTRL);
        command_first_key(&mut app, &ctx, size, egui::Key::F9, egui::Modifiers::NONE);
        finish(&mut app);
        assert_eq!(
            app.variables["left_value"].as_numeric().unwrap().get(0, 0),
            11.
        );
        assert_eq!(
            app.snapshot().layout.editor_columns,
            Some(session::EditorColumns { left: 0, right: 1 })
        );
    }

    #[test]
    fn discarded_save_cannot_follow_an_active_document_change_and_modal_cannot_activate_peer() {
        let ctx = egui::Context::default();
        let size = egui::vec2(960., 650.);
        let mut app = setup(&ctx, size);
        app.document_save.intercepted = Some(Vec::new());
        let mut passes = 0;
        crate::headless::run(
            &ctx,
            egui::RawInput {
                screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, size)),
                ..Default::default()
            },
            |root| {
                passes += 1;
                if passes == 1 {
                    app.document_save.queue(
                        root.ctx().cumulative_frame_nr(),
                        false,
                        document_save::Pane::Script,
                    );
                    root.ctx()
                        .request_discard("change document before final dispatch");
                    app.finish_document_action(root.ctx());
                    app.activate_script_index(1);
                } else {
                    app.finish_document_action(root.ctx());
                }
            },
        );
        assert!(passes >= 2);
        assert!(app.document_save.intercepted.as_ref().unwrap().is_empty());
        assert!(app.status.contains("active script changed"));
        assert!(app.documents.iter().all(|doc| doc.dirty));
        // The custom dispatch-only frame above did not paint TextEdits.
        frame_at(&mut app, &ctx, size, vec![]);
        let left = point(&app, &ctx, 0);
        app.request_script_close(1);
        assert!(app.file_action_pending());
        let before = app.documents[app.active].id;
        pointer_frame(&mut app, &ctx, size, left, true);
        pointer_frame(&mut app, &ctx, size, left, false);
        assert_eq!(app.documents[app.active].id, before);
        click_label_at(&mut app, &ctx, size, "Cancel");
        assert!(!app.file_action_pending());
        assert_eq!(app.documents.len(), 2);
        app.show_command_center();
        pointer_frame(&mut app, &ctx, size, left, true);
        pointer_frame(&mut app, &ctx, size, left, false);
        assert_eq!(
            app.documents[app.active].id, before,
            "hidden old editor region has no authority"
        );
        assert!(app.central_command());
    }

    #[test]
    fn real_pointer_and_save_on_discarded_pass_never_save_the_old_document() {
        let ctx = egui::Context::default();
        let size = egui::vec2(960., 650.);
        let mut app = setup(&ctx, size);
        app.document_save.intercepted = Some(Vec::new());
        let right = point(&app, &ctx, 1);
        let right_id = app.documents[1].id;
        let mut passes = 0;
        crate::headless::run(
            &ctx,
            egui::RawInput {
                screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, size)),
                events: vec![
                    egui::Event::ModifiersChanged(WINDOWS_CTRL),
                    egui::Event::PointerMoved(right),
                    egui::Event::PointerButton {
                        pos: right,
                        button: egui::PointerButton::Primary,
                        pressed: true,
                        modifiers: WINDOWS_CTRL,
                    },
                    egui::Event::Key {
                        key: egui::Key::S,
                        physical_key: None,
                        pressed: true,
                        repeat: false,
                        modifiers: WINDOWS_CTRL,
                    },
                ],
                ..Default::default()
            },
            |root| {
                passes += 1;
                if passes == 1 {
                    root.ctx().request_discard("unsettled editor geometry");
                }
                app.ui(root);
            },
        );
        assert!(passes >= 2);
        // A retry may reject a changed target; it must never fall back to the old tab.
        let requests = app.document_save.intercepted.as_ref().unwrap();
        assert!(requests.len() <= 1);
        assert!(requests.iter().all(|request| *request
            == document_save::Request::Script {
                document: right_id,
                save_as: false
            }));
        assert!(app.documents.iter().all(|doc| doc.dirty));
        pointer_frame(&mut app, &ctx, size, right, false);
    }

    #[test]
    fn explicit_tab_activation_transfers_only_editor_focus_before_find_and_undo() {
        let ctx = egui::Context::default();
        let size = egui::vec2(1440., 900.);
        let mut app = setup(&ctx, size);
        // Give the tabs distinct real names through the existing public save path.
        let dir = tempfile::tempdir().unwrap();
        for i in 0..2 {
            save_fixture(&mut app, i, &dir.path().join(format!("named{i}.m")));
        }
        for _ in 0..3 {
            frame_at(&mut app, &ctx, size, vec![]);
        }
        focus(&mut app, &ctx, size, 1);
        command_first_key(&mut app, &ctx, size, egui::Key::End, WINDOWS_CTRL);
        frame_at(
            &mut app,
            &ctx,
            size,
            vec![egui::Event::Text("% right retained".into())],
        );
        let right_text = app.documents[1].text.clone();
        click_tab(&mut app, &ctx, size, "named0.m");
        assert_eq!(app.active, 0);
        assert!(ctx.memory(|m| m.has_focus(editor::id(app.documents[0].id))));
        command_first_key(&mut app, &ctx, size, egui::Key::End, WINDOWS_CTRL);
        frame_at(
            &mut app,
            &ctx,
            size,
            vec![egui::Event::Text("% left temporary".into())],
        );
        command_first_key(&mut app, &ctx, size, egui::Key::Z, WINDOWS_CTRL);
        assert_eq!(app.documents[0].text, "left_value=11;\n");
        assert_eq!(app.documents[1].text, right_text);
        command_first_key(&mut app, &ctx, size, egui::Key::F, WINDOWS_CTRL);
        frame_at(
            &mut app,
            &ctx,
            size,
            vec![egui::Event::Text("left_value".into())],
        );
        let field = ctx
            .read_response(ctx.memory(|m| m.focused()).unwrap())
            .unwrap();
        let left = ctx.read_response(editor::id(app.documents[0].id)).unwrap();
        let right = ctx.read_response(editor::id(app.documents[1].id)).unwrap();
        assert!(field.rect.center().x < right.rect.left());
        assert!(field.rect.center().x >= left.rect.left());
        // Command focus remains authoritative when background script activation occurs.
        app.show_command_center();
        for _ in 0..3 {
            frame_at(&mut app, &ctx, size, vec![]);
        }
        ctx.memory_mut(|m| m.request_focus(egui::Id::new("command_input")));
        app.activate_script_index(1);
        assert!(ctx.memory(|m| m.has_focus(egui::Id::new("command_input"))));
    }

    #[test]
    fn new_open_close_and_reset_keep_other_dirty_buffers_and_recovery_indices() {
        let ctx = egui::Context::default();
        let size = egui::vec2(1440., 900.);
        let mut app = setup(&ctx, size);
        let original = app
            .documents
            .iter()
            .map(|d| (d.id, d.text.clone()))
            .collect::<Vec<_>>();
        focus(&mut app, &ctx, size, 1);
        app.new_document("third=43;");
        for _ in 0..3 {
            frame_at(&mut app, &ctx, size, vec![]);
        }
        assert_eq!(
            app.snapshot().layout.editor_columns,
            Some(session::EditorColumns { left: 0, right: 2 })
        );
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("fourth.m");
        std::fs::write(&path, "fourth=47;\r\n").unwrap();
        app.open_path(path);
        finish(&mut app);
        for _ in 0..3 {
            frame_at(&mut app, &ctx, size, vec![]);
        }
        assert_eq!(
            app.snapshot().layout.editor_columns,
            Some(session::EditorColumns { left: 0, right: 3 })
        );
        assert_eq!(app.documents[3].text, "fourth=47;\r\n");
        app.request_script_close(3); // clean public Open closes immediately
        assert_eq!(app.active, 0);
        assert!(app.snapshot().layout.editor_columns.is_none());
        for (index, (id, text)) in original.iter().enumerate() {
            assert_eq!(app.documents[index].id, *id);
            assert_eq!(&app.documents[index].text, text);
            assert!(app.documents[index].dirty);
        }
        click_label_at(&mut app, &ctx, size, "Editor actions");
        click_label_at(&mut app, &ctx, size, "Side by side");
        app.reset_layout(&ctx);
        assert!(app.snapshot().layout.editor_columns.is_none());
        assert_eq!(app.documents.len(), 3);
        assert!(app.documents.iter().all(|doc| doc.dirty));
    }

    #[test]
    fn split_save_conflict_and_keyboard_close_cancel_preserve_both_buffers_and_disk() {
        let ctx = egui::Context::default();
        let size = egui::vec2(960., 650.);
        let mut app = setup(&ctx, size);
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("right.m");
        save_fixture(&mut app, 1, &path);
        focus(&mut app, &ctx, size, 1);
        app.documents[1].text.push_str("% unsaved");
        app.documents[1].changed();
        std::fs::write(&path, "external=101;\n").unwrap();
        let before = app.documents.clone();
        document_save_key(&mut app, &ctx, size, false);
        finish(&mut app);
        assert_eq!(app.script_conflict.as_ref().unwrap().document, before[1].id);
        assert_eq!(std::fs::read_to_string(&path).unwrap(), "external=101;\n");
        click_label_at(&mut app, &ctx, size, "Cancel");
        assert!(app.script_conflict.is_none());
        command_first_key(&mut app, &ctx, size, egui::Key::W, WINDOWS_CTRL);
        assert_eq!(app.pending_close, Some(1));
        click_label_at(&mut app, &ctx, size, "Cancel");
        for (doc, original) in app.documents.iter().zip(&before) {
            assert!(doc.same_snapshot(original));
        }
        assert_eq!(
            app.snapshot().layout.editor_columns,
            Some(session::EditorColumns { left: 0, right: 1 })
        );
        assert_eq!(std::fs::read_to_string(&path).unwrap(), "external=101;\n");
    }

    #[test]
    fn actual_session_roundtrip_restores_two_views_without_mutating_dirty_work_or_workspace() {
        let ctx = egui::Context::default();
        let size = egui::vec2(960., 650.);
        let mut app = setup(&ctx, size);
        app.run("shared=[1 2;3 4];alias=shared;".into());
        finish(&mut app);
        focus(&mut app, &ctx, size, 1);
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("session.json");
        let snapshot = app.snapshot();
        snapshot.save(&path).unwrap();
        let loaded = Session::load(&path).unwrap();
        assert_eq!(
            loaded.layout.editor_columns,
            Some(session::EditorColumns { left: 0, right: 1 })
        );
        assert_eq!(loaded.active, 1);
        assert!(loaded.documents.iter().all(|d| d.dirty));
        let restored_ctx = egui::Context::default();
        let mut restored = Workspace::with_context(restored_ctx.clone(), true);
        restored.restore_session(loaded);
        restored.start_session_recovery(path.clone());
        finish(&mut restored);
        assert!(!restored.recovering_session && !restored.recovery_blocked);
        assert!(restored.variables["shared"].same_snapshot(&restored.variables["alias"]));
        assert_eq!(
            restored.variables["shared"].as_numeric().unwrap().data(),
            app.variables["shared"].as_numeric().unwrap().data()
        );
        for _ in 0..3 {
            frame_at(&mut restored, &restored_ctx, size, vec![]);
        }
        for index in 0..2 {
            let _ = point(&restored, &restored_ctx, index);
            assert_eq!(restored.documents[index].text, app.documents[index].text);
        }
        assert_eq!(restored.active, 1);
        assert!(app.variables["shared"].same_snapshot(&app.variables["alias"]));
        assert_eq!(
            restored.snapshot().layout.editor_columns,
            Some(session::EditorColumns { left: 0, right: 1 })
        );
    }

    fn named_tab_position(
        app: &mut Workspace,
        ctx: &egui::Context,
        size: egui::Vec2,
    ) -> egui::Pos2 {
        let output = frame_at(app, ctx, size, vec![]);
        output
            .shapes
            .iter()
            .filter_map(|shape| text_position(&shape.shape, "left.m"))
            .min_by(|a, b| a.y.total_cmp(&b.y))
            .expect("actual left script tab")
    }

    #[test]
    fn actual_tab_focus_survives_discard_and_rejects_late_document_or_modal() {
        // 0: genuine discarded click succeeds; 1: active ID changes; 2: modal opens.
        for late in 0..3 {
            let ctx = egui::Context::default();
            let size = egui::vec2(960., 650.);
            let mut app = setup(&ctx, size);
            let dir = tempfile::tempdir().unwrap();
            save_fixture(&mut app, 0, &dir.path().join("left.m"));
            save_fixture(&mut app, 1, &dir.path().join("right.m"));
            frame_at(&mut app, &ctx, size, vec![]);
            focus(&mut app, &ctx, size, 1);
            let left_id = app.documents[0].id;
            let tab = named_tab_position(&mut app, &ctx, size);
            pointer_frame(&mut app, &ctx, size, tab, true);
            let mut passes = 0;
            let mut blocked_focus = None;
            crate::headless::run(
                &ctx,
                egui::RawInput {
                    screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, size)),
                    events: vec![
                        egui::Event::PointerMoved(tab),
                        egui::Event::PointerButton {
                            pos: tab,
                            button: egui::PointerButton::Primary,
                            pressed: false,
                            modifiers: egui::Modifiers::NONE,
                        },
                    ],
                    ..Default::default()
                },
                |root| {
                    passes += 1;
                    if passes == 1 {
                        root.ctx()
                            .request_discard("real tab click before final editor focus");
                        app.ui(root);
                        assert_eq!(
                            app.documents[app.active].id, left_id,
                            "case {late}: actual tab activation"
                        );
                        if late == 1 {
                            app.activate_script_index(1);
                        }
                        if late == 2 {
                            app.documents[0].text.push_str("% unsaved");
                            app.documents[0].changed();
                            app.request_script_close(0);
                            assert!(app.file_action_pending());
                        }
                        blocked_focus = Some(root.ctx().memory(|m| m.focused()));
                    } else if late == 0 {
                        app.ui(root);
                    } else {
                        // Complete that exact queued intent after a later state change.
                        app.finish_pane_focus(root.ctx());
                        assert_eq!(
                            root.ctx().memory(|m| m.focused()),
                            blocked_focus.unwrap(),
                            "late case {late} must not focus a stale target"
                        );
                    }
                },
            );
            assert!(passes >= 2);
            if late == 0 {
                assert_eq!(app.active, 0);
                assert!(ctx.memory(|m| m.has_focus(editor::id(left_id))));
                frame_at(
                    &mut app,
                    &ctx,
                    size,
                    vec![egui::Event::Text("% left after click".into())],
                );
                assert!(app.documents[0].text.contains("% left after click"));
                assert_eq!(app.documents[1].text, "right_value=29;\n");
            } else if late == 1 {
                assert_eq!(app.active, 1);
            } else {
                assert!(app.file_action_pending());
                assert_eq!(app.pending_close, Some(0));
            }
            assert_eq!(app.documents.len(), 2);
        }
    }

    #[test]
    fn empty_input_notifications_keep_figure_save_with_either_retained_editor_column() {
        for index in 0..2 {
            for event in [
                egui::Event::Text(String::new()),
                egui::Event::Paste(String::new()),
                egui::Event::Ime(egui::ImeEvent::Commit(String::new())),
                egui::Event::Ime(egui::ImeEvent::Preedit {
                    text: String::new(),
                    active_range_chars: None,
                }),
                egui::Event::Ime(egui::ImeEvent::DeleteSurrounding {
                    before_chars: 0,
                    after_chars: 0,
                }),
            ] {
                let ctx = egui::Context::default();
                let size = egui::vec2(1440., 900.);
                let mut app = setup(&ctx, size);
                app.run("plot([0 1],[1 0]);".into());
                finish(&mut app);
                app.layout.mode = layout::PaneMode::Split;
                for _ in 0..3 {
                    frame_at(&mut app, &ctx, size, vec![]);
                }
                focus(&mut app, &ctx, size, index);
                let before = app.documents.clone();
                // Select the existing figure without changing the simultaneous Split layout.
                app.document_save.activate(document_save::Pane::Figure);
                frame_at(&mut app, &ctx, size, vec![]);
                assert_eq!(app.generic_save_target(), document_save::Pane::Figure);
                assert!(ctx.memory(|m| m.has_focus(editor::id(before[index].id))));
                frame_at(&mut app, &ctx, size, vec![event]);
                app.document_save.intercepted = Some(Vec::new());
                for save_as in [false, true] {
                    document_save_key(&mut app, &ctx, size, save_as);
                }
                assert_eq!(
                    app.document_save.intercepted.as_deref(),
                    Some(
                        &[
                            document_save::Request::Figure {
                                figure: 1,
                                save_as: false
                            },
                            document_save::Request::Figure {
                                figure: 1,
                                save_as: true
                            },
                        ][..]
                    )
                );
                assert_eq!(app.documents.len(), before.len());
                for (a, b) in app.documents.iter().zip(&before) {
                    assert!(a.same_snapshot(b));
                }
            }
        }
    }

    #[test]
    fn meaningful_ime_and_composition_completion_keep_the_exact_column_save_target() {
        for index in 0..2 {
            let ctx = egui::Context::default();
            let size = egui::vec2(1440., 900.);
            let mut app = setup(&ctx, size);
            app.run("plot([0 1],[1 0]);".into());
            finish(&mut app);
            app.layout.mode = layout::PaneMode::Split;
            for _ in 0..3 {
                frame_at(&mut app, &ctx, size, vec![]);
            }
            focus(&mut app, &ctx, size, index);
            let id = app.documents[index].id;
            let peer = app.documents[1 - index].clone();
            // Select the existing figure without changing the simultaneous Split layout.
            app.document_save.activate(document_save::Pane::Figure);
            frame_at(&mut app, &ctx, size, vec![]);
            frame_at(
                &mut app,
                &ctx,
                size,
                vec![egui::Event::Ime(egui::ImeEvent::Preedit {
                    text: "typed".into(),
                    active_range_chars: None,
                })],
            );
            assert_eq!(app.generic_save_target(), document_save::Pane::Script);
            frame_at(
                &mut app,
                &ctx,
                size,
                vec![egui::Event::Ime(egui::ImeEvent::Commit(String::new()))],
            );
            app.document_save.intercepted = Some(Vec::new());
            document_save_key(&mut app, &ctx, size, false);
            assert_eq!(
                app.document_save.intercepted.as_deref(),
                Some(
                    &[document_save::Request::Script {
                        document: id,
                        save_as: false
                    },][..]
                )
            );
            assert_eq!(app.documents[app.active].id, id);
            assert!(app.documents[1 - index].same_snapshot(&peer));
            // Select the existing figure without changing the simultaneous Split layout.
            app.document_save.activate(document_save::Pane::Figure);
            frame_at(&mut app, &ctx, size, vec![]);
            frame_at(
                &mut app,
                &ctx,
                size,
                vec![egui::Event::Ime(egui::ImeEvent::Commit(String::new()))],
            );
            assert_eq!(app.generic_save_target(), document_save::Pane::Figure);
        }
    }
}
