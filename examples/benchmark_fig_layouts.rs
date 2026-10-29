use matrust::{Engine, files};
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
            let folder = std::path::Path::new("target/evidence/plots/fig-layouts");
            std::fs::create_dir_all(folder).unwrap();
            measure("read_matlab_2_axes_4_samples", workers, || {
                black_box(
                    files::load_figure(std::path::Path::new(
                        "tests/fixtures/fig/matlab-subplots.fig",
                    ))
                    .unwrap(),
                );
            });
            let mut e = Engine::default();
            e.execute("x=1:100000;tiledlayout(4,2);for k=1:8;nexttile;plot(x);end;")
                .unwrap();
            let large = e.figure.as_ref().unwrap();
            let path = folder.join(format!("large-{workers}.fig"));
            measure("save_8_axes_800000_samples", workers, || {
                files::save_figure(&path, large).unwrap();
            });
            measure("load_8_axes_800000_samples", workers, || {
                black_box(files::load_figure(&path).unwrap());
            });
            let loaded = files::load_figure(&path).unwrap();
            assert_eq!(loaded.point_count(), 800000);
            assert_eq!(loaded.all_axes().count(), 8);
            for (_, a) in loaded.all_axes() {
                assert_eq!(a.series[0].points.get(99999).unwrap(), [100000., 100000.]);
            }
            e.execute(include_str!("fig_layouts.m")).unwrap();
            let small = e.figure.as_ref().unwrap();
            measure("svg_3_positioned_axes_600_samples", workers, || {
                black_box(small.svg());
            });
            let native = folder.join(format!("small-{workers}.mfig"));
            measure("native_save_3_axes_600_samples", workers, || {
                files::save_figure(&native, small).unwrap();
            });
            assert_eq!(files::load_figure(&native).unwrap().svg(), small.svg());
            if workers == 1 {
                let fig = folder.join("roundtrip.fig");
                files::save_figure(&fig, small).unwrap();
                let restored = files::load_figure(&fig).unwrap();
                assert_eq!(restored.svg(), small.svg());
                files::save_figure(&folder.join("roundtrip.mfig"), &restored).unwrap();
                files::write_figure_csv(&folder.join("roundtrip.csv"), &restored).unwrap();
                #[cfg(feature = "plot-export")]
                {
                    for ext in ["svg", "png", "jpg", "bmp", "tiff"] {
                        matrust::export::export_figure(
                            &restored,
                            &folder.join(format!("roundtrip.{ext}")),
                            1920,
                            1120,
                        )
                        .unwrap();
                    }
                    matrust::export::export_figure(
                        &restored,
                        std::path::Path::new("output/pdf/fig-layouts.pdf"),
                        1920,
                        1120,
                    )
                    .unwrap();
                }
            }
        });
}
