//! One-shot EPS transport. No interpreter starts until the parent sends a request.
pub use crate::pdf_protocol::{Header, Reply, read_json, read_reply, write_json};
use serde::{Deserialize, Serialize};
use std::path::PathBuf;
pub const MAX_INPUT: usize = 64 * 1024 * 1024;
pub const MAX_PNG: usize = 32 * 1024 * 1024;
pub const MAX_LOG: usize = 64 * 1024;
pub const MAX_SIDE: usize = 2048;
#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Request {
    pub renderer: PathBuf,
    pub scratch: PathBuf,
    pub input_len: usize,
    pub side: usize,
}
impl Request {
    pub fn validate(&self) -> matrust::Result<()> {
        if !self.renderer.is_absolute()
            || self.renderer.as_os_str().len() > 32768
            || !self
                .renderer
                .extension()
                .and_then(|e| e.to_str())
                .is_some_and(|e| e.eq_ignore_ascii_case("exe"))
            || !self.scratch.is_absolute()
            || self.scratch.as_os_str().len() > 32768
            || self.input_len == 0
            || self.input_len > MAX_INPUT
            || self.side == 0
            || self.side > MAX_SIDE
        {
            return Err("Invalid bounded EPS preview request".into());
        }
        Ok(())
    }
}
/// Reject additional pages/trailing output before decoding. This validates the
/// PNG container only; PostScript is interpreted exclusively by Ghostscript.
pub fn single_png(bytes: &[u8], side: usize) -> matrust::Result<()> {
    if bytes.len() > MAX_PNG || !bytes.starts_with(b"\x89PNG\r\n\x1a\n") {
        return Err("EPS renderer did not return a bounded PNG".into());
    }
    let mut offset = 8usize;
    let mut first = true;
    while offset.checked_add(12).is_some_and(|end| end <= bytes.len()) {
        let len = u32::from_be_bytes(bytes[offset..offset + 4].try_into().unwrap()) as usize;
        let end = offset
            .checked_add(12)
            .and_then(|v| v.checked_add(len))
            .filter(|&v| v <= bytes.len())
            .ok_or("Truncated EPS PNG")?;
        let kind = &bytes[offset + 4..offset + 8];
        if first {
            if kind != b"IHDR"
                || len != 13
                || u32::from_be_bytes(bytes[offset + 8..offset + 12].try_into().unwrap()) as usize
                    != side
                || u32::from_be_bytes(bytes[offset + 12..offset + 16].try_into().unwrap()) as usize
                    != side
            {
                return Err("EPS renderer changed the fixed preview dimensions".into());
            }
            first = false;
        } else if kind == b"IHDR" || kind == b"acTL" {
            return Err("EPS preview requires one static PNG page".into());
        }
        if kind == b"IEND" {
            return if len == 0 && end == bytes.len() {
                Ok(())
            } else {
                Err("EPS renderer returned extra page data".into())
            };
        }
        offset = end;
    }
    Err("Incomplete EPS PNG".into())
}

#[cfg(test)]
mod tests {
    use super::*;
    use image::ImageEncoder;
    #[test]
    fn png_requires_exact_single_static_page_and_fixed_dimensions() {
        let mut bytes = Vec::new();
        image::codecs::png::PngEncoder::new(&mut bytes)
            .write_image(&[255; 12], 2, 2, image::ExtendedColorType::Rgb8)
            .unwrap();
        single_png(&bytes, 2).unwrap();
        assert!(single_png(&bytes, 1).is_err());
        let mut double = bytes.clone();
        double.extend_from_slice(&bytes);
        assert!(single_png(&double, 2).is_err());
        assert!(single_png(&bytes[..bytes.len() - 1], 2).is_err());
        let mut malicious = bytes;
        malicious[8..12].copy_from_slice(&u32::MAX.to_be_bytes());
        assert!(single_png(&malicious, 2).is_err());
    }
    #[test]
    fn request_rejects_unbounded_paths_payloads_and_dimensions() {
        let renderer = std::env::current_dir().unwrap().join("gswin64c.exe");
        let mut request = Request {
            renderer,
            scratch: std::env::current_dir().unwrap(),
            input_len: 10,
            side: 64,
        };
        request.validate().unwrap();
        request.input_len = MAX_INPUT + 1;
        assert!(request.validate().is_err());
        request.input_len = 1;
        request.side = MAX_SIDE + 1;
        assert!(request.validate().is_err());
        request.side = 64;
        request.renderer = "relative.exe".into();
        assert!(request.validate().is_err());
    }
}
