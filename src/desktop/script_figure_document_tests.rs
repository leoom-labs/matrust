use super::*;

#[test]
fn script_figure_files_invalid_or_cancelled_view_context_is_atomic() {
    let dir = tempfile::tempdir().unwrap();
    let mut engine = Engine::default();
    let mut documents = Documents::default();
    documents
        .perform(
            &mut engine,
            Job::Script("plot([1 2]);figure(2);plot([3 4]);".into()),
        )
        .0
        .unwrap();
    let figures: Vec<_> = engine
        .figures()
        .map(|(id, figure)| (id, figure.clone()))
        .collect();
    let statuses = documents.statuses();
    let job = || {
        Job::ScriptViewContext(
            Box::new(Job::ScriptInFolder(
                "savefig skipped.mfig;".into(),
                dir.path().to_owned(),
            )),
            vec![
                (1, ([0., 3., 0., 5.], [false, false])),
                (2, ([3., 0., 0., 5.], [false, false])),
            ],
        )
    };
    assert!(documents.perform(&mut engine, job()).0.is_err());
    assert_eq!(statuses, documents.statuses());
    for (id, figure) in engine.figures() {
        assert!(figure.same_snapshot(&figures.iter().find(|(n, _)| *n == id).unwrap().1));
    }
    engine
        .cancellation()
        .store(true, std::sync::atomic::Ordering::Relaxed);
    assert!(documents.perform(&mut engine, job()).0.is_err());
    assert!(!dir.path().join("skipped.mfig").exists());
}

#[test]
fn script_figure_files_documents_track_success_before_failure_and_detect_disk_changes() {
    let dir = tempfile::tempdir().unwrap();
    let mut engine = Engine::default();
    engine.set_file_directory(dir.path()).unwrap();
    let mut documents = Documents::default();
    assert!(
        documents
            .perform(
                &mut engine,
                Job::Script("plot([1 2]);savefig saved.mfig;unknown();".into())
            )
            .0
            .is_err()
    );
    let status = &documents.statuses()[&1];
    assert!(!status.dirty && status.source_version.is_some());
    let path = status.path.clone().unwrap();
    documents
        .perform(&mut engine, Job::Script("title('changed');".into()))
        .0
        .unwrap();
    assert!(documents.statuses()[&1].dirty);
    std::fs::write(&path, b"external change").unwrap();
    assert!(
        documents
            .perform(&mut engine, Job::SaveFigure(path.clone(), None))
            .0
            .is_err()
    );
    assert_eq!(std::fs::read(&path).unwrap(), b"external change");
    documents
        .perform(
            &mut engine,
            Job::Script("savefig saved.mfig;close;openfig saved.mfig;title('later');".into()),
        )
        .0
        .unwrap();
    assert!(documents.statuses()[&1].dirty);
    assert_eq!(documents.statuses()[&1].path.as_ref(), Some(&path));
    documents
        .perform(
            &mut engine,
            Job::Script("savefig saved.mfig;close;figure(1);plot([8 9]);".into()),
        )
        .0
        .unwrap();
    assert!(documents.statuses()[&1].path.is_none());
}
