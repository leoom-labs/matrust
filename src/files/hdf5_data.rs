//! HDF5 datasets for h5read, h5create, h5write and NetCDF-4 ncread, on the
//! statically linked libhdf5 of the mat-v73 feature. Dimensions cross the
//! boundary in MATLAB order (the reverse of HDF5's C order, so the C-order
//! buffer is the column-major MATLAB buffer). External filter plugins stay
//! off and the v7.3 storage checks apply (no external or virtual storage,
//! only deflate/shuffle/fletcher32, bounded chunks).
use super::mat_catalog::{hdf5_plugins_off, hdf5_storage};
use crate::{NumericClass, Result, dsp::check_cancel};
use hdf5::types::{FloatSize, IntSize, TypeDescriptor, VarLenAscii, VarLenUnicode};
use hdf5::{Dataset, H5Type, Hyperslab, SliceOrIndex};
use std::{path::Path, sync::atomic::AtomicBool};

pub use super::array_data::{H5Array, H5Data, H5Selection, NcAttributes};
use super::array_data::{checked_elements, matlab_dims, resolve_selection};

/// Largest decoded read (bytes).
const MAX_READ_BYTES: usize = 2 << 30;
const MAX_CHUNK_BYTES: usize = 8 * 1024 * 1024;

fn h<T>(value: hdf5::Result<T>) -> Result<T> {
    value.map_err(|e| e.to_string())
}

// File locking is off: libhdf5 opens files without close-on-exec, so a
// child process started meanwhile (system, webread's curl) would inherit
// the descriptor and keep the lock after the file is closed here; network
// drives often refuse the lock as well.
fn open_read(path: &Path) -> Result<hdf5::File> {
    hdf5_plugins_off()?;
    h(hdf5::File::with_options()
        .with_fapl(|p| {
            p.chunk_cache(521, MAX_CHUNK_BYTES, 0.75)
                .file_locking(false)
        })
        .open(path))
}

fn open_write(path: &Path) -> Result<hdf5::File> {
    hdf5_plugins_off()?;
    h(hdf5::File::with_options()
        .with_fapl(|p| {
            p.chunk_cache(521, MAX_CHUNK_BYTES, 0.75)
                .file_locking(false)
        })
        .open_rw(path))
}

fn object_name(name: &str) -> Result<String> {
    let trimmed = name.trim_start_matches('/');
    if trimmed.is_empty()
        || name.contains('\0')
        || trimmed
            .split('/')
            .any(|p| p.is_empty() || p == "." || p == "..")
    {
        return Err(format!(
            "'{name}' is not a valid dataset name; use a path such as '/group/data'"
        ));
    }
    Ok(trimmed.to_owned())
}

/// The dataset at `name` ('/a/b' or 'a/b').
fn dataset(file: &hdf5::File, name: &str) -> Result<Dataset> {
    let path = object_name(name)?;
    // Every link on the path must exist before HDF5 is asked for the next.
    let mut prefix = String::new();
    for part in path.split('/') {
        if !prefix.is_empty() {
            prefix.push('/');
        }
        prefix.push_str(part);
        if !file.link_exists(&prefix) {
            return Err(format!("the file has no dataset '/{path}'"));
        }
    }
    file.dataset(&path)
        .map_err(|_| format!("'/{path}' is not a dataset"))
}

/// HDF5 selection and its HDF5-order counts.
fn hyperslab(shape: &[usize], sel: &H5Selection) -> Result<(Hyperslab, Vec<usize>)> {
    let spans = resolve_selection(shape, sel)?;
    let slices = spans
        .iter()
        .map(|s| SliceOrIndex::SliceCount {
            start: s.start,
            step: s.step,
            count: s.count,
            block: 1,
        })
        .collect::<Vec<_>>();
    Ok((
        Hyperslab::from(slices),
        spans.iter().map(|s| s.count).collect(),
    ))
}

/// Reads the selection (or the whole dataset) into `n` elements of
/// `dtype`, a fixed-size type with no variable-length parts, as raw bytes.
fn read_bytes(
    ds: &Dataset,
    slab: Option<&(Hyperslab, Vec<usize>)>,
    dtype: &hdf5::Datatype,
    n: usize,
    cancel: &AtomicBool,
) -> Result<Vec<u8>> {
    check_cancel(cancel)?;
    let width = dtype.size();
    let bytes = n.checked_mul(width).filter(|&b| b <= MAX_READ_BYTES);
    let bytes = bytes.ok_or_else(|| {
        format!(
            "the data exceeds the {} MiB read limit",
            MAX_READ_BYTES >> 20
        )
    })?;
    let mut out = vec![0u8; bytes];
    if n == 0 {
        return Ok(out);
    }
    let (file_space, memory) = match slab {
        Some((selection, counts)) => (
            h(h(ds.space())?.select(selection.clone()))?,
            h(hdf5::Dataspace::try_new(counts.clone()))?,
        ),
        None => (h(ds.space())?, h(ds.space())?),
    };
    if memory.size() != n || file_space.selection_size() != n {
        return Err("internal size mismatch".into());
    }
    h(hdf5::sync::sync(|| {
        // SAFETY: `out` holds exactly n elements of `width` bytes; both
        // dataspaces select n elements and `dtype` has no variable-length
        // parts, so HDF5 writes inside the buffer and allocates nothing in it.
        hdf5::h5check(unsafe {
            hdf5_sys::h5d::H5Dread(
                ds.id(),
                dtype.id(),
                memory.id(),
                file_space.id(),
                hdf5_sys::h5p::H5P_DEFAULT,
                out.as_mut_ptr().cast(),
            )
        })
    }))?;
    check_cancel(cancel)?;
    Ok(out)
}

/// `n` values converted by HDF5 from the file type to the plain number `T`.
fn read_values<T: H5Type + Copy + bytemuck::Pod>(
    ds: &Dataset,
    slab: Option<&(Hyperslab, Vec<usize>)>,
    n: usize,
    cancel: &AtomicBool,
) -> Result<Vec<T>> {
    let dtype = h(hdf5::Datatype::from_type::<T>())?;
    let bytes = read_bytes(ds, slab, &dtype, n, cancel)?;
    Ok(bytes
        .chunks_exact(std::mem::size_of::<T>())
        .map(bytemuck::pod_read_unaligned)
        .collect())
}

fn int_class(signed: bool, size: IntSize) -> NumericClass {
    match (signed, size) {
        (true, IntSize::U1) => NumericClass::Int8,
        (true, IntSize::U2) => NumericClass::Int16,
        (true, IntSize::U4) => NumericClass::Int32,
        (true, IntSize::U8) => NumericClass::Int64,
        (false, IntSize::U1) => NumericClass::Uint8,
        (false, IntSize::U2) => NumericClass::Uint16,
        (false, IntSize::U4) => NumericClass::Uint32,
        (false, IntSize::U8) => NumericClass::Uint64,
    }
}

fn read_dataset(ds: &Dataset, sel: Option<&H5Selection>, cancel: &AtomicBool) -> Result<H5Array> {
    hdf5_storage(ds)?;
    let shape = ds.shape();
    let desc = h(h(ds.dtype())?.to_descriptor())?;
    let slab = sel.map(|s| hyperslab(&shape, s)).transpose()?;
    let counts = slab
        .as_ref()
        .map_or_else(|| shape.clone(), |(_, c)| c.clone());
    let dtype = h(ds.dtype())?;
    // Decoded bytes per element, conversion buffers included.
    let per_element = match desc {
        TypeDescriptor::Float(_) => 8,
        TypeDescriptor::Integer(_) | TypeDescriptor::Unsigned(_) => 8 + 16,
        TypeDescriptor::Boolean => 2,
        _ => dtype.size() + 48,
    };
    let n = checked_elements(&counts, per_element, MAX_READ_BYTES)?;
    let data = match desc {
        TypeDescriptor::Float(FloatSize::U8) => {
            H5Data::Double(read_values(ds, slab.as_ref(), n, cancel)?)
        }
        TypeDescriptor::Float(_) => H5Data::Single(read_values(ds, slab.as_ref(), n, cancel)?),
        TypeDescriptor::Integer(size) => H5Data::Int(
            int_class(true, size),
            read_values::<i64>(ds, slab.as_ref(), n, cancel)?
                .into_iter()
                .map(i128::from)
                .collect(),
        ),
        TypeDescriptor::Unsigned(size) => H5Data::Int(
            int_class(false, size),
            read_values::<u64>(ds, slab.as_ref(), n, cancel)?
                .into_iter()
                .map(i128::from)
                .collect(),
        ),
        // HDF5 booleans are a one-byte enum (FALSE=0, TRUE=1); the stored
        // bytes are read as they are and any nonzero byte is true.
        TypeDescriptor::Boolean if dtype.size() == 1 => H5Data::Logical(
            read_bytes(ds, slab.as_ref(), &dtype, n, cancel)?
                .into_iter()
                .map(|b| u8::from(b != 0))
                .collect(),
        ),
        TypeDescriptor::FixedAscii(_) | TypeDescriptor::FixedUnicode(_) => {
            let width = dtype.size().max(1);
            let bytes = read_bytes(ds, slab.as_ref(), &dtype, n, cancel)?;
            H5Data::Text(
                bytes
                    .chunks_exact(width)
                    .map(|b| {
                        let end = b.iter().position(|&c| c == 0).unwrap_or(b.len());
                        String::from_utf8_lossy(&b[..end])
                            .trim_end_matches(' ')
                            .to_owned()
                    })
                    .collect(),
            )
        }
        TypeDescriptor::VarLenUnicode | TypeDescriptor::VarLenAscii => {
            if slab.is_some() {
                return Err(
                    "reading part of a variable-length text dataset is not supported; read all of it"
                        .into(),
                );
            }
            H5Data::Text(read_varlen_text(ds, &desc, n)?)
        }
        other => {
            return Err(format!(
                "datasets of type {other} are not supported (numbers, logicals and text are)"
            ));
        }
    };
    Ok(H5Array {
        dims: matlab_dims(&counts),
        data,
    })
}

/// Variable-length text, after HDF5 reports the payload fits the read limit.
/// Bytes are decoded as UTF-8 with replacement (never as unchecked str).
fn read_varlen_text(ds: &Dataset, desc: &TypeDescriptor, n: usize) -> Result<Vec<String>> {
    let dtype = match desc {
        TypeDescriptor::VarLenUnicode => h(hdf5::Datatype::from_type::<VarLenUnicode>())?,
        _ => h(hdf5::Datatype::from_type::<VarLenAscii>())?,
    };
    let space = h(ds.space())?;
    let mut payload: hdf5_sys::h5::hsize_t = 0;
    h(hdf5::sync::sync(|| {
        // SAFETY: all handles are live; HDF5 only writes the size.
        hdf5::h5check(unsafe {
            hdf5_sys::h5d::H5Dvlen_get_buf_size(ds.id(), dtype.id(), space.id(), &mut payload)
        })
    }))?;
    let total = usize::try_from(payload)
        .ok()
        .and_then(|p| p.checked_add(n.checked_mul(48)?))
        .filter(|&t| t <= MAX_READ_BYTES);
    if total.is_none() {
        return Err(format!(
            "the text exceeds the {} MiB read limit",
            MAX_READ_BYTES >> 20
        ));
    }
    Ok(match desc {
        TypeDescriptor::VarLenUnicode => h(ds.read_raw::<VarLenUnicode>())?
            .iter()
            .map(|v| String::from_utf8_lossy(v.as_bytes()).into_owned())
            .collect(),
        _ => h(ds.read_raw::<VarLenAscii>())?
            .iter()
            .map(|v| String::from_utf8_lossy(v.as_bytes()).into_owned())
            .collect(),
    })
}

/// h5read: a dataset or a hyperslab of it.
pub fn h5_read(
    path: &Path,
    name: &str,
    sel: Option<&H5Selection>,
    cancel: &AtomicBool,
) -> Result<H5Array> {
    let file = open_read(path)?;
    let ds = dataset(&file, name)?;
    read_dataset(&ds, sel, cancel)
}

fn attr_number(ds: &Dataset, name: &str) -> Option<f64> {
    if !ds.attr_names().ok()?.iter().any(|n| n == name) {
        return None;
    }
    let attr = ds.attr(name).ok()?;
    let desc = attr.dtype().ok()?.to_descriptor().ok()?;
    if !matches!(
        desc,
        TypeDescriptor::Float(_) | TypeDescriptor::Integer(_) | TypeDescriptor::Unsigned(_)
    ) {
        return None;
    }
    // Only a single value is meaningful; larger attributes are ignored.
    if attr.size() != 1 {
        return None;
    }
    attr.read_raw::<f64>().ok()?.first().copied()
}

/// ncread on a NetCDF-4 (HDF5) file: the variable and its fill, missing,
/// scale and offset attributes.
pub fn nc4_read(
    path: &Path,
    name: &str,
    sel: Option<&H5Selection>,
    cancel: &AtomicBool,
) -> Result<(H5Array, NcAttributes)> {
    let file = open_read(path)?;
    let ds = dataset(&file, name).map_err(|_| {
        format!(
            "the file has no variable '{}'",
            name.trim_start_matches('/')
        )
    })?;
    let mut data = read_dataset(&ds, sel, cancel)?;
    // NetCDF-4 stores NC_CHAR as one-byte strings: a character array.
    if let H5Data::Text(texts) = &data.data
        && h(h(ds.dtype())?.to_descriptor())? == TypeDescriptor::FixedAscii(1)
    {
        let units = texts
            .iter()
            .map(|t| t.encode_utf16().next().unwrap_or(0))
            .collect();
        data.data = H5Data::Chars(units);
    }
    let attributes = NcAttributes {
        fill: attr_number(&ds, "_FillValue"),
        missing: attr_number(&ds, "missing_value"),
        scale: attr_number(&ds, "scale_factor"),
        offset: attr_number(&ds, "add_offset"),
    };
    Ok((data, attributes))
}

/// h5create: a new fixed-size dataset (MATLAB-order size) of class `class`.
pub fn h5_create(
    path: &Path,
    name: &str,
    dims: &[usize],
    class: &str,
    chunk: Option<&[usize]>,
    deflate: Option<u8>,
    fill: Option<f64>,
) -> Result<()> {
    let object = object_name(name)?;
    if dims.is_empty() || dims.len() > 32 {
        return Err("the size must have 1 to 32 elements".into());
    }
    let to_hdf5 = |d: &[usize]| -> Vec<usize> {
        if d.len() == 1 {
            d.to_vec()
        } else {
            d.iter().rev().copied().collect()
        }
    };
    let shape = to_hdf5(dims);
    let chunk = chunk.map(|c| {
        if c.len() != dims.len() {
            return Err("ChunkSize must have one element per dimension".to_owned());
        }
        if c.iter().zip(dims).any(|(&c, &d)| c == 0 || c > d.max(1)) {
            return Err("ChunkSize values must be between 1 and the dataset size".to_owned());
        }
        Ok(to_hdf5(c))
    });
    let chunk = chunk.transpose()?;
    if let Some(c) = &chunk
        && c.iter()
            .try_fold(8usize, |n, &d| n.checked_mul(d))
            .is_none_or(|n| n > MAX_CHUNK_BYTES)
    {
        return Err("a chunk can hold at most 8 MiB".into());
    }
    if deflate.is_some() && chunk.is_none() {
        return Err("Deflate needs a ChunkSize".into());
    }
    hdf5_plugins_off()?;
    let file = if path.exists() {
        open_write(path)?
    } else {
        h(hdf5::File::with_options()
            .with_fapl(|p| p.file_locking(false))
            .create(path))?
    };
    // Every existing link on the path must be a group.
    let mut prefix = String::new();
    for part in object.split('/') {
        if !prefix.is_empty() {
            prefix.push('/');
        }
        prefix.push_str(part);
        if file.link_exists(&prefix) {
            if prefix == object {
                return Err(format!("the dataset '/{object}' already exists"));
            }
            if file.group(&prefix).is_err() {
                return Err(format!("'/{prefix}' is not a group"));
            }
        }
    }
    macro_rules! create {
        ($t:ty) => {{
            let mut builder = file.new_dataset::<$t>();
            if let Some(c) = &chunk {
                builder = builder.chunk(c.clone());
            }
            if let Some(level) = deflate {
                builder = builder.deflate(level);
            }
            if let Some(v) = fill {
                builder = builder.fill_value(v as $t);
            }
            h(builder.shape(shape.clone()).create(object.as_str()))?;
        }};
    }
    match class {
        "double" => create!(f64),
        "single" => create!(f32),
        "int8" => create!(i8),
        "int16" => create!(i16),
        "int32" => create!(i32),
        "int64" => create!(i64),
        "uint8" => create!(u8),
        "uint16" => create!(u16),
        "uint32" => create!(u32),
        "uint64" => create!(u64),
        _ => {
            return Err(format!(
                "Datatype '{class}' is not supported; use double, single or an integer class"
            ));
        }
    }
    h(file.flush())?;
    Ok(())
}

/// Writes `T` values to the selection (or the whole dataset).
fn write_values<T: H5Type>(
    ds: &Dataset,
    slab: Option<&(Hyperslab, Vec<usize>)>,
    values: &[T],
) -> Result<()> {
    let dtype = h(hdf5::Datatype::from_type::<T>())?;
    let file_space = match slab {
        Some((selection, _)) => h(h(ds.space())?.select(selection.clone()))?,
        None => h(ds.space())?,
    };
    let counts = slab.map_or_else(|| ds.shape(), |(_, c)| c.clone());
    let memory = h(hdf5::Dataspace::try_new(counts))?;
    if memory.size() != values.len() || file_space.selection_size() != values.len() {
        return Err("internal size mismatch".into());
    }
    if values.is_empty() {
        return Ok(());
    }
    h(hdf5::sync::sync(|| {
        // SAFETY: `values` holds exactly as many T values as both dataspaces select.
        hdf5::h5check(unsafe {
            hdf5_sys::h5d::H5Dwrite(
                ds.id(),
                dtype.id(),
                memory.id(),
                file_space.id(),
                hdf5_sys::h5p::H5P_DEFAULT,
                values.as_ptr().cast(),
            )
        })
    }))?;
    Ok(())
}

/// MATLAB-order dimensions without trailing singletons (at least one).
fn squeeze_trailing(d: &[usize]) -> Vec<usize> {
    let mut v = d.to_vec();
    while v.len() > 1 && v.last() == Some(&1) {
        v.pop();
    }
    v
}

/// h5write: `data` (MATLAB-order `dims`) into an existing dataset, whole or
/// at `start` with `stride` (zero-based, MATLAB order).
pub fn h5_write(
    path: &Path,
    name: &str,
    data: &H5Data,
    dims: &[usize],
    start: Option<&[usize]>,
    stride: Option<&[usize]>,
) -> Result<()> {
    let file = open_write(path)?;
    let ds = dataset(&file, name)?;
    hdf5_storage(&ds)?;
    let shape = ds.shape();
    let numel: usize = dims.iter().product();
    let slab = match start {
        None => {
            let target = matlab_dims(&shape);
            let fits = squeeze_trailing(dims) == squeeze_trailing(&target)
                || shape.len() == 1
                    && numel == shape[0]
                    && dims.iter().filter(|&&d| d != 1).count() <= 1;
            if !fits {
                return Err(format!(
                    "the data is {} but the dataset is {}; give start and count to write part of it",
                    dims.iter()
                        .map(usize::to_string)
                        .collect::<Vec<_>>()
                        .join("x"),
                    target
                        .iter()
                        .map(usize::to_string)
                        .collect::<Vec<_>>()
                        .join("x")
                ));
            }
            None
        }
        Some(start) => {
            let rank = shape.len().max(1);
            let mut count: Vec<Option<usize>> = dims.iter().map(|&d| Some(d)).collect();
            if shape.len() == 1 {
                if dims.iter().filter(|&&d| d != 1).count() > 1 {
                    return Err("the data must be a vector for a one-dimensional dataset".into());
                }
                count = vec![Some(numel), Some(1)];
            } else {
                if count.len() > rank && count[rank..].iter().any(|c| *c != Some(1)) {
                    return Err(format!(
                        "the data has more dimensions than the dataset ({rank})"
                    ));
                }
                count.resize(rank, Some(1));
            }
            let stride = stride.map_or_else(|| vec![1; start.len()], <[usize]>::to_vec);
            let sel = H5Selection {
                start: start.to_vec(),
                count,
                stride,
            };
            Some(hyperslab(&shape, &sel)?)
        }
    };
    let desc = h(h(ds.dtype())?.to_descriptor())?;
    let reals = || -> Vec<f64> {
        match data {
            H5Data::Double(v) => v.clone(),
            H5Data::Single(v) => v.iter().map(|&x| f64::from(x)).collect(),
            H5Data::Int(_, v) => v.iter().map(|&x| x as f64).collect(),
            H5Data::Logical(v) => v.iter().map(|&x| f64::from(x)).collect(),
            H5Data::Text(_) | H5Data::Chars(_) => Vec::new(),
        }
    };
    if matches!(data, H5Data::Text(_) | H5Data::Chars(_)) {
        return Err("text data cannot be written yet; write numbers".into());
    }
    // Integer targets: MATLAB rounding (half away from zero), NaN as 0, saturated.
    let ints = |class: NumericClass| -> Vec<i128> {
        match data {
            H5Data::Int(_, v) => v.iter().map(|&x| class.saturate(x)).collect(),
            _ => reals().into_iter().map(|x| class.from_f64(x)).collect(),
        }
    };
    match desc {
        TypeDescriptor::Float(FloatSize::U8) => write_values(&ds, slab.as_ref(), &reals()),
        TypeDescriptor::Float(_) => {
            let v: Vec<f32> = reals().into_iter().map(|x| x as f32).collect();
            write_values(&ds, slab.as_ref(), &v)
        }
        TypeDescriptor::Integer(size) => {
            let v: Vec<i64> = ints(int_class(true, size))
                .into_iter()
                .map(|x| x as i64)
                .collect();
            write_values(&ds, slab.as_ref(), &v)
        }
        TypeDescriptor::Unsigned(size) => {
            let v: Vec<u64> = ints(int_class(false, size))
                .into_iter()
                .map(|x| x as u64)
                .collect();
            write_values(&ds, slab.as_ref(), &v)
        }
        TypeDescriptor::Boolean => {
            let v: Vec<bool> = reals().into_iter().map(|x| x != 0.).collect();
            write_values(&ds, slab.as_ref(), &v)
        }
        other => Err(format!(
            "writing to datasets of type {other} is not supported"
        )),
    }?;
    h(file.flush())
}
