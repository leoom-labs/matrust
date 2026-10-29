use matrust::{Engine, files};
use std::sync::atomic::Ordering;

fn engine(path: &std::path::Path) -> Engine {
    let mut e = Engine::default();
    e.set_file_directory(path).unwrap();
    e.execute("plot([0 1 2],[0 1 0]);title('Exported signal');")
        .unwrap();
    e
}

#[test]
fn script_exports_core_formats_explicit_bytes_paths_and_shared_samples() {
    let dir = tempfile::tempdir().unwrap();
    let mut e = engine(dir.path());
    let original = e.figure.clone().unwrap();
    e.execute("saveas(gcf,'native.mfig');").unwrap();
    assert_eq!(e.take_figure_file_events().len(), 1);
    assert!(original.same_snapshot(e.figure.as_ref().unwrap()));
    let native = files::load_figure(&dir.path().join("native.mfig")).unwrap();
    assert_eq!(native.title, original.title);
    e.execute("saveas(gcf,'default');saveas(gcf,'vectors.dat','SVG');")
        .unwrap();
    assert_eq!(e.take_figure_file_events().len(), 1);
    assert_eq!(
        std::fs::read_to_string(dir.path().join("vectors.dat")).unwrap(),
        original.svg()
    );
    e.execute("saveas(gcf,'figure.png','fig');").unwrap();
    assert!(e.take_figure_file_events().is_empty());
    let saved = files::load_matlab_figure(&dir.path().join("figure.png")).unwrap();
    assert_eq!(saved.title, original.title);
    assert_eq!(
        saved.series[0].points.iter().collect::<Vec<_>>().as_slice(),
        original.series[0]
            .points
            .iter()
            .collect::<Vec<_>>()
            .as_slice()
    );
    assert!((original.series[0].points).same_storage(&e.figure.as_ref().unwrap().series[0].points));
    e.execute("figure(2);plot([7 8]);saveas(1,'quoted % '' path','svg');")
        .unwrap();
    assert_eq!(e.current_figure, 2);
    assert_eq!(
        std::fs::read_to_string(dir.path().join("quoted % ' path.svg")).unwrap(),
        original.svg()
    );
}

#[test]
fn script_exports_invalid_outputs_cancel_and_failure_preserve_existing_files() {
    let dir = tempfile::tempdir().unwrap();
    let mut e = engine(dir.path());
    let original = e.figure.clone().unwrap();
    let path = dir.path().join("kept.svg");
    std::fs::write(&path, b"keep").unwrap();
    for script in [
        "saveas()",
        "saveas(gcf)",
        "saveas(gcf,42)",
        "saveas(gcf,'kept.svg',4)",
        "saveas(gcf,'kept.svg','mfig')",
        "saveas(gcf,'kept.svg','eps')",
        "saveas(gcf,'kept.svg','svg','extra')",
        "saveas(999,'kept.svg')",
        "saveas(1.5,'kept.svg')",
        "saveas([1 2],'kept.svg')",
        "saved=saveas(gcf,'kept.svg')",
        "disp(saveas(gcf,'kept.svg'))",
        "saveas(gcf,'https://example.invalid/a.svg')",
        "saveas(gcf,'')",
        "saveas(gcf,'missing/sub/kept.svg')",
    ] {
        assert!(e.execute(script).is_err(), "{script}");
        assert_eq!(std::fs::read(&path).unwrap(), b"keep", "{script}");
        assert!(
            original.same_snapshot(e.figure.as_ref().unwrap()),
            "{script}"
        );
        assert!(e.take_figure_file_events().is_empty());
    }
    e.cancellation().store(true, Ordering::Relaxed);
    assert!(e.execute("saveas(gcf,'kept.svg')").is_err());
    assert_eq!(std::fs::read(path).unwrap(), b"keep");
    let destination = dir.path().join("directory.svg");
    std::fs::create_dir(&destination).unwrap();
    e.cancellation().store(false, Ordering::Relaxed);
    assert!(e.execute("saveas(gcf,'directory.svg')").is_err());
    assert!(destination.is_dir());
    assert!(
        std::fs::read_dir(dir.path()).unwrap().all(|p| !p
            .unwrap()
            .file_name()
            .to_string_lossy()
            .starts_with(".tmp"))
    );
}

#[test]
fn script_exports_function_and_variable_shadowing_and_later_errors() {
    let dir = tempfile::tempdir().unwrap();
    let mut e = engine(dir.path());
    assert!(
        e.execute("saveas(gcf,'completed.svg');unknown_function();")
            .is_err()
    );
    assert!(dir.path().join("completed.svg").is_file());
    assert!(e.take_figure_file_events().is_empty());
    e.execute("saveas=[11 22];value=saveas(2);").unwrap();
    assert_eq!(e.variables["value"].as_scalar().unwrap(), 22.);
    e.execute("clear; function v=saveas(x)\nv=x+4;\nend\nanswer=saveas(3);")
        .unwrap();
    assert_eq!(e.variables["answer"].as_scalar().unwrap(), 7.);
}

#[cfg(feature = "plot-export")]
#[test]
fn script_exports_rasters_decode_with_exact_lossless_pixels_and_compression() {
    let dir = tempfile::tempdir().unwrap();
    let mut e = engine(dir.path());
    e.execute("saveas(gcf,'reference.png');").unwrap();
    let reference = image::open(dir.path().join("reference.png"))
        .unwrap()
        .to_rgb8();
    assert_eq!(reference.dimensions(), (1440, 840));
    for (format, expected) in [
        ("png", image::ImageFormat::Png),
        ("jpeg", image::ImageFormat::Jpeg),
        ("bmp", image::ImageFormat::Bmp),
        ("tiff", image::ImageFormat::Tiff),
        ("tiffn", image::ImageFormat::Tiff),
    ] {
        e.execute(&format!("saveas(gcf,'{format}.data','{format}');"))
            .unwrap();
        let bytes = std::fs::read(dir.path().join(format!("{format}.data"))).unwrap();
        assert_eq!(image::guess_format(&bytes).unwrap(), expected);
        let decoded = image::load_from_memory(&bytes).unwrap().to_rgb8();
        assert_eq!(decoded.dimensions(), reference.dimensions());
        if format != "jpeg" {
            assert_eq!(decoded, reference);
        } else {
            let mean_error = decoded
                .as_raw()
                .iter()
                .zip(reference.as_raw())
                .map(|(&a, &b)| a.abs_diff(b) as u64)
                .sum::<u64>() as f64
                / reference.as_raw().len() as f64;
            assert!(mean_error < 5., "JPEG mean error: {mean_error}");
        }
        if matches!(format, "tiff" | "tiffn") {
            let mut decoder = tiff::decoder::Decoder::new(std::io::Cursor::new(bytes)).unwrap();
            assert_eq!(
                decoder.get_tag_u32(tiff::tags::Tag::Compression).unwrap(),
                if format == "tiff" { 8 } else { 1 }
            );
        }
    }
    assert!(
        std::fs::metadata(dir.path().join("tiff.data"))
            .unwrap()
            .len()
            < std::fs::metadata(dir.path().join("tiffn.data"))
                .unwrap()
                .len()
                / 4
    );
    e.execute("saveas(gcf,'photo','jpeg');saveas(gcf,'compressed','tiff');saveas(gcf,'plain','tiffn');saveas(gcf,'vector.bin','pdf');").unwrap();
    for name in ["photo.jpg", "compressed.tif", "plain.tif"] {
        assert!(dir.path().join(name).is_file());
    }
    assert!(
        std::fs::read(dir.path().join("vector.bin"))
            .unwrap()
            .starts_with(b"%PDF")
    );
    assert!(e.take_figure_file_events().is_empty());
}

#[cfg(not(feature = "plot-export"))]
#[test]
fn script_exports_optional_renderers_fail_without_creating_output() {
    let dir = tempfile::tempdir().unwrap();
    let mut e = engine(dir.path());
    for name in ["plot.png", "plot.pdf", "plot.tif"] {
        assert!(
            e.execute(&format!("saveas(gcf,'{name}')"))
                .unwrap_err()
                .contains("plot-export")
        );
        assert!(!dir.path().join(name).exists());
    }
}

#[cfg(feature = "plot-export")]
#[test]
fn script_exports_explicit_codec_rejects_bad_size_format_and_cancel_before_publication() {
    use matrust::export::export_figure_as_with_cancel;
    use std::sync::atomic::AtomicBool;
    let dir = tempfile::tempdir().unwrap();
    let e = engine(dir.path());
    let figure = e.figure.as_ref().unwrap();
    let path = dir.path().join("kept.dat");
    std::fs::write(&path, b"keep").unwrap();
    for (format, width, height, cancelled) in [
        ("unknown", 1440, 840, false),
        ("png", 0, 840, false),
        ("tiff", 8192, 8192, false),
        ("pdf", 1440, 840, true),
        ("tiff", 1440, 840, true),
    ] {
        assert!(
            export_figure_as_with_cancel(
                figure,
                &path,
                format,
                width,
                height,
                &AtomicBool::new(cancelled)
            )
            .is_err()
        );
        assert_eq!(std::fs::read(&path).unwrap(), b"keep");
    }
}
