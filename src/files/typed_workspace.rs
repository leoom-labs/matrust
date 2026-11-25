//! Typed Level-5 writer: borrowed source arrays and streamed per-array compression.
use super::{MAX_MAT_BYTES, atomic_write, valid_name};
use crate::{Array, Result, dsp::check_cancel};
use std::{collections::BTreeMap, io::Write, path::Path, sync::atomic::AtomicBool};

fn put(out: &mut impl Write, bytes: &[u8]) -> Result<()> {
    out.write_all(bytes).map_err(|e| e.to_string())
}
fn tag(out: &mut impl Write, kind: u32, len: usize) -> Result<()> {
    put(out, &kind.to_le_bytes())?;
    put(out, &(len as u32).to_le_bytes())
}
fn padding(out: &mut impl Write, len: usize) -> Result<()> {
    put(out, &[0; 8][..(8 - len % 8) % 8])
}
fn element(out: &mut impl Write, kind: u32, bytes: &[u8]) -> Result<()> {
    tag(out, kind, bytes.len())?;
    put(out, bytes)?;
    padding(out, bytes.len())
}
/// Save a compressed Level-5 MAT workspace, preserving supported numeric, character and cell types.
pub fn save_array_workspace(
    path: &Path,
    variables: &BTreeMap<String, Array>,
    cancel: &AtomicBool,
) -> Result<()> {
    save(path, variables, cancel, true)
}
/// Save uncompressed Level-5 data for applications preferring direct payload reads.
pub fn save_array_workspace_uncompressed(
    path: &Path,
    variables: &BTreeMap<String, Array>,
    cancel: &AtomicBool,
) -> Result<()> {
    save(path, variables, cancel, false)
}
fn save(
    path: &Path,
    variables: &BTreeMap<String, Array>,
    cancel: &AtomicBool,
    compressed: bool,
) -> Result<()> {
    check_cancel(cancel)?;
    if variables.len() > 10000 {
        return Err("Workspace exceeds 10000 variables".into());
    }
    let mut total = 128usize;
    let mut payload = 0usize;
    for (name, a) in variables {
        if !valid_name(name) {
            return Err(format!("Invalid variable name: {name}"));
        }
        payload = payload.checked_add(a.bytes()).ok_or("MAT size overflow")?;
        total = total
            .checked_add(encoded_len(name, a, cancel, 1)?)
            .ok_or("MAT size overflow")?;
    }
    if payload > 256 * 1024 * 1024 || total > MAX_MAT_BYTES {
        return Err("Workspace exceeds 256 MiB or MAT file size limit".into());
    }
    let mut out = Vec::with_capacity(if compressed { total.min(65536) } else { total });
    out.resize(128, b' ');
    let text = b"MATLAB 5.0 MAT-file, MatRust typed numerical workspace";
    out[..text.len()].copy_from_slice(text);
    out[116..124].fill(0);
    out[124..128].copy_from_slice(&[0, 1, b'I', b'M']);
    for (name, a) in variables {
        check_cancel(cancel)?;
        if compressed {
            let start = out.len();
            tag(&mut out, 15, 0)?;
            let mut encoder =
                flate2::write::ZlibEncoder::new(&mut out, flate2::Compression::default());
            write_array(&mut encoder, name, a, cancel)?;
            encoder.finish().map_err(|e| e.to_string())?;
            let size = (out.len() - start - 8) as u32;
            out[start + 4..start + 8].copy_from_slice(&size.to_le_bytes());
        } else {
            write_array(&mut out, name, a, cancel)?;
        }
        if out.len() > MAX_MAT_BYTES {
            return Err("MAT file exceeds its encoded size limit".into());
        }
    }
    debug_assert!(compressed || out.len() == total);
    check_cancel(cancel)?;
    atomic_write(path, &out)
}
// Only metadata is visited. The immutable cell constructor already bounds the
// recursive graph; checked encoded accounting additionally includes every tag.
fn encoded_len(name: &str, a: &Array, cancel: &AtomicBool, depth: usize) -> Result<usize> {
    check_cancel(cancel)?;
    if matches!(a, Array::Str(_)) {
        return Err("string arrays cannot be saved to MAT files yet; use cellstr(...)".into());
    }
    if matches!(a, Array::Table(_)) {
        return Err("save: table variables cannot be saved yet".into());
    }
    if matches!(a, Array::Categorical(_)) {
        return Err("categorical is not supported by save yet".into());
    }
    if matches!(a, Array::Datetime(_) | Array::Duration(_)) {
        return Err("datetime/duration variables cannot be saved yet".into());
    }
    if matches!(a, Array::Graphics(_)) {
        return Err("MAT persistence of graphics handles is not implemented".into());
    }
    if matches!(a, Array::Handle(_)) {
        return Err("save: function handles are not supported yet".into());
    }
    let dimension_element = 8 + (a.dims().len() * 4).next_multiple_of(8);
    let mut len = 8 + 16 + dimension_element + 8 + name.len().next_multiple_of(8);
    if let Array::Sparse(s) = a {
        let fits = |n: usize| n <= i32::MAX as usize;
        if !fits(s.rows()) || !fits(s.cols()) || !fits(s.nnz()) {
            return Err(format!(
                "save: sparse variable '{name}' has dimensions or a nonzero count that do not fit the MAT file's int32 indices"
            ));
        }
        let element = |bytes: usize| 8 + bytes.next_multiple_of(8);
        let value_bytes = if s.is_logical() { s.nnz() } else { 8 * s.nnz() };
        len = len
            .checked_add(element(4 * s.nnz()) + element(4 * (s.cols() + 1)) + element(value_bytes))
            .ok_or("MAT encoded size overflow")?;
    } else if let Array::Struct(st) = a {
        if depth > 16 {
            return Err("Struct nesting exceeds 16 levels".into());
        }
        let width = st
            .field_names()
            .iter()
            .map(|n| n.len() + 1)
            .max()
            .unwrap_or(1);
        len += 16 + 8 + (width * st.field_count()).next_multiple_of(8);
        for index in 0..st.slot_count() {
            let field = &st.field_names()[index % st.field_count()];
            len = len
                .checked_add(
                    encoded_len("", st.slot(index)?, cancel, depth + 1)
                        .map_err(|e| format!("{field}: {e}"))?,
                )
                .ok_or("MAT encoded size overflow")?;
            if len > MAX_MAT_BYTES {
                return Err("MAT encoded struct exceeds 272 MiB".into());
            }
        }
    } else if let Array::Cell(cell) = a {
        if depth > 16 {
            return Err("Cell nesting exceeds 16 levels".into());
        }
        for index in 0..cell.len() {
            check_cancel(cancel)?;
            len = len
                .checked_add(encoded_len(
                    "",
                    cell.get_linear(index)?.as_ref(),
                    cancel,
                    depth + 1,
                )?)
                .ok_or("MAT encoded size overflow")?;
            if len > MAX_MAT_BYTES {
                return Err("MAT encoded cell exceeds 272 MiB".into());
            }
        }
    } else if let Array::Complex(matrix) = a {
        let width = if matrix.is_single() { 4 } else { 8 };
        let component = 8 + (matrix.len() * width).next_multiple_of(8);
        len = len
            .checked_add(2 * component)
            .ok_or("MAT encoded size overflow")?;
    } else {
        len = len
            .checked_add(8 + a.bytes().next_multiple_of(8))
            .ok_or("MAT encoded size overflow")?;
    }
    if len > MAX_MAT_BYTES {
        return Err("MAT encoded array exceeds 272 MiB".into());
    }
    Ok(len)
}
// Keep the leaf scratch out of recursively active cell frames.
#[inline(never)]
fn write_complex(
    out: &mut impl Write,
    matrix: &crate::ComplexMatrix,
    cancel: &AtomicBool,
) -> Result<()> {
    // MAT v5 stores separate column-major real and imaginary components.
    // One bounded scratch buffer avoids cloning either complete component.
    let mut scratch = [0u8; 65536];
    for imaginary in [false, true] {
        check_cancel(cancel)?;
        let width = if matrix.is_single() { 4 } else { 8 };
        tag(
            out,
            if matrix.is_single() { 7 } else { 9 },
            matrix.len() * width,
        )?;
        for chunk in matrix.data().chunks(8192) {
            check_cancel(cancel)?;
            for (value, bytes) in chunk.iter().zip(scratch.chunks_exact_mut(width)) {
                let component = if imaginary { value.im } else { value.re };
                if matrix.is_single() {
                    bytes.copy_from_slice(&(component as f32).to_le_bytes());
                } else {
                    bytes.copy_from_slice(&component.to_le_bytes());
                }
            }
            put(out, &scratch[..chunk.len() * width])?;
        }
        padding(out, matrix.len() * width)?;
    }
    check_cancel(cancel)
}
fn write_int32(out: &mut impl Write, values: &[usize], cancel: &AtomicBool) -> Result<()> {
    tag(out, 5, values.len() * 4)?;
    let mut buffer = Vec::with_capacity(8192 * 4);
    for chunk in values.chunks(8192) {
        check_cancel(cancel)?;
        buffer.clear();
        for &v in chunk {
            buffer.extend_from_slice(&(v as i32).to_le_bytes());
        }
        put(out, &buffer)?;
    }
    padding(out, values.len() * 4)
}
/// mxSPARSE payload: 0-based int32 row indices (ir), int32 column pointers
/// (jc), then double values (uint8 for a logical sparse array).
fn write_sparse(
    out: &mut impl Write,
    s: &crate::sparse::SparseMatrix,
    cancel: &AtomicBool,
) -> Result<()> {
    write_int32(out, s.rowidx(), cancel)?;
    write_int32(out, s.colptr(), cancel)?;
    match s.values() {
        crate::sparse::SparseValues::Double(values) => {
            tag(out, 9, values.len() * 8)?;
            for chunk in values.chunks(8192) {
                check_cancel(cancel)?;
                for v in chunk {
                    put(out, &v.to_le_bytes())?;
                }
            }
            padding(out, values.len() * 8)?;
        }
        crate::sparse::SparseValues::Logical(values) => {
            tag(out, 2, values.len())?;
            for chunk in values.chunks(65536) {
                check_cancel(cancel)?;
                let bytes: Vec<u8> = chunk.iter().map(|&v| u8::from(v)).collect();
                put(out, &bytes)?;
            }
            padding(out, values.len())?;
        }
    }
    check_cancel(cancel)
}
fn write_array(out: &mut impl Write, name: &str, a: &Array, cancel: &AtomicBool) -> Result<()> {
    if matches!(a, Array::Str(_)) {
        return Err("string arrays cannot be saved to MAT files yet; use cellstr(...)".into());
    }
    if matches!(a, Array::Table(_)) {
        return Err("save: table variables cannot be saved yet".into());
    }
    if matches!(a, Array::Categorical(_)) {
        return Err("categorical is not supported by save yet".into());
    }
    if matches!(a, Array::Datetime(_) | Array::Duration(_)) {
        return Err("datetime/duration variables cannot be saved yet".into());
    }
    if matches!(a, Array::Graphics(_)) {
        return Err("MAT persistence of graphics handles is not implemented".into());
    }
    if matches!(a, Array::Handle(_)) {
        return Err("save: function handles are not supported yet".into());
    }
    let len = encoded_len(name, a, cancel, 1)? - 8;
    tag(out, 14, len)?;
    let mut flags = [0u8; 8];
    flags[..4].copy_from_slice(
        &(if let Array::Sparse(s) = a {
            if s.is_logical() { 0x205u32 } else { 5u32 }
        } else if matches!(a, Array::Cell(_)) {
            1u32
        } else if matches!(a, Array::Struct(_)) {
            2u32
        } else if matches!(a, Array::Character(_)) {
            4u32
        } else if a.is_logical() {
            0x209u32
        } else if let Array::Complex(matrix) = a {
            if matrix.is_single() {
                0x807u32
            } else {
                0x806u32
            }
        } else if let Array::Typed(t) = a {
            super::mat_typed::mx_class(t.class())
        } else {
            6u32
        })
        .to_le_bytes(),
    );
    if let Array::Sparse(s) = a {
        // Second flags word of a sparse array: its capacity (nzmax).
        flags[4..].copy_from_slice(&(s.nnz() as u32).to_le_bytes());
    }
    element(out, 6, &flags)?;
    let dims = a
        .dims()
        .iter()
        .flat_map(|&d| (d as i32).to_le_bytes())
        .collect::<Vec<_>>();
    element(out, 5, &dims)?;
    element(out, 1, name.as_bytes())?;
    if let Array::Struct(st) = a {
        let width = st
            .field_names()
            .iter()
            .map(|n| n.len() + 1)
            .max()
            .unwrap_or(1);
        element(out, 5, &(width as i32).to_le_bytes())?;
        let mut names = vec![0u8; width * st.field_count()];
        for (field, target) in st.field_names().iter().zip(names.chunks_mut(width)) {
            target[..field.len()].copy_from_slice(field.as_bytes());
        }
        element(out, 1, &names)?;
        for index in 0..st.slot_count() {
            check_cancel(cancel)?;
            write_array(out, "", st.slot(index)?, cancel)?;
        }
        return check_cancel(cancel);
    }
    if let Array::Cell(cell) = a {
        for index in 0..cell.len() {
            check_cancel(cancel)?;
            write_array(out, "", cell.get_linear(index)?.as_ref(), cancel)?;
        }
        return check_cancel(cancel);
    }
    if let Array::Character(row) = a {
        tag(out, 17, row.bytes())?;
        let mut bytes = [0u8; 2048];
        for chunk in row.units().chunks(1024) {
            check_cancel(cancel)?;
            for (unit, pair) in chunk.iter().zip(bytes.as_chunks_mut::<2>().0) {
                pair.copy_from_slice(&unit.to_le_bytes());
            }
            put(out, &bytes[..chunk.len() * 2])?;
        }
        padding(out, row.bytes())?;
        return check_cancel(cancel);
    }
    if let Array::Complex(matrix) = a {
        return write_complex(out, matrix, cancel);
    }
    if let Array::Sparse(s) = a {
        return write_sparse(out, s, cancel);
    }
    if let Array::Typed(t) = a {
        check_cancel(cancel)?;
        element(out, super::mat_typed::mi_type(t.class()), &t.to_le_bytes())?;
        return check_cancel(cancel);
    }
    tag(out, if a.is_logical() { 2 } else { 9 }, a.bytes())?;
    match a {
        Array::Complex(_) => unreachable!("complex data handled above"),
        Array::Str(_)
        | Array::Character(_)
        | Array::Cell(_)
        | Array::Graphics(_)
        | Array::Struct(_)
        | Array::Typed(_)
        | Array::Handle(_)
        | Array::Categorical(_)
        | Array::Datetime(_)
        | Array::Duration(_)
        | Array::Table(_)
        | Array::Sparse(_) => {
            unreachable!("containers handled above")
        }
        Array::Numeric(m) => {
            for chunk in m.data().chunks(8192) {
                check_cancel(cancel)?;
                #[cfg(target_endian = "little")]
                put(out, bytemuck::cast_slice(chunk))?;
                #[cfg(target_endian = "big")]
                for v in chunk {
                    put(out, &v.to_le_bytes())?;
                }
            }
        }
        Array::Logical(m) => {
            for chunk in m.data().chunks(65536) {
                check_cancel(cancel)?;
                put(out, chunk)?;
            }
        }
    }
    padding(out, a.bytes())
}
pub fn write_array_csv(path: &Path, a: &Array, cancel: &AtomicBool) -> Result<()> {
    // Dispatch once, before formatting cells. Logical values format directly
    // as bytes, while numeric export retains its specialized f64 formatter.
    match a {
        Array::Complex(_) => Err("Complex file export is not implemented".into()),
        Array::Str(_)
        | Array::Character(_)
        | Array::Cell(_)
        | Array::Graphics(_)
        | Array::Struct(_)
        | Array::Typed(_)
        | Array::Handle(_)
        | Array::Categorical(_)
        | Array::Datetime(_)
        | Array::Duration(_)
        | Array::Table(_)
        | Array::Sparse(_) => Err("CSV export requires real/logical numeric data".into()),
        Array::Numeric(m) => write_csv_values(path, m.rows(), m.cols(), |r, c| m.get(r, c), cancel),
        Array::Logical(m) => write_csv_values(
            path,
            m.rows(),
            m.cols(),
            |r, c| u8::from(m.get(r, c)),
            cancel,
        ),
    }
}
fn write_csv_values<T: std::fmt::Display>(
    path: &Path,
    rows: usize,
    cols: usize,
    value: impl Fn(usize, usize) -> T,
    cancel: &AtomicBool,
) -> Result<()> {
    check_cancel(cancel)?;
    let mut bytes = Vec::new();
    for r in 0..rows {
        for c in 0..cols {
            if (r * cols + c).is_multiple_of(8192) {
                check_cancel(cancel)?;
            }
            if c > 0 {
                bytes.push(b',');
            }
            write!(bytes, "{}", value(r, c)).map_err(|e| e.to_string())?;
            if bytes.len() > super::MAX_CSV_BYTES {
                return Err("CSV exceeds its 512 MiB file limit".into());
            }
        }
        bytes.push(b'\n');
    }
    check_cancel(cancel)?;
    atomic_write(path, &bytes)
}

#[cfg(test)]
mod complex_writer_tests {
    use super::*;
    use crate::{Complex64, ComplexMatrix};
    use std::sync::atomic::Ordering;
    #[test]
    fn split_component_writer_observes_active_cancellation_without_source_copy() {
        struct CancelWriter<'a> {
            written: usize,
            stop: usize,
            cancel: &'a AtomicBool,
        }
        impl Write for CancelWriter<'_> {
            fn write(&mut self, bytes: &[u8]) -> std::io::Result<usize> {
                self.written += bytes.len();
                if self.written >= self.stop {
                    self.cancel.store(true, Ordering::Relaxed);
                }
                Ok(bytes.len())
            }
            fn flush(&mut self) -> std::io::Result<()> {
                Ok(())
            }
        }
        let cancel = AtomicBool::new(false);
        let matrix = ComplexMatrix::filled(32769, 1, Complex64::new(1., 2.), &cancel).unwrap();
        let source = Array::Complex(matrix.clone());
        for stop in [65536, 32769 * 8 + 65536] {
            cancel.store(false, Ordering::Relaxed);
            let mut sink = CancelWriter {
                written: 0,
                stop,
                cancel: &cancel,
            };
            assert!(
                write_array(&mut sink, "Z", &source, &cancel)
                    .unwrap_err()
                    .contains("cancelled")
            );
            assert!(sink.written < source.bytes());
            assert!(sink.written <= stop + 65536);
            assert!(source.as_complex().unwrap().shares_storage(&matrix));
        }
    }
}

#[cfg(test)]
mod container_writer_tests {
    use super::*;
    use crate::{CellArray, CharacterRow, Matrix};
    use std::sync::{Arc, atomic::Ordering};
    #[test]
    fn container_emission_observes_stop_without_changing_source() {
        struct Sink<'a> {
            bytes: usize,
            stop: &'a AtomicBool,
        }
        impl Write for Sink<'_> {
            fn write(&mut self, b: &[u8]) -> std::io::Result<usize> {
                self.bytes += b.len();
                if self.bytes >= 4096 {
                    self.stop.store(true, Ordering::Relaxed);
                }
                Ok(b.len())
            }
            fn flush(&mut self) -> std::io::Result<()> {
                Ok(())
            }
        }
        let stop = AtomicBool::new(false);
        let text: Array = CharacterRow::from_text(&"x".repeat(65536), &stop)
            .unwrap()
            .into();
        let payload: Array = Matrix::filled(1, 16384, 3.).unwrap().into();
        let cell: Array =
            CellArray::from_shared(1, 2, vec![Arc::new(text.clone()), Arc::new(payload)], &stop)
                .unwrap()
                .into();
        for value in [text, cell] {
            let alias = value.clone();
            stop.store(false, Ordering::Relaxed);
            let mut sink = Sink {
                bytes: 0,
                stop: &stop,
            };
            assert!(
                write_array(&mut sink, "value", &value, &stop)
                    .unwrap_err()
                    .contains("cancel")
            );
            assert!(sink.bytes <= 4096 + 65536);
            assert!(value.same_snapshot(&alias));
        }
    }
}
