//! Shared prepared input for metadata and native pixels. Original file offsets
//! never address the inflated dataset; no header is manufactured or rewritten.
use super::custom_dict::Dictionary;
use super::parser::{self, Backing, ParsedFile};
use crate::Result;
use flate2::{Decompress, FlushDecompress, Status};
use std::{
    fs::File,
    io::{Cursor, Read, Seek, SeekFrom},
    sync::atomic::AtomicBool,
};

const CHUNK: usize = 32 * 1024;
pub(crate) const MAX_DECODED: usize = 256 * 1024 * 1024;
// Two 32 KiB chunks plus conservative fixed decoder/window/stack headroom.
// Locked zlib-rs allocates one fixed State + (32 KiB + 64) window, no input-sized
// allocations (inflate::InflateAllocOffsets). 512 KiB covers this and flate2.
const INFLATE_WORKSPACE: usize = 512 * 1024;

pub(super) struct Prepared {
    pub(super) original: File,
    pub(super) dataset: DatasetBacking,
    pub(super) parsed: ParsedFile,
    pub(super) pixel_budget: usize,
    pub(super) inflate_units: usize,
}

pub(super) enum DatasetBacking {
    Original,
    Inflated(Cursor<Vec<u8>>),
}

pub(super) enum DatasetReader<'a> {
    Original(&'a mut File),
    Inflated(&'a mut Cursor<Vec<u8>>),
}
impl DatasetBacking {
    pub(super) fn reader<'a>(&'a mut self, original: &'a mut File) -> DatasetReader<'a> {
        match self {
            Self::Original => DatasetReader::Original(original),
            Self::Inflated(cursor) => DatasetReader::Inflated(cursor),
        }
    }
}
impl Read for DatasetReader<'_> {
    fn read(&mut self, out: &mut [u8]) -> std::io::Result<usize> {
        match self {
            Self::Original(r) => r.read(out),
            Self::Inflated(r) => r.read(out),
        }
    }
}
impl Seek for DatasetReader<'_> {
    fn seek(&mut self, position: SeekFrom) -> std::io::Result<u64> {
        match self {
            Self::Original(r) => r.seek(position),
            Self::Inflated(r) => r.seek(position),
        }
    }
}

pub(super) fn prepare(
    mut original: File,
    file_size: u64,
    budget: usize,
    work_bytes: usize,
    cancel: &AtomicBool,
    dictionary: &Dictionary,
) -> Result<Prepared> {
    let envelope =
        parser::envelope_with_dictionary(&mut original, file_size, budget, cancel, dictionary)?;
    let (mut dataset, backing, size, capacity, inflate_units) = if envelope.deflated {
        original
            .seek(SeekFrom::Start(envelope.dataset_start))
            .map_err(|_| "dicominfo: unreadable deflated dataset")?;
        let bytes = inflate(
            &mut original,
            file_size - envelope.dataset_start,
            budget
                .checked_sub(envelope.metadata_cost)
                .ok_or("dicominfo: deflate memory budget exhausted")?,
            work_bytes,
            MAX_DECODED,
            cancel,
        )
        .map_err(|error| format!("{error} (TransferSyntaxUID 1.2.840.10008.1.2.1.99)"))?;
        let size = bytes.len() as u64;
        let capacity = bytes.capacity();
        let units = bytes
            .len()
            .checked_add(
                usize::try_from(file_size - envelope.dataset_start).map_err(|_| memory_error())?,
            )
            .ok_or_else(memory_error)?
            .div_ceil(8);
        (
            DatasetBacking::Inflated(Cursor::new(bytes)),
            Backing::Inflated,
            size,
            capacity,
            units,
        )
    } else {
        (DatasetBacking::Original, Backing::Original, file_size, 0, 0)
    };
    // Inflation scratch has been dropped. Charge actual retained Vec capacity,
    // then envelope/dataset descriptors AND conservative metadata projection.
    let available = budget
        .checked_sub(capacity)
        .ok_or("dicominfo: retained dataset exceeds memory budget")?;
    let parsed = parser::dataset_with_dictionary(
        &mut dataset.reader(&mut original),
        envelope,
        backing,
        size,
        available,
        cancel,
        dictionary,
    )?;
    let metadata_cost = available.min(parser::MAX_METADATA_BUDGET) - parsed.remaining_budget;
    Ok(Prepared {
        original,
        dataset,
        parsed,
        pixel_budget: available - metadata_cost,
        inflate_units,
    })
}

fn memory_error() -> String {
    "dicominfo: deflated decoded data exceeds the 256 MiB cap or available memory budget".into()
}

/// A single RFC1951 stream only. StreamEnd (BFINAL), rather than reader EOF,
/// proves completion. PS3.5 A.5 requires exactly one terminal NUL for odd stream
/// byte counts and none for even counts; all other trailers are errors.
fn inflate<R: Read>(
    reader: &mut R,
    compressed_len: u64,
    budget: usize,
    work_bytes: usize,
    decoded_limit: usize,
    cancel: &AtomicBool,
) -> Result<Vec<u8>> {
    crate::dsp::check_cancel(cancel)?;
    if budget < INFLATE_WORKSPACE {
        return Err(memory_error());
    }
    let mut decoder = Decompress::new(false); // raw RFC1951, never zlib/gzip
    let mut input = [0u8; CHUNK];
    let mut output = [0u8; CHUNK];
    let mut unread = compressed_len;
    let (mut position, mut available) = (0, 0);
    let mut decoded = Vec::new();
    loop {
        crate::dsp::check_cancel(cancel)?;
        if position == available && unread > 0 {
            available = usize::try_from(unread.min(CHUNK as u64)).map_err(|_| memory_error())?;
            reader
                .read_exact(&mut input[..available])
                .map_err(|_| "dicominfo: truncated deflate stream")?;
            unread -= available as u64;
            position = 0;
        }
        crate::dsp::check_cancel(cancel)?;
        let before_in = decoder.total_in();
        let before_out = decoder.total_out();
        let status = decoder
            .decompress(
                &input[position..available],
                &mut output,
                FlushDecompress::None,
            )
            .map_err(|_| "dicominfo: corrupt raw deflate stream")?;
        let consumed =
            usize::try_from(decoder.total_in() - before_in).map_err(|_| memory_error())?;
        let produced =
            usize::try_from(decoder.total_out() - before_out).map_err(|_| memory_error())?;
        position = position
            .checked_add(consumed)
            .filter(|&n| n <= available)
            .ok_or("dicominfo: invalid deflate input progress")?;
        let needed = decoded
            .len()
            .checked_add(produced)
            .ok_or_else(memory_error)?;
        if needed > decoded_limit.min(MAX_DECODED) {
            return Err(memory_error());
        }
        let work = decoder
            .total_in()
            .checked_add(needed as u64)
            .ok_or("dicominfo: deflate work accounting overflow")?;
        if work > work_bytes as u64 {
            return Err("dicominfo: deflate exceeds the evaluation work budget".into());
        }
        if needed > decoded.capacity() {
            // Bound BOTH old and new allocations during a possible realloc,
            // along with chunks/decoder and the caller's retained meta/workspace.
            let max_new = budget
                .checked_sub(INFLATE_WORKSPACE)
                .and_then(|n| n.checked_sub(decoded.capacity()))
                .ok_or_else(memory_error)?;
            if needed > max_new {
                return Err(memory_error());
            }
            let target = decoded
                .capacity()
                .saturating_mul(2)
                .max(CHUNK)
                .max(needed)
                .min(decoded_limit.min(MAX_DECODED))
                .min(max_new);
            decoded
                .try_reserve_exact(target - decoded.len())
                .map_err(|_| memory_error())?;
            // Charge actual retained capacity (Vec may reserve more than asked).
            if decoded.capacity() > target || decoded.capacity() > decoded_limit.min(MAX_DECODED) {
                return Err(memory_error());
            }
        }
        decoded.extend_from_slice(&output[..produced]);
        crate::dsp::check_cancel(cancel)?;
        if status == Status::StreamEnd {
            let trailing = compressed_len
                .checked_sub(decoder.total_in())
                .ok_or("dicominfo: invalid deflate stream length")?;
            let required_pad = decoder.total_in() % 2;
            if trailing != required_pad {
                return Err(
                    "dicominfo: invalid deflate terminal padding or trailing stream/junk".into(),
                );
            }
            if required_pad == 1 {
                let pad = if position < available {
                    input[position]
                } else {
                    let mut pad = [0];
                    reader
                        .read_exact(&mut pad)
                        .map_err(|_| "dicominfo: truncated deflate terminal padding")?;
                    pad[0]
                };
                if pad != 0 {
                    return Err("dicominfo: invalid deflate terminal padding byte".into());
                }
            }
            return Ok(decoded);
        }
        if consumed == 0 && produced == 0 {
            return Err(if unread == 0 && position == available {
                "dicominfo: truncated deflate stream (no completed final block)"
            } else {
                "dicominfo: corrupt deflate stream (zero progress)"
            }
            .into());
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use flate2::{Compression, write::DeflateEncoder};
    use std::{io::Write, sync::atomic::Ordering};

    fn compressed(bytes: &[u8]) -> Vec<u8> {
        let mut encoder = DeflateEncoder::new(Vec::new(), Compression::fast());
        encoder.write_all(bytes).unwrap();
        let mut data = encoder.finish().unwrap();
        if data.len() % 2 == 1 {
            data.push(0);
        }
        data
    }

    #[test]
    fn cap_budget_work_and_chunk_boundaries() {
        let cancel = AtomicBool::new(false);
        for length in [0, 1, CHUNK - 1, CHUNK, CHUNK + 1, CHUNK * 3] {
            let data = vec![37; length];
            let encoded = compressed(&data);
            let result = inflate(
                &mut Cursor::new(&encoded),
                encoded.len() as u64,
                2 * 1024 * 1024,
                usize::MAX,
                length,
                &cancel,
            )
            .unwrap();
            assert_eq!(result, data);
            assert!(result.capacity() <= length);
            if length > 0 {
                assert!(
                    inflate(
                        &mut Cursor::new(&encoded),
                        encoded.len() as u64,
                        2 * 1024 * 1024,
                        usize::MAX,
                        length - 1,
                        &cancel
                    )
                    .unwrap_err()
                    .contains("cap")
                );
                assert!(
                    inflate(
                        &mut Cursor::new(&encoded),
                        encoded.len() as u64,
                        2 * 1024 * 1024,
                        length - 1,
                        MAX_DECODED,
                        &cancel
                    )
                    .unwrap_err()
                    .contains("work budget")
                );
            }
        }
        let encoded = compressed(&[0; CHUNK]);
        assert!(
            inflate(
                &mut Cursor::new(&encoded),
                encoded.len() as u64,
                INFLATE_WORKSPACE + CHUNK - 1,
                usize::MAX,
                MAX_DECODED,
                &cancel
            )
            .unwrap_err()
            .contains("memory")
        );
    }

    #[test]
    fn cancellation_after_inflate_has_started() {
        struct Watched<'a> {
            data: Cursor<Vec<u8>>,
            cancel: &'a AtomicBool,
            reads: usize,
        }
        impl Read for Watched<'_> {
            fn read(&mut self, out: &mut [u8]) -> std::io::Result<usize> {
                assert!(out.len() <= CHUNK);
                let n = self.data.read(out)?;
                self.reads += 1;
                if self.reads == 2 {
                    self.cancel.store(true, Ordering::Relaxed);
                }
                Ok(n)
            }
        }
        // Deterministic incompressible input requires multiple bounded reads.
        let mut state = 0x19283746u32;
        let data: Vec<u8> = (0..4 * CHUNK)
            .map(|_| {
                state ^= state << 13;
                state ^= state >> 17;
                state ^= state << 5;
                state as u8
            })
            .collect();
        let encoded = compressed(&data);
        assert!(encoded.len() > CHUNK);
        let cancel = AtomicBool::new(false);
        let mut reader = Watched {
            data: Cursor::new(encoded.clone()),
            cancel: &cancel,
            reads: 0,
        };
        assert!(
            inflate(
                &mut reader,
                encoded.len() as u64,
                2 * 1024 * 1024,
                usize::MAX,
                MAX_DECODED,
                &cancel
            )
            .unwrap_err()
            .contains("cancelled")
        );
        assert_eq!(reader.reads, 2);
    }

    #[test]
    fn terminal_padding_across_input_boundaries_and_incomplete_final_block() {
        let cancel = AtomicBool::new(false);
        for length in [CHUNK - 6, CHUNK - 5, CHUNK - 4] {
            // One final uncompressed RFC1951 block, with exact byte accounting.
            let mut encoded = vec![1];
            encoded.extend_from_slice(&(length as u16).to_le_bytes());
            encoded.extend_from_slice(&(!(length as u16)).to_le_bytes());
            encoded.extend(std::iter::repeat_n(47, length));
            if encoded.len() % 2 == 1 {
                encoded.push(0);
            }
            let decoded = inflate(
                &mut Cursor::new(&encoded),
                encoded.len() as u64,
                2 * 1024 * 1024,
                usize::MAX,
                MAX_DECODED,
                &cancel,
            )
            .unwrap();
            assert_eq!(decoded, vec![47; length]);
            encoded[0] = 0; // BFINAL absent; EOF must never be success.
            assert!(
                inflate(
                    &mut Cursor::new(&encoded),
                    encoded.len() as u64,
                    2 * 1024 * 1024,
                    usize::MAX,
                    MAX_DECODED,
                    &cancel
                )
                .unwrap_err()
                .contains("truncated")
            );
        }
    }
}
