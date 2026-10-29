//! File I/O: fopen/fclose and the line, binary and formatted readers/writers,
//! textscan, dlmread/dlmwrite, csvread/csvwrite, type and exist.
//!
//! Open files live in a per-session table (fids 3, 4, ...; at most 64). Each
//! open file keeps its bytes in memory (at most 8 MiB) and writes through to
//! disk at once, so a file can be read back before `fclose`. Dropping the engine
//! closes every file.
use super::{
    Engine, Execution, Value,
    text_format::{self as tf, Item, MAX_TEXT},
};
use crate::{
    Array, CellArray, CharacterRow, LogicalMatrix, Matrix, Result,
    script_containers::MAX_CHARACTER_UNITS,
};
use std::{
    borrow::Cow,
    fs::{File, OpenOptions},
    io::{Read, Seek, SeekFrom, Write},
    path::{Path, PathBuf},
    sync::{Arc, Mutex, PoisonError},
};

const NAMES: &[&str] = &[
    "fopen", "fclose", "fgetl", "fgets", "feof", "ftell", "fseek", "frewind", "fwrite", "fread",
    "fscanf", "textscan", "dlmread", "dlmwrite", "csvread", "csvwrite", "type", "exist",
];

pub(super) fn handles(name: &str) -> bool {
    NAMES.contains(&name)
}

const MAX_OPEN: usize = 64;
const FIRST_FID: usize = 3;
const MAX_FILE_BYTES: usize = 8 * 1024 * 1024;
const BAD_FID: &str = "Invalid file identifier.  Use fopen to generate a valid file identifier.";
const TOO_FEW: &str = "Not enough input arguments.";
const TOO_MANY: &str = "Too many input arguments.";
const CHECK_EVERY: usize = 4096;

struct OpenFile {
    file: File,
    name: String,
    perm: String,
    readable: bool,
    writable: bool,
    append: bool,
    /// Windows MATLAB text translation, on every platform. Positions stay raw.
    text: bool,
    /// Machine format from fopen: big endian for later fread/fwrite.
    big: bool,
    /// Encoding from fopen: ISO-8859-1 (one byte per character) instead of UTF-8.
    latin1: bool,
    data: Vec<u8>,
    pos: usize,
    eof: bool,
}

impl OpenFile {
    fn write(&mut self, bytes: &[u8]) -> Result<usize> {
        if !self.writable {
            return Ok(0);
        }
        let at = if self.append {
            self.data.len()
        } else {
            self.pos
        };
        let newlines = if self.text {
            bytes.iter().filter(|&&b| b == b'\n').count()
        } else {
            0
        };
        let end = at.saturating_add(bytes.len()).saturating_add(newlines);
        if end > MAX_FILE_BYTES {
            return Err("An open file is limited to 8 MiB".into());
        }
        let written = bytes.len();
        let bytes = if newlines == 0 {
            Cow::Borrowed(bytes)
        } else {
            let mut translated = Vec::with_capacity(bytes.len() + newlines);
            for &b in bytes {
                if b == b'\n' {
                    translated.push(b'\r');
                }
                translated.push(b);
            }
            Cow::Owned(translated)
        };
        if end > self.data.len() {
            self.data.resize(end, 0);
        }
        self.data[at..end].copy_from_slice(&bytes);
        self.file
            .seek(SeekFrom::Start(at as u64))
            .and_then(|_| self.file.write_all(&bytes))
            .map_err(|e| e.to_string())?;
        self.pos = end;
        Ok(written)
    }

    /// Read a logical byte while retaining the physical file position.
    fn read_byte(&mut self) -> Option<u8> {
        let mut b = *self.data.get(self.pos)?;
        self.pos += 1;
        if self.text && b == b'\r' && self.data.get(self.pos) == Some(&b'\n') {
            b = b'\n';
            self.pos += 1;
        }
        Some(b)
    }

    /// Scanners need contiguous input; binary mode borrows the original bytes.
    fn scan_data(&self) -> Cow<'_, [u8]> {
        let rest = &self.data[self.pos..];
        if !self.text || !rest.windows(2).any(|w| w == b"\r\n") {
            return Cow::Borrowed(rest);
        }
        Cow::Owned(
            rest.iter()
                .enumerate()
                .filter(|&(i, &b)| b != b'\r' || rest.get(i + 1) != Some(&b'\n'))
                .map(|(_, &b)| b)
                .collect(),
        )
    }

    fn advance_scanned(&mut self, consumed: usize) {
        if self.text {
            for _ in 0..consumed {
                self.read_byte();
            }
        } else {
            self.pos += consumed;
        }
    }

    /// One line, with its terminator (LF, CR LF or a lone CR, as the reference's
    /// fgets reads them). `None` at end of file.
    fn line(&mut self, limit: Option<usize>) -> Option<Vec<u8>> {
        if !self.readable || self.pos >= self.data.len() {
            self.eof = true;
            return None;
        }
        let limit = limit.unwrap_or(usize::MAX);
        let mut line = Vec::new();
        while line.len() < limit {
            let Some(b) = self.read_byte() else { break };
            line.push(b);
            if b == b'\n' {
                break;
            }
            if b == b'\r' {
                // In text mode CR LF already reads as LF, so this CR is lone.
                if line.len() < limit && self.data.get(self.pos) == Some(&b'\n') {
                    self.pos += 1;
                    line.push(b'\n');
                }
                break;
            }
        }
        if self.pos >= self.data.len() {
            self.eof = true;
        }
        Some(line)
    }
}

/// The per-session table; clones share it, like the operating system's files.
#[derive(Clone, Default)]
pub(super) struct FileTable(Arc<Mutex<Vec<Option<OpenFile>>>>);

impl FileTable {
    fn lock(&self) -> std::sync::MutexGuard<'_, Vec<Option<OpenFile>>> {
        self.0.lock().unwrap_or_else(PoisonError::into_inner)
    }
}

fn io_message(e: &std::io::Error) -> String {
    match e.kind() {
        std::io::ErrorKind::NotFound => "No such file or directory".into(),
        std::io::ErrorKind::PermissionDenied => "Permission denied".into(),
        std::io::ErrorKind::AlreadyExists => "File exists".into(),
        _ => e.to_string(),
    }
}

/// Bytes to text: UTF-8 when valid, otherwise one character per byte.
fn bytes_text(bytes: &[u8]) -> String {
    match std::str::from_utf8(bytes) {
        Ok(s) => s.to_owned(),
        Err(_) => bytes.iter().map(|&b| char::from(b)).collect(),
    }
}

/// Bytes of a file opened with an encoding to text.
fn decode(bytes: &[u8], latin1: bool) -> String {
    if latin1 {
        bytes.iter().map(|&b| char::from(b)).collect()
    } else {
        bytes_text(bytes)
    }
}

/// A regular file's bytes, at most `MAX_FILE_BYTES`. Devices, FIFOs and other
/// special files are refused before opening (they can block or never end).
fn read_regular(path: &Path) -> std::result::Result<Vec<u8>, String> {
    let not_regular = || "Not a regular file".to_owned();
    let meta = std::fs::metadata(path).map_err(|e| io_message(&e))?;
    if !meta.is_file() {
        return Err(not_regular());
    }
    let file = File::open(path).map_err(|e| io_message(&e))?;
    if !file.metadata().is_ok_and(|m| m.is_file()) {
        return Err(not_regular());
    }
    let mut data = Vec::new();
    file.take(MAX_FILE_BYTES as u64 + 1)
        .read_to_end(&mut data)
        .map_err(|e| io_message(&e))?;
    if data.len() > MAX_FILE_BYTES {
        return Err("File exceeds the 8 MiB limit".into());
    }
    Ok(data)
}

fn is_ws(b: u8) -> bool {
    matches!(b, b' ' | b'\t' | b'\n' | b'\r' | 0x0b | 0x0c)
}

#[derive(Clone, Copy, PartialEq)]
enum Kind {
    U8,
    I8,
    U16,
    I16,
    U32,
    I32,
    U64,
    I64,
    F32,
    F64,
}

impl Kind {
    fn size(self) -> usize {
        match self {
            Kind::U8 | Kind::I8 => 1,
            Kind::U16 | Kind::I16 => 2,
            Kind::U32 | Kind::I32 | Kind::F32 => 4,
            Kind::U64 | Kind::I64 | Kind::F64 => 8,
        }
    }

    fn decode(self, b: &[u8], big: bool) -> f64 {
        macro_rules! num {
            ($t:ty) => {{
                let mut a = [0u8; std::mem::size_of::<$t>()];
                a.copy_from_slice(b);
                if big {
                    <$t>::from_be_bytes(a)
                } else {
                    <$t>::from_le_bytes(a)
                }
            }};
        }
        match self {
            Kind::U8 => f64::from(b[0]),
            Kind::I8 => f64::from(b[0] as i8),
            Kind::U16 => f64::from(num!(u16)),
            Kind::I16 => f64::from(num!(i16)),
            Kind::U32 => f64::from(num!(u32)),
            Kind::I32 => f64::from(num!(i32)),
            Kind::U64 => num!(u64) as f64,
            Kind::I64 => num!(i64) as f64,
            Kind::F32 => f64::from(num!(f32)),
            Kind::F64 => num!(f64),
        }
    }

    /// Integers round to nearest and saturate; NaN becomes 0.
    fn encode(self, v: f64, big: bool, out: &mut Vec<u8>) {
        macro_rules! put {
            ($x:expr) => {{
                let x = $x;
                out.extend_from_slice(&if big {
                    x.to_be_bytes()
                } else {
                    x.to_le_bytes()
                });
            }};
        }
        let r = v.round();
        match self {
            Kind::U8 => out.push(r as u8),
            Kind::I8 => out.push(r as i8 as u8),
            Kind::U16 => put!(r as u16),
            Kind::I16 => put!(r as i16),
            Kind::U32 => put!(r as u32),
            Kind::I32 => put!(r as i32),
            Kind::U64 => put!(r as u64),
            Kind::I64 => put!(r as i64),
            Kind::F32 => put!(v as f32),
            Kind::F64 => put!(v),
        }
    }
}

/// `(source, output is char, big endian)` from a precision and architecture.
fn precision(who: &str, text: &str) -> Result<(Kind, bool)> {
    let clean: String = text.chars().filter(|c| !c.is_whitespace()).collect();
    let (source, dest) = if let Some(star) = clean.strip_prefix('*') {
        (star.to_owned(), Some(star.to_owned()))
    } else if let Some((s, d)) = clean.split_once("=>") {
        (s.to_owned(), Some(d.to_owned()))
    } else {
        (clean, None)
    };
    let kind = match source.as_str() {
        "uint8" | "uchar" | "unsignedchar" | "char" => Kind::U8,
        "int8" | "schar" | "signedchar" | "integer*1" => Kind::I8,
        "int16" | "short" | "integer*2" => Kind::I16,
        "uint16" | "ushort" | "unsignedshort" => Kind::U16,
        "int32" | "int" | "integer*4" => Kind::I32,
        "uint32" | "uint" | "unsignedint" => Kind::U32,
        "int64" | "integer*8" => Kind::I64,
        "uint64" => Kind::U64,
        "single" | "float32" | "float" | "real*4" => Kind::F32,
        "double" | "float64" | "real*8" => Kind::F64,
        _ => return Err(format!("{who}: unsupported precision '{text}'")),
    };
    Ok((kind, dest.as_deref() == Some("char")))
}

fn big_endian(text: &str) -> Result<bool> {
    match text.to_ascii_lowercase().as_str() {
        "n" | "native" | "l" | "ieee-le" | "a" | "ieee-le.l64" => Ok(false),
        "b" | "ieee-be" | "s" | "ieee-be.l64" => Ok(true),
        // The reference dropped the VAX and Cray formats too.
        _ => Err("Invalid machine format.".into()),
    }
}

fn text_arg<'a>(who: &str, what: &str, v: Option<&'a Value>) -> Result<&'a str> {
    match v {
        Some(Value::Text(s)) => Ok(s),
        _ => Err(format!("{who}: {what} must be a character row")),
    }
}

fn scalar_arg(who: &str, what: &str, v: &Value) -> Result<f64> {
    match v {
        Value::Matrix(m) if m.len() == 1 => Ok(m.data()[0]),
        Value::Logical(m) if m.len() == 1 => Ok(f64::from(m.data()[0])),
        _ => Err(format!("{who}: {what} must be a numeric scalar")),
    }
}

fn number(v: f64) -> Value {
    Value::Matrix(Matrix::scalar(v))
}

fn offset_arg(who: &str, what: &str, v: &Value) -> Result<usize> {
    let x = scalar_arg(who, what, v)?;
    if !x.is_finite() || x < 0. || x.fract() != 0. || x > 1e9 {
        return Err(format!("{who}: {what} must be a non-negative integer"));
    }
    Ok(x as usize)
}

/// `size` argument of fread/fscanf: `(max elements, rows, columns fixed?)`.
#[derive(Clone, Copy)]
pub(super) struct SizeSpec {
    pub limit: usize,
    pub rows: Option<usize>,
}

pub(super) fn size_spec(who: &str, v: Option<&Value>) -> Result<SizeSpec> {
    let unlimited = SizeSpec {
        limit: usize::MAX,
        rows: None,
    };
    let Some(v) = v else {
        return Ok(unlimited);
    };
    let Value::Matrix(m) = v else {
        return Err(format!("{who}: size must be numeric"));
    };
    let bad = || format!("{who}: size must be Inf, N or [M N]");
    let whole = |x: f64| x >= 0. && x.fract() == 0. && x <= (1u64 << 40) as f64;
    match m.data() {
        [n] if *n == f64::INFINITY => Ok(unlimited),
        [n] if whole(*n) => Ok(SizeSpec {
            limit: *n as usize,
            rows: None,
        }),
        [r, c] if whole(*r) && *r >= 1. && (whole(*c) || *c == f64::INFINITY) => {
            let limit = if *c == f64::INFINITY {
                usize::MAX
            } else {
                (*r as usize).saturating_mul(*c as usize)
            };
            Ok(SizeSpec {
                limit,
                rows: Some(*r as usize),
            })
        }
        _ => Err(bad()),
    }
}

/// `(rows, cols)` and padded values for `take` elements under a size spec.
pub(super) fn shape_values(mut values: Vec<f64>, spec: SizeSpec) -> (usize, usize, Vec<f64>) {
    let take = values.len();
    if take == 0 {
        return (0, 0, values);
    }
    match spec.rows {
        None => (take, 1, values),
        Some(r) if take < r => (take, 1, values),
        Some(r) => {
            let cols = take.div_ceil(r);
            values.resize(r * cols, 0.);
            (r, cols, values)
        }
    }
}

impl Engine {
    /// Resolve a file name against the current folder; no network paths.
    fn fileio_path(&self, who: &str, name: &str) -> Result<PathBuf> {
        if name.is_empty() || name.len() > 32768 || name.contains('\0') {
            return Err(format!(
                "{who}: file name is empty, oversized or contains NUL"
            ));
        }
        if name.contains("://") || name.starts_with("\\\\") || name.starts_with("//") {
            return Err(format!("{who}: network paths and URLs are not supported"));
        }
        let path = PathBuf::from(name);
        if path.is_relative() {
            let base = self
                .file_directory
                .as_ref()
                .ok_or_else(|| format!("{who}: no current folder is available"))?;
            return Ok(base.join(path));
        }
        Ok(path)
    }

    fn fileio_slot(value: &Value) -> Result<usize> {
        let Value::Matrix(m) = value else {
            return Err(BAD_FID.into());
        };
        match m.data() {
            [v] if v.fract() == 0.
                && *v >= FIRST_FID as f64
                && *v < (FIRST_FID + MAX_OPEN) as f64 =>
            {
                Ok(*v as usize - FIRST_FID)
            }
            _ => Err(BAD_FID.into()),
        }
    }

    fn fileio_with<R>(&self, fid: &Value, f: impl FnOnce(&mut OpenFile) -> Result<R>) -> Result<R> {
        let slot = Self::fileio_slot(fid)?;
        let mut table = self.fileio.lock();
        match table.get_mut(slot) {
            Some(Some(file)) => f(file),
            _ => Err(BAD_FID.into()),
        }
    }

    /// Text for `fprintf(fid, ...)` to a file opened with `fopen`.
    pub(super) fn fileio_write_text(&self, fid: f64, text: &str) -> Result<usize> {
        self.fileio_with(&number(fid), |f| {
            if !f.latin1 {
                return f.write(text.as_bytes());
            }
            let bytes = text
                .chars()
                .map(|c| {
                    u8::try_from(c)
                        .map_err(|_| format!("fprintf: character '{c}' is not in ISO-8859-1"))
                })
                .collect::<Result<Vec<u8>>>()?;
            f.write(&bytes)
        })
    }

    /// Close every open file (end of session or `fclose('all')`).
    pub(super) fn fileio_close_all(&self) {
        self.fileio.lock().clear();
    }

    pub(super) fn fileio_call(
        &mut self,
        name: &str,
        args: &[Value],
        out: &mut Execution,
    ) -> Result<Option<Value>> {
        if !handles(name) {
            return Ok(None);
        }
        let mut outputs = self.fileio_run(name, args, 1, out)?;
        Ok(Some(if outputs.is_empty() {
            Value::Void
        } else {
            outputs.swap_remove(0)
        }))
    }

    pub(super) fn fileio_outputs(
        &mut self,
        name: &str,
        args: &[Value],
        count: usize,
        out: &mut Execution,
    ) -> Result<Vec<Value>> {
        let mut outputs = self.fileio_run(name, args, count, out)?;
        if count > outputs.len() {
            return Err("Too many output arguments.".into());
        }
        outputs.truncate(count);
        Ok(outputs)
    }

    fn fileio_run(
        &mut self,
        name: &str,
        args: &[Value],
        count: usize,
        out: &mut Execution,
    ) -> Result<Vec<Value>> {
        self.check_cancel()?;
        let args = self.text_arguments(args.to_vec())?;
        let args = args.as_slice();
        if args.is_empty() {
            return Err(TOO_FEW.into());
        }
        let one = |v: Value| Ok(vec![v]);
        match name {
            "fopen" => self.fileio_fopen(args, count),
            "fclose" => {
                if args.len() > 1 {
                    return Err(TOO_MANY.into());
                }
                if matches!(&args[0], Value::Text(t) if t == "all") {
                    self.fileio_close_all();
                    return one(number(0.));
                }
                if let Value::Matrix(m) = &args[0]
                    && matches!(m.data(), [v] if (0. ..3.).contains(v) && v.fract() == 0.)
                {
                    return one(number(0.));
                }
                let slot = Self::fileio_slot(&args[0])?;
                let mut table = self.fileio.lock();
                match table.get_mut(slot) {
                    Some(entry @ Some(_)) => *entry = None,
                    _ => return Err(BAD_FID.into()),
                }
                one(number(0.))
            }
            "fgetl" | "fgets" => self.fileio_line(name, args),
            "feof" => {
                if args.len() > 1 {
                    return Err(TOO_MANY.into());
                }
                let eof = self.fileio_with(&args[0], |f| Ok(f.eof))?;
                one(Value::Logical(LogicalMatrix::scalar(eof)))
            }
            "ftell" => {
                if args.len() > 1 {
                    return Err(TOO_MANY.into());
                }
                one(number(self.fileio_with(&args[0], |f| Ok(f.pos as f64))?))
            }
            "frewind" => {
                if args.len() > 1 {
                    return Err(TOO_MANY.into());
                }
                self.fileio_with(&args[0], |f| {
                    f.pos = 0;
                    f.eof = false;
                    Ok(())
                })?;
                Ok(Vec::new())
            }
            "fseek" => self.fileio_fseek(args),
            "fwrite" => self.fileio_fwrite(args),
            "fread" => self.fileio_fread(args),
            "fscanf" => self.fileio_fscanf(args),
            "textscan" => self.fileio_textscan(args),
            "dlmread" | "csvread" => self.fileio_dlmread(name, args),
            "dlmwrite" | "csvwrite" => self.fileio_dlmwrite(name, args).map(|()| Vec::new()),
            "type" => {
                if args.len() > 1 {
                    return Err(TOO_MANY.into());
                }
                let file = text_arg("type", "file name", args.first())?;
                let text = self.fileio_slurp("type", file)?;
                // The reference prints a blank line before the file's text.
                let text = format!("\n{}", bytes_text(&text).replace("\r\n", "\n"));
                self.print_text(out, &text)?;
                Ok(Vec::new())
            }
            "exist" => self.fileio_exist(args),
            _ => unreachable!("names checked above"),
        }
    }

    /// Whole file, bounded by the open-file size limit; `.m` is tried when the
    /// name has no extension and does not exist (type).
    fn fileio_slurp(&self, who: &str, name: &str) -> Result<Vec<u8>> {
        let mut path = self.fileio_path(who, name)?;
        if who == "type" && !path.exists() && path.extension().is_none() {
            let mut with_m = path.clone().into_os_string();
            with_m.push(".m");
            path = PathBuf::from(with_m);
        }
        let unopened = |why: &str| {
            if who == "type" {
                format!("{who}: unable to open file '{name}'")
            } else {
                format!("The file '{name}' could not be opened because: {why}")
            }
        };
        let meta = std::fs::metadata(&path).map_err(|e| unopened(&io_message(&e)))?;
        if !meta.is_file() {
            return Err(unopened("Not a regular file"));
        }
        if meta.len() > MAX_FILE_BYTES as u64 {
            return Err(format!("{who}: file exceeds the 8 MiB limit"));
        }
        read_regular(&path).map_err(|e| format!("{who}: {e}"))
    }

    fn fileio_fopen(&mut self, args: &[Value], count: usize) -> Result<Vec<Value>> {
        let who = "fopen";
        if args.len() > 4 {
            return Err(TOO_MANY.into());
        }
        let msg = |s: &str| Value::Text(s.to_owned());
        match &args[0] {
            Value::Text(t) if t == "all" && args.len() == 1 => {
                let table = self.fileio.lock();
                let fids: Vec<f64> = table
                    .iter()
                    .enumerate()
                    .filter(|(_, f)| f.is_some())
                    .map(|(i, _)| (i + FIRST_FID) as f64)
                    .collect();
                let m = if fids.is_empty() {
                    Matrix::filled(0, 0, 0.)?
                } else {
                    Matrix::from_column_vec(1, fids.len(), fids)?
                };
                return Ok(vec![Value::Matrix(m)]);
            }
            Value::Matrix(m) if args.len() == 1 => {
                let fid = scalar_arg(who, "file id", &args[0])?;
                let _ = m;
                if (0. ..3.).contains(&fid) && fid.fract() == 0. {
                    let name = ["stdin", "stdout", "stderr"][fid as usize];
                    let perm = if fid == 0. { "r" } else { "w" };
                    return Ok(vec![msg(name), msg(perm), msg("native"), msg("UTF-8")]);
                }
                return self.fileio_with(&args[0], |f| {
                    Ok(vec![
                        msg(&f.name),
                        msg(&f.perm),
                        msg(if f.big { "ieee-be" } else { "native" }),
                        msg(if f.latin1 { "ISO-8859-1" } else { "UTF-8" }),
                    ])
                });
            }
            _ => {}
        }
        let file_name = text_arg(who, "file name", args.first())?;
        let perm = match args.get(1) {
            None => "r",
            Some(Value::Text(p)) => p.as_str(),
            Some(_) => return Err("fopen: permission must be a character row".into()),
        };
        let mode: String = perm.chars().filter(|&c| c != 'b' && c != 't').collect();
        if perm.len() != mode.len() + perm.matches(['b', 't']).count()
            || perm.matches(['b', 't']).count() > 1
            || !matches!(mode.as_str(), "r" | "w" | "a" | "r+" | "w+" | "a+")
        {
            return Err("Invalid permission.".into());
        }
        let big = match args.get(2) {
            None => false,
            Some(Value::Text(t)) => big_endian(t)?,
            Some(_) => return Err("fopen: machine format must be a character row".into()),
        };
        let latin1 = match args.get(3) {
            None => false,
            Some(Value::Text(e)) => match e.to_ascii_lowercase().as_str() {
                "utf-8" | "utf8" => false,
                "iso-8859-1" | "iso8859-1" | "latin1" => true,
                _ => {
                    return Err(format!(
                        "fopen: encoding '{e}' is not supported yet (only UTF-8 and ISO-8859-1)"
                    ));
                }
            },
            Some(_) => return Err("fopen: encoding must be a character row".into()),
        };
        let fail = |text: String| Ok(vec![number(-1.), Value::Text(text)]);
        let path = self.fileio_path(who, file_name)?;
        // Opening a FIFO blocks and a device never ends: only regular files.
        match std::fs::metadata(&path) {
            Ok(m) if m.is_dir() => return fail("Is a directory".into()),
            Ok(m) if !m.is_file() => return fail("Not a regular file".into()),
            _ => {}
        }
        let mut table = self.fileio.lock();
        let slot = match table.iter().position(Option::is_none) {
            Some(i) => i,
            None if table.len() < MAX_OPEN => {
                table.push(None);
                table.len() - 1
            }
            None => return fail("Too many open files".into()),
        };
        let mut options = OpenOptions::new();
        match mode.as_str() {
            "r" => options.read(true),
            "w" => options.write(true).create(true).truncate(true),
            "a" => options.write(true).create(true),
            "r+" => options.read(true).write(true),
            "w+" => options.read(true).write(true).create(true).truncate(true),
            _ => options.read(true).write(true).create(true),
        };
        let file = match options.open(&path) {
            Ok(f) => f,
            Err(e) => return fail(io_message(&e)),
        };
        let meta = file.metadata().map_err(|e| io_message(&e))?;
        if meta.is_dir() {
            return fail("Is a directory".into());
        }
        if !meta.is_file() {
            return fail("Not a regular file".into());
        }
        if meta.len() > MAX_FILE_BYTES as u64 {
            return fail("File exceeds the 8 MiB limit".into());
        }
        let mut data = Vec::new();
        if mode != "w" && mode != "w+" {
            data = match read_regular(&path) {
                Ok(data) => data,
                Err(text) => return fail(text),
            };
        }
        let append = mode.starts_with('a');
        let pos = if mode == "a" { data.len() } else { 0 };
        table[slot] = Some(OpenFile {
            file,
            name: path.to_string_lossy().into_owned(),
            perm: perm.to_owned(),
            readable: mode == "r" || mode.ends_with('+'),
            writable: mode != "r",
            append,
            text: perm.contains('t'),
            big,
            latin1,
            data,
            pos,
            eof: false,
        });
        let fid = (slot + FIRST_FID) as f64;
        drop(table);
        let _ = count;
        Ok(vec![number(fid), Value::Text(String::new())])
    }

    fn fileio_line(&mut self, name: &str, args: &[Value]) -> Result<Vec<Value>> {
        if args.len() > 2 {
            return Err(TOO_MANY.into());
        }
        let limit = match args.get(1) {
            None => None,
            Some(v) => {
                let n = scalar_arg(name, "nchar", v)?;
                if n < 0. || n.fract() != 0. {
                    return Err(format!("{name}: nchar must be a non-negative integer"));
                }
                Some(n as usize)
            }
        };
        // The reference reads '' (not -1) from a file opened only for writing.
        let (line, latin1) = self.fileio_with(&args[0], |f| {
            Ok((f.readable.then(|| f.line(limit)), f.latin1))
        })?;
        let Some(line) = line else {
            return Ok(vec![Value::Text(String::new()), Value::Text(String::new())]);
        };
        let Some(mut line) = line else {
            return Ok(vec![number(-1.), Value::Text(String::new())]);
        };
        let ends = match line.as_slice() {
            [.., b'\r', b'\n'] => 2,
            [.., b'\n' | b'\r'] => 1,
            _ => 0,
        };
        let terminator = line.split_off(line.len() - ends);
        if name == "fgets" {
            line.extend_from_slice(&terminator);
        }
        let text = decode(&line, latin1);
        if text.chars().count() > MAX_CHARACTER_UNITS {
            return Err(format!(
                "{name}: line exceeds {MAX_CHARACTER_UNITS} characters"
            ));
        }
        Ok(vec![
            Value::Text(text),
            Value::Text(bytes_text(&terminator)),
        ])
    }

    fn fileio_fseek(&mut self, args: &[Value]) -> Result<Vec<Value>> {
        if args.len() < 3 {
            return Err(TOO_FEW.into());
        }
        if args.len() > 3 {
            return Err(TOO_MANY.into());
        }
        let offset = scalar_arg("fseek", "offset", &args[1])?;
        let origin = match &args[2] {
            Value::Text(t) => match t.as_str() {
                "bof" => -1.,
                "cof" => 0.,
                "eof" => 1.,
                _ => return Err("fseek: origin must be 'bof', 'cof', 'eof', -1, 0 or 1".into()),
            },
            other => scalar_arg("fseek", "origin", other)?,
        };
        if !offset.is_finite() || offset.fract() != 0. {
            return Err("fseek: offset must be an integer".into());
        }
        let status = self.fileio_with(&args[0], |f| {
            let base = match origin {
                -1. => 0.,
                0. => f.pos as f64,
                1. => f.data.len() as f64,
                _ => return Err("fseek: origin must be 'bof', 'cof', 'eof', -1, 0 or 1".into()),
            };
            // The reference clears the end-of-file flag even when the seek fails.
            f.eof = false;
            let target = base + offset;
            if target < 0. || target > f.data.len() as f64 {
                return Ok(-1.);
            }
            f.pos = target as usize;
            Ok(0.)
        })?;
        Ok(vec![number(status)])
    }

    fn fileio_fwrite(&mut self, args: &[Value]) -> Result<Vec<Value>> {
        if args.len() < 2 {
            return Err(TOO_FEW.into());
        }
        if args.len() > 5 {
            return Err(TOO_MANY.into());
        }
        let (kind, _) = match args.get(2) {
            None => (Kind::U8, false),
            Some(Value::Text(p)) => precision("fwrite", p)?,
            Some(_) => return Err("fwrite: precision must be a character row".into()),
        };
        let big = match Self::fileio_arch("fwrite", &args[3.min(args.len())..])? {
            Some(big) => big,
            None => self.fileio_with(&args[0], |f| Ok(f.big))?,
        };
        let mut bytes = Vec::new();
        let put = |bytes: &mut Vec<u8>, v: f64| kind.encode(v, big, bytes);
        match &args[1] {
            Value::Matrix(m) => {
                for (i, &v) in m.data().iter().enumerate() {
                    if i % CHECK_EVERY == 0 {
                        self.check_cancel()?;
                    }
                    put(&mut bytes, v);
                }
            }
            Value::Logical(m) => {
                for &v in m.data() {
                    put(&mut bytes, f64::from(v));
                }
            }
            Value::Text(s) => {
                for c in s.chars() {
                    put(&mut bytes, f64::from(u32::from(c)));
                }
            }
            Value::Container(Array::Character(c)) => {
                for &u in c.units() {
                    put(&mut bytes, f64::from(u));
                }
            }
            _ => return Err("fwrite: data must be numeric, logical or character".into()),
        }
        if bytes.len() > MAX_FILE_BYTES {
            return Err("fwrite: data exceeds the 8 MiB limit".into());
        }
        let written = self.fileio_with(&args[0], |f| f.write(&bytes))?;
        Ok(vec![number((written / kind.size()) as f64)])
    }

    /// Optional `skip, arch` tail of fread/fwrite; `None` keeps the file's own.
    fn fileio_arch(who: &str, rest: &[Value]) -> Result<Option<bool>> {
        let mut big = None;
        for (i, v) in rest.iter().enumerate() {
            match v {
                Value::Text(t) if i == rest.len() - 1 => big = Some(big_endian(t)?),
                Value::Matrix(m) if i == 0 && m.data() == [0.] => {}
                _ => return Err(format!("{who}: skip and unsupported arguments")),
            }
        }
        Ok(big)
    }

    fn fileio_fread(&mut self, args: &[Value]) -> Result<Vec<Value>> {
        if args.len() > 5 {
            return Err(TOO_MANY.into());
        }
        let spec = size_spec("fread", args.get(1))?;
        let (kind, as_char) = match args.get(2) {
            None => (Kind::U8, false),
            Some(Value::Text(p)) => precision("fread", p)?,
            Some(_) => return Err("fread: precision must be a character row".into()),
        };
        let arch = Self::fileio_arch("fread", &args[3.min(args.len())..])?;
        let width = kind.size();
        let (values, _) = self.fileio_with(&args[0], |f| {
            let big = arch.unwrap_or(f.big);
            let mut values = Vec::new();
            if !f.readable {
                return Ok((values, 0));
            }
            if f.text {
                let mut bytes = [0; 8];
                'values: while values.len() < spec.limit {
                    for b in &mut bytes[..width] {
                        let Some(next) = f.read_byte() else {
                            f.eof = true;
                            break 'values;
                        };
                        *b = next;
                    }
                    values.push(kind.decode(&bytes[..width], big));
                }
                let taken = values.len();
                return Ok((values, taken));
            }
            let available = (f.data.len() - f.pos) / width;
            let take = available.min(spec.limit);
            values.reserve(take);
            for (i, chunk) in f.data[f.pos..f.pos + take * width]
                .chunks_exact(width)
                .enumerate()
            {
                debug_assert!(i < take);
                values.push(kind.decode(chunk, big));
            }
            f.pos += take * width;
            if take < spec.limit {
                f.pos = f.data.len();
                f.eof = true;
            }
            Ok((values, take))
        })?;
        let taken = values.len();
        let (rows, cols, values) = shape_values(values, spec);
        self.charge(values.len())?;
        let result = if as_char {
            if values.is_empty() {
                Value::Text(String::new())
            } else {
                let units = values
                    .iter()
                    .map(|&v| v.clamp(0., 65535.) as u16)
                    .collect::<Vec<_>>();
                Value::Container(
                    CharacterRow::from_column_major(rows, cols, units, &self.cancel)?.into(),
                )
            }
        } else {
            Value::Matrix(Matrix::from_column_vec(rows, cols, values)?)
        };
        Ok(vec![result, number(taken as f64)])
    }

    fn fileio_exist(&mut self, args: &[Value]) -> Result<Vec<Value>> {
        if args.len() > 2 {
            return Err(TOO_MANY.into());
        }
        let name = text_arg("exist", "name", args.first())?;
        let kind = match args.get(1) {
            None => "",
            Some(Value::Text(k)) if matches!(k.as_str(), "var" | "file" | "dir" | "builtin") => k,
            Some(_) => {
                return Err("exist: type must be 'var', 'file', 'dir' or 'builtin'".into());
            }
        };
        let is_var = self.contains_array(name);
        let path_kind = || -> f64 {
            if name.is_empty() || name.contains('\0') || name.contains("://") {
                return 0.;
            }
            match self.fileio_path("exist", name).map(std::fs::metadata) {
                Ok(Ok(m)) if m.is_dir() => 7.,
                Ok(Ok(_)) => 2.,
                _ => 0.,
            }
        };
        let builtin = || crate::session::FUNCTIONS.contains(&name);
        let code = match kind {
            "var" => f64::from(u8::from(is_var)),
            "dir" => {
                if path_kind() == 7. {
                    7.
                } else {
                    0.
                }
            }
            "file" => {
                let k = path_kind();
                if k == 0. && builtin() { 2. } else { k }
            }
            "builtin" => {
                if builtin() {
                    5.
                } else {
                    0.
                }
            }
            _ => {
                if is_var {
                    1.
                } else if path_kind() != 0. {
                    path_kind()
                } else if builtin() {
                    5.
                } else {
                    0.
                }
            }
        };
        Ok(vec![number(code)])
    }
}

// ---- fscanf -----------------------------------------------------------------

pub(super) enum ScanItem {
    Space,
    Lit(u8),
    Conv {
        skip: bool,
        width: Option<usize>,
        kind: u8,
    },
}

pub(super) fn scan_format(who: &str, fmt: &str) -> Result<Vec<ScanItem>> {
    let text = tf::unescape(fmt);
    let b = text.as_bytes();
    let mut items = Vec::new();
    let mut i = 0;
    while i < b.len() {
        match b[i] {
            c if is_ws(c) => {
                while i < b.len() && is_ws(b[i]) {
                    i += 1;
                }
                items.push(ScanItem::Space);
            }
            b'%' if b.get(i + 1) == Some(&b'%') => {
                items.push(ScanItem::Lit(b'%'));
                i += 2;
            }
            b'%' => {
                i += 1;
                let skip = b.get(i) == Some(&b'*');
                i += usize::from(skip);
                let start = i;
                while i < b.len() && b[i].is_ascii_digit() {
                    i += 1;
                }
                let width = text[start..i].parse::<usize>().ok();
                let kind = *b
                    .get(i)
                    .ok_or_else(|| format!("{who}: incomplete conversion in format"))?;
                i += 1;
                if !matches!(
                    kind,
                    b'd' | b'i'
                        | b'u'
                        | b'f'
                        | b'e'
                        | b'g'
                        | b'E'
                        | b'G'
                        | b's'
                        | b'c'
                        | b'x'
                        | b'o'
                ) {
                    return Err(format!(
                        "{who}: unsupported conversion '%{}'",
                        char::from(kind)
                    ));
                }
                items.push(ScanItem::Conv { skip, width, kind });
            }
            c => {
                items.push(ScanItem::Lit(c));
                i += 1;
            }
        }
    }
    Ok(items)
}

/// Length of the number starting at `at`: integer or floating point text,
/// `Inf`/`NaN` included. 0 when there is none.
fn number_length(b: &[u8], at: usize, integer: bool, width: usize) -> usize {
    let end = b.len().min(at.saturating_add(width));
    let b = &b[..end];
    let mut i = at;
    if i < b.len() && (b[i] == b'+' || b[i] == b'-') {
        i += 1;
    }
    if !integer {
        let word = |w: &str| {
            b.len() >= i + w.len() && b[i..i + w.len()].eq_ignore_ascii_case(w.as_bytes())
        };
        if word("inf") {
            return i + 3 - at;
        }
        if word("nan") {
            return i + 3 - at;
        }
    }
    let digits = i;
    while i < b.len() && b[i].is_ascii_digit() {
        i += 1;
    }
    let mut seen = i - digits;
    if !integer {
        if i < b.len() && b[i] == b'.' {
            let frac = i + 1;
            i += 1;
            while i < b.len() && b[i].is_ascii_digit() {
                i += 1;
            }
            seen += i - frac;
        }
        if seen > 0 && i < b.len() && (b[i] == b'e' || b[i] == b'E') {
            let mut j = i + 1;
            if j < b.len() && (b[j] == b'+' || b[j] == b'-') {
                j += 1;
            }
            let exp = j;
            while j < b.len() && b[j].is_ascii_digit() {
                j += 1;
            }
            if j > exp {
                i = j;
            }
        }
    }
    if seen == 0 { 0 } else { i - at }
}

fn parse_number(text: &[u8]) -> f64 {
    let s = String::from_utf8_lossy(text);
    let s = s.trim_start_matches('+');
    s.parse::<f64>().unwrap_or(f64::NAN)
}

pub(super) struct Scanned {
    pub values: Vec<f64>,
    pub fields: usize,
    pub consumed: usize,
    pub all_text: bool,
}

pub(super) fn scan(
    data: &[u8],
    items: &[ScanItem],
    limit: usize,
    latin1: bool,
    cancel: &std::sync::atomic::AtomicBool,
) -> Result<Scanned> {
    let mut s = Scanned {
        values: Vec::new(),
        fields: 0,
        consumed: 0,
        all_text: true,
    };
    let conversions = items
        .iter()
        .filter(|i| matches!(i, ScanItem::Conv { .. }))
        .count();
    if conversions == 0 {
        return Ok(s);
    }
    s.all_text = items
        .iter()
        .all(|i| !matches!(i, ScanItem::Conv { kind, .. } if !matches!(kind, b's' | b'c')));
    // A leading %c reads white space too, so only the end of the data stops it.
    let raw_start = matches!(items.first(), Some(ScanItem::Conv { kind: b'c', .. }));
    let mut at = 0;
    let mut passes = 0usize;
    'outer: while s.values.len() < limit {
        if passes.is_multiple_of(256) {
            crate::dsp::check_cancel(cancel)?;
        }
        passes += 1;
        let mut look = at;
        while !raw_start && look < data.len() && is_ws(data[look]) {
            look += 1;
        }
        if look >= data.len() {
            at = data.len();
            break;
        }
        for item in items {
            if s.values.len() >= limit {
                break 'outer;
            }
            match item {
                ScanItem::Space => {
                    while at < data.len() && is_ws(data[at]) {
                        at += 1;
                    }
                }
                ScanItem::Lit(c) => {
                    if data.get(at) == Some(c) {
                        at += 1;
                    } else {
                        break 'outer;
                    }
                }
                ScanItem::Conv { skip, width, kind } => {
                    if *width == Some(0) {
                        // The reference reads nothing for a zero width ('%0d', '%*0s').
                        break 'outer;
                    }
                    let width = width.unwrap_or(usize::MAX);
                    if *kind != b'c' {
                        while at < data.len() && is_ws(data[at]) {
                            at += 1;
                        }
                    }
                    if at >= data.len() {
                        break 'outer;
                    }
                    let mut pushed: Vec<f64> = Vec::new();
                    match kind {
                        b'x' | b'o' => {
                            let radix = if *kind == b'x' { 16 } else { 8 };
                            let n = data[at..]
                                .iter()
                                .take(width)
                                .take_while(|&&b| char::from(b).is_digit(radix))
                                .count();
                            if n == 0 {
                                break 'outer;
                            }
                            pushed.push(
                                u64::from_str_radix(
                                    &String::from_utf8_lossy(&data[at..at + n]),
                                    radix,
                                )
                                .map_err(|_| "Scanned integer is too large")?
                                    as f64,
                            );
                            at += n;
                        }
                        b'd' | b'i' | b'u' | b'f' | b'e' | b'g' | b'E' | b'G' => {
                            let integer = matches!(kind, b'd' | b'i' | b'u');
                            let n = number_length(data, at, integer, width);
                            if n == 0 {
                                break 'outer;
                            }
                            pushed.push(parse_number(&data[at..at + n]));
                            at += n;
                        }
                        b's' => {
                            let end = data[at..]
                                .iter()
                                .take(width)
                                .position(|&b| is_ws(b))
                                .map_or(data.len().min(at.saturating_add(width)), |i| at + i);
                            pushed.extend(
                                decode(&data[at..end], latin1)
                                    .chars()
                                    .map(|c| f64::from(u32::from(c))),
                            );
                            at = end;
                        }
                        _ => {
                            let end = data.len().min(at.saturating_add(width.min(1 << 20)));
                            let w = if width == usize::MAX { 1 } else { width };
                            let end = end.min(at.saturating_add(w));
                            pushed.extend(
                                decode(&data[at..end], latin1)
                                    .chars()
                                    .map(|c| f64::from(u32::from(c))),
                            );
                            at = end;
                        }
                    }
                    if !*skip {
                        s.fields += 1;
                        s.values.extend(pushed);
                    }
                }
            }
        }
    }
    s.consumed = at;
    Ok(s)
}

impl Engine {
    fn fileio_fscanf(&mut self, args: &[Value]) -> Result<Vec<Value>> {
        if args.len() < 2 {
            return Err(TOO_FEW.into());
        }
        if args.len() > 3 {
            return Err(TOO_MANY.into());
        }
        let fmt = text_arg("fscanf", "format", args.get(1))?;
        let items = scan_format("fscanf", fmt)?;
        let spec = size_spec("fscanf", args.get(2))?;
        let cancel = self.cancel.clone();
        let scanned = self.fileio_with(&args[0], |f| {
            if !f.readable {
                return Ok(Scanned {
                    values: Vec::new(),
                    fields: 0,
                    consumed: 0,
                    all_text: false,
                });
            }
            let s = scan(&f.scan_data(), &items, spec.limit, f.latin1, &cancel)?;
            f.advance_scanned(s.consumed);
            if f.pos >= f.data.len() {
                f.eof = true;
            }
            Ok(s)
        })?;
        self.charge(scanned.values.len())?;
        let fields = scanned.fields;
        let result = if scanned.all_text && !scanned.values.is_empty() {
            let units: Vec<u16> = scanned
                .values
                .iter()
                .map(|&v| v.clamp(0., 65535.) as u16)
                .collect();
            let (rows, cols) = match spec.rows {
                Some(r) if units.len() >= r => (r, units.len().div_ceil(r)),
                Some(_) => (units.len(), 1),
                None => (1, units.len()),
            };
            let mut units = units;
            units.resize(rows * cols, 0);
            Value::Container(
                CharacterRow::from_column_major(rows, cols, units, &self.cancel)?.into(),
            )
        } else {
            let (rows, cols, values) = shape_values(scanned.values, spec);
            Value::Matrix(Matrix::from_column_vec(rows, cols, values)?)
        };
        // The reference's fscanf has two outputs; a third is "Too many output arguments."
        Ok(vec![result, number(fields as f64)])
    }
}

// ---- textscan ---------------------------------------------------------------

enum TextItem {
    Number { skip: bool },
    Str { skip: bool },
    Lit(String),
}

enum Column {
    Num(Vec<f64>),
    Str(Vec<String>),
}

struct TextOptions {
    delimiters: Vec<u8>,
    header_lines: usize,
    collect: bool,
    latin1: bool,
}

fn text_format(fmt: &str) -> Result<Vec<TextItem>> {
    let b = fmt.as_bytes();
    let mut items = Vec::new();
    let mut i = 0;
    while i < b.len() {
        if b[i] == b'%' {
            i += 1;
            let skip = b.get(i) == Some(&b'*');
            i += usize::from(skip);
            while i < b.len() && (b[i].is_ascii_digit() || b[i] == b'.') {
                i += 1;
            }
            let kind = *b
                .get(i)
                .ok_or("textscan: incomplete conversion in format")?;
            i += 1;
            // `%d8`-style and `%f32`-style size suffixes.
            while i < b.len() && b[i].is_ascii_digit() && matches!(kind, b'd' | b'u' | b'f') {
                i += 1;
            }
            match kind {
                b'f' | b'd' | b'u' | b'n' => items.push(TextItem::Number { skip }),
                b's' | b'q' => items.push(TextItem::Str { skip }),
                _ => {
                    return Err(format!(
                        "textscan: unsupported conversion '%{}'",
                        char::from(kind)
                    ));
                }
            }
        } else if is_ws(b[i]) {
            i += 1;
        } else {
            let start = i;
            while i < b.len() && b[i] != b'%' && !is_ws(b[i]) {
                i += 1;
            }
            items.push(TextItem::Lit(fmt[start..i].to_owned()));
        }
    }
    Ok(items)
}

/// Returns the columns of every converted field and the bytes consumed.
fn text_scan(
    data: &[u8],
    items: &[TextItem],
    options: &TextOptions,
    repeat: Option<usize>,
    cancel: &std::sync::atomic::AtomicBool,
) -> Result<(Vec<Column>, usize)> {
    let mut columns: Vec<Column> = items
        .iter()
        .filter_map(|i| match i {
            TextItem::Number { skip: false } => Some(Column::Num(Vec::new())),
            TextItem::Str { skip: false } => Some(Column::Str(Vec::new())),
            _ => None,
        })
        .collect();
    let mut at = 0;
    for _ in 0..options.header_lines {
        match data[at..].iter().position(|&b| b == b'\n') {
            Some(i) => at += i + 1,
            None => {
                at = data.len();
                break;
            }
        }
    }
    let delimited = !options.delimiters.is_empty();
    let blank = |b: u8| matches!(b, b' ' | b'\t' | 0x08) && !options.delimiters.contains(&b);
    let is_delim = |b: u8| options.delimiters.contains(&b);
    let mut rows = 0usize;
    'rows: loop {
        if repeat.is_some_and(|n| rows >= n) {
            break;
        }
        crate::dsp::check_cancel(cancel)?;
        // Skip blank lines between records; trailing whitespace ends the data.
        let mut look = at;
        while look < data.len() && (is_ws(data[look]) && !(delimited && is_delim(data[look]))) {
            look += 1;
        }
        if look >= data.len() {
            at = data.len();
            break;
        }
        let row_start = at;
        at = look;
        let mut column = 0;
        let mut after_delim = false;
        for item in items {
            while at < data.len()
                && (blank(data[at])
                    || (!delimited && is_ws(data[at]) && !matches!(data[at], b'\r' | b'\n')))
            {
                at += 1;
            }
            if at >= data.len() && !after_delim {
                // The data ends before this field: keep the fields read so far
                // (the columns differ in length, as in MATLAB) and stay at the end.
                break 'rows;
            }
            after_delim = false;
            if matches!(data.get(at), Some(b'\n' | b'\r')) {
                // End this record before reading anything from the next line.
                for missing in &mut columns[column..] {
                    match missing {
                        Column::Num(v) => v.push(f64::NAN),
                        Column::Str(v) => v.push(String::new()),
                    }
                }
                break;
            }
            if let TextItem::Lit(text) = item {
                if data[at..].starts_with(text.as_bytes()) {
                    at += text.len();
                    continue;
                }
                at = row_start;
                break 'rows;
            }
            let start = at;
            while at < data.len()
                && if delimited {
                    !is_delim(data[at]) && data[at] != b'\n' && data[at] != b'\r'
                } else {
                    !is_ws(data[at])
                }
            {
                at += 1;
            }
            let mut token = &data[start..at];
            while token.last().is_some_and(|&b| blank(b)) {
                token = &token[..token.len() - 1];
            }
            let value = match item {
                TextItem::Number { .. } => {
                    if token.is_empty() {
                        if !delimited {
                            at = row_start;
                            break 'rows;
                        }
                        Some(f64::NAN)
                    } else {
                        let n = number_length(token, 0, false, usize::MAX);
                        if n == token.len() {
                            Some(parse_number(token))
                        } else {
                            None
                        }
                    }
                }
                _ => Some(0.),
            };
            let Some(value) = value else {
                // A field that does not convert stops the scan at that field;
                // the fields read before it stay (columns may differ in length).
                at = start;
                break 'rows;
            };
            if !matches!(
                item,
                TextItem::Number { skip: true } | TextItem::Str { skip: true }
            ) {
                match &mut columns[column] {
                    Column::Num(v) => v.push(value),
                    Column::Str(v) => v.push(decode(token, options.latin1)),
                }
                column += 1;
            }
            // Keep a line break until the remaining fields have been filled.
            if delimited
                && at < data.len()
                && is_delim(data[at])
                && !matches!(data[at], b'\n' | b'\r')
            {
                at += 1;
                after_delim = true;
            }
        }
        if data.get(at) == Some(&b'\r') {
            at += 1 + usize::from(data.get(at + 1) == Some(&b'\n'));
        } else if data.get(at) == Some(&b'\n') {
            at += 1;
        }
        rows += 1;
    }
    Ok((columns, at))
}

impl Engine {
    fn fileio_textscan(&mut self, args: &[Value]) -> Result<Vec<Value>> {
        let who = "textscan";
        let mut rest = &args[1..];
        let mut fmt = "%f";
        if let Some(Value::Text(f)) = rest.first() {
            fmt = f;
            rest = &rest[1..];
        } else if matches!(rest.first(), Some(Value::Matrix(m)) if m.is_empty()) {
            rest = &rest[1..];
        }
        if fmt.is_empty() {
            fmt = "%f";
        }
        let mut repeat = None;
        if let Some(Value::Matrix(m)) = rest.first() {
            let n = scalar_arg(who, "repeat count", &Value::Matrix(m.clone()))?;
            if n != -1. && !n.is_infinite() {
                if n < 0. || n.fract() != 0. {
                    return Err("textscan: repeat count must be a non-negative integer".into());
                }
                repeat = Some(n as usize);
            }
            rest = &rest[1..];
        }
        let mut options = TextOptions {
            delimiters: Vec::new(),
            header_lines: 0,
            collect: false,
            latin1: false,
        };
        if !rest.len().is_multiple_of(2) {
            return Err("textscan: options must be name/value pairs".into());
        }
        for pair in rest.chunks(2) {
            let Value::Text(key) = &pair[0] else {
                return Err("textscan: option names must be character rows".into());
            };
            match (key.to_ascii_lowercase().as_str(), &pair[1]) {
                ("delimiter", Value::Text(d)) => {
                    options.delimiters = tf::unescape(d).into_bytes();
                }
                ("headerlines", v) => options.header_lines = offset_arg(who, "HeaderLines", v)?,
                ("collectoutput", v) => {
                    options.collect = scalar_arg(who, "CollectOutput", v)? != 0.
                }
                _ => return Err(format!("textscan: unsupported option '{key}'")),
            }
        }
        let items = text_format(fmt)?;
        if !items
            .iter()
            .any(|i| matches!(i, TextItem::Number { .. } | TextItem::Str { .. }))
        {
            return Err("textscan: format has no conversions".into());
        }
        let cancel = self.cancel.clone();
        let (columns, from_file) = match &args[0] {
            Value::Text(text) => {
                if text.len() > MAX_TEXT {
                    return Err("textscan: text is too long".into());
                }
                let (columns, _) = text_scan(text.as_bytes(), &items, &options, repeat, &cancel)?;
                (columns, false)
            }
            fid => {
                options.latin1 = self.fileio_with(fid, |f| Ok(f.latin1))?;
                let columns = self.fileio_with(fid, |f| {
                    let (columns, used) =
                        text_scan(&f.scan_data(), &items, &options, repeat, &cancel)?;
                    f.advance_scanned(used);
                    if f.pos >= f.data.len() {
                        f.eof = true;
                    }
                    Ok(columns)
                })?;
                (columns, true)
            }
        };
        let _ = from_file;
        // Adjacent numeric columns of one length merge into a matrix.
        let mut cells: Vec<Array> = Vec::new();
        let mut pending: Vec<Vec<f64>> = Vec::new();
        let flush = |pending: &mut Vec<Vec<f64>>, cells: &mut Vec<Array>| -> Result<()> {
            if pending.is_empty() {
                return Ok(());
            }
            let rows = pending[0].len();
            let cols = pending.len();
            let data: Vec<f64> = pending.drain(..).flatten().collect();
            cells.push(Matrix::from_column_vec(rows, cols, data)?.into());
            Ok(())
        };
        for column in columns {
            match column {
                Column::Num(values) => {
                    if options.collect && pending.first().is_some_and(|p| p.len() != values.len()) {
                        flush(&mut pending, &mut cells)?;
                    }
                    pending.push(values);
                    if !options.collect {
                        flush(&mut pending, &mut cells)?;
                    }
                }
                Column::Str(strings) => {
                    flush(&mut pending, &mut cells)?;
                    let mut slots = Vec::with_capacity(strings.len());
                    for s in &strings {
                        slots.push(Arc::new(Array::from(CharacterRow::from_text(
                            s,
                            &self.cancel,
                        )?)));
                    }
                    cells.push(CellArray::from_shared(slots.len(), 1, slots, &self.cancel)?.into());
                }
            }
        }
        flush(&mut pending, &mut cells)?;
        let slots: Vec<Arc<Array>> = cells.into_iter().map(Arc::new).collect();
        let cell = CellArray::from_shared(1, slots.len(), slots, &self.cancel)?;
        Ok(vec![Value::Container(cell.into())])
    }
}

// ---- dlmread / dlmwrite -----------------------------------------------------

fn dlm_parse(text: &[u8], delimiter: Option<&[u8]>) -> Result<Vec<Vec<f64>>> {
    let lines: Vec<(usize, &[u8])> = text
        .split(|&b| b == b'\n')
        .map(|l| l.strip_suffix(b"\r").unwrap_or(l))
        .enumerate()
        .filter(|(_, l)| !l.iter().all(|&b| is_ws(b)))
        .collect();
    let delimiter: Vec<u8> = match delimiter {
        Some(d) if !d.is_empty() => d.to_vec(),
        _ => {
            let first = lines.first().map_or(&b""[..], |&(_, line)| line);
            b",\t;"
                .iter()
                .find(|d| first.contains(d))
                .map_or(vec![b' '], |&d| vec![d])
        }
    };
    let space = delimiter == *b" ";
    let mut rows: Vec<Vec<f64>> = Vec::with_capacity(lines.len());
    for (row, line) in lines {
        let fields: Vec<&[u8]> = if space {
            line.split(|&b| is_ws(b))
                .filter(|f| !f.is_empty())
                .collect()
        } else {
            line.split(|b| delimiter.contains(b)).collect()
        };
        let mut values = Vec::with_capacity(fields.len());
        let mut offset = 0;
        for (column, field) in fields.into_iter().enumerate() {
            if space {
                while offset < line.len() && is_ws(line[offset]) {
                    offset += 1;
                }
            }
            let f = field.trim_ascii();
            if f.is_empty() {
                values.push(0.);
            } else if number_length(f, 0, false, usize::MAX) == f.len() {
                values.push(parse_number(f));
            } else {
                let rest = &line[offset..];
                return Err(format!(
                    "Mismatch between file and format string. Trouble reading 'Numeric' field from file (row number {}, field number {}) ==> {}\n",
                    row + 1,
                    column + 1,
                    bytes_text(&rest[..rest.len().min(256)])
                ));
            }
            offset += field.len() + usize::from(!space);
        }
        rows.push(values);
    }
    Ok(rows)
}

impl Engine {
    fn fileio_dlmread(&mut self, name: &str, args: &[Value]) -> Result<Vec<Value>> {
        let file = text_arg(name, "file name", args.first())?;
        let mut rest = &args[1..];
        let mut delimiter: Option<Vec<u8>> = None;
        if name == "csvread" {
            delimiter = Some(b",".to_vec());
        } else if let Some(Value::Text(d)) = rest.first() {
            delimiter = Some(tf::unescape(d).into_bytes());
            rest = &rest[1..];
        } else if rest
            .first()
            .is_some_and(|v| matches!(v, Value::Matrix(m) if m.is_empty()))
            && rest.len() > 1
        {
            rest = &rest[1..];
        }
        let (mut r1, mut c1, mut r2, mut c2) = (0usize, 0usize, usize::MAX, usize::MAX);
        match rest {
            [] => {}
            [Value::Matrix(m)] if m.len() == 4 => {
                let d = m.data();
                (r1, c1, r2, c2) = (
                    offset_arg(name, "range", &number(d[0]))?,
                    offset_arg(name, "range", &number(d[1]))?,
                    offset_arg(name, "range", &number(d[2]))?,
                    offset_arg(name, "range", &number(d[3]))?,
                );
            }
            [r, c] => {
                r1 = offset_arg(name, "row offset", r)?;
                c1 = offset_arg(name, "column offset", c)?;
            }
            // csvread(filename, r, c, range) with r, c equal to range(1:2).
            [r, c, Value::Matrix(m)] if name == "csvread" && (m.is_empty() || m.len() == 4) => {
                let [a, b, x, y] = m.data() else {
                    return Err("Index exceeds matrix dimensions.".into());
                };
                (r1, c1, r2, c2) = (
                    offset_arg(name, "range", &number(*a))?,
                    offset_arg(name, "range", &number(*b))?,
                    offset_arg(name, "range", &number(*x))?,
                    offset_arg(name, "range", &number(*y))?,
                );
                // The reference only warns here and returns a zero-padded matrix.
                if offset_arg(name, "row offset", r)? != r1
                    || offset_arg(name, "column offset", c)? != c1
                {
                    return Err(format!(
                        "{name}: R and C should match RANGE(1:2).  Use DLMREAD(FILE,DELIMITER,RANGE) instead."
                    ));
                }
            }
            [Value::Matrix(m), ..] if m.len() != 4 => {
                return Err(format!("{name}: range must be [r1 c1 r2 c2]"));
            }
            [Value::Text(_)] => {
                return Err(format!(
                    "{name}: spreadsheet-style ranges are not supported"
                ));
            }
            _ => return Err(TOO_MANY.into()),
        }
        let data = self.fileio_slurp(name, file)?;
        self.check_cancel()?;
        let rows = dlm_parse(&data, delimiter.as_deref())?;
        let width = rows.iter().map(Vec::len).max().unwrap_or(0);
        let r_end = r2.saturating_add(1).min(rows.len());
        let c_end = c2.saturating_add(1).min(width);
        if r1 > 0 && r1 >= rows.len() {
            return Err("Empty format string is not supported at the end of a file.".into());
        }
        if r1 >= r_end || c1 >= c_end {
            return Ok(vec![Value::Matrix(Matrix::filled(0, 0, 0.)?)]);
        }
        let (nr, nc) = (r_end - r1, c_end - c1);
        Matrix::checked_len(nr, nc)?;
        self.charge(nr * nc)?;
        let mut values = vec![0.; nr * nc];
        for (i, row) in rows[r1..r_end].iter().enumerate() {
            if i % CHECK_EVERY == 0 {
                self.check_cancel()?;
            }
            for (j, &v) in row.iter().enumerate().skip(c1).take(nc) {
                values[(j - c1) * nr + i] = v;
            }
        }
        Ok(vec![Value::Matrix(Matrix::from_column_vec(
            nr, nc, values,
        )?)])
    }

    fn fileio_dlmwrite(&mut self, name: &str, args: &[Value]) -> Result<()> {
        if args.len() < 2 {
            return Err(TOO_FEW.into());
        }
        let file = text_arg(name, "file name", args.first())?;
        let (rows, cols, data): (usize, usize, Vec<f64>) = match &args[1] {
            Value::Matrix(m) => (m.rows(), m.cols(), m.data().to_vec()),
            Value::Logical(m) => (
                m.rows(),
                m.cols(),
                m.data().iter().map(|&v| f64::from(v)).collect(),
            ),
            Value::Complex(m) => (m.rows(), m.cols(), m.data().iter().map(|z| z.re).collect()),
            _ => return Err(format!("{name}: data must be a numeric matrix")),
        };
        let mut rest = &args[2..];
        let mut delimiter = ",".to_owned();
        let mut format = "%.5g".to_owned();
        let mut newline = if cfg!(windows) { "\r\n" } else { "\n" }.to_owned();
        let (mut roff, mut coff) = (0usize, 0usize);
        let mut append = false;
        let is_option = |s: &str| {
            matches!(
                s.to_ascii_lowercase().as_str(),
                "delimiter" | "precision" | "newline" | "roffset" | "coffset" | "-append"
            )
        };
        if name == "csvwrite" {
            if rest.len() > 2 {
                return Err(TOO_MANY.into());
            }
            if rest.len() == 2 {
                roff = offset_arg(name, "row offset", &rest[0])?;
                coff = offset_arg(name, "column offset", &rest[1])?;
            } else if !rest.is_empty() {
                return Err(TOO_FEW.into());
            }
            rest = &[];
        } else {
            if let Some(Value::Text(d)) = rest.first()
                && !is_option(d)
            {
                delimiter = tf::unescape(d);
                rest = &rest[1..];
            }
            if let [r, c, ..] = rest
                && matches!(r, Value::Matrix(_))
                && matches!(c, Value::Matrix(_))
            {
                roff = offset_arg(name, "row offset", r)?;
                coff = offset_arg(name, "column offset", c)?;
                rest = &rest[2..];
            }
        }
        let mut i = 0;
        while i < rest.len() {
            let Value::Text(key) = &rest[i] else {
                return Err(format!("{name}: options must be name/value pairs"));
            };
            let key = key.to_ascii_lowercase();
            if key == "-append" {
                append = true;
                i += 1;
                continue;
            }
            let value = rest
                .get(i + 1)
                .ok_or_else(|| format!("{name}: option '{key}' needs a value"))?;
            match (key.as_str(), value) {
                ("delimiter", Value::Text(d)) => delimiter = tf::unescape(d),
                ("precision", Value::Text(p)) => format = p.clone(),
                ("precision", v) => {
                    let n = scalar_arg(name, "precision", v)?;
                    if !(0. ..=64.).contains(&n) || n.fract() != 0. {
                        return Err(format!("{name}: precision must be an integer in 0..64"));
                    }
                    format = format!("%.{}g", n as usize);
                }
                ("newline", Value::Text(n)) => {
                    newline = match n.to_ascii_lowercase().as_str() {
                        "pc" => "\r\n".into(),
                        "unix" => "\n".into(),
                        // The reference's own message; it has no 'mac' option.
                        _ => return Err("Newline must be either 'pc' or 'unix'.".into()),
                    }
                }
                ("roffset", v) => roff = offset_arg(name, "roffset", v)?,
                ("coffset", v) => coff = offset_arg(name, "coffset", v)?,
                _ => return Err(format!("{name}: unsupported option '{key}'")),
            }
            i += 2;
        }
        let path = self.fileio_path(name, file)?;
        let width = cols
            .checked_add(coff)
            .ok_or_else(|| format!("{name}: output exceeds the 8 MiB limit"))?;
        // Lower bound of the output (offsets, delimiters, line breaks) before any
        // text is built, so huge offsets are refused at once.
        let (dlen, nlen) = (delimiter.len(), newline.len());
        let blank_len = dlen
            .checked_mul(width.saturating_sub(1))
            .and_then(|n| n.checked_add(nlen));
        let row_len = coff
            .checked_add(cols.saturating_sub(1))
            .and_then(|n| n.checked_mul(dlen))
            .and_then(|n| n.checked_add(nlen));
        let blank_rows = if cols > 0 || coff > 0 { roff } else { 0 };
        let least = blank_len
            .and_then(|b| b.checked_mul(blank_rows))
            .zip(row_len.and_then(|r| r.checked_mul(rows)))
            .and_then(|(a, b)| a.checked_add(b));
        if least.is_none_or(|n| n > MAX_FILE_BYTES) {
            return Err(format!("{name}: output exceeds the 8 MiB limit"));
        }
        // Every part is bounded on its own, whatever the row count: the offset
        // prefix, the delimiters of one line and the per-row format text.
        let line_cap = |n: Option<usize>| n.is_none_or(|n| n > MAX_FILE_BYTES);
        if line_cap(dlen.checked_mul(2).and_then(|d| d.checked_mul(width)))
            || line_cap(
                format
                    .len()
                    .checked_add(2 * dlen)
                    .and_then(|n| n.checked_mul(cols.max(1))),
            )
            || line_cap(nlen.checked_mul(2))
        {
            return Err(format!("{name}: output exceeds the 8 MiB limit"));
        }
        // The precision is checked on one number, even when no value is written.
        tf::format_items(
            name,
            &format!("{format}\n"),
            &[Item::Num(0.0)],
            &self.cancel,
        )?;
        let escape = |s: &str| s.replace('\\', "\\\\").replace('%', "%%");
        let mut row_format = String::new();
        row_format.push_str(&escape(&delimiter).repeat(coff));
        for j in 0..cols {
            if j > 0 {
                row_format.push_str(&escape(&delimiter));
            }
            row_format.push_str(&format);
        }
        row_format.push_str(&escape(&newline));
        let mut text = String::new();
        let blank_row = format!("{}{}", delimiter.repeat(width.saturating_sub(1)), newline);
        for i in 0..blank_rows {
            if i % CHECK_EVERY == 0 {
                self.check_cancel()?;
            }
            text.push_str(&blank_row);
        }
        let chunk_rows = (16384 / cols.max(1))
            .min((1 << 20) / row_format.len().max(1))
            .max(1);
        let mut row = 0;
        while row < rows {
            self.check_cancel()?;
            let end = (row + chunk_rows).min(rows);
            let mut items = Vec::with_capacity((end - row) * cols);
            for r in row..end {
                items.extend((0..cols).map(|c| Item::Num(data[c * rows + r])));
            }
            let mut chunk_format = String::new();
            for _ in row..end {
                chunk_format.push_str(&row_format);
            }
            let formatted = tf::format_items(name, &chunk_format, &items, &self.cancel)?;
            text.push_str(&formatted);
            if text.len() > MAX_FILE_BYTES {
                return Err(format!("{name}: output exceeds the 8 MiB limit"));
            }
            row = end;
        }
        // Everything is checked and formatted: only now open (and truncate) the file.
        let mut options = OpenOptions::new();
        options.write(true).create(true);
        if append {
            options.append(true);
        } else {
            options.truncate(true);
        }
        if std::fs::metadata(&path).is_ok_and(|m| !m.is_file()) {
            return Err(format!("{name}: Not a regular file"));
        }
        let mut sink = options
            .open(&path)
            .map_err(|e| format!("{name}: {}", io_message(&e)))?;
        sink.write_all(text.as_bytes())
            .map_err(|e| format!("{name}: {}", io_message(&e)))?;
        Ok(())
    }
}
