//! RLE Lossless (1.2.840.10008.1.2.5) frames from checked encapsulated fragments.
//! PS3.5 A.4/A.4.2 frame admission and Annex G segments/PackBits, pure Rust.
//! Decoded segments are normalized to the shared planar little-endian layout;
//! BitsStored masking, sign extension and column-major output stay shared.
use super::parser::{self, ElementValue, Fragment, ParsedFile, PixelEncoding, PixelLocation};
use super::pixels::{self, Image};
use crate::{Result, typed_array::TypedArray};
use std::{
    io::{Read, Seek, SeekFrom},
    sync::atomic::AtomicBool,
};

const HEADER: usize = 64;
const MAX_SEGMENTS: usize = 15;
const CHUNK: usize = 64 * 1024;
const POLL: usize = 4096;

/// Admitted RLE decode plan. Created before any fragment payload byte is read.
pub(crate) struct Layout {
    image: Image,
    bytes_per_sample: usize,
    max_fragment: usize,
    /// Work units to charge before decoding: output cells + compressed bytes/8.
    pub(crate) units: usize,
}

/// Shared image admission, then RLE-only transport, fragment/frame and memory
/// admission. Reads only the Basic Offset Table value (at most 4*frames bytes).
pub(crate) fn layout<R: Read + Seek>(
    reader: &mut R,
    parsed: &ParsedFile,
    budget: usize,
    max_elements: usize,
    cancel: &AtomicBool,
) -> Result<Layout> {
    if parsed.encoding != PixelEncoding::Rle {
        return Err("dicomread: RLE decoder requires the RLE Lossless transfer syntax".into());
    }
    let image = pixels::image(reader, parsed, max_elements, cancel)?;
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
        _ => {
            return Err(
                "dicomread: RLE Lossless requires undefined-length encapsulated OB PixelData"
                    .into(),
            );
        }
    };
    let bytes_per_sample = usize::from(image.allocated / 8);
    // PS3.5 A.4.2: each frame is encoded in one and only one fragment.
    if fragments.len() != image.frames {
        return Err(format!(
            "dicomread: RLE requires exactly one fragment per frame ({} fragments for {} frames)",
            fragments.len(),
            image.frames
        ));
    }
    let mut max_fragment = 0usize;
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
            return Err("dicomread: RLE fragment range exceeds admitted backing size".into());
        }
        let length = usize::try_from(value.length).map_err(|_| memory_error())?;
        if length < HEADER {
            return Err("dicomread: RLE fragment is shorter than its 64-byte header".into());
        }
        max_fragment = max_fragment.max(length);
        compressed = compressed.checked_add(length).ok_or_else(memory_error)?;
    }
    let table_len = usize::try_from(offset_table.length).map_err(|_| memory_error())?;
    let image_bytes = image.image_bytes()?;
    // Simultaneous peak: offset table + one compressed frame scratch + normalized
    // decoded planes + typed Vec + transient Vec-to-Arc copy.
    let peak = image_bytes
        .checked_mul(3)
        .and_then(|n| n.checked_add(max_fragment))
        .and_then(|n| n.checked_add(table_len))
        .ok_or_else(memory_error)?;
    if peak > budget {
        return Err("dicomread: decoded image exceeds the memory/output budget".into());
    }
    if table_len != 0 {
        let frames4 = image.frames.checked_mul(4).ok_or_else(memory_error)?;
        if table_len != frames4 {
            return Err(format!(
                "dicomread: Basic Offset Table has {} entries for {} frames",
                table_len / 4,
                image.frames
            ));
        }
        let table = parser::read_value(reader, offset_table, parsed.dataset_backing, cancel)?;
        for (index, (entry, fragment)) in table.as_chunks::<4>().0.iter().zip(fragments).enumerate()
        {
            let offset = u32::from_le_bytes(*entry);
            if u64::from(offset) != fragment.item_offset {
                return Err(format!(
                    "dicomread: Basic Offset Table entry {} does not point to fragment item {}",
                    index + 1,
                    index + 1
                ));
            }
        }
    }
    let units = image
        .len
        .checked_add(compressed.div_ceil(8))
        .ok_or_else(memory_error)?;
    Ok(Layout {
        image,
        bytes_per_sample,
        max_fragment,
        units,
    })
}

fn memory_error() -> String {
    "dicomread: RLE memory accounting overflow".into()
}

/// Read each fragment into one reused scratch buffer, decode its segments into
/// the normalized planes, then share BitsStored and column-major conversion.
pub(crate) fn decode<R: Read + Seek>(
    reader: &mut R,
    parsed: &ParsedFile,
    layout: &Layout,
    cancel: &AtomicBool,
) -> Result<TypedArray> {
    crate::dsp::check_cancel(cancel)?;
    let Some(PixelLocation::Encapsulated { fragments, .. }) = &parsed.dataset.pixels else {
        return Err("dicomread: RLE Lossless requires encapsulated PixelData".into());
    };
    let image = &layout.image;
    let frame_bytes = image.image_bytes()? / image.frames;
    let mut out = Vec::new();
    out.try_reserve_exact(image.image_bytes()?)
        .map_err(|_| "dicomread: unable to allocate decoded RLE planes")?;
    out.resize(image.image_bytes()?, 0);
    let mut scratch = Vec::new();
    scratch
        .try_reserve_exact(layout.max_fragment)
        .map_err(|_| "dicomread: unable to allocate RLE fragment buffer")?;
    for (frame, fragment) in fragments.iter().enumerate() {
        read_fragment(reader, fragment, &mut scratch, cancel)?;
        decode_frame(
            &scratch,
            &mut out[frame * frame_bytes..(frame + 1) * frame_bytes],
            image,
            layout.bytes_per_sample,
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
    let result = pixels::convert_planes(&out, image, cancel);
    crate::dsp::check_cancel(cancel)?;
    result
}

fn read_fragment<R: Read + Seek>(
    reader: &mut R,
    fragment: &Fragment,
    scratch: &mut Vec<u8>,
    cancel: &AtomicBool,
) -> Result<()> {
    crate::dsp::check_cancel(cancel)?;
    let length = usize::try_from(fragment.value.length).map_err(|_| memory_error())?;
    if length > scratch.capacity() {
        return Err("dicomread: RLE fragment exceeds its admitted buffer".into());
    }
    scratch.clear();
    scratch.resize(length, 0);
    reader
        .seek(SeekFrom::Start(fragment.value.offset))
        .map_err(|_| "dicomread: unable to seek RLE fragment")?;
    for chunk in scratch.chunks_mut(CHUNK) {
        crate::dsp::check_cancel(cancel)?;
        reader
            .read_exact(chunk)
            .map_err(|_| "dicomread: truncated or unreadable RLE fragment")?;
    }
    Ok(())
}

/// PS3.5 G.5 header: 16 little-endian ULs; segment count, then offsets. The
/// count must equal SamplesPerPixel*BytesPerSample (G.2); the first offset is
/// 64, offsets strictly increase inside the fragment and unused entries are 0.
/// Segment i covers [offset i, offset i+1) or [offset n, fragment end).
fn decode_frame(
    frame: &[u8],
    out: &mut [u8],
    image: &Image,
    bytes_per_sample: usize,
    cancel: &AtomicBool,
) -> Result<()> {
    crate::dsp::check_cancel(cancel)?;
    if frame.len() < HEADER {
        return Err("dicomread: RLE fragment is shorter than its 64-byte header".into());
    }
    let word = |i: usize| u32::from_le_bytes(frame[4 * i..4 * i + 4].try_into().unwrap());
    let count = word(0) as usize;
    let expected = image.samples * bytes_per_sample;
    if count != expected || count > MAX_SEGMENTS {
        return Err(format!(
            "dicomread: RLE header declares {count} segments; expected {expected} (SamplesPerPixel*BitsAllocated/8)"
        ));
    }
    if (count + 1..16).any(|i| word(i) != 0) {
        return Err("dicomread: unused RLE header segment offsets must be zero".into());
    }
    let mut starts = [0usize; MAX_SEGMENTS + 1];
    for i in 0..count {
        starts[i] = word(i + 1) as usize;
        let valid = if i == 0 {
            starts[0] == HEADER
        } else {
            starts[i] > starts[i - 1]
        };
        if !valid || starts[i] >= frame.len() {
            return Err("dicomread: invalid RLE segment offsets (first must be 64, then strictly increasing within the fragment)".into());
        }
    }
    starts[count] = frame.len();
    for segment in 0..count {
        // G.2: MSB byte segments first, then the next byte, for each sample in
        // order. Normalized 16-bit cells are little-endian: MSB goes to +1.
        let sample = segment / bytes_per_sample;
        let shift = bytes_per_sample - 1 - segment % bytes_per_sample;
        let base = sample * image.plane_len * bytes_per_sample + shift;
        packbits(
            &frame[starts[segment]..starts[segment + 1]],
            out,
            base,
            bytes_per_sample,
            image.rows,
            image.cols,
            cancel,
        )
        .map_err(|error| {
            if error.contains("cancelled") {
                error
            } else {
                format!("{error} (segment {})", segment + 1)
            }
        })?;
    }
    Ok(())
}

/// PS3.5 G.3.1 PackBits. Control n in 0..=127: n+1 literal bytes; -127..=-1:
/// repeat the next byte 1-n times; -128: no-op. Every row is encoded separately
/// (G.3.1), so a run crossing a row end is rejected, as is any run past the
/// plane or a segment that ends before rows*cols bytes. G.3.1 segments are
/// even-length: after the plane is complete exactly one 0x00 pad follows an
/// odd consumed byte count and nothing follows an even one. Work is bounded by the
/// segment length (each step consumes at least one input byte) and polls
/// cancellation every 4096 steps.
fn packbits(
    segment: &[u8],
    out: &mut [u8],
    base: usize,
    stride: usize,
    rows: usize,
    cols: usize,
    cancel: &AtomicBool,
) -> Result<()> {
    let total = rows * cols;
    let (mut position, mut produced, mut row_end, mut steps) = (0usize, 0usize, cols, 0usize);
    while produced < total {
        steps += 1;
        if steps.is_multiple_of(POLL) {
            crate::dsp::check_cancel(cancel)?;
        }
        let control = *segment
            .get(position)
            .ok_or("dicomread: RLE segment ends before its plane is filled")?
            as i8;
        position += 1;
        let count = match control {
            -128 => continue,
            0..=127 => control as usize + 1,
            _ => (1 - i16::from(control)) as usize,
        };
        if produced + count > row_end {
            return Err("dicomread: RLE run crosses a row boundary or overfills its plane".into());
        }
        if control >= 0 {
            let literal = segment
                .get(position..position + count)
                .ok_or("dicomread: truncated RLE literal run")?;
            for (k, &byte) in literal.iter().enumerate() {
                out[base + (produced + k) * stride] = byte;
            }
            position += count;
        } else {
            let byte = *segment
                .get(position)
                .ok_or("dicomread: truncated RLE replicate run")?;
            for k in 0..count {
                out[base + (produced + k) * stride] = byte;
            }
            position += 1;
        }
        produced += count;
        if produced == row_end {
            row_end += cols;
        }
    }
    match &segment[position..] {
        [] if position.is_multiple_of(2) => Ok(()),
        [0] if position % 2 == 1 => Ok(()),
        _ => Err("dicomread: invalid RLE segment end (segments are even-length; after the plane only one zero pad byte is allowed)".into()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::Ordering;

    fn run(segment: &[u8], rows: usize, cols: usize) -> Result<Vec<u8>> {
        let mut out = vec![0; rows * cols];
        packbits(segment, &mut out, 0, 1, rows, cols, &AtomicBool::new(false)).map(|_| out)
    }

    #[test]
    fn minimal_replicate_exact_bytes() {
        assert_eq!(run(&[0xff, 7], 1, 2).unwrap(), [7, 7]);
    }

    #[test]
    fn packbits_controls_noop_rows_and_padding() {
        // literal 3, replicate 3 (-2), no-op between runs and as first byte.
        assert_eq!(
            run(&[0x80, 2, 1, 2, 3, 0x80, 0xFE, 9], 2, 3).unwrap(),
            [1, 2, 3, 9, 9, 9]
        );
        // 128-byte replicate (-127) and 128-byte literal (127).
        assert_eq!(run(&[0x81, 7], 1, 128).unwrap(), [7; 128]);
        let mut literal = vec![127];
        literal.extend(0..128);
        literal.push(0); // 129 consumed bytes: G.3.1 even-length pad
        assert_eq!(
            run(&literal, 1, 128).unwrap(),
            (0..128).collect::<Vec<u8>>()
        );
        // Odd consumed count: exactly one zero pad required; even: none.
        assert_eq!(run(&[1, 5, 6, 0, 7, 0], 1, 3).unwrap(), [5, 6, 7]);
        assert_eq!(run(&[0, 5, 0xFF, 6], 1, 3).unwrap(), [5, 6, 6]);
        for bad in [
            &[1, 5, 6, 0, 7][..],
            &[1, 5, 6, 0, 7, 1],
            &[1, 5, 6, 0, 7, 0x80],
            &[1, 5, 6, 0, 7, 0, 0],
            &[0, 5, 0xFF, 6, 0],
            &[0xFE, 6, 0],
            &[0xFE, 6, 0x80],
        ] {
            assert!(
                run(bad, 1, 3).unwrap_err().contains("segment end"),
                "{bad:?}"
            );
        }
        // Row crossing, plane overfill, underfill and truncation.
        assert!(run(&[0xFD, 1], 2, 3).unwrap_err().contains("row boundary"));
        assert!(
            run(&[3, 1, 2, 3, 4], 1, 3)
                .unwrap_err()
                .contains("row boundary")
        );
        assert!(run(&[1, 1, 2], 1, 3).unwrap_err().contains("ends before"));
        assert!(run(&[2, 1, 2], 1, 3).unwrap_err().contains("literal"));
        assert!(run(&[0xFE], 1, 3).unwrap_err().contains("replicate"));
        assert!(run(&[0x80; 64], 1, 3).unwrap_err().contains("ends before"));
    }

    #[test]
    fn frame_admission_budget_boundary_and_cancellable_fragment_reads() {
        use std::io::Cursor;
        const FRAMES: &[u8] = include_bytes!("../../../tests/fixtures/dicom/rle_frames_bot.dcm");
        struct CancelOnRead<'a> {
            input: Cursor<&'static [u8]>,
            cancel: &'a AtomicBool,
            reads: usize,
        }
        impl Read for CancelOnRead<'_> {
            fn read(&mut self, out: &mut [u8]) -> std::io::Result<usize> {
                self.reads += 1;
                self.cancel.store(true, Ordering::Relaxed);
                self.input.read(out)
            }
        }
        impl Seek for CancelOnRead<'_> {
            fn seek(&mut self, pos: SeekFrom) -> std::io::Result<u64> {
                self.input.seek(pos)
            }
        }
        let cancel = AtomicBool::new(false);
        let mut input = Cursor::new(FRAMES);
        let parsed = parser::parse(&mut input, FRAMES.len() as u64, 1 << 20, &cancel).unwrap();
        // 3 frames of 2x3 uint16: 3*36 output bytes + one 80-byte fragment + 12-byte BOT.
        let error = layout(&mut input, &parsed, 199, 1 << 20, &cancel)
            .err()
            .unwrap();
        assert!(error.contains("memory/output budget"), "{error}");
        let error = layout(&mut input, &parsed, 200, 17, &cancel).err().unwrap();
        assert!(error.contains("element budget"), "{error}");
        let plan = layout(&mut input, &parsed, 200, 18, &cancel).unwrap();
        assert_eq!((plan.max_fragment, plan.units), (80, 18 + 30));
        let image = decode(&mut input, &parsed, &plan, &cancel).unwrap();
        assert_eq!(image.len(), 18);
        let mut reader = CancelOnRead {
            input,
            cancel: &cancel,
            reads: 0,
        };
        let error = decode(&mut reader, &parsed, &plan, &cancel).unwrap_err();
        assert!(error.contains("cancelled"), "{error}");
        assert_eq!(reader.reads, 1);
    }

    #[test]
    fn packbits_noops_poll_cancellation() {
        let cancel = AtomicBool::new(true);
        let mut out = [0; 1];
        let error = packbits(&[0x80; 2 * POLL], &mut out, 0, 1, 1, 1, &cancel).unwrap_err();
        assert!(error.contains("cancelled"), "{error}");
        cancel.store(false, Ordering::Relaxed);
        let mut noops = vec![0x80; POLL - 2];
        noops.extend([0, 4]);
        assert!(packbits(&noops, &mut out, 0, 1, 1, 1, &cancel).is_ok());
        assert_eq!(out, [4]);
    }
}
