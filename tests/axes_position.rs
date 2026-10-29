use matrust::{Engine, files, plot::Position};

#[test]
fn position_edit_preserves_all_samples_and_roundtrips_without_overlap_deletion() {
    let mut e = Engine::default();
    e.execute("tiledlayout(2,2);nexttile;plot([1 2]);title('First');nexttile;plot([3 4]);title('Second');nexttile([1 2]);plot([5 6]);title('Third');sgtitle('Overview');").unwrap();
    let mut f = e.figure.unwrap();
    let original = f.clone();
    let original_positions = original.normalized_axes_positions();
    let active = [0.12, 0.10, 0.76, 0.30];
    f.reposition_axes(active).unwrap();
    assert_eq!(f.active_position().normalized_plot_box(), Some(active));
    assert!(f.layout.as_ref().unwrap().tiled.is_none());
    assert_eq!(f.layout.as_ref().unwrap().title, "Overview");
    assert_eq!(f.all_axes().count(), 3);
    for (_, axes) in f.all_axes() {
        let (old_pos, old) = original
            .all_axes()
            .find(|(_, a)| a.title == axes.title)
            .unwrap();
        assert!(axes.same_snapshot(old));
        assert!((axes.series[0].points).same_storage(&old.series[0].points));
        if old_pos != original.active_position() {
            let expected = original_positions
                .iter()
                .find(|(p, _)| *p == old_pos)
                .unwrap()
                .1;
            let actual = f.all_axes().find(|(_, a)| a.title == axes.title).unwrap().0;
            assert_eq!(actual.normalized_plot_box(), Some(expected));
        }
    }
    let snapshot = f.clone();
    for invalid in [
        [0., 0., 1., 1.],
        [-0.1, 0., 0.5, 0.5],
        [0., 0., f64::NAN, 0.5],
    ] {
        assert!(f.reposition_axes(invalid).is_err());
        assert!(f.same_snapshot(&snapshot));
    }
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("edited.mfig");
    files::save_figure(&path, &f).unwrap();
    let restored = files::load_figure(&path).unwrap();
    assert_eq!(restored.svg(), f.svg());
    f.set_shared_title(String::new()).unwrap();
    let path = dir.path().join("edited.fig");
    files::save_figure(&path, &f).unwrap();
    assert_eq!(files::load_figure(&path).unwrap().svg(), f.svg());
}

#[test]
fn geometry_conversion_matches_fig_and_handles_single_empty_and_minimum_boxes() {
    let mut e = Engine::default();
    e.execute("plot([1 2 3]);").unwrap();
    let mut f = e.figure.unwrap();
    let p = f.normalized_axes_positions()[0].1;
    for (actual, expected) in p
        .into_iter()
        .zip([70. / 960., 70. / 560., 850. / 960., 430. / 560.])
    {
        assert!((actual - expected).abs() < 1e-15);
    }
    let samples = f.series[0].points.clone();
    f.reposition_axes([0., 0., 1. / 256., 1. / 256.]).unwrap();
    assert!((samples).same_storage(&f.series[0].points));
    assert_eq!(f.point_count(), 3);

    e = Engine::default();
    e.execute("tiledlayout(2,2);").unwrap();
    let empty = e.figure.as_mut().unwrap();
    assert!(empty.normalized_axes_positions().is_empty());
    assert!(empty.reposition_axes([0.1, 0.1, 0.8, 0.8]).is_err());
    assert!(empty.empty_layout());
    e.execute("nexttile;plot([1 2]);nexttile;plot([3 4]);")
        .unwrap();
    let f = e.figure.as_ref().unwrap();
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("grid.fig");
    files::save_figure(&path, f).unwrap();
    let loaded = files::load_figure(&path).unwrap();
    let mut expected: Vec<_> = f
        .normalized_axes_positions()
        .into_iter()
        .map(|(_, v)| Position::from_normalized(v).unwrap())
        .collect();
    let mut actual: Vec<_> = loaded.all_axes().map(|(p, _)| p).collect();
    expected.sort();
    actual.sort();
    assert_eq!(actual, expected);
}
