//! Immutable TIFF snapshots. Only the first-directory pointer is overlaid for
//! the existing image decoder; samples, tags and file bytes are never rewritten.
use super::image_load::{self, Preview, check};
use image::ImageDecoder;
use matrust::Result;
use std::{
    collections::HashSet,
    io::{self, BufReader, Read, Seek, SeekFrom},
    path::{Path, PathBuf},
    sync::{Arc, atomic::AtomicBool},
};

pub const MAX_PAGES: usize = 1000;
const MAX_PAGE_TAGS: u64 = 4096;
const MAX_TOTAL_TAGS: u64 = 100_000;
const MAX_READ_BYTES: u64 = 512 * 1024 * 1024;
pub struct Document {
    pub path: PathBuf,
    pub page_count: u32,
    bytes: Arc<Vec<u8>>,
    original: image_load::Original,
    offsets: Vec<u64>,
    little: bool,
    big: bool,
}
fn number(bytes: &[u8], offset: u64, width: usize, little: bool) -> Result<u64> {
    let start = usize::try_from(offset).map_err(|_| "TIFF offset is too large")?;
    let end = start.checked_add(width).ok_or("TIFF offset overflow")?;
    let source = bytes.get(start..end).ok_or("Truncated TIFF directory")?;
    let mut value = [0; 8];
    if little {
        value[..width].copy_from_slice(source);
        Ok(u64::from_le_bytes(value))
    } else {
        value[8 - width..].copy_from_slice(source);
        Ok(u64::from_be_bytes(value))
    }
}
pub fn open(path: &Path, cancel: &AtomicBool) -> Result<Arc<Document>> {
    let bytes = image_load::read_file(path, image_load::MAX_FILE_BYTES, cancel)?;
    let little = match bytes.get(..2) {
        Some(b"II") => true,
        Some(b"MM") => false,
        _ => return Err("Expected a TIFF document".into()),
    };
    let big = match number(&bytes, 2, 2, little)? {
        42 => false,
        43 if number(&bytes, 4, 2, little)? == 8 && number(&bytes, 6, 2, little)? == 0 => true,
        _ => return Err("Unsupported TIFF header".into()),
    };
    let (pointer_start, pointer_width, count_width, entry_width, header_len) = if big {
        (8, 8, 8, 20, 16)
    } else {
        (4, 4, 2, 12, 8)
    };
    let mut offset = number(&bytes, pointer_start, pointer_width, little)?;
    let mut offsets = Vec::new();
    let mut visited = HashSet::new();
    let mut total_tags = 0;
    while offset != 0 {
        check(cancel)?;
        if offsets.len() >= MAX_PAGES {
            return Err("TIFF exceeds 1000 pages".into());
        }
        if offset < header_len || !visited.insert(offset) {
            return Err("Invalid or cyclic TIFF page directory".into());
        }
        let count = number(&bytes, offset, count_width, little)?;
        if count > MAX_PAGE_TAGS || total_tags + count > MAX_TOTAL_TAGS {
            return Err("TIFF directory metadata exceeds supported limits".into());
        }
        total_tags += count;
        let next = offset
            .checked_add(count_width as u64)
            .and_then(|n| n.checked_add(count * entry_width))
            .ok_or("TIFF directory overflow")?;
        offsets.push(offset);
        offset = number(&bytes, next, pointer_width, little)?;
    }
    if offsets.is_empty() {
        return Err("TIFF has no pages".into());
    }
    let mut original = image_load::Original::identity(&bytes, cancel)?;
    let bytes = Arc::new(bytes);
    original.bytes = Some(bytes.clone());
    Ok(Arc::new(Document {
        original,
        path: path.to_owned(),
        page_count: offsets.len() as u32,
        bytes,
        offsets,
        little,
        big,
    }))
}

struct PageReader<'a> {
    bytes: &'a [u8],
    position: u64,
    patch: [u8; 8],
    patch_start: usize,
    patch_len: usize,
    read_bytes: u64,
    cancel: &'a AtomicBool,
}
impl Read for PageReader<'_> {
    fn read(&mut self, out: &mut [u8]) -> io::Result<usize> {
        check(self.cancel).map_err(io::Error::other)?;
        let start = usize::try_from(self.position).unwrap_or(usize::MAX);
        let source = self.bytes.get(start..).unwrap_or_default();
        let count = out.len().min(source.len());
        self.read_bytes += count as u64;
        if self.read_bytes > MAX_READ_BYTES {
            return Err(io::Error::other("TIFF decoder exceeded its read budget"));
        }
        out[..count].copy_from_slice(&source[..count]);
        if count > 0 {
            let a = start.max(self.patch_start);
            let b = (start + count).min(self.patch_start + self.patch_len);
            if a < b {
                out[a - start..b - start]
                    .copy_from_slice(&self.patch[a - self.patch_start..b - self.patch_start]);
            }
        }
        self.position += count as u64;
        Ok(count)
    }
}
impl Seek for PageReader<'_> {
    fn seek(&mut self, seek: SeekFrom) -> io::Result<u64> {
        check(self.cancel).map_err(io::Error::other)?;
        let next = match seek {
            SeekFrom::Start(p) => i128::from(p),
            SeekFrom::Current(p) => i128::from(self.position) + i128::from(p),
            SeekFrom::End(p) => self.bytes.len() as i128 + i128::from(p),
        };
        self.position = u64::try_from(next).map_err(|_| io::Error::other("Invalid TIFF seek"))?;
        Ok(self.position)
    }
}
pub fn render(
    document: &Document,
    index: u32,
    texture_side: usize,
    cancel: &AtomicBool,
) -> Result<Preview> {
    check(cancel)?;
    let offset = *document
        .offsets
        .get(index as usize)
        .ok_or("TIFF page is out of range")?;
    let encoded = if document.little {
        offset.to_le_bytes()
    } else {
        offset.to_be_bytes()
    };
    let mut patch = [0; 8];
    let (patch_start, patch_len) = if document.big { (8, 8) } else { (4, 4) };
    patch[..patch_len].copy_from_slice(if !document.big && !document.little {
        &encoded[4..]
    } else {
        &encoded[..patch_len]
    });
    let reader = BufReader::new(PageReader {
        bytes: &document.bytes,
        position: 0,
        patch,
        patch_start,
        patch_len,
        read_bytes: 0,
        cancel,
    });
    let mut decoder = image::codecs::tiff::TiffDecoder::new(reader).map_err(|e| e.to_string())?;
    let mut limits = image::Limits::default();
    limits.max_image_width = Some(8192);
    limits.max_image_height = Some(8192);
    limits.max_alloc = Some(256 * 1024 * 1024);
    decoder.set_limits(limits).map_err(|e| e.to_string())?;
    let (width, height) = decoder.dimensions();
    image_load::dimensions(width, height, decoder.total_bytes())?;
    let orientation = decoder.orientation().map_err(|e| e.to_string())?;
    let decoded = image::DynamicImage::from_decoder(decoder).map_err(|e| e.to_string())?;
    let mut preview = image_load::prepare(
        decoded,
        orientation,
        &document.path,
        "TIFF (page)",
        document.bytes.len(),
        texture_side,
        cancel,
    )?;
    preview.original = Some(document.original.clone());
    Ok(preview)
}

#[cfg(test)]
mod tests {
    use super::*;
    fn fixture(name: &str) -> PathBuf {
        Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("tests/fixtures/preview/tiff-preview")
            .join(name)
    }
    #[test]
    fn tiff_pages_match_independent_pixels_endian_bigtiff_alpha_and_orientation() {
        let cancel = AtomicBool::new(false);
        for name in ["pages", "big-endian", "big-tiff", "big-tiff-endian"] {
            let document = open(&fixture(&format!("{name}.tiff")), &cancel).unwrap();
            assert_eq!(document.page_count, 3);
            for index in 0..3 {
                let preview = render(&document, index, 4096, &cancel).unwrap();
                let reference = image::open(fixture(&format!("reference-{}.png", index + 1)))
                    .unwrap()
                    .into_rgba8();
                assert_eq!(
                    preview.original_size,
                    [reference.width(), reference.height()]
                );
                for (actual, expected) in preview.pixels.pixels.iter().zip(reference.pixels()) {
                    assert_eq!(
                        actual.to_srgba_unmultiplied(),
                        expected.0,
                        "{name}, page {index}"
                    );
                }
                let small = render(&document, index, 96, &cancel).unwrap();
                assert!(small.pixels.size.iter().all(|&s| s <= 96));
            }
        }
    }
    #[test]
    fn tiff_snapshot_survives_disk_changes_and_releases_shared_bytes() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("pages.tiff");
        std::fs::copy(fixture("pages.tiff"), &path).unwrap();
        let cancel = AtomicBool::new(false);
        let document = open(&path, &cancel).unwrap();
        let pointer = document.bytes.as_ptr();
        let alias = document.clone();
        let weak = Arc::downgrade(&document);
        std::fs::write(&path, b"broken").unwrap();
        assert!(open(&path, &cancel).is_err());
        std::fs::remove_file(path).unwrap();
        drop(document);
        assert_eq!(alias.bytes.as_ptr(), pointer);
        assert!(render(&alias, 2, 4096, &cancel).is_ok());
        drop(alias);
        assert!(weak.upgrade().is_none());
    }
    #[test]
    fn tiff_rejects_cycles_truncation_excess_pages_metadata_dimensions_and_cancellation() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("invalid.tiff");
        let cancel = AtomicBool::new(false);
        assert!(matches!(open(&path, &AtomicBool::new(true)),Err(e) if e == "Cancelled"));
        for bytes in [
            b"II*\0\x08\0\0\0\0\0\x08\0\0\0".to_vec(),
            b"II*\0\x08\0\0\0\xff\xff".to_vec(),
            b"MM\0+\0\x04\0\0".to_vec(),
        ] {
            std::fs::write(&path, bytes).unwrap();
            assert!(open(&path, &cancel).is_err());
        }
        let mut bytes = b"II*\0\x08\0\0\0".to_vec();
        for i in 0..=MAX_PAGES {
            bytes.extend_from_slice(&0u16.to_le_bytes());
            bytes.extend_from_slice(
                &(if i == MAX_PAGES {
                    0
                } else {
                    8 + (i as u32 + 1) * 6
                })
                .to_le_bytes(),
            );
        }
        std::fs::write(&path, bytes).unwrap();
        assert!(matches!(open(&path,&cancel),Err(e) if e.contains("1000 pages")));
        let doc = open(&fixture("invalid-later.tiff"), &cancel).unwrap();
        assert!(render(&doc, 0, 4096, &cancel).is_ok());
        assert!(render(&doc, 1, 4096, &cancel).is_err());
        assert!(render(&doc, 2, 4096, &cancel).is_err());
        assert!(matches!(render(&doc,0,4096,&AtomicBool::new(true)),Err(e) if e == "Cancelled"));
        std::fs::File::create(&path)
            .unwrap()
            .set_len(image_load::MAX_FILE_BYTES as u64 + 1)
            .unwrap();
        assert!(matches!(open(&path,&cancel),Err(e) if e.contains("128 MiB")));
    }
    #[test]
    fn tiff_page_reader_bounds_repeated_reads_and_checks_cancellation() {
        let cancel = AtomicBool::new(false);
        let mut reader = PageReader {
            bytes: &[1; 32],
            position: 2,
            patch: [9; 8],
            patch_start: 4,
            patch_len: 4,
            read_bytes: 0,
            cancel: &cancel,
        };
        let mut bytes = [0; 8];
        assert_eq!(reader.read(&mut bytes).unwrap(), 8);
        assert_eq!(bytes, [1, 1, 9, 9, 9, 9, 1, 1]);
        reader.seek(SeekFrom::Start(u64::MAX)).unwrap();
        assert_eq!(reader.read(&mut bytes).unwrap(), 0);
        reader.seek(SeekFrom::Start(0)).unwrap();
        reader.read_bytes = MAX_READ_BYTES - 1;
        assert!(reader.read(&mut bytes).is_err());
        reader.read_bytes = 0;
        cancel.store(true, std::sync::atomic::Ordering::Relaxed);
        assert!(reader.read(&mut bytes).is_err());
        assert!(reader.seek(SeekFrom::Start(0)).is_err());
    }
}
