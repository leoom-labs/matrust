use super::*;
use std::sync::atomic::Ordering;
fn run(d: &mut Documents, e: &mut Engine, source: &str) {
    d.perform(e, Job::Script(source.into())).0.unwrap();
}
#[test]
fn figure_reopen_worker_selects_without_decoding_and_preserves_history_and_samples() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("saved.mfig");
    let (mut d, mut e) = (Documents::default(), Engine::default());
    run(&mut d, &mut e, "keep=[1 2];plot([1 2],[3 4]);");
    d.perform(&mut e, Job::SaveFigure(path.clone(), None))
        .0
        .unwrap();
    let original = e.figure.clone().unwrap();
    let mut edited = original.clone();
    edited.title = "Local".into();
    d.perform(
        &mut e,
        Job::EditFigure(Box::new(super::super::FigureEdit {
            id: 1,
            original,
            updated: edited,
            view_before: None,
        })),
    )
    .0
    .unwrap();
    let local = e.figure.clone().unwrap();
    let state = d.statuses();
    let history = d.history_counts();
    run(&mut d, &mut e, "figure(2);plot([7 8]);");
    let other = e.figure.clone().unwrap();
    std::fs::write(
        &path,
        b"invalid replacement; must not be decoded on reselection",
    )
    .unwrap();
    let result = d
        .perform(
            &mut e,
            Job::OpenFigureOrSelect(path.canonicalize().unwrap()),
        )
        .0
        .unwrap();
    assert!(result.output[0].contains("Selected open figure 1"));
    assert_eq!(e.current_figure, 1);
    assert_eq!(e.figure_ids(), vec![1, 2]);
    assert!(e.figure.as_ref().unwrap().same_snapshot(&local));
    assert!((e.figure.as_ref().unwrap().series[0].points).same_storage(&local.series[0].points));
    assert_eq!(d.statuses()[&1], state[&1]);
    assert_eq!(d.history_counts()[&1], history[&1]);
    assert!(
        e.figures()
            .find(|(id, _)| *id == 2)
            .unwrap()
            .1
            .same_snapshot(&other)
    );
}
#[test]
fn figure_reopen_worker_capacity_and_missing_reload_fail_before_reading() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("saved.mfig");
    let (mut d, mut e) = (Documents::default(), Engine::default());
    run(&mut d, &mut e, "plot([1 2]);");
    d.perform(&mut e, Job::SaveFigure(path.clone(), None))
        .0
        .unwrap();
    for id in 2..=16 {
        run(&mut d, &mut e, &format!("figure({id});"));
    }
    d.perform(&mut e, Job::OpenFigureOrSelect(path)).0.unwrap();
    assert_eq!(e.current_figure, 1);
    assert_eq!(e.figure_ids().len(), 16);
    let missing = dir.path().join("missing.mfig");
    let error = d
        .perform(&mut e, Job::OpenFigure(missing.clone()))
        .0
        .unwrap_err();
    assert!(error.contains("At most 16"), "{error}");
    let error = d
        .perform(&mut e, Job::ReloadFigure(99, missing))
        .0
        .unwrap_err();
    assert_eq!(error, "Figure no longer exists");
    assert_eq!(e.current_figure, 1);
}
#[test]
fn figure_reopen_worker_explicit_disk_copy_and_active_duplicate_remain_available() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("saved.mfig");
    let (mut d, mut e) = (Documents::default(), Engine::default());
    run(&mut d, &mut e, "plot([1 2]);title('Disk');");
    d.perform(&mut e, Job::SaveFigure(path.clone(), None))
        .0
        .unwrap();
    run(&mut d, &mut e, "title('Local original');");
    let original = e.figure.clone().unwrap();
    d.perform(&mut e, Job::OpenFigure(path.clone())).0.unwrap();
    assert_eq!(e.current_figure, 2);
    assert_eq!(e.figure.as_ref().unwrap().title, "Disk");
    run(&mut d, &mut e, "title('Local copy');");
    d.perform(&mut e, Job::OpenFigureOrSelect(path)).0.unwrap();
    assert_eq!(e.current_figure, 2);
    assert_eq!(e.figure.as_ref().unwrap().title, "Local copy");
    assert_eq!(e.figure_ids(), vec![1, 2]);
    assert!(
        e.figures()
            .find(|(id, _)| *id == 1)
            .unwrap()
            .1
            .same_snapshot(&original)
    );
}
#[test]
fn figure_reopen_worker_cancel_and_windows_path_aliases_preserve_state() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("saved.mfig");
    let (mut d, mut e) = (Documents::default(), Engine::default());
    run(&mut d, &mut e, "plot([1 2]);");
    d.perform(&mut e, Job::SaveFigure(path.clone(), None))
        .0
        .unwrap();
    run(&mut d, &mut e, "figure(2);plot([3 4]);");
    let current = e.figure.clone().unwrap();
    e.cancellation().store(true, Ordering::Relaxed);
    assert!(
        d.perform(&mut e, Job::OpenFigureOrSelect(path.clone()))
            .0
            .unwrap_err()
            .contains("cancelled")
    );
    assert_eq!(e.current_figure, 2);
    assert!(e.figure.as_ref().unwrap().same_snapshot(&current));
    e.cancellation().store(false, Ordering::Relaxed);
    #[cfg(windows)]
    let alias = std::path::PathBuf::from(path.to_string_lossy().to_uppercase());
    #[cfg(not(windows))]
    let alias = dir.path().join(".").join("saved.mfig");
    d.perform(&mut e, Job::OpenFigureOrSelect(alias)).0.unwrap();
    assert_eq!(e.current_figure, 1);
    assert_eq!(e.figure_ids(), vec![1, 2]);
}
