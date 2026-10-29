pub use matrust::{Result, files, plot};
#[allow(dead_code)]
#[path = "baselines/export_before_figure_copy.rs"]
mod before;
#[path = "../src/desktop/figure_copy.rs"]
mod figure_copy;
use std::{hint::black_box, path::Path, sync::atomic::AtomicBool, time::Instant};

fn measure(name: &str, workers: usize, mut action: impl FnMut()) {
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
        "{name},{workers},11,{:.6},{:.6},{:.6}",
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
    let folder = Path::new("target/evidence/plots/figure-copy");
    std::fs::create_dir_all(folder).unwrap();
    let cancel = AtomicBool::new(false);
    let mut engine = matrust::Engine::default();
    engine.execute("t=linspace(0,1,2000);plot(t,sin(2*pi*5*t),t,cos(2*pi*9*t));title('Copy figure');xlabel('Time (s)');ylabel('Amplitude');legend('5 Hz','9 Hz');").unwrap();
    let simple = engine.figure.clone().unwrap();
    engine.execute(include_str!("fig_layouts.m")).unwrap();
    let layout = engine.figure.clone().unwrap();
    engine
        .execute("figure; t=linspace(0,1,100000);plot(t,sin(2*pi*50*t));title('Dense figure');")
        .unwrap();
    let dense = engine.figure.unwrap();
    let cases = [
        ("plot", simple, 1920, 1120),
        ("layout", layout, 1600, 1600),
        ("dense", dense, 1920, 1120),
    ];
    println!("case,workers,repetitions,median_ms,min_ms,max_ms");
    rayon::ThreadPoolBuilder::new()
        .num_threads(workers)
        .build()
        .unwrap()
        .install(|| {
            for (name, mut figure, width, height) in cases {
                // Keep this historical white-margin exporter comparison on its
                // original fixed Light appearance; automatic themes differ intentionally.
                figure.appearance = None;
                let request = figure_copy::Request {
                    figure,
                    width,
                    height,
                };
                let prepared =
                    figure_copy::prepare_in_theme(&request, plot::BaseTheme::Light, &cancel)
                        .unwrap();
                let bytes: Vec<_> = prepared.pixels.iter().flat_map(|p| p.to_array()).collect();
                image::RgbaImage::from_raw(width, height, bytes.clone())
                    .unwrap()
                    .save(folder.join(format!("{name}-copy.png")))
                    .unwrap();
                // Retained original exporter is an independent implementation snapshot.
                // Compare every raster codec once before timing any case.
                for ext in ["png", "jpg", "bmp", "tiff"] {
                    let old = folder.join(format!("{name}-before.{ext}"));
                    let new = folder.join(format!("{name}-current.{ext}"));
                    before::export_figure(&request.figure, &old, width, height).unwrap();
                    matrust::export::export_figure(&request.figure, &new, width, height).unwrap();
                    assert_eq!(std::fs::read(&old).unwrap(), std::fs::read(&new).unwrap());
                    if ext != "jpg" {
                        assert_eq!(image::open(&old).unwrap().into_rgba8().as_raw(), &bytes);
                    }
                }
                let path = folder.join(format!("{name}-timed.png"));
                measure(&format!("{name}_direct_prepare"), workers, || {
                    drop(black_box(
                        figure_copy::prepare_in_theme(
                            black_box(&request),
                            plot::BaseTheme::Light,
                            &cancel,
                        )
                        .unwrap(),
                    ));
                });
                measure(&format!("{name}_export_reopen_prepare"), workers, || {
                    before::export_figure(&request.figure, &path, width, height).unwrap();
                    let rgba = image::open(&path).unwrap().into_rgba8();
                    let pixels = eframe::egui::ColorImage::from_rgba_unmultiplied(
                        [width as usize, height as usize],
                        rgba.as_raw(),
                    );
                    drop(black_box(pixels));
                });
                // Alternate original/current exports to expose refactor regressions.
                let mut old_times = Vec::new();
                let mut new_times = Vec::new();
                for i in 0..14 {
                    for old in if i % 2 == 0 {
                        [true, false]
                    } else {
                        [false, true]
                    } {
                        let start = Instant::now();
                        if old {
                            before::export_figure(&request.figure, &path, width, height).unwrap();
                        } else {
                            matrust::export::export_figure(&request.figure, &path, width, height)
                                .unwrap();
                        }
                        if i >= 3 {
                            if old { &mut old_times } else { &mut new_times }
                                .push(start.elapsed().as_secs_f64() * 1000.);
                        }
                    }
                }
                for (label, mut times) in [("before_png", old_times), ("current_png", new_times)] {
                    times.sort_by(f64::total_cmp);
                    println!(
                        "{name}_{label},{workers},11,{:.6},{:.6},{:.6}",
                        times[5], times[0], times[10]
                    );
                }
            }
        });
}
