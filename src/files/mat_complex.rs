//! Bounded split-component MAT v5 decoding into one interleaved allocation.
use super::{MatVariable, convert, read, tag, u32le};
use crate::{Array, Complex64, ComplexMatrix, Result, dsp::check_cancel};
use std::{io::Read, sync::atomic::AtomicBool};

fn component(
    reader: &mut impl Read,
    count: usize,
    remaining: usize,
    cancel: &AtomicBool,
    mut accept: impl FnMut(&[f64]),
) -> Result<usize> {
    check_cancel(cancel)?;
    if remaining < 8 {
        return Err("Missing complex MAT component".into());
    }
    let t = tag(reader)?;
    let word = u32le(&t[..4]);
    let short = (word >> 16) as usize;
    let kind = if short > 0 { word & 65535 } else { word };
    let width = match kind {
        1 | 2 => 1,
        3 | 4 => 2,
        5..=7 => 4,
        9 | 12 | 13 => 8,
        _ => return Err("Unsupported complex MAT component storage".into()),
    };
    let bytes = if short > 0 {
        short
    } else {
        u32le(&t[4..]) as usize
    };
    let consumed = 8 + if short > 0 {
        0
    } else {
        bytes.next_multiple_of(8)
    };
    if short > 4 || bytes != count * width || consumed > remaining {
        return Err("Complex MAT component length does not match dimensions".into());
    }
    let mut numbers = [0.; 8192];
    if short > 0 {
        convert(kind, &t[4..4 + short], &mut numbers[..count])?;
        accept(&numbers[..count]);
    } else {
        let mut input = [0u8; 65536];
        for first in (0..count).step_by(numbers.len()) {
            check_cancel(cancel)?;
            let n = (count - first).min(numbers.len());
            read(reader, &mut input[..n * width])?;
            convert(kind, &input[..n * width], &mut numbers[..n])?;
            accept(&numbers[..n]);
        }
        read(reader, &mut [0u8; 7][..(8 - bytes % 8) % 8])?;
    }
    check_cancel(cancel)?;
    Ok(consumed)
}

pub(super) fn read_array(
    reader: &mut impl Read,
    remaining: usize,
    info: &MatVariable,
    cancel: &AtomicBool,
) -> Result<Array> {
    check_cancel(cancel)?;
    if info.class != "double" || !info.complex || info.unavailable.is_some() {
        return Err("Unsupported complex MAT metadata".into());
    }
    let rows = info.dimensions[0];
    let cols = info.dimensions[1];
    let count = ComplexMatrix::checked_len(rows, cols)?;
    let mut values = Vec::new();
    values
        .try_reserve_exact(count)
        .map_err(|_| "Cannot allocate complex MAT array")?;
    let used = component(reader, count, remaining, cancel, |numbers| {
        values.extend(numbers.iter().map(|&re| Complex64::new(re, 0.)));
    })?;
    let mut next = 0;
    let imaginary = component(reader, count, remaining - used, cancel, |numbers| {
        for (value, &im) in values[next..next + numbers.len()].iter_mut().zip(numbers) {
            value.im = im;
        }
        next += numbers.len();
    })?;
    if used + imaginary != remaining {
        return Err("Trailing complex MAT component data".into());
    }
    Ok(ComplexMatrix::from_column_vec(rows, cols, values, cancel)?.into())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::{io::Cursor, sync::atomic::Ordering};

    #[test]
    fn component_read_cancels_during_each_component_without_publishing() {
        struct CancelReader<'a> {
            bytes: Cursor<Vec<u8>>,
            cancel: &'a AtomicBool,
            stop: u64,
        }
        impl Read for CancelReader<'_> {
            fn read(&mut self, output: &mut [u8]) -> std::io::Result<usize> {
                let n = self.bytes.read(output)?;
                if self.bytes.position() >= self.stop {
                    self.cancel.store(true, Ordering::Relaxed);
                }
                Ok(n)
            }
        }
        let count = 32769;
        let mut bytes = Vec::new();
        for _ in 0..2 {
            bytes.extend(9u32.to_le_bytes());
            bytes.extend((count as u32 * 8).to_le_bytes());
            bytes.resize(bytes.len() + count * 8, 0);
        }
        let info = MatVariable {
            name: "Z".into(),
            dimensions: vec![count, 1],
            class: "double".into(),
            complex: true,
            decoded_bytes: Some(count as u64 * 16),
            unavailable: None,
            compressed: false,
        };
        for stop in [65536, count as u64 * 8 + 8 + 65536] {
            let cancel = AtomicBool::new(false);
            let mut input = CancelReader {
                bytes: Cursor::new(bytes.clone()),
                cancel: &cancel,
                stop,
            };
            assert!(
                read_array(&mut input, bytes.len(), &info, &cancel)
                    .unwrap_err()
                    .contains("cancelled")
            );
            assert!(input.bytes.position() < bytes.len() as u64);
            assert!(input.bytes.position() <= stop + 65536);
        }
    }
}
