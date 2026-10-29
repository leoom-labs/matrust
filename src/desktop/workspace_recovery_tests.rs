#[test]
fn workspace_recovery_restores_after_exit_tracks_edits_and_clear_without_copying_arrays() {
    let dir=tempfile::tempdir().unwrap(); let path=dir.path().join("session.json");
    let ctx=egui::Context::default(); let mut app=legacy_sample_desktop(ctx.clone(),true);
    app.recovery=Some(recovery::Recovery::open(path.clone()).unwrap());
    app.run("A=reshape(1:100,10,10);B=A;mask=A>50;".into()); finish(&mut app);
    assert!(app.recovery_dirty);
    let snapshot=app.snapshot(); assert!(snapshot.workspace.as_ref().unwrap()["A"].same_snapshot(&app.variables["A"]));
    app.recovery_dirty=false;
    app.submit(Job::SetCell("A".into(),0,0,99.0.into())); finish(&mut app); assert!(app.recovery_dirty);
    eframe::App::on_exit(&mut app,None); drop(app);
    let mut restored=legacy_sample_desktop(ctx,true);
    restored.recovery=Some(recovery::Recovery::open(path.clone()).unwrap());
    restored.restore_session(Session::load(&path).unwrap()); restored.start_session_recovery(path.clone());
    assert!(restored.busy && restored.recovering_session); finish(&mut restored);
    assert!(!restored.recovery_blocked && !restored.recovering_session);
    assert_eq!(restored.variables["A"].scalar(0, 0).unwrap().into_real().unwrap(),99.); assert_eq!(restored.variables["B"].scalar(0, 0).unwrap().into_real().unwrap(),1.); assert!(restored.variables["mask"].is_logical());
    restored.run("check=A(1)+B(1);".into()); finish(&mut restored); assert_eq!(restored.variables["check"].scalar(0, 0).unwrap().into_real().unwrap(),100.);
    restored.recovery_dirty=false; restored.run("clear".into()); finish(&mut restored); assert!(restored.recovery_dirty && restored.variables.is_empty());
    eframe::App::on_exit(&mut restored,None); drop(restored);
    assert!(Session::load(&path).unwrap().workspace_manifest.unwrap().entries.is_empty());
}
#[test]
fn workspace_recovery_failure_preserves_checkpoint_and_recovers_other_work() {
    let dir=tempfile::tempdir().unwrap(); let path=dir.path().join("session.json");
    let ctx=egui::Context::default(); let mut app=legacy_sample_desktop(ctx.clone(),true);
    app.run("A=1:20;plot(A);".into()); finish(&mut app);
    app.documents[0].text="unsaved = 42;".into(); app.documents[0].changed(); app.snapshot().save(&path).unwrap(); drop(app);
    let saved=Session::load(&path).unwrap();
    let payload=matrust::session::workspace::directory(&path).unwrap().join(&saved.workspace_manifest.as_ref().unwrap().entries["A"].file);
    std::fs::write(payload,[0;8]).unwrap(); let original=std::fs::read(&path).unwrap();
    let mut restored=legacy_sample_desktop(ctx,true); restored.recovery=Some(recovery::Recovery::open(path.clone()).unwrap());
    restored.restore_session(saved); restored.start_session_recovery(path.clone()); finish(&mut restored);
    assert!(restored.recovery_blocked && restored.variables.is_empty()); assert!(restored.figure.is_some());
    assert_eq!(restored.documents[0].text,"unsaved = 42;");
    assert!(restored.console.iter().any(|s| s.contains("Workspace:") && s.contains("mismatch")));
    eframe::App::on_exit(&mut restored,None); drop(restored); assert_eq!(original,std::fs::read(&path).unwrap());
}
