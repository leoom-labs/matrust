use matrust::plot::{Figure, LegendLocation, PlotKind, Series};
use std::{hint::black_box, path::Path, time::Instant};
fn fixture(count: usize, points: usize, long: bool) -> Figure {
    let mut f = Figure::default();
    for index in 0..count {
        let mut series = Series::new(
            (0..points)
                .map(|i| [i as f64, (i % 37) as f64 + index as f64])
                .collect(),
            PlotKind::Line,
        );
        series.label = if long {
            format!(
                "Series {index:02} {}",
                "Long label <&> a\u{301} ".repeat(20)
            )
        } else {
            format!("Series {index:02}")
        };
        series.color = matrust::plot::COLORS[index % 7];
        series.visible = index != 3;
        f.add(vec![series]).unwrap();
    }
    f.legend = true;
    f.title = "Bounded export legend".into();
    f
}
fn save(dir: &Path, name: &str, f: &Figure) {
    matrust::files::save_figure(&dir.join(format!("{name}.mfig")), f).unwrap();
    std::fs::write(dir.join(format!("{name}.svg")), f.svg()).unwrap();
    #[cfg(feature = "plot-export")]
    for ext in ["png", "pdf"] {
        matrust::export::export_figure(f, &dir.join(format!("{name}.{ext}")), 960, 560).unwrap();
    }
}
fn main() {
    let workers: usize = std::env::args()
        .nth(1)
        .unwrap_or("1".into())
        .parse()
        .unwrap();
    assert!([1, 8].contains(&workers));
    println!("series,points_per_series,long_labels,workers,repetitions,median_ms,min_ms,max_ms");
    rayon::ThreadPoolBuilder::new()
        .num_threads(workers)
        .build()
        .unwrap()
        .install(|| {
            for count in [1, 64] {
                for points in [16, 1024] {
                    for long in [false, true] {
                        let f = fixture(count, points, long);
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
                            "{count},{points},{long},{workers},11,{:.6},{:.6},{:.6}",
                            times[5], times[0], times[10]
                        );
                    }
                }
            }
        });
    if workers == 1 {
        let dir = Path::new("target/evidence/plots/export-legend");
        std::fs::create_dir_all(dir).unwrap();
        let mut dense = fixture(64, 16, true);
        save(dir, "dense", &dense);
        dense.legend_location = LegendLocation::Southwest;
        dense.reposition_axes([0.2, 0.2, 0.2, 0.2]).unwrap();
        save(dir, "small", &dense);
        let mut engine = matrust::Engine::default();
        engine.execute("subplot(1,2,1);plot([1 2]);legend('url(#axes)');subplot(1,2,2);plot([3 4]);legend('url(#axes)');").unwrap();
        save(dir, "clips", engine.figure.as_ref().unwrap());
    }
}
