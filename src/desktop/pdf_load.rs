//! PDF previews use one hidden, disposable renderer process per open document.
//! This keeps Windows PDF/graphics teardown out of the desktop process.
#[path = "pdf_job.rs"]
mod job;
use super::image_load::{Preview, check};
use super::pdf_protocol as protocol;
use matrust::Result;
use protocol::{Header, Reply, Request};
use std::{
    path::{Path, PathBuf},
    process::{Child, Command, Stdio},
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, Ordering},
        mpsc,
    },
    time::{Duration, Instant},
};

pub const MAX_FILE_BYTES: usize = 64 * 1024 * 1024;
pub const MAX_PAGES: u32 = 1000;
pub struct Document {
    pub path: PathBuf,
    pub file_bytes: usize,
    source_digest: [u8; 32],
    pub page_count: u32,
    connection: Mutex<Connection>,
}
#[derive(Clone)]
pub struct Page {
    pub document: Arc<Document>,
    pub index: u32,
}

struct Connection {
    _job: job::Job,
    child: Child,
    send: Option<mpsc::SyncSender<Request>>,
    receive: mpsc::Receiver<Result<Reply>>,
    thread: Option<std::thread::JoinHandle<()>>,
}
impl Connection {
    fn start() -> Result<Self> {
        let exe = std::env::current_exe().map_err(|e| e.to_string())?;
        let mut folder = exe.parent().ok_or("Missing application directory")?;
        if folder
            .file_name()
            .is_some_and(|n| n == "deps" || n == "examples")
        {
            folder = folder.parent().ok_or("Missing build directory")?;
        }
        let helper = folder.join("matrust-pdf-worker.exe");
        let mut command = Command::new(&helper);
        command
            .arg("--pdf-protocol-v1")
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::null());
        #[cfg(windows)]
        {
            use std::os::windows::process::CommandExt;
            command.creation_flags(0x08000000); // CREATE_NO_WINDOW
        }
        let mut child = command.spawn().map_err(|e| {
            format!(
                "Could not start {}: {e}. Keep the PDF worker beside the desktop executable.",
                crate::path_label::display(&helper)
            )
        })?;
        let job = match job::Job::new(&child) {
            Ok(job) => job,
            Err(error) => {
                let _ = child.kill();
                let _ = child.wait();
                return Err(format!("Could not constrain PDF renderer: {error}"));
            }
        };
        let mut input = child.stdin.take().ok_or("Missing PDF input pipe")?;
        let mut output = child.stdout.take().ok_or("Missing PDF output pipe")?;
        let (send, requests) = mpsc::sync_channel(1);
        let (responses, receive) = mpsc::sync_channel(1);
        let thread = std::thread::Builder::new()
            .name("pdf-transport".into())
            .spawn(move || {
                while let Ok(request) = requests.recv() {
                    let reply = protocol::write_json(&mut input, &request)
                        .and_then(|_| protocol::read_reply(&mut output));
                    let failed = reply.is_err();
                    if responses.try_send(reply).is_err() || failed {
                        break;
                    }
                }
            })
            .map_err(|error| {
                let _ = child.kill();
                let _ = child.wait();
                format!("Could not start PDF transport: {error}")
            })?;
        Ok(Self {
            _job: job,
            child,
            send: Some(send),
            receive,
            thread: Some(thread),
        })
    }
    fn stop(&mut self) {
        if self.thread.is_none() {
            return;
        }
        self.send.take();
        // The helper only reads documents and writes its pipe. Terminating it
        // cannot interrupt a user-file save, and bypasses native DLL exit hooks.
        let _ = self.child.kill();
        let _ = self.child.wait();
        if let Some(thread) = self.thread.take() {
            let _ = thread.join();
        }
        while self.receive.try_recv().is_ok() {}
    }
    fn request(&mut self, request: Request, cancel: &AtomicBool) -> Result<Reply> {
        check(cancel)?;
        self.send
            .as_ref()
            .ok_or("PDF renderer closed; use Reload image to reopen the document")?
            .try_send(request)
            .map_err(|e| format!("PDF request channel: {e}"))?;
        self.receive_reply(cancel, Duration::from_secs(30))
    }
    fn receive_reply(&mut self, cancel: &AtomicBool, timeout: Duration) -> Result<Reply> {
        let start = Instant::now();
        loop {
            if cancel.load(Ordering::Relaxed) || start.elapsed() >= timeout {
                self.stop();
                return Err(if cancel.load(Ordering::Relaxed) {
                    "Cancelled"
                } else {
                    "PDF renderer exceeded 30 seconds; use Reload image to retry"
                }
                .into());
            }
            match self.receive.recv_timeout(Duration::from_millis(10)) {
                Ok(Ok(reply)) => return Ok(reply),
                Ok(Err(error)) => {
                    self.stop();
                    return Err(format!(
                        "PDF renderer stopped: {error}; use Reload image to retry"
                    ));
                }
                Err(mpsc::RecvTimeoutError::Timeout) => {}
                Err(mpsc::RecvTimeoutError::Disconnected) => {
                    self.stop();
                    return Err("PDF renderer stopped; use Reload image to retry".into());
                }
            }
        }
    }
}
impl Drop for Connection {
    fn drop(&mut self) {
        self.stop();
    }
}

fn valid(header: &Header) -> Result<()> {
    if let Some(error) = &header.error {
        return Err(error.clone());
    }
    if header.page_count == 0 || header.page_count > MAX_PAGES || header.file_bytes > MAX_FILE_BYTES
    {
        return Err("PDF renderer returned invalid document metadata".into());
    }
    Ok(())
}
pub fn open(path: &Path, cancel: &AtomicBool) -> Result<Arc<Document>> {
    if !cfg!(windows) {
        return Err("PDF preview currently requires Windows".into());
    }
    check(cancel)?;
    let path = std::path::absolute(path).map_err(|e| e.to_string())?;
    let metadata = path.metadata().map_err(|e| e.to_string())?;
    if !metadata.is_file() || metadata.len() > MAX_FILE_BYTES as u64 {
        return Err("Open a regular PDF file no larger than 64 MiB".into());
    }
    let mut connection = Connection::start()?;
    let reply = connection.request(Request::Open(path.clone()), cancel)?;
    valid(&reply.header)?;
    check(cancel)?;
    Ok(Arc::new(Document {
        path,
        file_bytes: reply.header.file_bytes,
        source_digest: reply
            .header
            .source_digest
            .ok_or("PDF worker lacks original-document identity; use a matching worker build")?,
        page_count: reply.header.page_count,
        connection: Mutex::new(connection),
    }))
}
pub fn render(
    document: Arc<Document>,
    index: u32,
    texture_side: usize,
    cancel: &AtomicBool,
) -> Result<(Preview, Page)> {
    check(cancel)?;
    if index >= document.page_count {
        return Err("PDF page is out of range".into());
    }
    let reply = document
        .connection
        .lock()
        .map_err(|_| "PDF renderer state is unavailable")?
        .request(
            Request::Render {
                index,
                texture_side: texture_side.clamp(1, 4096),
            },
            cancel,
        )?;
    valid(&reply.header)?;
    if reply.header.size.contains(&0)
        || reply.header.source_digest != Some(document.source_digest)
        || reply.header.file_bytes != document.file_bytes
        || reply.header.page_count != document.page_count
    {
        return Err("PDF renderer returned mismatched page metadata".into());
    }
    check(cancel)?;
    let pixels = eframe::egui::ColorImage::from_rgba_premultiplied(
        reply.header.size.map(|s| s as usize),
        &reply.pixels,
    );
    check(cancel)?;
    let preview = Preview {
        original: Some(super::image_load::Original {
            digest: document.source_digest,
            len: document.file_bytes,
            bytes: None,
        }),
        path: document.path.clone(),
        file_bytes: document.file_bytes,
        original_size: reply.header.original_size,
        format: "PDF (rendered page)",
        pixels,
    };
    Ok((preview, Page { document, index }))
}

#[cfg(all(test, windows))]
mod tests {
    use super::*;
    use std::os::windows::io::{AsRawHandle, FromRawHandle, OwnedHandle};
    use windows::Win32::{
        Foundation::{HANDLE, WAIT_OBJECT_0, WAIT_TIMEOUT},
        System::Threading::{OpenProcess, PROCESS_SYNCHRONIZE, WaitForSingleObject},
    };

    fn fixture() -> PathBuf {
        Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/preview/pdf-preview/pages.pdf")
    }
    fn process(document: &Document) -> OwnedHandle {
        let id = document.connection.lock().unwrap().child.id();
        // SAFETY: OpenProcess returns a new owned handle with synchronize rights.
        unsafe {
            OwnedHandle::from_raw_handle(OpenProcess(PROCESS_SYNCHRONIZE, false, id).unwrap().0)
        }
    }
    fn status(handle: &OwnedHandle, timeout: u32) -> windows::Win32::Foundation::WAIT_EVENT {
        // SAFETY: The borrowed handle remains owned and live throughout this wait.
        unsafe { WaitForSingleObject(HANDLE(handle.as_raw_handle()), timeout) }
    }
    #[test]
    fn isolated_pdf_snapshot_survives_disk_removal_and_last_owner_stops_helper() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("snapshot.pdf");
        std::fs::copy(fixture(), &path).unwrap();
        let cancel = AtomicBool::new(false);
        let document = open(&path, &cancel).unwrap();
        let handle = process(&document);
        let (preview, page) = render(document.clone(), 0, 128, &cancel).unwrap();
        let original = preview.original.unwrap();
        let bytes = std::fs::read(&path).unwrap();
        assert_eq!(original.digest, *blake3::hash(&bytes).as_bytes());
        assert_eq!(original.len, bytes.len());
        std::fs::remove_file(path).unwrap();
        assert!(render(document.clone(), 2, 128, &cancel).is_ok());
        drop(document);
        assert_eq!(status(&handle, 0), WAIT_TIMEOUT);
        drop(page);
        assert_eq!(status(&handle, 5000), WAIT_OBJECT_0);
    }
    #[test]
    fn isolated_pdf_cancellation_deadline_and_crash_release_transport() {
        let cancel = AtomicBool::new(false);
        for mode in ["cancel", "deadline", "crash"] {
            let document = open(&fixture(), &cancel).unwrap();
            let handle = process(&document);
            let mut connection = document.connection.lock().unwrap();
            let result = if mode == "crash" {
                connection.child.kill().unwrap();
                connection.request(
                    Request::Render {
                        index: 0,
                        texture_side: 128,
                    },
                    &cancel,
                )
            } else {
                connection
                    .send
                    .as_ref()
                    .unwrap()
                    .try_send(Request::Render {
                        index: 0,
                        texture_side: 128,
                    })
                    .unwrap();
                connection.receive_reply(
                    &AtomicBool::new(mode == "cancel"),
                    if mode == "deadline" {
                        Duration::ZERO
                    } else {
                        Duration::from_secs(30)
                    },
                )
            };
            let error = result.err().unwrap();
            if mode == "cancel" {
                assert_eq!(error, "Cancelled");
            } else {
                assert!(error.contains("Reload image"), "{error}");
            }
            assert!(connection.thread.is_none());
            assert_eq!(status(&handle, 5000), WAIT_OBJECT_0);
            drop(connection);
            assert!(render(document.clone(), 0, 128, &cancel).is_err());
            // A fresh Reload can recover after each failure mode.
            let fresh = open(&fixture(), &cancel).unwrap();
            assert!(render(fresh, 0, 128, &cancel).is_ok());
        }
    }
    #[test]
    fn isolated_pdf_job_closure_terminates_the_owned_helper() {
        use std::os::windows::process::CommandExt;
        let exe = std::env::current_exe().unwrap();
        let helper = exe
            .parent()
            .unwrap()
            .parent()
            .unwrap()
            .join("matrust-pdf-worker.exe");
        let mut child = Command::new(helper)
            .arg("--pdf-protocol-v1")
            .stdin(Stdio::piped())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .creation_flags(0x08000000)
            .spawn()
            .unwrap();
        let job = job::Job::new(&child).unwrap();
        // SAFETY: OpenProcess returns a new owned handle with synchronize rights.
        let handle = unsafe {
            OwnedHandle::from_raw_handle(
                OpenProcess(PROCESS_SYNCHRONIZE, false, child.id())
                    .unwrap()
                    .0,
            )
        };
        drop(job);
        assert_eq!(status(&handle, 5000), WAIT_OBJECT_0);
        child.wait().unwrap();
    }
}
