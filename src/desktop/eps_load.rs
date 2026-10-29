//! Read-only EPS snapshots interpreted in an owned, bounded Windows job.
#[path = "eps_job.rs"]
mod job;
// Shared with the one-shot helper; each binary uses one transport direction.
#[allow(dead_code, unused_imports)]
#[path = "eps_protocol.rs"]
mod wire;
use super::image_load::{Preview, check};
use matrust::Result;
use std::{
    io::{Read, Write},
    path::{Path, PathBuf},
    process::{Child, Command, Stdio},
    sync::{Arc, atomic::AtomicBool, mpsc},
    time::{Duration, Instant},
};
struct Render {
    job: Option<job::Job>,
    _scratch: tempfile::TempDir,
    child: Child,
    thread: Option<std::thread::JoinHandle<()>>,
    receive: mpsc::Receiver<Result<wire::Reply>>,
}
impl Render {
    fn start(renderer: PathBuf, bytes: Arc<Vec<u8>>, side: usize) -> Result<Self> {
        let scratch = tempfile::Builder::new()
            .prefix("matrust-eps-")
            .tempdir()
            .map_err(|e| e.to_string())?;
        let request = wire::Request {
            renderer,
            scratch: std::fs::canonicalize(scratch.path()).map_err(|e| e.to_string())?,
            input_len: bytes.len(),
            side,
        };
        request.validate()?;
        let exe = std::env::current_exe().map_err(|e| e.to_string())?;
        let mut folder = exe.parent().ok_or("Missing application directory")?;
        if folder
            .file_name()
            .is_some_and(|n| n == "deps" || n == "examples")
        {
            folder = folder.parent().ok_or("Missing build directory")?;
        }
        let mut command = Command::new(folder.join("matrust-pdf-worker.exe"));
        command
            .arg("--eps-protocol-v1")
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::null());
        #[cfg(windows)]
        {
            use std::os::windows::process::CommandExt;
            command.creation_flags(windows::Win32::System::Threading::DETACHED_PROCESS.0);
        }
        let mut child = command.spawn().map_err(|e| {
            format!("Could not start preview worker: {e}. Keep it beside the desktop executable.")
        })?;
        let job = match job::Job::new(&child) {
            Ok(job) => job,
            Err(error) => {
                let _ = child.kill();
                let _ = child.wait();
                return Err(error);
            }
        };
        // The helper is dormant until this first request, after job admission.
        let input = child.stdin.take();
        let output = child.stdout.take();
        let (send, receive) = mpsc::sync_channel(1);
        let mut owned = Self {
            job: Some(job),
            _scratch: scratch,
            child,
            thread: None,
            receive,
        };
        let mut input = input.ok_or("Missing EPS input pipe")?;
        let mut output = output.ok_or("Missing EPS output pipe")?;
        owned.thread = Some(
            std::thread::Builder::new()
                .name("eps-transport".into())
                .spawn(move || {
                    let result = wire::write_json(&mut input, &request).and_then(|_| {
                        input
                            .write_all(bytes.as_slice())
                            .map_err(|e| e.to_string())?;
                        drop(input);
                        wire::read_reply(&mut output)
                    });
                    let _ = send.try_send(result);
                })
                .map_err(|e| e.to_string())?,
        );
        Ok(owned)
    }
    fn receive(&self, cancel: &AtomicBool, timeout: Duration) -> Result<wire::Reply> {
        let start = Instant::now();
        loop {
            check(cancel)?;
            if start.elapsed() >= timeout {
                return Err("EPS preview exceeded 30 seconds".into());
            }
            match self.receive.recv_timeout(Duration::from_millis(10)) {
                Ok(reply) => return reply,
                Err(mpsc::RecvTimeoutError::Timeout) => {}
                Err(mpsc::RecvTimeoutError::Disconnected) => {
                    return Err("EPS preview worker stopped".into());
                }
            }
        }
    }
}
impl Drop for Render {
    fn drop(&mut self) {
        // Terminate the whole job BEFORE joining: Ghostscript owns pipe handles
        // too. Killing only its parent would leave transport joins blocked.
        if let Some(job) = self.job.take() {
            job.terminate();
        } // the job handle closes here, before the joins below
        let _ = self.child.kill();
        let _ = self.child.wait();
        if let Some(thread) = self.thread.take() {
            let _ = thread.join();
        }
    }
}
fn unchanged(path: &Path, bytes: &[u8], cancel: &AtomicBool) -> Result<()> {
    let mut file = std::fs::File::open(path)
        .map_err(|e| format!("EPS changed or disappeared while rendering: {e}"))?;
    let metadata = file.metadata().map_err(|e| e.to_string())?;
    if !metadata.is_file() || metadata.len() != bytes.len() as u64 {
        return Err("EPS changed while rendering; reopen it".into());
    }
    let mut buffer = [0; 64 * 1024];
    for expected in bytes.chunks(buffer.len()) {
        check(cancel)?;
        file.read_exact(&mut buffer[..expected.len()])
            .map_err(|_| "EPS changed while rendering")?;
        if &buffer[..expected.len()] != expected {
            return Err("EPS changed while rendering; reopen it".into());
        }
    }
    if file.read(&mut buffer[..1]).map_err(|e| e.to_string())? != 0 {
        return Err("EPS changed while rendering; reopen it".into());
    }
    check(cancel)
}
pub fn load(
    path: &Path,
    renderer: &Path,
    texture_side: usize,
    cancel: &AtomicBool,
) -> Result<Preview> {
    if !cfg!(windows) {
        return Err("Configured EPS preview currently requires Windows".into());
    }
    check(cancel)?;
    let path = std::fs::canonicalize(path).map_err(|e| e.to_string())?;
    let renderer = std::fs::canonicalize(renderer).map_err(|e| {
        format!(
            "Configured Ghostscript is unavailable: {e}. Choose it again in EPS preview settings."
        )
    })?;
    let bytes = Arc::new(super::image_load::read_file(
        &path,
        wire::MAX_INPUT,
        cancel,
    )?);
    let side = texture_side.clamp(1, wire::MAX_SIDE);
    let process = Render::start(renderer, bytes.clone(), side)?;
    let reply = process.receive(cancel, Duration::from_secs(30))?;
    drop(process);
    if let Some(error) = reply.header.error {
        return Err(error);
    }
    if reply.header.file_bytes != bytes.len()
        || reply.header.page_count != 1
        || reply.header.size != [side as u32; 2]
        || reply.header.original_size != reply.header.size
    {
        return Err("EPS renderer returned mismatched preview metadata".into());
    }
    unchanged(&path, &bytes, cancel)?;
    let pixels = eframe::egui::ColorImage::from_rgba_unmultiplied([side; 2], &reply.pixels);
    check(cancel)?;
    Ok(Preview {
        original: Some(super::image_load::Original::identity(&bytes, cancel)?),
        path,
        original_size: [side as u32; 2],
        format: "EPS (fitted preview canvas)",
        file_bytes: bytes.len(),
        pixels,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn snapshot_check_rejects_same_size_replacement_and_cancellation() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("source.eps");
        let original = b"%!PS-Adobe-3.0 EPSF-3.0\nshowpage\n";
        std::fs::write(&path, original).unwrap();
        unchanged(&path, original, &AtomicBool::new(false)).unwrap();
        let mut changed = original.to_vec();
        changed[5] = b'X';
        std::fs::write(&path, changed).unwrap();
        assert!(
            unchanged(&path, original, &AtomicBool::new(false))
                .unwrap_err()
                .contains("changed")
        );
        std::fs::write(&path, original).unwrap();
        assert_eq!(
            unchanged(&path, original, &AtomicBool::new(true)).unwrap_err(),
            "Cancelled"
        );
        std::fs::remove_file(&path).unwrap();
        assert!(unchanged(&path, original, &AtomicBool::new(false)).is_err());
    }
    #[cfg(windows)]
    #[test]
    fn owned_eps_transport_cancel_deadline_and_drop_reap_helper_without_renderer() {
        use std::os::windows::io::{AsRawHandle, FromRawHandle, OwnedHandle};
        use windows::Win32::{
            Foundation::{HANDLE, WAIT_OBJECT_0},
            System::Threading::{OpenProcess, PROCESS_SYNCHRONIZE, WaitForSingleObject},
        };
        for cancel in [false, true] {
            let directory = tempfile::tempdir().unwrap();
            let process = Render::start(
                directory.path().join("not-installed.exe"),
                Arc::new(b"%!PS\nshowpage\n".to_vec()),
                16,
            )
            .unwrap();
            // SAFETY: The new synchronization handle is owned until after wait.
            let handle = unsafe {
                OwnedHandle::from_raw_handle(
                    OpenProcess(PROCESS_SYNCHRONIZE, false, process.child.id())
                        .unwrap()
                        .0,
                )
            };
            let scratch = process._scratch.path().to_path_buf();
            assert!(scratch.is_dir());
            let error = process
                .receive(&AtomicBool::new(cancel), Duration::ZERO)
                .err()
                .unwrap();
            assert_eq!(
                error,
                if cancel {
                    "Cancelled"
                } else {
                    "EPS preview exceeded 30 seconds"
                }
            );
            drop(process);
            assert!(!scratch.exists());
            // SAFETY: This live handle has synchronization rights.
            assert_eq!(
                unsafe { WaitForSingleObject(HANDLE(handle.as_raw_handle()), 5000) },
                WAIT_OBJECT_0
            );
        }
    }
}

#[cfg(all(test, windows))]
mod configured_interpreter_acceptance {
    use super::*;
    fn assert_fitted_rectangle(image: &Preview, side: usize, portrait: bool, case: &str) {
        use eframe::egui::Color32;
        assert_eq!(image.pixels.size, [side, side], "{case}");
        let original_side = u32::try_from(side).unwrap();
        assert_eq!(
            image.original_size,
            [original_side, original_side],
            "{case}"
        );
        let [left, top, right, bottom] = if portrait {
            [side / 4, 0, 3 * side / 4, side]
        } else {
            [0, side / 4, side, 3 * side / 4]
        };
        let mut red_count = 0usize;
        let mut red_bounds = [side, side, 0, 0];
        for (index, &pixel) in image.pixels.pixels.iter().enumerate() {
            let x = index % side;
            let y = index / side;
            if pixel == Color32::RED {
                red_count += 1;
                red_bounds[0] = red_bounds[0].min(x);
                red_bounds[1] = red_bounds[1].min(y);
                red_bounds[2] = red_bounds[2].max(x + 1);
                red_bounds[3] = red_bounds[3].max(y + 1);
            }
            // Ignore only a two-pixel edge band for raster edge conventions.
            // Every remaining interior/background pixel is checked, not merely
            // a center that an unscaled 40-by-20 source can happen to cover.
            if x >= left + 2 && x + 2 < right && y >= top + 2 && y + 2 < bottom {
                assert_eq!(pixel, Color32::RED, "{case}: interior ({x},{y})");
            }
            if x + 2 < left || x >= right + 2 || y + 2 < top || y >= bottom + 2 {
                assert_eq!(pixel, Color32::WHITE, "{case}: letterbox ({x},{y})");
            }
        }
        let expected_area = (right - left) * (bottom - top);
        assert!(
            red_count.abs_diff(expected_area) <= 4 * side + 4,
            "{case}: red area {red_count}, expected {expected_area}"
        );
        for (actual, expected) in red_bounds.into_iter().zip([left, top, right, bottom]) {
            assert!(actual.abs_diff(expected) <= 1, "{case}: red bounds differ");
        }
    }

    #[test]
    #[ignore = "requires explicitly configured external Ghostscript and matching built preview worker"]
    fn observed_render_process_cancellation_reaps_job_and_parent_scratch() {
        use std::{
            os::windows::io::{AsRawHandle, FromRawHandle, OwnedHandle},
            sync::atomic::Ordering,
        };
        use windows::Win32::{
            Foundation::{HANDLE, WAIT_OBJECT_0, WAIT_TIMEOUT},
            System::Threading::{OpenProcess, PROCESS_SYNCHRONIZE, WaitForSingleObject},
        };
        let renderer = PathBuf::from(
            std::env::var_os("MATRUST_EPS_TEST_RENDERER")
                .expect("Set the external Ghostscript console path explicitly"),
        );
        assert!(
            renderer.is_absolute(),
            "Configure an absolute executable path"
        );
        let renderer = std::fs::canonicalize(renderer).unwrap();
        // Independent EPS keeps the actual render invocation alive; no file or
        // renderer-specific marker is used as a substitute for process evidence.
        let bytes =
            Arc::new(b"%!PS-Adobe-3.0 EPSF-3.0\n%%BoundingBox: 0 0 10 10\n{} loop\n".to_vec());
        let process = Render::start(renderer.clone(), bytes, 16).unwrap();
        let helper_pid = process.child.id();
        // SAFETY: Own a separate synchronization handle surviving Render Drop.
        let helper = unsafe {
            OwnedHandle::from_raw_handle(
                OpenProcess(PROCESS_SYNCHRONIZE, false, helper_pid)
                    .unwrap()
                    .0,
            )
        };
        let scratch = process._scratch.path().to_path_buf();
        assert!(scratch.is_dir());
        let deadline = Instant::now() + Duration::from_secs(10);
        let (render_pid, render_handle) = loop {
            if let Some(observed) = process
                .job
                .as_ref()
                .unwrap()
                .observed_renderer(helper_pid, &renderer)
                .unwrap()
            {
                break observed;
            }
            assert!(
                Instant::now() < deadline,
                "No live post-probe renderer observed in the owned job"
            );
            assert!(
                matches!(process.receive.try_recv(), Err(mpsc::TryRecvError::Empty)),
                "Helper returned before renderer observation"
            );
            // Polling interval is not evidence; only job/process checks admit Stop.
            std::thread::sleep(Duration::from_millis(1));
        };
        assert_ne!(helper_pid, render_pid);
        // SAFETY: Both independent owned handles have synchronization rights.
        assert_eq!(
            unsafe { WaitForSingleObject(HANDLE(helper.as_raw_handle()), 0) },
            WAIT_TIMEOUT
        );
        assert_eq!(
            unsafe { WaitForSingleObject(HANDLE(render_handle.as_raw_handle()), 0) },
            WAIT_TIMEOUT
        );
        eprintln!(
            "EPS_RENDER_OBSERVED helper_pid={helper_pid} render_pid={render_pid} lifetime_processes=3"
        );
        let cancel = AtomicBool::new(false);
        cancel.store(true, Ordering::Relaxed);
        let result = process.receive(&cancel, Duration::from_secs(30));
        // Production Drop terminates the whole job before joining transport,
        // then releases the parent's scratch directory.
        drop(process);
        assert_eq!(result.err().as_deref(), Some("Cancelled"));
        assert_eq!(
            unsafe { WaitForSingleObject(HANDLE(helper.as_raw_handle()), 5000) },
            WAIT_OBJECT_0
        );
        assert_eq!(
            unsafe { WaitForSingleObject(HANDLE(render_handle.as_raw_handle()), 5000) },
            WAIT_OBJECT_0
        );
        assert!(
            !scratch.exists(),
            "Parent-owned scratch survived cancellation"
        );
        eprintln!(
            "EPS_RENDER_CANCELLED helper_pid={helper_pid} render_pid={render_pid} both_terminal=true scratch_removed=true"
        );
    }
    #[test]
    #[ignore = "requires an explicitly configured installed Ghostscript and matching built preview worker"]
    fn genuine_eps_fit_safer_timeout_and_workspace_independent_pixels() {
        let renderer = PathBuf::from(
            std::env::var_os("MATRUST_EPS_TEST_RENDERER")
                .expect("Set the system Ghostscript console path explicitly"),
        );
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("offset-box.eps");
        // Independent executable PostScript, not application-generated EPS.
        // The original no-EndComments case remains included unchanged in form.
        // Both source orientations and origins must fit the same fixed square.
        for side in [256, 2048] {
            for (x, y) in [(10, 20), (-30, -10)] {
                for end_comments in [false, true] {
                    let marker = if end_comments { "%%EndComments\n" } else { "" };
                    let eps = format!(
                        "%!PS-Adobe-3.0 EPSF-3.0\n%%BoundingBox: {x} {y} {} {}\n{marker}1 0 0 setrgbcolor {x} {y} 40 20 rectfill showpage\n%%EOF\n",
                        x + 40,
                        y + 20
                    );
                    std::fs::write(&path, &eps).unwrap();
                    let image = load(&path, &renderer, side, &AtomicBool::new(false)).unwrap();
                    let case = format!("side={side} origin=({x},{y}) EndComments={end_comments}");
                    assert_fitted_rectangle(&image, side, false, &case);
                    let original = image.original.as_ref().unwrap();
                    let encoded = std::fs::read(&path).unwrap();
                    assert_eq!(original.digest, *blake3::hash(&encoded).as_bytes());
                    let target = path.with_file_name(format!("copy-{side}.eps"));
                    let plan = crate::original_copy::prepare(
                        crate::original_copy::Prepare {
                            source: path.clone(),
                            destination: target.clone(),
                            original: original.clone(),
                        },
                        &AtomicBool::new(false),
                    )
                    .unwrap();
                    crate::original_copy::save(plan, &AtomicBool::new(false)).unwrap();
                    assert_eq!(std::fs::read(target).unwrap(), encoded);
                    assert_eq!(std::fs::read(&path).unwrap(), eps.as_bytes());
                }
            }
            // HiResBoundingBox must override the deliberately loose integer
            // box. A square medium must retain portrait orientation as well.
            for (portrait, eps) in [
                (
                    false,
                    "%!PS-Adobe-3.0 EPSF-3.0\n%%BoundingBox: 10 20 52 42\n%%HiResBoundingBox: 10.25 20.5 50.25 40.5\n%%EndComments\n1 0 0 setrgbcolor 10.25 20.5 40 20 rectfill showpage\n%%EOF\n",
                ),
                (
                    true,
                    "%!PS-Adobe-3.0 EPSF-3.0\n%%BoundingBox: -10 -30 10 10\n%%EndComments\n1 0 0 setrgbcolor -10 -30 20 40 rectfill showpage\n%%EOF\n",
                ),
            ] {
                std::fs::write(&path, eps).unwrap();
                let image = load(&path, &renderer, side, &AtomicBool::new(false)).unwrap();
                let case = format!("side={side} portrait={portrait} HiRes={}", !portrait);
                assert_fitted_rectangle(&image, side, portrait, &case);
                assert_eq!(std::fs::read(&path).unwrap(), eps.as_bytes());
            }
        }
        let denied = directory.path().join("must-not-exist.txt");
        let escaped = denied
            .to_string_lossy()
            .replace('\\', "/")
            .replace('(', "\\(")
            .replace(')', "\\)");
        std::fs::write(&path, format!("%!PS-Adobe-3.0 EPSF-3.0\n%%BoundingBox: 0 0 10 10\n({escaped}) (w) file dup (unexpected) writestring closefile showpage\n")).unwrap();
        assert!(load(&path, &renderer, 64, &AtomicBool::new(false)).is_err());
        assert!(!denied.exists());
        std::fs::write(
            &path,
            b"%!PS-Adobe-3.0 EPSF-3.0\n%%BoundingBox: 0 0 10 10\n{} loop\n",
        )
        .unwrap();
        let start = Instant::now();
        let error = load(&path, &renderer, 64, &AtomicBool::new(false))
            .err()
            .unwrap();
        assert!(error.contains("seconds"), "{error}");
        assert!(start.elapsed() < Duration::from_secs(40));
    }
}
