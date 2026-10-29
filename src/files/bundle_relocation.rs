//! Explicit generated-bundle metadata. This never discovers dependencies in arbitrary code.
use crate::Result;
use serde::{Deserialize, Serialize};
use std::{
    fs::{self, File},
    io::{Read, Write},
    path::{Path, PathBuf},
    sync::atomic::AtomicBool,
};

const PREFIX: &str = "% MatRust reconstruction bundle ";
const V1: &str = "% MatRust reconstruction bundle v1: ";
const MANIFEST: &str = "reconstruction.json";
const MANIFEST_LIMIT: usize = 4096;
const CHUNK: usize = 65536;

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Manifest {
    version: u32,
    companion: String,
    // UTF-8 text with leading BOMs removed and CRLF normalized to LF.
    script_blake3: String,
    data_bytes: u64,
    data_blake3: String,
}

fn leaf(value: &str) -> Result<&str> {
    if !(9..=80).contains(&value.len())
        || !value.starts_with("mr_plot_")
        || !value
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b == b'_')
    {
        return Err("Invalid reconstruction companion name; re-export the figure".into());
    }
    Ok(value)
}

pub(crate) fn declaration(text: &str) -> Result<Option<&str>> {
    let first = text
        .trim_start_matches('\u{feff}')
        .split('\n')
        .next()
        .unwrap_or("")
        .trim_end_matches('\r');
    if let Some(value) = first.strip_prefix(V1) {
        return leaf(value).map(Some);
    }
    if first.starts_with(PREFIX) {
        return Err("Unsupported reconstruction bundle version; re-export the figure".into());
    }
    Ok(None)
}

pub(crate) fn legacy(text: &str) -> bool {
    let mut lines = text.trim_start_matches('\u{feff}').lines();
    let Some(name) = lines
        .next()
        .and_then(|line| line.strip_prefix("% Keep this script and "))
        .and_then(|line| line.strip_suffix("/data.mat together."))
    else {
        return false;
    };
    leaf(name).is_ok()
        && lines.next().is_some_and(|line| {
            line == format!("load(fullfile(fileparts(mfilename('fullpath')),'{name}','data.mat'));")
        })
}

pub(super) fn header(name: &str) -> Result<String> {
    Ok(format!("{V1}{}\n", leaf(name)?))
}

fn script_digest(text: &str) -> String {
    blake3::hash(
        text.trim_start_matches('\u{feff}')
            .replace("\r\n", "\n")
            .as_bytes(),
    )
    .to_hex()
    .to_string()
}

pub(crate) fn needs_conversion(saved: &str, text: &str) -> Result<bool> {
    let declared = declaration(saved)?.is_some();
    let current_declared_comment = text.trim_start_matches('\u{feff}').starts_with(PREFIX);
    Ok((declared && script_digest(saved) != script_digest(text))
        || (!declared && current_declared_comment))
}

/// Explicit conversion changes a declaration comment only, never executable code.
/// The notice also prevents the retained historical Keep/load pair from being
/// mistaken for a legacy declaration after the first line is removed.
pub(crate) fn ordinary_text(text: &str) -> String {
    // Match read_script's leading transport-BOM normalization. Otherwise a BOM
    // could become internal, or the saved snapshot could differ from disk reads.
    // Executable body bytes and original line endings remain unchanged.
    let body = text.trim_start_matches('\u{feff}');
    let first = body.split('\n').next().unwrap_or("");
    let ending = if first.ends_with('\r') { "\r\n" } else { "\n" };
    let rest = if first.starts_with(PREFIX) {
        body.find('\n').map_or("", |index| &body[index + 1..])
    } else {
        body
    };
    format!(
        "% MatRust ordinary script: companion dependencies are not copied automatically.{ending}{rest}"
    )
}

pub(crate) fn stage_ordinary(
    path: &Path,
    text: &str,
    cancel: &AtomicBool,
) -> Result<super::PreparedMatlabPlotScript> {
    super::plot_bundle::stage_text(path, text, None, cancel)
}

fn digest_valid(value: &str) -> bool {
    value.len() == 64
        && value
            .bytes()
            .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
}

/// The user-selected parent is an authority boundary: canonicalize it once,
/// allowing ordinary junction/symlink parents. Only companion descendants and
/// an existing destination script itself must remain unlinked.
fn selected_parent(path: &Path) -> Result<PathBuf> {
    if path.as_os_str().len() > 32768 {
        return Err("Reconstruction path exceeds the 32 KiB limit".into());
    }
    let canonical = fs::canonicalize(parent(path)).map_err(|e| e.to_string())?;
    if !fs::metadata(&canonical)
        .map_err(|e| e.to_string())?
        .is_dir()
    {
        return Err("Reconstruction parent must be a directory".into());
    }
    Ok(canonical)
}

fn unlinked(path: &Path) -> Result<()> {
    let metadata = fs::symlink_metadata(path).map_err(|e| e.to_string())?;
    #[cfg(windows)]
    let reparse = {
        use std::os::windows::fs::MetadataExt;
        metadata.file_attributes() & 0x400 != 0
    };
    #[cfg(not(windows))]
    let reparse = false;
    if metadata.file_type().is_symlink() || reparse {
        return Err(
            "Reconstruction companion files must not use symlinks or reparse points".into(),
        );
    }
    Ok(())
}

pub(crate) fn destination(path: &Path) -> Result<()> {
    let selected = selected_parent(path)?;
    let name = path.file_name().ok_or("Choose a script filename")?;
    let target = selected.join(name);
    match fs::symlink_metadata(&target) {
        Ok(_) => unlinked(&target),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(e) => Err(e.to_string()),
    }
}

fn parent(path: &Path) -> &Path {
    path.parent()
        .filter(|p| !p.as_os_str().is_empty())
        .unwrap_or(Path::new("."))
}

fn read_manifest(directory: &Path, name: &str, text: &str) -> Result<(Manifest, Vec<u8>)> {
    unlinked(directory)?;
    let mut names = Vec::new();
    for entry in fs::read_dir(directory).map_err(|e| e.to_string())? {
        let entry = entry.map_err(|e| e.to_string())?;
        if names.len() == 2 {
            return Err("Unexpected files in reconstruction companion".into());
        }
        unlinked(&entry.path())?;
        if !entry.file_type().map_err(|e| e.to_string())?.is_file() {
            return Err("Reconstruction companion needs regular files".into());
        }
        names.push(entry.file_name());
    }
    names.sort();
    if names
        != [
            std::ffi::OsString::from("data.mat"),
            std::ffi::OsString::from(MANIFEST),
        ]
    {
        return Err("Missing reconstruction data or manifest; re-export the figure".into());
    }
    let bytes = super::read_bounded(&directory.join(MANIFEST), MANIFEST_LIMIT)?;
    let manifest: Manifest = serde_json::from_slice(&bytes)
        .map_err(|e| format!("Invalid reconstruction manifest: {e}"))?;
    if manifest.version != 1
        || manifest.companion != name
        || !digest_valid(&manifest.script_blake3)
        || !digest_valid(&manifest.data_blake3)
        || manifest.data_bytes > super::MAX_MAT_BYTES as u64
    {
        return Err("Invalid reconstruction manifest identity or bounds".into());
    }
    if manifest.script_blake3 != script_digest(text) {
        return Err("Generated reconstruction text was modified. Re-export the figure to relocate its data; arbitrary script dependencies are not copied".into());
    }
    Ok((manifest, bytes))
}

fn transfer(
    path: &Path,
    output: &mut impl Write,
    cancel: &AtomicBool,
    mut progress: impl FnMut(u64),
) -> Result<(u64, String)> {
    unlinked(path)?;
    let mut input = File::open(path).map_err(|e| e.to_string())?;
    let metadata = input.metadata().map_err(|e| e.to_string())?;
    if !metadata.is_file() || metadata.len() > super::MAX_MAT_BYTES as u64 {
        return Err("Reconstruction MAT data exceeds the regular-file limit".into());
    }
    let mut buffer = [0_u8; CHUNK];
    let mut total = 0_u64;
    let mut digest = blake3::Hasher::new();
    loop {
        crate::dsp::check_cancel(cancel)?;
        let count = input.read(&mut buffer).map_err(|e| e.to_string())?;
        if count == 0 {
            break;
        }
        total = total
            .checked_add(count as u64)
            .ok_or("Reconstruction data size overflow")?;
        if total > super::MAX_MAT_BYTES as u64 {
            return Err("Reconstruction data grew beyond its limit".into());
        }
        digest.update(&buffer[..count]);
        output
            .write_all(&buffer[..count])
            .map_err(|e| e.to_string())?;
        progress(total);
    }
    crate::dsp::check_cancel(cancel)?;
    unlinked(path)?;
    Ok((total, digest.finalize().to_hex().to_string()))
}

fn matches_data(actual: (u64, String), manifest: &Manifest) -> Result<()> {
    if actual != (manifest.data_bytes, manifest.data_blake3.clone()) {
        return Err(
            "Reconstruction MAT data changed or is incomplete; re-export the figure".into(),
        );
    }
    Ok(())
}

pub(super) fn write_manifest(
    directory: &Path,
    name: &str,
    script: &str,
    cancel: &AtomicBool,
) -> Result<()> {
    let (data_bytes, data_blake3) = transfer(
        &directory.join("data.mat"),
        &mut std::io::sink(),
        cancel,
        |_| {},
    )?;
    let manifest = Manifest {
        version: 1,
        companion: leaf(name)?.into(),
        script_blake3: script_digest(script),
        data_bytes,
        data_blake3,
    };
    let bytes = serde_json::to_vec_pretty(&manifest).map_err(|e| e.to_string())?;
    if bytes.len() > MANIFEST_LIMIT {
        return Err("Reconstruction manifest exceeds limit".into());
    }
    crate::dsp::check_cancel(cancel)?;
    let mut file = File::options()
        .write(true)
        .create_new(true)
        .open(directory.join(MANIFEST))
        .map_err(|e| e.to_string())?;
    file.write_all(&bytes)
        .and_then(|()| file.sync_all())
        .map_err(|e| e.to_string())?;
    crate::dsp::check_cancel(cancel)
}

/// Native Open/Reload call this on the existing worker, never while painting UI.
/// Legacy/no-declaration scripts retain their existing text-only Open behavior.
pub(crate) fn verify(path: &Path, text: &str, cancel: &AtomicBool) -> Result<()> {
    crate::dsp::check_cancel(cancel)?;
    if let Some(name) = declaration(text)? {
        if !path.is_absolute() {
            return Err("Reconstruction bundle needs an absolute saved script path".into());
        }
        let directory = selected_parent(path)?.join(name);
        let (manifest, original) = read_manifest(&directory, name, text)?;
        matches_data(
            transfer(
                &directory.join("data.mat"),
                &mut std::io::sink(),
                cancel,
                |_| {},
            )?,
            &manifest,
        )?;
        if read_manifest(&directory, name, text)?.1 != original {
            return Err("Reconstruction manifest changed during verification".into());
        }
    }
    Ok(())
}

pub(crate) fn prepare(
    source: &Path,
    saved: &str,
    text: &str,
    target: &Path,
    cancel: &AtomicBool,
) -> Result<super::PreparedMatlabPlotScript> {
    prepare_checked(source, saved, text, target, cancel, |_| {})
}

fn prepare_checked(
    source: &Path,
    saved: &str,
    text: &str,
    target: &Path,
    cancel: &AtomicBool,
    progress: impl FnMut(u64),
) -> Result<super::PreparedMatlabPlotScript> {
    crate::dsp::check_cancel(cancel)?;
    if !source.is_absolute() {
        return Err("Reconstruction bundle needs an absolute saved script path".into());
    }
    let name =
        declaration(saved)?.ok_or("Generated bundle has no saved origin; re-export the figure")?;
    // Checking the saved snapshot prevents deleting/editing the declaration to
    // turn a loaded generated bundle into an ordinary blind Save As.
    if declaration(text)? != Some(name) || script_digest(saved) != script_digest(text) {
        return Err("Generated reconstruction text was modified. Re-export the figure before Save As; its companion data was not copied".into());
    }
    super::validate_matlab_plot_script_path(target)?;
    super::plot_bundle::validate_loading_name(target)?;
    destination(target)?;
    let source_parent = selected_parent(source)?;
    let target_parent = selected_parent(target)?;
    let directory = source_parent.join(name);
    let (manifest, original) = read_manifest(&directory, name, saved)?;
    let same_directory = crate::document::same_path(&source_parent, &target_parent);
    let generation = if same_directory {
        matches_data(
            transfer(
                &directory.join("data.mat"),
                &mut std::io::sink(),
                cancel,
                progress,
            )?,
            &manifest,
        )?;
        None
    } else {
        // An exact, nonrandom name preserves every generated script byte. With
        // zero random bytes tempfile creates this directory exclusively; an
        // existing generation is a conflict, never an overwrite or merge.
        let copy = tempfile::Builder::new().prefix(name).rand_bytes(0).tempdir_in(&target_parent)
            .map_err(|e| format!("Cannot create reconstruction companion (choose another folder or remove an unrelated conflicting generation): {e}"))?;
        let mut data = File::options()
            .write(true)
            .create_new(true)
            .open(copy.path().join("data.mat"))
            .map_err(|e| e.to_string())?;
        matches_data(
            transfer(&directory.join("data.mat"), &mut data, cancel, progress)?,
            &manifest,
        )?;
        data.sync_all().map_err(|e| e.to_string())?;
        drop(data);
        let mut metadata = File::options()
            .write(true)
            .create_new(true)
            .open(copy.path().join(MANIFEST))
            .map_err(|e| e.to_string())?;
        metadata
            .write_all(&original)
            .and_then(|()| metadata.sync_all())
            .map_err(|e| e.to_string())?;
        drop(metadata);
        Some(copy)
    };
    // Re-read bounded authority after transfer; failed/cancelled staging owns
    // and removes only its newly created destination generation.
    if read_manifest(&directory, name, saved)?.1 != original {
        return Err("Reconstruction manifest changed during relocation".into());
    }
    destination(target)?;
    crate::dsp::check_cancel(cancel)?;
    super::plot_bundle::stage_text(target, text, generation, cancel)
}

#[cfg(test)]
mod tests;
