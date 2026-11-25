//! `dicomCollection`: fast, bounded, cancellable DICOM folder/file scanner
//! returning a series-grouped table. Stops at PixelData; never reads pixel payloads.
use super::custom_dict::Dictionary;
use super::parser::{self, Backing, Dataset, ElementValue, TransferSyntax, read_value};
use crate::datetime_array::{DEFAULT_FORMAT, DateTimeValue, TimeZone};
use crate::engine::{Engine, Execution, Value};
use crate::table_array::{TableArray, TableParts};
use crate::{Array, CellArray, DatetimeArray, Matrix, Result, StringArray};
use std::collections::BTreeMap;
use std::fs::{self, File};
use std::io::{Cursor, Read, Seek, SeekFrom};
use std::path::Path;
use std::sync::Arc;
use std::sync::atomic::AtomicBool;

pub const MAX_FILES: usize = 100_000;
const DICOMDIR_SOP_UID: &str = "1.2.840.10008.1.3.10";

const TAG_MEDIA_STORAGE_SOP_CLASS_UID: u32 = 0x0002_0002;
const TAG_SOP_CLASS_UID: u32 = 0x0008_0016;
const TAG_STUDY_DATE: u32 = 0x0008_0020;
const TAG_SERIES_DATE: u32 = 0x0008_0021;
const TAG_STUDY_TIME: u32 = 0x0008_0030;
const TAG_SERIES_TIME: u32 = 0x0008_0031;
const TAG_MODALITY: u32 = 0x0008_0060;
const TAG_STUDY_DESCRIPTION: u32 = 0x0008_1030;
const TAG_SERIES_DESCRIPTION: u32 = 0x0008_103E;
const TAG_PATIENT_NAME: u32 = 0x0010_0010;
const TAG_PATIENT_SEX: u32 = 0x0010_0040;
const TAG_STUDY_INSTANCE_UID: u32 = 0x0020_000D;
const TAG_SERIES_INSTANCE_UID: u32 = 0x0020_000E;
const TAG_INSTANCE_NUMBER: u32 = 0x0020_0013;
const TAG_SAMPLES_PER_PIXEL: u32 = 0x0028_0002;
const TAG_NUMBER_OF_FRAMES: u32 = 0x0028_0008;
const TAG_ROWS: u32 = 0x0028_0010;
const TAG_COLUMNS: u32 = 0x0028_0011;

const VAR_NAMES: [&str; 14] = [
    "StudyDateTime",
    "SeriesDateTime",
    "PatientName",
    "PatientSex",
    "Modality",
    "StudyDescription",
    "SeriesDescription",
    "StudyInstanceUID",
    "SeriesInstanceUID",
    "Rows",
    "Columns",
    "Channels",
    "Frames",
    "Filenames",
];

/// Rust API result returned by [`collect`].
#[derive(Clone, Debug)]
pub struct CollectionOutput {
    pub table: TableArray,
    pub skipped: usize,
    /// All walked regular files absent from Filenames, excluding DICOMDIR.
    pub uncollected: usize,
}

// A scoped Rust-only observation of the public collection call. No workspace
// variables, warning parsing, second walk, or MATLAB-visible outputs are needed.
thread_local! {
    static APP_COUNT: std::cell::RefCell<Option<Result<usize>>> = const { std::cell::RefCell::new(None) };
}

/// Observe the collection count from an app worker's script on this thread.
/// Nested scopes and unwinding restore the previous observer; nothing is retained
/// between jobs. A missing or failed scan carries an explicit unknown reason.
#[doc(hidden)]
pub fn with_dicom_collection_count<T>(run: impl FnOnce() -> T) -> (T, Result<usize>) {
    struct Restore(Option<Result<usize>>);
    impl Drop for Restore {
        fn drop(&mut self) {
            APP_COUNT.with(|slot| *slot.borrow_mut() = self.0.take());
        }
    }
    let previous = APP_COUNT.with(|slot| {
        slot.replace(Some(Err(
            "The collection scan did not report a count".into()
        )))
    });
    let _restore = Restore(previous);
    let result = run();
    let count = APP_COUNT.with(|slot| slot.borrow_mut().take().unwrap());
    (result, count)
}

#[derive(Clone, Debug)]
struct FileSummary {
    study_dt: DateTimeValue,
    series_dt: DateTimeValue,
    patient_name: String,
    patient_sex: String,
    modality: String,
    study_desc: String,
    series_desc: String,
    study_uid: String,
    series_uid: String,
    rows: f64,
    cols: f64,
    channels: f64,
    frames: f64,
    instance_number: i64,
    file_name: String,
    path_text: String,
}

/// Public Rust entry point for the engine and desktop DICOM Browser app.
/// Polls `engine.cancellation()` (i.e. `&engine.cancel`) between files and during parsing.
/// `header_limit` bounds retained header bytes, independently of pixel/file size.
pub fn collect(
    engine: &Engine,
    source: &Path,
    include_subfolders: bool,
    header_limit: u64,
) -> Result<CollectionOutput> {
    collect_limited(engine, source, include_subfolders, header_limit, MAX_FILES)
}

fn collect_limited(
    engine: &Engine,
    source: &Path,
    include_subfolders: bool,
    header_limit: u64,
    max_files: usize,
) -> Result<CollectionOutput> {
    let cancel = &engine.cancel;
    crate::dsp::check_cancel(cancel)?;
    let stat = fs::metadata(source).map_err(|e| format!("dicomCollection: {e}"))?;
    let mut budget = ScanBudget::new(engine)?;
    let mut index = BTreeMap::new();
    let mut groups = Vec::new();
    let mut skipped = 0usize;
    let mut ignored = 0usize;

    if stat.is_file() {
        if is_dicomdir_name(source) {
            return Err("dicomCollection: DICOMDIR input is not supported yet".into());
        }
        budget.file(source)?;
        match inspect_file(engine, source, header_limit, budget.remaining, cancel) {
            Ok(Some(summary)) => retain_summary(summary, &mut index, &mut groups),
            Ok(None) => ignored += 1,
            Err(e) => {
                if e.contains("DICOMDIR") || is_budget_error(&e) {
                    return Err(e);
                }
                crate::dsp::check_cancel(cancel)?;
                skipped += 1;
            }
        }
    } else if stat.is_dir() {
        let root = fs::canonicalize(source).map_err(|e| format!("dicomCollection: {e}"))?;
        budget.directory(&root)?;
        let mut stack = vec![root.clone()];
        let mut seen_dirs = std::collections::HashSet::from([root.clone()]);
        let mut visited = 0usize;
        while let Some(dir) = stack.pop() {
            crate::dsp::check_cancel(cancel)?;
            let entries = fs::read_dir(&dir).map_err(|e| format!("dicomCollection: {e}"))?;
            for entry in entries {
                crate::dsp::check_cancel(cancel)?;
                let entry = entry.map_err(|e| format!("dicomCollection: {e}"))?;
                let path = entry.path();
                let kind = entry
                    .file_type()
                    .map_err(|e| format!("dicomCollection: {e}"))?;
                if kind.is_dir() || (kind.is_symlink() && path.is_dir()) {
                    if include_subfolders {
                        let canonical =
                            fs::canonicalize(&path).map_err(|e| format!("dicomCollection: {e}"))?;
                        if canonical.starts_with(&root) && !seen_dirs.contains(&canonical) {
                            if seen_dirs.len() >= MAX_DIRECTORIES {
                                return Err(format!(
                                    "dicomCollection: source exceeds the {MAX_DIRECTORIES}-directory limit"
                                ));
                            }
                            budget.directory(&canonical)?;
                            seen_dirs.insert(canonical.clone());
                            stack.push(canonical);
                        }
                    }
                    continue;
                }
                if !(kind.is_file() || (kind.is_symlink() && path.is_file()))
                    || is_dicomdir_name(&path)
                {
                    continue;
                }
                visited += 1;
                if visited > max_files {
                    return Err(format!(
                        "dicomCollection: source exceeds the {max_files}-file limit"
                    ));
                }
                budget.file(&path)?;
                match inspect_file(engine, &path, header_limit, budget.remaining, cancel) {
                    Ok(Some(summary)) => retain_summary(summary, &mut index, &mut groups),
                    Ok(None) => ignored += 1,
                    Err(e) if is_budget_error(&e) => return Err(e),
                    // Media indexes inside folders are intentionally ignored.
                    Err(e) if e.contains("DICOMDIR") => {}
                    Err(_) => {
                        crate::dsp::check_cancel(cancel)?;
                        skipped += 1;
                    }
                }
            }
        }
    } else {
        return Err("dicomCollection: source must be a folder or regular file".into());
    }

    // Every candidate's reservation includes its table construction peak.
    let table = build_table(index, groups, cancel)?;
    Ok(CollectionOutput {
        table,
        skipped,
        uncollected: skipped + ignored,
    })
}

const MAX_DIRECTORIES: usize = 10_000;
const SCAN_FIXED_BYTES: usize = 32 * 1024;
const FILE_RESERVATION: usize = 8 * 1024;

struct ScanBudget {
    remaining: usize,
}

impl ScanBudget {
    fn new(engine: &Engine) -> Result<Self> {
        let remaining = super::super::limits::memory_bytes(&engine.limits)
            .saturating_sub(engine.workspace_bytes())
            .saturating_sub(engine.dicom_dictionary.memory_bytes());
        let mut budget = Self { remaining };
        budget.charge(SCAN_FIXED_BYTES)?;
        Ok(budget)
    }

    fn charge(&mut self, bytes: usize) -> Result<()> {
        self.remaining = self
            .remaining
            .checked_sub(bytes)
            .ok_or_else(scan_memory_error)?;
        Ok(())
    }

    fn file(&mut self, path: &Path) -> Result<()> {
        // 8 KiB covers all capped fields, FileSummary/UID index nodes, Vec
        // growth + sorting scratch, and simultaneous table columns/Arc copies.
        // 12x path bytes covers lossy UTF-8 expansion, filename, paths and
        // temporary/output copies. Keep skipped-file reservations too: this is
        // deliberately conservative whole-scan admission, never a fresh budget.
        self.charge(FILE_RESERVATION.saturating_add(path.as_os_str().len().saturating_mul(12)))
    }

    fn directory(&mut self, path: &Path) -> Result<()> {
        // Canonical root, pending paths, visited-set nodes/buckets and growth.
        self.charge(1024usize.saturating_add(path.as_os_str().len().saturating_mul(8)))
    }
}

fn scan_memory_error() -> String {
    "dicomCollection: whole-scan memory budget exhausted".into()
}

fn is_budget_error(error: &str) -> bool {
    error.contains("budget") || error.contains("unable to allocate")
}

type GroupIndex = BTreeMap<(String, String), usize>;

fn retain_summary(
    mut summary: FileSummary,
    index: &mut GroupIndex,
    groups: &mut Vec<Vec<FileSummary>>,
) {
    // Move unique UID keys into the index; summaries carry no cloned UID text.
    let key = (
        std::mem::take(&mut summary.study_uid),
        std::mem::take(&mut summary.series_uid),
    );
    let group = *index.entry(key).or_insert_with(|| {
        groups.push(Vec::new());
        groups.len() - 1
    });
    groups[group].push(summary);
}

pub(in crate::engine) fn run_parsed(
    engine: &mut Engine,
    path: &Path,
    include_subfolders: bool,
    out: &mut Execution,
    file_limit: u64,
) -> Result<Value> {
    let result = collect(engine, path, include_subfolders, file_limit);
    APP_COUNT.with(|slot| {
        if let Some(count) = slot.borrow_mut().as_mut() {
            *count = result
                .as_ref()
                .map(|scan| scan.uncollected)
                .map_err(Clone::clone);
        }
    });
    let result = result?;
    if result.skipped > 0 {
        let msg = format!(
            "Skipped {} DICOM file(s): headers could not be collected.",
            result.skipped
        );
        let warn_args = ["dicomCollection:skipped", "%s", msg.as_str()]
            .map(|s| Value::Text(s.into()))
            .to_vec();
        engine.call("warning", warn_args, out)?;
    }
    Ok(Value::Container(Array::Table(result.table)))
}

pub(in crate::engine) fn parse_logical_scalar(value: &Value) -> Result<bool> {
    match value {
        Value::Logical(m) if m.len() == 1 => Ok(m.data()[0] != 0),
        Value::Matrix(m) if m.len() == 1 => match m.data()[0] {
            0.0 => Ok(false),
            1.0 => Ok(true),
            _ => Err("dicomCollection: IncludeSubfolders must be a logical scalar".into()),
        },
        _ => Err("dicomCollection: IncludeSubfolders must be a logical scalar".into()),
    }
}

fn is_dicomdir_name(path: &Path) -> bool {
    path.file_name()
        .and_then(|n| n.to_str())
        .is_some_and(|n| n.eq_ignore_ascii_case("DICOMDIR"))
}

fn has_dicom_extension(path: &Path) -> bool {
    path.extension()
        .and_then(|e| e.to_str())
        .is_some_and(|e| e.eq_ignore_ascii_case("dcm") || e.eq_ignore_ascii_case("dicom"))
}

fn inspect_file(
    engine: &Engine,
    path: &Path,
    header_limit: u64,
    budget: usize,
    cancel: &AtomicBool,
) -> Result<Option<FileSummary>> {
    crate::dsp::check_cancel(cancel)?;
    let mut file = File::open(path).map_err(|e| e.to_string())?;
    // Always validate the opened target, including for directory file symlinks.
    let stat = file.metadata().map_err(|e| e.to_string())?;
    if !stat.is_file() {
        return Err("dicomCollection: source must be a regular file".into());
    }
    let file_len = stat.len();
    let mut magic = [0u8; 132];
    if file_len < 132 || file.read_exact(&mut magic).is_err() || &magic[128..132] != b"DICM" {
        if has_dicom_extension(path) {
            return Err("dicomCollection: invalid or truncated DICOM file".into());
        }
        return Ok(None);
    }
    let dictionary = &engine.dicom_dictionary;
    let envelope =
        parser::envelope_with_dictionary(&mut file, file_len, budget, cancel, dictionary)?;
    let backing = if envelope.deflated {
        Backing::Inflated
    } else {
        Backing::Original
    };
    let mut header_budget = budget
        .checked_sub(envelope.metadata_cost)
        .ok_or_else(scan_memory_error)?;
    if envelope.deflated {
        // zlib-rs state/window + compressed read-ahead; no inflated pixel Vec.
        header_budget = header_budget
            .checked_sub(512 * 1024)
            .ok_or_else(scan_memory_error)?;
    }
    // Bound both old/new prefix allocations during growth. The parser and tag
    // projection receive only the budget left after the retained prefix.
    let limit = (header_budget / 4)
        .min(parser::MAX_METADATA_BUDGET)
        .min(usize::try_from(header_limit).unwrap_or(usize::MAX));
    let header = if envelope.deflated {
        file.seek(SeekFrom::Start(envelope.dataset_start))
            .map_err(|e| e.to_string())?;
        let input = HeaderInput {
            reader: (&mut file).take(file_len - envelope.dataset_start),
            remaining: super::inflate_work_bytes(engine).min(parser::MAX_METADATA_BUDGET) as u64,
            cancel,
        };
        let mut decoder = flate2::read::DeflateDecoder::new(input);
        let header = header_prefix(
            &mut decoder,
            envelope.syntax,
            Vec::new(),
            limit,
            cancel,
            dictionary,
        )?;
        if decoder.total_in().saturating_add(decoder.total_out())
            > super::inflate_work_bytes(engine) as u64
        {
            return Err("dicomCollection: deflate exceeds the evaluation work budget".into());
        }
        header
    } else {
        file.seek(SeekFrom::Start(0)).map_err(|e| e.to_string())?;
        let mut prefix = Vec::new();
        prefix_read(
            &mut file,
            &mut prefix,
            usize::try_from(envelope.dataset_start).map_err(|_| header_memory_error())?,
            limit,
            cancel,
        )?;
        header_prefix(
            &mut file,
            envelope.syntax,
            prefix,
            limit,
            cancel,
            dictionary,
        )?
    };
    let available = budget
        .checked_sub(header.capacity())
        .ok_or_else(scan_memory_error)?;
    let header_len = header.len() as u64;
    let mut reader = Cursor::new(header);
    let parsed = parser::dataset_with_dictionary(
        &mut reader,
        envelope,
        backing,
        header_len,
        available,
        cancel,
        dictionary,
    )?;
    let meta_sop = read_text_tag(
        &mut file,
        &parsed.meta,
        Backing::Original,
        TAG_MEDIA_STORAGE_SOP_CLASS_UID,
        false,
        cancel,
    )?;
    let utf8 = dataset_utf8(&mut reader, &parsed.dataset, parsed.dataset_backing, cancel)?;
    let ds_sop = read_text_tag(
        &mut reader,
        &parsed.dataset,
        parsed.dataset_backing,
        TAG_SOP_CLASS_UID,
        utf8,
        cancel,
    )?;

    if meta_sop.as_deref() == Some(DICOMDIR_SOP_UID) || ds_sop.as_deref() == Some(DICOMDIR_SOP_UID)
    {
        return Err("dicomCollection: DICOMDIR input is not supported yet".into());
    }

    let study_date = read_text_tag(
        &mut reader,
        &parsed.dataset,
        parsed.dataset_backing,
        TAG_STUDY_DATE,
        utf8,
        cancel,
    )?;
    let study_time = read_text_tag(
        &mut reader,
        &parsed.dataset,
        parsed.dataset_backing,
        TAG_STUDY_TIME,
        utf8,
        cancel,
    )?;
    let series_date = read_text_tag(
        &mut reader,
        &parsed.dataset,
        parsed.dataset_backing,
        TAG_SERIES_DATE,
        utf8,
        cancel,
    )?;
    let series_time = read_text_tag(
        &mut reader,
        &parsed.dataset,
        parsed.dataset_backing,
        TAG_SERIES_TIME,
        utf8,
        cancel,
    )?;

    let patient_name = read_text_tag(
        &mut reader,
        &parsed.dataset,
        parsed.dataset_backing,
        TAG_PATIENT_NAME,
        utf8,
        cancel,
    )?
    .unwrap_or_default();
    let patient_sex = read_text_tag(
        &mut reader,
        &parsed.dataset,
        parsed.dataset_backing,
        TAG_PATIENT_SEX,
        utf8,
        cancel,
    )?
    .unwrap_or_default();
    let modality = read_text_tag(
        &mut reader,
        &parsed.dataset,
        parsed.dataset_backing,
        TAG_MODALITY,
        utf8,
        cancel,
    )?
    .unwrap_or_default();
    let study_desc = read_text_tag(
        &mut reader,
        &parsed.dataset,
        parsed.dataset_backing,
        TAG_STUDY_DESCRIPTION,
        utf8,
        cancel,
    )?
    .unwrap_or_default();
    let series_desc = read_text_tag(
        &mut reader,
        &parsed.dataset,
        parsed.dataset_backing,
        TAG_SERIES_DESCRIPTION,
        utf8,
        cancel,
    )?
    .unwrap_or_default();
    let study_uid = read_text_tag(
        &mut reader,
        &parsed.dataset,
        parsed.dataset_backing,
        TAG_STUDY_INSTANCE_UID,
        utf8,
        cancel,
    )?
    .unwrap_or_default();
    let series_uid = read_text_tag(
        &mut reader,
        &parsed.dataset,
        parsed.dataset_backing,
        TAG_SERIES_INSTANCE_UID,
        utf8,
        cancel,
    )?
    .unwrap_or_default();

    let rows = read_u16_tag(
        &mut reader,
        &parsed.dataset,
        parsed.dataset_backing,
        parsed.syntax,
        TAG_ROWS,
        cancel,
    )?
    .unwrap_or(0) as f64;
    let cols = read_u16_tag(
        &mut reader,
        &parsed.dataset,
        parsed.dataset_backing,
        parsed.syntax,
        TAG_COLUMNS,
        cancel,
    )?
    .unwrap_or(0) as f64;
    let channels = read_u16_tag(
        &mut reader,
        &parsed.dataset,
        parsed.dataset_backing,
        parsed.syntax,
        TAG_SAMPLES_PER_PIXEL,
        cancel,
    )?
    .unwrap_or(1) as f64;
    let frames = read_is_tag(
        &mut reader,
        &parsed.dataset,
        parsed.dataset_backing,
        TAG_NUMBER_OF_FRAMES,
        cancel,
    )?
    .unwrap_or(1)
    .max(1) as f64;
    let instance_number = read_is_tag(
        &mut reader,
        &parsed.dataset,
        parsed.dataset_backing,
        TAG_INSTANCE_NUMBER,
        cancel,
    )?
    .unwrap_or(0);

    let file_name = path
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_default();
    let path_text = super::super::env_files::envx_path_text(path);

    Ok(Some(FileSummary {
        study_dt: parse_dicom_datetime(study_date.as_deref(), study_time.as_deref()),
        series_dt: parse_dicom_datetime(series_date.as_deref(), series_time.as_deref()),
        patient_name,
        patient_sex,
        modality,
        study_desc,
        series_desc,
        study_uid,
        series_uid,
        rows,
        cols,
        channels,
        frames,
        instance_number,
        file_name,
        path_text,
    }))
}

struct HeaderInput<'a, R> {
    reader: R,
    remaining: u64,
    cancel: &'a AtomicBool,
}

impl<R: Read> Read for HeaderInput<'_, R> {
    fn read(&mut self, out: &mut [u8]) -> std::io::Result<usize> {
        crate::dsp::check_cancel(self.cancel).map_err(std::io::Error::other)?;
        if out.is_empty() {
            return Ok(0);
        }
        if self.remaining == 0 {
            return Err(std::io::Error::other(
                "dicomCollection: compressed header exceeds the memory/work budget",
            ));
        }
        let count = (out.len().min(32 * 1024) as u64).min(self.remaining) as usize;
        let read = self.reader.read(&mut out[..count])?;
        self.remaining -= read as u64;
        Ok(read)
    }
}

// Retain only the pre-pixel header. Read + Seek parser validation is still
// delegated to the shared parser; this framing pass never reads pixel values.
// A nested pixel attribute cannot be projected without its payload, so reject it.
fn header_prefix<R: Read>(
    reader: &mut R,
    syntax: TransferSyntax,
    mut bytes: Vec<u8>,
    limit: usize,
    cancel: &AtomicBool,
    dictionary: &Dictionary,
) -> Result<Vec<u8>> {
    let mut frames: Vec<(usize, u32)> = Vec::new();
    let mut nodes = 0usize;
    loop {
        crate::dsp::check_cancel(cancel)?;
        while frames.last().is_some_and(|&(end, _)| end == bytes.len()) {
            frames.pop();
        }
        if frames.last().is_some_and(|&(end, _)| end < bytes.len()) {
            return Err("dicomCollection: invalid sequence length".into());
        }
        let start = bytes.len();
        let mut tag_bytes = [0u8; 4];
        let count = reader
            .read(&mut tag_bytes[..1])
            .map_err(|e| e.to_string())?;
        if count == 0 {
            if frames.is_empty() {
                return Ok(bytes);
            }
            return Err("dicomCollection: truncated sequence".into());
        }
        reader
            .read_exact(&mut tag_bytes[1..])
            .map_err(|e| e.to_string())?;
        let tag = (u32::from(syntax.u16([tag_bytes[0], tag_bytes[1]])) << 16)
            | u32::from(syntax.u16([tag_bytes[2], tag_bytes[3]]));
        if matches!(tag, 0x7FE0_0010 | 0x7FE0_0008 | 0x7FE0_0009) {
            if frames.is_empty() {
                return Ok(bytes);
            }
            return Err("dicomCollection: nested pixel data is not supported yet".into());
        }
        nodes += 1;
        if nodes > parser::MAX_NODES {
            return Err("dicomCollection: element/item count exceeds 10000".into());
        }
        prefix_extend(&mut bytes, &tag_bytes, limit)?;
        prefix_read(reader, &mut bytes, 4, limit, cancel)?;
        let (vr, length) = if tag >> 16 == 0xFFFE || syntax == TransferSyntax::ImplicitLittle {
            (
                dictionary
                    .lookup(tag)
                    .map_or(*b"UN", |entry| entry.implicit_vr(tag, 0)),
                syntax.u32(bytes[start + 4..start + 8].try_into().unwrap()),
            )
        } else {
            let vr = [bytes[start + 4], bytes[start + 5]];
            let length = if matches!(
                &vr,
                b"OB"
                    | b"OD"
                    | b"OF"
                    | b"OL"
                    | b"OV"
                    | b"OW"
                    | b"SQ"
                    | b"SV"
                    | b"UC"
                    | b"UN"
                    | b"UR"
                    | b"UT"
                    | b"UV"
            ) {
                prefix_read(reader, &mut bytes, 4, limit, cancel)?;
                syntax.u32(bytes[start + 8..start + 12].try_into().unwrap())
            } else {
                u32::from(syntax.u16([bytes[start + 6], bytes[start + 7]]))
            };
            (vr, length)
        };
        if matches!(tag, 0xFFFE_E00D | 0xFFFE_E0DD) {
            if length != 0 || frames.pop() != Some((usize::MAX, tag)) {
                return Err("dicomCollection: mismatched sequence delimiter".into());
            }
        } else if vr == *b"SQ" || tag == 0xFFFE_E000 {
            if frames.len() >= 2 * parser::MAX_DEPTH {
                return Err("dicomCollection: sequence nesting exceeds 32 levels".into());
            }
            let end = if length == u32::MAX {
                usize::MAX
            } else {
                bytes
                    .len()
                    .checked_add(length as usize)
                    .filter(|&end| end <= limit)
                    .ok_or_else(header_memory_error)?
            };
            frames.push((
                end,
                if tag == 0xFFFE_E000 {
                    0xFFFE_E00D
                } else {
                    0xFFFE_E0DD
                },
            ));
        } else {
            if length == u32::MAX {
                return Err("dicomCollection: undefined non-sequence length".into());
            }
            if length as usize > text_cap(tag, vr) {
                return Err("dicomCollection: text exceeds the retained-field limit".into());
            }
            prefix_read(reader, &mut bytes, length as usize, limit, cancel)?;
        }
    }
}

fn header_memory_error() -> String {
    "dicomCollection: header exceeds memory budget or metadata limit".into()
}

fn prefix_extend(bytes: &mut Vec<u8>, extra: &[u8], limit: usize) -> Result<()> {
    prefix_reserve(bytes, extra.len(), limit)?;
    bytes.extend_from_slice(extra);
    Ok(())
}

fn prefix_reserve(bytes: &mut Vec<u8>, count: usize, limit: usize) -> Result<()> {
    let needed = bytes
        .len()
        .checked_add(count)
        .filter(|&n| n <= limit)
        .ok_or_else(header_memory_error)?;
    if needed > bytes.capacity() {
        let target = needed.max(bytes.capacity().saturating_mul(2)).min(limit);
        bytes
            .try_reserve_exact(target - bytes.len())
            .map_err(|_| header_memory_error())?;
        if bytes.capacity() > limit {
            return Err(header_memory_error());
        }
    }
    Ok(())
}

fn prefix_read<R: Read>(
    reader: &mut R,
    bytes: &mut Vec<u8>,
    count: usize,
    limit: usize,
    cancel: &AtomicBool,
) -> Result<()> {
    prefix_reserve(bytes, count, limit)?;
    let start = bytes.len();
    bytes.resize(start + count, 0);
    for chunk in bytes[start..].chunks_mut(32 * 1024) {
        crate::dsp::check_cancel(cancel)?;
        reader.read_exact(chunk).map_err(|e| e.to_string())?;
    }
    Ok(())
}

fn text_cap(tag: u32, vr: [u8; 2]) -> usize {
    let field_cap = match tag {
        TAG_MEDIA_STORAGE_SOP_CLASS_UID
        | TAG_SOP_CLASS_UID
        | TAG_STUDY_INSTANCE_UID
        | TAG_SERIES_INSTANCE_UID => 64,
        TAG_PATIENT_NAME => 64 * 5,
        TAG_PATIENT_SEX | TAG_MODALITY => 16,
        TAG_STUDY_DESCRIPTION | TAG_SERIES_DESCRIPTION => 1024,
        parser::SPECIFIC_CHARACTER_SET => 64,
        _ => return usize::MAX,
    };
    field_cap.min(match &vr {
        b"UI" | b"LO" => 64,
        b"PN" => 64 * 5,
        _ => usize::MAX,
    })
}

fn dataset_utf8<R: Read + Seek>(
    reader: &mut R,
    dataset: &Dataset,
    backing: Backing,
    cancel: &AtomicBool,
) -> Result<bool> {
    let Some(element) = dataset
        .elements
        .iter()
        .find(|e| e.tag == parser::SPECIFIC_CHARACTER_SET)
    else {
        return Ok(false);
    };
    let ElementValue::Primitive(range) = element.value else {
        return Err("dicomCollection: invalid SpecificCharacterSet".into());
    };
    let bytes = read_value(reader, range, backing, cancel)?;
    let charset = super::decode(&bytes, false, *b"CS")?;
    match charset.as_str() {
        "" | "ISO_IR 6" => Ok(false),
        "ISO_IR 192" => Ok(true),
        _ => Err("dicomCollection: unsupported DICOM character set".into()),
    }
}

fn read_text_tag<R: Read + Seek>(
    reader: &mut R,
    dataset: &Dataset,
    backing: Backing,
    tag: u32,
    utf8: bool,
    cancel: &AtomicBool,
) -> Result<Option<String>> {
    let Some(element) = dataset.elements.iter().find(|e| e.tag == tag) else {
        return Ok(None);
    };
    let ElementValue::Primitive(range) = element.value else {
        return Ok(None);
    };
    if range.length > text_cap(tag, element.vr) as u64 {
        return Err("dicomCollection: text exceeds the retained-field limit".into());
    }
    let bytes = read_value(reader, range, backing, cancel)?;
    let text = super::decode(&bytes, utf8, element.vr)?;
    if text.len() > text_cap(tag, element.vr) {
        return Err("dicomCollection: text exceeds the retained-field limit".into());
    }
    Ok(Some(text))
}

fn read_u16_tag<R: Read + Seek>(
    reader: &mut R,
    dataset: &Dataset,
    backing: Backing,
    syntax: TransferSyntax,
    tag: u32,
    cancel: &AtomicBool,
) -> Result<Option<u16>> {
    let Some(element) = dataset.elements.iter().find(|e| e.tag == tag) else {
        return Ok(None);
    };
    let ElementValue::Primitive(range) = element.value else {
        return Ok(None);
    };
    let bytes = read_value(reader, range, backing, cancel)?;
    if bytes.len() == 2 && matches!(&element.vr, b"US" | b"OW") {
        return Ok(Some(syntax.u16([bytes[0], bytes[1]])));
    }
    if bytes.len() == 2 && &element.vr == b"SS" {
        let v = syntax.u16([bytes[0], bytes[1]]) as i16;
        return Ok(u16::try_from(v).ok());
    }
    if &element.vr == b"IS"
        && let Ok(text) = super::decode(&bytes, false, *b"IS")
        && let Ok(v) = text.trim().parse::<u16>()
    {
        return Ok(Some(v));
    }
    Ok(None)
}

fn read_is_tag<R: Read + Seek>(
    reader: &mut R,
    dataset: &Dataset,
    backing: Backing,
    tag: u32,
    cancel: &AtomicBool,
) -> Result<Option<i64>> {
    let Some(element) = dataset.elements.iter().find(|e| e.tag == tag) else {
        return Ok(None);
    };
    let ElementValue::Primitive(range) = element.value else {
        return Ok(None);
    };
    let bytes = read_value(reader, range, backing, cancel)?;
    if let Ok(text) = super::decode(&bytes, false, element.vr)
        && let Ok(v) = text.trim().parse::<i64>()
    {
        return Ok(Some(v));
    }
    Ok(None)
}

fn parse_dicom_datetime(date_str: Option<&str>, time_str: Option<&str>) -> DateTimeValue {
    let Some(date) = date_str
        .map(|s| s.trim_end_matches(' '))
        .filter(|s| !s.is_empty())
    else {
        return DateTimeValue::NAT;
    };
    if date.len() != 8 || !date.bytes().all(|b| b.is_ascii_digit()) {
        return DateTimeValue::NAT;
    }
    let y = date[..4].parse::<i64>().unwrap();
    let m = date[4..6].parse::<usize>().unwrap();
    let d = date[6..].parse::<i64>().unwrap();
    let leap = y % 4 == 0 && (y % 100 != 0 || y % 400 == 0);
    let month_days = [
        0,
        31,
        if leap { 29 } else { 28 },
        31,
        30,
        31,
        30,
        31,
        31,
        30,
        31,
        30,
        31,
    ];
    if y == 0 || !(1..=12).contains(&m) || d < 1 || d > month_days[m] {
        return DateTimeValue::NAT;
    }
    let Some(nanos) = parse_dicom_time_nanos(time_str) else {
        return DateTimeValue::NAT;
    };
    DateTimeValue {
        days: super::super::dates::civil_serial(y, m, d),
        nanos,
    }
}

fn parse_dicom_time_nanos(time_str: Option<&str>) -> Option<i64> {
    let Some(raw) = time_str
        .map(|s| s.trim_end_matches(' '))
        .filter(|s| !s.is_empty())
    else {
        return Some(0);
    };
    let (main, frac) = raw.split_once('.').unwrap_or((raw, ""));
    if !matches!(main.len(), 2 | 4 | 6)
        || !main.bytes().all(|b| b.is_ascii_digit())
        || (raw.contains('.') && (main.len() != 6 || frac.is_empty() || frac.len() > 6))
        || !frac.bytes().all(|b| b.is_ascii_digit())
    {
        return None;
    }
    let hh = main[..2].parse::<i64>().ok()?;
    let mm = if main.len() >= 4 {
        main[2..4].parse::<i64>().ok()?
    } else {
        0
    };
    let ss = if main.len() == 6 {
        main[4..6].parse::<i64>().ok()?
    } else {
        0
    };
    // PS3.5 6.2 permits SS=60 only for a leap second. Fixed 86400-second
    // datetime days cannot preserve that value: return NaT rather than clamp.
    if hh > 23 || mm > 59 || ss > 59 {
        return None;
    }
    let frac_ns = if frac.is_empty() {
        0
    } else {
        frac.parse::<i64>().ok()? * 10i64.pow(9 - frac.len() as u32)
    };
    Some((hh * 3600 + mm * 60 + ss) * 1_000_000_000 + frac_ns)
}

fn build_table(
    index: GroupIndex,
    mut groups: Vec<Vec<FileSummary>>,
    cancel: &AtomicBool,
) -> Result<TableArray> {
    let n = index.len();
    let mut study_dts = Vec::with_capacity(n);
    let mut series_dts = Vec::with_capacity(n);
    let mut patient_names = Vec::with_capacity(n);
    let mut patient_sexes = Vec::with_capacity(n);
    let mut modalities = Vec::with_capacity(n);
    let mut study_descs = Vec::with_capacity(n);
    let mut series_descs = Vec::with_capacity(n);
    let mut study_uids = Vec::with_capacity(n);
    let mut series_uids = Vec::with_capacity(n);
    let mut rows_col = Vec::with_capacity(n);
    let mut cols_col = Vec::with_capacity(n);
    let mut channels_col = Vec::with_capacity(n);
    let mut frames_col = Vec::with_capacity(n);
    let mut filenames_cells = Vec::with_capacity(n);

    for ((study_uid, series_uid), group) in index {
        let mut items = std::mem::take(&mut groups[group]);
        crate::dsp::check_cancel(cancel)?;
        items.sort_by(|a, b| {
            (a.instance_number, &a.file_name, &a.path_text).cmp(&(
                b.instance_number,
                &b.file_name,
                &b.path_text,
            ))
        });
        let first = &items[0];
        let total_frames: f64 = items.iter().map(|it| it.frames).sum();

        study_dts.push(first.study_dt);
        series_dts.push(first.series_dt);
        patient_names.push(Some(Arc::from(first.patient_name.as_str())));
        patient_sexes.push(Some(Arc::from(first.patient_sex.as_str())));
        modalities.push(Some(Arc::from(first.modality.as_str())));
        study_descs.push(Some(Arc::from(first.study_desc.as_str())));
        series_descs.push(Some(Arc::from(first.series_desc.as_str())));
        study_uids.push(Some(Arc::from(study_uid.as_str())));
        series_uids.push(Some(Arc::from(series_uid.as_str())));
        rows_col.push(first.rows);
        cols_col.push(first.cols);
        channels_col.push(first.channels);
        frames_col.push(total_frames);

        let file_strings: Vec<Option<Arc<str>>> = items
            .into_iter()
            .map(|it| Some(Arc::from(it.path_text.as_str())))
            .collect();
        let file_arr = StringArray::new(file_strings.len(), 1, file_strings)?;
        filenames_cells.push(Arc::new(Array::Str(file_arr)));
    }

    let filenames_col = if n == 0 {
        Array::Cell(CellArray::empty(0, 1, cancel)?)
    } else {
        Array::Cell(CellArray::from_shared(n, 1, filenames_cells, cancel)?)
    };

    let columns = vec![
        Array::Datetime(DatetimeArray::new(
            n,
            1,
            study_dts,
            DEFAULT_FORMAT,
            TimeZone::Unzoned,
        )?),
        Array::Datetime(DatetimeArray::new(
            n,
            1,
            series_dts,
            DEFAULT_FORMAT,
            TimeZone::Unzoned,
        )?),
        Array::Str(StringArray::new(n, 1, patient_names)?),
        Array::Str(StringArray::new(n, 1, patient_sexes)?),
        Array::Str(StringArray::new(n, 1, modalities)?),
        Array::Str(StringArray::new(n, 1, study_descs)?),
        Array::Str(StringArray::new(n, 1, series_descs)?),
        Array::Str(StringArray::new(n, 1, study_uids)?),
        Array::Str(StringArray::new(n, 1, series_uids)?),
        Array::Numeric(Matrix::from_column_vec(n, 1, rows_col)?),
        Array::Numeric(Matrix::from_column_vec(n, 1, cols_col)?),
        Array::Numeric(Matrix::from_column_vec(n, 1, channels_col)?),
        Array::Numeric(Matrix::from_column_vec(n, 1, frames_col)?),
        filenames_col,
    ];

    let mut parts = TableParts::new(
        VAR_NAMES.iter().map(|&s| s.to_owned()).collect(),
        columns,
        n,
    );
    parts.row_names = Some((1..=n).map(|i| format!("s{i}")).collect());
    TableArray::new(parts)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn file_cap_errors_explicitly() {
        let dir = tempfile::tempdir().unwrap();
        for i in 0..6 {
            std::fs::write(dir.path().join(format!("f{i}.txt")), b"x").unwrap();
        }
        let engine = Engine::default();
        let err = collect_limited(&engine, dir.path(), true, 1024 * 1024, 5).unwrap_err();
        assert!(err.contains("5-file limit"), "unexpected err: {err}");
    }
}
