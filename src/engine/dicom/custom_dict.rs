//! Bounded replacement text dictionaries. Factory tables remain allocation-free.
use super::dictionary::{self, Entry};
use crate::Result;
use std::{
    collections::{BTreeMap, BTreeSet},
    fs::File,
    io::Read,
    path::{Path, PathBuf},
    sync::{Arc, atomic::AtomicBool},
};

pub(crate) const MAX_BYTES: usize = 16 * 1024 * 1024;
const MAX_ENTRIES: usize = 65536;
const MAX_REPEATERS: usize = 1024;
const MAX_LINE: usize = 4096;

#[derive(Clone, Default)]
pub(crate) enum Dictionary {
    #[default]
    Factory,
    Custom(Arc<CustomDictionary>),
}

pub(crate) struct CustomDictionary {
    path: PathBuf,
    entries: Vec<CustomEntry>,
    keywords: Vec<usize>,
    repeaters: Vec<usize>,
    memory: usize,
}
struct CustomEntry {
    value: u32,
    mask: u32,
    keyword: String,
    vrs: Vec<[u8; 2]>,
}

#[derive(Clone, Copy)]
pub(crate) struct Definition<'a> {
    pub(crate) value: u32,
    pub(crate) mask: u32,
    pub(crate) keyword: &'a str,
    pub(crate) vrs: &'a [[u8; 2]],
}
impl<'a> From<&'a Entry> for Definition<'a> {
    fn from(entry: &'a Entry) -> Self {
        Self {
            value: entry.value,
            mask: entry.mask,
            keyword: entry.keyword,
            vrs: entry.vrs,
        }
    }
}
impl<'a> From<&'a CustomEntry> for Definition<'a> {
    fn from(entry: &'a CustomEntry) -> Self {
        Self {
            value: entry.value,
            mask: entry.mask,
            keyword: &entry.keyword,
            vrs: &entry.vrs,
        }
    }
}
impl Definition<'_> {
    pub(crate) fn field_name(self, tag: u32) -> String {
        dictionary::field_name_parts(self.keyword, self.value, self.mask, tag)
    }
    pub(crate) fn implicit_vr(self, tag: u32, representation: u16) -> [u8; 2] {
        dictionary::implicit_vr_parts(self.vrs, tag, representation)
    }
}
impl Dictionary {
    pub(crate) fn lookup(&self, tag: u32) -> Option<Definition<'_>> {
        match self {
            Self::Factory => dictionary::lookup(tag).map(Definition::from),
            Self::Custom(custom) => {
                if let Ok(index) = custom
                    .entries
                    .binary_search_by_key(&(tag, u32::MAX), |e| (e.value, e.mask))
                {
                    return Some((&custom.entries[index]).into());
                }
                custom
                    .repeaters
                    .iter()
                    .map(|&i| &custom.entries[i])
                    .find(|e| tag & e.mask == e.value)
                    .map(Definition::from)
            }
        }
    }
    fn by_keyword(&self, keyword: &str) -> Option<Definition<'_>> {
        match self {
            Self::Factory => dictionary::by_keyword(keyword).map(Definition::from),
            Self::Custom(custom) => {
                let index = custom
                    .keywords
                    .binary_search_by(|&i| custom.entries[i].keyword.as_str().cmp(keyword))
                    .ok()?;
                Some((&custom.entries[custom.keywords[index]]).into())
            }
        }
    }
    pub(crate) fn tag_by_name(&self, name: &str) -> Option<u32> {
        if let Some(entry) = self.by_keyword(name) {
            return Some(entry.value);
        }
        let (keyword, suffix) = name.rsplit_once('_')?;
        let entry = self.by_keyword(keyword)?;
        if entry.mask == u32::MAX || suffix.is_empty() || suffix.len() > 8 {
            return None;
        }
        let wildcard = u32::from_str_radix(suffix, 16).ok()?;
        let wildcard = if entry.mask & 0xFFFF == 0xFFFF {
            wildcard.checked_mul(65536)?
        } else {
            wildcard
        };
        if wildcard & entry.mask != 0 {
            return None;
        }
        let tag = entry.value | wildcard;
        let found = self.lookup(tag)?;
        (found.field_name(tag) == name).then_some(tag)
    }
    pub(crate) fn is_custom(&self) -> bool {
        matches!(self, Self::Custom(_))
    }
    pub(crate) fn memory_bytes(&self) -> usize {
        match self {
            Self::Factory => 0,
            Self::Custom(c) => c.memory,
        }
    }
    /// Custom: the canonical file given to 'set'. Factory: a verified generated
    /// copy of the embedded text (see `factory_file`).
    pub(crate) fn path(&self, cancel: &AtomicBool) -> Result<PathBuf> {
        match self {
            Self::Custom(c) => Ok(c.path.clone()),
            Self::Factory => factory_file(cancel),
        }
    }
}

/// The compiled factory table as editable text, generated from the same cached
/// PS3.6 source: `gen_dictionary.py --text-output assets/dicom/dicom-dict.txt`.
#[cfg(not(target_arch = "wasm32"))]
const FACTORY_TEXT: &str = include_str!("../../../assets/dicom/dicom-dict.txt");

/// `<factory_base>/<first 16 hex of the text's SHA-256>/dicom-dict.txt`.
/// The digest keeps different editions apart; equal bytes share one file.
#[cfg(not(target_arch = "wasm32"))]
fn factory_file(cancel: &AtomicBool) -> Result<PathBuf> {
    static FOLDER: std::sync::OnceLock<String> = std::sync::OnceLock::new();
    let folder = FOLDER.get_or_init(|| {
        let mut digest = crate::bench::sha256_hex(FACTORY_TEXT.as_bytes());
        digest.truncate(16);
        digest
    });
    let folder = factory_base()?.join(folder);
    materialize(&std::path::absolute(&folder).unwrap_or(folder), cancel)
}
/// Per-user base folder. Windows and macOS temp folders are already per user;
/// on other Unix systems /tmp is shared, so use the user's cache folder there.
#[cfg(not(target_arch = "wasm32"))]
fn factory_base() -> Result<PathBuf> {
    #[cfg(all(unix, not(target_os = "macos")))]
    {
        let cache = std::env::var_os("XDG_CACHE_HOME")
            .filter(|v| Path::new(v).is_absolute())
            .map(PathBuf::from)
            .or_else(|| std::env::var_os("HOME").map(|h| PathBuf::from(h).join(".cache")));
        cache
            .filter(|c| c.is_absolute())
            .map(|cache| cache.join("matrust").join("dicom"))
            .ok_or_else(|| {
                "dicomdict: no absolute per-user cache directory; set XDG_CACHE_HOME or HOME".into()
            })
    }
    #[cfg(not(all(unix, not(target_os = "macos"))))]
    Ok(std::env::temp_dir().join("matrust-dicom"))
}
#[cfg(target_arch = "wasm32")]
fn factory_file(_cancel: &AtomicBool) -> Result<PathBuf> {
    Err(
        "dicomdict: the factory dictionary file needs file access, which the browser version lacks"
            .into(),
    )
}

/// Reuse a byte-identical copy; otherwise publish one by temporary file and
/// atomic rename, so concurrent Engines and processes only ever see whole text.
#[cfg(not(target_arch = "wasm32"))]
fn materialize(folder: &Path, cancel: &AtomicBool) -> Result<PathBuf> {
    crate::dsp::check_cancel(cancel)?;
    let path = folder.join("dicom-dict.txt");
    if is_factory_text(&path) {
        return Ok(path);
    }
    let written = std::fs::create_dir_all(folder)
        .map_err(|e| e.to_string())
        .and_then(|()| {
            crate::files::atomic_write_with_cancel(&path, FACTORY_TEXT.as_bytes(), cancel)
        });
    match written {
        Ok(()) => Ok(path),
        // Another Engine published the same bytes while this one was writing.
        Err(_) if is_factory_text(&path) => Ok(path),
        Err(e) => Err(format!(
            "dicomdict: cannot write the factory dictionary copy '{}': {e}",
            path.display()
        )),
    }
}

/// Regular files only: never follow a planted link or block on a FIFO.
#[cfg(not(target_arch = "wasm32"))]
fn is_factory_text(path: &Path) -> bool {
    let expected = FACTORY_TEXT.as_bytes();
    if !std::fs::symlink_metadata(path)
        .is_ok_and(|stat| stat.is_file() && stat.len() == expected.len() as u64)
    {
        return false;
    }
    let Ok(mut file) = File::open(path) else {
        return false;
    };
    let mut chunk = [0u8; 32768];
    let mut offset = 0;
    loop {
        match file.read(&mut chunk) {
            Ok(0) => return offset == expected.len(),
            Ok(n) if expected[offset..].starts_with(&chunk[..n]) => offset += n,
            _ => return false,
        }
    }
}

fn error(path: &Path, line: usize, message: impl std::fmt::Display) -> String {
    format!(
        "DICOM dictionary '{}', line {line}: {message}",
        path.display()
    )
}
fn valid_vr(vr: &str) -> bool {
    matches!(
        vr,
        "AE" | "AS"
            | "AT"
            | "CS"
            | "DA"
            | "DS"
            | "DT"
            | "FD"
            | "FL"
            | "IS"
            | "LO"
            | "LT"
            | "OB"
            | "OD"
            | "OF"
            | "OL"
            | "OV"
            | "OW"
            | "PN"
            | "SH"
            | "SL"
            | "SQ"
            | "SS"
            | "ST"
            | "SV"
            | "TM"
            | "UC"
            | "UI"
            | "UL"
            | "UN"
            | "UR"
            | "US"
            | "UT"
            | "UV"
    )
}
fn positive(s: &str) -> Option<u32> {
    if s.is_empty() || !s.bytes().all(|b| b.is_ascii_digit()) {
        return None;
    }
    s.parse::<u32>().ok().filter(|n| *n > 0 && *n <= 1_000_000)
}
fn valid_vm(vm: &str) -> bool {
    vm.split(" or ").all(|part| {
        if positive(part).is_some() {
            return true;
        }
        let Some((low, high)) = part.split_once('-') else {
            return false;
        };
        let Some(low) = positive(low) else {
            return false;
        };
        if high == "n" {
            return true;
        }
        if let Some(multiple) = high.strip_suffix('n') {
            return positive(multiple).is_some_and(|n| n >= low);
        }
        positive(high).is_some_and(|n| n >= low)
    })
}
fn tag_pattern(s: &str) -> Option<(u32, u32)> {
    let b = s.as_bytes();
    if b.len() != 11 || b[0] != b'(' || b[5] != b',' || b[10] != b')' {
        return None;
    }
    let (mut value, mut mask) = (0u32, 0u32);
    for &byte in b[1..5].iter().chain(&b[6..10]) {
        value <<= 4;
        mask <<= 4;
        if matches!(byte, b'x' | b'X') {
            continue;
        }
        value |= char::from(byte).to_digit(16)?;
        mask |= 15;
    }
    Some((value, mask))
}

pub(crate) fn load(path: &Path, budget: usize, cancel: &AtomicBool) -> Result<Dictionary> {
    crate::dsp::check_cancel(cancel).map_err(|e| error(path, 1, e))?;
    if path
        .extension()
        .is_some_and(|e| e.eq_ignore_ascii_case("mat"))
    {
        return Err(error(
            path,
            1,
            "MAT-format dictionaries are not supported yet; use a .txt dictionary",
        ));
    }
    if !path
        .extension()
        .is_some_and(|e| e.eq_ignore_ascii_case("txt"))
    {
        return Err(error(
            path,
            1,
            "unsupported dictionary format; expected a .txt text dictionary",
        ));
    }
    // Reject directories/devices/FIFOs before open can block on a non-file.
    if !std::fs::metadata(path)
        .map_err(|e| error(path, 1, e))?
        .is_file()
    {
        return Err(error(path, 1, "expected a regular text file"));
    }
    let mut file = File::open(path).map_err(|e| error(path, 1, e))?;
    let stat = file.metadata().map_err(|e| error(path, 1, e))?;
    if !stat.is_file() {
        return Err(error(path, 1, "expected a regular text file"));
    }
    let size =
        usize::try_from(stat.len()).map_err(|_| error(path, 1, "file size limit exceeded"))?;
    if size > MAX_BYTES {
        return Err(error(path, 1, "16 MiB file size limit exceeded"));
    }
    if size.saturating_mul(4).saturating_add(4096) > budget {
        return Err(error(path, 1, "dictionary memory budget exceeded"));
    }
    let mut bytes = Vec::new();
    bytes
        .try_reserve_exact(size.saturating_add(1))
        .map_err(|_| error(path, 1, "text allocation failed"))?;
    let mut chunk = [0u8; 32768];
    loop {
        crate::dsp::check_cancel(cancel).map_err(|e| error(path, 1, e))?;
        let remaining = (MAX_BYTES + 1).saturating_sub(bytes.len()).min(chunk.len());
        let n = file
            .read(&mut chunk[..remaining])
            .map_err(|e| error(path, 1, e))?;
        if n == 0 {
            break;
        }
        bytes.extend_from_slice(&chunk[..n]);
        if bytes.len() > MAX_BYTES {
            return Err(error(path, 1, "16 MiB file size limit exceeded"));
        }
        if bytes.len().saturating_mul(4).saturating_add(4096) > budget {
            return Err(error(path, 1, "dictionary memory budget exceeded"));
        }
    }
    if bytes.starts_with(b"MATLAB") || bytes.starts_with(b"\x89HDF") {
        return Err(error(
            path,
            1,
            "MAT-format dictionaries are not supported yet; use text",
        ));
    }
    let text = std::str::from_utf8(&bytes).map_err(|e| {
        error(
            path,
            1 + bytes[..e.valid_up_to()]
                .iter()
                .filter(|&&b| b == b'\n')
                .count(),
            "invalid UTF-8 text / binary dictionary",
        )
    })?;
    let mut entries = Vec::new();
    let mut tags = BTreeMap::new();
    let mut keywords = BTreeSet::new();
    let mut repeat_count = 0;
    let mut memory = 4096usize;
    for (index, raw) in text.lines().enumerate() {
        let line = index + 1;
        crate::dsp::check_cancel(cancel).map_err(|e| error(path, line, e))?;
        if raw.len() > MAX_LINE {
            return Err(error(path, line, "4096-byte line limit exceeded"));
        }
        if raw.chars().any(|c| c.is_control() && c != '\t') {
            return Err(error(path, line, "binary/control bytes in text dictionary"));
        }
        let row = raw.split('#').next().unwrap_or("").trim();
        if row.is_empty() {
            continue;
        }
        let mut columns = row.split_whitespace();
        let (value, mask) = columns.next().and_then(tag_pattern).ok_or_else(|| {
            error(
                path,
                line,
                "invalid tag; expected (gggg,eeee) with hexadecimal digits or x masks",
            )
        })?;
        let vr = columns
            .next()
            .ok_or_else(|| error(path, line, "missing VR"))?;
        let mut vrs = Vec::new();
        for vr in vr.split('/') {
            if !valid_vr(vr) || vrs.len() >= 3 {
                return Err(error(path, line, "invalid PS3.5 VR code"));
            }
            vrs.push([vr.as_bytes()[0], vr.as_bytes()[1]]);
        }
        let keyword = columns
            .next()
            .ok_or_else(|| error(path, line, "missing keyword"))?;
        if keyword.len() > 63
            || !keyword
                .as_bytes()
                .first()
                .is_some_and(u8::is_ascii_alphabetic)
            || !keyword
                .bytes()
                .all(|b| b.is_ascii_alphanumeric() || b == b'_')
        {
            return Err(error(
                path,
                line,
                "invalid keyword identifier (1..63 ASCII identifier characters)",
            ));
        }
        if !columns.next().is_some_and(valid_vm) {
            return Err(error(path, line, "invalid or missing VM"));
        }
        while let Some(separator) = columns.next() {
            if separator != "or" || !columns.next().is_some_and(valid_vm) {
                return Err(error(path, line, "invalid VM alternatives"));
            }
        }
        if entries.len() >= MAX_ENTRIES {
            return Err(error(path, line, "65536-entry limit exceeded"));
        }
        memory = memory.saturating_add(512 + keyword.len() * 2);
        if bytes.capacity().saturating_mul(2).saturating_add(memory) > budget {
            return Err(error(path, line, "dictionary memory budget exceeded"));
        }
        if let Some(previous) = tags.insert((value, mask), line) {
            return Err(error(
                path,
                line,
                format!("duplicate tag (first defined at line {previous})"),
            ));
        }
        if !keywords.insert(keyword.to_owned()) {
            return Err(error(path, line, "duplicate keyword"));
        }
        if mask != u32::MAX {
            repeat_count += 1;
        }
        if repeat_count > MAX_REPEATERS {
            return Err(error(path, line, "1024-repeater limit exceeded"));
        }
        entries
            .try_reserve(1)
            .map_err(|_| error(path, line, "entry allocation failed"))?;
        entries.push(CustomEntry {
            value,
            mask,
            keyword: keyword.to_owned(),
            vrs,
        });
    }
    if entries.is_empty() {
        return Err(error(path, 1, "empty dictionary"));
    }
    entries.sort_by_key(|e| (e.value, e.mask));
    let mut keywords: Vec<_> = (0..entries.len()).collect();
    keywords.sort_by(|&a, &b| entries[a].keyword.cmp(&entries[b].keyword));
    let mut repeaters: Vec<_> = (0..entries.len())
        .filter(|&i| entries[i].mask != u32::MAX)
        .collect();
    // Exact tags win; the most specific overlapping mask wins deterministically.
    repeaters.sort_by_key(|&i| {
        (
            std::cmp::Reverse(entries[i].mask.count_ones()),
            entries[i].value,
            entries[i].mask,
        )
    });
    crate::dsp::check_cancel(cancel).map_err(|e| error(path, 1, e))?;
    let path = path.canonicalize().map_err(|e| error(path, 1, e))?;
    Ok(Dictionary::Custom(Arc::new(CustomDictionary {
        path,
        entries,
        keywords,
        repeaters,
        memory,
    })))
}

#[cfg(all(test, not(target_arch = "wasm32")))]
mod tests {
    use super::*;

    fn published(path: &Path) -> (std::time::SystemTime, u64) {
        let stat = std::fs::symlink_metadata(path).unwrap();
        assert!(stat.is_file());
        #[cfg(unix)]
        let file = std::os::unix::fs::MetadataExt::ino(&stat);
        #[cfg(not(unix))]
        let file = 0;
        (stat.modified().unwrap(), file)
    }

    #[test]
    fn factory_copy_is_reused_and_rewritten_when_corrupted() {
        let cancel = AtomicBool::new(false);
        let dir = tempfile::tempdir().unwrap();
        let folder = dir.path().join("matrust-dicom").join("0123456789abcdef");
        let path = materialize(&folder, &cancel).unwrap();
        assert_eq!(path, folder.join("dicom-dict.txt"));
        assert_eq!(std::fs::read(&path).unwrap(), FACTORY_TEXT.as_bytes());
        // A byte-identical copy is reused, never republished.
        let first = published(&path);
        assert_eq!(materialize(&folder, &cancel).unwrap(), path);
        assert_eq!(published(&path), first);
        // Same-length damage, truncation, appended rows and an empty file.
        let text = FACTORY_TEXT.as_bytes();
        let mut flipped = text.to_vec();
        flipped[text.len() / 2] ^= 1;
        for damaged in [
            flipped,
            text[..1000].to_vec(),
            [text, b"(0011,1001) US Extra 1\n"].concat(),
            Vec::new(),
        ] {
            std::fs::write(&path, &damaged).unwrap();
            assert_eq!(materialize(&folder, &cancel).unwrap(), path);
            assert_eq!(std::fs::read(&path).unwrap(), text);
        }
        // Publication leaves no temporary file beside the copy.
        assert_eq!(std::fs::read_dir(&folder).unwrap().count(), 1);
    }

    #[cfg(unix)]
    #[test]
    fn factory_copy_replaces_a_planted_link_without_following_it() {
        let dir = tempfile::tempdir().unwrap();
        let victim = dir.path().join("victim.txt");
        std::fs::write(&victim, b"victim").unwrap();
        let path = dir.path().join("dicom-dict.txt");
        std::os::unix::fs::symlink(&victim, &path).unwrap();
        assert_eq!(
            materialize(dir.path(), &AtomicBool::new(false)).unwrap(),
            path
        );
        published(&path);
        assert_eq!(std::fs::read(&path).unwrap(), FACTORY_TEXT.as_bytes());
        assert_eq!(std::fs::read(&victim).unwrap(), b"victim");
    }

    #[test]
    fn factory_copy_errors_name_the_path_and_cancellation_writes_nothing() {
        let dir = tempfile::tempdir().unwrap();
        let blocker = dir.path().join("blocker");
        std::fs::write(&blocker, b"a file where a folder is needed").unwrap();
        let folder = blocker.join("0123456789abcdef");
        let error = materialize(&folder, &AtomicBool::new(false)).unwrap_err();
        let path = folder.join("dicom-dict.txt").display().to_string();
        assert!(
            error.starts_with("dicomdict: cannot write the factory dictionary copy")
                && error.contains(&path),
            "{error}"
        );
        let folder = dir.path().join("cancelled");
        let error = materialize(&folder, &AtomicBool::new(true)).unwrap_err();
        assert!(error.contains("cancelled"), "{error}");
        assert!(!folder.exists());
    }

    #[test]
    fn factory_text_parses_back_to_the_compiled_table() {
        let cancel = AtomicBool::new(false);
        let dir = tempfile::tempdir().unwrap();
        let path = materialize(dir.path(), &cancel).unwrap();
        let Dictionary::Custom(custom) = load(&path, 64 << 20, &cancel).unwrap() else {
            panic!("expected a parsed text table")
        };
        assert_eq!(custom.entries.len(), 5299);
        assert_eq!(
            custom.entries.len(),
            dictionary::EXACT.len() + dictionary::REPEATERS.len()
        );
        assert_eq!(custom.repeaters.len(), dictionary::REPEATERS.len());
        for entry in &custom.entries {
            let compiled = dictionary::by_keyword(&entry.keyword).unwrap();
            assert_eq!(
                (compiled.value, compiled.mask, compiled.vrs),
                (entry.value, entry.mask, entry.vrs.as_slice()),
                "{}",
                entry.keyword
            );
        }
    }
}
