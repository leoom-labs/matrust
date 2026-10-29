//! Bounded native monitor metadata. No callback allocation, locks or UI calls.
use matrust::session::figure_window::MonitorArea;
#[cfg(windows)]
use matrust::session::figure_window::{MAX_MONITORS, PhysicalRect};

#[cfg(windows)]
pub(super) fn enumerate() -> Option<Vec<MonitorArea>> {
    use windows::Win32::{
        Foundation::{LPARAM, RECT},
        Graphics::Gdi::*,
    };
    struct Buffer {
        entries: [MONITORINFOEXW; MAX_MONITORS],
        len: usize,
        failed: bool,
    }
    // EnumDisplayMonitors is synchronous. The stack buffer outlives every call;
    // no pointer escapes. All indexing is checked; callback never allocates.
    unsafe extern "system" fn collect(
        monitor: HMONITOR,
        _: HDC,
        _: *mut RECT,
        data: LPARAM,
    ) -> windows::core::BOOL {
        let Some(buffer) = (unsafe { (data.0 as *mut Buffer).as_mut() }) else {
            return false.into();
        };
        let Some(slot) = buffer.entries.get_mut(buffer.len) else {
            buffer.failed = true;
            return false.into();
        };
        slot.monitorInfo.cbSize = std::mem::size_of::<MONITORINFOEXW>() as u32;
        if !unsafe { GetMonitorInfoW(monitor, &mut slot.monitorInfo) }.as_bool() {
            buffer.failed = true;
            return false.into();
        }
        buffer.len += 1;
        true.into()
    }
    let mut buffer = Buffer {
        entries: [MONITORINFOEXW::default(); MAX_MONITORS],
        len: 0,
        failed: false,
    };
    let success = unsafe {
        EnumDisplayMonitors(
            None,
            None,
            Some(collect),
            LPARAM((&mut buffer as *mut Buffer) as isize),
        )
    }
    .as_bool();
    if !success || buffer.failed || buffer.len == 0 {
        return None;
    }
    let mut result = Vec::with_capacity(buffer.len);
    for entry in &buffer.entries[..buffer.len] {
        let r = entry.monitorInfo.rcWork;
        let work =
            PhysicalRect::new([r.left as f64, r.top as f64, r.right as f64, r.bottom as f64])
                .ok()?;
        let end = entry
            .szDevice
            .iter()
            .position(|&c| c == 0)
            .unwrap_or(entry.szDevice.len());
        let device = String::from_utf16(&entry.szDevice[..end]).ok()?;
        result.push(MonitorArea {
            work,
            device: (!device.is_empty()).then_some(device),
            primary: entry.monitorInfo.dwFlags & 1 != 0,
        });
    }
    Some(result)
}

#[cfg(not(windows))]
pub(super) fn enumerate() -> Option<Vec<MonitorArea>> {
    None
}

/// Minimum physical outer size and title-control height at observed CHILD DPI.
#[cfg(windows)]
pub(super) fn controls(native: f64) -> Option<Controls> {
    use windows::Win32::UI::{HiDpi::GetSystemMetricsForDpi, WindowsAndMessaging::*};
    if !native.is_finite() || !(0.25..=16.).contains(&native) {
        return None;
    }
    let dpi = (native * 96.).round() as u32;
    let metric = |index| {
        let n = unsafe { GetSystemMetricsForDpi(index, dpi) };
        (n > 0).then_some(n as f64)
    };
    // Three caption buttons plus a small drag region, in addition to OS minimum.
    Some(Controls {
        minimum: [
            metric(SM_CXMINTRACK)?.max(4. * metric(SM_CXSIZE)?),
            metric(SM_CYMINTRACK)?,
        ],
        title: metric(SM_CYCAPTION)?,
    })
}

#[cfg(not(windows))]
pub(super) fn controls(_: f64) -> Option<Controls> {
    None
}

#[derive(Clone, Copy, Debug)]
pub(super) struct Controls {
    pub minimum: [f64; 2],
    pub title: f64,
}
