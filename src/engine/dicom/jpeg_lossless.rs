//! T.81 Annex H SOF3: validate every lossless difference before the bounded
//! jpeg-decoder call. Frame grouping and allocation admission live in jpeg.rs.
use super::{Bits, Huffman, Image, POLL, Result, huffman, marker, segment};
use std::{io::Read, sync::atomic::AtomicBool};

fn bad(reason: &str) -> String {
    format!("dicomread: invalid JPEG Lossless frame: {reason}")
}

pub(super) fn workspace(image: &Image) -> Option<usize> {
    // jpeg-decoder 0.3.2: i32 differences + u16 result planes = 6/sample.
    // Colour interleaving / native-endian byte conversion peaks at 8/sample.
    // Include fixed tables/metadata. Shared admission separately counts all
    // output frames, typed storage, codestream and APP metadata copies.
    image
        .len
        .checked_div(image.frames)?
        .checked_mul(8)?
        .checked_add(256 * 1024)
}

struct Header {
    end: usize,
    precision_at: usize,
    point_at: usize,
    point: u8,
}

/// All headers and all entropy symbols must be complete. A decoder that pads
/// premature entropy endings with zero bits can otherwise return a full-sized
/// but partial image. This walk allocates tables only, never sample planes.
fn inspect(data: &[u8], image: &Image, selection_one: bool, cancel: &AtomicBool) -> Result<Header> {
    crate::dsp::check_cancel(cancel)?;
    if data.get(..2) != Some(&[0xFF, 0xD8]) {
        return Err(bad("stream does not start with SOI"));
    }
    let mut pos = 2;
    let mut precision_at = 0;
    let mut ids = Vec::new();
    let mut tables: [Option<Huffman>; 4] = [None, None, None, None];
    let mut segments = 0usize;
    loop {
        segments += 1;
        if segments.is_multiple_of(64) {
            crate::dsp::check_cancel(cancel)?;
        }
        let code = marker(data, &mut pos)?;
        if matches!(code, 0xC0..=0xCF) && !matches!(code, 0xC3 | 0xC4) {
            return Err(bad(&format!(
                "SOF3 is required by JPEG Lossless {}; found marker FF{code:02X}",
                if selection_one {
                    "1.2.840.10008.1.2.4.70"
                } else {
                    "1.2.840.10008.1.2.4.57"
                }
            )));
        }
        if !matches!(code, 0xC3 | 0xC4 | 0xDA | 0xDD | 0xE0..=0xEF | 0xFE) {
            return Err(bad(&format!(
                "marker FF{code:02X} is not allowed before the scan"
            )));
        }
        let body_at = pos + 2;
        let body = segment(data, &mut pos)?;
        match code {
            0xC3 => {
                if !ids.is_empty() || body.len() < 6 || body.len() != 6 + 3 * usize::from(body[5]) {
                    return Err(bad("malformed or duplicate SOF3 header"));
                }
                if u16::from(body[0]) != image.stored {
                    return Err(bad("SOF3 precision P must equal BitsStored"));
                }
                if usize::from(body[5]) != image.samples {
                    return Err(bad("SOF3 components do not equal SamplesPerPixel"));
                }
                let rows = usize::from(u16::from_be_bytes([body[1], body[2]]));
                let cols = usize::from(u16::from_be_bytes([body[3], body[4]]));
                if (rows, cols) != (image.rows, image.cols) {
                    return Err(bad("SOF3 dimensions do not equal Rows x Columns"));
                }
                for component in body[6..].as_chunks::<3>().0 {
                    if component[1] != 0x11 || component[2] != 0 || ids.contains(&component[0]) {
                        return Err(bad(
                            "lossless components require distinct IDs, 1x1 sampling and Tq=0",
                        ));
                    }
                    ids.push(component[0]);
                }
                precision_at = body_at;
            }
            0xC4 => {
                let mut rest = body;
                while !rest.is_empty() {
                    let (head, tail) = rest
                        .split_at_checked(17)
                        .ok_or_else(|| bad("malformed Huffman table"))?;
                    let id = usize::from(head[0]);
                    let total: usize = head[1..].iter().map(|&n| usize::from(n)).sum();
                    if id > 3
                        || total == 0
                        || total > 256
                        || total > tail.len()
                        || tail[..total].iter().any(|&n| n > 16)
                    {
                        return Err(bad(
                            "lossless requires DC tables 0..3 with categories 0..16",
                        ));
                    }
                    tables[id] = Some(huffman(&head[1..], &tail[..total], false)?);
                    rest = &tail[total..];
                }
            }
            0xDD => {
                if body != [0, 0] {
                    // The dependency does not reset its predictor at restarts.
                    // Reject rather than returning subtly corrupted samples.
                    return Err(bad("lossless restart intervals are not supported yet"));
                }
            }
            0xDA => {
                let count = ids.len();
                if count == 0
                    || body.first().copied().map(usize::from) != Some(count)
                    || body.len() != 4 + 2 * count
                {
                    return Err(bad(
                        "only one interleaved lossless scan with all components is supported yet",
                    ));
                }
                let predictor = body[1 + 2 * count];
                if !(1..=7).contains(&predictor) {
                    return Err(bad("lossless predictor Ss must be in 1..7"));
                }
                if selection_one && predictor != 1 {
                    return Err(bad("1.2.840.10008.1.2.4.70 requires predictor 1 (Ss=1)"));
                }
                let point = body[3 + 2 * count];
                if body[2 + 2 * count] != 0 || point >= image.stored as u8 {
                    return Err(bad(
                        "lossless requires Se=0, Ah=0 and point transform Pt < P",
                    ));
                }
                let mut scan_tables = Vec::new();
                for (&id, entry) in ids.iter().zip(body[1..1 + 2 * count].as_chunks::<2>().0) {
                    let table_id = usize::from(entry[1] >> 4);
                    if entry[0] != id || table_id > 3 || entry[1] & 15 != 0 {
                        return Err(bad("malformed lossless scan component selector"));
                    }
                    scan_tables.push(
                        tables[table_id]
                            .as_ref()
                            .ok_or_else(|| bad("scan uses an undefined Huffman table"))?,
                    );
                }
                let mut bits = Bits {
                    data,
                    pos,
                    acc: 0,
                    n: 0,
                };
                for pixel in 0..image.plane_len {
                    if pixel.is_multiple_of(POLL) {
                        crate::dsp::check_cancel(cancel)?;
                    }
                    for table in &scan_tables {
                        let category = bits.symbol(table)?;
                        // H.1.2.2: category 16 is the single modulo difference
                        // 32768, and has no following amplitude bits.
                        if category != 16 {
                            bits.skip(u32::from(category))?;
                        }
                    }
                }
                bits.fill();
                if bits.n != 0 && bits.n < 8 && bits.acc & ((1 << bits.n) - 1) != (1 << bits.n) - 1
                {
                    return Err(bad("entropy padding bits must be ones"));
                }
                bits.align()?;
                pos = bits.pos;
                if marker(data, &mut pos)? != 0xD9 {
                    return Err(bad(
                        "only one lossless scan followed by EOI is supported yet",
                    ));
                }
                if !matches!(&data[pos..], [] | [0]) {
                    return Err(bad(
                        "unexpected data after EOI (only one 0x00 pad byte is allowed)",
                    ));
                }
                return Ok(Header {
                    end: pos,
                    precision_at,
                    point_at: body_at + 3 + 2 * count,
                    point,
                });
            }
            _ => {}
        }
    }
}

/// A bounded, cancellable view with two header-byte substitutions. Decode the
/// reduced samples at Pt=0, then apply Pt once to the final raw values. This
/// avoids jpeg-decoder 0.3.2's repeated shift of already shifted predictions.
/// Reduced P=1 uses P=2 and a constant +1 prediction bias, removed below;
/// every H.1 predictor is translation invariant, including boundary rules.
struct Reader<'a> {
    data: &'a [u8],
    pos: usize,
    header: &'a Header,
    precision: u8,
    cancel: &'a AtomicBool,
}
impl Read for Reader<'_> {
    fn read(&mut self, out: &mut [u8]) -> std::io::Result<usize> {
        if self.pos.is_multiple_of(4096) && self.cancel.load(std::sync::atomic::Ordering::Relaxed) {
            return Err(std::io::Error::other("execution cancelled"));
        }
        let len = out
            .len()
            .min(self.data.len() - self.pos)
            .min(4096 - self.pos % 4096);
        out[..len].copy_from_slice(&self.data[self.pos..self.pos + len]);
        for (at, value) in [
            (self.header.precision_at, self.precision),
            (self.header.point_at, 0),
        ] {
            if at >= self.pos && at - self.pos < len {
                out[at - self.pos] = value;
            }
        }
        self.pos += len;
        Ok(len)
    }
}

pub(super) fn decode_frame(
    data: &[u8],
    out: &mut [u8],
    image: &Image,
    selection_one: bool,
    cancel: &AtomicBool,
) -> Result<()> {
    let header = inspect(data, image, selection_one, cancel)
        .map_err(|e| e.replace("JPEG Baseline", "JPEG Lossless"))?;
    crate::dsp::check_cancel(cancel)?;
    let reduced = image.stored as u8 - header.point;
    let precision = reduced.max(2);
    super::decoder_boundary("Lossless", || {
        let reader = Reader {
            data: &data[..header.end],
            pos: 0,
            header: &header,
            precision,
            cancel,
        };
        let mut decoder = jpeg_decoder::Decoder::new(reader);
        decoder.set_color_transform(jpeg_decoder::ColorTransform::None);
        decoder.set_max_decoding_buffer_size(out.len());
        let decoded = decoder.decode();
        crate::dsp::check_cancel(cancel)?;
        let decoded = decoded.map_err(|e| bad(&format!("decoder rejected frame: {e}")))?;
        let sample_bytes = if precision == 8 { 1 } else { 2 };
        let samples = image.len / image.frames;
        if decoded.len() != samples * sample_bytes {
            return Err(bad(
                "decoder output does not contain exactly Rows*Columns*Samples samples",
            ));
        }
        let allocated = usize::from(image.allocated / 8);
        for (index, sample) in decoded.chunks_exact(sample_bytes).enumerate() {
            if index.is_multiple_of(POLL) {
                crate::dsp::check_cancel(cancel)?;
            }
            let raw = if sample_bytes == 1 {
                u16::from(sample[0])
            } else {
                u16::from_ne_bytes([sample[0], sample[1]])
            };
            let raw = raw.wrapping_sub(u16::from(reduced == 1)) << header.point;
            let at = index * allocated;
            out[at] = raw as u8;
            if allocated == 2 {
                out[at + 1] = (raw >> 8) as u8;
            }
        }
        Ok(())
    })
}

#[cfg(test)]
mod tests {
    use super::super::{decode, layout};
    use super::*;

    #[test]
    fn decoder_panics_become_malformed_errors() {
        let result =
            super::super::decoder_boundary::<()>("Lossless", || panic!("crafted decoder panic"));
        assert!(result.unwrap_err().contains("malformed"));
    }

    use crate::engine::dicom::parser::{self, PixelLocation};
    use std::{
        io::{Cursor, Seek, SeekFrom},
        ops::Range,
        sync::atomic::Ordering,
    };

    const FRAMES: &[u8] = include_bytes!("../../../tests/fixtures/dicom/jl_frames_bot.dcm");
    struct Guarded<'a> {
        input: Cursor<&'static [u8]>,
        forbidden: Vec<Range<u64>>,
        cancel_after: Option<(usize, &'a AtomicBool)>,
        reads: usize,
    }
    impl Read for Guarded<'_> {
        fn read(&mut self, out: &mut [u8]) -> std::io::Result<usize> {
            let start = self.input.position();
            let end = start + out.len() as u64;
            assert!(
                self.forbidden
                    .iter()
                    .all(|r| end <= r.start || start >= r.end),
                "fragment payload read by metadata/admission"
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
        fn seek(&mut self, from: SeekFrom) -> std::io::Result<u64> {
            self.input.seek(from)
        }
    }

    #[test]
    fn lossless_admission_is_payload_free_bounded_and_cancellable() {
        let cancel = AtomicBool::new(false);
        let mut input = Cursor::new(FRAMES);
        let parsed = parser::parse(&mut input, FRAMES.len() as u64, 1 << 20, &cancel).unwrap();
        let Some(PixelLocation::Encapsulated { fragments, .. }) = &parsed.dataset.pixels else {
            panic!("expected fragments")
        };
        let forbidden = fragments
            .iter()
            .map(|f| f.value.offset..f.value.offset + f.value.length)
            .collect();
        let mut guard = Guarded {
            input: Cursor::new(FRAMES),
            forbidden,
            cancel_after: None,
            reads: 0,
        };
        // The structural parser used by dicominfo cannot access any payload.
        let parsed = parser::parse(&mut guard, FRAMES.len() as u64, 1 << 20, &cancel).unwrap();
        let plan = layout(&mut guard, &parsed, 1 << 20, 1 << 20, &cancel).unwrap();
        assert_eq!(plan.frames, [0..2, 2..4]);
        let exact = 3 * plan.image.image_bytes().unwrap()
            + 4 * plan.max_stream
            + workspace(&plan.image).unwrap()
            + 8
            + 2 * size_of::<Range<usize>>();
        let error = layout(&mut guard, &parsed, exact - 1, 1 << 20, &cancel)
            .err()
            .unwrap();
        assert!(error.contains("memory/output budget"), "{error}");
        layout(&mut guard, &parsed, exact, 1 << 20, &cancel).unwrap();
        // A large metadata-only image fits the matrix count but exceeds the
        // hard 256 MiB lossless peak even with an unlimited caller budget.
        let mut large = FRAMES.to_vec();
        for (tag, value) in [(0x0028_0010, 4096u16), (0x0028_0011, 4096u16)] {
            let element = parsed
                .dataset
                .elements
                .iter()
                .find(|e| e.tag == tag)
                .unwrap();
            let parser::ElementValue::Primitive(range) = element.value else {
                panic!("expected US")
            };
            large[range.offset as usize..range.offset as usize + 2]
                .copy_from_slice(&value.to_le_bytes());
        }
        let mut large = Cursor::new(large);
        let error = layout(&mut large, &parsed, usize::MAX, usize::MAX, &cancel)
            .err()
            .unwrap();
        assert!(error.contains("memory/output budget"), "{error}");
        guard.forbidden.clear();
        guard.reads = 0;
        guard.cancel_after = Some((1, &cancel));
        let error = decode(&mut guard, &parsed, &plan, &cancel).unwrap_err();
        assert!(error.contains("cancelled"), "{error}");
        assert_eq!(guard.reads, 1);
        cancel.store(false, Ordering::Relaxed);
        guard.reads = 0;
        // Frame 1's two fragments are decoded before reading frame 2.
        guard.cancel_after = Some((3, &cancel));
        let error = decode(&mut guard, &parsed, &plan, &cancel).unwrap_err();
        assert!(error.contains("cancelled"), "{error}");
        assert_eq!(guard.reads, 3);
    }
}
