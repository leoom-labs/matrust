mod workspace_mat_save_cases {
    use super::*;
    const SIZE: egui::Vec2 = egui::vec2(1440., 900.);

    fn setup(size: egui::Vec2) -> (egui::Context, Workspace, tempfile::TempDir) {
        let ctx = egui::Context::default();
        let mut app = Workspace::with_context(ctx.clone(), true);
        let dir = tempfile::tempdir().unwrap();
        assert!(app.change_folder(dir.path().to_path_buf()));
        finish_folder(&mut app);
        app.run("source=[1 2;3 4];alias=source;mask=logical([1 0]);z=complex([1 2],[3 4]);text='saved';cells={source,text,mask};".into());
        finish(&mut app); finish_folder(&mut app);
        for _ in 0..3 { frame_at(&mut app, &ctx, size, vec![]); }
        (ctx, app, dir)
    }
    fn position(output: &egui::FullOutput, label: &str) -> egui::Pos2 {
        output.shapes.iter().rev().find_map(|shape| text_position(&shape.shape, label))
            .unwrap_or_else(|| panic!("Missing actual menu label {label}"))
    }
    fn format_position(app: &mut Workspace, ctx: &egui::Context, size: egui::Vec2, v73: bool) -> egui::Pos2 {
        click_label_at(app, ctx, size, "Menu");
        let output = frame_at(app, ctx, size, vec![]);
        // The menu is above the identically named Workspace pane heading.
        let point = output.shapes.iter().filter_map(|shape| text_position(&shape.shape, "Workspace"))
            .min_by(|a,b| a.y.total_cmp(&b.y)).expect("Workspace menu");
        pointer_frame(app, ctx, size, point, true);
        pointer_frame(app, ctx, size, point, false);
        click_label_at(app, ctx, size, "Save workspace");
        let mut output = frame_at(app, ctx, size, vec![]);
        for _ in 0..2 { output = frame_at(app, ctx, size, vec![]); }
        position(&output, if v73 { "MATLAB v7.3 (HDF5)" } else { "MATLAB v7 (Level-5)" })
    }
    fn release_discarded(app: &mut Workspace, ctx: &egui::Context, size: egui::Vec2,
        point: egui::Pos2, mut between: impl FnMut(&mut Workspace, &egui::Context)) {
        let calls = app.workspace_mat_save.picker_calls;
        let mut passes = 0;
        crate::headless::run(ctx, egui::RawInput {
            screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, size)),
            events: vec![egui::Event::PointerMoved(point), egui::Event::PointerButton {
                pos: point, button: egui::PointerButton::Primary, pressed: false,
                modifiers: egui::Modifiers::NONE,
            }], ..Default::default()
        }, |root| {
            passes += 1;
            if passes == 1 { root.ctx().request_discard("MAT Save waits for final menu admission"); }
            app.ui(root);
            if passes == 1 {
                assert_eq!(app.workspace_mat_save.picker_calls, calls);
                assert!(!app.busy, "No worker dispatch on the discarded format release");
                between(app, ctx);
            }
        });
        assert!(passes >= 2);
    }
    fn same_values(a: &matrust::Array, b: &matrust::Array) {
        use matrust::Array;
        assert_eq!((a.rows(), a.cols(), a.class()), (b.rows(), b.cols(), b.class()));
        match (a,b) {
            (Array::Numeric(a), Array::Numeric(b)) => assert!(a.data().iter().zip(b.data()).all(|(a,b)| a.to_bits()==b.to_bits())),
            (Array::Logical(a), Array::Logical(b)) => assert_eq!(a.data(), b.data()),
            (Array::Complex(a), Array::Complex(b)) => assert!(a.data().iter().zip(b.data()).all(|(a,b)| a.re.to_bits()==b.re.to_bits() && a.im.to_bits()==b.im.to_bits())),
            (Array::Character(a), Array::Character(b)) => assert_eq!(a.units(), b.units()),
            (Array::Cell(a), Array::Cell(b)) => for i in 0..a.len() { same_values(a.get_linear(i).unwrap(), b.get_linear(i).unwrap()); },
            _ => panic!("Unexpected MAT roundtrip class"),
        }
    }

    #[test]
    fn mat_format_menu_release_waits_then_saves_both_real_worker_formats_and_keeps_aliases() {
        for size in [egui::vec2(960.,650.), SIZE] {
            for v73 in [false, true] {
                let (ctx,mut app,dir) = setup(size);
                let before = app.variables.clone();
                app.workspace_mat_save.picker = Some(|app| Some(app.folder.join("workspace.mat")));
                let point = format_position(&mut app,&ctx,size,v73);
                pointer_frame(&mut app,&ctx,size,point,true);
                release_discarded(&mut app,&ctx,size,point,|_,_| {});
                assert_eq!(app.workspace_mat_save.picker_calls,1);
                assert!(app.busy);
                finish(&mut app);
                let path=dir.path().join("workspace.mat");
                let bytes=std::fs::read(&path).unwrap();
                if v73 { assert_eq!(&bytes[512..520], b"\x89HDF\r\n\x1a\n"); }
                else { assert!(bytes.starts_with(b"MATLAB 5.0 MAT-file")); }
                let cancel=std::sync::atomic::AtomicBool::new(false);
                let catalog=matrust::files::inspect_workspace(&path,&cancel).unwrap();
                let names=before.keys().cloned().collect();
                let loaded=matrust::files::load_selected_arrays(&catalog,&names,&cancel).unwrap();
                assert_eq!(loaded.len(),before.len());
                for (name,value) in &before {
                    same_values(value,&loaded[name]);
                    assert!(value.same_snapshot(&app.variables[name]));
                }
                assert!(app.variables["source"].same_snapshot(&app.variables["alias"]));
                frame_at(&mut app,&ctx,size,vec![]);
                assert_eq!(app.workspace_mat_save.picker_calls,1);
            }
        }
    }

    #[test]
    fn mat_picker_cancel_is_consumed_once_but_another_real_gesture_remains_usable() {
        let (ctx,mut app,_dir)=setup(SIZE);
        app.workspace_mat_save.picker=Some(|_| None);
        for (expected,v73) in [(1,false),(2,true)] {
            let point=format_position(&mut app,&ctx,SIZE,v73);
            pointer_frame(&mut app,&ctx,SIZE,point,true);
            release_discarded(&mut app,&ctx,SIZE,point,|_,_| {});
            assert_eq!(app.workspace_mat_save.picker_calls,expected);
            assert!(!app.busy);
            frame_at(&mut app,&ctx,SIZE,vec![]);
            assert_eq!(app.workspace_mat_save.picker_calls,expected);
        }
        let point=format_position(&mut app,&ctx,SIZE,false);
        pointer_frame(&mut app,&ctx,SIZE,point,true);
        let mut passes=0;
        crate::headless::run(&ctx,egui::RawInput {
            screen_rect:Some(egui::Rect::from_min_size(egui::Pos2::ZERO,SIZE)),
            events:vec![egui::Event::PointerMoved(point),egui::Event::PointerButton {
                pos:point,button:egui::PointerButton::Primary,pressed:false,modifiers:egui::Modifiers::NONE}],
            ..Default::default()
        },|root| {
            passes+=1;app.ui(root);
            if passes==1 {root.ctx().request_discard("Later layout cannot replay picker Cancel");}
        });
        assert!(passes>=2);assert_eq!(app.workspace_mat_save.picker_calls,3);assert!(!app.busy);
    }

    fn invalidate(app: &mut Workspace, case: usize) {
        match case {
            0=>app.stop_work(), 1=>app.busy=true, 2=>app.pending_quit=true,
            3=>app.allow_close=true, 4=>app.recovering_session=true,
            5=>app.recovery_blocked=true, 6=>app.folder.push("different"),
            7=>app.new_document("different_owner=1;"),
            8=>{ app.begin_workspace_action(false,true);assert!(app.workspace_tools.dialog.is_some()); },
            _=>unreachable!(),
        }
    }
    #[test]
    fn mat_save_rejects_late_stop_busy_modal_quit_recovery_and_owner_changes() {
        for case in 0..10 {
            let (ctx,mut app,_dir)=setup(SIZE);
            app.workspace_mat_save.picker=Some(|_| panic!("Stale MAT action opened picker"));
            let before=app.variables["source"].clone();
            let point=format_position(&mut app,&ctx,SIZE,false);
            pointer_frame(&mut app,&ctx,SIZE,point,true);
            release_discarded(&mut app,&ctx,SIZE,point,|app,ctx| {
                if case==9 {ctx.memory_mut(|m|m.request_focus(egui::Id::new("late_mat_owner")));}
                else {invalidate(app,case);}
            });
            assert_eq!(app.workspace_mat_save.picker_calls,0,"case {case}");
            assert!(before.same_snapshot(&app.variables["source"]));
            if case==1 {app.busy=false;}
            frame_at(&mut app,&ctx,SIZE,vec![]);
            assert_eq!(app.workspace_mat_save.picker_calls,0);
        }
    }
    #[test]
    fn mat_save_revalidates_after_picker_before_dispatching_the_existing_job() {
        for case in 0..9 {
            let (ctx,mut app,dir)=setup(SIZE);
            app.workspace_mat_save.picker=Some(|app| {
                let path=app.folder.join("must-not-exist.mat");
                let case=app.status.strip_prefix("MAT test case ").unwrap().parse().unwrap();
                invalidate(app,case);Some(path)
            });
            let point=format_position(&mut app,&ctx,SIZE,true);
            pointer_frame(&mut app,&ctx,SIZE,point,true);
            app.status=format!("MAT test case {case}");
            pointer_frame(&mut app,&ctx,SIZE,point,false);
            assert_eq!(app.workspace_mat_save.picker_calls,1,"case {case}");
            assert_eq!(app.busy,case==1);
            assert!(!dir.path().join("must-not-exist.mat").exists());
        }
    }

    #[test]
    fn global_search_final_action_can_queue_mat_save_after_its_popup_closes() {
        let (ctx,mut app,_dir)=setup(SIZE);
        app.workspace_mat_save.picker=Some(|_| None);
        click_label_at(&mut app,&ctx,SIZE,"Search functions and actions");
        for _ in 0..2 {frame_at(&mut app,&ctx,SIZE,vec![]);}
        assert!(app.global_search.is_open());
        assert!(ctx.memory(|m|m.has_focus(global_search::query_id())));
        frame_at(&mut app,&ctx,SIZE,vec![egui::Event::Text("MATLAB v7.3".into())]);
        frame_at(&mut app,&ctx,SIZE,vec![]);
        command_first_key(&mut app,&ctx,SIZE,egui::Key::Enter,egui::Modifiers::NONE);
        assert!(!app.global_search.is_open());
        assert_eq!(app.workspace_mat_save.picker_calls,1);
        assert!(!app.busy);
    }
}
