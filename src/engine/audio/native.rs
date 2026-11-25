#[cfg(target_os = "linux")]
#[path = "alsa.rs"]
mod alsa;

#[cfg(not(target_os = "linux"))]
use super::enabled::Clip;
use super::enabled::Format;
use crate::{Matrix, Result, engine::Engine};
#[cfg(not(target_os = "linux"))]
use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
#[cfg(not(target_os = "linux"))]
use cpal::{
    FromSample, Sample, SampleFormat, SizedSample, Stream, StreamConfig, SupportedStreamConfig,
};
#[cfg(not(target_os = "linux"))]
use std::ops::Range;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering::Relaxed};
use std::sync::{Arc, Mutex, mpsc};
use std::thread::{self, Thread};
use std::time::{Duration, Instant};

#[cfg(target_os = "linux")]
pub(super) use alsa::Output;

pub(super) const NO_OUTPUT: &str = "No audio output device found; sound is not played.";
pub(super) const NO_INPUT: &str = "No audio input device found.";
pub(super) const MAX_STREAMS: usize = 16;
const POLL: Duration = Duration::from_millis(10);
const START_TIMEOUT: Duration = Duration::from_secs(2);
const OPEN_FAILED: &str = "Could not open the audio stream.";
#[cfg(not(target_os = "linux"))]
const PLAY_FAILED: &str = "Could not start the audio stream.";
const DID_NOT_START: &str = "The audio device did not start.";
const DEVICE_ID_MISSING: &str = "Audio output DeviceID was not found.";
static ACTIVE_STREAMS: AtomicUsize = AtomicUsize::new(0);

#[cfg(any(target_os = "dragonfly", target_os = "freebsd", target_os = "netbsd"))]
fn silence_alsa_errors() {
    use std::ffi::{c_char, c_int, c_void};
    type LocalHandler = unsafe extern "C" fn(
        *const c_char,
        c_int,
        *const c_char,
        c_int,
        *const c_char,
        *mut c_void,
    );
    unsafe extern "C" {
        fn snd_lib_error_set_local(func: Option<LocalHandler>) -> Option<LocalHandler>;
    }
    unsafe extern "C" fn ignore(
        _: *const c_char,
        _: c_int,
        _: *const c_char,
        _: c_int,
        _: *const c_char,
        _: *mut c_void,
    ) {
    }
    // SAFETY: `ignore` matches ALSA's `snd_lib_error_set_local` signature and touches no pointers.
    unsafe {
        snd_lib_error_set_local(Some(ignore));
    }
}

#[cfg(not(any(
    target_os = "linux",
    target_os = "dragonfly",
    target_os = "freebsd",
    target_os = "netbsd"
)))]
fn silence_alsa_errors() {}

#[cfg(not(target_os = "linux"))]
fn is_fatal_stream_error(error: &cpal::StreamError) -> bool {
    !matches!(error, cpal::StreamError::BufferUnderrun)
}

struct StreamSlot {
    released: AtomicBool,
}

impl StreamSlot {
    fn acquire() -> Result<Arc<Self>> {
        ACTIVE_STREAMS
            .try_update(Relaxed, Relaxed, |count| {
                (count < MAX_STREAMS).then_some(count + 1)
            })
            .map_err(|_| "Too many simultaneous audio streams.")?;
        Ok(Arc::new(Self {
            released: AtomicBool::new(false),
        }))
    }

    fn release(&self) {
        if !self.released.swap(true, Relaxed) {
            ACTIVE_STREAMS.fetch_sub(1, Relaxed);
        }
    }
}

struct ReleaseOnDrop(Arc<StreamSlot>);
impl Drop for ReleaseOnDrop {
    fn drop(&mut self) {
        self.0.release();
    }
}

// Keep native streams on their opening thread. The worker owns the stream and releases it
// when playback drains, the recording fills, or its controller is dropped.
struct Worker {
    stop: Arc<AtomicBool>,
    thread: Thread,
    slot: Arc<StreamSlot>,
}

impl Worker {
    fn start_custom<T>(
        engine: &Engine,
        timeout: Duration,
        run: impl FnOnce(Arc<AtomicBool>, mpsc::SyncSender<Result<T>>) + Send + 'static,
    ) -> Result<(Self, T)>
    where
        T: Send + 'static,
    {
        let slot = StreamSlot::acquire()?;
        let worker_slot = ReleaseOnDrop(slot.clone());
        let (ready, startup) = mpsc::sync_channel(1);
        let stop = Arc::new(AtomicBool::new(false));
        let stopped = stop.clone();
        let thread = thread::Builder::new()
            .name("matrust-audio".into())
            .spawn(move || {
                let _slot = worker_slot;
                run(stopped, ready);
            })
            .inspect_err(|_| slot.release())
            .map_err(|error| format!("Could not start audio worker: {error}"))?;
        let worker = Self {
            stop,
            thread: thread.thread().clone(),
            slot,
        };
        let start = Instant::now();
        loop {
            engine.check_cancel()?;
            match startup.recv_timeout(POLL) {
                Ok(result) => return result.map(|state| (worker, state)),
                Err(mpsc::RecvTimeoutError::Timeout) if start.elapsed() < timeout => (),
                Err(mpsc::RecvTimeoutError::Timeout) => {
                    return Err(DID_NOT_START.into());
                }
                Err(mpsc::RecvTimeoutError::Disconnected) => {
                    return Err("Audio worker closed during startup.".into());
                }
            }
        }
    }

    #[cfg(not(target_os = "linux"))]
    fn start<T, F>(
        engine: &Engine,
        open: impl FnOnce() -> Result<(Stream, T, F)> + Send + 'static,
    ) -> Result<(Self, T)>
    where
        T: Send + 'static,
        F: Fn() -> bool,
    {
        Self::start_custom(engine, START_TIMEOUT, move |stopped, ready| {
            silence_alsa_errors();
            let result = open().and_then(|(stream, state, finished)| {
                if !stopped.load(Relaxed) {
                    stream.play().map_err(|_| PLAY_FAILED)?;
                }
                Ok((stream, state, finished))
            });
            match result {
                Ok((_stream, state, finished)) => {
                    if ready.send(Ok(state)).is_ok() {
                        while !stopped.load(Relaxed) && !finished() {
                            thread::park_timeout(POLL);
                        }
                    }
                }
                Err(error) => {
                    let _ = ready.send(Err(error));
                }
            }
        })
    }

    #[cfg(target_os = "linux")]
    fn unpark(&self) {
        self.thread.unpark();
    }

    fn stop(&self) {
        self.stop.store(true, Relaxed);
        self.slot.release();
        self.thread.unpark();
    }
}

impl Drop for Worker {
    fn drop(&mut self) {
        self.stop();
    }
}

#[cfg(not(target_os = "linux"))]
fn device_config(
    device: &cpal::Device,
    format: Format,
    input: bool,
) -> Option<SupportedStreamConfig> {
    let default = if input {
        device.default_input_config()
    } else {
        device.default_output_config()
    }
    .ok()?;
    if default.sample_rate() == format.rate && usize::from(default.channels()) == format.channels {
        return Some(default);
    }
    let ranges = if input {
        device
            .supported_input_configs()
            .ok()
            .map(|i| i.collect::<Vec<_>>())
    } else {
        device
            .supported_output_configs()
            .ok()
            .map(|i| i.collect::<Vec<_>>())
    };
    ranges
        .unwrap_or_default()
        .into_iter()
        .filter(|r| {
            usize::from(r.channels()) == format.channels
                && (r.min_sample_rate()..=r.max_sample_rate()).contains(&format.rate)
        })
        .max_by_key(|r| r.sample_format() == default.sample_format())
        .map(|r| r.with_sample_rate(format.rate))
        .or(Some(default))
}

// Map native stream sample formats through CPAL's sample conversion traits.
#[cfg(not(target_os = "linux"))]
macro_rules! stream {
    ($format:expr, $build:ident($($arg:expr),*), $missing:expr) => {
        match $format {
            SampleFormat::I8 => $build::<i8>($($arg),*),
            SampleFormat::I16 => $build::<i16>($($arg),*),
            SampleFormat::I24 => $build::<cpal::I24>($($arg),*),
            SampleFormat::I32 => $build::<i32>($($arg),*),
            SampleFormat::I64 => $build::<i64>($($arg),*),
            SampleFormat::U8 => $build::<u8>($($arg),*),
            SampleFormat::U16 => $build::<u16>($($arg),*),
            SampleFormat::U24 => $build::<cpal::U24>($($arg),*),
            SampleFormat::U32 => $build::<u32>($($arg),*),
            SampleFormat::U64 => $build::<u64>($($arg),*),
            SampleFormat::F32 => $build::<f32>($($arg),*),
            SampleFormat::F64 => $build::<f64>($($arg),*),
            _ => return Err($missing.into()),
        }
    };
}

fn pcm(sample: f32, bits: u8) -> f32 {
    let full = (1_u32 << (bits - 1)) as f32;
    (sample.clamp(-1., 1.) * full).round().min(full - 1.) / full
}

fn interpolated(previous: f32, next: f32, fraction: f32) -> f32 {
    previous + (next - previous) * fraction
}

#[derive(Default)]
struct Render {
    phase: u64,
    paused: bool,
    drain: Option<Instant>,
    failed: bool,
}

impl Render {
    fn finished(&self) -> bool {
        self.failed || self.drain.is_some_and(|end| Instant::now() >= end)
    }
}

#[cfg(not(target_os = "linux"))]
pub(super) struct Output {
    _worker: Worker,
    render: Arc<Mutex<Render>>,
    range: Range<usize>,
    device_rate: u32,
}

#[cfg(not(target_os = "linux"))]
impl Output {
    pub(super) fn start(
        engine: &Engine,
        clip: Clip,
        format: Format,
        range: Range<usize>,
    ) -> Result<Self> {
        let selection = range.clone();
        let (worker, (render, device_rate)) = Worker::start(engine, move || {
            let host = cpal::default_host();
            let device = if format.device < 0 {
                host.default_output_device().ok_or(NO_OUTPUT)?
            } else {
                host.output_devices()
                    .map_err(|_| OPEN_FAILED)?
                    .nth(format.device as usize)
                    .ok_or(DEVICE_ID_MISSING)?
            };
            let supported = device_config(&device, format, false).ok_or(OPEN_FAILED)?;
            let config = supported.config();
            let render = Arc::new(Mutex::new(Render::default()));
            let stream = stream!(
                supported.sample_format(),
                output_stream(&device, &config, clip, format, selection, render.clone()),
                OPEN_FAILED
            )?;
            let watched = render.clone();
            Ok((stream, (render, config.sample_rate), move || {
                watched.lock().unwrap().finished()
            }))
        })?;
        Ok(Self {
            _worker: worker,
            render,
            range,
            device_rate,
        })
    }

    pub(super) fn status(&self) -> (bool, usize) {
        let render = self.render.lock().unwrap();
        if render.finished() {
            (false, 1)
        } else {
            let index =
                ((render.phase / u64::from(self.device_rate)) as usize).min(self.range.len() - 1);
            (!render.paused, self.range.start + index + 1)
        }
    }

    pub(super) fn finished(&self) -> bool {
        self.render.lock().unwrap().finished()
    }

    pub(super) fn failed(&self) -> bool {
        self.render.lock().unwrap().failed
    }

    pub(super) fn pause(&self, paused: bool) {
        let mut render = self.render.lock().unwrap();
        if render.drain.is_none() {
            render.paused = paused;
        }
    }
}

#[cfg(not(target_os = "linux"))]
fn output_stream<T: SizedSample + FromSample<f32>>(
    device: &cpal::Device,
    config: &StreamConfig,
    clip: Clip,
    format: Format,
    range: Range<usize>,
    render: Arc<Mutex<Render>>,
) -> Result<Stream> {
    let native_channels = usize::from(config.channels);
    let native_rate = u64::from(config.sample_rate);
    let step = u64::from(format.rate);
    let end = range.len() as u64 * native_rate;
    let failed = render.clone();
    device
        .build_output_stream(
            config,
            move |samples: &mut [T], info| {
                let mut render = render.lock().unwrap();
                samples.fill(T::EQUILIBRIUM);
                if render.paused || render.drain.is_some() || render.failed {
                    return;
                }
                for (offset, frame) in samples.chunks_exact_mut(native_channels).enumerate() {
                    let current = range.start + (render.phase / native_rate) as usize;
                    let next = (current + 1).min(range.end - 1);
                    let fraction = (render.phase % native_rate) as f32 / native_rate as f32;
                    let sample = |channel| {
                        interpolated(
                            clip.sample(current, channel),
                            clip.sample(next, channel),
                            fraction,
                        )
                    };
                    let left = sample(0);
                    let right = if clip.channels == 2 { sample(1) } else { left };
                    if native_channels == 1 {
                        frame[0] = T::from_sample(pcm((left + right) * 0.5, format.bits));
                    } else {
                        frame[0] = T::from_sample(pcm(left, format.bits));
                        frame[1] = T::from_sample(pcm(right, format.bits));
                    }
                    render.phase += step;
                    if render.phase >= end {
                        let time = info.timestamp();
                        let latency = time
                            .playback
                            .duration_since(&time.callback)
                            .unwrap_or_default();
                        let remaining =
                            Duration::from_secs_f64((offset + 1) as f64 / native_rate as f64);
                        render.drain = Instant::now()
                            .checked_add(latency.saturating_add(remaining))
                            .or(Some(Instant::now()));
                        break;
                    }
                }
            },
            move |error| {
                if is_fatal_stream_error(&error) {
                    failed.lock().unwrap().failed = true;
                }
            },
            None,
        )
        .map_err(|_| OPEN_FAILED.into())
}

struct Capture {
    values: Vec<f32>,
    limit: usize,
    running: bool,
    failed: bool,
    /// Sample count at the last moment samples were still arriving.
    mark: usize,
    /// When `mark` last changed, or when the wait for samples began.
    mark_at: Option<Instant>,
}

impl Capture {
    /// No samples for `timeout` after start, or after the last sample, fails
    /// the stream. A capture overrun can leave the device prepared and silent;
    /// `recordblocking` has to return instead of waiting forever.
    fn fail_if_silent(&mut self, timeout: Duration) {
        if !self.running {
            return;
        }
        let count = self.values.len();
        if let Some(at) = self.mark_at
            && count == self.mark
        {
            if at.elapsed() >= timeout {
                self.running = false;
                self.failed = true;
            }
            return;
        }
        self.mark = count;
        self.mark_at = Some(Instant::now());
    }
}

pub(super) struct Input {
    worker: Worker,
    capture: Arc<Mutex<Capture>>,
    channels: usize,
}

impl Input {
    #[cfg(target_os = "linux")]
    pub(super) fn start(engine: &Engine, format: Format, frames: usize) -> Result<Self> {
        let (worker, capture) = alsa::start_input(engine, format, frames)?;
        Ok(Self {
            worker,
            capture,
            channels: format.channels,
        })
    }

    #[cfg(not(target_os = "linux"))]
    pub(super) fn start(engine: &Engine, format: Format, frames: usize) -> Result<Self> {
        let budget = crate::engine::limits::memory_bytes(&engine.limits)
            .saturating_sub(engine.workspace_bytes());
        let limit = frames.saturating_mul(format.channels);
        let memory_error = crate::engine::limits::memory_error("Audio recording", &engine.limits);
        let (worker, capture) = Worker::start(engine, move || {
            let device = cpal::default_host()
                .default_input_device()
                .ok_or(NO_INPUT)?;
            let supported = device_config(&device, format, true).ok_or(OPEN_FAILED)?;
            if limit.saturating_mul(size_of::<f32>()) > budget {
                return Err(memory_error);
            }
            let mut values = Vec::new();
            values
                .try_reserve_exact(limit)
                .map_err(|_| "Audio recording could not allocate its sample buffer.")?;
            let capture = Arc::new(Mutex::new(Capture {
                values,
                limit,
                running: true,
                failed: false,
                mark: 0,
                mark_at: None,
            }));
            let stream = stream!(
                supported.sample_format(),
                input_stream(&device, &supported.config(), format, capture.clone()),
                OPEN_FAILED
            )?;
            let watched = capture.clone();
            Ok((stream, capture, move || {
                let mut capture = watched.lock().unwrap();
                capture.fail_if_silent(START_TIMEOUT);
                !capture.running
            }))
        })?;
        Ok(Self {
            worker,
            capture,
            channels: format.channels,
        })
    }

    pub(super) fn status(&self) -> (bool, usize) {
        let mut capture = self.capture.lock().unwrap();
        capture.fail_if_silent(START_TIMEOUT);
        (capture.running, capture.values.len() / self.channels)
    }

    pub(super) fn bytes(&self) -> usize {
        self.capture.lock().unwrap().limit * size_of::<f32>()
    }

    pub(super) fn failed(&self) -> bool {
        self.capture.lock().unwrap().failed
    }

    pub(super) fn stop(&self) {
        self.capture.lock().unwrap().running = false;
        self.worker.stop();
    }

    pub(super) fn matrix(&self, engine: &mut Engine) -> Result<Matrix> {
        let frames = self.status().1;
        let len = frames * self.channels;
        if engine
            .workspace_bytes()
            .saturating_add(len.saturating_mul(size_of::<f64>()))
            > crate::engine::limits::memory_bytes(&engine.limits)
        {
            return Err(crate::engine::limits::memory_error(
                "Audio data",
                &engine.limits,
            ));
        }
        engine.charge(len)?;
        let mut values = Vec::new();
        values
            .try_reserve_exact(len)
            .map_err(|_| "Could not allocate audio data.")?;
        // Read a fixed prefix, in chunks, so a live input can keep appending samples.
        for channel in 0..self.channels {
            for start in (0..frames).step_by(4096) {
                engine.check_cancel()?;
                let end = (start + 4096).min(frames);
                let capture = self.capture.lock().unwrap();
                values.extend(
                    (start..end)
                        .map(|frame| f64::from(capture.values[frame * self.channels + channel])),
                );
            }
        }
        Matrix::from_column_vec(frames, self.channels, values)
    }
}

#[cfg(not(target_os = "linux"))]
fn input_stream<T: SizedSample>(
    device: &cpal::Device,
    config: &StreamConfig,
    format: Format,
    capture: Arc<Mutex<Capture>>,
) -> Result<Stream>
where
    f32: FromSample<T>,
{
    let native_channels = usize::from(config.channels);
    let native_rate = u64::from(config.sample_rate);
    let rate = u64::from(format.rate);
    let mut native_time = 0;
    let mut output_time = 0;
    let mut previous = [0.; 2];
    let failed = capture.clone();
    device
        .build_input_stream(
            config,
            move |samples: &[T], _| {
                let mut capture = capture.lock().unwrap();
                if !capture.running {
                    return;
                }
                for frame in samples.chunks_exact(native_channels) {
                    let left = f32::from_sample(frame[0]);
                    let right = if native_channels >= 2 {
                        f32::from_sample(frame[1])
                    } else {
                        left
                    };
                    let next = if format.channels == 1 {
                        [(left + right) * 0.5, 0.]
                    } else {
                        [left, right]
                    };
                    while output_time <= native_time {
                        let fraction = 1. - (native_time - output_time) as f32 / rate as f32;
                        for channel in 0..format.channels {
                            let value = interpolated(previous[channel], next[channel], fraction);
                            capture
                                .values
                                .push(pcm(if value.is_finite() { value } else { 0. }, format.bits));
                        }
                        output_time += native_rate;
                        if capture.values.len() == capture.limit {
                            capture.running = false;
                            return;
                        }
                    }
                    previous = next;
                    native_time += rate;
                }
            },
            move |error| {
                if !is_fatal_stream_error(&error) {
                    return;
                }
                let mut capture = failed.lock().unwrap();
                if capture.running {
                    capture.running = false;
                    capture.failed = true;
                }
            },
            None,
        )
        .map_err(|_| OPEN_FAILED.into())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[cfg(not(target_os = "linux"))]
    #[test]
    fn buffer_underrun_is_not_fatal() {
        assert!(!is_fatal_stream_error(&cpal::StreamError::BufferUnderrun));
        assert!(is_fatal_stream_error(
            &cpal::StreamError::DeviceNotAvailable
        ));
        assert!(is_fatal_stream_error(&cpal::StreamError::StreamInvalidated));
        assert!(is_fatal_stream_error(&cpal::StreamError::BackendSpecific {
            err: cpal::BackendSpecificError {
                description: "device lost".into(),
            },
        }));
    }

    #[test]
    fn hung_open_releases_its_slot() {
        let engine = Engine::default();
        let before = ACTIVE_STREAMS.load(Relaxed);
        let (release_tx, release_rx) = mpsc::channel::<()>();
        let err = Worker::start_custom(
            &engine,
            Duration::from_millis(40),
            move |_stopped, ready: mpsc::SyncSender<Result<()>>| {
                let _ = release_rx.recv();
                let _ = ready.send(Err(NO_OUTPUT.into()));
            },
        )
        .err()
        .unwrap();
        assert_eq!(err, DID_NOT_START);
        assert_ne!(err, NO_OUTPUT);
        assert_ne!(err, NO_INPUT);
        assert_eq!(ACTIVE_STREAMS.load(Relaxed), before);
        let _ = release_tx.send(());
        thread::sleep(Duration::from_millis(20));
        assert_eq!(ACTIVE_STREAMS.load(Relaxed), before);
    }

    #[test]
    fn silent_capture_fails_after_two_seconds_of_no_data() {
        let mut capture = Capture {
            values: Vec::new(),
            limit: 8,
            running: true,
            failed: false,
            mark: 0,
            mark_at: None,
        };
        capture.fail_if_silent(Duration::from_millis(30));
        assert!(capture.running);
        assert!(!capture.failed);
        capture.mark_at = Some(Instant::now() - Duration::from_millis(40));
        capture.fail_if_silent(Duration::from_millis(30));
        assert!(!capture.running);
        assert!(capture.failed);
    }

    #[test]
    fn capture_progress_resets_the_silence_timeout() {
        let mut capture = Capture {
            values: vec![0.0],
            limit: 8,
            running: true,
            failed: false,
            mark: 0,
            mark_at: Some(Instant::now() - Duration::from_secs(5)),
        };
        capture.fail_if_silent(Duration::from_millis(30));
        assert!(capture.running);
        assert!(!capture.failed);
        assert_eq!(capture.mark, 1);
    }
}
