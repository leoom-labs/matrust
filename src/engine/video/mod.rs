//! Engine video I/O with native AVI and optional executable backends.
mod avi;
mod avi_writer;
mod ffmpeg;
mod file_io;
mod jpeg_encode;
mod writer;
pub(super) use writer::{WRITER_METHODS, Writer, profiles};
mod jpeg;
mod reader;
use crate::Result;
pub(super) use reader::{METHODS, Reader, call, formats};
use std::sync::atomic::AtomicBool;

#[derive(Clone)]
pub(super) struct Metadata {
    pub width: usize,
    pub height: usize,
    pub rate: f64,
    pub frames: usize,
    pub duration: Option<f64>,
    pub bits: u16,
    pub format: &'static str,
    pub palette: Vec<[u8; 3]>,
}
/// Extension seam for 7b3: implementations own their path/index and return
/// top-down interleaved RGB, or one byte per gray/indexed pixel. `budget`
/// bounds all scratch allocations; implementations must poll `cancel`.
pub(super) trait VideoSource: Send + Sync {
    fn metadata(&self) -> &Metadata;
    fn frame(&self, i: usize, budget: usize, cancel: &AtomicBool) -> Result<Vec<u8>>;
    fn palette(&self, _i: usize) -> &[[u8; 3]] {
        &self.metadata().palette
    }
    fn max_palette_len(&self) -> usize {
        self.metadata().palette.len()
    }
    fn bytes(&self) -> usize;
    fn reset(&self) {}
}
fn admit(bytes: usize, budget: usize) -> Result<()> {
    if bytes > budget {
        Err("VideoReader exceeds the evaluation memory limit".into())
    } else {
        Ok(())
    }
}
fn buffer(bytes: usize, budget: usize) -> Result<Vec<u8>> {
    admit(bytes, budget)?;
    let mut out = Vec::new();
    out.try_reserve_exact(bytes)
        .map_err(|_| "VideoReader: unable to allocate frame")?;
    out.resize(bytes, 0);
    Ok(out)
}
