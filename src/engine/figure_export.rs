//! Script exports share bounded codecs while keeping editable document state.
use super::{Engine, Value};
use crate::{Result, files};
use std::path::Path;

impl Engine {
    pub(super) fn figure_export_command(&mut self, args: Vec<Value>) -> Result<()> {
        self.check_cancel()?;
        let [id, Value::Text(filename), options @ ..] = args.as_slice() else {
            return Err("Use saveas(figure_number, filename[, format])".into());
        };
        let id_number = id.clone().matrix(&self.cancel)?.as_scalar()?;
        if !id_number.is_finite()
            || id_number.fract() != 0.
            || !(1. ..=1_000_000.).contains(&id_number)
        {
            return Err("Figure number must be an integer in 1..1000000".into());
        }
        let extension = Path::new(filename)
            .extension()
            .and_then(|s| s.to_str())
            .filter(|s| !s.is_empty())
            .unwrap_or("fig");
        let (format, explicit) = match options {
            [] => (extension.to_lowercase(), false),
            [Value::Text(format)] => (format.to_lowercase(), true),
            _ => return Err("saveas accepts one optional quoted format".into()),
        };
        // MATLAB's m/mfig format arguments mean a FIG plus an opening M-file.
        // MatRust's native .mrfig/.mfig filename extensions have a separate meaning.
        let default_extension = match format.as_str() {
            "fig" => ".fig",
            "mrfig" if !explicit => ".mrfig",
            "mfig" if !explicit => ".mfig",
            "png" => ".png",
            "jpg" | "jpeg" => ".jpg",
            "svg" => ".svg",
            "pdf" => ".pdf",
            "epsc" if explicit => ".eps",
            "eps" | "eps2" | "epsc2" => return Err("Monochrome and Level-2 EPS are unsupported; use explicit saveas(gcf, filename, 'epsc') for opaque color Level-3 EPS".into()),
            "bmp" => ".bmp",
            "tif" | "tiff" | "tiffn" => ".tif",
            _ => return Err("Supported saveas formats: FIG, PNG, JPEG, SVG, PDF, explicit EPSC, BMP, TIFF and TIFFN; MATLAB M/MFIG pairs, EMF and block-diagram handles are not supported yet".into()),
        };
        let path = self.file_path(filename, default_extension)?;
        let figure = self
            .figures()
            .find(|(number, _)| *number == id_number as u32)
            .map(|(_, figure)| figure)
            .ok_or("Figure does not exist; create or select a figure first")?;
        if format == "mrfig"
            || format == "mfig"
            || (format == "fig"
                && path
                    .extension()
                    .and_then(|s| s.to_str())
                    .is_some_and(|s| s.eq_ignore_ascii_case("fig")))
        {
            // The normal editable extension can safely become the document's
            // saved-file identity, with the same exact-version checks as savefig.
            self.figure_file_command("savefig", vec![id.clone(), Value::Text(filename.clone())])?;
            return Ok(());
        }
        if format == "fig" {
            // Respect an explicit format with a mismatched filename. Such an
            // export is not an extension-classified editable document association.
            return files::save_matlab_figure_with_theme(
                &path,
                figure,
                self.graphics_theme_context(),
                &self.cancel,
            );
        }
        if format == "svg" {
            figure.validate()?;
            let svg = figure.svg_with_theme(self.graphics_theme_context());
            self.check_cancel()?;
            return files::atomic_write(&path, svg.as_bytes());
        }
        #[cfg(feature = "plot-export")]
        {
            crate::export::export_figure_as_in_theme_with_cancel(
                figure,
                &path,
                &format,
                [1440, 840],
                self.graphics_theme_context(),
                &self.cancel,
            )
        }
        #[cfg(not(feature = "plot-export"))]
        Err("Raster, PDF and color EPS exports require the optional plot-export feature (included in the desktop build)".into())
    }
}
