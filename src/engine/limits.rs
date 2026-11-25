//! Run limits per host. The browser keeps fixed step/output/memory budgets; the desktop app
//! and the CLI run until Stop or Ctrl+C, with memory guards sized from physical RAM.

/// `None` means no limit for an optional budget. Recursion depth (128 levels) is not part of these limits.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Limits {
    /// Statement and loop steps per run.
    pub steps: Option<usize>,
    /// Output entries per run.
    pub output_entries: Option<usize>,
    /// Output bytes per run.
    pub output_bytes: Option<usize>,
    /// Maximum bytes in one displayed value, independent of the per-run output budget.
    pub display_bytes: usize,
    /// Per-statement evaluation budget in 8-byte units (see `Engine::charge`).
    pub evaluation_units: Option<usize>,
    /// Bytes retained by the inputs of active `for` loops.
    pub loop_bytes: Option<usize>,
    /// Memory budget in bytes for one statement's scratch and the workspace
    /// (read through `memory_bytes`).
    pub memory_bytes: usize,
}

const MIB: usize = 1024 * 1024;
const BROWSER_EVALUATION_UNITS: usize = 32 * MIB;
const BROWSER_LOOP_BYTES: usize = 256 * MIB;
const BROWSER_MEMORY_BYTES: usize = 256 * MIB;
/// Ceiling of the local memory budget, whatever the physical RAM.
#[cfg(target_pointer_width = "64")]
const LOCAL_MEMORY_CAP: usize = 4096 * MIB;
/// On 32-bit targets (wasm32) 4 GiB does not fit in `usize`; the local cap stays at the browser budget.
#[cfg(not(target_pointer_width = "64"))]
const LOCAL_MEMORY_CAP: usize = BROWSER_MEMORY_BYTES;

impl Default for Limits {
    fn default() -> Self {
        Self::browser()
    }
}

impl Limits {
    /// The fixed budgets of the browser console (and of `Engine::default()`).
    pub const fn browser() -> Self {
        Self {
            steps: Some(1_000_000),
            output_entries: Some(2000),
            output_bytes: Some(2 * MIB),
            display_bytes: 2 * MIB,
            evaluation_units: Some(BROWSER_EVALUATION_UNITS),
            loop_bytes: Some(BROWSER_LOOP_BYTES),
            memory_bytes: BROWSER_MEMORY_BYTES,
        }
    }
    /// Desktop app and CLI: no step or per-run output limit; memory guards of half the physical RAM.
    pub fn local() -> Self {
        local_for_memory(physical_memory())
    }
}

/// Local limits for a machine with `memory` bytes of physical RAM (`None`: unknown, so the
/// memory guards keep the browser values). The guards never drop below the browser values.
pub fn local_for_memory(memory: Option<u64>) -> Limits {
    let half = memory
        .map(|bytes| usize::try_from(bytes / 2).unwrap_or(usize::MAX))
        .unwrap_or(0);
    Limits {
        steps: None,
        output_entries: None,
        output_bytes: None,
        display_bytes: 32 * MIB,
        evaluation_units: Some((half / 8).max(BROWSER_EVALUATION_UNITS)),
        loop_bytes: Some(half.max(BROWSER_LOOP_BYTES)),
        memory_bytes: half.clamp(BROWSER_MEMORY_BYTES, LOCAL_MEMORY_CAP),
    }
}

/// Memory budget of one statement's scratch and of the workspace: 256 MiB
/// under the default (browser) limits; half the physical RAM, at most 4 GiB,
/// under `Limits::local()`.
pub fn memory_bytes(limits: &Limits) -> usize {
    limits.memory_bytes
}

/// The memory budget in 8-byte units (element-count checks).
pub(super) fn memory_units(limits: &Limits) -> usize {
    memory_bytes(limits) / 8
}

/// Error text naming the actual budget: "`what` exceeds the N MiB memory limit".
pub(super) fn memory_error(what: &str, limits: &Limits) -> String {
    format!(
        "{what} exceeds the {} MiB memory limit",
        mib(memory_bytes(limits))
    )
}

pub(super) fn workspace_error(limits: &Limits) -> String {
    format!(
        "Workspace exceeds its {} MiB logical storage limit; use clear",
        mib(memory_bytes(limits))
    )
}

/// Size in MiB (rounded down) for error texts.
fn mib(bytes: usize) -> usize {
    bytes / MIB
}

pub(super) fn steps_error(max: usize) -> String {
    format!("Execution exceeds {max} statement/loop steps; split the work or use array operations")
}

pub(super) fn output_error(limits: &Limits) -> String {
    let what = match (limits.output_entries, limits.output_bytes) {
        (Some(entries), Some(bytes)) => format!("{entries} entries or {} MiB", mib(bytes)),
        (Some(entries), None) => format!("{entries} entries"),
        (None, Some(bytes)) => format!("{} MiB", mib(bytes)),
        (None, None) => "its limit".into(),
    };
    format!("Execution output exceeds {what}; use semicolons to suppress output")
}

pub(super) fn evaluation_error(units: usize) -> String {
    format!(
        "Statement exceeds its {} MiB evaluation budget; split the calculation into smaller steps",
        mib(units.saturating_mul(8))
    )
}

pub(super) fn loop_bytes_error(bytes: usize) -> String {
    format!("Active for-loop inputs exceed {} MiB", mib(bytes))
}

/// Physical RAM in bytes, or `None` when the platform does not report it.
pub fn physical_memory() -> Option<u64> {
    platform_memory().filter(|&bytes| bytes > 0)
}

#[cfg(any(target_os = "linux", target_os = "android"))]
fn platform_memory() -> Option<u64> {
    let text = std::fs::read_to_string("/proc/meminfo").ok()?;
    meminfo_total(&text)
}

/// `MemTotal` of a `/proc/meminfo` text, in bytes.
#[cfg_attr(
    not(any(target_os = "linux", target_os = "android", test)),
    allow(dead_code)
)]
pub(super) fn meminfo_total(text: &str) -> Option<u64> {
    let line = text
        .lines()
        .find_map(|line| line.strip_prefix("MemTotal:"))?;
    let kib = line.trim().strip_suffix("kB")?.trim().parse::<u64>().ok()?;
    kib.checked_mul(1024)
}

#[cfg(windows)]
fn platform_memory() -> Option<u64> {
    #[repr(C)]
    struct MemoryStatusEx {
        length: u32,
        memory_load: u32,
        total_phys: u64,
        avail_phys: u64,
        total_page_file: u64,
        avail_page_file: u64,
        total_virtual: u64,
        avail_virtual: u64,
        avail_extended_virtual: u64,
    }
    #[link(name = "kernel32")]
    unsafe extern "system" {
        fn GlobalMemoryStatusEx(buffer: *mut MemoryStatusEx) -> i32;
    }
    let mut status = MemoryStatusEx {
        length: std::mem::size_of::<MemoryStatusEx>() as u32,
        memory_load: 0,
        total_phys: 0,
        avail_phys: 0,
        total_page_file: 0,
        avail_page_file: 0,
        total_virtual: 0,
        avail_virtual: 0,
        avail_extended_virtual: 0,
    };
    // SAFETY: `status` is a correctly sized MEMORYSTATUSEX with `dwLength` set.
    let ok = unsafe { GlobalMemoryStatusEx(&mut status) };
    (ok != 0).then_some(status.total_phys)
}

#[cfg(target_os = "macos")]
fn platform_memory() -> Option<u64> {
    use std::ffi::{c_char, c_int, c_void};
    unsafe extern "C" {
        fn sysctlbyname(
            name: *const c_char,
            old: *mut c_void,
            old_len: *mut usize,
            new: *mut c_void,
            new_len: usize,
        ) -> c_int;
    }
    let mut bytes = 0u64;
    let mut len = std::mem::size_of::<u64>();
    // SAFETY: NUL-terminated name; the output buffer is a u64 and `len` holds its size.
    let result = unsafe {
        sysctlbyname(
            c"hw.memsize".as_ptr(),
            (&mut bytes as *mut u64).cast(),
            &mut len,
            std::ptr::null_mut(),
            0,
        )
    };
    (result == 0 && len == std::mem::size_of::<u64>()).then_some(bytes)
}

#[cfg(not(any(
    target_os = "linux",
    target_os = "android",
    windows,
    target_os = "macos"
)))]
fn platform_memory() -> Option<u64> {
    None
}
