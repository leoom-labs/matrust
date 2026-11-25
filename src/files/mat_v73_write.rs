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
/// Complex-single records per native transfer, capped by the larger 16-byte
/// `Complex64` memory width rather than the 8-byte wire record.
const SINGLE_TILE_RECORDS: usize = MAX_WRITE_BYTES / std::mem::size_of::<crate::Complex64>();

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
        if matches!(value, Array::Table(_)) {
            return Err("save: table variables cannot be saved yet".into());
        }
        if matches!(value, Array::Categorical(_)) {
            return Err("categorical is not supported by save yet".into());
        }
        if matches!(value, Array::Datetime(_) | Array::Duration(_)) {
            return Err("datetime/duration variables cannot be saved yet".into());
        }
        if matches!(value, Array::Sparse(_)) {
            return Err(format!(
                "save: {name}: sparse variables cannot be saved to v7.3 yet"
            ));
        }
        if matches!(
            value,
            Array::Str(_)
                | Array::Character(_)
                | Array::Cell(_)
                | Array::Struct(_)
                | Array::Graphics(_)
                | Array::Handle(_)
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
        .with_fapl(|p| {
            p.chunk_cache(521, 8 * 1024 * 1024, 0.75)
                .file_locking(false)
        })
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
        let dims = value.dims();
        let dataset = h(file.new_dataset::<u64>().shape(dims.len()).create(name))?;
        h(dataset.write_raw(&dims.iter().map(|&d| d as u64).collect::<Vec<_>>()))?;
        h(h(dataset.new_attr::<u8>().create("MATLAB_empty"))?.write_scalar(&1))?;
        dataset
    } else {
        match value {
            Array::Str(_) => {
                return Err(
                    "string arrays cannot be saved to MAT files yet; use cellstr(...)".into(),
                );
            }
            Array::Handle(_) => {
                return Err("save: function handles are not supported yet".into());
            }
            Array::Table(_) => {
                return Err("save: table variables cannot be saved yet".into());
            }
            Array::Categorical(_) => return Err("categorical is not supported by save yet".into()),
            Array::Datetime(_) | Array::Duration(_) => {
                return Err("datetime/duration variables cannot be saved yet".into());
            }
            Array::Sparse(_) => {
                return Err("sparse variables cannot be saved to v7.3 yet".into());
            }
            Array::Typed(a) => match a.data() {
                crate::typed_array::TypedData::Int8(data) => {
                    write_values_nd(file, name, &a.dims(), data, checkpoint)?
                }
                crate::typed_array::TypedData::Uint8(data) => {
                    write_values_nd(file, name, &a.dims(), data, checkpoint)?
                }
                crate::typed_array::TypedData::Int16(data) => {
                    write_values_nd(file, name, &a.dims(), data, checkpoint)?
                }
                crate::typed_array::TypedData::Uint16(data) => {
                    write_values_nd(file, name, &a.dims(), data, checkpoint)?
                }
                crate::typed_array::TypedData::Int32(data) => {
                    write_values_nd(file, name, &a.dims(), data, checkpoint)?
                }
                crate::typed_array::TypedData::Uint32(data) => {
                    write_values_nd(file, name, &a.dims(), data, checkpoint)?
                }
                crate::typed_array::TypedData::Int64(data) => {
                    write_values_nd(file, name, &a.dims(), data, checkpoint)?
                }
                crate::typed_array::TypedData::Uint64(data) => {
                    write_values_nd(file, name, &a.dims(), data, checkpoint)?
                }
                crate::typed_array::TypedData::Single(data) => {
                    write_values_nd(file, name, &a.dims(), data, checkpoint)?
                }
            },
            Array::Cell(_) | Array::Graphics(_) => {
                return Err("Nested graphics handles are not supported in MAT files".into());
            }
            Array::Struct(_) => {
                return Err("MAT persistence of struct values is not supported yet".into());
            }
            Array::Character(text) => {
                write_values_nd(file, name, &text.dims(), text.units(), checkpoint)?
            }
            Array::Complex(matrix) if matrix.is_single() => {
                write_single_complex(file, name, &matrix.dims(), matrix.data(), checkpoint)?
            }
            Array::Complex(matrix) => {
                let dtype = h(hdf5::Datatype::from_descriptor(
                    &mat_v73_complex::descriptor(),
                ))?;
                write_storage_nd(
                    file,
                    name,
                    &matrix.dims(),
                    matrix.data(),
                    &dtype,
                    checkpoint,
                )?
            }
            Array::Numeric(matrix) => {
                write_values_nd(file, name, &matrix.dims(), matrix.data(), checkpoint)?
            }
            Array::Logical(matrix) => {
                write_values_nd(file, name, &matrix.dims(), matrix.data(), checkpoint)?
            }
        }
    };
    write_class(&dataset, value)?;
    Ok(dataset)
}
fn write_class(dataset: &Dataset, value: &Array) -> Result<()> {
    match value {
        Array::Str(_) => {
            return Err("string arrays cannot be saved to MAT files yet; use cellstr(...)".into());
        }
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
        Array::Complex(matrix) if matrix.is_single() => {
            let class = FixedAscii::<6>::from_ascii("single").map_err(|e| e.to_string())?;
            h(h(dataset.new_attr::<FixedAscii<6>>().create("MATLAB_class"))?.write_scalar(&class))?;
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
        Array::Table(_) => {
            return Err("save: table variables cannot be saved yet".into());
        }
        Array::Categorical(_) => return Err("categorical is not supported by save yet".into()),
        Array::Datetime(_) | Array::Duration(_) => {
            return Err("datetime/duration variables cannot be saved yet".into());
        }
        Array::Sparse(_) => {
            return Err("sparse variables cannot be saved to v7.3 yet".into());
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

fn write_values_nd<T: H5Type>(
    file: &hdf5::File,
    name: &str,
    dims: &[usize],
    values: &[T],
    checkpoint: &mut impl FnMut() -> Result<()>,
) -> Result<Dataset> {
    if dims.len() <= 2 {
        return write_values(file, name, dims[0], dims[1], values, checkpoint);
    }
    let dtype = h(hdf5::Datatype::from_type::<T>())?;
    write_storage_nd(file, name, dims, values, &dtype, checkpoint)
}

fn write_storage_nd<T>(
    file: &hdf5::File,
    name: &str,
    dims: &[usize],
    values: &[T],
    dtype: &hdf5::Datatype,
    checkpoint: &mut impl FnMut() -> Result<()>,
) -> Result<Dataset> {
    if dims.len() <= 2 {
        return write_storage(file, name, dims[0], dims[1], values, dtype, checkpoint);
    }
    let count = dims
        .iter()
        .try_fold(1usize, |n, &d| n.checked_mul(d))
        .ok_or("Array dimensions overflow")?;
    if count != values.len() || dtype.size() != std::mem::size_of::<T>() {
        return Err("HDF5 source size mismatch".into());
    }
    let mut shape = dims.to_vec();
    shape.reverse();
    checkpoint()?;
    let dataset = h(file
        .new_dataset_builder()
        .empty_as(&h(dtype.to_descriptor())?)
        .shape(shape)
        .create(name))?;
    if !values.is_empty() {
        // Reversing MATLAB dimensions makes the HDF5 C-order payload identical
        // to the source's column-major linear order.
        let space = h(dataset.space())?;
        h(hdf5::sync::sync(|| {
            // SAFETY: `values` contains exactly `count` initialized elements
            // of the datatype used to create the complete matching dataspace.
            hdf5::h5check(unsafe {
                hdf5_sys::h5d::H5Dwrite(
                    dataset.id(),
                    dtype.id(),
                    space.id(),
                    space.id(),
                    hdf5_sys::h5p::H5P_DEFAULT,
                    values.as_ptr().cast(),
                )
            })
        }))?;
    }
    checkpoint()?;
    Ok(dataset)
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
    let tile_elements = MAX_WRITE_BYTES / std::mem::size_of::<T>();
    write_storage_as(
        file,
        name,
        (rows, cols),
        values,
        (dtype, dtype),
        tile_elements,
        checkpoint,
    )
}

/// Writes `values` (memory type `types.0`, exactly `T`) into a new 2-D dataset
/// stored as `types.1`, at most `tile_elements` elements per native transfer.
fn write_storage_as<T>(
    file: &hdf5::File,
    name: &str,
    (rows, cols): (usize, usize),
    values: &[T],
    (dtype, wire): (&hdf5::Datatype, &hdf5::Datatype),
    tile_elements: usize,
    checkpoint: &mut impl FnMut() -> Result<()>,
) -> Result<Dataset> {
    if rows.checked_mul(cols) != Some(values.len())
        || dtype.size() != std::mem::size_of::<T>()
        || tile_elements == 0
    {
        return Err("HDF5 source size mismatch".into());
    }
    let shape = (cols, rows);
    let descriptor = h(wire.to_descriptor())?;
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
    let element_bytes = wire.size();
    // A chunk never exceeds one transfer, so a batch of whole chunks is bounded.
    let chunk_elements = (MAX_CHUNK_BYTES / element_bytes).min(tile_elements);
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
            let records = selected_file.selection_size();
            if records > tile_elements || selected_memory.selection_size() != records {
                return Err("HDF5 write tile exceeds its transfer bound".into());
            }
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
            #[cfg(test)]
            write_probe::transfer(records);
        }
    }
    Ok(dataset)
}

fn single_tile_records() -> usize {
    #[cfg(test)]
    if let Some(records) = write_probe::SINGLE_TILE.with(std::cell::Cell::get) {
        return records.clamp(1, SINGLE_TILE_RECORDS);
    }
    SINGLE_TILE_RECORDS
}

/// Saves tagged complex single from its actual `Complex64` backing into packed
/// `(real:f32, imag:f32)` records. HDF5 narrows each named member inside its
/// own bounded conversion buffer; the values are already single-representable.
/// No staging copy: at most `SINGLE_TILE_RECORDS` records per native transfer
/// for any rank, with a checkpoint before each transfer.
fn write_single_complex(
    file: &hdf5::File,
    name: &str,
    dims: &[usize],
    values: &[crate::Complex64],
    checkpoint: &mut impl FnMut() -> Result<()>,
) -> Result<Dataset> {
    let memory_type = h(hdf5::Datatype::from_descriptor(
        &mat_v73_complex::descriptor(),
    ))?;
    let wire = h(hdf5::Datatype::from_descriptor(
        &mat_v73_complex::single_wire_descriptor(),
    ))?;
    if memory_type.size() != std::mem::size_of::<crate::Complex64>()
        || wire.size() != 2 * std::mem::size_of::<f32>()
        || dims.len() < 2
    {
        return Err("HDF5 complex-single source size mismatch".into());
    }
    let bound = single_tile_records();
    if dims.len() == 2 {
        return write_storage_as(
            file,
            name,
            (dims[0], dims[1]),
            values,
            (&memory_type, &wire),
            bound,
            checkpoint,
        );
    }
    let count = dims
        .iter()
        .try_fold(1usize, |n, &d| n.checked_mul(d))
        .ok_or("Array dimensions overflow")?;
    if count != values.len() {
        return Err("HDF5 complex-single source size mismatch".into());
    }
    // Reversed MATLAB dimensions: the HDF5 C order is the source's column-major order.
    let mut shape = dims.to_vec();
    shape.reverse();
    checkpoint()?;
    let dataset = h(file
        .new_dataset_builder()
        .empty_as(&h(wire.to_descriptor())?)
        .shape(shape.clone())
        .create(name))?;
    if count == 0 {
        return Ok(dataset);
    }
    // Box sides in HDF5 axis order: whole fastest axes while they fit, then a
    // partial axis, then single slices; each box holds at most `bound` records.
    let rank = shape.len();
    let mut step = vec![1; rank];
    let mut room = bound;
    for axis in (0..rank).rev() {
        step[axis] = shape[axis].min(room);
        if step[axis] < shape[axis] {
            break;
        }
        room /= shape[axis];
    }
    let file_space = h(dataset.space())?;
    let memory_space = h(hdf5::Dataspace::try_new(shape.clone()))?;
    let mut start = vec![0; rank];
    let mut transferred = 0usize;
    loop {
        checkpoint()?;
        let slab = || {
            Hyperslab::from(
                (0..rank)
                    .map(|axis| SliceOrIndex::SliceCount {
                        start: start[axis],
                        step: 1,
                        count: step[axis].min(shape[axis] - start[axis]),
                        block: 1,
                    })
                    .collect::<Vec<_>>(),
            )
        };
        let selected_file = h(file_space.select(slab()))?;
        let selected_memory = h(memory_space.select(slab()))?;
        let records = selected_file.selection_size();
        if records == 0 || records > bound || selected_memory.selection_size() != records {
            return Err("HDF5 complex-single tile exceeds its transfer bound".into());
        }
        h(hdf5::sync::sync(|| {
            // SAFETY: `values` is the immutable, initialized `Complex64` backing
            // of exactly `count` elements, the extent of `memory_space`, and
            // `memory_type` is the checked 16-byte `Complex64` layout. Both
            // validated dataspaces select the same in-bounds box of `records`
            // whole elements. Members are fixed-size floats and every handle
            // stays live under the HDF5 global lock.
            hdf5::h5check(unsafe {
                hdf5_sys::h5d::H5Dwrite(
                    dataset.id(),
                    memory_type.id(),
                    selected_memory.id(),
                    selected_file.id(),
                    hdf5_sys::h5p::H5P_DEFAULT,
                    values.as_ptr().cast(),
                )
            })
        }))?;
        transferred = transferred
            .checked_add(records)
            .ok_or("HDF5 tile size overflow")?;
        #[cfg(test)]
        write_probe::transfer(records);
        // Advance the box origin, slowest axis outermost.
        let mut axis = rank;
        loop {
            if axis == 0 {
                if transferred != count {
                    return Err("HDF5 complex-single tiles do not cover the array".into());
                }
                return Ok(dataset);
            }
            axis -= 1;
            start[axis] += step[axis];
            if start[axis] < shape[axis] {
                break;
            }
            start[axis] = 0;
        }
    }
}

#[cfg(test)]
mod write_probe {
    use std::cell::{Cell, RefCell};
    thread_local! {
        pub static SINGLE_TILE: Cell<Option<usize>> = const { Cell::new(None) };
        pub static TRANSFERS: RefCell<Option<Vec<usize>>> = const { RefCell::new(None) };
    }
    pub fn transfer(records: usize) {
        TRANSFERS.with(|t| {
            if let Some(t) = t.borrow_mut().as_mut() {
                t.push(records);
            }
        });
    }
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
    /// Independent raw view of one saved dataset: HDF5 shape, stored type,
    /// `MATLAB_class` and the selected records read back as packed f32 pairs
    /// through a hand-built memory type (not the writer's descriptors).
    struct Raw {
        shape: Vec<usize>,
        stored: hdf5::types::TypeDescriptor,
        class: String,
        chunk: Option<Vec<usize>>,
        records: Vec<[f32; 2]>,
    }
    fn raw(path: &Path, name: &str, range: Option<std::ops::Range<usize>>) -> Raw {
        use hdf5::types::{CompoundField, CompoundType, TypeDescriptor};
        let file = hdf5::File::open(path).unwrap();
        let ds = file.dataset(name).unwrap();
        let attr = ds.attr("MATLAB_class").unwrap();
        assert_eq!(attr.dtype().unwrap().size(), 6, "exact-width class");
        let class = attr
            .read_scalar::<FixedAscii<6>>()
            .unwrap()
            .as_str()
            .to_owned();
        let shape = ds.shape();
        let total: usize = shape.iter().product();
        let range = range.unwrap_or(0..total);
        let pairs = TypeDescriptor::Compound(CompoundType {
            fields: vec![
                CompoundField::new(
                    "real",
                    TypeDescriptor::Float(hdf5::types::FloatSize::U4),
                    0,
                    0,
                ),
                CompoundField::new(
                    "imag",
                    TypeDescriptor::Float(hdf5::types::FloatSize::U4),
                    4,
                    1,
                ),
            ],
            size: 8,
        });
        let memory = hdf5::Datatype::from_descriptor(&pairs).unwrap();
        let mut records = vec![[0f32; 2]; range.len()];
        if !records.is_empty() {
            let file_space = ds.space().unwrap();
            // Flat selection on the C-order linear index of a column-shaped view.
            let flat = hdf5::Dataspace::try_new(range.len()).unwrap();
            let coords: Vec<hdf5_sys::h5::hsize_t> = range
                .clone()
                .flat_map(|mut linear| {
                    let mut c = vec![0; shape.len()];
                    for axis in (0..shape.len()).rev() {
                        c[axis] = (linear % shape[axis]) as hdf5_sys::h5::hsize_t;
                        linear /= shape[axis];
                    }
                    c
                })
                .collect();
            hdf5::sync::sync(|| {
                // SAFETY: `records` holds exactly `range.len()` packed f32 pairs,
                // the size of `memory`, and the point selection has as many cells.
                unsafe {
                    hdf5::h5check(hdf5_sys::h5s::H5Sselect_elements(
                        file_space.id(),
                        hdf5_sys::h5s::H5S_seloper_t::H5S_SELECT_SET,
                        range.len(),
                        coords.as_ptr(),
                    ))?;
                    hdf5::h5check(hdf5_sys::h5d::H5Dread(
                        ds.id(),
                        memory.id(),
                        flat.id(),
                        file_space.id(),
                        hdf5_sys::h5p::H5P_DEFAULT,
                        records.as_mut_ptr().cast(),
                    ))
                }
            })
            .unwrap();
        }
        Raw {
            shape,
            stored: ds.dtype().unwrap().to_descriptor().unwrap(),
            class,
            chunk: ds.chunk(),
            records,
        }
    }
    fn wire() -> hdf5::types::TypeDescriptor {
        use hdf5::types::{CompoundField, CompoundType, FloatSize, TypeDescriptor};
        TypeDescriptor::Compound(CompoundType {
            fields: vec![
                CompoundField::new("real", TypeDescriptor::Float(FloatSize::U4), 0, 0),
                CompoundField::new("imag", TypeDescriptor::Float(FloatSize::U4), 4, 1),
            ],
            size: 8,
        })
    }
    /// Distinct single-representable records; no two components repeat.
    fn distinct(count: usize) -> Vec<[f32; 2]> {
        (0..count)
            .map(|i| [i as f32 + 0.25, -(i as f32) - 0.5])
            .collect()
    }
    fn single(dims: &[usize], pairs: &[[f32; 2]]) -> Array {
        let data = pairs
            .iter()
            .map(|&[re, im]| crate::Complex64::new(re.into(), im.into()))
            .collect();
        let m = crate::ComplexMatrix::from_column_vec(
            dims[0],
            dims[1..].iter().product(),
            data,
            &AtomicBool::new(false),
        )
        .unwrap()
        .with_dims(dims)
        .unwrap()
        .with_single_class(true);
        m.into()
    }
    struct Probe;
    impl Probe {
        fn install(bound: Option<usize>) -> Self {
            write_probe::SINGLE_TILE.with(|c| c.set(bound));
            write_probe::TRANSFERS.with(|t| *t.borrow_mut() = Some(Vec::new()));
            Probe
        }
        fn transfers(&self) -> Vec<usize> {
            write_probe::TRANSFERS.with(|t| t.borrow().clone().unwrap())
        }
    }
    impl Drop for Probe {
        fn drop(&mut self) {
            write_probe::SINGLE_TILE.with(|c| c.set(None));
            write_probe::TRANSFERS.with(|t| *t.borrow_mut() = None);
        }
    }
    fn reload(path: &Path, name: &str) -> Array {
        let stop = AtomicBool::new(false);
        let catalog = crate::files::inspect_workspace(path, &stop).unwrap();
        let info = catalog.variables().find(|v| v.name == name).unwrap();
        assert!(info.unavailable.is_none(), "{:?}", info.unavailable);
        let names = std::collections::BTreeSet::from([name.to_owned()]);
        crate::files::load_selected_arrays(&catalog, &names, &stop)
            .unwrap()
            .remove(name)
            .unwrap()
    }
    fn assert_single(value: &Array, dims: &[usize], pairs: &[[f32; 2]]) {
        let Array::Complex(z) = value else {
            panic!("not complex: {}", value.class())
        };
        assert!(z.is_single());
        assert_eq!(value.class(), "single");
        assert_eq!(&*z.dims(), dims);
        let got: Vec<[u64; 2]> = z
            .data()
            .iter()
            .map(|z| [z.re.to_bits(), z.im.to_bits()])
            .collect();
        let want: Vec<[u64; 2]> = pairs
            .iter()
            .map(|&[re, im]| [f64::from(re).to_bits(), f64::from(im).to_bits()])
            .collect();
        assert_eq!(got, want);
    }
    fn bits(pairs: &[[f32; 2]]) -> Vec<[u32; 2]> {
        pairs.iter().map(|p| p.map(f32::to_bits)).collect()
    }

    #[test]
    fn corpus_single_write_tiles_injected_bound_splits_columns_between_distinct_records() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("tiles.mat");
        let pairs = distinct(14);
        let probe = Probe::install(Some(4));
        let values = BTreeMap::from([("Z".into(), single(&[7, 2], &pairs))]);
        save(&path, &values, || Ok(())).unwrap();
        // 7 rows per column, at most 4 records per transfer.
        assert_eq!(probe.transfers(), [4, 3, 4, 3]);
        drop(probe);
        let raw = raw(&path, "Z", None);
        assert_eq!(raw.class, "single");
        assert_eq!(raw.stored, wire());
        assert_eq!(raw.shape, [2, 7]);
        assert_eq!(bits(&raw.records), bits(&pairs));
        // Records 3|4 straddle the first transfer edge, 6|7 the column edge.
        assert_eq!(raw.records[3], [3.25, -3.5]);
        assert_eq!(raw.records[4], [4.25, -4.5]);
        assert_single(&reload(&path, "Z"), &[7, 2], &pairs);
    }

    #[test]
    fn corpus_single_write_tiles_asymmetric_nd_keeps_reversed_shape_and_column_major_order() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("nd.mat");
        // Each record encodes its own 1-based MATLAB subscripts.
        let mut pairs = Vec::new();
        for k in 0..4 {
            for j in 0..2 {
                for i in 0..3 {
                    let linear = i + 3 * (j + 2 * k);
                    pairs.push([
                        (100 * (i + 1) + 10 * (j + 1) + k + 1) as f32,
                        -(linear as f32) - 0.5,
                    ]);
                }
            }
        }
        let probe = Probe::install(Some(5));
        let values = BTreeMap::from([("N".into(), single(&[3, 2, 4], &pairs))]);
        save(&path, &values, || Ok(())).unwrap();
        // HDF5 (4,2,3): whole 3-record rows, the next axis no longer fits in 5.
        assert_eq!(probe.transfers(), [3; 8]);
        drop(probe);
        let raw = raw(&path, "N", None);
        assert_eq!(
            (raw.class.as_str(), raw.shape.as_slice()),
            ("single", &[4, 2, 3][..])
        );
        assert_eq!(raw.stored, wire());
        assert_eq!(bits(&raw.records), bits(&pairs));
        assert_eq!(raw.records[1], [211., -1.5]);
        assert_eq!(raw.records[3], [121., -3.5]);
        assert_eq!(raw.records[23], [324., -23.5]);
        assert_single(&reload(&path, "N"), &[3, 2, 4], &pairs);
    }

    #[test]
    fn corpus_single_write_tiles_cancel_after_partial_transfer_preserves_destination_and_alias() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("existing.mat");
        let original = b"\x00MATLAB original \xff bytes\r\n".to_vec();
        std::fs::write(&path, &original).unwrap();
        let pairs = distinct(14);
        let source = single(&[7, 2], &pairs);
        let alias = source.clone();
        let values = BTreeMap::from([("Z".into(), source)]);
        let probe = Probe::install(Some(4));
        let error = save(&path, &values, || {
            if write_probe::TRANSFERS.with(|t| t.borrow().as_ref().unwrap().len()) >= 1 {
                Err("cancelled after a partial single transfer".into())
            } else {
                Ok(())
            }
        })
        .unwrap_err();
        assert!(error.contains("partial single transfer"));
        assert_eq!(probe.transfers(), [4]);
        drop(probe);
        assert_eq!(std::fs::read(&path).unwrap(), original);
        assert_eq!(std::fs::read_dir(dir.path()).unwrap().count(), 1);
        assert!(alias.same_snapshot(&values["Z"]));
        assert_single(&alias, &[7, 2], &pairs);
        save(&path, &values, || Ok(())).unwrap();
        assert_eq!(std::fs::read_dir(dir.path()).unwrap().count(), 1);
        assert_single(&reload(&path, "Z"), &[7, 2], &pairs);
    }

    #[test]
    fn corpus_single_write_tiles_zero_extents_write_no_records_and_keep_class() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("empty.mat");
        let probe = Probe::install(Some(4));
        let values = BTreeMap::from([
            ("A".into(), single(&[0, 3], &[])),
            ("B".into(), single(&[2, 0, 4], &[])),
        ]);
        save(&path, &values, || Ok(())).unwrap();
        assert!(probe.transfers().is_empty());
        drop(probe);
        for (name, dims, shape) in [
            ("A", &[0, 3][..], &[3, 0][..]),
            ("B", &[2, 0, 4], &[4, 0, 2]),
        ] {
            let raw = raw(&path, name, None);
            assert_eq!(
                (raw.class.as_str(), raw.shape.as_slice()),
                ("single", shape)
            );
            assert_eq!(raw.stored, wire());
            assert_single(&reload(&path, name), dims, &[]);
        }
    }

    #[test]
    fn corpus_single_write_tiles_real_bound_is_524288_records_for_matrix_and_nd() {
        assert_eq!(SINGLE_TILE_RECORDS, 524288);
        let count = SINGLE_TILE_RECORDS + 1;
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("edge.mat");
        let mut data = vec![crate::Complex64::new(1., -1.); count];
        data[count - 2] = crate::Complex64::new(0.5, -0.75);
        data[count - 1] = crate::Complex64::new(-2.5, 8.);
        let matrix = crate::ComplexMatrix::from_column_vec(count, 1, data, &AtomicBool::new(false))
            .unwrap()
            .with_single_class(true);
        let nd = matrix
            .reshape(1, count, &AtomicBool::new(false))
            .unwrap()
            .with_dims(&[1, 1, count])
            .unwrap();
        assert!(nd.shares_storage(&matrix) && nd.is_single());
        for (dims, value) in [(&[count, 1][..], matrix), (&[1, 1, count], nd)] {
            let probe = Probe::install(None);
            let values = BTreeMap::from([("Z".into(), Array::from(value))]);
            save(&path, &values, || Ok(())).unwrap();
            assert_eq!(probe.transfers(), [SINGLE_TILE_RECORDS, 1], "{dims:?}");
            drop(probe);
            let raw = raw(&path, "Z", Some(count - 3..count));
            assert_eq!(raw.class, "single");
            assert_eq!(raw.stored, wire());
            assert_eq!(raw.records, [[1., -1.], [0.5, -0.75], [-2.5, 8.]]);
            let mut shape = dims.to_vec();
            shape.reverse();
            assert_eq!(raw.shape, shape);
            // 2-D keeps bounded compressed chunks; N-D stays contiguous.
            assert_eq!(raw.chunk.is_some(), dims.len() == 2);
            if let Some(chunk) = raw.chunk {
                assert!(chunk.iter().product::<usize>() * 8 <= MAX_CHUNK_BYTES);
            }
            let Array::Complex(z) = reload(&path, "Z") else {
                panic!()
            };
            assert!(z.is_single());
            assert_eq!(&*z.dims(), dims);
            assert_eq!(z.data()[count - 2], crate::Complex64::new(0.5, -0.75));
            assert_eq!(z.data()[count - 1], crate::Complex64::new(-2.5, 8.));
        }
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
