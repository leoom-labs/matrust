use crate::{Result, files};
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
fn new_id() -> u64 {
    static NEXT: AtomicU64 = AtomicU64::new(1);
    NEXT.fetch_add(1, Ordering::Relaxed)
}

/// Bounded, normalized script contents observed when presenting a conflict.
/// Comparing contents also detects edits with unchanged timestamps and sizes.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum DiskVersion {
    Missing,
    Text(String),
}
#[derive(Debug, PartialEq, Eq)]
pub enum SaveOutcome {
    Saved,
    Conflict(DiskVersion),
}
fn disk_version(path: &Path) -> Result<DiskVersion> {
    match std::fs::metadata(path) {
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(DiskVersion::Missing),
        Err(e) => Err(e.to_string()),
        Ok(_) => files::read_script(path).map(DiskVersion::Text),
    }
}
pub fn absolute_path(path: &Path) -> Result<PathBuf> {
    std::fs::canonicalize(path)
        .or_else(|_| std::path::absolute(path))
        .map_err(|e| e.to_string())
}
pub fn same_path(a: &Path, b: &Path) -> bool {
    if a == b {
        return true;
    }
    let (Ok(a), Ok(b)) = (absolute_path(a), absolute_path(b)) else {
        return false;
    };
    #[cfg(windows)]
    {
        use std::os::windows::ffi::OsStrExt;
        // Match ordinary and canonical Windows prefixes even after deletion.
        let key = |p: &Path| {
            let mut units: Vec<u16> = p.as_os_str().encode_wide().collect();
            if units.starts_with(&[92, 92, 63, 92]) {
                units.drain(..4);
                if units.starts_with(&[85, 78, 67, 92]) {
                    units.splice(..4, [92, 92]);
                }
            }
            units
                .into_iter()
                .map(|v| if (65..=90).contains(&v) { v + 32 } else { v })
                .collect::<Vec<_>>()
        };
        key(&a) == key(&b)
    }
    #[cfg(not(windows))]
    {
        a == b
    }
}

/// A script's saved snapshot is only replaced after an atomic save succeeds.
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Document {
    #[serde(skip, default = "new_id")]
    pub id: u64,
    pub text: String,
    pub path: Option<PathBuf>,
    saved: String,
    pub dirty: bool,
}
impl Document {
    pub fn new() -> Self {
        Self {
            id: new_id(),
            text: String::new(),
            path: None,
            saved: String::new(),
            dirty: false,
        }
    }
    pub fn open(path: &Path) -> Result<Self> {
        let text = files::read_script(path)?;
        Ok(Self {
            id: new_id(),
            saved: text.clone(),
            text,
            path: Some(absolute_path(path)?),
            dirty: false,
        })
    }
    /// Open the exact bounded version read by the execution worker. Later saves
    /// compare against this baseline, including if the file changed meanwhile.
    pub fn from_loaded_script(path: PathBuf, text: String) -> Result<Self> {
        if !path.is_absolute() || text.len() > 1_000_000 {
            return Err("Loaded script needs an absolute path and at most 1 MB".into());
        }
        Ok(Self {
            id: new_id(),
            saved: text.clone(),
            text,
            path: Some(path),
            dirty: false,
        })
    }
    pub fn changed(&mut self) {
        self.dirty = self.text != self.saved;
    }
    pub fn discard(&mut self) {
        self.text.clone_from(&self.saved);
        self.dirty = false;
    }
    pub fn validate_recovery(&mut self) -> Result<()> {
        if self.text.len() > 1_000_000 || self.saved.len() > 1_000_000 {
            return Err("Recovered script exceeds 1 MB".into());
        }
        if files::bundle_relocation::declaration(&self.saved)?.is_some()
            && !self
                .path
                .as_ref()
                .is_some_and(|p| p.is_absolute() && p.as_os_str().len() <= 32768)
        {
            return Err(
                "Recovered reconstruction needs an absolute saved path within 32 KiB".into(),
            );
        }
        files::bundle_relocation::declaration(&self.text)?;
        self.changed();
        Ok(())
    }
    /// Explicit bounded companion verification for background Open/Reload.
    /// Text-only construction and session restoration do not hash MAT data.
    pub fn verify_reconstruction(&self, cancel: &AtomicBool) -> Result<()> {
        if self.saved.len() > 1_000_000 {
            return Err("Saved script exceeds 1 MB".into());
        }
        if let Some(path) = &self.path {
            files::bundle_relocation::verify(path, &self.saved, cancel)?;
        }
        Ok(())
    }
    /// Full bounded editor/saved snapshot identity for an explicit native choice.
    pub fn same_snapshot(&self, other: &Self) -> bool {
        self.id == other.id
            && self.path == other.path
            && self.text == other.text
            && self.saved == other.saved
            && self.dirty == other.dirty
    }
    pub fn needs_reconstruction_conversion(&self) -> Result<bool> {
        if self.text.len() > 1_000_000 || self.saved.len() > 1_000_000 {
            return Err("Script exceeds 1 MB".into());
        }
        files::bundle_relocation::needs_conversion(&self.saved, &self.text)
    }
    /// Explicit caller consent saves only script text. Existing companion files
    /// are never modified, copied or removed. Native UI binds this to a snapshot.
    pub fn try_save_edited_reconstruction_with_cancel(
        &mut self,
        path: &Path,
        confirmed: Option<&DiskVersion>,
        cancel: &AtomicBool,
    ) -> Result<SaveOutcome> {
        crate::dsp::check_cancel(cancel)?;
        if !self.needs_reconstruction_conversion()? {
            return Err("Reconstruction conversion choice no longer matches this document".into());
        }
        let converted = files::bundle_relocation::ordinary_text(&self.text);
        self.save_checked(path, confirmed, cancel, Some(converted))
    }
    pub fn save(&mut self, path: &Path) -> Result<()> {
        match self.try_save(path, None)? {
            SaveOutcome::Saved => Ok(()),
            SaveOutcome::Conflict(_) => Err("Script changed or was deleted on disk; reload, save as, or explicitly confirm overwriting the current version".into()),
        }
    }
    pub fn external_change(&self) -> Result<Option<DiskVersion>> {
        let Some(path) = &self.path else {
            return Ok(None);
        };
        let disk = disk_version(path)?;
        // Disk reads remove leading transport BOMs. Preserve editor/saved bytes,
        // but compare with that same normalization to avoid false conflicts.
        Ok((!matches!(&disk, DiskVersion::Text(text) if text.as_str() == self.saved.trim_start_matches('\u{feff}'))).then_some(disk))
    }
    /// Confirmation applies only to the exact disk version presented to the user.
    /// This is a best-effort check before atomic replacement, not a filesystem lock.
    pub fn try_save(
        &mut self,
        path: &Path,
        confirmed: Option<&DiskVersion>,
    ) -> Result<SaveOutcome> {
        self.try_save_with_cancel(path, confirmed, &AtomicBool::new(false))
    }
    /// A successful script publication is acknowledged even if Stop arrives
    /// immediately afterwards. Before that point bundle copy/hash is cancellable.
    pub fn try_save_with_cancel(
        &mut self,
        path: &Path,
        confirmed: Option<&DiskVersion>,
        cancel: &AtomicBool,
    ) -> Result<SaveOutcome> {
        self.save_checked(path, confirmed, cancel, None)
    }
    fn save_checked(
        &mut self,
        path: &Path,
        confirmed: Option<&DiskVersion>,
        cancel: &AtomicBool,
        converted: Option<String>,
    ) -> Result<SaveOutcome> {
        crate::dsp::check_cancel(cancel)?;
        if self.text.len() > 1_000_000
            || self.saved.len() > 1_000_000
            || converted
                .as_ref()
                .is_some_and(|text| text.len() > 1_000_000)
        {
            return Err("Script exceeds 1 MB".into());
        }
        let converting = converted.is_some();
        let bundled = !converting
            && (files::bundle_relocation::declaration(&self.saved)?.is_some()
                || files::bundle_relocation::declaration(&self.text)?.is_some());
        if bundled {
            files::bundle_relocation::destination(path)?;
        }
        let path = absolute_path(path)?;
        let same_path = self.path.as_ref().is_some_and(|p| same_path(p, &path));
        if !converting
            && !same_path
            && (files::bundle_relocation::legacy(&self.saved)
                || files::bundle_relocation::legacy(&self.text))
        {
            let same_folder = self
                .path
                .as_ref()
                .and_then(|p| p.parent())
                .zip(path.parent())
                .is_some_and(|(a, b)| crate::document::same_path(a, b));
            if !same_folder {
                return Err("This older generated reconstruction has no relocation manifest. Re-export the figure, or manually keep its script and companion folder together; Save As did not copy data".into());
            }
        }
        let mut checked_disk = None;
        if bundled || converting || same_path || confirmed.is_some() {
            let disk = disk_version(&path)?;
            let matches = confirmed.map_or_else(
                || if same_path {
                    matches!(&disk, DiskVersion::Text(text) if text.as_str() == self.saved.trim_start_matches('\u{feff}'))
                } else {
                    matches!(&disk, DiskVersion::Missing)
                },
                |expected| *expected == disk,
            );
            if !matches {
                return Ok(SaveOutcome::Conflict(disk));
            }
            checked_disk = Some(disk);
        }
        if bundled || converting {
            let prepared = if let Some(text) = &converted {
                files::bundle_relocation::stage_ordinary(&path, text, cancel)?
            } else {
                let source = self
                    .path
                    .as_deref()
                    .ok_or("Generated bundle has no saved origin; re-export the figure")?;
                files::bundle_relocation::prepare(source, &self.saved, &self.text, &path, cancel)?
            };
            let expected = checked_disk
                .as_ref()
                .ok_or("Missing script publication version")?;
            let replace = !matches!(expected, DiskVersion::Missing);
            prepared.publish(replace, cancel, || {
                if bundled { files::bundle_relocation::destination(&path)?; }
                if disk_version(&path)? != *expected {
                    return Err("Script destination changed during relocation; save again to review its current version".into());
                }
                Ok(())
            })?;
        } else {
            crate::dsp::check_cancel(cancel)?;
            files::atomic_write(&path, self.text.as_bytes())?;
        }
        if let Some(text) = converted {
            self.text = text;
        }
        self.path = Some(path);
        self.saved = self.text.clone();
        self.dirty = false;
        Ok(SaveOutcome::Saved)
    }
    pub fn reload(&mut self) -> Result<()> {
        let path = self
            .path
            .as_ref()
            .ok_or("Untitled script has no file to reload")?;
        let reopened = Self::open(path)?;
        self.text = reopened.text;
        self.saved = reopened.saved;
        self.path = reopened.path;
        self.dirty = false;
        Ok(())
    }
    pub fn title(&self) -> String {
        let name = self
            .path
            .as_ref()
            .and_then(|p| p.file_name())
            .map(|s| s.to_string_lossy().into_owned())
            .unwrap_or_else(|| "Untitled.mr".into());
        format!("{name}{}", if self.dirty { " *" } else { "" })
    }
}
impl Default for Document {
    fn default() -> Self {
        Self::new()
    }
}
