//! Native painting of axes-owned objects: `text`, the `yyaxis` right axis
//! and colorbars. Uses the plot's final transform, so pan and zoom move the
//! overlays with the data. Same font and axis colours as the plot itself.
use eframe::egui::{self, Color32};
use egui_plot::{PlotPoint, PlotTransform};
use matrust::plot::{
    Axes, AxisTransform, ColorbarLocation, ConstantLineHAlign, ConstantLineVAlign, InterceptAxis,
    LinePattern, ResolvedAppearance, TextAlign, VerticalAlign, YyAxis,
};

#[derive(Clone, Copy, Default)]
pub(super) struct TextEmphasis {
    pub bold: bool,
    pub italic: bool,
}

impl From<matrust::plot::LabelStyle> for TextEmphasis {
    fn from(style: matrust::plot::LabelStyle) -> Self {
        Self {
            bold: style.bold == Some(true),
            italic: style.italic == Some(true),
        }
    }
}

fn append_markup_nodes(
    job: &mut egui::text::LayoutJob,
    nodes: &[matrust::plot::text_markup::Node],
    font: &egui::FontId,
    color: Color32,
    emphasis: TextEmphasis,
    depth: usize,
) {
    if depth >= 32 {
        return;
    }
    for node in nodes {
        match node {
            matrust::plot::text_markup::Node::Run(run) => {
                let mut format = egui::TextFormat {
                    font_id: egui::FontId::new(font.size * run.scale, font.family.clone()),
                    color,
                    italics: emphasis.italic || run.italic,
                    ..Default::default()
                };
                if emphasis.bold || run.bold {
                    format.coords.push("wght", 700.);
                }
                format.valign = if run.shift > 0. {
                    egui::Align::TOP
                } else {
                    egui::Align::BOTTOM
                };
                job.append(&run.text, 0., format);
            }
            matrust::plot::text_markup::Node::Fraction {
                num,
                den,
                scale,
                bold,
                italic,
            } => {
                append_markup_nodes(job, num, font, color, emphasis, depth + 1);
                let mut slash = egui::TextFormat {
                    font_id: egui::FontId::new(font.size * *scale, font.family.clone()),
                    color,
                    italics: emphasis.italic || *italic,
                    valign: egui::Align::BOTTOM,
                    ..Default::default()
                };
                if emphasis.bold || *bold {
                    slash.coords.push("wght", 700.);
                }
                job.append("/", 0., slash);
                append_markup_nodes(job, den, font, color, emphasis, depth + 1);
            }
            matrust::plot::text_markup::Node::Sqrt {
                body,
                scale,
                bold,
                italic,
            } => {
                let mut rad = egui::TextFormat {
                    font_id: egui::FontId::new(font.size * *scale, font.family.clone()),
                    color,
                    italics: emphasis.italic || *italic,
                    valign: egui::Align::BOTTOM,
                    ..Default::default()
                };
                if emphasis.bold || *bold {
                    rad.coords.push("wght", 700.);
                }
                job.append("√(", 0., rad.clone());
                append_markup_nodes(job, body, font, color, emphasis, depth + 1);
                job.append(")", 0., rad);
            }
        }
    }
}

pub(super) fn markup_job(
    source: &str,
    interpreter: matrust::plot::text_markup::Interpreter,
    font: egui::FontId,
    color: Color32,
) -> egui::text::LayoutJob {
    markup_job_styled(source, interpreter, font, color, TextEmphasis::default())
}

pub(super) fn markup_job_styled(
    source: &str,
    interpreter: matrust::plot::text_markup::Interpreter,
    font: egui::FontId,
    color: Color32,
    emphasis: TextEmphasis,
) -> egui::text::LayoutJob {
    let nodes = matrust::plot::text_markup::parse_nodes(source, interpreter);
    markup_job_from_nodes_styled(&nodes, &font, color, emphasis)
}

pub(super) fn markup_job_from_nodes_styled(
    nodes: &[matrust::plot::text_markup::Node],
    font: &egui::FontId,
    color: Color32,
    emphasis: TextEmphasis,
) -> egui::text::LayoutJob {
    let mut job = egui::text::LayoutJob::default();
    append_markup_nodes(&mut job, nodes, font, color, emphasis, 0);
    job
}

pub(super) struct MarkupLayout {
    pub(super) size: egui::Vec2,
    pub(super) glyphs: Vec<(egui::Vec2, std::sync::Arc<egui::Galley>)>,
    pub(super) rules: Vec<([egui::Pos2; 2], f32)>,
    bold: bool,
}

impl MarkupLayout {
    pub(super) fn from_galley(galley: std::sync::Arc<egui::Galley>) -> Self {
        Self {
            size: galley.size(),
            glyphs: vec![(egui::Vec2::ZERO, galley)],
            rules: Vec::new(),
            bold: false,
        }
    }

    pub(super) fn with_bold(mut self, bold: bool) -> Self {
        self.bold = bold;
        self
    }

    pub(super) fn paint(
        &self,
        painter: &egui::Painter,
        top_left: egui::Pos2,
        color: Color32,
        angle: f32,
    ) {
        let (sin, cos) = angle.sin_cos();
        let rotate = |v: egui::Vec2| -> egui::Vec2 {
            if angle == 0. {
                v
            } else {
                egui::vec2(v.x * cos - v.y * sin, v.x * sin + v.y * cos)
            }
        };
        for (offset, galley) in &self.glyphs {
            let pos = top_left + rotate(*offset);
            if self.bold {
                let bold_offset = rotate(egui::vec2(1. / painter.pixels_per_point(), 0.));
                let bold_shape =
                    egui::epaint::TextShape::new(pos + bold_offset, galley.clone(), color);
                painter.add(if angle == 0. {
                    bold_shape
                } else {
                    bold_shape.with_angle(angle)
                });
            }
            let shape = egui::epaint::TextShape::new(pos, galley.clone(), color);
            painter.add(if angle == 0. {
                shape
            } else {
                shape.with_angle(angle)
            });
        }
        for ([p0, p1], width) in &self.rules {
            let a = top_left + rotate(p0.to_vec2());
            let b = top_left + rotate(p1.to_vec2());
            painter.line_segment([a, b], egui::Stroke::new(*width, color));
        }
    }
}

pub(super) fn paint_bold_copy(
    painter: &egui::Painter,
    pos: egui::Pos2,
    galley: &std::sync::Arc<egui::Galley>,
    color: Color32,
) {
    // The default figure font is static. Overprint one device pixel to the right.
    let offset = egui::vec2(1. / painter.pixels_per_point(), 0.);
    painter.galley(pos + offset, galley.clone(), color);
}

#[cfg(test)]
pub(super) fn layout_markup(
    painter: &egui::Painter,
    source: &str,
    interpreter: matrust::plot::text_markup::Interpreter,
    font: egui::FontId,
    color: Color32,
) -> MarkupLayout {
    layout_markup_styled(
        painter,
        source,
        interpreter,
        font,
        color,
        TextEmphasis::default(),
    )
}

pub(super) fn layout_markup_styled(
    painter: &egui::Painter,
    source: &str,
    interpreter: matrust::plot::text_markup::Interpreter,
    font: egui::FontId,
    color: Color32,
    emphasis: TextEmphasis,
) -> MarkupLayout {
    let nodes = matrust::plot::text_markup::parse_nodes(source, interpreter);
    layout_structured_markup_styled(painter, &nodes, &font, color, emphasis).unwrap_or_else(|| {
        MarkupLayout::from_galley(
            painter.layout_job(markup_job_from_nodes_styled(&nodes, &font, color, emphasis)),
        )
        .with_bold(emphasis.bold)
    })
}

pub(super) fn layout_structured_markup_styled(
    painter: &egui::Painter,
    nodes: &[matrust::plot::text_markup::Node],
    font: &egui::FontId,
    color: Color32,
    emphasis: TextEmphasis,
) -> Option<MarkupLayout> {
    matrust::plot::markup_layout::has_boxes(nodes)
        .then(|| layout_positioned_markup(painter, nodes, font, color, emphasis))
}

fn layout_positioned_markup(
    painter: &egui::Painter,
    nodes: &[matrust::plot::text_markup::Node],
    font: &egui::FontId,
    color: Color32,
    emphasis: TextEmphasis,
) -> MarkupLayout {
    use matrust::plot::markup_layout::{self, MeasuredRun};

    let line_height = painter.ctx().fonts_mut(|fonts| fonts.row_height(font));
    let layout = markup_layout::layout(
        nodes,
        font.size,
        line_height,
        TextAlign::Center,
        |text, size, run_bold, italic| {
            let mut format = egui::TextFormat {
                font_id: egui::FontId::new(size, font.family.clone()),
                color,
                italics: emphasis.italic || italic,
                ..Default::default()
            };
            if emphasis.bold || run_bold {
                format.coords.push("wght", 700.);
            }
            let mut job = egui::text::LayoutJob::default();
            job.append(text, 0., format);
            let glyph = painter.layout_job(job);
            MeasuredRun {
                width: glyph.size().x,
                ascent: glyph.size().y * 0.8,
                descent: glyph.size().y * 0.2,
                glyph,
            }
        },
    );
    MarkupLayout {
        size: egui::vec2(layout.size[0], layout.size[1]),
        bold: emphasis.bold,
        glyphs: layout
            .glyphs
            .into_iter()
            .map(|run| (egui::vec2(run.origin[0], run.origin[1]), run.glyph))
            .collect(),
        rules: layout
            .rules
            .into_iter()
            .map(|rule| {
                (
                    [
                        egui::pos2(rule.start[0], rule.start[1]),
                        egui::pos2(rule.end[0], rule.end[1]),
                    ],
                    rule.thickness,
                )
            })
            .collect(),
    }
}

pub(super) fn layout_subtitle(
    painter: &egui::Painter,
    subtitle: &matrust::plot::Subtitle,
    mut font: egui::FontId,
    title_color: Color32,
) -> Option<(MarkupLayout, Color32)> {
    if subtitle.string.is_empty() {
        return None;
    }
    let color = subtitle.color.map_or(title_color, rgb);
    // The desktop's 10 pt default subtitle stays at its historical 10.2 logical px,
    // while explicit FontSize uses the standard 4/3 px/pt conversion.
    font.size = subtitle.font_size.map_or(10.2, |pt| pt as f32 * (4. / 3.));
    let nodes = matrust::plot::text_markup::parse_nodes(&subtitle.string, subtitle.interpreter);
    Some((
        layout_positioned_markup(
            painter,
            &nodes,
            &font,
            color,
            TextEmphasis {
                bold: subtitle.bold,
                italic: subtitle.italic,
            },
        ),
        color,
    ))
}

const RIGHT_AXIS: f32 = 48.;
const BAR_GAP: f32 = 12.;
const BAR_WIDTH: f32 = 14.;
const BAR_LABELS: f32 = 48.;
const BAR_BOTTOM: f32 = 48.;

fn rgb(rgb: [f64; 3]) -> Color32 {
    let [r, g, b] = rgb.map(|v| (v.clamp(0., 1.) * 255.).round() as u8);
    Color32::from_rgb(r, g, b)
}
fn bytes(rgb: [u8; 3]) -> Color32 {
    Color32::from_rgb(rgb[0], rgb[1], rgb[2])
}

/// Horizontal space the right axis and an east colorbar take from the plot.
pub(super) fn right_reserve(axes: &Axes) -> f32 {
    let extras = &axes.extras;
    let mut reserve = 0.;
    if let Some(yy) = &extras.yy {
        reserve += RIGHT_AXIS + if yy.right_label.is_empty() { 0. } else { 16. };
    }
    if let Some(bar) = extras.colorbar.as_ref().filter(|b| b.visible)
        && bar.location == ColorbarLocation::EastOutside
    {
        reserve += BAR_GAP + BAR_WIDTH + BAR_LABELS + if bar.label.is_empty() { 0. } else { 16. };
    }
    reserve
}
/// Vertical space a vertical colorbar title takes above the plot.
pub(super) fn top_reserve(axes: &Axes) -> f32 {
    match axes.extras.colorbar.as_ref().filter(|b| b.visible) {
        Some(bar) if bar.location == ColorbarLocation::EastOutside && !bar.title.is_empty() => 26.,
        _ => 0.,
    }
}
/// Vertical space a south colorbar takes from the plot.
pub(super) fn bottom_reserve(axes: &Axes) -> f32 {
    match axes.extras.colorbar.as_ref().filter(|b| b.visible) {
        Some(bar) if bar.location == ColorbarLocation::SouthOutside => {
            BAR_BOTTOM + if bar.label.is_empty() { 0. } else { 16. }
        }
        _ => 0.,
    }
}

fn ticks(range: [f64; 2]) -> Vec<f64> {
    let span = range[1] - range[0];
    if !(span.is_finite() && span > 0.) {
        return range.to_vec();
    }
    let raw = span / 5.;
    let unit = 10f64.powf(raw.log10().floor());
    let step = unit
        * match raw / unit {
            r if r <= 1. => 1.,
            r if r <= 2. => 2.,
            r if r <= 5. => 5.,
            _ => 10.,
        };
    let first = (range[0] / step).ceil();
    (0..=10)
        .map(|i| (first + i as f64) * step)
        .filter(|v| *v <= range[1] + step * 1e-9)
        .collect()
}
fn label(value: f64) -> String {
    if value == 0. {
        return "0".into();
    }
    let text = format!("{:.4}", value);
    let text = text.trim_end_matches('0').trim_end_matches('.');
    text.to_owned()
}

/// Hover y value on yyaxis axes, whose right series are drawn in left-axis
/// coordinates. `y` is the left-axis value under the hovered data point; a
/// right-axis series reports its own data value instead. `name` is the
/// hovered item's label: when a left and a right series share it, the one
/// whose drawn points hold `position` decides.
pub(super) fn hover_y(
    axes: &Axes,
    points: &[Vec<PlotPoint>],
    name: &str,
    position: PlotPoint,
    y: f64,
) -> f64 {
    let Some(yy) = &axes.extras.yy else {
        return y;
    };
    let candidates: Vec<_> = axes
        .series
        .iter()
        .zip(points)
        .filter(|(s, _)| s.visible && s.label == name)
        .map(|(s, p)| (yy.is_right(&s.identity), p))
        .collect();
    let on_right = if candidates.iter().all(|(r, _)| *r) {
        !candidates.is_empty()
    } else if candidates.iter().all(|(r, _)| !*r) {
        false
    } else {
        candidates
            .iter()
            .any(|(r, p)| *r && p.iter().any(|q| q.x == position.x && q.y == position.y))
    };
    if !on_right {
        return y;
    }
    // Same mapping as the drawn points and the painted right-axis ticks.
    match axes.yy_ranges() {
        Some((left, right)) => {
            right[0] + (y - left[0]) * (right[1] - right[0]) / (left[1] - left[0])
        }
        None => y,
    }
}

fn yy_gutter_end(x: f32, labeled: bool) -> f32 {
    x + RIGHT_AXIS + if labeled { 16. } else { 0. }
}

#[allow(clippy::too_many_arguments)]
fn paint_yy_axis(
    ui: &egui::Ui,
    painter: &egui::Painter,
    yy: &YyAxis,
    left: [f64; 2],
    right: [f64; 2],
    transforms: [AxisTransform; 2],
    transform: &PlotTransform,
    frame: egui::Rect,
) -> f32 {
    let color = rgb(yy.right_color);
    let x = frame.right();
    let font = |points: f64| super::theme::figure_font(ui.ctx(), points as f32 * (4. / 3.));
    painter.line_segment(
        [egui::pos2(x, frame.top()), egui::pos2(x, frame.bottom())],
        egui::Stroke::new(1., color),
    );
    for value in ticks(right) {
        let mapped = left[0] + (value - right[0]) * (left[1] - left[0]) / (right[1] - right[0]);
        let point =
            transform.position_from_point(&PlotPoint::new(0., transforms[1].forward(mapped)));
        if !(frame.top() - 0.5..=frame.bottom() + 0.5).contains(&point.y) {
            continue;
        }
        painter.line_segment(
            [egui::pos2(x - 4., point.y), egui::pos2(x, point.y)],
            egui::Stroke::new(1., color),
        );
        painter.text(
            egui::pos2(x + 6., point.y),
            egui::Align2::LEFT_CENTER,
            label(value),
            font(9.),
            color,
        );
    }
    let next_right = x + RIGHT_AXIS;
    if !yy.right_label.is_empty() {
        let galley = painter.layout_no_wrap(yy.right_label.clone(), font(10.), color);
        let size = galley.size();
        let center = egui::pos2(next_right + 8., frame.center().y);
        let shape = egui::epaint::TextShape::new(
            center + egui::vec2(size.y / 2., -size.x / 2.),
            galley,
            color,
        )
        .with_angle(std::f32::consts::FRAC_PI_2);
        painter.add(shape);
    }
    yy_gutter_end(x, !yy.right_label.is_empty())
}

/// The yyaxis right axis line, ticks and label. Layer 'top' calls this after
/// the grid and the late tick marks, so an x grid line or an x tick on the
/// frame edge does not cover the axis.
pub(super) fn paint_right_axis(
    ui: &egui::Ui,
    axes: &Axes,
    transforms: [AxisTransform; 2],
    transform: &PlotTransform,
) {
    let Some(yy) = &axes.extras.yy else {
        return;
    };
    let Some((left, right)) = axes.yy_ranges() else {
        return;
    };
    let frame = *transform.frame();
    let painter = ui.painter().with_clip_rect(ui.clip_rect());
    let _ = paint_yy_axis(ui, &painter, yy, left, right, transforms, transform, frame);
}

pub(super) fn paint(
    ui: &egui::Ui,
    axes: &Axes,
    transforms: [AxisTransform; 2],
    transform: &PlotTransform,
    appearance: ResolvedAppearance,
    x_artwork_bottom: Option<f32>,
    defer_right_axis: bool,
) {
    let extras = &axes.extras;
    if extras.texts.is_empty()
        && extras.colorbar.is_none()
        && extras.yy.is_none()
        && extras.constant_lines.is_empty()
        && extras.rectangles.is_empty()
    {
        return;
    }
    let frame = *transform.frame();
    let painter = ui.painter().with_clip_rect(ui.clip_rect());
    let font = |points: f64| super::theme::figure_font(ui.ctx(), points as f32 * (4. / 3.));
    let text_color = bytes(appearance.text);
    let axis_color = bytes(appearance.axis);
    let mut next_right = frame.right();
    if let Some(yy) = &extras.yy
        && let Some((left, right)) = axes.yy_ranges()
    {
        if defer_right_axis {
            next_right = yy_gutter_end(frame.right(), !yy.right_label.is_empty());
        } else {
            next_right = paint_yy_axis(ui, &painter, yy, left, right, transforms, transform, frame);
        }
    }
    paint_rectangles_and_constant_lines(ui, axes, transforms, transform);
    for text in extras
        .texts
        .iter()
        .filter(|t| t.visible && !t.string.is_empty())
    {
        let point = PlotPoint::new(
            transforms[0].forward(text.position[0]),
            transforms[1].forward(text.position[1]),
        );
        if !point.x.is_finite() || !point.y.is_finite() {
            continue;
        }
        let anchor = transform.position_from_point(&point);
        if !frame.expand(400.).contains(anchor) {
            continue;
        }
        let color = text.color.map_or(text_color, rgb);
        let layout = layout_markup_styled(
            &painter,
            &text.string,
            text.interpreter,
            font(text.font_size),
            color,
            TextEmphasis {
                bold: text.bold,
                italic: text.italic,
            },
        );
        let size = layout.size;
        let fx = match text.horizontal {
            TextAlign::Left => 0.,
            TextAlign::Center => 0.5,
            TextAlign::Right => 1.,
        };
        let fy = match text.vertical {
            VerticalAlign::Top | VerticalAlign::Cap => 0.,
            VerticalAlign::Middle => 0.5,
            VerticalAlign::Bottom | VerticalAlign::Baseline => 1.,
        };
        let angle = -(text.rotation as f32).to_radians();
        let offset = egui::vec2(-fx * size.x, -fy * size.y);
        let (sin, cos) = angle.sin_cos();
        let rotated = egui::vec2(
            offset.x * cos - offset.y * sin,
            offset.x * sin + offset.y * cos,
        );
        layout.paint(&painter, anchor + rotated, color, angle);
    }
    if let Some(bar) = extras.colorbar.as_ref().filter(|b| b.visible) {
        let map = extras.effective_colormap();
        let limits = extras.color_limits();
        let rows = map.len().clamp(1, 256);
        let size = extras.colorbar_font_size();
        let fraction = |v: f64| ((v - limits[0]) / (limits[1] - limits[0])) as f32;
        match bar.location {
            ColorbarLocation::EastOutside => {
                let x = next_right + BAR_GAP;
                let rect = egui::Rect::from_min_max(
                    egui::pos2(x, frame.top()),
                    egui::pos2(x + BAR_WIDTH, frame.bottom()),
                );
                for i in 0..rows {
                    let top = rect.bottom() - rect.height() * (i + 1) as f32 / rows as f32;
                    let bottom = rect.bottom() - rect.height() * i as f32 / rows as f32;
                    painter.rect_filled(
                        egui::Rect::from_min_max(
                            egui::pos2(x, top),
                            egui::pos2(x + BAR_WIDTH, bottom + 0.5),
                        ),
                        0.,
                        rgb(map[i * map.len() / rows]),
                    );
                }
                painter.rect_stroke(
                    rect,
                    0.,
                    egui::Stroke::new(1., axis_color),
                    egui::StrokeKind::Inside,
                );
                // End labels stay inside the pane instead of under its header.
                let clip = painter.clip_rect();
                let half = size as f32 * (4. / 3.) * 0.6;
                for value in ticks(limits) {
                    let y = (rect.bottom() - rect.height() * fraction(value)).clamp(
                        clip.top() + half,
                        (clip.bottom() - half).max(clip.top() + half),
                    );
                    painter.text(
                        egui::pos2(rect.right() + 6., y),
                        egui::Align2::LEFT_CENTER,
                        label(value),
                        font(size),
                        text_color,
                    );
                }
                if !bar.label.is_empty() {
                    let galley = painter.layout_no_wrap(bar.label.clone(), font(10.), text_color);
                    let size = galley.size();
                    let center = egui::pos2(rect.right() + BAR_LABELS, rect.center().y);
                    painter.add(
                        egui::epaint::TextShape::new(
                            center + egui::vec2(size.y / 2., -size.x / 2.),
                            galley,
                            text_color,
                        )
                        .with_angle(std::f32::consts::FRAC_PI_2),
                    );
                }
                if !bar.title.is_empty() {
                    painter.text(
                        egui::pos2(rect.center().x, rect.top() - 8.),
                        egui::Align2::CENTER_BOTTOM,
                        &bar.title,
                        font(10.),
                        text_color,
                    );
                }
            }
            ColorbarLocation::SouthOutside => {
                let y = (frame.bottom() + 36.)
                    .max(x_artwork_bottom.map_or(frame.bottom(), |bottom| bottom + 4.));
                let rect = egui::Rect::from_min_max(
                    egui::pos2(frame.left(), y),
                    egui::pos2(frame.right(), y + BAR_WIDTH),
                );
                for i in 0..rows {
                    let left = rect.left() + rect.width() * i as f32 / rows as f32;
                    let right = rect.left() + rect.width() * (i + 1) as f32 / rows as f32;
                    painter.rect_filled(
                        egui::Rect::from_min_max(
                            egui::pos2(left, y),
                            egui::pos2(right + 0.5, y + BAR_WIDTH),
                        ),
                        0.,
                        rgb(map[i * map.len() / rows]),
                    );
                }
                painter.rect_stroke(
                    rect,
                    0.,
                    egui::Stroke::new(1., axis_color),
                    egui::StrokeKind::Inside,
                );
                for value in ticks(limits) {
                    let x = rect.left() + rect.width() * fraction(value);
                    painter.text(
                        egui::pos2(x, rect.bottom() + 4.),
                        egui::Align2::CENTER_TOP,
                        label(value),
                        font(size),
                        text_color,
                    );
                }
                if !bar.label.is_empty() {
                    painter.text(
                        egui::pos2(rect.center().x, rect.bottom() + 20.),
                        egui::Align2::CENTER_TOP,
                        &bar.label,
                        font(10.),
                        text_color,
                    );
                }
            }
        }
    }
}

fn rounded_rect_points(rect: egui::Rect, rx: f32, ry: f32) -> Vec<egui::Pos2> {
    if rx <= 0. && ry <= 0. {
        return vec![
            rect.left_top(),
            rect.right_top(),
            rect.right_bottom(),
            rect.left_bottom(),
        ];
    }
    let rx = rx.min(rect.width() * 0.5);
    let ry = ry.min(rect.height() * 0.5);
    let steps = 12usize;
    let corners = [
        (
            rect.right() - rx,
            rect.top() + ry,
            -std::f32::consts::FRAC_PI_2,
            0.0,
        ),
        (
            rect.right() - rx,
            rect.bottom() - ry,
            0.0,
            std::f32::consts::FRAC_PI_2,
        ),
        (
            rect.left() + rx,
            rect.bottom() - ry,
            std::f32::consts::FRAC_PI_2,
            std::f32::consts::PI,
        ),
        (
            rect.left() + rx,
            rect.top() + ry,
            std::f32::consts::PI,
            3.0 * std::f32::consts::FRAC_PI_2,
        ),
    ];
    let mut pts = Vec::with_capacity(corners.len() * (steps + 1));
    for (cx, cy, a0, a1) in corners {
        for i in 0..=steps {
            let a = a0 + (a1 - a0) * (i as f32 / steps as f32);
            pts.push(egui::pos2(cx + rx * a.cos(), cy + ry * a.sin()));
        }
    }
    pts
}

fn paint_styled_polyline(
    painter: &egui::Painter,
    mut pts: Vec<egui::Pos2>,
    stroke: egui::Stroke,
    pattern: LinePattern,
    closed: bool,
) {
    if let Some((dash, gap)) = pattern.dash_gap() {
        if closed && let Some(&first) = pts.first() {
            pts.push(first);
        }
        painter.extend(egui::Shape::dashed_line_with_offset(
            &pts, stroke, dash, gap, 0.,
        ));
    } else if closed {
        painter.add(egui::Shape::closed_line(pts, stroke));
    } else if pts.len() == 2 {
        painter.line_segment([pts[0], pts[1]], stroke);
    } else {
        painter.add(egui::Shape::line(pts, stroke));
    }
}

fn paint_rectangles_and_constant_lines(
    ui: &egui::Ui,
    axes: &Axes,
    transforms: [AxisTransform; 2],
    transform: &PlotTransform,
) {
    let extras = &axes.extras;
    if extras.rectangles.is_empty() && extras.constant_lines.is_empty() {
        return;
    }
    let frame = *transform.frame();
    let painter = ui.painter().with_clip_rect(frame);
    let font = super::theme::figure_font(ui.ctx(), 12.);
    for rect in extras.rectangles.iter().filter(|r| r.visible) {
        let p0 = transform.position_from_point(&PlotPoint::new(
            transforms[0].forward(rect.position[0]),
            transforms[1].forward(rect.position[1]),
        ));
        let p1 = transform.position_from_point(&PlotPoint::new(
            transforms[0].forward(rect.position[0] + rect.position[2]),
            transforms[1].forward(rect.position[1] + rect.position[3]),
        ));
        if !p0.x.is_finite() || !p0.y.is_finite() || !p1.x.is_finite() || !p1.y.is_finite() {
            continue;
        }
        let screen = egui::Rect::from_two_pos(p0, p1);
        let fill = rect.face_color.map_or(Color32::TRANSPARENT, rgb);
        let stroke = if rect.pattern == LinePattern::None {
            egui::Stroke::NONE
        } else {
            rect.edge_color.map_or(egui::Stroke::NONE, |c| {
                egui::Stroke::new((rect.line_width as f32 * 4. / 3.).max(0.5), rgb(c))
            })
        };
        let rx = rect.curvature[0] as f32 * 0.5 * screen.width();
        let ry = rect.curvature[1] as f32 * 0.5 * screen.height();
        let pts = rounded_rect_points(screen, rx, ry);
        if fill != Color32::TRANSPARENT {
            painter.add(egui::Shape::convex_polygon(
                pts.clone(),
                fill,
                egui::Stroke::NONE,
            ));
        }
        if stroke != egui::Stroke::NONE {
            paint_styled_polyline(&painter, pts, stroke, rect.pattern, true);
        }
    }
    for line in extras
        .constant_lines
        .iter()
        .filter(|l| l.visible && l.pattern != LinePattern::None)
    {
        let base = line
            .color
            .map_or_else(|| Color32::from_rgb(38, 38, 38), rgb);
        let a = (line.alpha.clamp(0., 1.) * 255.).round() as u8;
        let color = Color32::from_rgba_unmultiplied(base.r(), base.g(), base.b(), a);
        let stroke = egui::Stroke::new((line.line_width as f32 * 4. / 3.).max(0.5), color);
        match line.axis {
            InterceptAxis::X => {
                let p = transform
                    .position_from_point(&PlotPoint::new(transforms[0].forward(line.value), 0.));
                if !p.x.is_finite() || p.x < frame.left() - 0.5 || p.x > frame.right() + 0.5 {
                    continue;
                }
                paint_styled_polyline(
                    &painter,
                    vec![
                        egui::pos2(p.x, frame.top()),
                        egui::pos2(p.x, frame.bottom()),
                    ],
                    stroke,
                    line.pattern,
                    false,
                );
                if !line.label.is_empty() {
                    let tx = match line.label_h_align {
                        ConstantLineHAlign::Left => p.x - 4.,
                        ConstantLineHAlign::Center => p.x,
                        ConstantLineHAlign::Right => p.x + 4.,
                    };
                    let ty = match line.label_v_align {
                        ConstantLineVAlign::Top => frame.top() + 6.,
                        ConstantLineVAlign::Middle => frame.center().y,
                        ConstantLineVAlign::Bottom => frame.bottom() - 6.,
                    };
                    let align = match (line.label_h_align, line.label_v_align) {
                        (ConstantLineHAlign::Left, ConstantLineVAlign::Top) => {
                            egui::Align2::RIGHT_TOP
                        }
                        (ConstantLineHAlign::Left, ConstantLineVAlign::Middle) => {
                            egui::Align2::RIGHT_CENTER
                        }
                        (ConstantLineHAlign::Left, ConstantLineVAlign::Bottom) => {
                            egui::Align2::RIGHT_BOTTOM
                        }
                        (ConstantLineHAlign::Center, ConstantLineVAlign::Top) => {
                            egui::Align2::CENTER_TOP
                        }
                        (ConstantLineHAlign::Center, ConstantLineVAlign::Middle) => {
                            egui::Align2::CENTER_CENTER
                        }
                        (ConstantLineHAlign::Center, ConstantLineVAlign::Bottom) => {
                            egui::Align2::CENTER_BOTTOM
                        }
                        (ConstantLineHAlign::Right, ConstantLineVAlign::Top) => {
                            egui::Align2::LEFT_TOP
                        }
                        (ConstantLineHAlign::Right, ConstantLineVAlign::Middle) => {
                            egui::Align2::LEFT_CENTER
                        }
                        (ConstantLineHAlign::Right, ConstantLineVAlign::Bottom) => {
                            egui::Align2::LEFT_BOTTOM
                        }
                    };
                    painter.text(egui::pos2(tx, ty), align, &line.label, font.clone(), color);
                }
            }
            InterceptAxis::Y => {
                let p = transform
                    .position_from_point(&PlotPoint::new(0., transforms[1].forward(line.value)));
                if !p.y.is_finite() || p.y < frame.top() - 0.5 || p.y > frame.bottom() + 0.5 {
                    continue;
                }
                paint_styled_polyline(
                    &painter,
                    vec![
                        egui::pos2(frame.left(), p.y),
                        egui::pos2(frame.right(), p.y),
                    ],
                    stroke,
                    line.pattern,
                    false,
                );
                if !line.label.is_empty() {
                    let tx = match line.label_h_align {
                        ConstantLineHAlign::Left => frame.left() + 6.,
                        ConstantLineHAlign::Center => frame.center().x,
                        ConstantLineHAlign::Right => frame.right() - 6.,
                    };
                    let ty = match line.label_v_align {
                        ConstantLineVAlign::Top => p.y - 4.,
                        ConstantLineVAlign::Middle => p.y,
                        ConstantLineVAlign::Bottom => p.y + 4.,
                    };
                    let align = match (line.label_h_align, line.label_v_align) {
                        (ConstantLineHAlign::Left, ConstantLineVAlign::Top) => {
                            egui::Align2::LEFT_BOTTOM
                        }
                        (ConstantLineHAlign::Left, ConstantLineVAlign::Middle) => {
                            egui::Align2::LEFT_CENTER
                        }
                        (ConstantLineHAlign::Left, ConstantLineVAlign::Bottom) => {
                            egui::Align2::LEFT_TOP
                        }
                        (ConstantLineHAlign::Center, ConstantLineVAlign::Top) => {
                            egui::Align2::CENTER_BOTTOM
                        }
                        (ConstantLineHAlign::Center, ConstantLineVAlign::Middle) => {
                            egui::Align2::CENTER_CENTER
                        }
                        (ConstantLineHAlign::Center, ConstantLineVAlign::Bottom) => {
                            egui::Align2::CENTER_TOP
                        }
                        (ConstantLineHAlign::Right, ConstantLineVAlign::Top) => {
                            egui::Align2::RIGHT_BOTTOM
                        }
                        (ConstantLineHAlign::Right, ConstantLineVAlign::Middle) => {
                            egui::Align2::RIGHT_CENTER
                        }
                        (ConstantLineHAlign::Right, ConstantLineVAlign::Bottom) => {
                            egui::Align2::RIGHT_TOP
                        }
                    };
                    painter.text(egui::pos2(tx, ty), align, &line.label, font.clone(), color);
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::hover_y;
    use egui_plot::PlotPoint;
    use matrust::plot::AxisTransform;

    #[test]
    fn bold_subtitle_paints_more_ink() {
        use eframe::egui;
        use std::collections::HashSet;

        let ink_area = |bold| {
            let ctx = egui::Context::default();
            crate::theme::configure(&ctx);
            let subtitle = matrust::plot::Subtitle {
                string: "H".into(),
                bold,
                ..Default::default()
            };
            let mut atlas = None;
            let output = crate::headless::run(&ctx, egui::RawInput::default(), |root| {
                egui::CentralPanel::default().show(root, |ui| {
                    let painter = ui.painter();
                    let (layout, color) = super::layout_subtitle(
                        painter,
                        &subtitle,
                        crate::theme::figure_body(&ctx),
                        egui::Color32::BLACK,
                    )
                    .unwrap();
                    layout.paint(painter, egui::pos2(20., 20.), color, 0.);
                    atlas = Some(ctx.fonts(|fonts| fonts.image()));
                });
            });
            let atlas = atlas.unwrap();
            let mut ink = HashSet::new();
            for item in &output.shapes {
                let egui::Shape::Text(text) = &item.shape else {
                    continue;
                };
                if text.galley.text() != "H" {
                    continue;
                }
                // The single glyph is painted at one device pixel per point.
                let uv = text.galley.rows[0].glyphs[0].uv_rect;
                for y in uv.min[1]..uv.max[1] {
                    for x in uv.min[0]..uv.max[0] {
                        if atlas[(usize::from(x), usize::from(y))].a() > 0 {
                            ink.insert((
                                text.pos.x as i32 + i32::from(x - uv.min[0]),
                                text.pos.y as i32 + i32::from(y - uv.min[1]),
                            ));
                        }
                    }
                }
            }
            ink.len()
        };
        let normal = ink_area(false);
        let bold = ink_area(true);
        assert!(normal > 0 && bold > normal, "normal {normal}, bold {bold}");
    }

    fn display_points(axes: &matrust::plot::Axes) -> Vec<Vec<PlotPoint>> {
        axes.series
            .iter()
            .map(|s| {
                s.display_points_axes(10_000, [AxisTransform::default(); 2])
                    .into_iter()
                    .map(|[x, y]| PlotPoint::new(x, y))
                    .collect()
            })
            .collect()
    }

    #[test]
    fn hover_reports_each_yyaxis_side_in_its_own_units() {
        let mut engine = matrust::Engine::default();
        engine
            .execute(
                "yyaxis left; plot([1 2 3], [0 5 10]); ylim([-5 15]); \
                 yyaxis right; plot([1 2 3], [1020 1000 1010]); ylim([990 1030]);",
            )
            .unwrap();
        let source = engine.figure.as_ref().unwrap().axes.clone();
        let display = source.yy_display().unwrap();
        let points = display_points(&display);
        let [left, right] = [&display.series[0], &display.series[1]];
        // Right series drawn at left-axis y 10, 0, 5; left series at 0, 5, 10.
        let drawn: Vec<f64> = points[1].iter().map(|p| p.y).collect();
        assert_eq!(drawn, [10., 0., 5.]);
        // Both the display copy (detached/prepared views) and the source
        // axes (docked view, mapped points) report the right-axis value.
        for axes in [&display, &source] {
            let at = points[1][0];
            let value = hover_y(axes, &points, &right.label, at, at.y);
            assert!((value - 1020.).abs() < 1e-9, "{value}");
            // A left series keeps its own value.
            let at = points[0][2];
            assert_eq!(hover_y(axes, &points, &left.label, at, at.y), 10.);
        }
        // Same label on both sides: the side whose points hold the hover decides.
        let mut shared = display.clone();
        shared.series[1].label = shared.series[0].label.clone();
        let label = shared.series[0].label.clone();
        let value = hover_y(&shared, &points, &label, points[1][2], points[1][2].y);
        assert!((value - 1010.).abs() < 1e-9, "{value}");
        let at = points[0][1];
        assert_eq!(hover_y(&shared, &points, &label, at, at.y), 5.);
        // Without a right axis the left value passes through.
        let plain = matrust::plot::Axes::default();
        assert_eq!(hover_y(&plain, &[], "data1", at, 7.), 7.);
    }

    #[test]
    fn latex_frac_and_sqrt_layout_stacks_numerator_over_denominator_with_bar_and_overbar() {
        let ctx = eframe::egui::Context::default();
        crate::theme::configure(&ctx);
        let mut texts = Vec::new();
        let mut rules = Vec::new();
        let _ = crate::headless::run(&ctx, eframe::egui::RawInput::default(), |root| {
            eframe::egui::CentralPanel::default().show(root, |ui| {
                let layout = super::layout_markup(
                    ui.painter(),
                    r"$\frac{1}{2}+\sqrt{x}$",
                    matrust::plot::text_markup::Interpreter::Latex,
                    crate::theme::figure_body(&ctx),
                    eframe::egui::Color32::BLACK,
                );
                for (offset, galley) in &layout.glyphs {
                    texts.push((galley.text().to_owned(), *offset, galley.size()));
                }
                rules = layout.rules.clone();
            });
        });
        let find = |needle: &str| {
            texts
                .iter()
                .find(|(t, _, _)| t == needle)
                .unwrap_or_else(|| panic!("missing glyph '{needle}' in {texts:?}"))
        };
        assert!(
            !texts
                .iter()
                .any(|(t, _, _)| t.contains('/') || t.contains("√("))
        );
        let (_, num_off, num_size) = find("1");
        let (_, den_off, _) = find("2");
        let (_, rad_off, _) = find("√");
        let (_, body_off, body_size) = find("x");
        assert_eq!(rules.len(), 2, "expected fraction bar and radical overbar");
        let ([frac_a, frac_b], _) = rules[0];
        let ([over_a, over_b], _) = rules[1];
        assert!(
            num_off.y + num_size.y <= frac_a.y + 0.5,
            "numerator {num_off:?} should sit above fraction bar {frac_a:?}"
        );
        assert!(
            den_off.y >= frac_a.y - 0.5,
            "denominator {den_off:?} should sit below fraction bar {frac_a:?}"
        );
        assert_eq!(frac_a.y, frac_b.y);
        assert!(frac_b.x > frac_a.x);
        assert!(rad_off.x < body_off.x);
        assert!(
            over_a.y < body_off.y,
            "radical overbar {over_a:?} should sit above radicand {body_off:?}"
        );
        assert_eq!(over_a.y, over_b.y);
        assert!(over_b.x >= body_off.x + body_size.x - 1.0);
    }

    #[test]
    fn xline_dashed_and_rectangle_dotted_paint_svg_dash_lengths() {
        let mut engine = matrust::Engine::default();
        engine
            .execute(
                "plot(1:4); xline(2,'--r'); rectangle('Position',[1 1 2 1],'LineStyle',':','EdgeColor','b');",
            )
            .unwrap();
        let figure = engine.figure.unwrap();
        let (axes, points) = crate::graphics_frames::display_points(&figure);
        let ctx = eframe::egui::Context::default();
        crate::theme::configure(&ctx);
        let mut output = None;
        for pass in 0..2 {
            let out = crate::headless::run(
                &ctx,
                eframe::egui::RawInput {
                    screen_rect: Some(eframe::egui::Rect::from_min_size(
                        eframe::egui::Pos2::ZERO,
                        eframe::egui::vec2(640., 420.),
                    )),
                    ..Default::default()
                },
                |root| {
                    eframe::egui::CentralPanel::default().show(root, |ui| {
                        let _ = crate::figure_view::show_interactive(
                            ui,
                            &figure,
                            &points,
                            axes,
                            pass == 0,
                            None,
                        );
                    });
                },
            );
            output = Some(out);
        }
        let mut red_dashes = Vec::new();
        let mut blue_dots = Vec::new();
        fn collect(
            shape: &eframe::egui::Shape,
            red_dashes: &mut Vec<[eframe::egui::Pos2; 2]>,
            blue_dots: &mut Vec<[eframe::egui::Pos2; 2]>,
        ) {
            match shape {
                eframe::egui::Shape::LineSegment { points, stroke } => {
                    if stroke.color.r() > 0 && stroke.color.g() == 0 && stroke.color.b() == 0 {
                        red_dashes.push(*points);
                    } else if stroke.color.b() > 0 && stroke.color.r() == 0 && stroke.color.g() == 0
                    {
                        blue_dots.push(*points);
                    }
                }
                eframe::egui::Shape::Vec(children) => {
                    for child in children {
                        collect(child, red_dashes, blue_dots);
                    }
                }
                _ => {}
            }
        }
        for clipped in &output.unwrap().shapes {
            collect(&clipped.shape, &mut red_dashes, &mut blue_dots);
        }
        assert!(red_dashes.len() > 4, "expected dashed xline segments");
        for pair in red_dashes.windows(2).take(4) {
            let dash_len = pair[0][0].distance(pair[0][1]);
            let gap_len = pair[0][1].distance(pair[1][0]);
            assert!((dash_len - 8.).abs() < 0.05, "xline dash {dash_len}");
            assert!((gap_len - 5.).abs() < 0.05, "xline gap {gap_len}");
        }
        assert!(blue_dots.len() > 8, "expected dotted rectangle segments");
        for pair in blue_dots.windows(2).take(4) {
            let dot_len = pair[0][0].distance(pair[0][1]);
            let gap_len = pair[0][1].distance(pair[1][0]);
            assert!((dot_len - 2.).abs() < 0.05, "rect dot {dot_len}");
            assert!((gap_len - 4.).abs() < 0.05, "rect gap {gap_len}");
        }
    }
}
