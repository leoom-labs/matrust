use matrust::{
    files,
    plot::{Figure, PlotKind, Series},
};
use std::{hint::black_box, time::Instant};
fn main() {
    println!("operation,format,points,threads,repetitions,median_ms,file_bytes");
    for threads in [1, 8] {
        rayon::ThreadPoolBuilder::new()
            .num_threads(threads)
            .build()
            .unwrap()
            .install(|| {
                let dir = tempfile::tempdir().unwrap();
                for points in [1000, 100000, 1000000] {
                    let mut f = Figure::default();
                    f.add(vec![Series::new(
                        (0..points)
                            .map(|i| [i as f64, (i as f64 * 0.01).sin()])
                            .collect(),
                        PlotKind::Line,
                    )])
                    .unwrap();
                    for extension in ["mfig", "fig"] {
                        let path = dir.path().join(format!("figure.{extension}"));
                        files::save_figure(&path, &f).unwrap();
                        let imported = files::load_figure(&path).unwrap();
                        assert_eq!(imported.series[0].points, f.series[0].points);
                        let bytes = std::fs::metadata(&path).unwrap().len();
                        for op in ["load", "save"] {
                            let mut times = Vec::new();
                            for _ in 0..10 {
                                let start = Instant::now();
                                let output = if op == "load" {
                                    Some(black_box(files::load_figure(&path).unwrap()))
                                } else {
                                    files::save_figure(&path, &f).unwrap();
                                    None
                                };
                                times.push(start.elapsed().as_secs_f64() * 1000.);
                                drop(output);
                            }
                            times.sort_by(f64::total_cmp);
                            println!(
                                "{op},{extension},{points},{threads},10,{:.6},{bytes}",
                                (times[4] + times[5]) * 0.5
                            );
                        }
                    }
                }
            });
    }
}
