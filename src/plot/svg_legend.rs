//! Bounded export legends. Original labels and samples remain in the figure.
use super::{color_hex, draw_marker, escape};
use crate::plot::{Axes, LinePattern, PlotKind, ResolvedAppearance};
use std::borrow::Cow;
use unicode_segmentation::UnicodeSegmentation;

#[derive(Debug)]
struct Layout {
    rows: usize,
    columns: usize,
    column_width: f64,
    scale: f64,
}
impl Layout {
    fn new(count: usize, width: f64, height: f64) -> Self {
        debug_assert!(count > 0);
        let preferred = count.div_ceil(18);
        let mut best = Self {
            rows: count,
            columns: 1,
            column_width: 220.,
            scale: 0.,
        };
        for columns in 1..=count {
            let rows = count.div_ceil(columns);
            let scale = 1_f64
                .min(width / (140. * columns as f64))
                .min(height / (22. * rows as f64 + 12.));
            if scale > best.scale
                || (scale == best.scale
                    && columns.abs_diff(preferred) < best.columns.abs_diff(preferred))
            {
                best = Self {
                    rows,
                    columns,
                    column_width: (width / (columns as f64 * scale)).min(220.),
                    scale,
                };
            }
        }
        best
    }
    fn width(&self) -> f64 {
        self.column_width * self.columns as f64
    }
    fn height(&self) -> f64 {
        22. * self.rows as f64 + 12.
    }
}

// A cheap estimate chooses an abbreviation; the SVG clip is the hard geometric
// bound across fonts. Whole extended grapheme clusters are kept. The byte cap
// also prevents a pathological cluster from filling the renderer with glyphs.
fn fit_label(label: &str, width: f64) -> Cow<'_, str> {
    let mut used = 0.;
    let mut end = 0;
    let budget = (width - 12.).max(0.);
    for (index, grapheme) in label.grapheme_indices(true) {
        let advance = if grapheme.is_ascii() {
            match grapheme.as_bytes()[0] {
                b'i' | b'l' | b'I' | b' ' | b'.' | b',' => 4.,
                b'W' | b'M' | b'@' => 11.,
                _ => 7.,
            }
        } else {
            12.
        };
        if used + advance > budget || index + grapheme.len() > 512 {
            return Cow::Owned(format!("{}\u{2026}", &label[..end]));
        }
        used += advance;
        end = index + grapheme.len();
    }
    Cow::Borrowed(label)
}

pub(super) fn extent(axes: &Axes, available: [f64; 2]) -> Option<[f64; 2]> {
    let count = axes.legend_entries().count();
    if !axes.legend || count == 0 || available.iter().any(|v| !v.is_finite() || *v <= 0.) {
        return None;
    }
    let layout = Layout::new(count, available[0], available[1]);
    Some([
        layout.width() * layout.scale,
        layout.height() * layout.scale,
    ])
}

pub(super) fn draw(
    svg: &mut impl std::fmt::Write,
    axes: &Axes,
    bounds: [f64; 4],
    clip_id: &str,
    placement: Option<crate::plot::LegendRect>,
    location: crate::plot::LegendLocation,
    appearance: ResolvedAppearance,
) {
    if !axes.legend {
        return;
    }
    let appearance = axes.legend_appearance(appearance);
    let entries: Vec<_> = axes.legend_entries().collect();
    if entries.is_empty() {
        return;
    }
    let [axis_left, axis_top, axis_right, axis_bottom] = bounds;
    // Preserve ordinary placement, reducing padding for the minimum custom
    // axes instead of allowing an inset larger than the data box itself.
    let axis_width = axis_right - axis_left;
    let axis_height = axis_bottom - axis_top;
    if axis_width <= 0. || axis_height <= 0. {
        return;
    }
    let inset_x = 10_f64.min(axis_width * 0.1);
    let inset_y = 10_f64.min(axis_height * 0.1);
    let width = axis_width - inset_x;
    let height = axis_height - 2. * inset_y;
    let layout = Layout::new(
        entries.len(),
        placement.map_or(width, |r| r.width()),
        placement.map_or(height, |r| r.height()),
    );
    let [horizontal, vertical] = location.alignment();
    let left = placement.map_or_else(
        || axis_left + inset_x + (width - layout.width() * layout.scale).max(0.) * horizontal,
        |r| r.left,
    );
    let top = placement.map_or_else(
        || axis_top + inset_y + (height - layout.height() * layout.scale).max(0.) * vertical,
        |r| r.top,
    );
    let transform = if layout.scale < 1. {
        format!(
            " transform=\"matrix({} 0 0 {} {} {})\"",
            layout.scale,
            layout.scale,
            left * (1. - layout.scale),
            top * (1. - layout.scale)
        )
    } else {
        String::new()
    };
    let background = if appearance.legend_background == [255; 3] {
        String::from("white")
    } else {
        color_hex(appearance.legend_background)
    };
    let border = if appearance.legend_border == [170; 3] {
        String::from("#aaa")
    } else {
        color_hex(appearance.legend_border)
    };
    let _ = write!(
        svg,
        "<g font-family=\"Arial\" font-size=\"12\"{transform}><rect x=\"{left}\" y=\"{top}\" width=\"{}\" height=\"{}\" fill=\"{background}\" stroke=\"{border}\"/>",
        layout.width(),
        layout.height()
    );
    for (row, (index, series)) in entries.into_iter().enumerate() {
        let y = top + 20. + (row % layout.rows) as f64 * 22.;
        let x = left + 10. + (row / layout.rows) as f64 * layout.column_width;
        let color = color_hex(if series.visible {
            appearance.series_color(series)
        } else {
            appearance.disabled
        });
        let width = series.stroke_width(2.).min(3.);
        let _ = write!(svg, "<g><title>{}</title>", escape(&series.label));
        match series.kind {
            PlotKind::Bar | PlotKind::Patch => {
                let _ = write!(
                    svg,
                    "<rect x=\"{x}\" y=\"{}\" width=\"24\" height=\"10\" fill=\"{color}\"/>",
                    y - 5.
                );
                if let Some(edge) = series.face_edge_color
                    && series.pattern != LinePattern::None
                {
                    let edge = color_hex(if series.visible {
                        edge
                    } else {
                        appearance.disabled
                    });
                    let dash = match series.pattern {
                        LinePattern::Dashed => "8 5",
                        LinePattern::Dotted => "2 4",
                        LinePattern::DashDot => "8 4 2 4",
                        _ => "",
                    };
                    let _ = write!(
                        svg,
                        "<rect x=\"{x}\" y=\"{}\" width=\"24\" height=\"10\" fill=\"none\" stroke=\"{edge}\" stroke-width=\"{}\" stroke-dasharray=\"{dash}\"/>",
                        y - 5.,
                        series.face_edge_width().min(3.)
                    );
                }
            }
            PlotKind::Line | PlotKind::Stairs | PlotKind::Stem
                if series.pattern != LinePattern::None =>
            {
                let dash = match series.pattern {
                    LinePattern::Solid | LinePattern::None => "",
                    LinePattern::Dashed => "8 5",
                    LinePattern::Dotted => "2 4",
                    LinePattern::DashDot => "8 4 2 4",
                };
                let _ = write!(
                    svg,
                    "<path d=\"M{x} {y}h24\" stroke=\"{color}\" stroke-width=\"{width}\" stroke-dasharray=\"{dash}\"/>"
                );
            }
            PlotKind::Scatter | PlotKind::Line | PlotKind::Stairs | PlotKind::Stem => {}
        }
        if series.kind == PlotKind::Scatter || series.markers {
            draw_marker(svg, series, x + 12., y, &color, true, appearance);
        }
        let text_x = x + 32.;
        let label_width = (layout.column_width - 52.).max(1.);
        let label = fit_label(&series.label, label_width);
        let _ = write!(
            svg,
            "<defs><clipPath id=\"{clip_id}-legend-{index}\"><rect x=\"{text_x}\" y=\"{}\" width=\"{label_width}\" height=\"22\"/></clipPath></defs><g clip-path=\"url(#{clip_id}-legend-{index})\"><text x=\"{text_x}\" y=\"{}\" fill=\"{}\">{}</text></g></g>",
            y - 14.,
            y + 4.,
            color_hex(if series.visible {
                appearance.text
            } else {
                appearance.disabled
            }),
            escape(&label)
        );
    }
    let _ = svg.write_str("</g>");
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn export_legend_layout_fits_dense_and_small_axes() {
        for count in 1..=64 {
            for width in [24., 80., 160., 320., 840.] {
                for height in [12., 60., 120., 410.] {
                    let layout = Layout::new(count, width, height);
                    assert!(layout.scale > 0. && layout.scale <= 1.);
                    assert!(layout.width() * layout.scale <= width + 1e-6);
                    assert!(layout.height() * layout.scale <= height + 1e-6);
                    assert!(layout.rows * layout.columns >= count);
                    assert!(layout.column_width >= 140. - 1e-6 && layout.column_width <= 220.);
                }
            }
        }
        let normal = Layout::new(64, 840., 410.);
        assert_eq!(normal.scale, 1.);
        assert_eq!(normal.columns, 4);
        assert_eq!(normal.column_width, 210.);
    }
    #[test]
    fn export_legend_abbreviation_keeps_unicode_clusters_and_bounded_prefix() {
        let cluster = "a\u{301}";
        let label = cluster.repeat(80);
        let abbreviated = fit_label(&label, 70.);
        assert!(abbreviated.ends_with('\u{2026}'));
        let prefix = abbreviated.trim_end_matches('\u{2026}');
        assert!(prefix.graphemes(true).all(|g| g == cluster));
        let emoji = "\u{1f469}\u{200d}\u{1f4bb}";
        let label = emoji.repeat(80);
        assert!(
            fit_label(&label, 70.)
                .trim_end_matches('\u{2026}')
                .graphemes(true)
                .all(|g| g == emoji)
        );
        assert_eq!(fit_label("Signal", 168.), "Signal");
        let pathological = format!("a{}", "\u{301}".repeat(1000));
        assert_eq!(fit_label(&pathological, 168.), "\u{2026}");
    }
}
