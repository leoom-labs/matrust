use matrust::{Engine, files};
use std::{hint::black_box, path::Path, time::Instant};

fn measure(case: &str, workers: usize, samples: usize, mut action: impl FnMut()) {
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
        "{case},{workers},{samples},11,{:.6},{:.6},{:.6}",
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
    println!("case,workers,samples,repetitions,median_ms,min_ms,max_ms");
    rayon::ThreadPoolBuilder::new()
        .num_threads(workers)
        .build()
        .unwrap()
        .install(|| {
            for count in [1000, 100000] {
                let mut e = Engine::default();
                e.execute(&format!(
                    "x=1:{count};tiledlayout(4,2);for k=1:8;nexttile;plot(x);end;"
                ))
                .unwrap();
                let grid = e.figure.unwrap();
                let mut position = grid.normalized_axes_positions()[0].1;
                position[0] += 0.005;
                position[2] -= 0.01;
                let mut custom = grid.clone();
                custom.reposition_axes(position).unwrap();
                for (label, source) in [("1000_grid_edits", &grid), ("1000_custom_edits", &custom)]
                {
                    measure(label, workers, count * 8, || {
                        for _ in 0..1000 {
                            let mut edited = source.clone();
                            edited.reposition_axes(position).unwrap();
                            black_box(edited);
                        }
                    });
                }
            }
            if workers == 1 {
                let mut e = Engine::default();
                e.execute(include_str!("fig_layouts.m")).unwrap();
                let mut f = e.figure.unwrap();
                f.reposition_axes([0.15, 0.10, 0.70, 0.30]).unwrap();
                let folder = Path::new("target/evidence/plots/axes-position");
                std::fs::create_dir_all(folder).unwrap();
                for ext in ["fig", "mfig"] {
                    let path = folder.join(format!("edited.{ext}"));
                    files::save_figure(&path, &f).unwrap();
                    assert_eq!(files::load_figure(&path).unwrap().svg(), f.svg());
                }
                files::write_figure_csv(&folder.join("edited.csv"), &f).unwrap();
                #[cfg(feature = "plot-export")]
                {
                    for ext in ["svg", "png"] {
                        matrust::export::export_figure(
                            &f,
                            &folder.join(format!("edited.{ext}")),
                            1920,
                            1120,
                        )
                        .unwrap();
                    }
                    matrust::export::export_figure(
                        &f,
                        Path::new("output/pdf/axes-position.pdf"),
                        1920,
                        1120,
                    )
                    .unwrap();
                }
            }
        });
}
