//! MAT v5 class 5 (mxSPARSE) per MATLAB's published MAT-file format: after
//! the header come `ir` (0-based int32 row indices, at least nnz of them),
//! `jc` (n+1 int32 column pointers; jc[n] = nnz) and `pr` (values: double, or
//! uint8 for a logical sparse array). Complex sparse arrays are rejected at
//! inspection, so `pi` is never read. The same element layout is read at top
//! level and inside cell arrays.
use super::{MatVariable, check_cancel, convert, read, tag, u32le};
use crate::{
    Array, Result,
    sparse::{MAX_NNZ, SparseMatrix, SparseValues},
};
use std::{io::Read, sync::atomic::AtomicBool};

/// Data element header: type, byte length and a compact payload.
fn element(reader: &mut impl Read, remaining: &mut usize) -> Result<(u32, usize, Option<[u8; 4]>)> {
    if *remaining < 8 {
        return Err("Truncated sparse MAT array".into());
    }
    *remaining -= 8;
    let t = tag(reader)?;
    let word = u32le(&t[..4]);
    let short = (word >> 16) as usize;
    if short > 0 {
        if short > 4 {
            return Err("Invalid compact MAT tag".into());
        }
        return Ok((word & 65535, short, Some(t[4..8].try_into().unwrap())));
    }
    let len = u32le(&t[4..]) as usize;
    if len.next_multiple_of(8) > *remaining {
        return Err("Sparse MAT element exceeds its array".into());
    }
    *remaining -= len.next_multiple_of(8);
    Ok((word, len, None))
}
/// Element body in chunks of at most 64 KiB; padding is consumed.
fn body(
    reader: &mut impl Read,
    len: usize,
    compact: Option<[u8; 4]>,
    cancel: &AtomicBool,
    mut each: impl FnMut(&[u8]) -> Result<()>,
) -> Result<()> {
    if let Some(bytes) = compact {
        return each(&bytes[..len]);
    }
    let mut buffer = vec![0u8; 65536.min(len.next_multiple_of(8)).max(8)];
    let mut left = len.next_multiple_of(8);
    let mut data_left = len;
    while left > 0 {
        check_cancel(cancel)?;
        let n = left.min(buffer.len());
        read(reader, &mut buffer[..n])?;
        let take = n.min(data_left);
        if take > 0 {
            each(&buffer[..take])?;
        }
        data_left -= take;
        left -= n;
    }
    Ok(())
}
/// Most values reserved up front: a larger list grows only as its data is read,
/// so a length claimed by a damaged file never becomes one big allocation.
const RESERVE: usize = 1 << 20;

/// An int32 element: (count, the first `keep` values, the last value).
fn int32_list(
    reader: &mut impl Read,
    remaining: &mut usize,
    what: &str,
    expected: Option<usize>,
    keep: usize,
    cancel: &AtomicBool,
) -> Result<(usize, Vec<usize>, usize)> {
    let (kind, len, compact) = element(reader, remaining)?;
    if kind != 5 || len % 4 != 0 {
        return Err(format!("Sparse {what} must be int32"));
    }
    let count = len / 4;
    if expected.is_some_and(|n| n != count) {
        return Err(format!(
            "Sparse {what} length does not match the dimensions"
        ));
    }
    if count as u64 > MAX_NNZ + 1 {
        return Err(format!("Sparse {what} exceeds {MAX_NNZ} entries"));
    }
    let mut values = Vec::with_capacity(count.min(keep).min(RESERVE));
    let mut last = 0usize;
    body(reader, len, compact, cancel, |bytes| {
        for b in bytes.as_chunks::<4>().0 {
            let v = i32::from_le_bytes(*b);
            if v < 0 {
                return Err(format!("Negative sparse {what}"));
            }
            last = v as usize;
            if values.len() < keep {
                values.push(v as usize);
            }
        }
        Ok(())
    })?;
    Ok((count, values, last))
}
/// Value element header: (MAT type, byte width, byte length, compact payload).
fn value_element(
    reader: &mut impl Read,
    remaining: &mut usize,
    nnz: usize,
) -> Result<(u32, usize, usize, Option<[u8; 4]>)> {
    let (kind, len, compact) = element(reader, remaining)?;
    let width = match kind {
        1 | 2 => 1,
        3 | 4 => 2,
        5..=7 => 4,
        9 | 12 | 13 => 8,
        _ => return Err("Unsupported sparse MAT value storage".into()),
    };
    if len % width != 0 || len / width < nnz {
        return Err("Sparse MAT values do not match the column pointers".into());
    }
    Ok((kind, width, len, compact))
}

/// Stored entry count (jc[n]) without keeping any list. With `count_values`
/// the values are streamed too and explicit zeros (dropped on loading) are not
/// counted, so the count equals the loaded array's. `remaining` is reduced by
/// the bytes consumed.
pub(super) fn inspect(
    reader: &mut impl Read,
    remaining: &mut usize,
    info: &MatVariable,
    count_values: bool,
    cancel: &AtomicBool,
) -> Result<u64> {
    let n = info.dimensions[1];
    let (kind, len, compact) = element(reader, remaining)?;
    if kind != 5 || len % 4 != 0 {
        return Err("Sparse row indices must be int32".into());
    }
    body(reader, len, compact, cancel, |_| Ok(()))?;
    let (_, _, nnz) = int32_list(reader, remaining, "column pointers", Some(n + 1), 0, cancel)?;
    if nnz > len / 4 {
        return Err("Sparse column pointers exceed the row indices".into());
    }
    if !count_values {
        return Ok(nnz as u64);
    }
    let (kind, width, len, compact) = value_element(reader, remaining, nnz)?;
    let mut numbers = vec![0.; 65536 / 8];
    let (mut seen, mut stored) = (0usize, 0u64);
    body(reader, len, compact, cancel, |bytes| {
        for chunk in bytes.chunks(numbers.len() * width) {
            let count = chunk.len() / width;
            convert(kind, chunk, &mut numbers[..count])?;
            let take = count.min(nnz - seen);
            stored += numbers[..take].iter().filter(|&&v| v != 0.).count() as u64;
            seen += take;
        }
        Ok(())
    })?;
    Ok(stored)
}

/// MatRust's byte count for a sparse variable (see `SparseMatrix::bytes`).
pub(super) fn decoded_bytes(nnz: u64, cols: usize, logical: bool) -> Option<u64> {
    nnz.checked_mul(if logical { 9 } else { 16 })?
        .checked_add((cols as u64).checked_add(1)?.checked_mul(8)?)
}
/// The stored-entry count behind `decoded_bytes` (inverse of the above).
pub(super) fn nnz_of(info: &MatVariable) -> Option<usize> {
    let per = if info.class == "logical sparse" {
        9
    } else {
        16
    };
    let pointers = (*info.dimensions.get(1)? as u64)
        .checked_add(1)?
        .checked_mul(8)?;
    usize::try_from(info.decoded_bytes?.checked_sub(pointers)? / per).ok()
}

/// Load one sparse array. `nnz` is the stored-entry count found by
/// inspection when known (top-level variables): no more row indices than
/// that are kept or reserved. `remaining` is reduced by the bytes consumed.
pub(super) fn read_array(
    reader: &mut impl Read,
    remaining: &mut usize,
    info: &MatVariable,
    nnz: Option<usize>,
    cancel: &AtomicBool,
) -> Result<Array> {
    let (m, n) = (info.dimensions[0], info.dimensions[1]);
    let logical = info.class == "logical sparse";
    let keep = nnz.unwrap_or(usize::MAX);
    let (_, mut ir, _) = int32_list(reader, remaining, "row indices", None, keep, cancel)?;
    let (_, jc, _) = int32_list(
        reader,
        remaining,
        "column pointers",
        Some(n + 1),
        n + 1,
        cancel,
    )?;
    let nnz = jc[n];
    if nnz > ir.len() {
        return Err("Sparse column pointers exceed the row indices".into());
    }
    ir.truncate(nnz);
    let (kind, width, len, compact) = value_element(reader, remaining, nnz)?;
    let mut values = Vec::with_capacity(nnz);
    let mut numbers = vec![0.; 65536];
    body(reader, len, compact, cancel, |bytes| {
        let count = bytes.len() / width;
        convert(kind, bytes, &mut numbers[..count])?;
        let room = nnz - values.len();
        values.extend_from_slice(&numbers[..count.min(room)]);
        Ok(())
    })?;
    let values = if logical {
        if values.iter().any(|&v| v != 0. && v != 1.) {
            return Err("Invalid logical MAT values".into());
        }
        SparseValues::Logical(values.iter().map(|&v| v != 0.).collect())
    } else {
        SparseValues::Double(values)
    };
    Ok(Array::Sparse(SparseMatrix::from_parts(
        m, n, jc, ir, values,
    )?))
}
