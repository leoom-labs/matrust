#[allow(dead_code)]
#[path = "../src/desktop/image_export.rs"]
mod image_export;
#[allow(dead_code)]
#[path = "../src/desktop/image_load.rs"]
mod image_load;
#[allow(dead_code)]
#[path = "../src/desktop/path_label.rs"]
mod path_label;
#[allow(dead_code)]
#[path = "../src/desktop/pdf_load.rs"]
mod pdf_load;
#[path = "../src/desktop/pdf_protocol.rs"]
mod pdf_protocol;
#[allow(dead_code)]
#[path = "../src/desktop/svg_load.rs"]
mod svg_load;
#[allow(dead_code)]
#[path = "../src/desktop/tiff_load.rs"]
mod tiff_load;
use std::{
    hint::black_box,
    path::Path,
    sync::{Arc, atomic::AtomicBool},
    time::Instant,
};
fn measure(name: &str, workers: usize, size: [usize; 2], mut action: impl FnMut()) {
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
        "{name},{workers},{},{},11,{:.6},{:.6},{:.6}",
        size[0], size[1], times[5], times[0], times[10]
    );
}
fn reference(path: &Path, image: &eframe::egui::ColorImage) {
    let bytes: Vec<_> = image
        .pixels
        .iter()
        .flat_map(|p| p.to_srgba_unmultiplied())
        .collect();
    image::RgbaImage::from_raw(image.size[0] as u32, image.size[1] as u32, bytes)
        .unwrap()
        .save(path)
        .unwrap();
}
fn main() {
    let workers: usize = std::env::args()
        .nth(1)
        .unwrap_or("8".into())
        .parse()
        .unwrap();
    assert!([1, 8].contains(&workers));
    let folder = Path::new("target/evidence/plots/preview-export");
    std::fs::create_dir_all(folder).unwrap();
    let cancel = AtomicBool::new(false);
    let mut engine = matrust::Engine::default();
    engine.execute("t=linspace(0,1,2000);plot(t,sin(2*pi*5*t),t,cos(2*pi*9*t));title('Save and copy a preview');xlabel('Time (s)');ylabel('Amplitude');legend('5 Hz','9 Hz');").unwrap();
    for ext in ["png", "svg", "pdf"] {
        matrust::export::export_figure(
            engine.figure.as_ref().unwrap(),
            &folder.join(format!("source.{ext}")),
            1920,
            1120,
        )
        .unwrap();
    }
    let alpha = tiff_load::open(
        Path::new("tests/fixtures/preview/tiff-preview/pages.tiff"),
        &cancel,
    )
    .unwrap();
    let cases = vec![
        (
            "plot",
            image_load::load(&folder.join("source.png"), 4096, &cancel).unwrap(),
        ),
        (
            "alpha",
            tiff_load::render(&alpha, 1, 4096, &cancel).unwrap(),
        ),
        (
            "rotated",
            tiff_load::render(&alpha, 2, 4096, &cancel).unwrap(),
        ),
        (
            "svg",
            svg_load::load(&folder.join("source.svg"), 4096, &cancel).unwrap(),
        ),
        (
            "pdf",
            pdf_load::render(
                pdf_load::open(&folder.join("source.pdf"), &cancel).unwrap(),
                0,
                4096,
                &cancel,
            )
            .unwrap()
            .0,
        ),
        (
            "large",
            image_load::Preview {
                original: None,
                path: folder.join("generated-source"),
                original_size: [4000, 2000],
                format: "generated",
                file_bytes: 0,
                pixels: eframe::egui::ColorImage::filled(
                    [4000, 2000],
                    eframe::egui::Color32::from_rgba_unmultiplied(60, 140, 220, 128),
                ),
            },
        ),
    ];
    println!("case,workers,width,height,repetitions,median_ms,min_ms,max_ms");
    rayon::ThreadPoolBuilder::new()
        .num_threads(workers)
        .build()
        .unwrap()
        .install(|| {
            for (name, preview) in cases {
                reference(
                    &folder.join(format!("{name}-reference.png")),
                    &preview.pixels,
                );
                let pixels = Arc::new(preview.pixels);
                for format in image_export::Format::ALL {
                    let request = image_export::Request {
                        path: folder.join(format!("{name}.{}", format.extensions()[0])),
                        source: preview.path.clone(),
                        pixels: pixels.clone(),
                    };
                    measure(
                        &format!("{name}_{}", format.extensions()[0]),
                        workers,
                        pixels.size,
                        || image_export::save(black_box(&request), &cancel).unwrap(),
                    );
                }
                let copy = image_export::clipboard(&pixels, &cancel).unwrap();
                reference(&folder.join(format!("{name}-clipboard.png")), &copy);
                drop(copy);
                measure(
                    &format!("{name}_clipboard_prepare"),
                    workers,
                    pixels.size,
                    || {
                        drop(black_box(
                            image_export::clipboard(&pixels, &cancel).unwrap(),
                        ))
                    },
                );
            }
        });
}
