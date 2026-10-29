//! Bounded raster decoding on the desktop worker; no numerical-engine state.
use eframe::egui::ColorImage;
use image::{ImageDecoder, ImageFormat, ImageReader};
use matrust::Result;
use std::{
    fs::File,
    io::{Cursor, Read},
    path::{Path, PathBuf},
    sync::atomic::{AtomicBool, Ordering},
};

pub const MAX_FILE_BYTES: usize = 128 * 1024 * 1024;
pub const MAX_PIXELS: u64 = 32_000_000;
pub const MAX_PREVIEW_PIXELS: f64 = 8_000_000.;
pub struct Preview {
    pub path: PathBuf,
    pub original_size: [u32; 2],
    pub format: &'static str,
    pub file_bytes: usize,
    pub pixels: ColorImage,
}
fn check(cancel: &AtomicBool) -> Result<()> {
    if cancel.load(Ordering::Relaxed) {
        Err("Cancelled".into())
    } else {
        Ok(())
    }
}
fn dimensions(width: u32, height: u32, decoded_bytes: u64) -> Result<()> {
    if width == 0
        || height == 0
        || width > 8192
        || height > 8192
        || u64::from(width) * u64::from(height) > MAX_PIXELS
        || decoded_bytes > 256 * 1024 * 1024
    {
        return Err(
            "Image exceeds 8192 pixels per side, 32 million pixels, or 256 MiB decoded data".into(),
        );
    }
    Ok(())
}
pub fn load(path: &Path, texture_side: usize, cancel: &AtomicBool) -> Result<Preview> {
    check(cancel)?;
    let mut file = File::open(path).map_err(|e| format!("{}: {e}", path.display()))?;
    let metadata = file.metadata().map_err(|e| e.to_string())?;
    if !metadata.is_file() || metadata.len() > MAX_FILE_BYTES as u64 {
        return Err("Open a regular image file no larger than 128 MiB".into());
    }
    let mut bytes = Vec::with_capacity(metadata.len() as usize);
    let mut chunk = [0; 64 * 1024];
    loop {
        check(cancel)?;
        let n = file.read(&mut chunk).map_err(|e| e.to_string())?;
        if n == 0 {
            break;
        }
        if bytes.len() + n > MAX_FILE_BYTES {
            return Err("Image file grew beyond 128 MiB".into());
        }
        bytes.extend_from_slice(&chunk[..n]);
    }
    let file_bytes = bytes.len();
    let mut reader = ImageReader::new(Cursor::new(&bytes))
        .with_guessed_format()
        .map_err(|e| e.to_string())?;
    let format = match reader.format() {
        Some(ImageFormat::Png) => "PNG",
        Some(ImageFormat::Jpeg) => "JPEG",
        Some(ImageFormat::Bmp) => "BMP",
        Some(ImageFormat::Tiff) => "TIFF (first page)",
        _ => return Err("Preview supports PNG, JPEG, BMP and TIFF images".into()),
    };
    let mut limits = image::Limits::default();
    limits.max_image_width = Some(8192);
    limits.max_image_height = Some(8192);
    limits.max_alloc = Some(256 * 1024 * 1024);
    reader.limits(limits);
    let mut decoder = reader.into_decoder().map_err(|e| e.to_string())?;
    let (width, height) = decoder.dimensions();
    dimensions(width, height, decoder.total_bytes())?;
    let orientation = decoder.orientation().map_err(|e| e.to_string())?;
    check(cancel)?;
    let mut decoded = image::DynamicImage::from_decoder(decoder).map_err(|e| e.to_string())?;
    drop(bytes);
    check(cancel)?;
    decoded.apply_orientation(orientation);
    let original_size = [decoded.width(), decoded.height()];
    let side = texture_side.clamp(1, 4096) as f64;
    let scale = (side / f64::from(decoded.width()))
        .min(side / f64::from(decoded.height()))
        .min(
            (MAX_PREVIEW_PIXELS / (f64::from(decoded.width()) * f64::from(decoded.height())))
                .sqrt(),
        )
        .min(1.);
    if scale < 1. {
        decoded = decoded.thumbnail(
            (f64::from(decoded.width()) * scale).floor().max(1.) as u32,
            (f64::from(decoded.height()) * scale).floor().max(1.) as u32,
        );
    }
    check(cancel)?;
    let rgba = decoded.into_rgba8();
    let pixels = ColorImage::from_rgba_unmultiplied(
        [rgba.width() as usize, rgba.height() as usize],
        rgba.as_raw(),
    );
    check(cancel)?;
    Ok(Preview {
        path: path.to_owned(),
        original_size,
        format,
        file_bytes,
        pixels,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn raster_preview_decodes_export_formats_and_preserves_alpha() {
        let dir = tempfile::tempdir().unwrap();
        for (ext, format) in [
            ("png", ImageFormat::Png),
            ("jpg", ImageFormat::Jpeg),
            ("bmp", ImageFormat::Bmp),
            ("tiff", ImageFormat::Tiff),
        ] {
            let path = dir.path().join(format!("solid.{ext}"));
            let image = image::RgbImage::from_pixel(64, 32, image::Rgb([12, 80, 200]));
            image.save_with_format(&path, format).unwrap();
            let preview = load(&path, 4096, &AtomicBool::new(false)).unwrap();
            assert_eq!(preview.original_size, [64, 32]);
            assert_eq!(preview.pixels.size, [64, 32]);
            for pixel in &preview.pixels.pixels {
                let actual = pixel.to_srgba_unmultiplied();
                for (a, b) in actual.into_iter().zip([12, 80, 200, 255]) {
                    assert!(a.abs_diff(b) <= if format == ImageFormat::Jpeg { 3 } else { 0 });
                }
            }
            let reduced = load(&path, 32, &AtomicBool::new(false)).unwrap();
            assert_eq!(reduced.original_size, [64, 32]);
            assert_eq!(reduced.pixels.size, [32, 16]);
        }
        let path = dir.path().join("alpha.png");
        image::RgbaImage::from_pixel(4, 2, image::Rgba([255, 0, 0, 128]))
            .save(&path)
            .unwrap();
        assert_eq!(
            load(&path, 2048, &AtomicBool::new(false))
                .unwrap()
                .pixels
                .pixels[0]
                .to_srgba_unmultiplied(),
            [255, 0, 0, 128]
        );
    }
    #[test]
    fn raster_preview_bounds_and_cancellation_reject_before_publication() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("image.png");
        assert!(matches!(load(&path,4096,&AtomicBool::new(true)),Err(e) if e=="Cancelled"));
        std::fs::write(&path, b"invalid image").unwrap();
        assert!(load(&path, 4096, &AtomicBool::new(false)).is_err());
        let file = File::create(&path).unwrap();
        file.set_len(MAX_FILE_BYTES as u64 + 1).unwrap();
        assert!(matches!(load(&path,4096,&AtomicBool::new(false)),Err(e) if e.contains("128 MiB")));
        drop(file);
        image::RgbImage::new(8193, 1).save(&path).unwrap();
        assert!(load(&path, 4096, &AtomicBool::new(false)).is_err());
        assert!(dimensions(8192, 8192, 1).is_err());
        assert!(dimensions(64, 64, 256 * 1024 * 1024 + 1).is_err());
        assert!(dimensions(8000, 4000, 256 * 1024 * 1024).is_ok());
        assert!(dimensions(0, 1, 0).is_err());
    }
    #[test]
    fn raster_preview_applies_exif_orientation_before_reporting_dimensions() {
        use image::ImageEncoder;
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("rotated.jpg");
        let file = File::create(&path).unwrap();
        let mut encoder = image::codecs::jpeg::JpegEncoder::new(file);
        // Little-endian TIFF IFD with one SHORT Orientation=6 (rotate 90 clockwise).
        encoder
            .set_exif_metadata(vec![
                b'I', b'I', 42, 0, 8, 0, 0, 0, 1, 0, 0x12, 1, 3, 0, 1, 0, 0, 0, 6, 0, 0, 0, 0, 0,
                0, 0,
            ])
            .unwrap();
        encoder
            .write_image(&[200; 16 * 8 * 3], 16, 8, image::ExtendedColorType::Rgb8)
            .unwrap();
        let preview = load(&path, 4096, &AtomicBool::new(false)).unwrap();
        assert_eq!(preview.original_size, [8, 16]);
        assert_eq!(preview.pixels.size, [8, 16]);
    }
}
