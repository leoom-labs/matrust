//! Clocks for the engine. std's Instant and SystemTime::now panic on
//! wasm32-unknown-unknown; there the browser clocks (web-time) stand in.
#[cfg(not(target_arch = "wasm32"))]
pub(crate) use std::time::Instant;
#[cfg(target_arch = "wasm32")]
pub(crate) use web_time::Instant;

use std::time::SystemTime;

#[cfg(not(target_arch = "wasm32"))]
pub(crate) fn system_now() -> SystemTime {
    SystemTime::now()
}

/// The browser's Date.now() as a std SystemTime, so callers keep one type.
#[cfg(target_arch = "wasm32")]
pub(crate) fn system_now() -> SystemTime {
    let since = web_time::SystemTime::now()
        .duration_since(web_time::UNIX_EPOCH)
        .unwrap_or_default();
    std::time::UNIX_EPOCH + since
}
