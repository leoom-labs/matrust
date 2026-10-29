//! A bounded vector canvas that preserves the canonical plot's proportions.
use super::{BaseTheme, Figure};
use crate::Result;
use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Canvas {
    pub width: f32,
    pub height: f32,
}
impl Default for Canvas {
    fn default() -> Self {
        Self {
            width: 960.,
            height: 560.,
        }
    }
}
impl Canvas {
    pub fn validate(self) -> Result<()> {
        if !self.width.is_finite()
            || !self.height.is_finite()
            || !(64.0..=8192.).contains(&self.width)
            || !(64.0..=8192.).contains(&self.height)
            || self.width.ceil() as u64 * self.height.ceil() as u64 > 32_000_000
        {
            return Err("Canvas size must be 64–8192 units per side, at most 32 million rounded square units".into());
        }
        Ok(())
    }
}
impl Figure {
    /// Fit the existing vector artwork to a canvas, centered on the figure background. PDF
    /// export interprets units as points; SVG uses its intrinsic pixel units.
    pub fn svg_on_canvas(&self, canvas: Canvas) -> Result<String> {
        self.svg_on_canvas_with_theme(canvas, BaseTheme::Light)
    }
    pub fn svg_on_canvas_with_theme(&self, canvas: Canvas, host: BaseTheme) -> Result<String> {
        canvas.validate()?;
        self.validate()?;
        self.fit_svg_canvas(
            self.svg_with_theme(host),
            canvas,
            self.resolve_appearance(host).canvas,
            None,
        )
    }
    #[cfg(all(test, feature = "plot-export"))]
    pub(crate) fn svg_on_canvas_with_cancel(
        &self,
        canvas: Canvas,
        cancel: &std::sync::atomic::AtomicBool,
    ) -> Result<String> {
        self.svg_on_canvas_with_theme_and_cancel(canvas, BaseTheme::Light, cancel)
    }
    /// Bounded EPS preparation with a captured host appearance.
    #[cfg(feature = "plot-export")]
    pub(crate) fn svg_on_canvas_with_theme_and_cancel(
        &self,
        canvas: Canvas,
        host: BaseTheme,
        cancel: &std::sync::atomic::AtomicBool,
    ) -> Result<String> {
        crate::dsp::check_cancel(cancel)?;
        canvas.validate()?;
        self.validate()?;
        crate::dsp::check_cancel(cancel)?;
        self.fit_svg_canvas(
            self.svg_opaque_with_cancel_in_theme(host, cancel)?,
            canvas,
            self.resolve_appearance(host).canvas,
            Some(cancel),
        )
    }
    fn fit_svg_canvas(
        &self,
        mut svg: String,
        canvas: Canvas,
        color: [u8; 3],
        cancel: Option<&std::sync::atomic::AtomicBool>,
    ) -> Result<String> {
        if canvas == Canvas::default() {
            return Ok(svg);
        }
        let scale = (canvas.width / 960.).min(canvas.height / 560.);
        let left = (canvas.width - 960. * scale) * 0.5;
        let top = (canvas.height - 560. * scale) * 0.5;
        let background = if color == [255; 3] {
            std::borrow::Cow::Borrowed("white")
        } else {
            std::borrow::Cow::Owned(format!("#{:02x}{:02x}{:02x}", color[0], color[1], color[2]))
        };
        let header = format!(
            "<svg xmlns=\"http://www.w3.org/2000/svg\" width=\"{}\" height=\"{}\" viewBox=\"0 0 {} {}\"><rect width=\"{}\" height=\"{}\" fill=\"{background}\"/><g transform=\"translate({left} {top}) scale({scale})\">",
            canvas.width, canvas.height, canvas.width, canvas.height, canvas.width, canvas.height
        );
        // The producer owns this root. Replace its short header in-place rather
        // than creating another full SVG copy or copying any sample buffers.
        let end = svg.find('>').ok_or("Invalid generated SVG root")?;
        if !svg.ends_with("</svg>") {
            return Err("Invalid generated SVG closing tag".into());
        }
        if let Some(cancel) = cancel {
            crate::dsp::check_cancel(cancel)?;
            let extra = header.len() + 4;
            let size = svg
                .len()
                .checked_add(extra)
                .filter(|&n| n <= 32 * 1024 * 1024)
                .ok_or("Sized SVG exceeds 32 MiB limit")?;
            if size > svg.capacity() {
                svg.try_reserve_exact(size - svg.len())
                    .map_err(|_| "Could not allocate sized SVG")?;
            }
        } else {
            svg.reserve(header.len() + 4);
        }
        svg.replace_range(svg.len() - 6.., "</g></svg>");
        svg.replace_range(..=end, &header);
        Ok(svg)
    }
}
