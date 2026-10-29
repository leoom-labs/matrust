//! Figure/live-Axes compatibility subset with bounded tight cropping and DPI metadata.
use super::{Engine, Value};
use crate::Result;
use std::path::Path;

#[derive(Clone, Copy, PartialEq, Eq)]
enum Content {
    Auto,
    Vector,
    Image,
}

impl Engine {
    pub(super) fn export_graphics_command(&mut self, args: Vec<Value>) -> Result<()> {
        self.check_cancel()?;
        let [target, Value::Text(filename), options @ ..] = args.as_slice() else {
            return Err("Use exportgraphics(figure_or_axes, 'file.ext'[, supported name/value options]); target must be one live Figure/Axes handle or numeric figure ID".into());
        };
        let format = Path::new(filename)
            .extension()
            .and_then(|s| s.to_str())
            .unwrap_or("")
            .to_ascii_lowercase();
        if !matches!(
            format.as_str(),
            "png" | "jpg" | "jpeg" | "tif" | "tiff" | "pdf" | "svg"
        ) {
            return Err(
                "exportgraphics requires a PNG, JPEG, TIFF, PDF or SVG filename extension".into(),
            );
        }
        if !options.len().is_multiple_of(2) {
            return Err("exportgraphics options require quoted name/value pairs".into());
        }
        let mut content = Content::Auto;
        let mut seen_content = false;
        let mut seen_background = false;
        let mut seen_resolution = false;
        let mut resolution = 150_u16;
        let mut transparent = false;
        let mut current_background = true;
        for pair in options.as_chunks::<2>().0 {
            let Value::Text(name) = &pair[0] else {
                return Err("exportgraphics option names must be quoted text".into());
            };
            match name.to_ascii_lowercase().as_str() {
                "contenttype" => {
                    if seen_content {
                        return Err("Duplicate exportgraphics ContentType option".into());
                    }
                    seen_content = true;
                    content = match &pair[1] {
                        Value::Text(value) if value.eq_ignore_ascii_case("auto") => Content::Auto,
                        Value::Text(value) if value.eq_ignore_ascii_case("vector") => Content::Vector,
                        Value::Text(value) if value.eq_ignore_ascii_case("image") => Content::Image,
                        _ => return Err("ContentType must be 'auto', 'vector' or 'image'".into()),
                    };
                }
                "backgroundcolor" => {
                    if seen_background {
                        return Err("Duplicate exportgraphics BackgroundColor option".into());
                    }
                    seen_background = true;
                    current_background = matches!(&pair[1], Value::Text(value) if value.eq_ignore_ascii_case("current"));
                    transparent = matches!(&pair[1], Value::Text(value) if value.eq_ignore_ascii_case("none"));
                    let white = match &pair[1] {
                        Value::Text(value) => matches!(value.to_ascii_lowercase().as_str(), "current" | "white" | "w" | "#ffffff"),
                        Value::Matrix(value) => value.rows() == 1 && value.cols() == 3 && value.data() == [1., 1., 1.],
                        _ => false,
                    };
                    if !white && !transparent {
                        return Err("BackgroundColor supports 'current', 'white', 'w', '#ffffff', [1 1 1] or 'none'; custom colors are unsupported".into());
                    }
                }
                "resolution" => {
                    if seen_resolution {return Err("Duplicate exportgraphics Resolution option".into());}
                    seen_resolution=true;
                    let Value::Matrix(value)=&pair[1] else {return Err("Resolution must be a numeric integer in 1..65535 DPI".into());};
                    let dpi=value.as_scalar()?;
                    if !dpi.is_finite() || dpi.fract()!=0. || !(1. ..=65535.).contains(&dpi) {return Err("Resolution must be an integer in 1..65535 DPI; raster dimensions are additionally bounded".into());}
                    resolution=dpi as u16;
                }
                _ => return Err("Unsupported exportgraphics option; only ContentType, Resolution and white/current/none BackgroundColor are supported. Append, Colorspace, dimensions, padding and units are pending".into()),
            }
        }
        let vector = matches!(format.as_str(), "pdf" | "svg");
        if (content == Content::Vector && !vector) || (content == Content::Image && vector) {
            return Err("exportgraphics supports vector content for PDF/SVG and image content for PNG/JPEG/TIFF; image-in-PDF/SVG and vector-in-raster requests are unsupported".into());
        }
        let path = self.file_path(filename, "")?;
        let (figure, axes) = self.export_graphics_target(target)?;
        #[cfg(feature = "plot-export")]
        {
            let options = crate::export::GraphicsOptions {
                resolution,
                transparent,
            };
            let host = self.graphics_theme_context();
            let background = if current_background {
                figure.resolve_appearance(host).canvas
            } else {
                [255; 3]
            };
            crate::export::export_graphics_in_theme_with_cancel(
                figure,
                (axes, background),
                &path,
                &format,
                options,
                host,
                &self.cancel,
            )
        }
        #[cfg(not(feature = "plot-export"))]
        {
            let _ = (
                path,
                figure,
                axes,
                resolution,
                transparent,
                current_background,
            );
            Err("exportgraphics requires the optional plot-export feature for font-aware tight cropping (included in desktop)".into())
        }
    }
}
