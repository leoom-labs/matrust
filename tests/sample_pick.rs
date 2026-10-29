use matrust::plot::{Axes, AxisScale, AxisTransform, PlotKind, SampleQuery, Series};
use std::sync::atomic::AtomicBool;

fn query() -> SampleQuery {
    SampleQuery {
        transforms: [AxisTransform::default(); 2],
        bounds: [0., 10., 0., 10.],
        pointer: [5., 5.],
        pixels: [1000., 100.],
    }
}
#[test]
fn original_sample_pick_uses_screen_metric_visibility_view_and_stable_ties() {
    let mut axes = Axes {
        series: vec![Series::new(
            vec![[5.1, 5.], [5., 5.5], [f64::NAN, 5.], [11., 5.]],
            PlotKind::Line,
        )],
        ..Axes::default()
    };
    let hit = axes
        .nearest_sample(query(), &AtomicBool::new(false))
        .unwrap()
        .unwrap();
    assert_eq!((hit.series, hit.index, hit.point), (0, 1, [5., 5.5]));
    axes.series
        .push(Series::new(vec![[5., 5.5], [5., 5.]], PlotKind::Scatter));
    axes.series[1].visible = false;
    assert_eq!(
        axes.nearest_sample(query(), &AtomicBool::new(false))
            .unwrap()
            .unwrap()
            .series,
        0
    );
    axes.series[1].visible = true;
    axes.series[1].points = std::sync::Arc::new(vec![[5., 5.5]]).into();
    assert_eq!(
        axes.nearest_sample(query(), &AtomicBool::new(false))
            .unwrap()
            .unwrap()
            .series,
        0
    );
    axes.series[0].points = std::sync::Arc::new(vec![[11., 5.], [f64::INFINITY, 3.]]).into();
    axes.series[1].visible = false;
    assert!(
        axes.nearest_sample(query(), &AtomicBool::new(false))
            .unwrap()
            .is_none()
    );
}
#[test]
fn original_sample_pick_handles_signed_logs_bar_offsets_and_invalid_queries() {
    let axes = Axes {
        xscale: AxisScale::Log,
        yscale: AxisScale::Log,
        series: vec![Series::new(
            vec![[-10., -100.], [0., 0.], [10., 100.]],
            PlotKind::Line,
        )],
        ..Axes::default()
    };
    let q = SampleQuery {
        transforms: [AxisTransform {
            scale: AxisScale::Log,
            negative: true,
        }; 2],
        bounds: [-3., 0., -3., 0.],
        pointer: [-1., -2.],
        pixels: [800., 600.],
    };
    let hit = axes
        .nearest_sample(q, &AtomicBool::new(false))
        .unwrap()
        .unwrap();
    assert_eq!(
        (hit.index, hit.point, hit.distance_squared),
        (0, [-10., -100.], 0.)
    );
    assert!(
        axes.nearest_sample(q, &AtomicBool::new(true))
            .unwrap_err()
            .contains("Cancelled")
    );
    let mut bad = q;
    bad.pixels[0] = f64::NAN;
    assert!(axes.nearest_sample(bad, &AtomicBool::new(false)).is_err());
    bad = q;
    bad.bounds[1] = bad.bounds[0];
    assert!(axes.nearest_sample(bad, &AtomicBool::new(false)).is_err());
    let mut bars = Axes::default();
    let mut series = Series::new(vec![[4., 5.]], PlotKind::Bar);
    series.bar_offset = 1.;
    bars.series.push(series);
    let hit = bars
        .nearest_sample(query(), &AtomicBool::new(false))
        .unwrap()
        .unwrap();
    assert_eq!((hit.point, hit.distance_squared), ([4., 5.], 0.));
}
#[test]
fn original_sample_pick_is_identical_with_one_and_eight_workers_and_keeps_indices() {
    let mut axes = Axes::default();
    axes.series.push(Series::new(
        (0..500000)
            .map(|i| [i as f64 / 50000., (i % 997) as f64 / 100.])
            .collect(),
        PlotKind::Line,
    ));
    let mut results = vec![];
    for workers in [1, 8] {
        let pool = rayon::ThreadPoolBuilder::new()
            .num_threads(workers)
            .build()
            .unwrap();
        results.push(pool.install(|| {
            axes.nearest_sample(query(), &AtomicBool::new(false))
                .unwrap()
        }));
    }
    assert_eq!(results[0], results[1]);
    let hit = results[0].unwrap();
    assert_eq!(hit.point, axes.series[0].points.get(hit.index).unwrap());
    let q = query();
    let reference = axes.series[0]
        .points
        .iter()
        .enumerate()
        .min_by(|(_, a), (_, b)| {
            let score = |p: &[f64; 2]| {
                ((p[0] - q.pointer[0]) * 100.).powi(2) + ((p[1] - q.pointer[1]) * 10.).powi(2)
            };
            score(a).total_cmp(&score(b))
        })
        .unwrap()
        .0;
    assert_eq!(hit.index, reference);
}
