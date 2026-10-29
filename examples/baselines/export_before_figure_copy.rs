//! Publication export uses the same SVG geometry for raster and vector formats.
use crate::{Result, files::atomic_write, plot::Figure};
use std::{
    path::Path,
    sync::{
        Arc, OnceLock,
        atomic::{AtomicBool, Ordering},
    },
};

pub const FORMATS: &[&str] = &["png", "jpg", "jpeg", "svg", "pdf", "bmp", "tif", "tiff"];
static FONTS: OnceLock<Arc<resvg::usvg::fontdb::Database>> = OnceLock::new();
/// Shared system-font database for native SVG exports and desktop previews.
pub fn svg_render_options() -> resvg::usvg::Options<'static> {
    resvg::usvg::Options {
        fontdb: FONTS
            .get_or_init(|| {
                let mut db = resvg::usvg::fontdb::Database::new();
                db.load_system_fonts();
                Arc::new(db)
            })
            .clone(),
        ..Default::default()
    }
}
pub fn export_figure(figure: &Figure, path: &Path, width: u32, height: u32) -> Result<()> {
    export_figure_with_cancel(figure, path, width, height, &AtomicBool::new(false))
}
pub fn validate_size(width: u32, height: u32) -> Result<()> {
    if width < 64
        || height < 64
        || width > 8192
        || height > 8192
        || u64::from(width) * u64::from(height) > 32_000_000
    {
        return Err(
            "Export size must be 64–8192 pixels per side, at most 32 million pixels".into(),
        );
    }
    Ok(())
}
/// Cancellation is checked between bounded rendering/encoding phases and before
/// atomic publication. Individual codec and rasterizer calls are not interruptible.
pub fn export_figure_with_cancel(
    figure: &Figure,
    path: &Path,
    width: u32,
    height: u32,
    cancel: &AtomicBool,
) -> Result<()> {
    let check = || {
        if cancel.load(Ordering::Relaxed) {
            Err("Cancelled".to_owned())
        } else {
            Ok(())
        }
    };
    check()?;
    validate_size(width, height)?;
    figure.validate()?;
    check()?;
    let ext = path
        .extension()
        .and_then(|s| s.to_str())
        .unwrap_or("")
        .to_lowercase();
    if !FORMATS.contains(&ext.as_str()) {
        return Err("Choose PNG, JPEG, SVG, PDF, BMP or TIFF".into());
    }
    let svg = figure.svg();
    check()?;
    if ext == "svg" {
        return atomic_write(path, svg.as_bytes());
    }
    let options = svg_render_options();
    let tree = resvg::usvg::Tree::from_str(&svg, &options).map_err(|e| e.to_string())?;
    check()?;
    if ext == "pdf" {
        let bytes = svg2pdf::to_pdf(&tree, Default::default(), Default::default())
            .map_err(|e| e.to_string())?;
        check()?;
        return atomic_write(path, &bytes);
    }
    let mut pixmap =
        resvg::tiny_skia::Pixmap::new(width, height).ok_or("Could not allocate export image")?;
    pixmap.fill(resvg::tiny_skia::Color::WHITE);
    let scale = (width as f32 / 960.0).min(height as f32 / 560.0);
    let left = (width as f32 - 960.0 * scale) * 0.5;
    let top = (height as f32 - 560.0 * scale) * 0.5;
    resvg::render(
        &tree,
        resvg::tiny_skia::Transform::from_row(scale, 0., 0., scale, left, top),
        &mut pixmap.as_mut(),
    );
    check()?;
    let rgba =
        image::RgbaImage::from_raw(width, height, pixmap.take()).ok_or("Invalid pixel buffer")?;
    let format = match ext.as_str() {
        "png" => image::ImageFormat::Png,
        "jpg" | "jpeg" => image::ImageFormat::Jpeg,
        "bmp" => image::ImageFormat::Bmp,
        _ => image::ImageFormat::Tiff,
    };
    let mut encoded = std::io::Cursor::new(Vec::new());
    image::DynamicImage::ImageRgba8(rgba)
        .into_rgb8()
        .write_to(&mut encoded, format)
        .map_err(|e| e.to_string())?;
    check()?;
    atomic_write(path, encoded.get_ref())
}
