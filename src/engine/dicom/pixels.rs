//! Native grayscale/RGB frame decoding from the shared parser's descriptors.
//! No file-format parsing, engine state, rescaling, or parallel pool lives here.
use super::parser::{
    self, Dataset, ElementValue, ParsedFile, PixelEncoding, PixelLocation, TransferSyntax,
    ValueRange,
};
use crate::{
    Matrix, Result,
    typed_array::{NumericClass, TypedArray},
};
use std::{
    io::{Read, Seek, SeekFrom},
    sync::atomic::{AtomicBool, Ordering},
};

/// Shared, codec-independent image metadata and output-element admission.
/// Native and encapsulated codecs both start here; nothing reads pixel bytes.
pub(crate) struct Image {
    pub(crate) rows: usize,
    pub(crate) cols: usize,
    pub(crate) samples: usize,
    pub(crate) frames: usize,
    pub(crate) folded_cols: usize,
    pub(crate) plane_len: usize,
    /// Native PlanarConfiguration (true for monochrome). Codecs that define
    /// their own component layout (RLE segments) must not use this flag.
    pub(crate) planar: bool,
    /// PhotometricInterpretation is YBR_FULL or YBR_FULL_422. Only codecs whose
    /// decoder converts to RGB admit it (`image_with`); native/RLE never do.
    #[cfg_attr(not(feature = "plot-export"), allow(dead_code))]
    pub(crate) ybr: bool,
    pub(crate) len: usize,
    pub(crate) class: NumericClass,
    pub(crate) allocated: u16,
    pub(crate) stored: u16,
}
impl Image {
    /// Typed output bytes (one cell per stored sample).
    pub(crate) fn image_bytes(&self) -> Result<usize> {
        self.len
            .checked_mul(self.class.size())
            .ok_or_else(|| "dicomread: pixel byte size overflow".into())
    }
}

/// Created only after all metadata, length, dimension and image-budget checks.
/// The byte range is absolute in the same reader that was structurally parsed.
pub(crate) struct Layout {
    image: Image,
    pub(crate) len: usize,
    syntax: TransferSyntax,
    swap_bytes: bool,
    value: ValueRange,
    payload_len: usize,
}

fn required_range(data: &Dataset, tag: u32, vr: [u8; 2], name: &str) -> Result<ValueRange> {
    let e = data
        .elements
        .iter()
        .find(|e| e.tag == tag)
        .ok_or_else(|| format!("dicomread: missing required {name}"))?;
    match e.value {
        ElementValue::Primitive(range) if e.vr == vr => Ok(range),
        _ => Err(format!("dicomread: invalid {name} value/VR")),
    }
}

fn us<R: Read + Seek>(
    reader: &mut R,
    parsed: &ParsedFile,
    tag: u32,
    name: &str,
    cancel: &AtomicBool,
) -> Result<u16> {
    let range = required_range(&parsed.dataset, tag, *b"US", name)?;
    if range.length != 2 {
        return Err(format!("dicomread: {name} must be a single US value"));
    }
    let bytes = parser::read_value(reader, range, parsed.dataset_backing, cancel)?;
    let bytes = bytes
        .as_slice()
        .try_into()
        .map_err(|_| format!("dicomread: invalid {name}"))?;
    Ok(parsed.syntax.u16(bytes))
}

pub(super) fn ascii<R: Read + Seek>(
    reader: &mut R,
    parsed: &ParsedFile,
    tag: u32,
    vr: [u8; 2],
    name: &str,
    cancel: &AtomicBool,
) -> Result<String> {
    let range = required_range(&parsed.dataset, tag, vr, name)?;
    if range.length == 0 || range.length > 16 {
        return Err(format!("dicomread: invalid {name} length"));
    }
    let bytes = parser::read_value(reader, range, parsed.dataset_backing, cancel)?;
    if !bytes.is_ascii() {
        return Err(format!("dicomread: invalid {name} text"));
    }
    String::from_utf8(bytes).map_err(|_| format!("dicomread: invalid {name} text"))
}

/// Validate a supported native layout without reading or allocating pixel data.
/// `budget` is available scratch/output bytes AFTER parser/workspace accounting;
/// it is intentionally independent of the parser's metadata-value cap.
pub(crate) fn layout<R: Read + Seek>(
    reader: &mut R,
    parsed: &ParsedFile,
    budget: usize,
    max_elements: usize,
    cancel: &AtomicBool,
) -> Result<Layout> {
    if parsed.encoding != PixelEncoding::Native {
        return Err("dicomread: encapsulated PixelData requires its codec layout".into());
    }
    let image = image(reader, parsed, max_elements, cancel)?;
    native(parsed, image, budget)
}

/// Shared image metadata admission for every transfer syntax: dimensions,
/// photometric/sample/planar rules, frames, bit layout and the element cap.
pub(crate) fn image<R: Read + Seek>(
    reader: &mut R,
    parsed: &ParsedFile,
    max_elements: usize,
    cancel: &AtomicBool,
) -> Result<Image> {
    image_with(reader, parsed, max_elements, cancel, false)
}

/// `image` with a JPEG codec hook: additionally admit YBR_FULL and (baseline
/// only) YBR_FULL_422. Lossless preserves the stored components, while baseline
/// returns RGB. Native and RLE callers keep their original admission rules.
pub(crate) fn image_with<R: Read + Seek>(
    reader: &mut R,
    parsed: &ParsedFile,
    max_elements: usize,
    cancel: &AtomicBool,
    ybr_to_rgb: bool,
) -> Result<Image> {
    crate::dsp::check_cancel(cancel)?;
    let rows = usize::from(us(reader, parsed, 0x0028_0010, "Rows", cancel)?);
    let cols = usize::from(us(reader, parsed, 0x0028_0011, "Columns", cancel)?);
    if rows == 0 || cols == 0 {
        return Err("dicomread: Rows and Columns must be positive".into());
    }
    let samples = usize::from(us(reader, parsed, 0x0028_0002, "SamplesPerPixel", cancel)?);
    let photo = ascii(
        reader,
        parsed,
        0x0028_0004,
        *b"CS",
        "PhotometricInterpretation",
        cancel,
    )?;
    let photo = photo.trim_end_matches(' ');
    let lossless = matches!(parsed.encoding, PixelEncoding::JpegLossless { .. });
    let j2k = matches!(parsed.encoding, PixelEncoding::Jpeg2000 { .. });
    let ybr = ybr_to_rgb
        && if j2k {
            matches!(photo, "YBR_RCT" | "YBR_ICT")
        } else {
            photo == "YBR_FULL" || (!lossless && photo == "YBR_FULL_422")
        };
    let expected_samples = match photo {
        "MONOCHROME1" | "MONOCHROME2" => 1,
        "RGB" => 3,
        _ if ybr => 3,
        _ if j2k => return Err("dicomread: unsupported PhotometricInterpretation for JPEG 2000; only MONOCHROME1/2, RGB, YBR_RCT and YBR_ICT are supported".into()),
        _ if lossless => return Err("dicomread: unsupported PhotometricInterpretation for JPEG Lossless; only MONOCHROME1, MONOCHROME2, RGB and stored YBR_FULL are supported yet".into()),
        _ if ybr_to_rgb => return Err("dicomread: unsupported PhotometricInterpretation for JPEG Baseline; only MONOCHROME1, MONOCHROME2, YBR_FULL_422, YBR_FULL and RGB are supported yet".into()),
        _ => return Err("dicomread: unsupported PhotometricInterpretation; only MONOCHROME1, MONOCHROME2 and RGB are supported yet".into()),
    };
    if samples != expected_samples {
        return Err(if ybr_to_rgb {
            "dicomread: SamplesPerPixel contradicts PhotometricInterpretation (MONOCHROME1/2 requires 1; RGB, YBR_FULL and YBR_FULL_422 require 3)"
        } else {
            "dicomread: SamplesPerPixel contradicts PhotometricInterpretation (MONOCHROME1/2 requires 1; RGB requires 3)"
        }
        .into());
    }
    let planar = if samples == 3 {
        match us(reader, parsed, 0x0028_0006, "PlanarConfiguration", cancel)? {
            0 => false,
            1 => true,
            _ => return Err("dicomread: RGB PlanarConfiguration must be 0 or 1".into()),
        }
    } else {
        true
    };
    let frames = if parsed.dataset.elements.iter().any(|e| e.tag == 0x0028_0008) {
        let text = ascii(
            reader,
            parsed,
            0x0028_0008,
            *b"IS",
            "NumberOfFrames",
            cancel,
        )?;
        // IS is decimal integer text. Parse directly to the target count type:
        // no float rounding, NaN/Inf, fractional or overflowing count is accepted.
        text.trim_matches(' ')
            .parse::<usize>()
            .ok()
            .filter(|&n| n > 0)
            .ok_or("dicomread: NumberOfFrames must be a positive representable integer")?
    } else {
        1
    };
    let folded_cols = cols
        .checked_mul(samples)
        .and_then(|n| n.checked_mul(frames))
        .ok_or("dicomread: Rows*Columns*Samples*Frames dimensions overflow")?;
    let len = Matrix::checked_len(rows, folded_cols)?;
    if len > max_elements {
        return Err("dicomread: decoded image exceeds the memory element budget".into());
    }
    let plane_len = Matrix::checked_len(rows, cols)?;
    let allocated = us(reader, parsed, 0x0028_0100, "BitsAllocated", cancel)?;
    let stored = us(reader, parsed, 0x0028_0101, "BitsStored", cancel)?;
    let high = us(reader, parsed, 0x0028_0102, "HighBit", cancel)?;
    let representation = us(reader, parsed, 0x0028_0103, "PixelRepresentation", cancel)?;
    if representation > 1 {
        return Err("dicomread: invalid PixelRepresentation (must be 0 or 1)".into());
    }
    if samples == 3 && representation != 0 && !j2k {
        return Err(
            "dicomread: signed RGB is not supported yet; RGB requires unsigned 8/16-bit samples"
                .into(),
        );
    }
    let class = match (allocated, representation) {
        (8, 0) => NumericClass::Uint8,
        (8, 1) => NumericClass::Int8,
        (16, 0) => NumericClass::Uint16,
        (16, 1) => NumericClass::Int16,
        _ => return Err("dicomread: unsupported BitsAllocated/PixelRepresentation; only uint8, int8, uint16 and int16 are supported yet".into()),
    };
    if stored == 0 || stored > allocated {
        return Err("dicomread: BitsStored must be between 1 and BitsAllocated".into());
    }
    if high != stored - 1 {
        return Err("dicomread: HighBit must equal BitsStored-1; shifted stored-bit layouts are not supported yet".into());
    }
    Ok(Image {
        rows,
        cols,
        samples,
        frames,
        folded_cols,
        plane_len,
        planar,
        ybr,
        len,
        class,
        allocated,
        stored,
    })
}

/// Native-only payload checks: exact uncompressed byte count, VR and range.
fn native(parsed: &ParsedFile, image: Image, budget: usize) -> Result<Layout> {
    let allocated = image.allocated;
    // Float/double pixel payloads have their own tags and may not masquerade as
    // integer PixelData, even when metadata happens to describe integer cells.
    required_range(
        &parsed.dataset,
        0x7FE0_0010,
        match parsed.dataset.pixels {
            Some(PixelLocation::Native { vr, .. }) => vr,
            _ => *b"OW",
        },
        "PixelData",
    )?;
    let (value, vr) = match &parsed.dataset.pixels {
        Some(PixelLocation::Native { value, vr }) => (*value, *vr),
        Some(PixelLocation::Encapsulated { .. }) => {
            return Err("dicomread: encapsulated PixelData is not supported yet".into());
        }
        None => return Err("dicomread: missing PixelData".into()),
    };
    if vr != *b"OW" && !(allocated == 8 && vr == *b"OB") {
        return Err("dicomread: invalid PixelData VR for BitsAllocated (16-bit requires OW; 8-bit requires OB or OW)".into());
    }
    let image_len = image.image_bytes()?;
    let payload_len = image_len
        .checked_add(image_len % 2)
        .ok_or("dicomread: padded pixel byte size overflow")?;
    if value.length != payload_len as u64 {
        return Err("dicomread: PixelData length does not match Rows*Columns*Samples*Frames*bytes plus even-length padding".into());
    }
    if value.backing != parsed.dataset_backing
        || value
            .offset
            .checked_add(value.length)
            .is_none_or(|end| end > parsed.dataset_size)
    {
        return Err("dicomread: PixelData range exceeds admitted backing size".into());
    }
    // Input bytes + typed Vec + transient Vec-to-Arc copy. No i128/double image
    // is allocated. All output storage is admitted before the first pixel read.
    let peak = image_len
        .checked_mul(2)
        .and_then(|n| n.checked_add(payload_len))
        .ok_or("dicomread: image memory accounting overflow")?;
    if peak > budget {
        return Err("dicomread: decoded image exceeds the memory/output budget".into());
    }
    Ok(Layout {
        len: image.len,
        image,
        syntax: parsed.syntax,
        swap_bytes: allocated == 8 && vr == *b"OW" && !parsed.syntax.little(),
        value,
        payload_len,
    })
}

/// Read exactly the admitted range, then transpose stored samples into immutable
/// typed column-major storage. Cancellation is polled in reads and conversion.
pub(crate) fn decode<R: Read + Seek>(
    reader: &mut R,
    layout: &Layout,
    cancel: &AtomicBool,
) -> Result<TypedArray> {
    crate::dsp::check_cancel(cancel)?;
    let mut bytes = Vec::new();
    bytes
        .try_reserve_exact(layout.payload_len)
        .map_err(|_| "dicomread: unable to allocate pixel buffer")?;
    bytes.resize(layout.payload_len, 0);
    reader
        .seek(SeekFrom::Start(layout.value.offset))
        .map_err(|_| "dicomread: unable to seek PixelData")?;
    for chunk in bytes.chunks_mut(64 * 1024) {
        crate::dsp::check_cancel(cancel)?;
        reader
            .read_exact(chunk)
            .map_err(|_| "dicomread: truncated or unreadable PixelData")?;
    }
    let result = convert(&bytes, layout, cancel);
    crate::dsp::check_cancel(cancel)?;
    result
}

fn convert(bytes: &[u8], layout: &Layout, cancel: &AtomicBool) -> Result<TypedArray> {
    convert_samples(
        bytes,
        &layout.image,
        layout.image.planar,
        layout.syntax,
        layout.swap_bytes,
        cancel,
    )
}

/// Normalized decoder output: per frame, each sample plane in row-major order;
/// 16-bit cells little-endian. Codecs reconstruct exactly this layout, then
/// share BitsStored masking/sign extension and column-major construction.
pub(crate) fn convert_planes(
    bytes: &[u8],
    image: &Image,
    cancel: &AtomicBool,
) -> Result<TypedArray> {
    convert_samples(
        bytes,
        image,
        true,
        TransferSyntax::ExplicitLittle,
        false,
        cancel,
    )
}

/// Decoder output that is already interleaved per pixel (frame after frame,
/// row-major, samples adjacent), whatever PlanarConfiguration says. Cells are
/// normalized to the admitted 8/16-bit little-endian layout before conversion.
#[cfg_attr(not(feature = "plot-export"), allow(dead_code))]
pub(crate) fn convert_interleaved(
    bytes: &[u8],
    image: &Image,
    cancel: &AtomicBool,
) -> Result<TypedArray> {
    convert_samples(
        bytes,
        image,
        false,
        TransferSyntax::ExplicitLittle,
        false,
        cancel,
    )
}

fn convert_samples(
    bytes: &[u8],
    layout: &Image,
    planar: bool,
    syntax: TransferSyntax,
    swap_bytes: bool,
    cancel: &AtomicBool,
) -> Result<TypedArray> {
    crate::dsp::check_cancel(cancel)?;
    let mask = (1u32 << layout.stored) - 1;
    let sign = 1u32 << (layout.stored - 1);
    let mut index: usize = 0;
    let values = std::iter::from_fn(|| {
        if index == layout.len || (index.is_multiple_of(4096) && cancel.load(Ordering::Relaxed)) {
            return None;
        }
        // Destination order is r + R*(c + C*(s + S*f)). Every product below
        // is bounded by the admitted total sample count. Planes are per frame,
        // and OW byte swapping applies to the full stream, not each frame.
        let plane = index / layout.plane_len;
        let within = index % layout.plane_len;
        let pixel = (within % layout.rows) * layout.cols + within / layout.rows;
        let sample = if planar {
            plane * layout.plane_len + pixel
        } else {
            (plane / layout.samples * layout.plane_len + pixel) * layout.samples
                + plane % layout.samples
        };
        let raw = if matches!(layout.class, NumericClass::Uint8 | NumericClass::Int8) {
            u32::from(*bytes.get(if swap_bytes { sample ^ 1 } else { sample })?)
        } else {
            let offset = sample.checked_mul(2)?;
            u32::from(syntax.u16([*bytes.get(offset)?, *bytes.get(offset + 1)?]))
        } & mask;
        index += 1;
        Some(
            if matches!(layout.class, NumericClass::Int16 | NumericClass::Int8) && raw & sign != 0 {
                i128::from(raw) - (1i128 << layout.stored)
            } else {
                i128::from(raw)
            },
        )
    });
    let result = TypedArray::from_ints(layout.class, layout.rows, layout.folded_cols, values);
    crate::dsp::check_cancel(cancel)?;
    result?.with_dims(&[layout.rows, layout.cols, layout.samples, layout.frames])
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Cursor;

    const SMALL: &[u8] = include_bytes!("../../../tests/fixtures/dicom/pixels_explicit_le.dcm");
    const LARGE: &[u8] = include_bytes!("../../../tests/fixtures/dicom/pixels_large.dcm");
    const RGB: &[u8] = include_bytes!("../../../tests/fixtures/dicom/frames_baseline.dcm");

    #[test]
    fn admission_does_not_read_pixels() {
        struct NoPixels {
            input: Cursor<&'static [u8]>,
            start: u64,
        }
        impl Read for NoPixels {
            fn read(&mut self, out: &mut [u8]) -> std::io::Result<usize> {
                assert!(self.input.position() + out.len() as u64 <= self.start);
                self.input.read(out)
            }
        }
        impl Seek for NoPixels {
            fn seek(&mut self, pos: SeekFrom) -> std::io::Result<u64> {
                self.input.seek(pos)
            }
        }
        let cancel = AtomicBool::new(false);
        for (bytes, payload, len, samples, frames) in [(SMALL, 12, 6, 1, 1), (RGB, 36, 36, 3, 2)] {
            let mut reader = NoPixels {
                input: Cursor::new(bytes),
                start: bytes.len() as u64 - payload as u64,
            };
            let parsed = parser::parse(&mut reader, bytes.len() as u64, 1 << 20, &cancel).unwrap();
            assert!(
                layout(&mut reader, &parsed, 3 * payload - 1, 100, &cancel)
                    .err()
                    .unwrap()
                    .contains("memory/output")
            );
            assert!(
                layout(&mut reader, &parsed, 3 * payload, len - 1, &cancel)
                    .err()
                    .unwrap()
                    .contains("element budget")
            );
            let layout = layout(&mut reader, &parsed, 3 * payload, len, &cancel).unwrap();
            assert_eq!(
                (layout.image.rows, layout.image.cols, layout.len),
                (2, 3, len)
            );
            assert_eq!(
                (layout.image.samples, layout.image.frames),
                (samples, frames)
            );
        }
    }

    #[test]
    fn pixel_reads_are_chunked_and_cancellable() {
        struct CancelOnRead<'a> {
            input: Cursor<&'static [u8]>,
            cancel: &'a AtomicBool,
            calls: usize,
        }
        impl Read for CancelOnRead<'_> {
            fn read(&mut self, out: &mut [u8]) -> std::io::Result<usize> {
                assert!(out.len() <= 64 * 1024);
                self.calls += 1;
                let n = self.input.read(out)?;
                self.cancel.store(true, Ordering::Relaxed);
                Ok(n)
            }
        }
        impl Seek for CancelOnRead<'_> {
            fn seek(&mut self, pos: SeekFrom) -> std::io::Result<u64> {
                self.input.seek(pos)
            }
        }
        let cancel = AtomicBool::new(false);
        let mut input = Cursor::new(LARGE);
        let parsed = parser::parse(&mut input, LARGE.len() as u64, 16 << 20, &cancel).unwrap();
        let layout = layout(&mut input, &parsed, 16 << 20, 1 << 20, &cancel).unwrap();
        let mut reader = CancelOnRead {
            input,
            cancel: &cancel,
            calls: 0,
        };
        assert!(
            decode(&mut reader, &layout, &cancel)
                .unwrap_err()
                .to_lowercase()
                .contains("cancel")
        );
        assert_eq!(reader.calls, 1);
    }

    #[test]
    fn truncated_after_parse_and_cancelled_conversion_return_errors() {
        let cancel = AtomicBool::new(false);
        let mut input = Cursor::new(SMALL);
        let parsed = parser::parse(&mut input, SMALL.len() as u64, 1 << 20, &cancel).unwrap();
        let layout = layout(&mut input, &parsed, 1 << 20, 100, &cancel).unwrap();
        let mut truncated = Cursor::new(&SMALL[..SMALL.len() - 2]);
        assert!(
            decode(&mut truncated, &layout, &cancel)
                .unwrap_err()
                .contains("truncated")
        );
        assert!(convert(&[0; 2], &layout, &cancel).is_err());
        cancel.store(true, Ordering::Relaxed);
        assert!(
            convert(&[0; 12], &layout, &cancel)
                .unwrap_err()
                .to_lowercase()
                .contains("cancel")
        );
    }
}
