//! Archive, image, audio and import functions: zip unzip gzip gunzip
//! tar untar importdata imread imwrite imfinfo audioread audiowrite audioinfo
//! fileattrib. PNG (zlib via flate2, CRC32), WAV and ustar are encoded here;
//! ZIP uses the zip crate. Every read is bounded before it allocates.
#![allow(unknown_lints, clippy::chunks_exact_to_as_chunks)]
use super::{Engine, Execution, Value};
use crate::typed_array::{NumericClass, TypedArray};
use crate::{Array, CellArray, LogicalMatrix, Matrix, Result, StructArray, syntax::Expr};
use std::{
    collections::BTreeSet,
    fs,
    io::{Read, Write},
    path::{MAIN_SEPARATOR, Path, PathBuf},
    sync::Arc,
};

const NAMES: &[&str] = &[
    "zip",
    "unzip",
    "gzip",
    "gunzip",
    "tar",
    "untar",
    "importdata",
    "imread",
    "imwrite",
    "imfinfo",
    "audioread",
    "audiowrite",
    "audioinfo",
    "fileattrib",
];
pub(super) fn fl2_handles(name: &str) -> bool {
    NAMES.contains(&name)
}
/// Largest file read or written, and largest total archive extraction.
const MAX_BYTES: u64 = 256 * 1024 * 1024;
const MAX_ENTRIES: usize = 10_000;
const PNG_SIGNATURE: [u8; 8] = [137, 80, 78, 71, 13, 10, 26, 10];
const PNG_INFO_FIELDS: &[&str] = &[
    "Filename",
    "FileModDate",
    "FileSize",
    "Format",
    "FormatVersion",
    "Width",
    "Height",
    "BitDepth",
    "ColorType",
    "FormatSignature",
    "Colormap",
    "Histogram",
    "InterlaceType",
    "Transparency",
    "SimpleTransparencyData",
    "BackgroundColor",
    "RenderingIntent",
    "Chromaticities",
    "Gamma",
    "XResolution",
    "YResolution",
    "ResolutionUnit",
    "XOffset",
    "YOffset",
    "OffsetUnit",
    "SignificantBits",
    "ImageModTime",
    "Title",
    "Author",
    "Description",
    "Copyright",
    "CreationTime",
    "Software",
    "Disclaimer",
    "Warning",
    "Source",
    "Comment",
    "OtherText",
];
/// PNG text keywords with their own imfinfo field (the rest go to OtherText).
const PNG_TEXT_KEYS: &[&str] = &[
    "Title",
    "Author",
    "Description",
    "Copyright",
    "CreationTime",
    "Software",
    "Disclaimer",
    "Warning",
    "Source",
    "Comment",
];
const AUDIO_EXTENSIONS: &[&str] = &["flac", "m4a", "mp4", "oga", "ogg", "wav"];
const IMAGE_FORMATS: &[(&str, &str)] = &[
    ("png", "png"),
    ("jpg", "jpg"),
    ("jpeg", "jpg"),
    ("tif", "tif"),
    ("tiff", "tif"),
    ("bmp", "bmp"),
    ("gif", "gif"),
    ("pbm", "pbm"),
    ("pgm", "pgm"),
    ("ppm", "ppm"),
    ("pcx", "pcx"),
    ("ras", "ras"),
    ("hdf", "hdf"),
    ("ico", "ico"),
    ("cur", "cur"),
    ("jp2", "jp2"),
    ("j2k", "jp2"),
    ("jpx", "jp2"),
];
const FILEATTRIB_FIELDS: &[&str] = &[
    "Name",
    "archive",
    "system",
    "hidden",
    "directory",
    "UserRead",
    "UserWrite",
    "UserExecute",
    "GroupRead",
    "GroupWrite",
    "GroupExecute",
    "OtherRead",
    "OtherWrite",
    "OtherExecute",
];

fn fl2_num(value: f64) -> Value {
    Value::Matrix(Matrix::scalar(value))
}
fn fl2_bool(value: bool) -> Value {
    Value::Logical(LogicalMatrix::scalar(value))
}
fn fl2_str(value: impl Into<String>) -> Value {
    Value::Text(value.into())
}
/// Cell results (textdata grids) are bounded by the cell slot limit before any allocation.
fn fl2_cell_slots(rows: usize, cols: usize) -> Result<()> {
    if rows.saturating_mul(cols) > crate::script_containers::MAX_CELL_SLOTS {
        return Err("The text part of the file exceeds the cell array limit.".into());
    }
    Ok(())
}
/// Trailing separators go, except on a root: `/` stays `/`, `C:\` stays `C:\` (checked 2026-10-24).
fn fl2_trim_name(full: &str) -> String {
    let t = full.trim_end_matches(['/', '\\']);
    let drive = t.len() == 2 && t.ends_with(':');
    if (t.is_empty() || drive) && t.len() < full.len() {
        full[..t.len() + 1].to_owned()
    } else {
        t.to_owned()
    }
}
fn fl2_empty() -> Value {
    Value::Matrix(Matrix::from_column_vec(0, 0, Vec::new()).expect("empty matrix"))
}
fn fl2_arity(args: &[Value], min: usize, max: usize) -> Result<()> {
    if args.len() < min {
        Err("Not enough input arguments.".into())
    } else if args.len() > max {
        Err("Too many input arguments.".into())
    } else {
        Ok(())
    }
}
fn fl2_extension(text: &str) -> String {
    let name = text.rsplit(['/', '\\']).next().unwrap_or(text);
    match name.rfind('.') {
        Some(at) if at > 0 || name.len() > 1 => name[at + 1..].to_ascii_lowercase(),
        _ => String::new(),
    }
}
fn fl2_has_extension(text: &str) -> bool {
    let name = text.rsplit(['/', '\\']).next().unwrap_or(text);
    name.rfind('.').is_some()
}
/// Joins a folder text and a name the way fullfile does, native separators.
fn fl2_join(folder: &str, name: &str) -> String {
    let name = fl2_native(name);
    if folder.is_empty() {
        return name;
    }
    let trimmed = folder.trim_end_matches(['/', '\\']);
    if trimmed.is_empty() {
        return format!("{MAIN_SEPARATOR}{name}");
    }
    format!("{trimmed}{MAIN_SEPARATOR}{name}")
}
fn fl2_native(text: &str) -> String {
    if cfg!(windows) {
        text.replace('/', "\\")
    } else {
        text.into()
    }
}
fn fl2_base_name(text: &str) -> &str {
    text.rsplit(['/', '\\']).next().unwrap_or(text)
}
fn fl2_parent_text(text: &str) -> &str {
    match text.rfind(['/', '\\']) {
        Some(0) => &text[..1],
        Some(at) => &text[..at],
        None => "",
    }
}
fn fl2_is_wild(text: &str) -> bool {
    text.contains(['*', '?'])
}
fn fl2_wildcard(pattern: &str, name: &str) -> bool {
    let p: Vec<char> = pattern.chars().collect();
    let n: Vec<char> = name.chars().collect();
    let (mut i, mut j, mut star, mut retry) = (0, 0, None, 0);
    while j < n.len() {
        if i < p.len()
            && (p[i] == '?' || p[i] == n[j] || cfg!(windows) && p[i].eq_ignore_ascii_case(&n[j]))
        {
            i += 1;
            j += 1;
        } else if i < p.len() && p[i] == '*' {
            star = Some(i);
            i += 1;
            retry = j;
        } else if let Some(s) = star {
            retry += 1;
            j = retry;
            i = s + 1;
        } else {
            return false;
        }
    }
    while i < p.len() && p[i] == '*' {
        i += 1;
    }
    i == p.len()
}
fn fl2_io(error: std::io::Error) -> String {
    error.to_string()
}
fn fl2_crc(kind: &[u8], data: &[u8]) -> u32 {
    let mut crc = flate2::Crc::new();
    crc.update(kind);
    crc.update(data);
    crc.sum()
}
fn fl2_read_limited(path: &Path, limit: u64) -> Result<Vec<u8>> {
    let file = fs::File::open(path).map_err(fl2_io)?;
    let length = file.metadata().map_err(fl2_io)?.len();
    if length > limit {
        return Err(format!("File exceeds the {limit}-byte limit"));
    }
    let mut bytes = Vec::new();
    bytes
        .try_reserve_exact(length as usize)
        .map_err(|_| "Unable to allocate the file buffer")?;
    file.take(limit + 1)
        .read_to_end(&mut bytes)
        .map_err(fl2_io)?;
    if bytes.len() as u64 > limit {
        return Err(format!("File exceeds the {limit}-byte limit"));
    }
    Ok(bytes)
}
/// Decompress with a hard output bound (zip bombs stop at the limit).
fn fl2_inflate(reader: impl Read, limit: u64) -> Result<Vec<u8>> {
    let mut out = Vec::new();
    reader
        .take(limit + 1)
        .read_to_end(&mut out)
        .map_err(fl2_io)?;
    if out.len() as u64 > limit {
        return Err(format!("Decompressed data exceeds the {limit}-byte limit"));
    }
    Ok(out)
}

/// Numeric samples of an argument, with its class name.
struct Fl2Data {
    class: &'static str,
    rows: usize,
    cols: usize,
    reals: Vec<f64>,
}
fn fl2_data(value: &Value, cancel: &std::sync::atomic::AtomicBool) -> Result<Fl2Data> {
    let array = value.clone().array(cancel)?;
    let class = array.class();
    let (rows, cols, reals) = match &array {
        Array::Numeric(m) => (m.rows(), m.cols(), m.data().to_vec()),
        Array::Logical(l) => (
            l.rows(),
            l.cols(),
            l.data().iter().map(|&b| f64::from(b)).collect(),
        ),
        Array::Typed(t) => (t.rows(), t.cols(), t.to_f64_vec()?),
        Array::Character(c) => (
            c.rows(),
            c.cols(),
            c.units().iter().map(|&u| f64::from(u)).collect(),
        ),
        _ => (0, 0, Vec::new()),
    };
    Ok(Fl2Data {
        class,
        rows,
        cols,
        reals,
    })
}

struct Fl2Png {
    width: usize,
    height: usize,
    depth: u8,
    color: u8,
    interlace: u8,
    palette: Vec<[u8; 3]>,
    trns: Option<Vec<u8>>,
    texts: Vec<(String, String)>,
    gamma: Option<f64>,
    time: Option<String>,
    phys: Option<(u32, u32, u8)>,
    background: Option<Vec<u8>>,
    /// Row-major samples, `channels` per pixel; empty when not decoded.
    samples: Vec<u16>,
}
fn fl2_png_channels(color: u8) -> usize {
    match color {
        2 => 3,
        4 => 2,
        6 => 4,
        _ => 1,
    }
}
fn fl2_png_decode(
    bytes: &[u8],
    pixels: bool,
    cancel: &std::sync::atomic::AtomicBool,
) -> Result<Fl2Png> {
    if bytes.len() < 8 || bytes[..8] != PNG_SIGNATURE {
        return Err("Not a PNG file.".into());
    }
    let mut png = Fl2Png {
        width: 0,
        height: 0,
        depth: 0,
        color: 0,
        interlace: 0,
        palette: Vec::new(),
        trns: None,
        texts: Vec::new(),
        gamma: None,
        time: None,
        phys: None,
        background: None,
        samples: Vec::new(),
    };
    let mut idat = Vec::new();
    let mut pos = 8;
    let mut seen_header = false;
    while pos + 12 <= bytes.len() {
        crate::dsp::check_cancel(cancel)?;
        let length = u32::from_be_bytes(bytes[pos..pos + 4].try_into().unwrap()) as usize;
        let kind = &bytes[pos + 4..pos + 8];
        let end = pos
            .checked_add(12)
            .and_then(|v| v.checked_add(length))
            .filter(|&v| v <= bytes.len())
            .ok_or("PNG file is truncated.")?;
        let data = &bytes[pos + 8..pos + 8 + length];
        let crc = u32::from_be_bytes(bytes[end - 4..end].try_into().unwrap());
        if crc != fl2_crc(kind, data) {
            return Err("PNG file has a bad chunk checksum.".into());
        }
        match kind {
            b"IHDR" if length == 13 => {
                png.width = u32::from_be_bytes(data[0..4].try_into().unwrap()) as usize;
                png.height = u32::from_be_bytes(data[4..8].try_into().unwrap()) as usize;
                png.depth = data[8];
                png.color = data[9];
                png.interlace = data[12];
                seen_header = true;
            }
            b"PLTE" => {
                if data.is_empty() || data.len() > 768 || !data.len().is_multiple_of(3) {
                    return Err("Invalid PNG palette.".into());
                }
                png.palette = data.chunks_exact(3).map(|c| [c[0], c[1], c[2]]).collect();
            }
            b"tRNS" => png.trns = Some(data.to_vec()),
            b"bKGD" => png.background = Some(data.to_vec()),
            b"gAMA" if length == 4 => {
                png.gamma = Some(u32::from_be_bytes(data.try_into().unwrap()) as f64 / 100000.)
            }
            b"pHYs" if length == 9 => {
                png.phys = Some((
                    u32::from_be_bytes(data[0..4].try_into().unwrap()),
                    u32::from_be_bytes(data[4..8].try_into().unwrap()),
                    data[8],
                ))
            }
            b"tIME" if length == 7 => {
                const MONTHS: [&str; 12] = [
                    "Jan", "Feb", "Mar", "Apr", "May", "Jun", "Jul", "Aug", "Sep", "Oct", "Nov",
                    "Dec",
                ];
                let year = u16::from_be_bytes([data[0], data[1]]);
                let month = MONTHS
                    .get(usize::from(data[2]).wrapping_sub(1))
                    .unwrap_or(&"Jan");
                png.time = Some(format!(
                    "{} {} {} {:02}:{:02}:{:02} +0000",
                    data[3], month, year, data[4], data[5], data[6]
                ));
            }
            b"tEXt" => {
                if let Some(at) = data.iter().position(|&b| b == 0) {
                    let key: String = data[..at].iter().map(|&b| char::from(b)).collect();
                    let text: String = data[at + 1..].iter().map(|&b| char::from(b)).collect();
                    png.texts.push((key, text));
                }
            }
            b"IDAT" => {
                if idat.len() + data.len() > MAX_BYTES as usize {
                    return Err("PNG image data exceeds the size limit.".into());
                }
                idat.extend_from_slice(data);
            }
            b"IEND" => break,
            _ => {}
        }
        pos = end;
    }
    if !seen_header {
        return Err("PNG file has no IHDR chunk.".into());
    }
    let valid = match png.color {
        0 => matches!(png.depth, 1 | 2 | 4 | 8 | 16),
        3 => matches!(png.depth, 1 | 2 | 4 | 8),
        2 | 4 | 6 => matches!(png.depth, 8 | 16),
        _ => false,
    };
    if !valid || png.width == 0 || png.height == 0 {
        return Err("PNG file has an invalid header.".into());
    }
    if !pixels {
        return Ok(png);
    }
    if png.interlace != 0 {
        return Err("Interlaced (Adam7) PNG images are not supported yet.".into());
    }
    let channels = fl2_png_channels(png.color);
    let count = Matrix::checked_len(png.height, png.width)?;
    let bits = channels * usize::from(png.depth);
    let stride = (png.width * bits).div_ceil(8);
    let expected = (stride + 1)
        .checked_mul(png.height)
        .ok_or("PNG image is too large.")?;
    let raw = fl2_inflate(
        flate2::read::ZlibDecoder::new(idat.as_slice()),
        expected as u64,
    )?;
    if raw.len() != expected {
        return Err("PNG image data is truncated.".into());
    }
    let bpp = (bits / 8).max(1);
    let mut previous = vec![0u8; stride];
    let mut line = vec![0u8; stride];
    let mut samples = Vec::new();
    samples
        .try_reserve_exact(count * channels)
        .map_err(|_| "Unable to allocate the image")?;
    for row in 0..png.height {
        if row % 256 == 0 {
            crate::dsp::check_cancel(cancel)?;
        }
        let start = row * (stride + 1);
        let filter = raw[start];
        line.copy_from_slice(&raw[start + 1..start + 1 + stride]);
        for i in 0..stride {
            let a = if i >= bpp { line[i - bpp] } else { 0 };
            let b = previous[i];
            let c = if i >= bpp { previous[i - bpp] } else { 0 };
            let predictor = match filter {
                0 => 0,
                1 => a,
                2 => b,
                3 => ((u16::from(a) + u16::from(b)) / 2) as u8,
                4 => {
                    let p = i16::from(a) + i16::from(b) - i16::from(c);
                    let (pa, pb, pc) = (
                        (p - i16::from(a)).abs(),
                        (p - i16::from(b)).abs(),
                        (p - i16::from(c)).abs(),
                    );
                    if pa <= pb && pa <= pc {
                        a
                    } else if pb <= pc {
                        b
                    } else {
                        c
                    }
                }
                _ => return Err("PNG image has an invalid filter type.".into()),
            };
            line[i] = line[i].wrapping_add(predictor);
        }
        let per_row = png.width * channels;
        match png.depth {
            8 => samples.extend(line[..per_row].iter().map(|&b| u16::from(b))),
            16 => samples.extend(
                line.chunks_exact(2)
                    .take(per_row)
                    .map(|p| u16::from_be_bytes([p[0], p[1]])),
            ),
            depth => {
                let depth = usize::from(depth);
                let mask = (1u16 << depth) - 1;
                for k in 0..per_row {
                    let bit = k * depth;
                    let byte = line[bit / 8];
                    let shift = 8 - depth - bit % 8;
                    samples.push((u16::from(byte) >> shift) & mask);
                }
            }
        }
        std::mem::swap(&mut previous, &mut line);
    }
    png.samples = samples;
    Ok(png)
}
fn fl2_png_chunk(out: &mut Vec<u8>, kind: &[u8; 4], data: &[u8]) {
    out.extend_from_slice(&(data.len() as u32).to_be_bytes());
    out.extend_from_slice(kind);
    out.extend_from_slice(data);
    out.extend_from_slice(&fl2_crc(kind, data).to_be_bytes());
}

/// Parsed WAV stream: interleaved raw data and its format.
struct Fl2Wav {
    float: bool,
    channels: usize,
    rate: u32,
    bits: u16,
    data: Vec<u8>,
    title: Option<String>,
    artist: Option<String>,
    comment: Option<String>,
}
impl Fl2Wav {
    fn frames(&self) -> usize {
        let width = usize::from(self.bits / 8) * self.channels;
        self.data.len().checked_div(width).unwrap_or(0)
    }
    /// Integer sample value (PCM) at interleaved index `i`.
    fn int(&self, i: usize) -> i64 {
        let width = usize::from(self.bits / 8);
        let b = &self.data[i * width..(i + 1) * width];
        match self.bits {
            8 => i64::from(b[0]),
            16 => i64::from(i16::from_le_bytes([b[0], b[1]])),
            24 => i64::from(i32::from_le_bytes([0, b[0], b[1], b[2]]) >> 8),
            _ => i64::from(i32::from_le_bytes([b[0], b[1], b[2], b[3]])),
        }
    }
    fn real(&self, i: usize) -> f64 {
        let width = usize::from(self.bits / 8);
        let b = &self.data[i * width..(i + 1) * width];
        if self.bits == 32 {
            f64::from(f32::from_le_bytes([b[0], b[1], b[2], b[3]]))
        } else {
            f64::from_le_bytes(b.try_into().unwrap())
        }
    }
}
fn fl2_wav_decode(bytes: &[u8]) -> Result<Fl2Wav> {
    if bytes.len() < 12 || &bytes[..4] != b"RIFF" || &bytes[8..12] != b"WAVE" {
        return Err(
            "File is not a WAV (RIFF/WAVE) file; other audio formats are not supported yet.".into(),
        );
    }
    let mut wav = Fl2Wav {
        float: false,
        channels: 0,
        rate: 0,
        bits: 0,
        data: Vec::new(),
        title: None,
        artist: None,
        comment: None,
    };
    let mut have_format = false;
    let mut have_data = false;
    let mut pos = 12;
    while pos + 8 <= bytes.len() {
        let kind = &bytes[pos..pos + 4];
        let length = u32::from_le_bytes(bytes[pos + 4..pos + 8].try_into().unwrap()) as usize;
        let start = pos + 8;
        let end = start.saturating_add(length);
        if end > bytes.len() {
            return Err(
                "The WAV file is truncated: a chunk extends past the end of the file.".into(),
            );
        }
        let body = &bytes[start..end];
        match kind {
            b"fmt " if body.len() >= 16 => {
                let mut tag = u16::from_le_bytes([body[0], body[1]]);
                wav.channels = usize::from(u16::from_le_bytes([body[2], body[3]]));
                wav.rate = u32::from_le_bytes(body[4..8].try_into().unwrap());
                wav.bits = u16::from_le_bytes([body[14], body[15]]);
                if tag == 0xFFFE && body.len() >= 26 {
                    tag = u16::from_le_bytes([body[24], body[25]]);
                }
                wav.float = match tag {
                    1 => false,
                    3 => true,
                    _ => return Err("Compressed WAV encodings are not supported yet.".into()),
                };
                have_format = true;
            }
            b"data" => {
                wav.data = body.to_vec();
                have_data = true;
            }
            b"LIST" if body.len() >= 4 && &body[..4] == b"INFO" => {
                let mut p = 4;
                while p + 8 <= body.len() {
                    let id = &body[p..p + 4];
                    let n = u32::from_le_bytes(body[p + 4..p + 8].try_into().unwrap()) as usize;
                    let text_end = (p + 8).saturating_add(n).min(body.len());
                    let text: String = body[p + 8..text_end]
                        .iter()
                        .take_while(|&&b| b != 0)
                        .map(|&b| char::from(b))
                        .collect();
                    match id {
                        b"INAM" => wav.title = Some(text),
                        b"IART" => wav.artist = Some(text),
                        b"ICMT" => wav.comment = Some(text),
                        _ => {}
                    }
                    p = text_end + (n & 1);
                }
            }
            _ => {}
        }
        pos = end + (length & 1);
    }
    let valid_bits = if wav.float {
        matches!(wav.bits, 32 | 64)
    } else {
        matches!(wav.bits, 8 | 16 | 24 | 32)
    };
    if !have_format || !have_data || wav.channels == 0 || !valid_bits {
        return Err("WAV file has an invalid or unsupported format chunk.".into());
    }
    Ok(wav)
}

/// One file to archive: its path on disk, its archive entry name ('/'
/// separators) and the name reported back to the caller.
struct Fl2Entry {
    path: PathBuf,
    entry: String,
    reported: String,
}

/// Split a UTF-8 name between the ustar prefix and name byte fields.
fn fl2_tar_name_parts(name: &str) -> Option<(&[u8], &[u8])> {
    let bytes = name.as_bytes();
    if bytes.len() <= 100 {
        Some((&[][..], bytes))
    } else {
        let split = bytes
            .iter()
            .enumerate()
            .filter(|&(i, &b)| b == b'/' && i <= 155 && bytes.len() - i - 1 <= 100 && i > 0)
            .map(|(i, _)| i)
            .next_back()?;
        Some((&bytes[..split], &bytes[split + 1..]))
    }
}
fn fl2_tar_header(name: &str, size: u64, mtime: u64, kind: u8) -> Result<[u8; 512]> {
    let mut block = [0u8; 512];
    let (prefix, base) = fl2_tar_name_parts(name)
        .ok_or_else(|| format!("Name too long for a tar header: {name}"))?;
    block[..base.len()].copy_from_slice(base);
    let octal = |block: &mut [u8; 512], at: usize, width: usize, value: u64| {
        let text = format!("{value:0w$o}", w = width - 1);
        block[at..at + width - 1].copy_from_slice(text.as_bytes());
    };
    octal(&mut block, 100, 8, if kind == b'5' { 0o755 } else { 0o644 });
    octal(&mut block, 108, 8, 0);
    octal(&mut block, 116, 8, 0);
    octal(&mut block, 124, 12, size);
    octal(&mut block, 136, 12, mtime);
    block[156] = kind;
    block[257..263].copy_from_slice(b"ustar\0");
    block[263..265].copy_from_slice(b"00");
    block[345..345 + prefix.len()].copy_from_slice(prefix);
    block[148..156].copy_from_slice(b"        ");
    let sum: u32 = block.iter().map(|&b| u32::from(b)).sum();
    let text = format!("{sum:06o}\0 ");
    block[148..156].copy_from_slice(text.as_bytes());
    Ok(block)
}
fn fl2_tar_octal(field: &[u8]) -> Option<u64> {
    let text: String = field
        .iter()
        .take_while(|&&b| b != 0)
        .map(|&b| char::from(b))
        .collect();
    let text = text.trim();
    if text.is_empty() {
        return Some(0);
    }
    u64::from_str_radix(text, 8).ok()
}
fn fl2_tar_valid(block: &[u8]) -> bool {
    let Some(stored) = fl2_tar_octal(&block[148..156]) else {
        return false;
    };
    let sum: u64 = block
        .iter()
        .enumerate()
        .map(|(i, &b)| {
            if (148..156).contains(&i) {
                32
            } else {
                u64::from(b)
            }
        })
        .sum();
    sum == stored
}
/// Creates `relative` under `base` one component at a time, refusing any
/// existing link so extraction never writes outside `base`.
fn fl2_create_dirs(base: &Path, relative: &Path) -> Result<()> {
    let mut current = base.to_path_buf();
    for part in relative.components() {
        current.push(part);
        match fs::symlink_metadata(&current) {
            Ok(m) if m.file_type().is_symlink() => {
                return Err(format!(
                    "Refusing to extract through the link {}",
                    current.display()
                ));
            }
            Ok(m) if m.is_dir() => {}
            Ok(_) => return Err(format!("{} is not a folder", current.display())),
            Err(_) => fs::create_dir(&current).map_err(fl2_io)?,
        }
    }
    Ok(())
}
/// Archive names never escape the output folder.
fn fl2_safe_relative(name: &str) -> Option<PathBuf> {
    let mut path = PathBuf::new();
    for part in name.split(['/', '\\']) {
        match part {
            "" | "." => {}
            ".." => return None,
            p if p.contains(':') => return None,
            p => path.push(p),
        }
    }
    (!path.as_os_str().is_empty()).then_some(path)
}

/// importdata delimiter.
#[derive(Clone, Copy, PartialEq)]
enum Fl2Delim {
    Space,
    Char(char),
}
impl Fl2Delim {
    fn text(self) -> String {
        match self {
            Fl2Delim::Space => " ".into(),
            Fl2Delim::Char(c) => c.to_string(),
        }
    }
    fn split(self, line: &str) -> Vec<String> {
        match self {
            Fl2Delim::Space => line.split_whitespace().map(str::to_owned).collect(),
            Fl2Delim::Char(c) => line
                .split_terminator(c)
                .map(|s| s.trim().to_owned())
                .collect(),
        }
    }
}
fn fl2_number(token: &str) -> Option<f64> {
    let t = token.trim();
    if t.is_empty()
        || t.chars()
            .any(|c| c.is_ascii_alphabetic() && !"eEinfaINFA".contains(c))
    {
        return None;
    }
    let lower = t.to_ascii_lowercase();
    if lower.contains("infinity") {
        return None;
    }
    t.parse::<f64>().ok()
}
/// A data row: optional leading text labels, then numbers (empty fields of a
/// character delimiter count as NaN). Returns (labels, numbers).
fn fl2_data_row(tokens: &[String], delim: Fl2Delim) -> Option<(usize, Vec<f64>)> {
    let first = tokens.iter().position(|t| fl2_number(t).is_some())?;
    let mut numbers = Vec::with_capacity(tokens.len() - first);
    for token in &tokens[first..] {
        match fl2_number(token) {
            Some(v) => numbers.push(v),
            None if token.is_empty() && matches!(delim, Fl2Delim::Char(_)) => {
                numbers.push(f64::NAN)
            }
            None => return None,
        }
    }
    Some((first, numbers))
}

impl Engine {
    /// Single-output entry for `fn call`; None when `name` is not in this family.
    pub(super) fn fl2_call(
        &mut self,
        name: &str,
        args: &[Value],
        out: &mut Execution,
    ) -> Result<Option<Value>> {
        if !fl2_handles(name) {
            return Ok(None);
        }
        let mut values = self.fl2_run(name, args, 1, out)?;
        if values.is_empty() {
            return Err("Too many output arguments.".into());
        }
        Ok(Some(values.remove(0)))
    }

    pub(super) fn fl2_eval_outputs(
        &mut self,
        expression: &Expr,
        count: usize,
        out: &mut Execution,
    ) -> Result<Option<Vec<Value>>> {
        let (name, expressions) = match expression {
            Expr::Call(name, args) => (name.as_str(), args.as_slice()),
            Expr::Name(name) => (name.as_str(), &[][..]),
            _ => return Ok(None),
        };
        if !fl2_handles(name) || self.contains_array(name) {
            return Ok(None);
        }
        let args = self.function_arguments(expressions, None, out)?;
        if let Some(function) = self.resolve_function(name)? {
            return self
                .invoke_function(function, args, count, None, out)
                .map(Some);
        }
        let values = self.fl2_run(name, &args, count, out)?;
        if values.len() < count {
            return Err("Too many output arguments.".into());
        }
        Ok(Some(values))
    }

    /// Statement form (no outputs requested). Functions that still return a
    /// value with nargout 0 (imread, importdata, ...) set and show `ans`.
    pub(super) fn fl2_statement(
        &mut self,
        name: &str,
        expressions: &[Expr],
        quiet: bool,
        out: &mut Execution,
    ) -> Result<bool> {
        if !fl2_handles(name) {
            return Ok(false);
        }
        let args = self.function_arguments(expressions, None, out)?;
        let mut values = self.fl2_run(name, &args, 0, out)?;
        if !values.is_empty() {
            let value = values.remove(0).array(&self.cancel)?;
            self.assign_workspace_array("ans", value, quiet, out)?;
        }
        Ok(true)
    }

    fn fl2_run(
        &mut self,
        name: &str,
        args: &[Value],
        nargout: usize,
        out: &mut Execution,
    ) -> Result<Vec<Value>> {
        self.check_cancel()?;
        let max = match name {
            "imwrite" | "audiowrite" => 0,
            "imread" | "importdata" | "fileattrib" => 3,
            "audioread" => 2,
            _ => 1,
        };
        if nargout > max {
            return Err("Too many output arguments.".into());
        }
        match name {
            "imwrite" => self.fl2_imwrite(args).map(|()| Vec::new()),
            "imread" => self.fl2_imread(args, nargout),
            "imfinfo" => self.fl2_imfinfo(args, out).map(|v| vec![v]),
            "audiowrite" => self.fl2_audiowrite(args, out).map(|()| Vec::new()),
            "audioread" => self.fl2_audioread(args),
            "audioinfo" => self.fl2_audioinfo(args).map(|v| vec![v]),
            "importdata" => self.fl2_importdata(args),
            "zip" | "tar" => self.fl2_pack(name, args, nargout),
            "unzip" | "untar" => self.fl2_unpack(name, args, nargout),
            "gzip" => self.fl2_gzip(args, nargout),
            "gunzip" => self.fl2_gunzip(args, nargout),
            "fileattrib" => self.fl2_fileattrib(args, nargout, out),
            _ => Err(format!("Unsupported function {name}")),
        }
    }

    fn fl2_text(&self, value: &Value) -> Option<String> {
        match value {
            Value::Text(s) | Value::QuotedString(s) => Some(s.clone()),
            Value::Container(Array::Character(c)) if c.is_text_row() => {
                c.to_utf8(&self.cancel).ok()
            }
            _ => None,
        }
    }
    /// A file name argument: text, bounded, no URLs.
    fn fl2_name(&self, value: &Value, error: &str) -> Result<String> {
        let text = self.fl2_text(value).ok_or(error)?;
        if text.len() > 32768 || text.contains('\0') {
            return Err("File name is oversized or contains NUL".into());
        }
        if text.contains("://") {
            return Err("Remote file URLs are not supported".into());
        }
        Ok(text)
    }
    fn fl2_resolve(&self, text: &str) -> Result<PathBuf> {
        self.file_path(text, "")
    }
    /// Absolute display text for a name: as given when absolute, otherwise
    /// joined to the current folder.
    fn fl2_full_text(&self, text: &str) -> String {
        if Path::new(text).is_absolute() {
            return text.into();
        }
        match self.current_folder() {
            Some(folder) => fl2_join(&super::env_files::envx_path_text(folder), text),
            None => text.into(),
        }
    }
    fn fl2_text_list(&self, value: &Value, error: &str) -> Result<Vec<String>> {
        if let Value::Container(Array::Cell(cell)) = value {
            if cell.len() > MAX_ENTRIES {
                return Err("Too many file names".into());
            }
            let mut names = Vec::with_capacity(cell.len());
            for i in 0..cell.len() {
                let item = cell.get_linear(i)?;
                match item.as_ref() {
                    Array::Character(c) if c.is_text_row() => names.push(c.to_utf8(&self.cancel)?),
                    _ => return Err(error.into()),
                }
            }
            return Ok(names);
        }
        Ok(vec![self.fl2_name(value, error)?])
    }
    fn fl2_cell_row(&self, items: Vec<Value>) -> Result<Value> {
        self.fl2_cell(1, items.len(), items)
    }
    fn fl2_cell(&self, rows: usize, cols: usize, items: Vec<Value>) -> Result<Value> {
        let slots = items
            .into_iter()
            .map(|v| v.array(&self.cancel).map(Arc::new))
            .collect::<Result<Vec<_>>>()?;
        Ok(Value::Container(Array::Cell(CellArray::from_shared(
            rows,
            cols,
            slots,
            &self.cancel,
        )?)))
    }
    fn fl2_struct(&self, names: &[&str], values: Vec<Value>) -> Result<Value> {
        let slots = values
            .into_iter()
            .map(|v| v.array(&self.cancel).map(Arc::new))
            .collect::<Result<Vec<_>>>()?;
        Ok(Value::Container(Array::Struct(StructArray::from_slots(
            1,
            1,
            names.iter().map(|s| (*s).to_owned()).collect(),
            slots,
            &self.cancel,
        )?)))
    }
    fn fl2_matrix(rows: usize, cols: usize, data: Vec<f64>) -> Result<Value> {
        Ok(Value::Matrix(Matrix::from_column_vec(rows, cols, data)?))
    }
    fn fl2_mod_date(&mut self, path: &Path, out: &mut Execution) -> Result<Value> {
        let listing = self
            // dir refuses `\\?\` names as network paths: pass the plain local form.
            .envx_call(
                "dir",
                &[fl2_str(super::env_files::envx_path_text(path))],
                out,
            )?
            .ok_or("dir is unavailable")?;
        if let Value::Container(Array::Struct(s)) = &listing
            && s.len() == 1
            && let Some(field) = s.field_index("date")
        {
            return Ok(Value::Container(s.get(field, 0)?.as_ref().clone()));
        }
        Err("Unable to read the file date.".into())
    }
    fn fl2_write_file(&self, path: &Path, bytes: &[u8]) -> Result<()> {
        self.check_cancel()?;
        if bytes.len() as u64 > MAX_BYTES {
            return Err(format!("Output exceeds the {MAX_BYTES}-byte limit"));
        }
        let mut file = fs::File::create(path)
            .map_err(|e| format!("Unable to write {}: {e}", path.display()))?;
        for chunk in bytes.chunks(1 << 20) {
            self.check_cancel()?;
            file.write_all(chunk).map_err(fl2_io)?;
        }
        Ok(())
    }

    // ---------------------------------------------------------------- images

    fn fl2_imwrite(&mut self, args: &[Value]) -> Result<()> {
        if args.len() < 2 {
            return Err("Wrong number of input arguments.".into());
        }
        let (map, file_at) = if self.fl2_text(&args[1]).is_some() {
            (None, 1)
        } else {
            if args.len() < 3 || self.fl2_text(&args[2]).is_none() {
                return Err("Wrong number of input arguments.".into());
            }
            (Some(&args[1]), 2)
        };
        let filename = self.fl2_name(&args[file_at], "Wrong number of input arguments.")?;
        let mut rest = &args[file_at + 1..];
        let mut format = None;
        if let Some(text) = rest.first().and_then(|v| self.fl2_text(v))
            && let Some((_, f)) = IMAGE_FORMATS
                .iter()
                .find(|(e, _)| e.eq_ignore_ascii_case(&text))
        {
            format = Some(*f);
            rest = &rest[1..];
        }
        let format = match format {
            Some(f) => f,
            None => {
                let ext = fl2_extension(&filename);
                IMAGE_FORMATS
                    .iter()
                    .find(|(e, _)| *e == ext)
                    .map(|(_, f)| *f)
                    .ok_or("Unable to determine the file format from the file name.")?
            }
        };
        if !rest.len().is_multiple_of(2) {
            return Err("Parameter names and values must come in pairs.".into());
        }
        let mut pairs = Vec::new();
        for pair in rest.chunks(2) {
            let key = self
                .fl2_text(&pair[0])
                .ok_or("Parameter names must be character vectors.")?;
            pairs.push((key, &pair[1]));
        }
        if format != "png" {
            if format == "jpg"
                && let Some((_, value)) = pairs
                    .iter()
                    .find(|(k, _)| k.eq_ignore_ascii_case("Quality"))
            {
                let quality = fl2_data(value, &self.cancel)?;
                if quality.reals.len() != 1 || quality.reals[0] > 100. {
                    return Err("Expected QUALITY to be a scalar with value <= 100.".into());
                }
            }
            return Err(format!(
                "imwrite: {} files are not supported yet; only PNG images can be written",
                format.to_ascii_uppercase()
            ));
        }
        let data = fl2_data(&args[0], &self.cancel)?;
        if !matches!(
            data.class,
            "double" | "single" | "logical" | "uint8" | "uint16"
        ) {
            return Err(format!(
                "Expected input to be one of these types:\n\ndouble, single, logical, uint8, uint16\n\nInstead its type was {}.",
                data.class
            ));
        }
        if data.reals.is_empty() {
            return Err("Expected DATA to be nonempty.".into());
        }
        let mut depth = None;
        let mut alpha = None;
        let mut texts = Vec::new();
        for (key, value) in pairs {
            match key.to_ascii_lowercase().as_str() {
                "bitdepth" => {
                    let d = fl2_data(value, &self.cancel)?;
                    if d.reals.len() != 1 {
                        return Err("BitDepth must be a scalar.".into());
                    }
                    depth = Some(d.reals[0]);
                }
                "alpha" => alpha = Some(fl2_data(value, &self.cancel)?),
                "transparency" | "background" | "gamma" | "chromaticities" | "xresolution"
                | "yresolution" | "resolutionunit" | "significantbits" | "interlacetype"
                | "imagemodtime" | "colorspace" => {
                    return Err(format!(
                        "imwrite: the PNG parameter '{key}' is not supported yet"
                    ));
                }
                _ => {
                    let text = self.fl2_text(value).ok_or("Text chunk must be a string.")?;
                    let canonical = PNG_TEXT_KEYS
                        .iter()
                        .find(|k| k.eq_ignore_ascii_case(&key))
                        .map(|k| (*k).to_owned())
                        .unwrap_or(key);
                    if canonical.is_empty() || canonical.len() > 79 {
                        return Err("PNG text keywords must have 1 to 79 characters.".into());
                    }
                    texts.push((canonical, text));
                }
            }
        }
        let path = self.fl2_resolve(&filename)?;
        let bytes = self.fl2_png_encode(&data, map, depth, alpha.as_ref(), &texts)?;
        self.fl2_write_file(&path, &bytes)
    }

    fn fl2_png_encode(
        &self,
        data: &Fl2Data,
        map: Option<&Value>,
        depth: Option<f64>,
        alpha: Option<&Fl2Data>,
        texts: &[(String, String)],
    ) -> Result<Vec<u8>> {
        let (rows, cols) = (data.rows, data.cols);
        let mut palette = Vec::new();
        let color: u8;
        let bit_depth: u32;
        // Integer levels per pixel per channel, row-major.
        let mut levels: Vec<u16> = Vec::with_capacity(rows * cols * 2);
        if let Some(map) = map {
            let map = fl2_data(map, &self.cancel)?;
            if map.cols != 3 || map.rows == 0 || map.rows > 256 {
                return Err("The colormap must be a 1x3 to 256x3 array.".into());
            }
            for r in 0..map.rows {
                let mut entry = [0u8; 3];
                for (c, e) in entry.iter_mut().enumerate() {
                    let v = map.reals[c * map.rows + r];
                    if !(0. ..=1.).contains(&v) {
                        return Err("Colormap values must be in the range [0, 1].".into());
                    }
                    *e = (v * 255.).round() as u8;
                }
                palette.extend_from_slice(&entry);
            }
            let offset = if matches!(data.class, "double" | "single") {
                1.
            } else {
                0.
            };
            bit_depth = match depth {
                None => 8,
                Some(d) if [1., 2., 4., 8.].contains(&d) => d as u32,
                Some(_) => {
                    return Err("Invalid bitdepth for indexed image; must be 1, 2, 4, or 8.".into());
                }
            };
            color = 3;
            for r in 0..rows {
                for c in 0..cols {
                    let v = data.reals[c * rows + r] - offset;
                    if !(v >= 0.
                        && v.fract() == 0.
                        && v < map.rows as f64
                        && v < f64::from(1u32 << bit_depth))
                    {
                        return Err("Index values must be valid rows of the colormap.".into());
                    }
                    levels.push(v as u16);
                }
            }
        } else {
            let default = match data.class {
                "logical" => 1,
                "uint16" => 16,
                _ => 8,
            };
            bit_depth = match depth {
                None => default,
                Some(d) if alpha.is_none() && [1., 2., 4., 8., 16.].contains(&d) => d as u32,
                Some(d) if alpha.is_some() && [8., 16.].contains(&d) => d as u32,
                Some(_) if alpha.is_some() => {
                    return Err(
                        "Invalid bitdepth for grayscale image with alpha; must be 8 or 16.".into(),
                    );
                }
                Some(_) => {
                    return Err(
                        "Invalid bitdepth for grayscale image; must be 1, 2, 4, 8, or 16.".into(),
                    );
                }
            };
            let bit_depth = if alpha.is_some() && bit_depth < 8 {
                8
            } else {
                bit_depth
            };
            let top = f64::from((1u32 << bit_depth) - 1);
            let level = |class: &str, v: f64| -> u16 {
                if bit_depth == 1 && class != "double" && class != "single" {
                    return u16::from(v != 0.);
                }
                let scaled = match class {
                    "logical" => v * top,
                    "uint8" if bit_depth == 16 => v * 257.,
                    "uint8" if bit_depth == 8 => v,
                    "uint8" => (v * top / 255.).round(),
                    "uint16" if bit_depth == 16 => v,
                    "uint16" => (v * top / 65535.).round(),
                    _ if v.is_nan() => top,
                    _ => (v.clamp(0., 1.) * top).round(),
                };
                scaled as u16
            };
            if let Some(a) = alpha
                && (a.rows != rows || a.cols != cols)
            {
                return Err("The alpha data must be the same size as the image.".into());
            }
            color = if alpha.is_some() { 4 } else { 0 };
            for r in 0..rows {
                for c in 0..cols {
                    levels.push(level(data.class, data.reals[c * rows + r]));
                    if let Some(a) = alpha {
                        levels.push(level(a.class, a.reals[c * rows + r]));
                    }
                }
            }
            return self.fl2_png_assemble(rows, cols, bit_depth, color, &[], &levels, texts);
        }
        self.fl2_png_assemble(rows, cols, bit_depth, color, &palette, &levels, texts)
    }

    #[allow(clippy::too_many_arguments)]
    fn fl2_png_assemble(
        &self,
        rows: usize,
        cols: usize,
        depth: u32,
        color: u8,
        palette: &[u8],
        levels: &[u16],
        texts: &[(String, String)],
    ) -> Result<Vec<u8>> {
        let width = u32::try_from(cols).map_err(|_| "Image is too wide")?;
        let height = u32::try_from(rows).map_err(|_| "Image is too tall")?;
        let channels = fl2_png_channels(color);
        let per_row = cols * channels;
        let stride = (per_row * depth as usize).div_ceil(8);
        let mut raw = Vec::new();
        raw.try_reserve_exact((stride + 1) * rows)
            .map_err(|_| "Unable to allocate the image")?;
        for r in 0..rows {
            if r % 256 == 0 {
                self.check_cancel()?;
            }
            raw.push(0);
            let line = &levels[r * per_row..(r + 1) * per_row];
            match depth {
                8 => raw.extend(line.iter().map(|&v| v as u8)),
                16 => raw.extend(line.iter().flat_map(|v| v.to_be_bytes())),
                d => {
                    let mut byte = 0u8;
                    let mut used = 0;
                    for &v in line {
                        byte |= (v as u8) << (8 - d - used);
                        used += d;
                        if used == 8 {
                            raw.push(byte);
                            byte = 0;
                            used = 0;
                        }
                    }
                    if used > 0 {
                        raw.push(byte);
                    }
                }
            }
        }
        let mut encoder =
            flate2::write::ZlibEncoder::new(Vec::new(), flate2::Compression::default());
        encoder.write_all(&raw).map_err(fl2_io)?;
        let compressed = encoder.finish().map_err(fl2_io)?;
        let mut out = PNG_SIGNATURE.to_vec();
        let mut header = Vec::with_capacity(13);
        header.extend_from_slice(&width.to_be_bytes());
        header.extend_from_slice(&height.to_be_bytes());
        header.extend_from_slice(&[depth as u8, color, 0, 0, 0]);
        fl2_png_chunk(&mut out, b"IHDR", &header);
        if !palette.is_empty() {
            fl2_png_chunk(&mut out, b"PLTE", palette);
        }
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_secs())
            .unwrap_or(0);
        fl2_png_chunk(&mut out, b"tIME", &fl2_utc_time(now));
        for (key, text) in texts {
            let mut body: Vec<u8> = key.chars().map(|c| c as u32 as u8).collect();
            body.push(0);
            body.extend(text.chars().map(|c| u8::try_from(c as u32).unwrap_or(b'?')));
            fl2_png_chunk(&mut out, b"tEXt", &body);
        }
        fl2_png_chunk(&mut out, b"IDAT", &compressed);
        fl2_png_chunk(&mut out, b"IEND", &[]);
        Ok(out)
    }

    /// Detects an image format from the file's first bytes.
    fn fl2_sniff(bytes: &[u8]) -> Option<&'static str> {
        if bytes.starts_with(&PNG_SIGNATURE) {
            Some("png")
        } else if bytes.starts_with(&[0xFF, 0xD8, 0xFF]) {
            Some("jpg")
        } else if bytes.starts_with(b"BM") {
            Some("bmp")
        } else if bytes.starts_with(b"II*\0") || bytes.starts_with(b"MM\0*") {
            Some("tif")
        } else if bytes.starts_with(b"GIF8") {
            Some("gif")
        } else {
            None
        }
    }

    /// Opens an image file: (path, bytes). `missing` formats the not-found error.
    fn fl2_open_image(
        &self,
        args: &[Value],
        unknown: &str,
        missing: impl Fn(&str) -> String,
    ) -> Result<(PathBuf, Vec<u8>)> {
        let filename = self.fl2_name(&args[0], "The filename must be a character vector.")?;
        let format = match args.get(1) {
            Some(v) => Some(
                self.fl2_text(v)
                    .ok_or("The format must be a character vector.")?
                    .to_ascii_lowercase(),
            ),
            None => None,
        };
        if let Some(f) = &format
            && !IMAGE_FORMATS.iter().any(|(alias, _)| alias == f)
        {
            // Reference behavior: imread treats an unknown format as a PNG option name; imfinfo looks it up.
            return Err(if unknown.contains("the file") {
                format!(
                    "Expected input to match one of these strings:\n\n'backgroundcolor'\n\nThe input, '{f}', did not match any of the valid strings."
                )
            } else {
                format!("Could not find format \"{f}\" in format registry.")
            });
        }
        let mut path = self.fl2_resolve(&filename)?;
        if !path.is_file()
            && let Some(f) = &format
        {
            let candidate = self.fl2_resolve(&format!("{filename}.{f}"))?;
            if candidate.is_file() {
                path = candidate;
            }
        }
        if !path.is_file() {
            return Err(missing(&filename));
        }
        let bytes = fl2_read_limited(&path, MAX_BYTES)?;
        let detected = Self::fl2_sniff(&bytes);
        if let Some(format) = &format
            && let Some((_, expected)) = IMAGE_FORMATS.iter().find(|(alias, _)| alias == format)
            && detected != Some(*expected)
        {
            let label = match *expected {
                "jpg" => "JPEG".to_owned(),
                "tif" => "TIFF".to_owned(),
                other => other.to_ascii_uppercase(),
            };
            return Err(format!("'{}' is not a {label} file.", path.display()));
        }
        match detected {
            Some("png") => Ok((path, bytes)),
            Some(other) => Err(format!(
                "{} images are not supported yet; only PNG images can be read",
                other.to_ascii_uppercase()
            )),
            None => Err(unknown.into()),
        }
    }

    fn fl2_imread(&mut self, args: &[Value], nargout: usize) -> Result<Vec<Value>> {
        if args.is_empty() {
            return Err("Too few input arguments.".into());
        }
        if args.len() > 2 {
            return Err(
                "imread: only imread(filename) and imread(filename, fmt) are supported yet".into(),
            );
        }
        let (_, bytes) =
            self.fl2_open_image(args, "Unable to determine the file format.", |name| {
                format!("File \"{name}\" does not exist.")
            })?;
        let png = fl2_png_decode(&bytes, true, &self.cancel)?;
        self.fl2_png_values(&png, nargout.max(1))
    }

    fn fl2_png_values(&self, png: &Fl2Png, count: usize) -> Result<Vec<Value>> {
        let (rows, cols) = (png.height, png.width);
        let channels = fl2_png_channels(png.color);
        if matches!(png.color, 2 | 6) {
            return Err("Truecolor PNG images need N-D arrays, which are not supported yet".into());
        }
        let n = rows * cols;
        let column_major = |channel: usize| -> Vec<u16> {
            let mut v = Vec::with_capacity(n);
            for c in 0..cols {
                for r in 0..rows {
                    v.push(png.samples[(r * cols + c) * channels + channel]);
                }
            }
            v
        };
        let gray = column_major(0);
        let image = |levels: Vec<u16>| -> Result<Value> {
            Ok(match (png.color, png.depth) {
                (0, 1) => Value::Logical(LogicalMatrix::from_column_vec(
                    rows,
                    cols,
                    levels.into_iter().map(|v| v as u8).collect(),
                    &self.cancel,
                )?),
                (_, 16) => Value::Container(Array::Typed(TypedArray::from_ints(
                    NumericClass::Uint16,
                    rows,
                    cols,
                    levels.into_iter().map(i128::from),
                )?)),
                (3, _) | (_, 8) => Value::Container(Array::Typed(TypedArray::from_ints(
                    NumericClass::Uint8,
                    rows,
                    cols,
                    levels.into_iter().map(i128::from),
                )?)),
                (_, depth) => {
                    let scale = 255 / ((1u16 << depth) - 1);
                    Value::Container(Array::Typed(TypedArray::from_ints(
                        NumericClass::Uint8,
                        rows,
                        cols,
                        levels.into_iter().map(|v| i128::from(v * scale)),
                    )?))
                }
            })
        };
        let mut values = vec![image(gray)?];
        if count >= 2 {
            values.push(if png.color == 3 {
                let k = png.palette.len();
                let mut data = vec![0.; k * 3];
                for (r, entry) in png.palette.iter().enumerate() {
                    for c in 0..3 {
                        data[c * k + r] = f64::from(entry[c]) / 255.;
                    }
                }
                Self::fl2_matrix(k, 3, data)?
            } else {
                fl2_empty()
            });
        }
        if count >= 3 {
            values.push(if png.color == 4 {
                image(column_major(1))?
            } else {
                fl2_empty()
            });
        }
        Ok(values)
    }

    fn fl2_imfinfo(&mut self, args: &[Value], out: &mut Execution) -> Result<Value> {
        if args.is_empty() {
            return Err("Not enough input arguments.".into());
        }
        fl2_arity(args, 1, 2)?;
        let (path, bytes) =
            self.fl2_open_image(args, "Unable to determine file format.", |name| {
                format!("Unable to open file \"{name}\" for reading.")
            })?;
        let png = fl2_png_decode(&bytes, false, &self.cancel)?;
        let date = self.fl2_mod_date(&path, out)?;
        let bit_depth = match png.color {
            2 => 3 * u32::from(png.depth),
            6 => 4 * u32::from(png.depth),
            _ => u32::from(png.depth),
        };
        let color_type = match png.color {
            0 | 4 => "grayscale",
            3 => "indexed",
            _ => "truecolor",
        };
        let colormap = if png.color == 3 {
            let k = png.palette.len();
            let mut data = vec![0.; k * 3];
            for (r, entry) in png.palette.iter().enumerate() {
                for c in 0..3 {
                    data[c * k + r] = f64::from(entry[c]) / 255.;
                }
            }
            Self::fl2_matrix(k, 3, data)?
        } else {
            fl2_empty()
        };
        let transparency = if matches!(png.color, 4 | 6) {
            "alpha"
        } else if png.trns.is_some() {
            "simple"
        } else {
            "none"
        };
        let simple = match &png.trns {
            Some(t) if png.color == 3 => {
                Self::fl2_matrix(t.len(), 1, t.iter().map(|&b| f64::from(b) / 255.).collect())?
            }
            Some(t) if t.len() >= 2 => {
                let values: Vec<f64> = t
                    .chunks_exact(2)
                    .map(|p| f64::from(u16::from_be_bytes([p[0], p[1]])))
                    .collect();
                Self::fl2_matrix(1, values.len(), values)?
            }
            _ => fl2_empty(),
        };
        let background = match &png.background {
            Some(b) if png.color == 3 && !b.is_empty() => fl2_num(f64::from(b[0]) + 1.),
            Some(b) if b.len() >= 2 => {
                let values: Vec<f64> = b
                    .chunks_exact(2)
                    .map(|p| f64::from(u16::from_be_bytes([p[0], p[1]])))
                    .collect();
                Self::fl2_matrix(1, values.len(), values)?
            }
            _ => fl2_empty(),
        };
        let (x_res, y_res, unit) = match png.phys {
            Some((x, y, u)) => (
                fl2_num(f64::from(x)),
                fl2_num(f64::from(y)),
                fl2_str(if u == 1 { "meter" } else { "unknown" }),
            ),
            None => (fl2_empty(), fl2_empty(), fl2_empty()),
        };
        let mut values = vec![
            fl2_str(super::env_files::envx_path_text(&path)),
            date,
            fl2_num(bytes.len() as f64),
            fl2_str("png"),
            fl2_empty(),
            fl2_num(png.width as f64),
            fl2_num(png.height as f64),
            fl2_num(f64::from(bit_depth)),
            fl2_str(color_type),
            Self::fl2_matrix(1, 8, PNG_SIGNATURE.iter().map(|&b| f64::from(b)).collect())?,
            colormap,
            fl2_empty(),
            fl2_str(if png.interlace == 1 { "adam7" } else { "none" }),
            fl2_str(transparency),
            simple,
            background,
            fl2_empty(),
            fl2_empty(),
            png.gamma.map_or_else(fl2_empty, fl2_num),
            x_res,
            y_res,
            unit,
            fl2_empty(),
            fl2_empty(),
            fl2_empty(),
            fl2_empty(),
            png.time.clone().map_or_else(fl2_empty, fl2_str),
        ];
        for key in PNG_TEXT_KEYS {
            values.push(
                png.texts
                    .iter()
                    .find(|(k, _)| k == key)
                    .map_or_else(fl2_empty, |(_, t)| fl2_str(t.clone())),
            );
        }
        let other: Vec<&(String, String)> = png
            .texts
            .iter()
            .filter(|(k, _)| !PNG_TEXT_KEYS.contains(&k.as_str()))
            .collect();
        values.push(if other.is_empty() {
            fl2_empty()
        } else {
            let n = other.len();
            let mut items = Vec::with_capacity(2 * n);
            items.extend(other.iter().map(|(k, _)| fl2_str(k.clone())));
            items.extend(other.iter().map(|(_, t)| fl2_str(t.clone())));
            self.fl2_cell(n, 2, items)?
        });
        self.fl2_struct(PNG_INFO_FIELDS, values)
    }

    // ----------------------------------------------------------------- audio

    fn fl2_audiowrite(&mut self, args: &[Value], out: &mut Execution) -> Result<()> {
        if args.len() < 3 {
            return Err("Not enough input arguments.".into());
        }
        let filename = self.fl2_name(&args[0], "The filename must be a character vector.")?;
        let y = fl2_data(&args[1], &self.cancel)?;
        if !matches!(
            args[1],
            Value::Matrix(_) | Value::Container(Array::Typed(_))
        ) || !matches!(y.class, "uint8" | "int16" | "int32" | "single" | "double")
        {
            return Err(format!(
                "The value of 'y' is invalid. Expected input to be one of these types:\n\nuint8, int16, int32, single, double\n\nInstead its type was {}.",
                y.class
            ));
        }
        if y.reals.is_empty() {
            return Err("The value of 'y' is invalid. Expected input to be nonempty.".into());
        }
        let fs = fl2_data(&args[2], &self.cancel)?;
        if fs.reals.len() != 1 || fs.class == "logical" || !fs.reals[0].is_finite() {
            return Err(
                "The value of 'Fs' is invalid. Expected input to be a finite numeric scalar."
                    .into(),
            );
        }
        if fs.reals[0] <= 0. {
            return Err("The value of 'Fs' is invalid. Expected input to be positive.".into());
        }
        if fs.reals[0].fract() != 0. {
            return Err(
                "The value of 'Fs' is invalid. Expected input to be integer-valued.".into(),
            );
        }
        let rate = fs.reals[0];
        if rate > f64::from(u32::MAX) {
            return Err("The sample rate is too large for a WAV file.".into());
        }
        let ext = fl2_extension(&filename);
        if !AUDIO_EXTENSIONS.contains(&ext.as_str()) {
            return Err(format!(
                "Incorrect file extension for file {filename}.\nSupported extensions are:\n{}",
                AUDIO_EXTENSIONS
                    .iter()
                    .map(|e| format!("\t.{e}"))
                    .collect::<Vec<_>>()
                    .join("\n")
            ));
        }
        if ext != "wav" {
            return Err(format!(
                "audiowrite: .{ext} files are not supported yet; only WAV files can be written"
            ));
        }
        if !(args.len() - 3).is_multiple_of(2) {
            return Err("Parameter names and values must come in pairs.".into());
        }
        let mut bits = 16u16;
        let (mut title, mut artist, mut comment) = (None, None, None);
        for pair in args[3..].chunks(2) {
            let key = self
                .fl2_text(&pair[0])
                .ok_or("Parameter names must be character vectors.")?;
            match key.to_ascii_lowercase().as_str() {
                "bitspersample" => {
                    let d = fl2_data(&pair[1], &self.cancel)?;
                    let v = d.reals.first().copied().unwrap_or(f64::NAN);
                    if d.reals.len() != 1 || ![8., 16., 24., 32., 64.].contains(&v) {
                        return Err(format!(
                            "Invalid BitsPerSample {}.  Valid values are: 8,16,24,32,64",
                            if v.fract() == 0. {
                                format!("{v}")
                            } else {
                                format!("{v:.4}")
                            }
                        ));
                    }
                    bits = v as u16;
                }
                "bitrate" | "quality" => {
                    let parameter = if key.eq_ignore_ascii_case("bitrate") {
                        "BitRate"
                    } else {
                        "Quality"
                    };
                    return Err(format!(
                        "The parameter {parameter} is not supported for .wav files."
                    ));
                }
                "title" | "artist" | "comment" => {
                    let text = self.fl2_text(&pair[1]).ok_or_else(|| {
                        format!("The value of '{key}' is invalid. It must be a character vector.")
                    })?;
                    match key.to_ascii_lowercase().as_str() {
                        "title" => title = Some(text),
                        "artist" => artist = Some(text),
                        _ => comment = Some(text),
                    }
                }
                _ => {
                    return Err(format!(
                        "'{key}' is not a recognized parameter. For a list of valid name-value pair arguments, see the documentation for this function."
                    ));
                }
            }
        }
        let (rows, cols) = if y.rows == 1 {
            (y.cols, 1)
        } else {
            (y.rows, y.cols)
        };
        if cols > u16::MAX as usize || cols == 0 {
            return Err("Audio data must have between 1 and 65535 channels.".into());
        }
        let float_input = matches!(y.class, "single" | "double");
        let mut samples = y.reals;
        if float_input && samples.iter().any(|v| v.abs() > 1.) {
            for v in samples.iter_mut() {
                *v = v.clamp(-1., 1.);
            }
            self.lang_utils_call(
                "warning",
                &[
                    fl2_str("MATLAB:audiovideo:audiowrite:dataClipped"),
                    fl2_str("%s"),
                    fl2_str("Data clipped when writing file."),
                ],
                out,
            )?;
        }
        // Source integer width: values are signed relative to it.
        let source_bits = match y.class {
            "uint8" => 8,
            "int16" => 16,
            "int32" => 32,
            _ => 0,
        };
        let float_output = bits == 64 || (bits == 32 && y.class != "int32");
        let width = usize::from(bits / 8);
        if cols * width > u16::MAX as usize
            || (rate as u64) * (cols * width) as u64 > u64::from(u32::MAX)
        {
            return Err(
                "The channel count and sample width do not fit the WAV header fields.".into(),
            );
        }
        let mut data = Vec::new();
        data.try_reserve_exact(rows * cols * width)
            .map_err(|_| "Unable to allocate the audio data")?;
        for r in 0..rows {
            if r % 65536 == 0 {
                self.check_cancel()?;
            }
            for c in 0..cols {
                let v = samples[c * rows + r];
                let signed = if y.class == "uint8" { v - 128. } else { v };
                if float_output {
                    let real = if float_input {
                        v
                    } else {
                        signed / f64::from(1u32 << (source_bits - 1))
                    };
                    if bits == 32 {
                        data.extend_from_slice(&(real as f32).to_le_bytes());
                    } else {
                        data.extend_from_slice(&real.to_le_bytes());
                    }
                    continue;
                }
                let b = i32::from(bits);
                let level: i64 = if float_input {
                    let top = (1i64 << (b - 1)) as f64;
                    (v * top).round().clamp(-top, top - 1.) as i64
                } else if b >= source_bits {
                    (signed as i64) << (b - source_bits)
                } else {
                    (signed as i64) >> (source_bits - b)
                };
                match bits {
                    8 => data.push((level + 128) as u8),
                    16 => data.extend_from_slice(&(level as i16).to_le_bytes()),
                    24 => data.extend_from_slice(&(level as i32).to_le_bytes()[..3]),
                    _ => data.extend_from_slice(&(level as i32).to_le_bytes()),
                }
            }
        }
        let mut info = Vec::new();
        for (id, text) in [(b"INAM", &title), (b"IART", &artist), (b"ICMT", &comment)] {
            if let Some(text) = text {
                let mut body: Vec<u8> = text
                    .chars()
                    .map(|c| u8::try_from(c as u32).unwrap_or(b'?'))
                    .collect();
                body.push(0);
                info.extend_from_slice(id);
                info.extend_from_slice(&(body.len() as u32).to_le_bytes());
                info.extend_from_slice(&body);
                if body.len() % 2 == 1 {
                    info.push(0);
                }
            }
        }
        let block = (cols * width) as u16;
        let mut bytes = Vec::with_capacity(44 + data.len() + info.len() + 12);
        let list_len = if info.is_empty() { 0 } else { 12 + info.len() };
        let riff = 4 + 24 + 8 + data.len() + (data.len() & 1) + list_len;
        if riff > u32::MAX as usize {
            return Err("Audio data is too large for a WAV file.".into());
        }
        bytes.extend_from_slice(b"RIFF");
        bytes.extend_from_slice(&(riff as u32).to_le_bytes());
        bytes.extend_from_slice(b"WAVEfmt ");
        bytes.extend_from_slice(&16u32.to_le_bytes());
        bytes.extend_from_slice(&(if float_output { 3u16 } else { 1u16 }).to_le_bytes());
        bytes.extend_from_slice(&(cols as u16).to_le_bytes());
        bytes.extend_from_slice(&(rate as u32).to_le_bytes());
        bytes.extend_from_slice(&((rate as u32).wrapping_mul(u32::from(block))).to_le_bytes());
        bytes.extend_from_slice(&block.to_le_bytes());
        bytes.extend_from_slice(&bits.to_le_bytes());
        bytes.extend_from_slice(b"data");
        bytes.extend_from_slice(&(data.len() as u32).to_le_bytes());
        bytes.extend_from_slice(&data);
        if data.len() % 2 == 1 {
            bytes.push(0);
        }
        if !info.is_empty() {
            bytes.extend_from_slice(b"LIST");
            bytes.extend_from_slice(&((info.len() + 4) as u32).to_le_bytes());
            bytes.extend_from_slice(b"INFO");
            bytes.extend_from_slice(&info);
        }
        let path = self.fl2_resolve(&filename)?;
        self.fl2_write_file(&path, &bytes)
    }

    fn fl2_open_audio(&self, value: &Value) -> Result<(PathBuf, Fl2Wav)> {
        let filename = self.fl2_name(value, "The filename must be a character vector.")?;
        let path = self.fl2_resolve(&filename)?;
        if !path.is_file() {
            return Err("File not found.".into());
        }
        let ext = fl2_extension(&filename);
        if ext != "wav" && AUDIO_EXTENSIONS.contains(&ext.as_str()) {
            return Err(format!(
                "audioread: .{ext} files are not supported yet; only WAV files can be read"
            ));
        }
        let bytes = fl2_read_limited(&path, MAX_BYTES)?;
        Ok((path, fl2_wav_decode(&bytes)?))
    }

    fn fl2_audioread(&mut self, args: &[Value]) -> Result<Vec<Value>> {
        if args.is_empty() {
            return Err("Not enough input arguments.".into());
        }
        fl2_arity(args, 1, 3)?;
        let (_, wav) = self.fl2_open_audio(&args[0])?;
        let mut range = None;
        let mut kind = "double".to_owned();
        for (i, arg) in args.iter().enumerate().skip(1) {
            if let Some(text) = self.fl2_text(arg) {
                let lower = text.to_ascii_lowercase();
                if lower != "double" && lower != "native" {
                    return Err(format!(
                        "Expected datatype to match one of these strings:\n\n'double', 'native'\n\nThe input, '{text}', did not match any of the valid strings."
                    ));
                }
                kind = lower;
            } else if i == 1 {
                range = Some(fl2_data(arg, &self.cancel)?);
            } else {
                return Err("The datatype must be 'double' or 'native'.".into());
            }
        }
        let total = wav.frames();
        let (first, last) = match range {
            None => (1, total),
            Some(r) => {
                let bad = "Range must be a 1x2 vector of sample indexes with the first value less than or equal to the second value.";
                if r.reals.len() != 2 {
                    return Err(bad.into());
                }
                let (a, b) = (r.reals[0], r.reals[1]);
                if !(a >= 1.
                    && a.fract() == 0.
                    && a <= b
                    && (b.fract() == 0. || b == f64::INFINITY))
                {
                    return Err(bad.into());
                }
                let b = if b == f64::INFINITY { total as f64 } else { b };
                if b > total as f64 || a > total as f64 {
                    return Err(format!(
                        "Range requested is greater than the total number of samples in the file. TotalSamples is {total}."
                    ));
                }
                (a as usize, b as usize)
            }
        };
        let rows = last + 1 - first;
        let cols = wav.channels;
        Matrix::checked_len(rows, cols)?;
        let index = |r: usize, c: usize| (first - 1 + r) * cols + c;
        let native = kind == "native";
        let value = if native && wav.float && wav.bits == 64 || !native {
            let mut data = Vec::with_capacity(rows * cols);
            for c in 0..cols {
                self.check_cancel()?;
                for r in 0..rows {
                    let i = index(r, c);
                    data.push(if wav.float {
                        wav.real(i)
                    } else {
                        match wav.bits {
                            8 => (wav.int(i) - 128) as f64 / 128.,
                            16 => wav.int(i) as f64 / 32768.,
                            24 => wav.int(i) as f64 / 8388608.,
                            _ => wav.int(i) as f64 / 2147483648.,
                        }
                    });
                }
            }
            Self::fl2_matrix(rows, cols, data)?
        } else {
            let mut levels = Vec::with_capacity(rows * cols);
            for c in 0..cols {
                self.check_cancel()?;
                for r in 0..rows {
                    let i = index(r, c);
                    levels.push(match wav.bits {
                        24 => i128::from(wav.int(i)) << 8,
                        _ => i128::from(wav.int(i)),
                    });
                }
            }
            let typed = if wav.float {
                let reals: Vec<f64> = (0..cols)
                    .flat_map(|c| (0..rows).map(move |r| (r, c)))
                    .map(|(r, c)| wav.real(index(r, c)))
                    .collect();
                TypedArray::from_reals(NumericClass::Single, rows, cols, reals.into_iter())?
            } else {
                let class = match wav.bits {
                    8 => NumericClass::Uint8,
                    16 => NumericClass::Int16,
                    _ => NumericClass::Int32,
                };
                TypedArray::from_ints(class, rows, cols, levels.into_iter())?
            };
            Value::Container(Array::Typed(typed))
        };
        Ok(vec![value, fl2_num(f64::from(wav.rate))])
    }

    fn fl2_audioinfo(&mut self, args: &[Value]) -> Result<Value> {
        if args.is_empty() {
            return Err("Not enough input arguments.".into());
        }
        fl2_arity(args, 1, 1)?;
        let (path, wav) = self.fl2_open_audio(&args[0])?;
        let total = wav.frames();
        let text = |t: &Option<String>| t.clone().map_or_else(fl2_empty, fl2_str);
        self.fl2_struct(
            &[
                "Filename",
                "CompressionMethod",
                "NumChannels",
                "SampleRate",
                "TotalSamples",
                "Duration",
                "Title",
                "Comment",
                "Artist",
                "BitsPerSample",
            ],
            vec![
                fl2_str(super::env_files::envx_path_text(&path)),
                fl2_str("Uncompressed"),
                fl2_num(wav.channels as f64),
                fl2_num(f64::from(wav.rate)),
                fl2_num(total as f64),
                fl2_num(total as f64 / f64::from(wav.rate.max(1))),
                text(&wav.title),
                text(&wav.comment),
                text(&wav.artist),
                fl2_num(f64::from(wav.bits)),
            ],
        )
    }

    // ------------------------------------------------------------ importdata

    fn fl2_importdata(&mut self, args: &[Value]) -> Result<Vec<Value>> {
        if args.is_empty() {
            return Err("Not enough input arguments.".into());
        }
        fl2_arity(args, 1, 3)?;
        let filename = self.fl2_name(&args[0], "The filename must be a character vector.")?;
        if filename == "-pastespecial" {
            return Err("importdata('-pastespecial') is not supported".into());
        }
        let path = self.fl2_resolve(&filename)?;
        if !path.is_file() {
            return Err("Unable to open file.".into());
        }
        let ext = fl2_extension(&filename);
        if IMAGE_FORMATS.iter().any(|(e, _)| *e == ext) {
            let mut signature = Vec::with_capacity(PNG_SIGNATURE.len());
            fs::File::open(&path)
                .map_err(fl2_io)?
                .take(PNG_SIGNATURE.len() as u64)
                .read_to_end(&mut signature)
                .map_err(fl2_io)?;
            // An image extension alone does not make this an image. Keep
            // recognized formats on the image path (including its limits),
            // but let ordinary text with an image extension use text import.
            if Self::fl2_sniff(&signature).is_some() {
                let mut values = self.fl2_imread(&args[..1], 1)?;
                values.extend([fl2_str(""), fl2_num(0.)]);
                return Ok(values);
            }
        }
        if AUDIO_EXTENSIONS.contains(&ext.as_str()) || ext == "au" || ext == "snd" {
            let mut audio = self.fl2_audioread(&args[..1])?;
            let fs = audio.pop().unwrap();
            let data = audio.pop().unwrap();
            return Ok(vec![
                self.fl2_struct(&["data", "fs"], vec![data, fs])?,
                fl2_str(""),
                fl2_num(0.),
            ]);
        }
        if ext == "mat" {
            let catalog = crate::files::inspect_workspace(&path, &self.cancel)?;
            let order: Vec<String> = catalog.variables().map(|v| v.name.clone()).collect();
            let selected: BTreeSet<String> = order.iter().cloned().collect();
            let mut loaded = crate::files::load_selected_arrays(&catalog, &selected, &self.cancel)?;
            let value = if order.len() == 1 {
                Value::Container(loaded.remove(&order[0]).ok_or("MAT variable is missing")?)
            } else {
                let names: Vec<&str> = order.iter().map(String::as_str).collect();
                let values = order
                    .iter()
                    .map(|n| {
                        loaded
                            .remove(n)
                            .map(Value::Container)
                            .ok_or("MAT variable is missing")
                    })
                    .collect::<std::result::Result<Vec<_>, _>>()?;
                self.fl2_struct(&names, values)?
            };
            return Ok(vec![value, fl2_str(""), fl2_num(0.)]);
        }
        let delim = match args.get(1) {
            Some(v) => {
                let text = self
                    .fl2_text(v)
                    .ok_or("The delimiter must be a character vector.")?;
                let text = match text.as_str() {
                    "\\t" => "\t".to_owned(),
                    _ => text,
                };
                let mut chars = text.chars();
                match (chars.next(), chars.next()) {
                    (Some(' '), None) => Some(Fl2Delim::Space),
                    (Some(c), None) => Some(Fl2Delim::Char(c)),
                    _ => return Err("importdata: the delimiter must be one character".into()),
                }
            }
            None => None,
        };
        let header = match args.get(2) {
            Some(v) => {
                let d = fl2_data(v, &self.cancel)?;
                match d.reals.as_slice() {
                    [h] if *h >= 0. && h.fract() == 0. => Some(*h as usize),
                    _ => {
                        return Err(
                            "The number of header lines must be a nonnegative integer.".into()
                        );
                    }
                }
            }
            None => None,
        };
        let bytes = fl2_read_limited(&path, 64 * 1024 * 1024)?;
        let text = String::from_utf8_lossy(&bytes);
        let mut lines: Vec<&str> = text.lines().collect();
        while lines.last().is_some_and(|l| l.trim().is_empty()) {
            lines.pop();
        }
        self.fl2_import_text(&lines, delim, header)
    }

    fn fl2_import_text(
        &self,
        lines: &[&str],
        delim: Option<Fl2Delim>,
        header: Option<usize>,
    ) -> Result<Vec<Value>> {
        if lines.is_empty() {
            return Ok(vec![fl2_empty(), fl2_str(""), fl2_num(0.)]);
        }
        let delim = delim.unwrap_or_else(|| {
            if lines.iter().any(|l| l.contains(',')) {
                Fl2Delim::Char(',')
            } else if lines.iter().any(|l| l.contains('\t')) {
                Fl2Delim::Char('\t')
            } else {
                Fl2Delim::Space
            }
        });
        let header = match header {
            Some(h) => h.min(lines.len()),
            None => lines
                .iter()
                .position(|l| fl2_data_row(&delim.split(l), delim).is_some())
                .unwrap_or(lines.len()),
        };
        // Data rows: (labels, numbers); stop at the first row that is not data.
        let mut rows: Vec<(Vec<String>, Vec<f64>)> = Vec::new();
        for (i, line) in lines[header..].iter().enumerate() {
            if i % 4096 == 0 {
                self.check_cancel()?;
            }
            if line.trim().is_empty() {
                continue;
            }
            let tokens = delim.split(line);
            match fl2_data_row(&tokens, delim) {
                Some((labels, numbers)) => rows.push((tokens[..labels].to_vec(), numbers)),
                None => break,
            }
        }
        if rows.is_empty() {
            // Text only: one cell per line.
            let items: Vec<Value> = lines.iter().map(|l| fl2_str(*l)).collect();
            let n = items.len();
            return Ok(vec![
                self.fl2_cell(n, 1, items)?,
                fl2_str(delim.text()),
                fl2_num(header as f64),
            ]);
        }
        let columns = rows.iter().map(|(_, n)| n.len()).max().unwrap_or(0);
        let count = rows.len();
        Matrix::checked_len(count, columns)?;
        let mut data = vec![f64::NAN; count * columns];
        for (r, (_, numbers)) in rows.iter().enumerate() {
            for (c, v) in numbers.iter().enumerate() {
                data[c * count + r] = *v;
            }
        }
        let matrix = Self::fl2_matrix(count, columns, data)?;
        let labels = rows.iter().map(|(l, _)| l.len()).max().unwrap_or(0);
        let tail = vec![fl2_str(delim.text()), fl2_num(header as f64)];
        if header == 0 && labels == 0 {
            let mut values = vec![matrix];
            values.extend(tail);
            return Ok(values);
        }
        let last: Vec<String> = if header > 0 {
            delim.split(lines[header - 1])
        } else {
            Vec::new()
        };
        let value = if labels == 0 {
            // Header lines: earlier lines whole in column 1, the last split.
            let width = last.len().max(1);
            fl2_cell_slots(header, width)?;
            let mut grid = vec![fl2_empty(); header * width];
            for (r, line) in lines[..header - 1].iter().enumerate() {
                grid[r] = fl2_str(*line);
            }
            for (c, token) in last.iter().enumerate() {
                grid[c * header + header - 1] = fl2_str(token.clone());
            }
            let textdata = self.fl2_cell(header, width, grid)?;
            if last.len() == columns {
                let colheaders =
                    self.fl2_cell_row(last.iter().map(|t| fl2_str(t.clone())).collect())?;
                self.fl2_struct(
                    &["data", "textdata", "colheaders"],
                    vec![matrix, textdata, colheaders],
                )?
            } else {
                self.fl2_struct(&["data", "textdata"], vec![matrix, textdata])?
            }
        } else if header == 0 {
            fl2_cell_slots(count, labels)?;
            let mut grid = vec![fl2_str(""); count * labels];
            for (r, (l, _)) in rows.iter().enumerate() {
                for (c, token) in l.iter().enumerate() {
                    grid[c * count + r] = fl2_str(token.clone());
                }
            }
            let textdata = self.fl2_cell(count, labels, grid.clone())?;
            let rowheaders = self.fl2_cell(count, 1, grid[..count].to_vec())?;
            self.fl2_struct(
                &["data", "textdata", "rowheaders"],
                vec![matrix, textdata, rowheaders],
            )?
        } else {
            let width = last.len().max(labels + columns);
            let height = header + count;
            fl2_cell_slots(height, width)?;
            let mut grid = vec![fl2_empty(); height * width];
            for (r, line) in lines[..header - 1].iter().enumerate() {
                grid[r] = fl2_str(*line);
            }
            for (c, token) in last.iter().enumerate() {
                grid[c * height + header - 1] = fl2_str(token.clone());
            }
            for (r, (l, _)) in rows.iter().enumerate() {
                for c in 0..width {
                    grid[c * height + header + r] = fl2_str(l.get(c).cloned().unwrap_or_default());
                }
            }
            let textdata = self.fl2_cell(height, width, grid)?;
            self.fl2_struct(&["data", "textdata"], vec![matrix, textdata])?
        };
        let mut values = vec![value];
        values.extend(tail);
        Ok(values)
    }

    // -------------------------------------------------------------- archives

    /// Expand file specs (files, folders, wildcards) relative to `root`.
    fn fl2_collect(
        &self,
        specs: &[String],
        root: Option<&str>,
        skip: &Path,
        expand_wildcards: bool,
    ) -> Result<Vec<Fl2Entry>> {
        let mut entries = Vec::new();
        for spec in specs {
            self.check_cancel()?;
            if entries.len() > MAX_ENTRIES {
                return Err("Too many files to archive".into());
            }
            let absolute = Path::new(spec).is_absolute();
            let full_text = match root {
                Some(r) if !absolute => fl2_join(r, spec),
                _ => self.fl2_full_text(spec),
            };
            let reported_base = if absolute {
                spec.clone()
            } else {
                spec.replace('\\', "/")
            };
            if expand_wildcards && fl2_is_wild(fl2_base_name(spec)) {
                let folder_text = fl2_parent_text(&full_text).to_owned();
                let folder = self.fl2_resolve(&folder_text)?;
                let pattern = fl2_base_name(spec);
                let mut names: Vec<String> = fs::read_dir(&folder)
                    .map_err(|_| format!("File \"{full_text}\" does not exist."))?
                    .filter_map(|e| e.ok())
                    .map(|e| e.file_name().to_string_lossy().into_owned())
                    .filter(|n| fl2_wildcard(pattern, n))
                    .collect();
                // The reference adds matching files before expanding folders (Q4).
                names.sort_by_cached_key(|n| (folder.join(n).is_dir(), n.clone()));
                let spec_parent = fl2_parent_text(&reported_base).to_owned();
                for name in names {
                    let path = folder.join(&name);
                    if path == skip {
                        continue;
                    }
                    let relative = if spec_parent.is_empty() {
                        name.clone()
                    } else {
                        format!("{}/{name}", spec_parent.replace('\\', "/"))
                    };
                    let reported = if absolute {
                        fl2_join(&spec_parent, &name)
                    } else {
                        relative.clone()
                    };
                    // Names returned by read_dir are literal, even if they
                    // contain wildcard characters on the host filesystem.
                    entries.extend(self.fl2_collect(&[reported], root, skip, false)?);
                    if entries.len() > MAX_ENTRIES {
                        return Err("Too many files to archive".into());
                    }
                }
                continue;
            }
            let path = self.fl2_resolve(&full_text)?;
            if path.is_dir() {
                let base_entry = if absolute {
                    fl2_base_name(spec.trim_end_matches(['/', '\\'])).to_owned()
                } else {
                    reported_base.trim_end_matches('/').to_owned()
                };
                let mut stack = vec![(path.clone(), String::new())];
                let mut found = Vec::new();
                while let Some((dir, prefix)) = stack.pop() {
                    self.check_cancel()?;
                    let mut items: Vec<_> = fs::read_dir(&dir)
                        .map_err(fl2_io)?
                        .filter_map(|e| e.ok())
                        .collect();
                    items.sort_by_key(|e| e.file_name());
                    for item in items.into_iter().rev() {
                        let name = item.file_name().to_string_lossy().into_owned();
                        let child = item.path();
                        let rel = if prefix.is_empty() {
                            name
                        } else {
                            format!("{prefix}/{name}")
                        };
                        if child.is_dir() {
                            stack.push((child, rel));
                        } else if child != skip && child.is_file() {
                            found.push((child, rel));
                        }
                    }
                    if found.len() > MAX_ENTRIES {
                        return Err("Too many files to archive".into());
                    }
                }
                found.sort_by(|a, b| a.1.cmp(&b.1));
                for (child, rel) in found {
                    let entry = format!("{base_entry}/{rel}");
                    let reported = if absolute {
                        fl2_join(spec.trim_end_matches(['/', '\\']), &rel)
                    } else {
                        entry.clone()
                    };
                    entries.push(Fl2Entry {
                        path: child,
                        entry,
                        reported,
                    });
                }
                continue;
            }
            if !path.is_file() {
                return Err(format!("File \"{full_text}\" does not exist."));
            }
            let entry = if absolute {
                fl2_base_name(spec).to_owned()
            } else {
                reported_base.clone()
            };
            entries.push(Fl2Entry {
                path,
                entry,
                reported: reported_base,
            });
        }
        if entries.len() > MAX_ENTRIES {
            return Err("Too many files to archive".into());
        }
        Ok(entries)
    }

    fn fl2_pack(&mut self, name: &str, args: &[Value], nargout: usize) -> Result<Vec<Value>> {
        fl2_arity(args, 2, 3)?;
        let mut archive =
            self.fl2_name(&args[0], "The archive name must be a character vector.")?;
        let lower = archive.to_ascii_lowercase();
        let gz = name == "tar" && (lower.ends_with(".tgz") || lower.ends_with(".tar.gz"));
        if name == "zip" && fl2_extension(&archive) != "zip" {
            archive.push_str(".zip");
        } else if name == "tar" && !gz && fl2_extension(&archive) != "tar" {
            archive.push_str(".tar");
        }
        let specs = self.fl2_text_list(
            &args[1],
            "File names must be character vectors or a cell array of them.",
        )?;
        let root = match args.get(2) {
            Some(v) => Some(self.fl2_name(v, "The root folder must be a character vector.")?),
            None => None,
        };
        let target = self.fl2_resolve(&archive)?;
        let entries = self.fl2_collect(&specs, root.as_deref(), &target, true)?;
        let mut total = 0u64;
        for e in &entries {
            total += fs::metadata(&e.path).map_err(fl2_io)?.len();
            if total > MAX_BYTES {
                return Err(format!(
                    "Files to archive exceed the {MAX_BYTES}-byte limit"
                ));
            }
        }
        let bytes = if name == "zip" {
            let mut writer = zip::ZipWriter::new(std::io::Cursor::new(Vec::new()));
            let options = zip::write::SimpleFileOptions::default()
                .compression_method(zip::CompressionMethod::Deflated);
            for e in &entries {
                self.check_cancel()?;
                let content = fl2_read_limited(&e.path, MAX_BYTES)?;
                writer
                    .start_file(e.entry.as_str(), options)
                    .map_err(|err| err.to_string())?;
                writer.write_all(&content).map_err(fl2_io)?;
            }
            writer.finish().map_err(|err| err.to_string())?.into_inner()
        } else {
            let mut tar = Vec::new();
            for e in &entries {
                self.check_cancel()?;
                let content = fl2_read_limited(&e.path, MAX_BYTES)?;
                let mtime = fs::metadata(&e.path)
                    .and_then(|m| m.modified())
                    .ok()
                    .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
                    .map_or(0, |d| d.as_secs());
                let header_name = if fl2_tar_name_parts(&e.entry).is_some() {
                    e.entry.as_str()
                } else {
                    // GNU long-name records carry the complete UTF-8 name
                    // for the following regular-file header.
                    tar.extend_from_slice(&fl2_tar_header(
                        "././@LongLink",
                        (e.entry.len() + 1) as u64,
                        mtime,
                        b'L',
                    )?);
                    tar.extend_from_slice(e.entry.as_bytes());
                    tar.push(0);
                    tar.resize(tar.len().div_ceil(512) * 512, 0);
                    "././@LongLink"
                };
                tar.extend_from_slice(&fl2_tar_header(
                    header_name,
                    content.len() as u64,
                    mtime,
                    b'0',
                )?);
                tar.extend_from_slice(&content);
                tar.resize(tar.len().div_ceil(512) * 512, 0);
            }
            tar.resize(tar.len() + 1024, 0);
            if gz {
                let mut encoder =
                    flate2::write::GzEncoder::new(Vec::new(), flate2::Compression::default());
                encoder.write_all(&tar).map_err(fl2_io)?;
                encoder.finish().map_err(fl2_io)?
            } else {
                tar
            }
        };
        self.fl2_write_file(&target, &bytes)?;
        if nargout == 0 {
            return Ok(Vec::new());
        }
        let names = entries.into_iter().map(|e| fl2_str(e.reported)).collect();
        Ok(vec![self.fl2_cell_row(names)?])
    }

    fn fl2_unpack(&mut self, name: &str, args: &[Value], nargout: usize) -> Result<Vec<Value>> {
        fl2_arity(args, 1, 2)?;
        let archive = self.fl2_name(&args[0], "The archive name must be a character vector.")?;
        let output = match args.get(1) {
            Some(v) => Some(self.fl2_name(v, "The output folder must be a character vector.")?),
            None => None,
        };
        let upper = name.to_ascii_uppercase();
        let mut path = self.fl2_resolve(&archive)?;
        if !path.is_file() && !fl2_has_extension(&archive) {
            let candidate = self.fl2_resolve(&format!("{archive}.{name}", name = &name[2..]))?;
            if candidate.is_file() {
                path = candidate;
            }
        }
        let full = self.fl2_full_text(&archive);
        if !path.is_file() {
            return Err(format!(
                "Function {upper} was unable to find file ''{full}''."
            ));
        }
        let bytes = fl2_read_limited(&path, MAX_BYTES)?;
        let destination = match &output {
            Some(o) => self.fl2_resolve(o)?,
            None => self
                .current_folder()
                .ok_or("No current folder is available")?
                .to_owned(),
        };
        // (relative path, contents) of every file, in archive order.
        let mut files: Vec<(PathBuf, Vec<u8>)> = Vec::new();
        let mut directories = Vec::new();
        let mut total = 0u64;
        if name == "unzip" {
            let mut zip = zip::ZipArchive::new(std::io::Cursor::new(bytes.as_slice()))
                .map_err(|_| format!("Invalid zip file \"{full}\"."))?;
            if zip.len() > MAX_ENTRIES {
                return Err("The archive has too many entries".into());
            }
            for i in 0..zip.len() {
                self.check_cancel()?;
                let mut entry = zip
                    .by_index(i)
                    .map_err(|_| format!("Invalid zip file \"{full}\"."))?;
                let raw = entry.name().map_err(|e| e.to_string())?.into_owned();
                let relative = fl2_safe_relative(&raw)
                    .ok_or_else(|| format!("Unsafe archive entry name: {raw}"))?;
                if entry.is_dir() {
                    directories.push(relative);
                    continue;
                }
                let content = fl2_inflate(&mut entry, MAX_BYTES - total)?;
                total += content.len() as u64;
                files.push((relative, content));
            }
        } else {
            let tar = if bytes.starts_with(&[0x1f, 0x8b]) {
                fl2_inflate(
                    flate2::read::MultiGzDecoder::new(bytes.as_slice()),
                    MAX_BYTES,
                )
                .map_err(|_| format!("Invalid Tar file {full}."))?
            } else {
                bytes
            };
            let invalid = || format!("Invalid Tar file {full}.");
            let empty = tar.len() >= 512 && tar[..512].iter().all(|&b| b == 0);
            if tar.len() < 512 || (!empty && !fl2_tar_valid(&tar[..512])) {
                return Err(invalid());
            }
            let mut pos = 0;
            let mut long_name: Option<String> = None;
            while pos + 512 <= tar.len() {
                self.check_cancel()?;
                let block = &tar[pos..pos + 512];
                if block.iter().all(|&b| b == 0) {
                    break;
                }
                if !fl2_tar_valid(block) {
                    return Err(invalid());
                }
                let field = |a: usize, b: usize| -> String {
                    let raw = block[a..b].split(|&c| c == 0).next().unwrap_or(&[]);
                    String::from_utf8_lossy(raw).into_owned()
                };
                let size = fl2_tar_octal(&block[124..136]).ok_or_else(invalid)? as usize;
                let start = pos + 512;
                let end = start
                    .checked_add(size)
                    .filter(|&e| e <= tar.len())
                    .ok_or_else(invalid)?;
                let mut entry_name = field(0, 100);
                if &block[257..262] == b"ustar" {
                    let prefix = field(345, 500);
                    if !prefix.is_empty() {
                        entry_name = format!("{prefix}/{entry_name}");
                    }
                }
                if let Some(long) = long_name.take() {
                    entry_name = long;
                }
                let kind = block[156];
                match kind {
                    b'L' => {
                        let raw = tar[start..end].split(|&c| c == 0).next().unwrap_or(&[]);
                        long_name = Some(String::from_utf8_lossy(raw).into_owned());
                    }
                    // "./" names the output folder itself.
                    b'5' if entry_name
                        .split(['/', '\\'])
                        .all(|p| p.is_empty() || p == ".") => {}
                    b'0' | 0 | b'5' => {
                        let relative = fl2_safe_relative(&entry_name)
                            .ok_or_else(|| format!("Unsafe archive entry name: {entry_name}"))?;
                        if kind == b'5' || entry_name.ends_with('/') {
                            directories.push(relative);
                        } else {
                            total += size as u64;
                            if total > MAX_BYTES {
                                return Err(format!(
                                    "Archive contents exceed the {MAX_BYTES}-byte limit"
                                ));
                            }
                            files.push((relative, tar[start..end].to_vec()));
                        }
                    }
                    _ => {}
                }
                if files.len() + directories.len() > MAX_ENTRIES {
                    return Err("The archive has too many entries".into());
                }
                pos = start + size.div_ceil(512) * 512;
            }
        }
        fs::create_dir_all(&destination).map_err(fl2_io)?;
        for directory in &directories {
            fl2_create_dirs(&destination, directory)?;
        }
        let mut reported = Vec::with_capacity(files.len());
        for (relative, content) in &files {
            if let Some(parent) = relative.parent() {
                fl2_create_dirs(&destination, parent)?;
            }
            let target = destination.join(relative);
            if fs::symlink_metadata(&target).is_ok_and(|m| m.file_type().is_symlink()) {
                return Err(format!(
                    "Refusing to write through the link {}",
                    target.display()
                ));
            }
            self.fl2_write_file(&target, content)?;
            let shown = relative.to_string_lossy().into_owned();
            reported.push(match &output {
                Some(o) => fl2_join(o, &shown),
                None => fl2_native(&shown),
            });
        }
        if name == "unzip" {
            reported.reverse();
        }
        if nargout == 0 {
            return Ok(Vec::new());
        }
        let names = reported.into_iter().map(fl2_str).collect();
        Ok(vec![self.fl2_cell_row(names)?])
    }

    /// Files named by gzip/gunzip specs: (path, name text as given or expanded).
    fn fl2_gz_inputs(
        &self,
        specs: &[String],
        suffix: Option<&str>,
    ) -> Result<Vec<(PathBuf, String)>> {
        let mut inputs = Vec::new();
        for spec in specs {
            self.check_cancel()?;
            if fl2_is_wild(fl2_base_name(spec)) {
                let parent = fl2_parent_text(spec).to_owned();
                let folder = if parent.is_empty() {
                    self.current_folder()
                        .ok_or("No current folder is available")?
                        .to_owned()
                } else {
                    self.fl2_resolve(&parent)?
                };
                let pattern = fl2_base_name(spec);
                let mut names: Vec<String> = fs::read_dir(&folder)
                    .map_err(|_| format!("File \"{}\" does not exist.", self.fl2_full_text(spec)))?
                    .filter_map(|e| e.ok())
                    .filter(|e| e.path().is_file())
                    .map(|e| e.file_name().to_string_lossy().into_owned())
                    .filter(|n| fl2_wildcard(pattern, n))
                    .filter(|n| suffix.is_none_or(|s| n.to_ascii_lowercase().ends_with(s)))
                    .collect();
                names.sort();
                for name in names {
                    inputs.push((folder.join(&name), fl2_join(&parent, &name)));
                }
            } else {
                let path = self.fl2_resolve(spec)?;
                if path.is_dir() {
                    if suffix.is_none() {
                        for entry in self.fl2_collect(
                            std::slice::from_ref(spec),
                            None,
                            Path::new(""),
                            false,
                        )? {
                            inputs.push((entry.path, fl2_native(&entry.reported)));
                        }
                        if inputs.len() > MAX_ENTRIES {
                            return Err("Too many files".into());
                        }
                        continue;
                    }
                    let mut names: Vec<String> = fs::read_dir(&path)
                        .map_err(fl2_io)?
                        .filter_map(|e| e.ok())
                        .filter(|e| e.path().is_file())
                        .map(|e| e.file_name().to_string_lossy().into_owned())
                        .collect();
                    names.sort();
                    for name in names {
                        inputs.push((path.join(&name), fl2_join(spec, &name)));
                    }
                } else if path.is_file() {
                    inputs.push((path, spec.clone()));
                } else {
                    return Err(format!(
                        "File \"{}\" does not exist.",
                        self.fl2_full_text(spec)
                    ));
                }
            }
            if inputs.len() > MAX_ENTRIES {
                return Err("Too many files".into());
            }
        }
        Ok(inputs)
    }

    fn fl2_gzip(&mut self, args: &[Value], nargout: usize) -> Result<Vec<Value>> {
        fl2_arity(args, 1, 2)?;
        let specs = self.fl2_text_list(
            &args[0],
            "File names must be character vectors or a cell array of them.",
        )?;
        let output = match args.get(1) {
            Some(v) => Some(self.fl2_name(v, "The output folder must be a character vector.")?),
            None => None,
        };
        let inputs = self.fl2_gz_inputs(&specs, None)?;
        if let Some(o) = &output {
            fs::create_dir_all(self.fl2_resolve(o)?).map_err(fl2_io)?;
        }
        let mut reported = Vec::with_capacity(inputs.len());
        for (path, text) in inputs {
            let content = fl2_read_limited(&path, MAX_BYTES)?;
            let base = fl2_base_name(&text).to_owned();
            let name = match &output {
                Some(o) => fl2_join(o, &format!("{base}.gz")),
                None => format!("{text}.gz"),
            };
            let mut encoder = flate2::GzBuilder::new()
                .filename(base.as_bytes())
                .write(Vec::new(), flate2::Compression::default());
            encoder.write_all(&content).map_err(fl2_io)?;
            let bytes = encoder.finish().map_err(fl2_io)?;
            self.fl2_write_file(&self.fl2_resolve(&name)?, &bytes)?;
            reported.push(fl2_str(name));
        }
        if nargout == 0 {
            return Ok(Vec::new());
        }
        Ok(vec![self.fl2_cell_row(reported)?])
    }

    fn fl2_gunzip(&mut self, args: &[Value], nargout: usize) -> Result<Vec<Value>> {
        fl2_arity(args, 1, 2)?;
        let specs = self.fl2_text_list(
            &args[0],
            "File names must be character vectors or a cell array of them.",
        )?;
        let output = match args.get(1) {
            Some(v) => Some(self.fl2_name(v, "The output folder must be a character vector.")?),
            None => None,
        };
        let inputs = self.fl2_gz_inputs(&specs, Some(".gz"))?;
        if let Some(o) = &output {
            fs::create_dir_all(self.fl2_resolve(o)?).map_err(fl2_io)?;
        }
        let mut reported = Vec::with_capacity(inputs.len());
        for (path, text) in inputs {
            let bytes = fl2_read_limited(&path, MAX_BYTES)?;
            let shown = path.to_string_lossy().replace('\\', "/");
            if !bytes.starts_with(&[0x1f, 0x8b]) {
                return Err(format!("File \"{shown}\" is not in GZIP format."));
            }
            let content = fl2_inflate(
                flate2::read::MultiGzDecoder::new(bytes.as_slice()),
                MAX_BYTES,
            )
            .map_err(|_| format!("File \"{shown}\" is not in GZIP format."))?;
            let base = fl2_base_name(&text);
            let stripped = &base[..base.rfind('.').unwrap_or(base.len())];
            let name = match &output {
                Some(o) => fl2_join(o, stripped),
                None => fl2_join(fl2_parent_text(&text), stripped),
            };
            let target = self.fl2_resolve(&name)?;
            if target == path
                || fs::canonicalize(&target)
                    .is_ok_and(|p| fs::canonicalize(&path).is_ok_and(|s| p == s))
            {
                return Err("gunzip: output would overwrite the compressed source.".into());
            }
            self.fl2_write_file(&target, &content)?;
            reported.push(fl2_str(name));
        }
        if nargout == 0 {
            return Ok(Vec::new());
        }
        Ok(vec![self.fl2_cell_row(reported)?])
    }

    // ------------------------------------------------------------ fileattrib

    fn fl2_fileattrib(
        &mut self,
        args: &[Value],
        nargout: usize,
        out: &mut Execution,
    ) -> Result<Vec<Value>> {
        fl2_arity(args, 0, 4)?;
        let name = match args.first() {
            Some(v) => self.fl2_text(v).ok_or("Argument must contain a string.")?,
            None => self
                .current_folder()
                .ok_or("No current folder is available")?
                .to_string_lossy()
                .into_owned(),
        };
        let mut texts = Vec::new();
        for v in args.iter().skip(1) {
            texts.push(self.fl2_text(v).ok_or("Argument must contain a string.")?);
        }
        let failure = |message: &str, id: &str| -> Result<Vec<Value>> {
            if nargout == 0 {
                return Err(message.into());
            }
            Ok(vec![fl2_bool(false), fl2_str(message), fl2_str(id)])
        };
        let full = self.fl2_full_text(&name);
        let targets: Vec<(PathBuf, String)> = if fl2_is_wild(fl2_base_name(&name)) {
            let parent = fl2_parent_text(&full).to_owned();
            let folder = self.fl2_resolve(&parent)?;
            let pattern = fl2_base_name(&name);
            let mut found: Vec<String> = fs::read_dir(&folder)
                .map(|r| {
                    r.filter_map(|e| e.ok())
                        .map(|e| e.file_name().to_string_lossy().into_owned())
                        .filter(|n| fl2_wildcard(pattern, n))
                        .collect()
                })
                .unwrap_or_default();
            if found.len() > MAX_ENTRIES {
                return Err(format!(
                    "fileattrib: more than {MAX_ENTRIES} files match the wildcard"
                ));
            }
            found.sort();
            found
                .into_iter()
                .map(|n| (folder.join(&n), fl2_join(&parent, &n)))
                .collect()
        } else {
            let path = self.fl2_resolve(&name)?;
            if fs::symlink_metadata(&path).is_ok() {
                vec![(path, fl2_trim_name(&full))]
            } else {
                Vec::new()
            }
        };
        if targets.is_empty() {
            return failure(
                "No such file or directory.",
                "MATLAB:FILEATTRIB:CannotFindFile",
            );
        }
        if let Some(mode) = texts.first() {
            if texts.len() > 2 && texts[2] != "s" {
                return failure(
                    "Invalid modifier string.",
                    "MATLAB:FILEATTRIB:InvalidRecursionFlag",
                );
            }
            if texts.len() > 2 {
                return Err("fileattrib: recursive ('s') changes are not supported yet".into());
            }
            let users = texts.get(1).map(String::as_str).unwrap_or("");
            let Some(changes) = fl2_parse_mode(mode, users) else {
                return failure(
                    "Invalid file mode characters on the current platform.",
                    "MATLAB:FILEATTRIB:InvalidMode",
                );
            };
            for (path, _) in &targets {
                self.check_cancel()?;
                if let Err(message) = fl2_apply_mode(path, &changes) {
                    return failure(&message, "MATLAB:FILEATTRIB:CannotChangeAttributes");
                }
            }
            if nargout == 0 {
                return Ok(Vec::new());
            }
            return Ok(vec![fl2_bool(true), fl2_str(""), fl2_str("")]);
        }
        let mut slots = Vec::with_capacity(targets.len() * FILEATTRIB_FIELDS.len());
        let count = targets.len();
        for (path, shown) in targets {
            self.check_cancel()?;
            let metadata = fs::metadata(&path)
                .or_else(|_| fs::symlink_metadata(&path))
                .map_err(fl2_io)?;
            for value in fl2_attributes(&metadata, shown) {
                slots.push(Arc::new(value.array(&self.cancel)?));
            }
        }
        let info = Value::Container(Array::Struct(StructArray::from_slots(
            1,
            count,
            FILEATTRIB_FIELDS.iter().map(|s| (*s).to_owned()).collect(),
            slots,
            &self.cancel,
        )?));
        if nargout == 0 {
            if let Some(text) =
                crate::display_format::disp(&info.array(&self.cancel)?, &self.display)?
            {
                if !self.display.compact {
                    self.emit(out, String::new())?;
                }
                self.emit(out, text)?;
            }
            return Ok(Vec::new());
        }
        Ok(vec![fl2_bool(true), info, fl2_str("")])
    }
}

/// tIME chunk body for a Unix time in UTC.
fn fl2_utc_time(seconds: u64) -> [u8; 7] {
    let days = (seconds / 86400) as i64;
    let rest = seconds % 86400;
    // Civil-from-days (proleptic Gregorian).
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1460 + doe / 36524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let day = (doy - (153 * mp + 2) / 5 + 1) as u8;
    let month = if mp < 10 { mp + 3 } else { mp - 9 } as u8;
    let year = (yoe + era * 400 + i64::from(month <= 2)) as u16;
    let y = year.to_be_bytes();
    [
        y[0],
        y[1],
        month,
        day,
        (rest / 3600) as u8,
        (rest % 3600 / 60) as u8,
        (rest % 60) as u8,
    ]
}

/// (attribute letter, set) pairs; None when a letter is invalid here.
fn fl2_parse_mode(mode: &str, users: &str) -> Option<Vec<(char, bool, String)>> {
    let valid: &[char] = if cfg!(windows) {
        &['a', 'h', 's', 'w']
    } else {
        &['r', 'w', 'x']
    };
    let users = if users.is_empty() {
        "u".to_owned()
    } else {
        users.to_owned()
    };
    if cfg!(unix) && !users.chars().all(|c| "ugoa".contains(c)) {
        return None;
    }
    let mut changes = Vec::new();
    for token in mode.split_whitespace() {
        let mut chars = token.chars();
        let set = match chars.next()? {
            '+' => true,
            '-' => false,
            _ => return None,
        };
        let letters: Vec<char> = chars.collect();
        if letters.is_empty() {
            return None;
        }
        for letter in letters {
            if !valid.contains(&letter) {
                return None;
            }
            changes.push((letter, set, users.clone()));
        }
    }
    (!changes.is_empty()).then_some(changes)
}

#[cfg(windows)]
mod fl2_windows {
    #[link(name = "kernel32")]
    unsafe extern "system" {
        fn GetFileAttributesW(name: *const u16) -> u32;
        fn SetFileAttributesW(name: *const u16, attributes: u32) -> i32;
    }
    fn wide(path: &std::path::Path) -> Vec<u16> {
        use std::os::windows::ffi::OsStrExt;
        path.as_os_str()
            .encode_wide()
            .chain(std::iter::once(0))
            .collect()
    }
    pub(super) fn update(path: &std::path::Path, flag: u32, set: bool) -> Result<(), String> {
        let name = wide(path);
        // SAFETY: `name` is a NUL-terminated UTF-16 path that outlives both calls;
        // neither function retains the pointer.
        let current = unsafe { GetFileAttributesW(name.as_ptr()) };
        if current == u32::MAX {
            return Err("No such file or directory.".into());
        }
        let mut next = if set { current | flag } else { current & !flag };
        if next == 0 {
            next = 0x80;
        }
        // SAFETY: as above.
        if unsafe { SetFileAttributesW(name.as_ptr(), next) } == 0 {
            return Err("Unable to change the file attributes.".into());
        }
        Ok(())
    }
}

fn fl2_apply_mode(
    path: &Path,
    changes: &[(char, bool, String)],
) -> std::result::Result<(), String> {
    for (letter, set, users) in changes {
        #[cfg(windows)]
        {
            let _ = users;
            let flag = match letter {
                'a' => 0x20,
                'h' => 0x2,
                's' => 0x4,
                _ => {
                    let metadata = fs::metadata(path).map_err(|e| e.to_string())?;
                    let mut permissions = metadata.permissions();
                    permissions.set_readonly(!set);
                    fs::set_permissions(path, permissions).map_err(|e| e.to_string())?;
                    continue;
                }
            };
            fl2_windows::update(path, flag, *set)?;
        }
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let metadata = fs::metadata(path).map_err(|e| e.to_string())?;
            let mut mode = metadata.permissions().mode();
            let bit = match letter {
                'r' => 4,
                'w' => 2,
                _ => 1,
            };
            for user in users.chars() {
                let shifts: &[u32] = match user {
                    'u' => &[6],
                    'g' => &[3],
                    'o' => &[0],
                    _ => &[6, 3, 0],
                };
                for shift in shifts {
                    if *set {
                        mode |= bit << shift;
                    } else {
                        mode &= !(bit << shift);
                    }
                }
            }
            fs::set_permissions(path, fs::Permissions::from_mode(mode))
                .map_err(|e| e.to_string())?;
        }
        #[cfg(not(any(windows, unix)))]
        {
            let _ = (path, letter, set, users);
            return Err("File attributes are not supported on this platform.".into());
        }
    }
    Ok(())
}

/// fileattrib values in FILEATTRIB_FIELDS order. Windows has archive, system
/// and hidden flags but no group/other permissions; UNIX the reverse (NaN).
fn fl2_attributes(metadata: &fs::Metadata, name: String) -> Vec<Value> {
    let directory = metadata.is_dir();
    #[cfg(windows)]
    {
        use std::os::windows::fs::MetadataExt;
        let flags = metadata.file_attributes();
        let mut values = vec![
            fl2_str(name),
            fl2_bool(flags & 0x20 != 0),
            fl2_bool(flags & 0x4 != 0),
            fl2_bool(flags & 0x2 != 0),
            fl2_bool(directory),
            fl2_bool(true),
            fl2_bool(!metadata.permissions().readonly()),
            fl2_bool(true),
        ];
        values.extend((0..6).map(|_| fl2_num(f64::NAN)));
        values
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mode = metadata.permissions().mode();
        let mut values = vec![
            fl2_str(name),
            fl2_num(f64::NAN),
            fl2_num(f64::NAN),
            fl2_num(f64::NAN),
            fl2_bool(directory),
        ];
        for shift in [6, 3, 0] {
            for bit in [4, 2, 1] {
                values.push(fl2_bool(mode & (bit << shift) != 0));
            }
        }
        values
    }
    #[cfg(not(any(windows, unix)))]
    {
        let mut values = vec![fl2_str(name)];
        values.extend((0..3).map(|_| fl2_num(f64::NAN)));
        values.push(fl2_bool(directory));
        values.extend((0..9).map(|_| fl2_num(f64::NAN)));
        values
    }
}
