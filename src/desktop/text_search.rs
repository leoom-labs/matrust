//! Bounded disk-text search. No Engine state, UI widgets or file execution.
use matrust::Result;
use std::{
    fs::{self, File, Metadata},
    io::Read,
    ops::Range,
    path::{Path, PathBuf},
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
};

pub const MAX_FILE: usize = 1_000_000;
pub const MAX_FILES: usize = 2_000;
pub const MAX_ENTRIES: usize = 20_000;
pub const MAX_READ: usize = 64 * 1024 * 1024;
pub const MAX_RESULTS: usize = 2_000;
pub const MAX_OUTPUT: usize = 4 * 1024 * 1024;
const MAX_DEPTH: usize = 16;
const MAX_PATH: usize = 4096;
struct Limits {
    files: usize,
    entries: usize,
    read: usize,
    results: usize,
    output: usize,
    depth: usize,
}
impl Default for Limits {
    fn default() -> Self {
        Self {
            files: MAX_FILES,
            entries: MAX_ENTRIES,
            read: MAX_READ,
            results: MAX_RESULTS,
            output: MAX_OUTPUT,
            depth: MAX_DEPTH,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Query {
    pub folder: PathBuf,
    pub text: String,
    pub match_case: bool,
    pub subfolders: bool,
}
pub struct Request {
    pub generation: u64,
    pub query: Query,
}
#[derive(Clone, Debug)]
pub struct Hit {
    pub path: Arc<PathBuf>,
    pub root: Arc<PathBuf>,
    pub raw_digest: blake3::Hash,
    pub text_digest: blake3::Hash,
    pub text_len: usize,
    pub bytes: Range<usize>,
    pub line: usize,
    pub context: String,
}
#[derive(Default, Debug)]
pub struct Counts {
    pub visited: usize,
    pub files: usize,
    pub read_bytes: usize,
    pub unsupported: usize,
    pub binary: usize,
    pub oversized: usize,
    pub linked: usize,
    pub unreadable: usize,
    pub depth: usize,
}
pub struct Results {
    pub hits: Vec<Hit>,
    pub counts: Counts,
    pub truncated: Option<&'static str>,
}
pub struct Finished {
    pub generation: u64,
    pub query: Query,
    pub result: Result<Results>,
}

fn check(cancel: &AtomicBool) -> Result<()> {
    if cancel.load(Ordering::Relaxed) {
        Err("Text search cancelled".into())
    } else {
        Ok(())
    }
}
fn linked(metadata: &Metadata) -> bool {
    if metadata.file_type().is_symlink() {
        return true;
    }
    #[cfg(windows)]
    {
        use std::os::windows::fs::MetadataExt;
        if metadata.file_attributes() & 0x400 != 0 {
            return true;
        }
    }
    false
}
fn supported(path: &Path) -> bool {
    path.extension()
        .and_then(|s| s.to_str())
        .is_some_and(|s| ["mr", "m", "txt"].iter().any(|e| s.eq_ignore_ascii_case(e)))
}
/// The selected canonical root is trusted; every descendant must be unlinked.
fn descendant(root: &Path, path: &Path) -> Result<()> {
    let relative = path
        .strip_prefix(root)
        .map_err(|_| "Search path escaped Current Folder")?;
    let mut current = root.to_owned();
    for component in relative.components() {
        if !matches!(component, std::path::Component::Normal(_)) {
            return Err("Invalid search path component".into());
        }
        current.push(component);
        if linked(&fs::symlink_metadata(&current).map_err(|e| e.to_string())?) {
            return Err("Linked search descendants are skipped".into());
        }
    }
    if !fs::canonicalize(path)
        .map_err(|e| e.to_string())?
        .starts_with(root)
    {
        return Err("Search path escaped Current Folder".into());
    }
    Ok(())
}
struct Text {
    raw_digest: blake3::Hash,
    text: String,
}
fn read_text(root: &Path, path: &Path, cancel: &AtomicBool, budget: &mut usize) -> Result<Text> {
    check(cancel)?;
    descendant(root, path)?;
    let mut file = File::open(path).map_err(|e| e.to_string())?;
    let before = file.metadata().map_err(|e| e.to_string())?;
    if !before.is_file() || before.len() > MAX_FILE as u64 {
        return Err("File exceeds 1 MB or is not regular".into());
    }
    let mut data = Vec::with_capacity((before.len() as usize).min(MAX_FILE));
    let mut hash = blake3::Hasher::new();
    let mut chunk = [0u8; 65536];
    loop {
        check(cancel)?;
        if *budget == 0 {
            return Err("Cumulative search read limit reached".into());
        }
        let take = chunk.len().min(*budget).min(MAX_FILE + 1 - data.len());
        let n = file.read(&mut chunk[..take]).map_err(|e| e.to_string())?;
        if n == 0 {
            break;
        }
        *budget -= n;
        hash.update(&chunk[..n]);
        data.extend_from_slice(&chunk[..n]);
        if data.len() > MAX_FILE {
            return Err("File grew beyond 1 MB".into());
        }
    }
    check(cancel)?;
    descendant(root, path)?;
    let after = file.metadata().map_err(|e| e.to_string())?;
    if before.len() != after.len() || before.modified().ok() != after.modified().ok() {
        return Err("File changed during search".into());
    }
    let mut text = String::from_utf8(data).map_err(|_| "Not UTF-8 text".to_owned())?;
    if text
        .chars()
        .any(|c| c.is_control() && !matches!(c, '\t' | '\r' | '\n'))
    {
        return Err("Binary control bytes in text file".into());
    }
    let bom = text.len() - text.trim_start_matches('\u{feff}').len();
    if bom != 0 {
        text.drain(..bom);
    }
    Ok(Text {
        raw_digest: hash.finalize(),
        text,
    })
}
fn matcher(query: &Query) -> Result<regex::Regex> {
    if query.text.is_empty()
        || query.text.len() > 1024
        || query.text.chars().count() > 256
        || query.text.chars().any(|c| c.is_control())
    {
        return Err("Enter a literal query of 1–256 characters (at most 1024 UTF-8 bytes), without line breaks".into());
    }
    regex::RegexBuilder::new(&regex::escape(&query.text))
        .case_insensitive(!query.match_case)
        .size_limit(2 * 1024 * 1024)
        .dfa_size_limit(2 * 1024 * 1024)
        .build()
        .map_err(|e| format!("Search query cannot be compiled within its limit: {e}"))
}
fn context(line: &str, at: usize) -> String {
    let start = line[..at]
        .char_indices()
        .rev()
        .nth(48)
        .map_or(0, |(i, _)| i);
    let mut output = String::new();
    if start != 0 {
        output.push('…');
    }
    let mut chars = line[start..].chars();
    for ch in chars.by_ref().take(160) {
        output.push(ch);
    }
    if chars.next().is_some() {
        output.push('…');
    }
    output
}
pub fn perform(request: Request, cancel: &AtomicBool) -> Finished {
    let result = scan(&request.query, cancel);
    Finished {
        generation: request.generation,
        query: request.query,
        result,
    }
}
pub fn scan(query: &Query, cancel: &AtomicBool) -> Result<Results> {
    scan_limited(query, cancel, Limits::default())
}
fn scan_limited(query: &Query, cancel: &AtomicBool, limits: Limits) -> Result<Results> {
    check(cancel)?;
    let matcher = matcher(query)?;
    let root = Arc::new(fs::canonicalize(&query.folder).map_err(|e| e.to_string())?);
    if !root.is_dir() {
        return Err("Current Folder is not a directory".into());
    }
    if root.as_os_str().as_encoded_bytes().len() > MAX_PATH {
        return Err("Current Folder path exceeds the search path limit".into());
    }
    let mut result = Results {
        hits: Vec::new(),
        counts: Counts::default(),
        truncated: None,
    };
    let mut stack = vec![(root.as_ref().clone(), 0usize)];
    let mut stack_bytes = root.as_os_str().as_encoded_bytes().len();
    let mut budget = limits.read;
    let mut output_bytes = 0usize;
    'directories: while let Some((folder, depth)) = stack.pop() {
        stack_bytes = stack_bytes.saturating_sub(folder.as_os_str().as_encoded_bytes().len());
        check(cancel)?;
        if folder != *root
            && let Err(error) = descendant(&root, &folder)
        {
            if error == "Linked search descendants are skipped" {
                result.counts.linked += 1;
            } else {
                result.counts.unreadable += 1;
            }
            continue;
        }
        let entries = match fs::read_dir(&folder) {
            Ok(e) => e,
            Err(_) => {
                result.counts.unreadable += 1;
                continue;
            }
        };
        for entry in entries {
            check(cancel)?;
            if result.counts.visited == limits.entries {
                result.truncated = Some("directory-entry limit");
                break 'directories;
            }
            result.counts.visited += 1;
            let entry = match entry {
                Ok(e) => e,
                Err(_) => {
                    result.counts.unreadable += 1;
                    continue;
                }
            };
            let path = entry.path();
            let cost = path.as_os_str().as_encoded_bytes().len();
            if cost > MAX_PATH {
                result.counts.unreadable += 1;
                continue;
            }
            let metadata = match fs::symlink_metadata(&path) {
                Ok(m) => m,
                Err(_) => {
                    result.counts.unreadable += 1;
                    continue;
                }
            };
            if linked(&metadata) {
                result.counts.linked += 1;
                continue;
            }
            if metadata.is_dir() {
                if query.subfolders {
                    if depth == limits.depth {
                        result.counts.depth += 1;
                        result.truncated = Some("subfolder depth limit");
                    } else if stack_bytes + cost > limits.output {
                        result.truncated = Some("pending-directory memory limit");
                        break 'directories;
                    } else {
                        stack_bytes += cost;
                        stack.push((path, depth + 1));
                    }
                }
                continue;
            }
            if !metadata.is_file() || !supported(&path) {
                result.counts.unsupported += 1;
                continue;
            }
            if metadata.len() > MAX_FILE as u64 {
                result.counts.oversized += 1;
                continue;
            }
            if result.counts.files == limits.files {
                result.truncated = Some("file limit");
                break 'directories;
            }
            if metadata.len() as usize >= budget {
                result.truncated = Some("cumulative read limit");
                break 'directories;
            }
            result.counts.files += 1;
            let file = match read_text(&root, &path, cancel, &mut budget) {
                Ok(file) => file,
                Err(error) => {
                    check(cancel)?;
                    if budget == 0 {
                        result.truncated = Some("cumulative read limit");
                        break 'directories;
                    }
                    if error == "Not UTF-8 text" || error == "Binary control bytes in text file" {
                        result.counts.binary += 1;
                    } else {
                        result.counts.unreadable += 1;
                    }
                    continue;
                }
            };
            let text_digest = blake3::hash(file.text.as_bytes());
            let path = Arc::new(path);
            let mut start = 0;
            let mut line_number = 1;
            while start < file.text.len() {
                check(cancel)?;
                let end = file.text[start..]
                    .find(['\r', '\n'])
                    .map_or(file.text.len(), |n| start + n);
                let line = &file.text[start..end];
                for found in matcher.find_iter(line) {
                    check(cancel)?;
                    let context = context(line, found.start());
                    let cost = std::mem::size_of::<Hit>()
                        + context.len()
                        + path.as_os_str().as_encoded_bytes().len();
                    if result.hits.len() == limits.results || output_bytes + cost > limits.output {
                        result.truncated = Some("result limit");
                        break 'directories;
                    }
                    output_bytes += cost;
                    result.hits.push(Hit {
                        path: path.clone(),
                        root: root.clone(),
                        raw_digest: file.raw_digest,
                        text_digest,
                        text_len: file.text.len(),
                        bytes: start + found.start()..start + found.end(),
                        line: line_number,
                        context,
                    });
                }
                if end == file.text.len() {
                    break;
                }
                start = end + 1;
                if file.text.as_bytes()[end] == b'\r'
                    && file.text.as_bytes().get(start) == Some(&b'\n')
                {
                    start += 1;
                }
                line_number += 1;
            }
        }
    }
    check(cancel)?;
    result.counts.read_bytes = limits.read - budget;
    result
        .hits
        .sort_by(|a, b| a.path.cmp(&b.path).then(a.bytes.start.cmp(&b.bytes.start)));
    Ok(result)
}
/// Revalidate on the worker immediately before the existing asynchronous Open.
pub fn verify(hit: &Hit, cancel: &AtomicBool) -> Result<()> {
    let text = read_text(&hit.root, &hit.path, cancel, &mut (MAX_FILE + 1))?;
    if text.text.len() != hit.text_len
        || text.raw_digest != hit.raw_digest
        || blake3::hash(text.text.as_bytes()) != hit.text_digest
        || hit.bytes.start >= hit.bytes.end
        || text.text.get(hit.bytes.clone()).is_none()
    {
        return Err("Search result changed on disk; search again".into());
    }
    Ok(())
}

#[cfg(test)]
#[path = "text_search_tests.rs"]
mod tests;
