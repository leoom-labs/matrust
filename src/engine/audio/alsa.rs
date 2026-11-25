//! Linux ALSA PCM backend loaded at runtime through `libloading`.
//!
//! Keeps `libasound.so.2` out of the binary's `DT_NEEDED` list so `matrust` starts
//! on Linux systems without `libasound2` installed.

use super::super::enabled::{Clip, Format};
use super::{
    Capture, DEVICE_ID_MISSING, NO_INPUT, NO_OUTPUT, OPEN_FAILED, POLL, Render, START_TIMEOUT,
    Worker, interpolated, pcm,
};
use crate::{Result, engine::Engine};
use libloading::Library;
use std::ffi::{CStr, CString, OsStr, OsString, c_char, c_int, c_long, c_uint, c_ulong, c_void};
use std::ops::Range;
use std::ptr;
use std::sync::atomic::{AtomicBool, Ordering::Relaxed};
use std::sync::{Arc, Mutex, OnceLock};
use std::thread;
use std::time::Instant;

const ALSA_LIB_NAME: &str = "libasound.so.2";
const ALSA_LIB_OVERRIDE: &str = "MATRUST_ALSA_LIB";

const SND_PCM_STREAM_PLAYBACK: c_int = 0;
const SND_PCM_STREAM_CAPTURE: c_int = 1;
const SND_PCM_NONBLOCK: c_int = 1;
const SND_PCM_ACCESS_RW_INTERLEAVED: c_int = 3;

#[cfg(target_endian = "little")]
const SND_PCM_FORMAT_S16: c_int = 2;
#[cfg(target_endian = "big")]
const SND_PCM_FORMAT_S16: c_int = 3;

#[cfg(target_endian = "little")]
const SND_PCM_FORMAT_S32: c_int = 10;
#[cfg(target_endian = "big")]
const SND_PCM_FORMAT_S32: c_int = 11;

#[cfg(target_endian = "little")]
const SND_PCM_FORMAT_FLOAT: c_int = 14;
#[cfg(target_endian = "big")]
const SND_PCM_FORMAT_FLOAT: c_int = 15;

// Linux errno values returned negated by ALSA PCM calls.
const ENOENT: c_int = 2;
const EINTR: c_int = 4;
const EAGAIN: c_int = 11;
const ENODEV: c_int = 19;
const EPIPE: c_int = 32;
const ESTRPIPE: c_int = 86;

const WAIT_MS: c_int = 10;
const TARGET_PERIOD_US: c_uint = 20_000;
const TARGET_BUFFER_US: c_uint = 80_000;

#[repr(C)]
struct SndPcm {
    _private: [u8; 0],
}

#[repr(C)]
struct SndPcmHwParams {
    _private: [u8; 0],
}

type LocalErrorHandler =
    unsafe extern "C" fn(*const c_char, c_int, *const c_char, c_int, *const c_char, *mut c_void);

unsafe extern "C" {
    fn free(ptr: *mut c_void);
}

struct AlsaLib {
    _lib: Library,
    snd_pcm_open: unsafe extern "C" fn(*mut *mut SndPcm, *const c_char, c_int, c_int) -> c_int,
    snd_pcm_close: unsafe extern "C" fn(*mut SndPcm) -> c_int,
    snd_pcm_hw_params_malloc: unsafe extern "C" fn(*mut *mut SndPcmHwParams) -> c_int,
    snd_pcm_hw_params_free: unsafe extern "C" fn(*mut SndPcmHwParams),
    snd_pcm_hw_params_any: unsafe extern "C" fn(*mut SndPcm, *mut SndPcmHwParams) -> c_int,
    snd_pcm_hw_params_set_access:
        unsafe extern "C" fn(*mut SndPcm, *mut SndPcmHwParams, c_int) -> c_int,
    snd_pcm_hw_params_set_format:
        unsafe extern "C" fn(*mut SndPcm, *mut SndPcmHwParams, c_int) -> c_int,
    snd_pcm_hw_params_set_channels_near:
        unsafe extern "C" fn(*mut SndPcm, *mut SndPcmHwParams, *mut c_uint) -> c_int,
    snd_pcm_hw_params_set_rate_near:
        unsafe extern "C" fn(*mut SndPcm, *mut SndPcmHwParams, *mut c_uint, *mut c_int) -> c_int,
    snd_pcm_hw_params_set_period_time_near:
        unsafe extern "C" fn(*mut SndPcm, *mut SndPcmHwParams, *mut c_uint, *mut c_int) -> c_int,
    snd_pcm_hw_params_set_buffer_time_near:
        unsafe extern "C" fn(*mut SndPcm, *mut SndPcmHwParams, *mut c_uint, *mut c_int) -> c_int,
    snd_pcm_hw_params_get_period_size:
        unsafe extern "C" fn(*const SndPcmHwParams, *mut c_ulong, *mut c_int) -> c_int,
    snd_pcm_hw_params: unsafe extern "C" fn(*mut SndPcm, *mut SndPcmHwParams) -> c_int,
    snd_pcm_prepare: unsafe extern "C" fn(*mut SndPcm) -> c_int,
    snd_pcm_writei: unsafe extern "C" fn(*mut SndPcm, *const c_void, c_ulong) -> c_long,
    snd_pcm_readi: unsafe extern "C" fn(*mut SndPcm, *mut c_void, c_ulong) -> c_long,
    snd_pcm_recover: unsafe extern "C" fn(*mut SndPcm, c_int, c_int) -> c_int,
    snd_pcm_drain: unsafe extern "C" fn(*mut SndPcm) -> c_int,
    snd_pcm_drop: unsafe extern "C" fn(*mut SndPcm) -> c_int,
    snd_pcm_wait: unsafe extern "C" fn(*mut SndPcm, c_int) -> c_int,
    snd_strerror: unsafe extern "C" fn(c_int) -> *const c_char,
    snd_lib_error_set_local:
        Option<unsafe extern "C" fn(Option<LocalErrorHandler>) -> Option<LocalErrorHandler>>,
    snd_device_name_hint:
        Option<unsafe extern "C" fn(c_int, *const c_char, *mut *mut *mut c_void) -> c_int>,
    snd_device_name_get_hint:
        Option<unsafe extern "C" fn(*const c_void, *const c_char) -> *mut c_char>,
    snd_device_name_free_hint: Option<unsafe extern "C" fn(*mut *mut c_void) -> c_int>,
}

// SAFETY: `Library` is Send+Sync and all fields are plain C function pointers whose
// lifetime is tied to `_lib` staying loaded for as long as `AlsaLib` lives.
unsafe impl Send for AlsaLib {}
unsafe impl Sync for AlsaLib {}

static DEFAULT_ALSA: OnceLock<std::result::Result<Arc<AlsaLib>, ()>> = OnceLock::new();

impl AlsaLib {
    fn load(path: &OsStr) -> std::result::Result<Self, ()> {
        // SAFETY: We only load libasound and resolve its documented C ABI symbols.
        unsafe {
            let lib = Library::new(path).map_err(|_| ())?;
            macro_rules! sym {
                ($name:literal, $ty:ty) => {{
                    let symbol: libloading::Symbol<$ty> = lib.get($name).map_err(|_| ())?;
                    *symbol
                }};
            }
            macro_rules! opt_sym {
                ($name:literal, $ty:ty) => {{ lib.get::<$ty>($name).ok().map(|symbol| *symbol) }};
            }
            Ok(Self {
                snd_pcm_open: sym!(
                    b"snd_pcm_open\0",
                    unsafe extern "C" fn(*mut *mut SndPcm, *const c_char, c_int, c_int) -> c_int
                ),
                snd_pcm_close: sym!(
                    b"snd_pcm_close\0",
                    unsafe extern "C" fn(*mut SndPcm) -> c_int
                ),
                snd_pcm_hw_params_malloc: sym!(
                    b"snd_pcm_hw_params_malloc\0",
                    unsafe extern "C" fn(*mut *mut SndPcmHwParams) -> c_int
                ),
                snd_pcm_hw_params_free: sym!(
                    b"snd_pcm_hw_params_free\0",
                    unsafe extern "C" fn(*mut SndPcmHwParams)
                ),
                snd_pcm_hw_params_any: sym!(
                    b"snd_pcm_hw_params_any\0",
                    unsafe extern "C" fn(*mut SndPcm, *mut SndPcmHwParams) -> c_int
                ),
                snd_pcm_hw_params_set_access: sym!(
                    b"snd_pcm_hw_params_set_access\0",
                    unsafe extern "C" fn(*mut SndPcm, *mut SndPcmHwParams, c_int) -> c_int
                ),
                snd_pcm_hw_params_set_format: sym!(
                    b"snd_pcm_hw_params_set_format\0",
                    unsafe extern "C" fn(*mut SndPcm, *mut SndPcmHwParams, c_int) -> c_int
                ),
                snd_pcm_hw_params_set_channels_near: sym!(
                    b"snd_pcm_hw_params_set_channels_near\0",
                    unsafe extern "C" fn(*mut SndPcm, *mut SndPcmHwParams, *mut c_uint) -> c_int
                ),
                snd_pcm_hw_params_set_rate_near: sym!(
                    b"snd_pcm_hw_params_set_rate_near\0",
                    unsafe extern "C" fn(
                        *mut SndPcm,
                        *mut SndPcmHwParams,
                        *mut c_uint,
                        *mut c_int,
                    ) -> c_int
                ),
                snd_pcm_hw_params_set_period_time_near: sym!(
                    b"snd_pcm_hw_params_set_period_time_near\0",
                    unsafe extern "C" fn(
                        *mut SndPcm,
                        *mut SndPcmHwParams,
                        *mut c_uint,
                        *mut c_int,
                    ) -> c_int
                ),
                snd_pcm_hw_params_set_buffer_time_near: sym!(
                    b"snd_pcm_hw_params_set_buffer_time_near\0",
                    unsafe extern "C" fn(
                        *mut SndPcm,
                        *mut SndPcmHwParams,
                        *mut c_uint,
                        *mut c_int,
                    ) -> c_int
                ),
                snd_pcm_hw_params_get_period_size: sym!(
                    b"snd_pcm_hw_params_get_period_size\0",
                    unsafe extern "C" fn(*const SndPcmHwParams, *mut c_ulong, *mut c_int) -> c_int
                ),
                snd_pcm_hw_params: sym!(
                    b"snd_pcm_hw_params\0",
                    unsafe extern "C" fn(*mut SndPcm, *mut SndPcmHwParams) -> c_int
                ),
                snd_pcm_prepare: sym!(
                    b"snd_pcm_prepare\0",
                    unsafe extern "C" fn(*mut SndPcm) -> c_int
                ),
                snd_pcm_writei: sym!(
                    b"snd_pcm_writei\0",
                    unsafe extern "C" fn(*mut SndPcm, *const c_void, c_ulong) -> c_long
                ),
                snd_pcm_readi: sym!(
                    b"snd_pcm_readi\0",
                    unsafe extern "C" fn(*mut SndPcm, *mut c_void, c_ulong) -> c_long
                ),
                snd_pcm_recover: sym!(
                    b"snd_pcm_recover\0",
                    unsafe extern "C" fn(*mut SndPcm, c_int, c_int) -> c_int
                ),
                snd_pcm_drain: sym!(
                    b"snd_pcm_drain\0",
                    unsafe extern "C" fn(*mut SndPcm) -> c_int
                ),
                snd_pcm_drop: sym!(
                    b"snd_pcm_drop\0",
                    unsafe extern "C" fn(*mut SndPcm) -> c_int
                ),
                snd_pcm_wait: sym!(
                    b"snd_pcm_wait\0",
                    unsafe extern "C" fn(*mut SndPcm, c_int) -> c_int
                ),
                snd_strerror: sym!(
                    b"snd_strerror\0",
                    unsafe extern "C" fn(c_int) -> *const c_char
                ),
                snd_lib_error_set_local: opt_sym!(
                    b"snd_lib_error_set_local\0",
                    unsafe extern "C" fn(Option<LocalErrorHandler>) -> Option<LocalErrorHandler>
                ),
                snd_device_name_hint: opt_sym!(
                    b"snd_device_name_hint\0",
                    unsafe extern "C" fn(c_int, *const c_char, *mut *mut *mut c_void) -> c_int
                ),
                snd_device_name_get_hint: opt_sym!(
                    b"snd_device_name_get_hint\0",
                    unsafe extern "C" fn(*const c_void, *const c_char) -> *mut c_char
                ),
                snd_device_name_free_hint: opt_sym!(
                    b"snd_device_name_free_hint\0",
                    unsafe extern "C" fn(*mut *mut c_void) -> c_int
                ),
                _lib: lib,
            })
        }
    }

    fn silence_errors(&self) {
        unsafe extern "C" fn ignore(
            _: *const c_char,
            _: c_int,
            _: *const c_char,
            _: c_int,
            _: *const c_char,
            _: *mut c_void,
        ) {
        }
        if let Some(setter) = self.snd_lib_error_set_local {
            // SAFETY: `ignore` matches ALSA's `snd_local_error_handler_t` signature and touches no pointers.
            unsafe {
                setter(Some(ignore));
            }
        }
    }

    #[allow(dead_code)]
    fn strerror(&self, err: c_int) -> String {
        // SAFETY: `snd_strerror` returns a static NUL-terminated string owned by libasound.
        unsafe {
            let ptr = (self.snd_strerror)(err);
            if ptr.is_null() {
                format!("ALSA error {err}")
            } else {
                CStr::from_ptr(ptr).to_string_lossy().into_owned()
            }
        }
    }

    fn output_device_name(&self, index: usize) -> Option<CString> {
        let (hint_fn, get_fn, free_fn) = (
            self.snd_device_name_hint?,
            self.snd_device_name_get_hint?,
            self.snd_device_name_free_hint?,
        );
        let mut hints: *mut *mut c_void = ptr::null_mut();
        // SAFETY: Passing -1 enumerates all cards for the "pcm" interface.
        let rc = unsafe { hint_fn(-1, c"pcm".as_ptr(), &mut hints) };
        if rc < 0 || hints.is_null() {
            return None;
        }
        let mut found = None;
        let mut seen = 0usize;
        // SAFETY: `hints` is a NULL-terminated array of opaque hint pointers.
        unsafe {
            let mut cursor = hints;
            while !(*cursor).is_null() {
                let hint = *cursor;
                let ioid = get_fn(hint, c"IOID".as_ptr());
                let is_output = if ioid.is_null() {
                    true
                } else {
                    let ok = CStr::from_ptr(ioid).to_bytes() != b"Input";
                    free(ioid.cast());
                    ok
                };
                if is_output {
                    let name = get_fn(hint, c"NAME".as_ptr());
                    if !name.is_null() {
                        let cstr = CStr::from_ptr(name).to_owned();
                        free(name.cast());
                        if cstr.as_bytes() != b"null" {
                            if seen == index {
                                found = Some(cstr);
                                break;
                            }
                            seen += 1;
                        }
                    }
                }
                cursor = cursor.add(1);
            }
            free_fn(hints);
        }
        found
    }
}

fn alsa_lib_override(engine: &Engine) -> Option<OsString> {
    // An empty value is `setenv('MATRUST_ALSA_LIB')`, which removes the override.
    engine
        .envx_child_variables()
        .get(ALSA_LIB_OVERRIDE)
        .map(OsString::from)
        .or_else(|| std::env::var_os(ALSA_LIB_OVERRIDE))
        .filter(|value| !value.is_empty())
}

fn load_alsa(override_path: Option<&OsStr>) -> std::result::Result<Arc<AlsaLib>, ()> {
    if let Some(path) = override_path {
        return AlsaLib::load(path).map(Arc::new);
    }
    DEFAULT_ALSA
        .get_or_init(|| AlsaLib::load(OsStr::new(ALSA_LIB_NAME)).map(Arc::new))
        .clone()
}

struct PcmHandle {
    lib: Arc<AlsaLib>,
    raw: *mut SndPcm,
}

impl Drop for PcmHandle {
    fn drop(&mut self) {
        // SAFETY: `self.raw` was opened with `snd_pcm_open` and is closed once on drop.
        unsafe {
            (self.lib.snd_pcm_close)(self.raw);
        }
    }
}

struct HwParams {
    lib: Arc<AlsaLib>,
    raw: *mut SndPcmHwParams,
}

impl HwParams {
    fn alloc(lib: Arc<AlsaLib>) -> std::result::Result<Self, ()> {
        let mut raw = ptr::null_mut();
        // SAFETY: Allocates an opaque `snd_pcm_hw_params_t` freed in `Drop`.
        let rc = unsafe { (lib.snd_pcm_hw_params_malloc)(&mut raw) };
        if rc < 0 || raw.is_null() {
            Err(())
        } else {
            Ok(Self { lib, raw })
        }
    }
}

impl Drop for HwParams {
    fn drop(&mut self) {
        // SAFETY: `self.raw` was allocated with `snd_pcm_hw_params_malloc`.
        unsafe {
            (self.lib.snd_pcm_hw_params_free)(self.raw);
        }
    }
}

#[derive(Clone, Copy)]
enum PcmFormat {
    Float,
    S32,
    S16,
}

struct PcmConfig {
    format: PcmFormat,
    channels: usize,
    rate: u32,
    period_frames: usize,
}

/// Session error for a failed `snd_pcm_open`.
///
/// `enabled.rs` caches `NO_OUTPUT` / `NO_INPUT` for the whole session, so only
/// a missing device (`-ENOENT`, `-ENODEV`) uses `missing`. `-EBUSY`, `-EAGAIN`
/// (a previous worker still holding an exclusive PCM) and every other open
/// failure return `OPEN_FAILED`, which is not cached.
fn pcm_open_error(rc: c_int, missing: &'static str) -> &'static str {
    if rc == -ENOENT || rc == -ENODEV {
        missing
    } else {
        OPEN_FAILED
    }
}

fn open_and_configure(
    lib: Arc<AlsaLib>,
    name: &CStr,
    stream: c_int,
    requested: Format,
    missing_error: &'static str,
) -> Result<(PcmHandle, PcmConfig)> {
    let mut raw = ptr::null_mut();
    // SAFETY: `name` is a valid NUL-terminated C string; `raw` receives the PCM handle.
    let rc = unsafe { (lib.snd_pcm_open)(&mut raw, name.as_ptr(), stream, SND_PCM_NONBLOCK) };
    if rc < 0 || raw.is_null() {
        return Err(pcm_open_error(rc, missing_error).into());
    }
    let pcm = PcmHandle {
        lib: Arc::clone(&lib),
        raw,
    };
    let params = HwParams::alloc(Arc::clone(&lib)).map_err(|()| OPEN_FAILED)?;
    // SAFETY: `pcm.raw` and `params.raw` are valid ALSA handles.
    unsafe {
        if (lib.snd_pcm_hw_params_any)(pcm.raw, params.raw) < 0 {
            return Err(OPEN_FAILED.into());
        }
        if (lib.snd_pcm_hw_params_set_access)(pcm.raw, params.raw, SND_PCM_ACCESS_RW_INTERLEAVED)
            < 0
        {
            return Err(OPEN_FAILED.into());
        }
        let format =
            if (lib.snd_pcm_hw_params_set_format)(pcm.raw, params.raw, SND_PCM_FORMAT_FLOAT) >= 0 {
                PcmFormat::Float
            } else if (lib.snd_pcm_hw_params_set_format)(pcm.raw, params.raw, SND_PCM_FORMAT_S32)
                >= 0
            {
                PcmFormat::S32
            } else if (lib.snd_pcm_hw_params_set_format)(pcm.raw, params.raw, SND_PCM_FORMAT_S16)
                >= 0
            {
                PcmFormat::S16
            } else {
                return Err(OPEN_FAILED.into());
            };
        let mut channels = requested.channels.max(1) as c_uint;
        if (lib.snd_pcm_hw_params_set_channels_near)(pcm.raw, params.raw, &mut channels) < 0
            || channels == 0
        {
            return Err(OPEN_FAILED.into());
        }
        let mut rate = requested.rate.max(1) as c_uint;
        let mut dir: c_int = 0;
        if (lib.snd_pcm_hw_params_set_rate_near)(pcm.raw, params.raw, &mut rate, &mut dir) < 0
            || rate == 0
        {
            return Err(OPEN_FAILED.into());
        }
        let mut period_us = TARGET_PERIOD_US;
        let _ = (lib.snd_pcm_hw_params_set_period_time_near)(
            pcm.raw,
            params.raw,
            &mut period_us,
            &mut dir,
        );
        let mut buffer_us = TARGET_BUFFER_US;
        let _ = (lib.snd_pcm_hw_params_set_buffer_time_near)(
            pcm.raw,
            params.raw,
            &mut buffer_us,
            &mut dir,
        );
        if (lib.snd_pcm_hw_params)(pcm.raw, params.raw) < 0 {
            return Err(OPEN_FAILED.into());
        }
        let mut period_frames: c_ulong = 0;
        let period_frames =
            if (lib.snd_pcm_hw_params_get_period_size)(params.raw, &mut period_frames, &mut dir)
                >= 0
                && period_frames > 0
            {
                (period_frames as usize).clamp(64, 4096)
            } else {
                (rate as usize / 50).clamp(64, 4096)
            };
        if (lib.snd_pcm_prepare)(pcm.raw) < 0 {
            return Err(OPEN_FAILED.into());
        }
        Ok((
            pcm,
            PcmConfig {
                format,
                channels: channels as usize,
                rate,
                period_frames,
            },
        ))
    }
}

fn is_recoverable_alsa_error(err: c_int) -> bool {
    matches!(err, x if x == -EPIPE || x == -ESTRPIPE || x == -EINTR)
}

#[allow(clippy::too_many_arguments)]
fn write_frames(
    pcm: &PcmHandle,
    format: PcmFormat,
    channels: usize,
    samples: &[f32],
    frames: usize,
    stop: &AtomicBool,
    scratch_i32: &mut Vec<i32>,
    scratch_i16: &mut Vec<i16>,
) -> std::result::Result<(), ()> {
    let total_samples = frames * channels;
    let (ptr, bytes_per_frame): (*const u8, usize) = match format {
        PcmFormat::Float => (samples.as_ptr().cast(), channels * size_of::<f32>()),
        PcmFormat::S32 => {
            scratch_i32.clear();
            scratch_i32.extend(samples[..total_samples].iter().map(|&s| {
                let clamped = s.clamp(-1.0, 1.0);
                (f64::from(clamped) * 2_147_483_648.0)
                    .round()
                    .clamp(i32::MIN as f64, i32::MAX as f64) as i32
            }));
            (scratch_i32.as_ptr().cast(), channels * size_of::<i32>())
        }
        PcmFormat::S16 => {
            scratch_i16.clear();
            scratch_i16.extend(samples[..total_samples].iter().map(|&s| {
                let clamped = s.clamp(-1.0, 1.0);
                (clamped * 32768.0)
                    .round()
                    .clamp(i16::MIN as f32, i16::MAX as f32) as i16
            }));
            (scratch_i16.as_ptr().cast(), channels * size_of::<i16>())
        }
    };

    let mut written = 0usize;
    while written < frames && !stop.load(Relaxed) {
        let remaining = frames - written;
        // SAFETY: `ptr` points to at least `frames * bytes_per_frame` bytes matching `format`.
        let rc = unsafe {
            (pcm.lib.snd_pcm_writei)(
                pcm.raw,
                ptr.add(written * bytes_per_frame).cast(),
                remaining as c_ulong,
            )
        };
        if rc > 0 {
            written += rc as usize;
        } else if rc == -c_long::from(EAGAIN) {
            // SAFETY: Valid PCM handle; wait up to 10 ms for space in the ring buffer.
            unsafe {
                (pcm.lib.snd_pcm_wait)(pcm.raw, WAIT_MS);
            }
        } else if rc == 0 {
            thread::park_timeout(POLL);
        } else {
            let err = rc as c_int;
            if is_recoverable_alsa_error(err) {
                // SAFETY: Valid PCM handle; `silent = 1` suppresses ALSA stderr output.
                let rec = unsafe { (pcm.lib.snd_pcm_recover)(pcm.raw, err, 1) };
                if rec < 0 {
                    return Err(());
                }
            } else {
                return Err(());
            }
        }
    }
    Ok(())
}

fn drain_playback(pcm: &PcmHandle, stop: &AtomicBool) {
    let deadline = Instant::now() + START_TIMEOUT;
    loop {
        if stop.load(Relaxed) {
            // SAFETY: Valid PCM handle; stop immediately on user stop/drop.
            unsafe {
                (pcm.lib.snd_pcm_drop)(pcm.raw);
            }
            return;
        }
        // SAFETY: Valid PCM handle.
        let rc = unsafe { (pcm.lib.snd_pcm_drain)(pcm.raw) };
        if rc != -EAGAIN || Instant::now() >= deadline {
            return;
        }
        thread::park_timeout(POLL);
    }
}

pub(in crate::engine::audio) struct Output {
    worker: Worker,
    render: Arc<Mutex<Render>>,
    range: Range<usize>,
    device_rate: u32,
}

impl Output {
    pub(in crate::engine::audio) fn start(
        engine: &Engine,
        clip: Clip,
        format: Format,
        range: Range<usize>,
    ) -> Result<Self> {
        let lib_override = alsa_lib_override(engine);
        let selection = range.clone();
        let (worker, (render, device_rate)) =
            Worker::start_custom(engine, START_TIMEOUT, move |stopped, ready| {
                let lib = match load_alsa(lib_override.as_deref()) {
                    Ok(lib) => lib,
                    Err(()) => {
                        let _ = ready.send(Err(NO_OUTPUT.into()));
                        return;
                    }
                };
                lib.silence_errors();
                let (device_name, missing_error) = if format.device < 0 {
                    (c"default".to_owned(), NO_OUTPUT)
                } else {
                    match lib.output_device_name(format.device as usize) {
                        Some(name) => (name, OPEN_FAILED),
                        None => {
                            let _ = ready.send(Err(DEVICE_ID_MISSING.into()));
                            return;
                        }
                    }
                };
                let (pcm_handle, cfg) = match open_and_configure(
                    lib,
                    &device_name,
                    SND_PCM_STREAM_PLAYBACK,
                    format,
                    missing_error,
                ) {
                    Ok(opened) => opened,
                    Err(err) => {
                        let _ = ready.send(Err(err));
                        return;
                    }
                };
                let render = Arc::new(Mutex::new(Render::default()));
                if ready.send(Ok((Arc::clone(&render), cfg.rate))).is_err() {
                    return;
                }

                let native_channels = cfg.channels;
                let native_rate = u64::from(cfg.rate);
                let step = u64::from(format.rate);
                let end = selection.len() as u64 * native_rate;
                let mut f32_buf = vec![0.0_f32; cfg.period_frames * native_channels];
                let mut scratch_i32 = Vec::new();
                let mut scratch_i16 = Vec::new();
                let mut was_paused = false;

                while !stopped.load(Relaxed) {
                    let (frames_to_write, done) = {
                        let mut state = render.lock().unwrap();
                        if state.failed {
                            break;
                        }
                        if state.paused {
                            if !was_paused {
                                // SAFETY: Valid PCM handle; drop buffered audio on pause.
                                unsafe {
                                    (pcm_handle.lib.snd_pcm_drop)(pcm_handle.raw);
                                }
                                was_paused = true;
                            }
                            (0, false)
                        } else {
                            if was_paused {
                                // SAFETY: Valid PCM handle; prepare stream before resuming.
                                if unsafe { (pcm_handle.lib.snd_pcm_prepare)(pcm_handle.raw) } < 0 {
                                    state.failed = true;
                                    break;
                                }
                                was_paused = false;
                            }
                            let mut count = 0usize;
                            let mut reached_end = false;
                            for frame in f32_buf.chunks_exact_mut(native_channels) {
                                let current =
                                    selection.start + (state.phase / native_rate) as usize;
                                let next = (current + 1).min(selection.end - 1);
                                let fraction =
                                    (state.phase % native_rate) as f32 / native_rate as f32;
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
                                    frame[0] = pcm((left + right) * 0.5, format.bits);
                                } else {
                                    frame[0] = pcm(left, format.bits);
                                    frame[1] = pcm(right, format.bits);
                                    for extra in &mut frame[2..] {
                                        *extra = 0.0;
                                    }
                                }
                                count += 1;
                                state.phase += step;
                                if state.phase >= end {
                                    reached_end = true;
                                    break;
                                }
                            }
                            (count, reached_end)
                        }
                    };

                    if frames_to_write == 0 {
                        thread::park_timeout(POLL);
                        continue;
                    }

                    if write_frames(
                        &pcm_handle,
                        cfg.format,
                        native_channels,
                        &f32_buf,
                        frames_to_write,
                        &stopped,
                        &mut scratch_i32,
                        &mut scratch_i16,
                    )
                    .is_err()
                    {
                        render.lock().unwrap().failed = true;
                        break;
                    }

                    if done {
                        drain_playback(&pcm_handle, &stopped);
                        render.lock().unwrap().drain = Some(Instant::now());
                        break;
                    }
                }

                if stopped.load(Relaxed) {
                    // SAFETY: Valid PCM handle; stop playback immediately on stop/drop.
                    unsafe {
                        (pcm_handle.lib.snd_pcm_drop)(pcm_handle.raw);
                    }
                }
            })?;
        Ok(Self {
            worker,
            render,
            range,
            device_rate,
        })
    }

    pub(in crate::engine::audio) fn status(&self) -> (bool, usize) {
        let render = self.render.lock().unwrap();
        if render.finished() {
            (false, 1)
        } else {
            let index =
                ((render.phase / u64::from(self.device_rate)) as usize).min(self.range.len() - 1);
            (!render.paused, self.range.start + index + 1)
        }
    }

    pub(in crate::engine::audio) fn finished(&self) -> bool {
        self.render.lock().unwrap().finished()
    }

    pub(in crate::engine::audio) fn failed(&self) -> bool {
        self.render.lock().unwrap().failed
    }

    pub(in crate::engine::audio) fn pause(&self, paused: bool) {
        let mut render = self.render.lock().unwrap();
        if render.drain.is_none() {
            render.paused = paused;
        }
        drop(render);
        self.worker.unpark();
    }
}

pub(super) fn start_input(
    engine: &Engine,
    format: Format,
    frames: usize,
) -> Result<(Worker, Arc<Mutex<Capture>>)> {
    let lib_override = alsa_lib_override(engine);
    let budget = crate::engine::limits::memory_bytes(&engine.limits)
        .saturating_sub(engine.workspace_bytes());
    let limit = frames.saturating_mul(format.channels);
    let memory_error = crate::engine::limits::memory_error("Audio recording", &engine.limits);
    Worker::start_custom(engine, START_TIMEOUT, move |stopped, ready| {
        let lib = match load_alsa(lib_override.as_deref()) {
            Ok(lib) => lib,
            Err(()) => {
                let _ = ready.send(Err(NO_INPUT.into()));
                return;
            }
        };
        lib.silence_errors();
        let (pcm_handle, cfg) =
            match open_and_configure(lib, c"default", SND_PCM_STREAM_CAPTURE, format, NO_INPUT) {
                Ok(opened) => opened,
                Err(err) => {
                    let _ = ready.send(Err(err));
                    return;
                }
            };
        if limit.saturating_mul(size_of::<f32>()) > budget {
            let _ = ready.send(Err(memory_error));
            return;
        }
        let mut values = Vec::new();
        if values.try_reserve_exact(limit).is_err() {
            let _ = ready.send(Err(
                "Audio recording could not allocate its sample buffer.".into()
            ));
            return;
        }
        let capture = Arc::new(Mutex::new(Capture {
            values,
            limit,
            running: true,
            failed: false,
            mark: 0,
            mark_at: None,
        }));
        if ready.send(Ok(Arc::clone(&capture))).is_err() {
            return;
        }

        let native_channels = cfg.channels;
        let native_rate = u64::from(cfg.rate);
        let rate = u64::from(format.rate);
        let mut native_time = 0_u64;
        let mut output_time = 0_u64;
        let mut previous = [0.0_f32; 2];

        let sample_count = cfg.period_frames * native_channels;
        let mut raw_f32 = vec![0.0_f32; sample_count];
        let mut raw_i32 = vec![
            0_i32;
            if matches!(cfg.format, PcmFormat::S32) {
                sample_count
            } else {
                0
            }
        ];
        let mut raw_i16 = vec![
            0_i16;
            if matches!(cfg.format, PcmFormat::S16) {
                sample_count
            } else {
                0
            }
        ];

        while !stopped.load(Relaxed) {
            {
                let mut state = capture.lock().unwrap();
                state.fail_if_silent(START_TIMEOUT);
                if !state.running {
                    break;
                }
            }
            let buf_ptr: *mut c_void = match cfg.format {
                PcmFormat::Float => raw_f32.as_mut_ptr().cast(),
                PcmFormat::S32 => raw_i32.as_mut_ptr().cast(),
                PcmFormat::S16 => raw_i16.as_mut_ptr().cast(),
            };
            // SAFETY: `buf_ptr` holds `cfg.period_frames` interleaved frames of `cfg.format`.
            let rc = unsafe {
                (pcm_handle.lib.snd_pcm_readi)(
                    pcm_handle.raw,
                    buf_ptr,
                    cfg.period_frames as c_ulong,
                )
            };
            if rc > 0 {
                let got_frames = rc as usize;
                let got_samples = got_frames * native_channels;
                match cfg.format {
                    PcmFormat::Float => {}
                    PcmFormat::S32 => {
                        for (dst, &src) in raw_f32[..got_samples]
                            .iter_mut()
                            .zip(&raw_i32[..got_samples])
                        {
                            *dst = (f64::from(src) / 2_147_483_648.0) as f32;
                        }
                    }
                    PcmFormat::S16 => {
                        for (dst, &src) in raw_f32[..got_samples]
                            .iter_mut()
                            .zip(&raw_i16[..got_samples])
                        {
                            *dst = f32::from(src) / 32768.0;
                        }
                    }
                }
                let mut state = capture.lock().unwrap();
                if !state.running {
                    break;
                }
                for frame in raw_f32[..got_samples].chunks_exact(native_channels) {
                    let left = frame[0];
                    let right = if native_channels >= 2 { frame[1] } else { left };
                    let next = if format.channels == 1 {
                        [(left + right) * 0.5, 0.0]
                    } else {
                        [left, right]
                    };
                    while output_time <= native_time {
                        let fraction = 1.0 - (native_time - output_time) as f32 / rate as f32;
                        for channel in 0..format.channels {
                            let value = interpolated(previous[channel], next[channel], fraction);
                            state.values.push(pcm(
                                if value.is_finite() { value } else { 0.0 },
                                format.bits,
                            ));
                        }
                        output_time += native_rate;
                        if state.values.len() == state.limit {
                            state.running = false;
                            break;
                        }
                    }
                    if !state.running {
                        break;
                    }
                    previous = next;
                    native_time += rate;
                }
            } else if rc == -c_long::from(EAGAIN) {
                // SAFETY: Valid PCM handle; wait up to 10 ms for captured frames.
                unsafe {
                    (pcm_handle.lib.snd_pcm_wait)(pcm_handle.raw, WAIT_MS);
                }
            } else if rc == 0 {
                thread::park_timeout(POLL);
            } else {
                let err = rc as c_int;
                if is_recoverable_alsa_error(err) {
                    // SAFETY: Valid PCM handle; `silent = 1` suppresses ALSA stderr output.
                    let rec = unsafe { (pcm_handle.lib.snd_pcm_recover)(pcm_handle.raw, err, 1) };
                    if rec < 0 {
                        let mut state = capture.lock().unwrap();
                        if state.running {
                            state.running = false;
                            state.failed = true;
                        }
                        break;
                    }
                } else {
                    let mut state = capture.lock().unwrap();
                    if state.running {
                        state.running = false;
                        state.failed = true;
                    }
                    break;
                }
            }
        }
        // SAFETY: Valid PCM handle; stop capture before closing.
        unsafe {
            (pcm_handle.lib.snd_pcm_drop)(pcm_handle.raw);
        }
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn xrun_and_suspend_errors_are_recoverable() {
        assert!(is_recoverable_alsa_error(-EPIPE));
        assert!(is_recoverable_alsa_error(-ESTRPIPE));
        assert!(is_recoverable_alsa_error(-EINTR));
        assert!(!is_recoverable_alsa_error(-EAGAIN));
        assert!(!is_recoverable_alsa_error(-19)); // -ENODEV
    }

    #[test]
    fn missing_alsa_library_path_fails_cleanly() {
        assert!(load_alsa(Some(OsStr::new("/nonexistent/libasound.so.2"))).is_err());
    }

    #[test]
    fn open_errno_maps_only_a_missing_device_to_no_device() {
        const EIO: c_int = 5;
        const EBUSY: c_int = 16;
        const EINVAL: c_int = 22;
        assert_eq!(pcm_open_error(-ENOENT, NO_OUTPUT), NO_OUTPUT);
        assert_eq!(pcm_open_error(-ENODEV, NO_OUTPUT), NO_OUTPUT);
        assert_eq!(pcm_open_error(-ENOENT, NO_INPUT), NO_INPUT);
        assert_eq!(pcm_open_error(-ENODEV, NO_INPUT), NO_INPUT);
        assert_eq!(pcm_open_error(-EBUSY, NO_OUTPUT), OPEN_FAILED);
        assert_eq!(pcm_open_error(-EAGAIN, NO_OUTPUT), OPEN_FAILED);
        assert_eq!(pcm_open_error(-EBUSY, NO_INPUT), OPEN_FAILED);
        assert_eq!(pcm_open_error(-EAGAIN, NO_INPUT), OPEN_FAILED);
        assert_eq!(pcm_open_error(-EINVAL, NO_OUTPUT), OPEN_FAILED);
        assert_eq!(pcm_open_error(-EIO, NO_OUTPUT), OPEN_FAILED);
        assert_eq!(pcm_open_error(0, NO_OUTPUT), OPEN_FAILED);
        // A named DeviceID passes `OPEN_FAILED` as its missing-device error.
        assert_eq!(pcm_open_error(-ENOENT, OPEN_FAILED), OPEN_FAILED);
    }
}
