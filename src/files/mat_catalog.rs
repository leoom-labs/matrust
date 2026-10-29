//! Header-only Level-5 MAT inspection and bounded, cancellable selected loading.
use crate::{Array, LogicalMatrix, Matrix, Result, dsp::check_cancel};
use std::{
    collections::{BTreeMap, BTreeSet},
    fs::File,
    io::{BufReader, Read, Seek, SeekFrom},
    path::{Path, PathBuf},
    sync::atomic::AtomicBool,
    time::SystemTime,
};

#[cfg(test)]
use pair_tests::{ReadPhase, ReadProbe};

const MAX_CONTAINER: u64 = 8 * 1024 * 1024 * 1024;
const MAX_VARIABLES: usize = 10000;
#[path = "mat_complex.rs"]
mod complex;
#[path = "mat_containers.rs"]
mod containers;
#[cfg(feature = "mat-v73")]
#[path = "mat_v73.rs"]
mod v73;
/// Without the HDF5-based mat-v73 feature (browser build) v7.3 files fail cleanly.
#[cfg(not(feature = "mat-v73"))]
mod v73 {
    use super::{Entry, MatCatalog};
    use crate::{Array, Result, files::NO_MAT_V73};
    use std::{
        collections::{BTreeMap, BTreeSet},
        path::Path,
        sync::atomic::AtomicBool,
    };

    pub(super) fn inspect(_path: &Path, _cancel: &AtomicBool) -> Result<Vec<Entry>> {
        Err(NO_MAT_V73.into())
    }

    pub(super) fn load(
        _catalog: &MatCatalog,
        _names: &BTreeSet<String>,
        _cancel: &AtomicBool,
        _preserve_logical: bool,
    ) -> Result<BTreeMap<String, Array>> {
        Err(NO_MAT_V73.into())
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct MatVariable {
    pub name: String,
    pub dimensions: Vec<usize>,
    pub class: String,
    /// Complex storage flag, including arrays with zero imaginary components.
    pub complex: bool,
    pub decoded_bytes: Option<u64>,
    pub unavailable: Option<String>,
    pub compressed: bool,
}
impl MatVariable {
    /// Bytes retained by the typed loader. decoded_bytes includes both complex
    /// components; logical storage uses one byte rather than a real double.
    pub fn storage_bytes(&self) -> Option<u64> {
        // Integer and single values keep their own element size.
        let class = crate::NumericClass::parse(&self.class);
        self.decoded_bytes.map(|n| match class {
            _ if self.class == "logical" => n / 8,
            Some(class) => n / 8 * class.size() as u64,
            None => n,
        })
    }
}
#[derive(Clone, Debug)]
struct Entry {
    info: MatVariable,
    offset: u64,
    length: u32,
    kind: u32,
}
#[derive(Clone, Debug, PartialEq, Eq)]
struct Stamp {
    bytes: u64,
    modified: SystemTime,
}
#[derive(Clone, Debug)]
pub struct MatCatalog {
    path: PathBuf,
    stamp: Stamp,
    entries: Vec<Entry>,
    v73: bool,
}
impl MatCatalog {
    pub fn path(&self) -> &Path {
        &self.path
    }
    pub fn file_bytes(&self) -> u64 {
        self.stamp.bytes
    }
    pub fn variables(&self) -> impl ExactSizeIterator<Item = &MatVariable> {
        self.entries.iter().map(|e| &e.info)
    }
    /// Borrow one metadata row in catalog order without collecting an intermediate list.
    pub fn variable(&self, index: usize) -> Option<&MatVariable> {
        self.entries.get(index).map(|entry| &entry.info)
    }
}
fn stamp(file: &File) -> Result<Stamp> {
    let meta = file.metadata().map_err(|e| e.to_string())?;
    Ok(Stamp {
        bytes: meta.len(),
        modified: meta.modified().map_err(|e| e.to_string())?,
    })
}
fn u32le(bytes: &[u8]) -> u32 {
    u32::from_le_bytes(bytes.try_into().unwrap())
}
fn read(reader: &mut impl Read, bytes: &mut [u8]) -> Result<()> {
    reader
        .read_exact(bytes)
        .map_err(|e| format!("Truncated MAT data: {e}"))
}
fn tag(reader: &mut impl Read) -> Result<[u8; 8]> {
    let mut t = [0; 8];
    read(reader, &mut t)?;
    Ok(t)
}
fn small_element(reader: &mut impl Read, max: usize) -> Result<(u32, Vec<u8>, usize)> {
    let t = tag(reader)?;
    let kind = u32le(&t[..4]);
    let short = (kind >> 16) as usize;
    if short > 0 {
        if short > 4 {
            return Err("Invalid compact MAT tag".into());
        }
        return Ok((kind & 65535, t[4..4 + short].to_vec(), 8));
    }
    let len = u32le(&t[4..]) as usize;
    if len > max {
        return Err("MAT metadata exceeds inspection limit".into());
    }
    let padded = len.next_multiple_of(8);
    let mut body = vec![0; padded];
    read(reader, &mut body)?;
    body.truncate(len);
    Ok((kind, body, 8 + padded))
}
fn header(
    reader: &mut impl Read,
    body_len: usize,
    compressed: bool,
) -> Result<(MatVariable, usize)> {
    header_named(reader, body_len, compressed, false)
}
fn header_named(
    reader: &mut impl Read,
    body_len: usize,
    compressed: bool,
    anonymous: bool,
) -> Result<(MatVariable, usize)> {
    let (kind, flags, n1) = small_element(reader, 8)?;
    if kind != 6 || flags.len() != 8 {
        return Err("Invalid MAT array flags".into());
    }
    let flags = u32le(&flags[..4]);
    let class = flags & 255;
    let (kind, dims, n2) = small_element(reader, 128)?;
    if kind != 5 || dims.len() < 8 || dims.len() % 4 != 0 {
        return Err("Invalid MAT dimensions".into());
    }
    let dimensions: Vec<_> = dims
        .as_chunks::<4>()
        .0
        .iter()
        .map(|b| i32::from_le_bytes(*b))
        .collect();
    if dimensions.iter().any(|&n| n < 0) {
        return Err("Negative MAT dimension".into());
    }
    let dimensions: Vec<usize> = dimensions.into_iter().map(|n| n as usize).collect();
    let (kind, name, n3) = small_element(reader, 64)?;
    if kind != 1 {
        return Err("Invalid MAT variable name tag".into());
    }
    let name = std::str::from_utf8(&name)
        .map_err(|_| "Invalid MAT variable name")?
        .trim_end_matches('\0')
        .to_owned();
    if !(anonymous && name.is_empty()) && !super::valid_name(&name) {
        return Err(format!("Invalid MAT variable name: {name}"));
    }
    let used = n1 + n2 + n3;
    if used > body_len {
        return Err("MAT metadata exceeds array length".into());
    }
    let complex = flags & 0x800 != 0;
    let mut decoded_bytes = dimensions
        .iter()
        .try_fold(if complex { 16u64 } else { 8u64 }, |n, &d| {
            n.checked_mul(d as u64)
        });
    let class_name = match class {
        1 => "cell",
        2 => "struct",
        3 => "object",
        4 => "char",
        5 => "sparse",
        6 => "double",
        7 => "single",
        8 => "int8",
        9 => "uint8",
        10 => "int16",
        11 => "uint16",
        12 => "int32",
        13 => "uint32",
        14 => "int64",
        15 => "uint64",
        _ => "unsupported",
    };
    let logical = flags & 0x200 != 0;
    let container = class == 1 || class == 4;
    if class == 4 {
        decoded_bytes = dimensions
            .iter()
            .try_fold(2u64, |n, &d| n.checked_mul(d as u64));
    }
    if class == 1 {
        decoded_bytes = None;
    }
    let count = dimensions[0].checked_mul(dimensions[1]);
    let unavailable = if container && (complex || logical) {
        Some("Character/cell arrays cannot have numeric logical/complex flags".into())
    } else if container && dimensions.len() != 2 {
        Some("Character/cell MAT loading requires exactly two dimensions".into())
    } else if container
        && (dimensions[..2].iter().any(|&n| n > 65536) || count.is_none_or(|n| n > 65536))
    {
        Some("Character/cell dimensions exceed 65536 units/slots".into())
    } else if class == 1 && body_len > super::MAX_MAT_BYTES {
        Some("Cell encoded extent exceeds 272 MiB".into())
    } else if complex && (class != 6 || logical) {
        Some("Only nonlogical dense complex-double arrays are supported".into())
    } else if !container && !(6..=15).contains(&class) {
        Some(format!("{class_name} arrays are not supported"))
    } else if dimensions[2..].iter().any(|&n| n != 1) {
        Some("Only two-dimensional arrays are supported".into())
    } else if class != 1 && decoded_bytes.is_none_or(|n| n > 128 * 1024 * 1024) {
        Some("Array exceeds the 128 MiB decoded limit".into())
    } else {
        None
    };
    Ok((
        MatVariable {
            name,
            dimensions,
            class: if logical {
                "logical".into()
            } else {
                class_name.into()
            },
            complex,
            decoded_bytes,
            unavailable,
            compressed,
        },
        used,
    ))
}

fn inspect_header(
    reader: &mut impl Read,
    len: usize,
    compressed: bool,
    cancel: &AtomicBool,
) -> Result<MatVariable> {
    let (mut info, used) = header(reader, len, compressed)?;
    if info.class == "cell" && info.unavailable.is_none() {
        match containers::inspect(reader, len - used, &info, cancel) {
            Ok(bytes) => info.decoded_bytes = Some(bytes),
            Err(reason) => {
                info.unavailable = Some(format!("Unsupported or malformed cell: {reason}"))
            }
        }
        check_cancel(cancel)?;
    }
    Ok(info)
}
/// Numeric inspection validates headers and element extents, not unselected payloads
/// or their compression checksums. Numeric compressed arrays inflate only their prefix.
/// Cells traverse bounded nested metadata and skip leaf payloads to estimate storage;
/// this may inflate a complete compressed cell without allocating its leaf arrays.
pub fn inspect_workspace(path: &Path, cancel: &AtomicBool) -> Result<MatCatalog> {
    check_cancel(cancel)?;
    let path = std::path::absolute(path).map_err(|e| e.to_string())?;
    let file = File::open(&path).map_err(|e| format!("{}: {e}", path.display()))?;
    let original = stamp(&file)?;
    if original.bytes > MAX_CONTAINER {
        return Err("MAT container exceeds the 8 GiB inspection limit".into());
    }
    let mut reader = BufReader::with_capacity(64 * 1024, file);
    let mut file_header = [0; 128];
    read(&mut reader, &mut file_header)?;
    if is_v73_header(&file_header) {
        let entries = v73::inspect(&path, cancel)?;
        if stamp(reader.get_ref())? != original
            || stamp(&File::open(&path).map_err(|e| e.to_string())?)? != original
        {
            return Err("MAT file changed during inspection; inspect it again".into());
        }
        return Ok(MatCatalog {
            path,
            stamp: original,
            entries,
            v73: true,
        });
    }
    if &file_header[126..] != b"IM" || file_header[124..126] != [0, 1] {
        return Err("Expected a little-endian MATLAB v5/v7 or v7.3 MAT file".into());
    }
    let mut offset = 128;
    let mut entries = Vec::new();
    let mut names = BTreeSet::new();
    while offset < original.bytes {
        check_cancel(cancel)?;
        if entries.len() >= MAX_VARIABLES {
            return Err("MAT file exceeds 10000 variables".into());
        }
        reader
            .seek(SeekFrom::Start(offset))
            .map_err(|e| e.to_string())?;
        let outer = tag(&mut reader)?;
        let kind = u32le(&outer[..4]);
        let length = u32le(&outer[4..]);
        if kind != 14 && kind != 15 {
            return Err("Unsupported top-level MAT element".into());
        }
        let next = offset
            + 8
            + if kind == 15 {
                length as u64
            } else {
                (length as u64).next_multiple_of(8)
            };
        if next > original.bytes {
            return Err("Truncated MAT array".into());
        }
        let mut body = reader.by_ref().take(length as u64);
        let info = if kind == 15 {
            let mut decoder = flate2::read::ZlibDecoder::new(&mut body);
            let inner = tag(&mut decoder)?;
            if u32le(&inner[..4]) != 14 {
                return Err("Invalid compressed MAT array".into());
            }
            let len = u32le(&inner[4..]) as usize;
            inspect_header(&mut decoder.by_ref().take(len as u64), len, true, cancel)?
        } else {
            inspect_header(&mut body, length as usize, false, cancel)?
        };
        if !names.insert(info.name.clone()) {
            return Err(format!("Duplicate variable: {}", info.name));
        }
        entries.push(Entry {
            info,
            offset,
            length,
            kind,
        });
        offset = next;
    }
    if stamp(reader.get_ref())? != original {
        return Err("MAT file changed during inspection; inspect it again".into());
    }
    Ok(MatCatalog {
        path,
        stamp: original,
        entries,
        v73: false,
    })
}

pub fn load_selected_workspace(
    catalog: &MatCatalog,
    names: &BTreeSet<String>,
    cancel: &AtomicBool,
) -> Result<BTreeMap<String, Matrix>> {
    load_arrays(catalog, names, cancel, false)?
        .into_iter()
        .map(|(name, a)| Ok((name, a.to_numeric(cancel)?)))
        .collect()
}
pub fn load_selected_arrays(
    catalog: &MatCatalog,
    names: &BTreeSet<String>,
    cancel: &AtomicBool,
) -> Result<BTreeMap<String, Array>> {
    load_arrays(catalog, names, cancel, true)
}
fn load_arrays(
    catalog: &MatCatalog,
    names: &BTreeSet<String>,
    cancel: &AtomicBool,
    preserve_logical: bool,
) -> Result<BTreeMap<String, Array>> {
    let file = selected_file(catalog, names, cancel, preserve_logical)?;
    load_prepared(
        catalog,
        names,
        cancel,
        preserve_logical,
        file,
        #[cfg(test)]
        None,
    )
}

fn selected_file(
    catalog: &MatCatalog,
    names: &BTreeSet<String>,
    cancel: &AtomicBool,
    preserve_logical: bool,
) -> Result<File> {
    check_cancel(cancel)?;
    if names.is_empty() {
        return Err("Select at least one variable".into());
    }
    let mut count = 0;
    let mut bytes = 0u64;
    for entry in &catalog.entries {
        if names.contains(&entry.info.name) {
            if (entry.info.complex || matches!(entry.info.class.as_str(), "char" | "cell"))
                && !preserve_logical
            {
                return Err(format!(
                    "{}: complex/character/cell arrays require the typed load_selected_arrays API",
                    entry.info.name
                ));
            }
            if let Some(reason) = &entry.info.unavailable {
                return Err(format!("{}: {reason}", entry.info.name));
            }
            bytes = bytes
                .checked_add(if preserve_logical {
                    entry.info.storage_bytes().unwrap()
                } else {
                    entry.info.decoded_bytes.unwrap()
                })
                .ok_or("MAT size overflow")?;
            count += 1;
        }
    }
    if count != names.len() {
        return Err("Selected variable is not in this MAT file".into());
    }
    if bytes > 256 * 1024 * 1024 {
        return Err("Selected variables exceed the 256 MiB workspace limit".into());
    }
    let file = File::open(&catalog.path).map_err(|e| e.to_string())?;
    if stamp(&file)? != catalog.stamp {
        return Err("MAT file changed since inspection; inspect it again".into());
    }
    Ok(file)
}

fn load_prepared(
    catalog: &MatCatalog,
    names: &BTreeSet<String>,
    cancel: &AtomicBool,
    preserve_logical: bool,
    file: File,
    #[cfg(test)] probe: ReadProbe<'_>,
) -> Result<BTreeMap<String, Array>> {
    if catalog.v73 {
        let variables = v73::load(catalog, names, cancel, preserve_logical)?;
        check_cancel(cancel)?;
        if stamp(&file)? != catalog.stamp
            || stamp(&File::open(&catalog.path).map_err(|e| e.to_string())?)? != catalog.stamp
        {
            return Err("MAT file changed while loading".into());
        }
        return Ok(variables);
    }
    let mut reader = BufReader::with_capacity(64 * 1024, file);
    let mut variables = BTreeMap::new();
    for entry in &catalog.entries {
        if !names.contains(&entry.info.name) {
            continue;
        }
        let matrix = read_selected_entry(
            &mut reader,
            entry,
            cancel,
            preserve_logical,
            #[cfg(test)]
            probe,
        )?;
        variables.insert(entry.info.name.clone(), matrix);
    }
    check_cancel(cancel)?;
    if stamp(reader.get_ref())? != catalog.stamp {
        return Err("MAT file changed while loading".into());
    }
    Ok(variables)
}

fn read_selected_entry(
    reader: &mut (impl Read + Seek),
    entry: &Entry,
    cancel: &AtomicBool,
    preserve_logical: bool,
    #[cfg(test)] probe: ReadProbe<'_>,
) -> Result<Array> {
    check_cancel(cancel)?;
    reader
        .seek(SeekFrom::Start(entry.offset))
        .map_err(|e| e.to_string())?;
    let outer = tag(reader)?;
    if u32le(&outer[..4]) != entry.kind || u32le(&outer[4..]) != entry.length {
        return Err("MAT array changed since inspection".into());
    }
    let mut body = reader.by_ref().take(entry.length as u64);
    (|| -> Result<Array> {
        if entry.kind == 15 {
            let mut decoder = flate2::read::ZlibDecoder::new(&mut body);
            let inner = tag(&mut decoder)?;
            if u32le(&inner[..4]) != 14 {
                return Err("Invalid compressed MAT array".into());
            }
            let len = u32le(&inner[4..]) as usize;
            let matrix = read_matrix(
                &mut decoder.by_ref().take(len as u64),
                len,
                &entry.info,
                cancel,
                preserve_logical,
                #[cfg(test)]
                probe,
            )?;
            let mut extra = [0; 1];
            if decoder
                .read(&mut extra)
                .map_err(|e| format!("Compressed MAT: {e}"))?
                != 0
                || decoder.total_in() != entry.length as u64
            {
                return Err("Invalid compressed MAT length or trailing data".into());
            }
            Ok(matrix)
        } else {
            read_matrix(
                &mut body,
                entry.length as usize,
                &entry.info,
                cancel,
                preserve_logical,
                #[cfg(test)]
                probe,
            )
        }
    })()
    .map_err(|error| format!("{}: {error}", entry.info.name))
}

pub(super) fn is_v73_header(header: &[u8]) -> bool {
    header.len() >= 128
        && header.starts_with(b"MATLAB 7.3 MAT-file")
        && header[124..128] == [0, 2, b'I', b'M']
}

fn read_matrix(
    reader: &mut impl Read,
    len: usize,
    expected: &MatVariable,
    cancel: &AtomicBool,
    preserve_logical: bool,
    #[cfg(test)] probe: ReadProbe<'_>,
) -> Result<Array> {
    let (mut info, used) = header(reader, len, expected.compressed)?;
    // Cell storage was established by bounded catalog traversal; revalidate its
    // complete charge during loading, before accepting the new private value.
    if info.class == "cell" {
        info.decoded_bytes = expected.decoded_bytes;
    }
    if &info != expected {
        return Err("MAT metadata changed since inspection".into());
    }
    if matches!(info.class.as_str(), "char" | "cell") {
        if !preserve_logical {
            return Err("Character/cell arrays require the typed loader".into());
        }
        return containers::read_array(reader, len - used, &info, cancel);
    }
    read_payload_inner(
        reader,
        len,
        &info,
        used,
        cancel,
        preserve_logical,
        #[cfg(test)]
        probe,
    )
}
fn read_payload(
    reader: &mut impl Read,
    len: usize,
    info: &MatVariable,
    used: usize,
    cancel: &AtomicBool,
    preserve_logical: bool,
) -> Result<Array> {
    read_payload_inner(
        reader,
        len,
        info,
        used,
        cancel,
        preserve_logical,
        #[cfg(test)]
        None,
    )
}

fn read_payload_inner(
    reader: &mut impl Read,
    len: usize,
    info: &MatVariable,
    used: usize,
    cancel: &AtomicBool,
    preserve_logical: bool,
    #[cfg(test)] probe: ReadProbe<'_>,
) -> Result<Array> {
    if info.complex {
        if !preserve_logical {
            return Err("Complex arrays require the typed loader".into());
        }
        return complex::read_array(reader, len - used, info, cancel);
    }
    let count = Matrix::checked_len(info.dimensions[0], info.dimensions[1])?;
    let t = tag(reader)?;
    let word = u32le(&t[..4]);
    let short = (word >> 16) as usize;
    let kind = if short > 0 { word & 65535 } else { word };
    let width = match kind {
        1 | 2 => 1,
        3 | 4 => 2,
        5..=7 => 4,
        9 | 12 | 13 => 8,
        _ => return Err("Unsupported MAT numeric storage".into()),
    };
    let data_bytes = if short > 0 {
        short
    } else {
        u32le(&t[4..]) as usize
    };
    if short > 4
        || data_bytes != count * width
        || len
            != used
                + 8
                + if short > 0 {
                    0
                } else {
                    data_bytes.next_multiple_of(8)
                }
    {
        return Err("MAT data length does not match dimensions".into());
    }
    if preserve_logical && info.class == "logical" {
        let mut values = vec![0u8; count];
        if short > 0 {
            let mut numbers = [0.; 4];
            convert(kind, &t[4..4 + short], &mut numbers[..count])?;
            for (out, v) in values.iter_mut().zip(&numbers) {
                if *v != 0. && *v != 1. {
                    return Err("Invalid logical MAT values".into());
                }
                *out = *v as u8;
            }
        } else if kind == 2 {
            for chunk in values.chunks_mut(8192) {
                check_cancel(cancel)?;
                read(reader, chunk)?;
                if chunk.iter().any(|v| *v > 1) {
                    return Err("Invalid logical MAT values".into());
                }
            }
        } else {
            let mut input = [0u8; 65536];
            let mut numbers = [0.; 8192];
            for chunk in values.chunks_mut(8192) {
                check_cancel(cancel)?;
                read(reader, &mut input[..chunk.len() * width])?;
                convert(
                    kind,
                    &input[..chunk.len() * width],
                    &mut numbers[..chunk.len()],
                )?;
                for (out, v) in chunk.iter_mut().zip(&numbers) {
                    if *v != 0. && *v != 1. {
                        return Err("Invalid logical MAT values".into());
                    }
                    *out = *v as u8;
                }
            }
        }
        if short == 0 {
            let mut pad = [0; 7];
            read(reader, &mut pad[..(8 - data_bytes % 8) % 8])?;
        }
        return Ok(LogicalMatrix::from_canonical_vec(
            info.dimensions[0],
            info.dimensions[1],
            values,
        )?
        .into());
    }
    if preserve_logical && let Some(class) = crate::NumericClass::parse(&info.class) {
        // Integer and single classes keep their class; data stored in the
        // class's own type is copied exactly (64-bit values never pass f64).
        let rows = info.dimensions[0];
        let cols = info.dimensions[1];
        let mut raw = vec![0u8; data_bytes];
        if short > 0 {
            raw.copy_from_slice(&t[4..4 + short]);
        } else {
            for chunk in raw.chunks_mut(65536) {
                check_cancel(cancel)?;
                read(reader, chunk)?;
            }
            let mut pad = [0; 7];
            read(reader, &mut pad[..(8 - data_bytes % 8) % 8])?;
        }
        if kind == super::mat_typed::mi_type(class) {
            return Ok(crate::TypedArray::from_le_bytes(class, rows, cols, &raw)?.into());
        }
        let mut values = vec![0.; count];
        convert(kind, &raw, &mut values)?;
        return Ok(crate::TypedArray::from_reals(class, rows, cols, values.into_iter())?.into());
    }
    let mut values = vec![0.; count];
    if short > 0 {
        convert(kind, &t[4..4 + short], &mut values)?;
    } else if kind == 9 {
        for chunk in values.chunks_mut(8192) {
            check_cancel(cancel)?;
            read(reader, bytemuck::cast_slice_mut(chunk))?;
            #[cfg(target_endian = "big")]
            for v in chunk.iter_mut() {
                *v = f64::from_bits(v.to_bits().swap_bytes());
            }
            #[cfg(test)]
            if let Some(probe) = probe {
                probe(&info.name, ReadPhase::Decoded, chunk);
            }
        }
    } else {
        let mut buffer = [0u8; 64 * 1024];
        for chunk in values.chunks_mut(buffer.len() / width) {
            check_cancel(cancel)?;
            let input = &mut buffer[..chunk.len() * width];
            read(reader, input)?;
            convert(kind, input, chunk)?;
        }
    }
    if short == 0 {
        let mut pad = [0; 7];
        read(reader, &mut pad[..(8 - data_bytes % 8) % 8])?;
    }
    if info.class == "logical" && values.iter().any(|&v| v != 0. && v != 1.) {
        return Err("Invalid logical MAT values".into());
    }
    Ok(Matrix::from_column_vec(info.dimensions[0], info.dimensions[1], values)?.into())
}
fn convert(kind: u32, input: &[u8], output: &mut [f64]) -> Result<()> {
    let width = match kind {
        1 | 2 => 1,
        3 | 4 => 2,
        5..=7 => 4,
        _ => 8,
    };
    for (value, b) in output.iter_mut().zip(input.chunks_exact(width)) {
        *value = match kind {
            1 => b[0] as i8 as f64,
            2 => b[0] as f64,
            3 => i16::from_le_bytes(b.try_into().unwrap()) as f64,
            4 => u16::from_le_bytes(b.try_into().unwrap()) as f64,
            5 => i32::from_le_bytes(b.try_into().unwrap()) as f64,
            6 => u32::from_le_bytes(b.try_into().unwrap()) as f64,
            7 => f32::from_le_bytes(b.try_into().unwrap()) as f64,
            9 => f64::from_le_bytes(b.try_into().unwrap()),
            12 => {
                let n = i64::from_le_bytes(b.try_into().unwrap());
                if n.unsigned_abs() > 9_007_199_254_740_992 {
                    return Err("Integer values exceed exact f64 precision".into());
                }
                n as f64
            }
            13 => {
                let n = u64::from_le_bytes(b.try_into().unwrap());
                if n > 9_007_199_254_740_992 {
                    return Err("Integer values exceed exact f64 precision".into());
                }
                n as f64
            }
            _ => unreachable!(),
        };
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn selected_numeric_read_observes_cancellation_between_bounded_chunks() {
        struct CancelReader<'a> {
            input: std::io::Cursor<Vec<u8>>,
            cancel: &'a AtomicBool,
        }
        impl Read for CancelReader<'_> {
            fn read(&mut self, out: &mut [u8]) -> std::io::Result<usize> {
                let n = self.input.read(out)?;
                if self.input.position() > 1024 * 1024 {
                    self.cancel
                        .store(true, std::sync::atomic::Ordering::Relaxed);
                }
                Ok(n)
            }
        }
        let mut bytes = Vec::new();
        for (kind, body) in [
            (6, vec![6u8, 0, 0, 0, 0, 0, 0, 0]),
            (5, [262144i32.to_le_bytes(), 1i32.to_le_bytes()].concat()),
            (1, b"data".to_vec()),
        ] {
            bytes.extend((kind as u32).to_le_bytes());
            bytes.extend((body.len() as u32).to_le_bytes());
            bytes.extend(&body);
            bytes.resize(bytes.len() + (8 - body.len() % 8) % 8, 0);
        }
        bytes.extend(9u32.to_le_bytes());
        bytes.extend((262144u32 * 8).to_le_bytes());
        bytes.resize(bytes.len() + 262144 * 8, 0);
        let len = bytes.len();
        let cancel = AtomicBool::new(false);
        let info = header(&mut std::io::Cursor::new(&bytes), len, false)
            .unwrap()
            .0;
        let mut input = CancelReader {
            input: std::io::Cursor::new(bytes),
            cancel: &cancel,
        };
        assert!(
            read_matrix(&mut input, len, &info, &cancel, false, None)
                .unwrap_err()
                .contains("cancelled")
        );
        assert!(input.input.position() < len as u64);
    }
}

#[cfg(test)]
#[path = "mat_pair_tests.rs"]
mod pair_tests;
