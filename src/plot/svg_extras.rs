//! SVG drawing of axes-owned objects: text, the right y-axis and colorbars.
use super::buffer::SvgBuffer;
use super::{color_hex, escape, tick_text, ticks};
use crate::plot::{
    Axes, ColorbarLocation, ConstantLineHAlign, ConstantLineVAlign, InterceptAxis, LinePattern,
    ResolvedAppearance, TextAlign, VerticalAlign, YyAxis,
};
use std::fmt::Write;

const RIGHT_AXIS: f64 = 56.;
const BAR_WIDTH: f64 = 14.;
const BAR_GAP: f64 = 16.;
const BAR_LABELS: f64 = 64.;
const BAR_BOTTOM: f64 = 64.;

fn percent(rgb: [f64; 3]) -> String {
    color_hex(rgb.map(|v| (v.clamp(0., 1.) * 255.).round() as u8))
}

/// Pixels taken from the right and bottom of the data rectangle.
pub(super) fn reserve(axes: &Axes) -> [f64; 2] {
    let extras = &axes.extras;
    let mut right = 0.;
    let mut bottom = 0.;
    if extras.yy.as_ref().is_some_and(|y| y.resolved.is_some()) {
        right += RIGHT_AXIS
            + if axes.extras.yy.as_ref().unwrap().right_label.is_empty() {
                0.
            } else {
                20.
            };
    }
    if let Some(bar) = extras.colorbar.as_ref().filter(|b| b.visible) {
        match bar.location {
            ColorbarLocation::EastOutside => {
                right +=
                    BAR_GAP + BAR_WIDTH + BAR_LABELS + if bar.label.is_empty() { 0. } else { 20. }
            }
            ColorbarLocation::SouthOutside => {
                bottom += BAR_BOTTOM + if bar.label.is_empty() { 0. } else { 18. }
            }
        }
    }
    [right, bottom]
}

/// Right edge of the yyaxis gutter, whether or not the axis is painted here.
fn right_axis_end(right: f64, labeled: bool) -> f64 {
    if labeled {
        right + RIGHT_AXIS + 20.
    } else {
        right + RIGHT_AXIS
    }
}

fn write_right_axis(
    svg: &mut SvgBuffer<'_>,
    yy: &YyAxis,
    [l, r]: [[f64; 2]; 2],
    [top, right, bottom]: [f64; 3],
    py: &dyn Fn(f64) -> f64,
) {
    let color = percent(yy.right_color);
    let _ = write!(
        svg,
        "<g data-matrust-right-axis=\"true\" fill=\"{color}\"><path d=\"M{right} {top}V{bottom}\" fill=\"none\" stroke=\"{color}\"/>"
    );
    let (values, step) = ticks(r);
    for value in values {
        let mapped = l[0] + (value - r[0]) * (l[1] - l[0]) / (r[1] - r[0]);
        let y = py(mapped);
        if !y.is_finite() {
            continue;
        }
        let _ = write!(
            svg,
            "<path d=\"M{} {y:.2}H{right}\" stroke=\"{color}\"/><text x=\"{}\" y=\"{y:.2}\" font-size=\"12\" dominant-baseline=\"central\">{}</text>",
            right - 6.,
            right + 8.,
            tick_text(value, step)
        );
    }
    if !yy.right_label.is_empty() {
        let x = right + RIGHT_AXIS + 6.;
        let _ = write!(
            svg,
            "<text transform=\"translate({x},{}) rotate(90)\" text-anchor=\"middle\" font-size=\"{}\"{}>{}</text>",
            (top + bottom) / 2.,
            yy.right_label_style
                .export_font_size_pixels(16., super::svg::EXPORT_PX_PER_POINT),
            super::svg::style_attrs(yy.right_label_style),
            escape(&yy.right_label)
        );
    }
    svg.push_str("</g>");
}

/// The yyaxis right axis, painted on its own so a Layer-top grid can sit under it.
pub(super) fn draw_right_axis(
    svg: &mut SvgBuffer<'_>,
    axes: &Axes,
    [top, right, bottom]: [f64; 3],
    py: &dyn Fn(f64) -> f64,
) {
    let Some(yy) = &axes.extras.yy else {
        return;
    };
    let Some([l, r]) = yy.resolved else {
        return;
    };
    svg.push_str("<g font-family=\"Arial\">");
    write_right_axis(svg, yy, [l, r], [top, right, bottom], py);
    svg.push_str("</g>");
}

pub(super) fn draw(
    svg: &mut SvgBuffer<'_>,
    axes: &Axes,
    [left, top, right, bottom]: [f64; 4],
    px: &dyn Fn(f64) -> f64,
    py: &dyn Fn(f64) -> f64,
    appearance: ResolvedAppearance,
    defer_right_axis: bool,
) {
    let extras = &axes.extras;
    let yy_here = extras.yy.as_ref().is_some_and(|yy| yy.resolved.is_some()) && !defer_right_axis;
    if extras.texts.is_empty()
        && extras.colorbar.is_none()
        && !yy_here
        && extras.constant_lines.is_empty()
        && extras.rectangles.is_empty()
    {
        return;
    }
    let text_color = color_hex(appearance.text);
    let axis_color = color_hex(appearance.axis);
    svg.push_str("<g font-family=\"Arial\">");
    let mut next_right = right;
    if let Some(yy) = &extras.yy
        && let Some([l, r]) = yy.resolved
    {
        if yy_here {
            write_right_axis(svg, yy, [l, r], [top, right, bottom], py);
        }
        next_right = right_axis_end(right, !yy.right_label.is_empty());
    }
    if !extras.rectangles.is_empty() || !extras.constant_lines.is_empty() {
        let w = (right - left).max(0.);
        let h = (bottom - top).max(0.);
        let _ = write!(
            svg,
            "<svg x=\"{left:.2}\" y=\"{top:.2}\" width=\"{w:.2}\" height=\"{h:.2}\" viewBox=\"{left:.2} {top:.2} {w:.2} {h:.2}\">"
        );
        for rect in extras.rectangles.iter().filter(|r| r.visible) {
            let x0 = px(rect.position[0]);
            let x1 = px(rect.position[0] + rect.position[2]);
            let y0 = py(rect.position[1]);
            let y1 = py(rect.position[1] + rect.position[3]);
            if ![x0, x1, y0, y1]
                .iter()
                .all(|v| v.is_finite() && v.abs() <= 1e6)
            {
                continue;
            }
            let sx = x0.min(x1);
            let sy = y0.min(y1);
            let sw = (x1 - x0).abs();
            let sh = (y1 - y0).abs();
            let fill = rect.face_color.map_or_else(|| "none".to_owned(), percent);
            let stroke = if rect.pattern == LinePattern::None {
                "none".to_owned()
            } else {
                rect.edge_color.map_or_else(|| "none".to_owned(), percent)
            };
            let stroke_width = rect.line_width * 4. / 3.;
            let dash = rect
                .pattern
                .svg_dasharray()
                .map_or(String::new(), |d| format!(" stroke-dasharray=\"{d}\""));
            let rx = rect.curvature[0] * 0.5 * sw;
            let ry = rect.curvature[1] * 0.5 * sh;
            let radii = if rx > 0. || ry > 0. {
                format!(" rx=\"{rx:.2}\" ry=\"{ry:.2}\"")
            } else {
                String::new()
            };
            let _ = write!(
                svg,
                "<rect x=\"{sx:.2}\" y=\"{sy:.2}\" width=\"{sw:.2}\" height=\"{sh:.2}\"{radii} fill=\"{fill}\" stroke=\"{stroke}\" stroke-width=\"{stroke_width}\"{dash}/>"
            );
        }
        for line in extras
            .constant_lines
            .iter()
            .filter(|l| l.visible && l.pattern != LinePattern::None)
        {
            let stroke = line.color.map_or_else(|| "#262626".to_owned(), percent);
            let stroke_width = line.line_width * 4. / 3.;
            let dash = line
                .pattern
                .svg_dasharray()
                .map_or(String::new(), |d| format!(" stroke-dasharray=\"{d}\""));
            let opacity = if line.alpha < 1. {
                format!(" stroke-opacity=\"{}\"", line.alpha)
            } else {
                String::new()
            };
            match line.axis {
                InterceptAxis::X => {
                    let x = px(line.value);
                    if !x.is_finite() || x < left - 0.5 || x > right + 0.5 {
                        continue;
                    }
                    let _ = write!(
                        svg,
                        "<line x1=\"{x:.2}\" y1=\"{top:.2}\" x2=\"{x:.2}\" y2=\"{bottom:.2}\" stroke=\"{stroke}\" stroke-width=\"{stroke_width}\"{opacity}{dash}/>"
                    );
                    if !line.label.is_empty() {
                        let (tx, anchor) = match line.label_h_align {
                            ConstantLineHAlign::Left => (x - 4., "end"),
                            ConstantLineHAlign::Center => (x, "middle"),
                            ConstantLineHAlign::Right => (x + 4., "start"),
                        };
                        let (ty, baseline) = match line.label_v_align {
                            ConstantLineVAlign::Top => (top + 6., "hanging"),
                            ConstantLineVAlign::Middle => ((top + bottom) / 2., "central"),
                            ConstantLineVAlign::Bottom => (bottom - 6., "text-after-edge"),
                        };
                        let _ = write!(
                            svg,
                            "<text x=\"{tx:.2}\" y=\"{ty:.2}\" fill=\"{stroke}\" font-size=\"12\" text-anchor=\"{anchor}\" dominant-baseline=\"{baseline}\">{}</text>",
                            escape(&line.label)
                        );
                    }
                }
                InterceptAxis::Y => {
                    let y = py(line.value);
                    if !y.is_finite() || y < top - 0.5 || y > bottom + 0.5 {
                        continue;
                    }
                    let _ = write!(
                        svg,
                        "<line x1=\"{left:.2}\" y1=\"{y:.2}\" x2=\"{right:.2}\" y2=\"{y:.2}\" stroke=\"{stroke}\" stroke-width=\"{stroke_width}\"{opacity}{dash}/>"
                    );
                    if !line.label.is_empty() {
                        let (tx, anchor) = match line.label_h_align {
                            ConstantLineHAlign::Left => (left + 6., "start"),
                            ConstantLineHAlign::Center => ((left + right) / 2., "middle"),
                            ConstantLineHAlign::Right => (right - 6., "end"),
                        };
                        let (ty, baseline) = match line.label_v_align {
                            ConstantLineVAlign::Top => (y - 4., "text-after-edge"),
                            ConstantLineVAlign::Middle => (y, "central"),
                            ConstantLineVAlign::Bottom => (y + 4., "hanging"),
                        };
                        let _ = write!(
                            svg,
                            "<text x=\"{tx:.2}\" y=\"{ty:.2}\" fill=\"{stroke}\" font-size=\"12\" text-anchor=\"{anchor}\" dominant-baseline=\"{baseline}\">{}</text>",
                            escape(&line.label)
                        );
                    }
                }
            }
        }
        svg.push_str("</svg>");
    }
    for text in extras
        .texts
        .iter()
        .filter(|t| t.visible && !t.string.is_empty())
    {
        let x = px(text.position[0]);
        let y = py(text.position[1]);
        if !x.is_finite() || !y.is_finite() || x.abs() > 1e6 || y.abs() > 1e6 {
            continue;
        }
        let anchor = match text.horizontal {
            TextAlign::Left => "start",
            TextAlign::Center => "middle",
            TextAlign::Right => "end",
        };
        let baseline = match text.vertical {
            VerticalAlign::Middle => "central",
            VerticalAlign::Top | VerticalAlign::Cap => "hanging",
            VerticalAlign::Bottom => "text-after-edge",
            VerticalAlign::Baseline => "alphabetic",
        };
        let fill = text.color.map_or_else(|| text_color.clone(), percent);
        let rotate = if text.rotation != 0. {
            format!(" transform=\"rotate({} {x:.2} {y:.2})\"", -text.rotation)
        } else {
            String::new()
        };
        let font_size = if text.font_size_explicit {
            text.font_size * f64::from(super::svg::EXPORT_PX_PER_POINT)
        } else {
            text.font_size * 4. / 3.
        };
        let weight = if text.bold { "bold" } else { "normal" };
        let style_attr = if text.italic {
            r#" font-style="italic""#
        } else {
            ""
        };
        let markup = super::SvgMarkup::new_with_style(
            &text.string,
            text.interpreter,
            font_size as f32,
            text.horizontal,
            text.bold,
            text.italic,
        );
        let height = markup.height(font_size);
        let top = match text.vertical {
            VerticalAlign::Middle => y - height / 2.,
            VerticalAlign::Top | VerticalAlign::Cap => y,
            VerticalAlign::Bottom => y - height,
            VerticalAlign::Baseline => y + font_size * 0.2 - height,
        };
        markup.write(
            svg,
            format_args!(r#" x="{x:.2}" y="{y:.2}" text-anchor="{anchor}" dominant-baseline="{baseline}""#),
            [x, top],
            text.horizontal,
            format_args!(r#" data-matrust-text="true" font-size="{font_size}" font-weight="{weight}"{style_attr} fill="{fill}"{rotate}"#),
            &fill,
        );
    }
    if let Some(bar) = extras.colorbar.as_ref().filter(|b| b.visible) {
        let map = extras.effective_colormap();
        let limits = extras.color_limits();
        let rows = map.len().clamp(1, 256);
        let font = extras.colorbar_font_size() * 4. / 3.;
        let (values, step) = ticks(limits);
        let fraction = |v: f64| (v - limits[0]) / (limits[1] - limits[0]);
        let _ = write!(
            svg,
            "<g data-matrust-colorbar=\"{}\" fill=\"{text_color}\">",
            bar.location.as_str()
        );
        match bar.location {
            ColorbarLocation::EastOutside => {
                let x = next_right + BAR_GAP;
                let height = bottom - top;
                for i in 0..rows {
                    let rgb = map[i * map.len() / rows];
                    let y0 = bottom - height * (i + 1) as f64 / rows as f64;
                    let _ = write!(
                        svg,
                        "<rect x=\"{x:.2}\" y=\"{y0:.2}\" width=\"{BAR_WIDTH}\" height=\"{:.2}\" fill=\"{}\"/>",
                        height / rows as f64 + 0.5,
                        percent(rgb)
                    );
                }
                let _ = write!(
                    svg,
                    "<rect x=\"{x:.2}\" y=\"{top}\" width=\"{BAR_WIDTH}\" height=\"{height}\" fill=\"none\" stroke=\"{axis_color}\"/>"
                );
                for value in values {
                    let y = bottom - height * fraction(value);
                    let _ = write!(
                        svg,
                        "<text x=\"{:.2}\" y=\"{y:.2}\" font-size=\"{font}\" dominant-baseline=\"central\">{}</text>",
                        x + BAR_WIDTH + 6.,
                        tick_text(value, step)
                    );
                }
                if !bar.label.is_empty() {
                    let _ = write!(
                        svg,
                        "<text transform=\"translate({:.2},{:.2}) rotate(90)\" text-anchor=\"middle\" font-size=\"16\">{}</text>",
                        x + BAR_WIDTH + BAR_LABELS,
                        (top + bottom) / 2.,
                        escape(&bar.label)
                    );
                }
                if !bar.title.is_empty() {
                    let title_y = (top - 10.).max(14.);
                    let _ = write!(
                        svg,
                        "<text x=\"{:.2}\" y=\"{title_y:.2}\" text-anchor=\"middle\" font-size=\"16\">{}</text>",
                        x + BAR_WIDTH / 2.,
                        escape(&bar.title)
                    );
                }
            }
            ColorbarLocation::SouthOutside => {
                let y = bottom + 34.;
                let width = right - left;
                for i in 0..rows {
                    let rgb = map[i * map.len() / rows];
                    let x0 = left + width * i as f64 / rows as f64;
                    let _ = write!(
                        svg,
                        "<rect x=\"{x0:.2}\" y=\"{y:.2}\" width=\"{:.2}\" height=\"{BAR_WIDTH}\" fill=\"{}\"/>",
                        width / rows as f64 + 0.5,
                        percent(rgb)
                    );
                }
                let _ = write!(
                    svg,
                    "<rect x=\"{left}\" y=\"{y:.2}\" width=\"{width}\" height=\"{BAR_WIDTH}\" fill=\"none\" stroke=\"{axis_color}\"/>"
                );
                for value in values {
                    let x = left + width * fraction(value);
                    let _ = write!(
                        svg,
                        "<text x=\"{x:.2}\" y=\"{:.2}\" font-size=\"{font}\" text-anchor=\"middle\">{}</text>",
                        y + BAR_WIDTH + 14.,
                        tick_text(value, step)
                    );
                }
                if !bar.label.is_empty() {
                    let _ = write!(
                        svg,
                        "<text x=\"{:.2}\" y=\"{:.2}\" text-anchor=\"middle\" font-size=\"16\">{}</text>",
                        (left + right) / 2.,
                        y + BAR_WIDTH + 34.,
                        escape(&bar.label)
                    );
                }
            }
        }
        svg.push_str("</g>");
    }
    svg.push_str("</g>");
}
