mod focus_area_cases {
    use super::*;
    use focus_areas::Area;
    const CTRL: egui::Modifiers = egui::Modifiers { ctrl: true, command: true,
        alt: false, shift: false, mac_cmd: false };

    fn setup(size: egui::Vec2, theme: session::DesktopTheme)
        -> (egui::Context, Workspace, tempfile::TempDir) {
        let ctx = egui::Context::default();
        let mut app = Workspace::with_context(ctx.clone(), true);
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("focus.m"), "untouched=1;\n").unwrap();
        assert!(app.change_folder(dir.path().to_owned())); finish_folder(&mut app);
        app.new_document("left_focus=11;\n"); app.new_document("right_focus=29;\n");
        app.activate_script_index(0);
        app.run("keep=[1 2 3];alias=keep;plot(keep);".into()); finish(&mut app);
        // Worker completion starts another asynchronous listing refresh. Drain that
        // generation explicitly before requiring the ready Files focus destination.
        finish_folder(&mut app);
        assert!(!app.folder_browser.loading);
        assert!(app.entries.iter().any(|entry| entry.name == "focus.m"));
        app.layout.theme = Some(theme); app.apply_desktop_theme(&ctx);
        app.select_document_layout(layout::PaneMode::Split);
        for _ in 0..3 { frame_at(&mut app, &ctx, size, vec![]); }
        click_label_at(&mut app, &ctx, size, "Editor actions");
        click_label_at(&mut app, &ctx, size, "Side by side");
        // Enabling two buffers intentionally expands Editor. This fixture needs
        // both Editor buffers and the Figure pane in the visible focus ring.
        assert_eq!(app.layout.mode, layout::PaneMode::Editor);
        app.select_document_layout(layout::PaneMode::Split);
        for _ in 0..3 { frame_at(&mut app, &ctx, size, vec![]); }
        assert_eq!(app.layout.mode, layout::PaneMode::Split);
        let columns = app.editor_columns_snapshot().expect("two visible Editor buffers");
        assert_ne!(columns.left, columns.right);
        assert!(focus_areas::observations(&ctx, false).iter()
            .any(|(area, _)| *area == Area::Figure(app.current_figure)),
            "Split fixture must paint its actual Figure focus target");
        (ctx, app, dir)
    }
    fn key(app: &mut Workspace, ctx: &egui::Context, size: egui::Vec2,
        key: egui::Key, modifiers: egui::Modifiers) {
        for pressed in [true, false] {
            frame_at(app, ctx, size, vec![egui::Event::Key {
                key, physical_key: Some(key), pressed, repeat: false, modifiers,
            }]);
        }
    }
    fn owner(ctx: &egui::Context) -> Option<egui::Id> { ctx.memory(|m| m.focused()) }
    fn focus_editor(app: &mut Workspace, ctx: &egui::Context, size: egui::Vec2, index: usize) {
        let id = editor::id(app.documents[index].id);
        let r = ctx.read_response(id).unwrap();
        let point = r.interact_rect.intersect(ctx.content_rect()).center();
        pointer_frame(app, ctx, size, point, true); pointer_frame(app, ctx, size, point, false);
        assert_eq!(owner(ctx), Some(id)); assert_eq!(app.active, index);
    }
    fn cursor(ctx: &egui::Context, id: u64) -> std::ops::Range<usize> {
        let range = egui::text_edit::TextEditState::load(ctx, editor::id(id)).unwrap()
            .cursor.char_range().unwrap().as_sorted_char_range();
        range.start.0..range.end.0
    }

    #[test]
    fn actual_ctrl_f6_cycles_visible_real_widgets_and_both_editors_without_mutating_work() {
        for theme in [session::DesktopTheme::Light, session::DesktopTheme::Dark] {
            for size in [egui::vec2(960.,650.), egui::vec2(1440.,900.)] {
                let (ctx, mut app, _dir) = setup(size, theme);
                for index in 0..2 { focus_editor(&mut app, &ctx, size, index);
                    key(&mut app, &ctx, size, egui::Key::End, CTRL); }
                let docs = app.documents.clone();
                let cursors: Vec<_> = docs.iter().map(|doc| cursor(&ctx, doc.id)).collect();
                let value = app.variables["keep"].clone();
                let figure = app.figure.as_ref().unwrap().clone();
                let expected = vec![Area::Files, Area::Workspace, Area::Editor(docs[0].id),
                    Area::Editor(docs[1].id), Area::Figure(app.current_figure), Area::Command];
                let actual = focus_areas::observations(&ctx, false);
                assert_eq!(actual.iter().map(|(area,_)| *area).collect::<Vec<_>>(), expected);
                // Start from the actual second editor, then walk the complete ring.
                for index in [4,5,0,1,2,3] {
                    key(&mut app, &ctx, size, egui::Key::F6, CTRL);
                    assert_eq!(owner(&ctx), Some(actual[index].1), "forward {:?}", actual[index].0);
                    if let Area::Editor(id) = actual[index].0 { assert_eq!(app.documents[app.active].id,id); }
                }
                for index in [2,1,0,5,4,3] {
                    key(&mut app, &ctx, size, egui::Key::F6, egui::Modifiers { shift:true,..CTRL });
                    assert_eq!(owner(&ctx), Some(actual[index].1), "reverse {:?}", actual[index].0);
                }
                assert!(app.documents.iter().zip(&docs).all(|(a,b)| a.same_snapshot(b)));
                for (doc, before) in docs.iter().zip(cursors) { assert_eq!(cursor(&ctx,doc.id),before); }
                assert!(app.variables["keep"].same_snapshot(&value));
                assert!(app.variables["alias"].same_snapshot(&value));
                assert_eq!(app.variables["alias"].as_numeric().unwrap().data().as_ptr(),
                    app.variables["keep"].as_numeric().unwrap().data().as_ptr());
                assert!(app.figure.as_ref().unwrap().same_snapshot(&figure));
            }
        }
    }

    #[test]
    fn hidden_areas_are_skipped_and_selected_tool_tabs_join_the_focus_ring() {
        let size=egui::vec2(1440.,900.);
        let (ctx,mut app,_dir)=setup(size,session::DesktopTheme::Light);
        // Files and Workspace are always-visible cards; only Command can be hidden.
        app.navigation.command=false;
        app.select_document_layout(layout::PaneMode::Editor);
        for _ in 0..3 { frame_at(&mut app,&ctx,size,vec![]); }
        focus_editor(&mut app,&ctx,size,0);
        let visible=focus_areas::observations(&ctx,false);
        assert_eq!(visible.iter().map(|(a,_)|*a).collect::<Vec<_>>(),
            // The figure keeps its own column beside the Editor.
            vec![Area::Files,Area::Workspace,Area::Editor(app.documents[0].id),
                Area::Editor(app.documents[1].id),Area::Figure(1)]);
        for index in [3,4,0,1,2,3] { key(&mut app,&ctx,size,egui::Key::F6,CTRL);
            assert_eq!(owner(&ctx),Some(visible[index].1)); }
        // A selected tool tab replaces the hidden editors; Command docks below it.
        app.navigation.command=true;
        app.toolkits.open_tool(toolkits::Tool::Designer);
        app.toolkits.open_tool(toolkits::Tool::Model);
        for _ in 0..3 { frame_at(&mut app,&ctx,size,vec![]); }
        let areas=|ctx:&egui::Context| focus_areas::observations(ctx,false).iter().map(|(a,_)|*a).collect::<Vec<_>>();
        assert_eq!(areas(&ctx),vec![Area::Files,Area::Workspace,Area::Command,Area::Model]);
        let docs=app.documents.clone();
        let settle=|app:&mut Workspace| for _ in 0..3 { frame_at(app,&ctx,size,vec![]); };
        // F6 walks the ring (cards included); a tool tab joins it when selected.
        let reach=|app:&mut Workspace,area:Area,modifiers:egui::Modifiers| {
            for _ in 0..8 {
                key(app,&ctx,size,egui::Key::F6,modifiers); settle(app);
                let seen=focus_areas::observations(&ctx,false);
                if seen.iter().any(|(a,id)|*a==area&&owner(&ctx)==Some(*id)) { return true; }
            }
            false
        };
        assert!(reach(&mut app,Area::Command,CTRL));
        assert!(reach(&mut app,Area::Designer,CTRL));
        assert_eq!(app.toolkits.selected_tool(),Some(toolkits::Tool::Designer));
        assert_eq!(areas(&ctx),vec![Area::Files,Area::Workspace,Area::Command,Area::Designer]);
        assert!(reach(&mut app,Area::Model,CTRL));
        assert_eq!(app.toolkits.selected_tool(),Some(toolkits::Tool::Model));
        assert_eq!(app.generic_save_target(),document_save::Pane::Model);
        assert_eq!(owner(&ctx),Some(egui::Id::new("model_diagram_keyboard")));
        key(&mut app,&ctx,size,egui::Key::ArrowRight,egui::Modifiers::NONE);
        assert!(app.documents.iter().zip(&docs).all(|(a,b)|a.same_snapshot(b)));
        // Shift goes back through the other tool.
        assert!(reach(&mut app,Area::Designer,CTRL|egui::Modifiers::SHIFT));
        assert_eq!(app.toolkits.selected_tool(),Some(toolkits::Tool::Designer));
        assert!(app.documents.iter().zip(&docs).all(|(a,b)|a.same_snapshot(b)));
    }

    #[test]
    fn traversal_waits_for_final_pass_and_rejects_new_owner_hidden_target_stop_and_modal() {
        for change in 0..5 {
            let size=egui::vec2(1440.,900.);
            let (ctx,mut app,_dir)=setup(size,session::DesktopTheme::Light);
            focus_editor(&mut app,&ctx,size,0);
            let original=app.documents[0].id; let other=app.documents[1].id;
            let command=egui::Id::new("command_input");
            let mut passes=0;
            crate::headless::run(&ctx,egui::RawInput {
                screen_rect:Some(egui::Rect::from_min_size(egui::Pos2::ZERO,size)),
                events:vec![egui::Event::ModifiersChanged(CTRL),egui::Event::Key {
                    key:egui::Key::F6,physical_key:Some(egui::Key::F6),pressed:true,repeat:false,modifiers:CTRL}],
                ..Default::default()
            },|root| {
                passes+=1;
                if passes==1 { root.ctx().request_discard("focus must commit once at final admission"); }
                app.ui(root);
                if passes==1 {
                    assert_eq!(app.documents[app.active].id,original);
                    match change {
                        1=>root.ctx().memory_mut(|m|m.request_focus(command)),
                        2=>app.select_document_layout(layout::PaneMode::Figure),
                        3=>app.stop_work(),
                        4=>{app.request_script_close(app.active);assert!(app.pending_close.is_some());},
                        _=>{},
                    }
                }
            });
            assert!(passes>=2);
            if change==0 { assert_eq!(app.documents[app.active].id,other);
                assert_eq!(owner(&ctx),Some(editor::id(other))); }
            else { assert_eq!(app.documents[app.active].id,original);
                assert_ne!(owner(&ctx),Some(editor::id(other))); }
        }
    }

    #[test]
    fn busy_popup_and_modified_chords_do_not_take_focus_or_reveal_hidden_areas() {
        let size=egui::vec2(960.,650.);
        let (ctx,mut app,_dir)=setup(size,session::DesktopTheme::Dark);
        focus_editor(&mut app,&ctx,size,0);
        let original=owner(&ctx); let active=app.active;
        key(&mut app,&ctx,size,egui::Key::F6,egui::Modifiers{alt:true,..CTRL});
        assert_eq!(owner(&ctx),original); assert_eq!(app.active,active);
        app.busy=true; key(&mut app,&ctx,size,egui::Key::F6,CTRL);
        assert_eq!(app.active,active); app.busy=false;
        egui::Popup::open_id(&ctx,egui::Id::new("focus_test_foreign_popup"));
        assert!(egui::Popup::is_any_open(&ctx));
        key(&mut app,&ctx,size,egui::Key::F6,CTRL);
        assert_eq!(app.active,active);
        egui::Popup::close_all(&ctx);
    }

    #[test]
    fn shortcut_help_is_bounded_discoverable_and_escape_remains_usable_when_busy() {
        for theme in [session::DesktopTheme::Light,session::DesktopTheme::Dark] {
            for size in [egui::vec2(960.,650.),egui::vec2(1440.,900.)] {
                for scale in [75,100,150] {
                    let (ctx,mut app,_dir)=setup(size,theme);
                    app.set_desktop_scale(&ctx,scale);
                    for _ in 0..3 { frame_at(&mut app,&ctx,size,vec![]); }
                    focus_editor(&mut app,&ctx,size,0);
                    let docs=app.documents.clone();
                    key(&mut app,&ctx,size,egui::Key::Slash,CTRL);
                    assert!(app.focus_areas.is_open());
                    let output=frame_at(&mut app,&ctx,size,vec![]);
                    fn ink(shape:&egui::Shape,label:&str,clip:egui::Rect)->bool {
                        match shape {
                            egui::Shape::Text(text) if text.galley.text()==label =>
                                clip.contains_rect(text.visual_bounding_rect()),
                            egui::Shape::Vec(shapes)=>shapes.iter().any(|s|ink(s,label,clip)),
                            _=>false,
                        }
                    }
                    for label in ["Navigation shortcuts","Close"] {
                        assert!(output.shapes.iter().any(|shape|ink(&shape.shape,label,
                            shape.clip_rect.intersect(ctx.content_rect()))),"{label} {size:?} {scale}");
                    }
                    app.busy=true;
                    key(&mut app,&ctx,size,egui::Key::Escape,egui::Modifiers::NONE);
                    assert!(!app.focus_areas.is_open()); app.busy=false;
                    assert!(app.documents.iter().zip(&docs).all(|(a,b)|a.same_snapshot(b)));
                }
            }
        }
    }

    #[test]
    fn layout_menu_opens_actual_help_and_close_release_survives_a_discarded_pass() {
        let size=egui::vec2(960.,650.);
        let (ctx,mut app,_dir)=setup(size,session::DesktopTheme::Light);
        click_label_at(&mut app,&ctx,size,"Menu");
        click_label_at(&mut app,&ctx,size,"Layout");
        click_label_at(&mut app,&ctx,size,"Navigation shortcuts");
        assert!(app.focus_areas.is_open());
        let output=frame_at(&mut app,&ctx,size,vec![]);
        let point=output.shapes.iter().find_map(|shape|text_position(&shape.shape,"Close")).unwrap();
        pointer_frame(&mut app,&ctx,size,point,true);
        let mut passes=0;
        crate::headless::run(&ctx,egui::RawInput{
            screen_rect:Some(egui::Rect::from_min_size(egui::Pos2::ZERO,size)),
            events:vec![egui::Event::PointerMoved(point),egui::Event::PointerButton{
                pos:point,button:egui::PointerButton::Primary,pressed:false,modifiers:egui::Modifiers::NONE}],
            ..Default::default()
        },|root|{
            passes+=1;
            if passes==1 {root.ctx().request_discard("help close waits for final pass");}
            app.ui(root);
            if passes==1 {assert!(app.focus_areas.is_open());}
        });
        assert!(passes>=2);assert!(!app.focus_areas.is_open());
    }

    #[test]
    fn discarded_help_shortcut_does_not_open_over_a_later_focus_or_document_choice() {
        for change in 0..3 {
            let size=egui::vec2(960.,650.);
            let (ctx,mut app,_dir)=setup(size,session::DesktopTheme::Light);
            focus_editor(&mut app,&ctx,size,0);
            let mut passes=0;
            crate::headless::run(&ctx,egui::RawInput{
                screen_rect:Some(egui::Rect::from_min_size(egui::Pos2::ZERO,size)),
                events:vec![egui::Event::ModifiersChanged(CTRL),egui::Event::Key{
                    key:egui::Key::Slash,physical_key:Some(egui::Key::Slash),pressed:true,repeat:false,modifiers:CTRL}],
                ..Default::default()
            },|root|{
                passes+=1;
                if passes==1 {root.ctx().request_discard("help must not steal a later choice");}
                app.ui(root);
                if passes==1 {
                    assert!(!app.focus_areas.is_open());
                    match change {
                        1=>root.ctx().memory_mut(|m|m.request_focus(egui::Id::new("command_input"))),
                        2=>app.activate_script_index(1),
                        _=>{},
                    }
                }
            });
            assert!(passes>=2);assert_eq!(app.focus_areas.is_open(),change==0);
        }
    }
}
