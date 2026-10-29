//! Atomic figure checkpoints. The UI passes shared snapshots; a recovery writer
//! encodes new data and publishes the JSON manifest only after its files exist.
use super::{MAX_SESSION_BYTES, Session};
use crate::{Result, files, plot::Figure};
use serde::{Deserialize, Deserializer, Serialize};
use std::{
    collections::{BTreeMap, BTreeSet},
    io::Write,
    path::{Path, PathBuf},
    sync::atomic::{AtomicBool, Ordering},
};

const MAX_FIGURE_BYTES: usize = 17_000_000;
const MAX_GENERATION_BYTES: u64 = 256 * 1024 * 1024;

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct FigureView {
    pub bounds: [f64; 4],
    pub auto: [bool; 2],
    pub dirty: bool,
}
impl FigureView {
    fn validate(&self, figure: &Figure) -> Result<()> {
        for (axis, scale) in [figure.xscale, figure.yscale].into_iter().enumerate() {
            if !self.auto[axis] {
                scale.validate_limits([self.bounds[axis * 2], self.bounds[axis * 2 + 1]])?;
            }
        }
        if self.bounds.iter().any(|v| !v.is_finite()) {
            return Err("Invalid recovered figure view".into());
        }
        Ok(())
    }
}
#[derive(Clone, Debug)]
pub struct FigureDocument {
    /// Native presentation metadata; does not change figure appearance or identity.
    pub window: Option<super::figure_window::FigureWindow>,
    pub id: u32,
    pub figure: Figure,
    pub path: Option<PathBuf>,
    pub source_version: Option<files::figure_version::Version>,
    pub dirty: bool,
    pub view: Option<FigureView>,
}
#[derive(Clone, Debug, Default)]
pub struct FigureSession {
    pub current: u32,
    pub figures: Vec<FigureDocument>,
}
impl FigureSession {
    pub fn validate(&self) -> Result<()> {
        if self.figures.len() > 16 {
            return Err("Figure recovery exceeds 16 figures".into());
        }
        let mut ids = BTreeSet::new();
        for doc in &self.figures {
            if !(1..=1_000_000).contains(&doc.id) || !ids.insert(doc.id) {
                return Err("Invalid or duplicate recovery figure number".into());
            }
            doc.figure.validate()?;
            if let Some(view) = &doc.view {
                view.validate(&doc.figure)?;
            }
            if doc.path.as_ref().is_some_and(|p| !p.is_absolute()) {
                return Err("Recovered figure paths must be absolute".into());
            }
        }
        if !ids.is_empty() && !ids.contains(&self.current) {
            return Err("Missing active recovery figure".into());
        }
        Ok(())
    }
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct FigureManifest {
    pub version: u32,
    pub current: u32,
    pub entries: Vec<FigureReference>,
    #[serde(skip)]
    pub error: Option<String>,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct FigureReference {
    #[serde(
        default,
        skip_serializing_if = "super::figure_window::WindowRecovery::is_absent"
    )]
    pub window: super::figure_window::WindowRecovery,
    pub id: u32,
    pub file: String,
    pub bytes: u64,
    pub crc32: u32,
    pub path: Option<PathBuf>,
    #[serde(default)]
    pub source_version: Option<files::figure_version::Version>,
    pub dirty: bool,
    pub view: Option<FigureView>,
}
pub(super) fn recover_manifest<'de, D: Deserializer<'de>>(
    d: D,
) -> std::result::Result<Option<FigureManifest>, D::Error> {
    let value = serde_json::Value::deserialize(d)?;
    if value.is_null() {
        return Ok(None);
    }
    Ok(Some(serde_json::from_value(value).unwrap_or_else(|e| {
        FigureManifest {
            version: 1,
            current: 1,
            entries: vec![],
            error: Some(format!("Cannot read figure recovery manifest: {e}")),
        }
    })))
}
/// Recovery writes `figure-<16 alphanumerics>.mrfig`; older sessions used `.mfig`.
pub(crate) fn owned_name(name: &str) -> bool {
    name.strip_prefix("figure-")
        .and_then(|n| n.strip_suffix(".mrfig").or_else(|| n.strip_suffix(".mfig")))
        .is_some_and(|token| token.len() == 16 && token.bytes().all(|b| b.is_ascii_alphanumeric()))
}
pub fn figure_directory(session: &Path) -> Result<PathBuf> {
    let path = std::path::absolute(session).map_err(|e| e.to_string())?;
    let name = path
        .file_name()
        .ok_or("Invalid session filename")?
        .to_string_lossy();
    Ok(path.with_file_name(format!("{name}.figures")))
}
fn checked_directory(session: &Path, create: bool) -> Result<PathBuf> {
    let dir = figure_directory(session)?;
    if create {
        std::fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
    }
    let parent = dir
        .parent()
        .ok_or("Invalid recovery directory")?
        .canonicalize()
        .map_err(|e| e.to_string())?;
    let actual = dir
        .canonicalize()
        .map_err(|e| format!("Cannot open figure recovery: {e}"))?;
    if actual != parent.join(dir.file_name().unwrap()) {
        return Err("Figure recovery directory is redirected".into());
    }
    Ok(actual)
}
fn crc32(bytes: &[u8]) -> u32 {
    let mut crc = flate2::Crc::new();
    crc.update(bytes);
    crc.sum()
}
pub struct FigureRestoration {
    /// Nonblocking optional layout resets; never disable data autosave.
    pub placement_notes: Vec<String>,
    pub figures: FigureSession,
    pub warnings: Vec<String>,
    bindings: super::workspace::FigureBindings,
}
impl FigureRestoration {
    pub fn empty() -> Self {
        Self {
            figures: FigureSession::default(),
            warnings: vec![],
            placement_notes: vec![],
            bindings: super::workspace::FigureBindings::empty(),
        }
    }
    pub fn workspace_bindings(&self) -> &super::workspace::FigureBindings {
        &self.bindings
    }
}
pub fn restore_figures(
    path: &Path,
    manifest: &FigureManifest,
    cancel: &AtomicBool,
) -> FigureRestoration {
    let mut result = FigureRestoration {
        figures: FigureSession::default(),
        warnings: vec![],
        placement_notes: vec![],
        bindings: Default::default(),
    };
    if let Some(error) = &manifest.error {
        result.warnings.push(error.clone());
        return result;
    }
    if manifest.version != 1
        || manifest.entries.len() > 16
        || manifest
            .entries
            .iter()
            .map(|e| e.bytes)
            .try_fold(0u64, u64::checked_add)
            .is_none_or(|n| n > MAX_GENERATION_BYTES)
    {
        result
            .warnings
            .push("Invalid or oversized figure recovery manifest".into());
        return result;
    }
    if manifest.entries.is_empty() {
        result.figures.current = 1;
        result.bindings = super::workspace::FigureBindings::empty();
        return result;
    }
    let dir = match checked_directory(path, false) {
        Ok(p) => p,
        Err(e) => {
            result.warnings.push(e);
            // Entire payload directory unavailable: retain typed, detached
            // handles using the same validated checkpoint manifest.
            match super::workspace::FigureBindings::recovered(&result.figures, manifest, cancel) {
                Ok(bindings) => result.bindings = bindings,
                Err(error) => result.warnings.push(format!("Graphics bindings: {error}")),
            }
            return result;
        }
    };
    let mut ids = BTreeSet::new();
    for entry in &manifest.entries {
        if cancel.load(Ordering::Relaxed) {
            result.warnings.push("Figure recovery cancelled".into());
            break;
        }
        let read = || -> Result<FigureDocument> {
            if !(1..=1_000_000).contains(&entry.id)
                || !owned_name(&entry.file)
                || entry.bytes > MAX_FIGURE_BYTES as u64
            {
                return Err("Invalid figure recovery reference".into());
            }
            let file = dir.join(&entry.file);
            if !std::fs::symlink_metadata(&file)
                .map_err(|e| e.to_string())?
                .file_type()
                .is_file()
            {
                return Err("Figure recovery reference is not a regular file".into());
            }
            let bytes = files::read_bounded(&file, MAX_FIGURE_BYTES)?;
            if bytes.len() as u64 != entry.bytes || crc32(&bytes) != entry.crc32 {
                return Err("Figure recovery checksum or length mismatch".into());
            }
            let figure = files::decode_native_figure(&bytes)?;
            if let Some(v) = &entry.view {
                v.validate(&figure)?;
            }
            if entry.path.as_ref().is_some_and(|p| !p.is_absolute()) {
                return Err("Relative saved-file identity in recovery".into());
            }
            Ok(FigureDocument {
                window: entry.window.as_ref().cloned(),
                id: entry.id,
                figure,
                path: entry.path.clone(),
                source_version: entry.source_version,
                dirty: entry.dirty,
                view: entry.view.clone(),
            })
        };
        if !ids.insert(entry.id) {
            result
                .warnings
                .push(format!("Duplicate recovery figure {}", entry.id));
            continue;
        }
        match read() {
            Ok(doc) => {
                if entry.window.was_reset() {
                    result
                        .placement_notes
                        .push(format!("Figure {} window placement was reset", entry.id));
                }
                result.figures.figures.push(doc);
            }
            Err(e) => result.warnings.push(format!("Figure {}: {e}", entry.id)),
        }
    }
    result.figures.current = if result
        .figures
        .figures
        .iter()
        .any(|f| f.id == manifest.current)
    {
        manifest.current
    } else {
        result.figures.figures.first().map_or(1, |f| f.id)
    };
    if !manifest.entries.is_empty() && !manifest.entries.iter().any(|e| e.id == manifest.current) {
        result
            .warnings
            .push("Active recovery figure is missing".into());
    }
    match super::workspace::FigureBindings::recovered(&result.figures, manifest, cancel) {
        Ok(bindings) => result.bindings = bindings,
        Err(error) => result.warnings.push(format!("Graphics bindings: {error}")),
    }
    result
}

struct CachedFigure {
    stamp: crate::plot::SnapshotStamp,
    reference: FigureReference,
}
impl CachedFigure {
    fn new(doc: &FigureDocument, reference: FigureReference) -> Self {
        Self {
            stamp: crate::plot::SnapshotStamp::new(&doc.figure),
            reference,
        }
    }
    fn matches(&self, figure: &Figure) -> bool {
        self.stamp.matches(figure)
    }
}
struct StagedFiles {
    paths: Vec<PathBuf>,
    committed: bool,
}
impl Drop for StagedFiles {
    fn drop(&mut self) {
        if !self.committed {
            for p in &self.paths {
                let _ = std::fs::remove_file(p);
            }
        }
    }
}
struct PendingFigures {
    files: StagedFiles,
    manifest: FigureManifest,
    cache: BTreeMap<u32, CachedFigure>,
}
/// Long-lived writer cache uses weak sample references, so replacing a plot
/// does not retain its old data until the next recovery checkpoint.
pub struct SessionStore {
    workspace: super::workspace::Store,
    path: PathBuf,
    cache: BTreeMap<u32, CachedFigure>,
    committed: BTreeSet<String>,
    previous: BTreeSet<String>,
}
impl SessionStore {
    pub fn new(path: &Path) -> Self {
        #[derive(Deserialize)]
        struct Index {
            #[serde(default, deserialize_with = "recover_manifest")]
            figure_manifest: Option<FigureManifest>,
        }
        let committed = files::read_bounded(path, MAX_SESSION_BYTES)
            .ok()
            .and_then(|b| serde_json::from_slice::<Index>(&b).ok())
            .and_then(|i| i.figure_manifest)
            .filter(|m| m.error.is_none() && m.entries.len() <= 16)
            .map(|m| m.entries.into_iter().map(|e| e.file).collect())
            .unwrap_or_default();
        Self {
            workspace: super::workspace::Store::new(path),
            path: path.to_owned(),
            cache: BTreeMap::new(),
            committed,
            previous: BTreeSet::new(),
        }
    }
    pub fn save(&mut self, session: Session) -> Result<()> {
        self.save_with_cancel(session, &AtomicBool::new(false))
    }
    /// Stage a complete generation privately; cancellation before the JSON commit preserves the old checkpoint.
    pub fn save_with_cancel(&mut self, mut session: Session, cancel: &AtomicBool) -> Result<()> {
        crate::dsp::check_cancel(cancel)?;
        session.validate()?;
        let figures = session.figures.take();
        let staged_figures = figures
            .as_ref()
            .map(|f| self.stage_figures(f, cancel))
            .transpose()?;
        let context = figures
            .as_ref()
            .zip(staged_figures.as_ref().map(|p| &p.manifest));
        // Struct values cannot be recovered yet; they must not block the checkpoint.
        // Their names travel in the manifest so restoring can say what was left out.
        let pending = session
            .workspace
            .take()
            .map(|mut values| {
                let mut skipped = Vec::new();
                values.retain(|name, value| {
                    let keep = !value.contains_struct();
                    if !keep {
                        skipped.push(name.clone());
                    }
                    keep
                });
                self.workspace
                    .stage_with_figures(values, context, cancel)
                    .map(|mut pending| {
                        pending.manifest.skipped = skipped;
                        pending
                    })
            })
            .transpose()?;
        if let Some(pending) = &pending {
            session.workspace_manifest = Some(pending.manifest.clone());
        }
        if let Some(pending) = &staged_figures {
            session.figure_manifest = Some(pending.manifest.clone());
        }
        // One publication point: both private payload sets exist and validate.
        session.save_json(&self.path, cancel)?;
        if let Some(mut pending) = staged_figures {
            pending.files.committed = true;
            self.previous = std::mem::replace(
                &mut self.committed,
                pending
                    .manifest
                    .entries
                    .iter()
                    .map(|r| r.file.clone())
                    .collect(),
            );
            self.cache = pending.cache;
            if let Ok(dir) = checked_directory(&self.path, false) {
                let _ = cleanup(
                    &dir,
                    &self.committed.union(&self.previous).cloned().collect(),
                );
            }
        }
        if let Some(pending) = pending {
            self.workspace.commit(pending);
        }
        Ok(())
    }
    fn stage_figures(
        &self,
        figures: &FigureSession,
        cancel: &AtomicBool,
    ) -> Result<PendingFigures> {
        figures.validate()?;
        let dir = checked_directory(&self.path, true)?;
        let keep: BTreeSet<_> = self.committed.union(&self.previous).cloned().collect();
        cleanup_cancel(&dir, &keep, cancel)?;
        let mut staged = StagedFiles {
            paths: vec![],
            committed: false,
        };
        let mut entries = Vec::new();
        let mut cache = BTreeMap::new();
        for doc in &figures.figures {
            crate::dsp::check_cancel(cancel)?;
            let existing = self.cache.get(&doc.id).filter(|c| {
                c.matches(&doc.figure)
                    && std::fs::symlink_metadata(dir.join(&c.reference.file))
                        .is_ok_and(|m| m.file_type().is_file() && m.len() == c.reference.bytes)
            });
            let mut reference = if let Some(cached) = existing {
                cached.reference.clone()
            } else {
                let bytes = files::encode_native_figure(&doc.figure)?;
                let mut file = tempfile::Builder::new()
                    .prefix("figure-")
                    .suffix(".mrfig")
                    .rand_bytes(16)
                    .tempfile_in(&dir)
                    .map_err(|e| e.to_string())?;
                for chunk in bytes.chunks(65536) {
                    crate::dsp::check_cancel(cancel)?;
                    file.write_all(chunk).map_err(|e| e.to_string())?;
                }
                file.as_file().sync_all().map_err(|e| e.to_string())?;
                let (_, path) = file.keep().map_err(|e| e.to_string())?;
                staged.paths.push(path.clone());
                FigureReference {
                    window: Default::default(),
                    id: doc.id,
                    file: path
                        .file_name()
                        .unwrap()
                        .to_str()
                        .ok_or("Invalid recovery filename")?
                        .into(),
                    bytes: bytes.len() as u64,
                    crc32: crc32(&bytes),
                    path: None,
                    source_version: None,
                    dirty: false,
                    view: None,
                }
            };
            reference.path = doc.path.clone();
            reference.source_version = doc.source_version;
            reference.dirty = doc.dirty;
            reference.view = doc.view.clone();
            reference.window = doc.window.clone().map_or_else(
                Default::default,
                super::figure_window::WindowRecovery::admitted,
            );
            cache.insert(doc.id, CachedFigure::new(doc, reference.clone()));
            entries.push(reference);
        }
        let manifest = FigureManifest {
            version: 1,
            current: figures.current,
            entries,
            error: None,
        };
        Ok(PendingFigures {
            files: staged,
            manifest,
            cache,
        })
    }
}
fn cleanup(dir: &Path, keep: &BTreeSet<String>) -> Result<()> {
    cleanup_cancel(dir, keep, &AtomicBool::new(false))
}
fn cleanup_cancel(dir: &Path, keep: &BTreeSet<String>, cancel: &AtomicBool) -> Result<()> {
    let mut retained = 0u64;
    for (count, entry) in std::fs::read_dir(dir)
        .map_err(|e| e.to_string())?
        .enumerate()
    {
        crate::dsp::check_cancel(cancel)?;
        if count >= 4096 {
            return Err("Too many files in figure recovery directory".into());
        }
        let entry = entry.map_err(|e| e.to_string())?;
        let name = entry.file_name();
        let Some(name) = name.to_str().filter(|n| owned_name(n)) else {
            continue;
        };
        if !entry.file_type().map_err(|e| e.to_string())?.is_file() {
            continue;
        }
        if !keep.contains(name) && std::fs::remove_file(entry.path()).is_ok() {
            continue;
        }
        retained = retained.saturating_add(entry.metadata().map_err(|e| e.to_string())?.len());
    }
    if retained > 2 * MAX_GENERATION_BYTES {
        return Err("Figure recovery disk budget exceeded; existing checkpoint preserved".into());
    }
    Ok(())
}
