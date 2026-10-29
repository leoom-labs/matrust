use matrust::{
    Engine,
    plot::{Figure, MarkerShape, PlotKind, Series},
};
use std::{hint::black_box, time::Instant};
fn main() {
    let workers: usize = std::env::args()
        .nth(1)
        .unwrap_or("1".into())
        .parse()
        .unwrap();
    assert!([1, 8].contains(&workers));
    println!("marker,points,workers,repetitions,median_ms,min_ms,max_ms");
    rayon::ThreadPoolBuilder::new()
        .num_threads(workers)
        .build()
        .unwrap()
        .install(|| {
            for points in [1000, 10000] {
                for shape in [
                    MarkerShape::Circle,
                    MarkerShape::Cross,
                    MarkerShape::Square,
                    MarkerShape::Up,
                ] {
                    let mut s = Series::new(
                        (0..points).map(|i| [i as f64, (i % 37) as f64]).collect(),
                        PlotKind::Scatter,
                    );
                    s.marker_shape = Some(shape);
                    s.marker_size = Some(6.);
                    s.line_width = Some(1.5);
                    let mut f = Figure::default();
                    f.add(vec![s]).unwrap();
                    let mut times = Vec::new();
                    for sample in 0..14 {
                        let start = Instant::now();
                        let svg = black_box(f.svg());
                        let elapsed = start.elapsed().as_secs_f64() * 1000.;
                        assert!(!svg.contains("NaN"));
                        if sample >= 3 {
                            times.push(elapsed);
                        }
                    }
                    times.sort_by(f64::total_cmp);
                    println!(
                        "{shape:?},{points},{workers},11,{:.6},{:.6},{:.6}",
                        times[5], times[0], times[10]
                    );
                }
            }
        });
    if workers == 1 {
        let dir = std::path::Path::new("target/evidence/plots/marker-styles");
        std::fs::create_dir_all(dir).unwrap();
        let mut engine = Engine::default();
        engine.execute(include_str!("marker_styles.m")).unwrap();
        let mut demo = engine.figure.unwrap();
        matrust::files::save_figure(&dir.join("markers.mfig"), &demo).unwrap();
        std::fs::write(dir.join("markers.svg"), demo.svg()).unwrap();
        std::fs::write(
            dir.join("markers.m"),
            matrust::files::matlab_plot_script(&demo, &std::sync::atomic::AtomicBool::new(false))
                .unwrap(),
        )
        .unwrap();
        #[cfg(feature = "plot-export")]
        for ext in ["png", "pdf"] {
            matrust::export::export_figure(&demo, &dir.join(format!("markers.{ext}")), 960, 560)
                .unwrap();
        }
        demo.legend = false;
        matrust::files::save_figure(&dir.join("markers.fig"), &demo).unwrap();
        let mut all = Figure::default();
        for (i, shape) in MarkerShape::ALL.into_iter().enumerate() {
            let mut s = Series::new(
                vec![[1., i as f64], [2., i as f64], [3., i as f64]],
                PlotKind::Line,
            );
            s.markers = true;
            s.marker_shape = Some(shape);
            s.marker_size = Some(12.);
            s.line_width = Some(1.5);
            s.color = [255, 0, 0];
            s.explicit_color = true;
            s.label = shape.label().into();
            all.add(vec![s]).unwrap();
        }
        all.xgrid = false;
        all.ygrid = false;
        all.title = "All supported marker shapes".into();
        matrust::files::save_figure(&dir.join("all-shapes.fig"), &all).unwrap();
        std::fs::write(dir.join("all-shapes.svg"), all.svg()).unwrap();
    }
}
