//! Byte-exact original-document copies. All source and destination I/O is worker-only.
use super::{
    export_batch::{Stamp, stamp},
    image_load::{MAX_FILE_BYTES, Original, check},
};
use matrust::Result;
use std::{
    fs,
    io::{Read, Write},
    path::{Path, PathBuf},
    sync::atomic::AtomicBool,
};

pub struct Prepare {
    pub source: PathBuf,
    pub original: Original,
    pub destination: PathBuf,
}
pub struct Plan {
    request: Prepare,
    expected: Option<Stamp>,
}
impl Plan {
    pub fn replaces(&self) -> bool {
        self.expected.is_some()
    }
    pub fn destination(&self) -> &Path {
        &self.request.destination
    }
}
pub enum Action {
    Prepare(Prepare),
    Commit(Plan),
}
pub struct Request {
    pub ticket: u64,
    pub action: Action,
}
pub enum Outcome {
    Review(Plan),
    Saved(PathBuf),
}
pub struct Finished {
    pub ticket: u64,
    pub result: Result<Outcome>,
}
pub fn perform(request: Request, cancel: &AtomicBool) -> Finished {
    let result = match request.action {
        Action::Prepare(request) => prepare(request, cancel).map(Outcome::Review),
        Action::Commit(plan) => save(plan, cancel).map(Outcome::Saved),
    };
    Finished {
        ticket: request.ticket,
        result,
    }
}

fn different(source: &Path, destination: &Path) -> Result<()> {
    let equal = |a: &Path, b: &Path| {
        a == b
            || (cfg!(windows)
                && a.as_os_str()
                    .as_encoded_bytes()
                    .eq_ignore_ascii_case(b.as_os_str().as_encoded_bytes()))
    };
    if !source.is_absolute() || !destination.is_absolute() {
        return Err("Original-copy paths must be absolute".into());
    }
    if equal(source, destination)
        || matches!((source.canonicalize(), destination.canonicalize()), (Ok(a), Ok(b)) if equal(&a,&b))
    {
        return Err("Choose a different file to preserve the original document".into());
    }
    Ok(())
}
fn visit(
    request: &Prepare,
    cancel: &AtomicBool,
    mut consume: impl FnMut(&[u8]) -> Result<()>,
) -> Result<()> {
    check(cancel)?;
    let original = &request.original;
    if original.len > MAX_FILE_BYTES {
        return Err("Original document exceeds 128 MiB".into());
    }
    let mut hash = blake3::Hasher::new();
    let mut count = 0usize;
    let mut chunk = |bytes: &[u8]| -> Result<()> {
        check(cancel)?;
        count = count
            .checked_add(bytes.len())
            .filter(|n| *n <= original.len)
            .ok_or("Original document changed; reload it before saving an original copy")?;
        hash.update(bytes);
        consume(bytes)?;
        check(cancel)
    };
    if let Some(bytes) = &original.bytes {
        for bytes in bytes.chunks(64 * 1024) {
            chunk(bytes)?;
        }
    } else {
        let mut source = fs::File::open(&request.source).map_err(|e| {
            format!("Original source is unavailable; reload it before copying: {e}")
        })?;
        let meta = source.metadata().map_err(|e| e.to_string())?;
        if !meta.is_file() || meta.len() != original.len as u64 {
            return Err(
                "Original document changed; reload it before saving an original copy".into(),
            );
        }
        let mut buffer = [0; 64 * 1024];
        loop {
            check(cancel)?;
            let n = source.read(&mut buffer).map_err(|e| e.to_string())?;
            if n == 0 {
                break;
            }
            chunk(&buffer[..n])?;
        }
    }
    if count != original.len || hash.finalize().as_bytes() != &original.digest {
        return Err("Original document changed; reload it before saving an original copy".into());
    }
    check(cancel)
}
pub fn prepare(mut request: Prepare, cancel: &AtomicBool) -> Result<Plan> {
    check(cancel)?;
    request.source = std::path::absolute(&request.source).map_err(|e| e.to_string())?;
    let filename = request
        .destination
        .file_name()
        .ok_or("Choose an original-copy file name")?
        .to_owned();
    let parent = request
        .destination
        .parent()
        .filter(|p| !p.as_os_str().is_empty())
        .unwrap_or(Path::new("."));
    request.destination = parent
        .canonicalize()
        .map_err(|e| e.to_string())?
        .join(filename);
    if request.destination.extension().is_none()
        && let Some(extension) = request.source.extension()
    {
        request.destination.set_extension(extension);
    }
    if request.source.extension().is_none()
        || !request.destination.extension().is_some_and(|a| {
            request.source.extension().is_some_and(|b| {
                a.as_encoded_bytes()
                    .eq_ignore_ascii_case(b.as_encoded_bytes())
            })
        })
    {
        return Err("Keep the original file extension; this action copies the original document without conversion".into());
    }
    different(&request.source, &request.destination)?;
    visit(&request, cancel, |_| Ok(()))?;
    let expected = stamp(&request.destination, cancel)?;
    check(cancel)?;
    Ok(Plan { request, expected })
}
pub fn save(plan: Plan, cancel: &AtomicBool) -> Result<PathBuf> {
    save_checked(plan, cancel, || {})
}
fn save_checked(plan: Plan, cancel: &AtomicBool, mut checkpoint: impl FnMut()) -> Result<PathBuf> {
    check(cancel)?;
    let path = &plan.request.destination;
    different(&plan.request.source, path)?;
    if stamp(path, cancel)? != plan.expected {
        return Err("Destination changed after review; choose the destination again".into());
    }
    let mut temporary =
        tempfile::NamedTempFile::new_in(path.parent().ok_or("Missing destination folder")?)
            .map_err(|e| e.to_string())?;
    visit(&plan.request, cancel, |bytes| {
        temporary.write_all(bytes).map_err(|e| e.to_string())?;
        checkpoint();
        Ok(())
    })?;
    temporary.flush().map_err(|e| e.to_string())?;
    different(&plan.request.source, path)?;
    if stamp(path, cancel)? != plan.expected {
        return Err("Destination changed after review; choose the destination again".into());
    }
    check(cancel)?;
    // Match existing export publication semantics: atomic no-clobber for new
    // files; reviewed replacement is best-effort, not a cross-process lock.
    // Rename replaces the destination directory entry, never truncates any
    // original inode shared through a hard link.
    if plan.expected.is_some() {
        temporary.persist(path).map_err(|e| e.error.to_string())?;
    } else {
        temporary
            .persist_noclobber(path)
            .map_err(|e| e.error.to_string())?;
    }
    Ok(path.clone())
}

#[cfg(test)]
#[path = "original_copy_tests.rs"]
mod tests;
