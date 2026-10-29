//! Shared page navigation for native PDF and TIFF snapshots.
use super::{image_load::Preview, pdf_load, tiff_load};
use matrust::Result;
use std::{
    path::Path,
    sync::{Arc, atomic::AtomicBool},
};
pub struct Document {
    pub page_count: u32,
    source: Source,
}
enum Source {
    Pdf(Arc<pdf_load::Document>),
    Tiff(Arc<tiff_load::Document>),
}
#[derive(Clone)]
pub struct Page {
    pub document: Arc<Document>,
    pub index: u32,
}
pub fn open(path: &Path, index: u32, side: usize, cancel: &AtomicBool) -> Result<(Preview, Page)> {
    let (source, page_count) = match path
        .extension()
        .and_then(|s| s.to_str())
        .unwrap_or("")
        .to_ascii_lowercase()
        .as_str()
    {
        "pdf" => {
            let doc = pdf_load::open(path, cancel)?;
            let count = doc.page_count;
            (Source::Pdf(doc), count)
        }
        "tif" | "tiff" => {
            let doc = tiff_load::open(path, cancel)?;
            let count = doc.page_count;
            (Source::Tiff(doc), count)
        }
        _ => return Err("Page navigation supports PDF and TIFF files".into()),
    };
    render(
        Arc::new(Document { page_count, source }),
        index,
        side,
        cancel,
    )
}
pub fn render(
    document: Arc<Document>,
    index: u32,
    side: usize,
    cancel: &AtomicBool,
) -> Result<(Preview, Page)> {
    let image = match &document.source {
        Source::Pdf(doc) => {
            let (image, page) = pdf_load::render(doc.clone(), index, side, cancel)?;
            debug_assert_eq!(page.index, index);
            debug_assert!(Arc::ptr_eq(&page.document, doc));
            image
        }
        Source::Tiff(doc) => tiff_load::render(doc, index, side, cancel)?,
    };
    Ok((image, Page { document, index }))
}
