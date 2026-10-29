use matrust::{
    Engine, files,
    plot::{AxisScale, Figure, PlotKind, Series},
};
use std::{path::Path, sync::Arc};

#[test]
fn visibility_preserves_samples_and_controls_bounds_geometry_and_exports() {
    let mut e = Engine::default();
    e.execute("plot([1 2 3],[1 2 3],'b-', [1 2 3],[100 200 300],'r-');legend('Shown','Hidden');")
        .unwrap();
    let mut f = e.figure.unwrap();
    f.appearance = None; // Exercise the pre-theme native capability.
    f.axes.palette_cycle = None; // Empty layout axes are not in all_axes_mut().
    for axes in f.all_axes_mut() {
        axes.palette_cycle = None;
        for series in &mut axes.series {
            series.auto_palette = None;
        }
    }
    let data = f.series[1].points.clone();
    let original = f.clone();
    f.series[1].visible = false;
    assert!(!f.same_snapshot(&original));
    assert_eq!(f.point_count(), 6);
    assert!((data).same_storage(&f.series[1].points));
    assert!(f.series[1].display_points(100).is_empty());
    assert_eq!(f.bounds(), ([1., 3.], [1., 3.]));
    let svg = f.svg();
    assert!(svg.contains("Hidden"));
    assert!(svg.contains("#919191"));
    assert!(!svg.contains("#ff0000"));
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("hidden.mfig");
    files::save_figure(&path, &f).unwrap();
    let loaded = files::load_figure(&path).unwrap();
    let bytes = std::fs::read(&path).unwrap();
    assert!(f.legend_state.is_some() && loaded.legend_state.is_some());
    assert!(loaded.axes.same_metadata(&f.axes));
    assert!(bytes.starts_with(b"MATRFIGJ"));
    let mut downgraded = bytes;
    downgraded[..8].copy_from_slice(b"MATRFIG4");
    std::fs::write(&path, downgraded).unwrap();
    assert_eq!(
        files::load_figure(&path).unwrap_err(),
        "Advanced figure metadata requires MFIGJ"
    );
    assert!(!loaded.series[1].visible);
    assert_eq!(loaded.series[1].points, data);
    let csv = dir.path().join("all.csv");
    files::write_figure_csv(&csv, &loaded).unwrap();
    let text = std::fs::read_to_string(csv).unwrap();
    assert_eq!(text.lines().count(), 7);
    assert!(
        text.lines()
            .last()
            .unwrap()
            .contains("3.00000000000000000e2")
    );
    f.series[0].visible = false;
    assert_eq!(f.bounds(), ([0., 1.], [0., 1.]));
    f.xscale = AxisScale::Log;
    f.yscale = AxisScale::Log;
    assert_eq!(f.bounds(), ([1., 10.], [1., 10.]));
    f.xlim = Some([2., 20.]);
    f.ylim = Some([3., 30.]);
    assert_eq!(f.bounds(), ([2., 20.], [3., 30.]));
}

#[test]
fn old_native_figures_default_to_visible_and_hidden_matlab_objects_roundtrip() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("old.mfig");
    let mut f = Figure::default();
    f.add(vec![Series::new(vec![[1., 2.], [3., 4.]], PlotKind::Line)])
        .unwrap();
    f.appearance = None; // This fixture specifically exercises pre-theme native versions.
    f.axes.palette_cycle = None; // Empty layout axes are not in all_axes_mut().
    for axes in f.all_axes_mut() {
        axes.palette_cycle = None;
        for series in &mut axes.series {
            series.auto_palette = None;
        }
    }
    files::save_figure(&path, &f).unwrap();
    let bytes = std::fs::read(&path).unwrap();
    let len = u32::from_le_bytes(bytes[8..12].try_into().unwrap()) as usize;
    let mut meta: serde_json::Value = serde_json::from_slice(&bytes[12..12 + len]).unwrap();
    meta["series"][0].as_object_mut().unwrap().remove("visible");
    let json = serde_json::to_vec(&meta).unwrap();
    for version in [2, 3] {
        let mut old = format!("MATRFIG{version}").into_bytes();
        old.extend((json.len() as u32).to_le_bytes());
        old.extend(&json);
        old.extend(&bytes[12 + len..]);
        std::fs::write(&path, old).unwrap();
        assert!(files::load_figure(&path).unwrap().series[0].visible);
    }
    let imported = files::load_figure(Path::new("tests/fixtures/fig/matlab-hidden.fig")).unwrap();
    assert_eq!(imported.series.len(), 2);
    assert!(imported.series[0].visible);
    assert!(!imported.series[1].visible);
    assert_eq!(
        imported.series[1]
            .points
            .iter()
            .collect::<Vec<_>>()
            .as_slice(),
        &[[1., 100.], [2., 200.], [3., 300.]]
    );
    let imported_snapshot = imported.clone();
    let imported_native_path = dir.path().join("imported-snapshot.mfig");
    files::save_figure(&imported_native_path, &imported).unwrap();
    let imported_native = std::fs::read(&imported_native_path).unwrap();
    let imported_legend_colors = imported.colors.and_then(|c| c.legend).unwrap();
    let mut export = imported.clone();
    export.legend = false;
    let path = dir.path().join("hidden.fig");
    // Import preserves the saved legend object and its exact membership. Hiding
    // that object does not remove metadata the restricted FIG writer cannot save.
    let legend = imported.legend_state.as_ref().unwrap();
    assert!(!legend.auto_update);
    assert_eq!(
        imported
            .legend_members()
            .map(|(i, _)| i)
            .collect::<Vec<_>>(),
        vec![0, 1]
    );
    let hidden_legend = export.clone();
    std::fs::write(&path, b"existing figure destination").unwrap();
    assert!(
        files::save_figure(&path, &export)
            .unwrap_err()
            .contains("legends")
    );
    assert_eq!(
        std::fs::read(&path).unwrap(),
        b"existing figure destination"
    );
    assert!(export.same_snapshot(&hidden_legend));
    // This roundtrip tests hidden series after explicitly removing the legend
    // from this export clone; the imported figure keeps its saved legend state.
    export.legend_state = None;
    // Explicit legend RGB is now preserved on import. Removing only the object
    // leaves latent style metadata, which FIG correctly refuses to discard.
    let orphan_style = export.clone();
    assert!(
        files::save_figure(&path, &export)
            .unwrap_err()
            .contains("legends")
    );
    assert_eq!(
        std::fs::read(&path).unwrap(),
        b"existing figure destination"
    );
    assert!(export.same_snapshot(&orphan_style));
    // Author this no-legend export clone explicitly; retain every other color
    // and the authentic imported document, including its saved legend style.
    export.colors.as_mut().unwrap().legend = None;
    files::save_figure(&path, &export).unwrap();
    let reopened = files::load_figure(&path).unwrap();
    assert!(!reopened.series[1].visible);
    assert_eq!(reopened.series[1].points, export.series[1].points);
    assert!(imported.legend_state.is_some());
    assert_eq!(
        imported.colors.unwrap().legend,
        Some(imported_legend_colors)
    );
    assert!(imported.same_snapshot(&imported_snapshot));
    files::save_figure(&imported_native_path, &imported).unwrap();
    assert_eq!(
        std::fs::read(&imported_native_path).unwrap(),
        imported_native
    );
    assert!(
        imported.series[1]
            .points
            .same_storage(&export.series[1].points)
    );
}

#[test]
fn hidden_positive_series_do_not_choose_log_domain_or_evade_limits() {
    let mut f = Figure::default();
    f.add(vec![
        Series::new(vec![[-10., -100.], [-1., -10.]], PlotKind::Line),
        Series::new(vec![[1., 10.], [10., 100.]], PlotKind::Line),
    ])
    .unwrap();
    f.xscale = AxisScale::Log;
    f.yscale = AxisScale::Log;
    f.series[1].visible = false;
    assert!(f.axis_transforms().iter().all(|a| a.negative));
    assert_eq!(f.bounds(), ([-10., -1.], [-100., -10.]));
    f.series[1].kind = PlotKind::Bar;
    f.series[1].points = Arc::new(vec![[1., 2.]; 10001]).into();
    assert!(f.validate().is_err());
}
