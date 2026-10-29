use eframe::egui::{Color32, ColorImage};
use matrust::{Result, plot::Figure};
use std::sync::atomic::{AtomicBool, Ordering};

pub struct Request {
    pub figure: Figure,
    pub width: u32,
    pub height: u32,
}

#[cfg(test)]
pub fn prepare(request: &Request, cancel: &AtomicBool) -> Result<ColorImage> {
    prepare_in_theme(request, matrust::plot::BaseTheme::Light, cancel)
}
pub fn prepare_in_theme(
    request: &Request,
    host: matrust::plot::BaseTheme,
    cancel: &AtomicBool,
) -> Result<ColorImage> {
    let rgba = matrust::export::render_figure_in_theme_with_cancel(
        &request.figure,
        [request.width, request.height],
        host,
        cancel,
    )?;
    let mut pixels = Vec::with_capacity((request.width as usize) * (request.height as usize));
    for chunk in rgba.as_raw().chunks(8192 * 4) {
        if cancel.load(Ordering::Relaxed) {
            return Err("Cancelled".into());
        }
        // The shared renderer composites on the resolved canvas. Opaque pixels avoid native
        // clipboard implementations disagreeing about alpha premultiplication.
        pixels.extend(
            chunk
                .as_chunks::<4>()
                .0
                .iter()
                .map(|p| Color32::from_rgb(p[0], p[1], p[2])),
        );
    }
    if cancel.load(Ordering::Relaxed) {
        return Err("Cancelled".into());
    }
    Ok(ColorImage::new(
        [request.width as usize, request.height as usize],
        pixels,
    ))
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn figure_copy_pixels_match_png_and_keep_opaque_margins() {
        let mut engine = matrust::Engine::default();
        engine.execute("subplot(2,1,1);plot([1 2 3],[3 1 2]);title('Top');subplot(2,1,2);stairs([1 2 3],[2 4 1]);title('Bottom');").unwrap();
        let request = Request {
            figure: engine.figure.unwrap(),
            width: 400,
            height: 400,
        };
        let cancel = AtomicBool::new(false);
        let copied = prepare(&request, &cancel).unwrap();
        assert_eq!(copied.size, [400, 400]);
        assert!(copied.pixels.iter().all(|p| p.a() == 255));
        let background = request
            .figure
            .resolve_appearance(matrust::plot::BaseTheme::Light)
            .canvas;
        assert_eq!(
            copied.pixels[0],
            Color32::from_rgb(background[0], background[1], background[2])
        );
        // Small subplot strokes are antialiased against white; test their blue
        // chroma rather than requiring an unblended palette pixel.
        assert!(
            copied
                .pixels
                .iter()
                .any(|p| p.b().saturating_sub(p.r()) > 30 && p.b() > p.g())
        );
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("same-view.png");
        matrust::export::export_figure(&request.figure, &path, 400, 400).unwrap();
        let png = image::open(path).unwrap().into_rgba8();
        for (actual, expected) in copied.pixels.iter().zip(png.pixels()) {
            assert_eq!(actual.to_array(), expected.0);
        }
    }
    #[test]
    fn figure_copy_rejects_cancel_invalid_dimensions_and_invalid_figure() {
        let mut engine = matrust::Engine::default();
        engine.execute("plot([1 2 3]);").unwrap();
        let mut request = Request {
            figure: engine.figure.unwrap(),
            width: 400,
            height: 300,
        };
        assert_eq!(
            prepare(&request, &AtomicBool::new(true)).unwrap_err(),
            "Cancelled"
        );
        for (width, height) in [(0, 400), (63, 400), (8193, 400), (8192, 8192)] {
            request.width = width;
            request.height = height;
            assert!(prepare(&request, &AtomicBool::new(false)).is_err());
        }
        request.width = 400;
        request.height = 300;
        request.figure.xlim = Some([1., 1.]);
        assert!(prepare(&request, &AtomicBool::new(false)).is_err());
    }
}
