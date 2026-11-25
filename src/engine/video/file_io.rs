//! Validate native video paths without blocking on named pipes.
use crate::Result;
use std::{fs, path::Path};

pub(super) fn check_output(path: &Path) -> Result<()> {
    match fs::metadata(path) {
        Ok(m) if !m.is_file() => Err("VideoWriter: output must be a regular file".into()),
        Ok(_) => Ok(()),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(e) => Err(format!("VideoWriter: cannot inspect output file: {e}")),
    }
}

pub(super) fn open_input(path: &Path) -> Result<fs::File> {
    let error = |e| format!("VideoReader: cannot open file: {e}");
    // Also gives sockets/devices/directories a consistent diagnostic. On Unix
    // the nonblocking open and handle check below close the path-swap race.
    if !fs::metadata(path).map_err(error)?.is_file() {
        return Err("VideoReader: input must be a regular file".into());
    }
    let mut options = fs::OpenOptions::new();
    options.read(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        // libc is not a direct dependency. These are the platform O_NONBLOCK
        // ABI values (including Linux's MIPS and SPARC exceptions).
        let nonblock = match std::env::consts::OS {
            "linux" => match std::env::consts::ARCH {
                "mips" | "mips64" | "mips32r6" | "mips64r6" => 0x80,
                "sparc" | "sparc64" => 0x4000,
                _ => 0x800,
            },
            "android" | "emscripten" | "l4re" => 0x800,
            "macos" | "ios" | "tvos" | "watchos" | "visionos" | "freebsd" | "openbsd"
            | "netbsd" | "dragonfly" | "aix" => 4,
            "solaris" | "illumos" | "haiku" | "nto" => 0x80,
            "hurd" => 8,
            "redox" => 0x40000,
            "cygwin" | "espidf" | "horizon" | "vita" => 0x4000,
            _ => {
                return Err(
                    "VideoReader: nonblocking file open is unsupported on this Unix platform"
                        .into(),
                );
            }
        };
        options.custom_flags(nonblock);
    }
    let file = options.open(path).map_err(error)?;
    if !file.metadata().map_err(error)?.is_file() {
        return Err("VideoReader: input must be a regular file".into());
    }
    Ok(file)
}
