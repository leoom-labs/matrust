#[test]
fn figure_recovery_restores_worker_documents_zoom_and_save_close_workflows() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("session.json");
    let source = dir.path().join("saved.mfig");
    let ctx = egui::Context::default();
    let mut app = legacy_sample_desktop(ctx.clone(), true);
    app.recovery = Some(recovery::Recovery::open(path.clone()).unwrap());
    app.recovery_due = Instant::now() + Duration::from_secs(3600);
    app.run("workspace_only=9;figure(2);plot([1 2 3],[1 4 9],'r--');title('Saved line');legend('Signal');figure(5);bar([3 -1 2]);title('Unsaved bars');figure(2);".into());
    finish(&mut app);
    app.submit(Job::SaveFigure(source.clone(), None));
    finish(&mut app);
    let original = std::fs::read(&source).unwrap();
    for _ in 0..3 {
        frame(&mut app, &ctx, vec![]);
    }
    let plot_id = egui::Id::new("workspace_figure");
    let mut memory = egui_plot::PlotMemory::load(&ctx, plot_id).unwrap();
    let bounds = egui_plot::PlotBounds::from_min_max([1.2, 2.], [2.8, 8.]);
    memory.set_bounds(bounds);
    memory.auto_bounds = [false, false].into();
    memory.store(&ctx, plot_id);
    frame(&mut app, &ctx, vec![]);
    assert!(app.figure_dirty(2));
    assert!(!app.figure_documents[&2].dirty);
    assert!(app.recovery_dirty);
    app.recovery
        .as_ref()
        .unwrap()
        .flush(app.snapshot())
        .unwrap();
    drop(app);
    let ctx = egui::Context::default();
    let mut restored = legacy_sample_desktop(ctx.clone(), true);
    restored.recovery = Some(recovery::Recovery::open(path.clone()).unwrap());
    restored.restore_session(Session::load(&path).unwrap());
    restored.start_session_recovery(path.clone());
    assert!(restored.busy && restored.recovering_session);
    finish(&mut restored);
    assert!(!restored.recovery_blocked && !restored.recovering_session);
    assert_eq!(restored.figure_ids, vec![2, 5]);
    assert_eq!(restored.current_figure, 2);
    assert_eq!(restored.variables["workspace_only"].scalar(0, 0).unwrap().into_real().unwrap(), 9.);
    assert_eq!(restored.figure_documents[&2].path, Some(source.clone()));
    assert!(!restored.figure_documents[&2].dirty);
    assert!(restored.figure_dirty(2));
    assert!(restored.figure_documents[&5].path.is_none());
    assert!(restored.figure_dirty(5));
    for _ in 0..3 {
        frame(&mut restored, &ctx, vec![]);
    }
    assert_eq!(
        egui_plot::PlotMemory::load(&ctx, plot_id).unwrap().bounds(),
        &bounds
    );
    restored.run("figure(5);".into());
    finish(&mut restored);
    for _ in 0..3 {
        frame(&mut restored, &ctx, vec![]);
    }
    restored.run("figure(2);".into());
    finish(&mut restored);
    for _ in 0..3 {
        frame(&mut restored, &ctx, vec![]);
    }
    assert_eq!(
        egui_plot::PlotMemory::load(&ctx, plot_id).unwrap().bounds(),
        &bounds
    );
    assert_eq!(std::fs::read(&source).unwrap(), original);
    assert!(restored.save_figure_document(2, false));
    finish(&mut restored);
    assert!(!restored.figure_dirty(2));
    let saved = matrust::files::load_figure(&source).unwrap();
    assert_eq!(saved.xlim, Some([1.2, 2.8]));
    assert_eq!(saved.ylim, Some([2., 8.]));
    restored.request_figure_close(5);
    click_label(&mut restored, &ctx, "Discard changes");
    finish(&mut restored);
    restored
        .recovery
        .as_ref()
        .unwrap()
        .flush(restored.snapshot())
        .unwrap();
    let state = Session::load(&path).unwrap();
    let recovered = matrust::session::figures::restore_figures(
        &path,
        state.figure_manifest.as_ref().unwrap(),
        &std::sync::atomic::AtomicBool::new(false),
    );
    assert!(recovered.warnings.is_empty());
    assert_eq!(recovered.figures.figures.len(), 1);
    assert_eq!(recovered.figures.figures[0].id, 2);
}

#[test]
fn figure_quit_discard_applies_only_to_final_checkpoint_and_cancel_keeps_figures() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("session.json");
    let ctx = egui::Context::default();
    let mut app = legacy_sample_desktop(ctx.clone(), true);
    app.recovery = Some(recovery::Recovery::open(path.clone()).unwrap());
    app.run("figure(1);plot([1 2 3]);figure(2);plot([3 2 1]);".into());
    finish(&mut app);
    app.submit(Job::SaveFigure(dir.path().join("clean.mfig"), None));
    finish(&mut app);
    app.documents[0].text.push_str("\nunsaved_note=1;");
    app.documents[0].changed();
    app.request_quit(&ctx);
    click_label(&mut app, &ctx, "Discard changes");
    assert!(app.pending_quit);
    assert_eq!(app.snapshot().figures.unwrap().figures.len(), 2);
    assert_eq!(app.exit_snapshot().figures.unwrap().figures.len(), 1);
    click_label(&mut app, &ctx, "Cancel");
    assert!(!app.pending_quit);
    assert!(app.quit_discarded_figures.is_empty());
    assert_eq!(app.exit_snapshot().figures.unwrap().figures.len(), 2);
    assert_eq!(app.figure_ids, vec![1, 2]);
    app.request_quit(&ctx);
    assert!(app.quit_discarded_figures.is_empty());
    click_label(&mut app, &ctx, "Discard changes");
    click_label(&mut app, &ctx, "Discard changes");
    assert!(app.allow_close);
    eframe::App::on_exit(&mut app, None);
    let saved = Session::load(&path).unwrap();
    let manifest = saved.figure_manifest.unwrap();
    assert_eq!(manifest.entries.len(), 1);
    assert_eq!(manifest.entries[0].id, 2);
    assert!(!manifest.entries[0].dirty);
}

#[test]
fn incomplete_figure_recovery_preserves_checkpoint_and_disables_automatic_overwrite() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("session.json");
    let ctx = egui::Context::default();
    let mut app = legacy_sample_desktop(ctx, true);
    app.run("figure(1);plot([1 2 3]);figure(2);plot([3 2 1]);".into());
    finish(&mut app);
    app.snapshot().save(&path).unwrap();
    drop(app);
    let stored = Session::load(&path).unwrap();
    let manifest = stored.figure_manifest.as_ref().unwrap();
    let data_dir = matrust::session::figures::figure_directory(&path).unwrap();
    std::fs::remove_file(data_dir.join(&manifest.entries[0].file)).unwrap();
    let original = std::fs::read(&path).unwrap();
    let ctx = egui::Context::default();
    let mut app = legacy_sample_desktop(ctx.clone(), true);
    app.recovery = Some(recovery::Recovery::open(path.clone()).unwrap());
    app.restore_session(stored);
    app.start_session_recovery(path.clone());
    // Exiting before receipt of the worker result must preserve the old manifest.
    eframe::App::on_exit(&mut app, None);
    assert_eq!(std::fs::read(&path).unwrap(), original);
    assert!(app.worker.send.try_send(Job::Script("a=1;".into()).into()).is_err());
    drop(app);
    // Exit now joins/disconnects the worker. Model the actual next session instead
    // of trying to resume work on an application whose final exit hook has run.
    let mut app = legacy_sample_desktop(ctx.clone(), true);
    app.recovery = Some(recovery::Recovery::open(path.clone()).unwrap());
    app.restore_session(Session::load(&path).unwrap());
    app.start_session_recovery(path.clone());
    finish(&mut app);
    assert_eq!(app.figure_ids, vec![2]);
    assert!(app.recovery_blocked);
    assert!(!app.recovering_session);
    app.run("new_work=9;".into());
    finish(&mut app);
    app.recovery_due = Instant::now();
    let output = frame(&mut app, &ctx, vec![]);
    assert!(
        output
            .shapes
            .iter()
            .any(|s| text_position(&s.shape, "Recovery disabled").is_some())
    );
    assert_eq!(std::fs::read(&path).unwrap(), original);
    let manual = dir.path().join("rescued.mfig");
    app.submit(Job::SaveFigure(manual.clone(), None));
    finish(&mut app);
    assert_eq!(
        matrust::files::load_figure(&manual).unwrap().series[0].points.get(0).unwrap(),
        [1., 3.]
    );
    eframe::App::on_exit(&mut app, None);
    assert_eq!(std::fs::read(&path).unwrap(), original);
}
#[test]
fn graphics_handle_recovery_worker_rebinds_nested_aliases_before_live_edits() {
    let dir=tempfile::tempdir().unwrap();let path=dir.path().join("session.json");
    let mut app=legacy_sample_desktop(egui::Context::default(),true);
    app.recovery=Some(recovery::Recovery::open(path.clone()).unwrap());
    app.recovery_due=Instant::now()+Duration::from_secs(3600);
    app.run("f=figure(4);a=gca;h=plot(a,[1 2],[3 4]);alias=h;C={h,{a,f}};lg=legend(a,h,{'line'},'Interpreter','none');".into());finish(&mut app);
    app.recovery.as_ref().unwrap().flush(app.snapshot()).unwrap();
    let manifest=Session::load(&path).unwrap();assert_eq!(manifest.workspace_manifest.as_ref().unwrap().version,4);
    let old=app.variables["h"].clone();drop(app);
    let mut restored=legacy_sample_desktop(egui::Context::default(),true);
    restored.recovery=Some(recovery::Recovery::open(path.clone()).unwrap());
    restored.restore_session(manifest);restored.start_session_recovery(path.clone());
    assert!(restored.busy&&restored.recovering_session);finish(&mut restored);
    assert!(!restored.recovery_blocked&&!restored.recovering_session);
    assert_eq!(restored.figure_ids,vec![4]);
    assert!(restored.variables["h"].same_snapshot(&restored.variables["alias"]));
    let matrust::Array::Graphics(before)=old else {panic!()};
    let matrust::Array::Graphics(after)=&restored.variables["h"] else {panic!()};
    assert!(!before.scalar_handle().unwrap().same_object(after.scalar_handle().unwrap()));
    restored.run("set(C{1},'DisplayName','restored');set(lg,'Visible','off');alive=isgraphics(alias);parent=get(h,'Parent');".into());finish(&mut restored);
    let matrust::Array::Logical(alive)=&restored.variables["alive"] else {panic!()};
    assert_eq!(alive.data(),&[1]);
    assert_eq!(restored.figure.as_ref().unwrap().axes.series[0].label,"restored");
    restored.recovery.as_ref().unwrap().flush(restored.snapshot()).unwrap();
    assert_eq!(Session::load(&path).unwrap().workspace_manifest.unwrap().version,4);
}



fn placement_checkpoint() -> matrust::session::figure_window::FigureWindow {
    use matrust::session::figure_window::{FigureWindow, NormalWindow};
    FigureWindow::new(true, Some(NormalWindow {
        inner_size_dip: [800.,600.], outer_rect_px: [80.,60.,880.,660.],
        work_area_px: [0.,0.,1920.,1040.], monitor_device: None,
    }), false).unwrap()
}

#[test]
fn window_placement_worker_restore_binds_admitted_identity_and_prunes_reused_numbers() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("windows.json");
    let mut app = legacy_sample_desktop(egui::Context::default(), true);
    app.run("A=[1 2 3]; alias=A; figure(1);plot([1 2 3]);figure(2);bar([4 5]);figure(1);".into());
    finish(&mut app);
    let original_identity = app.all_figures[&2].identity.clone();
    let mut snapshot = app.snapshot();
    for doc in &mut snapshot.figures.as_mut().unwrap().figures {
        doc.window = Some(placement_checkpoint());
    }
    matrust::session::SessionStore::new(&path).save(snapshot).unwrap();
    drop(app);
    let mut restored = legacy_sample_desktop(egui::Context::default(), true);
    restored.restore_session(Session::load(&path).unwrap());
    restored.start_session_recovery(path.clone());
    finish(&mut restored);
    assert!(!restored.recovery_blocked && !restored.recovering_session);
    assert_eq!(restored.current_figure, 1);
    assert_eq!(restored.figure_windows.len(), 2);
    assert!(restored.variables["A"].same_snapshot(&restored.variables["alias"]));
    for (&id, placement) in &restored.figure_windows {
        assert!(placement.identity.same_object(&restored.all_figures[&id].identity));
        assert_eq!(placement.window, placement_checkpoint());
    }
    assert!(!original_identity.same_object(&restored.figure_windows[&2].identity));
    assert!(restored.figure_checkpoint().figures.iter().all(|f| f.window.is_some()));
    let stale = restored.figure_windows[&2].clone();
    let first_points = restored.all_figures[&1].series[0].points.clone();
    let first_window = restored.figure_windows[&1].window.clone();
    restored.run("close(2);figure(2);plot([9 8 7]);figure(1);".into());
    finish(&mut restored);
    assert!(!stale.identity.same_object(&restored.all_figures[&2].identity));
    assert!(!restored.figure_windows.contains_key(&2));
    assert_eq!(restored.figure_windows[&1].window, first_window);
    assert!((first_points).same_storage(&restored.all_figures[&1].series[0].points));
    assert!(restored.figure_checkpoint().figures.iter().find(|d| d.id == 2).unwrap().window.is_none());
    // A delayed old recovery context cannot attach to a same-number document.
    restored.recovered_figures(worker::RecoveredFigures {
        windows: std::collections::BTreeMap::from([(2, stale)]),
        placement_notes: vec![], views: Default::default(), warnings: vec![],
    });
    assert!(!restored.figure_windows.contains_key(&2));
    assert!(restored.figure_checkpoint().figures.iter().all(|f| f.window.is_none()));
}

#[test]
fn placement_reset_notice_is_nonblocking_but_missing_payload_stays_hard_warning() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("windows.json");
    let mut app = legacy_sample_desktop(egui::Context::default(), true);
    app.run("A=[1 2]; alias=A;figure(1);plot([1 2]);figure(2);plot([3 4]);".into());
    finish(&mut app);
    let mut snapshot = app.snapshot();
    for doc in &mut snapshot.figures.as_mut().unwrap().figures { doc.window = Some(placement_checkpoint()); }
    matrust::session::SessionStore::new(&path).save(snapshot).unwrap();
    drop(app);
    let pristine: serde_json::Value = serde_json::from_slice(&std::fs::read(&path).unwrap()).unwrap();
    let mut changed = pristine.clone();
    changed["figure_manifest"]["entries"][0]["window"] = serde_json::json!({"version":99});
    std::fs::write(&path, serde_json::to_vec(&changed).unwrap()).unwrap();
    let mut restored = legacy_sample_desktop(egui::Context::default(), true);
    restored.restore_session(Session::load(&path).unwrap());
    restored.start_session_recovery(path.clone());
    finish(&mut restored);
    assert!(!restored.recovery_blocked);
    assert_eq!(restored.figure_ids, vec![1,2]);
    assert_eq!(restored.figure_windows.len(), 1);
    assert!(!restored.figure_windows.contains_key(&1));
    assert!(restored.figure_windows[&2].identity.same_object(&restored.all_figures[&2].identity));
    assert!(restored.console.iter().any(|line| line == "Session layout: Figure 1 window placement was reset"));
    assert!(!restored.console.iter().any(|line| line.contains("Automatic recovery is disabled")));
    assert!(restored.variables["A"].same_snapshot(&restored.variables["alias"]));
    drop(restored);
    // Valid metadata cannot resurrect a figure whose owned payload is missing.
    let missing_name = pristine["figure_manifest"]["entries"][1]["file"].as_str().unwrap();
    std::fs::remove_file(matrust::session::figures::figure_directory(&path).unwrap().join(missing_name)).unwrap();
    std::fs::write(&path, serde_json::to_vec(&pristine).unwrap()).unwrap();
    let mut missing = legacy_sample_desktop(egui::Context::default(), true);
    missing.restore_session(Session::load(&path).unwrap());
    missing.start_session_recovery(path.clone());
    finish(&mut missing);
    assert!(missing.recovery_blocked);
    assert_eq!(missing.figure_ids, vec![1]);
    assert_eq!(missing.figure_windows.len(), 1);
    assert!(!missing.figure_windows.contains_key(&2));
    assert!(missing.variables["A"].same_snapshot(&missing.variables["alias"]));
    assert!(missing.console.iter().any(|line| line.contains("Automatic recovery is disabled")));
    missing.run("figure(2);plot([7 8]);".into());
    finish(&mut missing);
    assert!(!missing.figure_windows.contains_key(&2));
}
