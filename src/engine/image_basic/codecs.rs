//! JPEG, BMP and TIFF for imread, imwrite and imfinfo (PNG stays in
//! files_more). Pixels go through the `image` crate of the desktop build;
//! imfinfo reads the headers itself (JPEG SOF markers, the BMP info header,
//! the first TIFF IFD), so it works in every build.
#[cfg(feature = "plot-export")]
use super::{Img, Kind, image, numbers, to_value};
#[cfg(feature = "plot-export")]
use crate::NumericClass;
use crate::engine::{Engine, Value};
use crate::{Array, CellArray, Matrix, Result};

/// The formats of this module, by their imformats extension.
pub(in crate::engine) fn handles(format: &str) -> bool {
    matches!(format, "jpg" | "bmp" | "tif")
}

#[cfg_attr(not(feature = "plot-export"), allow(dead_code))]
fn label(format: &str) -> &'static str {
    match format {
        "jpg" => "JPEG",
        "bmp" => "BMP",
        _ => "TIFF",
    }
}

fn empty() -> Value {
    Value::Matrix(Matrix::from_column_vec(0, 0, Vec::new()).expect("empty matrix"))
}
fn num(v: f64) -> Value {
    Value::Matrix(Matrix::scalar(v))
}
fn row(values: &[f64]) -> Result<Value> {
    Ok(Value::Matrix(Matrix::from_column_vec(
        1,
        values.len(),
        values.to_vec(),
    )?))
}
fn text_value(s: impl Into<String>) -> Value {
    Value::Text(s.into())
}

// ------------------------------------------------------------------- imfinfo

fn u16_le(b: &[u8], at: usize) -> Option<u16> {
    Some(u16::from_le_bytes(b.get(at..at + 2)?.try_into().ok()?))
}
fn u32_le(b: &[u8], at: usize) -> Option<u32> {
    Some(u32::from_le_bytes(b.get(at..at + 4)?.try_into().ok()?))
}

/// imfinfo fields after FileSize, as (names, values).
pub(in crate::engine) fn info(
    engine: &Engine,
    bytes: &[u8],
    format: &str,
) -> Result<(Vec<&'static str>, Vec<Value>)> {
    match format {
        "jpg" => jpeg_info(engine, bytes),
        "bmp" => bmp_info(bytes),
        _ => tiff_info(bytes),
    }
}

fn jpeg_info(engine: &Engine, bytes: &[u8]) -> Result<(Vec<&'static str>, Vec<Value>)> {
    const BAD: &str = "JPEG file is truncated or damaged.";
    let mut pos = 2;
    let mut frame = None;
    let mut comments = Vec::new();
    while pos + 4 <= bytes.len() {
        if bytes[pos] != 0xFF {
            return Err(BAD.into());
        }
        let marker = bytes[pos + 1];
        if marker == 0xFF {
            pos += 1;
            continue;
        }
        if marker == 0xD9 || marker == 0xDA {
            break;
        }
        if (0xD0..=0xD7).contains(&marker) || marker == 0x01 {
            pos += 2;
            continue;
        }
        let length = usize::from(u16::from_be_bytes([bytes[pos + 2], bytes[pos + 3]]));
        let body = bytes.get(pos + 4..pos + 2 + length).ok_or(BAD)?;
        match marker {
            0xC0..=0xCF if !matches!(marker, 0xC4 | 0xC8 | 0xCC) && body.len() >= 6 => {
                let precision = body[0];
                let height = u16::from_be_bytes([body[1], body[2]]);
                let width = u16::from_be_bytes([body[3], body[4]]);
                frame = Some((marker, precision, height, width, body[5]));
            }
            0xFE => comments.push(body.iter().map(|&b| char::from(b)).collect::<String>()),
            _ => {}
        }
        pos += 2 + length;
    }
    let (marker, precision, height, width, samples) = frame.ok_or(BAD)?;
    let color = match samples {
        1 => "grayscale",
        3 => "truecolor",
        _ => "CMYK",
    };
    let method = if marker >= 0xC9 {
        "Arithmetic"
    } else {
        "Huffman"
    };
    let process = match marker & 0x03 {
        2 => "Progressive",
        3 => "Lossless",
        _ => "Sequential",
    };
    let slots = comments
        .into_iter()
        .map(|c| text_value(c).array(&engine.cancel).map(std::sync::Arc::new))
        .collect::<Result<Vec<_>>>()?;
    let n = slots.len();
    let comment = Value::Container(Array::Cell(CellArray::from_shared(
        usize::from(n > 0),
        n,
        slots,
        &engine.cancel,
    )?));
    Ok((
        vec![
            "Format",
            "FormatVersion",
            "Width",
            "Height",
            "BitDepth",
            "ColorType",
            "FormatSignature",
            "NumberOfSamples",
            "CodingMethod",
            "CodingProcess",
            "Comment",
        ],
        vec![
            text_value("jpg"),
            text_value(""),
            num(f64::from(width)),
            num(f64::from(height)),
            num(f64::from(precision) * f64::from(samples)),
            text_value(color),
            text_value(""),
            num(f64::from(samples)),
            text_value(method),
            text_value(process),
            comment,
        ],
    ))
}

fn bmp_info(bytes: &[u8]) -> Result<(Vec<&'static str>, Vec<Value>)> {
    const BAD: &str = "BMP file is truncated or damaged.";
    let header = u32_le(bytes, 14).ok_or(BAD)? as usize;
    let (width, height, depth, compression, used) = if header == 12 {
        (
            f64::from(u16_le(bytes, 18).ok_or(BAD)?),
            f64::from(u16_le(bytes, 20).ok_or(BAD)?),
            u16_le(bytes, 24).ok_or(BAD)?,
            0,
            0,
        )
    } else {
        (
            f64::from(u32_le(bytes, 18).ok_or(BAD)? as i32),
            f64::from((u32_le(bytes, 22).ok_or(BAD)? as i32).unsigned_abs()),
            u16_le(bytes, 28).ok_or(BAD)?,
            u32_le(bytes, 30).ok_or(BAD)?,
            u32_le(bytes, 46).ok_or(BAD)?,
        )
    };
    let version = match header {
        12 => "Version 2 (Microsoft Windows 2.x)",
        40 => "Version 3 (Microsoft Windows 3.x)",
        108 => "Version 4 (Microsoft Windows 95)",
        124 => "Version 5 (Microsoft Windows 2000)",
        _ => "Unknown",
    };
    let entries = if depth <= 8 {
        if used > 0 {
            used as usize
        } else {
            1usize << depth
        }
    } else {
        0
    };
    // Palette: BGR(A) entries right after the info header.
    let width_entry = if header == 12 { 3 } else { 4 };
    let start = 14 + header;
    // Bound the palette by the file size before it is allocated (the header
    // can claim up to 2^32 entries); the loop below checks each entry.
    if entries > bytes.len() / width_entry {
        return Err(BAD.into());
    }
    let mut map = vec![0.; entries * 3];
    for e in 0..entries {
        let at = start + e * width_entry;
        let entry = bytes.get(at..at + 3).ok_or(BAD)?;
        for (k, &b) in [entry[2], entry[1], entry[0]].iter().enumerate() {
            map[k * entries + e] = f64::from(b) / 255.;
        }
    }
    let colormap = if entries > 0 {
        Value::Matrix(Matrix::from_column_vec(entries, 3, map)?)
    } else {
        empty()
    };
    let compression = match compression {
        0 => "none",
        1 => "8-bit RLE",
        2 => "4-bit RLE",
        3 => "bitfields",
        _ => "unknown",
    };
    Ok((
        vec![
            "Format",
            "FormatVersion",
            "Width",
            "Height",
            "BitDepth",
            "ColorType",
            "FormatSignature",
            "NumColormapEntries",
            "Colormap",
            "CompressionType",
        ],
        vec![
            text_value("bmp"),
            text_value(version),
            num(width),
            num(height),
            num(f64::from(depth)),
            text_value(if depth <= 8 { "indexed" } else { "truecolor" }),
            text_value("BM"),
            num(entries as f64),
            colormap,
            text_value(compression),
        ],
    ))
}

fn tiff_info(bytes: &[u8]) -> Result<(Vec<&'static str>, Vec<Value>)> {
    const BAD: &str = "TIFF file is truncated or damaged.";
    let little = bytes.starts_with(b"II");
    let u16_at = |at: usize| -> Option<u16> {
        let b: [u8; 2] = bytes.get(at..at + 2)?.try_into().ok()?;
        Some(if little {
            u16::from_le_bytes(b)
        } else {
            u16::from_be_bytes(b)
        })
    };
    let u32_at = |at: usize| -> Option<u32> {
        let b: [u8; 4] = bytes.get(at..at + 4)?.try_into().ok()?;
        Some(if little {
            u32::from_le_bytes(b)
        } else {
            u32::from_be_bytes(b)
        })
    };
    let ifd = u32_at(4).ok_or(BAD)? as usize;
    let count = usize::from(u16_at(ifd).ok_or(BAD)?);
    let (mut width, mut height, mut samples, mut photometric) = (0u32, 0u32, 1u32, 1u32);
    let mut bits = vec![1u32];
    for e in 0..count {
        let at = ifd + 2 + 12 * e;
        let tag = u16_at(at).ok_or(BAD)?;
        let kind = u16_at(at + 2).ok_or(BAD)?;
        let n = u32_at(at + 4).ok_or(BAD)? as usize;
        // One SHORT or LONG value stored in the entry itself.
        let value = |k: usize| -> Option<u32> {
            let size = if kind == 3 { 2 } else { 4 };
            let base = if n * size <= 4 {
                at + 8
            } else {
                u32_at(at + 8)? as usize
            };
            if kind == 3 {
                u16_at(base + k * 2).map(u32::from)
            } else {
                u32_at(base + k * 4)
            }
        };
        match tag {
            256 => width = value(0).ok_or(BAD)?,
            257 => height = value(0).ok_or(BAD)?,
            258 => {
                bits = (0..n.min(16))
                    .map(&value)
                    .collect::<Option<Vec<_>>>()
                    .ok_or(BAD)?
            }
            262 => photometric = value(0).ok_or(BAD)?,
            277 => samples = value(0).ok_or(BAD)?,
            _ => {}
        }
    }
    // A BitsPerSample tag with count 0 names no depth: the file is damaged.
    let per_sample = f64::from(*bits.first().ok_or(BAD)?);
    let color = match photometric {
        2 => "truecolor",
        3 => "indexed",
        5 => "CMYK",
        _ => "grayscale",
    };
    let signature: &[f64] = if little {
        &[73., 73., 42., 0.]
    } else {
        &[77., 77., 0., 42.]
    };
    Ok((
        vec![
            "Format",
            "FormatVersion",
            "Width",
            "Height",
            "BitDepth",
            "ColorType",
            "FormatSignature",
            "ByteOrder",
            "BitsPerSample",
            "SamplesPerPixel",
        ],
        vec![
            text_value("tif"),
            empty(),
            num(f64::from(width)),
            num(f64::from(height)),
            num(per_sample * f64::from(samples)),
            text_value(color),
            row(signature)?,
            text_value(if little {
                "little-endian"
            } else {
                "big-endian"
            }),
            row(&bits.iter().map(|&b| f64::from(b)).collect::<Vec<_>>())?,
            num(f64::from(samples)),
        ],
    ))
}

// -------------------------------------------------------------- imread/write

#[cfg(not(feature = "plot-export"))]
pub(in crate::engine) fn read(_: &Engine, _: &[u8], _: &str, _: usize) -> Result<Vec<Value>> {
    Err("imread: JPEG/BMP/TIFF need the desktop build".into())
}

#[cfg(not(feature = "plot-export"))]
pub(in crate::engine) fn write(
    _: &Engine,
    _: &Value,
    _: Option<&Value>,
    _: &str,
    _: &[(String, &Value)],
) -> Result<Vec<u8>> {
    Err("imwrite: JPEG/BMP/TIFF need the desktop build".into())
}

/// Planes of interleaved samples (`channels` per pixel, row-major) as an
/// H-by-W(-by-C) image of `kind`.
#[cfg(feature = "plot-export")]
fn planes(h: usize, w: usize, channels: usize, keep: &[usize], kind: Kind, samples: &[f64]) -> Img {
    let n = h * w;
    let mut data = vec![0.; n * keep.len()];
    for (k, &ch) in keep.iter().enumerate() {
        for c in 0..w {
            for r in 0..h {
                data[k * n + c * h + r] = samples[(r * w + c) * channels + ch];
            }
        }
    }
    Img {
        h,
        w,
        c: keep.len(),
        kind,
        data,
    }
}

#[cfg(feature = "plot-export")]
pub(in crate::engine) fn read(
    engine: &Engine,
    bytes: &[u8],
    format: &str,
    count: usize,
) -> Result<Vec<Value>> {
    use image::{DynamicImage, ImageDecoder, codecs};
    use std::io::Cursor;
    let fail =
        |e: image::ImageError| format!("imread: cannot decode the {} file: {e}", label(format));
    // Size guard before decoding (the dense element limit, as for PNG).
    let guard = |(w, h): (u32, u32), channels: usize| -> Result<()> {
        let (w, h) = (w as usize, h as usize);
        if w == 0 || h == 0 {
            return Err(format!("{} image has no pixels.", label(format)));
        }
        Matrix::checked_len(h, w.checked_mul(channels).ok_or("Image is too large.")?)?;
        Ok(())
    };
    let mut map = None;
    let decoded = match format {
        "jpg" => {
            let d = codecs::jpeg::JpegDecoder::new(Cursor::new(bytes)).map_err(fail)?;
            guard(d.dimensions(), usize::from(d.color_type().channel_count()))?;
            DynamicImage::from_decoder(d).map_err(fail)?
        }
        "bmp" => {
            let mut d = codecs::bmp::BmpDecoder::new(Cursor::new(bytes)).map_err(fail)?;
            if let Some(palette) = d.get_palette() {
                // Indexed BMP: the indices (0-based, uint8) and the colormap
                // of the header's colours-used count (the decoder pads to 2^bits).
                let used = u32_le(bytes, 46)
                    .filter(|_| u32_le(bytes, 14) >= Some(40))
                    .unwrap_or(0) as usize;
                let k = if used > 0 {
                    used.min(palette.len())
                } else {
                    palette.len()
                };
                let mut m = vec![0.; k * 3];
                for (e, entry) in palette.iter().take(k).enumerate() {
                    for c in 0..3 {
                        m[c * k + e] = f64::from(entry[c]) / 255.;
                    }
                }
                map = Some(Value::Matrix(Matrix::from_column_vec(k, 3, m)?));
                d.set_indexed_color(true);
            }
            guard(d.dimensions(), usize::from(d.color_type().channel_count()))?;
            DynamicImage::from_decoder(d).map_err(fail)?
        }
        _ => {
            let d = codecs::tiff::TiffDecoder::new(Cursor::new(bytes)).map_err(fail)?;
            guard(d.dimensions(), usize::from(d.color_type().channel_count()))?;
            DynamicImage::from_decoder(d).map_err(fail)?
        }
    };
    engine.check_cancel()?;
    let (w, h) = (decoded.width() as usize, decoded.height() as usize);
    let u8 = Kind::Typed(NumericClass::Uint8);
    let u16 = Kind::Typed(NumericClass::Uint16);
    let (channels, kind, samples): (usize, Kind, Vec<f64>) = match &decoded {
        DynamicImage::ImageLuma8(b) => (1, u8, b.iter().map(|&v| f64::from(v)).collect()),
        DynamicImage::ImageLumaA8(b) => (2, u8, b.iter().map(|&v| f64::from(v)).collect()),
        DynamicImage::ImageRgb8(b) => (3, u8, b.iter().map(|&v| f64::from(v)).collect()),
        DynamicImage::ImageRgba8(b) => (4, u8, b.iter().map(|&v| f64::from(v)).collect()),
        DynamicImage::ImageLuma16(b) => (1, u16, b.iter().map(|&v| f64::from(v)).collect()),
        DynamicImage::ImageLumaA16(b) => (2, u16, b.iter().map(|&v| f64::from(v)).collect()),
        DynamicImage::ImageRgb16(b) => (3, u16, b.iter().map(|&v| f64::from(v)).collect()),
        DynamicImage::ImageRgba16(b) => (4, u16, b.iter().map(|&v| f64::from(v)).collect()),
        other => {
            let b = other.to_rgba32f();
            (4, Kind::Double, b.iter().map(|&v| f64::from(v)).collect())
        }
    };
    let colour: &[usize] = if channels >= 3 { &[0, 1, 2] } else { &[0] };
    let mut values = vec![to_value(
        engine,
        planes(h, w, channels, colour, kind, &samples),
    )?];
    if count >= 2 {
        values.push(map.unwrap_or_else(empty));
    }
    if count >= 3 {
        // JPEG and BMP alpha is not returned (as for the reference); TIFF keeps it.
        values.push(if format == "tif" && (channels == 2 || channels == 4) {
            to_value(
                engine,
                planes(h, w, channels, &[channels - 1], kind, &samples),
            )?
        } else {
            empty()
        });
    }
    Ok(values)
}

/// 8-bit (or 16-bit for TIFF uint16) samples of an image for writing.
#[cfg(feature = "plot-export")]
fn levels(img: &Img, sixteen: bool) -> Vec<u16> {
    let top = if sixteen { 65535. } else { 255. };
    let (h, w, c) = (img.h, img.w, img.c);
    let n = h * w;
    let mut out = Vec::with_capacity(n * c);
    for r in 0..h {
        for col in 0..w {
            for k in 0..c {
                let v = img.data[k * n + col * h + r];
                let level = match img.kind {
                    Kind::Typed(NumericClass::Uint8) if sixteen => v * 257.,
                    Kind::Typed(NumericClass::Uint8) => v,
                    Kind::Typed(NumericClass::Uint16) if sixteen => v,
                    Kind::Typed(NumericClass::Uint16) => (v / 257.).round(),
                    Kind::Logical => v * top,
                    // Floating data: [0, 1] scaled, NaN as the top level (as PNG).
                    _ if v.is_nan() => top,
                    _ => (v.clamp(0., 1.) * top).round(),
                };
                out.push(level as u16);
            }
        }
    }
    out
}

#[cfg(feature = "plot-export")]
pub(in crate::engine) fn write(
    engine: &Engine,
    value: &Value,
    map: Option<&Value>,
    format: &str,
    pairs: &[(String, &Value)],
) -> Result<Vec<u8>> {
    use image::{ExtendedColorType, ImageEncoder, codecs};
    let mut img = image(engine, "imwrite", value)?;
    if !matches!(
        img.kind,
        Kind::Double
            | Kind::Logical
            | Kind::Typed(NumericClass::Single | NumericClass::Uint8 | NumericClass::Uint16)
    ) {
        return Err(format!(
            "Expected input to be one of these types:\n\ndouble, single, logical, uint8, uint16\n\nInstead its type was {}.",
            img.kind.name()
        ));
    }
    if img.data.is_empty() {
        return Err("Expected DATA to be nonempty.".into());
    }
    if img.c != 1 && img.c != 3 {
        return Err(format!(
            "{} image data must be either MxN or MxNx3.",
            label(format)
        ));
    }
    let mut quality = 75.;
    for (key, v) in pairs {
        match (format, key.to_ascii_lowercase().as_str()) {
            ("jpg", "quality") => {
                let q = numbers(v)
                    .filter(|q| q.len() == 1)
                    .ok_or("Expected QUALITY to be a scalar with value <= 100.")?;
                if q[0].is_nan() || q[0] > 100. {
                    return Err("Expected QUALITY to be a scalar with value <= 100.".into());
                }
                if q[0] < 0. {
                    return Err("Expected QUALITY to be a scalar with value >= 0.".into());
                }
                quality = q[0];
            }
            (_, k) => {
                return Err(format!(
                    "imwrite: the {} parameter '{k}' is not supported yet",
                    label(format)
                ));
            }
        }
    }
    let (w, h) = (img.w, img.h);
    let width = u32::try_from(w).map_err(|_| "Image is too wide")?;
    let height = u32::try_from(h).map_err(|_| "Image is too tall")?;
    let mut buffer = Vec::new();
    let fail =
        |e: image::ImageError| format!("imwrite: cannot encode the {} file: {e}", label(format));
    // Indexed data with a colormap: a BMP palette; JPEG and TIFF get RGB.
    let mut palette = None;
    if let Some(map) = map {
        let m = image(engine, "imwrite", map)?;
        if m.c != 1 || m.w != 3 || m.h == 0 || m.h > 256 {
            return Err("The colormap must be a 1x3 to 256x3 array.".into());
        }
        if m.data.iter().any(|v| !(0. ..=1.).contains(v)) {
            return Err("Colormap values must be in the range [0, 1].".into());
        }
        if img.c != 1 {
            return Err("imwrite: an indexed image with a colormap must be 2-D".into());
        }
        let offset = if img.kind.is_float() { 1. } else { 0. };
        for v in &mut img.data {
            let x = *v - offset;
            if !(x >= 0. && x.fract() == 0. && x < m.h as f64) {
                return Err("Index values must be valid rows of the colormap.".into());
            }
            *v = x;
        }
        let entries: Vec<[u8; 3]> = (0..m.h)
            .map(|r| [0, 1, 2].map(|k| (m.data[k * m.h + r] * 255.).round() as u8))
            .collect();
        if format == "bmp" {
            img.kind = Kind::Typed(NumericClass::Uint8);
            palette = Some(entries);
        } else {
            let n = img.plane_len();
            let mut data = vec![0.; 3 * n];
            for i in 0..n {
                let e = entries[img.data[i] as usize];
                for k in 0..3 {
                    data[k * n + i] = f64::from(e[k]);
                }
            }
            img = Img {
                c: 3,
                kind: Kind::Typed(NumericClass::Uint8),
                data,
                ..img
            };
        }
    }
    engine.check_cancel()?;
    let sixteen = format == "tif" && img.kind == Kind::Typed(NumericClass::Uint16);
    let samples = levels(&img, sixteen);
    let color = match (img.c, sixteen) {
        (1, false) => ExtendedColorType::L8,
        (1, true) => ExtendedColorType::L16,
        (_, false) => ExtendedColorType::Rgb8,
        (_, true) => ExtendedColorType::Rgb16,
    };
    match format {
        "jpg" => {
            let bytes: Vec<u8> = samples.iter().map(|&v| v as u8).collect();
            let q = quality.round().clamp(1., 100.) as u8;
            codecs::jpeg::JpegEncoder::new_with_quality(&mut buffer, q)
                .write_image(&bytes, width, height, color)
                .map_err(fail)?;
        }
        "bmp" => {
            let bytes: Vec<u8> = samples.iter().map(|&v| v as u8).collect();
            codecs::bmp::BmpEncoder::new(&mut buffer)
                .encode_with_palette(&bytes, width, height, color, palette.as_deref())
                .map_err(fail)?;
        }
        _ => {
            let mut cursor = std::io::Cursor::new(&mut buffer);
            let encoder = codecs::tiff::TiffEncoder::new(&mut cursor);
            if sixteen {
                let bytes: Vec<u8> = samples.iter().flat_map(|v| v.to_ne_bytes()).collect();
                encoder
                    .write_image(&bytes, width, height, color)
                    .map_err(fail)?;
            } else {
                let bytes: Vec<u8> = samples.iter().map(|&v| v as u8).collect();
                encoder
                    .write_image(&bytes, width, height, color)
                    .map_err(fail)?;
            }
        }
    }
    Ok(buffer)
}
