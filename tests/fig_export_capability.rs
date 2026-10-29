use matrust::{Engine, files};

#[test]
fn fig_guidance_matches_real_writer_and_keeps_unsupported_destinations() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("keep.fig");
    for (script, reason) in [
        ("plot([1 2],[3 4]);legend('signal');", "legends"),
        (
            "plot([1 2],[3 4]);annotation('textbox',[.1 .8 .8 .1],'String','note','Interpreter','none','EdgeColor','none');",
            "textbox",
        ),
        ("stem([1 2],[3 4]);", "stairs/stem"),
        ("stairs([1 2],[3 4]);", "stairs/stem"),
        ("bar([1 2],[3 4]);", "bar export"),
        (
            "subplot(1,2,1);plot([1 2],[3 4]);subplot(1,2,2);plot([1 2],[5 6]);legend('second');",
            "legends",
        ),
        (
            "tiledlayout(1,1);nexttile;plot([1 2],[3 4]);sgtitle('shared');",
            "shared titles",
        ),
    ] {
        let mut engine = Engine::default();
        engine.execute(script).unwrap();
        let figure = engine.figure.as_ref().unwrap();
        let original = figure.clone();
        let restriction = files::matlab_figure_export_restriction(figure).unwrap();
        assert!(restriction.contains(reason), "{script}: {restriction}");
        std::fs::write(&path, b"existing destination").unwrap();
        let error = files::save_matlab_figure(&path, figure).unwrap_err();
        assert_eq!(error, restriction);
        assert_eq!(std::fs::read(&path).unwrap(), b"existing destination");
        assert!(figure.same_snapshot(&original));
        let editable = directory.path().join("preserve.mfig");
        files::save_figure(&editable, figure).unwrap();
        let reopened = files::load_figure(&editable).unwrap();
        assert_eq!(reopened.series_count(), figure.series_count());
        assert_eq!(reopened.point_count(), figure.point_count());
        assert_eq!(reopened.textboxes, figure.textboxes);
    }
    for script in ["plot([1 2],[3 4]);", "scatter([1 2],[3 4]);"] {
        let mut engine = Engine::default();
        engine.execute(script).unwrap();
        let figure = engine.figure.as_ref().unwrap();
        assert!(files::matlab_figure_export_restriction(figure).is_none());
        files::save_matlab_figure(&path, figure).unwrap();
        assert_eq!(files::load_matlab_figure(&path).unwrap().point_count(), 2);
    }
}

#[test]
fn metadata_guidance_does_not_replace_full_validation() {
    let mut engine = Engine::default();
    engine.execute("plot([1 2],[3 4]);").unwrap();
    let mut figure = engine.figure.unwrap();
    figure.name = "x".repeat(4097);
    assert!(files::matlab_figure_export_restriction(&figure).is_none());
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("keep.fig");
    std::fs::write(&path, b"existing destination").unwrap();
    assert!(files::save_matlab_figure(&path, &figure).is_err());
    assert_eq!(std::fs::read(&path).unwrap(), b"existing destination");
}
