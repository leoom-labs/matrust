//! Runs only in the already job-admitted, one-shot preview helper.
use super::eps_protocol as wire;
use matrust::Result;
use std::{
    io::{Read, Write},
    process::{Child, Command, Stdio},
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
    time::{Duration, Instant},
};

struct OwnedChild(Child);
impl Drop for OwnedChild {
    fn drop(&mut self) {
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}
fn drain(mut reader: impl Read, limit: usize, exceeded: &AtomicBool) -> Result<Vec<u8>> {
    let mut result = Vec::new();
    let mut buffer = [0; 16 * 1024];
    loop {
        let count = reader.read(&mut buffer).map_err(|e| e.to_string())?;
        if count == 0 {
            return Ok(result);
        }
        if result.len().saturating_add(count) > limit {
            exceeded.store(true, Ordering::Relaxed);
        } else if !exceeded.load(Ordering::Relaxed) {
            result.try_reserve(count).map_err(|e| e.to_string())?;
            result.extend_from_slice(&buffer[..count]);
        }
    }
}
fn valid_version(bytes: &[u8]) -> bool {
    if bytes.len() > 32 {
        return false;
    }
    let Ok(text) = std::str::from_utf8(bytes) else {
        return false;
    };
    let fields: Vec<_> = text.trim().split('.').collect();
    if fields.len() != 3
        || fields
            .iter()
            .any(|s| s.is_empty() || s.len() > 3 || !s.bytes().all(|b| b.is_ascii_digit()))
    {
        return false;
    }
    let values: Vec<_> = fields
        .iter()
        .filter_map(|s| s.parse::<u32>().ok())
        .collect();
    values.len() == 3 && (values[0], values[1]) >= (10, 8)
}
fn probe(tool: &std::path::Path) -> Result<()> {
    let mut command = Command::new(tool);
    command
        .env_clear()
        .arg("--version")
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    if let Some(root) = std::env::var_os("SystemRoot") {
        command.env("SystemRoot", root);
    }
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        command.creation_flags(windows::Win32::System::Threading::DETACHED_PROCESS.0);
    }
    let mut child = OwnedChild(
        command
            .spawn()
            .map_err(|e| format!("Could not validate configured Ghostscript: {e}"))?,
    );
    let output = child.0.stdout.take().ok_or("Missing version output")?;
    let error = child.0.stderr.take().ok_or("Missing version diagnostics")?;
    let exceeded = Arc::new(AtomicBool::new(false));
    let out_limit = exceeded.clone();
    let err_limit = exceeded.clone();
    let reader = std::thread::Builder::new()
        .name("eps-version".into())
        .spawn(move || drain(output, wire::MAX_LOG, &out_limit))
        .map_err(|e| e.to_string())?;
    let errors = std::thread::Builder::new()
        .name("eps-version-log".into())
        .spawn(move || drain(error, wire::MAX_LOG, &err_limit))
        .map_err(|e| e.to_string())?;
    let start = Instant::now();
    let status = loop {
        if exceeded.load(Ordering::Relaxed) || start.elapsed() >= Duration::from_secs(3) {
            let _ = child.0.kill();
            break child.0.wait().map_err(|e| e.to_string())?;
        }
        if let Some(status) = child.0.try_wait().map_err(|e| e.to_string())? {
            break status;
        }
        std::thread::sleep(Duration::from_millis(10));
    };
    let version = reader.join().map_err(|_| "EPS version reader stopped")??;
    let _ = errors.join().map_err(|_| "EPS version log stopped")??;
    if !status.success()
        || exceeded.load(Ordering::Relaxed)
        || start.elapsed() >= Duration::from_secs(3)
        || !valid_version(&version)
    {
        return Err(
            "Choose an installed Ghostscript console executable, version 10.08.0 or newer".into(),
        );
    }
    Ok(())
}
fn scratch_within_limit(path: &std::path::Path) -> Result<bool> {
    let mut count = 0usize;
    let mut bytes = 0u64;
    for entry in std::fs::read_dir(path).map_err(|e| e.to_string())? {
        let entry = entry.map_err(|e| e.to_string())?;
        count += 1;
        if count > 64 {
            return Ok(false);
        }
        let kind = match entry.file_type() {
            Ok(kind) => kind,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => continue,
            Err(error) => return Err(error.to_string()),
        };
        if !kind.is_file() {
            return Ok(false);
        }
        let metadata = match entry.metadata() {
            Ok(metadata) => metadata,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => continue,
            Err(error) => return Err(error.to_string()),
        };
        bytes = bytes.saturating_add(metadata.len());
        if bytes > 64 * 1024 * 1024 {
            return Ok(false);
        }
    }
    Ok(true)
}
fn render(request: &wire::Request, bytes: Vec<u8>) -> Result<Vec<u8>> {
    request.validate()?;
    if !cfg!(windows) {
        return Err("EPS preview currently requires Windows".into());
    }
    let tool = std::fs::canonicalize(&request.renderer)
        .map_err(|e| format!("Configured EPS renderer is unavailable: {e}"))?;
    if !tool.is_file() {
        return Err("Choose the installed Ghostscript console executable".into());
    }
    let before = std::fs::metadata(&tool).map_err(|e| e.to_string())?;
    probe(&tool)?;
    let after = std::fs::metadata(&tool).map_err(|e| e.to_string())?;
    if before.len() != after.len()
        || before.modified().map_err(|e| e.to_string())?
            != after.modified().map_err(|e| e.to_string())?
    {
        return Err("Configured renderer changed during validation; choose it again".into());
    }
    let private = std::fs::canonicalize(&request.scratch).map_err(|e| e.to_string())?;
    if !private.is_dir() {
        return Err("EPS scratch directory is unavailable".into());
    }
    let mut command = Command::new(&tool);
    // No shell, user flags, GS_OPTIONS, GS_LIB, PATH search or repository tools.
    command.env_clear();
    if let Some(root) = std::env::var_os("SystemRoot") {
        command.env("SystemRoot", root);
    }
    command
        .env("TEMP", &private)
        .env("TMP", &private)
        .current_dir(&private);
    command
        .args([
            "-q",
            "-dSAFER",
            "-dBATCH",
            "-dNOPAUSE",
            "-dNOPROMPT",
            "-sDEVICE=png16m",
            "--permit-devices=png16m",
            "-sOutputFile=%stdout",
            "-sstdout=%stderr",
            "-dEPSCrop",
            "-dPSFitPage",
            "-dFIXEDMEDIA",
            "-r72",
            "-dFirstPage=1",
            "-dLastPage=1",
            "-dNumRenderingThreads=1",
        ])
        .arg(format!("-g{}x{}", request.side, request.side))
        .args(["-f", "-"])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        // No BREAKAWAY_FROM_JOB: admission is inherited from the helper.
        command.creation_flags(windows::Win32::System::Threading::DETACHED_PROCESS.0);
    }
    let mut child = OwnedChild(
        command
            .spawn()
            .map_err(|e| format!("Could not start configured Ghostscript: {e}"))?,
    );
    let mut input = child.0.stdin.take().ok_or("EPS stdin unavailable")?;
    let output = child.0.stdout.take().ok_or("EPS stdout unavailable")?;
    let error = child.0.stderr.take().ok_or("EPS stderr unavailable")?;
    let exceeded = Arc::new(AtomicBool::new(false));
    let out_limit = exceeded.clone();
    let err_limit = exceeded.clone();
    let sender = std::thread::Builder::new()
        .name("eps-input".into())
        .spawn(move || input.write_all(&bytes))
        .map_err(|e| e.to_string())?;
    let reader = std::thread::Builder::new()
        .name("eps-png".into())
        .spawn(move || drain(output, wire::MAX_PNG, &out_limit))
        .map_err(|e| e.to_string())?;
    let errors = std::thread::Builder::new()
        .name("eps-log".into())
        .spawn(move || drain(error, wire::MAX_LOG, &err_limit))
        .map_err(|e| e.to_string())?;
    let start = Instant::now();
    let mut scratch_exceeded = false;
    let status = loop {
        scratch_exceeded |= !scratch_within_limit(&private)?;
        if scratch_exceeded
            || exceeded.load(Ordering::Relaxed)
            || start.elapsed() >= Duration::from_secs(25)
        {
            let _ = child.0.kill();
            break child.0.wait().map_err(|e| e.to_string())?;
        }
        if let Some(status) = child.0.try_wait().map_err(|e| e.to_string())? {
            break status;
        }
        std::thread::sleep(Duration::from_millis(10));
    };
    let sent = sender.join().map_err(|_| "EPS input transport stopped")?;
    let png = reader.join().map_err(|_| "EPS image transport stopped")??;
    let log = errors.join().map_err(|_| "EPS log transport stopped")??;
    if scratch_exceeded {
        return Err("EPS renderer exceeded its private scratch allowance".into());
    }
    if exceeded.load(Ordering::Relaxed) {
        return Err("EPS renderer exceeded its bounded image or diagnostic output".into());
    }
    if start.elapsed() >= Duration::from_secs(25) {
        return Err("EPS renderer exceeded 25 seconds".into());
    }
    if !status.success() {
        let message: String = String::from_utf8_lossy(&log).chars().take(2048).collect();
        return Err(format!("Ghostscript could not preview this EPS: {message}"));
    }
    // LastPage may close input after the first page; a pipe close is acceptable
    // only with a successful interpreter status and a complete bounded image.
    if let Err(error) = sent
        && error.kind() != std::io::ErrorKind::BrokenPipe
    {
        return Err(error.to_string());
    }
    wire::single_png(&png, request.side)?;
    let completed = std::fs::metadata(&tool).map_err(|e| e.to_string())?;
    if before.len() != completed.len()
        || before.modified().map_err(|e| e.to_string())?
            != completed.modified().map_err(|e| e.to_string())?
    {
        return Err("Configured renderer changed while rendering; choose it again".into());
    }
    Ok(png)
}
pub fn run() -> Result<()> {
    // Blocking here is essential: parent assigns its job before writing this.
    let mut input = std::io::stdin().lock();
    let request: wire::Request = wire::read_json(&mut input)?;
    request.validate()?;
    let mut bytes = Vec::new();
    bytes
        .try_reserve_exact(request.input_len)
        .map_err(|e| e.to_string())?;
    bytes.resize(request.input_len, 0);
    input.read_exact(&mut bytes).map_err(|e| e.to_string())?;
    let result = render(&request, bytes).and_then(|png| {
        let mut reader =
            image::ImageReader::with_format(std::io::Cursor::new(png), image::ImageFormat::Png);
        let mut limits = image::Limits::default();
        limits.max_image_width = Some(request.side as u32);
        limits.max_image_height = Some(request.side as u32);
        limits.max_alloc = Some(64 * 1024 * 1024);
        reader.limits(limits);
        let image = reader.decode().map_err(|e| e.to_string())?.into_rgba8();
        if image.width() as usize != request.side || image.height() as usize != request.side {
            return Err("EPS image dimensions changed during decode".into());
        }
        Ok(image.into_raw())
    });
    let mut header = wire::Header {
        file_bytes: request.input_len,
        page_count: 1,
        ..Default::default()
    };
    match &result {
        Ok(_) => {
            header.size = [request.side as u32; 2];
            header.original_size = header.size;
        }
        Err(error) => header.error = Some(error.clone()),
    }
    let mut output = std::io::stdout().lock();
    wire::write_json(&mut output, &header)?;
    if let Ok(pixels) = result {
        output.write_all(&pixels).map_err(|e| e.to_string())?;
    }
    output.flush().map_err(|e| e.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn scratch_admission_checks_actual_file_count_size_and_nonregular_entries() {
        let directory = tempfile::tempdir().unwrap();
        assert!(scratch_within_limit(directory.path()).unwrap());
        let file = std::fs::File::create(directory.path().join("scratch")).unwrap();
        file.set_len(64 * 1024 * 1024 + 1).unwrap();
        assert!(!scratch_within_limit(directory.path()).unwrap());
        file.set_len(1).unwrap();
        assert!(scratch_within_limit(directory.path()).unwrap());
        for i in 0..63 {
            std::fs::File::create(directory.path().join(format!("part{i}"))).unwrap();
        }
        assert!(scratch_within_limit(directory.path()).unwrap());
        std::fs::File::create(directory.path().join("one-too-many")).unwrap();
        assert!(!scratch_within_limit(directory.path()).unwrap());
        let other = tempfile::tempdir().unwrap();
        std::fs::create_dir(other.path().join("unexpected-directory")).unwrap();
        assert!(!scratch_within_limit(other.path()).unwrap());
    }
    #[test]
    fn version_admission_is_explicit_and_logs_are_bounded() {
        for text in [b"10.08.0\r\n".as_slice(), b"10.09.0", b"11.0.0"] {
            assert!(valid_version(text));
        }
        for text in [
            b"9.56.1".as_slice(),
            b"10.07.1",
            b"10.08",
            b"tool 10.08.0",
            b"9999.0.0",
            b"10.08.0\nextra",
        ] {
            assert!(!valid_version(text));
        }
        let exceeded = AtomicBool::new(false);
        let output = drain(&b"0123456789"[..], 9, &exceeded).unwrap();
        assert!(exceeded.load(Ordering::Relaxed));
        assert!(output.len() <= 9);
    }
}
