//! Figure file fingerprints tied to the exact decoded or published bytes.
//! Conflict checks stream bounded chunks; they do not retain a second figure.
use crate::{Result, plot::Figure};
use serde::{Deserialize, Serialize};
use std::{
    fs::File,
    io::{ErrorKind, Read},
    path::Path,
    sync::atomic::{AtomicBool, Ordering},
};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Version {
    pub bytes: u64,
    blake3: [u8; 32],
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DiskVersion {
    /// Older recovery documents have no reliable saved-file fingerprint.
    Unknown,
    Missing,
    Present(Version),
}
#[derive(Debug)]
pub enum SaveOutcome {
    Saved(Version),
    Conflict(DiskVersion),
}
fn cancelled(cancel: &AtomicBool) -> Result<()> {
    if cancel.load(Ordering::Relaxed) {
        Err("Figure file operation cancelled".into())
    } else {
        Ok(())
    }
}
fn is_matlab(path: &Path) -> Result<bool> {
    match path.extension().and_then(|s| s.to_str()) {
        Some(s) if s.eq_ignore_ascii_case("fig") => Ok(true),
        Some(s) if s.eq_ignore_ascii_case("mrfig") || s.eq_ignore_ascii_case("mfig") => Ok(false),
        _ => Err("Editable figures must use .mrfig, .mfig or .fig".into()),
    }
}
fn limit(path: &Path) -> Result<usize> {
    Ok(if is_matlab(path)? {
        super::fig_mat::LIMIT
    } else {
        17_000_000
    })
}
fn version(bytes: &[u8], cancel: &AtomicBool) -> Result<Version> {
    let mut hash = blake3::Hasher::new();
    for chunk in bytes.chunks(64 * 1024) {
        cancelled(cancel)?;
        hash.update(chunk);
    }
    cancelled(cancel)?;
    Ok(Version {
        bytes: bytes.len() as u64,
        blake3: *hash.finalize().as_bytes(),
    })
}
fn read_version(
    path: &Path,
    cancel: &AtomicBool,
    mut collect: Option<&mut Vec<u8>>,
) -> Result<DiskVersion> {
    cancelled(cancel)?;
    let limit = limit(path)?;
    let mut file = match File::open(path) {
        Ok(f) => f,
        Err(e) if e.kind() == ErrorKind::NotFound => return Ok(DiskVersion::Missing),
        Err(e) => return Err(e.to_string()),
    };
    let metadata = file.metadata().map_err(|e| e.to_string())?;
    if !metadata.is_file() || metadata.len() > limit as u64 {
        return Err(format!(
            "Figure must be a regular file of at most {limit} bytes"
        ));
    }
    if let Some(bytes) = &mut collect {
        bytes.reserve(metadata.len() as usize);
    }
    let mut hash = blake3::Hasher::new();
    let mut count = 0;
    let mut buffer = [0; 256 * 1024];
    loop {
        cancelled(cancel)?;
        let n = match file.read(&mut buffer) {
            Err(e) if e.kind() == ErrorKind::Interrupted => continue,
            result => result.map_err(|e| e.to_string())?,
        };
        if n == 0 {
            break;
        }
        count += n;
        if count > limit {
            return Err("Figure exceeds file size limit".into());
        }
        hash.update(&buffer[..n]);
        if let Some(bytes) = &mut collect {
            bytes.extend_from_slice(&buffer[..n]);
        }
    }
    cancelled(cancel)?;
    Ok(DiskVersion::Present(Version {
        bytes: count as u64,
        blake3: *hash.finalize().as_bytes(),
    }))
}
/// Reads only a fixed-size buffer regardless of figure size.
pub fn inspect(path: &Path, cancel: &AtomicBool) -> Result<DiskVersion> {
    read_version(path, cancel, None)
}
pub fn load(path: &Path, cancel: &AtomicBool) -> Result<(Figure, Version)> {
    let mut bytes = Vec::new();
    let DiskVersion::Present(version) = read_version(path, cancel, Some(&mut bytes))? else {
        return Err("Figure file was deleted or moved".into());
    };
    let figure = if is_matlab(path)? {
        super::matlab_fig::decode_matlab_figure(&bytes)?
    } else {
        super::decode_native_figure(&bytes)?
    };
    cancelled(cancel)?;
    Ok((figure, version))
}
/// Check immediately before atomic replacement. This is not a cross-process lock.
/// `None` is for a new/Save-as destination confirmed by the native file dialog.
pub fn save(
    path: &Path,
    figure: &Figure,
    expected: Option<DiskVersion>,
    cancel: &AtomicBool,
) -> Result<SaveOutcome> {
    save_with_theme(
        path,
        figure,
        expected,
        crate::plot::BaseTheme::Light,
        cancel,
    )
}
pub fn save_with_theme(
    path: &Path,
    figure: &Figure,
    expected: Option<DiskVersion>,
    host: crate::plot::BaseTheme,
    cancel: &AtomicBool,
) -> Result<SaveOutcome> {
    cancelled(cancel)?;
    let bytes = if is_matlab(path)? {
        super::matlab_fig::encode_matlab_figure_with_theme(figure, host)?
    } else {
        super::encode_native_figure(figure)?
    };
    let version = version(&bytes, cancel)?;
    if let Some(expected) = expected {
        let actual = inspect(path, cancel)?;
        if actual != expected {
            return Ok(SaveOutcome::Conflict(actual));
        }
    }
    cancelled(cancel)?;
    super::atomic_write(path, &bytes)?;
    Ok(SaveOutcome::Saved(version))
}
