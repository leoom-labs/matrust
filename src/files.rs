//! Bounded, atomic file exchange. Parsing finishes before callers replace state.
use crate::{Matrix, Result, plot::Figure};
use serde::{Deserialize, Serialize};
mod csv;
mod text_matrix;
pub use text_matrix::{read_matrix_text, write_matrix_text};
pub mod array_data;
/// h5read/h5write/h5create and NetCDF-4 ncread (src/engine/readers/hdf5.rs).
#[cfg(feature = "mat-v73")]
pub mod hdf5_data;
mod mat_typed;
#[cfg(feature = "mat-v73")]
mod mat_v73_complex;
#[cfg(feature = "mat-v73")]
mod mat_v73_write;
pub mod netcdf3;
pub mod read_fallback;
mod typed_workspace;
#[cfg(feature = "mat-v73")]
pub use mat_v73_write::save_array_workspace_v73;
pub use typed_workspace::{
    save_array_workspace, save_array_workspace_uncompressed, write_array_csv,
};
mod fig_mat;
pub mod figure_version;
mod mat_catalog;
mod mat_fast;
pub use mat_catalog::{
    MatCatalog, MatVariable, inspect_workspace, load_selected_arrays, load_selected_workspace,
};
pub(crate) mod bundle_relocation;

/// MAT-file v7.3 is HDF5, a C library; the browser build (web/) leaves it out.
#[cfg(not(feature = "mat-v73"))]
pub(crate) const NO_MAT_V73: &str =
    "MAT-file version 7.3 needs the mat-v73 feature (not available in the browser)";

#[cfg(not(feature = "mat-v73"))]
pub fn save_array_workspace_v73(
    _path: &Path,
    _variables: &BTreeMap<String, crate::Array>,
    _cancel: &std::sync::atomic::AtomicBool,
) -> Result<()> {
    Err(NO_MAT_V73.into())
}
mod plot_bundle;
mod plot_script;
pub use plot_bundle::{
    MatlabPlotBundle, PreparedMatlabPlotScript, prepare_matlab_plot_reconstruction,
    prepare_matlab_plot_reconstruction_with_theme, write_matlab_plot_bundle_with_cancel,
    write_matlab_plot_bundle_with_theme, write_matlab_plot_reconstruction_with_cancel,
    write_matlab_plot_reconstruction_with_theme,
};
pub use plot_script::{
    matlab_plot_script, matlab_plot_script_with_theme, validate_matlab_plot_script_path,
    write_matlab_plot_script_with_cancel, write_matlab_plot_script_with_theme,
};
mod indexed_patch;
mod matlab_fig;
pub use matlab_fig::{
    load_matlab_figure, matlab_figure_export_restriction,
    matlab_figure_export_restriction_with_theme, save_matlab_figure,
    save_matlab_figure_with_cancel, save_matlab_figure_with_theme,
};
use std::{
    borrow::Cow,
    collections::BTreeMap,
    io::{Read, Write},
    path::Path,
    sync::Arc,
};

pub const MAX_FILE_BYTES: usize = 128 * 1024 * 1024;
// Text and MAT headers require more bytes than the decoded numeric workspace.
pub const MAX_CSV_BYTES: usize = 512 * 1024 * 1024;
pub const MAX_MAT_BYTES: usize = 272 * 1024 * 1024;

/// Strip the UTF-8 signature from the start of a text file before decoding.
pub(crate) fn strip_utf8_bom(bytes: &[u8]) -> &[u8] {
    bytes.strip_prefix(b"\xef\xbb\xbf").unwrap_or(bytes)
}

/// Product of MAT dimension extents; overflow is a malformed or hostile file, never a wrap.
pub(crate) fn checked_dimension_product(dims: &[usize]) -> Result<usize> {
    dims.iter()
        .try_fold(1usize, |n, &d| n.checked_mul(d))
        .ok_or_else(|| "MAT array dimensions overflow the addressable element count".into())
}
pub fn read_bounded(path: &Path, limit: usize) -> Result<Vec<u8>> {
    let limit_error = || {
        if limit == 1_000_000 {
            "File exceeds the 1 MB (1,000,000 bytes) limit".to_owned()
        } else if limit >= 1024 * 1024 && limit.is_multiple_of(1024 * 1024) {
            format!("File exceeds the {} MiB limit", limit / 1024 / 1024)
        } else {
            format!("File exceeds the {limit} byte limit")
        }
    };
    let file = std::fs::File::open(path).map_err(|e| format!("{}: {e}", path.display()))?;
    let length = file.metadata().map_err(|e| e.to_string())?.len();
    if length > limit as u64 {
        return Err(limit_error());
    }
    let mut bytes = Vec::with_capacity(length as usize);
    file.take(limit as u64 + 1)
        .read_to_end(&mut bytes)
        .map_err(|e| e.to_string())?;
    if bytes.len() > limit {
        return Err(limit_error());
    }
    Ok(bytes)
}
pub fn atomic_write(path: &Path, bytes: &[u8]) -> Result<()> {
    let parent = path
        .parent()
        .filter(|p| !p.as_os_str().is_empty())
        .unwrap_or(Path::new("."));
    let mut file = tempfile::NamedTempFile::new_in(parent).map_err(|e| e.to_string())?;
    file.write_all(bytes)
        .and_then(|()| file.as_file().sync_all())
        .map_err(|e| e.to_string())?;
    persist_atomic(file, path, |_| {})
}
fn persist_atomic(
    file: tempfile::NamedTempFile,
    path: &Path,
    mut before_retry: impl FnMut(usize),
) -> Result<()> {
    persist_atomic_checked(file, path, &mut before_retry, || Ok(()))
}
fn persist_atomic_checked(
    mut file: tempfile::NamedTempFile,
    path: &Path,
    mut before_retry: impl FnMut(usize),
    mut checkpoint: impl FnMut() -> Result<()>,
) -> Result<()> {
    // On Windows a reader/scanner can briefly prevent replacement. Retain the
    // same fully synced temporary file and retry only the atomic publication.
    // Never delete the destination or fall back to an in-place overwrite.
    // A fresh executable or compressed file can attract readers for longer
    // than 50 ms. Bound the total retry sleep to 470 ms; successful writes
    // have no added delay and permanent locks still fail without data loss.
    let delays = [5, 15, 30, 60, 120, 240];
    for attempt in 0..=delays.len() {
        checkpoint()?;
        match file.persist(path) {
            Ok(_) => return Ok(()),
            Err(error) => {
                let retryable =
                    cfg!(windows) && matches!(error.error.raw_os_error(), Some(5 | 32 | 33));
                if !retryable || attempt == delays.len() {
                    return Err(format!(
                        "Could not save {}: {}",
                        path.display(),
                        error.error
                    ));
                }
                file = error.file;
                before_retry(attempt);
                std::thread::sleep(std::time::Duration::from_millis(delays[attempt]));
            }
        }
    }
    unreachable!()
}
/// Internal cancellable file publication. OS write/sync/rename calls are not interruptible.
/// Cancellation observed before a persist attempt preserves the destination; a
/// cancellation racing a successful atomic rename cannot undo that completed file.
pub(crate) fn atomic_write_with_cancel(
    path: &Path,
    bytes: &[u8],
    cancel: &std::sync::atomic::AtomicBool,
) -> Result<()> {
    atomic_write_checked(path, bytes, || crate::dsp::check_cancel(cancel))
}
fn atomic_write_checked(
    path: &Path,
    bytes: &[u8],
    mut checkpoint: impl FnMut() -> Result<()>,
) -> Result<()> {
    checkpoint()?;
    let parent = path
        .parent()
        .filter(|p| !p.as_os_str().is_empty())
        .unwrap_or(Path::new("."));
    let mut file = tempfile::NamedTempFile::new_in(parent).map_err(|e| e.to_string())?;
    for chunk in bytes.chunks(65536) {
        checkpoint()?;
        file.write_all(chunk).map_err(|e| e.to_string())?;
    }
    checkpoint()?;
    file.as_file().sync_all().map_err(|e| e.to_string())?;
    persist_atomic_checked(file, path, |_| {}, checkpoint)
}
pub fn read_script(path: &Path) -> Result<String> {
    let mut text =
        decode_script_source(path, read_bounded(path, 1_000_000)?).map_err(
            |error| match error {
                ScriptDecodeError::Nul => format!("Script {error}"),
                ScriptDecodeError::Utf16Bom | ScriptDecodeError::InvalidUtf8(_) => {
                    "Script is not valid UTF-8".to_owned()
                }
            },
        )?;
    if text.len() > 1_000_000 {
        return Err("Script exceeds the 1 MB decoded text limit".into());
    }
    let prefix = text.len() - text.trim_start_matches('\u{feff}').len();
    if prefix != 0 {
        text.drain(..prefix);
    }
    Ok(text)
}

/// Why file bytes are not script source text.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ScriptDecodeError {
    /// A NUL byte anywhere: UTF-16 text or binary data, never source text.
    Nul,
    /// A leading UTF-16 byte-order mark (`FF FE` or `FE FF`).
    Utf16Bom,
    /// Invalid UTF-8 outside the Windows-1252 fallback for legacy .m files.
    InvalidUtf8(std::str::Utf8Error),
}

impl std::fmt::Display for ScriptDecodeError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Nul => f.write_str("contains NUL bytes (UTF-16 or binary file?)"),
            Self::Utf16Bom => f.write_str("starts with a UTF-16 byte-order mark, not UTF-8 text"),
            Self::InvalidUtf8(error) => error.fmt(f),
        }
    }
}

impl std::error::Error for ScriptDecodeError {}

/// Reuse valid UTF-8 (a UTF-8 BOM stays for the caller to strip) and fall back
/// to Windows-1252 for legacy .m files.
pub fn decode_script_source(
    path: &Path,
    bytes: Vec<u8>,
) -> std::result::Result<String, ScriptDecodeError> {
    // NUL is valid UTF-8, so it is refused before decoding: BOM-less UTF-16 or
    // binary data would otherwise load as an empty or wrong program.
    if bytes.contains(&0) {
        return Err(ScriptDecodeError::Nul);
    }
    // Reject UTF-16 BOMs rather than decoding them as Windows-1252.
    if bytes.starts_with(&[0xff, 0xfe]) || bytes.starts_with(&[0xfe, 0xff]) {
        return Err(ScriptDecodeError::Utf16Bom);
    }
    let error = match String::from_utf8(bytes) {
        Ok(text) => return Ok(text),
        Err(error) => error,
    };
    if !path
        .extension()
        .is_some_and(|ext| ext.eq_ignore_ascii_case("m"))
    {
        return Err(ScriptDecodeError::InvalidUtf8(error.utf8_error()));
    }
    // Windows code page 1252 maps five unused positions to their C1 controls.
    const C1: [char; 32] = [
        '€', '\u{81}', '‚', 'ƒ', '„', '…', '†', '‡', 'ˆ', '‰', 'Š', '‹', 'Œ', '\u{8d}', 'Ž',
        '\u{8f}', '\u{90}', '‘', '’', '“', '”', '•', '–', '—', '˜', '™', 'š', '›', 'œ', '\u{9d}',
        'ž', 'Ÿ',
    ];
    Ok(error
        .as_bytes()
        .iter()
        .map(|&byte| match byte {
            0x80..=0x9f => C1[usize::from(byte - 0x80)],
            _ => char::from(byte),
        })
        .collect())
}

#[cfg(all(test, windows))]
mod atomic_save_tests {
    use super::*;
    use std::os::windows::fs::OpenOptionsExt;

    #[test]
    fn atomic_publication_retries_sharing_denials_and_preserves_locked_targets() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("saved.mat");
        std::fs::write(&path, b"original").unwrap();
        // Allow readers, but deliberately deny deletion/atomic replacement.
        let lock = || {
            std::fs::OpenOptions::new()
                .read(true)
                .share_mode(1)
                .open(&path)
                .unwrap()
        };
        let mut held = Some(lock());
        let mut staged = tempfile::NamedTempFile::new_in(dir.path()).unwrap();
        staged.write_all(b"replacement").unwrap();
        staged.as_file().sync_all().unwrap();
        let mut attempts = 0;
        persist_atomic(staged, &path, |attempt| {
            assert_eq!(attempt, attempts);
            assert_eq!(std::fs::read(&path).unwrap(), b"original");
            attempts += 1;
            // Hold past the former 50 ms retry budget, without a timer race.
            if attempt == 3 {
                drop(held.take());
            }
        })
        .unwrap();
        assert_eq!(attempts, 4);
        assert_eq!(std::fs::read(&path).unwrap(), b"replacement");

        let held = lock();
        assert!(atomic_write(&path, b"must not replace").is_err());
        assert_eq!(std::fs::read(&path).unwrap(), b"replacement");
        drop(held);
        assert_eq!(std::fs::read_dir(dir.path()).unwrap().count(), 1);
        atomic_write(&path, b"final").unwrap();
        assert_eq!(std::fs::read(&path).unwrap(), b"final");

        let staged = tempfile::NamedTempFile::new_in(dir.path()).unwrap();
        assert!(
            persist_atomic(staged, &dir.path().join("missing/file"), |_| {
                panic!("A missing destination directory must not trigger sharing retries")
            })
            .is_err()
        );
    }
}
pub fn valid_name(name: &str) -> bool {
    !name.is_empty()
        && name.len() <= 63
        && name.as_bytes()[0].is_ascii_alphabetic()
        && name.bytes().all(|c| c.is_ascii_alphanumeric() || c == b'_')
}
pub fn read_csv(path: &Path) -> Result<Matrix> {
    csv::read(path)
}
/// Cancellable permissive numeric CSV import; delimiter behavior matches read_csv.
pub fn read_csv_with_cancel(path: &Path, cancel: &std::sync::atomic::AtomicBool) -> Result<Matrix> {
    csv::read_with_cancel(path, cancel)
}
pub fn write_csv(path: &Path, matrix: &Matrix) -> Result<()> {
    let mut bytes = Vec::new();
    for r in 0..matrix.rows() {
        for c in 0..matrix.cols() {
            if c > 0 {
                bytes.push(b',');
            }
            write!(bytes, "{}", matrix.get(r, c)).map_err(|e| e.to_string())?;
        }
        bytes.push(b'\n');
    }
    atomic_write(path, &bytes)
}
pub fn save_workspace(path: &Path, variables: &BTreeMap<String, Matrix>) -> Result<()> {
    let mut arrays = Vec::with_capacity(variables.len());
    let mut total = 0;
    for (name, m) in variables {
        if !valid_name(name) {
            return Err(format!("Invalid variable name: {name}"));
        }
        total += m.len();
        if total > 32 * 1024 * 1024 {
            return Err("Workspace exceeds 256 MiB".into());
        }
        arrays.push(mat5::NamedArray {
            name: name.clone(),
            matrix: mat5::Matrix::Dense(mat5::DenseMatrix {
                rows: m.rows(),
                cols: m.cols(),
                values: mat5::Values::F64(m.data().to_vec()),
            }),
        });
    }
    let file = mat5::MatFile {
        header_text: "MATLAB 5.0 MAT-file, MatRust numerical workspace".into(),
        arrays,
    };
    let bytes = file.write_bytes().map_err(|e| e.to_string())?;
    atomic_write(path, &bytes)
}
pub fn load_workspace(path: &Path) -> Result<BTreeMap<String, Matrix>> {
    if let Some(variables) = mat_fast::load_doubles(path)? {
        return Ok(variables);
    }
    let bytes = read_bounded(path, MAX_MAT_BYTES)?;
    let normalized = normalize_mat(&bytes)?;
    let file = mat5::MatFile::read_bytes(&normalized).map_err(|e| format!("MAT file: {e}"))?;
    let mut variables = BTreeMap::new();
    for array in file.arrays {
        if !valid_name(&array.name) {
            return Err(format!("Invalid variable name: {}", array.name));
        }
        let mat5::Matrix::Dense(m) = array.matrix else {
            return Err("Sparse MAT variables are not supported yet".into());
        };
        if let mat5::Values::I64(v) = &m.values
            && v.iter().any(|x| x.unsigned_abs() > 9_007_199_254_740_992)
        {
            return Err("Integer values exceed exact f64 precision".into());
        }
        if let mat5::Values::U64(v) = &m.values
            && v.iter().any(|&x| x > 9_007_199_254_740_992)
        {
            return Err("Integer values exceed exact f64 precision".into());
        }
        let values = match m.values {
            mat5::Values::F64(values) => values,
            other => other.to_f64(),
        };
        let value = Matrix::from_column_vec(m.rows, m.cols, values)?;
        if variables.insert(array.name.clone(), value).is_some() {
            return Err(format!("Duplicate variable: {}", array.name));
        }
    }
    Ok(variables)
}
// Bound decompression and validate compact tags/dimensions before using mat5.
// This avoids its unbounded compressed reader and malformed-small-tag subtraction.
fn element<'a>(data: &'a [u8], pos: &mut usize) -> Result<(u32, &'a [u8])> {
    if data.len().saturating_sub(*pos) < 8 {
        return Err("Truncated MAT element".into());
    }
    let first = u32::from_le_bytes(data[*pos..*pos + 4].try_into().unwrap());
    let small = first >> 16;
    if small > 0 {
        if small > 4 {
            return Err("Invalid compact MAT element".into());
        }
        let body = &data[*pos + 4..*pos + 4 + small as usize];
        *pos += 8;
        return Ok((first & 65535, body));
    }
    let len = u32::from_le_bytes(data[*pos + 4..*pos + 8].try_into().unwrap()) as usize;
    let start = *pos + 8;
    let end = start.checked_add(len).ok_or("MAT length overflow")?;
    if end > data.len() {
        return Err("Truncated MAT payload".into());
    }
    let body = &data[start..end];
    *pos = if first == 15 {
        end
    } else {
        end.saturating_add((8 - len % 8) % 8).min(data.len())
    };
    Ok((first, body))
}
fn normalize_mat(data: &[u8]) -> Result<Cow<'_, [u8]>> {
    if data.len() < 128 || &data[126..128] != b"IM" {
        return Err("Expected a little-endian MATLAB v5/v7 MAT payload".into());
    }
    let mut out: Option<Vec<u8>> = None;
    let mut pos = 128;
    let mut total = 0usize;
    while pos < data.len() {
        let element_start = pos;
        let (kind, payload) = element(data, &mut pos)?;
        let mut expanded = Vec::new();
        let matrix = if kind == 15 {
            flate2::read::ZlibDecoder::new(payload)
                .take(MAX_MAT_BYTES as u64 + 1)
                .read_to_end(&mut expanded)
                .map_err(|e| e.to_string())?;
            if expanded.len() > MAX_MAT_BYTES {
                return Err("Compressed MAT array exceeds memory budget".into());
            }
            let mut p = 0;
            let (inner, m) = element(&expanded, &mut p)?;
            if inner != 14 || p != expanded.len() {
                return Err("Invalid compressed MAT matrix".into());
            }
            out.get_or_insert_with(|| data[..element_start].to_vec());
            m
        } else if kind == 14 {
            payload
        } else {
            return Err("Unsupported MAT element".into());
        };
        let mut p = 0;
        let (_, flags) = element(matrix, &mut p)?;
        if flags.len() != 8 {
            return Err("Invalid MAT flags".into());
        }
        let flags = u32::from_le_bytes(flags[..4].try_into().unwrap());
        if flags & 0x800 != 0 || !(6..=15).contains(&(flags & 255)) {
            return Err("Only real dense numeric MAT arrays are currently supported".into());
        }
        let (kind, dims) = element(matrix, &mut p)?;
        if kind != 5 || dims.len() != 8 {
            return Err("Only two-dimensional MAT arrays are currently supported".into());
        }
        let rows = i32::from_le_bytes(dims[..4].try_into().unwrap());
        let cols = i32::from_le_bytes(dims[4..].try_into().unwrap());
        if rows < 0 || cols < 0 {
            return Err("Negative MAT dimension".into());
        }
        total = total
            .checked_add(Matrix::checked_len(rows as usize, cols as usize)?)
            .ok_or("MAT size overflow")?;
        if total > 32 * 1024 * 1024 {
            return Err("MAT workspace exceeds 256 MiB".into());
        }
        while p < matrix.len() {
            element(matrix, &mut p)?;
        }
        if let Some(out) = &mut out {
            if out.len() + matrix.len() + 8 > MAX_MAT_BYTES {
                return Err("Expanded MAT file exceeds 272 MiB".into());
            }
            out.extend_from_slice(&14u32.to_le_bytes());
            out.extend_from_slice(&(matrix.len() as u32).to_le_bytes());
            out.extend_from_slice(matrix);
            out.resize(out.len() + (8 - matrix.len() % 8) % 8, 0);
        }
    }
    Ok(out.map_or(Cow::Borrowed(data), Cow::Owned))
}

#[derive(Clone, Copy, Default, Serialize, Deserialize)]
enum VolumeDataClass {
    #[default]
    Double,
    Logical,
    Int8,
    UInt8,
    Int16,
    UInt16,
    Int32,
    UInt32,
    Int64,
    UInt64,
    Single,
}

impl VolumeDataClass {
    fn from_image_class(c: crate::plot::ImageDataClass) -> Self {
        use crate::NumericClass::*;
        match c {
            crate::plot::ImageDataClass::Double => Self::Double,
            crate::plot::ImageDataClass::Logical => Self::Logical,
            crate::plot::ImageDataClass::Typed(t) => match t {
                Int8 => Self::Int8,
                Uint8 => Self::UInt8,
                Int16 => Self::Int16,
                Uint16 => Self::UInt16,
                Int32 => Self::Int32,
                Uint32 => Self::UInt32,
                Int64 => Self::Int64,
                Uint64 => Self::UInt64,
                Single => Self::Single,
            },
        }
    }
    fn to_image_class(self) -> crate::plot::ImageDataClass {
        use crate::NumericClass::*;
        match self {
            Self::Double => crate::plot::ImageDataClass::Double,
            Self::Logical => crate::plot::ImageDataClass::Logical,
            Self::Int8 => crate::plot::ImageDataClass::Typed(Int8),
            Self::UInt8 => crate::plot::ImageDataClass::Typed(Uint8),
            Self::Int16 => crate::plot::ImageDataClass::Typed(Int16),
            Self::UInt16 => crate::plot::ImageDataClass::Typed(Uint16),
            Self::Int32 => crate::plot::ImageDataClass::Typed(Int32),
            Self::UInt32 => crate::plot::ImageDataClass::Typed(Uint32),
            Self::Int64 => crate::plot::ImageDataClass::Typed(Int64),
            Self::UInt64 => crate::plot::ImageDataClass::Typed(Uint64),
            Self::Single => crate::plot::ImageDataClass::Typed(Single),
        }
    }
}

fn default_isosurface_color() -> [f64; 3] {
    crate::plot::DEFAULT_ISOSURFACE_COLOR
}

fn is_default_isosurface_color(c: &[f64; 3]) -> bool {
    *c == crate::plot::DEFAULT_ISOSURFACE_COLOR
}

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct VolumeMetadata {
    dims: [usize; 3],
    #[serde(default)]
    data_class: VolumeDataClass,
    rendering_style: crate::plot::RenderingStyle,
    colormap: Vec<[f64; 3]>,
    alphamap: Vec<f64>,
    data_limits: [f64; 2],
    #[serde(default)]
    data_limits_manual: bool,
    isosurface_value: f64,
    #[serde(
        default = "default_isosurface_color",
        skip_serializing_if = "is_default_isosurface_color"
    )]
    isosurface_color: [f64; 3],
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    slice_plane_values: Vec<[f64; 4]>,
    transformation: [f64; 3],
    visible: bool,
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    exact_data: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    overlay: Option<VolumeOverlayMetadata>,
}

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct VolumeOverlayMetadata {
    class: Option<NativeOverlayClass>,
    rendering_style: crate::plot::OverlayRenderingStyle,
    colormap_rows: usize,
    alphamap_len: usize,
    display_range: [f64; 2],
    display_range_manual: bool,
}

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
enum NativeOverlayClass {
    Numeric(VolumeDataClass),
    Categorical {
        categories: Vec<String>,
        ordinal: bool,
    },
}

impl NativeOverlayClass {
    fn encode(class: &crate::plot::OverlayDataClass) -> Result<Self> {
        use crate::plot::{ImageDataClass, OverlayDataClass};
        Ok(match class {
            OverlayDataClass::Double => Self::Numeric(VolumeDataClass::Double),
            OverlayDataClass::Logical => Self::Numeric(VolumeDataClass::Logical),
            OverlayDataClass::Typed(class) => Self::Numeric(VolumeDataClass::from_image_class(
                ImageDataClass::Typed(*class),
            )),
            OverlayDataClass::Categorical {
                categories,
                ordinal,
            } => {
                if categories
                    .iter()
                    .map(|name| name.len().saturating_add(3))
                    .sum::<usize>()
                    > NATIVE_EXTENDED_METADATA_LIMIT
                {
                    return Err("Figure exceeds save metadata limit".into());
                }
                Self::Categorical {
                    categories: categories.as_ref().clone(),
                    ordinal: *ordinal,
                }
            }
        })
    }

    fn decode(self) -> crate::plot::OverlayDataClass {
        use crate::plot::{ImageDataClass, OverlayDataClass};
        match self {
            Self::Numeric(class) => match class.to_image_class() {
                ImageDataClass::Double => OverlayDataClass::Double,
                ImageDataClass::Logical => OverlayDataClass::Logical,
                ImageDataClass::Typed(class) => OverlayDataClass::Typed(class),
            },
            Self::Categorical {
                categories,
                ordinal,
            } => OverlayDataClass::Categorical {
                categories: Arc::new(categories),
                ordinal,
            },
        }
    }
}

fn validate_native_overlay(class: &crate::plot::OverlayDataClass, samples: &[f64]) -> Result<()> {
    use crate::plot::OverlayDataClass;
    match class {
        OverlayDataClass::Logical if samples.iter().any(|&v| v != 0.0 && v != 1.0) => {
            return Err("Invalid logical OverlayData sample".into());
        }
        OverlayDataClass::Categorical { categories, .. } => {
            let mut names = std::collections::HashSet::new();
            if categories
                .iter()
                .any(|name| name.is_empty() || name == "<undefined>" || !names.insert(name))
                || samples
                    .iter()
                    .any(|&v| !(0.0..=categories.len() as f64).contains(&v) || v.fract() != 0.0)
            {
                return Err("Invalid categorical OverlayData names or codes".into());
            }
        }
        _ => {}
    }
    Ok(())
}

impl VolumeMetadata {
    fn encode(v: &crate::plot::VolumeObject) -> Result<Self> {
        let class = v
            .overlay_data
            .as_ref()
            .map(|data| {
                validate_native_overlay(&data.class, &data.data)?;
                NativeOverlayClass::encode(&data.class)
            })
            .transpose()?;
        Ok(Self {
            dims: v.dims,
            data_class: VolumeDataClass::from_image_class(v.data_class),
            rendering_style: v.rendering_style,
            colormap: v.colormap.as_ref().clone(),
            alphamap: v.alphamap.as_ref().clone(),
            data_limits: v.data_limits,
            data_limits_manual: v.data_limits_manual,
            isosurface_value: v.isosurface_value,
            isosurface_color: v.isosurface_color,
            slice_plane_values: v.slice_plane_values.clone(),
            transformation: v.transformation,
            visible: v.visible,
            exact_data: v.exact_data.is_some(),
            overlay: Some(VolumeOverlayMetadata {
                class,
                rendering_style: v.overlay_rendering_style,
                colormap_rows: v.overlay_colormap.len(),
                alphamap_len: v.overlay_alphamap.len(),
                display_range: v.overlay_display_range,
                display_range_manual: v.overlay_display_range_manual,
            }),
        })
    }

    fn exact_class(&self) -> Result<Option<crate::NumericClass>> {
        if !self.exact_data {
            return Ok(None);
        }
        match self.data_class {
            VolumeDataClass::Int64 => Ok(Some(crate::NumericClass::Int64)),
            VolumeDataClass::UInt64 => Ok(Some(crate::NumericClass::Uint64)),
            _ => Err("Invalid exact volume data class".into()),
        }
    }
}

mod limit_serde {
    use serde::{Deserialize, Deserializer, Serialize, Serializer};
    #[derive(Serialize, Deserialize)]
    #[serde(untagged)]
    enum End {
        Number(f64),
        Text(String),
    }
    pub fn serialize<S: Serializer>(
        value: &Option<[f64; 2]>,
        serializer: S,
    ) -> Result<S::Ok, S::Error> {
        value
            .map(|v| {
                v.map(|n| {
                    if n == f64::INFINITY {
                        End::Text("inf".into())
                    } else if n == f64::NEG_INFINITY {
                        End::Text("-inf".into())
                    } else {
                        End::Number(n)
                    }
                })
            })
            .serialize(serializer)
    }
    pub fn deserialize<'de, D: Deserializer<'de>>(
        deserializer: D,
    ) -> Result<Option<[f64; 2]>, D::Error> {
        let value = Option::<[End; 2]>::deserialize(deserializer)?;
        value
            .map(|v| {
                let mut out = [0.; 2];
                for (i, end) in v.into_iter().enumerate() {
                    out[i] = match end {
                        End::Number(n) => n,
                        End::Text(s) if s == "inf" => f64::INFINITY,
                        End::Text(s) if s == "-inf" => f64::NEG_INFINITY,
                        _ => return Err(serde::de::Error::custom("invalid axis limit")),
                    };
                }
                Ok(out)
            })
            .transpose()
    }
}

fn default_figure_position() -> [f64; 4] {
    crate::plot::DEFAULT_FIGURE_POSITION
}

fn is_default_figure_position(p: &[f64; 4]) -> bool {
    *p == crate::plot::DEFAULT_FIGURE_POSITION
}

fn default_true() -> bool {
    true
}

fn is_true(v: &bool) -> bool {
    *v
}

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct FigureMetadata {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    palette_cycle: Option<crate::plot::PaletteCycle>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    axes_colors: Option<crate::plot::AxesColors>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    appearance: Option<crate::plot::FigureAppearance>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    invert_hardcopy: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    latent_baselines: Option<Vec<u8>>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    baselines: Vec<baseline_metadata::NativeBaseline>,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    name: String,
    #[serde(default = "white_color", skip_serializing_if = "is_white_color")]
    color: [u8; 3],
    #[serde(
        default = "default_figure_position",
        skip_serializing_if = "is_default_figure_position"
    )]
    position: [f64; 4],
    #[serde(default, skip_serializing_if = "crate::plot::FigureUnits::is_default")]
    units: crate::plot::FigureUnits,
    #[serde(default = "default_true", skip_serializing_if = "is_true")]
    number_title: bool,
    #[serde(default = "default_true", skip_serializing_if = "is_true")]
    visible: bool,
    #[serde(default = "default_true", skip_serializing_if = "is_true")]
    resize: bool,
    #[serde(default, skip_serializing_if = "crate::plot::WindowState::is_default")]
    window_state: crate::plot::WindowState,

    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    textboxes: Vec<crate::plot::TextBox>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    annotations: Vec<crate::plot::Annotation>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    label_interpreters: Option<[crate::plot::text_markup::Interpreter; 4]>,
    #[serde(default, skip_serializing_if = "crate::plot::Subtitle::is_default")]
    subtitle: crate::plot::Subtitle,
    #[serde(default, skip_serializing_if = "crate::plot::LabelStyle::is_default")]
    title_style: crate::plot::LabelStyle,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    label_styles: Option<[crate::plot::LabelStyle; 3]>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    label_lines: Option<[Option<crate::plot::text_markup::TextSource>; 4]>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    tick_label_interpreter: Option<crate::plot::text_markup::Interpreter>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    legend_interpreter: Option<crate::plot::text_markup::Interpreter>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    colorbar: Option<crate::plot::Colorbar>,
    title: String,
    xlabel: String,
    ylabel: String,
    count: usize,
    #[serde(default)]
    series: Vec<SeriesMetadata>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    volumes: Vec<VolumeMetadata>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    slice_viewers: Vec<slice_viewer_file::ViewerMetadata>,
    /// Topmost first, using indices into each record's own series and volumes.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    data_children: Option<Vec<crate::plot::AxesDataChild>>,
    #[serde(default)]
    hold: bool,
    #[serde(default)]
    legend: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    legend_state: Option<crate::plot::LegendMetadata>,
    #[serde(
        default,
        skip_serializing_if = "crate::plot::LegendLocation::is_default"
    )]
    legend_location: crate::plot::LegendLocation,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    placement: Option<crate::plot::AxesPlacement>,
    #[serde(default = "default_grid")]
    grid: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    ygrid: Option<bool>,
    #[serde(default, with = "limit_serde")]
    xlim: Option<[f64; 2]>,
    #[serde(default, with = "limit_serde")]
    ylim: Option<[f64; 2]>,
    #[serde(default)]
    xscale: crate::plot::AxisScale,
    #[serde(default)]
    yscale: crate::plot::AxisScale,
    // 3-D axes state. Absent in every 2-D figure, so 2-D files are unchanged.
    #[serde(default = "view_2d", skip_serializing_if = "is_view_2d")]
    view: [f64; 2],
    #[serde(default, skip_serializing_if = "Option::is_none", with = "limit_serde")]
    zlim: Option<[f64; 2]>,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    zlabel: String,
    /// Stored only when it differs from the x grid (`grid on` sets both).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    zgrid: Option<bool>,
    /// `Box`, stored only when off; absent reopens with the outline.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    r#box: Option<bool>,
    /// [x, y, z] minor grids, stored only when one is on.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    minor_grid: Option<[bool; 3]>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    rulers: Option<[crate::plot::AxisRuler; 3]>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    tick_dir: Option<crate::plot::TickDir>,
    /// Axes `Layer`, stored only when `top`; absent reopens as `bottom`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    layer: Option<crate::plot::AxesLayer>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    tick_length: Option<[f64; 2]>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    line_width: Option<f64>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    constant_lines: Vec<crate::plot::ConstantLineObject>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    rectangles: Vec<crate::plot::RectangleObject>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    clim: Option<[f64; 2]>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    auto_clim: Option<[f64; 2]>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    colormap: Option<Vec<[f64; 3]>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    colormap_payload_rows: Option<usize>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    texts: Vec<crate::plot::TextObject>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    color_order: Option<Vec<[f64; 3]>>,
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    square_plot_box: bool,
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    axis_off: bool,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    axes_tag: String,
}
fn view_2d() -> [f64; 2] {
    crate::plot::view3d::VIEW_2D
}
fn is_view_2d(view: &[f64; 2]) -> bool {
    *view == crate::plot::view3d::VIEW_2D
}
fn white_color() -> [u8; 3] {
    [255; 3]
}
fn is_white_color(color: &[u8; 3]) -> bool {
    *color == [255; 3]
}
fn default_grid() -> bool {
    true
}
/// A single series keeps the original x,y CSV layout. Multiple series use
/// series,x,y rows so unequal lengths do not invent padding samples.
/// Visibility affects rendering, not data export: hidden samples are retained.
pub fn write_figure_csv(path: &Path, figure: &Figure) -> Result<()> {
    write_figure_csv_with_cancel(path, figure, &std::sync::atomic::AtomicBool::new(false))
}
pub fn write_figure_csv_with_cancel(
    path: &Path,
    figure: &Figure,
    cancel: &std::sync::atomic::AtomicBool,
) -> Result<()> {
    use std::fmt::Write as _;
    let check = || {
        if cancel.load(std::sync::atomic::Ordering::Relaxed) {
            Err("Cancelled".to_owned())
        } else {
            Ok(())
        }
    };
    check()?;
    figure.validate()?;
    if figure.all_axes().any(|(_, a)| {
        a.series
            .iter()
            .any(|s| s.kind == crate::plot::PlotKind::Patch)
    }) {
        return Err(
            "CSV figure export cannot preserve patch face topology; use MFIG or a plot script"
                .into(),
        );
    }
    let mut text = String::new();
    let mut axes: Vec<_> = figure.all_axes().collect();
    axes.sort_by_key(|(p, _)| *p);
    if figure.layout.is_some() {
        text.push_str("axes,series,x,y\n");
    } else if figure.series.len() > 1 {
        text.push_str("series,x,y\n");
    }
    for (axis, (_, a)) in axes.into_iter().enumerate() {
        for (i, s) in a.series.iter().enumerate() {
            for (index, [x, y]) in s.points.iter().enumerate() {
                if index % 4096 == 0 {
                    check()?;
                }
                if figure.layout.is_some() {
                    let _ = write!(text, "{},{},", axis + 1, i + 1);
                } else if figure.series.len() > 1 {
                    let _ = write!(text, "{},", i + 1);
                }
                let _ = writeln!(text, "{x:.17e},{y:.17e}");
            }
        }
    }
    check()?;
    atomic_write(path, text.as_bytes())
}
#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct SeriesMetadata {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    auto_palette: Option<crate::plot::AutoPalette>,
    #[serde(default, skip_serializing_if = "bar_visibility_off")]
    bar_baseline_visible: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    coordinates: Option<crate::plot::coordinates::CoordinateDescriptor>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    face_edge_color: Option<[u8; 3]>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    base_value: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    patch: Option<crate::plot::PatchFaces>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    indexed_patch: Option<indexed_patch::Metadata>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    slice_contour_level: Option<f64>,
    count: usize,
    label: String,
    kind: crate::plot::PlotKind,
    color: [u8; 3],
    explicit_color: bool,
    pattern: crate::plot::LinePattern,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    line_width: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    marker_shape: Option<crate::plot::MarkerShape>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    marker_size: Option<f64>,
    markers: bool,
    #[serde(default)]
    marker_filled: bool,
    #[serde(default = "default_grid")]
    visible: bool,
    bar_width: f64,
    bar_offset: f64,
    #[serde(default, skip_serializing_if = "is_grouped_bar_layout")]
    bar_layout: crate::plot::BarLayout,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    bar_relative_width: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    bar_group_width: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    bar_group_id: Option<usize>,
    #[serde(default, skip_serializing_if = "is_default_face_color_mode")]
    face_color_mode: crate::plot::FaceColorMode,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    patch_cdata: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    patch_vertex_source_count: Option<usize>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    patch_vertex_corner_count: Option<usize>,
    /// Series `Annotation.LegendInformation.IconDisplayStyle`: "off" marks a
    /// chart helper that is never a legend member. Absent means "on".
    #[serde(default, skip_serializing_if = "Option::is_none")]
    icon_display_style: Option<String>,
    /// The payload carries `count` z values after this series' x,y pairs.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    z: bool,
    /// Per-face fill colours (surf/mesh): this many RGB byte triples follow
    /// the series' z values in the payload; it must equal the face count.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    face_colors: Option<usize>,
    /// Per-face edge colours, after the fill colours; same rule.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    face_edge_colors: Option<usize>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    face_alpha: Option<f64>,
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    face_color_none: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    edge_alpha: Option<f64>,
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    marker_sizes: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    scatter_cdata: Option<ScatterCDataKind>,
    #[serde(default, skip_serializing_if = "is_default_marker_color")]
    marker_face_color: crate::plot::MarkerColor,
    #[serde(default, skip_serializing_if = "is_default_marker_color")]
    marker_edge_color: crate::plot::MarkerColor,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    marker_face_alpha: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    marker_edge_alpha: Option<f64>,
}
#[derive(Clone, Copy, Serialize, Deserialize)]
enum ScatterCDataKind {
    Rgb,
    Indexed,
}
fn is_default_marker_color(c: &crate::plot::MarkerColor) -> bool {
    *c == crate::plot::MarkerColor::Default
}
fn bar_visibility_off(value: &bool) -> bool {
    !*value
}
fn is_grouped_bar_layout(value: &crate::plot::BarLayout) -> bool {
    *value == crate::plot::BarLayout::Grouped
}
fn is_default_face_color_mode(value: &crate::plot::FaceColorMode) -> bool {
    *value == crate::plot::FaceColorMode::Color
}
pub(crate) const NATIVE_FIGURE_LIMIT: usize = 17_000_000;
const NATIVE_AXES_METADATA_LIMIT: usize = 1_048_576;
const NATIVE_METADATA_LIMIT: usize = 65536;
const NATIVE_EXTENDED_METADATA_LIMIT: usize =
    NATIVE_METADATA_LIMIT + crate::plot::MAX_SERIES * 4096;
const NATIVE_SUBPLOT_METADATA_LIMIT: usize = NATIVE_METADATA_LIMIT + NATIVE_EXTENDED_METADATA_LIMIT;
// One revision for axes and subplot records. Figure-level viewer metadata can
// extend these records; embedded axes must stay independent of their owner.
const NATIVE_EXTENDED_AXES: &[u8] = b"MATRFIGc";
const NATIVE_EXTENDED_SUBPLOTS: &[u8] = b"MATRFIGd";

fn native_figure_limit_error(limit: usize, has_volumes: bool) -> String {
    if has_volumes && limit == NATIVE_FIGURE_LIMIT {
        "Figure exceeds 17 MB save limit".into()
    } else {
        format!("Figure exceeds {limit} byte save limit")
    }
}

pub(super) fn needs_mfigz(axes: &crate::plot::Axes) -> bool {
    axes.series.iter().any(|s| {
        s.legend_hidden || s.has_scatter_style() || s.face_color_none || s.edge_alpha.is_some()
    }) || !axes.extras.volumes.is_empty()
        || !axes.extras.constant_lines.is_empty()
        || !axes.extras.rectangles.is_empty()
        || axes.extras.clim.is_some()
        || axes.extras.auto_clim.is_some()
        || axes.extras.colormap.is_some()
}

fn has_multiple_bar_groups(axes: &crate::plot::Axes) -> bool {
    let mut first: Option<&crate::ObjectIdentity> = None;
    let mut has_none_bar = false;
    for s in &axes.series {
        if s.kind != crate::plot::PlotKind::Bar {
            continue;
        }
        match &s.bar_group_id {
            Some(id) => {
                if let Some(prev) = first {
                    if !prev.same_object(id) {
                        return true;
                    }
                } else {
                    first = Some(id);
                }
                if has_none_bar {
                    return true;
                }
            }
            None => {
                has_none_bar = true;
                if first.is_some() {
                    return true;
                }
            }
        }
    }
    false
}

pub(super) fn needs_mfigb(axes: &crate::plot::Axes) -> bool {
    axes.extras.colorbar.is_some()
        || axes.series.iter().any(|s| s.indexed_patch.is_some())
        || !axes.extras.texts.is_empty()
        || !axes.extras.subtitle.is_default()
        || !axes.extras.title_style.is_default()
        || axes.extras.label_styles.iter().any(|s| !s.is_default())
        || axes.extras.color_order.is_some()
        || has_multiple_bar_groups(axes)
        || axes.series.iter().any(|s| s.bar_group_width.is_some())
}

pub fn save_figure(path: &Path, figure: &Figure) -> Result<()> {
    save_figure_with_limit(path, figure, NATIVE_FIGURE_LIMIT)
}
pub fn save_figure_with_limit(path: &Path, figure: &Figure, limit: usize) -> Result<()> {
    if path
        .extension()
        .is_some_and(|s| s.eq_ignore_ascii_case("fig"))
    {
        return save_matlab_figure(path, figure);
    }
    atomic_write(path, &encode_native_figure_with_limit(figure, limit)?)
}
pub fn encode_native_figure(figure: &Figure) -> Result<Vec<u8>> {
    encode_native_figure_with_limit(figure, NATIVE_FIGURE_LIMIT)
}
pub fn encode_native_figure_with_limit(figure: &Figure, limit: usize) -> Result<Vec<u8>> {
    let limit = limit.min(NATIVE_FIGURE_LIMIT);
    figure.validate()?;
    if figure.layout.is_some() {
        return subplot_file::encode_with_limit(figure, limit);
    }
    let slice_viewers = slice_viewer_file::capture(figure, &[&figure.axes])?;
    let viewer_payload = slice_viewer_file::payload_size(&slice_viewers, 1)?;
    let axes_colormap = figure
        .extras
        .colormap
        .as_ref()
        .filter(|_| !slice_viewer_file::shares_colormap(&slice_viewers, 0));
    let emit_bar_groups = has_multiple_bar_groups(&figure.axes);
    let mut bar_groups: Vec<crate::ObjectIdentity> = Vec::new();
    let mut bar_group_index = |id: Option<&crate::ObjectIdentity>| -> Option<usize> {
        if !emit_bar_groups {
            return None;
        }
        let id = id?;
        if let Some(pos) = bar_groups
            .iter()
            .position(|existing| existing.same_object(id))
        {
            Some(pos)
        } else {
            bar_groups.push(id.clone());
            Some(bar_groups.len() - 1)
        }
    };
    let metadata = FigureMetadata {
        palette_cycle: figure.axes.palette_cycle,
        axes_colors: figure.axes.colors,
        appearance: figure.appearance,
        invert_hardcopy: figure.invert_hardcopy,
        latent_baselines: baseline_metadata::encode_latent(&figure.axes),
        baselines: baseline_metadata::encode(&figure.axes)?,
        name: figure.name.clone(),
        color: figure.color,
        position: figure.position,
        units: figure.units,
        number_title: figure.number_title,
        visible: figure.visible,
        resize: figure.resize,
        window_state: figure.window_state,
        textboxes: figure.textboxes.clone(),
        annotations: figure.annotations.clone(),
        subtitle: figure.extras.subtitle.clone(),
        title_style: figure.extras.title_style,
        label_styles: figure
            .extras
            .label_styles
            .iter()
            .any(|s| !s.is_default())
            .then_some(figure.extras.label_styles),
        label_interpreters: (figure.extras.label_interpreters
            != [crate::plot::text_markup::Interpreter::Tex; 4])
            .then_some(figure.extras.label_interpreters),
        label_lines: figure
            .extras
            .label_lines
            .iter()
            .any(Option::is_some)
            .then(|| figure.extras.label_lines.clone()),
        tick_label_interpreter: (figure.extras.tick_label_interpreter != Default::default())
            .then_some(figure.extras.tick_label_interpreter),
        legend_interpreter: (figure.extras.legend_interpreter != Default::default())
            .then_some(figure.extras.legend_interpreter),
        colorbar: figure.extras.colorbar.clone(),
        title: figure.title.clone(),
        xlabel: figure.xlabel.clone(),
        ylabel: figure.ylabel.clone(),
        count: figure.point_count(),
        series: figure
            .series
            .iter()
            .map(|s| SeriesMetadata {
                auto_palette: s.auto_palette,
                bar_baseline_visible: s.bar_baseline_visible,
                coordinates: s.points.patch().and_then(|p| p.native_descriptor()),
                face_edge_color: s.face_edge_color,
                patch: s.patch,
                indexed_patch: s.indexed_patch.as_ref().map(indexed_patch::Metadata::from),
                slice_contour_level: s.slice_contour_level,
                base_value: s.base_value,
                count: s.points.len(),
                label: s.label.clone(),
                kind: s.kind,
                color: s.color,
                explicit_color: s.explicit_color,
                pattern: s.pattern,
                line_width: s.line_width,
                marker_shape: s.marker_shape,
                marker_size: s.marker_size,
                markers: s.markers,
                marker_filled: s.marker_filled,
                visible: s.visible,
                bar_width: s.bar_width,
                bar_offset: s.bar_offset,
                bar_layout: s.bar_layout,
                bar_relative_width: s.bar_relative_width,
                bar_group_width: s.bar_group_width,
                bar_group_id: bar_group_index(s.bar_group_id.as_ref()),
                face_color_mode: s.face_color_mode,
                patch_cdata: s.patch_cdata,
                patch_vertex_source_count: s.patch_vertex_cdata.as_ref().map(|v| v.source.len()),
                patch_vertex_corner_count: s.patch_vertex_cdata.as_ref().map(|v| v.corners.len()),
                icon_display_style: s.legend_hidden.then(|| "off".to_owned()),
                z: s.is_3d(),
                face_colors: s.face_colors.as_ref().map(|c| c.len()),
                face_edge_colors: s.face_edge_colors.as_ref().map(|c| c.len()),
                face_alpha: s.face_alpha,
                face_color_none: s.face_color_none,
                edge_alpha: s.edge_alpha,
                marker_sizes: s.marker_sizes.is_some(),
                scatter_cdata: s.scatter_cdata.as_ref().map(|c| match c {
                    crate::plot::ScatterCData::Rgb(_) => ScatterCDataKind::Rgb,
                    crate::plot::ScatterCData::Indexed(_) => ScatterCDataKind::Indexed,
                }),
                marker_face_color: s.marker_face_color,
                marker_edge_color: s.marker_edge_color,
                marker_face_alpha: s.marker_face_alpha,
                marker_edge_alpha: s.marker_edge_alpha,
            })
            .collect(),
        volumes: figure
            .axes
            .extras
            .volumes
            .iter()
            .map(VolumeMetadata::encode)
            .collect::<Result<_>>()?,
        slice_viewers,
        data_children: (!figure.axes.extras.volumes.is_empty())
            .then(|| crate::plot::axes_data_children(&figure.axes)),
        hold: figure.hold,
        legend: figure.legend,
        legend_state: figure.legend_metadata(),
        legend_location: figure.legend_location,
        placement: figure.placement,
        grid: figure.xgrid,
        ygrid: (figure.ygrid != figure.xgrid).then_some(figure.ygrid),
        xlim: figure.xlim,
        ylim: figure.ylim,
        xscale: figure.xscale,
        yscale: figure.yscale,
        view: figure.view,
        zlim: figure.zlim,
        zlabel: figure.zlabel.clone(),
        zgrid: (figure.zgrid != figure.xgrid && crate::plot::view3d::is_3d(figure.view))
            .then_some(figure.zgrid),
        r#box: (!figure.box_outline).then_some(false),
        minor_grid: figure
            .minor_grid
            .contains(&true)
            .then_some(figure.minor_grid),
        rulers: (!figure
            .extras
            .rulers
            .iter()
            .all(|r| *r == crate::plot::AxisRuler::default()))
        .then(|| figure.extras.rulers.clone()),
        tick_dir: figure.extras.tick_dir,
        layer: figure
            .extras
            .layer
            .is_top()
            .then_some(crate::plot::AxesLayer::Top),
        tick_length: figure.extras.tick_length,
        line_width: figure.extras.line_width,
        constant_lines: figure.extras.constant_lines.clone(),
        rectangles: figure.extras.rectangles.clone(),
        clim: figure.extras.clim,
        auto_clim: figure.extras.auto_clim,
        colormap: None,
        colormap_payload_rows: axes_colormap.map(|map| map.len()),
        texts: figure.extras.texts.clone(),
        color_order: figure
            .extras
            .color_order
            .as_ref()
            .map(|order| order.as_ref().clone()),
        square_plot_box: figure.extras.square_plot_box,
        axis_off: figure.extras.axis_off,
        axes_tag: figure.extras.tag.clone(),
    };
    let json = serde_json::to_vec(&metadata).map_err(|e| e.to_string())?;
    let compact = metadata.series.iter().any(|s| s.coordinates.is_some());
    let extended = !metadata.volumes.is_empty() || !metadata.slice_viewers.is_empty();
    if json.len() > NATIVE_AXES_METADATA_LIMIT {
        return Err("Figure exceeds save metadata limit".into());
    }
    let (_, series_payload) = native_series_payload(&metadata.series, compact)?;
    let volume_payload = native_volume_payload(&metadata.volumes)?;
    let limit_error = || native_figure_limit_error(limit, !metadata.volumes.is_empty());
    let colormap_bytes = metadata.colormap_payload_rows.unwrap_or(0) * 24;
    let total_size = 12usize
        .checked_add(json.len())
        .and_then(|n| n.checked_add(series_payload))
        .and_then(|n| n.checked_add(volume_payload))
        .and_then(|n| n.checked_add(colormap_bytes))
        .and_then(|n| n.checked_add(viewer_payload))
        .ok_or_else(limit_error)?;
    if total_size > limit {
        return Err(limit_error());
    }
    let mut bytes = Vec::with_capacity(total_size);
    bytes.extend_from_slice(if extended {
        NATIVE_EXTENDED_AXES
    } else if needs_mfigb(&figure.axes) {
        b"MATRFIGb"
    } else if needs_mfigz(&figure.axes) {
        b"MATRFIGZ"
    } else if figure.has_static_style_metadata() {
        b"MATRFIGX"
    } else if figure.has_palette_metadata() {
        b"MATRFIGV"
    } else if figure.axes.colors.is_some() {
        b"MATRFIGT"
    } else if figure.appearance.is_some() {
        b"MATRFIGR"
    } else if metadata.latent_baselines.is_some()
        || !figure.baselines.is_empty()
        || baseline_metadata::needs_bar_visibility(&figure.axes)
    {
        b"MATRFIGP"
    } else if compact {
        b"MATRFIGN"
    } else if figure
        .series
        .iter()
        .any(crate::plot::Series::has_face_style)
    {
        b"MATRFIGL"
    } else if figure.legend_state.is_some()
        || figure.xgrid != figure.ygrid
        || !figure.name.is_empty()
        || figure.color != [255; 3]
        || !figure.textboxes.is_empty()
        || !figure.annotations.is_empty()
        || figure
            .series
            .iter()
            .any(|s| s.patch.is_some() || s.base_value.is_some())
    {
        b"MATRFIGJ"
    } else if figure.legend_location.requires_extended_layout() || figure.placement.is_some() {
        b"MATRFIGH"
    } else if figure
        .series
        .iter()
        .any(|s| s.pattern == crate::plot::LinePattern::None)
    {
        b"MATRFIGF"
    } else if !figure.legend_location.is_default() {
        b"MATRFIGD"
    } else if figure.series.iter().any(|s| s.has_marker_style()) {
        b"MATRFIGB"
    } else if figure.series.iter().any(|s| s.line_width.is_some()) {
        b"MATRFIG9"
    } else if figure.series.iter().any(|s| {
        matches!(
            s.kind,
            crate::plot::PlotKind::Stairs | crate::plot::PlotKind::Stem
        )
    }) {
        b"MATRFIG5"
    } else {
        b"MATRFIG4"
    });
    bytes.extend_from_slice(&(json.len() as u32).to_le_bytes());
    bytes.extend_from_slice(&json);
    let cancel = std::sync::atomic::AtomicBool::new(false);
    for (series, item) in figure.series.iter().zip(&metadata.series) {
        if let Some(mesh) = &series.indexed_patch {
            indexed_patch::write(&mut bytes, series, mesh)?;
        } else if item.coordinates.is_some() {
            series
                .points
                .patch()
                .ok_or("Missing compact coordinates")?
                .write_native_payload(&mut bytes, &cancel)?;
        } else {
            for [x, y] in series.points.iter() {
                bytes.extend_from_slice(&x.to_le_bytes());
                bytes.extend_from_slice(&y.to_le_bytes());
            }
            for z in series.z.iter() {
                bytes.extend_from_slice(&z.to_le_bytes());
            }
        }
        for list in [&series.face_colors, &series.face_edge_colors]
            .into_iter()
            .flatten()
        {
            for rgb in list.iter() {
                bytes.extend_from_slice(rgb);
            }
        }
        if let Some(sizes) = &series.marker_sizes {
            for size in sizes.iter() {
                bytes.extend_from_slice(&size.to_le_bytes());
            }
        }
        if let Some(cdata) = &series.scatter_cdata {
            match cdata {
                crate::plot::ScatterCData::Rgb(rgbs) => {
                    for rgb in rgbs.iter() {
                        bytes.extend_from_slice(rgb);
                    }
                }
                crate::plot::ScatterCData::Indexed(vals) => {
                    for v in vals.iter() {
                        bytes.extend_from_slice(&v.to_le_bytes());
                    }
                }
            }
        }
        if let Some(vertex) = &series.patch_vertex_cdata {
            write_native_f64(&mut bytes, vertex.source.iter());
            for corner in vertex.corners.iter() {
                bytes.extend_from_slice(&corner.to_le_bytes());
            }
        }
    }
    for volume in &figure.axes.extras.volumes {
        if let Some(exact) = &volume.exact_data {
            exact.write_le_chunks(|chunk| {
                bytes.extend_from_slice(chunk);
                Ok(())
            })?;
        } else {
            write_native_f64(&mut bytes, volume.data.iter());
        }
        if let Some(overlay) = &volume.overlay_data {
            write_native_f64(&mut bytes, overlay.data.iter());
        }
        write_native_f64(&mut bytes, volume.overlay_colormap.iter().flatten());
        write_native_f64(&mut bytes, volume.overlay_alphamap.iter());
    }
    if let Some(map) = axes_colormap {
        write_native_f64(&mut bytes, map.iter().flatten());
    }
    slice_viewer_file::write_payload(&mut bytes, figure);
    if bytes.len() > limit {
        return Err(limit_error());
    }
    Ok(bytes)
}

fn write_native_f64<'a>(bytes: &mut Vec<u8>, values: impl Iterator<Item = &'a f64>) {
    for &value in values {
        bytes.extend_from_slice(&value.to_le_bytes());
    }
}

fn native_volume_payload(volumes: &[VolumeMetadata]) -> Result<usize> {
    if volumes.len() > crate::plot::MAX_VOLUMES {
        return Err("Volume count exceeds figure limit".into());
    }
    let mut total_voxels = 0usize;
    let mut map_samples = 0usize;
    for v in volumes {
        if v.dims.contains(&0) {
            return Err("Invalid volume dimensions".into());
        }
        let n = v.dims[0]
            .checked_mul(v.dims[1])
            .and_then(|x| x.checked_mul(v.dims[2]))
            .ok_or("Volume size overflow")?;
        v.exact_class()?;
        let has_overlay_data = v.overlay.as_ref().is_some_and(|o| o.class.is_some());
        total_voxels = n
            .checked_mul(1 + usize::from(has_overlay_data))
            .and_then(|n| total_voxels.checked_add(n))
            .ok_or("Volume voxel count overflow")?;
        if let Some(o) = &v.overlay {
            if !(1..=crate::plot::MAX_COLORMAP_ROWS).contains(&o.colormap_rows)
                || !(1..=crate::plot::MAX_COLORMAP_ROWS).contains(&o.alphamap_len)
                || !o.display_range.iter().all(|x| x.is_finite())
                || o.display_range[0] > o.display_range[1]
            {
                return Err("Invalid native volume overlay metadata".into());
            }
            map_samples += o.colormap_rows * 3 + o.alphamap_len;
        }
    }
    if total_voxels > crate::plot::MAX_VOLUME_VOXELS {
        return Err("Volume voxels exceed figure limit".into());
    }
    total_voxels
        .checked_add(map_samples)
        .and_then(|n| n.checked_mul(8))
        .ok_or_else(|| "Volume payload extent overflow".into())
}

fn native_axes_child_identities(
    order: &[crate::plot::AxesDataChild],
    series: usize,
    volumes: usize,
) -> Result<Vec<Option<crate::ObjectIdentity>>> {
    use crate::plot::AxesDataChild;
    if order.len() != series + volumes {
        return Err("Invalid native axes child order length".into());
    }
    let mut ids = vec![None; order.len()];
    for child in order.iter().rev() {
        let index = match *child {
            AxesDataChild::Series(i) if i < series => i,
            AxesDataChild::Volume(i) if i < volumes => series + i,
            _ => return Err("Invalid native axes child order index".into()),
        };
        if ids[index].is_some() {
            return Err("Duplicate native axes child order entry".into());
        }
        ids[index] = Some(crate::ObjectIdentity::default());
    }
    Ok(ids)
}

fn take_native_f64<'a>(
    payload: &mut &'a [u8],
    count: usize,
) -> impl ExactSizeIterator<Item = f64> + use<'a> {
    let samples = &payload[..count * 8];
    *payload = &payload[count * 8..];
    samples
        .as_chunks::<8>()
        .0
        .iter()
        .map(|&b| f64::from_le_bytes(b))
}

impl VolumeMetadata {
    fn decode(self, payload: &mut &[u8]) -> Result<crate::plot::VolumeObject> {
        use crate::plot::{OverlayVolume, VolumeObject};
        let [ny, nx, nz] = self.dims;
        let count = ny * nx * nz;
        let exact = self
            .exact_class()?
            .map(|class| {
                use std::io::Read;
                crate::TypedArray::from_le_reader(class, ny, nx * nz, |chunk| {
                    payload.read_exact(chunk).map_err(|e| e.to_string())
                })?
                .with_dims(&self.dims)
            })
            .transpose()?;
        let data = if let Some(data) = &exact {
            (0..count).map(|i| data.real(i)).collect()
        } else {
            take_native_f64(payload, count).collect()
        };
        let mut volume = VolumeObject::new(self.dims, data, self.data_class.to_image_class());
        volume.exact_data = exact;
        volume.rendering_style = self.rendering_style;
        volume.colormap = Arc::new(self.colormap);
        volume.alphamap = Arc::new(self.alphamap);
        volume.data_limits = self.data_limits;
        volume.data_limits_manual = self.data_limits_manual;
        volume.isosurface_value = self.isosurface_value;
        volume.isosurface_color = self.isosurface_color;
        volume.slice_plane_values = self.slice_plane_values;
        volume.transformation = self.transformation;
        volume.visible = self.visible;
        if let Some(overlay) = self.overlay {
            if let Some(class) = overlay.class {
                let class = class.decode();
                let data: Arc<[f64]> = take_native_f64(payload, count).collect();
                validate_native_overlay(&class, &data)?;
                volume.overlay_data = Some(OverlayVolume { class, data });
            }
            volume.overlay_rendering_style = overlay.rendering_style;
            volume.overlay_colormap = Arc::new(
                (0..overlay.colormap_rows)
                    .map(|_| {
                        let mut rgb = take_native_f64(payload, 3);
                        std::array::from_fn(|_| rgb.next().unwrap())
                    })
                    .collect(),
            );
            volume.overlay_alphamap =
                Arc::new(take_native_f64(payload, overlay.alphamap_len).collect());
            volume.overlay_display_range = overlay.display_range;
            volume.overlay_display_range_manual = overlay.display_range_manual;
        }
        Ok(volume)
    }
}

fn native_series_payload(
    series: &[SeriesMetadata],
    compact: bool,
) -> Result<(crate::plot::coordinates::GeometryLedger, usize)> {
    use crate::plot::{
        PlotKind,
        coordinates::{GeometryCost, GeometryLedger},
    };
    let mut ledger = GeometryLedger::default();
    let mut payload = 0usize;
    for item in series {
        let contour = item.slice_contour_level.is_some();
        if contour
            && (item.kind != PlotKind::Patch
                || item.patch.is_some()
                || item.coordinates.is_some()
                || item.count < 2
                || !item.z)
        {
            return Err("Invalid native contourslice patch".into());
        }
        let (cost, bytes) = if let Some(mesh) = &item.indexed_patch {
            mesh.preflight(item)?
        } else if let Some(descriptor) = &item.coordinates {
            if item.z {
                // Compact coordinates carry no z payload; a z flag would put the
                // decoder's z slice past the samples.
                return Err("Native z samples belong only to line and scatter series".into());
            }
            if !compact
                || item.kind != PlotKind::Patch
                || item.patch.is_none_or(|p| p.vertices_per_face() != 4)
                || item.count != descriptor.vertex_count()
            {
                return Err("Invalid native compact Patch topology".into());
            }
            descriptor.preflight()?
        } else {
            let cost = if let Some(faces) = item.patch {
                if item.kind != PlotKind::Patch {
                    return Err("Invalid native Patch kind".into());
                }
                GeometryCost::dense_face_allocation(item.count, faces.vertices_per_face())?
            } else {
                if item.kind == PlotKind::Patch && !contour {
                    return Err("Native Patch topology missing".into());
                }
                GeometryCost::ordinary_allocation(item.count, item.kind == PlotKind::Bar)?
            };
            let surface = item.kind == PlotKind::Patch
                && item.patch.is_some_and(|p| p.vertices_per_face() == 4);
            if item.z
                && !(matches!(item.kind, PlotKind::Line | PlotKind::Scatter) || surface || contour)
            {
                return Err(
                    "Native z samples belong only to line, scatter and 3-D patch series".into(),
                );
            }
            (
                cost,
                item.count
                    .checked_mul(if item.z { 24 } else { 16 })
                    .ok_or("Native sample extent overflow")?,
            )
        };
        ledger.add(cost)?;
        let mut colors = 0usize;
        for (index, list) in [item.face_colors, item.face_edge_colors]
            .into_iter()
            .enumerate()
        {
            let Some(list) = list else {
                continue;
            };
            let faces = match (index, item.kind, item.patch) {
                (_, PlotKind::Patch, Some(faces)) => faces.face_count(item.count)?,
                (0, PlotKind::Bar, None) => item.count,
                _ => return Err("Per-face colours belong only to Patch or Bar series".into()),
            };
            if list != faces {
                return Err("Per-face colour list length differs from the face count".into());
            }
            colors = colors
                .checked_add(list.checked_mul(3).ok_or("Native colour extent overflow")?)
                .ok_or("Native colour extent overflow")?;
        }
        if item.face_alpha.is_some_and(|a| {
            !matches!(item.kind, PlotKind::Patch | PlotKind::Bar)
                || !a.is_finite()
                || !(0. ..=1.).contains(&a)
        }) {
            return Err("Invalid native FaceAlpha".into());
        }
        if item
            .patch_cdata
            .is_some_and(|v| !v.is_finite() || item.kind != PlotKind::Patch)
        {
            return Err("Invalid native Patch CData".into());
        }
        // Per-vertex CData travels in the binary payload (8 bytes per source
        // value, 4 per corner); values and corner indices are checked on decode.
        let vertex_bytes = match (
            item.patch_vertex_source_count,
            item.patch_vertex_corner_count,
        ) {
            (None, None) => 0,
            (Some(sources), Some(corners))
                if item.kind == PlotKind::Patch
                    && corners % 3 == 0
                    && sources <= u32::MAX as usize =>
            {
                sources
                    .checked_mul(8)
                    .zip(corners.checked_mul(4))
                    .and_then(|(a, b)| a.checked_add(b))
                    .ok_or("Native vertex CData extent overflow")?
            }
            _ => return Err("Invalid native Patch vertex CData".into()),
        };
        if item
            .edge_alpha
            .is_some_and(|a| item.kind != PlotKind::Patch || !(0. ..=1.).contains(&a))
        {
            return Err("Invalid native EdgeAlpha".into());
        }
        let mut scatter_bytes = 0usize;
        if item.marker_sizes {
            if item.kind != PlotKind::Scatter {
                return Err("Per-point SizeData belongs only to Scatter series".into());
            }
            scatter_bytes = scatter_bytes
                .checked_add(
                    item.count
                        .checked_mul(8)
                        .ok_or("Native scatter extent overflow")?,
                )
                .ok_or("Native scatter extent overflow")?;
        }
        if let Some(kind) = item.scatter_cdata {
            if item.kind != PlotKind::Scatter {
                return Err("Per-point CData belongs only to Scatter series".into());
            }
            let elem = match kind {
                ScatterCDataKind::Rgb => 3,
                ScatterCDataKind::Indexed => 8,
            };
            scatter_bytes = scatter_bytes
                .checked_add(
                    item.count
                        .checked_mul(elem)
                        .ok_or("Native scatter extent overflow")?,
                )
                .ok_or("Native scatter extent overflow")?;
        }
        if let Some(a) = item.marker_face_alpha {
            crate::plot::validate_alpha(a, "MarkerFaceAlpha")?;
        }
        if let Some(a) = item.marker_edge_alpha {
            crate::plot::validate_alpha(a, "MarkerEdgeAlpha")?;
        }
        payload = payload
            .checked_add(bytes)
            .and_then(|p| p.checked_add(colors))
            .and_then(|p| p.checked_add(scatter_bytes))
            .and_then(|p| p.checked_add(vertex_bytes))
            .ok_or("Native payload extent overflow")?;
    }
    Ok((ledger, payload))
}

pub fn load_figure(path: &Path) -> Result<Figure> {
    if path
        .extension()
        .is_some_and(|s| s.eq_ignore_ascii_case("fig"))
    {
        return load_matlab_figure(path);
    }
    let bytes = read_bounded(path, NATIVE_FIGURE_LIMIT)?;
    decode_native_figure(&bytes)
}
pub(crate) fn decode_native_figure(bytes: &[u8]) -> Result<Figure> {
    decode_native_figure_with_ledger(bytes, None)
}
fn decode_native_figure_with_ledger(
    bytes: &[u8],
    parent_ledger: Option<crate::plot::coordinates::GeometryLedger>,
) -> Result<Figure> {
    if bytes.len() < 12 || bytes.len() > NATIVE_FIGURE_LIMIT {
        return Err("Invalid native figure length".into());
    }
    let magic = &bytes[..8];
    if subplot_file::is_container(magic) {
        return subplot_file::decode(bytes);
    }
    let extended = magic == NATIVE_EXTENDED_AXES;
    let version: &[u8] = if extended { b"MATRFIGb" } else { magic };
    if !matches!(
        version,
        b"MATRFIG1"
            | b"MATRFIG2"
            | b"MATRFIG3"
            | b"MATRFIG4"
            | b"MATRFIG5"
            | b"MATRFIG9"
            | b"MATRFIGB"
            | b"MATRFIGD"
            | b"MATRFIGF"
            | b"MATRFIGH"
            | b"MATRFIGJ"
            | b"MATRFIGL"
            | b"MATRFIGN"
            | b"MATRFIGP"
            | b"MATRFIGR"
            | b"MATRFIGT"
            | b"MATRFIGV"
            | b"MATRFIGX"
            | b"MATRFIGZ"
            | b"MATRFIGb"
    ) {
        return Err("Not a supported MatRust figure (.mrfig)".into());
    }
    let size = u32::from_le_bytes(bytes[8..12].try_into().unwrap()) as usize;
    let compact = matches!(
        version,
        b"MATRFIGN"
            | b"MATRFIGP"
            | b"MATRFIGR"
            | b"MATRFIGT"
            | b"MATRFIGV"
            | b"MATRFIGX"
            | b"MATRFIGZ"
            | b"MATRFIGb"
    );
    if size > NATIVE_AXES_METADATA_LIMIT || size > bytes.len() - 12 {
        return Err("Invalid figure metadata size".into());
    }
    let json = &bytes[12..12 + size];
    let mut metadata: FigureMetadata = serde_json::from_slice(json).map_err(|e| e.to_string())?;
    if !metadata.slice_viewers.is_empty() {
        if !extended {
            return Err("Slice viewer metadata requires MFIGc".into());
        }
        if parent_ledger.is_some() {
            return Err("Slice viewers belong to the figure, not an embedded axes record".into());
        }
    }
    let viewer_payload = slice_viewer_file::payload_size(&metadata.slice_viewers, 1)?;
    let palette_metadata = metadata.palette_cycle.is_some()
        || metadata
            .appearance
            .is_some_and(|a| a.series_palette.is_some())
        || metadata.series.iter().any(|s| s.auto_palette.is_some());
    if (metadata.invert_hardcopy.is_some()
        || metadata.axes_colors.is_some_and(|c| c.legend.is_some()))
        && !matches!(version, b"MATRFIGX" | b"MATRFIGZ" | b"MATRFIGb")
    {
        return Err("Legend RGB and print preference require MFIGX".into());
    }
    if !matches!(version, b"MATRFIGZ" | b"MATRFIGb")
        && metadata
            .series
            .iter()
            .any(|s| s.icon_display_style.is_some())
    {
        return Err("Legend helper metadata requires MFIGZ".into());
    }
    if version != b"MATRFIGb"
        && (metadata.colorbar.is_some()
            || metadata.series.iter().any(|s| s.indexed_patch.is_some()))
    {
        return Err("Colorbar and indexed patch metadata require MFIGb".into());
    }
    if version != b"MATRFIGb"
        && (!metadata.texts.is_empty()
            || !metadata.subtitle.is_default()
            || !metadata.title_style.is_default()
            || metadata.label_styles.is_some()
            || metadata.color_order.is_some()
            || metadata
                .series
                .iter()
                .any(|s| s.bar_group_id.is_some() || s.bar_group_width.is_some()))
    {
        return Err("Text, color order and bar group metadata require MFIGb".into());
    }
    if palette_metadata
        && !matches!(
            version,
            b"MATRFIGV" | b"MATRFIGX" | b"MATRFIGZ" | b"MATRFIGb"
        )
    {
        return Err("Automatic palette metadata requires MFIGV".into());
    }
    if let Some(cycle) = metadata.palette_cycle {
        cycle.validate()?;
    }
    for series in &metadata.series {
        if let Some(palette) = series.auto_palette {
            palette.validate()?;
            if series.kind == crate::plot::PlotKind::Patch {
                return Err(
                    "Patch colors require explicit RGB, not automatic palette metadata".into(),
                );
            }
        }
    }
    if metadata.axes_colors.is_some()
        && !matches!(
            version,
            b"MATRFIGT" | b"MATRFIGV" | b"MATRFIGX" | b"MATRFIGZ" | b"MATRFIGb"
        )
    {
        return Err("Axes RGB metadata requires MFIGT".into());
    }
    if let Some(colors) = metadata.axes_colors {
        colors.validate()?;
    }
    if metadata.appearance.is_some()
        && !matches!(
            version,
            b"MATRFIGR" | b"MATRFIGT" | b"MATRFIGV" | b"MATRFIGX" | b"MATRFIGZ" | b"MATRFIGb"
        )
    {
        return Err("Figure appearance intent requires MFIGR".into());
    }
    if metadata.series.len() > crate::plot::MAX_PLOT_SERIES
        || metadata.count > crate::plot::coordinates::MAX_EXPANDED_VERTICES
    {
        return Err("Figure samples exceed limits".into());
    }
    if !compact && metadata.series.iter().any(|s| s.coordinates.is_some()) {
        return Err("Compact coordinates require MFIGN".into());
    }
    if !matches!(
        version,
        b"MATRFIGJ"
            | b"MATRFIGL"
            | b"MATRFIGN"
            | b"MATRFIGP"
            | b"MATRFIGR"
            | b"MATRFIGT"
            | b"MATRFIGV"
            | b"MATRFIGX"
            | b"MATRFIGZ"
            | b"MATRFIGb"
    ) && (metadata.legend_state.is_some()
        || metadata.ygrid.is_some()
        || !metadata.name.is_empty()
        || metadata.color != [255; 3]
        || !metadata.textboxes.is_empty()
        || !metadata.annotations.is_empty()
        || metadata.series.iter().any(|s| {
            s.base_value.is_some() || s.patch.is_some() || s.kind == crate::plot::PlotKind::Patch
        }))
    {
        return Err("Advanced figure metadata requires MFIGJ".into());
    }
    if metadata.series.iter().any(|s| {
        s.face_edge_color.is_some()
            || (s.kind == crate::plot::PlotKind::Patch
                && (s.pattern != crate::plot::LinePattern::None || s.line_width.is_some()))
    }) && !matches!(
        version,
        b"MATRFIGL"
            | b"MATRFIGN"
            | b"MATRFIGP"
            | b"MATRFIGR"
            | b"MATRFIGT"
            | b"MATRFIGV"
            | b"MATRFIGX"
            | b"MATRFIGZ"
            | b"MATRFIGb"
    ) {
        return Err("Independent face outlines require MFIGL".into());
    }
    if (metadata.latent_baselines.is_some()
        || !metadata.baselines.is_empty()
        || metadata.series.iter().any(|s| s.bar_baseline_visible))
        && !matches!(
            version,
            b"MATRFIGP"
                | b"MATRFIGR"
                | b"MATRFIGT"
                | b"MATRFIGV"
                | b"MATRFIGX"
                | b"MATRFIGZ"
                | b"MATRFIGb"
        )
    {
        return Err("Independent axes baselines require MFIGP".into());
    }
    if metadata
        .series
        .iter()
        .any(|s| s.bar_baseline_visible && s.kind != crate::plot::PlotKind::Bar)
    {
        return Err("Native implicit Bar visibility requires a Bar series".into());
    }
    baseline_metadata::preflight(&metadata.baselines, &metadata.series)?;
    baseline_metadata::preflight_latent(
        metadata.latent_baselines.as_deref(),
        &metadata.baselines,
        &metadata.series,
    )?;
    if let Some(state) = &metadata.legend_state {
        if state.members.len() > metadata.series.len()
            || state.members.iter().any(|&i| i >= metadata.series.len())
        {
            return Err("Invalid native legend membership extent".into());
        }
        let mut seen = std::collections::HashSet::with_capacity(state.members.len());
        for index in &state.members {
            if !seen.insert(index) {
                return Err("Duplicate native legend member".into());
            }
        }
    }
    if metadata.name.len() > 4096 {
        return Err("Figure Name exceeds 4096 UTF-8 bytes".into());
    }
    for s in &metadata.series {
        if s.face_edge_color.is_some()
            && !matches!(
                s.kind,
                crate::plot::PlotKind::Bar | crate::plot::PlotKind::Patch
            )
        {
            return Err("Invalid independent face outline target".into());
        }
        if s.base_value
            .is_some_and(|v| !v.is_finite() || s.kind != crate::plot::PlotKind::Stem)
        {
            return Err("Invalid stem BaseValue metadata".into());
        }
        match (s.kind, s.patch) {
            (crate::plot::PlotKind::Patch, Some(p)) => {
                p.face_count(s.count)?;
            }
            (crate::plot::PlotKind::Patch, None) if s.slice_contour_level.is_some() => {}
            (crate::plot::PlotKind::Patch, None) | (_, Some(_)) => {
                return Err("Patch geometry and series kind disagree".into());
            }
            (_, None) => {}
        }
    }
    if version == b"MATRFIG1" {
        if !metadata.series.is_empty() {
            return Err("Unexpected series metadata in legacy figure".into());
        }
        metadata.series.push(SeriesMetadata {
            auto_palette: None,
            bar_baseline_visible: false,
            coordinates: None,
            face_edge_color: None,
            patch: None,
            indexed_patch: None,
            slice_contour_level: None,
            base_value: None,
            count: metadata.count,
            label: "data1".into(),
            kind: crate::plot::PlotKind::Line,
            color: crate::plot::COLORS[0],
            explicit_color: false,
            pattern: crate::plot::LinePattern::Solid,
            line_width: None,
            marker_shape: None,
            marker_size: None,
            markers: false,
            marker_filled: false,
            visible: true,
            bar_width: 0.8,
            bar_offset: 0.,
            bar_layout: crate::plot::BarLayout::Grouped,
            bar_relative_width: None,
            bar_group_width: None,
            bar_group_id: None,
            face_color_mode: crate::plot::FaceColorMode::Color,
            patch_cdata: None,
            patch_vertex_source_count: None,
            patch_vertex_corner_count: None,
            icon_display_style: None,
            z: false,
            face_colors: None,
            face_edge_colors: None,
            face_alpha: None,
            face_color_none: false,
            edge_alpha: None,
            marker_sizes: false,
            scatter_cdata: None,
            marker_face_color: crate::plot::MarkerColor::Default,
            marker_edge_color: crate::plot::MarkerColor::Default,
            marker_face_alpha: None,
            marker_edge_alpha: None,
        });
    }
    let total = metadata
        .series
        .iter()
        .try_fold(0usize, |n, s| n.checked_add(s.count));
    if !matches!(
        version,
        b"MATRFIGF"
            | b"MATRFIGH"
            | b"MATRFIGJ"
            | b"MATRFIGL"
            | b"MATRFIGN"
            | b"MATRFIGP"
            | b"MATRFIGR"
            | b"MATRFIGT"
            | b"MATRFIGV"
            | b"MATRFIGX"
            | b"MATRFIGZ"
            | b"MATRFIGb"
    ) && metadata
        .series
        .iter()
        .any(|s| s.pattern == crate::plot::LinePattern::None)
    {
        return Err("LineStyle none metadata requires MFIGF".into());
    }
    if !matches!(
        version,
        b"MATRFIG9"
            | b"MATRFIGB"
            | b"MATRFIGD"
            | b"MATRFIGF"
            | b"MATRFIGH"
            | b"MATRFIGJ"
            | b"MATRFIGL"
            | b"MATRFIGN"
            | b"MATRFIGP"
            | b"MATRFIGR"
            | b"MATRFIGT"
            | b"MATRFIGV"
            | b"MATRFIGX"
            | b"MATRFIGZ"
            | b"MATRFIGb"
    ) && metadata.series.iter().any(|s| s.line_width.is_some())
    {
        return Err("Explicit line widths require MFIG9".into());
    }
    if !matches!(
        version,
        b"MATRFIGH"
            | b"MATRFIGJ"
            | b"MATRFIGL"
            | b"MATRFIGN"
            | b"MATRFIGP"
            | b"MATRFIGR"
            | b"MATRFIGT"
            | b"MATRFIGV"
            | b"MATRFIGX"
            | b"MATRFIGZ"
            | b"MATRFIGb"
    ) && (metadata.placement.is_some() || metadata.legend_location.requires_extended_layout())
    {
        return Err("Extended legend/axes layout metadata requires MFIGH".into());
    }
    if !matches!(
        version,
        b"MATRFIGD"
            | b"MATRFIGF"
            | b"MATRFIGH"
            | b"MATRFIGJ"
            | b"MATRFIGL"
            | b"MATRFIGN"
            | b"MATRFIGP"
            | b"MATRFIGR"
            | b"MATRFIGT"
            | b"MATRFIGV"
            | b"MATRFIGX"
            | b"MATRFIGZ"
            | b"MATRFIGb"
    ) && !metadata.legend_location.is_default()
    {
        return Err("Legend location metadata requires MFIGD".into());
    }
    if !matches!(
        version,
        b"MATRFIGB"
            | b"MATRFIGD"
            | b"MATRFIGF"
            | b"MATRFIGH"
            | b"MATRFIGJ"
            | b"MATRFIGL"
            | b"MATRFIGN"
            | b"MATRFIGP"
            | b"MATRFIGR"
            | b"MATRFIGT"
            | b"MATRFIGV"
            | b"MATRFIGX"
            | b"MATRFIGZ"
            | b"MATRFIGb"
    ) && metadata
        .series
        .iter()
        .any(|s| s.marker_shape.is_some() || s.marker_size.is_some())
    {
        return Err("Marker style metadata requires MFIGB".into());
    }
    for s in &metadata.series {
        if let Some(size) = s.marker_size {
            crate::plot::validate_marker_size(size)?;
        }
        if let Some(width) = s.line_width {
            crate::plot::validate_line_width(width)?;
        }
    }
    if !matches!(
        version,
        b"MATRFIG5"
            | b"MATRFIG9"
            | b"MATRFIGB"
            | b"MATRFIGD"
            | b"MATRFIGF"
            | b"MATRFIGH"
            | b"MATRFIGJ"
            | b"MATRFIGL"
            | b"MATRFIGN"
            | b"MATRFIGP"
            | b"MATRFIGR"
            | b"MATRFIGT"
            | b"MATRFIGV"
            | b"MATRFIGX"
            | b"MATRFIGZ"
            | b"MATRFIGb"
    ) && metadata.series.iter().any(|s| {
        matches!(
            s.kind,
            crate::plot::PlotKind::Stairs | crate::plot::PlotKind::Stem
        )
    }) {
        return Err("Discrete plot kinds require MatRust figure version 5".into());
    }
    if metadata.series.len() > crate::plot::MAX_PLOT_SERIES || total != Some(metadata.count) {
        return Err("Invalid series count or point totals".into());
    }
    let (local_ledger, expected_series_payload) = native_series_payload(&metadata.series, compact)?;
    let series_count = metadata.series.len();
    let volume_count = metadata.volumes.len();
    if !extended
        && (metadata.data_children.is_some()
            || metadata
                .volumes
                .iter()
                .any(|v| v.exact_data || v.overlay.is_some()))
    {
        return Err("Extended volume metadata requires MFIGc".into());
    }
    let mut child_ids = match &metadata.data_children {
        Some(order) if volume_count > 0 => {
            native_axes_child_identities(order, series_count, volume_count)?
        }
        None if !extended || volume_count == 0 => Vec::new(),
        _ => return Err("Missing or unexpected native axes child order".into()),
    };
    let expected_volume_payload = native_volume_payload(&metadata.volumes)?;
    let expected_payload = expected_series_payload
        .checked_add(expected_volume_payload)
        .ok_or("Figure payload extent overflow")?;
    parent_ledger.unwrap_or_default().merge(local_ledger)?;
    let map_rows = metadata.colormap_payload_rows.unwrap_or(0);
    if map_rows > crate::plot::MAX_COLORMAP_ROWS
        || (metadata.colormap.is_some() && metadata.colormap_payload_rows.is_some())
        || (metadata.colormap_payload_rows.is_some()
            && !matches!(version, b"MATRFIGZ" | b"MATRFIGb"))
    {
        return Err("Invalid colormap payload".into());
    }
    let map_bytes = map_rows
        .checked_mul(24)
        .ok_or("Colormap payload overflow")?;
    let viewer_offset = (12 + size)
        .checked_add(expected_payload)
        .and_then(|n| n.checked_add(map_bytes))
        .ok_or("Figure payload extent overflow")?;
    if viewer_offset.checked_add(viewer_payload) != Some(bytes.len()) {
        return Err("Figure samples are truncated or trailing".into());
    }
    let cancel = std::sync::atomic::AtomicBool::new(false);
    if metadata.colormap_payload_rows.is_some() {
        let start = viewer_offset - map_bytes;
        metadata.colormap = Some(
            bytes[start..viewer_offset]
                .as_chunks::<24>()
                .0
                .iter()
                .map(|row| {
                    std::array::from_fn(|i| {
                        f64::from_le_bytes(row[i * 8..i * 8 + 8].try_into().unwrap())
                    })
                })
                .collect(),
        );
    }
    let mut offset = 12 + size;
    let mut series = Vec::new();
    let mut bar_groups: std::collections::BTreeMap<usize, crate::ObjectIdentity> =
        std::collections::BTreeMap::new();
    for (index, s) in metadata.series.into_iter().enumerate() {
        let (points, z, indexed_patch) = if let Some(metadata) = &s.indexed_patch {
            let mut input = &bytes[offset..];
            let mesh = metadata.decode(&s, &mut input)?;
            offset = bytes.len() - input.len();
            let (points, z) = mesh.expand(|| Ok(()))?;
            (points.into(), z.into(), Some(mesh))
        } else {
            let length = s
                .coordinates
                .as_ref()
                .map_or_else(|| Ok(s.count * 16), |d| d.preflight().map(|(_, n)| n))?;
            let end = offset + length;
            let z_end = if s.z { end + s.count * 8 } else { end };
            let points = if let Some(descriptor) = &s.coordinates {
                crate::plot::SeriesPoints::Patch(Arc::new(
                    descriptor.decode(&bytes[offset..end], &cancel)?,
                ))
            } else {
                let points = bytes[offset..end]
                    .as_chunks::<16>()
                    .0
                    .iter()
                    .map(|b| {
                        [
                            f64::from_le_bytes(b[..8].try_into().unwrap()),
                            f64::from_le_bytes(b[8..].try_into().unwrap()),
                        ]
                    })
                    .collect::<Vec<_>>();
                points.into()
            };
            let z: std::sync::Arc<[f64]> = bytes[end..z_end]
                .as_chunks::<8>()
                .0
                .iter()
                .map(|b| f64::from_le_bytes(*b))
                .collect();
            offset = z_end;
            (points, z, None)
        };
        let mut face_list = |count: Option<usize>| {
            count.map(|n| {
                let end = offset + 3 * n;
                let list: Arc<[[u8; 3]]> = bytes[offset..end]
                    .as_chunks::<3>()
                    .0
                    .iter()
                    .copied()
                    .collect();
                offset = end;
                list
            })
        };
        let face_colors = face_list(s.face_colors);
        let face_edge_colors = face_list(s.face_edge_colors);
        let marker_sizes: Option<Arc<[f64]>> = s.marker_sizes.then(|| {
            let end = offset + 8 * s.count;
            let list: Arc<[f64]> = bytes[offset..end]
                .as_chunks::<8>()
                .0
                .iter()
                .map(|b| f64::from_le_bytes(*b))
                .collect();
            offset = end;
            list
        });
        let scatter_cdata = s.scatter_cdata.map(|kind| match kind {
            ScatterCDataKind::Rgb => {
                let end = offset + 3 * s.count;
                let list: Arc<[[u8; 3]]> = bytes[offset..end]
                    .as_chunks::<3>()
                    .0
                    .iter()
                    .copied()
                    .collect();
                offset = end;
                crate::plot::ScatterCData::Rgb(list)
            }
            ScatterCDataKind::Indexed => {
                let end = offset + 8 * s.count;
                let list: Arc<[f64]> = bytes[offset..end]
                    .as_chunks::<8>()
                    .0
                    .iter()
                    .map(|b| f64::from_le_bytes(*b))
                    .collect();
                offset = end;
                crate::plot::ScatterCData::Indexed(list)
            }
        });
        let patch_vertex_cdata = match (s.patch_vertex_source_count, s.patch_vertex_corner_count) {
            (Some(sources), Some(corners)) => {
                let end = offset + 8 * sources;
                let source: Arc<[f64]> = bytes[offset..end]
                    .as_chunks::<8>()
                    .0
                    .iter()
                    .map(|b| f64::from_le_bytes(*b))
                    .collect();
                offset = end;
                let end = offset + 4 * corners;
                let corners: Arc<[u32]> = bytes[offset..end]
                    .as_chunks::<4>()
                    .0
                    .iter()
                    .map(|b| u32::from_le_bytes(*b))
                    .collect();
                offset = end;
                if !source.iter().all(|v| v.is_finite())
                    || !corners.iter().all(|&i| (i as usize) < source.len())
                {
                    return Err("Invalid native Patch vertex CData".into());
                }
                Some(crate::plot::VertexCData { source, corners })
            }
            _ => None,
        };
        let legend_hidden = match s.icon_display_style.as_deref() {
            None | Some("on") => false,
            Some("off") => true,
            Some(_) => return Err("Invalid figure series IconDisplayStyle".into()),
        };
        series.push(crate::plot::Series {
            auto_palette: s.auto_palette,
            identity: child_ids
                .get_mut(index)
                .and_then(Option::take)
                .unwrap_or_default(),
            baseline_identity: crate::ObjectIdentity::default(),
            bar_baseline_visible: s.bar_baseline_visible,
            points,
            face_edge_color: s.face_edge_color,
            patch: s.patch,
            indexed_patch,
            slice_contour_level: s.slice_contour_level,
            base_value: s.base_value,
            label: s.label,
            kind: s.kind,
            color: s.color,
            explicit_color: s.explicit_color,
            pattern: s.pattern,
            line_width: s.line_width,
            marker_shape: s.marker_shape,
            marker_size: s.marker_size,
            markers: s.markers,
            marker_filled: s.marker_filled,
            visible: s.visible,
            bar_width: s.bar_width,
            bar_offset: s.bar_offset,
            bar_layout: s.bar_layout,
            bar_relative_width: s.bar_relative_width,
            bar_group_width: s.bar_group_width,
            bar_group_id: s
                .bar_group_id
                .map(|id| bar_groups.entry(id).or_default().clone()),
            face_color_mode: s.face_color_mode,
            patch_cdata: s.patch_cdata,
            patch_vertex_cdata,
            z,
            legend_hidden,
            face_colors,
            face_edge_colors,
            face_alpha: s.face_alpha,
            face_color_none: s.face_color_none,
            edge_alpha: s.edge_alpha,
            // Display-only; never stored.
            face_styles: None,
            marker_sizes,
            scatter_cdata,
            marker_face_color: s.marker_face_color,
            marker_edge_color: s.marker_edge_color,
            marker_face_alpha: s.marker_face_alpha,
            marker_edge_alpha: s.marker_edge_alpha,
        });
    }
    let baselines = baseline_metadata::decode(metadata.baselines, &series)?;
    baseline_metadata::decode_latent(
        metadata.latent_baselines.as_deref(),
        &baselines,
        &mut series,
    );
    let mut volumes = Vec::with_capacity(volume_count);
    let mut volume_payload = &bytes[offset..offset + expected_volume_payload];
    for (index, vm) in metadata.volumes.into_iter().enumerate() {
        let mut vol = vm.decode(&mut volume_payload)?;
        if let Some(order) = child_ids
            .get_mut(series_count + index)
            .and_then(Option::take)
        {
            vol.stack_order = order;
        }
        volumes.push(vol);
    }
    infer_legacy_text_sizes(json, &mut metadata.texts)?;
    let mut figure = Figure {
        identity: crate::ObjectIdentity::default(),
        axes: crate::plot::Axes {
            palette_cycle: metadata.palette_cycle,
            colors: metadata.axes_colors,
            identity: crate::ObjectIdentity::default(),
            series,
            title: metadata.title,
            xlabel: metadata.xlabel,
            ylabel: metadata.ylabel,
            hold: metadata.hold,
            legend: metadata.legend,
            legend_state: None,
            baselines,
            legend_location: metadata.legend_location,
            placement: metadata.placement,
            xgrid: metadata.grid,
            ygrid: metadata.ygrid.unwrap_or(metadata.grid),
            xlim: metadata.xlim,
            ylim: metadata.ylim,
            xscale: metadata.xscale,
            yscale: metadata.yscale,
            ticks_hidden: false,
            view: metadata.view,
            zlim: metadata.zlim,
            zlabel: metadata.zlabel,
            zgrid: metadata.zgrid.unwrap_or(metadata.grid),
            box_outline: metadata.r#box.unwrap_or(true),
            minor_grid: metadata.minor_grid.unwrap_or_default(),
            frame_hidden: false,
            extras: crate::plot::AxesExtras {
                volumes,
                colorbar: metadata.colorbar,
                subtitle: metadata.subtitle,
                title_style: metadata.title_style,
                label_styles: metadata.label_styles.unwrap_or_default(),
                label_interpreters: metadata.label_interpreters.unwrap_or_default(),
                label_lines: metadata.label_lines.unwrap_or_default(),
                tick_label_interpreter: metadata.tick_label_interpreter.unwrap_or_default(),
                rulers: metadata.rulers.unwrap_or_default(),
                tick_dir: metadata.tick_dir,
                layer: metadata.layer.unwrap_or_default(),
                tick_length: metadata.tick_length,
                line_width: metadata.line_width,
                constant_lines: metadata.constant_lines,
                rectangles: metadata.rectangles,
                clim: metadata.clim,
                auto_clim: metadata.auto_clim,
                colormap: metadata.colormap.map(std::sync::Arc::new),
                texts: metadata.texts,
                color_order: metadata.color_order.map(std::sync::Arc::new),
                square_plot_box: metadata.square_plot_box,
                axis_off: metadata.axis_off,
                tag: metadata.axes_tag,
                ..Default::default()
            },
        },
        layout: None,
        textboxes: metadata.textboxes,
        annotations: metadata.annotations,
        name: metadata.name,
        color: metadata.color,
        appearance: metadata.appearance,
        invert_hardcopy: metadata.invert_hardcopy,
        alphamap: None,
        slice_viewers: Vec::new(),
        orthoslice_viewers: Vec::new(),
        position: metadata.position,
        units: metadata.units,
        number_title: metadata.number_title,
        visible: metadata.visible,
        resize: metadata.resize,
        window_state: metadata.window_state,
    };
    figure.axes.restore_legend_metadata(metadata.legend_state)?;
    if let Some(interpreter) = metadata.legend_interpreter {
        figure.axes.extras.legend_interpreter = interpreter;
    }
    let axes_id = figure.axes.identity.clone();
    slice_viewer_file::restore(
        metadata.slice_viewers,
        &bytes[viewer_offset..],
        &mut figure,
        &[axes_id],
    )?;
    figure.validate()?;
    Ok(figure)
}

fn infer_legacy_text_sizes(json: &[u8], texts: &mut [crate::plot::TextObject]) -> Result<()> {
    use crate::plot::TextObject;

    if texts
        .iter()
        .all(|t| t.font_size_explicit || t.font_size == TextObject::DEFAULT_FONT_SIZE)
    {
        return Ok(());
    }
    // A stored false flag keeps the historical scaling. Only a missing flag
    // on an older record needs inference from the stored point size.
    #[derive(Deserialize)]
    struct TextFlags {
        texts: Vec<TextFlag>,
    }
    #[derive(Deserialize)]
    struct TextFlag {
        font_size_explicit: Option<bool>,
    }
    let flags: TextFlags = serde_json::from_slice(json).map_err(|e| e.to_string())?;
    for (text, flag) in texts.iter_mut().zip(flags.texts) {
        if flag.font_size_explicit.is_none() {
            text.font_size_explicit = text.font_size != TextObject::DEFAULT_FONT_SIZE;
        }
    }
    Ok(())
}

mod baseline_metadata;
mod slice_viewer_file;
mod subplot_file;

#[cfg(test)]
mod cancellable_atomic_tests {
    use super::*;
    #[test]
    fn cancelled_chunks_sync_and_final_publish_preserve_destination_and_remove_owned_temp() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("saved.eps");
        let bytes = vec![b'x'; 65537];
        for stop in 1..=5 {
            std::fs::write(&path, b"original").unwrap();
            let mut calls = 0;
            let result = atomic_write_checked(&path, &bytes, || {
                calls += 1;
                if calls == stop {
                    Err("Cancelled".into())
                } else {
                    Ok(())
                }
            });
            assert!(result.is_err(), "checkpoint {stop}");
            assert_eq!(calls, stop);
            assert_eq!(std::fs::read(&path).unwrap(), b"original");
            assert_eq!(std::fs::read_dir(dir.path()).unwrap().count(), 1);
        }
        atomic_write_checked(&path, &bytes, || Ok(())).unwrap();
        assert_eq!(std::fs::read(&path).unwrap(), bytes);
    }
    #[test]
    fn checked_persist_refuses_publication_before_any_rename() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("saved.eps");
        std::fs::write(&path, b"old").unwrap();
        let mut staged = tempfile::NamedTempFile::new_in(dir.path()).unwrap();
        staged.write_all(b"new").unwrap();
        staged.as_file().sync_all().unwrap();
        assert!(
            persist_atomic_checked(
                staged,
                &path,
                |_| panic!("No retry expected"),
                || Err("Cancelled".into())
            )
            .is_err()
        );
        assert_eq!(std::fs::read(&path).unwrap(), b"old");
        assert_eq!(std::fs::read_dir(dir.path()).unwrap().count(), 1);
    }
}
