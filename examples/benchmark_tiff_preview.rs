#[allow(dead_code)]
#[path = "baselines/image_load_before_tiff.rs"]
mod before;
#[allow(dead_code)]
#[path = "../src/desktop/image_load.rs"]
mod image_load;
#[allow(dead_code)]
#[path = "../src/desktop/path_label.rs"]
mod path_label;
#[allow(dead_code)]
#[path = "../src/desktop/tiff_load.rs"]
mod tiff_load;
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
    let folder = Path::new("target/evidence/plots/tiff-preview");
    std::fs::create_dir_all(folder).unwrap();
    let mut engine = matrust::Engine::default();
    engine.execute("t=linspace(0,1,2000);plot(t,sin(2*pi*5*t),t,cos(2*pi*9*t));title('Reopened TIFF export');xlabel('Time (s)');ylabel('Amplitude');legend('5 Hz','9 Hz');").unwrap();
    for ext in ["tiff", "png", "jpg", "bmp"] {
        matrust::export::export_figure(
            engine.figure.as_ref().unwrap(),
            &folder.join(format!("plot.{ext}")),
            1920,
            1120,
        )
        .unwrap();
    }
    let cancel = AtomicBool::new(false);
    println!("case,workers,repetitions,median_ms,min_ms,max_ms");
    rayon::ThreadPoolBuilder::new()
        .num_threads(workers)
        .build()
        .unwrap()
        .install(|| {
            for name in [
                "plot",
                "pages",
                "big-endian",
                "big-tiff",
                "big-tiff-endian",
                "large-pages",
            ] {
                let committed =
                    Path::new("tests/fixtures/preview/tiff-preview").join(format!("{name}.tiff"));
                let path = if committed.is_file() {
                    committed
                } else {
                    folder.join(format!("{name}.tiff"))
                };
                let document = tiff_load::open(&path, &cancel).unwrap();
                match before::load(&path, 4096, &cancel) {
                    Ok(original) => assert_eq!(
                        original.pixels.pixels,
                        tiff_load::render(&document, 0, 4096, &cancel)
                            .unwrap()
                            .pixels
                            .pixels
                    ),
                    // The old generic format sniffer rejects BigTIFF before decoding.
                    // Independent fixture checks cover these newly admitted files.
                    Err(error) => assert!(
                        name.starts_with("big-tiff") && error.contains("Preview supports"),
                        "{name}: {error}"
                    ),
                }
                for page in 0..document.page_count {
                    let preview = tiff_load::render(&document, page, 4096, &cancel).unwrap();
                    let rgba: Vec<_> = preview
                        .pixels
                        .pixels
                        .iter()
                        .flat_map(|p| p.to_srgba_unmultiplied())
                        .collect();
                    image::RgbaImage::from_raw(
                        preview.pixels.size[0] as u32,
                        preview.pixels.size[1] as u32,
                        rgba,
                    )
                    .unwrap()
                    .save(folder.join(format!("{name}-native-{}.png", page + 1)))
                    .unwrap();
                }
                measure(&format!("{name}_open"), workers, || {
                    let doc = tiff_load::open(black_box(&path), &cancel).unwrap();
                    drop(black_box(
                        tiff_load::render(&doc, 0, 4096, &cancel).unwrap(),
                    ));
                });
                measure(
                    &format!("{name}_cached_{}_pages", document.page_count),
                    workers,
                    || {
                        for page in 0..document.page_count {
                            drop(black_box(
                                tiff_load::render(&document, page, 4096, &cancel).unwrap(),
                            ));
                        }
                    },
                );
            }
            // Alternate old/current single-page workflows within each timed sample.
            for ext in ["tiff", "png", "jpg", "bmp"] {
                let path = folder.join(format!("plot.{ext}"));
                let old_pixels = before::load(&path, 4096, &cancel).unwrap().pixels;
                let new_pixels = if ext == "tiff" {
                    tiff_load::render(&tiff_load::open(&path, &cancel).unwrap(), 0, 4096, &cancel)
                        .unwrap()
                        .pixels
                } else {
                    image_load::load(&path, 4096, &cancel).unwrap().pixels
                };
                assert_eq!(old_pixels, new_pixels);
                drop((old_pixels, new_pixels));
                let mut old = Vec::new();
                let mut new = Vec::new();
                for iteration in 0..14 {
                    for current in if iteration % 2 == 0 {
                        [false, true]
                    } else {
                        [true, false]
                    } {
                        let start = Instant::now();
                        if !current {
                            drop(black_box(before::load(&path, 4096, &cancel).unwrap()));
                        } else if ext == "tiff" {
                            let doc = tiff_load::open(&path, &cancel).unwrap();
                            drop(black_box(
                                tiff_load::render(&doc, 0, 4096, &cancel).unwrap(),
                            ));
                        } else {
                            drop(black_box(image_load::load(&path, 4096, &cancel).unwrap()));
                        }
                        if iteration >= 3 {
                            if current { &mut new } else { &mut old }
                                .push(start.elapsed().as_secs_f64() * 1000.);
                        }
                    }
                }
                for (label, mut times) in [("before", old), ("current", new)] {
                    times.sort_by(f64::total_cmp);
                    println!(
                        "paired_{ext}_{label},{workers},11,{:.6},{:.6},{:.6}",
                        times[5], times[0], times[10]
                    );
                }
            }
        });
}
