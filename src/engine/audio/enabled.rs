use super::native;
use crate::engine::{Engine, Execution, Value, limits};
use crate::{Array, LogicalMatrix, Matrix, Result};
use std::ops::Range;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering::Relaxed};
use std::sync::{Arc, Mutex};
use std::thread::{self, Thread};
use std::time::{Duration, Instant};

const DEFAULT_RATE: u32 = 8192;
const MAX_RATE: u32 = 384_000;
const DEFAULT_RECORD_BYTES: usize = 32 * 1024 * 1024;
const DEVICE_OVERRIDE: &str = "MATRUST_AUDIO_DEVICE";
const POLL: Duration = Duration::from_millis(5);
const PLAYER_PROPERTIES: &[&str] = &[
    "SampleRate",
    "BitsPerSample",
    "NumChannels",
    "DeviceID",
    "TotalSamples",
    "CurrentSample",
    "Running",
    "Tag",
    "UserData",
];
const RECORDER_PROPERTIES: &[&str] = &[
    "SampleRate",
    "BitsPerSample",
    "NumChannels",
    "TotalSamples",
    "CurrentSample",
    "Running",
    "Tag",
    "UserData",
];

#[derive(Clone, Copy)]
pub(super) struct Format {
    pub rate: u32,
    pub bits: u8,
    pub channels: usize,
    pub device: i32,
}

#[derive(Clone, Copy)]
enum Scale {
    Direct,
    Signed(f64),
    Unsigned8,
    Range { low: f64, high: f64 },
    Silence,
}

#[derive(Clone)]
pub(super) struct Clip {
    // Keep the source in shared column-major storage. Conversion happens in the callback.
    source: Array,
    pub frames: usize,
    pub channels: usize,
    scale: Scale,
}

fn real_at(array: &Array, index: usize) -> f64 {
    match array {
        Array::Numeric(m) => m.data()[index],
        Array::Typed(t) => t.real(index),
        _ => unreachable!("audio arguments are validated first"),
    }
}

fn real_array(engine: &Engine, value: &Value) -> Result<Array> {
    let array = value.clone().array(&engine.cancel)?;
    if !matches!(array, Array::Numeric(_) | Array::Typed(_)) || array.is_complex() || array.is_nd()
    {
        return Err("Audio data and parameters must be real numeric arrays.".into());
    }
    Ok(array)
}

fn integer(value: f64, min: f64, max: f64, error: &str) -> Result<f64> {
    if (min..=max).contains(&value) && value.fract() == 0. {
        Ok(value)
    } else {
        Err(error.into())
    }
}

fn scalar(array: &Array) -> Result<f64> {
    if array.len() != 1 {
        return Err("Expected a real numeric scalar.".into());
    }
    Ok(real_at(array, 0))
}

fn rate(value: f64) -> Result<u32> {
    integer(
        value,
        1.,
        MAX_RATE.into(),
        "SampleRate must be an integer from 1 to 384000 Hz.",
    )
    .map(|n| n as u32)
}

fn bits(value: f64) -> Result<u8> {
    match value {
        8. | 16. | 24. => Ok(value as u8),
        _ => Err("BitsPerSample must be 8, 16, or 24.".into()),
    }
}

fn vector_pair(array: &Array) -> bool {
    (array.rows() == 1 || array.cols() == 1) && array.len() == 2
}

impl Clip {
    fn new(
        engine: &mut Engine,
        source: Array,
        scaled: bool,
        range: Option<&Array>,
    ) -> Result<Self> {
        let (rows, cols) = (source.rows(), source.cols());
        let (frames, channels) = match (rows, cols) {
            (1, _) | (_, 1) | (0, 0) => (source.len(), 1),
            (_, 2) => (rows, 2),
            _ => return Err("Audio data must be a mono vector or an N-by-2 stereo matrix.".into()),
        };
        if source.retained_bytes() > limits::memory_bytes(&engine.limits) {
            return Err(limits::memory_error("Audio clip", &engine.limits));
        }
        let scale = if scaled {
            let (low, high) = if let Some(range) = range {
                if !vector_pair(range) {
                    return Err("soundsc: range must be a two-element vector.".into());
                }
                let (low, high) = (real_at(range, 0), real_at(range, 1));
                if !low.is_finite() || !high.is_finite() || low >= high {
                    return Err("soundsc: range must contain two increasing finite values.".into());
                }
                (low, high)
            } else {
                engine.charge(source.len())?;
                let (mut low, mut high) = (f64::INFINITY, f64::NEG_INFINITY);
                for index in 0..source.len() {
                    if index.is_multiple_of(16_384) {
                        engine.check_cancel()?;
                    }
                    let sample = real_at(&source, index);
                    if sample.is_finite() {
                        low = low.min(sample);
                        high = high.max(sample);
                    }
                }
                (low, high)
            };
            if low < high {
                Scale::Range { low, high }
            } else {
                Scale::Silence
            }
        } else {
            match source.class() {
                "int8" => Scale::Signed(128.),
                "int16" => Scale::Signed(32768.),
                "uint8" => Scale::Unsigned8,
                "single" | "double" => Scale::Direct,
                _ => {
                    return Err(
                        "Audio samples must be double, single, int8, int16, or uint8.".into(),
                    );
                }
            }
        };
        Ok(Self {
            source,
            frames,
            channels,
            scale,
        })
    }

    pub fn sample(&self, frame: usize, channel: usize) -> f32 {
        let value = real_at(&self.source, frame + channel * self.frames);
        let value = match self.scale {
            Scale::Direct => value,
            Scale::Signed(divisor) => value / divisor,
            Scale::Unsigned8 => (value - 128.) / 128.,
            Scale::Range { low, high } => {
                let value = value.clamp(low, high);
                let span = high - low;
                if span.is_infinite() {
                    // Halve before subtracting so opposite, near-maximum endpoints fit.
                    2. * ((value * 0.5 - low * 0.5) / (high * 0.5 - low * 0.5)) - 1.
                } else {
                    2. * ((value - low) / span) - 1.
                }
            }
            Scale::Silence => 0.,
        };
        if value.is_nan() {
            0.
        } else {
            value.clamp(-1., 1.) as f32
        }
    }
}

#[derive(Clone, Default)]
pub(in crate::engine) struct Session {
    silent_warning: bool,
    no_output_device: bool,
    no_input_device: bool,
    null_streams: Arc<AtomicUsize>,
    sound: Option<(Clip, Arc<Playback>)>,
}

impl Session {
    pub(in crate::engine) fn bytes(&self) -> usize {
        self.sound
            .as_ref()
            .map_or(0, |(clip, _)| clip.source.retained_bytes())
    }
}

/// Returns the normalized sample submitted to the audio backend, before PCM conversion.
#[doc(hidden)]
pub fn audio_output_sample(engine: &Engine, frame: usize, channel: usize) -> Option<f32> {
    let (clip, _) = engine.audio.sound.as_ref()?;
    (frame < clip.frames && channel < clip.channels).then(|| clip.sample(frame, channel))
}

fn force_null(engine: &Engine) -> bool {
    engine
        .envx_child_variables()
        .get(DEVICE_OVERRIDE)
        .map_or_else(
            || std::env::var_os(DEVICE_OVERRIDE).is_some_and(|s| s == "null"),
            |value| value == "null",
        )
}

fn no_output(engine: &mut Engine, out: &mut Execution) -> Result<()> {
    if !engine.audio.silent_warning {
        engine.lng_warn(native::NO_OUTPUT, "MatRust:audio:NoOutputDevice", out)?;
        engine.audio.silent_warning = true;
    }
    Ok(())
}

pub(in crate::engine) fn sound(
    engine: &mut Engine,
    args: &[Value],
    scaled: bool,
    out: &mut Execution,
) -> Result<Value> {
    let max = if scaled { 4 } else { 3 };
    if args.is_empty() || args.len() > max {
        return Err("Invalid number of inputs for sound playback.".into());
    }
    let mut parameters = args[1..]
        .iter()
        .map(|arg| real_array(engine, arg))
        .collect::<Result<Vec<_>>>()?;
    let range = if scaled && parameters.last().is_some_and(|a| a.len() != 1) {
        parameters.pop()
    } else {
        None
    };
    if parameters.len() > 2 {
        return Err("Invalid number of inputs for sound playback.".into());
    }
    let sample_rate = parameters
        .first()
        .map_or(Ok(DEFAULT_RATE), |a| rate(scalar(a)?))?;
    let depth = parameters.get(1).map_or(Ok(16), |a| bits(scalar(a)?))?;
    let clip = Clip::new(
        engine,
        real_array(engine, &args[0])?,
        scaled,
        range.as_ref(),
    )?;
    let format = Format {
        rate: sample_rate,
        bits: depth,
        channels: clip.channels,
        device: -1,
    };
    if engine
        .workspace_bytes()
        .saturating_sub(engine.audio.bytes())
        .saturating_add(clip.source.retained_bytes())
        > limits::memory_bytes(&engine.limits)
    {
        return Err(limits::workspace_error(&engine.limits));
    }
    // Consecutive calls replace standalone playback. Players have independent streams.
    engine.audio.sound = None;
    let playback = start_playback(engine, &clip, format, 0..clip.frames, out)?;
    engine.audio.sound = Some((clip, Arc::new(playback)));
    Ok(Value::Void)
}

#[derive(Default)]
enum Playback {
    #[default]
    Stopped,
    Null(NullPlayback),
    Device(native::Output),
}

struct NullSlot {
    counter: Arc<AtomicUsize>,
    released: AtomicBool,
}

impl NullSlot {
    fn acquire(counter: &Arc<AtomicUsize>) -> Result<Self> {
        counter
            .try_update(Relaxed, Relaxed, |count| {
                (count < native::MAX_STREAMS).then_some(count + 1)
            })
            .map_err(|_| "Too many simultaneous audio streams.")?;
        Ok(Self {
            counter: counter.clone(),
            released: AtomicBool::new(false),
        })
    }

    fn release(&self) {
        if !self.released.swap(true, Relaxed) {
            self.counter.fetch_sub(1, Relaxed);
        }
    }
}

impl Drop for NullSlot {
    fn drop(&mut self) {
        self.release();
    }
}

struct NullClock {
    range: Range<usize>,
    rate: u32,
    elapsed: Duration,
    started: Option<Instant>,
    slot: NullSlot,
    stopped: bool,
}

impl NullClock {
    fn played(&self) -> Duration {
        self.elapsed + self.started.map_or(Duration::ZERO, |t| t.elapsed())
    }

    fn offset(&self) -> usize {
        (self.played().as_secs_f64() * f64::from(self.rate)) as usize
    }

    fn ended(&self) -> bool {
        self.offset() >= self.range.len()
    }
}

struct NullPlayback {
    clock: Arc<Mutex<NullClock>>,
    thread: Thread,
}

impl NullPlayback {
    fn status(&self) -> (bool, usize) {
        let clock = self.clock.lock().unwrap();
        if clock.ended() {
            clock.slot.release();
            (false, 1)
        } else {
            (
                clock.started.is_some(),
                clock.range.start + clock.offset() + 1,
            )
        }
    }

    fn pause(&mut self) {
        let mut clock = self.clock.lock().unwrap();
        if let Some(started) = clock.started.take() {
            clock.elapsed += started.elapsed();
        }
        drop(clock);
        self.thread.unpark();
    }

    fn resume(&mut self) {
        let mut clock = self.clock.lock().unwrap();
        if clock.started.is_none() && !clock.ended() {
            clock.started = Some(Instant::now());
        }
        drop(clock);
        self.thread.unpark();
    }
}

impl Drop for NullPlayback {
    fn drop(&mut self) {
        {
            let mut clock = self.clock.lock().unwrap();
            clock.stopped = true;
            clock.slot.release();
        }
        self.thread.unpark();
    }
}

// The slot has to come back when the clip's own time runs out. Handles that
// stay reachable are not dropped, and nothing may be polling status.
fn watch_null(clock: Arc<Mutex<NullClock>>) {
    loop {
        let wait = {
            let guard = clock.lock().unwrap();
            if guard.stopped {
                return;
            }
            if guard.ended() {
                guard.slot.release();
                return;
            }
            if guard.started.is_none() {
                None
            } else {
                let left = guard.range.len() - guard.offset();
                Some(Duration::from_secs_f64(left as f64 / f64::from(guard.rate)))
            }
        };
        match wait {
            Some(left) => thread::park_timeout(left),
            None => thread::park(),
        }
    }
}

impl Playback {
    fn status(&self) -> (bool, usize) {
        match self {
            Self::Stopped => (false, 1),
            Self::Null(clock) => clock.status(),
            Self::Device(device) => device.status(),
        }
    }

    fn pause(&mut self) {
        match self {
            Self::Stopped => (),
            Self::Null(clock) => clock.pause(),
            Self::Device(device) => device.pause(true),
        }
    }

    fn resume(&mut self) {
        match self {
            Self::Stopped => (),
            Self::Null(clock) => clock.resume(),
            Self::Device(device) => device.pause(false),
        }
    }
}

fn start_playback(
    engine: &mut Engine,
    clip: &Clip,
    format: Format,
    range: Range<usize>,
    out: &mut Execution,
) -> Result<Playback> {
    if range.is_empty() {
        return Ok(Playback::Stopped);
    }
    if !force_null(engine) && !engine.audio.no_output_device {
        match native::Output::start(engine, clip.clone(), format, range.clone()) {
            Ok(output) => return Ok(Playback::Device(output)),
            // Only a missing device is remembered. A start timeout, a build or
            // play failure, and a missing DeviceID keep their own errors.
            Err(error) if error == native::NO_OUTPUT => {
                engine.audio.no_output_device = true;
            }
            Err(error) => return Err(error),
        }
    }
    let slot = NullSlot::acquire(&engine.audio.null_streams)?;
    no_output(engine, out)?;
    let clock = Arc::new(Mutex::new(NullClock {
        range,
        rate: format.rate,
        elapsed: Duration::ZERO,
        started: Some(Instant::now()),
        slot,
        stopped: false,
    }));
    let watched = Arc::clone(&clock);
    let handle = thread::Builder::new()
        .name("matrust-audio-null".into())
        .spawn(move || watch_null(watched))
        .map_err(|error| format!("Could not start audio worker: {error}"))?;
    Ok(Playback::Null(NullPlayback {
        thread: handle.thread().clone(),
        clock,
    }))
}

#[derive(Clone)]
enum Kind {
    Player {
        clip: Clip,
        playback: Arc<Mutex<Playback>>,
    },
    Recorder(Option<Arc<native::Input>>),
}

#[derive(Clone)]
pub(in crate::engine) struct Handle {
    kind: Kind,
    format: Format,
    tag: Arc<str>,
    pub user: Arc<Array>,
}

impl Handle {
    fn new(kind: Kind, format: Format) -> Result<Self> {
        Ok(Self {
            kind,
            format,
            tag: Arc::from(""),
            user: Arc::new(Array::Numeric(Matrix::filled(0, 0, 0.)?)),
        })
    }

    fn from_clip(clip: Clip, mut format: Format) -> Result<Self> {
        format.channels = clip.channels;
        Self::new(
            Kind::Player {
                clip,
                playback: Arc::new(Mutex::new(Playback::Stopped)),
            },
            format,
        )
    }

    pub fn player(engine: &mut Engine, args: &[Value]) -> Result<Self> {
        if !(2..=4).contains(&args.len()) {
            return Err("audioplayer requires audio data, sample rate, and optional bit depth and device ID.".into());
        }
        let arg = |i| scalar(&real_array(engine, &args[i])?);
        let rate = rate(arg(1)?)?;
        let source = real_array(engine, &args[0])?;
        let bits = if args.len() >= 3 {
            bits(arg(2)?)?
        } else if matches!(source.class(), "int8" | "uint8") {
            8
        } else {
            16
        };
        let device = if args.len() == 4 {
            integer(
                arg(3)?,
                -1.,
                i32::MAX.into(),
                "DeviceID must be -1 or a nonnegative integer.",
            )? as i32
        } else {
            -1
        };
        let clip = Clip::new(engine, source, false, None)?;
        Self::from_clip(
            clip,
            Format {
                rate,
                bits,
                channels: 1,
                device,
            },
        )
    }

    pub fn recorder(engine: &Engine, args: &[Value]) -> Result<Self> {
        let format = match args {
            [] => Format {
                rate: 8000,
                bits: 8,
                channels: 1,
                device: -1,
            },
            [fs, depth, channels] => Format {
                rate: rate(scalar(&real_array(engine, fs)?)?)?,
                bits: bits(scalar(&real_array(engine, depth)?)?)?,
                channels: integer(
                    scalar(&real_array(engine, channels)?)?,
                    1.,
                    2.,
                    "NumChannels must be 1 or 2.",
                )? as usize,
                device: -1,
            },
            _ => return Err(
                "audiorecorder takes no arguments or a sample rate, bit depth, and channel count."
                    .into(),
            ),
        };
        Self::new(Kind::Recorder(None), format)
    }

    pub fn class(&self) -> &'static str {
        match self.kind {
            Kind::Player { .. } => "audioplayer",
            Kind::Recorder(_) => "audiorecorder",
        }
    }

    pub fn properties(&self) -> &'static [&'static str] {
        match self.kind {
            Kind::Player { .. } => PLAYER_PROPERTIES,
            Kind::Recorder(_) => RECORDER_PROPERTIES,
        }
    }

    pub fn methods(&self) -> &'static [&'static str] {
        match self.kind {
            Kind::Player { .. } => &[
                "play",
                "playblocking",
                "pause",
                "resume",
                "stop",
                "isplaying",
                "get",
                "set",
                "disp",
                "isvalid",
                "delete",
            ],
            Kind::Recorder(_) => &[
                "record",
                "recordblocking",
                "stop",
                "getaudiodata",
                "getplayer",
                "get",
                "set",
                "disp",
                "isvalid",
                "delete",
            ],
        }
    }

    pub fn bytes(&self) -> usize {
        let data = match &self.kind {
            Kind::Player { clip, .. } => clip.source.retained_bytes(),
            Kind::Recorder(capture) => capture.as_deref().map_or(0, native::Input::bytes),
        };
        data + self.tag.len() + self.user.retained_bytes() + 256
    }

    fn status(&self) -> (bool, usize, usize) {
        match &self.kind {
            Kind::Player { clip, playback } => {
                let (running, sample) = playback.lock().unwrap().status();
                (
                    running,
                    if clip.frames == 0 { 0 } else { sample },
                    clip.frames,
                )
            }
            Kind::Recorder(capture) => capture.as_deref().map_or((false, 0, 0), |input| {
                let (running, frames) = input.status();
                (running, frames + usize::from(running), frames)
            }),
        }
    }

    pub fn property(&self, name: &str) -> Result<Value> {
        let n = match name {
            "SampleRate" => f64::from(self.format.rate),
            "BitsPerSample" => f64::from(self.format.bits),
            "NumChannels" => self.format.channels as f64,
            "DeviceID" if matches!(self.kind, Kind::Player { .. }) => f64::from(self.format.device),
            "TotalSamples" => self.status().2 as f64,
            "CurrentSample" => self.status().1 as f64,
            "Running" => return Ok(Value::Text(on_off(self.status().0).into())),
            "Tag" => return Ok(Value::Text(self.tag.to_string())),
            "UserData" => return Ok(Value::from(self.user.as_ref().clone())),
            _ => {
                return Err(format!(
                    "No public property '{name}' exists for class {}.",
                    self.class()
                ));
            }
        };
        Ok(Value::Matrix(Matrix::scalar(n)))
    }

    pub fn assign(&mut self, engine: &Engine, name: &str, value: Value) -> Result<()> {
        match name {
            "SampleRate" => {
                if self.status().0 {
                    return Err("Stop audio before changing SampleRate.".into());
                }
                // Paused playback can be resumed only at its original rate.
                if let Kind::Player { playback, .. } = &self.kind
                    && !playback.lock().unwrap().rate_can_change()
                {
                    return Err("Stop audio before changing SampleRate.".into());
                }
                self.format.rate = rate(scalar(&real_array(engine, &value)?)?)?;
            }
            "Tag" => self.tag = text(engine, &value)?.into(),
            "UserData" => self.user = Arc::new(value.array(&engine.cancel)?),
            _ if self.properties().contains(&name) => {
                return Err(format!(
                    "You cannot set the read-only property '{name}' of {}.",
                    self.class()
                ));
            }
            _ => {
                return Err(format!(
                    "No public property '{name}' exists for class {}.",
                    self.class()
                ));
            }
        }
        Ok(())
    }

    pub fn body(&self) -> String {
        let (running, current, total) = self.status();
        let device = if matches!(self.kind, Kind::Player { .. }) {
            format!("    DeviceID: {}\n", self.format.device)
        } else {
            String::new()
        };
        format!(
            "  {} with properties:\n\n    SampleRate: {}\n    BitsPerSample: {}\n    NumChannels: {}\n{device}    TotalSamples: {total}\n    CurrentSample: {current}\n    Running: '{}'\n    Tag: '{}'\n    UserData: [{}x{} {}]\n",
            self.class(),
            self.format.rate,
            self.format.bits,
            self.format.channels,
            on_off(running),
            self.tag,
            self.user.rows(),
            self.user.cols(),
            self.user.class()
        )
    }

    pub fn active(&self) -> bool {
        match &self.kind {
            // Running now. A finished or paused player must not keep the
            // per-statement cycle sweep alive.
            Kind::Player { playback, .. } => playback.lock().unwrap().status().0,
            Kind::Recorder(capture) => capture.as_ref().is_some_and(|input| input.status().0),
        }
    }

    pub fn stop(&self) {
        match &self.kind {
            Kind::Player { playback, .. } => *playback.lock().unwrap() = Playback::Stopped,
            Kind::Recorder(capture) => {
                if let Some(input) = capture {
                    input.stop();
                }
            }
        }
    }

    pub fn getplayer(&self, engine: &mut Engine) -> Result<Self> {
        let matrix = self.captured(engine)?;
        let clip = Clip {
            frames: matrix.rows(),
            channels: self.format.channels,
            source: Array::Numeric(matrix),
            scale: Scale::Direct,
        };
        Self::from_clip(clip, self.format)
    }

    fn captured(&self, engine: &mut Engine) -> Result<Matrix> {
        let Kind::Recorder(capture) = &self.kind else {
            unreachable!("getaudiodata and getplayer are recorder methods")
        };
        match capture {
            Some(input) => input.matrix(engine),
            None => Matrix::filled(0, self.format.channels, 0.),
        }
    }

    pub fn operate(
        &mut self,
        engine: &mut Engine,
        name: &str,
        args: &[Value],
        out: &mut Execution,
    ) -> Result<Value> {
        let scalar_arg = |engine: &Engine, value| scalar(&real_array(engine, value)?);
        match (&mut self.kind, name, args) {
            (Kind::Player { clip, playback }, "play" | "playblocking", [] | [_]) => {
                let range = playback_range(engine, args.first(), clip.frames)?;
                *playback.lock().unwrap() = Playback::Stopped;
                let started = start_playback(engine, clip, self.format, range, out)?;
                *playback.lock().unwrap() = started;
                if name == "playblocking" {
                    let result = wait(engine, || playback.lock().unwrap().status().0);
                    let mut playback = playback.lock().unwrap();
                    if result.is_err() {
                        *playback = Playback::Stopped;
                    }
                    result?;
                    if matches!(&*playback, Playback::Device(output) if output.failed()) {
                        return Err("The audio output stream stopped unexpectedly.".into());
                    }
                }
            }
            (Kind::Player { playback, .. }, "pause", []) => playback.lock().unwrap().pause(),
            (Kind::Player { playback, .. }, "resume", []) => playback.lock().unwrap().resume(),
            (Kind::Player { .. }, "isplaying", []) => {
                return Ok(Value::Logical(LogicalMatrix::scalar(self.status().0)));
            }
            (_, "stop", []) => self.stop(),
            (Kind::Recorder(capture), "record" | "recordblocking", [] | [_]) => {
                if name == "recordblocking" && args.is_empty() {
                    return Err("recordblocking requires a duration in seconds.".into());
                }
                if force_null(engine) || engine.audio.no_input_device {
                    return Err(native::NO_INPUT.into());
                }
                if capture.as_ref().is_some_and(|input| input.status().0) {
                    return Err("Stop the current recording before starting another.".into());
                }
                let budget =
                    limits::memory_bytes(&engine.limits).saturating_sub(engine.workspace_bytes());
                let frame_bytes = self.format.channels * size_of::<f32>();
                let max_frames = budget / frame_bytes;
                let limit = if let Some(value) = args.first() {
                    let duration = scalar_arg(engine, value)?;
                    let frames = (duration * f64::from(self.format.rate)).round();
                    if !duration.is_finite() || duration <= 0. || !frames.is_finite() || frames < 1.
                    {
                        return Err("Recording duration is outside the supported range.".into());
                    }
                    if frames > max_frames as f64 {
                        return Err(limits::memory_error("Audio recording", &engine.limits));
                    }
                    frames as usize
                } else {
                    let default_frames = budget.min(DEFAULT_RECORD_BYTES) / frame_bytes;
                    if default_frames == 0 {
                        return Err(limits::memory_error("Audio recording", &engine.limits));
                    }
                    default_frames
                };
                let input = match native::Input::start(engine, self.format, limit) {
                    Ok(input) => Arc::new(input),
                    // Only a missing device is remembered. A start timeout or a
                    // build/play failure keeps its own error.
                    Err(error) if error == native::NO_INPUT => {
                        engine.audio.no_input_device = true;
                        return Err(error);
                    }
                    Err(error) => return Err(error),
                };
                *capture = Some(input.clone());
                if name == "recordblocking" {
                    wait(engine, || input.status().0).inspect_err(|_| input.stop())?;
                    if input.failed() {
                        return Err("The audio input stream stopped unexpectedly.".into());
                    }
                }
            }
            (Kind::Recorder(_), "getaudiodata", []) => {
                return self.captured(engine).map(Value::Matrix);
            }
            (Kind::Recorder(_), "getaudiodata", [class]) => {
                let class = text(engine, class)?;
                if class == "double" {
                    return self.captured(engine).map(Value::Matrix);
                }
                let data = self.captured(engine)?;
                return capture_class(engine, data, &class);
            }
            _ => return Err(format!("{name}: invalid arguments for {}.", self.class())),
        }
        Ok(Value::Void)
    }
}

impl Playback {
    fn rate_can_change(&self) -> bool {
        match self {
            Self::Stopped => true,
            Self::Null(playback) => {
                let clock = playback.clock.lock().unwrap();
                clock.played().as_secs_f64() * f64::from(clock.rate) >= clock.range.len() as f64
            }
            Self::Device(device) => device.finished(),
        }
    }
}

fn capture_class(engine: &Engine, data: Matrix, class: &str) -> Result<Value> {
    use crate::{NumericClass, TypedArray};
    let class = match class {
        "single" => NumericClass::Single,
        "int16" => NumericClass::Int16,
        "int8" => NumericClass::Int8,
        "uint8" => NumericClass::Uint8,
        _ => return Err("getaudiodata: expected double, single, int16, int8, or uint8.".into()),
    };
    if engine
        .workspace_bytes()
        .saturating_add(data.len().saturating_mul(8 + class.size()))
        > limits::memory_bytes(&engine.limits)
    {
        return Err(limits::memory_error(
            "Audio data conversion",
            &engine.limits,
        ));
    }
    let samples = data
        .data()
        .iter()
        .copied()
        .enumerate()
        .take_while(|(i, _)| !i.is_multiple_of(16_384) || engine.check_cancel().is_ok())
        .map(|(_, x)| x);
    let (scale, offset) = match class {
        NumericClass::Int16 => (32768., 0.),
        NumericClass::Int8 => (128., 0.),
        NumericClass::Uint8 => (128., 128.),
        _ => (1., 0.),
    };
    let typed = TypedArray::from_reals(
        class,
        data.rows(),
        data.cols(),
        samples.map(|x| x * scale + offset),
    );
    engine.check_cancel()?;
    Ok(Value::from(Array::Typed(typed?)))
}

pub(in crate::engine) fn text(engine: &Engine, value: &Value) -> Result<String> {
    match value.clone().array(&engine.cancel)? {
        Array::Character(c) if c.rows() <= 1 => Ok(String::from_utf16_lossy(c.units())),
        Array::Str(s) if s.len() == 1 => s.data()[0]
            .as_ref()
            .map(ToString::to_string)
            .ok_or("Audio property text cannot be missing.".into()),
        _ => Err("Expected character text or a string scalar.".into()),
    }
}

fn on_off(flag: bool) -> &'static str {
    if flag { "on" } else { "off" }
}

fn playback_range(engine: &Engine, value: Option<&Value>, frames: usize) -> Result<Range<usize>> {
    let Some(value) = value else {
        return Ok(0..frames);
    };
    let value = real_array(engine, value)?;
    let index = |i| {
        integer(
            real_at(&value, i),
            1.,
            frames as f64,
            "Playback sample indices must be integers within the clip.",
        )
        .map(|n| n as usize)
    };
    let (start, end) = if value.len() == 1 {
        (index(0)?, frames)
    } else if vector_pair(&value) {
        (index(0)?, index(1)?)
    } else {
        return Err("Expected a start sample or a two-element sample range.".into());
    };
    if start > end {
        return Err("Playback start sample must precede or equal its last sample.".into());
    }
    Ok(start - 1..end)
}

fn wait(engine: &Engine, running: impl Fn() -> bool) -> Result<()> {
    while running() {
        engine.check_cancel()?;
        std::thread::sleep(POLL);
    }
    engine.check_cancel()
}
