#[allow(dead_code)]
#[path = "../src/desktop/image_load.rs"]
mod image_load;
#[allow(dead_code)]
#[path = "../src/desktop/path_label.rs"]
mod path_label;
#[path = "../src/desktop/svg_load.rs"]
mod svg_load;
use std::{hint::black_box, path::Path, sync::atomic::AtomicBool, time::Instant};

fn main() {
    let workers: usize = std::env::args()
        .nth(1)
        .unwrap_or("8".into())
        .parse()
        .unwrap();
    assert!([1, 8].contains(&workers));
    let folder = Path::new("target/evidence/plots/svg-preview");
    std::fs::create_dir_all(folder).unwrap();
    let mut engine = matrust::Engine::default();
    for (name, script) in [
        (
            "plot",
            "t=linspace(0,1,2000);plot(t,sin(2*pi*5*t),t,cos(2*pi*9*t));title('Reopened SVG export');xlabel('Time (s)');ylabel('Amplitude');legend('5 Hz','9 Hz');",
        ),
        (
            "layout",
            "clf;tiledlayout(2,1);nexttile;plot(t,sin(2*pi*5*t));title('Continuous signal');nexttile;stem([1 3 2 4]);title('Discrete samples');sgtitle('SVG layout preview');",
        ),
        (
            "dense",
            "clf;t=linspace(0,1,100000);plot(t,sin(2*pi*100*t));title('100000 original samples');",
        ),
    ] {
        engine.execute(script).unwrap();
        let figure = engine.figure.as_ref().unwrap();
        std::fs::write(folder.join(format!("{name}.svg")), figure.svg()).unwrap();
        matrust::export::export_figure(
            figure,
            &folder.join(format!("{name}-reference.png")),
            960,
            560,
        )
        .unwrap();
    }
    // Font initialization above is deliberately excluded from warmed timings.
    println!("file,workers,file_bytes,preview_pixels,repetitions,median_ms,min_ms,max_ms");
    rayon::ThreadPoolBuilder::new()
        .num_threads(workers)
        .build()
        .unwrap()
        .install(|| {
            for name in ["plot", "layout", "dense"] {
                let path = folder.join(format!("{name}.svg"));
                let cancel = AtomicBool::new(false);
                let preview = svg_load::load(&path, 4096, &cancel).unwrap();
                let expected =
                    image_load::load(&folder.join(format!("{name}-reference.png")), 4096, &cancel)
                        .unwrap();
                assert_eq!(preview.pixels, expected.pixels);
                let rgba: Vec<u8> = preview
                    .pixels
                    .pixels
                    .iter()
                    .flat_map(|p| p.to_srgba_unmultiplied())
                    .collect();
                image::RgbaImage::from_raw(960, 560, rgba)
                    .unwrap()
                    .save(folder.join(format!("{name}-preview.png")))
                    .unwrap();
                let bytes = preview.file_bytes;
                let pixels = preview.pixels.pixels.len();
                drop((preview, expected));
                let mut times = Vec::new();
                for iteration in 0..14 {
                    let start = Instant::now();
                    drop(black_box(
                        svg_load::load(black_box(&path), 4096, &cancel).unwrap(),
                    ));
                    if iteration >= 3 {
                        times.push(start.elapsed().as_secs_f64() * 1000.);
                    }
                }
                times.sort_by(f64::total_cmp);
                println!(
                    "{name},{workers},{bytes},{pixels},11,{:.6},{:.6},{:.6}",
                    times[5], times[0], times[10]
                );
            }
        });
}
