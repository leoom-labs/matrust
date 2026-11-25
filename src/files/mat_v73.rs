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
/// Records per complex-single transfer. A wire record is 8 bytes, but each one
/// decodes into a 16-byte `Complex64`, so the 8 MiB bound uses that width.
const SINGLE_TILE_RECORDS: usize = MAX_CHUNK_BYTES / std::mem::size_of::<Complex64>();

fn h<T>(value: hdf5::Result<T>) -> Result<T> {
    value.map_err(|e| format!("MAT v7.3: {e}"))
}

/// Turns off loading of external HDF5 filter plugins (once per process).
pub(in crate::files) fn plugins_off() -> Result<()> {
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
    PLUGINS_DISABLED.clone()
}

fn open(path: &Path) -> Result<hdf5::File> {
    plugins_off()?;
    // No HDF5 file locks (as in hdf5_data.rs): child processes inherit them,
    // network drives refuse them, and a reopen right after a write can hit EAGAIN.
    h(hdf5::File::with_options()
        .with_fapl(|p| {
            p.chunk_cache(521, MAX_CHUNK_BYTES, 0.75)
                .file_locking(false)
        })
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

pub(in crate::files) fn storage(dataset: &Dataset) -> Result<()> {
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
                    || !matches!(
                        h(h(attr.dtype())?.to_descriptor())?,
                        TypeDescriptor::Integer(IntSize::U4 | IntSize::U8)
                    )
                    || h(attr.read_scalar::<i64>())? != 2
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
            let count = if class == "char" {
                dimensions.iter().try_fold(1usize, |n, &d| n.checked_mul(d))
            } else {
                dimensions[0].checked_mul(dimensions[1])
            };
            if dimensions.iter().any(|&d| d > 65536) || count.is_none_or(|n| n > 65536) {
                unavailable =
                    Some("Character/cell array exceeds 65536 elements or dimensions".into());
            }
            decoded_bytes = count
                .and_then(|n| {
                    n.checked_mul(if class == "char" {
                        2
                    } else {
                        crate::containers::SLOT
                    })
                })
                .map(|n| n as u64);
        }
    } else if class == "struct" && empty {
        decoded_bytes = None;
    } else if descriptor(&class).is_none() {
        unavailable = Some(format!("{class} arrays are not supported"));
    } else if !empty && matches!(dtype, TypeDescriptor::Compound(_)) {
        if mat_v73_complex::precision(&class, &dtype).is_none() {
            unavailable = Some(
                "Only dense complex real/imag compound storage with double or single \
                 members matching its class is supported"
                    .into(),
            );
        }
    } else if !empty && descriptor(&class).as_ref() != Some(&dtype) {
        unavailable = Some("MATLAB class does not match HDF5 numeric storage".into());
    }
    if matches!(class.as_str(), "cell" | "struct") && dimensions.len() != 2 {
        unavailable = Some("Cell MAT loading requires exactly two dimensions".into());
    } else if dimensions.len() < 2 || dimensions.len() > 32 {
        unavailable = Some("MAT array rank must be between two and 32".into());
    } else if let Err(reason) =
        crate::files::checked_dimension_product(&dimensions[1..]).and_then(|cols| {
            if !empty && matches!(dtype, TypeDescriptor::Compound(_)) {
                ComplexMatrix::checked_len(dimensions[0], cols)
            } else {
                Matrix::checked_len(dimensions[0], cols)
            }
        })
    {
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
        } else {
            let location = if file.loc_type_by_name(name)? == LocationType::Dataset {
                file.dataset(name)?.as_location()?
            } else {
                file.group(name)?.as_location()?
            };
            let mut info =
                containers::location_metadata(&location, name).map_err(hdf5::Error::from)?;
            if matches!(info.class.as_str(), "cell" | "struct") && info.unavailable.is_none() {
                match inspector.inspect(&location, cancel, &mut container_visits) {
                    Ok(bytes) => info.decoded_bytes = Some(bytes as u64),
                    Err(reason) => info.unavailable = Some(reason),
                }
                check_cancel(cancel).map_err(hdf5::Error::from)?;
            }
            info
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

fn read_nd_into<T>(
    dataset: &Dataset,
    dims: &[usize],
    cancel: &AtomicBool,
    output: &mut [T],
    dtype: &hdf5::Datatype,
) -> Result<()> {
    let count = dims
        .iter()
        .try_fold(1usize, |n, &d| n.checked_mul(d))
        .ok_or("Array dimensions overflow")?;
    if output.len() != count || dtype.size() != std::mem::size_of::<T>() {
        return Err("HDF5 N-D destination size mismatch".into());
    }
    check_cancel(cancel)?;
    if output.is_empty() {
        return Ok(());
    }
    let mut shape = dims.to_vec();
    shape.reverse();
    let memory = h(hdf5::Dataspace::try_new(shape))?;
    let file = h(dataset.space())?;
    h(hdf5::sync::sync(|| {
        // SAFETY: `output` is an exclusive initialized array exactly matching
        // the file and memory dataspaces and the private caller's HDF5 type.
        hdf5::h5check(unsafe {
            hdf5_sys::h5d::H5Dread(
                dataset.id(),
                dtype.id(),
                memory.id(),
                file.id(),
                hdf5_sys::h5p::H5P_DEFAULT,
                output.as_mut_ptr().cast(),
            )
        })
    }))?;
    check_cancel(cancel)
}

fn single_tile_records() -> usize {
    #[cfg(test)]
    {
        if let Some(records) = read_probe::SINGLE_TILE.with(std::cell::Cell::get) {
            return records.min(SINGLE_TILE_RECORDS);
        }
    }
    SINGLE_TILE_RECORDS
}

/// Receives the start and side lengths of one box, in HDF5 axis order.
type BoxVisit<'a> = dyn FnMut(&[usize], &[usize]) -> Result<()> + 'a;

/// Visits `extent`, placed at `origin`, as row-major boxes of at most `step`
/// per axis. Every cell falls in exactly one box.
fn each_box(
    origin: &[usize],
    extent: &[usize],
    step: &[usize],
    visit: &mut BoxVisit<'_>,
) -> Result<()> {
    let rank = extent.len();
    if origin.len() != rank || step.len() != rank || step.contains(&0) {
        return Err("Invalid HDF5 tile plan".into());
    }
    if extent.contains(&0) {
        return Ok(());
    }
    let mut offset = vec![0; rank];
    let (mut start, mut count) = (vec![0; rank], vec![0; rank]);
    loop {
        for axis in 0..rank {
            start[axis] = origin[axis] + offset[axis];
            count[axis] = step[axis].min(extent[axis] - offset[axis]);
        }
        visit(&start, &count)?;
        let mut axis = rank;
        loop {
            if axis == 0 {
                return Ok(());
            }
            axis -= 1;
            offset[axis] = offset[axis].saturating_add(step[axis]);
            if offset[axis] < extent[axis] {
                break;
            }
            offset[axis] = 0;
        }
    }
}

/// Visits every transfer box of a nonempty complex-single dataset in HDF5 axis
/// order; no box holds more than `bound` records, for any rank or layout.
fn each_single_tile(
    shape: &[usize],
    chunk: Option<&[usize]>,
    bound: usize,
    visit: &mut BoxVisit<'_>,
) -> Result<()> {
    let ones = vec![1; shape.len()];
    let align = chunk.unwrap_or(&ones);
    if bound == 0 || align.len() != shape.len() || align.contains(&0) || shape.contains(&0) {
        return Err("Invalid HDF5 tile plan".into());
    }
    let records = |sides: &[usize]| {
        sides
            .iter()
            .try_fold(1usize, |n, &d| n.checked_mul(d))
            .ok_or("HDF5 tile size overflow")
    };
    // One stored chunk clipped to the dataset; a contiguous layout aligns to cells.
    let mut outer: Vec<usize> = align.iter().zip(shape).map(|(&c, &d)| c.min(d)).collect();
    let mut inner = outer.clone();
    if records(&outer)? <= bound {
        // Group whole chunks, fastest axis first: each chunk is decoded for one
        // transfer only and a contiguous layout moves long column-major runs.
        for axis in (0..shape.len()).rev() {
            let room = bound / (records(&outer)? / outer[axis]);
            outer[axis] = if room >= shape[axis] {
                shape[axis]
            } else {
                room / align[axis] * align[axis]
            };
        }
        inner.clone_from(&outer);
    } else {
        // One chunk alone decodes past the bound (its 8-byte wire records fit
        // the chunk limit): split it, slowest axis first. The pieces of a chunk
        // stay consecutive, so the cached chunk is not decoded again.
        for axis in 0..shape.len() {
            let others = records(&inner)? / inner[axis];
            if others.saturating_mul(inner[axis]) <= bound {
                break;
            }
            inner[axis] = (bound / others).max(1);
        }
    }
    let origin = vec![0; shape.len()];
    each_box(&origin, shape, &outer, &mut |start, extent| {
        each_box(start, extent, &inner, &mut *visit)
    })
}

/// Reads a complex-single dataset of any rank straight into its final
/// `Complex64` backing, at most `SINGLE_TILE_RECORDS` decoded records per
/// native transfer and with a cancellation check between transfers.
fn read_single_complex_into(
    dataset: &Dataset,
    dims: &[usize],
    cancel: &AtomicBool,
    output: &mut [Complex64],
) -> Result<()> {
    check_cancel(cancel)?;
    let mut shape = dims.to_vec();
    shape.reverse();
    let count = shape
        .iter()
        .try_fold(1usize, |n, &d| n.checked_mul(d))
        .ok_or("Array dimensions overflow")?;
    // File layout: packed 8-byte (f32, f32) records. Memory layout: the actual
    // `Complex64` elements of `output`; HDF5 widens each named member exactly.
    let memory_type = h(hdf5::Datatype::from_descriptor(
        &mat_v73_complex::descriptor(),
    ))?;
    if dataset.shape() != shape
        || output.len() != count
        || memory_type.size() != std::mem::size_of::<Complex64>()
    {
        return Err("HDF5 complex-single destination size mismatch".into());
    }
    if output.is_empty() {
        // A zero extent has no records: no selection and no transfer.
        return Ok(());
    }
    let bound = single_tile_records();
    let chunk = dataset.chunk();
    let file_space = h(dataset.space())?;
    let memory_space = h(hdf5::Dataspace::try_new(shape.clone()))?;
    let mut transferred = 0usize;
    each_single_tile(&shape, chunk.as_deref(), bound, &mut |start, sides| {
        check_cancel(cancel)?;
        let slab = || {
            Hyperslab::from(
                start
                    .iter()
                    .zip(sides)
                    .map(|(&start, &count)| SliceOrIndex::SliceCount {
                        start,
                        step: 1,
                        count,
                        block: 1,
                    })
                    .collect::<Vec<_>>(),
            )
        };
        // select() rejects any box outside the dataspace extents.
        let selected_file = h(file_space.select(slab()))?;
        let selected_memory = h(memory_space.select(slab()))?;
        let records = selected_file.selection_size();
        if records == 0 || records > bound || selected_memory.selection_size() != records {
            return Err("HDF5 complex-single tile exceeds its transfer bound".into());
        }
        h(hdf5::sync::sync(|| {
            // SAFETY: `output` is an exclusive, initialized `Complex64` slice of
            // exactly `count` elements, the extent of `memory_space`, and
            // `memory_type` is the checked 16-byte `Complex64` layout. Both
            // validated dataspaces select the same in-bounds box, so HDF5 stores
            // `records` whole elements inside the allocation. The members are
            // fixed-size floats (no variable-length data) and every handle stays
            // live under the HDF5 global lock.
            hdf5::h5check(unsafe {
                hdf5_sys::h5d::H5Dread(
                    dataset.id(),
                    memory_type.id(),
                    selected_memory.id(),
                    selected_file.id(),
                    hdf5_sys::h5p::H5P_DEFAULT,
                    output.as_mut_ptr().cast(),
                )
            })
        }))?;
        transferred = transferred
            .checked_add(records)
            .ok_or("HDF5 tile size overflow")?;
        #[cfg(test)]
        read_probe::single_transfer(records, cancel);
        Ok(())
    })?;
    if transferred != count {
        return Err("HDF5 complex-single tiles do not cover the dataset".into());
    }
    check_cancel(cancel)
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
    let rows = info.dimensions[0];
    let cols = crate::files::checked_dimension_product(&info.dimensions[1..])?;
    if info.complex {
        if !preserve_logical {
            return Err("Complex arrays require the typed loader".into());
        }
        // Class and exact member widths must still agree, as at inspection.
        let single =
            match mat_v73_complex::precision(&info.class, &h(h(dataset.dtype())?.to_descriptor())?)
            {
                Some(mat_v73_complex::Precision::Single) => true,
                Some(mat_v73_complex::Precision::Double) => false,
                None => return Err("Complex MAT class does not match its storage".into()),
            };
        let count = ComplexMatrix::checked_len(rows, cols)?;
        let mut output = Vec::new();
        output
            .try_reserve_exact(count)
            .map_err(|_| "Cannot allocate complex MAT output")?;
        for first in (0..count).step_by(4096) {
            check_cancel(cancel)?;
            output.resize((first + 4096).min(count), Complex64::new(0., 0.));
        }
        if single {
            read_single_complex_into(dataset, &info.dimensions, cancel, &mut output)?;
        } else {
            let dtype = h(hdf5::Datatype::from_descriptor(
                &mat_v73_complex::descriptor(),
            ))?;
            if info.dimensions.len() > 2 {
                check_cancel(cancel)?;
                read_nd_into(dataset, &info.dimensions, cancel, &mut output, &dtype)?;
            } else {
                read_direct_into(dataset, rows, cols, cancel, &mut output, &dtype)?;
            }
        }
        // Single components are exact in the binary64 backing; only the class differs.
        return Ok(ComplexMatrix::from_column_vec(rows, cols, output, cancel)?
            .with_dims(&info.dimensions)?
            .with_single_class(single)
            .into());
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
                if info.dimensions.len() > 2 {
                    check_cancel(cancel)?;
                    read_nd_into(dataset, &info.dimensions, cancel, &mut output, &dtype)?;
                } else {
                    read_direct_into(dataset, rows, cols, cancel, &mut output, &dtype)?;
                }
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
        return Ok(crate::TypedArray::checked(rows, cols, data)?
            .with_dims(&info.dimensions)?
            .into());
    }
    if preserve_logical && info.class == "logical" {
        let mut output = vec![0; count];
        if info.dimensions.len() > 2 {
            check_cancel(cancel)?;
            let dtype = h(hdf5::Datatype::from_type::<u8>())?;
            read_nd_into(dataset, &info.dimensions, cancel, &mut output, &dtype)?;
            if output.iter().any(|&v| v > 1) {
                return Err("Invalid logical MAT values".into());
            }
        } else {
            tiles::<u8>(dataset, rows, cols, cancel, |offset, values| {
                if values.iter().any(|&v| v > 1) {
                    return Err("Invalid logical MAT values".into());
                }
                output[offset..offset + values.len()].copy_from_slice(values);
                Ok(())
            })?;
        }
        return Ok(LogicalMatrix::from_canonical_vec(rows, cols, output)?
            .with_dims(&info.dimensions)?
            .into());
    }
    let mut output = vec![0.; count];
    match info.class.as_str() {
        "int64" => {
            let mut values_all = Vec::<i64>::new();
            if info.dimensions.len() > 2 {
                check_cancel(cancel)?;
                let dtype = h(hdf5::Datatype::from_type::<i64>())?;
                values_all.resize(count, 0);
                read_nd_into(dataset, &info.dimensions, cancel, &mut values_all, &dtype)?;
                for (out, value) in output.iter_mut().zip(values_all) {
                    if value.unsigned_abs() > 9_007_199_254_740_992 {
                        return Err("Integer values exceed exact f64 precision".into());
                    }
                    *out = value as f64;
                }
            } else {
                tiles::<i64>(dataset, rows, cols, cancel, |offset, values| {
                    for (out, &value) in
                        output[offset..offset + values.len()].iter_mut().zip(values)
                    {
                        if value.unsigned_abs() > 9_007_199_254_740_992 {
                            return Err("Integer values exceed exact f64 precision".into());
                        }
                        *out = value as f64;
                    }
                    Ok(())
                })?;
            }
        }
        "uint64" => {
            if info.dimensions.len() > 2 {
                check_cancel(cancel)?;
                let dtype = h(hdf5::Datatype::from_type::<u64>())?;
                let mut values = vec![0u64; count];
                read_nd_into(dataset, &info.dimensions, cancel, &mut values, &dtype)?;
                for (out, value) in output.iter_mut().zip(values) {
                    if value > 9_007_199_254_740_992 {
                        return Err("Integer values exceed exact f64 precision".into());
                    }
                    *out = value as f64;
                }
            } else {
                tiles::<u64>(dataset, rows, cols, cancel, |offset, values| {
                    for (out, &value) in
                        output[offset..offset + values.len()].iter_mut().zip(values)
                    {
                        if value > 9_007_199_254_740_992 {
                            return Err("Integer values exceed exact f64 precision".into());
                        }
                        *out = value as f64;
                    }
                    Ok(())
                })?;
            }
        }
        _ => {
            let dtype = h(hdf5::Datatype::from_type::<f64>())?;
            if info.dimensions.len() > 2 {
                check_cancel(cancel)?;
                read_nd_into(dataset, &info.dimensions, cancel, &mut output, &dtype)?;
            } else {
                read_direct_into(dataset, rows, cols, cancel, &mut output, &dtype)?;
            }
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
    Ok(Matrix::from_column_vec(rows, cols, output)?
        .with_dims(&info.dimensions)?
        .into())
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
        let location = if h(file.loc_type_by_name(&entry.info.name))? == LocationType::Dataset {
            h(h(file.dataset(&entry.info.name))?.as_location())?
        } else {
            h(h(file.group(&entry.info.name))?.as_location())?
        };
        let value = if matches!(entry.info.class.as_str(), "char" | "cell" | "struct") {
            if !preserve_logical {
                return Err("Character/cell arrays require typed loading".into());
            }
            containers.load(&location, &entry.info)
        } else {
            load_matrix(
                &h(location.as_dataset())?,
                &entry.info,
                cancel,
                preserve_logical,
            )
        }
        .map_err(|e| format!("{}: {e}", entry.info.name))?;
        variables.insert(entry.info.name.clone(), value);
    }
    Ok(variables)
}

#[cfg(test)]
mod read_probe {
    use std::{
        cell::{Cell, RefCell},
        sync::atomic::{AtomicBool, Ordering},
    };
    thread_local! {
        pub static ARMED: Cell<bool> = const { Cell::new(false) };
        /// Injected record bound for the complex-single transfer loop.
        pub static SINGLE_TILE: Cell<Option<usize>> = const { Cell::new(None) };
        /// Requests cancellation once this many complex-single transfers finished.
        pub static SINGLE_CANCEL_AFTER: Cell<usize> = const { Cell::new(0) };
        /// Records moved by each complex-single transfer, in order.
        pub static SINGLE_TRANSFERS: RefCell<Vec<usize>> = const { RefCell::new(Vec::new()) };
    }
    pub fn completed(cancel: &AtomicBool) {
        if ARMED.with(|flag| flag.replace(false)) {
            cancel.store(true, Ordering::Relaxed);
        }
    }
    pub fn single_transfer(records: usize, cancel: &AtomicBool) {
        let done = SINGLE_TRANSFERS.with(|log| {
            log.borrow_mut().push(records);
            log.borrow().len()
        });
        if SINGLE_CANCEL_AFTER.with(Cell::get) == done {
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

    /// Arms the complex-single probes and resets them when a test ends.
    struct SingleProbe;
    impl SingleProbe {
        fn bound(records: Option<usize>) -> Self {
            Self::reset();
            read_probe::SINGLE_TILE.with(|bound| bound.set(records));
            Self
        }
        fn cancel_after(&self, transfers: usize) {
            read_probe::SINGLE_CANCEL_AFTER.with(|after| after.set(transfers));
        }
        /// Record counts of the transfers made since the previous call.
        fn transfers(&self) -> Vec<usize> {
            read_probe::SINGLE_TRANSFERS.with(|log| std::mem::take(&mut *log.borrow_mut()))
        }
        fn reset() {
            read_probe::SINGLE_TILE.with(|bound| bound.set(None));
            read_probe::SINGLE_CANCEL_AFTER.with(|after| after.set(0));
            read_probe::SINGLE_TRANSFERS.with(|log| log.borrow_mut().clear());
        }
    }
    impl Drop for SingleProbe {
        fn drop(&mut self) {
            Self::reset();
        }
    }

    #[derive(Clone, Copy)]
    enum Layout {
        Contiguous,
        Chunked(&'static [usize]),
        Deflated(&'static [usize]),
    }

    /// A MAT v7.3 container around datasets that `body` writes with raw HDF5
    /// calls, independent of the MatRust writer.
    fn wire_file(path: &Path, body: impl FnOnce(&hdf5::File)) {
        {
            let file = hdf5::File::with_options()
                .with_fcpl(|p| p.userblock(512))
                .create(path)
                .unwrap();
            body(&file);
        }
        let mut header = [0u8; 128];
        header[..20].copy_from_slice(b"MATLAB 7.3 MAT-file,");
        header[124..].copy_from_slice(&[0, 2, b'I', b'M']);
        std::io::Write::write_all(
            &mut std::fs::OpenOptions::new().write(true).open(path).unwrap(),
            &header,
        )
        .unwrap();
    }

    /// One raw complex dataset of packed two-float records (`imag` first when
    /// `reversed`); `values` are (real, imag) in HDF5 row-major order.
    fn wire_dataset<T: H5Type + Copy>(
        file: &hdf5::File,
        name: &str,
        class: &str,
        shape: &[usize],
        layout: Layout,
        reversed: bool,
        values: &[(T, T)],
    ) {
        let width = std::mem::size_of::<T>();
        let (first, second) = if reversed {
            ("imag", "real")
        } else {
            ("real", "imag")
        };
        let wire_type = TypeDescriptor::Compound(hdf5::types::CompoundType {
            fields: vec![
                hdf5::types::CompoundField::typed::<T>(first, 0, 0),
                hdf5::types::CompoundField::typed::<T>(second, width, 1),
            ],
            size: 2 * width,
        });
        let builder = file
            .new_dataset_builder()
            .empty_as(&wire_type)
            .shape(shape.to_vec());
        let dataset = match layout {
            Layout::Contiguous => builder.create(name),
            Layout::Chunked(chunk) => builder.chunk(chunk.to_vec()).create(name),
            Layout::Deflated(chunk) => builder.chunk(chunk.to_vec()).deflate(1).create(name),
        }
        .unwrap();
        // The wire really is the packed record this fixture claims.
        assert_eq!(dataset.dtype().unwrap().to_descriptor().unwrap(), wire_type);
        dataset
            .new_attr::<FixedAscii<16>>()
            .create("MATLAB_class")
            .unwrap()
            .write_scalar(&FixedAscii::<16>::from_ascii(class).unwrap())
            .unwrap();
        let wire: Vec<[T; 2]> = values
            .iter()
            .map(|&(re, im)| if reversed { [im, re] } else { [re, im] })
            .collect();
        assert_eq!(dataset.size(), wire.len());
        if wire.is_empty() {
            return;
        }
        let memory_type = hdf5::Datatype::from_descriptor(&wire_type).unwrap();
        hdf5::sync::sync(|| {
            // SAFETY: `wire` holds one `[T; 2]` per record of the whole dataset
            // (checked above) and `wire_type` is exactly that array's layout.
            hdf5::h5check(unsafe {
                hdf5_sys::h5d::H5Dwrite(
                    dataset.id(),
                    memory_type.id(),
                    hdf5_sys::h5s::H5S_ALL,
                    hdf5_sys::h5s::H5S_ALL,
                    hdf5_sys::h5p::H5P_DEFAULT,
                    wire.as_ptr().cast(),
                )
            })
        })
        .unwrap();
    }

    /// Distinct, exactly representable single records, one per linear index.
    fn distinct(count: usize) -> Vec<(f32, f32)> {
        (0..count)
            .map(|i| (i as f32 + 0.5, -(i as f32) - 0.25))
            .collect()
    }

    fn load_names(
        path: &Path,
        names: &[&str],
        cancel: &AtomicBool,
    ) -> Result<BTreeMap<String, Array>> {
        let catalog = super::super::super::inspect_workspace(path, cancel)?;
        let selected = names.iter().map(|name| (*name).to_owned()).collect();
        super::super::super::load_selected_arrays(&catalog, &selected, cancel)
    }

    fn assert_single(value: &Array, dims: &[usize], expected: &[(f32, f32)]) {
        let z = value.as_complex().unwrap();
        assert!(z.is_single());
        assert_eq!(value.class(), "single");
        assert_eq!(z.dims().as_ref(), dims);
        assert_eq!(z.len(), expected.len());
        for (i, (actual, &(re, im))) in z.data().iter().zip(expected).enumerate() {
            assert_eq!(
                (actual.re.to_bits(), actual.im.to_bits()),
                (f64::from(re).to_bits(), f64::from(im).to_bits()),
                "record {i}"
            );
        }
    }

    /// Row-major linear index of every cell in a box.
    fn box_cells(shape: &[usize], start: &[usize], sides: &[usize]) -> Vec<usize> {
        let mut cells = vec![0];
        for axis in 0..shape.len() {
            cells = cells
                .into_iter()
                .flat_map(|prefix| {
                    (start[axis]..start[axis] + sides[axis]).map(move |i| prefix * shape[axis] + i)
                })
                .collect();
        }
        cells
    }

    /// Grid index of every stored chunk a box touches.
    fn box_chunks(start: &[usize], sides: &[usize], chunk: &[usize]) -> BTreeSet<Vec<usize>> {
        let mut chunks = vec![Vec::new()];
        for axis in 0..start.len() {
            let first = start[axis] / chunk[axis];
            let last = (start[axis] + sides[axis] - 1) / chunk[axis];
            chunks = chunks
                .into_iter()
                .flat_map(|prefix| {
                    (first..=last).map(move |i| {
                        let mut next = prefix.clone();
                        next.push(i);
                        next
                    })
                })
                .collect();
        }
        chunks.into_iter().collect()
    }

    #[test]
    fn corpus_single_read_tiles_plan_covers_each_record_once_within_the_bound() {
        let shapes: [&[usize]; 9] = [
            &[1, 1],
            &[1, 7],
            &[7, 1],
            &[4, 5],
            &[5, 3],
            &[4, 2, 3],
            &[3, 1, 5],
            &[2, 3, 2, 2],
            &[6],
        ];
        let mut plans = 0;
        for shape in shapes {
            let total: usize = shape.iter().product();
            // Contiguous, then every chunk shape up to one past each extent.
            let mut layouts = vec![None];
            let mut chunk = vec![1; shape.len()];
            'chunks: loop {
                layouts.push(Some(chunk.clone()));
                for axis in (0..shape.len()).rev() {
                    chunk[axis] += 1;
                    if chunk[axis] <= shape[axis] + 1 {
                        continue 'chunks;
                    }
                    chunk[axis] = 1;
                }
                break;
            }
            for chunk in &layouts {
                for bound in 1..=total + 1 {
                    let label = format!("shape {shape:?} chunk {chunk:?} bound {bound}");
                    let mut seen = vec![0u8; total];
                    let mut finished = BTreeSet::new();
                    let mut previous = BTreeSet::new();
                    each_single_tile(shape, chunk.as_deref(), bound, &mut |start, sides| {
                        let records: usize = sides.iter().product();
                        assert!((1..=bound).contains(&records), "{label}");
                        for cell in box_cells(shape, start, sides) {
                            seen[cell] += 1;
                        }
                        let Some(chunk) = chunk else {
                            return Ok(());
                        };
                        let touched = box_chunks(start, sides, chunk);
                        let whole_chunks = (0..shape.len()).all(|axis| {
                            let end = start[axis] + sides[axis];
                            start[axis] % chunk[axis] == 0
                                && (end % chunk[axis] == 0 || end == shape[axis])
                        });
                        // A transfer takes whole chunks, or a piece of one chunk.
                        assert!(whole_chunks || touched.len() == 1, "{label}");
                        // No chunk is read again after the loop has left it.
                        assert!(touched.is_disjoint(&finished), "{label}");
                        finished.extend(previous.difference(&touched).cloned());
                        previous = touched;
                        Ok(())
                    })
                    .unwrap();
                    assert!(seen.iter().all(|&n| n == 1), "{label}");
                    plans += 1;
                }
            }
        }
        assert!(plans > 1000, "{plans}");
        // Unusable plans are refused before any transfer.
        let mut never = |_: &[usize], _: &[usize]| -> Result<()> { panic!("visited") };
        assert!(each_single_tile(&[2, 2], None, 0, &mut never).is_err());
        assert!(each_single_tile(&[2, 2], Some(&[2]), 4, &mut never).is_err());
        assert!(each_single_tile(&[2, 2], Some(&[0, 2]), 4, &mut never).is_err());
        assert!(each_single_tile(&[2, 0], None, 4, &mut never).is_err());
        assert_eq!(SINGLE_TILE_RECORDS, 524288);
        assert_eq!(
            SINGLE_TILE_RECORDS * std::mem::size_of::<Complex64>(),
            MAX_CHUNK_BYTES
        );
    }

    #[test]
    fn corpus_single_read_tiles_keep_distinct_records_across_injected_edges() {
        let dir = tempfile::tempdir().unwrap();
        let cancel = AtomicBool::new(false);
        // HDF5 shape (MATLAB reversed), layout, imag-first members, injected
        // record bound, predicted records per transfer.
        type Case = (&'static [usize], Layout, bool, usize, &'static [usize]);
        let cases: [Case; 7] = [
            // A 7-row column is longer than the bound: split inside the column.
            (&[2, 7], Layout::Contiguous, true, 4, &[4, 3, 4, 3]),
            // Whole 3-row columns, one and two per transfer.
            (&[5, 3], Layout::Contiguous, false, 4, &[3, 3, 3, 3, 3]),
            (&[5, 3], Layout::Contiguous, true, 7, &[6, 6, 3]),
            // Two whole 2x2 chunks per transfer, then the clipped edge chunks.
            (&[4, 5], Layout::Chunked(&[2, 2]), true, 8, &[8, 2, 8, 2]),
            // A 12-record chunk exceeds the bound and is split into its rows.
            (
                &[4, 5],
                Layout::Deflated(&[3, 4]),
                false,
                5,
                &[4, 4, 4, 1, 1, 1, 4, 1],
            ),
            (&[4, 5], Layout::Deflated(&[2, 2]), false, 1, &[1; 20]),
            // The whole dataset fits one transfer.
            (&[4, 5], Layout::Chunked(&[3, 4]), true, 20, &[20]),
        ];
        for (case, (shape, layout, reversed, bound, predicted)) in cases.into_iter().enumerate() {
            let values = distinct(shape.iter().product());
            let path = dir.path().join(format!("edge{case}.mat"));
            wire_file(&path, |file| {
                wire_dataset(file, "Z", "single", shape, layout, reversed, &values);
            });
            let probe = SingleProbe::bound(Some(bound));
            let loaded = load_names(&path, &["Z"], &cancel).unwrap();
            let transfers = probe.transfers();
            assert_eq!(transfers, predicted, "case {case}");
            assert!(transfers.iter().all(|&records| records <= bound));
            let dims: Vec<usize> = shape.iter().rev().copied().collect();
            assert_single(&loaded["Z"], &dims, &values);
            if matches!(layout, Layout::Contiguous) && transfers.len() > 1 {
                // The records on either side of the first edge are distinct and
                // both arrive: the last of one transfer, the first of the next.
                let edge = transfers[0];
                let z = loaded["Z"].as_complex().unwrap().data();
                assert_eq!(
                    z[edge - 1],
                    Complex64::new(edge as f64 - 0.5, 0.75 - edge as f64)
                );
                assert_eq!(
                    z[edge],
                    Complex64::new(edge as f64 + 0.5, -(edge as f64) - 0.25)
                );
            }
        }
    }

    #[test]
    fn corpus_single_read_tiles_asymmetric_nd_keeps_column_major_order() {
        let dir = tempfile::tempdir().unwrap();
        let cancel = AtomicBool::new(false);
        // MATLAB 3x2x4 is HDF5 (4,2,3); each record encodes its own subscripts.
        let mut values = Vec::new();
        for k in 0..4 {
            for j in 0..2 {
                for i in 0..3 {
                    values.push((
                        (100 * (i + 1) + 10 * (j + 1) + k + 1) as f32,
                        -((i + 3 * (j + 2 * k)) as f32) - 0.5,
                    ));
                }
            }
        }
        let cases: [(Layout, Option<usize>, &[usize]); 5] = [
            (Layout::Contiguous, Some(5), &[3; 8]),
            (Layout::Chunked(&[2, 2, 2]), Some(8), &[8, 4, 8, 4]),
            (
                Layout::Deflated(&[3, 2, 2]),
                Some(5),
                &[4, 4, 4, 2, 2, 2, 4, 2],
            ),
            // The real bound moves this small array in one transfer.
            (Layout::Contiguous, None, &[24]),
            (Layout::Deflated(&[3, 2, 2]), None, &[24]),
        ];
        for (case, (layout, bound, predicted)) in cases.into_iter().enumerate() {
            let path = dir.path().join(format!("nd{case}.mat"));
            wire_file(&path, |file| {
                wire_dataset(
                    file,
                    "Z",
                    "single",
                    &[4, 2, 3],
                    layout,
                    case % 2 == 0,
                    &values,
                );
            });
            let probe = SingleProbe::bound(bound);
            let loaded = load_names(&path, &["Z"], &cancel).unwrap();
            assert_eq!(probe.transfers(), predicted, "case {case}");
            assert_single(&loaded["Z"], &[3, 2, 4], &values);
            let z = loaded["Z"].as_complex().unwrap();
            for k in 0..4 {
                for j in 0..2 {
                    for i in 0..3 {
                        let linear = i + 3 * (j + 2 * k);
                        assert_eq!(
                            z.data()[linear],
                            Complex64::new(
                                (100 * (i + 1) + 10 * (j + 1) + k + 1) as f64,
                                -(linear as f64) - 0.5
                            ),
                            "case {case} subscript ({i},{j},{k})"
                        );
                    }
                }
            }
        }
    }

    #[test]
    fn corpus_single_read_tiles_cancel_after_one_tile_publishes_nothing_and_retry_recovers() {
        let dir = tempfile::tempdir().unwrap();
        let cancel = AtomicBool::new(false);
        // Binary64 values that single precision cannot hold: the double control.
        let double = [(0.1_f64, 1. + 2f64.powi(-40)), (-1. / 3., f64::MAX)];
        let values = distinct(14);
        let cases: [(Layout, &[usize]); 2] = [
            (Layout::Contiguous, &[4, 3, 4, 3]),
            (Layout::Deflated(&[2, 3]), &[3, 3, 3, 3, 1, 1]),
        ];
        for (case, (layout, predicted)) in cases.into_iter().enumerate() {
            let path = dir.path().join(format!("cancel{case}.mat"));
            wire_file(&path, |file| {
                wire_dataset(
                    file,
                    "A",
                    "double",
                    &[2, 1],
                    Layout::Contiguous,
                    false,
                    &double,
                );
                wire_dataset(file, "Z", "single", &[2, 7], layout, true, &values);
            });
            let catalog = super::super::super::inspect_workspace(&path, &cancel).unwrap();
            let selected = BTreeSet::from(["A".to_owned(), "Z".to_owned()]);
            let probe = SingleProbe::bound(Some(4));
            probe.cancel_after(1);
            // A is complete and Z has one tile when the request arrives: the
            // call fails as a whole and returns neither variable.
            let error = super::super::super::load_selected_arrays(&catalog, &selected, &cancel)
                .unwrap_err();
            assert!(error.to_lowercase().contains("cancel"), "{error}");
            assert!(cancel.load(Ordering::Relaxed));
            assert_eq!(probe.transfers(), predicted[..1], "case {case}");
            cancel.store(false, Ordering::Relaxed);
            probe.cancel_after(0);
            let retry =
                super::super::super::load_selected_arrays(&catalog, &selected, &cancel).unwrap();
            assert_eq!(probe.transfers(), predicted, "case {case}");
            assert_single(&retry["Z"], &[7, 2], &values);
            // The double path is untouched: no single transfers, full precision.
            let a = retry["A"].as_complex().unwrap();
            assert!(!a.is_single());
            assert_eq!(retry["A"].class(), "double");
            assert_eq!(
                a.data(),
                &[
                    Complex64::new(double[0].0, double[0].1),
                    Complex64::new(double[1].0, double[1].1)
                ]
            );
            // The script command publishes every selected variable or none.
            let script = format!("load('cancel{case}.mat')");
            let mut engine = crate::Engine::default();
            engine.set_file_directory(dir.path()).unwrap();
            probe.cancel_after(1);
            let error = engine.execute(&script).unwrap_err();
            assert!(error.to_lowercase().contains("cancel"), "{error}");
            assert_eq!(probe.transfers(), predicted[..1], "case {case}");
            assert!(engine.array("A").is_none() && engine.array("Z").is_none());
            engine.cancellation().store(false, Ordering::Relaxed);
            probe.cancel_after(0);
            engine.execute(&script).unwrap();
            assert_eq!(probe.transfers(), predicted, "case {case}");
            assert_single(&engine.array("Z").unwrap(), &[7, 2], &values);
            assert_eq!(engine.array("A").unwrap().class(), "double");
        }
    }

    #[test]
    fn corpus_single_read_tiles_zero_extents_make_no_transfers() {
        let dir = tempfile::tempdir().unwrap();
        let cancel = AtomicBool::new(false);
        let path = dir.path().join("zero.mat");
        // HDF5 shapes; MATLAB sees them reversed.
        let shapes: [(&str, &[usize]); 4] = [
            ("E03", &[3, 0]),
            ("E30", &[0, 3]),
            ("E00", &[0, 0]),
            ("E302", &[2, 0, 3]),
        ];
        wire_file(&path, |file| {
            for (name, shape) in shapes {
                wire_dataset::<f32>(file, name, "single", shape, Layout::Contiguous, true, &[]);
            }
        });
        let probe = SingleProbe::bound(Some(4));
        let names: Vec<&str> = shapes.iter().map(|(name, _)| *name).collect();
        let loaded = load_names(&path, &names, &cancel).unwrap();
        assert!(probe.transfers().is_empty());
        for (name, shape) in shapes {
            let dims: Vec<usize> = shape.iter().rev().copied().collect();
            assert_single(&loaded[name], &dims, &[]);
        }
        // A pending request still stops a zero-extent read.
        cancel.store(true, Ordering::Relaxed);
        let catalog_cancel = AtomicBool::new(false);
        let catalog = super::super::super::inspect_workspace(&path, &catalog_cancel).unwrap();
        let selected = BTreeSet::from(["E03".to_owned()]);
        assert!(
            super::super::super::load_selected_arrays(&catalog, &selected, &cancel)
                .unwrap_err()
                .to_lowercase()
                .contains("cancel")
        );
        assert!(probe.transfers().is_empty());
    }

    #[test]
    fn corpus_single_read_tiles_real_bound_edge_falls_after_524288_records() {
        let dir = tempfile::tempdir().unwrap();
        let cancel = AtomicBool::new(false);
        let path = dir.path().join("real-bound.mat");
        // One modest case at the real bound: a 4 MiB wire column stored as one
        // chunk of 524289 records. Chunk admission counts its 8-byte wire
        // records, so the chunk is accepted; decoded it is one record over.
        const SHAPE: [usize; 2] = [1, SINGLE_TILE_RECORDS + 1];
        let count = SHAPE[1];
        let values = distinct(count);
        wire_file(&path, |file| {
            wire_dataset(
                file,
                "Z",
                "single",
                &SHAPE,
                Layout::Deflated(&SHAPE),
                false,
                &values,
            );
        });
        let probe = SingleProbe::bound(None);
        let loaded = load_names(&path, &["Z"], &cancel).unwrap();
        // No transfer decodes more than 8 MiB, although the chunk would.
        assert_eq!(probe.transfers(), [SINGLE_TILE_RECORDS, 1]);
        assert_single(&loaded["Z"], &[count, 1], &values);
        let z = loaded["Z"].as_complex().unwrap().data();
        assert_eq!(z[524287], Complex64::new(524287.5, -524287.25));
        assert_eq!(z[524288], Complex64::new(524288.5, -524288.25));
    }
}
