fn csv_assert_shared(before:&BTreeMap<String,matrust::Array>,app:&Workspace) {
    assert_eq!(before.len(),app.variables.len());
    for (name,value) in before {assert!(value.same_snapshot(&app.variables[name]),"{name}");}
}
#[test]
fn csv_drop_cancel_keeps_selection_documents_figures_and_shared_workspace() {
    for size in [egui::vec2(960.,650.),egui::vec2(1440.,900.)] {
        let ctx=egui::Context::default();let dir=tempfile::tempdir().unwrap();
        let path=dir.path().join("data.csv");std::fs::write(&path,"5,6\n7,8\n").unwrap();
        let mut app=folder_navigation_app(&ctx,dir.path());
        app.run("data=[1 2];Alias=data;keep=complex(3,4);plot(data);".into());finish(&mut app);
        app.selected=Some("keep".into());
        let before=app.variables.clone();let figure=app.figure.clone().unwrap();
        let text=app.documents[app.active].text.clone();let id=app.documents[app.active].id;
        drop_paths(&mut app,&ctx,vec![path]);finish(&mut app);
        assert!(app.csv_import.is_some() && app.file_action_pending() && !app.busy);
        csv_assert_shared(&before,&app);
        click_label_at(&mut app,&ctx,size,"Cancel");
        assert!(app.csv_import.is_none());csv_assert_shared(&before,&app);
        assert_eq!(app.selected.as_deref(),Some("keep"));
        assert_eq!(app.documents[app.active].id,id);assert_eq!(app.documents[app.active].text,text);
        assert!(app.figure.as_ref().unwrap().same_snapshot(&figure));
    }
}
#[test]
fn csv_open_fallback_new_name_and_explicit_replace_publish_only_approved_binding() {
    let ctx=egui::Context::default();let dir=tempfile::tempdir().unwrap();
    let path=dir.path().join("bad-name.csv");std::fs::write(&path,"5,6\n7,8\n").unwrap();
    let mut app=folder_navigation_app(&ctx,dir.path());
    app.run("data=complex([1 2],[3 4]);Alias=data;keep=true;".into());finish(&mut app);
    let old=app.variables["data"].clone();let keep=app.variables["keep"].clone();
    app.open_path(path.clone());finish(&mut app);
    click_label(&mut app,&ctx,"Use new name");click_label(&mut app,&ctx,"Import");finish(&mut app);
    assert!(app.csv_import.is_none());assert!(old.same_snapshot(&app.variables["data"]));
    assert_eq!(app.variables["data_1"].scalar(1,1).unwrap().into_real().unwrap(),8.);
    app.open_path(path);finish(&mut app);
    click_label(&mut app,&ctx,"Replace variable");finish(&mut app);
    assert!(app.csv_import.is_none());assert!(old.same_snapshot(&app.variables["Alias"]));
    assert!(keep.same_snapshot(&app.variables["keep"]));assert!(!old.same_snapshot(&app.variables["data"]));
    assert_eq!(app.variables["data"].scalar(0,1).unwrap().into_real().unwrap(),6.);
}
#[test]
fn csv_ribbon_and_workspace_menu_share_picker_preflight_and_pending_guards() {
    for menu in [false,true] {
        let ctx=egui::Context::default();let dir=tempfile::tempdir().unwrap();
        let path=dir.path().join("values.csv");std::fs::write(&path,"9\n").unwrap();
        let mut app=folder_navigation_app(&ctx,dir.path());
        app.layout.toolstrip_collapsed=false;
        {
            let picker=csv_import::picker_choice(Some(vec![path.clone()]));
            if menu {
                // The dock also paints "Workspace": open Menu, then its submenu.
                click_label(&mut app,&ctx,"Menu");
                let mut top=None;
                for _ in 0..3 {
                    top=frame(&mut app,&ctx,vec![]).shapes.iter()
                        .filter_map(|s|text_position(&s.shape,"Workspace").filter(|p|s.clip_rect.contains(*p)))
                        .min_by(|a,b|a.y.total_cmp(&b.y));
                }
                let top=top.expect("Workspace menu is painted");assert!(top.y<100.);
                for pressed in [true,false] {pointer_frame(&mut app,&ctx,egui::vec2(1440.,900.),top,pressed);}
                click_label(&mut app,&ctx,"Import CSV\u{2026}");
            }
            else {click_label(&mut app,&ctx,"Import data");}
            assert_eq!(picker.calls(),1);
        }
        frame(&mut app,&ctx,vec![]);finish(&mut app);
        assert!(app.csv_import.is_some());assert!(!app.variables.contains_key("values"));
        {
            let picker=csv_import::picker_choice(Some(vec![path]));
            app.request_csv_import_files(&ctx);assert_eq!(picker.calls(),0);
        }
        click_label(&mut app,&ctx,"Cancel");
        let picker=csv_import::picker_choice(None);
        click_label(&mut app,&ctx,"Import data");assert_eq!(picker.calls(),1);
        assert!(!app.busy && app.csv_import.is_none() && app.open_queue.is_empty());
    }
}
#[test]
fn csv_browser_double_click_uses_same_naming_decision() {
    let ctx=egui::Context::default();let dir=tempfile::tempdir().unwrap();
    std::fs::write(dir.path().join("browser.csv"),"17\n").unwrap();
    let mut app=folder_navigation_app(&ctx,dir.path());
    let size=egui::vec2(1440.,900.);
    let mut pos=None;
    for _ in 0..3 {pos=frame_at(&mut app,&ctx,size,vec![]).shapes.iter().rev().find_map(|s|text_position(&s.shape,"browser.csv"));}
    let pos=pos.expect("Actual folder row");
    for _ in 0..2 {for pressed in [true,false] {pointer_frame(&mut app,&ctx,size,pos,pressed);}}
    finish(&mut app);assert!(app.csv_import.is_some() && !app.variables.contains_key("browser"));
    click_label(&mut app,&ctx,"Import");finish(&mut app);
    assert_eq!(app.variables["browser"].scalar(0,0).unwrap().into_real().unwrap(),17.);
}
#[test]
fn csv_queue_waits_for_each_decision_and_stop_cancels_the_actual_accepted_job() {
    let ctx=egui::Context::default();let dir=tempfile::tempdir().unwrap();
    let first=dir.path().join("first.csv");let second=dir.path().join("second.csv");
    for path in [&first,&second] {std::fs::write(path,"11\n").unwrap();}
    let mut app=folder_navigation_app(&ctx,dir.path());app.run("keep=[1 2];Alias=keep;".into());finish(&mut app);
    let before=app.variables.clone();drop_paths(&mut app,&ctx,vec![first,second.clone()]);finish(&mut app);
    drop_paths(&mut app,&ctx,vec![second]);
    for _ in 0..3 {frame(&mut app,&ctx,vec![]);}
    assert_eq!(app.open_queue.len(),2);csv_assert_shared(&before,&app);
    let (send,receive)=std::sync::mpsc::sync_channel(1);
    let original=std::mem::replace(&mut app.worker.send,send);
    click_label(&mut app,&ctx,"Import");
    let job=receive.try_recv().expect("Actual Import submits once");assert!(receive.try_recv().is_err());
    assert!(app.busy);
    {
        let picker=csv_import::picker_choice(None);app.request_csv_import_files(&ctx);assert_eq!(picker.calls(),0);
    }
    click_label(&mut app,&ctx,"Stop");assert!(app.open_queue.is_empty());
    assert!(app.worker.cancel.load(Ordering::Relaxed));
    app.worker.send=original;assert!(app.worker.send.send(job).is_ok());finish(&mut app);
    csv_assert_shared(&before,&app);assert!(app.csv_import.is_some() && !app.busy);
    // The decision is retained after a cancelled read, and a real retry succeeds.
    click_label(&mut app,&ctx,"Import");finish(&mut app);
    assert_eq!(app.variables["first"].scalar(0,0).unwrap().into_real().unwrap(),11.);
    assert!(!app.variables.contains_key("second"));
}
#[test]
fn csv_name_editor_rejects_reserved_name_and_escape_beats_same_frame_import() {
    let ctx=egui::Context::default();let dir=tempfile::tempdir().unwrap();
    let path=dir.path().join("valid.csv");std::fs::write(&path,"4\n").unwrap();
    let mut app=folder_navigation_app(&ctx,dir.path());app.open_path(path);finish(&mut app);
    frame(&mut app,&ctx,vec![]);ctx.memory_mut(|m|m.request_focus(egui::Id::new("csv_import_name")));
    folder_navigation_key(&mut app,&ctx,egui::Modifiers {ctrl:true,command:true,..Default::default()},egui::Key::A);
    let selection=egui::text_edit::TextEditState::load(&ctx,egui::Id::new("csv_import_name")).unwrap().cursor.char_range().unwrap().as_sorted_char_range();
    assert_eq!(selection.start.0..selection.end.0,0..5,"Actual Select All must replace the entire name");
    let invalid=frame(&mut app,&ctx,vec![egui::Event::Text("while".into())]);
    assert!(invalid.shapes.iter().any(|s|text_position(&s.shape,"while").is_some()));
    assert!(invalid.shapes.iter().any(|s|text_position(&s.shape,"Reserved language keywords cannot be variable names").is_some()));
    let (send,receive)=std::sync::mpsc::sync_channel(1);
    let original=std::mem::replace(&mut app.worker.send,send);
    click_label(&mut app,&ctx,"Import");assert!(!app.busy && app.variables.is_empty());
    assert!(receive.try_recv().is_err(),"Disabled Import must not submit any job");
    ctx.memory_mut(|m|m.request_focus(egui::Id::new("csv_import_name")));
    folder_navigation_key(&mut app,&ctx,egui::Modifiers {ctrl:true,command:true,..Default::default()},egui::Key::A);
    let selection=egui::text_edit::TextEditState::load(&ctx,egui::Id::new("csv_import_name")).unwrap().cursor.char_range().unwrap().as_sorted_char_range();
    assert_eq!(selection.start.0..selection.end.0,0..5,"Actual Select All must replace the entire name");
    let valid=frame(&mut app,&ctx,vec![egui::Event::Text("chosen".into())]);
    assert!(valid.shapes.iter().any(|s|text_position(&s.shape,"chosen").is_some()));
    let mut pos=None;
    for _ in 0..3 {pos=frame(&mut app,&ctx,vec![]).shapes.iter().rev().find_map(|s|text_position(&s.shape,"Import"));}
    let pos=pos.unwrap();pointer_frame(&mut app,&ctx,egui::vec2(1440.,900.),pos,true);
    frame(&mut app,&ctx,vec![egui::Event::PointerButton{pos,button:egui::PointerButton::Primary,pressed:false,modifiers:egui::Modifiers::NONE},egui::Event::Key{key:egui::Key::Escape,physical_key:None,pressed:true,repeat:false,modifiers:egui::Modifiers::NONE}]);
    assert!(app.csv_import.is_none() && !app.busy && app.variables.is_empty());
    assert!(receive.try_recv().is_err(),"Escape must prevent even a same-frame Import submission");
    app.worker.send=original;
}
#[test]
fn csv_viewport_close_keeps_one_decision_idle_and_loading_then_preserves_dirty_close_flow() {
    let ctx=egui::Context::default();let dir=tempfile::tempdir().unwrap();
    let path=dir.path().join("value.csv");std::fs::write(&path,"2\n").unwrap();
    let mut app=folder_navigation_app(&ctx,dir.path());app.run("keep=[1 2];plot(keep);".into());finish(&mut app);
    app.documents[app.active].text="unsaved=1;".into();app.documents[app.active].changed();
    let before=app.variables.clone();let figure=app.figure.clone().unwrap();
    app.open_path(path);finish(&mut app);
    let close=|app:&mut Workspace| {
        let mut input=egui::RawInput{screen_rect:Some(egui::Rect::from_min_size(egui::Pos2::ZERO,egui::vec2(960.,650.))),..Default::default()};
        input.viewports.get_mut(&egui::ViewportId::ROOT).unwrap().events.push(egui::ViewportEvent::Close);
        let output=crate::headless::run(&ctx,input,|root|app.ui(root));
        assert!(output.viewport_output[&egui::ViewportId::ROOT].commands.iter().any(|c|matches!(c,egui::ViewportCommand::CancelClose)));
    };
    close(&mut app);assert!(app.csv_import.is_some() && app.figure_close.is_none() && !app.pending_quit && !app.allow_close);
    let (send,receive)=std::sync::mpsc::sync_channel(1);let original=std::mem::replace(&mut app.worker.send,send);
    click_label(&mut app,&ctx,"Import");let job=receive.try_recv().unwrap();
    close(&mut app);assert!(app.busy && app.csv_import.is_some() && app.figure_close.is_none() && !app.pending_quit);
    click_label(&mut app,&ctx,"Stop");app.worker.send=original;assert!(app.worker.send.send(job).is_ok());finish(&mut app);
    click_label(&mut app,&ctx,"Cancel");csv_assert_shared(&before,&app);assert!(app.figure.as_ref().unwrap().same_snapshot(&figure));
    close(&mut app);assert!(app.figure_close.is_some() || app.pending_quit);
    assert!(app.documents[app.active].dirty && !app.allow_close);
}
#[test]
fn csv_import_accepts_once_during_discarded_ui_passes() {
    let ctx=egui::Context::default();let dir=tempfile::tempdir().unwrap();
    let path=dir.path().join("data.csv");std::fs::write(&path,"2\n").unwrap();
    let mut app=folder_navigation_app(&ctx,dir.path());app.run("data=1;Alias=data;".into());finish(&mut app);
    app.open_path(path);finish(&mut app);
    let mut pos=None;for _ in 0..3 {pos=frame(&mut app,&ctx,vec![]).shapes.iter().rev().find_map(|s|text_position(&s.shape,"Replace variable"));}
    let pos=pos.unwrap();pointer_frame(&mut app,&ctx,egui::vec2(1440.,900.),pos,true);
    let (send,receive)=std::sync::mpsc::sync_channel(1);let original=std::mem::replace(&mut app.worker.send,send);
    let mut passes=0;
    let _=crate::headless::run(&ctx,egui::RawInput{screen_rect:Some(egui::Rect::from_min_size(egui::Pos2::ZERO,egui::vec2(1440.,900.))),events:vec![egui::Event::PointerButton{pos,button:egui::PointerButton::Primary,pressed:false,modifiers:egui::Modifiers::NONE}],..Default::default()},|root|{
        passes+=1;app.ui(root);if passes==1 {root.ctx().request_discard("CSV acceptance must publish only once");}
    });
    assert!(passes>=2 && app.busy);let job=receive.try_recv().expect("Accepted replacement");assert!(receive.try_recv().is_err());
    app.worker.send=original;assert!(app.worker.send.send(job).is_ok());finish(&mut app);
    assert_eq!(app.variables["data"].scalar(0,0).unwrap().into_real().unwrap(),2.);
    assert_eq!(app.variables["Alias"].scalar(0,0).unwrap().into_real().unwrap(),1.);
}

mod csv_import_picker_cases {
    use super::*;
    const SIZE: egui::Vec2 = egui::vec2(1440., 900.);

    fn setup(size: egui::Vec2) -> (egui::Context, Workspace, tempfile::TempDir) {
        let ctx = egui::Context::default();
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("picked.csv"), "7,8\n9,10\n").unwrap();
        let mut app = folder_navigation_app(&ctx, dir.path());
        app.run("keep=[1 2];Alias=keep;plot(keep);".into());
        finish(&mut app); finish_folder(&mut app);
        app.documents[app.active].text = "unsaved_csv_picker=1;".into();
        app.documents[app.active].changed();
        app.layout.toolstrip_collapsed = false;
        for _ in 0..3 { frame_at(&mut app, &ctx, size, vec![]); }
        assert!(!app.busy && !app.folder_browser.navigating() && !app.file_action_pending());
        (ctx, app, dir)
    }
    fn position(app: &mut Workspace, ctx: &egui::Context, size: egui::Vec2, menu: bool) -> egui::Pos2 {
        // Only a hidden toolstrip folds the tabs into menus.
        let folded = app.layout.toolstrip_collapsed;
        let tabs: &[&str] = if menu { &["Menu", "Workspace"] } else if folded { &["Home"] } else { &[] };
        for tab in tabs {
            let output = frame_at(app, ctx, size, vec![]);
            let top = output.shapes.iter().filter_map(|s| text_position(&s.shape, tab)
                .filter(|p| s.clip_rect.contains(*p))).min_by(|a,b| a.y.total_cmp(&b.y)).unwrap();
            assert!(top.y < 100.);
            pointer_frame(app, ctx, size, top, true);
            pointer_frame(app, ctx, size, top, false);
        }
        let label = if menu { "Import CSV\u{2026}" } else if folded { "Import CSV" } else { "Import data" };
        let mut point = None;
        for _ in 0..3 {
            point = frame_at(app, ctx, size, vec![]).shapes.iter().rev()
                .find_map(|s| text_position(&s.shape, label).filter(|p| s.clip_rect.contains(*p)));
        }
        point.unwrap_or_else(|| panic!("Actual {label} picker trigger at {size:?}"))
    }
    fn release_discarded(app: &mut Workspace, ctx: &egui::Context, size: egui::Vec2,
        point: egui::Pos2, picker: &csv_import::PickerChoice,
        mut between: impl FnMut(&mut Workspace, &egui::Context)) {
        let calls = picker.calls();
        let mut passes = 0;
        crate::headless::run(ctx, egui::RawInput {
            screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, size)),
            events: vec![egui::Event::PointerMoved(point), egui::Event::PointerButton {
                pos: point, button: egui::PointerButton::Primary, pressed: false,
                modifiers: egui::Modifiers::NONE,
            }], ..Default::default()
        }, |root| {
            passes += 1;
            if passes == 1 { root.ctx().request_discard("CSV picker waits for final admission"); }
            app.ui(root);
            if passes == 1 {
                assert_eq!(picker.calls(), calls);
                assert!(app.open_queue.is_empty() && !app.busy);
                between(app, ctx);
            }
        });
        assert!(passes >= 2);
    }

    #[test]
    fn csv_picker_actual_ribbon_and_menu_release_wait_then_import_once_in_both_sizes() {
        for size in [egui::vec2(960.,650.), SIZE] {
            for menu in [false,true] {
                let (ctx,mut app,dir) = setup(size);
                let before = app.variables.clone();
                let document = app.documents[app.active].clone();
                let figure = app.figure.clone().unwrap();
                let picker = csv_import::picker_choice(Some(vec![dir.path().join("picked.csv")]));
                let point = position(&mut app,&ctx,size,menu);
                pointer_frame(&mut app,&ctx,size,point,true);
                release_discarded(&mut app,&ctx,size,point,&picker,|_,_| {});
                assert_eq!(picker.calls(),1);
                frame_at(&mut app,&ctx,size,vec![]); finish(&mut app);
                assert!(app.csv_import.is_some());
                assert!(!app.variables.contains_key("picked"));
                csv_assert_shared(&before,&app);
                assert!(app.documents[app.active].same_snapshot(&document));
                assert!(app.figure.as_ref().unwrap().same_snapshot(&figure));
                click_label_at(&mut app,&ctx,size,"Import"); finish(&mut app);
                assert!(app.csv_import.is_none());
                assert_eq!(app.variables["picked"].scalar(1,1).unwrap().into_real().unwrap(),10.);
                for (name,value) in &before { assert!(value.same_snapshot(&app.variables[name])); }
                assert!(app.variables["keep"].same_snapshot(&app.variables["Alias"]));
                assert!(app.documents[app.active].same_snapshot(&document));
                assert!(app.figure.as_ref().unwrap().same_snapshot(&figure));
                frame_at(&mut app,&ctx,size,vec![]);
                assert_eq!(picker.calls(),1);
            }
        }
    }

    #[test]
    fn csv_picker_cancel_is_consumed_even_when_discard_requested_after_picker() {
        let (ctx,mut app,_dir) = setup(SIZE);
        let before = app.variables.clone();
        let picker = csv_import::picker_choice(None);
        for (expected,menu) in [(1,false),(2,true)] {
            let point = position(&mut app,&ctx,SIZE,menu);
            pointer_frame(&mut app,&ctx,SIZE,point,true);
            release_discarded(&mut app,&ctx,SIZE,point,&picker,|_,_| {});
            assert_eq!(picker.calls(),expected);
            assert!(app.open_queue.is_empty() && !app.busy && app.csv_import.is_none());
        }
        let point = position(&mut app,&ctx,SIZE,false);
        pointer_frame(&mut app,&ctx,SIZE,point,true);
        let mut passes = 0;
        crate::headless::run(&ctx,egui::RawInput {
            screen_rect:Some(egui::Rect::from_min_size(egui::Pos2::ZERO,SIZE)),
            events:vec![egui::Event::PointerMoved(point),egui::Event::PointerButton {
                pos:point,button:egui::PointerButton::Primary,pressed:false,
                modifiers:egui::Modifiers::NONE}],..Default::default()
        },|root| {
            passes+=1; app.ui(root);
            if passes==1 {root.ctx().request_discard("Completed picker Cancel cannot replay");}
        });
        assert!(passes>=2); assert_eq!(picker.calls(),3);
        assert!(app.open_queue.is_empty() && !app.busy && app.csv_import.is_none());
        csv_assert_shared(&before,&app);
    }

    fn invalidate(app: &mut Workspace, case: usize) {
        match case {
            0=>app.stop_work(), 1=>app.busy=true, 2=>app.pending_quit=true,
            3=>app.allow_close=true, 4=>app.recovering_session=true,
            5=>app.recovery_blocked=true, 6=>app.folder.push("different"),
            7=>app.new_document("new_csv_owner=1;"),
            8=>{app.begin_workspace_action(false,true);assert!(app.workspace_tools.dialog.is_some());},
            _=>unreachable!(),
        }
    }
    #[test]
    fn csv_picker_rejects_late_stop_busy_quit_recovery_modal_folder_document_and_focus() {
        for case in 0..10 {
            let (ctx,mut app,_dir) = setup(SIZE);
            let before = app.variables.clone();
            let picker = csv_import::picker_callback(|_| panic!("Stale action opened CSV picker"));
            let point = position(&mut app,&ctx,SIZE,true);
            pointer_frame(&mut app,&ctx,SIZE,point,true);
            release_discarded(&mut app,&ctx,SIZE,point,&picker,|app,ctx| {
                if case==9 {ctx.memory_mut(|m|m.request_focus(egui::Id::new("late_csv_owner")));}
                else {invalidate(app,case);}
            });
            assert_eq!(picker.calls(),0,"case {case}");
            assert!(app.open_queue.is_empty() && app.csv_import.is_none());
            csv_assert_shared(&before,&app);
            if case==1 {app.busy=false;}
            frame_at(&mut app,&ctx,SIZE,vec![]);
            assert_eq!(picker.calls(),0,"No stale replay on later frame, case {case}");
        }
    }
    #[test]
    fn csv_picker_revalidates_after_native_return_before_queueing_paths() {
        for case in 0..9 {
            let (ctx,mut app,dir) = setup(SIZE);
            let before = app.variables.clone();
            let picker = csv_import::picker_callback(|app| {
                let path=app.folder.join("picked.csv");
                let case=app.status.strip_prefix("CSV picker case ").unwrap().parse().unwrap();
                invalidate(app,case); Some(vec![path])
            });
            let point = position(&mut app,&ctx,SIZE,false);
            pointer_frame(&mut app,&ctx,SIZE,point,true);
            app.status=format!("CSV picker case {case}");
            pointer_frame(&mut app,&ctx,SIZE,point,false);
            assert_eq!(picker.calls(),1,"case {case}");
            assert_eq!(app.busy,case==1);
            assert!(app.open_queue.is_empty() && app.csv_import.is_none());
            csv_assert_shared(&before,&app);
            assert_eq!(std::fs::read(dir.path().join("picked.csv")).unwrap(),b"7,8\n9,10\n");
        }
    }
    #[test]
    fn global_search_import_csv_dispatches_after_its_owned_popup_closes() {
        let (ctx,mut app,_dir) = setup(SIZE);
        let picker=csv_import::picker_choice(None);
        click_label_at(&mut app,&ctx,SIZE,"Search functions and actions");
        for _ in 0..2 {frame_at(&mut app,&ctx,SIZE,vec![]);}
        assert!(app.global_search.is_open());
        assert!(ctx.memory(|m|m.has_focus(global_search::query_id())));
        frame_at(&mut app,&ctx,SIZE,vec![egui::Event::Text("Import CSV".into())]);
        frame_at(&mut app,&ctx,SIZE,vec![]);
        command_first_key(&mut app,&ctx,SIZE,egui::Key::Enter,egui::Modifiers::NONE);
        assert!(!app.global_search.is_open()); assert_eq!(picker.calls(),1);
        assert!(!app.busy && app.open_queue.is_empty() && app.csv_import.is_none());
    }
}


mod csv_naming_final_pass_cases {
    use super::*;
    const SIZE: egui::Vec2 = egui::vec2(1440., 900.);

    fn setup(size: egui::Vec2, occupied: bool) -> (egui::Context, Workspace, tempfile::TempDir) {
        let ctx = egui::Context::default();
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("data.csv");
        std::fs::write(&path, "5,6\n7,8\n").unwrap();
        let mut app = folder_navigation_app(&ctx, dir.path());
        app.run(if occupied { "data=[1 2];Alias=data;keep=true;plot(data);" }
            else { "Alias=[1 2];keep=true;plot(Alias);" }.into());
        finish(&mut app); finish_folder(&mut app);
        app.open_path(path); finish(&mut app); finish_folder(&mut app);
        for _ in 0..3 { frame_at(&mut app, &ctx, size, vec![]); }
        assert!(app.csv_import.is_some() && !app.busy);
        (ctx, app, dir)
    }
    fn press(app: &mut Workspace, ctx: &egui::Context, size: egui::Vec2, label: &str) -> egui::Pos2 {
        let mut point = None;
        for _ in 0..3 {
            point = frame_at(app, ctx, size, vec![]).shapes.iter().rev()
                .find_map(|s| text_position(&s.shape, label).filter(|p| s.clip_rect.contains(*p)));
        }
        let point = point.unwrap_or_else(|| panic!("Actual CSV naming control {label}"));
        pointer_frame(app, ctx, size, point, true);
        point
    }
    fn discarded_release(app: &mut Workspace, ctx: &egui::Context, size: egui::Vec2,
        point: egui::Pos2, escape: bool,
        mut between: impl FnMut(&mut Workspace, &egui::Context)) {
        let mut events = vec![egui::Event::PointerMoved(point), egui::Event::PointerButton {
            pos: point, button: egui::PointerButton::Primary, pressed: false,
            modifiers: egui::Modifiers::NONE,
        }];
        if escape { events.push(egui::Event::Key {key:egui::Key::Escape,physical_key:None,
            pressed:true,repeat:false,modifiers:egui::Modifiers::NONE}); }
        let mut passes = 0;
        crate::headless::run(ctx, egui::RawInput {
            screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, size)),
            events, ..Default::default()
        }, |root| {
            passes += 1;
            if passes == 1 { root.ctx().request_discard("CSV naming must await final pass"); }
            app.ui(root);
            if passes == 1 { between(app, ctx); }
        });
        assert!(passes >= 2);
    }

    #[test]
    fn actual_import_and_replace_wait_for_final_pass_then_publish_original_request_once() {
        for size in [egui::vec2(960.,650.), SIZE] {
            for occupied in [false, true] {
                let (ctx, mut app, _dir) = setup(size, occupied);
                let before = app.variables.clone();
                let figure = app.figure.clone().unwrap();
                let document = app.documents[app.active].clone();
                let point = press(&mut app, &ctx, size, if occupied {"Replace variable"} else {"Import"});
                let (send, receive) = std::sync::mpsc::sync_channel(1);
                let original = std::mem::replace(&mut app.worker.send, send);
                discarded_release(&mut app, &ctx, size, point, false, |app, _| {
                    assert!(!app.busy && app.csv_import.is_some());
                    assert!(receive.try_recv().is_err(), "Discarded pass must not enqueue a worker request");
                    csv_assert_shared(&before, app);
                });
                assert!(app.busy);
                let request = receive.try_recv().expect("Exactly one final-pass CSV request");
                let Job::ImportCsv(approved) = &request.job else { panic!("Expected actual CSV job"); };
                assert_eq!(approved.name, "data");
                if occupied {
                    assert!(approved.expected.as_ref().unwrap().same_snapshot(&before["data"]));
                } else { assert!(approved.expected.is_none()); }
                assert!(receive.try_recv().is_err());
                app.worker.send = original;
                assert!(app.worker.send.send(request).is_ok()); finish(&mut app);
                assert!(app.csv_import.is_none());
                assert_eq!(app.variables["data"].as_numeric().unwrap().data(), &[5.,7.,6.,8.]);
                assert!(before["Alias"].same_snapshot(&app.variables["Alias"]));
                assert!(before["keep"].same_snapshot(&app.variables["keep"]));
                assert!(figure.same_snapshot(app.figure.as_ref().unwrap()));
                assert_eq!(app.documents[app.active].id, document.id);
                assert_eq!(app.documents[app.active].text, document.text);
            }
        }
    }

    #[test]
    fn late_stop_ownership_dialog_and_equal_value_new_storage_reject_retained_replacement() {
        for case in 0..10 {
            let (ctx, mut app, dir) = setup(SIZE, true);
            let before = app.variables.clone();
            let point = press(&mut app, &ctx, SIZE, "Replace variable");
            let (send, receive) = std::sync::mpsc::sync_channel(1);
            let original = std::mem::replace(&mut app.worker.send, send);
            discarded_release(&mut app, &ctx, SIZE, point, false, |app, ctx| {
                assert!(!app.busy && receive.try_recv().is_err());
                match case {
                    0 => app.stop_work(),
                    1 => app.busy = true,
                    2 => app.pending_quit = true,
                    3 => app.recovering_session = true,
                    4 => ctx.memory_mut(|m|m.request_focus(egui::Id::new("csv_naming_late_focus"))),
                    5 => app.new_document("changed_csv_owner=1;"),
                    6 => {
                        // Replace the old modal first, then admit the real competing modal.
                        app.csv_import = None;
                        app.begin_workspace_action(false, true);
                        assert!(app.workspace_tools.dialog.is_some());
                    }
                    7 => {
                        app.csv_import = None;
                        app.begin_csv_import(dir.path().join("data.csv"));
                        assert!(app.csv_import.is_some());
                    }
                    8 => {
                        let equal = matrust::Matrix::from_column_vec(1,2,vec![1.,2.]).unwrap();
                        app.variables.insert("data".into(), equal.into());
                        assert!(!before["data"].same_snapshot(&app.variables["data"]));
                    }
                    9 => app.allow_close = true,
                    _ => unreachable!(),
                }
            });
            assert!(receive.try_recv().is_err(), "Stale naming decision, case {case}");
            assert!(before["Alias"].same_snapshot(&app.variables["Alias"]));
            if case == 0 {
                assert!(app.worker.cancel.load(Ordering::Relaxed), "Stop remains effective without a new job resetting it");
                assert!(!app.busy);
                frame_at(&mut app,&ctx,SIZE,vec![]);
                assert!(receive.try_recv().is_err(), "No later-frame replay");
                // A genuinely new gesture is still a usable retry.
                click_label_at(&mut app,&ctx,SIZE,"Replace variable");
                let request = receive.try_recv().unwrap();
                assert!(receive.try_recv().is_err());
                app.worker.send = original;
                assert!(app.worker.send.send(request).is_ok()); finish(&mut app);
                assert_eq!(app.variables["data"].scalar(0,0).unwrap().into_real().unwrap(),5.);
            } else {
                app.worker.send = original;
                if case == 1 { app.busy = false; }
            }
        }
    }

    #[test]
    fn naming_controls_wait_for_final_pass_and_escape_cancels_retained_submit() {
        for label in ["Use new name", "Cancel queued files", "Cancel"] {
            let (ctx, mut app, dir) = setup(SIZE, true);
            app.queue_open_paths([dir.path().join("later.csv")]);
            let before = app.variables.clone();
            let point = press(&mut app, &ctx, SIZE, label);
            let (send, receive) = std::sync::mpsc::sync_channel(1);
            let original = std::mem::replace(&mut app.worker.send, send);
            discarded_release(&mut app,&ctx,SIZE,point,false,|app,_| {
                assert!(app.csv_import.is_some() && !app.busy && app.open_queue.len()==1);
                assert!(receive.try_recv().is_err()); csv_assert_shared(&before,app);
            });
            assert!(receive.try_recv().is_err());
            match label {
                "Use new name" => {
                    let output=frame_at(&mut app,&ctx,SIZE,vec![]);
                    assert!(output.shapes.iter().any(|s|text_position(&s.shape,"data_1").is_some()));
                    assert_eq!(app.open_queue.len(),1);
                    click_label_at(&mut app,&ctx,SIZE,"Import");
                    let request=receive.try_recv().unwrap();
                    assert!(receive.try_recv().is_err());
                    app.open_queue.clear();app.worker.send=original;
                    assert!(app.worker.send.send(request).is_ok());finish(&mut app);
                    assert!(before["data"].same_snapshot(&app.variables["data"]));
                    assert_eq!(app.variables["data_1"].scalar(0,0).unwrap().into_real().unwrap(),5.);
                }
                "Cancel queued files" => {
                    assert!(app.open_queue.is_empty() && app.csv_import.is_some());app.worker.send=original;
                }
                "Cancel" => {
                    assert!(app.csv_import.is_none());assert_eq!(app.open_queue.len(),1);app.worker.send=original;
                }
                _=>unreachable!(),
            }
        }
        let (ctx,mut app,_dir)=setup(SIZE,true);
        let before=app.variables.clone();
        let point=press(&mut app,&ctx,SIZE,"Replace variable");
        let (send,receive)=std::sync::mpsc::sync_channel(1);
        let original=std::mem::replace(&mut app.worker.send,send);
        discarded_release(&mut app,&ctx,SIZE,point,true,|app,_| {
            assert!(app.csv_import.is_some() && !app.busy && receive.try_recv().is_err());
        });
        assert!(app.csv_import.is_none() && !app.busy && receive.try_recv().is_err());
        csv_assert_shared(&before,&app);app.worker.send=original;
    }

    #[test]
    fn retained_local_stop_does_not_cancel_a_later_unrelated_job_after_import_completion() {
        let (ctx,mut app,_dir)=setup(SIZE,false);
        let (send,receive)=std::sync::mpsc::sync_channel(1);
        let original=std::mem::replace(&mut app.worker.send,send.clone());
        click_label_at(&mut app,&ctx,SIZE,"Import");
        let mut import=Some(receive.try_recv().unwrap());
        assert!(app.busy && app.csv_import.is_some());
        let point=press(&mut app,&ctx,SIZE,"Stop");
        discarded_release(&mut app,&ctx,SIZE,point,false,|app,_| {
            assert!(!app.worker.cancel.load(Ordering::Relaxed), "Discarded Stop has no worker side effect");
            app.worker.send=original.clone();
            assert!(app.worker.send.send(import.take().unwrap()).is_ok());finish(app);
            assert!(app.csv_import.is_none() && !app.busy);
            app.worker.send=send.clone();
            app.run("unrelated_after_csv=23;".into());
            assert!(app.busy && !app.worker.cancel.load(Ordering::Relaxed));
        });
        assert!(!app.worker.cancel.load(Ordering::Relaxed), "Late Stop must not target the new worker job");
        let request=receive.try_recv().unwrap();assert!(receive.try_recv().is_err());
        app.worker.send=original;assert!(app.worker.send.send(request).is_ok());finish(&mut app);
        assert_eq!(app.variables["unrelated_after_csv"].scalar(0,0).unwrap().into_real().unwrap(),23.);
        assert_eq!(app.variables["data"].scalar(0,0).unwrap().into_real().unwrap(),5.);
    }

    #[test]
    fn discarded_loading_queue_cancel_and_stop_preserve_atomic_worker_retry() {
        let (ctx,mut app,dir)=setup(SIZE,true);
        let before=app.variables.clone();
        let (send,receive)=std::sync::mpsc::sync_channel(1);
        let original=std::mem::replace(&mut app.worker.send,send);
        click_label_at(&mut app,&ctx,SIZE,"Replace variable");
        let request=receive.try_recv().unwrap();assert!(app.busy);
        app.queue_open_paths([dir.path().join("later.csv")]);
        let point=press(&mut app,&ctx,SIZE,"Cancel queued files");
        discarded_release(&mut app,&ctx,SIZE,point,false,|app,_| {
            assert_eq!(app.open_queue.len(),1);
            assert!(app.busy && !app.worker.cancel.load(Ordering::Relaxed));
        });
        assert!(app.open_queue.is_empty() && app.busy);
        assert!(!app.worker.cancel.load(Ordering::Relaxed));
        app.queue_open_paths([dir.path().join("later.csv")]);
        let point=press(&mut app,&ctx,SIZE,"Stop");
        discarded_release(&mut app,&ctx,SIZE,point,false,|app,_| {
            assert_eq!(app.open_queue.len(),1);
            assert!(!app.worker.cancel.load(Ordering::Relaxed));
        });
        assert!(app.open_queue.is_empty() && app.worker.cancel.load(Ordering::Relaxed));
        assert!(receive.try_recv().is_err());
        app.worker.send=original;assert!(app.worker.send.send(request).is_ok());finish(&mut app);
        assert!(app.csv_import.is_some() && !app.busy);csv_assert_shared(&before,&app);
        click_label_at(&mut app,&ctx,SIZE,"Replace variable");finish(&mut app);
        assert_eq!(app.variables["data"].scalar(0,0).unwrap().into_real().unwrap(),5.);
        assert!(before["Alias"].same_snapshot(&app.variables["Alias"]));
        assert!(!app.variables.contains_key("later"));
    }
}
