use super::Tools;
use crate::Result;
use crate::engine::video::{Metadata, VideoSource, admit, buffer};
use std::{
    ffi::OsString,
    io::{Read, Write},
    path::{Path, PathBuf},
    process::{Child, Command, ExitStatus, Stdio},
    sync::{
        Arc, Mutex, OnceLock,
        atomic::{AtomicBool, AtomicU64, Ordering},
        mpsc::{self, Receiver, SyncSender},
    },
    thread,
    time::{Duration, Instant},
};
const TIMEOUT: Duration = Duration::from_secs(10);
const POLL: Duration = Duration::from_millis(20);
const CHUNK: usize = 8192;
const STDERR: usize = 32 * 1024;
const JSON: usize = 1024 * 1024;
const MAX_FILE: u64 = 1 << 30;
fn file(path: &Path) -> OsString {
    debug_assert!(path.is_absolute());
    let mut name = OsString::from("file:");
    name.push(path);
    name
}
fn command(tool: &Path) -> Command {
    assert!(tool.is_absolute(), "ffmpeg path must be absolute");
    let mut c = Command::new(tool);
    c.args([
        "-nostdin",
        "-hide_banner",
        "-loglevel",
        "error",
        "-filter_threads",
        "1",
        "-filter_complex_threads",
        "1",
    ]);
    c
}
/// ffprobe has no -nostdin option: its stdin is still always /dev/null.
fn probe_command(tool: &Path) -> Command {
    assert!(tool.is_absolute(), "ffprobe path must be absolute");
    let mut c = Command::new(tool);
    c.args(["-hide_banner", "-loglevel", "error"]);
    c
}
fn receive<T>(rx: &Receiver<T>, cancel: &AtomicBool, start: Instant) -> Result<T> {
    loop {
        crate::dsp::check_cancel(cancel)?;
        if start.elapsed() >= TIMEOUT {
            return Err("ffmpeg: operation timed out after 10 seconds".into());
        }
        match rx.recv_timeout(POLL) {
            Ok(v) => return Ok(v),
            Err(mpsc::RecvTimeoutError::Timeout) => {}
            Err(_) => return Err("ffmpeg: pipe closed unexpectedly".into()),
        }
    }
}
fn worker(f: impl FnOnce() + Send + 'static) -> Result<thread::JoinHandle<()>> {
    thread::Builder::new()
        .name("video pipe".into())
        .spawn(f)
        .map_err(|e| format!("ffmpeg: cannot create pipe worker: {e}"))
}
struct Process {
    child: Child,
    error: Arc<Mutex<Vec<u8>>>,
    stderr: Option<thread::JoinHandle<()>>,
    stdout: Option<Receiver<std::io::Result<Vec<u8>>>>,
    input: Option<SyncSender<Vec<u8>>>,
    written: Option<Receiver<std::io::Result<()>>>,
    workers: Vec<thread::JoinHandle<()>>,
}
impl Process {
    fn spawn(mut c: Command, writing: bool) -> Result<Self> {
        c.stdin(if writing {
            Stdio::piped()
        } else {
            Stdio::null()
        })
        .stdout(if writing {
            Stdio::null()
        } else {
            Stdio::piped()
        })
        .stderr(Stdio::piped());
        let mut child = c
            .spawn()
            .map_err(|e| format!("ffmpeg: cannot start executable: {e}"))?;
        let mut stderr = child.stderr.take().unwrap();
        let error = Arc::new(Mutex::new(Vec::new()));
        let errors = error.clone();
        let mut s = Self {
            child,
            error,
            stderr: None,
            stdout: None,
            input: None,
            written: None,
            workers: Vec::new(),
        };
        s.stderr = Some(worker(move || {
            let mut b = [0; CHUNK];
            while let Ok(n) = stderr.read(&mut b) {
                if n == 0 {
                    break;
                }
                let mut out = errors.lock().unwrap_or_else(|p| p.into_inner());
                let keep = n.min(STDERR.saturating_sub(out.len()));
                out.extend_from_slice(&b[..keep]);
            }
        })?);
        if writing {
            let mut stdin = s.child.stdin.take().unwrap();
            let (tx, rx) = mpsc::sync_channel::<Vec<u8>>(1);
            let (done, ack) = mpsc::sync_channel(1);
            s.workers.push(worker(move || {
                while let Ok(b) = rx.recv() {
                    let result = stdin.write_all(&b);
                    let failed = result.is_err();
                    if done.send(result).is_err() || failed {
                        break;
                    }
                }
            })?);
            s.input = Some(tx);
            s.written = Some(ack);
        } else {
            let mut stdout = s.child.stdout.take().unwrap();
            let (tx, rx) = mpsc::sync_channel(1);
            s.workers.push(worker(move || {
                loop {
                    let mut b = vec![0; CHUNK];
                    let result = stdout.read(&mut b).map(|n| {
                        b.truncate(n);
                        b
                    });
                    let end = !result.as_ref().is_ok_and(|b| !b.is_empty());
                    if tx.send(result).is_err() || end {
                        break;
                    }
                }
            })?);
            s.stdout = Some(rx);
        }
        Ok(s)
    }
    fn detail(&self, error: impl std::fmt::Display) -> String {
        let b = self.error.lock().unwrap_or_else(|p| p.into_inner());
        format!(
            "ffmpeg: {error}: {}{}",
            String::from_utf8_lossy(&b).trim(),
            if b.len() == STDERR {
                " [stderr truncated]"
            } else {
                ""
            }
        )
    }
    fn stop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
        self.input.take();
        self.stdout.take();
        self.written.take();
        // Let the bounded diagnostic pump consume bytes already in the pipe.
        // A substituted executable's descendants must not delay teardown.
        let drain = Instant::now();
        while self.stderr.as_ref().is_some_and(|t| !t.is_finished())
            && drain.elapsed() < Duration::from_millis(100)
        {
            thread::sleep(Duration::from_millis(1));
        }
        if self.stderr.as_ref().is_some_and(|t| t.is_finished())
            && let Some(t) = self.stderr.take()
        {
            let _ = t.join();
        }
    }
    fn wait(&mut self, cancel: &AtomicBool) -> Result<ExitStatus> {
        self.input.take();
        let start = Instant::now();
        loop {
            if let Err(e) = crate::dsp::check_cancel(cancel) {
                self.stop();
                return Err(self.detail(e));
            }
            match self.child.try_wait() {
                Ok(Some(status)) => {
                    while self.stderr.as_ref().is_some_and(|t| !t.is_finished()) {
                        crate::dsp::check_cancel(cancel)?;
                        if start.elapsed() >= TIMEOUT {
                            self.stop();
                            return Err(self.detail("stderr timed out after 10 seconds"));
                        }
                        thread::sleep(POLL);
                    }
                    if let Some(t) = self.stderr.take() {
                        let _ = t.join();
                    }
                    return if status.success() {
                        Ok(status)
                    } else {
                        Err(self.detail(status))
                    };
                }
                Ok(None) if start.elapsed() < TIMEOUT => thread::sleep(POLL),
                Ok(None) => {
                    self.stop();
                    return Err(self.detail("operation timed out after 10 seconds"));
                }
                Err(e) => {
                    self.stop();
                    return Err(self.detail(e));
                }
            }
        }
    }
    fn chunk(&mut self, cancel: &AtomicBool, start: Instant) -> Result<Vec<u8>> {
        match receive(self.stdout.as_ref().unwrap(), cancel, start) {
            Ok(Ok(b)) => Ok(b),
            Ok(Err(e)) => {
                self.stop();
                Err(self.detail(e))
            }
            Err(e) => {
                self.stop();
                Err(self.detail(e))
            }
        }
    }
    fn write(&mut self, b: Vec<u8>, cancel: &AtomicBool) -> Result<()> {
        // At most one submitted packet; the previous write is always acknowledged.
        self.input
            .as_ref()
            .unwrap()
            .send(b)
            .map_err(|_| self.detail("stdin closed"))?;
        match receive(self.written.as_ref().unwrap(), cancel, Instant::now()) {
            Ok(Ok(())) => Ok(()),
            Ok(Err(e)) => {
                self.stop();
                Err(self.detail(e))
            }
            Err(e) => {
                self.stop();
                Err(self.detail(e))
            }
        }
    }
}
impl Drop for Process {
    fn drop(&mut self) {
        self.stop();
        // A substituted executable may leave descendants holding pipes. Never
        // block engine teardown waiting for those unrelated pipe holders.
        for t in self.workers.drain(..).chain(self.stderr.take()) {
            if t.is_finished() {
                let _ = t.join();
            }
        }
    }
}
fn capture(c: Command, cap: usize, cancel: &AtomicBool) -> Result<Vec<u8>> {
    let mut p = Process::spawn(c, false)?;
    let start = Instant::now();
    let mut out = Vec::new();
    loop {
        let b = p.chunk(cancel, start)?;
        if b.is_empty() {
            break;
        }
        if out.len() + b.len() > cap {
            p.stop();
            return Err(p.detail("output exceeds capture limit"));
        }
        out.extend_from_slice(&b);
    }
    p.wait(cancel)?;
    Ok(out)
}
fn probe(t: &Tools, path: &Path, count: bool, cancel: &AtomicBool) -> Result<serde_json::Value> {
    let mut c = probe_command(t.ffprobe.as_ref().unwrap());
    c.args([
        "-threads",
        "1",
        "-protocol_whitelist",
        "file",
        "-of",
        "json",
        "-show_streams",
        "-show_format",
    ]);
    if count {
        c.arg("-count_frames");
    }
    c.arg(file(path));
    serde_json::from_slice(&capture(c, JSON, cancel)?)
        .map_err(|e| format!("VideoReader: invalid ffprobe JSON: {e}"))
}
fn video(json: &serde_json::Value) -> Result<&serde_json::Value> {
    json["streams"]
        .as_array()
        .and_then(|v| v.iter().find(|s| s["codec_type"] == "video"))
        .ok_or("VideoReader: no video stream".into())
}
fn num(v: &serde_json::Value) -> Option<f64> {
    v.as_f64().or_else(|| v.as_str()?.parse().ok())
}
fn rational(v: &serde_json::Value) -> Option<f64> {
    let (a, b) = v.as_str()?.split_once('/')?;
    let r = a.parse::<f64>().ok()? / b.parse::<f64>().ok()?;
    (r.is_finite() && r > 0.).then_some(r)
}
/// Frame PTS (rather than packet DTS or average rate) follow presentation order,
/// including codecs with reordered frames. An incomplete or ambiguous index is
/// never used: restart at the beginning and count decoded frames instead.
fn timestamps(source: &Source, budget: usize, cancel: &AtomicBool) -> Result<Option<Vec<f64>>> {
    let attempt = (|| {
        // The caller grants a per-frame scratch budget, not the whole engine
        // budget. Reserve the retained table and pipe storage first, then bound
        // JSON plus its parsed representation by the remaining allowance.
        let retained = source.meta.frames * size_of::<f64>() + STDERR + 4 * CHUNK;
        let cap = (budget.saturating_sub(retained) / 16).min(JSON);
        if cap == 0 {
            return Err("timestamp index exceeds scratch limit".into());
        }
        let mut c = probe_command(source.tools.ffprobe.as_ref().unwrap());
        c.args([
            "-threads",
            "1",
            "-protocol_whitelist",
            "file",
            "-select_streams",
            "v:0",
            "-show_frames",
            "-show_entries",
            "frame=pts_time",
            "-of",
            "json",
        ])
        .arg(file(&source.path));
        let bytes = capture(c, cap, cancel)?;
        let json: serde_json::Value = serde_json::from_slice(&bytes).map_err(|e| e.to_string())?;
        let frames = json["frames"]
            .as_array()
            .ok_or("missing frame timestamps")?;
        if frames.len() != source.meta.frames {
            return Err("timestamp count mismatch".into());
        }
        let mut pts = Vec::new();
        pts.try_reserve_exact(frames.len())
            .map_err(|e| e.to_string())?;
        for frame in frames {
            crate::dsp::check_cancel(cancel)?;
            let t = num(&frame["pts_time"]).ok_or("missing frame timestamp")?;
            // Negative times cannot be sought reliably by every demuxer.
            if !t.is_finite() || t < 0. {
                return Err("unusable frame timestamp".into());
            }
            pts.push(t);
        }
        // ffprobe emits decoded frames in presentation order. Reordering a
        // nonmonotonic stream here would change its sequential ordinal order.
        // Require an already sorted, strictly increasing seek table instead.
        if pts.windows(2).any(|w| w[0] >= w[1]) {
            return Err("ambiguous frame timestamps".into());
        }
        Ok::<_, String>(pts)
    })();
    // Cancellation must propagate instead of starting a fallback decoder.
    crate::dsp::check_cancel(cancel)?;
    Ok(attempt.ok())
}
struct Stream {
    process: Process,
    next: usize,
    pending: Vec<u8>,
    offset: usize,
}
pub(in crate::engine::video) struct Source {
    tools: Arc<Tools>,
    path: PathBuf,
    meta: Metadata,
    stream: Mutex<Option<Stream>>,
    timestamps: OnceLock<Option<Vec<f64>>>,
}
impl Source {
    pub(in crate::engine::video) fn open(
        tools: Arc<Tools>,
        path: &Path,
        budget: usize,
        cancel: &AtomicBool,
    ) -> Result<Self> {
        admit(JSON * 16 + 65536, budget)?;
        let mut json = probe(&tools, path, false, cancel)?;
        let s = video(&json)?;
        let dim = |k: &str| -> Result<usize> {
            let n = s[k].as_u64().ok_or("VideoReader: missing dimensions")?;
            if !(1..=32767).contains(&n) {
                return Err("VideoReader: invalid or absurd dimensions".into());
            }
            Ok(n as usize)
        };
        let (width, height) = (dim("width")?, dim("height")?);
        admit(
            width
                .checked_mul(height)
                .and_then(|n| n.checked_mul(64))
                .ok_or("VideoReader: dimensions overflow")?,
            budget,
        )?;
        let rate = rational(&s["avg_frame_rate"])
            .or_else(|| rational(&s["r_frame_rate"]))
            .ok_or("VideoReader: invalid frame rate")?;
        let gray = s["pix_fmt"]
            .as_str()
            .is_some_and(|p| p.starts_with("gray") || p.starts_with("mono") || p.starts_with("ya"));
        let duration = num(&s["duration"]).or_else(|| num(&json["format"]["duration"]));
        let mut frames = num(&s["nb_frames"]);
        if frames.is_none() {
            json = probe(&tools, path, true, cancel)?;
            frames = num(&video(&json)?["nb_read_frames"]);
        }
        let n = frames.ok_or("VideoReader: ffprobe could not count frames")?;
        if !n.is_finite() || n.fract() != 0. || !(1. ..=1_000_000.).contains(&n) {
            return Err("VideoReader: invalid or excessive frame count".into());
        }
        Ok(Self {
            tools,
            path: path.into(),
            meta: Metadata {
                width,
                height,
                rate,
                frames: n as usize,
                bits: if gray { 8 } else { 24 },
                format: if gray { "Grayscale" } else { "RGB24" },
                palette: Vec::new(),
                duration: duration.filter(|v| v.is_finite() && *v >= 0.),
            },
            stream: Mutex::new(None),
            timestamps: OnceLock::new(),
        })
    }
}
impl VideoSource for Source {
    fn metadata(&self) -> &Metadata {
        &self.meta
    }
    fn bytes(&self) -> usize {
        self.path.as_os_str().len()
            + STDERR
            + 4 * CHUNK
            + 1024
            + self
                .timestamps
                .get()
                .and_then(Option::as_ref)
                .map_or(0, |p| p.capacity() * size_of::<f64>())
    }
    fn reset(&self) {
        if let Ok(mut s) = self.stream.lock() {
            *s = None;
        }
    }
    fn frame(&self, i: usize, budget: usize, cancel: &AtomicBool) -> Result<Vec<u8>> {
        let mut slot = self
            .stream
            .lock()
            .map_err(|_| "VideoReader: poisoned stream")?;
        let result = (|| {
            crate::dsp::check_cancel(cancel)?;
            if i >= self.meta.frames {
                return Err("VideoReader: frame index out of range".into());
            }
            let len = self.meta.width * self.meta.height * if self.meta.bits == 8 { 1 } else { 3 };
            let index_bytes = self
                .timestamps
                .get()
                .and_then(Option::as_ref)
                .map_or(0, |p| p.capacity() * size_of::<f64>());
            admit(len + STDERR + 4 * CHUNK + index_bytes, budget)?;
            let mut out = buffer(len, budget)?;
            if slot.as_ref().is_none_or(|s| s.next != i) {
                *slot = None;
                // The shared source has no separate indexed-read interface. Build
                // once on the first nonzero seek; reads from frame zero need no index.
                if i != 0 && self.timestamps.get().is_none() {
                    let table = timestamps(self, budget.saturating_sub(len), cancel)?;
                    let _ = self.timestamps.set(table);
                }
                let timestamp = self.timestamps.get().and_then(Option::as_ref).map(|p| p[i]);
                let mut c = command(self.tools.ffmpeg.as_ref().unwrap());
                c.args(["-protocol_whitelist", "file"]);
                if i != 0
                    && let Some(t) = timestamp
                {
                    // Absolute stream PTS, including streams whose start_time is nonzero.
                    c.args([
                        "-seek_timestamp",
                        "1",
                        "-ss",
                        &format!("{t:.6}"),
                        "-accurate_seek",
                    ]);
                }
                c.args(["-noautorotate", "-threads", "1", "-i"]);
                c.arg(file(&self.path)).args([
                    "-map",
                    "0:v:0",
                    "-an",
                    "-sn",
                    "-dn",
                    "-f",
                    "rawvideo",
                    "-pix_fmt",
                    if self.meta.bits == 8 { "gray" } else { "rgb24" },
                    "-fps_mode",
                    "passthrough",
                    "-threads",
                    "1",
                    "pipe:1",
                ]);
                *slot = Some(Stream {
                    process: Process::spawn(c, false)?,
                    next: if i != 0 && timestamp.is_some() { i } else { 0 },
                    pending: Vec::new(),
                    offset: 0,
                });
            }
            let s = slot.as_mut().unwrap();
            // Fallback skips whole decoded frames with constant scratch space.
            while s.next <= i {
                crate::dsp::check_cancel(cancel)?;
                let mut filled = 0;
                let start = Instant::now();
                while filled < len {
                    if s.offset == s.pending.len() {
                        s.pending = s.process.chunk(cancel, start)?;
                        s.offset = 0;
                        if s.pending.is_empty() {
                            s.process.wait(cancel)?;
                            return Err(s.process.detail("truncated rawvideo frame"));
                        }
                    }
                    let n = (len - filled).min(s.pending.len() - s.offset);
                    if s.next == i {
                        out[filled..filled + n].copy_from_slice(&s.pending[s.offset..s.offset + n]);
                    }
                    s.offset += n;
                    filled += n;
                }
                s.next += 1;
            }
            Ok(out)
        })();
        if result.is_err() || i + 1 == self.meta.frames {
            *slot = None;
        }
        result
    }
}
static NEXT: AtomicU64 = AtomicU64::new(0);
#[derive(Clone, Copy)]
pub(in crate::engine::video) struct FrameSpec {
    pub width: usize,
    pub height: usize,
    pub channels: usize,
    pub depth: u8,
}
impl FrameSpec {
    fn pixel_format(self) -> &'static str {
        match (self.channels, self.depth) {
            (1, 8) => "gray",
            (1, 16) => "gray16le",
            (3, 16) => "rgb48le",
            _ => "rgb24",
        }
    }
}
pub(in crate::engine::video) struct Config {
    pub rate: f64,
    pub mp4: bool,
    pub quality: u8,
    pub lossless: bool,
    pub ratio: f64,
}
pub(in crate::engine::video) struct Sink {
    tools: Arc<Tools>,
    target: PathBuf,
    temp: PathBuf,
    process: Option<Process>,
    pub frames: usize,
    rate: f64,
    codec: String,
    mp4: bool,
    quality: u8,
    lossless: bool,
    ratio: f64,
}
impl Sink {
    pub(in crate::engine::video) fn new(
        tools: Arc<Tools>,
        target: &Path,
        config: Config,
        cancel: &AtomicBool,
    ) -> Result<Self> {
        let Config {
            rate,
            mp4,
            quality,
            lossless,
            ratio,
        } = config;
        let mut c = command(tools.ffmpeg.as_ref().unwrap());
        c.arg("-encoders");
        let enc = String::from_utf8_lossy(&capture(c, JSON, cancel)?).into_owned();
        let has = |name: &str| {
            enc.lines()
                .any(|l| l.split_whitespace().nth(1) == Some(name))
        };
        let codec = if mp4 {
            if has("libx264") {
                "libx264"
            } else if has("mpeg4") {
                "mpeg4"
            } else {
                return Err("VideoWriter: ffmpeg has no libx264 or mpeg4 encoder".into());
            }
        } else if has("libopenjpeg") {
            "libopenjpeg"
        } else if has("jpeg2000") {
            "jpeg2000"
        } else {
            return Err(
                "VideoWriter: ffmpeg has no JPEG 2000 encoder (libopenjpeg or jpeg2000)".into(),
            );
        };
        for _ in 0..100 {
            let temp = target.parent().unwrap().join(format!(
                ".matrust-video-{}-ff-{}.tmp",
                std::process::id(),
                NEXT.fetch_add(1, Ordering::Relaxed)
            ));
            match std::fs::OpenOptions::new()
                .write(true)
                .create_new(true)
                .open(&temp)
            {
                Ok(_) => {
                    return Ok(Self {
                        tools,
                        target: target.into(),
                        temp,
                        process: None,
                        frames: 0,
                        rate,
                        codec: codec.into(),
                        mp4,
                        quality,
                        lossless,
                        ratio,
                    });
                }
                Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => continue,
                Err(e) => return Err(format!("VideoWriter: {e}")),
            }
        }
        Err("VideoWriter: cannot create temporary file".into())
    }
    pub(in crate::engine::video) fn append(
        &mut self,
        data: &[u8],
        spec: FrameSpec,
        cancel: &AtomicBool,
    ) -> Result<()> {
        let FrameSpec {
            width: w,
            height: h,
            ..
        } = spec;
        if self.frames >= 1_000_000 {
            return Err("VideoWriter: frame count limit exceeded".into());
        }
        if self.process.is_none() {
            let mut c = command(self.tools.ffmpeg.as_ref().unwrap());
            c.args([
                "-f",
                "rawvideo",
                "-pix_fmt",
                spec.pixel_format(),
                "-video_size",
                &format!("{w}x{h}"),
                "-framerate",
                &self.rate.to_string(),
                "-i",
                "pipe:0",
                "-an",
                "-c:v",
                &self.codec,
                "-threads",
                "1",
            ]);
            if self.mp4 {
                c.args([
                    "-vf",
                    "pad=ceil(iw/2)*2:ceil(ih/2)*2",
                    "-pix_fmt",
                    "yuv420p",
                ]);
                if self.codec == "libx264" {
                    c.args(["-tune", "zerolatency", "-refs", "1", "-bf", "0"]);
                    c.args([
                        "-crf",
                        &format!("{:.2}", 35. - f64::from(self.quality) * 0.3),
                    ]);
                } else {
                    c.args([
                        "-q:v",
                        &format!("{:.2}", 31. - f64::from(self.quality) * 0.29),
                    ]);
                }
                c.args(["-f", "mp4"]);
            } else {
                c.args(["-pix_fmt", spec.pixel_format()]);
                if self.codec == "libopenjpeg" {
                    c.args([
                        "-irreversible",
                        if self.lossless { "0" } else { "1" },
                        "-numresolution",
                        "1",
                    ]);
                    if !self.lossless {
                        c.args([
                            "-compression_level",
                            &((self.ratio / 2.).ceil() as u32).to_string(),
                        ]);
                    }
                } else {
                    c.args(["-pred", if self.lossless { "1" } else { "0" }]);
                    if !self.lossless {
                        c.args(["-layer_rates", &self.ratio.to_string()]);
                    }
                }
                // MOV muxer selects the Motion JPEG 2000 brand for JPEG 2000.
                c.args(["-f", "mov", "-brand", "mjp2"]);
            }
            c.arg("-y").arg(file(&self.temp));
            self.process = Some(Process::spawn(c, true)?);
        }
        // Bounded chunks keep pipe memory independent of frame dimensions.
        for chunk in data.chunks(CHUNK) {
            self.process
                .as_mut()
                .unwrap()
                .write(chunk.to_vec(), cancel)?;
        }
        self.frames += 1;
        self.check_size()?;
        Ok(())
    }
    pub(in crate::engine::video) fn finish(&mut self, cancel: &AtomicBool) -> Result<()> {
        if let Some(mut p) = self.process.take() {
            p.wait(cancel)?;
        }
        if self.frames > 0 {
            self.check_size()?;
            std::fs::OpenOptions::new()
                .write(true)
                .open(&self.temp)
                .and_then(|f| f.sync_all())
                .map_err(|e| format!("VideoWriter: final sync: {e}"))?;
            std::fs::rename(&self.temp, &self.target)
                .map_err(|e| format!("VideoWriter: final rename: {e}"))?;
        } else {
            let _ = std::fs::remove_file(&self.temp);
        }
        self.frames = 0;
        Ok(())
    }
    fn check_size(&self) -> Result<()> {
        let size = std::fs::metadata(&self.temp)
            .map_err(|e| format!("VideoWriter: {e}"))?
            .len();
        if size > MAX_FILE {
            Err("VideoWriter: ffmpeg output exceeds the 1 GiB file limit".into())
        } else {
            Ok(())
        }
    }
    pub(in crate::engine::video) fn bytes(&self) -> usize {
        STDERR + 4 * CHUNK + 1024
    }
}
impl Drop for Sink {
    fn drop(&mut self) {
        self.process.take();
        let _ = std::fs::remove_file(&self.temp);
    }
}
