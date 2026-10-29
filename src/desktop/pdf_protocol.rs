//! Length-delimited, bounded messages between desktop and PDF renderer.
use matrust::Result;
use serde::{Deserialize, Serialize, de::DeserializeOwned};
use std::{
    io::{Read, Write},
    path::PathBuf,
};
const MAX_JSON: usize = 64 * 1024;
#[derive(Serialize, Deserialize)]
pub enum Request {
    Open(PathBuf),
    Render { index: u32, texture_side: usize },
}
#[derive(Serialize, Deserialize, Default)]
pub struct Header {
    pub error: Option<String>,
    #[serde(default)]
    pub source_digest: Option<[u8; 32]>,
    pub file_bytes: usize,
    pub page_count: u32,
    pub original_size: [u32; 2],
    pub size: [u32; 2],
}
pub struct Reply {
    pub header: Header,
    pub pixels: Vec<u8>,
}
pub fn write_json(writer: &mut impl Write, value: &impl Serialize) -> Result<()> {
    let bytes = serde_json::to_vec(value).map_err(|e| e.to_string())?;
    if bytes.len() > MAX_JSON {
        return Err("PDF message exceeds 64 KiB".into());
    }
    writer
        .write_all(&(bytes.len() as u32).to_le_bytes())
        .map_err(|e| e.to_string())?;
    writer.write_all(&bytes).map_err(|e| e.to_string())?;
    writer.flush().map_err(|e| e.to_string())
}
pub fn read_json<T: DeserializeOwned>(reader: &mut impl Read) -> Result<T> {
    let mut size = [0; 4];
    reader
        .read_exact(&mut size)
        .map_err(|e| format!("PDF response: {e}"))?;
    let size = u32::from_le_bytes(size) as usize;
    if size == 0 || size > MAX_JSON {
        return Err("Invalid PDF message length".into());
    }
    let mut bytes = vec![0; size];
    reader.read_exact(&mut bytes).map_err(|e| e.to_string())?;
    serde_json::from_slice(&bytes).map_err(|e| e.to_string())
}
pub fn read_reply(reader: &mut impl Read) -> Result<Reply> {
    let header: Header = read_json(reader)?;
    let [w, h] = header.size;
    let count = u64::from(w) * u64::from(h);
    if w > 4096
        || h > 4096
        || count > 8_000_000
        || header.original_size.iter().any(|&v| v > 100_000)
    {
        return Err("PDF response exceeds preview dimensions".into());
    }
    let mut pixels = vec![0; count as usize * 4];
    reader.read_exact(&mut pixels).map_err(|e| e.to_string())?;
    Ok(Reply { header, pixels })
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn pdf_protocol_rejects_oversized_truncated_and_invalid_frames() {
        assert!(read_reply(&mut &((MAX_JSON + 1) as u32).to_le_bytes()[..]).is_err());
        let mut bytes = Vec::new();
        write_json(
            &mut bytes,
            &Header {
                size: [4096, 4096],
                ..Default::default()
            },
        )
        .unwrap();
        assert!(read_reply(&mut bytes.as_slice()).is_err());
        bytes.clear();
        write_json(
            &mut bytes,
            &Header {
                size: [2, 1],
                ..Default::default()
            },
        )
        .unwrap();
        bytes.extend_from_slice(&[0; 4]);
        assert!(read_reply(&mut bytes.as_slice()).is_err());
        bytes.extend_from_slice(&[255; 4]);
        assert_eq!(read_reply(&mut bytes.as_slice()).unwrap().pixels.len(), 8);
    }
}
