use super::super::{Engine, Value};
use super::{Metadata, VideoSource, admit, avi, buffer};
use crate::{
    Array, CharacterRow, LogicalMatrix, Matrix, Result, StructArray, TypedArray,
    typed_array::TypedData,
};
use std::{path::PathBuf, sync::Arc};
pub(crate) const METHODS: &[&str] = &["hasFrame", "readFrame", "read", "disp", "isvalid", "delete"];
const PROPS: &[&str] = &[
    "Name",
    "Path",
    "Duration",
    "FrameRate",
    "NumFrames",
    "Height",
    "Width",
    "BitsPerPixel",
    "VideoFormat",
    "Colormap",
    "CurrentTime",
    "Tag",
    "UserData",
];
#[derive(Clone)]
pub(crate) struct Reader {
    source: Arc<dyn VideoSource>,
    path: PathBuf,
    pub current: f64,
    next: usize,
    tag: String,
    pub user: Arc<Array>,
}
fn scalar(v: &Array) -> Result<f64> {
    if v.len() != 1 || v.is_complex() {
        return Err("VideoReader: expected a real numeric scalar.".into());
    }
    match v {
        Array::Numeric(m) => Ok(m.data()[0]),
        Array::Typed(t) => Ok(t.real(0)),
        _ => Err("VideoReader: expected a numeric scalar.".into()),
    }
}
fn text(v: &Array) -> Result<String> {
    match v {
        Array::Character(c) if c.rows() <= 1 => Ok(String::from_utf16_lossy(c.units())),
        Array::Str(s) if s.len() == 1 => s.data()[0]
            .as_ref()
            .map(|s| s.to_string())
            .ok_or("VideoReader: text cannot be missing.".into()),
        _ => Err("VideoReader: expected text.".into()),
    }
}
fn value_text(e: &Engine, v: &Value) -> Result<String> {
    if let Value::Container(Array::Str(_)) = v {
        text(&v.clone().array(&e.cancel)?)
    } else {
        e.text_value(v.clone())
    }
}
fn budget(e: &Engine) -> usize {
    let eval = e.limits.evaluation_units.map_or(usize::MAX, |n| {
        n.saturating_sub(e.evaluated_elements).saturating_mul(8)
    });
    e.limits.memory_bytes.min(eval)
}
impl Reader {
    pub(crate) fn new(e: &mut Engine, args: &[Value]) -> Result<Self> {
        if args.is_empty() || args.len().is_multiple_of(2) {
            return Err("VideoReader requires a filename followed by name/value pairs.".into());
        }
        let name = value_text(e, &args[0])?;
        if name.len() > 32768 || name.contains('\0') || name.contains("://") {
            return Err("VideoReader: invalid or unsupported filename.".into());
        }
        #[cfg(target_arch = "wasm32")]
        if !std::path::Path::new(&name)
            .extension()
            .is_some_and(|v| v.eq_ignore_ascii_case("avi"))
        {
            return Err(super::ffmpeg::missing("VideoReader", "video"));
        }
        let path = e.file_path(&name, "")?;
        let path = std::fs::canonicalize(path)
            .map_err(|e| format!("VideoReader: cannot open file: {e}"))?;
        if !path.is_file() {
            return Err("VideoReader: input must be a regular file".into());
        }
        let source: Arc<dyn VideoSource> = if path
            .extension()
            .is_some_and(|v| v.eq_ignore_ascii_case("avi"))
        {
            match avi::open(&path, budget(e), &e.cancel) {
                Ok(s) => Arc::new(s),
                Err(error) if error.contains("unsupported codec") => {
                    super::ffmpeg::open(e, &path, budget(e))
                        .map_err(|reason| format!("{reason} ({error})"))?
                }
                Err(error) => return Err(error),
            }
        } else {
            super::ffmpeg::open(e, &path, budget(e))?
        };
        e.charge(source.bytes().div_ceil(8))?;
        let mut r = Self {
            source,
            path,
            current: 0.,
            next: 0,
            tag: String::new(),
            user: Arc::new(Array::Numeric(Matrix::filled(0, 0, 0.)?)),
        };
        for pair in args[1..].as_chunks::<2>().0 {
            let name = value_text(e, &pair[0])?;
            r = r.assign(&name, pair[1].clone().array(&e.cancel)?)?;
        }
        Ok(r)
    }
    pub(crate) fn property_names(&self) -> Vec<String> {
        PROPS.iter().map(|s| (*s).to_owned()).collect()
    }
    fn meta(&self) -> &Metadata {
        self.source.metadata()
    }
    fn duration(&self) -> f64 {
        self.meta()
            .duration
            .unwrap_or(self.meta().frames as f64 / self.meta().rate)
    }
    pub(crate) fn bytes(&self) -> usize {
        self.source.bytes()
            + self.tag.len()
            + self.path.as_os_str().len()
            + self.user.retained_bytes()
            + 128
    }
    pub(crate) fn assign(&self, name: &str, v: Array) -> Result<Self> {
        let mut r = self.clone();
        match name {
            "CurrentTime" => {
                let t = scalar(&v)?;
                if !t.is_finite() || t < 0. || t > self.duration() {
                    return Err(
                        "CurrentTime must be between zero and the duration of the video.".into(),
                    );
                }
                self.source.reset();
                r.current = t;
                r.next = if t == self.duration() {
                    self.meta().frames
                } else {
                    let p = t * self.meta().rate;
                    let n = p.round();
                    if (p - n).abs() < 1e-9 {
                        n as usize
                    } else {
                        p.floor() as usize
                    }
                };
            }
            "Tag" => r.tag = text(&v)?,
            "UserData" => r.user = Arc::new(v),
            _ if PROPS.contains(&name) => {
                return Err(format!(
                    "You cannot set the read-only property '{name}' of VideoReader."
                ));
            }
            _ => {
                return Err(format!(
                    "No public property '{name}' exists for class VideoReader."
                ));
            }
        }
        Ok(r)
    }
    fn colormap(&self, p: &[[u8; 3]]) -> Result<Array> {
        if self.meta().format != "Indexed" {
            return Ok(Array::Numeric(Matrix::filled(0, 0, 0.)?));
        }
        let data = (0..3)
            .flat_map(|c| p.iter().map(move |x| f64::from(x[c]) / 255.))
            .collect();
        Ok(Array::Numeric(Matrix::from_column_vec(p.len(), 3, data)?))
    }
    pub(crate) fn property(&self, name: &str) -> Result<Value> {
        let m = self.meta();
        let n = match name {
            "Name" => {
                return Ok(Value::Text(
                    self.path
                        .file_name()
                        .unwrap_or_default()
                        .to_string_lossy()
                        .into_owned(),
                ));
            }
            "Path" => {
                return Ok(Value::Text(
                    self.path
                        .parent()
                        .unwrap_or(&self.path)
                        .to_string_lossy()
                        .into_owned(),
                ));
            }
            "VideoFormat" => return Ok(Value::Text(m.format.into())),
            "Colormap" => return self.colormap(&m.palette).map(Value::from),
            "Tag" => return Ok(Value::Text(self.tag.clone())),
            "UserData" => return Ok(Value::from(self.user.as_ref().clone())),
            "Duration" => self.duration(),
            "FrameRate" => m.rate,
            "NumFrames" => m.frames as f64,
            "Width" => m.width as f64,
            "Height" => m.height as f64,
            "BitsPerPixel" => f64::from(m.bits),
            "CurrentTime" => self.current,
            _ => {
                return Err(format!(
                    "No appropriate method, property, or field '{name}' for class 'VideoReader'."
                ));
            }
        };
        Ok(Value::Matrix(Matrix::scalar(n)))
    }
    pub(crate) fn body(&self) -> String {
        let m = self.meta();
        format!(
            "  VideoReader with properties:\n\n    Name: '{}'\n    Path: '{}'\n    Duration: {}\n    FrameRate: {}\n    NumFrames: {}\n    Height: {}\n    Width: {}\n    BitsPerPixel: {}\n    VideoFormat: '{}'\n    Colormap: [{}x3 double]\n    CurrentTime: {}\n    Tag: '{}'\n    UserData: [{}x{} {}]\n",
            self.path.file_name().unwrap_or_default().to_string_lossy(),
            self.path.parent().unwrap_or(&self.path).display(),
            self.duration(),
            m.rate,
            m.frames,
            m.height,
            m.width,
            m.bits,
            m.format,
            if m.format == "Indexed" {
                m.palette.len()
            } else {
                0
            },
            self.current,
            self.tag,
            self.user.rows(),
            self.user.cols(),
            self.user.class()
        )
    }
}
fn uint8(data: Vec<u8>, dims: &[usize]) -> Result<Array> {
    let cols = dims[1..].iter().try_fold(1usize, |a, &b| {
        a.checked_mul(b).ok_or("VideoReader: dimensions overflow")
    })?;
    let t = TypedArray::checked(dims[0], cols, TypedData::Uint8(data.into()))?;
    Ok(Array::Typed(t.with_dims(dims)?))
}
fn index(v: f64, n: usize) -> Result<usize> {
    if v == f64::INFINITY {
        return n.checked_sub(1).ok_or("VideoReader: empty video".into());
    }
    if !v.is_finite() || v.fract() != 0. || v < 1. || v > n as f64 {
        return Err(
            "The frame index must be a positive integer within the number of frames in the video."
                .into(),
        );
    }
    Ok(v as usize - 1)
}
/// Returns the result plus an optional replacement state. The object table
/// installs state only after successful decoding, so errors do not advance it.
pub(crate) fn call(
    e: &mut Engine,
    r: &Reader,
    name: &str,
    args: &[Value],
) -> Result<(Value, Option<Reader>)> {
    if name == "read" {
        r.source.reset();
    }
    let result = call_inner(e, r, name, args);
    if result.is_err() || name == "read" {
        r.source.reset();
    }
    result
}
fn call_inner(
    e: &mut Engine,
    r: &Reader,
    name: &str,
    args: &[Value],
) -> Result<(Value, Option<Reader>)> {
    e.check_cancel()?;
    if name == "hasFrame" {
        if !args.is_empty() {
            return Err("Too many input arguments.".into());
        }
        return Ok((
            Value::Logical(LogicalMatrix::scalar(r.next < r.meta().frames)),
            None,
        ));
    }
    let mut args = args;
    let native = if let Some(last) = args.last() {
        if matches!(
            last,
            Value::Text(_)
                | Value::QuotedString(_)
                | Value::Container(Array::Character(_))
                | Value::Container(Array::Str(_))
        ) {
            if value_text(e, last)? != "native" {
                return Err("VideoReader: expected 'native'.".into());
            }
            args = &args[..args.len() - 1];
            true
        } else {
            false
        }
    } else {
        false
    };
    let m = r.meta();
    let (a, b) = if name == "readFrame" {
        if !args.is_empty() {
            return Err("Too many input arguments.".into());
        }
        if r.next >= m.frames {
            return Err("No more frames available to read from file.".into());
        }
        (r.next, r.next)
    } else if name == "read" {
        match args {
            [] => (0, m.frames - 1),
            [v] => {
                let a = v.clone().array(&e.cancel)?;
                if !(1..=2).contains(&a.len()) || !matches!(a, Array::Numeric(_) | Array::Typed(_))
                {
                    return Err(
                        "VideoReader: expected one numeric frame index or [first last].".into(),
                    );
                }
                let nums = a.to_numeric(&e.cancel)?;
                let data = nums.data();
                match data {
                    [x] => {
                        let k = index(*x, m.frames)?;
                        (k, k)
                    }
                    [a, b] => {
                        let a = index(*a, m.frames)?;
                        let b = index(*b, m.frames)?;
                        if a > b {
                            return Err("VideoReader: frame range must be increasing.".into());
                        }
                        (a, b)
                    }
                    _ => {
                        return Err("VideoReader: expected one frame index or [first last].".into());
                    }
                }
            }
            _ => return Err("Too many input arguments.".into()),
        }
    } else {
        return Err(format!("VideoReader: unsupported method '{name}'."));
    };
    let count = b - a + 1;
    let movie = native && m.format != "RGB24";
    let channels = if m.format == "Grayscale" || movie {
        1
    } else {
        3
    };
    let frame_len = m
        .width
        .checked_mul(m.height)
        .and_then(|n| n.checked_mul(channels))
        .ok_or("VideoReader: output size overflow")?;
    let bytes = frame_len
        .checked_mul(count)
        .ok_or("VideoReader: output size overflow")?;
    // Admit the engine's array/dimension bounds before reserving output.
    let array_frames = if movie { 1 } else { count };
    let array_cols = m
        .width
        .checked_mul(channels)
        .and_then(|n| n.checked_mul(array_frames))
        .ok_or("VideoReader: dimensions overflow")?;
    Matrix::checked_len(m.height, array_cols)?;
    let scratch = m
        .width
        .checked_mul(m.height)
        .and_then(|n| n.checked_mul(64))
        .and_then(|n| n.checked_add(64 * 1024))
        .ok_or("VideoReader: frame size overflow")?;
    let overhead = if movie {
        count
            .checked_mul(r.source.max_palette_len() * 24 + 128)
            .ok_or("VideoReader: movie size overflow")?
    } else {
        0
    };
    if movie && count > crate::script_containers::MAX_CELL_SLOTS / 2 {
        return Err("VideoReader: native movie exceeds the struct slot limit".into());
    }
    let output_storage = bytes
        .checked_mul(if movie { 2 } else { 1 })
        .ok_or("VideoReader: output size overflow")?;
    let total = output_storage
        .checked_add(scratch)
        .and_then(|n| n.checked_add(overhead))
        .ok_or("VideoReader: output size overflow")?;
    admit(total, budget(e))?;
    e.charge(total.div_ceil(8))?;
    let mut data = buffer(bytes, e.limits.memory_bytes)?;
    let mut slots = Vec::new();
    if movie {
        slots
            .try_reserve_exact(count * 2)
            .map_err(|_| "VideoReader: unable to allocate movie slots")?;
    }
    for (j, i) in (a..=b).enumerate() {
        e.check_cancel()?;
        let raw = r
            .source
            .frame(i, total - output_storage - overhead, &e.cancel)?;
        let palette = r.source.palette(i);
        let pixels = m.width * m.height;
        for y in 0..m.height {
            if y.is_multiple_of(128) {
                e.check_cancel()?;
            }
            for x in 0..m.width {
                for c in 0..channels {
                    let value = if m.format == "Indexed" && !movie {
                        palette[usize::from(raw[y * m.width + x])][c]
                    } else {
                        raw[(y * m.width + x) * channels + c]
                    };
                    data[j * frame_len + y + x * m.height + c * pixels] = value;
                }
            }
        }
        if movie {
            slots.push(Arc::new(uint8(
                data[j * frame_len..(j + 1) * frame_len].to_vec(),
                &[m.height, m.width],
            )?));
            slots.push(Arc::new(r.colormap(palette)?));
        }
    }
    let result = if movie {
        Value::from(Array::Struct(StructArray::from_slots(
            1,
            count,
            vec!["cdata".into(), "colormap".into()],
            slots,
            &e.cancel,
        )?))
    } else {
        Value::from(uint8(data, &[m.height, m.width, channels, count])?)
    };
    let changed = if name == "readFrame" {
        let mut next = r.clone();
        next.next += 1;
        next.current = (r.current + 1. / m.rate).min(r.duration());
        Some(next)
    } else {
        None
    };
    Ok((result, changed))
}
pub(crate) fn formats(e: &Engine) -> Result<Value> {
    let char_array = |s: &str| -> Result<Arc<Array>> {
        Ok(Arc::new(Array::Character(CharacterRow::from_units(
            s.encode_utf16().collect(),
            &e.cancel,
        )?)))
    };
    let mut records = vec![("avi", "AVI (Motion JPEG, uncompressed)")];
    if super::ffmpeg::tools(e)?.ffmpeg.is_some() {
        records.extend([
            ("mp4", "MPEG-4 (ffmpeg)"),
            ("mkv", "Matroska (ffmpeg)"),
            ("mov", "QuickTime (ffmpeg)"),
            ("mj2", "Motion JPEG 2000 (ffmpeg)"),
            ("webm", "WebM (ffmpeg)"),
        ]);
    }
    let count = records.len();
    let mut slots = Vec::new();
    for (ext, description) in records {
        slots.extend([
            char_array(ext)?,
            char_array(description)?,
            Arc::new(Array::Logical(LogicalMatrix::scalar(true))),
            Arc::new(Array::Logical(LogicalMatrix::scalar(false))),
        ]);
    }
    Ok(Value::from(Array::Struct(StructArray::from_slots(
        1,
        count,
        vec![
            "Extension".into(),
            "Description".into(),
            "ContainsVideo".into(),
            "ContainsAudio".into(),
        ],
        slots,
        &e.cancel,
    )?)))
}
