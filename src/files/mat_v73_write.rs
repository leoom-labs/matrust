//! Atomic MATLAB v7.3 export from borrowed column-major workspace arrays.
use super::{mat_v73_complex, persist_atomic, valid_name};
use crate::{Array, Result, dsp::check_cancel};
use hdf5::{Dataset, H5Type, Hyperslab, SliceOrIndex, types::FixedAscii};
use std::{
    collections::BTreeMap,
    io::{Seek, SeekFrom, Write},
    path::Path,
    sync::atomic::AtomicBool,
};

#[path = "mat_v73_cell_write.rs"]
mod containers;

const MAX_WRITE_BYTES: usize = 8 * 1024 * 1024;
const MAX_CHUNK_BYTES: usize = 256 * 1024;

fn h<T>(result: hdf5::Result<T>) -> Result<T> {
    result.map_err(|e| format!("MAT v7.3 save: {e}"))
}

/// Save supported numeric, logical, complex, UTF-16 row and cell arrays as MATLAB v7.3 data.
/// No source array is copied. Cancellation is checked between bounded native
/// writes; a native compression/flush call itself cannot be interrupted.
pub fn save_array_workspace_v73(
    path: &Path,
    variables: &BTreeMap<String, Array>,
    cancel: &AtomicBool,
) -> Result<()> {
    save(path, variables, || check_cancel(cancel))
}

fn save(
    path: &Path,
    variables: &BTreeMap<String, Array>,
    mut checkpoint: impl FnMut() -> Result<()>,
) -> Result<()> {
    checkpoint()?;
    if variables.len() > 10000 {
        return Err("Workspace exceeds 10000 variables".into());
    }
    let mut bytes = 0usize;
    let mut container_nodes = 0;
    let mut container_slots = 0;
    for (name, value) in variables {
        if value.contains_struct() {
            return Err("MAT persistence of struct values is not supported yet".into());
        }
        if matches!(
            value,
            Array::Character(_) | Array::Cell(_) | Array::Graphics(_) | Array::Handle(_)
        ) {
            containers::preflight(
                value,
                0,
                &mut container_nodes,
                &mut container_slots,
                &mut checkpoint,
            )?;
        }
        if !valid_name(name) {
            return Err(format!("Invalid variable name: {name}"));
        }
        bytes = bytes
            .checked_add(value.bytes())
            .ok_or("MAT size overflow")?;
    }
    if bytes > 256 * 1024 * 1024 {
        return Err("Workspace exceeds 256 MiB".into());
    }
    let parent = path
        .parent()
        .filter(|p| !p.as_os_str().is_empty())
        .unwrap_or(Path::new("."));
    let mut temporary = tempfile::NamedTempFile::new_in(parent).map_err(|e| e.to_string())?;
    let file = h(hdf5::File::with_options()
        .with_fcpl(|p| p.userblock(512))
        .with_fapl(|p| p.chunk_cache(521, 8 * 1024 * 1024, 0.75))
        .create(temporary.path()))?;
    let mut containers = containers::Writer::new(&file);
    for (name, value) in variables {
        checkpoint()?;
        containers.write_root(name, value, &mut checkpoint)?;
    }
    drop(containers);
    checkpoint()?;
    h(file.flush())?;
    h(file.close())?;
    checkpoint()?;
    if temporary
        .as_file()
        .metadata()
        .map_err(|e| e.to_string())?
        .len()
        > super::MAX_MAT_BYTES as u64
    {
        return Err("MAT file exceeds its encoded size limit".into());
    }
    let mut header = [b' '; 128];
    let description = b"MATLAB 7.3 MAT-file, Platform: MatRust, HDF5 schema 1.00 .";
    header[..description.len()].copy_from_slice(description);
    header[116..124].fill(0);
    header[124..].copy_from_slice(&[0, 2, b'I', b'M']);
    temporary
        .as_file_mut()
        .seek(SeekFrom::Start(0))
        .and_then(|_| temporary.write_all(&header))
        .and_then(|_| temporary.as_file().sync_all())
        .map_err(|e| e.to_string())?;
    checkpoint()?;
    persist_atomic(temporary, path, |_| {})
}

fn write_leaf(
    file: &hdf5::File,
    name: &str,
    value: &Array,
    checkpoint: &mut impl FnMut() -> Result<()>,
) -> Result<Dataset> {
    let dataset = if value.is_empty() && !value.is_complex() {
        let dataset = h(file.new_dataset::<u64>().shape(2).create(name))?;
        h(dataset.write_raw(&[value.rows() as u64, value.cols() as u64]))?;
        h(h(dataset.new_attr::<u8>().create("MATLAB_empty"))?.write_scalar(&1))?;
        dataset
    } else {
        match value {
            Array::Handle(_) => {
                return Err("save: function handles are not supported yet".into());
            }
            Array::Typed(a) => match a.data() {
                crate::typed_array::TypedData::Int8(data) => {
                    write_values(file, name, a.rows(), a.cols(), data, checkpoint)?
                }
                crate::typed_array::TypedData::Uint8(data) => {
                    write_values(file, name, a.rows(), a.cols(), data, checkpoint)?
                }
                crate::typed_array::TypedData::Int16(data) => {
                    write_values(file, name, a.rows(), a.cols(), data, checkpoint)?
                }
                crate::typed_array::TypedData::Uint16(data) => {
                    write_values(file, name, a.rows(), a.cols(), data, checkpoint)?
                }
                crate::typed_array::TypedData::Int32(data) => {
                    write_values(file, name, a.rows(), a.cols(), data, checkpoint)?
                }
                crate::typed_array::TypedData::Uint32(data) => {
                    write_values(file, name, a.rows(), a.cols(), data, checkpoint)?
                }
                crate::typed_array::TypedData::Int64(data) => {
                    write_values(file, name, a.rows(), a.cols(), data, checkpoint)?
                }
                crate::typed_array::TypedData::Uint64(data) => {
                    write_values(file, name, a.rows(), a.cols(), data, checkpoint)?
                }
                crate::typed_array::TypedData::Single(data) => {
                    write_values(file, name, a.rows(), a.cols(), data, checkpoint)?
                }
            },
            Array::Cell(_) | Array::Graphics(_) => {
                return Err("Nested graphics handles are not supported in MAT files".into());
            }
            Array::Struct(_) => {
                return Err("MAT persistence of struct values is not supported yet".into());
            }
            Array::Character(text) => write_values(
                file,
                name,
                text.rows(),
                text.cols(),
                text.units(),
                checkpoint,
            )?,
            Array::Complex(matrix) => {
                let dtype = h(hdf5::Datatype::from_descriptor(
                    &mat_v73_complex::descriptor(),
                ))?;
                write_storage(
                    file,
                    name,
                    matrix.rows(),
                    matrix.cols(),
                    matrix.data(),
                    &dtype,
                    checkpoint,
                )?
            }
            Array::Numeric(matrix) => write_values(
                file,
                name,
                matrix.rows(),
                matrix.cols(),
                matrix.data(),
                checkpoint,
            )?,
            Array::Logical(matrix) => write_values(
                file,
                name,
                matrix.rows(),
                matrix.cols(),
                matrix.data(),
                checkpoint,
            )?,
        }
    };
    write_class(&dataset, value)?;
    Ok(dataset)
}
fn write_class(dataset: &Dataset, value: &Array) -> Result<()> {
    match value {
        Array::Character(_) | Array::Cell(_) => {
            let class = if matches!(value, Array::Character(_)) {
                "char"
            } else {
                "cell"
            };
            let class = FixedAscii::<4>::from_ascii(class).map_err(|e| e.to_string())?;
            h(h(dataset.new_attr::<FixedAscii<4>>().create("MATLAB_class"))?.write_scalar(&class))?;
            if matches!(value, Array::Character(_)) && !value.is_empty() {
                h(h(dataset.new_attr::<i32>().create("MATLAB_int_decode"))?.write_scalar(&2))?;
            }
        }
        Array::Logical(_) => {
            let class = FixedAscii::<7>::from_ascii("logical").map_err(|e| e.to_string())?;
            h(h(dataset.new_attr::<FixedAscii<7>>().create("MATLAB_class"))?.write_scalar(&class))?;
            if !value.is_empty() {
                h(h(dataset.new_attr::<i32>().create("MATLAB_int_decode"))?.write_scalar(&1))?;
            }
        }
        Array::Numeric(_) | Array::Complex(_) => {
            let class = FixedAscii::<6>::from_ascii("double").map_err(|e| e.to_string())?;
            h(h(dataset.new_attr::<FixedAscii<6>>().create("MATLAB_class"))?.write_scalar(&class))?;
        }
        Array::Graphics(_) => {
            return Err("MAT persistence of graphics handles is not implemented".into());
        }
        Array::Struct(_) => {
            return Err("MAT persistence of struct values is not supported yet".into());
        }
        Array::Handle(_) => {
            return Err("save: function handles are not supported yet".into());
        }
        Array::Typed(a) => {
            // Exact width, as for the classes above: no NUL padding.
            let class = a.class().name();
            macro_rules! exact {
                ($n:literal) => {{
                    let class = FixedAscii::<$n>::from_ascii(class).map_err(|e| e.to_string())?;
                    h(
                        h(dataset.new_attr::<FixedAscii<$n>>().create("MATLAB_class"))?
                            .write_scalar(&class),
                    )?;
                }};
            }
            match class.len() {
                4 => exact!(4),
                5 => exact!(5),
                6 => exact!(6),
                _ => unreachable!("typed class names have 4 to 6 characters"),
            }
        }
    }
    Ok(())
}

fn write_values<T: H5Type>(
    file: &hdf5::File,
    name: &str,
    rows: usize,
    cols: usize,
    values: &[T],
    checkpoint: &mut impl FnMut() -> Result<()>,
) -> Result<Dataset> {
    let dtype = h(hdf5::Datatype::from_type::<T>())?;
    write_storage(file, name, rows, cols, values, &dtype, checkpoint)
}

// Private callers supply either H5Type's exact descriptor or the asserted
// repr(C) Complex64 descriptor; never accept a caller-provided datatype.
fn write_storage<T>(
    file: &hdf5::File,
    name: &str,
    rows: usize,
    cols: usize,
    values: &[T],
    dtype: &hdf5::Datatype,
    checkpoint: &mut impl FnMut() -> Result<()>,
) -> Result<Dataset> {
    if rows.checked_mul(cols) != Some(values.len()) || dtype.size() != std::mem::size_of::<T>() {
        return Err("HDF5 source size mismatch".into());
    }
    let shape = (cols, rows);
    let descriptor = h(dtype.to_descriptor())?;
    let builder = file
        .new_dataset_builder()
        .empty_as(&descriptor)
        .shape(shape);
    if values.is_empty() {
        // Zero-extent compound retains complex type and exact shape. MATLAB
        // reopening of this form requires independent vendor validation.
        checkpoint()?;
        return h(builder.create(name));
    }
    let element_bytes = std::mem::size_of::<T>();
    let chunk_elements = MAX_CHUNK_BYTES / element_bytes;
    let tile_elements = MAX_WRITE_BYTES / element_bytes;
    let chunk_rows = rows.min(chunk_elements);
    let chunk_cols = cols.min((chunk_elements / chunk_rows).max(1));
    // Small arrays use contiguous storage. Larger arrays use bounded, column-
    // aligned chunks; batches contain complete chunks except the dataset edge.
    let dataset = if std::mem::size_of_val(values) >= 4096 {
        let builder = builder.chunk((chunk_cols, chunk_rows));
        // Reversible byte shuffling exposes repeated exponent/high bytes to
        // DEFLATE without altering numeric values, including NaN payloads.
        let builder = if element_bytes > 1 {
            builder.shuffle()
        } else {
            builder
        };
        h(builder.deflate(1).create(name))?
    } else {
        h(builder.create(name))?
    };
    let memory = h(hdf5::Dataspace::try_new(shape))?;
    let space = h(dataset.space())?;
    let batch_rows = rows.min(tile_elements);
    let batch_cols = (tile_elements / batch_rows / chunk_cols).max(1) * chunk_cols;
    for col in (0..cols).step_by(batch_cols) {
        for row in (0..rows).step_by(batch_rows) {
            checkpoint()?;
            let selection = Hyperslab::from(vec![
                SliceOrIndex::from(col..(col + batch_cols).min(cols)),
                SliceOrIndex::from(row..(row + batch_rows).min(rows)),
            ]);
            let selected_file = h(space.select(selection.clone()))?;
            let selected_memory = h(memory.select(selection))?;
            h(hdf5::sync::sync(|| {
                // SAFETY: caller supplies an immutable matrix's full contiguous
                // storage, exactly rows*cols elements of T (native numeric types or repr(C) Complex64). Both
                // checked dataspaces select identical in-bounds regions. Handles
                // and the borrowed allocation stay live under the HDF5 lock.
                hdf5::h5check(unsafe {
                    hdf5_sys::h5d::H5Dwrite(
                        dataset.id(),
                        dtype.id(),
                        selected_memory.id(),
                        selected_file.id(),
                        hdf5_sys::h5p::H5P_DEFAULT,
                        values.as_ptr().cast(),
                    )
                })
            }))?;
        }
    }
    Ok(dataset)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn complex_partial_save_cancellation_preserves_existing_destination_and_aliases() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("existing.mat");
        std::fs::write(&path, b"original").unwrap();
        let source: Array = crate::ComplexMatrix::filled(
            MAX_WRITE_BYTES / 16 + 1,
            1,
            crate::Complex64::new(2., -3.),
            &AtomicBool::new(false),
        )
        .unwrap()
        .into();
        let values = BTreeMap::from([("Z".into(), source.clone())]);
        let mut checkpoints = 0;
        let error = save(&path, &values, || {
            checkpoints += 1;
            if checkpoints == 4 {
                Err("cancelled after first complex write".into())
            } else {
                Ok(())
            }
        })
        .unwrap_err();
        assert!(error.contains("after first complex write"));
        assert_eq!(checkpoints, 4);
        assert_eq!(std::fs::read(&path).unwrap(), b"original");
        assert_eq!(std::fs::read_dir(dir.path()).unwrap().count(), 1);
        assert!(source.same_snapshot(&values["Z"]));
    }
    #[test]
    fn cancelled_partial_save_preserves_destination_and_removes_temporary_file() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("existing.mat");
        std::fs::write(&path, b"original").unwrap();
        let elements = MAX_WRITE_BYTES / 8 + 1;
        let matrix = crate::Matrix::from_column_vec(elements, 1, vec![2.; elements]).unwrap();
        let variables = BTreeMap::from([("A".into(), matrix.into())]);
        let mut checkpoints = 0;
        let error = save(&path, &variables, || {
            checkpoints += 1;
            if checkpoints == 4 {
                Err("cancelled after first write".into())
            } else {
                Ok(())
            }
        })
        .unwrap_err();
        assert!(error.contains("cancelled after first write"));
        assert_eq!(std::fs::read(&path).unwrap(), b"original");
        assert_eq!(std::fs::read_dir(dir.path()).unwrap().count(), 1);
    }
}
