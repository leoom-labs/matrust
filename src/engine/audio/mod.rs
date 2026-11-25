//! Script audio builtins. The native device backend is optional.

#[cfg(all(feature = "audio", not(target_arch = "wasm32")))]
mod enabled;
#[cfg(all(feature = "audio", not(target_arch = "wasm32")))]
mod native;

#[cfg(all(feature = "audio", not(target_arch = "wasm32")))]
pub(super) use enabled::{Handle, Session, sound, text};

#[cfg(all(feature = "audio", not(target_arch = "wasm32")))]
pub use enabled::audio_output_sample;

#[cfg(not(all(feature = "audio", not(target_arch = "wasm32"))))]
pub(super) fn unavailable(
    engine: &mut super::Engine,
    out: &mut super::Execution,
) -> crate::Result<super::Value> {
    engine.lng_warn(
        "Audio output is not available in this build.",
        "MatRust:audio:Unavailable",
        out,
    )?;
    Ok(super::Value::Matrix(crate::Matrix::filled(0, 0, 0.)?))
}
