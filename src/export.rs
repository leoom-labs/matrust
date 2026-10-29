//! Publication export uses the same SVG geometry for raster and vector formats.
use crate::{
    Result,
    files::atomic_write,
    plot::{BaseTheme, Figure},
};
use std::{
    path::Path,
    sync::{
        Arc, OnceLock,
        atomic::{AtomicBool, Ordering},
    },
};

mod eps;
pub use eps::export_eps_with_cancel;

mod graphics;
pub use graphics::{
    GraphicsOptions, export_axes_graphics_with_cancel, export_graphics_colored_with_cancel,
    export_graphics_in_theme_with_cancel, export_graphics_with_cancel,
};

pub const FORMATS: &[&str] = &[
    "png", "jpg", "jpeg", "svg", "pdf", "eps", "bmp", "tif", "tiff",
];
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
/// Explicitly sized vector output. Legacy export_figure keeps its canonical
/// vector canvas; this entry point uses SVG pixels or PDF/EPS points (72 per inch).
/// EPS is opaque color vector output.
pub fn export_vector_with_cancel(
    figure: &Figure,
    path: &Path,
    canvas: crate::plot::Canvas,
    cancel: &AtomicBool,
) -> Result<()> {
    export_vector_in_theme_with_cancel(figure, path, canvas, BaseTheme::Light, cancel)
}
/// Explicit host appearance captured by a native export request.
pub fn export_vector_in_theme_with_cancel(
    figure: &Figure,
    path: &Path,
    canvas: crate::plot::Canvas,
    host: BaseTheme,
    cancel: &AtomicBool,
) -> Result<()> {
    check_cancel(cancel)?;
    let extension = path.extension().and_then(|e| e.to_str()).unwrap_or("");
    if extension.eq_ignore_ascii_case("eps") {
        return eps::export_eps_in_theme_with_cancel(figure, path, canvas, host, cancel);
    }
    let pdf = extension.eq_ignore_ascii_case("pdf");
    if !pdf && !extension.eq_ignore_ascii_case("svg") {
        return Err("Sized vector export requires SVG, PDF or color EPS".into());
    }
    let svg = figure.svg_on_canvas_with_theme(canvas, host)?;
    check_cancel(cancel)?;
    if !pdf {
        return atomic_write(path, svg.as_bytes());
    }
    let tree =
        resvg::usvg::Tree::from_str(&svg, &svg_render_options()).map_err(|e| e.to_string())?;
    check_cancel(cancel)?;
    let bytes = svg2pdf::to_pdf(&tree, Default::default(), svg2pdf::PageOptions { dpi: 72. })
        .map_err(|e| e.to_string())?;
    check_cancel(cancel)?;
    atomic_write(path, &bytes)
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
    export_figure_in_theme_with_cancel(figure, path, [width, height], BaseTheme::Light, cancel)
}
/// Opaque export with an explicit host theme; the figure is borrowed unchanged.
pub fn export_figure_in_theme_with_cancel(
    figure: &Figure,
    path: &Path,
    size: [u32; 2],
    host: BaseTheme,
    cancel: &AtomicBool,
) -> Result<()> {
    let [width, height] = size;
    let ext = path
        .extension()
        .and_then(|s| s.to_str())
        .unwrap_or("")
        .to_lowercase();
    // Preserve the existing GUI/API TIFF encoding; explicit script formats can
    // request compressed TIFF without changing historical output semantics.
    let format = if matches!(ext.as_str(), "tif" | "tiff") {
        "tiffn"
    } else {
        &ext
    };
    export_figure_as_in_theme_with_cancel(figure, path, format, [width, height], host, cancel)
}

/// Encode the requested format independently of the destination extension.
/// `tiff`/`tif` use DEFLATE; `tiffn` preserves the uncompressed TIFF option.
/// Raster dimensions are pixels; vector output retains the canonical canvas.
/// `eps`/`epsc` select color EPS in this Rust API. MATLAB saveas admission
/// separately rejects its monochrome `eps` spelling; use explicit `epsc` there.
pub fn export_figure_as_with_cancel(
    figure: &Figure,
    path: &Path,
    format: &str,
    width: u32,
    height: u32,
    cancel: &AtomicBool,
) -> Result<()> {
    export_figure_as_in_theme_with_cancel(
        figure,
        path,
        format,
        [width, height],
        BaseTheme::Light,
        cancel,
    )
}
/// Format-explicit variant sharing the same immutable appearance resolution.
pub fn export_figure_as_in_theme_with_cancel(
    figure: &Figure,
    path: &Path,
    format: &str,
    size: [u32; 2],
    host: BaseTheme,
    cancel: &AtomicBool,
) -> Result<()> {
    let [width, height] = size;
    let check = || check_cancel(cancel);
    check()?;
    validate_size(width, height)?;
    let ext = format.to_lowercase();
    if matches!(ext.as_str(), "eps" | "epsc") {
        return eps::export_eps_in_theme_with_cancel(
            figure,
            path,
            crate::plot::Canvas::default(),
            host,
            cancel,
        );
    }
    if !FORMATS.contains(&ext.as_str()) && ext != "tiffn" {
        return Err("Choose PNG, JPEG, SVG, PDF, color EPS, BMP or TIFF".into());
    }
    figure.validate()?;
    check()?;
    let svg = figure.svg_with_theme(host);
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
    let rgba = rasterize(
        &tree,
        width,
        height,
        figure.resolve_appearance(host).raster_background,
        cancel,
    )?;
    let rgb = image::DynamicImage::ImageRgba8(rgba).into_rgb8();
    let mut encoded = std::io::Cursor::new(Vec::new());
    if matches!(ext.as_str(), "tif" | "tiff") {
        use tiff::encoder::{Compression, DeflateLevel, TiffEncoder, colortype::RGB8};
        TiffEncoder::new(&mut encoded)
            .map_err(|e| e.to_string())?
            .with_compression(Compression::Deflate(DeflateLevel::Fast))
            .write_image::<RGB8>(width, height, rgb.as_raw())
            .map_err(|e| e.to_string())?;
    } else {
        let format = match ext.as_str() {
            "png" => image::ImageFormat::Png,
            "jpg" | "jpeg" => image::ImageFormat::Jpeg,
            "bmp" => image::ImageFormat::Bmp,
            _ => image::ImageFormat::Tiff,
        };
        rgb.write_to(&mut encoded, format)
            .map_err(|e| e.to_string())?;
    }
    check()?;
    atomic_write(path, encoded.get_ref())
}

/// Render the same opaque, resolved-background pixels used by raster file exports.
/// This avoids encoding, temporary files and decoding for clipboard consumers.
pub fn render_figure_with_cancel(
    figure: &Figure,
    width: u32,
    height: u32,
    cancel: &AtomicBool,
) -> Result<image::RgbaImage> {
    render_figure_in_theme_with_cancel(figure, [width, height], BaseTheme::Light, cancel)
}
/// Clipboard/raster rendering resolves Auto against a captured host theme.
pub fn render_figure_in_theme_with_cancel(
    figure: &Figure,
    size: [u32; 2],
    host: BaseTheme,
    cancel: &AtomicBool,
) -> Result<image::RgbaImage> {
    let [width, height] = size;
    check_cancel(cancel)?;
    validate_size(width, height)?;
    figure.validate()?;
    let svg = figure.svg_with_theme(host);
    check_cancel(cancel)?;
    let tree =
        resvg::usvg::Tree::from_str(&svg, &svg_render_options()).map_err(|e| e.to_string())?;
    check_cancel(cancel)?;
    rasterize(
        &tree,
        width,
        height,
        figure.resolve_appearance(host).raster_background,
        cancel,
    )
}

fn check_cancel(cancel: &AtomicBool) -> Result<()> {
    if cancel.load(Ordering::Relaxed) {
        Err("Cancelled".into())
    } else {
        Ok(())
    }
}

fn rasterize(
    tree: &resvg::usvg::Tree,
    width: u32,
    height: u32,
    background: [u8; 3],
    cancel: &AtomicBool,
) -> Result<image::RgbaImage> {
    let mut pixmap =
        resvg::tiny_skia::Pixmap::new(width, height).ok_or("Could not allocate export image")?;
    pixmap.fill(resvg::tiny_skia::Color::from_rgba8(
        background[0],
        background[1],
        background[2],
        255,
    ));
    let scale = (width as f32 / 960.0).min(height as f32 / 560.0);
    let left = (width as f32 - 960.0 * scale) * 0.5;
    let top = (height as f32 - 560.0 * scale) * 0.5;
    resvg::render(
        tree,
        resvg::tiny_skia::Transform::from_row(scale, 0., 0., scale, left, top),
        &mut pixmap.as_mut(),
    );
    check_cancel(cancel)?;
    image::RgbaImage::from_raw(width, height, pixmap.take()).ok_or("Invalid pixel buffer".into())
}
