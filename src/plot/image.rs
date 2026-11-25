//! Image objects (`image`, `imagesc`, `imshow`): a pixel grid drawn under the
//! series of an axes. Colour data stays raw so later `colormap`/`clim` calls
//! recolour it at draw time, as MATLAB does.
//!
//! Like the other axes extras, images are process-local: native `.fig` and
//! session files do not store them yet.
use super::colormap;
use crate::{NumericClass, ObjectIdentity, Result};
use std::sync::Arc;

/// Pixels one axes may hold across its images (a 4096 x 4096 picture).
pub const MAX_IMAGE_PIXELS: usize = 4096 * 4096;
pub const MAX_IMAGES: usize = 64;

/// MATLAB `CDataMapping` plus the truecolor case.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum ImageMapping {
    /// Values are colormap rows (1-based for doubles, 0-based for integers).
    Direct { zero_based: bool },
    /// Values scale through CLim onto the colormap.
    Scaled,
    /// Rows x cols x 3 RGB in 0..1 (doubles) or 0..255 / 0..65535 (integers).
    TrueColor { full_scale: f64 },
}

/// Stored class of `ImageObject::data`, returned by `get(h, 'CData')` and `h.CData`.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum ImageDataClass {
    #[default]
    Double,
    Logical,
    Typed(NumericClass),
}

#[derive(Clone, Debug)]
pub struct ImageObject {
    pub identity: ObjectIdentity,
    pub rows: usize,
    pub cols: usize,
    /// Column-major `rows * cols` values, times three planes for truecolor.
    pub data: Arc<[f64]>,
    /// Original array class of CData.
    pub data_class: ImageDataClass,
    pub mapping: ImageMapping,
    /// The CData class's index base, retained while using scaled mapping.
    pub(crate) direct_zero_based: bool,
    /// Centres of the first and last column (XData) and row (YData).
    pub x: [f64; 2],
    pub y: [f64; 2],
    pub visible: bool,
}

/// Zero-based colormap row index for direct `CDataMapping` (`zero_based` for
/// integer/logical classes, 1-based for single/double), clamped to `[0, colors - 1]`.
pub fn direct_colormap_index(value: f64, zero_based: bool, colors: usize) -> usize {
    if colors == 0 || value.is_nan() {
        return 0;
    }
    let max = (colors - 1) as f64;
    (if zero_based { value } else { value - 1. })
        .floor()
        .clamp(0., max) as usize
}

impl ImageObject {
    pub fn new(rows: usize, cols: usize, data: Arc<[f64]>, mapping: ImageMapping) -> Self {
        Self {
            identity: ObjectIdentity::default(),
            rows,
            cols,
            data,
            data_class: ImageDataClass::Double,
            mapping,
            direct_zero_based: matches!(mapping, ImageMapping::Direct { zero_based: true }),
            x: [1., cols as f64],
            y: [1., rows as f64],
            visible: true,
        }
    }
    pub fn validate(&self) -> Result<()> {
        let planes = if matches!(self.mapping, ImageMapping::TrueColor { .. }) {
            3
        } else {
            1
        };
        let pixels = self
            .rows
            .checked_mul(self.cols)
            .ok_or("Image is too large")?;
        if pixels > MAX_IMAGE_PIXELS {
            return Err(format!("An image holds at most {MAX_IMAGE_PIXELS} pixels"));
        }
        if self.data.len() != pixels * planes {
            return Err("Image data does not match its size".into());
        }
        if !self.x.iter().chain(&self.y).all(|v| v.is_finite()) {
            return Err("Image XData and YData must be finite".into());
        }
        Ok(())
    }
    /// Pixel width and height in data units (one pixel for a single column/row).
    fn steps(&self) -> [f64; 2] {
        let step = |range: [f64; 2], n: usize| {
            if n > 1 {
                (range[1] - range[0]) / (n - 1) as f64
            } else {
                1.
            }
        };
        [step(self.x, self.cols), step(self.y, self.rows)]
    }
    /// Outer edges: XData/YData are pixel centres, so the picture reaches half a
    /// pixel beyond them. Returned as [x0, x1] of column 1 .. column n and
    /// [y0, y1] of row 1 .. row m (either may run backwards).
    pub fn edges(&self) -> ([f64; 2], [f64; 2]) {
        let [dx, dy] = self.steps();
        (
            [self.x[0] - dx / 2., self.x[1] + dx / 2.],
            [self.y[0] - dy / 2., self.y[1] + dy / 2.],
        )
    }
    /// Increasing x and y extents of the picture.
    pub fn extent(&self) -> ([f64; 2], [f64; 2]) {
        let (x, y) = self.edges();
        (
            [x[0].min(x[1]), x[0].max(x[1])],
            [y[0].min(y[1]), y[0].max(y[1])],
        )
    }
    /// The data range of scaled colour values (finite ones), for automatic CLim.
    pub fn value_range(&self) -> Option<[f64; 2]> {
        let mut range = [f64::INFINITY, f64::NEG_INFINITY];
        for &v in self.data.iter().filter(|v| v.is_finite()) {
            range = [range[0].min(v), range[1].max(v)];
        }
        (range[0] <= range[1]).then_some(range)
    }
    /// RGB bytes, row-major from row 1 / column 1, resolved through the axes
    /// colormap and colour limits.
    pub fn rgb(&self, map: &[[f64; 3]], clim: [f64; 2]) -> Vec<u8> {
        let (rows, cols) = (self.rows, self.cols);
        let n = rows * cols;
        let mut out = vec![0u8; n * 3];
        let row_of = |v: f64, zero_based: bool| -> [u8; 3] {
            map.get(direct_colormap_index(v, zero_based, map.len()))
                .copied()
                .unwrap_or([0.; 3])
                .map(|c| (c * 255.).round() as u8)
        };
        for i in 0..rows {
            for j in 0..cols {
                let k = i + j * rows;
                let rgb = match self.mapping {
                    ImageMapping::Scaled => colormap::scaled_rgb(map, clim, self.data[k]),
                    ImageMapping::Direct { zero_based } => row_of(self.data[k], zero_based),
                    ImageMapping::TrueColor { full_scale } => [0, 1, 2].map(|p| {
                        let v = self.data[k + p * n] / full_scale;
                        if v.is_nan() {
                            0
                        } else {
                            (v.clamp(0., 1.) * 255.).round() as u8
                        }
                    }),
                };
                out[(i * cols + j) * 3..][..3].copy_from_slice(&rgb);
            }
        }
        out
    }
}

/// PNG bytes of an 8-bit RGB raster (zlib through flate2, as `imwrite` does).
pub fn encode_png(width: usize, height: usize, rgb: &[u8]) -> Vec<u8> {
    encode_png_impl(width, height, 2, 3, rgb)
}

/// PNG bytes of an 8-bit RGBA raster.
pub fn encode_png_rgba(width: usize, height: usize, rgba: &[[u8; 4]]) -> Vec<u8> {
    let mut flat = Vec::with_capacity(rgba.len() * 4);
    for px in rgba {
        flat.extend_from_slice(px);
    }
    encode_png_impl(width, height, 6, 4, &flat)
}

fn encode_png_impl(
    width: usize,
    height: usize,
    color_type: u8,
    channels: usize,
    pixels: &[u8],
) -> Vec<u8> {
    use std::io::Write;
    let chunk = |out: &mut Vec<u8>, kind: &[u8], data: &[u8]| {
        out.extend_from_slice(&(data.len() as u32).to_be_bytes());
        out.extend_from_slice(kind);
        out.extend_from_slice(data);
        let mut crc = flate2::Crc::new();
        crc.update(kind);
        crc.update(data);
        out.extend_from_slice(&crc.sum().to_be_bytes());
    };
    let mut out = b"\x89PNG\r\n\x1a\n".to_vec();
    let mut header = Vec::with_capacity(13);
    header.extend_from_slice(&(width as u32).to_be_bytes());
    header.extend_from_slice(&(height as u32).to_be_bytes());
    header.extend_from_slice(&[8, color_type, 0, 0, 0]);
    chunk(&mut out, b"IHDR", &header);
    let stride = width * channels;
    let mut raw = Vec::with_capacity(height * (stride + 1));
    for row in pixels.chunks(stride).take(height) {
        raw.push(0);
        raw.extend_from_slice(row);
    }
    let mut z = flate2::write::ZlibEncoder::new(Vec::new(), flate2::Compression::fast());
    let _ = z.write_all(&raw);
    let idat = z.finish().unwrap_or_default();
    chunk(&mut out, b"IDAT", &idat);
    chunk(&mut out, b"IEND", &[]);
    out
}

pub fn base64(bytes: &[u8]) -> String {
    const T: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut s = String::with_capacity(bytes.len().div_ceil(3) * 4);
    for c in bytes.chunks(3) {
        let n = (u32::from(c[0]) << 16)
            | (u32::from(*c.get(1).unwrap_or(&0)) << 8)
            | u32::from(*c.get(2).unwrap_or(&0));
        for (i, shift) in [18, 12, 6, 0].into_iter().enumerate() {
            s.push(if i <= c.len() {
                T[(n >> shift & 63) as usize] as char
            } else {
                '='
            });
        }
    }
    s
}

/// `axis equal`/`axis image`: widen the shorter range about its centre so
/// one data unit spans the same length in x and y on a `size` plot box
/// (egui_plot's `data_aspect(1)` does the same on the desktop).
pub(super) fn equal_aspect(
    axes: &super::Axes,
    x: [f64; 2],
    y: [f64; 2],
    [width, height]: [f64; 2],
) -> ([f64; 2], [f64; 2]) {
    let linear = axes.xscale == super::AxisScale::Linear && axes.yscale == super::AxisScale::Linear;
    let (dx, dy) = (x[1] - x[0], y[1] - y[0]);
    if !axes.extras.equal_aspect || !linear || !(dx > 0. && dy > 0. && width > 0. && height > 0.) {
        return (x, y);
    }
    let widen = |r: [f64; 2], span: f64| {
        let c = (r[0] + r[1]) / 2.;
        [c - span / 2., c + span / 2.]
    };
    if dx / width > dy / height {
        (x, widen(y, dx / width * height))
    } else {
        (widen(x, dy / height * width), y)
    }
}

/// Raster for display: rows top to bottom and columns left to right as they
/// appear on screen, given where row 1 and column 1 land.
pub fn screen_raster(
    image: &ImageObject,
    map: &[[f64; 3]],
    clim: [f64; 2],
    flip_rows: bool,
    flip_cols: bool,
) -> Vec<u8> {
    let mut rgb = image.rgb(map, clim);
    let (rows, cols) = (image.rows, image.cols);
    if flip_rows {
        for i in 0..rows / 2 {
            let (a, b) = rgb.split_at_mut((rows - 1 - i) * cols * 3);
            a[i * cols * 3..][..cols * 3].swap_with_slice(&mut b[..cols * 3]);
        }
    }
    if flip_cols {
        for row in rgb.chunks_mut(cols * 3) {
            for j in 0..cols / 2 {
                for c in 0..3 {
                    row.swap(j * 3 + c, (cols - 1 - j) * 3 + c);
                }
            }
        }
    }
    rgb
}

/// Images as embedded PNG `<image>` elements, nearest-neighbour scaled.
pub(super) fn draw_svg(
    svg: &mut impl std::fmt::Write,
    axes: &super::Axes,
    px: &dyn Fn(f64) -> f64,
    py: &dyn Fn(f64) -> f64,
) {
    let extras = &axes.extras;
    if extras.images.is_empty() {
        return;
    }
    let map = extras.effective_colormap();
    let clim = extras.color_limits();
    for image in extras
        .images
        .iter()
        .filter(|i| i.visible && i.rows > 0 && i.cols > 0)
    {
        let (ex, ey) = image.edges();
        let [x0, x1] = ex.map(px);
        let [y0, y1] = ey.map(py);
        if ![x0, x1, y0, y1].iter().all(|v| v.is_finite()) {
            continue;
        }
        let rgb = screen_raster(image, &map, clim, y0 > y1, x0 > x1);
        let png = encode_png(image.cols, image.rows, &rgb);
        let _ = write!(
            svg,
            "<image x=\"{}\" y=\"{}\" width=\"{}\" height=\"{}\" preserveAspectRatio=\"none\" image-rendering=\"optimizeSpeed\" style=\"image-rendering:pixelated\" href=\"data:image/png;base64,{}\"/>",
            x0.min(x1),
            y0.min(y1),
            (x1 - x0).abs(),
            (y1 - y0).abs(),
            base64(&png)
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn base64_matches_rfc4648_vectors() {
        for (input, expected) in [
            ("", ""),
            ("f", "Zg=="),
            ("fo", "Zm8="),
            ("foo", "Zm9v"),
            ("foobar", "Zm9vYmFy"),
        ] {
            assert_eq!(base64(input.as_bytes()), expected);
        }
    }
    #[test]
    fn scaled_image_resolves_through_map_and_limits() {
        let image = ImageObject::new(1, 2, Arc::from([0., 1.]), ImageMapping::Scaled);
        let map = [[0., 0., 0.], [1., 1., 1.]];
        assert_eq!(image.rgb(&map, [0., 1.]), vec![0, 0, 0, 255, 255, 255]);
        assert_eq!(image.extent(), ([0.5, 2.5], [0.5, 1.5]));
    }
}
