#[test]
fn file_preflight_reads_bounded_utf8_scripts_and_reuses_existing_dirty_tabs() {
    let ctx = egui::Context::default();
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("sample.m");
    std::fs::write(&path, "\u{feff}should_not_run=42;\n").unwrap();
    let mut app = folder_navigation_app(&ctx, dir.path());
    let count = app.documents.len();
    app.open_path("sample.m".into());
    assert!(app.busy && app.worker.opening && app.file_action_pending());
    assert_eq!(app.documents.len(), count);
    app.request_script_close(0);
    assert_eq!(app.documents.len(), count);
    finish(&mut app);
    assert!(!app.worker.opening);
    assert_eq!(app.documents.len(), count + 1);
    assert_eq!(app.documents[app.active].text, "should_not_run=42;\n");
    assert!(!app.variables.contains_key("should_not_run"));
    let id = app.documents[app.active].id;
    app.documents[app.active].text = "unsaved=99;".into();
    app.documents[app.active].changed();
    std::fs::write(&path, "external=100;").unwrap();
    app.open_path(path.clone());
    finish(&mut app);
    assert_eq!(app.documents.len(), count + 1);
    assert_eq!(app.documents[app.active].id, id);
    assert_eq!(app.documents[app.active].text, "unsaved=99;");
    assert!(app.documents[app.active].dirty);
    assert_eq!(
        app.recent
            .iter()
            .filter(|p| *p == &path.canonicalize().unwrap())
            .count(),
        1
    );
}
#[test]
fn file_preflight_rejects_bad_scripts_missing_and_unsupported_files_without_tabs() {
    let ctx = egui::Context::default();
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(dir.path().join("bad.m"), [0xff, 0xfe]).unwrap();
    std::fs::write(dir.path().join("huge.txt"), vec![b'x'; 1_000_001]).unwrap();
    std::fs::write(dir.path().join("model.slx"), b"unsupported").unwrap();
    std::fs::write(dir.path().join("model.mdl"), b"unsupported").unwrap();
    let mut app = folder_navigation_app(&ctx, dir.path());
    let count = app.documents.len();
    for name in ["bad.m", "huge.txt", "missing.m", "model.mdl"] {
        app.open_path(name.into());
        assert!(app.worker.opening);
        finish(&mut app);
        assert!(!app.worker.opening);
        assert_eq!(app.documents.len(), count);
        assert!(app.status.contains("Could not open file"), "{}", app.status);
    }
    // SLX now reaches the bounded model importer after file preflight.
    let model = app.toolkits.model.clone();
    app.open_path("model.slx".into());
    finish(&mut app);
    assert!(!app.worker.opening);
    assert_eq!(app.documents.len(), count);
    assert!(app.toolkits.model.same_storage(&model));
    assert!(app.status.contains("SLX: invalid or oversized archive"), "{}", app.status);
    let text = format!("%{}\n", "x".repeat(900_000));
    std::fs::write(dir.path().join("large.m"), &text).unwrap();
    app.open_path("large.m".into());
    finish(&mut app);
    assert_eq!(app.documents[app.active].text, text);
    assert!(app.variables.is_empty());
}
#[test]
fn file_preflight_stop_discards_ready_result_and_waiting_files() {
    let ctx = egui::Context::default();
    let dir = tempfile::tempdir().unwrap();
    for name in ["first.m", "second.m"] {
        std::fs::write(dir.path().join(name), "x=1;").unwrap();
    }
    let mut app = folder_navigation_app(&ctx, dir.path());
    let count = app.documents.len();
    app.open_path("first.m".into());
    app.open_path(dir.path().join("second.m"));
    assert_eq!(app.open_queue.len(), 1);
    app.stop_work();
    finish(&mut app);
    assert_eq!(app.documents.len(), count);
    assert!(app.open_queue.is_empty() && !app.worker.opening);
    assert!(app.status.contains("cancelled"));
    frame(&mut app, &ctx, vec![]);
    assert_eq!(app.documents.len(), count);
    app.open_path("second.m".into());
    finish(&mut app);
    assert_eq!(app.documents.len(), count + 1);
}
#[test]
fn file_preflight_tab_limit_keeps_existing_buffers_and_allows_reselection() {
    let ctx = egui::Context::default();
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(dir.path().join("old.m"), "old=1;").unwrap();
    std::fs::write(dir.path().join("new.m"), "new=1;").unwrap();
    let mut app = folder_navigation_app(&ctx, dir.path());
    app.open_path("old.m".into());
    finish(&mut app);
    let old = app.documents[app.active].id;
    while app.documents.len() < session::MAX_DOCUMENTS {
        app.new_document("unsaved=1;");
    }
    app.open_path("new.m".into());
    finish(&mut app);
    assert!(app.status.contains("32 script tabs"));
    assert_eq!(app.documents.len(), session::MAX_DOCUMENTS);
    app.open_path("old.m".into());
    finish(&mut app);
    assert_eq!(app.documents[app.active].id, old);
    assert_eq!(app.documents.len(), session::MAX_DOCUMENTS);
}
#[test]
fn file_preflight_preserves_workspace_plot_and_forwards_data_loads() {
    let ctx = egui::Context::default();
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(dir.path().join("values.csv"), "1,2\n3,4\n").unwrap();
    let mut app = folder_navigation_app(&ctx, dir.path());
    app.run("keep=[5 6 7];plot(keep);".into());
    finish(&mut app);
    let data = app.variables["keep"].clone();
    let figure = app.figure.as_ref().unwrap().clone();
    app.open_path("values.csv".into());
    finish(&mut app);
    assert!(app.csv_import.is_some() && !app.variables.contains_key("values"));
    click_label(&mut app,&ctx,"Import");finish(&mut app);
    assert!(app.variables.contains_key("values"));
    assert!(data.same_snapshot(&app.variables["keep"]));
    assert!(figure.same_snapshot(app.figure.as_ref().unwrap()));
    app.open_path(dir.path().into());
    finish(&mut app);
    finish_folder(&mut app);
    assert_eq!(app.folder, dir.path().canonicalize().unwrap());
}

mod root_open_picker_cases {
    use super::*;
    use open_files::PickerKind as Kind;
    const SIZE: egui::Vec2 = egui::vec2(1440.,900.);
    const CTRL: egui::Modifiers = egui::Modifiers { ctrl:true,command:true,alt:false,shift:false,mac_cmd:false };
    fn setup(size: egui::Vec2) -> (egui::Context,Workspace,tempfile::TempDir) {
        let ctx=egui::Context::default(); let dir=tempfile::tempdir().unwrap();
        let mut app=folder_navigation_app(&ctx,dir.path());
        app.run("keep=[1 2];alias=keep;plot(keep);".into()); finish(&mut app); finish_folder(&mut app);
        app.documents[app.active].text="unsaved_open=1;".into();app.documents[app.active].changed();
        app.layout.toolstrip_collapsed=false;
        app.root_open.picker=Some(|_,_|None);
        for _ in 0..3 {frame_at(&mut app,&ctx,size,vec![]);}
        assert!(!app.busy && !app.folder_browser.navigating() && !app.file_action_pending());
        (ctx,app,dir)
    }
    fn label(app:&mut Workspace,ctx:&egui::Context,size:egui::Vec2,text:&str,top:bool)->egui::Pos2 {
        let mut result=None;
        for _ in 0..3 {
            let output=frame_at(app,ctx,size,vec![]);
            let mut points:Vec<_>=output.shapes.iter().filter_map(|s|text_position(&s.shape,text).filter(|p|s.clip_rect.contains(*p))).collect();
            if top {points.sort_by(|a,b|a.y.total_cmp(&b.y));} else {points.reverse();}
            result=points.first().copied();
        }
        result.unwrap_or_else(||panic!("Missing actual root Open trigger {text} at {size:?}"))
    }
    fn click(app:&mut Workspace,ctx:&egui::Context,size:egui::Vec2,text:&str,top:bool) {
        let p=label(app,ctx,size,text,top);for down in [true,false] {pointer_frame(app,ctx,size,p,down);}
    }
    fn trigger(app:&mut Workspace,ctx:&egui::Context,size:egui::Vec2,kind:Kind,menu:bool,compact:bool)->egui::Pos2 {
        if menu {
            let item=if kind==Kind::Supported {"Open\u{2026}"} else {"Load workspace\u{2026}"};
            click(app,ctx,size,"Menu",true);click(app,ctx,size,"File",true); return label(app,ctx,size,item,false);
        }
        app.layout.toolstrip_collapsed=compact;
        let (tab,text)=match kind {
            Kind::Supported=>("Home",if compact {"Open script"} else {"Open"}),
            Kind::Figure=>("Plots","Open figure"),Kind::Image=>("Plots","Open plot image"),_=>unreachable!(),
        };
        click(app,ctx,size,tab,true);
        if !compact && size.x < tokens::OVERFLOW_WIDTH && kind == Kind::Image {
            click(app,ctx,size,"More",false);
        }
        label(app,ctx,size,text,false)
    }
    fn discard(app:&mut Workspace,ctx:&egui::Context,size:egui::Vec2,events:Vec<egui::Event>,mut between:impl FnMut(&mut Workspace,&egui::Context)) {
        let count=app.root_open.calls.len();let mut passes=0;
        crate::headless::run(ctx,egui::RawInput{screen_rect:Some(egui::Rect::from_min_size(egui::Pos2::ZERO,size)),events,..Default::default()},|ui|{
            passes+=1;if passes==1 {ui.ctx().request_discard("root chooser must wait for final admission");}
            app.ui(ui);
            if passes==1 {assert_eq!(app.root_open.calls.len(),count);assert!(app.open_queue.is_empty());between(app,ctx);}
        });assert!(passes>=2);
    }
    fn release(point:egui::Pos2)->Vec<egui::Event> {
        vec![egui::Event::PointerMoved(point),egui::Event::PointerButton{pos:point,button:egui::PointerButton::Primary,pressed:false,modifiers:egui::Modifiers::NONE}]
    }
    fn key(modifiers:egui::Modifiers,repeat:bool)->Vec<egui::Event> {
        vec![egui::Event::ModifiersChanged(modifiers),egui::Event::Key{key:egui::Key::O,physical_key:Some(egui::Key::O),pressed:true,repeat,modifiers}]
    }
    fn release_key(app:&mut Workspace,ctx:&egui::Context) {
        frame_at(app,ctx,SIZE,vec![egui::Event::ModifiersChanged(egui::Modifiers::NONE),
            egui::Event::Key{key:egui::Key::O,physical_key:Some(egui::Key::O),pressed:false,repeat:false,modifiers:egui::Modifiers::NONE}]);
    }
    #[test]
    fn root_open_actual_menus_and_expanded_compact_ribbons_consume_discarded_cancel_once() {
        for size in [egui::vec2(960.,650.),SIZE] {
            for kind in [Kind::Supported,Kind::Figure,Kind::Image] {
                for compact in [false,true] {
                    let (ctx,mut app,_dir)=setup(size);let docs=app.documents.clone();let keep=app.variables["keep"].clone();
                    let point=trigger(&mut app,&ctx,size,kind,false,compact);pointer_frame(&mut app,&ctx,size,point,true);
                    discard(&mut app,&ctx,size,release(point),|_,_|{});
                    assert_eq!(app.root_open.calls,vec![kind]);assert!(!app.busy && app.open_queue.is_empty());
                    frame_at(&mut app,&ctx,size,vec![]);assert_eq!(app.root_open.calls.len(),1);
                    assert!(app.documents.iter().zip(&docs).all(|(a,b)|a.same_snapshot(b)));
                    assert!(keep.same_snapshot(&app.variables["keep"]) && keep.same_snapshot(&app.variables["alias"]));
                }
            }
            let (ctx,mut app,_dir)=setup(size);
            for kind in [Kind::Supported,Kind::WorkspaceDefaultDirectory,Kind::Supported] {
                let point=trigger(&mut app,&ctx,size,kind,true,false);pointer_frame(&mut app,&ctx,size,point,true);
                discard(&mut app,&ctx,size,release(point),|_,_|{});
            }
            assert_eq!(app.root_open.calls,vec![Kind::Supported,Kind::WorkspaceDefaultDirectory,Kind::Supported]);
        }
    }
    #[test]
    fn root_ctrl_o_is_exact_retained_and_not_replayed_by_a_late_discard() {
        let (ctx,mut app,_dir)=setup(SIZE);
        for modifiers in [CTRL|egui::Modifiers::SHIFT,CTRL|egui::Modifiers::ALT,egui::Modifiers::NONE] {
            let mut untouched=false;
            crate::headless::run(&ctx,egui::RawInput{screen_rect:Some(egui::Rect::from_min_size(egui::Pos2::ZERO,SIZE)),events:key(modifiers,false),..Default::default()},|ui|{
                app.ui(ui);untouched=ui.ctx().input(|i|i.key_pressed(egui::Key::O));
            });assert!(untouched);assert!(app.root_open.calls.is_empty());release_key(&mut app,&ctx);
        }
        // egui derives repeat from held keys; the supplied flag alone is not a held event.
        frame_at(&mut app,&ctx,SIZE,key(CTRL|egui::Modifiers::SHIFT,false));
        assert!(app.root_open.calls.is_empty() && ctx.input(|i|i.key_down(egui::Key::O)));
        frame_at(&mut app,&ctx,SIZE,key(CTRL,true));assert!(app.root_open.calls.is_empty());release_key(&mut app,&ctx);

        discard(&mut app,&ctx,SIZE,key(CTRL,false),|_,_|{});assert_eq!(app.root_open.calls,vec![Kind::Supported]);
        release_key(&mut app,&ctx);
        let mut passes=0;
        crate::headless::run(&ctx,egui::RawInput{screen_rect:Some(egui::Rect::from_min_size(egui::Pos2::ZERO,SIZE)),events:key(CTRL,false),..Default::default()},|ui|{
            passes+=1;app.ui(ui);if passes==1 {ui.ctx().request_discard("late layout cannot replay completed chooser Cancel");}
        });assert!(passes>=2);assert_eq!(app.root_open.calls.len(),2);
    }
    fn invalidate(app:&mut Workspace,case:usize) {
        match case {
            0=>app.stop_work(),1=>app.busy=true,2=>app.pending_quit=true,3=>app.allow_close=true,
            4=>app.recovering_session=true,5=>app.recovery_blocked=true,6=>app.folder.push("changed"),
            7=>app.new_document("new_open_owner=1;"),
            8=>{app.begin_workspace_action(false,true);assert!(app.workspace_tools.dialog.is_some());},
            9=>app.queue_open_paths([app.folder.join("already-queued.m")]),_=>unreachable!(),
        }
    }
    #[test]
    fn root_open_late_and_postpicker_ownership_changes_publish_no_selected_path_or_layout() {
        for post in [false,true] {for case in 0..11 {
            if post && case==10 {continue;}
            let (ctx,mut app,_dir)=setup(SIZE);app.layout.center=None;app.layout.mode=layout::PaneMode::Editor;
            let keep=app.variables["keep"].clone();let layout=app.layout;
            app.root_open.picker=Some(|app,kind|{
                assert_eq!(kind,Kind::Figure);let path=app.folder.join("must-not-open.mfig");
                if let Some(case)=app.status.strip_prefix("Open picker case "){let case=case.parse().unwrap();invalidate(app,case);}
                Some(vec![path])
            });
            let point=trigger(&mut app,&ctx,SIZE,Kind::Figure,false,false);pointer_frame(&mut app,&ctx,SIZE,point,true);
            if post {app.status=format!("Open picker case {case}");pointer_frame(&mut app,&ctx,SIZE,point,false);}
            else {discard(&mut app,&ctx,SIZE,release(point),|app,ctx|{
                if case==10 {ctx.memory_mut(|m|m.request_focus(egui::Id::new("late_open_owner")));}else{invalidate(app,case);}
            });}
            assert_eq!(app.root_open.calls.len(),usize::from(post),"case {case}, post {post}");
            assert!(!app.open_queue.iter().any(|p|p.ends_with("must-not-open.mfig")));
            if case!=7 {assert_eq!(app.layout.mode,layout.mode);assert_eq!(app.layout.center,layout.center);}
            assert!(keep.same_snapshot(&app.variables["keep"]));
        }}
    }
    fn search_ready(app:&mut Workspace,ctx:&egui::Context,text:&str) {
        click_label_at(app,ctx,SIZE,"Search functions and actions");for _ in 0..2 {frame_at(app,ctx,SIZE,vec![]);}
        assert!(app.global_search.is_open() && ctx.memory(|m|m.has_focus(global_search::query_id())));
        frame_at(app,ctx,SIZE,vec![egui::Event::Text(text.into())]);frame_at(app,ctx,SIZE,vec![]);
    }
    fn search(app:&mut Workspace,ctx:&egui::Context,text:&str) {
        search_ready(app,ctx,text);
        command_first_key(app,ctx,SIZE,egui::Key::Enter,egui::Modifiers::NONE);
        assert!(!app.global_search.is_open());
    }
    #[test]
    fn root_open_search_uses_real_worker_routes_and_preserves_dirty_buffers_and_aliases() {
        for kind in [Kind::Supported,Kind::WorkspaceCurrentFolder,Kind::Figure,Kind::Image] {
            let (ctx,mut app,dir)=setup(SIZE);let docs=app.documents.clone();let keep=app.variables["keep"].clone();
            std::fs::write(dir.path().join("picked.m"),"never_executed=42;\n").unwrap();
            matrust::files::save_workspace(&dir.path().join("picked.mat"),&BTreeMap::from([("from_mat".into(),matrust::Matrix::scalar(9.))])).unwrap();
            matrust::files::save_figure(&dir.path().join("picked.mfig"),app.figure.as_ref().unwrap()).unwrap();
            image::RgbaImage::from_pixel(3,2,image::Rgba([21,42,63,255])).save(dir.path().join("picked.png")).unwrap();
            app.root_open.picker=Some(|app,kind|Some(vec![app.folder.join(match kind {
                Kind::Supported=>"picked.m",Kind::WorkspaceCurrentFolder=>"picked.mat",Kind::Figure=>"picked.mfig",Kind::Image=>"picked.png",_=>unreachable!(),
            })]));
            let text=match kind {Kind::Supported=>"Open script",Kind::WorkspaceCurrentFolder=>"Load workspace",Kind::Figure=>"Open figure",Kind::Image=>"Open plot image",_=>unreachable!()};
            search(&mut app,&ctx,text);assert_eq!(app.root_open.calls,vec![kind]);
            frame_at(&mut app,&ctx,SIZE,vec![]);finish(&mut app);
            match kind {
                Kind::Supported=>{
                    assert!(!app.variables.contains_key("never_executed"));
                    let id=app.documents[app.active].id;app.documents[app.active].text="dirty_picked=73;".into();app.documents[app.active].changed();
                    finish_folder(&mut app);search(&mut app,&ctx,text);frame_at(&mut app,&ctx,SIZE,vec![]);finish(&mut app);
                    assert_eq!(app.documents[app.active].id,id);assert_eq!(app.documents[app.active].text,"dirty_picked=73;");
                },
                Kind::WorkspaceCurrentFolder=>{assert!(app.workspace_import.is_some());assert!(!app.variables.contains_key("from_mat"));click_label(&mut app,&ctx,"Cancel");},
                Kind::Figure=>{let count=app.figure_ids.len();let id=app.current_figure;finish_folder(&mut app);search(&mut app,&ctx,text);frame_at(&mut app,&ctx,SIZE,vec![]);finish(&mut app);assert_eq!(app.figure_ids.len(),count);assert_eq!(app.current_figure,id);},
                Kind::Image=>{
                    assert!(app.pending_image.is_some(), "{}", app.status);
                    // Worker receive prepares pixels; the real next paint constructs the viewer.
                    frame_at(&mut app,&ctx,SIZE,vec![]);
                    assert_eq!(app.image_view.as_ref().unwrap().original_size,[3,2]);
                },_=>unreachable!(),
            }
            for doc in &docs {assert!(app.documents.iter().any(|actual|actual.same_snapshot(doc)));}
            assert!(keep.same_snapshot(&app.variables["keep"]) && keep.same_snapshot(&app.variables["alias"]));
        }
    }
    #[test]
    fn root_open_keeps_whole_selection_bound_and_non_cancel_legacy_figure_reveal() {
        let (ctx,mut app,_dir)=setup(SIZE);
        app.root_open.picker=Some(|app,_|Some((0..33).map(|i|app.folder.join(format!("{i}.m"))).collect()));
        command_first_key(&mut app,&ctx,SIZE,egui::Key::O,CTRL);
        assert!(app.open_queue.is_empty() && app.status.contains("32"));
        app.layout.center=None;app.layout.mode=layout::PaneMode::Editor;
        app.root_open.picker=Some(|_,_|None);
        search(&mut app,&ctx,"Open figure");assert_eq!(app.layout.mode,layout::PaneMode::Editor);
        app.root_open.picker=Some(|app,_|Some(vec![app.folder.join("selected.mfig")]));
        search_ready(&mut app,&ctx,"Open figure");
        // Observe the accepted press before the next frame advances the queue.
        frame_at(&mut app,&ctx,SIZE,vec![egui::Event::ModifiersChanged(egui::Modifiers::NONE),egui::Event::Key{
            key:egui::Key::Enter,physical_key:Some(egui::Key::Enter),pressed:true,repeat:false,modifiers:egui::Modifiers::NONE}]);
        assert!(!app.global_search.is_open());assert_eq!(app.layout.mode,layout::PaneMode::Split);
        assert_eq!(app.open_queue.len(),1);
    }

    fn pane_open_setup(size: egui::Vec2) -> (egui::Context, Workspace, tempfile::TempDir) {
        let (ctx, mut app, dir) = setup(size);
        app.layout.mode = layout::PaneMode::Figure;
        for _ in 0..3 { frame_at(&mut app, &ctx, size, vec![]); }
        (ctx, app, dir)
    }
    fn pane_open_point(app: &mut Workspace, ctx: &egui::Context, size: egui::Vec2, kind: Kind) -> egui::Pos2 {
        click(app, ctx, size, "Menu", true);
        click(app, ctx, size, "File", true);
        click(app, ctx, size, "Figures", false);
        label(app, ctx, size, match kind {
            Kind::FigurePaneSelect => "Open\u{2026}",
            Kind::FigurePaneCopy => "Open separate disk copy...",
            _ => unreachable!(),
        }, false)
    }
    #[test]
    fn figure_pane_open_discarded_cancel_is_once_and_a_fresh_gesture_is_admitted() {
        for size in [egui::vec2(960., 650.), SIZE] {
            for kind in [Kind::FigurePaneSelect, Kind::FigurePaneCopy] {
                let (ctx, mut app, _dir) = pane_open_setup(size);
                let docs = app.documents.clone();
                let figures = app.all_figures.clone();
                let keep = app.variables["keep"].clone();
                let (send, receive) = std::sync::mpsc::sync_channel(1);
                let original = std::mem::replace(&mut app.worker.send, send);
                let pos = pane_open_point(&mut app, &ctx, size, kind);
                pointer_frame(&mut app, &ctx, size, pos, true);
                discard(&mut app, &ctx, size, release(pos), |_, _| {
                    assert!(receive.try_recv().is_err(), "No discarded-pass worker request");
                });
                assert_eq!(app.root_open.calls, vec![kind]);
                assert!(!app.busy && app.open_queue.is_empty() && receive.try_recv().is_err());
                frame_at(&mut app, &ctx, size, vec![]);
                assert_eq!(app.root_open.calls, vec![kind]);
                let pos = pane_open_point(&mut app, &ctx, size, kind);
                pointer_frame(&mut app, &ctx, size, pos, true);
                // A discard requested after final dispatch cannot replay Cancel.
                let mut passes = 0;
                crate::headless::run(&ctx, egui::RawInput {
                    screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, size)),
                    events: release(pos), ..Default::default()
                }, |ui| {
                    passes += 1;
                    app.ui(ui);
                    if passes == 1 { ui.ctx().request_discard("Completed pane picker is not replayed"); }
                });
                assert!(passes >= 2);
                assert_eq!(app.root_open.calls, vec![kind, kind]);
                assert!(receive.try_recv().is_err());
                assert_eq!(app.documents.len(), docs.len());
                assert!(app.documents.iter().zip(&docs).all(|(a,b)| a.same_snapshot(b)));
                assert!(figures.iter().all(|(id,f)| f.same_snapshot(&app.all_figures[id])));
                assert!(keep.same_snapshot(&app.variables["keep"]) && keep.same_snapshot(&app.variables["alias"]));
                app.worker.send = original;
            }
        }
    }
    #[test]
    fn figure_pane_open_rejects_late_and_postpicker_owners_without_any_worker_request() {
        for kind in [Kind::FigurePaneSelect, Kind::FigurePaneCopy] {
            for post in [false, true] {
                for case in 0..11 {
                    if post && case == 10 { continue; }
                    let (ctx, mut app, _dir) = pane_open_setup(SIZE);
                    let figures = app.all_figures.clone();
                    let keep = app.variables["keep"].clone();
                    let (send, receive) = std::sync::mpsc::sync_channel(1);
                    let original = std::mem::replace(&mut app.worker.send, send);
                    app.root_open.picker = Some(|app, kind| {
                        assert!(matches!(kind, Kind::FigurePaneSelect | Kind::FigurePaneCopy));
                        let path = app.folder.join("must-not-open.mfig");
                        if let Some(case) = app.status.strip_prefix("Pane Open test ") {
                            let case = case.parse().unwrap();
                            invalidate(app, case);
                        }
                        Some(vec![path])
                    });
                    let pos = pane_open_point(&mut app, &ctx, SIZE, kind);
                    pointer_frame(&mut app, &ctx, SIZE, pos, true);
                    if post {
                        app.status = format!("Pane Open test {case}");
                        pointer_frame(&mut app, &ctx, SIZE, pos, false);
                    } else {
                        discard(&mut app, &ctx, SIZE, release(pos), |app, ctx| {
                            assert!(receive.try_recv().is_err());
                            if case == 10 { ctx.memory_mut(|m| m.request_focus(egui::Id::new("late_pane_open_owner"))); }
                            else { invalidate(app, case); }
                        });
                    }
                    assert_eq!(app.root_open.calls.len(), usize::from(post), "{kind:?}, {case}, post={post}");
                    while let Ok(request) = receive.try_recv() {
                        // The deliberately competing queue may advance on the
                        // retry; only its own preflight is permitted, never a
                        // Figure job from the retired chooser action.
                        assert!(case == 9 && matches!(request.job, Job::PrepareOpen(_)),
                            "Stale pane action submitted: {kind:?}, {case}, post={post}");
                    }
                    assert!(!app.open_queue.iter().any(|p| p.ends_with("must-not-open.mfig")));
                    assert!(figures.iter().all(|(id,f)| f.same_snapshot(&app.all_figures[id])));
                    assert!(keep.same_snapshot(&app.variables["keep"]) && keep.same_snapshot(&app.variables["alias"]));
                    assert!(app.documents[0].dirty);
                    app.worker.send = original;
                }
            }
        }
    }
    #[test]
    fn figure_pane_open_worker_reselects_canonical_dirty_document_but_disk_copy_is_new() {
        for size in [egui::vec2(960., 650.), SIZE] {
            let (ctx, mut app, dir) = pane_open_setup(size);
            app.run("title('Disk');".into()); finish(&mut app);
            let path = dir.path().join("pane.mfig");
            app.submit(Job::SaveFigure(path.clone(), None)); finish(&mut app);
            let disk = matrust::files::load_figure(&path).unwrap();
            app.run("title('Local');figure(2);plot([7 8]);title('Other');".into()); finish(&mut app);
            finish_folder(&mut app);
            let local = app.all_figures[&1].clone();
            let other = app.all_figures[&2].clone();
            let docs = app.documents.clone();
            let keep = app.variables["keep"].clone();
            let states = app.figure_documents.clone();
            assert!(app.figure_dirty(1));
            app.root_open.picker = Some(|app, _| Some(vec![app.folder.join(".").join("pane.mfig")]));
            for kind in [Kind::FigurePaneSelect, Kind::FigurePaneCopy] {
                let pos = pane_open_point(&mut app, &ctx, size, kind);
                pointer_frame(&mut app, &ctx, size, pos, true);
                let (send, receive) = std::sync::mpsc::sync_channel(1);
                let original = std::mem::replace(&mut app.worker.send, send);
                discard(&mut app, &ctx, size, release(pos), |_, _| {
                    assert!(receive.try_recv().is_err());
                });
                assert!(app.busy && app.open_queue.is_empty());
                let request = receive.try_recv().expect("One accepted pane Open job");
                assert!(receive.try_recv().is_err());
                let actual_path = match (&request.job, kind) {
                    (Job::OpenFigureOrSelect(path), Kind::FigurePaneSelect) => path,
                    (Job::OpenFigure(path), Kind::FigurePaneCopy) => path,
                    _ => panic!("Wrong distinct pane Open job"),
                };
                assert!(matrust::document::same_path(actual_path, &path));
                app.worker.send = original;
                assert!(app.worker.send.send(request).is_ok());
                finish(&mut app);
                if kind == Kind::FigurePaneSelect {
                    assert_eq!(app.figure_ids, vec![1,2]);
                    assert_eq!(app.current_figure, 1);
                    assert!(app.figure.as_ref().unwrap().same_snapshot(&local));
                } else {
                    assert_eq!(app.figure_ids, vec![1,2,3]);
                    assert_eq!(app.current_figure, 3);
                    let copy = app.figure.as_ref().unwrap();
                    assert_eq!(copy.title, "Disk");
                    assert_eq!(copy.series[0].points, disk.series[0].points);
                    assert_eq!(copy.appearance, disk.appearance);
                    assert_eq!(copy.palette_cycle, disk.palette_cycle);
                    assert_eq!(copy.series[0].auto_palette, disk.series[0].auto_palette);
                    assert!(!copy.identity.same_object(&local.identity));
                }
                assert!(app.figure_dirty(1));
                assert_eq!(app.figure_documents[&1].revision, states[&1].revision);
                assert_eq!(app.figure_documents[&1].path, states[&1].path);
                assert!(app.all_figures[&1].same_snapshot(&local));
                assert!(app.all_figures[&1].series[0].points.same_storage(&local.series[0].points));
                assert!(app.all_figures[&2].same_snapshot(&other));
                assert_eq!(app.documents.len(), docs.len());
                assert!(app.documents.iter().zip(&docs).all(|(a,b)| a.same_snapshot(b)));
                assert!(keep.same_snapshot(&app.variables["keep"]) && keep.same_snapshot(&app.variables["alias"]));
                finish_folder(&mut app);
            }
            assert_eq!(app.root_open.calls, vec![Kind::FigurePaneSelect, Kind::FigurePaneCopy]);
        }
    }
    #[test]
    fn figure_pane_open_rejects_a_non_single_picker_result_without_dispatch() {
        for kind in [Kind::FigurePaneSelect, Kind::FigurePaneCopy] {
            for count in [0,2] {
                let (ctx, mut app, _dir) = pane_open_setup(SIZE);
                let (send, receive) = std::sync::mpsc::sync_channel(1);
                let original = std::mem::replace(&mut app.worker.send, send);
                app.root_open.picker = Some(|app,_| {
                    let count: usize = app.status.strip_prefix("Selection count ").unwrap().parse().unwrap();
                    Some((0..count).map(|i| app.folder.join(format!("{i}.mfig"))).collect())
                });
                let pos = pane_open_point(&mut app, &ctx, SIZE, kind);
                pointer_frame(&mut app, &ctx, SIZE, pos, true);
                app.status = format!("Selection count {count}");
                pointer_frame(&mut app, &ctx, SIZE, pos, false);
                assert_eq!(app.root_open.calls, vec![kind]);
                assert!(app.status.contains("exactly one"));
                assert!(!app.busy && app.open_queue.is_empty() && receive.try_recv().is_err());
                app.worker.send = original;
            }
        }
    }
}
