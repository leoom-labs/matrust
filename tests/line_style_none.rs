use matrust::{
    Engine, files,
    plot::{AxisTransform, LinePattern, PlotKind, SampleQuery},
};
use std::sync::atomic::AtomicBool;

#[test]
fn line_style_none_keeps_markers_samples_and_explicit_property_precedence() {
    for command in ["plot", "semilogx", "semilogy", "loglog", "stairs", "stem"] {
        let mut e = Engine::default();
        e.execute(&format!(
            "{command}([1 2],[3 4],'r--o','LineStyle','none','DisplayName','Samples');legend on;"
        ))
        .unwrap();
        let f = e.figure.as_ref().unwrap();
        let s = &f.series[0];
        assert_eq!(s.pattern, LinePattern::None);
        assert!(s.markers && s.visible);
        assert_eq!(
            s.points.iter().collect::<Vec<_>>().as_slice(),
            &[[1., 3.], [2., 4.]]
        );
        let svg = f.svg();
        assert!(!svg.contains("stroke-dasharray="), "{command}");
        assert!(
            svg.contains("<circle"),
            "{command}: markers and legend icon remain"
        );
        assert!(svg.contains("Samples"));
        let script = files::matlab_plot_script(f, &AtomicBool::new(false)).unwrap();
        // A legacy LineSpec may carry the marker without materializing an
        // explicit Marker property. Verify the whole public reconstruction,
        // including the absent metadata that controls legacy marker rendering.
        let retained = f.clone();
        let mut reconstruction = script;
        for pass in 0..2 {
            assert!(reconstruction.contains("'LineStyle','none'"));
            let mut replay = Engine::default();
            replay.execute(&reconstruction).unwrap();
            let actual = replay.figure.as_ref().unwrap();
            assert_eq!(actual.series.len(), 1, "{command}/{pass}");
            let restored = &actual.series[0];
            assert_eq!(restored.kind, s.kind);
            assert_eq!(restored.pattern, LinePattern::None);
            assert_eq!(restored.markers, s.markers);
            assert_eq!(restored.marker_shape, s.marker_shape);
            assert_eq!(
                restored.marker_size.map(f64::to_bits),
                s.marker_size.map(f64::to_bits)
            );
            assert_eq!(restored.marker_filled, s.marker_filled);
            assert_eq!(
                restored.line_width.map(f64::to_bits),
                s.line_width.map(f64::to_bits)
            );
            assert_eq!(restored.color, s.color);
            assert_eq!(restored.explicit_color, s.explicit_color);
            assert_eq!(restored.visible, s.visible);
            assert_eq!(restored.label, s.label);
            assert_eq!(restored.points.len(), s.points.len());
            for (a, b) in restored.points.iter().zip(s.points.iter()) {
                assert_eq!(a.map(f64::to_bits), b.map(f64::to_bits));
            }
            assert_eq!(actual.xscale, f.xscale);
            assert_eq!(actual.yscale, f.yscale);
            assert_eq!(actual.xlim, f.xlim);
            assert_eq!(actual.ylim, f.ylim);
            assert_eq!(actual.bounds(), f.bounds());
            assert_eq!(actual.svg(), svg, "{command}/{pass}");
            reconstruction = files::matlab_plot_script(actual, &AtomicBool::new(false)).unwrap();
        }
        assert!(f.same_snapshot(&retained));
        assert!(s.points.same_storage(&retained.series[0].points));
    }
    let mut e = Engine::default();
    e.execute("plot([1 2],'rx','LineStyle','none','Marker','none');")
        .unwrap();
    let f = e.figure.as_ref().unwrap();
    assert_eq!(f.series[0].kind, PlotKind::Line);
    assert!(!f.series[0].markers);
    assert!(!f.svg().contains("#ff0000"));
    e.execute("plot([1 2],'LineStyle','none','LineStyle','--');")
        .unwrap();
    assert_eq!(
        e.figure.as_ref().unwrap().series[0].pattern,
        LinePattern::Dashed
    );
}

#[test]
fn line_style_none_retains_bounds_shared_snapshots_and_original_sample_inspection() {
    let mut e = Engine::default();
    e.execute("plot([1 2],[3 4],'r-');").unwrap();
    let before = e.figure.unwrap();
    let mut f = before.clone();
    f.series[0].pattern = LinePattern::None;
    assert!(!before.same_snapshot(&f));
    assert!((before.series[0].points).same_storage(&f.series[0].points));
    assert_eq!(before.bounds(), f.bounds());
    let query = SampleQuery {
        transforms: [AxisTransform::default(); 2],
        bounds: [0., 3., 0., 5.],
        pointer: [2., 4.],
        pixels: [600., 400.],
    };
    let hit = f
        .nearest_sample(query, &AtomicBool::new(false))
        .unwrap()
        .unwrap();
    assert_eq!(hit.index, 1);
    assert_eq!(hit.point, [2., 4.]);
    assert!(f.nearest_sample(query, &AtomicBool::new(true)).is_err());
    f.series[0].visible = false;
    assert!(
        f.nearest_sample(query, &AtomicBool::new(false))
            .unwrap()
            .is_none()
    );
}

#[test]
fn line_style_none_native_and_fig_roundtrip_marker_and_unmarked_lines() {
    let dir = tempfile::tempdir().unwrap();
    for marker in ["none", "o", "x", "s"] {
        let mut e = Engine::default();
        e.execute(&format!("plot([1 2],[3 4],'LineStyle','none','Marker','{marker}','LineWidth',2,'MarkerSize',12);")).unwrap();
        let f = e.figure.unwrap();
        for ext in ["mfig", "fig"] {
            let path = dir.path().join(format!("line.{ext}"));
            files::save_figure(&path, &f).unwrap();
            let loaded = files::load_figure(&path).unwrap();
            assert_eq!(loaded.series[0].kind, PlotKind::Line);
            assert_eq!(loaded.series[0].pattern, LinePattern::None);
            assert_eq!(loaded.series[0].markers, marker != "none");
            assert_eq!(loaded.series[0].points, f.series[0].points);
            assert_eq!(loaded.svg(), f.svg());
        }
    }
}

#[test]
fn line_style_none_native_envelopes_reject_downgrades_and_nested_layouts() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("line.mfig");
    let mut e = Engine::default();
    e.execute("plot([1 2],'LineStyle','none');").unwrap();
    e.figure.as_mut().unwrap().appearance = None; // Historical native capability.
    {
        let historical = e.figure.as_mut().unwrap();
        historical.axes.palette_cycle = None; // Empty layout axes are not in all_axes_mut().
        for axes in historical.all_axes_mut() {
            axes.palette_cycle = None;
            for series in &mut axes.series {
                series.auto_palette = None;
            }
        }
    }
    files::save_figure(&path, e.figure.as_ref().unwrap()).unwrap();
    let mut bytes = std::fs::read(&path).unwrap();
    assert!(bytes.starts_with(b"MATRFIGF"));
    bytes[..8].copy_from_slice(b"MATRFIGD");
    std::fs::write(&path, bytes).unwrap();
    assert!(files::load_figure(&path).unwrap_err().contains("MFIGF"));
    e.execute("subplot(1,2,1);plot([1 2],'LineStyle','none');subplot(1,2,2);stem([3 4],'MarkerSize',12);legend('Visible','Location','southwest');").unwrap();
    let mut f = e.figure.unwrap();
    // Keep the historical LineStyle codec regression independent of new
    // runtime legend-object metadata; advanced J/K has separate tests.
    for axes in f.all_axes_mut() {
        axes.legend_state = None;
    }
    files::save_figure(&path, &f).unwrap();
    let mut bytes = std::fs::read(&path).unwrap();
    assert!(bytes.starts_with(b"MATRFIGG"));
    assert_eq!(files::load_figure(&path).unwrap().svg(), f.svg());
    let mut nested = bytes.clone();
    let offset = 16 + u32::from_le_bytes(nested[8..12].try_into().unwrap()) as usize;
    nested[offset..offset + 8].copy_from_slice(b"MATRFIGG");
    std::fs::write(&path, nested).unwrap();
    assert!(files::load_figure(&path).unwrap_err().contains("Nested"));
    bytes[..8].copy_from_slice(b"MATRFIGE");
    std::fs::write(&path, bytes).unwrap();
    assert!(files::load_figure(&path).unwrap_err().contains("MFIGG"));
}

#[test]
fn line_style_none_preserves_old_variant_ids_and_serialized_names() {
    for (pattern, id, name) in [
        (LinePattern::Solid, 0, "Solid"),
        (LinePattern::Dashed, 1, "Dashed"),
        (LinePattern::Dotted, 2, "Dotted"),
        (LinePattern::DashDot, 3, "DashDot"),
        (LinePattern::None, 4, "None"),
    ] {
        assert_eq!(pattern as u8, id);
        assert_eq!(
            serde_json::to_string(&pattern).unwrap(),
            format!("\"{name}\"")
        );
        assert_eq!(
            serde_json::from_str::<LinePattern>(&format!("\"{name}\"")).unwrap(),
            pattern
        );
    }
    let mut e = Engine::default();
    e.execute("plot([1 2],'r--');").unwrap();
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("old.mfig");
    e.figure.as_mut().unwrap().appearance = None; // Historical native capability.
    {
        let historical = e.figure.as_mut().unwrap();
        historical.axes.palette_cycle = None; // Empty layout axes are not in all_axes_mut().
        for axes in historical.all_axes_mut() {
            axes.palette_cycle = None;
            for series in &mut axes.series {
                series.auto_palette = None;
            }
        }
    }
    files::save_figure(&path, e.figure.as_ref().unwrap()).unwrap();
    assert!(std::fs::read(&path).unwrap().starts_with(b"MATRFIG4"));
}

#[test]
fn line_style_none_invalid_value_preserves_figure_and_script_cancellation() {
    let mut e = Engine::default();
    e.execute("plot([1 2],'LineStyle','none','Marker','o');")
        .unwrap();
    let before = e.figure.clone().unwrap();
    for command in [
        "plot([1 2],'LineStyle','invisible')",
        "plot([1 2],'LineStyle',3)",
        "bar([1 2],'LineStyle','invisible')",
    ] {
        assert!(e.execute(command).is_err());
        assert!(before.same_snapshot(e.figure.as_ref().unwrap()));
    }
    assert!(files::matlab_plot_script(&before, &AtomicBool::new(true)).is_err());
    // Independent Bar outline support now admits LineStyle none. Keep the
    // invalid-style rollback above, and verify this newly valid combination.
    e.execute("bar([1 2],'FaceColor',[0 0 1],'EdgeColor',[1 0 0],'LineStyle','none');")
        .unwrap();
    let bar = &e.figure.as_ref().unwrap().series[0];
    assert_eq!(bar.pattern, LinePattern::None);
    assert_eq!(bar.face_edge_color, Some([255, 0, 0]));
    assert_eq!(bar.color, [0, 0, 255]);
    let svg = e.figure.as_ref().unwrap().svg();
    assert!(svg.contains("fill=\"#0000ff\""));
    assert!(!svg.contains("stroke=\"#ff0000\""));
}

#[cfg(feature = "plot-export")]
#[test]
fn line_style_none_raster_keeps_markers_without_connecting_pixels_and_pdf_exports() {
    let mut e = Engine::default();
    e.execute("plot([1 3],[1 3],'r-o','LineStyle','none');axis([0 4 0 4]);grid off;")
        .unwrap();
    let f = e.figure.unwrap();
    let image =
        matrust::export::render_figure_with_cancel(&f, 960, 560, &AtomicBool::new(false)).unwrap();
    let red = |p: &image::Rgba<u8>| p[0] > 230 && p[1] < 160 && p[2] < 160;
    assert!(image.pixels().any(red), "endpoint markers remain");
    assert!(
        !image
            .enumerate_pixels()
            .any(|(x, y, p)| (420..600).contains(&x) && (200..360).contains(&y) && red(p)),
        "center contains no connecting line"
    );
    let mut solid = f.clone();
    solid.series[0].pattern = LinePattern::Solid;
    let image =
        matrust::export::render_figure_with_cancel(&solid, 960, 560, &AtomicBool::new(false))
            .unwrap();
    assert!(
        image
            .enumerate_pixels()
            .any(|(x, y, p)| (420..600).contains(&x) && (200..360).contains(&y) && red(p)),
        "positive control must detect a connecting line"
    );
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("none.pdf");
    matrust::export::export_figure(&f, &path, 960, 560).unwrap();
    assert!(std::fs::read(&path).unwrap().starts_with(b"%PDF"));
    std::fs::write(&path, b"preserve").unwrap();
    assert!(
        matrust::export::export_figure_with_cancel(&f, &path, 960, 560, &AtomicBool::new(true))
            .is_err()
    );
    assert_eq!(std::fs::read(&path).unwrap(), b"preserve");
}
