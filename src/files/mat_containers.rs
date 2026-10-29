//! Bounded recursive Level-5 cell and exact UTF-16 character payloads.
//! Catalog traversal skips leaf payloads; only selected loading constructs arrays.
use super::{MatVariable, header_named, read, tag, u32le};
use crate::{Array, CellArray, CharacterRow, Result, dsp::check_cancel};
use std::{
    io::Read,
    sync::{Arc, atomic::AtomicBool},
};

const MAX_SLOTS: usize = crate::script_containers::MAX_CELL_SLOTS;
const MAX_BYTES: usize = 256 * 1024 * 1024;
const MAX_ENCODED: usize = crate::files::MAX_MAT_BYTES;

struct Budget {
    bytes: usize,
    slots: usize,
    limit: usize,
}
impl Budget {
    fn charge(&mut self, bytes: usize, slots: usize) -> Result<()> {
        self.bytes = self.bytes.checked_add(bytes).ok_or("Cell byte overflow")?;
        self.slots = self.slots.checked_add(slots).ok_or("Cell slot overflow")?;
        if self.bytes > self.limit || self.slots > MAX_SLOTS {
            return Err("Cell exceeds logical storage or 65536 nested slots".into());
        }
        Ok(())
    }
}
fn skip(mut reader: &mut dyn Read, mut count: usize, cancel: &AtomicBool) -> Result<()> {
    let mut bytes = [0u8; 65536];
    while count != 0 {
        check_cancel(cancel)?;
        let n = count.min(bytes.len());
        read(&mut reader, &mut bytes[..n])?;
        count -= n;
    }
    check_cancel(cancel)
}

pub(super) fn inspect(
    reader: &mut impl Read,
    remaining: usize,
    info: &MatVariable,
    cancel: &AtomicBool,
) -> Result<u64> {
    let mut budget = Budget {
        bytes: 0,
        slots: 0,
        limit: MAX_BYTES,
    };
    decode(reader, remaining, info, 1, false, &mut budget, cancel)?;
    Ok(budget.bytes as u64)
}

pub(super) fn read_array(
    reader: &mut impl Read,
    remaining: usize,
    info: &MatVariable,
    cancel: &AtomicBool,
) -> Result<Array> {
    let expected = usize::try_from(
        info.storage_bytes()
            .ok_or("Missing container storage estimate")?,
    )
    .map_err(|_| "Container size overflow")?;
    if expected > MAX_BYTES {
        return Err("Container exceeds workspace limit".into());
    }
    let mut budget = Budget {
        bytes: 0,
        slots: 0,
        limit: expected,
    };
    let array = decode(reader, remaining, info, 1, true, &mut budget, cancel)?
        .ok_or("Missing loaded container")?;
    if budget.bytes != expected || array.bytes() != expected {
        return Err("Container storage changed since inspection".into());
    }
    check_cancel(cancel)?;
    Ok(array)
}

// Type erasure prevents recursively growing Take<...> monomorphizations.
fn decode(
    mut reader: &mut dyn Read,
    remaining: usize,
    info: &MatVariable,
    depth: usize,
    materialize: bool,
    budget: &mut Budget,
    cancel: &AtomicBool,
) -> Result<Option<Array>> {
    check_cancel(cancel)?;
    if remaining > MAX_ENCODED {
        return Err("Cell encoded extent exceeds 272 MiB".into());
    }
    if let Some(reason) = &info.unavailable {
        return Err(reason.clone());
    }
    if info.class != "cell" {
        let bytes = usize::try_from(info.storage_bytes().ok_or("Unknown child storage size")?)
            .map_err(|_| "Child storage overflow")?;
        budget.charge(bytes, 0)?;
        if !materialize {
            skip(reader, remaining, cancel)?;
            return Ok(None);
        }
        if info.class == "char" {
            return read_char(reader, remaining, info, cancel).map(Some);
        }
        // Header was already consumed; retain the original leaf numeric/complex codec.
        return super::read_payload(&mut reader, remaining, info, 0, cancel, true).map(Some);
    }
    if depth > 16 {
        return Err("Cell nesting exceeds 16 levels".into());
    }
    let rows = info.dimensions[0];
    let cols = info.dimensions[1];
    let count = rows.checked_mul(cols).ok_or("Cell dimensions overflow")?;
    if rows > MAX_SLOTS || cols > MAX_SLOTS || count > MAX_SLOTS {
        return Err("Cell dimensions exceed 65536 slots".into());
    }
    budget.charge(
        count
            .checked_mul(crate::containers::SLOT)
            .ok_or("Cell metadata overflow")?,
        count,
    )?;
    let mut slots = Vec::new();
    if materialize {
        slots
            .try_reserve_exact(count)
            .map_err(|_| "Cannot allocate cell slots")?;
    }
    let mut remaining = remaining;
    for _ in 0..count {
        check_cancel(cancel)?;
        if remaining < 8 {
            return Err("Missing MAT cell child".into());
        }
        let t = tag(&mut reader)?;
        let len = u32le(&t[4..]) as usize;
        if u32le(&t[..4]) != 14
            || len > MAX_ENCODED
            || !len.is_multiple_of(8)
            || len > remaining - 8
        {
            return Err("Invalid MAT cell child extent or type".into());
        }
        let mut child = (&mut reader).take(len as u64);
        let (child_info, used) = header_named(&mut child, len, info.compressed, true)?;
        if !child_info.name.is_empty() {
            return Err("Nested cell matrices must be anonymous".into());
        }
        let value = decode(
            &mut child,
            len - used,
            &child_info,
            depth + 1,
            materialize,
            budget,
            cancel,
        )?;
        if child.limit() != 0 {
            return Err("Unconsumed MAT cell child bytes".into());
        }
        if let Some(value) = value {
            slots.push(Arc::new(value));
        }
        remaining -= 8 + len;
    }
    if remaining != 0 {
        return Err("Trailing MAT cell data".into());
    }
    check_cancel(cancel)?;
    if materialize {
        CellArray::from_shared(rows, cols, slots, cancel).map(|a| Some(a.into()))
    } else {
        Ok(None)
    }
}

fn read_char(
    mut reader: &mut dyn Read,
    remaining: usize,
    info: &MatVariable,
    cancel: &AtomicBool,
) -> Result<Array> {
    check_cancel(cancel)?;
    let count = info.dimensions[0]
        .checked_mul(info.dimensions[1])
        .ok_or("Character dimensions overflow")?;
    if count > crate::script_containers::MAX_CHARACTER_UNITS || remaining < 8 {
        return Err("Invalid bounded character extent".into());
    }
    let t = tag(&mut reader)?;
    let word = u32le(&t[..4]);
    let short = (word >> 16) as usize;
    let kind = if short != 0 { word & 65535 } else { word };
    let bytes = if short != 0 {
        short
    } else {
        u32le(&t[4..]) as usize
    };
    if short > 4
        || bytes > crate::script_containers::MAX_CHARACTER_UNITS * 3
        || remaining
            != 8 + if short != 0 {
                0
            } else {
                bytes.next_multiple_of(8)
            }
        || !matches!(kind, 4 | 16 | 17)
    {
        return Err("Unsupported character encoding or invalid payload extent".into());
    }
    if matches!(kind, 4 | 17) && bytes != count * 2 {
        return Err("UTF-16 character length does not match dimensions".into());
    }
    let mut input = Vec::new();
    input
        .try_reserve_exact(bytes)
        .map_err(|_| "Cannot allocate bounded character input")?;
    input.resize(bytes, 0);
    if short != 0 {
        input.copy_from_slice(&t[4..4 + short]);
    } else {
        for chunk in input.chunks_mut(2048) {
            check_cancel(cancel)?;
            read(&mut reader, chunk)?;
        }
        read(&mut reader, &mut [0u8; 7][..(8 - bytes % 8) % 8])?;
    }
    let row = if kind == 16 {
        let text = std::str::from_utf8(&input).map_err(|_| "Invalid UTF-8 MAT character data")?;
        CharacterRow::from_text(text, cancel)?.reshape(
            info.dimensions[0],
            info.dimensions[1],
            cancel,
        )?
    } else {
        let mut units = Vec::new();
        units
            .try_reserve_exact(count)
            .map_err(|_| "Cannot allocate UTF-16 row")?;
        for (i, pair) in input.as_chunks::<2>().0.iter().enumerate() {
            if i.is_multiple_of(1024) {
                check_cancel(cancel)?;
            }
            units.push(u16::from_le_bytes([pair[0], pair[1]]));
        }
        CharacterRow::from_column_major(info.dimensions[0], info.dimensions[1], units, cancel)?
    };
    if row.len() != count || (row.rows(), row.cols()) != (info.dimensions[0], info.dimensions[1]) {
        return Err("Character UTF-16 dimensions do not match stored shape".into());
    }
    check_cancel(cancel)?;
    Ok(row.into())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::{io::Cursor, sync::atomic::Ordering};
    #[test]
    fn character_component_stop_is_observed_after_bounded_actual_read() {
        struct Input<'a> {
            bytes: Cursor<Vec<u8>>,
            cancel: &'a AtomicBool,
        }
        impl Read for Input<'_> {
            fn read(&mut self, out: &mut [u8]) -> std::io::Result<usize> {
                let n = self.bytes.read(out)?;
                if self.bytes.position() >= 4096 {
                    self.cancel.store(true, Ordering::Relaxed);
                }
                Ok(n)
            }
        }
        let mut bytes = 17u32.to_le_bytes().to_vec();
        bytes.extend(131072u32.to_le_bytes());
        bytes.resize(131080, 0);
        let stop = AtomicBool::new(false);
        let mut reader = Input {
            bytes: Cursor::new(bytes),
            cancel: &stop,
        };
        let info = MatVariable {
            name: "S".into(),
            dimensions: vec![1, 65536],
            class: "char".into(),
            complex: false,
            decoded_bytes: Some(131072),
            unavailable: None,
            compressed: false,
        };
        assert!(
            read_array(&mut reader, 131080, &info, &stop)
                .unwrap_err()
                .contains("cancel")
        );
        assert!((4096..=6144).contains(&reader.bytes.position()));
        assert!(reader.bytes.position() < 131080);
    }
    #[test]
    fn nested_slot_charge_precedes_child_table_allocation() {
        // Outer one-slot cell plus a declared 65536-slot child exceeds the
        // admitted transitive count before any missing leaf payload is read.
        let mut child = Vec::new();
        for (kind, data) in [
            (6u32, [1u32.to_le_bytes(), 0u32.to_le_bytes()].concat()),
            (5, [1i32.to_le_bytes(), 65536i32.to_le_bytes()].concat()),
            (1, vec![]),
        ] {
            child.extend(kind.to_le_bytes());
            child.extend((data.len() as u32).to_le_bytes());
            child.extend(&data);
            child.resize(child.len() + (8 - data.len() % 8) % 8, 0);
        }
        let mut bytes = 14u32.to_le_bytes().to_vec();
        bytes.extend((child.len() as u32).to_le_bytes());
        bytes.extend(child);
        let info = MatVariable {
            name: "C".into(),
            dimensions: vec![1, 1],
            class: "cell".into(),
            complex: false,
            decoded_bytes: None,
            unavailable: None,
            compressed: false,
        };
        let stop = AtomicBool::new(false);
        let error = inspect(&mut Cursor::new(&bytes), bytes.len(), &info, &stop).unwrap_err();
        assert!(error.contains("65536 nested slots"), "{error}");
    }
}
