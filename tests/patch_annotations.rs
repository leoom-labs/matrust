use matrust::{
    Matrix, files,
    plot::{AxisScale, Figure, PlotKind, Series, TextAlign, TextBox},
};
use std::sync::atomic::AtomicBool;

fn patch() -> Series {
    let x = Matrix::from_column_slice(4, 2, &[0., 2., 2., 0., 3., 3., 5., 5.]).unwrap();
    let y = Matrix::from_column_slice(4, 2, &[0., 0., 1., 1., 2., 4., 4., 2.]).unwrap();
    Series::patch_from_matrices(&x, &y, [220, 30, 40], &AtomicBool::new(false)).unwrap()
}
fn figure() -> Figure {
    let mut f = Figure::default();
    f.add(vec![patch()]).unwrap();
    let mut textbox =
        TextBox::new([0., 0.9, 1., 0.1], "A <literal> & \u{03bb}\nsecond".into()).unwrap();
    textbox.horizontal = TextAlign::Center;
    textbox.bold = true;
    f.add_textbox(textbox).unwrap();
    f
}
#[test]
fn faces_are_independent_shared_and_never_decimated_or_inferred_as_bars() {
    let s = patch();
    assert_eq!(s.kind, PlotKind::Patch);
    assert_eq!(s.patch_face_count(), 2);
    assert_eq!(s.patch.unwrap().vertices_per_face(), 4);
    assert_eq!(s.display_points(1).len(), 8);
    let copy = s.clone();
    assert!((s.points).same_storage(&copy.points));
    assert_eq!(
        &s.points.range(0..4).unwrap().collect::<Vec<_>>(),
        &[[0., 0.], [2., 0.], [2., 1.], [0., 1.]]
    );
    assert_eq!(
        &s.points
            .range(4..s.points.len())
            .unwrap()
            .collect::<Vec<_>>(),
        &[[3., 2.], [3., 4.], [5., 4.], [5., 2.]]
    );
    let x = Matrix::from_column_slice(3, 1, &[-3., 1., 2.]).unwrap();
    let y = Matrix::from_column_slice(3, 1, &[1., 7., -2.]).unwrap();
    let triangle = Series::patch_from_matrices(&x, &y, [1, 2, 3], &AtomicBool::new(false)).unwrap();
    assert_eq!(triangle.patch.unwrap().vertices_per_face(), 3);
    assert_eq!(
        triangle.points.iter().collect::<Vec<_>>().as_slice(),
        &[[-3., 1.], [1., 7.], [2., -2.]]
    );
}
#[test]
fn unsupported_faces_and_cancellation_reject_before_publication() {
    let mut f = figure();
    let before = f.clone();
    let x = Matrix::from_column_slice(4, 1, &[0., 2., 1., -1.]).unwrap();
    let y = Matrix::from_column_slice(4, 1, &[0., 0., 1., 1.]).unwrap();
    assert!(Series::patch_from_matrices(&x, &y, [0; 3], &AtomicBool::new(false)).is_err());
    assert!(
        Series::patch_from_matrices(&x, &y, [0; 3], &AtomicBool::new(true))
            .unwrap_err()
            .contains("Cancelled")
    );
    let mut bad = patch();
    bad.kind = PlotKind::Line;
    assert!(f.add(vec![bad]).is_err());
    assert!(f.same_snapshot(&before));
    assert!(
        Series::patch_from_matrices(
            &Matrix::filled(4, 10001, 0.).unwrap(),
            &Matrix::filled(4, 10001, 0.).unwrap(),
            [0; 3],
            &AtomicBool::new(false)
        )
        .is_err()
    );
    for rows in [3, 4] {
        let empty = Matrix::filled(rows, 0, 0.).unwrap();
        assert_eq!(
            Series::patch_from_matrices(&empty, &empty, [0; 3], &AtomicBool::new(false))
                .unwrap()
                .patch_face_count(),
            0
        );
    }
}
#[test]
fn missing_faces_preserve_bits_but_do_not_influence_fill_bounds_or_log_sign() {
    let nan = f64::from_bits(0x7ff8000000001234);
    let x = Matrix::from_column_slice(3, 2, &[-0., 1., 2., -100., -200., nan]).unwrap();
    let y = Matrix::from_column_slice(3, 2, &[1., 2., 3., f64::INFINITY, 5., 6.]).unwrap();
    let s = Series::patch_from_matrices(&x, &y, [1; 3], &AtomicBool::new(false)).unwrap();
    assert_eq!(s.points.get(0).unwrap()[0].to_bits(), (-0f64).to_bits());
    assert_eq!(s.points.get(5).unwrap()[0].to_bits(), nan.to_bits());
    let mut f = Figure::default();
    f.add(vec![s]).unwrap();
    assert_eq!(f.bounds(), ([0., 2.], [1., 3.]));
    assert_eq!(f.svg().matches("<polygon ").count(), 1);
    f.xscale = AxisScale::Log;
    assert_eq!(f.bounds().0, [1., 10.]);
    assert!(!f.axis_transforms()[0].negative);
    assert_eq!(f.svg().matches("<polygon ").count(), 0);
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("missing.mfig");
    files::save_figure(&path, &f).unwrap();
    let restored = files::load_figure(&path).unwrap();
    assert!(
        restored.series[0]
            .points
            .iter()
            .zip(f.series[0].points.iter())
            .all(|(a, b)| a.iter().zip(b).all(|(a, b)| a.to_bits() == b.to_bits()))
    );
}
#[test]
fn textbox_metadata_is_bounded_atomic_and_changes_snapshot() {
    let mut f = figure();
    let before = f.clone();
    assert!(TextBox::new([0., 0., f64::NAN, 1.], "x".into()).is_err());
    assert!(TextBox::new([0.8, 0., 0.4, 1.], "x".into()).is_err());
    assert!(TextBox::new([0., 0., 1., 1.], "x".repeat(4097)).is_err());
    f.textboxes[0].text.push('!');
    assert!(!f.same_snapshot(&before));
    let mut capped = Figure::default();
    for _ in 0..4 {
        capped
            .add_textbox(TextBox::new([0., 0., 1., 1.], "x".repeat(4096)).unwrap())
            .unwrap();
    }
    let snapshot = capped.clone();
    assert!(
        capped
            .add_textbox(TextBox::new([0., 0., 1., 1.], "x".into()).unwrap())
            .is_err()
    );
    assert!(capped.same_snapshot(&snapshot));
}
#[test]
fn native_roundtrip_retains_geometry_annotations_and_rejects_downgrade() {
    let dir = tempfile::tempdir().unwrap();
    let file = dir.path().join("objects.mfig");
    let mut f = figure();
    f.appearance = None; // Historical codec fixture, independent of modern theme intent.
    f.axes.palette_cycle = None; // Empty layout axes are not in all_axes_mut().
    for axes in f.all_axes_mut() {
        axes.palette_cycle = None;
        for series in &mut axes.series {
            series.auto_palette = None;
        }
    }
    files::save_figure(&file, &f).unwrap();
    let bytes = std::fs::read(&file).unwrap();
    assert!(bytes.starts_with(b"MATRFIGJ"));
    let g = files::load_figure(&file).unwrap();
    assert_eq!(g.textboxes, f.textboxes);
    assert_eq!(g.series[0].patch, f.series[0].patch);
    assert_eq!(g.series[0].points, f.series[0].points);
    assert_eq!(g.svg(), f.svg());
    let mut downgrade = bytes.clone();
    downgrade[..8].copy_from_slice(b"MATRFIGH");
    std::fs::write(&file, downgrade).unwrap();
    assert!(
        files::load_figure(&file)
            .unwrap_err()
            .contains("requires MFIGJ")
    );
    let mut ordinary = Figure {
        appearance: None,
        axes: matrust::plot::Axes::default(),
        ..Figure::default()
    };
    ordinary
        .add(vec![Series::new(vec![[0., 1.]], PlotKind::Line)])
        .unwrap();
    files::save_figure(&file, &ordinary).unwrap();
    assert!(std::fs::read(&file).unwrap().starts_with(b"MATRFIG4"));
}
#[test]
fn subplot_container_owns_textboxes_once_and_has_explicit_new_header() {
    let dir = tempfile::tempdir().unwrap();
    let file = dir.path().join("panels.mfig");
    let mut f = figure();
    f.appearance = None; // Exercise the pre-theme native capability.
    f.axes.palette_cycle = None; // Empty layout axes are not in all_axes_mut().
    for axes in f.all_axes_mut() {
        axes.palette_cycle = None;
        for series in &mut axes.series {
            series.auto_palette = None;
        }
    }
    f.select_subplot(
        matrust::plot::Position::from_grid(1, 2, &[1]).unwrap(),
        false,
    )
    .unwrap();
    f.select_subplot(
        matrust::plot::Position::from_grid(1, 2, &[2]).unwrap(),
        false,
    )
    .unwrap();
    f.add(vec![patch()]).unwrap();
    files::save_figure(&file, &f).unwrap();
    let bytes = std::fs::read(&file).unwrap();
    assert!(bytes.starts_with(b"MATRFIGK"));
    let g = files::load_figure(&file).unwrap();
    assert_eq!(g.textboxes.len(), 1);
    assert_eq!(g.textboxes, f.textboxes);
    assert_eq!(g.svg().matches("id=\"textbox0\"").count(), 1);
    assert_eq!(g.point_count(), f.point_count());
    let mut downgrade = bytes;
    downgrade[..8].copy_from_slice(b"MATRFIGI");
    std::fs::write(&file, downgrade).unwrap();
    assert!(files::load_figure(&file).is_err());
}
#[test]
fn svg_and_script_use_real_faces_literal_text_and_explicit_format_guards() {
    let f = figure();
    let svg = f.svg();
    assert_eq!(svg.matches("<polygon ").count(), 2);
    assert!(svg.contains("A &lt;literal&gt; &amp; \u{03bb}"));
    assert!(svg.contains("url(#textbox0)"));
    let script = files::matlab_plot_script(&f, &AtomicBool::new(false)).unwrap();
    assert!(script.contains("reshape(mr_x,4,[])"));
    assert!(script.contains("annotation(mr_figure,'textbox'"));
    assert!(!script.contains("mr_left ="));
    assert!(script.contains("char(["));
    let dir = tempfile::tempdir().unwrap();
    for extension in ["fig", "csv"] {
        let path = dir.path().join(format!("existing.{extension}"));
        std::fs::write(&path, b"retain").unwrap();
        let result = if extension == "fig" {
            files::save_matlab_figure(&path, &f)
        } else {
            files::write_figure_csv(&path, &f)
        };
        assert!(result.is_err());
        assert_eq!(std::fs::read(&path).unwrap(), b"retain");
    }
}

#[test]
fn malformed_native_face_descriptors_and_textbox_properties_are_rejected() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("malformed.mfig");
    let mut legacy = figure();
    legacy.appearance = None; // Keep malformed geometry errors independent of theme gating.
    legacy.axes.palette_cycle = None; // Empty layout axes are not in all_axes_mut().
    for axes in legacy.all_axes_mut() {
        axes.palette_cycle = None;
        for series in &mut axes.series {
            series.auto_palette = None;
        }
    }
    files::save_figure(&path, &legacy).unwrap();
    let bytes = std::fs::read(&path).unwrap();
    let size = u32::from_le_bytes(bytes[8..12].try_into().unwrap()) as usize;
    let original: serde_json::Value = serde_json::from_slice(&bytes[12..12 + size]).unwrap();
    for case in 0..5 {
        let mut metadata = original.clone();
        match case {
            0 => metadata["series"][0]["patch"]["vertices_per_face"] = serde_json::json!(0),
            1 => metadata["series"][0]["patch"]["vertices_per_face"] = serde_json::json!(5),
            2 => metadata["series"][0]["kind"] = serde_json::json!("Line"),
            3 => metadata["textboxes"][0]["font_size"] = serde_json::json!(201.),
            _ => metadata["textboxes"][0]["position"] = serde_json::json!([0.9, 0., 0.9, 1.]),
        }
        let json = serde_json::to_vec(&metadata).unwrap();
        let mut bad = b"MATRFIGJ".to_vec();
        bad.extend_from_slice(&(json.len() as u32).to_le_bytes());
        bad.extend_from_slice(&json);
        bad.extend_from_slice(&bytes[12 + size..]);
        std::fs::write(&path, bad).unwrap();
        assert!(files::load_figure(&path).is_err(), "case {case}");
    }
}

#[test]
fn forbidden_xml_characters_are_sanitized_only_in_display() {
    let literal = "left\u{fffe}\u{ffff}\0\u{1f680}right";
    let mut f = Figure::default();
    f.add_textbox(TextBox::new([0., 0., 1., 1.], literal.into()).unwrap())
        .unwrap();
    let svg = f.svg();
    assert!(!svg.contains('\u{fffe}') && !svg.contains('\u{ffff}') && !svg.contains('\0'));
    assert!(svg.contains("left\u{fffd}\u{fffd}\u{fffd}\u{1f680}right"));
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("literal.mfig");
    files::save_figure(&path, &f).unwrap();
    let restored = files::load_figure(&path).unwrap();
    assert_eq!(restored.textboxes[0].text, literal);
    assert_eq!(restored.svg(), svg);
    let script = files::matlab_plot_script(&restored, &AtomicBool::new(false)).unwrap();
    assert!(script.contains("65534 65535 0"));
}
