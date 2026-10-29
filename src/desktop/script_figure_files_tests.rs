#[test]
fn script_figure_files_worker_saves_manual_view_and_preserves_history_on_selection() {
    let dir = tempfile::tempdir().unwrap();
    let mut app = folder_navigation_app(&egui::Context::default(),dir.path());
    app.run("plot(1:100);".into());finish(&mut app);
    let original = app.figure.clone().unwrap();
    let mut updated = original.clone();updated.title="edited".into();
    app.submit(Job::EditFigure(Box::new(worker::FigureEdit { id:1,original,updated,view_before:None })));finish(&mut app);
    assert_eq!(app.figure_history[&1],(1,0));
    let revision = app.figure_documents[&1].revision;
    app.figure_views.insert(1,figure_files::View { bounds:[20.,40.,10.,80.],auto:[false,false],dirty:true,revision });
    app.run("figure(2);figure(1);".into());finish(&mut app);
    assert_eq!(app.figure_history[&1],(1,0));
    app.run("savefig zoom.mfig;".into());finish(&mut app);
    assert!(!app.figure_dirty(1));
    assert_eq!(app.figure_history[&1],(1,0));
    let saved = matrust::files::load_figure(&dir.path().join("zoom.mfig")).unwrap();
    assert_eq!(saved.xlim,Some([20.,40.]));assert_eq!(saved.ylim,Some([10.,80.]));
    let revision = app.figure_documents[&1].revision;
    app.figure_views.insert(1,figure_files::View { bounds:[20.,40.,0.,200.],auto:[false,true],dirty:true,revision });
    app.run("savefig auto.mfig;".into());finish(&mut app);
    let automatic = matrust::files::load_figure(&dir.path().join("auto.mfig")).unwrap();
    assert_eq!(automatic.xlim,Some([20.,40.]));assert_eq!(automatic.ylim,None);
    app.run("close;openfig zoom.mfig;".into());finish(&mut app);
    assert!(!app.figure_dirty(1));
    assert_eq!(app.figure.as_ref().unwrap().xlim,Some([20.,40.]));
    assert!(app.figure_documents[&1].path.is_some());
}

#[test]
fn script_exports_worker_preserves_editable_identity_history_and_manual_view() {
    let dir = tempfile::tempdir().unwrap();
    let mut app = folder_navigation_app(&egui::Context::default(),dir.path());
    app.run("plot(1:100);savefig original.mfig;".into());finish(&mut app);
    let original = app.figure.clone().unwrap();
    let mut updated = original.clone();updated.title="Exported edit".into();
    app.submit(Job::EditFigure(Box::new(worker::FigureEdit { id:1,original,updated,view_before:None })));finish(&mut app);
    let before = app.figure_documents[&1].clone();
    let history = app.figure_history[&1];
    assert!(app.figure_dirty(1));
    app.figure_views.insert(1,figure_files::View { bounds:[20.,40.,10.,80.],auto:[false,false],dirty:true,revision:before.revision });
    app.run("saveas(gcf,'zoom.svg');".into());finish(&mut app);
    let mut expected = app.figure.clone().unwrap();
    expected.xlim=Some([20.,40.]);expected.ylim=Some([10.,80.]);
    assert_eq!(std::fs::read_to_string(dir.path().join("zoom.svg")).unwrap(),expected.svg());
    assert_eq!(app.figure_documents[&1].path,before.path);
    assert_eq!(app.figure_documents[&1].source_version,before.source_version);
    assert_eq!(app.figure_history[&1],history);
    assert!(app.figure_dirty(1));
    let saved = matrust::files::load_figure(&dir.path().join("original.mfig")).unwrap();
    assert_eq!(saved.title, "");
    assert!(saved.xlim.is_none());
}
