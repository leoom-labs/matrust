//! Shared VideoWriter handle state and the native sink selection seam for 7b3.
use super::super::{Engine, Value};
use super::avi_writer::Sink;
use super::ffmpeg;
use crate::{Array, CharacterRow, Matrix, Result, StructArray};
#[cfg(not(target_arch = "wasm32"))]
use std::sync::atomic::AtomicBool;
use std::{
    path::PathBuf,
    sync::{Arc, Mutex, MutexGuard},
};

pub(crate) const WRITER_METHODS: &[&str] =
    &["open", "writeVideo", "close", "delete", "disp", "isvalid"];
const PROPS: &[&str] = &[
    "Filename",
    "Path",
    "FileFormat",
    "FrameRate",
    "Quality",
    "ColorChannels",
    "VideoBitsPerPixel",
    "VideoCompressionMethod",
    "VideoFormat",
    "Height",
    "Width",
    "FrameCount",
    "Duration",
    "Colormap",
    "LosslessCompression",
    "CompressionRatio",
    "MJ2BitDepth",
];
const PROFILES: [&str; 4] = [
    "Motion JPEG AVI",
    "Uncompressed AVI",
    "Grayscale AVI",
    "Indexed AVI",
];
#[derive(Clone, Copy, PartialEq)]
enum Profile {
    Jpeg,
    Rgb,
    Gray,
    Indexed,
    Mpeg4,
    Jpeg2000,
    Archival,
}
/// 7b3 replaces only this branch for non-native profiles and adds its sink.
fn profile(name: &str) -> Result<Profile> {
    match name {
        "Motion JPEG AVI" => Ok(Profile::Jpeg),
        "Uncompressed AVI" => Ok(Profile::Rgb),
        "Grayscale AVI" => Ok(Profile::Gray),
        "Indexed AVI" => Ok(Profile::Indexed),
        "MPEG-4" => Ok(Profile::Mpeg4),
        "Motion JPEG 2000" => Ok(Profile::Jpeg2000),
        "Archival" => Ok(Profile::Archival),
        _ => Err(format!("VideoWriter: unknown profile '{name}'")),
    }
}
impl Profile {
    fn external(self) -> bool {
        matches!(self, Self::Mpeg4 | Self::Jpeg2000 | Self::Archival)
    }
    fn mj2(self) -> bool {
        matches!(self, Self::Jpeg2000 | Self::Archival)
    }
    fn extension(self) -> &'static str {
        if self == Self::Mpeg4 {
            "mp4"
        } else if self.mj2() {
            "mj2"
        } else {
            "avi"
        }
    }
    fn channels(self) -> usize {
        if matches!(self, Self::Gray | Self::Indexed) {
            1
        } else {
            3
        }
    }
    fn format(self) -> &'static str {
        match self {
            Self::Gray => "Grayscale",
            Self::Indexed => "Indexed",
            _ => "RGB24",
        }
    }
    fn compression(self) -> &'static str {
        match self {
            Self::Jpeg => "Motion JPEG",
            Self::Mpeg4 => "H.264",
            Self::Jpeg2000 | Self::Archival => "Motion JPEG 2000",
            _ => "None",
        }
    }
}
#[derive(Clone)]
pub(crate) struct Writer(Arc<Mutex<State>>);
struct State {
    path: PathBuf,
    profile: Profile,
    rate: f64,
    quality: u8,
    palette: Vec<[u8; 3]>,
    locked: bool,
    closed: bool,
    sink: Option<Sink>,
    #[cfg(not(target_arch = "wasm32"))]
    external: Option<ffmpeg::Sink>,
    #[cfg(not(target_arch = "wasm32"))]
    tools: Arc<ffmpeg::Tools>,
    #[cfg(not(target_arch = "wasm32"))]
    cancel: Arc<AtomicBool>,
    lossless: bool,
    ratio: f64,
    requested_depth: Option<u8>,
    depth: u8,
    channels: usize,
    width: usize,
    height: usize,
    frames: usize,
}
fn number(a: &Array, i: usize) -> Result<f64> {
    match a {
        Array::Numeric(m) if !a.is_complex() => Ok(m.data()[i]),
        Array::Typed(t) if !a.is_complex() => Ok(t.real(i)),
        _ => Err("VideoWriter: expected real numeric data".into()),
    }
}
fn scalar(a: &Array) -> Result<f64> {
    if a.len() != 1 {
        return Err("VideoWriter: expected a numeric scalar".into());
    }
    number(a, 0)
}
fn text(e: &Engine, v: &Value) -> Result<String> {
    if let Value::Container(Array::Str(s)) = v
        && s.len() == 1
    {
        return s.data()[0]
            .as_ref()
            .map(|s| s.to_string())
            .ok_or("VideoWriter: missing text".into());
    }
    e.text_value(v.clone())
}
fn palette(a: &Array) -> Result<Vec<[u8; 3]>> {
    if a.rows() == 0 || a.rows() > 256 || a.cols() != 3 || a.dims().len() > 2 || a.is_complex() {
        return Err(
            "VideoWriter: Colormap must be a nonempty P-by-3 matrix with at most 256 rows".into(),
        );
    }
    let mut out = vec![[0; 3]; a.rows()];
    for (i, row) in out.iter_mut().enumerate() {
        for (c, value) in row.iter_mut().enumerate() {
            let n = number(a, i + c * a.rows())?;
            if !n.is_finite() || !(0. ..=1.).contains(&n) {
                return Err("VideoWriter: Colormap values must be in [0,1]".into());
            }
            *value = (n * 255.).round() as u8;
        }
    }
    Ok(out)
}
fn budget(e: &Engine) -> usize {
    e.limits
        .memory_bytes
        .min(e.limits.evaluation_units.map_or(usize::MAX, |n| {
            n.saturating_sub(e.evaluated_elements).saturating_mul(8)
        }))
}
fn admit(n: usize, budget: usize) -> Result<()> {
    if n > budget {
        Err("VideoWriter exceeds the evaluation memory limit".into())
    } else {
        Ok(())
    }
}
impl State {
    fn finish(&mut self) -> Result<()> {
        self.closed = self.locked;
        let result = self.sink.as_mut().map_or(Ok(()), Sink::finish);
        self.sink = None;
        #[cfg(not(target_arch = "wasm32"))]
        if let Some(mut s) = self.external.take() {
            s.finish(&self.cancel)?;
        }
        result
    }
    fn abort(&mut self) {
        #[cfg(not(target_arch = "wasm32"))]
        {
            self.external = None;
        }
        self.closed = self.locked;
    }
    fn open(&mut self, available: usize) -> Result<()> {
        #[cfg(not(target_arch = "wasm32"))]
        if self.external.is_some() {
            return Ok(());
        }
        if self.sink.is_some() {
            return Ok(());
        }
        super::file_io::check_output(&self.path)?;
        if self.profile.external() {
            admit(3 * 1024 * 1024, available)?;
            #[cfg(not(target_arch = "wasm32"))]
            {
                self.external = Some(ffmpeg::Sink::new(
                    self.tools.clone(),
                    &self.path,
                    ffmpeg::Config {
                        rate: self.rate,
                        mp4: self.profile == Profile::Mpeg4,
                        quality: self.quality,
                        lossless: self.lossless,
                        ratio: self.ratio,
                    },
                    &self.cancel,
                )?);
            }
            #[cfg(target_arch = "wasm32")]
            {
                return Err(ffmpeg::missing("VideoWriter", self.profile.extension()));
            }
        } else {
            self.sink = Some(Sink::new(
                &self.path,
                self.rate,
                self.profile == Profile::Jpeg,
            )?);
        }
        self.locked = true;
        self.closed = false;
        self.frames = 0;
        self.width = 0;
        self.height = 0;
        self.depth = self.requested_depth.unwrap_or(8);
        self.channels = self.profile.channels();
        Ok(())
    }
    fn bytes(&self) -> usize {
        #[cfg(not(target_arch = "wasm32"))]
        let external = self.external.as_ref().map_or(0, ffmpeg::Sink::bytes);
        #[cfg(target_arch = "wasm32")]
        let external = 0;
        external
            + 512
            + self.path.as_os_str().len()
            + self.palette.capacity() * 3
            + self.sink.as_ref().map_or(0, Sink::bytes)
    }
    fn set(&mut self, name: &str, a: &Array) -> Result<()> {
        if !PROPS.contains(&name) {
            return Err(format!(
                "No public property '{name}' exists for class VideoWriter."
            ));
        }
        if !matches!(
            name,
            "FrameRate"
                | "Quality"
                | "Colormap"
                | "LosslessCompression"
                | "CompressionRatio"
                | "MJ2BitDepth"
        ) {
            return Err(format!(
                "You cannot set the read-only property '{name}' of VideoWriter."
            ));
        }
        if self.locked {
            return Err(format!(
                "VideoWriter: cannot set property '{name}' after calling open."
            ));
        }
        match name {
            "FrameRate" => {
                let n = scalar(a)?;
                if !n.is_finite() || !(0.001..=4294.).contains(&n) {
                    return Err("VideoWriter: FrameRate must be finite and in [0.001,4294]".into());
                }
                self.rate = n;
            }
            "LosslessCompression" | "CompressionRatio" | "MJ2BitDepth" => {
                if !self.profile.mj2() {
                    return Err(format!(
                        "VideoWriter: {name} applies only to Motion JPEG 2000"
                    ));
                }
                let n = if let Array::Logical(a) = a {
                    if a.len() != 1 {
                        return Err("VideoWriter: expected scalar".into());
                    }
                    f64::from(a.data()[0])
                } else {
                    scalar(a)?
                };
                match name {
                    "LosslessCompression" if n == 0. || n == 1. => self.lossless = n == 1.,
                    "CompressionRatio" if self.lossless => return Err(
                        "VideoWriter: CompressionRatio cannot be set with LosslessCompression true"
                            .into(),
                    ),
                    "CompressionRatio"
                        if n.is_finite() && n > 1. && n.fract() == 0. && n <= 1_000_000. =>
                    {
                        self.ratio = n
                    }
                    "MJ2BitDepth" if n == 8. || n == 16. => {
                        self.requested_depth = Some(n as u8);
                        self.depth = n as u8;
                    }
                    "MJ2BitDepth" => {
                        return Err(
                            "VideoWriter: this backend supports MJ2BitDepth 8 or 16 only".into(),
                        );
                    }
                    _ => return Err(format!("VideoWriter: invalid {name}")),
                }
            }
            "Quality" => {
                if !matches!(self.profile, Profile::Jpeg | Profile::Mpeg4) {
                    return Err(
                        "VideoWriter: Quality applies only to Motion JPEG AVI or MPEG-4".into(),
                    );
                }
                let n = scalar(a)?;
                if !n.is_finite() || !(0. ..=100.).contains(&n) || n.fract() != 0. {
                    return Err("VideoWriter: Quality must be an integer in [0,100]".into());
                }
                self.quality = n as u8;
            }
            "Colormap" => {
                if self.profile != Profile::Indexed {
                    return Err("VideoWriter: Colormap applies only to Indexed AVI".into());
                }
                self.palette = palette(a)?;
            }
            _ => unreachable!(),
        }
        Ok(())
    }
    fn array(&mut self, e: &mut Engine, a: &Array, map: Option<&Array>) -> Result<()> {
        let dims = a.dims();
        if a.is_empty()
            || dims.len() > 4
            || a.is_complex()
            || !(matches!(a.class(), "uint8" | "single" | "double")
                || (self.profile.mj2() && a.class() == "uint16"))
        {
            return Err(
                "VideoWriter: frames must be nonempty real uint8, single or double arrays".into(),
            );
        }
        let h = dims[0];
        let w = dims[1];
        let channels = dims.get(2).copied().unwrap_or(1);
        let frames = dims.get(3).copied().unwrap_or(1);
        if h > 32767
            || w > 32767
            || !matches!(channels, 1 | 3)
            || (self.profile.channels() == 1 && channels != 1)
        {
            return Err("VideoWriter: expected H-by-W[-by-1-by-F] or H-by-W-by-3[-by-F]; dimensions must be at most 32767".into());
        }
        let pixels = h.checked_mul(w).ok_or("VideoWriter: frame size overflow")?;
        Matrix::checked_len(
            h,
            w.checked_mul(channels)
                .ok_or("VideoWriter: dimensions overflow")?,
        )?;
        if self.frames > 0 && (self.width != w || self.height != h) {
            return Err(
                "VideoWriter: frame size must match the first frame's Height and Width".into(),
            );
        }
        let map = map.filter(|a| !a.is_empty());
        if self.profile == Profile::Gray && map.is_some() {
            return Err("VideoWriter: grayscale movie colormap must be empty".into());
        }
        let frame_map = if channels == 1 {
            map.map(palette).transpose()?
        } else {
            None
        };
        if self.profile == Profile::Indexed {
            if let Some(p) = &frame_map {
                if self.frames == 0 {
                    self.palette = p.clone();
                } else if p != &self.palette {
                    return Err("VideoWriter: frame colormap must match the first frame".into());
                }
            }
            if self.palette.is_empty() {
                return Err(
                    "VideoWriter: Indexed AVI requires a Colormap before the first frame".into(),
                );
            }
        }
        let out_channels = if self.profile.mj2() && channels == 1 && frame_map.is_none() {
            1
        } else {
            self.profile.channels()
        };
        let depth = if self.profile.mj2() {
            self.requested_depth
                .unwrap_or(if a.class() == "uint16" { 16 } else { 8 })
        } else {
            8
        };
        if self.frames > 0 && (self.channels != out_channels || self.depth != depth) {
            return Err(
                "VideoWriter: frame channels and bit depth must match the first frame".into(),
            );
        }
        let sample_bytes = usize::from(depth / 8);
        let scratch = pixels
            .checked_mul(if self.profile == Profile::Jpeg { 96 } else { 8 })
            .and_then(|n| n.checked_add(69632))
            .ok_or("VideoWriter: frame size overflow")?;
        admit(
            scratch
                .checked_add(self.bytes())
                .ok_or("VideoWriter: memory size overflow")?,
            budget(e),
        )?;
        let opened = self.sink.is_some();
        #[cfg(not(target_arch = "wasm32"))]
        let opened = opened || self.external.is_some();
        if !opened {
            if self.closed {
                return Err("VideoWriter: file is closed; call open before writing again".into());
            }
            self.open(budget(e))?;
        }
        for f in 0..frames {
            e.check_cancel()?;
            let available = budget(e);
            admit(scratch + self.bytes(), available)?;
            e.charge(scratch.div_ceil(8))?;
            let row = (w * out_channels).div_ceil(4) * 4;
            let len = if self.profile.external() {
                pixels * out_channels * sample_bytes
            } else if self.profile == Profile::Jpeg {
                pixels * 3
            } else {
                row * h
            };
            let mut data = Vec::new();
            data.try_reserve_exact(len)
                .map_err(|_| "VideoWriter: cannot allocate frame")?;
            data.resize(len, 0);
            let byte_input = matches!(a.class(), "uint8" | "uint16");
            for y in 0..h {
                if y.is_multiple_of(64) {
                    e.check_cancel()?;
                }
                for x in 0..w {
                    let base = f * pixels * channels + x * h + y;
                    let sample = number(a, base)?;
                    let p = if self.profile == Profile::Indexed {
                        Some(&self.palette)
                    } else {
                        frame_map.as_ref()
                    };
                    let index = if let (1, Some(p)) = (channels, p) {
                        let n = sample - if byte_input { 0. } else { 1. };
                        if !n.is_finite() || n.fract() != 0. || n < 0. || n >= p.len() as f64 {
                            return Err("VideoWriter: indexed pixels must be valid integer colormap indices (uint8 is zero based; floating point is one based)".into());
                        }
                        n as usize
                    } else {
                        0
                    };
                    for c in 0..out_channels {
                        let value = if self.profile == Profile::Indexed {
                            index as u16
                        } else if let (1, Some(p)) = (channels, p) {
                            u16::from(p[index][c]) * if depth == 16 { 257 } else { 1 }
                        } else {
                            let n = number(a, base + if channels == 3 { c * pixels } else { 0 })?;
                            if !n.is_finite() {
                                return Err("VideoWriter: frame values must be finite".into());
                            }
                            if byte_input {
                                (n as u16) & if depth == 16 { u16::MAX } else { 255 }
                            } else {
                                (n.clamp(0., 1.) * if depth == 16 { 65535. } else { 255. }).round()
                                    as u16
                            }
                        };
                        if self.profile.external() {
                            let offset = ((y * w + x) * out_channels + c) * sample_bytes;
                            if depth == 16 {
                                data[offset..offset + 2].copy_from_slice(&value.to_le_bytes());
                            } else {
                                data[offset] = value as u8;
                            }
                        } else if self.profile == Profile::Jpeg {
                            data[(y * w + x) * 3 + c] = value as u8;
                        } else {
                            data[(h - 1 - y) * row
                                + x * out_channels
                                + if out_channels == 3 { 2 - c } else { c }] = value as u8;
                        }
                    }
                }
            }
            if self.profile == Profile::Jpeg {
                data = super::jpeg_encode::encode(&data, w, h, self.quality, &e.cancel)?;
            }
            if self.profile.external() {
                #[cfg(not(target_arch = "wasm32"))]
                {
                    self.external.as_mut().unwrap().append(
                        &data,
                        ffmpeg::FrameSpec {
                            width: w,
                            height: h,
                            channels: out_channels,
                            depth,
                        },
                        &e.cancel,
                    )?;
                    self.frames += 1;
                    self.width = w;
                    self.height = h;
                    self.depth = depth;
                    self.channels = out_channels;
                    continue;
                }
            }
            let index_budget = available - scratch - self.bytes();
            let s = self.sink.as_mut().unwrap();
            if s.frames() == 0 {
                let gray: Vec<_>;
                let pal = if self.profile == Profile::Gray {
                    gray = (0..=255).map(|v| [v; 3]).collect();
                    &gray
                } else if self.profile == Profile::Indexed {
                    &self.palette
                } else {
                    &[][..]
                };
                s.configure(w, h, pal)?;
            }
            s.append(&data, index_budget)?;
            self.width = w;
            self.height = h;
            self.depth = depth;
            self.channels = out_channels;
            self.frames = s.frames();
        }
        Ok(())
    }
    fn write(&mut self, e: &mut Engine, a: &Array) -> Result<()> {
        if let Array::Struct(s) = a {
            if s.rows() != 1 || s.is_empty() {
                return Err("VideoWriter: movie must be a 1-by-F struct array".into());
            }
            let data = s
                .field_index("cdata")
                .ok_or("VideoWriter: movie requires cdata and colormap fields")?;
            let map = s
                .field_index("colormap")
                .ok_or("VideoWriter: movie requires cdata and colormap fields")?;
            for i in 0..s.len() {
                e.check_cancel()?;
                self.array(e, s.get(data, i)?, Some(s.get(map, i)?))?;
            }
            Ok(())
        } else {
            self.array(e, a, None)
        }
    }
}
impl Writer {
    fn state(&self) -> Result<MutexGuard<'_, State>> {
        self.0
            .lock()
            .map_err(|_| "VideoWriter: poisoned handle state".into())
    }
    pub(crate) fn new(e: &Engine, args: &[Value]) -> Result<Self> {
        if !(1..=2).contains(&args.len()) {
            return Err("VideoWriter requires a filename and optional profile".into());
        }
        let p = profile(&if args.len() == 2 {
            text(e, &args[1])?
        } else {
            PROFILES[0].into()
        })?;
        let tools = ffmpeg::tools(e)?;
        if p.external() && tools.ffmpeg.is_none() {
            return Err(ffmpeg::missing("VideoWriter", p.extension()));
        }
        let path = e.file_path(&text(e, &args[0])?, &format!(".{}", p.extension()))?;
        let parent = std::fs::canonicalize(path.parent().ok_or("VideoWriter: invalid filename")?)
            .map_err(|e| format!("VideoWriter: {e}"))?;
        let path = parent.join(path.file_name().ok_or("VideoWriter: invalid filename")?);
        super::file_io::check_output(&path)?;
        Ok(Self(Arc::new(Mutex::new(State {
            path,
            profile: p,
            rate: 30.,
            quality: 75,
            palette: Vec::new(),
            locked: false,
            closed: false,
            sink: None,
            #[cfg(not(target_arch = "wasm32"))]
            external: None,
            #[cfg(not(target_arch = "wasm32"))]
            tools,
            #[cfg(not(target_arch = "wasm32"))]
            cancel: e.cancellation(),
            lossless: p == Profile::Archival,
            ratio: 10.,
            requested_depth: None,
            depth: 8,
            channels: p.channels(),
            width: 0,
            height: 0,
            frames: 0,
        }))))
    }
    pub(crate) fn bytes(&self) -> usize {
        self.state().map_or(512, |s| s.bytes())
    }
    pub(crate) fn property_names(&self) -> Vec<String> {
        PROPS.iter().map(|s| (*s).into()).collect()
    }
    pub(crate) fn assign(&self, name: &str, value: Array) -> Result<()> {
        let mut s = self.state()?;
        let result = s.set(name, &value);
        if result.is_err() {
            if s.profile.external() {
                s.abort();
            } else {
                let _ = s.finish();
            }
        }
        result
    }
    pub(crate) fn property(&self, name: &str) -> Result<Value> {
        let s = self.state()?;
        let n = match name {
            "Filename" => {
                return Ok(Value::Text(
                    s.path
                        .file_name()
                        .unwrap_or_default()
                        .to_string_lossy()
                        .into(),
                ));
            }
            "Path" => {
                return Ok(Value::Text(
                    s.path.parent().unwrap().to_string_lossy().into(),
                ));
            }
            "FileFormat" => return Ok(Value::Text(s.profile.extension().into())),
            "LosslessCompression" | "CompressionRatio" | "MJ2BitDepth" => {
                if !s.profile.mj2() {
                    return Err(format!(
                        "VideoWriter: {name} applies only to Motion JPEG 2000"
                    ));
                }
                match name {
                    "LosslessCompression" => f64::from(s.lossless),
                    "CompressionRatio" => s.ratio,
                    _ => f64::from(s.requested_depth.unwrap_or(s.depth)),
                }
            }
            "VideoFormat" => {
                return Ok(Value::Text(
                    if s.profile.mj2() {
                        match (s.channels, s.depth) {
                            (1, 16) => "Mono16",
                            (1, _) => "Mono8",
                            (_, 16) => "RGB48",
                            _ => "RGB24",
                        }
                    } else {
                        s.profile.format()
                    }
                    .into(),
                ));
            }
            "VideoCompressionMethod" => return Ok(Value::Text(s.profile.compression().into())),
            "Colormap" => {
                if s.profile != Profile::Indexed {
                    return Err("VideoWriter: Colormap applies only to Indexed AVI".into());
                }
                let data = (0..3)
                    .flat_map(|c| s.palette.iter().map(move |p| f64::from(p[c]) / 255.))
                    .collect();
                return Ok(Value::Matrix(Matrix::from_column_vec(
                    s.palette.len(),
                    3,
                    data,
                )?));
            }
            "Quality" => {
                if !matches!(s.profile, Profile::Jpeg | Profile::Mpeg4) {
                    return Err(
                        "VideoWriter: Quality applies only to Motion JPEG AVI or MPEG-4".into(),
                    );
                }
                f64::from(s.quality)
            }
            "FrameRate" => s.rate,
            "ColorChannels" => s.channels as f64,
            "VideoBitsPerPixel" => (s.channels * usize::from(s.depth)) as f64,
            "Height" => {
                (if s.profile == Profile::Mpeg4 {
                    s.height.div_ceil(2) * 2
                } else {
                    s.height
                }) as f64
            }
            "Width" => {
                (if s.profile == Profile::Mpeg4 {
                    s.width.div_ceil(2) * 2
                } else {
                    s.width
                }) as f64
            }
            "FrameCount" => s.frames as f64,
            "Duration" => s.frames as f64 / s.rate,
            _ => {
                return Err(format!(
                    "No public property '{name}' exists for class VideoWriter."
                ));
            }
        };
        Ok(Value::Matrix(Matrix::scalar(n)))
    }
    pub(crate) fn call(&self, e: &mut Engine, name: &str, args: &[Value]) -> Result<Value> {
        let mut s = self.state()?;
        let result = (|| {
            e.check_cancel()?;
            match (name, args) {
                ("open", []) => s.open(budget(e)),
                ("close" | "delete", []) => s.finish(),
                ("writeVideo", [v]) => {
                    let a = v.clone().array(&e.cancel)?;
                    s.write(e, &a)
                }
                _ => Err(format!("VideoWriter: invalid arguments for {name}")),
            }
        })();
        match result {
            Ok(()) => Ok(Value::Void),
            Err(error) if s.profile.external() => {
                s.abort();
                Err(error)
            }
            Err(error) => match s.finish() {
                Ok(()) => Err(error),
                Err(close) => Err(format!("{error}; finalization failed: {close}")),
            },
        }
    }
    pub(crate) fn body(&self) -> String {
        self.state().map_or_else(|e| e, |s| format!("  VideoWriter with properties:\n\n    Filename: '{}'\n    FrameRate: {}\n    VideoFormat: '{}'\n    Width: {}\n    Height: {}\n    FrameCount: {}\n    Duration: {}\n", s.path.display(), s.rate, s.profile.format(), s.width, s.height, s.frames, s.frames as f64 / s.rate))
    }
}
impl Drop for State {
    fn drop(&mut self) {
        if self.profile.external() {
            self.abort();
        } else {
            let _ = self.finish();
        }
    }
}
pub(crate) fn profiles(e: &Engine) -> Result<Value> {
    let mut slots = Vec::new();
    let mut names = PROFILES.to_vec();
    if ffmpeg::tools(e)?.ffmpeg.is_some() {
        names.extend(["MPEG-4", "Motion JPEG 2000", "Archival"]);
    }
    let count = names.len();
    for name in names {
        let p = profile(name)?;
        let text = |s: &str| -> Result<Arc<Array>> {
            Ok(Arc::new(Array::Character(CharacterRow::from_units(
                s.encode_utf16().collect(),
                &e.cancel,
            )?)))
        };
        slots.extend([
            text(name)?,
            text(name)?,
            text(&format!(".{}", p.extension()))?,
            Arc::new(Array::Numeric(Matrix::scalar(p.channels() as f64))),
            Arc::new(Array::Numeric(Matrix::scalar(30.))),
            Arc::new(Array::Numeric(Matrix::scalar((p.channels() * 8) as f64))),
            text(p.compression())?,
            text(p.format())?,
        ]);
    }
    Ok(Value::from(Array::Struct(StructArray::from_slots(
        1,
        count,
        [
            "Name",
            "Description",
            "FileExtensions",
            "ColorChannels",
            "FrameRate",
            "VideoBitsPerPixel",
            "VideoCompressionMethod",
            "VideoFormat",
        ]
        .iter()
        .map(|s| (*s).into())
        .collect(),
        slots,
        &e.cancel,
    )?)))
}
