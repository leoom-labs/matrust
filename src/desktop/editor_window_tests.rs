mod native_editor_window_cases {
    use super::*;
    use std::{cell::RefCell, rc::Rc};

    const CTRL: egui::Modifiers = egui::Modifiers {
        ctrl: true,
        command: true,
        alt: false,
        shift: false,
        mac_cmd: false,
    };
    #[derive(Default)]
    struct Backend {
        events: Vec<egui::Event>,
        close: bool,
        focused: bool,
        documents: Vec<u64>,
        rects: Vec<egui::Rect>,
        output: Option<egui::FullOutput>,
        calls: usize,
        focus_owner: Option<egui::Id>,
    }

    // This is the real egui immediate-renderer hook with a CPU input/output
    // backend. It executes the production callback under its actual viewport
    // identity. It does not create a native OS window or claim native acceptance.
    fn install_backend(ctx: &egui::Context, size: egui::Vec2) -> Rc<RefCell<Backend>> {
        let state = Rc::new(RefCell::new(Backend {
            focused: true,
            ..Default::default()
        }));
        let backend = state.clone();
        ctx.set_embed_viewports(false);
        egui::Context::set_immediate_viewport_renderer(move |ctx, mut viewport| {
            assert_eq!(viewport.ids.this, editor_window::viewport());
            assert_eq!(viewport.ids.parent, egui::ViewportId::ROOT);
            let (events, close, focused, documents) = {
                let mut state = backend.borrow_mut();
                state.calls += 1;
                (
                    std::mem::take(&mut state.events),
                    std::mem::take(&mut state.close),
                    state.focused,
                    state.documents.clone(),
                )
            };
            let mut input = egui::RawInput {
                viewport_id: viewport.ids.this,
                screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, size)),
                events,
                ..Default::default()
            };
            let info = input.viewports.entry(viewport.ids.this).or_default();
            info.parent = Some(egui::ViewportId::ROOT);
            info.focused = Some(focused);
            if close {
                info.events.push(egui::ViewportEvent::Close);
            }
            let mut rects = Vec::new();
            let mut focus_owner = None;
            let output = crate::headless::run(ctx, input, |ui| {
                assert_eq!(ui.ctx().viewport_id(), editor_window::viewport());
                (viewport.viewport_ui_cb)(ui);
                focus_owner = ui.ctx().memory(|memory| memory.focused());
                rects = documents
                    .iter()
                    .filter_map(|id| ui.ctx().read_response(editor::id(*id)))
                    .map(|response| response.interact_rect.intersect(ui.clip_rect()))
                    .collect();
            });
            let mut state = backend.borrow_mut();
            state.rects = rects;
            state.focus_owner = focus_owner;
            state.output = Some(output);
        });
        state
    }

    fn native_frame(
        app: &mut Workspace,
        ctx: &egui::Context,
        size: egui::Vec2,
        backend: &Rc<RefCell<Backend>>,
        events: Vec<egui::Event>,
    ) {
        let modifiers = events
            .iter()
            .find_map(|event| match event {
                egui::Event::Key { modifiers, .. } => Some(*modifiers),
                _ => None,
            })
            .unwrap_or_default();
        backend.borrow_mut().events = std::iter::once(egui::Event::ModifiersChanged(modifiers))
            .chain(events)
            .collect();
        frame_at(app, ctx, size, vec![]);
    }
    fn native_key(
        app: &mut Workspace,
        ctx: &egui::Context,
        size: egui::Vec2,
        backend: &Rc<RefCell<Backend>>,
        key: egui::Key,
        modifiers: egui::Modifiers,
    ) {
        for pressed in [true, false] {
            native_frame(
                app,
                ctx,
                size,
                backend,
                vec![egui::Event::Key {
                    key,
                    physical_key: Some(key),
                    pressed,
                    repeat: false,
                    modifiers,
                }],
            );
        }
    }
    fn native_click(
        app: &mut Workspace,
        ctx: &egui::Context,
        size: egui::Vec2,
        backend: &Rc<RefCell<Backend>>,
        point: egui::Pos2,
    ) {
        for pressed in [true, false] {
            native_frame(
                app,
                ctx,
                size,
                backend,
                vec![
                    egui::Event::PointerMoved(point),
                    egui::Event::PointerButton {
                        pos: point,
                        button: egui::PointerButton::Primary,
                        pressed,
                        modifiers: egui::Modifiers::NONE,
                    },
                ],
            );
        }
    }
    fn setup(
        size: egui::Vec2,
        theme: session::DesktopTheme,
    ) -> (egui::Context, Workspace, Rc<RefCell<Backend>>) {
        let ctx = egui::Context::default();
        let mut app = Workspace::with_context(ctx.clone(), true);
        app.new_document("left_native=11;\n");
        app.new_document("right_native=29;\n");
        app.activate_script_index(0);
        app.layout.theme = Some(theme);
        app.apply_desktop_theme(&ctx);
        app.select_document_layout(layout::PaneMode::Editor);
        for _ in 0..3 {
            frame_at(&mut app, &ctx, size, vec![]);
        }
        click_label_at(&mut app, &ctx, size, "Editor actions");
        click_label_at(&mut app, &ctx, size, "Side by side");
        let backend = install_backend(&ctx, size);
        backend.borrow_mut().documents = app.documents.iter().map(|doc| doc.id).collect();
        click_label_at(&mut app, &ctx, size, "Editor actions");
        click_label_at(&mut app, &ctx, size, "Undock Editor");
        for _ in 0..3 {
            native_frame(&mut app, &ctx, size, &backend, vec![]);
        }
        assert!(app.editor_window.detached);
        assert!(backend.borrow().calls > 0);
        (ctx, app, backend)
    }

    #[test]
    fn native_editor_immediate_callback_edits_both_documents_and_docks_without_data_loss() {
        for theme in [session::DesktopTheme::Light, session::DesktopTheme::Dark] {
            for size in [egui::vec2(960., 650.), egui::vec2(1440., 900.)] {
                let (ctx, mut app, backend) = setup(size, theme);
                let ids: Vec<_> = app.documents.iter().map(|doc| doc.id).collect();
                for index in 0..2 {
                    let rect = backend.borrow().rects[index];
                    assert!(rect.width() > 100. && rect.height() > 80.);
                    native_click(&mut app, &ctx, size, &backend, rect.center());
                    assert_eq!(app.active, index);
                    native_key(&mut app, &ctx, size, &backend, egui::Key::End, CTRL);
                    let before = app.documents[index].text.clone();
                    native_frame(
                        &mut app,
                        &ctx,
                        size,
                        &backend,
                        vec![egui::Event::Text(format!("% native edit {index}\n"))],
                    );
                    assert!(
                        app.documents[index]
                            .text
                            .contains(&format!("% native edit {index}"))
                    );
                    let edited = app.documents[index].text.clone();
                    native_key(&mut app, &ctx, size, &backend, egui::Key::Z, CTRL);
                    assert_eq!(app.documents[index].text, before);
                    native_key(
                        &mut app,
                        &ctx,
                        size,
                        &backend,
                        egui::Key::Z,
                        egui::Modifiers {
                            shift: true,
                            ..CTRL
                        },
                    );
                    assert_eq!(app.documents[index].text, edited);
                    native_key(&mut app, &ctx, size, &backend, egui::Key::F, CTRL);
                    native_frame(
                        &mut app,
                        &ctx,
                        size,
                        &backend,
                        vec![egui::Event::Text(
                            if index == 0 {
                                "left_native"
                            } else {
                                "right_native"
                            }
                            .into(),
                        )],
                    );
                }
                // The empty Find field paints its hint on the first Text event pass.
                native_frame(&mut app, &ctx, size, &backend, vec![]);
                for query in ["left_native", "right_native"] {
                    assert!(
                        backend
                            .borrow()
                            .output
                            .as_ref()
                            .unwrap()
                            .shapes
                            .iter()
                            .any(|shape| text_position(&shape.shape, query).is_some()),
                        "missing query={query:?}; active={} focus={:?}; {}",
                        app.active,
                        backend.borrow().focus_owner,
                        app.editor.input_diagnostic()
                    );
                }
                let saved: Vec<_> = app.documents.clone();
                let pairing = app.editor_columns_snapshot();
                backend.borrow_mut().close = true;
                native_frame(&mut app, &ctx, size, &backend, vec![]);
                assert!(!app.editor_window.detached);
                assert_eq!(app.editor_columns_snapshot(), pairing);
                assert_eq!(
                    app.documents.iter().map(|doc| doc.id).collect::<Vec<_>>(),
                    ids
                );
                assert!(
                    app.documents
                        .iter()
                        .zip(&saved)
                        .all(|(a, b)| a.same_snapshot(b))
                );
                assert!(app.documents.iter().all(|doc| doc.dirty));
                assert!(app.pending_close.is_none() && !app.pending_quit);
                let snapshot = app.snapshot();
                assert_eq!(snapshot.documents.len(), 2);
                assert!(
                    snapshot
                        .documents
                        .iter()
                        .zip(&saved)
                        .all(|(a, b)| a.same_snapshot(b))
                );
                let recovery = tempfile::tempdir().unwrap();
                let recovery_path = recovery.path().join("editor-session.json");
                snapshot.save(&recovery_path).unwrap();
                let recovered = session::Session::load(&recovery_path).unwrap();
                assert_eq!(recovered.documents.len(), saved.len());
                for (actual, expected) in recovered.documents.iter().zip(&saved) {
                    assert_eq!(actual.text, expected.text);
                    assert_eq!(actual.path, expected.path);
                    assert_eq!(actual.dirty, expected.dirty);
                }
                assert_eq!(recovered.layout.editor_columns, pairing);
                let output = frame_at(&mut app, &ctx, size, vec![]);
                assert!(ctx.read_response(editor::id(ids[app.active])).is_some());
                for query in ["left_native", "right_native"] {
                    assert!(
                        output
                            .shapes
                            .iter()
                            .any(|shape| text_position(&shape.shape, query).is_some())
                    );
                }
            }
        }
    }

    #[test]
    fn native_editor_saved_path_run_selection_and_dirty_close_use_existing_worker_and_dialog() {
        let size = egui::vec2(960., 650.);
        let (ctx, mut app, backend) = setup(size, session::DesktopTheme::Light);
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("native_editor.m");
        assert!(!app.save_script_path(0, &path, None));
        finish(&mut app);
        app.run("shared=[1 2 3];alias=shared;".into());
        finish(&mut app);
        let shared = app.variables["shared"].clone();
        let point = backend.borrow().rects[0].center();
        native_click(&mut app, &ctx, size, &backend, point);
        native_key(&mut app, &ctx, size, &backend, egui::Key::End, CTRL);
        native_frame(
            &mut app,
            &ctx,
            size,
            &backend,
            vec![egui::Event::Text("native_saved=51;\n".into())],
        );
        let text = app.documents[0].text.clone();
        let disk_before = std::fs::read(&path).unwrap();
        native_key(
            &mut app,
            &ctx,
            size,
            &backend,
            egui::Key::S,
            egui::Modifiers { alt: true, ..CTRL },
        );
        assert_eq!(app.editor_window.dispatch_count("save"), 0);
        assert!(!app.script_io.pending());
        assert_eq!(std::fs::read(&path).unwrap(), disk_before);
        assert!(app.documents[0].dirty);
        native_key(&mut app, &ctx, size, &backend, egui::Key::S, CTRL);
        finish(&mut app);
        assert_eq!(std::fs::read_to_string(&path).unwrap(), text);
        assert!(!app.documents[0].dirty);
        assert_eq!(app.editor_window.dispatch_count("save"), 1);
        native_key(&mut app, &ctx, size, &backend, egui::Key::Enter, CTRL);
        finish(&mut app);
        assert!(
            matches!(app.variables["native_saved"].scalar(0,0).unwrap(), matrust::Scalar::Real(value) if value == 51.)
        );
        assert!(app.variables["shared"].same_snapshot(&shared));
        assert!(app.variables["alias"].same_snapshot(&shared));
        // Real TextEdit selection; F9 must not execute the unselected prefix.
        // The preceding worker operation need not retain keyboard focus. Establish
        // the real buffer focus explicitly before constructing this new selection.
        native_frame(&mut app, &ctx, size, &backend, vec![]);
        let point = backend.borrow().rects[0].center();
        native_click(&mut app, &ctx, size, &backend, point);
        assert_eq!(
            backend.borrow().focus_owner,
            Some(editor::id(app.documents[0].id))
        );
        native_key(&mut app, &ctx, size, &backend, egui::Key::A, CTRL);
        native_frame(
            &mut app,
            &ctx,
            size,
            &backend,
            vec![egui::Event::Text(
                "not_selected=88;\nselected_only=19;".into(),
            )],
        );
        assert_eq!(app.documents[0].text, "not_selected=88;\nselected_only=19;");
        native_key(&mut app, &ctx, size, &backend, egui::Key::End, CTRL);
        native_key(
            &mut app,
            &ctx,
            size,
            &backend,
            egui::Key::Home,
            egui::Modifiers {
                shift: true,
                ..Default::default()
            },
        );
        let (selected_document, selected_range) =
            app.selection.as_ref().expect("real keyboard selection");
        assert_eq!(*selected_document, 0);
        assert_eq!(
            session::selected_text(&app.documents[0].text, selected_range.clone()).unwrap(),
            "selected_only=19;"
        );
        native_key(
            &mut app,
            &ctx,
            size,
            &backend,
            egui::Key::F9,
            egui::Modifiers::NONE,
        );
        finish(&mut app);
        assert!(
            app.variables.contains_key("selected_only"),
            "F9 missing result: active={} focus={:?} selection={:?} text={:?} cursor={:?} dispatches={} status={:?} console={:?}",
            app.active,
            backend.borrow().focus_owner,
            app.selection,
            app.documents[app.active].text,
            egui::text_edit::TextEditState::load(&ctx, editor::id(app.documents[app.active].id))
                .and_then(|state| state.cursor.char_range()),
            app.editor_window.dispatch_count("selection"),
            app.status,
            app.console
        );
        assert!(
            matches!(app.variables["selected_only"].scalar(0,0).unwrap(), matrust::Scalar::Real(value) if value == 19.)
        );
        assert!(!app.variables.contains_key("not_selected"));
        let before = app.documents[0].clone();
        native_key(&mut app, &ctx, size, &backend, egui::Key::W, CTRL);
        assert_eq!(app.pending_close, Some(0));
        backend.borrow_mut().focused = false;
        for _ in 0..3 {
            frame_at(&mut app, &ctx, size, vec![]);
        }
        click_label_at(&mut app, &ctx, size, "Cancel");
        assert!(app.pending_close.is_none());
        assert!(app.documents[0].same_snapshot(&before));
        assert!(app.editor_window.detached);
    }

    #[test]
    fn native_editor_discarded_run_is_once_and_late_owner_stop_or_close_rejects() {
        for change in 0..5 {
            let size = egui::vec2(960., 650.);
            let (ctx, mut app, backend) = setup(size, session::DesktopTheme::Dark);
            let point = backend.borrow().rects[0].center();
            native_click(&mut app, &ctx, size, &backend, point);
            let before = app.editor_window.dispatch_count("run");
            let mut input = egui::RawInput {
                viewport_id: editor_window::viewport(),
                screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, size)),
                events: vec![
                    egui::Event::ModifiersChanged(CTRL),
                    egui::Event::Key {
                        key: egui::Key::Enter,
                        physical_key: Some(egui::Key::Enter),
                        pressed: true,
                        repeat: false,
                        modifiers: CTRL,
                    },
                ],
                ..Default::default()
            };
            input
                .viewports
                .entry(editor_window::viewport())
                .or_default()
                .focused = Some(true);
            let mut passes = 0;
            crate::headless::run(&ctx, input, |ui| {
                passes += 1;
                if passes == 1 {
                    ui.ctx()
                        .request_discard("native Editor exact action ticket");
                }
                app.editor_window.in_callback = true;
                app.editor_window_ui(ui);
                app.editor_window.in_callback = false;
                if passes == 1 {
                    assert_eq!(app.editor_window.dispatch_count("run"), before);
                    match change {
                        1 => app.activate_script_index(1),
                        2 => app.stop_work(),
                        3 => app.pending_close = Some(0),
                        4 => app.document_save.activate(document_save::Pane::Figure),
                        _ => {}
                    }
                }
            });
            assert!(passes >= 2);
            assert_eq!(
                app.editor_window.dispatch_count("run"),
                before + usize::from(change == 0)
            );
            if change == 0 {
                finish(&mut app);
                assert!(app.variables.contains_key("left_native"));
            } else {
                assert!(!app.variables.contains_key("left_native"));
            }
        }
    }

    #[test]
    fn native_editor_discarded_save_publishes_once_and_rejects_changed_authority() {
        for change in 0..4 {
            let size = egui::vec2(960., 650.);
            let (ctx, mut app, backend) = setup(size, session::DesktopTheme::Light);
            let dir = tempfile::tempdir().unwrap();
            let path = dir.path().join("discarded-native.m");
            assert!(!app.save_script_path(0, &path, None));
            finish(&mut app);
            let original = std::fs::read(&path).unwrap();
            let point = backend.borrow().rects[0].center();
            native_click(&mut app, &ctx, size, &backend, point);
            native_key(&mut app, &ctx, size, &backend, egui::Key::End, CTRL);
            native_frame(
                &mut app,
                &ctx,
                size,
                &backend,
                vec![egui::Event::Text(
                    "% saved only by admitted child ticket\n".into(),
                )],
            );
            let updated = app.documents[0].text.clone();
            let id = app.documents[0].id;
            let before = app.editor_window.dispatch_count("save");
            let mut input = egui::RawInput {
                viewport_id: editor_window::viewport(),
                screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, size)),
                events: vec![
                    egui::Event::ModifiersChanged(CTRL),
                    egui::Event::Key {
                        key: egui::Key::S,
                        physical_key: Some(egui::Key::S),
                        pressed: true,
                        repeat: false,
                        modifiers: CTRL,
                    },
                ],
                ..Default::default()
            };
            input
                .viewports
                .entry(editor_window::viewport())
                .or_default()
                .focused = Some(true);
            let mut passes = 0;
            crate::headless::run(&ctx, input, |ui| {
                passes += 1;
                if passes == 1 {
                    ui.ctx()
                        .request_discard("native Save publication authority");
                }
                app.editor_window.in_callback = true;
                app.editor_window_ui(ui);
                app.editor_window.in_callback = false;
                if passes == 1 {
                    assert_eq!(std::fs::read(&path).unwrap(), original);
                    assert!(!app.script_io.pending());
                    assert_eq!(app.editor_window.dispatch_count("save"), before);
                    match change {
                        1 => app.activate_script_index(1),
                        2 => app.stop_work(),
                        3 => app.document_save.activate(document_save::Pane::Figure),
                        _ => {}
                    }
                }
            });
            assert!(passes >= 2);
            assert_eq!(
                app.editor_window.dispatch_count("save"),
                before + usize::from(change == 0)
            );
            finish(&mut app);
            assert_eq!(app.documents[0].id, id);
            assert_eq!(app.documents[0].text, updated);
            if change == 0 {
                assert_eq!(std::fs::read(&path).unwrap(), updated.as_bytes());
                assert!(!app.documents[0].dirty);
            } else {
                assert_eq!(std::fs::read(&path).unwrap(), original);
                assert!(app.documents[0].dirty);
            }
        }
    }

    #[test]
    fn native_editor_close_event_survives_discard_without_closing_dirty_script_or_root_prompt() {
        let size = egui::vec2(960., 650.);
        let (ctx, mut app, backend) = setup(size, session::DesktopTheme::Dark);
        let point = backend.borrow().rects[0].center();
        native_click(&mut app, &ctx, size, &backend, point);
        native_key(&mut app, &ctx, size, &backend, egui::Key::W, CTRL);
        assert_eq!(app.pending_close, Some(0));
        let documents = app.documents.clone();
        let pairing = app.editor_columns_snapshot();
        let mut input = egui::RawInput {
            viewport_id: editor_window::viewport(),
            screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, size)),
            ..Default::default()
        };
        let info = input
            .viewports
            .entry(editor_window::viewport())
            .or_default();
        info.focused = Some(true);
        info.events.push(egui::ViewportEvent::Close);
        let mut passes = 0;
        crate::headless::run(&ctx, input, |ui| {
            passes += 1;
            if passes == 1 {
                ui.ctx()
                    .request_discard("native close event must survive layout retry");
            } else {
                assert!(!ui.input(|input| input.viewport().close_requested()));
            }
            app.editor_window.in_callback = true;
            app.editor_window_ui(ui);
            app.editor_window.in_callback = false;
            if passes == 1 {
                assert!(app.editor_window.detached);
            }
            assert_eq!(app.pending_close, Some(0));
        });
        assert!(passes >= 2);
        assert!(!app.editor_window.detached);
        assert_eq!(app.editor_columns_snapshot(), pairing);
        assert_eq!(app.documents.len(), documents.len());
        for (actual, original) in app.documents.iter().zip(&documents) {
            assert!(actual.same_snapshot(original));
        }
        assert_eq!(app.pending_close, Some(0));
        for _ in 0..3 {
            frame_at(&mut app, &ctx, size, vec![]);
        }
        click_label_at(&mut app, &ctx, size, "Cancel");
        assert!(app.pending_close.is_none());
        assert!(app.documents[0].same_snapshot(&documents[0]));
    }

    #[test]
    fn native_editor_manual_scroll_and_cursor_keep_the_same_state_across_both_hosts() {
        let size = egui::vec2(960., 650.);
        let (ctx, mut app, backend) = setup(size, session::DesktopTheme::Dark);
        let point = backend.borrow().rects[0].center();
        native_click(&mut app, &ctx, size, &backend, point);
        native_key(&mut app, &ctx, size, &backend, egui::Key::End, CTRL);
        let lines = (0..200)
            .map(|line| format!("% retained scroll line {line}\n"))
            .collect::<String>();
        native_frame(
            &mut app,
            &ctx,
            size,
            &backend,
            vec![egui::Event::Text(lines)],
        );
        native_key(&mut app, &ctx, size, &backend, egui::Key::Home, CTRL);
        let point = backend.borrow().rects[0].center();
        native_frame(
            &mut app,
            &ctx,
            size,
            &backend,
            vec![
                egui::Event::PointerMoved(point),
                egui::Event::MouseWheel {
                    unit: egui::MouseWheelUnit::Point,
                    delta: egui::vec2(0., -400.),
                    modifiers: egui::Modifiers::NONE,
                    phase: egui::TouchPhase::Move,
                },
            ],
        );
        native_frame(
            &mut app,
            &ctx,
            size,
            &backend,
            vec![egui::Event::MouseWheel {
                unit: egui::MouseWheelUnit::Point,
                delta: egui::Vec2::ZERO,
                modifiers: egui::Modifiers::NONE,
                phase: egui::TouchPhase::End,
            }],
        );
        for _ in 0..3 {
            native_frame(&mut app, &ctx, size, &backend, vec![]);
        }
        let document = app.documents[0].id;
        // UiBuilder and ScrollArea wrap their salts before deriving persistent IDs.
        // load must find the state created by the real ScrollArea, never seed it.
        let scroll = editor_window::group_id()
            .with(egui::IdSalt::new(("script_column", 0usize)))
            .with(egui::IdSalt::new(("script_scroll", document)));
        let before = egui::scroll_area::State::load(&ctx, scroll)
            .expect("real shared Editor scroll")
            .offset;
        assert!(
            before.y > 100. && before.y < 1000.,
            "interior manual offset: {before:?}"
        );
        let cursor = egui::text_edit::TextEditState::load(&ctx, editor::id(document))
            .unwrap()
            .cursor
            .char_range()
            .unwrap()
            .as_sorted_char_range();
        let text = app.documents[0].text.clone();
        backend.borrow_mut().close = true;
        native_frame(&mut app, &ctx, size, &backend, vec![]);
        for _ in 0..3 {
            frame_at(&mut app, &ctx, size, vec![]);
        }
        assert!(!app.editor_window.detached);
        let docked = egui::scroll_area::State::load(&ctx, scroll).unwrap().offset;
        assert!(
            (docked - before).length() < 0.1,
            "manual scroll changed on dock: {before:?} -> {docked:?}"
        );
        click_label_at(&mut app, &ctx, size, "Editor actions");
        click_label_at(&mut app, &ctx, size, "Undock Editor");
        for _ in 0..3 {
            native_frame(&mut app, &ctx, size, &backend, vec![]);
        }
        let detached = egui::scroll_area::State::load(&ctx, scroll).unwrap().offset;
        assert!(
            (detached - before).length() < 0.1,
            "manual scroll changed on undock: {before:?} -> {detached:?}"
        );
        let after = egui::text_edit::TextEditState::load(&ctx, editor::id(document))
            .unwrap()
            .cursor
            .char_range()
            .unwrap()
            .as_sorted_char_range();
        assert_eq!(after, cursor);
        assert_eq!(app.documents[0].text, text);
    }

    #[test]
    fn native_editor_foreground_new_open_and_find_keep_the_intended_focus_owner() {
        let size = egui::vec2(960., 650.);
        let (ctx, mut app, backend) = setup(size, session::DesktopTheme::Light);
        let old_ids = app.documents.iter().map(|doc| doc.id).collect::<Vec<_>>();
        native_key(&mut app, &ctx, size, &backend, egui::Key::N, CTRL);
        let created = app.documents[app.active].id;
        assert!(!old_ids.contains(&created));
        backend.borrow_mut().documents = app.documents.iter().map(|doc| doc.id).collect();
        native_frame(
            &mut app,
            &ctx,
            size,
            &backend,
            vec![egui::Event::Text("new_window_target=73;".into())],
        );
        assert_eq!(app.documents[app.active].text, "new_window_target=73;");
        assert_eq!(app.documents[0].text, "left_native=11;\n");
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("foreground_window.m");
        std::fs::write(&path, "opened_window_target=91;\n").unwrap();
        let canonical_path = std::fs::canonicalize(&path).unwrap();
        app.open_path(path.clone());
        finish(&mut app);
        assert_eq!(
            app.documents[app.active].path.as_deref(),
            Some(canonical_path.as_path())
        );
        let opened = app.documents[app.active].id;
        backend.borrow_mut().documents = app.documents.iter().map(|doc| doc.id).collect();
        for _ in 0..3 {
            native_frame(&mut app, &ctx, size, &backend, vec![]);
        }
        assert_eq!(backend.borrow().focus_owner, Some(editor::id(opened)));
        native_key(&mut app, &ctx, size, &backend, egui::Key::End, CTRL);
        native_frame(
            &mut app,
            &ctx,
            size,
            &backend,
            vec![egui::Event::Text("% correct foreground target\n".into())],
        );
        let before_find = app.documents[app.active].text.clone();
        // Root Find must raise the existing child query, not subsequently
        // replace its focus with the generic foreground document ticket.
        command_first_key(&mut app, &ctx, size, egui::Key::F, CTRL);
        native_frame(
            &mut app,
            &ctx,
            size,
            &backend,
            vec![egui::Event::Text("opened_window_target".into())],
        );
        // egui requests a repaint after replacing the empty-field hint.
        native_frame(&mut app, &ctx, size, &backend, vec![]);
        assert_eq!(app.documents[app.active].text, before_find);
        assert!(
            backend
                .borrow()
                .output
                .as_ref()
                .unwrap()
                .shapes
                .iter()
                .any(|shape| text_position(&shape.shape, "opened_window_target").is_some())
        );
        for stop in [false, true] {
            app.reveal_editor();
            if stop {
                app.stop_work();
            } else {
                app.document_save.command_selected();
            }
            native_frame(&mut app, &ctx, size, &backend, vec![]);
            assert_ne!(backend.borrow().focus_owner, Some(editor::id(opened)));
        }
    }

    #[test]
    fn native_editor_find_supersedes_a_queued_generic_document_focus() {
        let size = egui::vec2(960., 650.);
        let (ctx, mut app, backend) = setup(size, session::DesktopTheme::Dark);
        let point = backend.borrow().rects[0].center();
        native_click(&mut app, &ctx, size, &backend, point);
        let document = app.documents[app.active].id;
        let text = app.documents[app.active].text.clone();
        // Same path as root Focus Editor: retain a generic request until child paint.
        app.queue_editor_window_focus(&ctx);
        native_key(&mut app, &ctx, size, &backend, egui::Key::F, CTRL);
        native_frame(
            &mut app,
            &ctx,
            size,
            &backend,
            vec![egui::Event::Text("FIND_ONLY_SENTINEL".into())],
        );
        // Inspect the requested next paint, preserving exact query/focus outcomes.
        native_frame(&mut app, &ctx, size, &backend, vec![]);
        assert_eq!(app.documents[app.active].id, document);
        assert_eq!(app.documents[app.active].text, text);
        assert_ne!(backend.borrow().focus_owner, Some(editor::id(document)));
        assert!(
            backend
                .borrow()
                .output
                .as_ref()
                .unwrap()
                .shapes
                .iter()
                .any(|shape| text_position(&shape.shape, "FIND_ONLY_SENTINEL").is_some()),
            "focus={:?}; {}",
            backend.borrow().focus_owner,
            app.editor.input_diagnostic()
        );
    }

    fn tab_order_label(
        app: &mut Workspace,
        ctx: &egui::Context,
        size: egui::Vec2,
        backend: &Rc<RefCell<Backend>>,
        detached: bool,
        label: &str,
    ) {
        if detached {
            for _ in 0..3 {
                native_frame(app, ctx, size, backend, vec![]);
            }
            let point = backend
                .borrow()
                .output
                .as_ref()
                .unwrap()
                .shapes
                .iter()
                .rev()
                .find_map(|shape| text_position(&shape.shape, label))
                .unwrap();
            native_click(app, ctx, size, backend, point);
        } else {
            click_label_at(app, ctx, size, label);
        }
    }

    #[test]
    fn tab_order_real_menus_preserve_both_editors_selection_storage_and_recovery() {
        for detached in [false, true] {
            for theme in [session::DesktopTheme::Light, session::DesktopTheme::Dark] {
                let size = egui::vec2(960., 650.);
                let (ctx, mut app, backend) = setup(size, theme);
                app.new_document("move_value=83;\n");
                backend.borrow_mut().documents = app.documents.iter().map(|doc| doc.id).collect();
                for _ in 0..3 {
                    native_frame(&mut app, &ctx, size, &backend, vec![]);
                }
                if !detached {
                    backend.borrow_mut().close = true;
                    native_frame(&mut app, &ctx, size, &backend, vec![]);
                    for _ in 0..3 {
                        frame_at(&mut app, &ctx, size, vec![]);
                    }
                }
                let id = app.documents[app.active].id;
                let original_ids = app.documents.iter().map(|doc| doc.id).collect::<Vec<_>>();
                assert_eq!(app.active, 2);
                let original = app.documents.clone();
                let allocations = app
                    .documents
                    .iter()
                    .map(|doc| (doc.id, doc.text.as_ptr() as usize))
                    .collect::<std::collections::BTreeMap<_, _>>();
                let pairing = app.editor_columns_snapshot().unwrap();
                let paired_ids = [
                    app.documents[pairing.left].id,
                    app.documents[pairing.right].id,
                ];
                // Select actual text, then open Find in the same view. Neither
                // the selection nor per-view find state belongs to a tab index.
                if detached {
                    native_key(&mut app, &ctx, size, &backend, egui::Key::A, CTRL);
                    native_key(&mut app, &ctx, size, &backend, egui::Key::F, CTRL);
                    native_frame(
                        &mut app,
                        &ctx,
                        size,
                        &backend,
                        vec![egui::Event::Text("ORDER_FIND".into())],
                    );
                } else {
                    command_first_key(
                        &mut app,
                        &ctx,
                        size,
                        egui::Key::Num0,
                        egui::Modifiers {
                            shift: true,
                            ..CTRL
                        },
                    );
                    command_first_key(&mut app, &ctx, size, egui::Key::A, CTRL);
                    command_first_key(&mut app, &ctx, size, egui::Key::F, CTRL);
                    frame_at(
                        &mut app,
                        &ctx,
                        size,
                        vec![egui::Event::Text("ORDER_FIND".into())],
                    );
                }
                let selection = app.selection.clone().expect("real selected script text");
                let cursor = egui::text_edit::TextEditState::load(&ctx, editor::id(id))
                    .unwrap()
                    .cursor
                    .char_range()
                    .unwrap()
                    .as_sorted_char_range();
                tab_order_label(&mut app, &ctx, size, &backend, detached, "Tabs");
                tab_order_label(&mut app, &ctx, size, &backend, detached, "Move tab left");
                assert_eq!(
                    app.documents.iter().map(|doc| doc.id).collect::<Vec<_>>(),
                    [original_ids[0], original_ids[2], original_ids[1]]
                );
                assert_eq!(app.active, 1);
                assert_eq!(app.documents[app.active].id, id);
                assert_eq!(app.selection, Some((1, selection.1)));
                let columns = app.editor_columns_snapshot().unwrap();
                assert_eq!(
                    [
                        app.documents[columns.left].id,
                        app.documents[columns.right].id
                    ],
                    paired_ids
                );
                for doc in &app.documents {
                    assert_eq!(doc.text.as_ptr() as usize, allocations[&doc.id]);
                    assert!(
                        doc.same_snapshot(original.iter().find(|old| old.id == doc.id).unwrap())
                    );
                }
                assert_eq!(
                    egui::text_edit::TextEditState::load(&ctx, editor::id(id))
                        .unwrap()
                        .cursor
                        .char_range()
                        .unwrap()
                        .as_sorted_char_range(),
                    cursor
                );
                let output = if detached {
                    native_frame(&mut app, &ctx, size, &backend, vec![]);
                    backend.borrow().output.as_ref().unwrap().shapes.clone()
                } else {
                    frame_at(&mut app, &ctx, size, vec![]).shapes
                };
                assert!(
                    output
                        .iter()
                        .any(|shape| text_position(&shape.shape, "ORDER_FIND").is_some())
                );
                let dir = tempfile::tempdir().unwrap();
                let path = dir.path().join("tab-order-session.json");
                let snapshot = app.snapshot();
                snapshot.save(&path).unwrap();
                let saved = session::Session::load(&path).unwrap();
                assert_eq!(saved.active, app.active);
                assert_eq!(saved.layout.editor_columns, app.editor_columns_snapshot());
                assert_eq!(
                    saved
                        .documents
                        .iter()
                        .map(|doc| &doc.text)
                        .collect::<Vec<_>>(),
                    app.documents
                        .iter()
                        .map(|doc| &doc.text)
                        .collect::<Vec<_>>()
                );
                tab_order_label(&mut app, &ctx, size, &backend, detached, "Tabs");
                tab_order_label(&mut app, &ctx, size, &backend, detached, "Move tab right");
                assert_eq!(
                    app.documents.iter().map(|doc| doc.id).collect::<Vec<_>>(),
                    original_ids
                );
                assert_eq!(app.active, 2);
            }
        }
    }

    #[test]
    fn tab_order_after_move_save_and_run_still_target_the_same_document() {
        let size = egui::vec2(960., 650.);
        let (ctx, mut app, backend) = setup(size, session::DesktopTheme::Light);
        app.new_document("reordered_value=97;\n");
        let id = app.documents[app.active].id;
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("reordered.m");
        assert!(!app.save_script_path(app.active, &path, None));
        finish(&mut app);
        backend.borrow_mut().documents = app.documents.iter().map(|doc| doc.id).collect();
        for _ in 0..3 {
            native_frame(&mut app, &ctx, size, &backend, vec![]);
        }
        tab_order_label(&mut app, &ctx, size, &backend, true, "Tabs");
        tab_order_label(&mut app, &ctx, size, &backend, true, "Move tab left");
        assert_eq!(app.active, 1);
        assert_eq!(app.documents[app.active].id, id);
        native_key(
            &mut app,
            &ctx,
            size,
            &backend,
            egui::Key::Num0,
            egui::Modifiers {
                shift: true,
                ..CTRL
            },
        );
        native_key(&mut app, &ctx, size, &backend, egui::Key::End, CTRL);
        native_frame(
            &mut app,
            &ctx,
            size,
            &backend,
            vec![egui::Event::Text("% saved after reorder\n".into())],
        );
        let expected = app.documents[app.active].text.clone();
        native_key(&mut app, &ctx, size, &backend, egui::Key::S, CTRL);
        finish(&mut app);
        assert_eq!(std::fs::read_to_string(&path).unwrap(), expected);
        assert!(!app.documents[app.active].dirty);
        native_key(&mut app, &ctx, size, &backend, egui::Key::Enter, CTRL);
        finish(&mut app);
        assert!(
            matches!(app.variables["reordered_value"].scalar(0, 0).unwrap(),
            matrust::Scalar::Real(value) if value == 97.)
        );
        assert_eq!(app.documents[0].text, "left_native=11;\n");
        assert_eq!(app.documents[2].text, "right_native=29;\n");
    }

    #[test]
    fn tab_order_discarded_menu_release_is_once_and_rejects_late_authority() {
        for invalidate in 0..5 {
            let size = egui::vec2(960., 650.);
            let (ctx, mut app, backend) = setup(size, session::DesktopTheme::Dark);
            app.new_document("third_order=3;\n");
            for _ in 0..3 {
                native_frame(&mut app, &ctx, size, &backend, vec![]);
            }
            let before = app.documents.iter().map(|doc| doc.id).collect::<Vec<_>>();
            tab_order_label(&mut app, &ctx, size, &backend, true, "Tabs");
            for _ in 0..3 {
                native_frame(&mut app, &ctx, size, &backend, vec![]);
            }
            let point = backend
                .borrow()
                .output
                .as_ref()
                .unwrap()
                .shapes
                .iter()
                .rev()
                .find_map(|shape| text_position(&shape.shape, "Move tab left"))
                .unwrap();
            native_frame(
                &mut app,
                &ctx,
                size,
                &backend,
                vec![
                    egui::Event::PointerMoved(point),
                    egui::Event::PointerButton {
                        pos: point,
                        button: egui::PointerButton::Primary,
                        pressed: true,
                        modifiers: egui::Modifiers::NONE,
                    },
                ],
            );
            let mut input = egui::RawInput {
                viewport_id: editor_window::viewport(),
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
            };
            input
                .viewports
                .entry(editor_window::viewport())
                .or_default()
                .focused = Some(true);
            let mut passes = 0;
            crate::headless::run(&ctx, input, |ui| {
                passes += 1;
                if passes == 1 {
                    ui.ctx().request_discard("retain real tab menu release");
                }
                app.editor_window.in_callback = true;
                app.editor_window_ui(ui);
                app.editor_window.in_callback = false;
                if passes == 1 {
                    assert_eq!(
                        app.documents.iter().map(|doc| doc.id).collect::<Vec<_>>(),
                        before
                    );
                    match invalidate {
                        1 => app.stop_work(),
                        2 => app.activate_script_index(0),
                        3 => app.pending_close = Some(2),
                        4 => app.document_save.command_selected(),
                        _ => {}
                    }
                }
            });
            assert!(passes >= 2);
            let expected = if invalidate == 0 {
                vec![before[0], before[2], before[1]]
            } else {
                before.clone()
            };
            assert_eq!(
                app.documents.iter().map(|doc| doc.id).collect::<Vec<_>>(),
                expected
            );
            app.pending_close = None;
            for _ in 0..3 {
                native_frame(&mut app, &ctx, size, &backend, vec![]);
            }
            assert_eq!(
                app.documents.iter().map(|doc| doc.id).collect::<Vec<_>>(),
                expected,
                "a retired move must not replay in a later frame"
            );
        }
    }

    fn drag_frame(
        app: &mut Workspace,
        ctx: &egui::Context,
        size: egui::Vec2,
        backend: &Rc<RefCell<Backend>>,
        detached: bool,
        events: Vec<egui::Event>,
    ) {
        if detached {
            native_frame(app, ctx, size, backend, events);
        } else {
            frame_at(app, ctx, size, events);
        }
    }
    fn drag_tab(app: &Workspace, document: u64) -> editor_columns::TabDragObservation {
        let tab = app
            .script_tab_drag_observations()
            .into_iter()
            .flatten()
            .find(|tab| tab.document == document)
            .expect("actual stable-ID tab response");
        assert!(
            tab.frame > 0,
            "observation comes from an actual painted frame"
        );
        tab
    }
    // Match the enabled Glow renderer's clip rounding and target clamping.
    // Pointer admission still uses the logical clip; this measures painted visibility.
    fn drag_painted_clip(tab: editor_columns::TabDragObservation, size: egui::Vec2) -> egui::Rect {
        let ppp = tab.pixels_per_point;
        assert!(ppp.is_finite() && ppp > 0.);
        let width = (size.x * ppp).round() as i32;
        let height = (size.y * ppp).round() as i32;
        let min_x = ((tab.clip.min.x * ppp).round() as i32).clamp(0, width);
        let min_y = ((tab.clip.min.y * ppp).round() as i32).clamp(0, height);
        let max_x = ((tab.clip.max.x * ppp).round() as i32).clamp(min_x, width);
        let max_y = ((tab.clip.max.y * ppp).round() as i32).clamp(min_y, height);
        egui::Rect::from_min_max(
            egui::pos2(min_x as f32 / ppp, min_y as f32 / ppp),
            egui::pos2(max_x as f32 / ppp, max_y as f32 / ppp),
        )
    }

    fn drag_press(
        app: &mut Workspace,
        ctx: &egui::Context,
        size: egui::Vec2,
        backend: &Rc<RefCell<Backend>>,
        detached: bool,
        document: u64,
    ) {
        let tab = drag_tab(app, document);
        let point = tab.rect.center();
        assert!(
            tab.clip.contains(point),
            "source tab must actually be visible: {tab:?}"
        );
        drag_frame(
            app,
            ctx,
            size,
            backend,
            detached,
            vec![
                egui::Event::PointerMoved(point),
                egui::Event::PointerButton {
                    pos: point,
                    button: egui::PointerButton::Primary,
                    pressed: true,
                    modifiers: egui::Modifiers::NONE,
                },
            ],
        );
        let press_observation = drag_tab(app, document);
        let press_state = app.script_tab_drag_diagnostic(document);
        drag_frame(
            app,
            ctx,
            size,
            backend,
            detached,
            vec![egui::Event::PointerMoved(point + egui::vec2(12., 0.))],
        );
        let threshold_observation = drag_tab(app, document);
        let threshold_state = app.script_tab_drag_diagnostic(document);
        assert!(
            egui::DragAndDrop::has_any_payload(ctx),
            "real drag threshold must admit payload: document={document}, detached={detached}, before={tab:?}, press={press_observation:?}, press_state={press_state}, threshold={threshold_observation:?}, threshold_state={threshold_state}"
        );
    }
    fn drag_release(
        app: &mut Workspace,
        ctx: &egui::Context,
        size: egui::Vec2,
        backend: &Rc<RefCell<Backend>>,
        detached: bool,
        target: u64,
        before: bool,
    ) {
        let tab = drag_tab(app, target);
        let point = egui::pos2(
            if before {
                tab.rect.left() + 3.
            } else {
                tab.rect.right() - 3.
            },
            tab.rect.center().y,
        );
        assert!(
            tab.clip.contains(point),
            "drop boundary must actually be visible: {tab:?}"
        );
        drag_frame(
            app,
            ctx,
            size,
            backend,
            detached,
            vec![egui::Event::PointerMoved(point)],
        );
        drag_frame(
            app,
            ctx,
            size,
            backend,
            detached,
            vec![
                egui::Event::PointerMoved(point),
                egui::Event::PointerButton {
                    pos: point,
                    button: egui::PointerButton::Primary,
                    pressed: false,
                    modifiers: egui::Modifiers::NONE,
                },
            ],
        );
    }

    #[test]
    fn tab_drag_actual_pointer_preserves_active_pair_buffers_cursor_and_recovery_in_both_hosts() {
        for theme in [session::DesktopTheme::Light, session::DesktopTheme::Dark] {
            for detached in [false, true] {
                let size = egui::vec2(960., 650.);
                let (ctx, mut app, backend) = setup(size, theme);
                app.new_document("third_drag=31;\n");
                for _ in 0..3 {
                    native_frame(&mut app, &ctx, size, &backend, vec![]);
                }
                if !detached {
                    backend.borrow_mut().close = true;
                    native_frame(&mut app, &ctx, size, &backend, vec![]);
                    for _ in 0..3 {
                        frame_at(&mut app, &ctx, size, vec![]);
                    }
                }
                let ids = app.documents.iter().map(|doc| doc.id).collect::<Vec<_>>();
                let active = app.documents[app.active].id;
                let pair = app.editor_columns_snapshot().unwrap();
                let paired = [app.documents[pair.left].id, app.documents[pair.right].id];
                let old = app.documents.clone();
                let pointers = app
                    .documents
                    .iter()
                    .map(|doc| (doc.id, doc.text.as_ptr() as usize))
                    .collect::<std::collections::BTreeMap<_, _>>();
                let cursor = egui::text_edit::TextEditState::load(&ctx, editor::id(active))
                    .unwrap()
                    .cursor
                    .char_range();
                // Move an INACTIVE first tab after the last; dragging must not select it.
                drag_press(&mut app, &ctx, size, &backend, detached, ids[0]);
                drag_release(&mut app, &ctx, size, &backend, detached, ids[2], false);
                assert_eq!(
                    app.documents.iter().map(|doc| doc.id).collect::<Vec<_>>(),
                    [ids[1], ids[2], ids[0]]
                );
                assert_eq!(app.documents[app.active].id, active);
                let pair = app.editor_columns_snapshot().unwrap();
                assert_eq!(
                    [app.documents[pair.left].id, app.documents[pair.right].id],
                    paired
                );
                assert_eq!(
                    egui::text_edit::TextEditState::load(&ctx, editor::id(active))
                        .unwrap()
                        .cursor
                        .char_range(),
                    cursor
                );
                for doc in &app.documents {
                    assert_eq!(doc.text.as_ptr() as usize, pointers[&doc.id]);
                    assert!(doc.same_snapshot(old.iter().find(|old| old.id == doc.id).unwrap()));
                }
                let dir = tempfile::tempdir().unwrap();
                let path = dir.path().join("dragged-order.json");
                app.snapshot().save(&path).unwrap();
                let recovered = session::Session::load(&path).unwrap();
                assert_eq!(recovered.active, app.active);
                assert_eq!(
                    recovered.layout.editor_columns,
                    app.editor_columns_snapshot()
                );
                assert_eq!(
                    recovered
                        .documents
                        .iter()
                        .map(|doc| &doc.text)
                        .collect::<Vec<_>>(),
                    app.documents
                        .iter()
                        .map(|doc| &doc.text)
                        .collect::<Vec<_>>()
                );
                for _ in 0..3 {
                    drag_frame(&mut app, &ctx, size, &backend, detached, vec![]);
                }
                drag_press(&mut app, &ctx, size, &backend, detached, ids[0]);
                drag_release(&mut app, &ctx, size, &backend, detached, ids[1], true);
                assert_eq!(
                    app.documents.iter().map(|doc| doc.id).collect::<Vec<_>>(),
                    ids
                );
                // Reorder commits after painting; obtain the next real geometry before clicking.
                drag_frame(&mut app, &ctx, size, &backend, detached, vec![]);
                // The same response still supports an ordinary click, with no reorder.
                let point = drag_tab(&app, ids[1]).rect.center();
                for pressed in [true, false] {
                    drag_frame(
                        &mut app,
                        &ctx,
                        size,
                        &backend,
                        detached,
                        vec![
                            egui::Event::PointerMoved(point),
                            egui::Event::PointerButton {
                                pos: point,
                                button: egui::PointerButton::Primary,
                                pressed,
                                modifiers: egui::Modifiers::NONE,
                            },
                        ],
                    );
                }
                assert_eq!(app.documents[app.active].id, ids[1]);
                assert_eq!(
                    app.documents.iter().map(|doc| doc.id).collect::<Vec<_>>(),
                    ids
                );
            }
        }
    }

    #[test]
    fn tab_drag_discarded_release_is_once_and_rejects_changed_authority() {
        for invalidate in 0..7 {
            let size = egui::vec2(960., 650.);
            let (ctx, mut app, backend) = setup(size, session::DesktopTheme::Dark);
            app.new_document("third_drag=31;\n");
            for _ in 0..3 {
                native_frame(&mut app, &ctx, size, &backend, vec![]);
            }
            let ids = app.documents.iter().map(|doc| doc.id).collect::<Vec<_>>();
            drag_press(&mut app, &ctx, size, &backend, true, ids[0]);
            let target = drag_tab(&app, ids[2]);
            let point = egui::pos2(target.rect.right() - 3., target.rect.center().y);
            assert!(target.clip.contains(point));
            native_frame(
                &mut app,
                &ctx,
                size,
                &backend,
                vec![egui::Event::PointerMoved(point)],
            );
            let mut input = egui::RawInput {
                viewport_id: editor_window::viewport(),
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
            };
            input
                .viewports
                .entry(editor_window::viewport())
                .or_default()
                .focused = Some(true);
            let mut passes = 0;
            crate::headless::run(&ctx, input, |ui| {
                passes += 1;
                if passes == 1 {
                    ui.ctx().request_discard("actual tab drag release");
                }
                app.editor_window.in_callback = true;
                app.editor_window_ui(ui);
                app.editor_window.in_callback = false;
                if passes == 1 {
                    assert_eq!(
                        app.documents.iter().map(|doc| doc.id).collect::<Vec<_>>(),
                        ids
                    );
                    match invalidate {
                        1 => app.stop_work(),
                        2 => app.activate_script_index(1),
                        3 => app.pending_close = Some(2),
                        4 => app.document_save.command_selected(),
                        5 => app.busy = true,
                        6 => app.reset_editor_host(),
                        _ => {}
                    }
                }
            });
            assert!(passes >= 2);
            let expected = if invalidate == 0 {
                vec![ids[1], ids[2], ids[0]]
            } else {
                ids.clone()
            };
            assert_eq!(
                app.documents.iter().map(|doc| doc.id).collect::<Vec<_>>(),
                expected
            );
            app.pending_close = None;
            app.busy = false;
            for _ in 0..3 {
                native_frame(&mut app, &ctx, size, &backend, vec![]);
            }
            assert_eq!(
                app.documents.iter().map(|doc| doc.id).collect::<Vec<_>>(),
                expected,
                "retired drop must never replay in another frame"
            );
        }
    }

    #[test]
    fn tab_drag_scrolls_finite_strip_to_distant_target_and_escape_cancels() {
        let size = egui::vec2(960., 650.);
        let (ctx, mut app, backend) = setup(size, session::DesktopTheme::Light);
        for i in 2..session::MAX_DOCUMENTS {
            app.new_document(&format!("drag_{i}={i};\n"));
        }
        let ids = app.documents.iter().map(|doc| doc.id).collect::<Vec<_>>();
        for _ in 0..4 {
            native_frame(&mut app, &ctx, size, &backend, vec![]);
        }
        let source = *ids.last().unwrap();
        drag_press(&mut app, &ctx, size, &backend, true, source);
        // Start at the actual rightmost tab and drag against the left edge until
        // the first tab is fully visible; every step is a real pointer frame.
        let scroll_start = app.script_tab_scroll_diagnostic();
        let mut reached = false;
        for _ in 0..800 {
            let first = drag_tab(&app, ids[0]);
            if drag_painted_clip(first, size).contains_rect(first.rect) {
                reached = true;
                break;
            }
            let point = egui::pos2(first.clip.left() + 2., first.rect.center().y);
            native_frame(
                &mut app,
                &ctx,
                size,
                &backend,
                vec![egui::Event::PointerMoved(point)],
            );
        }
        assert!(
            reached,
            "finite edge scrolling must expose the distant tab: first={:?}; source={:?}; start={scroll_start}; last={}; drag={}",
            drag_tab(&app, ids[0]),
            drag_tab(&app, source),
            app.script_tab_scroll_diagnostic(),
            app.script_tab_drag_diagnostic(source)
        );
        drag_release(&mut app, &ctx, size, &backend, true, ids[0], true);
        assert_eq!(app.documents[0].id, source);
        assert_eq!(app.documents[app.active].id, source);
        let expected = app.documents.iter().map(|doc| doc.id).collect::<Vec<_>>();
        for _ in 0..3 {
            native_frame(&mut app, &ctx, size, &backend, vec![]);
        }
        drag_press(&mut app, &ctx, size, &backend, true, source);
        native_key(
            &mut app,
            &ctx,
            size,
            &backend,
            egui::Key::Escape,
            egui::Modifiers::NONE,
        );
        assert!(!egui::DragAndDrop::has_any_payload(&ctx));
        let outside = egui::pos2(size.x - 2., size.y - 2.);
        native_frame(
            &mut app,
            &ctx,
            size,
            &backend,
            vec![
                egui::Event::PointerMoved(outside),
                egui::Event::PointerButton {
                    pos: outside,
                    button: egui::PointerButton::Primary,
                    pressed: false,
                    modifiers: egui::Modifiers::NONE,
                },
            ],
        );
        assert_eq!(
            app.documents.iter().map(|doc| doc.id).collect::<Vec<_>>(),
            expected
        );
        for _ in 0..3 {
            native_frame(&mut app, &ctx, size, &backend, vec![]);
        }
        drag_press(&mut app, &ctx, size, &backend, true, source);
        // Another owner replaces the global plugin payload. Stop retires our
        // ticket without clearing that unrelated owner's data.
        egui::DragAndDrop::set_payload(&ctx, String::from("foreign drag"));
        app.stop_work();
        assert_eq!(
            egui::DragAndDrop::payload::<String>(&ctx)
                .as_deref()
                .map(String::as_str),
            Some("foreign drag")
        );
        assert_eq!(
            app.documents.iter().map(|doc| doc.id).collect::<Vec<_>>(),
            expected
        );
    }

    #[test]
    fn tab_drag_own_press_acknowledges_only_the_exact_pointer_observer_transition() {
        for foreign in [false, true] {
            let ctx = egui::Context::default();
            let mut app = Workspace::with_context(ctx.clone(), true);
            let size = egui::vec2(960., 650.);
            let point = egui::pos2(40., 40.);
            // Establish a real top-layer region, then observe the same primary
            // press through the existing DocumentSave final-pass path.
            for pressed in [false, true] {
                let events = if pressed {
                    vec![
                        egui::Event::PointerMoved(point),
                        egui::Event::PointerButton {
                            pos: point,
                            button: egui::PointerButton::Primary,
                            pressed: true,
                            modifiers: egui::Modifiers::NONE,
                        },
                    ]
                } else {
                    vec![]
                };
                crate::headless::run(
                    &ctx,
                    egui::RawInput {
                        screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, size)),
                        events,
                        ..Default::default()
                    },
                    |ui| {
                        app.document_save
                            .register_ui(document_save::Pane::Script, ui);
                        let before = app.document_save.interaction_generation();
                        if pressed && foreign {
                            app.document_save.command_selected();
                        }
                        app.finish_document_action(ui.ctx());
                        if pressed {
                            let observed = app.document_save.observed_script_press_generation(
                                ui.ctx(),
                                point,
                                before,
                            );
                            if foreign {
                                assert!(observed.is_none());
                            } else {
                                assert_eq!(
                                    observed,
                                    Some(app.document_save.interaction_generation())
                                );
                                assert_eq!(observed, Some(before.wrapping_add(1)));
                                assert!(
                                    app.document_save
                                        .observed_script_press_generation(
                                            ui.ctx(),
                                            point + egui::vec2(1., 0.),
                                            before
                                        )
                                        .is_none()
                                );
                                app.document_save.command_selected();
                                assert!(
                                    app.document_save
                                        .observed_script_press_generation(ui.ctx(), point, before)
                                        .is_none()
                                );
                            }
                        }
                    },
                );
            }
        }
    }

    #[test]
    fn actual_native_editor_focus_traversal_stays_in_its_two_borrowed_buffers() {
        let size=egui::vec2(960.,650.);
        let (ctx,mut app,backend)=setup(size,session::DesktopTheme::Dark);
        let ids:Vec<_>=app.documents.iter().map(|doc|doc.id).collect();
        let docs=app.documents.clone();
        let point=backend.borrow().rects[0].center();
        native_click(&mut app,&ctx,size,&backend,point);
        assert_eq!(backend.borrow().focus_owner,Some(editor::id(ids[0])));
        for index in [1,0,1] {
            native_key(&mut app,&ctx,size,&backend,egui::Key::F6,CTRL);
            assert_eq!(backend.borrow().focus_owner,Some(editor::id(ids[index])));
            assert_eq!(app.documents[app.active].id,ids[index]);
        }
        native_key(&mut app,&ctx,size,&backend,egui::Key::F6,
            egui::Modifiers{shift:true,..CTRL});
        assert_eq!(backend.borrow().focus_owner,Some(editor::id(ids[0])));
        assert!(app.documents.iter().zip(&docs).all(|(a,b)|a.same_snapshot(b)));
        native_frame(&mut app,&ctx,size,&backend,vec![egui::Event::Text("% focused left\n".into())]);
        assert!(app.documents[0].text.contains("focused left"));
        assert!(app.documents[1].same_snapshot(&docs[1]));
        native_key(&mut app,&ctx,size,&backend,egui::Key::Slash,CTRL);
        assert!(app.focus_areas.is_open());
        assert!(backend.borrow().output.as_ref().unwrap().shapes.iter()
            .any(|shape|text_position(&shape.shape,"Navigation shortcuts").is_some()));
        native_key(&mut app,&ctx,size,&backend,egui::Key::Escape,egui::Modifiers::NONE);
        assert!(!app.focus_areas.is_open());
    }

    #[test]
    fn embedded_editor_owns_f6_input_before_root_files_workspace_and_command() {
        let size=egui::vec2(1440.,900.);
        let ctx=egui::Context::default();
        ctx.set_embed_viewports(true);
        let mut app=Workspace::with_context(ctx.clone(),true);
        app.new_document("embedded_left=1;\n");app.new_document("embedded_right=2;\n");
        app.activate_script_index(0);app.select_document_layout(layout::PaneMode::Editor);
        for _ in 0..3 {frame_at(&mut app,&ctx,size,vec![]);}
        click_label_at(&mut app,&ctx,size, "Editor actions");
        click_label_at(&mut app,&ctx,size,"Side by side");
        let original_documents=app.documents.clone();
        click_label_at(&mut app,&ctx,size, "Editor actions");
        click_label_at(&mut app,&ctx,size,"Undock Editor");
        assert!(app.editor_window.detached);
        assert_eq!(ctx.read_response(editor_window::group_id()).unwrap().layer_id.order,
            egui::Order::Background,"transition frame retains its already painted docked tree");
        for _ in 0..3 {frame_at(&mut app,&ctx,size,vec![]);}
        assert!(app.editor_window.detached);
        assert_eq!(ctx.read_response(editor_window::group_id()).unwrap().layer_id.order,
            egui::Order::Middle,"next frame paints the same shared IDs in the embedded window");
        assert!(app.documents.iter().zip(&original_documents).all(|(a,b)|a.same_snapshot(b)));
        assert_eq!(app.editor_focus_viewport(),egui::ViewportId::ROOT);
        let ids:Vec<_>=app.documents.iter().map(|doc|doc.id).collect();
        let response=ctx.read_response(editor::id(ids[0])).unwrap();
        let point=response.interact_rect.intersect(ctx.content_rect()).center();
        pointer_frame(&mut app,&ctx,size,point,true);pointer_frame(&mut app,&ctx,size,point,false);
        assert!(ctx.memory(|m|m.has_focus(editor::id(ids[0]))));
        assert!(focus_areas::observations(&ctx,false).iter().any(|(a,_)|*a==focus_areas::Area::Workspace));
        for index in [1,0,1] {
            for pressed in [true,false] {frame_at(&mut app,&ctx,size,vec![egui::Event::Key{
                key:egui::Key::F6,physical_key:Some(egui::Key::F6),pressed,repeat:false,modifiers:CTRL}]);}
            assert!(ctx.memory(|m|m.has_focus(editor::id(ids[index]))));
            assert_eq!(app.documents[app.active].id,ids[index]);
        }
        for pressed in [true,false] {frame_at(&mut app,&ctx,size,vec![egui::Event::Key{
            key:egui::Key::Slash,physical_key:Some(egui::Key::Slash),pressed,repeat:false,modifiers:CTRL}]);}
        assert!(app.focus_areas.is_open());
        let output=frame_at(&mut app,&ctx,size,vec![]);
        assert!(output.shapes.iter().any(|shape|text_position(&shape.shape,"Navigation shortcuts").is_some()));
    }

    #[test]
    fn native_editor_close_retires_owned_help_without_losing_dirty_documents_or_blocking_root() {
        let size=egui::vec2(960.,650.);
        let (ctx,mut app,backend)=setup(size,session::DesktopTheme::Light);
        let docs=app.documents.clone();
        let point=backend.borrow().rects[0].center();
        native_click(&mut app,&ctx,size,&backend,point);
        native_key(&mut app,&ctx,size,&backend,egui::Key::Slash,CTRL);
        assert!(app.focus_areas.is_open());assert!(app.file_action_pending());
        let mut input=egui::RawInput{viewport_id:editor_window::viewport(),
            screen_rect:Some(egui::Rect::from_min_size(egui::Pos2::ZERO,size)),..Default::default()};
        let info=input.viewports.entry(editor_window::viewport()).or_default();
        info.focused=Some(true);info.events.push(egui::ViewportEvent::Close);
        let mut passes=0;
        crate::headless::run(&ctx,input,|ui|{
            passes+=1;
            if passes==1 {ui.ctx().request_discard("native help owner retires only on final dock");}
            app.editor_window.in_callback=true;app.editor_window_ui(ui);app.editor_window.in_callback=false;
            if passes==1 {assert!(app.editor_window.detached);assert!(app.focus_areas.is_open());}
        });
        assert!(passes>=2);
        assert!(!app.editor_window.detached);assert!(!app.focus_areas.is_open());
        assert!(!app.file_action_pending());assert!(!app.pending_quit);
        assert!(app.documents.iter().zip(&docs).all(|(a,b)|a.same_snapshot(b)));
        assert!(app.documents.iter().all(|doc|doc.dirty));
        for _ in 0..3 {frame_at(&mut app,&ctx,size,vec![]);}
        let point=ctx.read_response(editor::id(app.documents[0].id)).unwrap()
            .interact_rect.intersect(ctx.content_rect()).center();
        pointer_frame(&mut app,&ctx,size,point,true);pointer_frame(&mut app,&ctx,size,point,false);
        for pressed in [true,false] {frame_at(&mut app,&ctx,size,vec![egui::Event::Key{
            key:egui::Key::F6,physical_key:Some(egui::Key::F6),pressed,repeat:false,modifiers:CTRL}]);}
        assert!(ctx.memory(|m|m.has_focus(editor::id(app.documents[1].id))));
        assert!(app.documents.iter().zip(&docs).all(|(a,b)|a.same_snapshot(b)));
    }

    #[test]
    fn embedded_dock_release_across_discard_defers_reverse_tree_and_preserves_cursor() {
        let size = egui::vec2(1440., 900.);
        let ctx = egui::Context::default();
        ctx.set_embed_viewports(true);
        let mut app = Workspace::with_context(ctx.clone(), true);
        app.new_document("left_retained = 17;\n");
        app.new_document("right_retained = 29;\n");
        app.activate_script_index(0);
        app.select_document_layout(layout::PaneMode::Editor);
        for _ in 0..3 { frame_at(&mut app, &ctx, size, vec![]); }
        click_label_at(&mut app, &ctx, size, "Editor actions");
        click_label_at(&mut app, &ctx, size, "Side by side");
        click_label_at(&mut app, &ctx, size, "Editor actions");
        click_label_at(&mut app, &ctx, size, "Undock Editor");
        for _ in 0..3 { frame_at(&mut app, &ctx, size, vec![]); }
        assert!(app.editor_window.detached);
        let document = app.documents[app.active].id;
        let point = ctx.read_response(editor::id(document)).unwrap().interact_rect.center();
        pointer_frame(&mut app, &ctx, size, point, true);
        pointer_frame(&mut app, &ctx, size, point, false);
        assert!(ctx.memory(|m| m.has_focus(editor::id(document))));
        let cursor = egui::text_edit::TextEditState::load(&ctx, editor::id(document))
            .unwrap().cursor.char_range().unwrap();
        let documents = app.documents.clone();
        let allocations: Vec<_> = app.documents.iter().map(|d| d.text.as_ptr()).collect();
        let pairing = app.editor_columns_snapshot();
        click_label_at(&mut app, &ctx, size, "Editor actions");
        let output = frame_at(&mut app, &ctx, size, vec![]);
        let group = ctx.read_response(editor_window::group_id()).unwrap();
        assert_eq!(group.layer_id.order, egui::Order::Middle);
        let dock = output.shapes.iter().rev()
            .find_map(|shape| text_position(&shape.shape, "Dock back"))
            .expect("actual embedded Dock back menu item");
        pointer_frame(&mut app, &ctx, size, dock, true);
        let mut passes = 0;
        crate::headless::run(&ctx, egui::RawInput {
            screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, size)),
            events: vec![egui::Event::PointerMoved(dock), egui::Event::PointerButton {
                pos: dock, button: egui::PointerButton::Primary, pressed: false,
                modifiers: egui::Modifiers::NONE,
            }], ..Default::default()
        }, |root| {
            if passes == 0 { root.ctx().request_discard("retain real embedded Dock release"); }
            app.ui(root);
            if passes == 0 {
                assert!(app.editor_window.detached, "discarded Dock cannot commit early");
            } else {
                assert!(!app.editor_window.detached);
                // Exercise an immediate reverse-host request after the real Dock
                // action. It must not re-register the shared IDs on Background.
                app.editor_pane(root);
                assert_eq!(root.ctx().read_response(editor_window::group_id()).unwrap().layer_id.order,
                    egui::Order::Middle);
            }
            passes += 1;
        });
        assert!(passes >= 2);
        frame_at(&mut app, &ctx, size, vec![]);
        assert_eq!(ctx.read_response(editor_window::group_id()).unwrap().layer_id.order,
            egui::Order::Background);
        assert_eq!(app.documents[app.active].id, document);
        assert_eq!(app.editor_columns_snapshot(), pairing);
        assert_eq!(app.documents.len(), documents.len());
        assert!(app.documents.iter().zip(&documents).all(|(a,b)| a.same_snapshot(b)));
        assert_eq!(app.documents.iter().map(|d| d.text.as_ptr()).collect::<Vec<_>>(), allocations);
        assert_eq!(egui::text_edit::TextEditState::load(&ctx, editor::id(document))
            .unwrap().cursor.char_range().unwrap(), cursor);
    }
    #[test]
    fn native_editor_open_keeps_existing_final_host_route_through_discard() {
        let size=egui::vec2(960.,650.);
        let (ctx,mut app,backend)=setup(size,session::DesktopTheme::Light);
        app.root_open.picker=Some(|_,kind|{assert_eq!(kind,open_files::PickerKind::Supported);None});
        let point=backend.borrow().rects[0].center();native_click(&mut app,&ctx,size,&backend,point);
        let documents=app.documents.clone();let id=app.documents[app.active].id;
        let cursor=egui::text_edit::TextEditState::load(&ctx,editor::id(id)).unwrap().cursor.char_range();
        let mut input=egui::RawInput{viewport_id:editor_window::viewport(),screen_rect:Some(egui::Rect::from_min_size(egui::Pos2::ZERO,size)),
            events:vec![egui::Event::ModifiersChanged(CTRL),egui::Event::Key{key:egui::Key::O,physical_key:Some(egui::Key::O),pressed:true,repeat:false,modifiers:CTRL}],..Default::default()};
        input.viewports.entry(editor_window::viewport()).or_default().focused=Some(true);
        let mut passes=0;
        crate::headless::run(&ctx,input,|ui|{
            passes+=1;if passes==1 {ui.ctx().request_discard("native Editor Open keeps its own final admission");}
            app.editor_window.in_callback=true;app.editor_window_ui(ui);app.editor_window.in_callback=false;
            if passes==1 {assert!(app.root_open.calls.is_empty());}
        });
        assert!(passes>=2);assert_eq!(app.root_open.calls,vec![open_files::PickerKind::Supported]);
        assert_eq!(app.editor_window.last_dispatch(),Some((id,"open")));
        assert!(app.documents.iter().zip(&documents).all(|(a,b)|a.same_snapshot(b)));
        assert_eq!(egui::text_edit::TextEditState::load(&ctx,editor::id(id)).unwrap().cursor.char_range(),cursor);
        native_frame(&mut app,&ctx,size,&backend,vec![]);assert_eq!(app.root_open.calls.len(),1);
    }
    #[test]
    fn embedded_editor_ctrl_o_belongs_to_child_for_both_buffer_and_find_focus() {
        let size=egui::vec2(1440.,900.);let ctx=egui::Context::default();ctx.set_embed_viewports(true);
        let mut app=Workspace::with_context(ctx.clone(),true);
        app.new_document("open_left=1;\n");app.new_document("open_right=2;\n");
        app.activate_script_index(0);app.select_document_layout(layout::PaneMode::Editor);
        for _ in 0..3 {frame_at(&mut app,&ctx,size,vec![]);}
        click_label_at(&mut app,&ctx,size, "Editor actions");
        click_label_at(&mut app,&ctx,size,"Side by side");click_label_at(&mut app,&ctx,size, "Editor actions");
        click_label_at(&mut app,&ctx,size,"Undock Editor");
        for _ in 0..3 {frame_at(&mut app,&ctx,size,vec![]);}
        app.root_open.picker=Some(|_,kind|{assert_eq!(kind,open_files::PickerKind::Supported);None});
        let documents=app.documents.clone();let id=app.documents[app.active].id;
        let point=ctx.read_response(editor::id(id)).unwrap().interact_rect.center();
        for down in [true,false] {pointer_frame(&mut app,&ctx,size,point,down);}
        assert!(ctx.memory(|m|m.has_focus(editor::id(id))));assert!(app.embedded_editor_owns_focus(&ctx));
        for find in [false,true] {
            if find {
                command_first_key(&mut app,&ctx,size,egui::Key::F,CTRL);
                for _ in 0..3 {frame_at(&mut app,&ctx,size,vec![]);}
                assert!(ctx.memory(|m|m.focused().is_some_and(|focused|focused!=editor::id(id))));
                assert!(app.embedded_editor_owns_focus(&ctx));
            }
            let before=app.editor_window.dispatch_count("open");
            command_first_key(&mut app,&ctx,size,egui::Key::O,CTRL);
            assert_eq!(app.editor_window.dispatch_count("open"),before+1);
            assert_eq!(app.editor_window.last_dispatch(),Some((id,"open")));
            assert_eq!(app.root_open.calls.len(),if find {2}else{1});
            assert!(app.documents.iter().zip(&documents).all(|(a,b)|a.same_snapshot(b)));
        }
    }
}
