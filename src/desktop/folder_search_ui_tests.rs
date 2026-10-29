mod folder_text_search_cases {
    use super::*;
    fn setup(ctx: &egui::Context, folder: &std::path::Path) -> Workspace {
        let mut app = Workspace::with_context(ctx.clone(), true);
        app.new_document("unsaved=17;\n");
        app.commit_folder(std::fs::canonicalize(folder).unwrap(), None);
        app.run("shared=[1 2;3 4];alias=shared;".into());
        finish(&mut app);
        for _ in 0..3 {
            frame_at(&mut app, ctx, egui::vec2(960., 650.), vec![]);
        }
        click_label_at(&mut app, ctx, egui::vec2(960., 650.), "Search text");
        app
    }
    fn query(app: &mut Workspace, ctx: &egui::Context, text: &str) {
        let size = egui::vec2(960., 650.);
        frame_at(app, ctx, size, vec![]);
        let response = ctx
            .read_response(egui::Id::new("folder_text_query"))
            .expect("visible query input");
        pointer_frame(app, ctx, size, response.rect.center(), true);
        pointer_frame(app, ctx, size, response.rect.center(), false);
        command_first_key(
            app,
            ctx,
            size,
            egui::Key::A,
            egui::Modifiers {
                ctrl: true,
                command: true,
                ..Default::default()
            },
        );
        frame_at(app, ctx, size, vec![egui::Event::Text(text.into())]);
        assert_eq!(app.folder_search.query, text);
    }
    fn settle(app: &mut Workspace, ctx: &egui::Context) {
        finish(app);
        for _ in 0..3 {
            frame_at(app, ctx, egui::vec2(960., 650.), vec![]);
        }
    }
    fn result_label(app: &Workspace, index: usize) -> String {
        let hit = &app.folder_search.hits()[index];
        format!(
            "{}:{}  {}",
            crate::path_label::display(hit.path.strip_prefix(&*hit.root).unwrap()),
            hit.line,
            hit.context
        )
    }

    #[test]
    fn actual_search_opens_matching_source_and_preserves_unsaved_peer_and_shared_workspace() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("target.m");
        std::fs::write(&path, "first=1;\r\nneedle=3;\rfinal=5;\n").unwrap();
        let ctx = egui::Context::default();
        let mut app = setup(&ctx, dir.path());
        let old = app.documents[0].clone();
        let shared = app.variables["shared"].clone();
        query(&mut app, &ctx, "needle");
        click_label_at(&mut app, &ctx, egui::vec2(960., 650.), "Search");
        settle(&mut app, &ctx);
        assert_eq!(app.folder_search.hits().len(), 1);
        assert_eq!(app.folder_search.hits()[0].line, 2);
        let label = result_label(&app, 0);
        click_label_at(&mut app, &ctx, egui::vec2(960., 650.), &label);
        settle(&mut app, &ctx);
        let document = &app.documents[app.active];
        assert!(matrust::document::same_path(
            document.path.as_ref().unwrap(),
            &path
        ));
        assert_eq!(document.text, "first=1;\r\nneedle=3;\rfinal=5;\n");
        let range = egui::text_edit::TextEditState::load(&ctx, editor::id(document.id))
            .unwrap()
            .cursor
            .char_range()
            .unwrap()
            .as_sorted_char_range();
        assert_eq!(
            document
                .text
                .chars()
                .skip(range.start.0)
                .take(range.end.0 - range.start.0)
                .collect::<String>(),
            "needle"
        );
        assert!(
            app.documents
                .iter()
                .find(|d| d.id == old.id)
                .unwrap()
                .same_snapshot(&old)
        );
        assert!(app.variables["shared"].same_snapshot(&shared));
        assert!(app.variables["shared"].same_snapshot(&app.variables["alias"]));
        assert!(
            !app.variables.contains_key("needle"),
            "opening a result never executes its file"
        );
    }

    #[test]
    fn changed_disk_or_dirty_existing_buffer_never_receives_a_stale_jump_or_replacement() {
        for disk_changed in [false, true] {
            let dir = tempfile::tempdir().unwrap();
            let path = dir.path().join("target.m");
            std::fs::write(&path, "needle=7;\n").unwrap();
            let ctx = egui::Context::default();
            let mut app = setup(&ctx, dir.path());
            app.open_path(path.clone());
            settle(&mut app, &ctx);
            if !disk_changed {
                app.documents[app.active]
                    .text
                    .push_str("% retained dirty\n");
                app.documents[app.active].changed();
            }
            let snapshot = app.documents.clone();
            query(&mut app, &ctx, "needle");
            click_label_at(&mut app, &ctx, egui::vec2(960., 650.), "Search");
            settle(&mut app, &ctx);
            let label = result_label(&app, 0);
            if disk_changed {
                std::fs::write(&path, "change=7;\n").unwrap();
            }
            click_label_at(&mut app, &ctx, egui::vec2(960., 650.), &label);
            settle(&mut app, &ctx);
            assert_eq!(app.documents.len(), snapshot.len());
            for (a, b) in app.documents.iter().zip(&snapshot) {
                assert!(a.same_snapshot(b));
            }
            assert!(
                app.folder_search.notice().contains(if disk_changed {
                    "changed on disk"
                } else {
                    "Editor text differs"
                }),
                "{}",
                app.folder_search.notice()
            );
        }
    }

    #[test]
    fn query_folder_and_stop_invalidate_real_worker_results_without_data_publication() {
        for invalidate in 0..3 {
            let dir = tempfile::tempdir().unwrap();
            let other = tempfile::tempdir().unwrap();
            std::fs::write(dir.path().join("target.m"), "needle=7;\n").unwrap();
            let ctx = egui::Context::default();
            let mut app = setup(&ctx, dir.path());
            let snapshot = app.documents.clone();
            query(&mut app, &ctx, "needle");
            click_label_at(&mut app, &ctx, egui::vec2(960., 650.), "Search");
            assert!(app.busy);
            match invalidate {
                0 => query(&mut app, &ctx, "different"),
                1 => app.commit_folder(std::fs::canonicalize(other.path()).unwrap(), None),
                _ => app.stop_work(),
            }
            settle(&mut app, &ctx);
            assert!(app.folder_search.hits().is_empty());
            for (a, b) in app.documents.iter().zip(&snapshot) {
                assert!(a.same_snapshot(b));
            }
            assert!(app.variables["shared"].same_snapshot(&app.variables["alias"]));
        }
    }

    #[test]
    fn late_editor_activation_prevents_search_result_from_stealing_selection() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("target.m"), "needle=7;\n").unwrap();
        let ctx = egui::Context::default();
        let mut app = setup(&ctx, dir.path());
        query(&mut app, &ctx, "needle");
        click_label_at(&mut app, &ctx, egui::vec2(960., 650.), "Search");
        settle(&mut app, &ctx);
        let label = result_label(&app, 0);
        click_label_at(&mut app, &ctx, egui::vec2(960., 650.), &label);
        app.new_document("newer=23;\n");
        let document = app.documents[app.active].id;
        settle(&mut app, &ctx);
        assert_eq!(app.documents[app.active].id, document);
        assert_eq!(app.documents.len(), 2);
        assert!(app.folder_search.notice().contains("Selection changed"));
    }

    #[test]
    fn actual_query_options_and_discarded_result_click_respect_modal_and_generation_guards() {
        for modal in [false, true] {
            let dir = tempfile::tempdir().unwrap();
            std::fs::write(dir.path().join("target.m"), "K=7;\n").unwrap();
            std::fs::create_dir(dir.path().join("child")).unwrap();
            std::fs::write(dir.path().join("child/other.txt"), "k=9;\n").unwrap();
            let ctx = egui::Context::default();
            let size = egui::vec2(960., 650.);
            let mut app = setup(&ctx, dir.path());
            query(&mut app, &ctx, "k");
            click_label_at(&mut app, &ctx, size, "Search");
            settle(&mut app, &ctx);
            assert!(app.folder_search.hits().is_empty());
            click_label_at(&mut app, &ctx, size, "Match case");
            click_label_at(&mut app, &ctx, size, "Search");
            settle(&mut app, &ctx);
            assert_eq!(app.folder_search.hits().len(), 1);
            assert!(!app.folder_search.match_case);
            click_label_at(&mut app, &ctx, size, "Include subfolders");
            click_label_at(&mut app, &ctx, size, "Search");
            settle(&mut app, &ctx);
            assert_eq!(app.folder_search.hits().len(), 2);
            assert!(app.folder_search.subfolders);
            let label = result_label(&app, 0);
            let output = frame_at(&mut app, &ctx, size, vec![]);
            let point = output
                .shapes
                .iter()
                .find_map(|s| text_position(&s.shape, &label))
                .expect("visible search result");
            pointer_frame(&mut app, &ctx, size, point, true);
            let mut passes = 0;
            let before = app.documents[0].clone();
            crate::headless::run(
                &ctx,
                egui::RawInput {
                    screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, size)),
                    events: vec![
                        egui::Event::PointerMoved(point),
                        egui::Event::PointerButton {
                            pos: point,
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
                            .request_discard("search result cannot dispatch in discarded pass");
                    }
                    app.ui(root);
                    if passes == 1 {
                        assert!(!app.busy, "discarded result click must not submit Open");
                        if modal {
                            app.request_script_close(0);
                            assert!(app.file_action_pending());
                        } else {
                            app.folder_search.query = "different".into();
                            app.cancel_folder_search();
                        }
                    }
                },
            );
            assert!(passes >= 2);
            assert!(!app.busy);
            assert_eq!(app.documents.len(), 1);
            assert!(app.documents[0].same_snapshot(&before));
            if modal {
                assert_eq!(app.pending_close, Some(0));
            } else {
                assert!(app.folder_search.hits().is_empty());
            }
        }
    }

    #[test]
    fn successful_result_release_survives_discard_and_opens_once_on_final_pass() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("target.m");
        std::fs::write(&path, "needle=7;\n").unwrap();
        let ctx = egui::Context::default();
        let size = egui::vec2(960., 650.);
        let mut app = setup(&ctx, dir.path());
        query(&mut app, &ctx, "needle");
        click_label_at(&mut app, &ctx, size, "Search");
        settle(&mut app, &ctx);
        let before = app.documents[0].clone();
        let label = result_label(&app, 0);
        let output = frame_at(&mut app, &ctx, size, vec![]);
        let point = output
            .shapes
            .iter()
            .find_map(|s| text_position(&s.shape, &label))
            .expect("visible search result");
        pointer_frame(&mut app, &ctx, size, point, true);
        let mut passes = 0;
        crate::headless::run(
            &ctx,
            egui::RawInput {
                screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, size)),
                events: vec![
                    egui::Event::PointerMoved(point),
                    egui::Event::PointerButton {
                        pos: point,
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
                        .request_discard("retain successful search result click until final pass");
                }
                app.ui(root);
                if passes == 1 {
                    assert!(!app.busy, "discarded pass must not dispatch Open");
                    assert_eq!(app.documents.len(), 1);
                }
            },
        );
        assert!(passes >= 2);
        assert!(
            app.busy,
            "final pass must dispatch exactly the retained Open"
        );
        settle(&mut app, &ctx);
        assert_eq!(app.documents.len(), 2);
        let id = app.documents[app.active].id;
        assert!(matrust::document::same_path(
            app.documents[app.active].path.as_ref().unwrap(),
            &path
        ));
        assert!(
            app.documents
                .iter()
                .find(|d| d.id == before.id)
                .unwrap()
                .same_snapshot(&before)
        );
        let range = egui::text_edit::TextEditState::load(&ctx, editor::id(id))
            .unwrap()
            .cursor
            .char_range()
            .unwrap()
            .as_sorted_char_range();
        assert_eq!(
            app.documents[app.active]
                .text
                .chars()
                .skip(range.start.0)
                .take(range.end.0 - range.start.0)
                .collect::<String>(),
            "needle"
        );
        for _ in 0..3 {
            frame_at(&mut app, &ctx, size, vec![]);
            assert!(!app.busy);
        }
        assert_eq!(app.documents.len(), 2);
        assert_eq!(app.documents[app.active].id, id);
    }

    #[test]
    fn large_dirty_buffer_is_retained_without_unbounded_final_text_verification() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("target.m");
        std::fs::write(&path, "needle=7;\n").unwrap();
        let ctx = egui::Context::default();
        let mut app = setup(&ctx, dir.path());
        app.open_path(path);
        settle(&mut app, &ctx);
        app.documents[app.active].text = "x".repeat(worker::text_search::MAX_FILE + 1);
        app.documents[app.active].changed();
        let original = app.documents[app.active].clone();
        query(&mut app, &ctx, "needle");
        click_label_at(&mut app, &ctx, egui::vec2(960., 650.), "Search");
        settle(&mut app, &ctx);
        assert_eq!(app.folder_search.hits()[0].text_len, "needle=7;\n".len());
        let label = result_label(&app, 0);
        click_label_at(&mut app, &ctx, egui::vec2(960., 650.), &label);
        settle(&mut app, &ctx);
        assert!(app.documents[app.active].same_snapshot(&original));
        assert!(app.editor.jump.is_none());
        assert!(app.folder_search.notice().contains("Editor text differs"));
    }

    #[test]
    fn local_stop_cancels_held_verified_open_and_rejects_its_late_reply() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("target.m"), "needle=7;\n").unwrap();
        let ctx = egui::Context::default();
        let size = egui::vec2(960., 650.);
        let mut app = setup(&ctx, dir.path());
        query(&mut app, &ctx, "needle");
        click_label_at(&mut app, &ctx, size, "Search");
        settle(&mut app, &ctx);
        let original = app.documents.clone();
        let active = app.documents[app.active].id;
        let (sender, requests) = std::sync::mpsc::sync_channel(1);
        let real = std::mem::replace(&mut app.worker.send, sender);
        let label = result_label(&app, 0);
        click_label_at(&mut app, &ctx, size, &label);
        let request = requests.try_recv().expect("actual typed Open request");
        assert!(matches!(&request.job, Job::OpenSearchResult(_)));
        assert!(app.busy && app.worker.opening && app.file_action_pending());
        click_label_at(&mut app, &ctx, size, "Stop search");
        assert!(app.worker.cancel.load(Ordering::Relaxed));
        assert!(app.folder_search.hits().is_empty());
        app.worker.send = real;
        app.worker.send.send(request).unwrap(); // Preserve immutable request theme.
        settle(&mut app, &ctx);
        assert!(!app.busy && !app.worker.opening);
        assert_eq!(app.documents[app.active].id, active);
        assert_eq!(app.documents.len(), original.len());
        for (actual, before) in app.documents.iter().zip(&original) {
            assert!(actual.same_snapshot(before));
        }
        assert!(app.editor.jump.is_none());
        assert!(app.variables["shared"].same_snapshot(&app.variables["alias"]));
    }

    #[test]
    fn discarded_stop_precedes_verified_reply_publication_between_passes() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("target.m"), "needle=7;\n").unwrap();
        let ctx = egui::Context::default();
        let size = egui::vec2(960., 650.);
        let mut app = setup(&ctx, dir.path());
        query(&mut app, &ctx, "needle");
        click_label_at(&mut app, &ctx, size, "Search");
        settle(&mut app, &ctx);
        let original = app.documents.clone();
        let active = app.documents[app.active].id;
        let (sender, requests) = std::sync::mpsc::sync_channel(1);
        let real = std::mem::replace(&mut app.worker.send, sender);
        let label = result_label(&app, 0);
        click_label_at(&mut app, &ctx, size, &label);
        let mut held = Some(requests.try_recv().expect("actual typed Open request"));
        app.worker.send = real;
        assert!(matches!(
            &held.as_ref().unwrap().job,
            Job::OpenSearchResult(_)
        ));
        let output = frame_at(&mut app, &ctx, size, vec![]);
        let point = output
            .shapes
            .iter()
            .find_map(|s| text_position(&s.shape, "Stop search"))
            .expect("visible local Stop");
        pointer_frame(&mut app, &ctx, size, point, true);
        let mut passes = 0;
        crate::headless::run(
            &ctx,
            egui::RawInput {
                screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, size)),
                events: vec![
                    egui::Event::PointerMoved(point),
                    egui::Event::PointerButton {
                        pos: point,
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
                        .request_discard("Stop must precede between-pass Open reply");
                }
                app.ui(root);
                if passes == 1 {
                    assert!(!app.worker.cancel.load(Ordering::Relaxed));
                    app.worker.send.send(held.take().unwrap()).unwrap();
                    finish(&mut app);
                    assert!(!app.worker.opening);
                    assert_eq!(
                        app.documents.len(),
                        1,
                        "reply remains deferred until final pass"
                    );
                }
            },
        );
        assert!(passes >= 2);
        assert!(
            !app.busy && !app.worker.opening,
            "verified reply already completed; Stop retires publication, not a running worker"
        );
        assert_eq!(app.documents.len(), original.len());
        assert_eq!(app.documents[app.active].id, active);
        for (actual, before) in app.documents.iter().zip(&original) {
            assert!(actual.same_snapshot(before));
        }
        assert!(app.editor.jump.is_none());
        assert!(app.folder_search.hits().is_empty());
        app.document_save.activate(document_save::Pane::Script);
        assert!(
            app.document_save.open_completed(),
            "cancelled deferred Open must retire its old activation token"
        );
    }

    fn focus_results_by_tab(app: &mut Workspace, ctx: &egui::Context, size: egui::Vec2) {
        // Search completion must not steal focus. setup/query/settle use the
        // compact viewport; settle the requested size before reading pointer geometry,
        // as the actual responsive launcher fixture also does after viewport changes.
        let query_id = egui::Id::new("folder_text_query");
        let mut layouts = Vec::with_capacity(3);
        for _ in 0..3 {
            frame_at(app, ctx, size, vec![]);
            layouts.push((
                ctx.content_rect(),
                ctx.read_response(query_id)
                    .map(|response| (response.rect, response.interact_rect)),
            ));
        }
        let point = ctx
            .read_response(query_id)
            .expect("visible query before local Tab traversal")
            .interact_rect
            .center();
        pointer_frame(app, ctx, size, point, true);
        pointer_frame(app, ctx, size, point, false);
        assert!(
            ctx.memory(|m| m.has_focus(query_id)),
            "query must own local Tab entry: requested={size:?}, actual={:?}, layouts={layouts:?}, clicked={point:?}, current={:?}, focused={:?}",
            ctx.content_rect(),
            ctx.read_response(query_id)
                .map(|response| (response.rect, response.interact_rect)),
            ctx.memory(|memory| memory.focused())
        );
        for _ in 0..16 {
            if ctx.memory(|m| m.has_focus(folder_search_ui::list_id())) {
                return;
            }
            command_first_key(app, ctx, size, egui::Key::Tab, egui::Modifiers::NONE);
        }
        panic!("results must be reachable through actual Tab navigation");
    }
    fn many_results() -> (tempfile::TempDir, std::path::PathBuf) {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("target.m");
        let text = (1..=40)
            .map(|n| format!("% needle {n:02}\n"))
            .collect::<String>();
        std::fs::write(&path, text).unwrap();
        (dir, path)
    }
    fn assert_result_ink(
        output: &egui::FullOutput,
        label: &str,
        ctx: &egui::Context,
        size: egui::Vec2,
    ) {
        let geometry = folder_search_ui::result_geometry(ctx);
        let trace = format!(
            "requested={size:?}, actual={:?}, geometry={geometry:?}",
            ctx.content_rect()
        );
        fn visit(
            shape: &egui::Shape,
            clip: egui::Rect,
            label: &str,
            seen: &mut usize,
            trace: &str,
        ) {
            match shape {
                egui::Shape::Text(text) if text.galley.text() == label => {
                    assert!(!text.galley.elided, "elided result {label}; {trace}");
                    assert!(
                        clip.contains_rect(text.visual_bounding_rect()),
                        "clipped result {label}: ink={:?}, clip={clip:?}; {trace}",
                        text.visual_bounding_rect()
                    );
                    *seen += 1;
                }
                egui::Shape::Vec(shapes) => {
                    for shape in shapes {
                        visit(shape, clip, label, seen, trace);
                    }
                }
                _ => {}
            }
        }
        let mut seen = 0;
        for shape in &output.shapes {
            visit(&shape.shape, shape.clip_rect, label, &mut seen, &trace);
        }
        assert_eq!(seen, 1, "selected result must be actually visible: {label}");
    }

    #[test]
    fn results_tab_arrows_home_end_pages_and_enter_reveal_and_open_exact_match() {
        for size in [egui::vec2(960., 650.), egui::vec2(1440., 900.)] {
            let (dir, path) = many_results();
            let ctx = egui::Context::default();
            let mut app = setup(&ctx, dir.path());
            query(&mut app, &ctx, "needle");
            click_label_at(&mut app, &ctx, size, "Search");
            settle(&mut app, &ctx);
            assert_eq!(app.folder_search.hits().len(), 40);
            let before = app.documents[0].clone();
            focus_results_by_tab(&mut app, &ctx, size);
            command_first_key(&mut app, &ctx, size, egui::Key::End, egui::Modifiers::NONE);
            assert_eq!(app.folder_search.selected_index(), Some(39));
            let output = frame_at(&mut app, &ctx, size, vec![]);
            assert_result_ink(&output, &result_label(&app, 39), &ctx, size);
            command_first_key(
                &mut app,
                &ctx,
                size,
                egui::Key::PageUp,
                egui::Modifiers::NONE,
            );
            let page_up = app.folder_search.selected_index().unwrap();
            assert!(page_up < 39);
            command_first_key(
                &mut app,
                &ctx,
                size,
                egui::Key::PageDown,
                egui::Modifiers::NONE,
            );
            assert_eq!(app.folder_search.selected_index(), Some(39));
            command_first_key(&mut app, &ctx, size, egui::Key::Home, egui::Modifiers::NONE);
            assert_eq!(app.folder_search.selected_index(), Some(0));
            command_first_key(
                &mut app,
                &ctx,
                size,
                egui::Key::ArrowDown,
                egui::Modifiers::NONE,
            );
            assert_eq!(app.folder_search.selected_index(), Some(1));
            command_first_key(
                &mut app,
                &ctx,
                size,
                egui::Key::ArrowUp,
                egui::Modifiers::NONE,
            );
            assert_eq!(app.folder_search.selected_index(), Some(0));
            command_first_key(&mut app, &ctx, size, egui::Key::End, egui::Modifiers::NONE);
            let hit = app.folder_search.hits()[39].clone();
            command_first_key(
                &mut app,
                &ctx,
                size,
                egui::Key::Enter,
                egui::Modifiers::NONE,
            );
            settle(&mut app, &ctx);
            let document = &app.documents[app.active];
            assert!(matrust::document::same_path(
                document.path.as_ref().unwrap(),
                &path
            ));
            let range = egui::text_edit::TextEditState::load(&ctx, editor::id(document.id))
                .unwrap()
                .cursor
                .char_range()
                .unwrap()
                .as_sorted_char_range();
            assert_eq!(
                range.start.0..range.end.0,
                matrust::editor::byte_chars(&document.text, hit.bytes)
            );
            assert!(
                app.documents
                    .iter()
                    .find(|d| d.id == before.id)
                    .unwrap()
                    .same_snapshot(&before)
            );
            assert!(app.variables["shared"].same_snapshot(&app.variables["alias"]));
        }
    }

    #[test]
    fn query_and_script_editor_keep_navigation_keys_when_results_do_not_own_focus() {
        let (dir, _) = many_results();
        let ctx = egui::Context::default();
        let size = egui::vec2(960., 650.);
        let mut app = setup(&ctx, dir.path());
        query(&mut app, &ctx, "needle");
        click_label_at(&mut app, &ctx, size, "Search");
        settle(&mut app, &ctx);
        let response = ctx
            .read_response(egui::Id::new("folder_text_query"))
            .unwrap();
        pointer_frame(&mut app, &ctx, size, response.rect.center(), true);
        pointer_frame(&mut app, &ctx, size, response.rect.center(), false);
        for key in [
            egui::Key::Home,
            egui::Key::End,
            egui::Key::ArrowDown,
            egui::Key::PageDown,
            egui::Key::Enter,
        ] {
            command_first_key(&mut app, &ctx, size, key, egui::Modifiers::NONE);
            assert_eq!(app.folder_search.selected_index(), None);
            assert!(!app.busy);
        }
        assert_eq!(app.folder_search.query, "needle");
        assert_eq!(app.documents.len(), 1);
        let id = app.documents[app.active].id;
        app.reveal_editor();
        frame_at(&mut app, &ctx, size, vec![]);
        let response = ctx
            .read_response(editor::id(id))
            .expect("real script editor");
        // The visible (clipped) editor, not its full text rect under the console.
        pointer_frame(&mut app, &ctx, size, response.interact_rect.center(), true);
        pointer_frame(&mut app, &ctx, size, response.interact_rect.center(), false);
        assert!(ctx.memory(|m| m.has_focus(editor::id(id))));
        command_first_key(&mut app, &ctx, size, egui::Key::Home, egui::Modifiers::NONE);
        frame_at(
            &mut app,
            &ctx,
            size,
            vec![egui::Event::Text("% owned ".into())],
        );
        assert!(app.documents[app.active].text.contains("% owned "));
        assert_eq!(app.folder_search.selected_index(), None);
        assert_eq!(app.documents.len(), 1);
        assert!(!app.busy);
    }

    #[test]
    fn discarded_keyboard_open_uses_original_result_and_rechecks_focus_query_and_modal() {
        for change in 0..4 {
            let (dir, path) = many_results();
            let ctx = egui::Context::default();
            let size = egui::vec2(960., 650.);
            let mut app = setup(&ctx, dir.path());
            query(&mut app, &ctx, "needle");
            click_label_at(&mut app, &ctx, size, "Search");
            settle(&mut app, &ctx);
            let original = app.documents[0].clone();
            focus_results_by_tab(&mut app, &ctx, size);
            command_first_key(&mut app, &ctx, size, egui::Key::End, egui::Modifiers::NONE);
            assert_eq!(app.folder_search.selected_index(), Some(39));
            let mut passes = 0;
            crate::headless::run(
                &ctx,
                egui::RawInput {
                    screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, size)),
                    events: vec![egui::Event::Key {
                        key: egui::Key::Enter,
                        physical_key: Some(egui::Key::Enter),
                        pressed: true,
                        repeat: false,
                        modifiers: egui::Modifiers::NONE,
                    }],
                    ..Default::default()
                },
                |root| {
                    passes += 1;
                    if passes == 1 {
                        root.ctx()
                            .request_discard("keyboard result intent needs final guards");
                    }
                    app.ui(root);
                    if passes == 1 {
                        assert!(!app.busy);
                        assert_eq!(app.documents.len(), 1);
                        match change {
                            1 => {
                                app.folder_search.query = "different".into();
                                app.cancel_folder_search();
                            }
                            2 => app.request_script_close(0),
                            3 => root.ctx().memory_mut(|m| {
                                m.request_focus(egui::Id::new("folder_text_query"))
                            }),
                            _ => {}
                        }
                    }
                },
            );
            assert!(passes >= 2);
            if change == 0 {
                assert!(app.busy);
                settle(&mut app, &ctx);
                assert_eq!(app.documents.len(), 2);
                let document = &app.documents[app.active];
                assert!(matrust::document::same_path(
                    document.path.as_ref().unwrap(),
                    &path
                ));
                let hit = &app.folder_search.hits()[39];
                let range = egui::text_edit::TextEditState::load(&ctx, editor::id(document.id))
                    .unwrap()
                    .cursor
                    .char_range()
                    .unwrap()
                    .as_sorted_char_range();
                assert_eq!(
                    range.start.0..range.end.0,
                    matrust::editor::byte_chars(&document.text, hit.bytes.clone())
                );
            } else {
                assert!(!app.busy);
                assert_eq!(app.documents.len(), 1);
                assert!(app.documents[0].same_snapshot(&original));
            }
        }
    }
}
