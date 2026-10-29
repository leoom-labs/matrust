use matrust::{
    Engine, files,
    plot::{
        AxisScale, AxisTransform, Figure, LEGEND_PROBE_LIMIT, LegendBestCache, LegendBestView,
        LegendLocation, LegendRect, LinePattern, PlotKind, Series, best_legend_location,
        legend_geometry, outer_allocations,
    },
};
use std::sync::{Arc, atomic::AtomicBool};

fn rect(v: [f64; 4]) -> LegendRect {
    LegendRect::new(v)
}
fn scene() -> Figure {
    let mut f = Figure::default();
    let mut s = Series::new(vec![[0., 0.], [1., 1.]], PlotKind::Line);
    s.label = "Signal".into();
    f.add(vec![s]).unwrap();
    f.legend = true;
    f
}
fn view() -> LegendBestView {
    LegendBestView {
        bounds: [0., 1., 0., 1.],
        transforms: [AxisTransform::default(); 2],
        frame: rect([0., 0., 400., 300.]),
        extent: [100., 60.],
    }
}

#[test]
fn outside_geometry_reserves_the_correct_band_and_preserves_hidden_layout() {
    let outer = rect([0., 0., 960., 560.]);
    let inner = rect([70., 60., 920., 490.]);
    for location in LegendLocation::ALL.into_iter().filter(|v| v.is_outside()) {
        let g = legend_geometry(
            outer,
            inner,
            [70., 60., 40., 70.],
            Some([120., 80.]),
            location,
            false,
        )
        .unwrap();
        let legend = g.legend.unwrap();
        assert!(outer.contains(legend) && outer.contains(g.data));
        assert!(!legend.overlaps(g.data));
        assert!(g.data.width() >= 16. && g.data.height() >= 16.);
        match location.outside_side().unwrap() {
            0 => assert_eq!(g.data.left, inner.left + 128.),
            1 => assert_eq!(g.data.top, inner.top + 88.),
            2 => assert_eq!(g.data.right, inner.right - 128.),
            _ => assert_eq!(g.data.bottom, inner.bottom - 88.),
        }
        assert_eq!(
            legend_geometry(outer, inner, [70., 60., 40., 70.], None, location, false)
                .unwrap()
                .data,
            inner
        );
    }
    let fixed = rect([100., 100., 600., 400.]);
    let g = legend_geometry(
        outer,
        fixed,
        [20.; 4],
        Some([100., 70.]),
        LegendLocation::Eastoutside,
        true,
    )
    .unwrap();
    assert_eq!(g.data, fixed);
    assert_eq!(g.legend.unwrap().left, 628.);
    assert!(
        legend_geometry(
            outer,
            inner,
            [70., 60., 40., 70.],
            Some([120., 80.]),
            LegendLocation::Eastoutside,
            true
        )
        .is_err()
    );
    assert!(
        legend_geometry(
            rect([0., 0., 10., 10.]),
            rect([1., 1., 9., 9.]),
            [20.; 4],
            None,
            LegendLocation::North,
            false
        )
        .is_err()
    );
    assert!(!rect([-f64::MAX, 0., f64::MAX, 1.]).valid());
    assert!(
        legend_geometry(
            rect([0., 0., 100., 100.]),
            rect([-10., 20., 50., 80.]),
            [0.; 4],
            Some([10., 10.]),
            LegendLocation::Eastoutside,
            true
        )
        .is_err()
    );
}

#[test]
fn simultaneous_outer_allocation_is_order_independent_and_does_not_move_requests() {
    let canvas = rect([0., 0., 960., 560.]);
    // Irregular staggered custom panels exercise both allocation passes.
    let input = [
        rect([80., 60., 390., 220.]),
        rect([530., 80., 880., 260.]),
        rect([180., 350., 760., 490.]),
    ];
    let a = outer_allocations(&input, canvas, [70., 60., 40., 70.]).unwrap();
    for permutation in [[2, 0, 1], [1, 2, 0], [2, 1, 0]] {
        let b = outer_allocations(&permutation.map(|i| input[i]), canvas, [70., 60., 40., 70.])
            .unwrap();
        for (j, i) in permutation.into_iter().enumerate() {
            assert_eq!(a[i], b[j]);
        }
    }
    for (i, allocation) in a.iter().enumerate() {
        assert!(allocation.contains(input[i]));
        assert!(canvas.contains(*allocation));
        assert!(!a[..i].iter().any(|other| other.overlaps(*allocation)));
    }
    assert!(outer_allocations(&[input[0], input[0]], canvas, [0.; 4]).is_err());
}

#[test]
fn best_scores_original_segments_respects_gaps_and_log_domains() {
    let mut f = scene();
    // A line crossing the NE candidate has no sample inside it. Segment
    // overlap must still make NW preferable, with stable tie breaking.
    f.series[0].points = Arc::new(vec![[0.5, 0.9], [1.5, 0.9]]).into();
    let result = best_legend_location(&f.axes, view(), None).unwrap();
    assert_eq!(result.location, LegendLocation::Northwest);
    assert!(result.scores[0] > 0. && result.scores[1] == 0.);
    f.series[0].points = Arc::new(vec![[0.5, 0.9], [f64::NAN, f64::NAN], [1.5, 0.9]]).into();
    let gap = best_legend_location(&f.axes, view(), None).unwrap();
    assert_eq!(gap.scores, [0.; 8]);
    assert_eq!(gap.location, LegendLocation::Northeast);
    let mut logarithmic = view();
    logarithmic.transforms[0] = AxisTransform {
        scale: AxisScale::Log,
        negative: false,
    };
    f.series[0].points = Arc::new(vec![[-1., 0.9], [10., 0.9]]).into();
    assert_eq!(
        best_legend_location(&f.axes, logarithmic, None)
            .unwrap()
            .scores,
        [0.; 8]
    );
    logarithmic.transforms[0].negative = true;
    logarithmic.bounds[0] = -1.;
    logarithmic.bounds[1] = 0.;
    f.series[0].points = Arc::new(vec![[-10., 0.9], [-1., 0.9]]).into();
    assert!(
        best_legend_location(&f.axes, logarithmic, None)
            .unwrap()
            .scores[0]
            > 0.
    );
}

#[test]
fn best_probe_budget_cache_and_cancellation_do_not_retain_samples() {
    let mut f = scene();
    f.series[0].points = Arc::new(
        (0..100_000)
            .map(|i| [i as f64 / 99_999., 0.9])
            .collect::<Vec<_>>(),
    )
    .into();
    let points = f.series[0].points.clone();
    let owners = points.strong_count();
    let mut cache = LegendBestCache::default();
    let result = cache.resolve(&f.axes, view(), false).unwrap();
    assert_eq!(result.probes, LEGEND_PROBE_LIMIT);
    assert_eq!(points.strong_count(), owners);
    assert_eq!(cache.resolve(&f.axes, view(), false).unwrap(), result);
    f.series[0].visible = false;
    assert_eq!(cache.resolve(&f.axes, view(), true).unwrap(), result);
    assert_eq!(cache.resolve(&f.axes, view(), false).unwrap().probes, 0);
    assert_eq!(points.strong_count(), owners);
    assert!(
        best_legend_location(&f.axes, view(), Some(&AtomicBool::new(true)))
            .unwrap_err()
            .contains("Cancelled")
    );
    let mut tiny = view();
    tiny.extent = [1e-200; 2];
    assert!(best_legend_location(&f.axes, tiny, None).is_err());
    f.title = "X".repeat(4097);
    assert!(cache.resolve(&f.axes, view(), false).is_err());
}

#[test]
fn best_accounts_for_visible_markers_bars_and_stems_without_drawing_hidden_lines() {
    let mut f = scene();
    f.series[0].points = Arc::new(vec![[0.9, 0.9]]).into();
    f.series[0].pattern = LinePattern::None;
    assert_eq!(
        best_legend_location(&f.axes, view(), None).unwrap().scores,
        [0.; 8]
    );
    f.series[0].markers = true;
    assert!(best_legend_location(&f.axes, view(), None).unwrap().scores[0] > 0.);
    f.series[0].markers = false;
    f.series[0].kind = PlotKind::Bar;
    f.series[0].bar_width = 0.1;
    assert!(best_legend_location(&f.axes, view(), None).unwrap().scores[0] > 0.);
    f.series[0].kind = PlotKind::Stem;
    f.series[0].pattern = LinePattern::Solid;
    assert!(best_legend_location(&f.axes, view(), None).unwrap().scores[0] > 0.);
}

#[test]
fn custom_outside_addition_order_preserves_original_requests_and_shared_data() {
    fn build(reverse: bool) -> Figure {
        let mut e = Engine::default();
        e.execute("subplot('Position',[0.1 0.2 0.3 0.6]);plot([1 2]);subplot('Position',[0.6 0.2 0.3 0.6]);plot([3 4]);").unwrap();
        let requests = e.figure.as_ref().unwrap().normalized_axes_positions();
        let snapshots: Vec<_> = e
            .figure
            .as_ref()
            .unwrap()
            .all_axes()
            .map(|(p, a)| (p, a.series[0].points.clone()))
            .collect();
        for left in if reverse { [0.6, 0.1] } else { [0.1, 0.6] } {
            e.execute(&format!("subplot('Position',[{left} 0.2 0.3 0.6]);legend('Signal','Location','eastoutside');")).unwrap();
        }
        let f = e.figure.unwrap();
        for (p, a) in f.all_axes() {
            let original = requests.iter().find(|(q, _)| *q == p).unwrap().1;
            assert_eq!(a.placement.unwrap().requested_inner, original);
            assert!(
                (a.series[0].points)
                    .same_storage(&snapshots.iter().find(|(q, _)| *q == p).unwrap().1)
            );
        }
        f
    }
    let a = build(false);
    let b = build(true);
    for (p, axes) in a.all_axes() {
        let other = b.all_axes().find(|(q, _)| *q == p).unwrap().1;
        assert_eq!(axes.placement, other.placement);
    }
    let mut ar = a.resolved_axes_positions().unwrap();
    ar.sort_by_key(|(p, _)| *p);
    let mut br = b.resolved_axes_positions().unwrap();
    br.sort_by_key(|(p, _)| *p);
    assert_eq!(ar, br);
}

#[test]
fn explicit_inner_outer_properties_are_atomic_and_roundtrip_without_double_shrink() {
    let mut e = Engine::default();
    e.execute("subplot('Position',[0.15 0.25 0.45 0.5],'OuterPosition',[0.05 0.1 0.9 0.8],'PositionConstraint','outerposition');plot([1 2]);legend('Signal','Location','eastoutside');").unwrap();
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
    let before = e.figure.clone().unwrap();
    for command in [
        "subplot('Position',[0.15 0.25 0.45 0.5],'PositionConstraint','invalid')",
        "subplot('Position',[0.15 0.25 0.45 0.5],'OuterPosition',[0 0 -1 1])",
        "subplot('Position',[0.15 0.25 0.45 0.5],'PositionConstraint','outerposition','ActivePositionProperty','position')",
    ] {
        assert!(e.execute(command).is_err());
        assert!(e.figure.as_ref().unwrap().same_snapshot(&before));
    }
    let dir = tempfile::tempdir().unwrap();
    let file = dir.path().join("layout.mfig");
    files::save_figure(&file, &before).unwrap();
    assert!(std::fs::read(&file).unwrap().starts_with(b"MATRFIGK"));
    let restored = files::load_figure(&file).unwrap();
    assert!(
        restored
            .all_axes()
            .zip(before.all_axes())
            .all(|((p, a), (q, b))| p == q && a.same_metadata(b))
    );
    assert_eq!(restored.svg(), before.svg());
    assert_eq!(
        restored.resolved_axes_positions().unwrap(),
        before.resolved_axes_positions().unwrap()
    );
    let mut solved = before.clone();
    let p = solved.placement.as_mut().unwrap();
    p.requested_inner = [0.15, 0.25, 0.45, 0.5];
    p.imported_solved = true;
    let expected = p.requested_inner;
    solved.validate().unwrap();
    files::save_figure(&file, &solved).unwrap();
    let reopened = files::load_figure(&file).unwrap();
    for (a, b) in reopened.resolved_axes_positions().unwrap()[0]
        .1
        .into_iter()
        .zip(expected)
    {
        assert!((a - b).abs() < 1e-12);
    }
    assert_eq!(reopened.placement, solved.placement);
    let script = files::matlab_plot_script(&before, &AtomicBool::new(false)).unwrap();
    assert!(script.contains("'OuterPosition'"));
    assert!(script.contains("'ActivePositionProperty','outerposition'"));
    assert!(script.contains("'Location','eastoutside'"));
    assert!(files::save_matlab_figure(&dir.path().join("unsupported.fig"), &before).is_err());
}
