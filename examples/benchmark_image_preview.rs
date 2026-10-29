#[allow(dead_code)]
#[path = "../src/desktop/image_load.rs"]
mod image_load;
#[allow(dead_code)]
#[path = "../src/desktop/path_label.rs"]
mod path_label;
use std::{hint::black_box, path::Path, sync::atomic::AtomicBool, time::Instant};
fn main() {
    let workers: usize = std::env::args()
        .nth(1)
        .unwrap_or("8".into())
        .parse()
        .unwrap();
    assert!([1, 8].contains(&workers));
    let folder = Path::new("target/evidence/plots/image-preview");
    std::fs::create_dir_all(folder).unwrap();
    let mut engine = matrust::Engine::default();
    engine.execute("t=linspace(0,1,2000);plot(t,sin(2*pi*5*t),t,cos(2*pi*9*t));title('Reopened plot export');xlabel('Time (s)');ylabel('Amplitude');legend('5 Hz','9 Hz');").unwrap();
    for ext in ["png", "jpg", "bmp", "tiff"] {
        matrust::export::export_figure(
            engine.figure.as_ref().unwrap(),
            &folder.join(format!("fixture.{ext}")),
            1920,
            1120,
        )
        .unwrap();
    }
    image::RgbImage::from_pixel(8000, 4000, image::Rgb([12, 80, 200]))
        .save(folder.join("large.bmp"))
        .unwrap();
    println!(
        "file,workers,file_bytes,original_pixels,preview_pixels,repetitions,median_ms,min_ms,max_ms"
    );
    rayon::ThreadPoolBuilder::new()
        .num_threads(workers)
        .build()
        .unwrap()
        .install(|| {
            for name in [
                "fixture.png",
                "fixture.jpg",
                "fixture.bmp",
                "fixture.tiff",
                "large.bmp",
            ] {
                let path = folder.join(name);
                let cancel = AtomicBool::new(false);
                let decoded = image_load::load(&path, 4096, &cancel).unwrap();
                assert_eq!(
                    decoded.original_size,
                    if name == "large.bmp" {
                        [8000, 4000]
                    } else {
                        [1920, 1120]
                    }
                );
                assert!(decoded.pixels.size.iter().all(|&s| s <= 4096));
                assert!(decoded.pixels.pixels.len() <= 8_000_000);
                if name == "large.bmp" {
                    assert_eq!(decoded.pixels.size, [4000, 2000]);
                    assert_eq!(
                        decoded.pixels.pixels[0].to_srgba_unmultiplied(),
                        [12, 80, 200, 255]
                    );
                }
                let bytes = decoded.file_bytes;
                let source =
                    u64::from(decoded.original_size[0]) * u64::from(decoded.original_size[1]);
                let preview = decoded.pixels.pixels.len();
                drop(decoded);
                let mut times = Vec::new();
                for i in 0..14 {
                    let start = Instant::now();
                    black_box(image_load::load(&path, 4096, &cancel).unwrap());
                    if i >= 3 {
                        times.push(start.elapsed().as_secs_f64() * 1000.);
                    }
                }
                times.sort_by(f64::total_cmp);
                println!(
                    "{name},{workers},{bytes},{source},{preview},11,{:.6},{:.6},{:.6}",
                    times[5], times[0], times[10]
                );
            }
        });
}
