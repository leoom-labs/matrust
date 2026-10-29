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

#[cfg(not(windows))]
fn main() {
    eprintln!("PDF preview benchmark currently requires Windows");
}

#[cfg(windows)]
fn main() {
    use std::{hint::black_box, path::Path, sync::atomic::AtomicBool, time::Instant};
    let workers: usize = std::env::args()
        .nth(1)
        .unwrap_or("8".into())
        .parse()
        .unwrap();
    assert!([1, 8].contains(&workers));
    let folder = Path::new("target/evidence/plots/pdf-preview");
    std::fs::create_dir_all(folder).unwrap();
    let mut engine = matrust::Engine::default();
    for (name, script) in [
        (
            "plot",
            "t=linspace(0,1,2000);plot(t,sin(2*pi*5*t),t,cos(2*pi*9*t));title('Reopened PDF export');xlabel('Time (s)');ylabel('Amplitude');legend('5 Hz','9 Hz');",
        ),
        (
            "dense",
            "clf;t=linspace(0,1,100000);plot(t,sin(2*pi*100*t));title('100000 original samples');",
        ),
    ] {
        engine.execute(script).unwrap();
        for ext in ["pdf", "png"] {
            matrust::export::export_figure(
                engine.figure.as_ref().unwrap(),
                &folder.join(format!("{name}.{ext}")),
                960,
                560,
            )
            .unwrap();
        }
    }
    let cancel = AtomicBool::new(false);
    println!("case,workers,file_bytes,pages_rendered,repetitions,median_ms,min_ms,max_ms");
    let first = Instant::now();
    let (preview, _) = pdf_load::render(
        pdf_load::open(&folder.join("plot.pdf"), &cancel).unwrap(),
        0,
        4096,
        &cancel,
    )
    .unwrap();
    let first_ms = first.elapsed().as_secs_f64() * 1000.;
    println!(
        "first_native_open,{workers},{},1,1,{first_ms:.6},{first_ms:.6},{first_ms:.6}",
        preview.file_bytes
    );
    drop(preview);
    fn measure(name: &str, workers: usize, bytes: usize, pages: u32, mut action: impl FnMut()) {
        let mut times = Vec::new();
        for iteration in 0..14 {
            let start = Instant::now();
            action();
            if iteration >= 3 {
                times.push(start.elapsed().as_secs_f64() * 1000.);
            }
        }
        times.sort_by(f64::total_cmp);
        println!(
            "{name},{workers},{bytes},{pages},11,{:.6},{:.6},{:.6}",
            times[5], times[0], times[10]
        );
    }
    rayon::ThreadPoolBuilder::new()
        .num_threads(workers)
        .build()
        .unwrap()
        .install(|| {
            for name in ["plot", "dense", "pages"] {
                let path = if name == "pages" {
                    Path::new("tests/fixtures/preview/pdf-preview/pages.pdf").to_path_buf()
                } else {
                    folder.join(format!("{name}.pdf"))
                };
                let document = pdf_load::open(&path, &cancel).unwrap();
                for page in 0..document.page_count {
                    let (image, _) =
                        pdf_load::render(document.clone(), page, 4096, &cancel).unwrap();
                    let rgba: Vec<_> = image
                        .pixels
                        .pixels
                        .iter()
                        .flat_map(|p| p.to_srgba_unmultiplied())
                        .collect();
                    image::RgbaImage::from_raw(
                        image.pixels.size[0] as u32,
                        image.pixels.size[1] as u32,
                        rgba,
                    )
                    .unwrap()
                    .save(folder.join(format!("{name}-native-{}.png", page + 1)))
                    .unwrap();
                }
                measure(
                    &format!("{name}_open"),
                    workers,
                    document.file_bytes,
                    1,
                    Box::new(|| {
                        let doc = pdf_load::open(black_box(&path), &cancel).unwrap();
                        drop(black_box(pdf_load::render(doc, 0, 4096, &cancel).unwrap()));
                    }),
                );
                measure(
                    &format!("{name}_cached_cycle"),
                    workers,
                    document.file_bytes,
                    document.page_count,
                    Box::new(|| {
                        for page in 0..document.page_count {
                            drop(black_box(
                                pdf_load::render(document.clone(), page, 4096, &cancel).unwrap(),
                            ));
                        }
                    }),
                );
            }
        });
}
