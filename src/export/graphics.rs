//! Tight whole-figure or selected-axes export. Canonical SVG units are 1/96 inch; the crop
//! includes a fixed two-unit margin. These are MatRust compatibility choices.
use super::{check_cancel, svg_render_options};
use crate::{
    Result,
    files::atomic_write,
    plot::{BaseTheme, Figure, Position},
};
use std::{
    io::{Cursor, Seek, SeekFrom, Write},
    path::Path,
    sync::atomic::AtomicBool,
};

const MAX_BYTES: u64 = 128 * 1024 * 1024;
const MAX_SVG_BYTES: usize = 32 * 1024 * 1024;
const BASE_DPI: f32 = 96.;
#[derive(Clone, Copy, Debug)]
pub struct GraphicsOptions {
    pub resolution: u16,
    /// Vector output only. MATLAB's image-content "none" uses a white background.
    pub transparent: bool,
}
impl Default for GraphicsOptions {
    fn default() -> Self {
        Self {
            resolution: 150,
            transparent: false,
        }
    }
}

// The const limit permits small-buffer tests of the production writer policy.
// Bound requested capacity as well as encoded length; allocator metadata and
// upstream codec scratch are separate from this output buffer.
struct BoundedOutput<const LIMIT: usize>(Cursor<Vec<u8>>);
impl<const LIMIT: usize> Write for BoundedOutput<LIMIT> {
    fn write(&mut self, bytes: &[u8]) -> std::io::Result<usize> {
        let end = self
            .0
            .position()
            .checked_add(bytes.len() as u64)
            .filter(|&end| end <= LIMIT as u64)
            .ok_or_else(|| std::io::Error::other("Export exceeds encoded-output limit"))?;
        let end = usize::try_from(end)
            .map_err(|_| std::io::Error::other("Export output size exceeds addressable memory"))?;
        let buffer = self.0.get_mut();
        if end > buffer.capacity() {
            // Cursor<Vec>::write otherwise chooses its own geometric growth,
            // which can exceed LIMIT despite an accepted encoded length.
            let capacity = end.max(buffer.capacity().saturating_mul(2)).min(LIMIT);
            buffer
                .try_reserve_exact(capacity - buffer.len())
                .map_err(|_| std::io::Error::other("Could not allocate encoded export output"))?;
        }
        self.0.write(bytes)
    }
    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}
impl<const LIMIT: usize> Seek for BoundedOutput<LIMIT> {
    fn seek(&mut self, from: SeekFrom) -> std::io::Result<u64> {
        let old = self.0.position();
        let next = self.0.seek(from)?;
        if next > LIMIT as u64 {
            self.0.set_position(old);
            return Err(std::io::Error::other("Export seek exceeds output limit"));
        }
        Ok(next)
    }
}

pub fn export_graphics_with_cancel(
    figure: &Figure,
    path: &Path,
    format: &str,
    options: GraphicsOptions,
    cancel: &AtomicBool,
) -> Result<()> {
    export_before_publish(figure, path, format, options, cancel, || {})
}

/// Export only one existing axes, retaining its parent-canvas geometry. The
/// independent output bounds are measured from that axes' visible ink and key.
/// No figure-level title/annotation or other axes are included.
pub fn export_axes_graphics_with_cancel(
    figure: &Figure,
    position: Position,
    path: &Path,
    format: &str,
    options: GraphicsOptions,
    cancel: &AtomicBool,
) -> Result<()> {
    export_target_before_publish(
        figure,
        (Some(position), [255; 3], BaseTheme::Light),
        path,
        format,
        options,
        cancel,
        || {},
    )
}

/// Explicit surrounding color for a whole Figure or one existing axes.
/// Existing white-default APIs and GraphicsOptions remain source-compatible.
/// Transparent vector and white raster-none policies take precedence.
pub fn export_graphics_colored_with_cancel(
    figure: &Figure,
    axes: Option<Position>,
    path: &Path,
    format: &str,
    options: GraphicsOptions,
    background: [u8; 3],
    cancel: &AtomicBool,
) -> Result<()> {
    export_target_before_publish(
        figure,
        (axes, background, BaseTheme::Light),
        path,
        format,
        options,
        cancel,
        || {},
    )
}

/// Explicit immutable host context and surrounding color for tight exports.
pub fn export_graphics_in_theme_with_cancel(
    figure: &Figure,
    target: (Option<Position>, [u8; 3]),
    path: &Path,
    format: &str,
    options: GraphicsOptions,
    host: BaseTheme,
    cancel: &AtomicBool,
) -> Result<()> {
    export_target_before_publish(
        figure,
        (target.0, target.1, host),
        path,
        format,
        options,
        cancel,
        || {},
    )
}

// The generated SVG data clips encode the exact data rectangles inside each
// existing nested viewport. Insert resolved axes backgrounds after coverage measurement,
// before painting the existing axes contents. Legend text clips are excluded.
// This operates only on this module's generated SVG, never imported user SVG.
fn add_axes_underlays(
    svg: &mut String,
    colors: &[(String, [u8; 3])],
    cancel: &AtomicBool,
) -> Result<()> {
    let mut inserts = Vec::new();
    {
        let doc = resvg::usvg::roxmltree::Document::parse(svg).map_err(|e| e.to_string())?;
        for node in doc.descendants().filter(|n| n.has_tag_name("clipPath")) {
            check_cancel(cancel)?;
            let id = node.attribute("id").unwrap_or("");
            let is_data = id == "axes"
                || id.strip_prefix("axes").is_some_and(|s| {
                    !s.is_empty()
                        && s.bytes().all(|b| b.is_ascii_digit())
                        && s.parse::<usize>().is_ok_and(|i| i < 16)
                });
            if !is_data {
                continue;
            }
            if inserts.len() == 16 {
                return Err("Too many generated axes backgrounds".into());
            }
            let defs = node
                .parent()
                .filter(|n| n.has_tag_name("defs"))
                .ok_or("Unexpected generated axes clip parent")?;
            if !defs.parent().is_some_and(|n| n.has_tag_name("svg")) {
                return Err("Unexpected generated axes viewport".into());
            }
            let mut elements = node.children().filter(|n| n.is_element());
            let rect = elements
                .next()
                .filter(|n| n.has_tag_name("rect"))
                .ok_or("Missing generated axes data rectangle")?;
            if elements.next().is_some() || rect.attributes().len() != 4 {
                return Err("Unexpected generated axes rectangle schema".into());
            }
            for name in ["x", "y", "width", "height"] {
                let value = rect
                    .attribute(name)
                    .ok_or("Missing data rectangle coordinate")?
                    .parse::<f64>()
                    .map_err(|_| "Invalid data rectangle coordinate")?;
                if !value.is_finite() || (matches!(name, "width" | "height") && value <= 0.) {
                    return Err("Invalid generated axes background bounds".into());
                }
            }
            let raw = &svg[rect.range()];
            if raw.len() > 512 {
                return Err("Generated data rectangle is too large".into());
            }
            let body = raw
                .strip_suffix("/>")
                .ok_or("Unexpected data rectangle closing tag")?;
            let color = colors
                .iter()
                .find(|(key, _)| key == id)
                .map(|(_, color)| *color)
                .ok_or("Generated axes background has no matching model identity")?;
            let fill = if color == [255; 3] {
                "white".to_owned()
            } else {
                format!("#{:02x}{:02x}{:02x}", color[0], color[1], color[2])
            };
            inserts.push((defs.range().end, format!("{body} fill=\"{fill}\"/>")));
        }
    }
    if inserts.is_empty() {
        return Err("Generated export has no axes background geometry".into());
    }
    let extra: usize = inserts.iter().map(|(_, s)| s.len()).sum();
    if svg
        .len()
        .checked_add(extra)
        .is_none_or(|n| n > MAX_SVG_BYTES)
    {
        return Err("Axes backgrounds exceed SVG allocation limit".into());
    }
    svg.try_reserve_exact(extra)
        .map_err(|_| "Could not allocate axes backgrounds")?;
    inserts.sort_by_key(|(at, _)| *at);
    for (at, body) in inserts.into_iter().rev() {
        check_cancel(cancel)?;
        svg.insert_str(at, &body);
    }
    Ok(())
}

fn export_before_publish(
    figure: &Figure,
    path: &Path,
    format: &str,
    options: GraphicsOptions,
    cancel: &AtomicBool,
    before_publish: impl FnOnce(),
) -> Result<()> {
    export_target_before_publish(
        figure,
        (None, [255; 3], BaseTheme::Light),
        path,
        format,
        options,
        cancel,
        before_publish,
    )
}

fn export_target_before_publish(
    figure: &Figure,
    target: (Option<Position>, [u8; 3], BaseTheme),
    path: &Path,
    format: &str,
    options: GraphicsOptions,
    cancel: &AtomicBool,
    before_publish: impl FnOnce(),
) -> Result<()> {
    check_cancel(cancel)?;
    let (axes, background_color, host) = target;
    let appearance = figure.resolve_appearance(host);
    let format = format.to_ascii_lowercase();
    let vector = matches!(format.as_str(), "svg" | "pdf");
    if !matches!(
        format.as_str(),
        "svg" | "pdf" | "png" | "jpg" | "jpeg" | "tif" | "tiff"
    ) || options.resolution == 0
    {
        return Err("Invalid exportgraphics format or zero resolution".into());
    }
    figure.validate()?;
    let series = if let Some(position) = axes {
        figure
            .all_axes()
            .find(|(p, _)| *p == position)
            .ok_or("exportgraphics target axes does not exist")?
            .1
            .series
            .len()
    } else {
        figure.series_count()
    };
    if series == 0 {
        return Err("exportgraphics requires a figure containing plot data; empty figures/axes are unsupported".into());
    }
    let mut svg = if let Some(position) = axes {
        figure.svg_axes_without_background_in_theme(position, host, cancel)?
    } else {
        figure.svg_without_background_in_theme(host, cancel)?
    };
    if svg.len() > MAX_SVG_BYTES {
        return Err("Export SVG exceeds 32 MiB limit".into());
    }
    check_cancel(cancel)?;
    let tree =
        resvg::usvg::Tree::from_str(&svg, &svg_render_options()).map_err(|e| e.to_string())?;
    check_cancel(cancel)?;
    // A fixed-size coverage pass measures visible ink, including white strokes
    // and resolved glyphs, while respecting every axes clip and the figure edge.
    // Stroke/group bounds alone include offscreen outliers and are not a crop.
    let mut coverage =
        resvg::tiny_skia::Pixmap::new(960, 560).ok_or("Could not allocate crop coverage")?;
    resvg::render(
        &tree,
        resvg::tiny_skia::Transform::identity(),
        &mut coverage.as_mut(),
    );
    check_cancel(cancel)?;
    let mut bounds = None::<[u32; 4]>;
    for (y, row) in coverage
        .data()
        .as_chunks::<{ 960 * 4 }>()
        .0
        .iter()
        .enumerate()
    {
        check_cancel(cancel)?;
        for (x, pixel) in row.as_chunks::<4>().0.iter().enumerate() {
            if pixel[3] != 0 {
                let (x, y) = (x as u32, y as u32);
                let b = bounds.get_or_insert([x, y, x + 1, y + 1]);
                b[0] = b[0].min(x);
                b[1] = b[1].min(y);
                b[2] = b[2].max(x + 1);
                b[3] = b[3].max(y + 1);
            }
        }
    }
    drop(coverage);
    drop(tree);
    let [left, top, right, bottom] = bounds.ok_or("Figure has no visible export content")?;
    let left = left.saturating_sub(2);
    let top = top.saturating_sub(2);
    let right = (right + 2).min(960);
    let bottom = (bottom + 2).min(560);
    let width = right - left;
    let height = bottom - top;
    // Coverage above deliberately excludes every background. Adding data-area
    // backgrounds and the requested surrounding color cannot enlarge the crop.
    let axes_colors = if figure.layout.is_none() {
        vec![(
            "axes".to_owned(),
            figure.axes.resolve_colors(appearance).background,
        )]
    } else {
        let mut panels: Vec<_> = figure.all_axes().collect();
        panels.sort_by_key(|(position, _)| *position);
        panels
            .into_iter()
            .enumerate()
            .filter(|(_, (position, _))| axes.is_none_or(|selected| selected == *position))
            .map(|(index, (_, panel))| {
                (
                    format!("axes{index}"),
                    panel.resolve_colors(appearance).background,
                )
            })
            .collect()
    };
    if !options.transparent
        && axes_colors
            .iter()
            .any(|(_, color)| *color != background_color)
    {
        add_axes_underlays(&mut svg, &axes_colors, cancel)?;
    }
    let background = if vector && options.transparent {
        String::new()
    } else {
        let color = if options.transparent || background_color == [255; 3] {
            "white".to_owned()
        } else {
            format!(
                "#{:02x}{:02x}{:02x}",
                background_color[0], background_color[1], background_color[2]
            )
        };
        format!(
            "<rect x=\"{left}\" y=\"{top}\" width=\"{width}\" height=\"{height}\" fill=\"{color}\"/>"
        )
    };
    let header = format!(
        "<svg xmlns=\"http://www.w3.org/2000/svg\" width=\"{width}\" height=\"{height}\" viewBox=\"{left} {top} {width} {height}\">{background}"
    );
    let end = svg.find('>').ok_or("Invalid generated SVG root")?;
    let final_size = svg.len() - (end + 1) + header.len();
    if final_size > MAX_SVG_BYTES {
        return Err("Cropped SVG exceeds 32 MiB limit".into());
    }
    if final_size > svg.len() {
        svg.try_reserve_exact(final_size - svg.len())
            .map_err(|_| "Could not allocate cropped SVG")?;
    }
    svg.replace_range(..=end, &header);
    if svg.len() > MAX_SVG_BYTES {
        return Err("Cropped SVG exceeds 32 MiB limit".into());
    }
    check_cancel(cancel)?;
    let mut output = BoundedOutput::<{ MAX_BYTES as usize }>(Cursor::new(Vec::new()));
    if format == "svg" {
        output.0 = Cursor::new(svg.into_bytes());
    } else {
        let tree =
            resvg::usvg::Tree::from_str(&svg, &svg_render_options()).map_err(|e| e.to_string())?;
        drop(svg);
        check_cancel(cancel)?;
        if format == "pdf" {
            let bytes = svg2pdf::to_pdf(
                &tree,
                Default::default(),
                svg2pdf::PageOptions { dpi: BASE_DPI },
            )
            .map_err(|e| e.to_string())?;
            if bytes.len() as u64 > MAX_BYTES {
                return Err("PDF exceeds 128 MiB output limit".into());
            }
            output.0 = Cursor::new(bytes);
        } else {
            let scale = f32::from(options.resolution) / BASE_DPI;
            let pixel_width = (width * u32::from(options.resolution)).div_ceil(96);
            let pixel_height = (height * u32::from(options.resolution)).div_ceil(96);
            if pixel_width == 0
                || pixel_height == 0
                || pixel_width > 8192
                || pixel_height > 8192
                || u64::from(pixel_width) * u64::from(pixel_height) > 32_000_000
            {
                return Err("Resolution produces an export exceeding 8192 pixels per side or 32 million pixels".into());
            }
            let mut pixmap = resvg::tiny_skia::Pixmap::new(pixel_width, pixel_height)
                .ok_or("Could not allocate export image")?;
            let color = if options.transparent {
                [255; 3]
            } else {
                background_color
            };
            pixmap.fill(resvg::tiny_skia::Color::from_rgba8(
                color[0], color[1], color[2], 255,
            ));
            resvg::render(
                &tree,
                resvg::tiny_skia::Transform::from_scale(scale, scale),
                &mut pixmap.as_mut(),
            );
            check_cancel(cancel)?;
            let mut rgb = pixmap.take();
            let pixels = pixel_width as usize * pixel_height as usize;
            // Compact opaque RGBA to RGB in place; keep one output-sized buffer.
            for pixel in 0..pixels {
                if pixel.is_multiple_of(4096) {
                    check_cancel(cancel)?;
                }
                rgb.copy_within(pixel * 4..pixel * 4 + 3, pixel * 3);
            }
            rgb.truncate(pixels * 3);
            drop(tree);
            match format.as_str() {
                "png" => {
                    let mut encoder = png::Encoder::new(&mut output, pixel_width, pixel_height);
                    encoder.set_color(png::ColorType::Rgb);
                    encoder.set_depth(png::BitDepth::Eight);
                    let ppm = (f64::from(options.resolution) / 0.0254).round() as u32;
                    encoder.set_pixel_dims(Some(png::PixelDimensions {
                        xppu: ppm,
                        yppu: ppm,
                        unit: png::Unit::Meter,
                    }));
                    let mut writer = encoder.write_header().map_err(|e| e.to_string())?;
                    writer.write_image_data(&rgb).map_err(|e| e.to_string())?;
                    writer.finish().map_err(|e| e.to_string())?;
                }
                "jpg" | "jpeg" => {
                    let mut encoder = image::codecs::jpeg::JpegEncoder::new(&mut output);
                    encoder.set_pixel_density(image::codecs::jpeg::PixelDensity::dpi(
                        options.resolution,
                    ));
                    encoder
                        .encode(
                            &rgb,
                            pixel_width,
                            pixel_height,
                            image::ExtendedColorType::Rgb8,
                        )
                        .map_err(|e| e.to_string())?;
                }
                _ => {
                    use tiff::{
                        encoder::{
                            Compression, DeflateLevel, Rational, TiffEncoder, colortype::RGB8,
                        },
                        tags::Tag,
                    };
                    let mut encoder = TiffEncoder::new(&mut output)
                        .map_err(|e| e.to_string())?
                        .with_compression(Compression::Deflate(DeflateLevel::Fast));
                    let mut image = encoder
                        .new_image::<RGB8>(pixel_width, pixel_height)
                        .map_err(|e| e.to_string())?;
                    image
                        .encoder()
                        .write_tag(Tag::ResolutionUnit, 2u16)
                        .map_err(|e| e.to_string())?;
                    image
                        .encoder()
                        .write_tag(
                            Tag::XResolution,
                            Rational {
                                n: u32::from(options.resolution),
                                d: 1,
                            },
                        )
                        .map_err(|e| e.to_string())?;
                    image
                        .encoder()
                        .write_tag(
                            Tag::YResolution,
                            Rational {
                                n: u32::from(options.resolution),
                                d: 1,
                            },
                        )
                        .map_err(|e| e.to_string())?;
                    image.write_data(&rgb).map_err(|e| e.to_string())?;
                }
            }
        }
    }
    before_publish();
    check_cancel(cancel)?;
    // Existing atomic writer is noncancellable once publication begins.
    atomic_write(path, output.0.get_ref())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::Ordering;
    #[test]
    fn encoded_output_growth_caps_requested_capacity_before_cursor_write() {
        let mut bytes = Vec::with_capacity(70);
        bytes.extend_from_slice(&[7; 70]);
        let mut output = BoundedOutput::<128>(Cursor::new(bytes));
        output.seek(SeekFrom::End(0)).unwrap();
        output.write_all(&[8]).unwrap();
        assert_eq!(output.0.position(), 71);
        assert_eq!(&output.0.get_ref()[..70], &[7; 70]);
        assert_eq!(output.0.get_ref()[70], 8);
        assert!(output.0.get_ref().capacity() <= 128);
        output.seek(SeekFrom::Start(96)).unwrap();
        output.write_all(&[9, 10]).unwrap();
        assert_eq!(output.0.position(), 98);
        assert_eq!(&output.0.get_ref()[71..96], &[0; 25]);
        assert_eq!(&output.0.get_ref()[96..], &[9, 10]);
        output.seek(SeekFrom::Start(4)).unwrap();
        output.write_all(&[11, 12]).unwrap();
        assert_eq!(output.0.position(), 6);
        assert_eq!(output.0.get_ref().len(), 98);
        assert_eq!(&output.0.get_ref()[4..6], &[11, 12]);
        assert!(output.0.get_ref().capacity() <= 128);
    }

    #[test]
    fn encoded_output_rejected_write_and_seek_preserve_contents_and_position() {
        let mut output = BoundedOutput::<128>(Cursor::new(vec![3; 70]));
        output.seek(SeekFrom::Start(127)).unwrap();
        let bytes = output.0.get_ref().clone();
        let capacity = output.0.get_ref().capacity();
        assert!(output.write_all(&[1, 2]).is_err());
        assert_eq!(output.0.get_ref(), &bytes);
        assert_eq!(output.0.get_ref().capacity(), capacity);
        assert_eq!(output.0.position(), 127);
        for seek in [
            SeekFrom::Start(129),
            SeekFrom::Current(i64::MAX),
            SeekFrom::Start(u64::MAX),
        ] {
            assert!(output.seek(seek).is_err());
            assert_eq!(output.0.position(), 127);
            assert_eq!(output.0.get_ref(), &bytes);
            assert_eq!(output.0.get_ref().capacity(), capacity);
        }
        output.seek(SeekFrom::Start(128)).unwrap();
        assert!(output.write_all(&[1]).is_err());
        assert_eq!(output.0.position(), 128);
        assert_eq!(output.0.get_ref(), &bytes);
    }

    #[test]
    fn cancelled_after_encoding_retains_existing_destination() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("keep.png");
        std::fs::write(&path, b"original").unwrap();
        let vector = crate::Matrix::from_column_slice(1, 2, &[0., 1.]).unwrap();
        let figure = Figure::new(&vector, &vector).unwrap();
        let cancel = AtomicBool::new(false);
        assert!(
            export_before_publish(
                &figure,
                &path,
                "png",
                GraphicsOptions::default(),
                &cancel,
                || cancel.store(true, Ordering::Relaxed)
            )
            .is_err()
        );
        assert_eq!(std::fs::read(&path).unwrap(), b"original");
        assert_eq!(std::fs::read_dir(dir.path()).unwrap().count(), 1);
    }
    #[test]
    fn colored_export_cancelled_after_encoding_keeps_destination() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("keep.png");
        std::fs::write(&path, b"original").unwrap();
        let v = crate::Matrix::from_column_slice(1, 2, &[0., 1.]).unwrap();
        let figure = Figure::new(&v, &v).unwrap();
        let cancel = AtomicBool::new(false);
        assert!(
            export_target_before_publish(
                &figure,
                (None, [51, 102, 153], BaseTheme::Light),
                &path,
                "png",
                GraphicsOptions::default(),
                &cancel,
                || cancel.store(true, Ordering::Relaxed),
            )
            .is_err()
        );
        assert_eq!(std::fs::read(&path).unwrap(), b"original");
        assert_eq!(std::fs::read_dir(dir.path()).unwrap().count(), 1);
    }

    #[test]
    fn axes_cancelled_after_encoding_retains_existing_destination() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("keep.svg");
        std::fs::write(&path, b"original").unwrap();
        let v = crate::Matrix::from_column_slice(1, 2, &[0., 1.]).unwrap();
        let figure = Figure::new(&v, &v).unwrap();
        let cancel = AtomicBool::new(false);
        assert!(
            export_target_before_publish(
                &figure,
                (Some(figure.active_position()), [255; 3], BaseTheme::Light),
                &path,
                "svg",
                GraphicsOptions::default(),
                &cancel,
                || cancel.store(true, Ordering::Relaxed),
            )
            .is_err()
        );
        assert_eq!(std::fs::read(&path).unwrap(), b"original");
        assert_eq!(std::fs::read_dir(dir.path()).unwrap().count(), 1);
    }
}
