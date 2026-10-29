use matrust::{
    Engine, files,
    plot::{Arrangement, Figure, TileGrid},
};
use std::{hint::black_box, time::Instant};
fn measure(case: &str, workers: usize, mut action: impl FnMut()) {
    let mut times = Vec::new();
    for i in 0..14 {
        let start = Instant::now();
        action();
        if i >= 3 {
            times.push(start.elapsed().as_secs_f64() * 1000.);
        }
    }
    times.sort_by(f64::total_cmp);
    println!(
        "{case},{workers},11,{:.6},{:.6},{:.6}",
        times[5], times[0], times[10]
    );
}
fn main() {
    let workers: usize = std::env::args()
        .nth(1)
        .unwrap_or("8".into())
        .parse()
        .unwrap();
    assert!([1, 8].contains(&workers));
    println!("case,workers,repetitions,median_ms,min_ms,max_ms");
    rayon::ThreadPoolBuilder::new()
        .num_threads(workers)
        .build()
        .unwrap()
        .install(|| {
            let mut engine = Engine::default();
            engine
                .execute("x=1:100000;tiledlayout(4,2);for k=1:8;nexttile;plot(x);end;")
                .unwrap();
            let original = engine.figure.as_ref().unwrap();
            assert_eq!(original.point_count(), 800000);
            let mut figure = original.clone();
            measure("1000_select_800000_samples", workers, || {
                for i in 0..1000 {
                    figure.next_tile(Some(i % 8 + 1), None).unwrap();
                }
                black_box(&figure);
            });
            measure("clone_800000_samples", workers, || {
                black_box(original.clone());
            });
            measure("construct_8_axes_800000_samples", workers, || {
                engine
                    .execute("tiledlayout(4,2);for k=1:8;nexttile;plot(x);end;")
                    .unwrap();
                black_box(engine.figure.as_ref());
            });
            measure("flow_16_axes", workers, || {
                let mut f =
                    Figure::tiled_layout(TileGrid::new(Arrangement::Flow, 1, 1).unwrap()).unwrap();
                for _ in 0..16 {
                    f.next_tile(None, None).unwrap();
                }
                black_box(f);
            });
            let mut small = Engine::default();
            small.execute(include_str!("tiled_layout.m")).unwrap();
            let f = small.figure.as_ref().unwrap();
            measure("svg_3_axes_450_samples", workers, || {
                black_box(f.svg());
            });
            let folder = std::path::Path::new("target/evidence/plots/tiled");
            std::fs::create_dir_all(folder).unwrap();
            let file = folder.join(format!("benchmark-{workers}.mfig"));
            measure("save_3_axes_450_samples", workers, || {
                files::save_figure(&file, f).unwrap();
            });
            assert_eq!(files::load_figure(&file).unwrap().svg(), f.svg());
            if workers == 1 {
                files::save_figure(&folder.join("signals.mfig"), f).unwrap();
                files::write_figure_csv(&folder.join("signals.csv"), f).unwrap();
                #[cfg(feature = "plot-export")]
                {
                    for ext in ["svg", "png"] {
                        matrust::export::export_figure(
                            f,
                            &folder.join(format!("signals.{ext}")),
                            1920,
                            1120,
                        )
                        .unwrap();
                    }
                    std::fs::create_dir_all("output/pdf").unwrap();
                    matrust::export::export_figure(
                        f,
                        std::path::Path::new("output/pdf/tiled.pdf"),
                        1920,
                        1120,
                    )
                    .unwrap();
                }
            }
        });
}
