//! Hidden read-only PDF renderer. Never initialize a desktop graphics context here.
#[allow(dead_code)]
#[path = "desktop/image_load.rs"]
mod image_load;
#[path = "desktop/path_label.rs"]
mod path_label;
#[allow(dead_code)]
#[path = "desktop/pdf_native.rs"]
mod pdf_native;
#[allow(dead_code)]
#[path = "desktop/pdf_protocol.rs"]
mod pdf_protocol;
use pdf_protocol as protocol;
use std::{io::Write, sync::atomic::AtomicBool};

// Shared transport has different readers in desktop and helper.
#[allow(dead_code, unused_imports)]
#[path = "desktop/eps_protocol.rs"]
mod eps_protocol;
#[path = "desktop/eps_worker.rs"]
mod eps_worker;

fn run() -> matrust::Result<()> {
    if std::env::args().nth(1).as_deref() == Some("--eps-protocol-v1") {
        return eps_worker::run();
    }
    if std::env::args().nth(1).as_deref() != Some("--pdf-protocol-v1") {
        return Err("This helper is launched by the MatRust desktop".into());
    }
    let _apartment = pdf_native::Apartment::new()?;
    let cancel = AtomicBool::new(false);
    let mut document = None;
    let mut input = std::io::stdin().lock();
    let mut output = std::io::stdout().lock();
    while let Ok(request) = protocol::read_json::<protocol::Request>(&mut input) {
        let result: matrust::Result<Option<image_load::Preview>> = match request {
            protocol::Request::Open(path) => pdf_native::open(&path, &cancel).map(|doc| {
                document = Some(doc);
                None
            }),
            protocol::Request::Render {
                index,
                texture_side,
            } => document
                .as_ref()
                .ok_or_else(|| "Open a PDF first".to_string())
                .and_then(|doc| {
                    pdf_native::render(doc.clone(), index, texture_side, &cancel)
                        .map(|(image, _)| Some(image))
                }),
        };
        let mut header = protocol::Header::default();
        if let Some(doc) = &document {
            header.file_bytes = doc.file_bytes;
            header.source_digest = Some(doc.source_digest);
            header.page_count = doc.page_count;
        }
        let image = match result {
            Ok(image) => image,
            Err(error) => {
                header.error = Some(error);
                None
            }
        };
        if let Some(image) = &image {
            header.original_size = image.original_size;
            header.size = image.pixels.size.map(|v| v as u32);
        }
        protocol::write_json(&mut output, &header)?;
        if let Some(image) = image {
            let mut buffer = [0; 64 * 1024];
            for chunk in image.pixels.pixels.chunks(buffer.len() / 4) {
                for (pixel, bytes) in chunk.iter().zip(buffer.as_chunks_mut::<4>().0) {
                    bytes.copy_from_slice(&pixel.to_array());
                }
                output
                    .write_all(&buffer[..chunk.len() * 4])
                    .map_err(|e| e.to_string())?;
            }
            output.flush().map_err(|e| e.to_string())?;
        }
    }
    Ok(())
}
fn main() {
    if let Err(error) = run() {
        eprintln!("{error}");
        std::process::exit(1);
    }
}
