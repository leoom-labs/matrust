use super::checkpoint as check_cancel;
use crate::Result;
use std::{
    collections::BTreeSet,
    io::{Cursor, Read},
    path::Path,
    sync::atomic::AtomicBool,
};
use zip::{
    CompressionMethod, ZipArchive,
    read::{ArchiveOffset, Config},
};
const RAW: usize = 16 * 1024 * 1024;
const PART: usize = 8 * 1024 * 1024;
const TOTAL: u64 = 32 * 1024 * 1024;

pub(super) fn read(path: &Path, cancel: &AtomicBool) -> Result<Vec<u8>> {
    check_cancel(cancel)?;
    let mut file = std::fs::File::open(path).map_err(|e| e.to_string())?;
    let before = file.metadata().map_err(|e| e.to_string())?;
    if !before.is_file() || before.len() > RAW as u64 {
        return Err("SLX: regular archive at most 16 MiB required".into());
    }
    let mut bytes = Vec::new();
    bytes
        .try_reserve_exact(before.len() as usize)
        .map_err(|_| "SLX: archive allocation failed")?;
    let mut chunk = [0; 32768];
    loop {
        check_cancel(cancel)?;
        let n = file.read(&mut chunk).map_err(|e| e.to_string())?;
        if n == 0 {
            break;
        }
        if bytes.len().saturating_add(n) > RAW {
            return Err("SLX: archive grew beyond 16 MiB".into());
        }
        bytes.extend_from_slice(&chunk[..n]);
    }
    let after = file.metadata().map_err(|e| e.to_string())?;
    if before.len() != after.len()
        || before.modified().ok() != after.modified().ok()
        || bytes.len() as u64 != before.len()
    {
        return Err("SLX: source changed while opening; retry explicitly".into());
    }
    check_cancel(cancel)?;
    Ok(bytes)
}

/// Admission only: cap the classic directory before ZipArchive allocates from its count.
/// The proven ZIP reader still parses/validates all headers and owns inflation/CRC.
fn directory_limit(bytes: &[u8], cancel: &AtomicBool) -> Result<usize> {
    if bytes.len() < 22 || bytes.len() > RAW {
        return Err("SLX: invalid or oversized archive".into());
    }
    // zip9 retries earlier EOCD signatures anywhere in the input after a bad
    // central header, and permits trailing bytes after their comments. Admit
    // exactly one interpretation across the entire bounded input before it can
    // allocate a directory; a tail-only scan would not constrain that fallback.
    let mut candidate = None;
    for i in 0..=bytes.len() - 22 {
        if i.is_multiple_of(32768) {
            check_cancel(cancel)?;
        }
        if &bytes[i..i + 4] != b"PK\x05\x06" {
            continue;
        }
        let u16_at = |n| u16::from_le_bytes([bytes[i + n], bytes[i + n + 1]]) as usize;
        if i + 22 + u16_at(20) > bytes.len() {
            continue;
        }
        if candidate.is_some() {
            return Err("SLX: ambiguous ZIP directory footer".into());
        }
        candidate = Some(i);
    }
    let i = candidate.ok_or("SLX: classic single-disk ZIP required (ZIP64 unsupported)")?;
    let u16_at = |n| u16::from_le_bytes([bytes[i + n], bytes[i + n + 1]]) as usize;
    let u32_at = |n| u32::from_le_bytes(bytes[i + n..i + n + 4].try_into().unwrap()) as usize;
    if i + 22 + u16_at(20) != bytes.len() {
        return Err("SLX: trailing bytes after ZIP directory footer".into());
    }
    let count = u16_at(10);
    if u16_at(4) != 0
        || u16_at(6) != 0
        || u16_at(8) != count
        || !(1..=512).contains(&count)
        || u32_at(12) > 256 * 1024
        || u32_at(16).checked_add(u32_at(12)) != Some(i)
        || i >= 20 && &bytes[i - 20..i - 16] == b"PK\x06\x07"
    {
        return Err("SLX: ZIP64/multipart or directory resource limit unsupported".into());
    }
    Ok(count)
}

pub(super) fn part_name(name: &str) -> Result<()> {
    if name.is_empty()
        || name.len() > 256
        || !name.is_ascii()
        || name.contains(['\\', ':', '%', '\0'])
        || name
            .split('/')
            .any(|p| p.is_empty() || p == "." || p == "..")
    {
        return Err("SLX: noncanonical package part name".into());
    }
    Ok(())
}

pub(super) struct Package<'a> {
    archive: ZipArchive<Cursor<&'a [u8]>>,
    cancel: &'a AtomicBool,
    consumed: u64,
}
impl<'a> Package<'a> {
    pub fn new(bytes: &'a [u8], cancel: &'a AtomicBool) -> Result<Self> {
        let expected = directory_limit(bytes, cancel)?;
        check_cancel(cancel)?;
        let mut archive = ZipArchive::with_config(
            Config {
                archive_offset: ArchiveOffset::Known(0),
            },
            Cursor::new(bytes),
        )
        .map_err(|e| format!("SLX ZIP: {e}"))?;
        // Zip deduplicates names internally; comparing to admitted directory count detects that.
        if archive.len() != expected {
            return Err("SLX: duplicate archive member names".into());
        }
        let mut total = 0u64;
        let mut names = BTreeSet::new();
        for i in 0..archive.len() {
            check_cancel(cancel)?;
            let entry = archive
                .by_index_raw(i)
                .map_err(|e| format!("SLX ZIP: {e}"))?;
            let name =
                std::str::from_utf8(entry.name_raw()).map_err(|_| "SLX: non-UTF8 member name")?;
            part_name(name)?;
            if !names.insert(name.to_ascii_lowercase())
                || entry.encrypted()
                || !matches!(
                    entry.compression(),
                    CompressionMethod::Stored | CompressionMethod::Deflated
                )
                || entry
                    .unix_mode()
                    .is_some_and(|m| m & 0o170000 != 0 && m & 0o170000 != 0o100000)
            {
                return Err(
                    "SLX: ambiguous, encrypted, nonregular or unsupported compressed member".into(),
                );
            }
            total = total
                .checked_add(entry.size())
                .ok_or("SLX: expanded size overflow")?;
            if total > TOTAL || entry.size() > PART as u64 {
                return Err("SLX: declared expanded metadata exceeds bounds".into());
            }
        }
        Ok(Self {
            archive,
            cancel,
            consumed: 0,
        })
    }
    pub fn part(&mut self, name: &str) -> Result<Vec<u8>> {
        part_name(name)?;
        check_cancel(self.cancel)?;
        let mut file = self
            .archive
            .by_name(name)
            .map_err(|e| format!("SLX part {name}: {e}"))?;
        let declared = file.size();
        let mut bytes = Vec::new();
        bytes
            .try_reserve_exact(declared as usize)
            .map_err(|_| "SLX: metadata allocation failed")?;
        let mut chunk = [0; 32768];
        loop {
            check_cancel(self.cancel)?;
            let n = file
                .read(&mut chunk)
                .map_err(|e| format!("SLX part {name}: {e}"))?;
            if n == 0 {
                break;
            }
            self.consumed = self
                .consumed
                .checked_add(n as u64)
                .ok_or("SLX: expanded byte overflow")?;
            if bytes.len().saturating_add(n) > PART
                || bytes.len() as u64 + n as u64 > declared
                || self.consumed > TOTAL
            {
                return Err("SLX: actual inflated metadata exceeds bounds".into());
            }
            bytes.extend_from_slice(&chunk[..n]);
        }
        if bytes.len() as u64 != declared {
            return Err("SLX: truncated metadata part".into());
        }
        Ok(bytes)
    }
}
