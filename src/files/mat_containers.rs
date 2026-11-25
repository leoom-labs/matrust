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
    if matches!(info.class.as_str(), "sparse" | "logical sparse") {
        // Sparse cell element (class 5): its storage comes from the stored
        // entries, which inspection counts without keeping them.
        let mut left = remaining;
        let value = if materialize {
            let array = super::sparse::read_array(&mut reader, &mut left, info, None, cancel)?;
            budget.charge(array.bytes(), 0)?;
            Some(array)
        } else {
            let nnz = super::sparse::inspect(&mut reader, &mut left, info, true, cancel)?;
            let logical = info.class == "logical sparse";
            let bytes = super::sparse::decoded_bytes(nnz, info.dimensions[1], logical)
                .and_then(|n| usize::try_from(n).ok())
                .ok_or("Sparse cell element storage overflow")?;
            budget.charge(bytes, 0)?;
            None
        };
        skip(reader, left, cancel)?;
        return Ok(value);
    }
    if !matches!(info.class.as_str(), "cell" | "struct") {
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
    let mut remaining = remaining;
    let names = if info.class == "struct" {
        let (kind, width, used) = super::small_element(&mut reader, 4)?;
        remaining = remaining
            .checked_sub(used)
            .ok_or("Truncated MAT struct metadata")?;
        if kind != 5 || width.len() != 4 {
            return Err("Invalid MAT struct field-name length".into());
        }
        let width = u32le(&width) as usize;
        if !(1..=64).contains(&width) {
            return Err("Invalid MAT struct field-name length".into());
        }
        let (kind, fields, used) =
            super::small_element(&mut reader, 64 * crate::struct_array::MAX_FIELDS)?;
        remaining = remaining
            .checked_sub(used)
            .ok_or("Truncated MAT struct field names")?;
        if kind != 1
            || fields.len() % width != 0
            || fields.len() / width > crate::struct_array::MAX_FIELDS
        {
            return Err("Invalid MAT struct field names".into());
        }
        let mut names = Vec::new();
        for field in fields.chunks_exact(width) {
            let end = field
                .iter()
                .position(|&b| b == 0)
                .ok_or("Unterminated MAT struct field name")?;
            let name = std::str::from_utf8(&field[..end])
                .map_err(|_| "Invalid MAT struct field name")?
                .to_owned();
            if !crate::files::valid_name(&name) || names.contains(&name) {
                return Err("Invalid or duplicate MAT struct field name".into());
            }
            names.push(name);
        }
        budget.charge(names.iter().map(String::len).sum(), 0)?;
        Some(names)
    } else {
        None
    };
    let elements = count;
    let count = if let Some(names) = &names {
        crate::StructArray::slot_total(count, names.len())?
    } else {
        count
    };
    budget.charge(
        count
            .checked_mul(crate::containers::SLOT)
            .ok_or("Cell metadata overflow")?,
        count.max(elements),
    )?;
    let mut slots = Vec::new();
    if materialize {
        slots
            .try_reserve_exact(count)
            .map_err(|_| "Cannot allocate cell slots")?;
    }
    for index in 0..count {
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
        )
        .map_err(|e| match &names {
            Some(names) => format!("{}: {e}", names[index % names.len()]),
            None => e,
        })?;
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
        if let Some(names) = names {
            crate::StructArray::from_slots(rows, cols, names, slots, cancel).map(|a| Some(a.into()))
        } else {
            CellArray::from_shared(rows, cols, slots, cancel).map(|a| Some(a.into()))
        }
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
    let count = info
        .dimensions
        .iter()
        .try_fold(1usize, |n, &d| n.checked_mul(d))
        .ok_or("Character dimensions overflow")?;
    let rows = info.dimensions[0];
    let cols = crate::files::checked_dimension_product(&info.dimensions[1..])?;
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
        CharacterRow::from_text(text, cancel)?.reshape(rows, cols, cancel)?
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
        CharacterRow::from_column_major(rows, cols, units, cancel)?
    };
    if row.len() != count || (row.rows(), row.cols()) != (rows, cols) {
        return Err("Character UTF-16 dimensions do not match stored shape".into());
    }
    check_cancel(cancel)?;
    Ok(row.with_dims(&info.dimensions)?.into())
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
