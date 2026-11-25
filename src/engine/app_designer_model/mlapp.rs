//! Read-only `.mlapp` archive reader.
//!
//! `.mlapp` files are OPC/ZIP packages whose MATLAB class code is stored in
//! `matlab/document.xml` as Office Open XML paragraphs (`<w:p>`) containing
//! `<w:t>` text runs (often wrapped in CDATA or using XML entities).

use quick_xml::Reader;
use quick_xml::events::Event;
use std::io::{Cursor, Read};
use std::path::Path;

const MAX_MLAPP_BYTES: u64 = 16 * 1024 * 1024;
const MAX_DOCUMENT_XML_BYTES: usize = 8 * 1024 * 1024;
const MAX_XML_EVENTS: usize = 500_000;

fn mlapp_err(display_name: &str, reason: impl std::fmt::Display) -> String {
    format!(
        "Cannot open {display_name}: {reason}. MatRust opens .mlapp files read-only as code; save apps as .m."
    )
}

/// Open a `.mlapp` file from disk and return its extracted MATLAB class code.
pub fn open_mlapp(path: &Path) -> Result<String, String> {
    let display_name = path
        .file_name()
        .and_then(|s| s.to_str())
        .map(ToOwned::to_owned)
        .unwrap_or_else(|| path.display().to_string());
    let meta = std::fs::metadata(path).map_err(|e| mlapp_err(&display_name, e))?;
    if meta.len() > MAX_MLAPP_BYTES {
        return Err(mlapp_err(
            &display_name,
            format!("file exceeds {MAX_MLAPP_BYTES} byte limit"),
        ));
    }
    let bytes = std::fs::read(path).map_err(|e| mlapp_err(&display_name, e))?;
    read_mlapp_bytes(&bytes, &display_name)
}

/// Extract MATLAB class code from `.mlapp` zip bytes.
pub fn read_mlapp_bytes(bytes: &[u8], display_name: &str) -> Result<String, String> {
    if bytes.len() as u64 > MAX_MLAPP_BYTES {
        return Err(mlapp_err(
            display_name,
            format!("file exceeds {MAX_MLAPP_BYTES} byte limit"),
        ));
    }
    let mut zip = zip::ZipArchive::new(Cursor::new(bytes))
        .map_err(|_| mlapp_err(display_name, "not a valid zip archive"))?;

    let mut entry = match zip.by_name("matlab/document.xml") {
        Ok(entry) => entry,
        Err(zip::result::ZipError::FileNotFound) => {
            return Err(mlapp_err(display_name, "missing matlab/document.xml"));
        }
        Err(zip::result::ZipError::UnsupportedArchive(msg)) => {
            return Err(mlapp_err(
                display_name,
                format!("unsupported or encrypted archive entry ({msg})"),
            ));
        }
        Err(e) => return Err(mlapp_err(display_name, format!("corrupt archive ({e})"))),
    };

    if entry.encrypted() {
        return Err(mlapp_err(display_name, "file is encrypted"));
    }

    let mut xml_bytes = Vec::new();
    let mut limited = (&mut entry).take((MAX_DOCUMENT_XML_BYTES + 1) as u64);
    limited
        .read_to_end(&mut xml_bytes)
        .map_err(|_| mlapp_err(display_name, "failed to decompress matlab/document.xml"))?;
    if xml_bytes.len() > MAX_DOCUMENT_XML_BYTES {
        return Err(mlapp_err(
            display_name,
            "matlab/document.xml exceeds size limit",
        ));
    }

    let code = extract_ooxml_code(&xml_bytes).map_err(|r| mlapp_err(display_name, r))?;
    if !contains_classdef(&code) {
        return Err(mlapp_err(
            display_name,
            "no classdef found in matlab/document.xml",
        ));
    }
    Ok(code)
}

fn extract_ooxml_code(xml_bytes: &[u8]) -> Result<String, String> {
    let mut reader = Reader::from_reader(xml_bytes);
    let mut paragraphs: Vec<String> = Vec::new();
    let mut current_para = String::new();
    let mut in_p = false;
    let mut in_t = false;
    let mut events = 0usize;

    loop {
        events += 1;
        if events > MAX_XML_EVENTS {
            return Err("matlab/document.xml exceeds XML event limit".into());
        }
        match reader
            .read_event()
            .map_err(|e| format!("invalid XML in matlab/document.xml ({e})"))?
        {
            Event::Start(e) => {
                let local = e.local_name();
                if local.as_ref() == b"p" {
                    in_p = true;
                    current_para.clear();
                } else if local.as_ref() == b"t" && in_p {
                    in_t = true;
                }
            }
            Event::Empty(e) => {
                let local = e.local_name();
                if local.as_ref() == b"p" {
                    paragraphs.push(String::new());
                }
            }
            Event::End(e) => {
                let local = e.local_name();
                if local.as_ref() == b"t" {
                    in_t = false;
                } else if local.as_ref() == b"p" && in_p {
                    paragraphs.push(std::mem::take(&mut current_para));
                    in_p = false;
                }
            }
            Event::Text(e) if in_t => {
                let text = e.decode().map_err(|err| {
                    format!("invalid text encoding in matlab/document.xml ({err})")
                })?;
                current_para.push_str(&text);
            }
            Event::GeneralRef(e) if in_t => {
                let raw = e
                    .decode()
                    .map_err(|err| format!("invalid entity in matlab/document.xml ({err})"))?;
                let encoded = format!("&{raw};");
                let decoded = quick_xml::escape::unescape(&encoded)
                    .map_err(|_| format!("unsupported XML entity '&{raw};'"))?;
                current_para.push_str(&decoded);
            }
            Event::CData(e) if in_t => {
                let text = std::str::from_utf8(e.as_ref())
                    .map_err(|_| "non-UTF-8 CDATA in matlab/document.xml")?;
                current_para.push_str(text);
            }
            Event::Eof => break,
            _ => {}
        }
    }

    Ok(paragraphs.join("\n"))
}

fn contains_classdef(code: &str) -> bool {
    code.lines().any(|line| {
        let t = line.trim_start();
        t.starts_with("classdef ") || t.starts_with("classdef\t") || t == "classdef"
    })
}
