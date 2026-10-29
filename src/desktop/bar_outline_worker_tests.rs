use super::*;
#[test]
fn outline_worker_edit_undo_redo_and_stale_rejection_keep_shared_payload() {
    let mut e = Engine::default();
    let mut documents = figure_documents::Documents::default();
    documents
        .perform(&mut e, Job::Script("bar([1 2],[3 4]);".into()))
        .0
        .unwrap();
    let original = e.figure.clone().unwrap();
    let mut updated = original.clone();
    updated.series[0].face_edge_color = Some([220, 30, 90]);
    updated.series[0].pattern = matrust::plot::LinePattern::Dotted;
    updated.series[0].line_width = Some(2.);
    let edit = FigureEdit {
        view_before: None,
        id: 1,
        original: original.clone(),
        updated: updated.clone(),
    };
    documents
        .perform(&mut e, Job::EditFigure(Box::new(edit)))
        .0
        .unwrap();
    assert_eq!(
        e.figure.as_ref().unwrap().series[0].face_edge_color,
        Some([220, 30, 90])
    );
    assert!((original.series[0].points).same_storage(&e.figure.as_ref().unwrap().series[0].points));
    assert!(
        documents
            .perform(
                &mut e,
                Job::EditFigure(Box::new(FigureEdit {
                    view_before: None,
                    id: 1,
                    original: original.clone(),
                    updated
                }))
            )
            .0
            .is_err()
    );
    documents
        .perform(&mut e, Job::UndoFigure(1, None))
        .0
        .unwrap();
    assert!(
        e.figure.as_ref().unwrap().series[0]
            .face_edge_color
            .is_none()
    );
    documents
        .perform(&mut e, Job::RedoFigure(1, None))
        .0
        .unwrap();
    assert_eq!(
        e.figure.as_ref().unwrap().series[0].face_edge_color,
        Some([220, 30, 90])
    );
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("outline.mfig");
    documents
        .perform(&mut e, Job::SaveFigure(path.clone(), None))
        .0
        .unwrap();
    let loaded = files::load_figure(&path).unwrap();
    assert_eq!(loaded.series[0].face_edge_color, Some([220, 30, 90]));
    assert_eq!(loaded.series[0].pattern, matrust::plot::LinePattern::Dotted);
    assert!((original.series[0].points).same_storage(&e.figure.as_ref().unwrap().series[0].points));
}
