//! Shared encapsulated JPEG admission (PS3.5 A.4, 8.2.1). Baseline Process 1
//! (.50) uses `image`; lossless Process 14 (.57/.70) uses `jpeg-decoder` in the
//! lossless child module, available independently of desktop/plot-export.
//!
//! That decoder is deliberately lenient: it returns partly grey images for
//! truncated or corrupt entropy data and also accepts extended/progressive
//! frames. So each frame codestream first passes `inspect`, a strict ITU-T T.81
//! walk of the markers and of every Huffman-coded block. Only complete,
//! single-scan baseline streams of exactly the DICOM size reach the decoder,
//! and its colour handling (Adobe APP14 / R,G,B ids) is checked against
//! PhotometricInterpretation first.
//!
//! Known decoder trait (not corrected here): for 4:2:2/4:2:0 frames whose
//! width/height is even but not a multiple of the 16-sample MCU, the last
//! column/row is interpolated with MCU-padding chroma, where libjpeg
//! replicates the true edge. See the task notes for measured differences.
use super::parser::{self, ElementValue, Fragment, ParsedFile, PixelEncoding, PixelLocation};
use super::pixels::{self, Image};
use crate::{
    Result,
    typed_array::{NumericClass, TypedArray},
};
use std::{
    io::{Read, Seek, SeekFrom},
    ops::Range,
    sync::atomic::AtomicBool,
};

#[path = "jpeg_lossless.rs"]
mod lossless;

#[cfg(feature = "plot-export")]
use std::io::Cursor;

const UID: &str = "1.2.840.10008.1.2.4.50";
const CHUNK: usize = 64 * 1024;
const POLL: usize = 1024;

/// Admitted JPEG decode plan. Created before any fragment payload byte is read.
pub(crate) struct Layout {
    image: Image,
    /// Fragment index range of every frame, in frame order.
    frames: Vec<Range<usize>>,
    /// Largest concatenated frame codestream, including its fragment padding.
    max_stream: usize,
    /// Work units to charge before decoding: output cells + compressed bytes/8.
    pub(crate) units: usize,
}

fn memory_error() -> String {
    "dicomread: JPEG memory accounting overflow".into()
}

fn bad(reason: &str) -> String {
    format!("dicomread: invalid JPEG Baseline frame: {reason}")
}

fn truncated() -> String {
    bad("truncated JPEG stream: entropy-coded data or a marker segment ends without EOI")
}

/// Shared image admission, then JPEG-only header rules, fragment-to-frame
/// grouping and memory admission. Reads only the Basic Offset Table value.
pub(crate) fn layout<R: Read + Seek>(
    reader: &mut R,
    parsed: &ParsedFile,
    budget: usize,
    max_elements: usize,
    cancel: &AtomicBool,
) -> Result<Layout> {
    let lossless = matches!(parsed.encoding, PixelEncoding::JpegLossless { .. });
    if parsed.encoding != PixelEncoding::Jpeg && !lossless {
        return Err("dicomread: JPEG decoder requires the JPEG Baseline transfer syntax".into());
    }
    let image = pixels::image_with(reader, parsed, max_elements, cancel, true)?;
    // PS3.5 Table 8.2.1-1: Process 1 is 8 bits allocated/stored, unsigned.
    if lossless {
        if !(2..=16).contains(&image.stored) {
            return Err("dicomread: JPEG Lossless requires BitsStored (precision) in 2..16".into());
        }
    } else if image.allocated != 8 || image.stored != 8 || image.class != NumericClass::Uint8 {
        return Err(format!(
            "dicomread: JPEG Baseline ({UID}) requires BitsAllocated=8, BitsStored=8 and PixelRepresentation=0"
        ));
    }
    let data = &parsed.dataset;
    if data
        .elements
        .iter()
        .any(|e| matches!(e.tag, 0x7FE0_0001 | 0x7FE0_0002))
    {
        return Err("dicomread: Extended Offset Table is not supported yet".into());
    }
    let element = data
        .elements
        .iter()
        .find(|e| e.tag == 0x7FE0_0010)
        .ok_or("dicomread: missing PixelData")?;
    let (offset_table, fragments) = match (&element.value, &data.pixels) {
        (
            ElementValue::Encapsulated,
            Some(PixelLocation::Encapsulated {
                offset_table,
                fragments,
            }),
        ) if element.vr == *b"OB" => (*offset_table, fragments),
        _ if lossless => {
            return Err(
                "dicomread: JPEG Lossless requires undefined-length encapsulated OB PixelData"
                    .into(),
            );
        }
        _ => {
            return Err(format!(
                "dicomread: JPEG Baseline ({UID}) requires undefined-length encapsulated OB PixelData"
            ));
        }
    };
    if fragments.is_empty() {
        return Err("dicomread: encapsulated JPEG PixelData has no fragments".into());
    }
    let mut compressed = 0usize;
    for fragment in fragments {
        crate::dsp::check_cancel(cancel)?;
        let value = fragment.value;
        if value.backing != parsed.dataset_backing
            || value
                .offset
                .checked_add(value.length)
                .is_none_or(|end| end > parsed.dataset_size)
        {
            return Err("dicomread: JPEG fragment range exceeds admitted backing size".into());
        }
        let length = usize::try_from(value.length).map_err(|_| memory_error())?;
        compressed = compressed.checked_add(length).ok_or_else(memory_error)?;
    }
    let table_len = usize::try_from(offset_table.length).map_err(|_| memory_error())?;
    // Frame ranges and the table value are bounded by the already counted and
    // budgeted fragment descriptors (at most one frame per fragment).
    let frames = group(
        reader,
        parsed,
        offset_table,
        table_len,
        fragments,
        image.frames,
        cancel,
    )?;
    let mut max_stream = 0usize;
    for range in &frames {
        // Bounded by `compressed`, which was summed with overflow checks.
        let length: usize = fragments[range.clone()]
            .iter()
            .map(|fragment| fragment.value.length as usize)
            .sum();
        max_stream = max_stream.max(length);
    }
    let image_bytes = image.image_bytes()?;
    // Simultaneous peak: offset table + frame ranges + one concatenated frame
    // codestream, the decoder's own copy of it and its copies of APPn metadata
    // (<= 2x more) + decoder row workspace + all decoded frames + typed Vec +
    // transient Vec-to-Arc copy. Lossless uses its full-frame workspace bound.
    // JPEG files are never deflated, so there is no
    // retained inflated backing beyond what `budget` already excludes.
    let peak = image_bytes
        .checked_mul(3)
        .and_then(|n| n.checked_add(max_stream.checked_mul(4)?))
        .and_then(|n| {
            n.checked_add(if lossless {
                lossless::workspace(&image)?
            } else {
                workspace(&image)?
            })
        })
        .and_then(|n| n.checked_add(table_len))
        .and_then(|n| n.checked_add(frames.len().checked_mul(size_of::<Range<usize>>())?))
        .ok_or_else(memory_error)?;
    if peak > budget || (lossless && peak > 256 * 1024 * 1024) {
        return Err("dicomread: decoded image exceeds the memory/output budget".into());
    }
    let units = image
        .len
        .checked_add(compressed.div_ceil(8))
        .ok_or_else(memory_error)?;
    Ok(Layout {
        image,
        frames,
        max_stream,
        units,
    })
}

/// Conservative bound for the decoder's per-MCU-row scratch of one interleaved
/// baseline scan (coefficient and upsampling rows, tables): O(width). Derived
/// from the row buffers of zune-jpeg 0.5.15, about 570 bytes per column for
/// 4:2:0 colour; this admits roughly 2.7 times that plus 256 KiB fixed.
fn workspace(image: &Image) -> Option<usize> {
    image
        .cols
        .checked_add(64)?
        .checked_mul(512)?
        .checked_mul(image.samples)?
        .checked_add(256 * 1024)
}

/// PS3.5 A.4: a frame may span several fragments, a fragment never holds two
/// frames. A nonempty Basic Offset Table names the first fragment item of each
/// frame; an empty one is only unambiguous for one frame or one fragment per
/// frame. No SOI/EOI byte searching is ever used to find frame boundaries.
pub(super) fn group<R: Read + Seek>(
    reader: &mut R,
    parsed: &ParsedFile,
    offset_table: parser::ValueRange,
    table_len: usize,
    fragments: &[Fragment],
    frames: usize,
    cancel: &AtomicBool,
) -> Result<Vec<Range<usize>>> {
    let count = fragments.len();
    let mut ranges = Vec::new();
    if table_len == 0 {
        if frames != 1 && count != frames {
            return Err(format!(
                "dicomread: JPEG frames with an empty Basic Offset Table need exactly one fragment per frame ({count} fragments for {frames} frames)"
            ));
        }
        ranges
            .try_reserve_exact(frames)
            .map_err(|_| "dicomread: unable to allocate JPEG frame ranges")?;
        if frames == 1 {
            ranges.push(0..count);
        } else {
            ranges.extend((0..frames).map(|index| index..index + 1));
        }
        return Ok(ranges);
    }
    if frames.checked_mul(4) != Some(table_len) {
        return Err(format!(
            "dicomread: Basic Offset Table has {} entries for {} frames",
            table_len / 4,
            frames
        ));
    }
    if frames > count {
        return Err(format!(
            "dicomread: Basic Offset Table has {frames} entries for only {count} fragments"
        ));
    }
    ranges
        .try_reserve_exact(frames)
        .map_err(|_| "dicomread: unable to allocate JPEG frame ranges")?;
    let table = parser::read_value(reader, offset_table, parsed.dataset_backing, cancel)?;
    // Entries must name fragment item tags in strictly increasing order and
    // the first frame starts at the first fragment (entry 00000000H).
    let mut next = 0usize;
    let mut starts = Vec::new();
    starts
        .try_reserve_exact(frames)
        .map_err(|_| "dicomread: unable to allocate JPEG frame ranges")?;
    for (index, entry) in table.as_chunks::<4>().0.iter().enumerate() {
        crate::dsp::check_cancel(cancel)?;
        let offset = u64::from(u32::from_le_bytes(*entry));
        while next < count && fragments[next].item_offset < offset {
            next += 1;
        }
        if next == count || fragments[next].item_offset != offset || (index == 0 && next != 0) {
            return Err(format!(
                "dicomread: Basic Offset Table entry {} does not point to the first fragment item of frame {}",
                index + 1,
                index + 1
            ));
        }
        starts.push(next);
        next += 1;
    }
    for (index, &start) in starts.iter().enumerate() {
        ranges.push(start..starts.get(index + 1).copied().unwrap_or(count));
    }
    Ok(ranges)
}

/// Concatenate each frame's fragments into one reused scratch buffer, validate
/// and decode it straight into its slice of the admitted output, then share
/// the column-major conversion. Decoder output is interleaved per pixel; the
/// native PlanarConfiguration never applies to it.
pub(crate) fn decode<R: Read + Seek>(
    reader: &mut R,
    parsed: &ParsedFile,
    layout: &Layout,
    cancel: &AtomicBool,
) -> Result<TypedArray> {
    crate::dsp::check_cancel(cancel)?;
    let Some(PixelLocation::Encapsulated { fragments, .. }) = &parsed.dataset.pixels else {
        return Err("dicomread: JPEG Baseline requires encapsulated PixelData".into());
    };
    let image = &layout.image;
    let image_bytes = image.image_bytes()?;
    let frame_bytes = image_bytes / image.frames;
    let mut out = Vec::new();
    out.try_reserve_exact(image_bytes)
        .map_err(|_| "dicomread: unable to allocate decoded JPEG frames")?;
    out.resize(image_bytes, 0);
    let mut scratch = Vec::new();
    scratch
        .try_reserve_exact(layout.max_stream)
        .map_err(|_| "dicomread: unable to allocate JPEG frame buffer")?;
    for (frame, range) in layout.frames.iter().enumerate() {
        crate::dsp::check_cancel(cancel)?;
        let fragments = fragments
            .get(range.clone())
            .ok_or("dicomread: JPEG frame range is outside the parsed fragments")?;
        read_frame(reader, fragments, &mut scratch, cancel)?;
        let decode_frame =
            |stream: &[u8], out: &mut [u8], image: &Image, cancel: &AtomicBool| match parsed
                .encoding
            {
                PixelEncoding::JpegLossless { selection_one } => {
                    lossless::decode_frame(stream, out, image, selection_one, cancel)
                }
                #[cfg(feature = "plot-export")]
                _ => decode_frame(stream, out, image, cancel),
                #[cfg(not(feature = "plot-export"))]
                _ => Err("dicomread: JPEG Baseline decoder is unavailable in this build".into()),
            };
        decode_frame(
            &scratch,
            &mut out[frame * frame_bytes..(frame + 1) * frame_bytes],
            image,
            cancel,
        )
        .map_err(|error| {
            if error.contains("cancelled") || image.frames == 1 {
                error
            } else {
                format!("{error} (frame {})", frame + 1)
            }
        })?;
    }
    drop(scratch);
    let result = pixels::convert_interleaved(&out, image, cancel);
    crate::dsp::check_cancel(cancel)?;
    result
}

pub(super) fn read_frame<R: Read + Seek>(
    reader: &mut R,
    fragments: &[Fragment],
    scratch: &mut Vec<u8>,
    cancel: &AtomicBool,
) -> Result<()> {
    scratch.clear();
    for fragment in fragments {
        crate::dsp::check_cancel(cancel)?;
        let length = usize::try_from(fragment.value.length).map_err(|_| memory_error())?;
        let start = scratch.len();
        let end = start
            .checked_add(length)
            .filter(|&end| end <= scratch.capacity())
            .ok_or("dicomread: JPEG frame exceeds its admitted buffer")?;
        scratch.resize(end, 0);
        reader
            .seek(SeekFrom::Start(fragment.value.offset))
            .map_err(|_| "dicomread: unable to seek JPEG fragment")?;
        for chunk in scratch[start..].chunks_mut(CHUNK) {
            crate::dsp::check_cancel(cancel)?;
            reader
                .read_exact(chunk)
                .map_err(|_| "dicomread: truncated or unreadable JPEG fragment")?;
        }
    }
    Ok(())
}

fn decoder_boundary<T>(kind: &str, decode: impl FnOnce() -> Result<T>) -> Result<T> {
    // Keep decoder construction, destruction and copying inside the boundary.
    // A partial output is discarded by the caller on error.
    std::panic::catch_unwind(std::panic::AssertUnwindSafe(decode))
        .map_err(|_| format!("dicomread: malformed JPEG {kind} frame: decoder panic"))?
}

#[cfg(feature = "plot-export")]
fn decode_frame(stream: &[u8], out: &mut [u8], image: &Image, cancel: &AtomicBool) -> Result<()> {
    use ::image::{ColorType, ImageDecoder, codecs::jpeg::JpegDecoder};
    let end = inspect(stream, image, cancel)?;
    crate::dsp::check_cancel(cancel)?;
    let fail = |error: ::image::ImageError| {
        format!("dicomread: cannot decode the JPEG Baseline frame: {error}")
    };
    decoder_boundary("Baseline", || {
        // Bounded reader: exactly the validated SOI..EOI bytes of this frame.
        let decoder = JpegDecoder::new(Cursor::new(&stream[..end])).map_err(fail)?;
        let (width, height) = decoder.dimensions();
        let colour = if image.samples == 1 {
            ColorType::L8
        } else {
            ColorType::Rgb8
        };
        if (height as usize, width as usize) != (image.rows, image.cols)
            || decoder.color_type() != colour
            || decoder.total_bytes() != out.len() as u64
        {
            return Err(
            "dicomread: JPEG decoder output does not match Rows x Columns x SamplesPerPixel at 8 bits"
                .into(),
        );
        }
        decoder.read_image(out).map_err(fail)
    })
}

/// T.81 Annex C/F.2.2.3 canonical decoding tables for one Huffman table.
struct Huffman {
    mincode: [i32; 17],
    maxcode: [i32; 17],
    valptr: [usize; 17],
    values: [u8; 256],
}

fn huffman(counts: &[u8], values: &[u8], dc: bool) -> Result<Huffman> {
    let mut table = Huffman {
        mincode: [0; 17],
        maxcode: [-1; 17],
        valptr: [0; 17],
        values: [0; 256],
    };
    table.values[..values.len()].copy_from_slice(values);
    let (mut code, mut next) = (0i32, 0usize);
    for length in 1..=16 {
        let count = usize::from(counts[length - 1]);
        table.mincode[length] = code;
        table.valptr[length] = next;
        if count != 0 {
            code += count as i32;
            next += count;
            // The all-ones code of every length stays a prefix of longer codes.
            if code >= 1 << length {
                return Err(bad("Huffman table code lengths are over-subscribed"));
            }
            table.maxcode[length] = code - 1;
        }
        code <<= 1;
    }
    // 8-bit baseline DC difference categories are 0..=11 (T.81 Table F.1).
    if dc && values.iter().any(|&value| value > 11) {
        return Err(bad("DC Huffman table has a category above 11"));
    }
    Ok(table)
}

/// MSB-first entropy bit reader. FF00 is a stuffed FF data byte; any other FFxx
/// is a marker, where the reader stops for good.
struct Bits<'a> {
    data: &'a [u8],
    pos: usize,
    acc: u64,
    n: u32,
}
impl Bits<'_> {
    fn fill(&mut self) {
        while self.n <= 56 {
            let Some(&byte) = self.data.get(self.pos) else {
                break;
            };
            if byte == 0xFF {
                if self.data.get(self.pos + 1) != Some(&0) {
                    break;
                }
                self.pos += 2;
            } else {
                self.pos += 1;
            }
            self.acc = (self.acc << 8) | u64::from(byte);
            self.n += 8;
        }
    }
    fn ended(&self) -> String {
        if self.pos + 1 >= self.data.len() {
            truncated()
        } else {
            bad("entropy-coded data ends at a marker before all blocks are decoded")
        }
    }
    fn symbol(&mut self, table: &Huffman) -> Result<u8> {
        if self.n < 16 {
            self.fill();
        }
        let window = if self.n >= 16 {
            (self.acc >> (self.n - 16)) & 0xFFFF
        } else {
            (self.acc << (16 - self.n)) & 0xFFFF
        } as i32;
        for length in 1..=16usize {
            let code = window >> (16 - length);
            if code <= table.maxcode[length] {
                if length as u32 > self.n {
                    return Err(self.ended());
                }
                self.n -= length as u32;
                return Ok(
                    table.values[table.valptr[length] + (code - table.mincode[length]) as usize]
                );
            }
        }
        Err(if self.n < 16 {
            self.ended()
        } else {
            bad("invalid Huffman code in entropy-coded data")
        })
    }
    fn skip(&mut self, bits: u32) -> Result<()> {
        if self.n < bits {
            self.fill();
            if self.n < bits {
                return Err(self.ended());
            }
        }
        self.n -= bits;
        Ok(())
    }
    /// End of a restart interval or of the scan: at most the 0..=7 pad bits of
    /// the last byte may remain, and a marker must follow.
    fn align(&mut self) -> Result<()> {
        self.fill();
        if self.n >= 8 {
            return Err(bad(
                "unused entropy-coded data after the last block of an interval",
            ));
        }
        self.n = 0;
        self.acc = 0;
        if self.pos >= self.data.len() {
            return Err(truncated());
        }
        Ok(())
    }
}

/// One marker code at `pos`, after optional FFH fill bytes (T.81 B.1.1.2).
fn marker(data: &[u8], pos: &mut usize) -> Result<u8> {
    match data.get(*pos) {
        None => return Err(truncated()),
        Some(0xFF) => {}
        Some(_) => return Err(bad("data found where a marker is required")),
    }
    let mut at = *pos + 1;
    while data.get(at) == Some(&0xFF) {
        at += 1;
    }
    let code = *data.get(at).ok_or_else(truncated)?;
    if code == 0 {
        return Err(bad("stuffed FF00 found where a marker is required"));
    }
    *pos = at + 1;
    Ok(code)
}

fn segment<'a>(data: &'a [u8], pos: &mut usize) -> Result<&'a [u8]> {
    let header = data.get(*pos..*pos + 2).ok_or_else(truncated)?;
    let length = usize::from(u16::from_be_bytes([header[0], header[1]]));
    if length < 2 {
        return Err(bad("marker segment length below 2"));
    }
    let body = data.get(*pos + 2..*pos + length).ok_or_else(truncated)?;
    *pos += length;
    Ok(body)
}

#[cfg(feature = "plot-export")]
struct Component {
    id: u8,
    h: usize,
    v: usize,
    quant: usize,
}

/// Strictly validate one concatenated frame codestream against the admitted
/// image and return the offset just past EOI. Accepts exactly: SOI, tables and
/// APPn/COM segments, one SOF0 frame header (8-bit, Rows x Columns,
/// SamplesPerPixel components, 4:4:4/4:2:2/4:2:0), one interleaved scan whose
/// every MCU block decodes with the declared Huffman tables (restart markers
/// in order), EOI, then nothing or one 0x00 fragment pad byte. Work is linear
/// in the stream length and polls cancellation.
#[cfg(feature = "plot-export")]
fn inspect(data: &[u8], image: &Image, cancel: &AtomicBool) -> Result<usize> {
    if data.get(..2) != Some(&[0xFF, 0xD8]) {
        return Err(bad("stream does not start with an SOI marker"));
    }
    let mut pos = 2usize;
    let mut quant = [false; 4];
    let mut tables: [[Option<Huffman>; 2]; 2] = [[None, None], [None, None]];
    let mut components: Vec<Component> = Vec::new();
    let mut restart = 0usize;
    let mut adobe: Option<u8> = None;
    let mut segments = 0usize;
    loop {
        segments += 1;
        if segments.is_multiple_of(64) {
            crate::dsp::check_cancel(cancel)?;
        }
        let code = marker(data, &mut pos)?;
        match code {
            0xC0 | 0xC4 | 0xDA | 0xDB | 0xDD | 0xE0..=0xEF | 0xFE => {}
            0xC1..=0xCF if code != 0xC8 && code != 0xCC => {
                let process = match code {
                    0xC1 => "extended sequential DCT",
                    0xC2 => "progressive DCT",
                    0xC3 => "lossless",
                    0xC5..=0xC7 => "differential",
                    _ => "arithmetic-coded",
                };
                return Err(format!(
                    "dicomread: JPEG frame uses {process} coding (SOF{}); transfer syntax {UID} requires baseline sequential DCT (SOF0)",
                    code - 0xC0
                ));
            }
            0xD9 => return Err(bad("EOI before any scan")),
            _ => {
                return Err(format!(
                    "dicomread: invalid JPEG Baseline frame: marker FF{code:02X} is not allowed before the scan"
                ));
            }
        }
        let body = segment(data, &mut pos)?;
        match code {
            0xC0 => {
                if !components.is_empty() {
                    return Err(bad("more than one frame header"));
                }
                if body.len() < 6 || body.len() != 6 + 3 * usize::from(body[5]) {
                    return Err(bad("malformed SOF0 frame header"));
                }
                if body[0] != 8 {
                    return Err(format!(
                        "dicomread: JPEG frame has sample precision {}; JPEG Baseline requires 8 bits",
                        body[0]
                    ));
                }
                let rows = usize::from(u16::from_be_bytes([body[1], body[2]]));
                let cols = usize::from(u16::from_be_bytes([body[3], body[4]]));
                let count = usize::from(body[5]);
                if count != image.samples {
                    return Err(format!(
                        "dicomread: JPEG frame has {count} components but SamplesPerPixel is {}",
                        image.samples
                    ));
                }
                if (rows, cols) != (image.rows, image.cols) {
                    return Err(format!(
                        "dicomread: JPEG frame is {rows}x{cols} but Rows x Columns is {}x{}",
                        image.rows, image.cols
                    ));
                }
                for entry in body[6..].as_chunks::<3>().0 {
                    let component = Component {
                        id: entry[0],
                        h: usize::from(entry[1] >> 4),
                        v: usize::from(entry[1] & 15),
                        quant: usize::from(entry[2]),
                    };
                    if component.quant > 3 || components.iter().any(|c| c.id == component.id) {
                        return Err(bad("malformed SOF0 component"));
                    }
                    components.push(component);
                }
                let sampling = |index: usize| (components[index].h, components[index].v);
                let supported = match count {
                    1 => sampling(0) == (1, 1),
                    _ => {
                        matches!(sampling(0), (1, 1) | (2, 1) | (2, 2))
                            && sampling(1) == (1, 1)
                            && sampling(2) == (1, 1)
                    }
                };
                if !supported {
                    return Err("dicomread: unsupported JPEG sampling factors; only 1x1 grayscale and 4:4:4, 4:2:2 or 4:2:0 colour are supported yet".into());
                }
            }
            0xC4 => {
                let mut rest = body;
                while !rest.is_empty() {
                    let (head, tail) = rest
                        .split_at_checked(17)
                        .ok_or_else(|| bad("malformed Huffman table segment"))?;
                    let (class, id) = (usize::from(head[0] >> 4), usize::from(head[0] & 15));
                    let total: usize = head[1..].iter().map(|&n| usize::from(n)).sum();
                    if class > 1 || id > 1 || total > 256 || total > tail.len() {
                        return Err(bad(
                            "malformed Huffman table (baseline allows DC/AC tables 0 and 1)",
                        ));
                    }
                    tables[class][id] = Some(huffman(&head[1..], &tail[..total], class == 0)?);
                    rest = &tail[total..];
                }
            }
            0xDB => {
                let (entries, rest) = body.as_chunks::<65>();
                if !rest.is_empty() || entries.is_empty() {
                    return Err(bad(
                        "malformed quantization table segment (baseline tables are 8-bit)",
                    ));
                }
                for table in entries {
                    // High nibble is the precision: baseline requires 8-bit entries.
                    if table[0] > 3 {
                        return Err(bad("malformed or 16-bit quantization table"));
                    }
                    quant[usize::from(table[0])] = true;
                }
            }
            0xDD => {
                if body.len() != 2 {
                    return Err(bad("malformed restart interval segment"));
                }
                restart = usize::from(u16::from_be_bytes([body[0], body[1]]));
            }
            0xEE if body.starts_with(b"Adobe") => {
                // The decoder takes the colour transform from this segment.
                if body.len() < 12 || body[11] > 1 || !components.is_empty() {
                    return Err(bad(
                        "unsupported Adobe APP14 segment (transform must be 0 or 1, before the frame header)",
                    ));
                }
                adobe = Some(body[11]);
            }
            0xDA => {
                let count = components.len();
                if count == 0 {
                    return Err(bad("scan before the frame header"));
                }
                if body.first().map(|&n| usize::from(n)) != Some(count)
                    || body.len() != 4 + 2 * count
                {
                    return Err("dicomread: unsupported JPEG scan layout; only one interleaved baseline scan with all components is supported yet".into());
                }
                colour(image, &components, adobe)?;
                let mut blocks: Vec<(&Huffman, &Huffman)> = Vec::new();
                for (component, entry) in components.iter().zip(body[1..].as_chunks::<2>().0) {
                    let (dc, ac) = (usize::from(entry[1] >> 4), usize::from(entry[1] & 15));
                    if entry[0] != component.id || dc > 1 || ac > 1 {
                        return Err(bad("malformed scan component selector"));
                    }
                    if !quant[component.quant] {
                        return Err(bad("scan uses an undefined quantization table"));
                    }
                    let (Some(dc), Some(ac)) = (&tables[0][dc], &tables[1][ac]) else {
                        return Err(bad("scan uses an undefined Huffman table"));
                    };
                    blocks.extend(std::iter::repeat_n((dc, ac), component.h * component.v));
                }
                if body[1 + 2 * count..] != [0, 63, 0] {
                    return Err(bad(
                        "scan is not baseline sequential (Ss=0, Se=63, Ah=Al=0)",
                    ));
                }
                let mcus = image.cols.div_ceil(8 * components[0].h)
                    * image.rows.div_ceil(8 * components[0].v);
                pos = scan(data, pos, &blocks, mcus, restart, cancel)?;
                let code = marker(data, &mut pos)?;
                if code != 0xD9 {
                    return Err(format!(
                        "dicomread: unsupported JPEG frame: marker FF{code:02X} follows the scan; only one interleaved baseline scan followed by EOI is supported yet"
                    ));
                }
                return match &data[pos..] {
                    [] | [0] => Ok(pos),
                    _ => Err(bad(
                        "unexpected data after EOI (only one 0x00 fragment pad byte is allowed)",
                    )),
                };
            }
            // Other APPn (JFIF, Exif, ICC) and COM segments carry no pixel rules.
            _ => {}
        }
    }
}

/// The decoder converts three components from YCbCr to RGB unless the stream
/// signals RGB data: Adobe APP14 transform 0, or component ids 'R','G','B'.
/// PS3.5 8.2.1: the JPEG interchange format does not define the colour space,
/// so PhotometricInterpretation decides; a stream that would be decoded the
/// other way is rejected instead of returning wrongly converted samples.
#[cfg(feature = "plot-export")]
fn colour(image: &Image, components: &[Component], adobe: Option<u8>) -> Result<()> {
    if components.len() != 3 {
        return Ok(());
    }
    let ids = [components[0].id, components[1].id, components[2].id] == *b"RGB";
    if ids && adobe == Some(1) {
        return Err(bad(
            "contradictory colour signalling (R,G,B component ids with Adobe APP14 transform 1)",
        ));
    }
    let rgb = ids || adobe == Some(0);
    match (image.ybr, rgb) {
        (true, true) => Err("dicomread: JPEG stream signals untransformed RGB components (Adobe APP14 transform 0 or R,G,B ids), contradicting PhotometricInterpretation YBR_FULL/YBR_FULL_422".into()),
        (false, false) => Err("dicomread: PhotometricInterpretation RGB with a JPEG Baseline stream that does not signal RGB components (Adobe APP14 transform 0) is not supported yet; the decoder would apply a YCbCr conversion".into()),
        _ => Ok(()),
    }
}

/// Walk the Huffman-coded blocks of every MCU (T.81 F.2.2) without building
/// coefficients; returns the offset of the marker that ends the scan.
#[cfg(feature = "plot-export")]
fn scan(
    data: &[u8],
    start: usize,
    blocks: &[(&Huffman, &Huffman)],
    mcus: usize,
    restart: usize,
    cancel: &AtomicBool,
) -> Result<usize> {
    let mut bits = Bits {
        data,
        pos: start,
        acc: 0,
        n: 0,
    };
    let mut expected = 0u8;
    for mcu in 0..mcus {
        if mcu.is_multiple_of(POLL) {
            crate::dsp::check_cancel(cancel)?;
        }
        if restart != 0 && mcu != 0 && mcu.is_multiple_of(restart) {
            bits.align()?;
            let code = marker(data, &mut bits.pos)?;
            if code != 0xD0 + expected {
                return Err(format!(
                    "dicomread: invalid JPEG Baseline frame: expected restart marker RST{expected} in entropy-coded data, found FF{code:02X}"
                ));
            }
            expected = (expected + 1) & 7;
        }
        for (dc, ac) in blocks {
            let size = bits.symbol(dc)?;
            bits.skip(u32::from(size))?;
            let mut index = 1usize;
            while index < 64 {
                let symbol = bits.symbol(ac)?;
                let (run, size) = (usize::from(symbol >> 4), u32::from(symbol & 15));
                if size == 0 {
                    match run {
                        0 => break,
                        15 => index += 16,
                        _ => return Err(bad("invalid AC symbol in entropy-coded data")),
                    }
                } else {
                    index += run + 1;
                    bits.skip(size)?;
                }
                // 8-bit baseline AC magnitudes have at most 10 bits; a run may
                // not pass coefficient 63 (after ZRL one must still follow).
                if size > 10 || index > 64 || (size == 0 && index > 63) {
                    return Err(bad("invalid AC coefficient in entropy-coded data"));
                }
            }
        }
    }
    bits.align()?;
    Ok(bits.pos)
}

#[cfg(test)]
mod decoder_tests {
    use super::decoder_boundary;

    #[test]
    fn decoder_panics_become_malformed_errors() {
        let result = decoder_boundary::<()>("Baseline", || panic!("crafted decoder panic"));
        assert!(result.unwrap_err().contains("malformed"));
    }
}

#[cfg(all(test, feature = "plot-export"))]
mod tests {
    use super::*;

    use std::sync::atomic::Ordering;

    const GRAY: &[u8] = include_bytes!("../../../tests/fixtures/dicom/jpeg_baseline.dcm");
    const FRAMES: &[u8] = include_bytes!("../../../tests/fixtures/dicom/jpeg_gray_frames_bot.dcm");

    struct Guarded<'a> {
        input: Cursor<&'static [u8]>,
        forbidden: Vec<Range<u64>>,
        cancel_after: Option<(usize, &'a AtomicBool)>,
        reads: usize,
    }
    impl Read for Guarded<'_> {
        fn read(&mut self, out: &mut [u8]) -> std::io::Result<usize> {
            let (start, end) = (
                self.input.position(),
                self.input.position() + out.len() as u64,
            );
            assert!(
                self.forbidden
                    .iter()
                    .all(|r| end <= r.start || start >= r.end),
                "read {start}..{end} touches a fragment payload"
            );
            self.reads += 1;
            if let Some((after, cancel)) = self.cancel_after
                && self.reads >= after
            {
                cancel.store(true, Ordering::Relaxed);
            }
            self.input.read(out)
        }
    }
    impl Seek for Guarded<'_> {
        fn seek(&mut self, pos: SeekFrom) -> std::io::Result<u64> {
            self.input.seek(pos)
        }
    }
    fn payloads(parsed: &ParsedFile) -> Vec<Range<u64>> {
        let Some(PixelLocation::Encapsulated { fragments, .. }) = &parsed.dataset.pixels else {
            panic!("not encapsulated");
        };
        fragments
            .iter()
            .map(|f| f.value.offset..f.value.offset + f.value.length)
            .collect()
    }

    #[test]
    fn parse_and_admission_never_read_fragment_payloads() {
        let cancel = AtomicBool::new(false);
        for bytes in [GRAY, FRAMES] {
            let mut input = Cursor::new(bytes);
            let parsed = parser::parse(&mut input, bytes.len() as u64, 1 << 20, &cancel).unwrap();
            assert_eq!(parsed.encoding, PixelEncoding::Jpeg);
            let forbidden = payloads(&parsed);
            assert!(!forbidden.is_empty());
            let mut reader = Guarded {
                input: Cursor::new(bytes),
                forbidden,
                cancel_after: None,
                reads: 0,
            };
            // The dicominfo path: structural parse only, payloads seeked over.
            let parsed = parser::parse(&mut reader, bytes.len() as u64, 1 << 20, &cancel).unwrap();
            // dicomread admission reads header values and the offset table only.
            layout(&mut reader, &parsed, 1 << 20, 1 << 20, &cancel).unwrap();
        }
    }

    #[test]
    fn admission_boundaries_grouping_and_cancellation() {
        let cancel = AtomicBool::new(false);
        let mut input = Cursor::new(FRAMES);
        let parsed = parser::parse(&mut input, FRAMES.len() as u64, 1 << 20, &cancel).unwrap();
        let plan = layout(&mut input, &parsed, 1 << 20, 1 << 20, &cancel).unwrap();
        // 3 frames of 8x16 uint8, two fragments each, 12-byte offset table.
        assert_eq!(plan.frames, [0..2, 2..4, 4..6]);
        assert_eq!(plan.image.len, 384);
        let exact = 3 * 384
            + 4 * plan.max_stream
            + workspace(&plan.image).unwrap()
            + 12
            + 3 * size_of::<Range<usize>>();
        let error = layout(&mut input, &parsed, exact - 1, 1 << 20, &cancel)
            .err()
            .unwrap();
        assert!(error.contains("memory/output budget"), "{error}");
        let error = layout(&mut input, &parsed, exact, 383, &cancel)
            .err()
            .unwrap();
        assert!(error.contains("element budget"), "{error}");
        let plan = layout(&mut input, &parsed, exact, 384, &cancel).unwrap();
        let compressed: u64 = payloads(&parsed).iter().map(|r| r.end - r.start).sum();
        assert_eq!(plan.units, 384 + (compressed as usize).div_ceil(8));
        let image = decode(&mut input, &parsed, &plan, &cancel).unwrap();
        assert_eq!(image.len(), 384);
        // Cancellation during fragment concatenation: the first payload read
        // sets the flag, the next fragment/chunk poll stops the decode.
        let mut reader = Guarded {
            input: Cursor::new(FRAMES),
            forbidden: Vec::new(),
            cancel_after: Some((1, &cancel)),
            reads: 0,
        };
        let error = decode(&mut reader, &parsed, &plan, &cancel).unwrap_err();
        assert!(error.contains("cancelled"), "{error}");
        assert_eq!(reader.reads, 1);
        // Flag set once frame 1 (two fragments) is concatenated: validation stops.
        cancel.store(false, Ordering::Relaxed);
        let mut reader = Guarded {
            input: Cursor::new(FRAMES),
            forbidden: Vec::new(),
            cancel_after: Some((2, &cancel)),
            reads: 0,
        };
        let error = decode(&mut reader, &parsed, &plan, &cancel).unwrap_err();
        assert!(error.contains("cancelled"), "{error}");
        assert_eq!(reader.reads, 2);
        // Validation of one frame codestream polls cancellation itself.
        cancel.store(false, Ordering::Relaxed);
        let mut input = Cursor::new(GRAY);
        let parsed = parser::parse(&mut input, GRAY.len() as u64, 1 << 20, &cancel).unwrap();
        let plan = layout(&mut input, &parsed, 1 << 20, 1 << 20, &cancel).unwrap();
        let range = payloads(&parsed)[0].clone();
        let frame = &GRAY[range.start as usize..range.end as usize];
        assert!(inspect(frame, &plan.image, &cancel).is_ok());
        cancel.store(true, Ordering::Relaxed);
        let error = inspect(frame, &plan.image, &cancel).unwrap_err();
        assert!(error.contains("cancelled"), "{error}");
    }

    #[test]
    fn huffman_tables_and_bit_reader_are_strict() {
        // Two 1-bit codes use the reserved all-ones code.
        let mut counts = [0u8; 16];
        counts[0] = 2;
        assert!(huffman(&counts, &[0, 1], true).is_err());
        // Codes 0, 10, 110: canonical decode, stuffed FF and marker stop.
        let counts = [1, 1, 1, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0];
        assert!(huffman(&counts, &[0, 1, 12], true).is_err());
        let table = huffman(&counts, &[5, 6, 7], false).unwrap();
        let data = [0b0101_1010, 0xFF, 0x00, 0xFF, 0xD9];
        let mut bits = Bits {
            data: &data,
            pos: 0,
            acc: 0,
            n: 0,
        };
        assert_eq!(bits.symbol(&table).unwrap(), 5);
        assert_eq!(bits.symbol(&table).unwrap(), 6);
        assert_eq!(bits.symbol(&table).unwrap(), 7);
        assert_eq!(bits.symbol(&table).unwrap(), 6);
        // Remaining: eight stuffed one bits; all-ones is never a code, and a
        // whole unused byte before the marker is not padding.
        let error = bits.symbol(&table).unwrap_err();
        assert!(error.contains("ends at a marker"), "{error}");
        assert!(bits.align().unwrap_err().contains("unused"));
        bits.skip(8).unwrap();
        bits.align().unwrap();
        assert_eq!(marker(&data, &mut bits.pos).unwrap(), 0xD9);
        assert_eq!(bits.pos, data.len());
        // Fill bytes before a marker are skipped; FF00 is not a marker.
        let mut pos = 0;
        assert_eq!(marker(&[0xFF, 0xFF, 0xFF, 0xC0], &mut pos).unwrap(), 0xC0);
        assert!(marker(&[0xFF, 0x00], &mut 0).is_err());
        assert!(
            marker(&[0xFF, 0xFF], &mut 0)
                .unwrap_err()
                .contains("truncated")
        );
    }
}
