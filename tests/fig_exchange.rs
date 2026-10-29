use matrust::{
    Engine, files,
    plot::{AxisScale, LinePattern, PlotKind},
};
use std::path::Path;

#[test]
fn matlab_generated_figures_import_data_labels_scales_and_styles() {
    for name in ["matlab-lines.fig", "matlab-lines-v6.fig"] {
        let f = files::load_figure(&Path::new("tests/fixtures/fig").join(name)).unwrap();
        assert_eq!(f.series.len(), 2);
        assert_eq!(f.point_count(), 6);
        assert_eq!(
            (&*f.title, &*f.xlabel, &*f.ylabel),
            ("Shared figure", "Time", "Amplitude")
        );
        assert!(f.legend && f.xgrid && f.ygrid && f.hold);
        assert_eq!(f.xlim, Some([1., 100.]));
        assert_eq!(
            f.series[0].points.iter().collect::<Vec<_>>().as_slice(),
            &[[1., 1.], [10., 2.], [100., 4.]]
        );
        assert_eq!(f.series[0].pattern, LinePattern::Dashed);
        assert_eq!(f.series[0].color, [255, 0, 0]);
        assert!(f.series[0].markers);
        assert_eq!(
            f.series[1].points.iter().collect::<Vec<_>>().as_slice(),
            &[[1., 3.], [10., 4.], [100., 6.]]
        );
    }
    let f = files::load_figure(Path::new("tests/fixtures/fig/matlab-log-scatter.fig")).unwrap();
    assert_eq!(f.xscale, AxisScale::Log);
    assert_eq!(f.series[1].kind, PlotKind::Scatter);
    assert!(f.series[1].marker_filled);
    assert_eq!(f.series[1].color, [255, 0, 0]);
    let f = files::load_figure(Path::new("tests/fixtures/fig/matlab-bars.fig")).unwrap();
    assert_eq!(f.series[0].kind, PlotKind::Bar);
    assert!((f.series[0].bar_width - 0.8 * 2. / 7.).abs() < 1e-14);
    assert_eq!(f.series[0].bar_offset, -1. / 7.);
    assert_eq!(f.series[1].bar_offset, 1. / 7.);
}

#[test]
fn matlab_bar_geometry_uses_axis_units_and_bar_width() {
    // Independent reference fixtures; Face.VertexData was checked by the reference.
    for (name, width, offset) in [
        ("matlab-bar-spacing-1.fig", 0.8 * 6. / 7., 3. / 7.),
        ("matlab-bar-spacing-2.fig", 0.55 * 6. / 7., 3. / 7.),
        ("matlab-bar-spacing-3.fig", 0.6, 0.),
    ] {
        let f = files::load_figure(&Path::new("tests/fixtures/fig").join(name)).unwrap();
        assert!((f.series[0].bar_width - width).abs() < 1e-14);
        assert!((f.series[0].bar_offset + offset).abs() < 1e-14);
        if f.series.len() == 2 {
            assert!((f.series[1].bar_width - width).abs() < 1e-14);
            assert!((f.series[1].bar_offset - offset).abs() < 1e-14);
            assert_eq!(
                f.series[0].points.iter().collect::<Vec<_>>().as_slice(),
                &[[2., 2.], [5., 3.], [9., 4.]]
            );
        } else {
            assert_eq!(
                f.series[0].points.iter().collect::<Vec<_>>().as_slice(),
                &[[7., 4.]]
            );
        }
    }
}

#[test]
fn fig_writer_preserves_sample_bits_and_utf16_across_many_series() {
    use matrust::plot::{Figure, Series};
    let mut f = Figure {
        identity: matrust::ObjectIdentity::default(),
        axes: matrust::plot::Axes {
            title: "\u{03bb}\u{1f680}".into(),
            ..Default::default()
        },
        layout: None,
        textboxes: Vec::new(),
        name: String::new(),
        appearance: None,
        invert_hardcopy: None,
        color: [255; 3],
    };
    let samples = vec![
        [1., -0.],
        [2., f64::from_bits(0x7ff8_0000_0000_1234)],
        [3., f64::INFINITY],
        [4., 5.],
    ];
    for i in 0..64 {
        let mut s = Series::new(samples.clone(), PlotKind::Line);
        s.label = format!("series {i} \u{03bb}");
        s.visible = i % 2 == 0;
        f.add(vec![s]).unwrap();
    }
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("many.fig");
    files::save_matlab_figure(&path, &f).unwrap();
    let reopened = files::load_matlab_figure(&path).unwrap();
    assert_eq!(reopened.title, f.title);
    assert_eq!(reopened.series.len(), 64);
    for (a, b) in reopened.series.iter().zip(&f.series) {
        assert_eq!((a.visible, &a.label), (b.visible, &b.label));
        for (a, b) in a.points.iter().zip(b.points.iter()) {
            assert_eq!(a.map(f64::to_bits), b.map(f64::to_bits));
        }
    }
}

#[test]
fn fig_exports_roundtrip_without_using_custom_mfig_bytes() {
    let dir = tempfile::tempdir().unwrap();
    for source in [
        "semilogx([1 10 100],[1 2 3],'r--o');title('Unicode λ');xlabel('Time');ylabel('Value');",
        "plot([1 2],[3 4]);hold on;scatter([1 2],[5 6]);legend('A','B');",
        "bar([1 2 3],[2 4;3 5;4 6]);legend('A','B');",
    ] {
        let mut e = Engine::default();
        e.execute(source).unwrap();
        let f = e.figure.unwrap();
        let path = dir.path().join("test.fig");
        let mut f = f;
        if f.legend {
            std::fs::write(&path, b"previous").unwrap();
            assert!(files::save_figure(&path, &f).is_err());
            assert_eq!(std::fs::read(&path).unwrap(), b"previous");
            f.legend = false;
            // Hidden explicit legend state is still outside the supported FIG
            // writer schema; disabling visibility must not silently discard it.
            assert!(f.legend_state.is_some());
            assert!(files::save_figure(&path, &f).is_err());
            assert_eq!(std::fs::read(&path).unwrap(), b"previous");
            // This branch intentionally tests the separate nonlegend baseline.
            f.legend_state = None;
        }
        if f.series.iter().any(|s| s.kind == PlotKind::Bar) {
            assert!(files::save_figure(&path, &f).is_err());
            assert_eq!(std::fs::read(&path).unwrap(), b"previous");
            continue;
        }
        files::save_figure(&path, &f).unwrap();
        let bytes = std::fs::read(&path).unwrap();
        assert!(bytes.starts_with(b"MATLAB 5.0"));
        let reopened = files::load_figure(&path).unwrap();
        assert_eq!(reopened.title, f.title);
        assert_eq!(reopened.xscale, f.xscale);
        assert_eq!(reopened.legend, f.legend);
        assert_eq!(reopened.series.len(), f.series.len());
        for (a, b) in reopened.series.iter().zip(&f.series) {
            assert_eq!(a.points, b.points);
            assert_eq!(
                a.color,
                f.resolve_appearance(matrust::plot::BaseTheme::Light)
                    .series_color(b)
            );
            assert!(a.explicit_color && a.auto_palette.is_none());
            if b.explicit_color || b.auto_palette.is_none() {
                assert_eq!(a.color, b.color);
            }
            assert_eq!(a.pattern, b.pattern);
            assert_eq!(a.kind, b.kind);
            assert!((a.bar_width - b.bar_width).abs() < 1e-14);
        }
    }
}

#[test]
fn unsupported_and_truncated_figures_fail_without_panics() {
    assert!(files::load_figure(Path::new("tests/fixtures/fig/matlab-surface.fig")).is_err());
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("bad.fig");
    let source = std::fs::read("tests/fixtures/fig/matlab-lines.fig").unwrap();
    for end in (0..128)
        .chain((128..source.len()).step_by(137))
        .chain(source.len() - 8..source.len())
    {
        std::fs::write(&path, &source[..end]).unwrap();
        assert!(files::load_figure(&path).is_err(), "truncated at {end}");
    }
    let mut corrupt = source;
    corrupt[132..136].copy_from_slice(&u32::MAX.to_le_bytes());
    std::fs::write(&path, corrupt).unwrap();
    assert!(files::load_figure(&path).is_err());
    let mut random = 0x12345678_u32;
    for size in [0, 127, 128, 129, 256, 1024, 4096] {
        let mut bytes = vec![0; size];
        for b in &mut bytes {
            random ^= random << 13;
            random ^= random >> 17;
            random ^= random << 5;
            *b = random as u8;
        }
        if size >= 128 {
            bytes[126..128].copy_from_slice(b"IM");
        }
        std::fs::write(&path, bytes).unwrap();
        assert!(files::load_figure(&path).is_err());
    }
}
