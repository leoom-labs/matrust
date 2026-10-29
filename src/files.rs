//! Bounded, atomic file exchange. Parsing finishes before callers replace state.
use crate::{Matrix, Result, plot::Figure};
use serde::{Deserialize, Serialize};
mod csv;
mod text_matrix;
pub use text_matrix::{read_matrix_text, write_matrix_text};
mod mat_typed;
#[cfg(feature = "mat-v73")]
mod mat_v73_complex;
#[cfg(feature = "mat-v73")]
mod mat_v73_write;
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
    let mut text = String::from_utf8(read_bounded(path, 1_000_000)?)
        .map_err(|_| "Script is not valid UTF-8".to_owned())?;
    let prefix = text.len() - text.trim_start_matches('\u{feff}').len();
    if prefix != 0 {
        text.drain(..prefix);
    }
    Ok(text)
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

    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    textboxes: Vec<crate::plot::TextBox>,
    title: String,
    xlabel: String,
    ylabel: String,
    count: usize,
    #[serde(default)]
    series: Vec<SeriesMetadata>,
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
    #[serde(default)]
    xlim: Option<[f64; 2]>,
    #[serde(default)]
    ylim: Option<[f64; 2]>,
    #[serde(default)]
    xscale: crate::plot::AxisScale,
    #[serde(default)]
    yscale: crate::plot::AxisScale,
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
}
fn bar_visibility_off(value: &bool) -> bool {
    !*value
}
pub fn save_figure(path: &Path, figure: &Figure) -> Result<()> {
    if path
        .extension()
        .is_some_and(|s| s.eq_ignore_ascii_case("fig"))
    {
        return save_matlab_figure(path, figure);
    }
    atomic_write(path, &encode_native_figure(figure)?)
}
pub(crate) fn encode_native_figure(figure: &Figure) -> Result<Vec<u8>> {
    figure.validate()?;
    if figure.layout.is_some() {
        return subplot_file::encode(figure);
    }
    let metadata = FigureMetadata {
        palette_cycle: figure.axes.palette_cycle,
        axes_colors: figure.axes.colors,
        appearance: figure.appearance,
        invert_hardcopy: figure.invert_hardcopy,
        latent_baselines: baseline_metadata::encode_latent(&figure.axes),
        baselines: baseline_metadata::encode(&figure.axes)?,
        name: figure.name.clone(),
        color: figure.color,
        textboxes: figure.textboxes.clone(),
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
            })
            .collect(),
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
    };
    let json = serde_json::to_vec(&metadata).map_err(|e| e.to_string())?;
    let compact = metadata.series.iter().any(|s| s.coordinates.is_some());
    let metadata_limit = if compact {
        65536 + crate::plot::MAX_SERIES * 4096
    } else {
        65536
    };
    if json.len() > metadata_limit {
        return Err("Figure exceeds save metadata limit".into());
    }
    let (_, payload) = native_series_payload(&metadata.series, compact)?;
    let mut bytes = Vec::with_capacity(12 + json.len() + payload);
    bytes.extend_from_slice(if figure.has_static_style_metadata() {
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
        if item.coordinates.is_some() {
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
        }
    }
    Ok(bytes)
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
        let (cost, bytes) = if let Some(descriptor) = &item.coordinates {
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
                if item.kind == PlotKind::Patch {
                    return Err("Native Patch topology missing".into());
                }
                GeometryCost::ordinary_allocation(item.count, item.kind == PlotKind::Bar)?
            };
            (
                cost,
                item.count
                    .checked_mul(16)
                    .ok_or("Native sample extent overflow")?,
            )
        };
        ledger.add(cost)?;
        payload = payload
            .checked_add(bytes)
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
    let bytes = read_bounded(path, 17_000_000)?;
    decode_native_figure(&bytes)
}
pub(crate) fn decode_native_figure(bytes: &[u8]) -> Result<Figure> {
    decode_native_figure_with_ledger(bytes, crate::plot::coordinates::GeometryLedger::default())
}
fn decode_native_figure_with_ledger(
    bytes: &[u8],
    mut prefix: crate::plot::coordinates::GeometryLedger,
) -> Result<Figure> {
    if bytes.starts_with(b"MATRFIG6")
        || bytes.starts_with(b"MATRFIG7")
        || bytes.starts_with(b"MATRFIG8")
        || bytes.starts_with(b"MATRFIGA")
        || bytes.starts_with(b"MATRFIGC")
        || bytes.starts_with(b"MATRFIGE")
        || bytes.starts_with(b"MATRFIGG")
        || bytes.starts_with(b"MATRFIGI")
        || bytes.starts_with(b"MATRFIGK")
        || bytes.starts_with(b"MATRFIGO")
        || bytes.starts_with(b"MATRFIGM")
        || bytes.starts_with(b"MATRFIGQ")
        || bytes.starts_with(b"MATRFIGS")
        || bytes.starts_with(b"MATRFIGU")
        || bytes.starts_with(b"MATRFIGW")
        || bytes.starts_with(b"MATRFIGY")
    {
        return subplot_file::decode(bytes);
    }
    if bytes.len() < 12
        || !matches!(
            &bytes[..8],
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
        )
    {
        return Err("Not a supported MatRust figure (.mrfig)".into());
    }
    let size = u32::from_le_bytes(bytes[8..12].try_into().unwrap()) as usize;
    let compact = matches!(
        &bytes[..8],
        b"MATRFIGN" | b"MATRFIGP" | b"MATRFIGR" | b"MATRFIGT" | b"MATRFIGV" | b"MATRFIGX"
    );
    let metadata_limit = if compact {
        65536 + crate::plot::MAX_SERIES * 4096
    } else {
        65536
    };
    if size > metadata_limit || size > bytes.len() - 12 {
        return Err("Invalid figure metadata size".into());
    }
    let mut metadata: FigureMetadata =
        serde_json::from_slice(&bytes[12..12 + size]).map_err(|e| e.to_string())?;
    let palette_metadata = metadata.palette_cycle.is_some()
        || metadata
            .appearance
            .is_some_and(|a| a.series_palette.is_some())
        || metadata.series.iter().any(|s| s.auto_palette.is_some());
    if (metadata.invert_hardcopy.is_some()
        || metadata.axes_colors.is_some_and(|c| c.legend.is_some()))
        && !bytes.starts_with(b"MATRFIGX")
    {
        return Err("Legend RGB and print preference require MFIGX".into());
    }
    if palette_metadata && !matches!(&bytes[..8], b"MATRFIGV" | b"MATRFIGX") {
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
        && !matches!(&bytes[..8], b"MATRFIGT" | b"MATRFIGV" | b"MATRFIGX")
    {
        return Err("Axes RGB metadata requires MFIGT".into());
    }
    if let Some(colors) = metadata.axes_colors {
        colors.validate()?;
    }
    if metadata.appearance.is_some()
        && !matches!(
            &bytes[..8],
            b"MATRFIGR" | b"MATRFIGT" | b"MATRFIGV" | b"MATRFIGX"
        )
    {
        return Err("Figure appearance intent requires MFIGR".into());
    }
    if metadata.series.len() > crate::plot::MAX_SERIES
        || metadata.count > crate::plot::coordinates::MAX_EXPANDED_VERTICES
    {
        return Err("Figure samples exceed limits".into());
    }
    if !compact && metadata.series.iter().any(|s| s.coordinates.is_some()) {
        return Err("Compact coordinates require MFIGN".into());
    }
    if !matches!(
        &bytes[..8],
        b"MATRFIGJ"
            | b"MATRFIGL"
            | b"MATRFIGN"
            | b"MATRFIGP"
            | b"MATRFIGR"
            | b"MATRFIGT"
            | b"MATRFIGV"
            | b"MATRFIGX"
    ) && (metadata.legend_state.is_some()
        || metadata.ygrid.is_some()
        || !metadata.name.is_empty()
        || metadata.color != [255; 3]
        || !metadata.textboxes.is_empty()
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
        &bytes[..8],
        b"MATRFIGL"
            | b"MATRFIGN"
            | b"MATRFIGP"
            | b"MATRFIGR"
            | b"MATRFIGT"
            | b"MATRFIGV"
            | b"MATRFIGX"
    ) {
        return Err("Independent face outlines require MFIGL".into());
    }
    if (metadata.latent_baselines.is_some()
        || !metadata.baselines.is_empty()
        || metadata.series.iter().any(|s| s.bar_baseline_visible))
        && !matches!(
            &bytes[..8],
            b"MATRFIGP" | b"MATRFIGR" | b"MATRFIGT" | b"MATRFIGV" | b"MATRFIGX"
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
        if state.members.len() > crate::plot::MAX_SERIES
            || state.members.iter().any(|&i| i >= metadata.series.len())
        {
            return Err("Invalid native legend membership extent".into());
        }
        for (i, index) in state.members.iter().enumerate() {
            if state.members[..i].contains(index) {
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
            (crate::plot::PlotKind::Patch, None) | (_, Some(_)) => {
                return Err("Patch geometry and series kind disagree".into());
            }
            (_, None) => {}
        }
    }
    if &bytes[..8] == b"MATRFIG1" {
        if !metadata.series.is_empty() {
            return Err("Unexpected series metadata in legacy figure".into());
        }
        metadata.series.push(SeriesMetadata {
            auto_palette: None,
            bar_baseline_visible: false,
            coordinates: None,
            face_edge_color: None,
            patch: None,
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
        });
    }
    let total = metadata
        .series
        .iter()
        .try_fold(0usize, |n, s| n.checked_add(s.count));
    if !matches!(
        &bytes[..8],
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
    ) && metadata
        .series
        .iter()
        .any(|s| s.pattern == crate::plot::LinePattern::None)
    {
        return Err("LineStyle none metadata requires MFIGF".into());
    }
    if !matches!(
        &bytes[..8],
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
    ) && metadata.series.iter().any(|s| s.line_width.is_some())
    {
        return Err("Explicit line widths require MFIG9".into());
    }
    if !matches!(
        &bytes[..8],
        b"MATRFIGH"
            | b"MATRFIGJ"
            | b"MATRFIGL"
            | b"MATRFIGN"
            | b"MATRFIGP"
            | b"MATRFIGR"
            | b"MATRFIGT"
            | b"MATRFIGV"
            | b"MATRFIGX"
    ) && (metadata.placement.is_some() || metadata.legend_location.requires_extended_layout())
    {
        return Err("Extended legend/axes layout metadata requires MFIGH".into());
    }
    if !matches!(
        &bytes[..8],
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
    ) && !metadata.legend_location.is_default()
    {
        return Err("Legend location metadata requires MFIGD".into());
    }
    if !matches!(
        &bytes[..8],
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
        &bytes[..8],
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
    ) && metadata.series.iter().any(|s| {
        matches!(
            s.kind,
            crate::plot::PlotKind::Stairs | crate::plot::PlotKind::Stem
        )
    }) {
        return Err("Discrete plot kinds require MatRust figure version 5".into());
    }
    if metadata.series.len() > crate::plot::MAX_SERIES || total != Some(metadata.count) {
        return Err("Invalid series count or point totals".into());
    }
    let (local_ledger, expected_payload) = native_series_payload(&metadata.series, compact)?;
    prefix.merge(local_ledger)?;
    if bytes.len() - 12 - size != expected_payload {
        return Err("Figure samples are truncated or trailing".into());
    }
    let cancel = std::sync::atomic::AtomicBool::new(false);
    let mut offset = 12 + size;
    let mut series = Vec::new();
    for s in metadata.series {
        let length = s
            .coordinates
            .as_ref()
            .map_or_else(|| Ok(s.count * 16), |d| d.preflight().map(|(_, n)| n))?;
        let end = offset + length;
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
        offset = end;
        series.push(crate::plot::Series {
            auto_palette: s.auto_palette,
            identity: crate::ObjectIdentity::default(),
            baseline_identity: crate::ObjectIdentity::default(),
            bar_baseline_visible: s.bar_baseline_visible,
            points,
            face_edge_color: s.face_edge_color,
            patch: s.patch,
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
        });
    }
    let baselines = baseline_metadata::decode(metadata.baselines, &series)?;
    baseline_metadata::decode_latent(
        metadata.latent_baselines.as_deref(),
        &baselines,
        &mut series,
    );
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
        },
        layout: None,
        textboxes: metadata.textboxes,
        name: metadata.name,
        color: metadata.color,
        appearance: metadata.appearance,
        invert_hardcopy: metadata.invert_hardcopy,
    };
    figure.axes.restore_legend_metadata(metadata.legend_state)?;
    figure.validate()?;
    Ok(figure)
}

mod baseline_metadata;
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
