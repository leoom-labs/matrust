//! Streaming RIFF AVI sink. Index offsets are relative to the movi FOURCC.
use crate::Result;
use std::{
    fs::{File, OpenOptions},
    io::{Seek, SeekFrom, Write},
    path::{Path, PathBuf},
    sync::atomic::{AtomicU64, Ordering},
};

const MAX_MOVI: u64 = 1 << 30;
static NEXT: AtomicU64 = AtomicU64::new(0);

pub(super) struct Sink {
    target: PathBuf,
    temp: PathBuf,
    file: Option<File>,
    pub width: usize,
    pub height: usize,
    rate: f64,
    jpeg: bool,
    palette: Vec<[u8; 3]>,
    index: Vec<(u32, u32)>,
    movi: u64,
    header_len: u64,
    max_chunk: u32,
}
fn io(e: std::io::Error) -> String {
    format!("VideoWriter: {e}")
}
fn word(b: &mut Vec<u8>, n: u16) {
    b.extend_from_slice(&n.to_le_bytes());
}
fn dword(b: &mut Vec<u8>, n: u32) {
    b.extend_from_slice(&n.to_le_bytes());
}
fn chunk(b: &mut Vec<u8>, id: &[u8; 4], data: &[u8]) {
    b.extend_from_slice(id);
    dword(b, data.len() as u32);
    b.extend_from_slice(data);
}
fn list(b: &mut Vec<u8>, id: &[u8; 4], data: &[u8]) {
    b.extend_from_slice(b"LIST");
    dword(b, data.len() as u32 + 4);
    b.extend_from_slice(id);
    b.extend_from_slice(data);
}
impl Sink {
    pub(super) fn new(target: &Path, rate: f64, jpeg: bool) -> Result<Self> {
        super::file_io::check_output(target)?;
        let parent = target
            .parent()
            .ok_or("VideoWriter: invalid target directory")?;
        for _ in 0..100 {
            let temp = parent.join(format!(
                ".matrust-video-{}-{}.tmp",
                std::process::id(),
                NEXT.fetch_add(1, Ordering::Relaxed)
            ));
            match OpenOptions::new().write(true).create_new(true).open(&temp) {
                Ok(file) => {
                    return Ok(Self {
                        target: target.into(),
                        temp,
                        file: Some(file),
                        width: 0,
                        height: 0,
                        rate,
                        jpeg,
                        palette: Vec::new(),
                        index: Vec::new(),
                        movi: 0,
                        header_len: 0,
                        max_chunk: 0,
                    });
                }
                Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => continue,
                Err(e) => return Err(io(e)),
            }
        }
        Err("VideoWriter: cannot create a unique temporary file".into())
    }
    pub(super) fn bytes(&self) -> usize {
        self.index.capacity() * 8 + self.palette.capacity() * 3 + 512
    }
    pub(super) fn frames(&self) -> usize {
        self.index.len()
    }
    fn id(&self) -> &'static [u8; 4] {
        if self.jpeg { b"00dc" } else { b"00db" }
    }
    fn header(&self) -> Vec<u8> {
        let count = self.index.len() as u32;
        let mut avih = Vec::new();
        for n in [
            (1_000_000. / self.rate).round().clamp(1., u32::MAX as f64) as u32,
            (f64::from(self.max_chunk) * self.rate).min(u32::MAX as f64) as u32,
            0,
            0x10,
            count,
            0,
            1,
            self.max_chunk,
            self.width as u32,
            self.height as u32,
            0,
            0,
            0,
            0,
        ] {
            dword(&mut avih, n);
        }
        let mut strh = Vec::new();
        strh.extend_from_slice(b"vids");
        strh.extend_from_slice(if self.jpeg { b"MJPG" } else { b"DIB " });
        dword(&mut strh, 0);
        word(&mut strh, 0);
        word(&mut strh, 0);
        for n in [
            0,
            1_000_000,
            (self.rate * 1_000_000.).round() as u32,
            0,
            count,
            self.max_chunk,
            u32::MAX,
            0,
        ] {
            dword(&mut strh, n);
        }
        for n in [0, 0, self.width as u16, self.height as u16] {
            word(&mut strh, n);
        }
        let mut strf = Vec::new();
        for n in [40, self.width as u32, self.height as u32] {
            dword(&mut strf, n);
        }
        word(&mut strf, 1);
        word(&mut strf, if self.palette.is_empty() { 24 } else { 8 });
        dword(
            &mut strf,
            if self.jpeg {
                u32::from_le_bytes(*b"MJPG")
            } else {
                0
            },
        );
        let row = (self.width * if self.palette.is_empty() { 3 } else { 1 }).div_ceil(4) * 4;
        for n in [
            if self.jpeg {
                self.max_chunk
            } else {
                (row * self.height) as u32
            },
            0,
            0,
            if self.palette.is_empty() { 0 } else { 256 },
            0,
        ] {
            dword(&mut strf, n);
        }
        for i in 0..if self.palette.is_empty() { 0 } else { 256 } {
            let p = self.palette.get(i).copied().unwrap_or([0; 3]);
            strf.extend_from_slice(&[p[2], p[1], p[0], 0]);
        }
        let mut strl = Vec::new();
        chunk(&mut strl, b"strh", &strh);
        chunk(&mut strl, b"strf", &strf);
        let mut hdrl = Vec::new();
        chunk(&mut hdrl, b"avih", &avih);
        list(&mut hdrl, b"strl", &strl);
        let mut out = b"RIFF".to_vec();
        dword(&mut out, 0);
        out.extend_from_slice(b"AVI ");
        list(&mut out, b"hdrl", &hdrl);
        out.extend_from_slice(b"LIST");
        dword(&mut out, (self.movi + 4) as u32);
        out.extend_from_slice(b"movi");
        let size = out.len() as u64 + self.movi + 8 + u64::from(count) * 16 - 8;
        out[4..8].copy_from_slice(&(size as u32).to_le_bytes());
        out
    }
    pub(super) fn configure(&mut self, w: usize, h: usize, palette: &[[u8; 3]]) -> Result<()> {
        self.width = w;
        self.height = h;
        self.palette = palette.to_vec();
        let header = self.header();
        self.header_len = header.len() as u64;
        self.file
            .as_mut()
            .ok_or("VideoWriter: sink is closed")?
            .write_all(&header)
            .map_err(io)
    }
    pub(super) fn append(&mut self, data: &[u8], budget: usize) -> Result<()> {
        let len = u32::try_from(data.len()).map_err(|_| "VideoWriter: frame too large")?;
        let next = self
            .movi
            .checked_add(8 + u64::from(len) + u64::from(len & 1))
            .ok_or("VideoWriter: AVI size overflow")?;
        let count = self
            .index
            .len()
            .checked_add(1)
            .ok_or("VideoWriter: frame count overflow")?;
        if count > 1_000_000 {
            return Err("VideoWriter: native AVI is limited to 1000000 frames".into());
        }
        let total = self
            .header_len
            .checked_add(next)
            .and_then(|n| n.checked_add(8 + count as u64 * 16))
            .ok_or("VideoWriter: AVI size overflow")?;
        if next > MAX_MOVI || total > u64::from(u32::MAX) {
            return Err("VideoWriter: native AVI is limited to 1 GiB of movi data; OpenDML writing is not supported yet".into());
        }
        if self.index.len() == self.index.capacity() {
            let capacity = count.next_power_of_two().max(16);
            if capacity.checked_mul(8).is_none_or(|n| n > budget) {
                return Err("VideoWriter exceeds the evaluation memory limit (AVI index)".into());
            }
            self.index
                .try_reserve_exact(capacity - self.index.len())
                .map_err(|_| "VideoWriter: cannot allocate AVI index")?;
        }
        let id = *self.id();
        let f = self.file.as_mut().ok_or("VideoWriter: sink is closed")?;
        let start = self.header_len + self.movi;
        let result = (|| {
            f.write_all(&id)?;
            f.write_all(&len.to_le_bytes())?;
            f.write_all(data)?;
            if len & 1 != 0 {
                f.write_all(&[0])?;
            }
            Ok::<_, std::io::Error>(())
        })();
        if let Err(e) = result {
            // Restore the last complete frame before finalization on error.
            f.set_len(start).map_err(io)?;
            f.seek(SeekFrom::Start(start)).map_err(io)?;
            return Err(io(e));
        }
        self.index.push(((self.movi + 4) as u32, len));
        self.movi = next;
        self.max_chunk = self.max_chunk.max(len);
        Ok(())
    }
    pub(super) fn finish(&mut self) -> Result<()> {
        let Some(mut file) = self.file.take() else {
            return Ok(());
        };
        let result = (|| {
            if self.index.is_empty() {
                return Ok(());
            }
            file.seek(SeekFrom::Start(self.header_len + self.movi))
                .map_err(io)?;
            file.write_all(b"idx1").map_err(io)?;
            file.write_all(&(self.index.len() as u32 * 16).to_le_bytes())
                .map_err(io)?;
            let mut block = [0u8; 4096];
            for entries in self.index.chunks(256) {
                for (record, &(offset, size)) in
                    block.as_chunks_mut::<16>().0.iter_mut().zip(entries)
                {
                    record[..4].copy_from_slice(self.id());
                    for (field, n) in record[4..]
                        .as_chunks_mut::<4>()
                        .0
                        .iter_mut()
                        .zip([0x10u32, offset, size])
                    {
                        field.copy_from_slice(&n.to_le_bytes());
                    }
                }
                file.write_all(&block[..entries.len() * 16]).map_err(io)?;
            }
            file.seek(SeekFrom::Start(0)).map_err(io)?;
            file.write_all(&self.header()).map_err(io)?;
            file.set_len(self.header_len + self.movi + 8 + self.index.len() as u64 * 16)
                .map_err(io)?;
            file.sync_all().map_err(io)?;
            Ok(())
        })();
        drop(file);
        let result = result.and_then(|()| {
            if self.index.is_empty() {
                Ok(())
            } else {
                super::file_io::check_output(&self.target)?;
                std::fs::rename(&self.temp, &self.target).map_err(io)
            }
        });
        // A failed finalization never publishes the temporary file.
        let _ = std::fs::remove_file(&self.temp);
        result
    }
}
impl Drop for Sink {
    fn drop(&mut self) {
        let _ = self.finish();
    }
}
