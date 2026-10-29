#![cfg(feature = "plot-export")]
use matrust::{Engine, export};
use std::sync::atomic::AtomicBool;

#[test]
fn raster_exports_fit_without_distortion_and_center_white_margins() {
    let dir = tempfile::tempdir().unwrap();
    let mut engine = Engine::default();
    engine
        .execute("plot([0 1 2],[0 1 0]);title('Export size');xlabel('Time');ylabel('Value');")
        .unwrap();
    engine.figure.as_mut().unwrap().set_canvas_color([255; 3]); // Explicit white-margin fixture.
    let figure = engine.figure.as_ref().unwrap();
    let reference = dir.path().join("reference.png");
    export::export_figure(figure, &reference, 960, 560).unwrap();
    let reference = image::open(reference).unwrap().to_rgb8();
    for (width, height, left, top) in [(960, 960, 0, 200), (1200, 560, 120, 0)] {
        let path = dir.path().join(format!("{width}x{height}.png"));
        export::export_figure(figure, &path, width, height).unwrap();
        let result = image::open(path).unwrap().to_rgb8();
        assert_eq!(result.dimensions(), (width, height));
        assert_eq!(
            image::imageops::crop_imm(&result, left, top, 960, 560).to_image(),
            reference
        );
        for (x, y, pixel) in result.enumerate_pixels() {
            if x < left || x >= left + 960 || y < top || y >= top + 560 {
                assert_eq!(pixel.0, [255, 255, 255]);
            }
        }
    }
}

#[test]
fn custom_raster_sizes_decode_in_every_supported_image_format() {
    let dir = tempfile::tempdir().unwrap();
    let mut engine = Engine::default();
    engine.execute("plot([0 1 2],[0 1 0]);").unwrap();
    for extension in ["png", "jpg", "jpeg", "bmp", "tif", "tiff"] {
        let path = dir.path().join(format!("custom.{extension}"));
        export::export_figure(engine.figure.as_ref().unwrap(), &path, 400, 300).unwrap();
        let image = image::open(path).unwrap().to_rgb8();
        assert_eq!(image.dimensions(), (400, 300));
        assert!(
            image
                .pixels()
                .filter(|p| p.0[2] > p.0[0].saturating_add(40))
                .count()
                > 100
        );
    }
}

#[test]
fn rejected_or_cancelled_exports_preserve_destination_and_vector_canvas() {
    let dir = tempfile::tempdir().unwrap();
    let mut engine = Engine::default();
    engine.execute("plot([0 1 2],[0 1 0]);").unwrap();
    let figure = engine.figure.as_ref().unwrap();
    let path = dir.path().join("existing.png");
    std::fs::write(&path, b"keep").unwrap();
    for (w, h) in [
        (0, 560),
        (960, 63),
        (8193, 100),
        (8192, 8192),
        (u32::MAX, u32::MAX),
    ] {
        assert!(export::export_figure(figure, &path, w, h).is_err());
        assert_eq!(std::fs::read(&path).unwrap(), b"keep");
    }
    assert!(
        export::export_figure_with_cancel(figure, &path, 960, 560, &AtomicBool::new(true))
            .unwrap_err()
            .contains("Cancelled")
    );
    assert_eq!(std::fs::read(&path).unwrap(), b"keep");
    let first = dir.path().join("small.svg");
    let second = dir.path().join("large.svg");
    export::export_figure(figure, &first, 960, 560).unwrap();
    export::export_figure(figure, &second, 2880, 1680).unwrap();
    assert_eq!(
        std::fs::read(first).unwrap(),
        std::fs::read(second).unwrap()
    );
    let csv = dir.path().join("existing.csv");
    std::fs::write(&csv, b"keep").unwrap();
    assert!(
        matrust::files::write_figure_csv_with_cancel(&csv, figure, &AtomicBool::new(true)).is_err()
    );
    assert_eq!(std::fs::read(&csv).unwrap(), b"keep");
    matrust::files::write_figure_csv_with_cancel(&csv, figure, &AtomicBool::new(false)).unwrap();
    let comparison = dir.path().join("comparison.csv");
    matrust::files::write_figure_csv(&comparison, figure).unwrap();
    assert_eq!(
        std::fs::read(csv).unwrap(),
        std::fs::read(comparison).unwrap()
    );
}
