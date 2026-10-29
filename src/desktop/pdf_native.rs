//! Windows-native PDF snapshots and page rendering, called only on the worker.
//! The native API owns parser/render internals; encoded/output bounds are not RSS caps.
use super::image_load::Preview;
use matrust::Result;
use std::{
    path::{Path, PathBuf},
    sync::{Arc, atomic::AtomicBool},
};

pub const MAX_FILE_BYTES: usize = 64 * 1024 * 1024;
pub const MAX_PAGES: u32 = 1000;

/// Balanced WinRT initialization on the desktop worker. The marker prevents
/// moving this guard to another thread; all preview references close before join.
pub struct Apartment(std::marker::PhantomData<std::rc::Rc<()>>);
impl Apartment {
    pub fn new() -> Result<Self> {
        #[cfg(windows)]
        // SAFETY: This initializes the current worker thread without raw pointers;
        // the !Send guard balances the successful call on that same thread.
        unsafe {
            windows::Win32::System::WinRT::RoInitialize(
                windows::Win32::System::WinRT::RO_INIT_MULTITHREADED,
            )
            .map_err(|e| format!("Could not initialize Windows PDF runtime: {e}"))?;
        }
        Ok(Self(std::marker::PhantomData))
    }
}
impl Drop for Apartment {
    fn drop(&mut self) {
        #[cfg(windows)]
        // SAFETY: Constructed only after successful initialization, on this thread.
        unsafe {
            windows::Win32::System::WinRT::RoUninitialize();
        }
    }
}

pub struct Document {
    pub path: PathBuf,
    pub file_bytes: usize,
    pub source_digest: [u8; 32],
    pub page_count: u32,
    #[cfg(windows)]
    native: native::Snapshot,
}
#[derive(Clone)]
pub struct Page {
    pub document: Arc<Document>,
    /// Zero-based page index in this immutable document snapshot.
    pub index: u32,
}

pub fn open(path: &Path, cancel: &AtomicBool) -> Result<Arc<Document>> {
    #[cfg(windows)]
    {
        native::open(path, cancel)
    }
    #[cfg(not(windows))]
    {
        let _ = (path, cancel, MAX_FILE_BYTES, MAX_PAGES);
        Err("PDF preview currently requires Windows".into())
    }
}

pub fn render(
    document: Arc<Document>,
    index: u32,
    texture_side: usize,
    cancel: &AtomicBool,
) -> Result<(Preview, Page)> {
    #[cfg(windows)]
    {
        native::render(document, index, texture_side, cancel)
    }
    #[cfg(not(windows))]
    {
        let _ = (document, index, texture_side, cancel);
        Err("PDF preview currently requires Windows".into())
    }
}

#[cfg(windows)]
mod native {
    use super::*;
    use crate::image_load::{MAX_PREVIEW_PIXELS, check, read_file};
    use eframe::egui::ColorImage;
    use image::{ImageDecoder, ImageReader};
    use std::{
        io::Cursor,
        sync::{atomic::Ordering, mpsc},
        time::{Duration, Instant},
    };
    use windows::{
        Data::Pdf::{PdfDocument, PdfPage, PdfPageRenderOptions},
        Storage::Streams::{DataReader, DataWriter, InMemoryRandomAccessStream},
        UI::Color,
        core::Interface,
    };
    use windows_future::{
        AsyncActionCompletedHandler, AsyncOperationCompletedHandler, IAsyncAction, IAsyncInfo,
        IAsyncOperation,
    };

    pub struct Snapshot {
        pdf: PdfDocument,
        // Keep immutable input alive for lazy page decoding. No filesystem handle is retained.
        _source: InMemoryRandomAccessStream,
    }
    struct OpenPage(PdfPage);
    impl Drop for OpenPage {
        fn drop(&mut self) {
            let _ = self.0.Close();
        }
    }
    fn win<T>(result: windows::core::Result<T>) -> Result<T> {
        result.map_err(|e| format!("Windows PDF: {e}"))
    }
    pub(super) fn render_size(width: f64, height: f64, texture_side: usize) -> Result<[u32; 2]> {
        if !width.is_finite()
            || !height.is_finite()
            || width <= 0.
            || height <= 0.
            || width > 100_000.
            || height > 100_000.
        {
            return Err("PDF page has invalid or excessive dimensions".into());
        }
        let side = texture_side.clamp(1, 4096) as f64;
        let scale = (side / width)
            .min(side / height)
            .min((MAX_PREVIEW_PIXELS / (width * height)).sqrt())
            .min(1.);
        let mut size = [
            (width * scale).ceil().max(1.) as u32,
            (height * scale).ceil().max(1.) as u32,
        ];
        if u64::from(size[0]) * u64::from(size[1]) > MAX_PREVIEW_PIXELS as u64 {
            size = [
                (width * scale).floor().max(1.) as u32,
                (height * scale).floor().max(1.) as u32,
            ];
        }
        Ok(size)
    }
    fn wait<T: windows::core::RuntimeType>(
        operation: IAsyncOperation<T>,
        cancel: &AtomicBool,
    ) -> Result<T> {
        let (send, receive) = mpsc::sync_channel(1);
        win(
            operation.SetCompleted(&AsyncOperationCompletedHandler::new(move |_, _| {
                let _ = send.try_send(());
                Ok(())
            })),
        )?;
        wait_completion(&win(operation.cast::<IAsyncInfo>())?, receive, cancel)?;
        win(operation.GetResults())
    }
    fn wait_action(operation: IAsyncAction, cancel: &AtomicBool) -> Result<()> {
        let (send, receive) = mpsc::sync_channel(1);
        win(
            operation.SetCompleted(&AsyncActionCompletedHandler::new(move |_, _| {
                let _ = send.try_send(());
                Ok(())
            })),
        )?;
        wait_completion(&win(operation.cast::<IAsyncInfo>())?, receive, cancel)?;
        win(operation.GetResults())
    }
    fn wait_completion(
        info: &IAsyncInfo,
        receive: mpsc::Receiver<()>,
        cancel: &AtomicBool,
    ) -> Result<()> {
        let start = Instant::now();
        let mut reason = None;
        loop {
            if reason.is_none()
                && (cancel.load(Ordering::Relaxed) || start.elapsed() > Duration::from_secs(30))
            {
                reason = Some(if cancel.load(Ordering::Relaxed) {
                    "Cancelled"
                } else {
                    "PDF operation exceeded 30 seconds"
                });
                // Cancellation is cooperative. Keep input/output alive until completion,
                // even if the native operation takes longer to acknowledge cancellation.
                let _ = info.Cancel();
            }
            match receive.recv_timeout(Duration::from_millis(10)) {
                Ok(()) => break,
                Err(mpsc::RecvTimeoutError::Timeout) => continue,
                Err(mpsc::RecvTimeoutError::Disconnected) => {
                    return Err("PDF completion channel closed".into());
                }
            }
        }
        if let Some(reason) = reason {
            return Err(reason.into());
        }
        check(cancel)
    }
    pub fn open(path: &Path, cancel: &AtomicBool) -> Result<Arc<Document>> {
        let bytes = read_file(path, MAX_FILE_BYTES, cancel)?;
        if !bytes.starts_with(b"%PDF-") {
            return Err("Expected a PDF document".into());
        }
        let file_bytes = bytes.len();
        let source_digest = super::super::image_load::Original::identity(&bytes, cancel)?.digest;
        let source = win(InMemoryRandomAccessStream::new())?;
        let writer = win(DataWriter::CreateDataWriter(&win(
            source.GetOutputStreamAt(0)
        )?))?;
        win(writer.WriteBytes(&bytes))?;
        drop(bytes);
        let written = wait(win(writer.StoreAsync())?, cancel)?;
        if written as usize != file_bytes {
            return Err("Incomplete PDF snapshot write".into());
        }
        win(writer.DetachStream())?;
        win(writer.Close())?;
        win(source.Seek(0))?;
        let pdf = wait(win(PdfDocument::LoadFromStreamAsync(&source))?, cancel)?;
        let page_count = win(pdf.PageCount())?;
        if page_count == 0 || page_count > MAX_PAGES {
            return Err(format!("PDF preview supports 1-{MAX_PAGES} pages"));
        }
        if win(pdf.IsPasswordProtected())? {
            return Err("Password-protected PDF preview is not yet supported".into());
        }
        check(cancel)?;
        Ok(Arc::new(Document {
            path: path.to_owned(),
            file_bytes,
            source_digest,
            page_count,
            native: Snapshot {
                pdf,
                _source: source,
            },
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
        let page = OpenPage(win(document.native.pdf.GetPage(index))?);
        let original = win(page.0.Size())?;
        let (width, height) = (f64::from(original.Width), f64::from(original.Height));
        let size = render_size(width, height, texture_side)?;
        let options = win(PdfPageRenderOptions::new())?;
        win(options.SetDestinationWidth(size[0]))?;
        win(options.SetDestinationHeight(size[1]))?;
        win(options.SetBackgroundColor(Color {
            A: 255,
            R: 255,
            G: 255,
            B: 255,
        }))?;
        win(options.SetIsIgnoringHighContrast(true))?;
        let output = win(InMemoryRandomAccessStream::new())?;
        wait_action(
            win(page.0.RenderWithOptionsToStreamAsync(&output, &options))?,
            cancel,
        )?;
        let encoded_len = win(output.Size())?;
        if encoded_len == 0 || encoded_len > MAX_FILE_BYTES as u64 {
            return Err("Rendered PDF page exceeds the 64 MiB image limit".into());
        }
        let reader = win(DataReader::CreateDataReader(&win(
            output.GetInputStreamAt(0)
        )?))?;
        let read = wait(win(reader.LoadAsync(encoded_len as u32))?, cancel)?;
        if u64::from(read) != encoded_len {
            return Err("Incomplete rendered PDF page".into());
        }
        let mut encoded = vec![0; encoded_len as usize];
        win(reader.ReadBytes(&mut encoded))?;
        win(reader.Close())?;
        let mut decoder = ImageReader::with_format(Cursor::new(&encoded), image::ImageFormat::Png);
        let mut limits = image::Limits::default();
        limits.max_image_width = Some(size[0]);
        limits.max_image_height = Some(size[1]);
        limits.max_alloc = Some(64 * 1024 * 1024);
        decoder.limits(limits);
        let decoder = decoder.into_decoder().map_err(|e| e.to_string())?;
        if decoder.dimensions() != (size[0], size[1]) || decoder.total_bytes() > 32_000_000 {
            return Err("PDF renderer returned unexpected image dimensions".into());
        }
        let rgba = image::DynamicImage::from_decoder(decoder)
            .map_err(|e| e.to_string())?
            .into_rgba8();
        check(cancel)?;
        let pixels = ColorImage::from_rgba_unmultiplied(size.map(|v| v as usize), rgba.as_raw());
        let preview = Preview {
            original: Some(super::super::image_load::Original {
                digest: document.source_digest,
                len: document.file_bytes,
                bytes: None,
            }),
            path: document.path.clone(),
            original_size: [width.ceil() as u32, height.ceil() as u32],
            format: "PDF (rendered page)",
            file_bytes: document.file_bytes,
            pixels,
        };
        check(cancel)?;
        Ok((preview, Page { document, index }))
    }
    #[cfg(test)]
    mod tests {
        use super::*;
        #[test]
        fn pdf_preview_async_completion_and_cancellation_return_consistent_results() {
            let cancelled = AtomicBool::new(true);
            assert_eq!(
                wait(IAsyncOperation::ready(Ok(7_u32)), &AtomicBool::new(false)).unwrap(),
                7
            );
            assert_eq!(
                wait_action(IAsyncAction::ready(Ok(())), &cancelled).unwrap_err(),
                "Cancelled"
            );
            let (release, receive) = mpsc::channel();
            let action = IAsyncAction::spawn(move || {
                let _ = receive.recv();
                Ok(())
            });
            let releaser = std::thread::spawn(move || {
                std::thread::sleep(Duration::from_millis(20));
                release.send(()).unwrap();
            });
            assert_eq!(wait_action(action, &cancelled).unwrap_err(), "Cancelled");
            releaser.join().unwrap();
        }
    }
}

#[cfg(all(test, windows))]
mod tests {
    use super::*;
    fn fixture(name: &str) -> PathBuf {
        Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("tests/fixtures/preview/pdf-preview")
            .join(name)
    }
    #[test]
    fn pdf_preview_renders_independent_pages_rotation_crop_and_bounded_sizes() {
        let cancel = AtomicBool::new(false);
        let document = open(&fixture("pages.pdf"), &cancel).unwrap();
        assert_eq!(document.page_count, 3);
        let mut sizes = Vec::new();
        for (index, color) in [[255, 0, 0, 255], [0, 0, 255, 255], [0, 153, 0, 255]]
            .into_iter()
            .enumerate()
        {
            let (preview, page) = render(document.clone(), index as u32, 4096, &cancel).unwrap();
            assert!(Arc::ptr_eq(&document, &page.document));
            assert_eq!(page.index, index as u32);
            assert!(
                preview
                    .pixels
                    .pixels
                    .iter()
                    .filter(|p| p.to_array() == color)
                    .count()
                    > 10_000
            );
            assert!(preview.pixels.pixels.iter().all(|p| p.a() == 255));
            sizes.push(preview.original_size);
            let (small, _) = render(document.clone(), index as u32, 96, &cancel).unwrap();
            assert!(small.pixels.size.iter().all(|&s| s <= 96));
        }
        assert!(sizes[0][0] > sizes[0][1]);
        assert_eq!(sizes[1], [sizes[0][1], sizes[0][0]]);
        assert!(sizes[2][0] < sizes[0][0] && sizes[2][1] < sizes[0][1]);
    }
    #[test]
    fn pdf_preview_snapshot_survives_disk_changes_and_releases_with_last_page() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("snapshot.pdf");
        std::fs::copy(fixture("pages.pdf"), &path).unwrap();
        let cancel = AtomicBool::new(false);
        let document = open(&path, &cancel).unwrap();
        let weak = Arc::downgrade(&document);
        std::fs::write(&path, b"broken PDF").unwrap();
        assert!(open(&path, &cancel).is_err());
        let (_, page) = render(document.clone(), 1, 1024, &cancel).unwrap();
        std::fs::remove_file(&path).unwrap();
        assert!(render(document.clone(), 2, 1024, &cancel).is_ok());
        drop(document);
        assert!(weak.upgrade().is_some());
        drop(page);
        assert!(weak.upgrade().is_none());
    }
    #[test]
    fn pdf_preview_rejects_invalid_encrypted_large_cancelled_and_out_of_range_requests() {
        let cancel = AtomicBool::new(false);
        for name in ["encrypted.pdf", "empty.pdf", "too-many-pages.pdf"] {
            assert!(open(&fixture(name), &cancel).is_err(), "{name}");
        }
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("invalid.pdf");
        assert!(matches!(open(&path,&AtomicBool::new(true)),Err(e) if e == "Cancelled"));
        std::fs::write(&path, b"%PDF-1.7\ninvalid content").unwrap();
        assert!(open(&path, &cancel).is_err());
        std::fs::File::create(&path)
            .unwrap()
            .set_len(MAX_FILE_BYTES as u64 + 1)
            .unwrap();
        assert!(matches!(open(&path,&cancel),Err(e) if e.contains("64 MiB")));
        let document = open(&fixture("pages.pdf"), &cancel).unwrap();
        assert!(render(document.clone(), 3, 4096, &cancel).is_err());
        assert!(
            matches!(render(document,0,4096,&AtomicBool::new(true)),Err(e) if e == "Cancelled")
        );
        let big = open(&fixture("oversized-page.pdf"), &cancel).unwrap();
        // Windows can normalize out-of-spec media boxes. Admit only the bounded
        // effective page it reports, or retain its explicit decoding failure.
        if let Ok((image, _)) = render(big, 0, 4096, &cancel) {
            assert!(image.original_size.iter().all(|&s| s <= 100_000));
            assert!(image.pixels.size.iter().all(|&s| s <= 4096));
            assert!(image.pixels.pixels.len() <= 8_000_000);
        }
        for size in [
            (0., 1.),
            (f64::NAN, 1.),
            (1., f64::INFINITY),
            (100_001., 100.),
        ] {
            assert!(native::render_size(size.0, size.1, 4096).is_err());
        }
        let size = native::render_size(8192., 8192., 4096).unwrap();
        assert!(u64::from(size[0]) * u64::from(size[1]) <= 8_000_000);
    }
    #[test]
    fn pdf_preview_reopens_native_plot_export_with_labels_and_curves() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("plot.pdf");
        let mut engine = matrust::Engine::default();
        engine
            .execute("plot([1 3 2]);title('Reopened PDF plot');xlabel('Time');ylabel('Value');")
            .unwrap();
        matrust::export::export_figure(engine.figure.as_ref().unwrap(), &path, 960, 560).unwrap();
        let cancel = AtomicBool::new(false);
        let document = open(&path, &cancel).unwrap();
        let (image, _) = render(document, 0, 4096, &cancel).unwrap();
        // The exported PDF uses 960x560 points; Windows exposes 96-DPI units.
        assert_eq!(image.original_size, [1280, 747]);
        assert_eq!(image.pixels.size, [1280, 747]);
        assert!(
            image
                .pixels
                .pixels
                .iter()
                .filter(|p| p.b() > p.r().saturating_add(50))
                .count()
                > 500
        );
        assert!(
            image
                .pixels
                .pixels
                .iter()
                .filter(|p| p.r() < 100 && p.g() < 100 && p.b() < 100)
                .count()
                > 500
        );
    }
}
