//! Selective v7.3 import. HDF5 owns format decoding; MatRust bounds selections,
//! validates MATLAB metadata and retains column-major arrays without transposing.
use super::super::mat_v73_complex;
use super::{Entry, MAX_VARIABLES, MatCatalog, MatVariable};
use crate::{Array, Complex64, ComplexMatrix, LogicalMatrix, Matrix, Result, dsp::check_cancel};
use hdf5::types::{FixedAscii, FloatSize, IntSize, TypeDescriptor};
use hdf5::{Dataset, H5Type, Hyperslab, LinkType, Location, LocationType, SliceOrIndex};
use std::{
    collections::{BTreeMap, BTreeSet},
    path::Path,
    sync::{LazyLock, atomic::AtomicBool},
};

#[path = "mat_v73_cell_read.rs"]
mod containers;

const MAX_CHUNK_BYTES: usize = 8 * 1024 * 1024;
const TILE_ELEMENTS: usize = 1024 * 1024;

fn h<T>(value: hdf5::Result<T>) -> Result<T> {
    value.map_err(|e| format!("MAT v7.3: {e}"))
}

fn open(path: &Path) -> Result<hdf5::File> {
    static PLUGINS_DISABLED: LazyLock<Result<()>> = LazyLock::new(|| {
        // hdf5-metno-sys 0.12.4 incorrectly declares this argument as a pointer.
        // Match the actual H5PLpublic.h declaration, and hold the wrapper's lock.
        unsafe extern "C" {
            fn H5PLset_loading_state(mask: std::ffi::c_uint) -> std::ffi::c_int;
        }
        let result = hdf5::sync::sync(|| unsafe { H5PLset_loading_state(0) });
        if result < 0 {
            Err("Cannot disable external HDF5 plugins".into())
        } else {
            Ok(())
        }
    });
    PLUGINS_DISABLED.clone()?;
    h(hdf5::File::with_options()
        .with_fapl(|p| p.chunk_cache(521, MAX_CHUNK_BYTES, 0.75))
        .open(path))
}

fn class(location: &Location) -> Result<String> {
    let attr = h(location.attr("MATLAB_class"))?;
    let dtype = h(h(attr.dtype())?.to_descriptor())?;
    if attr.size() != 1 || !matches!(dtype, TypeDescriptor::FixedAscii(1..=32)) {
        return Err("Invalid or oversized MATLAB_class attribute".into());
    }
    let value = h(attr.read_scalar::<FixedAscii<32>>())?;
    // HDF5 does not enforce the declared ASCII character set on disk. Do not
    // call FixedAscii::as_str(), which assumes valid UTF-8 without checking it.
    let bytes = value.as_bytes();
    if !bytes.is_ascii() {
        return Err("Invalid ASCII in MATLAB_class attribute".into());
    }
    std::str::from_utf8(bytes)
        .map(str::to_owned)
        .map_err(|_| "Invalid ASCII in MATLAB_class attribute".into())
}

fn descriptor(class: &str) -> Option<TypeDescriptor> {
    Some(match class {
        "double" => TypeDescriptor::Float(FloatSize::U8),
        "single" => TypeDescriptor::Float(FloatSize::U4),
        "int8" => TypeDescriptor::Integer(IntSize::U1),
        "int16" => TypeDescriptor::Integer(IntSize::U2),
        "int32" => TypeDescriptor::Integer(IntSize::U4),
        "int64" => TypeDescriptor::Integer(IntSize::U8),
        "logical" | "uint8" => TypeDescriptor::Unsigned(IntSize::U1),
        "uint16" => TypeDescriptor::Unsigned(IntSize::U2),
        "uint32" => TypeDescriptor::Unsigned(IntSize::U4),
        "uint64" => TypeDescriptor::Unsigned(IntSize::U8),
        _ => return None,
    })
}

fn storage(dataset: &Dataset) -> Result<()> {
    let plist = h(dataset.dcpl())?;
    let external = hdf5::sync::sync(|| unsafe {
        // Query the count only, avoiding allocations of external filenames.
        hdf5_sys::h5p::H5Pget_external_count(plist.id())
    });
    if external != 0 || h(plist.get_layout())? == hdf5::dataset::Layout::Virtual {
        return Err("External and virtual HDF5 storage is not supported".into());
    }
    if let Some(chunk) = dataset.chunk() {
        let bytes = chunk
            .iter()
            .try_fold(h(dataset.dtype())?.size().max(8), |n, &d| n.checked_mul(d));
        if bytes.is_none_or(|n| n > MAX_CHUNK_BYTES) {
            return Err("HDF5 chunk exceeds the 8 MiB decoded limit".into());
        }
    }
    for filter in dataset.filters() {
        if !matches!(
            filter,
            hdf5::filters::Filter::Deflate(_)
                | hdf5::filters::Filter::Shuffle
                | hdf5::filters::Filter::Fletcher32
        ) {
            return Err("Unsupported HDF5 compression filter".into());
        }
    }
    Ok(())
}

fn metadata(dataset: &Dataset, name: &str) -> Result<MatVariable> {
    let class = class(dataset)?;
    let dtype = h(h(dataset.dtype())?.to_descriptor())?;
    let mut dimensions = dataset.shape();
    let mut unavailable = storage(dataset).err();
    let empty = if let Ok(attr) = dataset.attr("MATLAB_empty") {
        if attr.size() != 1
            || h(h(attr.dtype())?.to_descriptor())? != TypeDescriptor::Unsigned(IntSize::U1)
            || h(attr.read_scalar::<u8>())? != 1
        {
            return Err("Invalid MATLAB_empty attribute".into());
        }
        true
    } else {
        false
    };
    if empty {
        if dimensions.len() != 1
            || !(2..=32).contains(&dimensions[0])
            || dtype != TypeDescriptor::Unsigned(IntSize::U8)
        {
            return Err("Invalid MATLAB empty-array dimensions".into());
        }
        // Empty dimensions are the only payload read during inspection; bound
        // and validate its storage first, including external links and chunks.
        storage(dataset)?;
        dimensions = h(dataset.read_raw::<u64>())?
            .into_iter()
            .map(|n| usize::try_from(n).map_err(|_| "MAT dimension overflow".to_owned()))
            .collect::<Result<_>>()?;
        if !dimensions.contains(&0) {
            return Err("MATLAB_empty array has no empty dimension".into());
        }
    } else {
        dimensions.reverse();
    }
    let mut decoded_bytes = dimensions.iter().try_fold(
        if !empty && matches!(dtype, TypeDescriptor::Compound(_)) {
            16u64
        } else {
            8u64
        },
        |n, &d| n.checked_mul(d as u64),
    );
    if class == "char" || class == "cell" {
        if !empty
            && dtype
                != if class == "char" {
                    TypeDescriptor::Unsigned(IntSize::U2)
                } else {
                    TypeDescriptor::Reference(hdf5::types::Reference::Object)
                }
        {
            unavailable = Some("MATLAB character/cell class does not match storage".into());
        }
        if class == "char" && !empty {
            let decode = (|| -> Result<()> {
                let attr = h(dataset.attr("MATLAB_int_decode"))?;
                if attr.size() != 1
                    || h(h(attr.dtype())?.to_descriptor())? != TypeDescriptor::Integer(IntSize::U4)
                    || h(attr.read_scalar::<i32>())? != 2
                {
                    return Err("Unsupported MATLAB character decoding".into());
                }
                Ok(())
            })();
            if let Err(reason) = decode {
                unavailable = Some(reason);
            }
        }
        if dimensions.len() >= 2 {
            let rows = dimensions[0];
            let cols = dimensions[1];
            if rows > 65536 || cols > 65536 || rows.checked_mul(cols).is_none_or(|n| n > 65536) {
                unavailable =
                    Some("Character/cell array exceeds 65536 elements or dimensions".into());
            }
            decoded_bytes = rows
                .checked_mul(cols)
                .and_then(|n| {
                    n.checked_mul(if class == "char" {
                        2
                    } else {
                        crate::containers::SLOT
                    })
                })
                .map(|n| n as u64);
        }
    } else if descriptor(&class).is_none() {
        unavailable = Some(format!("{class} arrays are not supported"));
    } else if !empty && matches!(dtype, TypeDescriptor::Compound(_)) {
        if class != "double" || !mat_v73_complex::supported(&dtype) {
            unavailable =
                Some("Only dense complex-double real/imag compound storage is supported".into());
        }
    } else if !empty && descriptor(&class).as_ref() != Some(&dtype) {
        unavailable = Some("MATLAB class does not match HDF5 numeric storage".into());
    }
    if dimensions.len() < 2 || dimensions.len() > 32 || dimensions[2..].iter().any(|&d| d != 1) {
        unavailable = Some("Only two-dimensional arrays are supported".into());
    } else if let Err(reason) = if !empty && matches!(dtype, TypeDescriptor::Compound(_)) {
        ComplexMatrix::checked_len(dimensions[0], dimensions[1])
    } else {
        Matrix::checked_len(dimensions[0], dimensions[1])
    } {
        unavailable = Some(reason);
    }
    Ok(MatVariable {
        name: name.into(),
        dimensions,
        class,
        complex: !empty && matches!(dtype, TypeDescriptor::Compound(_)),
        decoded_bytes,
        unavailable,
        compressed: !dataset.filters().is_empty(),
    })
}

pub(super) fn inspect(path: &Path, cancel: &AtomicBool) -> Result<Vec<Entry>> {
    let file = open(path)?;
    let mut entries = Vec::new();
    let mut visited = 0;
    let mut container_visits = 0;
    let mut inspector = containers::Inspector::new(&file);
    h(file.iter_visit_default(|name, link| {
        check_cancel(cancel).map_err(hdf5::Error::from)?;
        visited += 1;
        if visited > MAX_VARIABLES + 1 {
            return Err("MAT file exceeds 10000 variables".into());
        }
        if name == "#refs#" {
            return Ok(());
        }
        if !super::super::valid_name(name) {
            return Err("Invalid MAT variable name".into());
        }
        let info = if link.link_type != LinkType::Hard {
            MatVariable {
                name: name.into(),
                dimensions: vec![],
                class: "link".into(),
                complex: false,
                decoded_bytes: None,
                unavailable: Some("External and soft HDF5 links are not supported".into()),
                compressed: false,
            }
        } else if file.loc_type_by_name(name)? == LocationType::Dataset {
            let dataset = file.dataset(name)?;
            let mut info = metadata(&dataset, name).map_err(hdf5::Error::from)?;
            if info.class == "cell" && info.unavailable.is_none() {
                match inspector.inspect(&dataset, cancel, &mut container_visits) {
                    Ok(bytes) => info.decoded_bytes = Some(bytes as u64),
                    Err(reason) => info.unavailable = Some(reason),
                }
                check_cancel(cancel).map_err(hdf5::Error::from)?;
            }
            info
        } else {
            MatVariable {
                name: name.into(),
                dimensions: vec![],
                class: "group".into(),
                complex: false,
                decoded_bytes: None,
                unavailable: Some("Structured and sparse HDF5 variables are not supported".into()),
                compressed: false,
            }
        };
        if entries.len() >= MAX_VARIABLES {
            return Err("MAT file exceeds 10000 variables".into());
        }
        entries.push(Entry {
            info,
            offset: 0,
            length: 0,
            kind: 0,
        });
        Ok(())
    }))?;
    check_cancel(cancel)?;
    Ok(entries)
}

#[derive(Clone, Copy)]
struct Tile {
    col: usize,
    row: usize,
    ncols: usize,
    nrows: usize,
}
impl Tile {
    fn selection(self, rank: usize) -> Hyperslab {
        let mut slices = vec![SliceOrIndex::Index(0); rank - 2];
        slices.push(SliceOrIndex::SliceCount {
            start: self.col,
            step: 1,
            count: self.ncols,
            block: 1,
        });
        slices.push(SliceOrIndex::SliceCount {
            start: self.row,
            step: 1,
            count: self.nrows,
            block: 1,
        });
        Hyperslab::from(slices)
    }
}

fn each_tile(
    dataset: &Dataset,
    rows: usize,
    cols: usize,
    cancel: &AtomicBool,
    mut visit: impl FnMut(Tile) -> Result<()>,
) -> Result<()> {
    if rows == 0 || cols == 0 {
        return Ok(());
    }
    let rank = dataset.ndim();
    let tile_elements = MAX_CHUNK_BYTES / h(dataset.dtype())?.size().max(8);
    // Group adjacent chunks without splitting them: HDF5 can batch traversal
    // within one native read, and each chunk is still decoded once per tile.
    // Contiguous reads cover at most 8 MiB between cancellation checks.
    let (tile_cols, tile_rows) = if let Some(chunk) = dataset.chunk() {
        let chunk_cols = chunk[rank - 2];
        let chunk_rows = chunk[rank - 1];
        let tile_rows =
            (chunk_rows * (tile_elements / chunk_cols.min(cols) / chunk_rows).max(1)).min(rows);
        let tile_cols = (chunk_cols * (tile_elements / tile_rows / chunk_cols).max(1)).min(cols);
        (tile_cols, tile_rows)
    } else {
        (
            (tile_elements / rows).max(1).min(cols),
            rows.min(tile_elements),
        )
    };
    for col in (0..cols).step_by(tile_cols) {
        for row in (0..rows).step_by(tile_rows) {
            check_cancel(cancel)?;
            visit(Tile {
                col,
                row,
                ncols: tile_cols.min(cols - col),
                nrows: tile_rows.min(rows - row),
            })?;
        }
    }
    check_cancel(cancel)
}

fn tiles<T: H5Type + Copy>(
    dataset: &Dataset,
    rows: usize,
    cols: usize,
    cancel: &AtomicBool,
    mut write: impl FnMut(usize, &[T]) -> Result<()>,
) -> Result<()> {
    each_tile(dataset, rows, cols, cancel, |tile| {
        let data = h(dataset.read_slice_2d::<T, _>(tile.selection(dataset.ndim())))?;
        let data = data.as_slice().ok_or("Noncontiguous HDF5 tile")?;
        for (i, values) in data.chunks_exact(tile.nrows).enumerate() {
            check_cancel(cancel)?;
            write((tile.col + i) * rows + tile.row, values)?;
        }
        Ok(())
    })
}

fn read_direct_into<T>(
    dataset: &Dataset,
    rows: usize,
    cols: usize,
    cancel: &AtomicBool,
    output: &mut [T],
    dtype: &hdf5::Datatype,
) -> Result<()> {
    if output.len() != Matrix::checked_len(rows, cols)? {
        return Err("HDF5 destination size mismatch".into());
    }
    if output.is_empty() {
        return check_cancel(cancel);
    }
    let file_space = h(dataset.space())?;
    let memory_space = h(hdf5::Dataspace::try_new((cols, rows)))?;
    if dtype.size() != std::mem::size_of::<T>() {
        return Err("HDF5 destination element size mismatch".into());
    }
    each_tile(dataset, rows, cols, cancel, |tile| {
        let selected_file = h(file_space.select(tile.selection(dataset.ndim())))?;
        let selected_memory = h(memory_space.select(tile.selection(2)))?;
        h(hdf5::sync::sync(|| {
            // SAFETY: output is an exclusive, initialized native numeric or Complex64 buffer of exactly
            // cols*rows elements. Both checked dataspaces select the same tile,
            // within that allocation; The private callers supply the exact native scalar/compound type and no
            // variable-length values. All handles remain live under its global
            // lock. Direct placement avoids a second buffer and full data copy.
            hdf5::h5check(unsafe {
                hdf5_sys::h5d::H5Dread(
                    dataset.id(),
                    dtype.id(),
                    selected_memory.id(),
                    selected_file.id(),
                    hdf5_sys::h5p::H5P_DEFAULT,
                    output.as_mut_ptr().cast(),
                )
            })
        }))?;
        #[cfg(test)]
        read_probe::completed(cancel);
        Ok(())
    })
}

fn load_matrix(
    dataset: &Dataset,
    info: &MatVariable,
    cancel: &AtomicBool,
    preserve_logical: bool,
) -> Result<Array> {
    if &metadata(dataset, &info.name)? != info {
        return Err("MAT metadata changed since inspection".into());
    }
    let (rows, cols) = (info.dimensions[0], info.dimensions[1]);
    if info.complex {
        if !preserve_logical {
            return Err("Complex arrays require the typed loader".into());
        }
        let count = ComplexMatrix::checked_len(rows, cols)?;
        let mut output = Vec::new();
        output
            .try_reserve_exact(count)
            .map_err(|_| "Cannot allocate complex MAT output")?;
        for first in (0..count).step_by(4096) {
            check_cancel(cancel)?;
            output.resize((first + 4096).min(count), Complex64::new(0., 0.));
        }
        let dtype = h(hdf5::Datatype::from_descriptor(
            &mat_v73_complex::descriptor(),
        ))?;
        read_direct_into(dataset, rows, cols, cancel, &mut output, &dtype)?;
        return Ok(ComplexMatrix::from_column_vec(rows, cols, output, cancel)?.into());
    }
    let count = Matrix::checked_len(rows, cols)?;
    if preserve_logical && let Some(class) = crate::NumericClass::parse(&info.class) {
        macro_rules! read_typed {
            ($variant:ident, $t:ty) => {{
                let mut output = Vec::<$t>::new();
                output
                    .try_reserve_exact(count)
                    .map_err(|_| "Cannot allocate typed MAT output")?;
                for first in (0..count).step_by(8192) {
                    check_cancel(cancel)?;
                    output.resize((first + 8192).min(count), 0 as $t);
                }
                let dtype = h(hdf5::Datatype::from_type::<$t>())?;
                read_direct_into(dataset, rows, cols, cancel, &mut output, &dtype)?;
                crate::typed_array::TypedData::$variant(output.into())
            }};
        }
        let data = match class {
            crate::NumericClass::Int8 => read_typed!(Int8, i8),
            crate::NumericClass::Uint8 => read_typed!(Uint8, u8),
            crate::NumericClass::Int16 => read_typed!(Int16, i16),
            crate::NumericClass::Uint16 => read_typed!(Uint16, u16),
            crate::NumericClass::Int32 => read_typed!(Int32, i32),
            crate::NumericClass::Uint32 => read_typed!(Uint32, u32),
            crate::NumericClass::Int64 => read_typed!(Int64, i64),
            crate::NumericClass::Uint64 => read_typed!(Uint64, u64),
            crate::NumericClass::Single => read_typed!(Single, f32),
        };
        return Ok(crate::TypedArray::checked(rows, cols, data)?.into());
    }
    if preserve_logical && info.class == "logical" {
        let mut output = vec![0; count];
        tiles::<u8>(dataset, rows, cols, cancel, |offset, values| {
            if values.iter().any(|&v| v > 1) {
                return Err("Invalid logical MAT values".into());
            }
            output[offset..offset + values.len()].copy_from_slice(values);
            Ok(())
        })?;
        return Ok(LogicalMatrix::from_canonical_vec(rows, cols, output)?.into());
    }
    let mut output = vec![0.; count];
    match info.class.as_str() {
        "int64" => tiles::<i64>(dataset, rows, cols, cancel, |offset, values| {
            for (out, &value) in output[offset..offset + values.len()].iter_mut().zip(values) {
                if value.unsigned_abs() > 9_007_199_254_740_992 {
                    return Err("Integer values exceed exact f64 precision".into());
                }
                *out = value as f64;
            }
            Ok(())
        })?,
        "uint64" => tiles::<u64>(dataset, rows, cols, cancel, |offset, values| {
            for (out, &value) in output[offset..offset + values.len()].iter_mut().zip(values) {
                if value > 9_007_199_254_740_992 {
                    return Err("Integer values exceed exact f64 precision".into());
                }
                *out = value as f64;
            }
            Ok(())
        })?,
        _ => {
            let dtype = h(hdf5::Datatype::from_type::<f64>())?;
            read_direct_into(dataset, rows, cols, cancel, &mut output, &dtype)?;
            if info.class == "logical" {
                for values in output.chunks(TILE_ELEMENTS) {
                    check_cancel(cancel)?;
                    if values.iter().any(|&v| v != 0. && v != 1.) {
                        return Err("Invalid logical MAT values".into());
                    }
                }
            }
        }
    }
    Ok(Matrix::from_column_vec(rows, cols, output)?.into())
}

pub(super) fn load(
    catalog: &MatCatalog,
    names: &BTreeSet<String>,
    cancel: &AtomicBool,
    preserve_logical: bool,
) -> Result<BTreeMap<String, Array>> {
    let file = open(&catalog.path)?;
    // Recheck root link types before following selected names in a stale file.
    let mut visited = 0;
    h(file.iter_visit_default(|name, link| {
        check_cancel(cancel).map_err(hdf5::Error::from)?;
        visited += 1;
        if visited > MAX_VARIABLES + 1 {
            return Err("MAT file exceeds 10000 variables".into());
        }
        if names.contains(name) && link.link_type != LinkType::Hard {
            return Err("MAT link changed since inspection".into());
        }
        Ok(())
    }))?;
    let mut variables = BTreeMap::new();
    let mut containers = containers::Loader::new(&file, cancel);
    for entry in &catalog.entries {
        if !names.contains(&entry.info.name) {
            continue;
        }
        check_cancel(cancel)?;
        let dataset = h(file.dataset(&entry.info.name))?;
        let value = if matches!(entry.info.class.as_str(), "char" | "cell") {
            if !preserve_logical {
                return Err("Character/cell arrays require typed loading".into());
            }
            containers.load(&dataset, &entry.info)
        } else {
            load_matrix(&dataset, &entry.info, cancel, preserve_logical)
        }
        .map_err(|e| format!("{}: {e}", entry.info.name))?;
        variables.insert(entry.info.name.clone(), value);
    }
    Ok(variables)
}

#[cfg(test)]
mod read_probe {
    use std::{
        cell::Cell,
        sync::atomic::{AtomicBool, Ordering},
    };
    thread_local! { pub static ARMED: Cell<bool> = const { Cell::new(false) }; }
    pub fn completed(cancel: &AtomicBool) {
        if ARMED.with(|flag| flag.replace(false)) {
            cancel.store(true, Ordering::Relaxed);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::Ordering;

    #[test]
    fn complex_direct_read_cancels_after_one_eight_mib_tile_without_publication() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("complex.mat");
        let cancel = AtomicBool::new(false);
        let count = MAX_CHUNK_BYTES / 16 + 1;
        let source: Array = ComplexMatrix::filled(count, 1, Complex64::new(3., -4.), &cancel)
            .unwrap()
            .into();
        super::super::super::save_array_workspace_v73(
            &path,
            &BTreeMap::from([("Z".into(), source.clone())]),
            &cancel,
        )
        .unwrap();
        let catalog = super::super::super::inspect_workspace(&path, &cancel).unwrap();
        struct Reset;
        impl Drop for Reset {
            fn drop(&mut self) {
                read_probe::ARMED.with(|flag| flag.set(false));
            }
        }
        let _reset = Reset;
        read_probe::ARMED.with(|flag| flag.set(true));
        let selected = BTreeSet::from(["Z".to_owned()]);
        let error =
            super::super::super::load_selected_arrays(&catalog, &selected, &cancel).unwrap_err();
        assert!(error.contains("cancel"));
        assert!(cancel.load(Ordering::Relaxed));
        assert!(!read_probe::ARMED.with(|flag| flag.get()));
        cancel.store(false, Ordering::Relaxed);
        let retry =
            super::super::super::load_selected_arrays(&catalog, &selected, &cancel).unwrap();
        assert_eq!(
            retry["Z"].as_complex().unwrap().data(),
            source.as_complex().unwrap().data()
        );
    }

    #[test]
    fn cancellation_stops_after_first_hdf5_tile() {
        let dir = tempfile::tempdir().unwrap();
        let file = hdf5::File::create(dir.path().join("tiles.h5")).unwrap();
        let dataset = file
            .new_dataset::<f64>()
            .shape((2, TILE_ELEMENTS + 1))
            .chunk((1, 8192))
            .create("x")
            .unwrap();
        let cancel = AtomicBool::new(false);
        let mut writes = 0;
        let error = tiles::<f64>(&dataset, TILE_ELEMENTS + 1, 2, &cancel, |_, values| {
            assert_eq!(values.len(), TILE_ELEMENTS);
            writes += 1;
            cancel.store(true, Ordering::Relaxed);
            Ok(())
        })
        .unwrap_err();
        assert!(error.to_lowercase().contains("cancel"));
        assert_eq!(writes, 1);
    }
}
