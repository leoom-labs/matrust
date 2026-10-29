use matrust::{
    Engine, files,
    plot::{Figure, LinePattern, PlotKind, Series, clip_face_edge},
};
use std::sync::atomic::AtomicBool;

fn bars() -> Figure {
    let mut f = Figure::default();
    let mut s = Series::new(vec![[1., 2.], [3., 4.]], PlotKind::Bar);
    // This fixture deliberately represents historical fill-only native Bar files.
    s.bar_baseline_visible = false;
    s.color = [17, 129, 241];
    s.explicit_color = true;
    f.add(vec![s]).unwrap();
    f.legend = false;
    f.xlim = Some([0., 4.]);
    f.ylim = Some([0., 5.]);
    f
}
#[test]
fn face_style_changes_snapshot_without_copying_points_and_validates_targets() {
    let f = bars();
    let mut g = f.clone();
    g.series[0].face_edge_color = Some([231, 33, 87]);
    g.series[0].line_width = Some(2.);
    assert!(!f.same_snapshot(&g));
    assert!((f.series[0].points).same_storage(&g.series[0].points));
    assert_eq!(g.series[0].color, f.series[0].color);
    g.validate().unwrap();
    for kind in [
        PlotKind::Line,
        PlotKind::Scatter,
        PlotKind::Stairs,
        PlotKind::Stem,
    ] {
        let mut bad = g.clone();
        bad.series[0].kind = kind;
        assert!(bad.validate().is_err());
    }
    assert!(f.series[0].face_edge_color.is_none());
    assert!(!f.svg().contains("stroke=\"#e72157\""));
    assert!(g.svg().contains("stroke=\"#e72157\""));
    assert!(g.svg().contains("fill=\"#1181f1\""));
}
#[test]
fn public_bar_patch_properties_replay_with_separate_colors_and_atomic_errors() {
    let mut e = Engine::default();
    e.execute("a=gca;h=bar(a,[1 3],[2 4],'FaceColor',[0 0 1],'EdgeColor',[1 0 0],'LineStyle','--','LineWidth',2);hold(a,'on');p=patch(a,'XData',[0;1;0],'YData',[0;0;1],'FaceColor',[0 1 0],'EdgeColor',[1 0 1],'LineStyle',':','LineWidth',1);ec=get(h,'EdgeColor');fc=get(h,'FaceColor');").unwrap();
    let before = e.figure.as_ref().unwrap().clone();
    for call in [
        "set(h,'FaceColor',[0 1 0],'EdgeColor','flat');",
        "set(p,'EdgeColor','interp');",
        "set(h,'EdgeColor',[1 0 0],'LineWidth',NaN);",
        "plot(a,[1 2],[3 4],'EdgeColor',[1 0 0]);",
    ] {
        assert!(e.execute(call).is_err(), "{call}");
        assert!(before.same_snapshot(e.figure.as_ref().unwrap()));
    }
    let script = files::matlab_plot_script(&before, &AtomicBool::new(false)).unwrap();
    let mut replay = Engine::default();
    replay.execute(&script).unwrap();
    let result = replay.figure.as_ref().unwrap();
    assert_eq!(result.series.len(), 2);
    assert_eq!(result.series[0].kind, PlotKind::Bar);
    for (a, b) in before.series.iter().zip(&result.series) {
        assert_eq!(a.color, b.color);
        assert_eq!(a.face_edge_color, b.face_edge_color);
        assert_eq!(a.pattern, b.pattern);
        assert_eq!(a.line_width, b.line_width);
    }
    // The ordinary Bar now retains its genuine chart and original geometry.
    let bar = &before.series[0];
    let restored = &result.series[0];
    assert_eq!(restored.points, bar.points);
    assert_eq!(restored.bar_width.to_bits(), bar.bar_width.to_bits());
    assert_eq!(restored.bar_offset.to_bits(), bar.bar_offset.to_bits());
    assert_eq!(before.series[1].points, result.series[1].points);
    e.execute("set(h,'EdgeColor','none');set(p,'EdgeColor','none');")
        .unwrap();
    assert!(
        e.figure
            .as_ref()
            .unwrap()
            .series
            .iter()
            .all(|s| s.face_edge_color.is_none())
    );
    assert_eq!(
        e.figure.as_ref().unwrap().series[0].pattern,
        LinePattern::Dashed
    );
    assert!((before.series[0].points).same_storage(&e.figure.as_ref().unwrap().series[0].points));
}
#[test]
fn mfig_outlines_require_new_capability_and_old_defaults_keep_old_bytes() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("face.mfig");
    let mut f = bars();
    f.appearance = None; // Historical codec fixture, independent of modern theme intent.
    f.axes.palette_cycle = None; // Empty layout axes are not in all_axes_mut().
    for axes in f.all_axes_mut() {
        axes.palette_cycle = None;
        for series in &mut axes.series {
            series.auto_palette = None;
        }
    }
    files::save_figure(&path, &f).unwrap();
    let old = std::fs::read(&path).unwrap();
    assert!(old.starts_with(b"MATRFIG4"));
    let old_loaded = files::load_figure(&path).unwrap();
    assert!(old_loaded.series[0].face_edge_color.is_none());
    files::save_figure(&path, &old_loaded).unwrap();
    assert_eq!(std::fs::read(&path).unwrap(), old);
    for multi in [false, true] {
        let mut g = f.clone();
        g.series[0].face_edge_color = Some([231, 33, 87]);
        g.series[0].pattern = LinePattern::DashDot;
        g.series[0].line_width = Some(3.);
        if multi {
            g.layout = Some(matrust::plot::Layout {
                active: matrust::plot::Position::from_grid(1, 2, &[1]).unwrap(),
                others: Vec::new(),
                tiled: None,
                title: String::new(),
            });
        }
        files::save_figure(&path, &g).unwrap();
        let mut bytes = std::fs::read(&path).unwrap();
        assert_eq!(&bytes[..8], if multi { b"MATRFIGM" } else { b"MATRFIGL" });
        let decoded = files::load_figure(&path).unwrap();
        assert_eq!(decoded.svg(), g.svg());
        assert_eq!(
            decoded.series[0].face_edge_color,
            g.series[0].face_edge_color
        );
        bytes[..8].copy_from_slice(if multi { b"MATRFIGK" } else { b"MATRFIGJ" });
        std::fs::write(&path, bytes).unwrap();
        assert!(files::load_figure(&path).is_err());
    }
    let mut invalid = f.clone();
    invalid.series[0].kind = PlotKind::Line;
    invalid.series[0].face_edge_color = Some([1, 2, 3]);
    std::fs::write(&path, b"sentinel").unwrap();
    assert!(files::save_figure(&path, &invalid).is_err());
    assert_eq!(std::fs::read(&path).unwrap(), b"sentinel");
    let fig = dir.path().join("guard.fig");
    std::fs::write(&fig, b"sentinel").unwrap();
    assert!(files::save_figure(&fig, &f).is_err());
    assert_eq!(std::fs::read(&fig).unwrap(), b"sentinel");
}
#[test]
fn original_edge_clipping_never_outlines_a_viewport_cut() {
    let bounds = [-1., 1., -1., 1.];
    assert_eq!(
        clip_face_edge([-2., 0.], [2., 0.], bounds),
        Some([[-1., 0.], [1., 0.]])
    );
    let extreme = [
        [-f64::MAX, -f64::MAX],
        [f64::MAX, -f64::MAX],
        [f64::MAX, f64::MAX],
        [-f64::MAX, f64::MAX],
    ];
    for i in 0..4 {
        assert!(clip_face_edge(extreme[i], extreme[(i + 1) % 4], bounds).is_none());
    }
    assert_eq!(
        clip_face_edge([-f64::MAX, 0.], [f64::MAX, 0.], bounds),
        Some([[-1., 0.], [1., 0.]])
    );
    assert!(clip_face_edge([f64::NAN, 0.], [1., 0.], bounds).is_none());
    assert!(clip_face_edge([0., 0.], [1., 0.], [1., -1., -1., 1.]).is_none());
}
#[cfg(feature = "plot-export")]
#[test]
fn real_raster_and_vector_exports_keep_fill_edge_and_cancel_destination() {
    let mut f = bars();
    let no_edge =
        matrust::export::render_figure_with_cancel(&f, 960, 560, &AtomicBool::new(false)).unwrap();
    f.series[0].face_edge_color = Some([231, 33, 87]);
    f.series[0].line_width = Some(3.);
    let edge =
        matrust::export::render_figure_with_cancel(&f, 960, 560, &AtomicBool::new(false)).unwrap();
    let count = |image: &image::RgbaImage, color: [u8; 3]| {
        image.pixels().filter(|p| p.0[..3] == color).count()
    };
    assert_eq!(count(&no_edge, [231, 33, 87]), 0);
    assert!(count(&edge, [231, 33, 87]) > 500);
    assert!(count(&edge, [17, 129, 241]) > 1000);
    let dir = tempfile::tempdir().unwrap();
    for ext in ["svg", "pdf", "eps"] {
        let path = dir.path().join(format!("outline.{ext}"));
        matrust::export::export_vector_with_cancel(
            &f,
            &path,
            matrust::plot::Canvas {
                width: 640.,
                height: 480.,
            },
            &AtomicBool::new(false),
        )
        .unwrap();
        let before = std::fs::read(&path).unwrap();
        assert!(before.len() > 100);
        assert!(
            matrust::export::export_vector_with_cancel(
                &f,
                &path,
                matrust::plot::Canvas {
                    width: 640.,
                    height: 480.
                },
                &AtomicBool::new(true)
            )
            .is_err()
        );
        assert_eq!(std::fs::read(&path).unwrap(), before);
    }
}

#[test]
fn patch_independent_edge_updates_preserve_explicit_none_line_style() {
    let mut e = Engine::default();
    e.execute("p=patch('XData',[0;1;0],'YData',[0;0;1],'FaceColor',[0 0 1],'EdgeColor','none');set(p,'EdgeColor','black');").unwrap();
    let p = &e.figure.as_ref().unwrap().series[0];
    assert_eq!(p.face_edge_color, Some([0; 3]));
    assert_eq!(p.pattern, LinePattern::None);
    let payload = p.points.clone();
    e.execute("set(p,'LineStyle','-');").unwrap();
    let p = &e.figure.as_ref().unwrap().series[0];
    assert_eq!(p.pattern, LinePattern::Solid);
    assert_eq!(p.color, [0, 0, 255]);
    assert!((payload).same_storage(&p.points));
    let script =
        files::matlab_plot_script(e.figure.as_ref().unwrap(), &AtomicBool::new(false)).unwrap();
    let mut replay = Engine::default();
    replay.execute(&script).unwrap();
    assert_eq!(
        replay.figure.as_ref().unwrap().series[0].face_edge_color,
        Some([0; 3])
    );
    assert_eq!(
        replay.figure.as_ref().unwrap().series[0].pattern,
        LinePattern::Solid
    );
}

// Composition-owned regressions exercise both capabilities together, rather
// than inferring their union from separate L/M and N/O tests.
fn compact_outlined_scene(multiple_axes: bool) -> Figure {
    let mut engine = Engine::default();
    if multiple_axes {
        engine.execute("subplot(1,2,1);").unwrap();
    }
    engine.execute("x=(1:3).';y=x+2;a=x-.25;b=x+.25;c=y*0;p=patch('XData',[a b b a].','YData',[c c y y].','FaceColor',[.2 .4 .6],'EdgeColor',[1 0 1],'LineStyle','--','LineWidth',2);title('Compact');").unwrap();
    assert!(
        engine.figure.as_ref().unwrap().series[0]
            .points
            .patch()
            .is_some()
    );
    if multiple_axes {
        engine.execute("subplot(1,2,2);bar([1 3],[2 4],'FaceColor',[.6 .4 .2],'EdgeColor',[0 1 0],'LineStyle',':','LineWidth',1);title('Dense bar');").unwrap();
    }
    let mut figure = engine.figure.unwrap();
    // Retain the independent historical L/M + N/O capability coverage.
    for axes in figure.all_axes_mut() {
        for series in &mut axes.series {
            series.bar_baseline_visible = false;
        }
    }
    figure.validate().unwrap();
    figure
}

#[test]
fn compact_and_outline_native_capabilities_roundtrip_together_and_reject_downgrades() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("combined.mfig");
    for multiple_axes in [false, true] {
        let mut source = compact_outlined_scene(multiple_axes);
        source.appearance = None; // Historical codec fixture, independent of modern theme intent.
        source.axes.palette_cycle = None; // Empty layout axes are not in all_axes_mut().
        for axes in source.all_axes_mut() {
            axes.palette_cycle = None;
            for series in &mut axes.series {
                series.auto_palette = None;
            }
        }
        let before = source.clone();
        files::save_figure(&path, &source).unwrap();
        let bytes = std::fs::read(&path).unwrap();
        assert_eq!(
            &bytes[..8],
            if multiple_axes {
                b"MATRFIGO"
            } else {
                b"MATRFIGN"
            }
        );
        if multiple_axes {
            assert!(bytes.windows(8).any(|p| p == b"MATRFIGN"));
            assert!(bytes.windows(8).any(|p| p == b"MATRFIGL"));
        }
        let loaded = files::load_figure(&path).unwrap();
        assert_eq!(loaded.svg(), source.svg());
        assert_eq!(
            loaded.geometry_ledger().unwrap(),
            source.geometry_ledger().unwrap()
        );
        for ((position, a), (other, b)) in source.all_axes().zip(loaded.all_axes()) {
            assert_eq!(position, other);
            assert_eq!(a.series.len(), b.series.len());
            for (a, b) in a.series.iter().zip(&b.series) {
                assert_eq!(a.kind, b.kind);
                assert_eq!(a.face_edge_color, b.face_edge_color);
                assert_eq!(a.pattern, b.pattern);
                assert_eq!(a.line_width, b.line_width);
                assert_eq!(a.points.patch().is_some(), b.points.patch().is_some());
                assert_eq!(a.points.len(), b.points.len());
                for (a, b) in a.points.iter().zip(b.points.iter()) {
                    assert_eq!(a.map(f64::to_bits), b.map(f64::to_bits));
                }
            }
        }
        files::save_figure(&path, &loaded).unwrap();
        assert_eq!(std::fs::read(&path).unwrap(), bytes);
        for header in if multiple_axes {
            [b"MATRFIGM", b"MATRFIGK"]
        } else {
            [b"MATRFIGL", b"MATRFIGJ"]
        } {
            let mut invalid = bytes.clone();
            invalid[..8].copy_from_slice(header);
            std::fs::write(&path, invalid).unwrap();
            assert!(files::load_figure(&path).is_err());
        }
        assert!(source.same_snapshot(&before));
    }
}

#[test]
fn outlined_compact_script_and_companion_replay_preserve_style_and_coordinates_twice() {
    let source = compact_outlined_scene(false);
    let before = source.clone();
    let stop = AtomicBool::new(false);
    let dir = tempfile::tempdir().unwrap();
    let inline = files::matlab_plot_script(&source, &stop).unwrap();
    let mut direct = Engine::default();
    direct.execute(&inline).unwrap();
    assert_eq!(
        direct.figure.as_ref().unwrap().series[0].face_edge_color,
        Some([255, 0, 255])
    );
    let mut current = source.clone();
    for pass in 0..2 {
        let path = dir.path().join(format!("outlined_{pass}.m"));
        let bundle = files::write_matlab_plot_bundle_with_cancel(&path, &current, &stop).unwrap();
        assert!(bundle.data.is_file());
        let document = matrust::document::Document::open(&path).unwrap();
        let mut replay = Engine::default();
        replay.execute_document(&document.text, &path).unwrap();
        let result = replay.figure.unwrap();
        let a = &current.series[0];
        let b = &result.series[0];
        assert!(b.points.patch().is_some());
        assert_eq!(a.kind, b.kind);
        assert_eq!(a.color, b.color);
        assert_eq!(a.face_edge_color, b.face_edge_color);
        assert_eq!(a.pattern, b.pattern);
        assert_eq!(a.line_width, b.line_width);
        assert_eq!(a.visible, b.visible);
        assert_eq!(a.points.len(), b.points.len());
        for (a, b) in a.points.iter().zip(b.points.iter()) {
            assert_eq!(a.map(f64::to_bits), b.map(f64::to_bits));
        }
        assert_eq!(current.svg(), result.svg());
        current = result;
    }
    assert!(source.same_snapshot(&before));
    assert!(
        source.series[0]
            .points
            .same_storage(&before.series[0].points)
    );
}
