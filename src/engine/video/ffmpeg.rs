//! Executable-only video backend. No shell, linked codec, or unbounded capture.
use super::super::Engine;
use crate::Result;
use std::{
    path::{Path, PathBuf},
    sync::Arc,
};
#[cfg(not(target_arch = "wasm32"))]
mod process;
#[cfg(not(target_arch = "wasm32"))]
pub(super) use process::{Config, FrameSpec, Sink, Source};

#[derive(Clone, Default)]
pub(super) struct Tools {
    pub ffmpeg: Option<PathBuf>,
    pub ffprobe: Option<PathBuf>,
}
pub(super) fn missing(who: &str, kind: &str) -> String {
    if cfg!(target_arch = "wasm32") {
        format!("{who}: ffmpeg is not available in this build")
    } else {
        format!(
            "{who}: {} {kind} needs ffmpeg on PATH (not found). Native: AVI with Motion JPEG or uncompressed video.",
            if who == "VideoReader" {
                "reading"
            } else {
                "writing"
            }
        )
    }
}
#[cfg(not(target_arch = "wasm32"))]
fn environment(e: &Engine, key: &str) -> Option<std::ffi::OsString> {
    e.envx_child_variables()
        .get(key)
        .map(std::ffi::OsString::from)
        .or_else(|| std::env::var_os(key))
}
#[cfg(not(target_arch = "wasm32"))]
fn locate(
    name: &str,
    variable: &str,
    override_path: &Option<std::ffi::OsString>,
    search_path: &Option<std::ffi::OsString>,
) -> Result<Option<PathBuf>> {
    let executable = |p: &Path| {
        if !p.is_file() {
            return false;
        }
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            p.metadata()
                .is_ok_and(|m| m.permissions().mode() & 0o111 != 0)
        }
        #[cfg(not(unix))]
        {
            true
        }
    };
    if let Some(p) = override_path.as_ref().filter(|p| !p.is_empty()) {
        let p = PathBuf::from(p);
        if !p.is_absolute() {
            return Err(format!("{variable} must be an absolute path"));
        }
        return Ok(executable(&p).then_some(p));
    }
    let name = if cfg!(windows) {
        format!("{name}.exe")
    } else {
        name.into()
    };
    Ok(
        std::env::split_paths(&search_path.clone().unwrap_or_default())
            .filter(|dir| dir.is_absolute())
            .map(|dir| dir.join(&name))
            .find(|p| executable(p)),
    )
}
/// A weak engine identity avoids changing the Engine layout or retaining engines.
/// Changing PATH or either override resets that engine's cache.
pub(super) fn tools(e: &Engine) -> Result<Arc<Tools>> {
    #[cfg(target_arch = "wasm32")]
    {
        let _ = e;
        Ok(Arc::new(Tools::default()))
    }
    #[cfg(not(target_arch = "wasm32"))]
    {
        use std::sync::{Mutex, OnceLock, Weak, atomic::AtomicBool};
        type Entry = (
            Weak<AtomicBool>,
            Vec<Option<std::ffi::OsString>>,
            Arc<Tools>,
        );
        static CACHE: OnceLock<Mutex<Vec<Entry>>> = OnceLock::new();
        let identity = e.cancellation();
        let keys = ["MATRUST_FFMPEG", "MATRUST_FFPROBE", "PATH"]
            .map(|k| environment(e, k))
            .to_vec();
        let mut cache = CACHE
            .get_or_init(Mutex::default)
            .lock()
            .unwrap_or_else(|p| p.into_inner());
        cache.retain(|(id, _, _)| id.strong_count() != 0);
        if let Some((_, _, t)) = cache
            .iter()
            .find(|(id, k, _)| id.ptr_eq(&Arc::downgrade(&identity)) && *k == keys)
        {
            return Ok(t.clone());
        }
        cache.retain(|(id, _, _)| !id.ptr_eq(&Arc::downgrade(&identity)));
        let t = Arc::new(Tools {
            ffmpeg: locate("ffmpeg", "MATRUST_FFMPEG", &keys[0], &keys[2])?,
            ffprobe: locate("ffprobe", "MATRUST_FFPROBE", &keys[1], &keys[2])?,
        });
        cache.push((Arc::downgrade(&identity), keys, t.clone()));
        Ok(t)
    }
}
pub(super) fn open(e: &Engine, path: &Path, budget: usize) -> Result<Arc<dyn super::VideoSource>> {
    let t = tools(e)?;
    let kind = path.extension().unwrap_or_default().to_string_lossy();
    if t.ffmpeg.is_none() {
        return Err(missing("VideoReader", &kind));
    }
    if t.ffprobe.is_none() {
        return Err(format!(
            "{} ffprobe was not found.",
            missing("VideoReader", &kind)
        ));
    }
    #[cfg(not(target_arch = "wasm32"))]
    {
        Ok(Arc::new(Source::open(t, path, budget, &e.cancellation())?))
    }
    #[cfg(target_arch = "wasm32")]
    {
        let _ = budget;
        Err(missing("VideoReader", &kind))
    }
}
