//! Bounded exports of the prepared preview, independent of numerical workspaces.
use super::image_load::check;
use eframe::egui::ColorImage;
use image::ImageEncoder;
use matrust::Result;
use std::{
    io::{self, Cursor, Seek, SeekFrom, Write},
    path::{Path, PathBuf},
    sync::{Arc, atomic::AtomicBool},
};
const MAX_ENCODED: usize = 64 * 1024 * 1024;
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Format {
    #[default]
    Png,
    Jpeg,
    Bmp,
    Tiff,
}
impl Format {
    pub const ALL: [Self; 4] = [Self::Png, Self::Jpeg, Self::Bmp, Self::Tiff];
    pub fn label(self) -> &'static str {
        match self {
            Self::Png => "PNG",
            Self::Jpeg => "JPEG",
            Self::Bmp => "BMP",
            Self::Tiff => "TIFF",
        }
    }
    pub fn extensions(self) -> &'static [&'static str] {
        match self {
            Self::Png => &["png"],
            Self::Jpeg => &["jpg", "jpeg"],
            Self::Bmp => &["bmp"],
            Self::Tiff => &["tiff", "tif"],
        }
    }
    pub fn path(self, mut path: PathBuf) -> Result<PathBuf> {
        if path.extension().is_none() {
            path.set_extension(self.extensions()[0]);
        }
        if !path
            .extension()
            .and_then(|s| s.to_str())
            .is_some_and(|s| self.extensions().iter().any(|e| s.eq_ignore_ascii_case(e)))
        {
            return Err(format!(
                "Choose a .{} file for {} output",
                self.extensions()[0],
                self.label()
            ));
        }
        Ok(path)
    }
    fn from_path(path: &Path) -> Result<Self> {
        Self::ALL
            .into_iter()
            .find(|format| format.path(path.to_owned()).is_ok() && path.extension().is_some())
            .ok_or_else(|| "Choose PNG, JPEG, BMP or TIFF".into())
    }
}
pub struct Request {
    pub path: PathBuf,
    pub source: PathBuf,
    pub pixels: Arc<ColorImage>,
}
fn validate(image: &ColorImage) -> Result<()> {
    let [w, h] = image.size;
    if w == 0 || h == 0 || w > 4096 || h > 4096 || w * h > 8_000_000 || w * h != image.pixels.len()
    {
        return Err("Invalid preview dimensions or pixel buffer".into());
    }
    Ok(())
}
pub(super) fn different_destination(source: &Path, destination: &Path) -> Result<()> {
    let same = |a: &Path, b: &Path| {
        a == b
            || (cfg!(windows)
                && a.as_os_str()
                    .as_encoded_bytes()
                    .eq_ignore_ascii_case(b.as_os_str().as_encoded_bytes()))
    };
    let source_path = std::path::absolute(source).map_err(|e| e.to_string())?;
    let destination_path = std::path::absolute(destination).map_err(|e| e.to_string())?;
    if same(&source_path, &destination_path)
        || matches!((source.canonicalize(),destination.canonicalize()),(Ok(a),Ok(b)) if same(&a,&b))
    {
        return Err(
            "Save the preview to a different file to preserve the original document".into(),
        );
    }
    Ok(())
}
struct Output<'a> {
    cursor: Cursor<Vec<u8>>,
    cancel: &'a AtomicBool,
}
impl Write for Output<'_> {
    fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
        check(self.cancel).map_err(io::Error::other)?;
        let needed = self
            .cursor
            .position()
            .checked_add(bytes.len() as u64)
            .filter(|&n| n <= MAX_ENCODED as u64)
            .ok_or_else(|| io::Error::other("Encoded preview exceeds 64 MiB"))?
            as usize;
        let buffer = self.cursor.get_mut();
        if needed > buffer.capacity() {
            buffer.reserve_exact(needed.next_power_of_two().min(MAX_ENCODED) - buffer.len());
        }
        self.cursor.write(bytes)
    }
    fn flush(&mut self) -> io::Result<()> {
        check(self.cancel).map_err(io::Error::other)
    }
}
impl Seek for Output<'_> {
    fn seek(&mut self, from: SeekFrom) -> io::Result<u64> {
        check(self.cancel).map_err(io::Error::other)?;
        let next = match from {
            SeekFrom::Start(n) => i128::from(n),
            SeekFrom::Current(n) => i128::from(self.cursor.position()) + i128::from(n),
            SeekFrom::End(n) => self.cursor.get_ref().len() as i128 + i128::from(n),
        };
        if !(0..=MAX_ENCODED as i128).contains(&next) {
            return Err(io::Error::other("Invalid preview output seek"));
        }
        self.cursor.set_position(next as u64);
        Ok(next as u64)
    }
}
pub fn save(request: &Request, cancel: &AtomicBool) -> Result<()> {
    let bytes = encode(request, cancel)?;
    matrust::files::atomic_write(&request.path, &bytes)
}
pub(super) fn encode(request: &Request, cancel: &AtomicBool) -> Result<Vec<u8>> {
    check(cancel)?;
    validate(&request.pixels)?;
    let format = Format::from_path(&request.path)?;
    different_destination(&request.source, &request.path)?;
    let opaque = matches!(format, Format::Jpeg | Format::Bmp);
    let channels = if opaque { 3 } else { 4 };
    let mut bytes = Vec::with_capacity(request.pixels.pixels.len() * channels);
    for chunk in request.pixels.pixels.chunks(8192) {
        check(cancel)?;
        for pixel in chunk {
            if opaque {
                let white = 255 - pixel.a();
                bytes.extend_from_slice(&[
                    pixel.r().saturating_add(white),
                    pixel.g().saturating_add(white),
                    pixel.b().saturating_add(white),
                ]);
            } else {
                bytes.extend_from_slice(&pixel.to_srgba_unmultiplied());
            }
        }
    }
    let [width, height] = request.pixels.size.map(|v| v as u32);
    let mut output = Output {
        cursor: Cursor::new(Vec::new()),
        cancel,
    };
    let encoded = match format {
        Format::Png => image::codecs::png::PngEncoder::new_with_quality(
            &mut output,
            image::codecs::png::CompressionType::Fast,
            image::codecs::png::FilterType::Adaptive,
        )
        .write_image(&bytes, width, height, image::ExtendedColorType::Rgba8),
        Format::Jpeg => image::codecs::jpeg::JpegEncoder::new_with_quality(&mut output, 90)
            .write_image(&bytes, width, height, image::ExtendedColorType::Rgb8),
        Format::Bmp => image::codecs::bmp::BmpEncoder::new(&mut output).write_image(
            &bytes,
            width,
            height,
            image::ExtendedColorType::Rgb8,
        ),
        Format::Tiff => image::codecs::tiff::TiffEncoder::new(&mut output).write_image(
            &bytes,
            width,
            height,
            image::ExtendedColorType::Rgba8,
        ),
    };
    encoded.map_err(|e| e.to_string())?;
    drop(bytes);
    check(cancel)?;
    different_destination(&request.source, &request.path)?;
    Ok(output.cursor.into_inner())
}
pub fn clipboard(image: &ColorImage, cancel: &AtomicBool) -> Result<ColorImage> {
    check(cancel)?;
    validate(image)?;
    let mut pixels = Vec::with_capacity(image.pixels.len());
    for chunk in image.pixels.chunks(8192) {
        check(cancel)?;
        // The native clipboard integration transports Color32 bytes directly.
        // Flatten to the viewer's white background so clipboard consumers receive
        // unambiguous opaque RGB, independent of their alpha interpretation.
        pixels.extend(chunk.iter().map(|pixel| {
            let white = 255 - pixel.a();
            eframe::egui::Color32::from_rgb(
                pixel.r().saturating_add(white),
                pixel.g().saturating_add(white),
                pixel.b().saturating_add(white),
            )
        }));
    }
    check(cancel)?;
    Ok(ColorImage {
        size: image.size,
        source_size: image.source_size,
        pixels,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use eframe::egui::Color32;
    fn fixture() -> Arc<ColorImage> {
        let mut pixels = Vec::new();
        for _y in 0..64 {
            for x in 0..128 {
                pixels.push(if x < 64 {
                    Color32::from_rgba_unmultiplied(255, 0, 0, 128)
                } else {
                    Color32::TRANSPARENT
                });
            }
        }
        Arc::new(ColorImage::new([128, 64], pixels))
    }
    #[test]
    fn preview_export_formats_preserve_alpha_or_composite_on_white() {
        let dir = tempfile::tempdir().unwrap();
        let source = dir.path().join("original.tiff");
        std::fs::write(&source, b"preserve original").unwrap();
        let pixels = fixture();
        for format in Format::ALL {
            let path = dir
                .path()
                .join(format!("preview.{}", format.extensions()[0]));
            save(
                &Request {
                    path: path.clone(),
                    source: source.clone(),
                    pixels: pixels.clone(),
                },
                &AtomicBool::new(false),
            )
            .unwrap();
            let decoded = image::open(&path).unwrap().into_rgba8();
            assert_eq!(decoded.dimensions(), (128, 64));
            let opaque = matches!(format, Format::Jpeg | Format::Bmp);
            for (x, expected) in [
                (
                    24,
                    if opaque {
                        [255, 127, 127, 255]
                    } else {
                        [255, 0, 0, 128]
                    },
                ),
                (
                    96,
                    if opaque {
                        [255, 255, 255, 255]
                    } else {
                        [0, 0, 0, 0]
                    },
                ),
            ] {
                for (actual, want) in decoded.get_pixel(x, 32).0.into_iter().zip(expected) {
                    assert!(
                        actual.abs_diff(want) <= if format == Format::Jpeg { 3 } else { 0 },
                        "{format:?}: {actual} != {want}"
                    );
                }
            }
        }
        assert_eq!(std::fs::read(source).unwrap(), b"preserve original");
        assert_eq!(pixels.pixels[0].a(), 128);
    }
    #[test]
    fn preview_exports_reject_original_bad_buffers_and_cancel_before_publication() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("original.png");
        std::fs::write(&path, b"keep").unwrap();
        let pixels = fixture();
        let mut request = Request {
            path: path.clone(),
            source: path.clone(),
            pixels: pixels.clone(),
        };
        assert!(
            save(&request, &AtomicBool::new(false))
                .unwrap_err()
                .contains("different file")
        );
        request.path = dir.path().join("./original.png");
        assert!(save(&request, &AtomicBool::new(false)).is_err());
        request.source = dir.path().join("elsewhere.pdf");
        assert_eq!(
            save(&request, &AtomicBool::new(true)).unwrap_err(),
            "Cancelled"
        );
        request.pixels = Arc::new(ColorImage {
            size: [1, 1],
            source_size: eframe::egui::vec2(1., 1.),
            pixels: vec![],
        });
        assert!(save(&request, &AtomicBool::new(false)).is_err());
        assert_eq!(std::fs::read(&path).unwrap(), b"keep");
        request.pixels = pixels;
        request.path = dir.path().join("missing/out.png");
        assert!(save(&request, &AtomicBool::new(false)).is_err());
        request.path = dir.path().join("out.svg");
        assert!(save(&request, &AtomicBool::new(false)).is_err());
        assert!(!request.path.exists());
        assert!(Format::Png.path(dir.path().join("out.jpg")).is_err());
        assert_eq!(
            Format::Png
                .path(dir.path().join("out"))
                .unwrap()
                .extension()
                .unwrap(),
            "png"
        );
    }
    #[test]
    fn preview_output_bounds_and_cancellation_apply_to_write_seek_and_clipboard() {
        let cancel = AtomicBool::new(false);
        let mut output = Output {
            cursor: Cursor::new(Vec::new()),
            cancel: &cancel,
        };
        assert!(
            output
                .seek(SeekFrom::Start(MAX_ENCODED as u64 + 1))
                .is_err()
        );
        output.seek(SeekFrom::Start(MAX_ENCODED as u64)).unwrap();
        assert!(output.write(&[1]).is_err());
        assert!(output.cursor.get_ref().is_empty());
        assert!(
            output
                .seek(SeekFrom::Current(-(MAX_ENCODED as i64) - 1))
                .is_err()
        );
        output.seek(SeekFrom::Start(0)).unwrap();
        cancel.store(true, std::sync::atomic::Ordering::Relaxed);
        assert!(output.write(&[1]).is_err());
        assert!(output.seek(SeekFrom::Start(0)).is_err());
        let image = fixture();
        assert!(clipboard(&image, &cancel).is_err());
        cancel.store(false, std::sync::atomic::Ordering::Relaxed);
        let copy = clipboard(&image, &cancel).unwrap();
        assert_eq!(copy.size, image.size);
        assert!(copy.pixels.iter().all(|p| p.a() == 255));
        assert_eq!(copy.pixels[0], Color32::from_rgb(255, 127, 127));
        assert_eq!(copy.pixels[96], Color32::WHITE);
        assert_eq!(image.pixels[0].a(), 128);
    }
}
