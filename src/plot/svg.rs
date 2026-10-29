use super::*;
use std::fmt::Write;
#[path = "svg_buffer.rs"]
mod buffer;
#[path = "svg_clip.rs"]
mod clipping;
#[path = "svg_legend.rs"]
mod legend;
use buffer::SvgBuffer;
use std::sync::atomic::AtomicBool;
#[cfg(feature = "plot-export")]
use std::sync::atomic::Ordering;
#[derive(Clone, Copy)]
struct Surface {
    opaque: bool,
    color: [u8; 3],
    appearance: ResolvedAppearance,
}
impl Surface {
    fn fill(self) -> std::borrow::Cow<'static, str> {
        if self.color == [255; 3] {
            std::borrow::Cow::Borrowed("white")
        } else {
            std::borrow::Cow::Owned(format!(
                "#{:02x}{:02x}{:02x}",
                self.color[0], self.color[1], self.color[2]
            ))
        }
    }
}
impl Axes {
    pub(super) fn export_legend_geometry(
        &self,
        width: f64,
        height: f64,
        position: Option<[f64; 4]>,
        layout_canvas: Option<LegendRect>,
    ) -> Result<LegendGeometry> {
        let initial = LegendRect::new(position.unwrap_or([70., 60., width - 40., height - 70.]));
        let canvas = layout_canvas.unwrap_or_else(|| LegendRect::new([0., 0., width, height]));
        if !self.legend_location.is_outside() && self.placement.is_none() {
            if self.legend_location == LegendLocation::Best
                && (!initial.valid() || initial.width() < 16. || initial.height() < 16.)
            {
                return Err(
                    "Best legend placement needs an axes box of at least 16 by 16 export units"
                        .into(),
                );
            }
            return Ok(LegendGeometry {
                data: initial,
                legend: None,
                outer: canvas,
            });
        }
        let (outer, inner, fixed_inner) = self.placement.map_or((canvas, initial, false), |p| {
            let fixed = p.imported_solved || p.constraint == PositionConstraint::InnerPosition;
            (
                if fixed {
                    canvas
                } else {
                    LegendRect::from_normalized(p.requested_outer, canvas)
                },
                LegendRect::from_normalized(p.requested_inner, canvas),
                fixed,
            )
        });
        let extent = if self.legend_location.is_outside() {
            let available = if matches!(self.legend_location.outside_side(), Some(0 | 2)) {
                [
                    (outer.width() * 0.35).min(240.),
                    (outer.height() - 24.).max(1.),
                ]
            } else {
                [
                    (outer.width() - 24.).max(1.),
                    (outer.height() * 0.35).min(180.),
                ]
            };
            legend::extent(self, available)
        } else {
            None
        };
        legend_geometry(
            outer,
            inner,
            [70., 60., 40., 70.],
            extent,
            self.legend_location,
            fixed_inner,
        )
    }
    pub fn svg(&self) -> String {
        self.svg_sized(
            960.,
            560.,
            "axes",
            Surface {
                opaque: true,
                color: [255; 3],
                appearance: ResolvedAppearance::default(),
            },
            None,
        )
    }
    fn svg_sized(
        &self,
        width: f64,
        height: f64,
        clip_id: &str,
        surface: Surface,
        cancel: Option<&AtomicBool>,
    ) -> String {
        self.svg_positioned([width, height], None, None, clip_id, surface, cancel)
    }
    fn svg_positioned(
        &self,
        size: [f64; 2],
        position: Option<[f64; 4]>,
        layout_canvas: Option<LegendRect>,
        clip_id: &str,
        surface: Surface,
        cancel: Option<&AtomicBool>,
    ) -> String {
        let [width, height] = size;
        let appearance = surface.appearance;
        let colors = self.resolve_colors(appearance);
        let intent = self.colors.unwrap_or_default();
        let text_color = color_hex(appearance.text);
        let grid_x = color_hex(colors.grid_x);
        let grid_y = color_hex(colors.grid_y);
        let grid_opacity = if intent.grid.is_some() || !appearance.legacy {
            format!(" stroke-opacity=\"{}\"", colors.grid_alpha)
        } else {
            String::new()
        };
        // Preserve the exact legacy SVG spelling when no override is present.
        let fill = |present: bool, rgb| {
            if present {
                format!(" fill=\"{}\"", color_hex(rgb))
            } else {
                String::new()
            }
        };
        let title_fill = fill(intent.title.is_some(), colors.title);
        let xlabel_fill = fill(intent.xlabel.is_some() || intent.x.is_some(), colors.xlabel);
        let ylabel_fill = fill(intent.ylabel.is_some() || intent.y.is_some(), colors.ylabel);
        let x_fill = fill(intent.x.is_some(), colors.x);
        let y_fill = fill(intent.y.is_some(), colors.y);
        let axis_color = if appearance.axis == [119; 3] {
            String::from("#777")
        } else {
            color_hex(appearance.axis)
        };
        if self.baselines.len() > MAX_BASELINES {
            return String::new();
        }
        let geometry = match self.export_legend_geometry(width, height, position, layout_canvas) {
            Ok(g) => g,
            Err(_) => return String::new(),
        };
        let data = geometry.data;
        let mut placed_legend = geometry.legend;
        let [left, top, right, bottom] = data.values();
        let relocated =
            position.is_some() || self.legend_location.is_outside() || self.placement.is_some();
        let title_y = if relocated { (top - 18.).max(18.) } else { 32. };
        let xlabel_y = if relocated {
            (bottom + 44.).min(height - 4.)
        } else {
            height - 14.
        };
        let ylabel_x = if relocated {
            (left - 52.).max(12.)
        } else {
            18.
        };
        let label_x = if relocated {
            (left + right) / 2.
        } else {
            width / 2.
        };
        let label_y = if relocated {
            (top + bottom) / 2.
        } else {
            height / 2.
        };
        let mut background = if position.is_some() || !surface.opaque {
            String::new()
        } else {
            format!(
                "<rect width=\"{width}\" height=\"{height}\" fill=\"{}\"/>",
                surface.fill()
            )
        };
        if surface.opaque && surface.color != colors.background {
            let _ = write!(
                background,
                "<rect x=\"{left}\" y=\"{top}\" width=\"{}\" height=\"{}\" fill=\"{}\"/>",
                right - left,
                bottom - top,
                if colors.background == [255; 3] {
                    String::from("white")
                } else {
                    color_hex(colors.background)
                }
            );
        }
        let (x, y) = self.bounds();
        let axes = self.axis_transforms();
        let tx = x.map(|v| axes[0].forward(v));
        let ty = y.map(|v| axes[1].forward(v));
        let px = |v| left + (right - left) * scale(axes[0].forward(v), tx[0], tx[1]);
        let py = |v| bottom - (bottom - top) * scale(axes[1].forward(v), ty[0], ty[1]);
        let mut svg = SvgBuffer::new(
            format!(
                "<svg xmlns=\"http://www.w3.org/2000/svg\" width=\"{width}\" height=\"{height}\" viewBox=\"0 0 {width} {height}\">{background}<defs><clipPath id=\"{clip_id}\"><rect x=\"{left}\" y=\"{top}\" width=\"{}\" height=\"{}\"/></clipPath></defs><g font-family=\"Arial\" font-size=\"16\" fill=\"{text_color}\">",
                right - left,
                bottom - top
            ),
            cancel,
        );
        let _ = write!(
            svg,
            "<text x=\"{left}\" y=\"{title_y}\" font-size=\"20\"{title_fill}>{}</text><text x=\"{}\" y=\"{}\" text-anchor=\"middle\"{xlabel_fill}>{}</text><text transform=\"translate({ylabel_x},{}) rotate(-90)\" text-anchor=\"middle\"{ylabel_fill}>{}</text>",
            escape(&self.title),
            label_x,
            xlabel_y,
            escape(&self.xlabel),
            label_y,
            escape(&self.ylabel)
        );
        for (range, horizontal, axis) in [(x, true, axes[0]), (y, false, axes[1])] {
            let (ticks, step) = if axis.scale == AxisScale::Log {
                (
                    axis.ticks(range.map(|v| axis.forward(v)))
                        .into_iter()
                        .map(|v| axis.inverse(v))
                        .collect(),
                    0.,
                )
            } else {
                ticks(range)
            };
            for value in ticks {
                let p = if horizontal { px(value) } else { py(value) };
                let label = if axis.scale == AxisScale::Log {
                    axis.label(axis.forward(value))
                } else {
                    tick_text(value, step)
                };
                if horizontal {
                    if self.xgrid {
                        let _ = write!(
                            svg,
                            "<path d=\"M{p} {top}V{bottom}\" stroke=\"{grid_x}\"{grid_opacity}/>"
                        );
                    }
                    let _ = write!(
                        svg,
                        "<text x=\"{p}\" y=\"{}\" text-anchor=\"middle\" font-size=\"12\"{x_fill}>{label}</text>",
                        bottom + 24.
                    );
                } else {
                    if self.ygrid {
                        let _ = write!(
                            svg,
                            "<path d=\"M{left} {p}H{right}\" stroke=\"{grid_y}\"{grid_opacity}/>"
                        );
                    }
                    let _ = write!(
                        svg,
                        "<text x=\"{}\" y=\"{p}\" text-anchor=\"end\" font-size=\"12\"{y_fill}>{label}</text>",
                        left - 10.
                    );
                }
            }
        }
        if intent.x.is_some() || intent.y.is_some() {
            let x_color = color_hex(colors.x);
            let y_color = color_hex(colors.y);
            let _ = write!(
                svg,
                "</g><path d=\"M{left} {top}H{right}M{left} {bottom}H{right}\" fill=\"none\" stroke=\"{x_color}\"/><path d=\"M{left} {top}V{bottom}M{right} {top}V{bottom}\" fill=\"none\" stroke=\"{y_color}\"/><g clip-path=\"url(#{clip_id})\">"
            );
        } else {
            let _ = write!(
                svg,
                "</g><rect x=\"{left}\" y=\"{top}\" width=\"{}\" height=\"{}\" fill=\"none\" stroke=\"{axis_color}\"/><g clip-path=\"url(#{clip_id})\">",
                right - left,
                bottom - top
            );
        }
        for (series_index, s) in self.series.iter().enumerate().filter(|(_, s)| s.visible) {
            // Resolve bounded membership once per series, never once per sample.
            let series_base = if matches!(s.kind, PlotKind::Bar | PlotKind::Stem) {
                match self.render_baseline(series_index, axes[1]) {
                    Ok(v) => v,
                    Err(_) => return String::new(),
                }
            } else {
                s.stem_baseline(axes[1])
            };
            let color = color_hex(appearance.series_color(s));
            let width = s.stroke_width(2.);
            if matches!(s.kind, PlotKind::Line | PlotKind::Stairs | PlotKind::Stem)
                && s.pattern != LinePattern::None
            {
                let dash = match s.pattern {
                    LinePattern::Solid | LinePattern::None => "",
                    LinePattern::Dashed => "8 5",
                    LinePattern::Dotted => "2 4",
                    LinePattern::DashDot => "8 4 2 4",
                };
                let checkpoint = svg.checkpoint();
                let mut hazardous = false;
                let _ = write!(
                    svg,
                    "<path fill=\"none\" stroke=\"{color}\" stroke-width=\"{width}\" stroke-dasharray=\"{dash}\" d=\""
                );
                let mut first = true;
                let mut previous_y = 0.;
                for (point_index, [a, b]) in s.points.iter().enumerate() {
                    if svg.stopped(point_index) {
                        return String::new();
                    }
                    if !axes[0].accepts(a) || !axes[1].accepts(b) {
                        first = true;
                        continue;
                    }
                    let point = [px(a), py(b)];
                    if clipping::hazardous(point) {
                        hazardous = true;
                        break;
                    }
                    if s.kind == PlotKind::Stairs && !first {
                        let _ = write!(svg, "L{:.2},{:.2} ", point[0], previous_y);
                    }
                    if s.kind == PlotKind::Stem {
                        let baseline = series_base;
                        let baseline_y = py(baseline);
                        if clipping::hazardous([point[0], baseline_y]) {
                            hazardous = true;
                            break;
                        }
                        let _ = write!(
                            svg,
                            "M{:.2},{:.2} L{:.2},{:.2} ",
                            point[0], baseline_y, point[0], point[1]
                        );
                        continue;
                    }
                    let _ = write!(
                        svg,
                        "{}{:.2},{:.2} ",
                        if first { "M" } else { "L" },
                        point[0],
                        point[1]
                    );
                    first = false;
                    previous_y = point[1];
                }
                if hazardous {
                    svg.rewind(checkpoint);
                    let baseline = series_base;
                    clipping::draw(
                        &mut svg,
                        s,
                        &color,
                        dash,
                        [left, top, right, bottom],
                        py(baseline),
                        |a, b| axes[0].accepts(a) && axes[1].accepts(b),
                        |a, b| [px(a), py(b)],
                    );
                } else {
                    let _ = svg.write_str("\"/>");
                }
            }
            if s.kind == PlotKind::Scatter || s.markers {
                for (point_index, [a, b]) in s.points.iter().enumerate() {
                    if svg.stopped(point_index) {
                        return String::new();
                    }
                    if axes[0].accepts(a) && axes[1].accepts(b) {
                        let point = [px(a), py(b)];
                        if !clipping::hazardous(point)
                            || clipping::marker_visible(s, point, [left, top, right, bottom])
                        {
                            draw_marker(&mut svg, s, point[0], point[1], &color, false, appearance);
                        }
                    }
                }
            }
            if s.kind == PlotKind::Patch {
                let Some(patch) = s.patch else {
                    return String::new();
                };
                let width = patch.vertices_per_face();
                if !matches!(width, 3 | 4) {
                    return String::new();
                }
                let bounds = [tx[0], tx[1], ty[0], ty[1]];
                for face_index in 0..s.points.len() / width {
                    let Some(face) = s.points.face(face_index, width) else {
                        continue;
                    };
                    let face = face.as_slice();
                    if svg.stopped(0) {
                        return String::new();
                    }
                    let mut transformed = [[0.; 2]; 4];
                    for (p, q) in transformed.iter_mut().zip(face) {
                        *p = [axes[0].forward(q[0]), axes[1].forward(q[1])];
                    }
                    let polygon = clip_patch_face(&transformed[..width], bounds);
                    if polygon.points().len() >= 3 {
                        let _ = write!(svg, "<polygon fill=\"{color}\" stroke=\"none\" points=\"");
                        for &p in polygon.points() {
                            let [x, y] = patch_screen_point(p, bounds, [left, top, right, bottom]);
                            let _ = write!(svg, "{x:.2},{y:.2} ");
                        }
                        svg.push_str("\"/>");
                    }
                    face_outline(
                        &mut svg,
                        s,
                        &transformed[..width],
                        bounds,
                        [left, top, right, bottom],
                        [true; 4],
                    );
                }
            }
            if s.kind == PlotKind::Bar {
                for (point_index, [a, b]) in s.points.iter().enumerate() {
                    if svg.stopped(point_index) {
                        return String::new();
                    }
                    if axes[0].accepts(a) && axes[1].accepts(b) {
                        let left = a + s.bar_offset - s.bar_width / 2.;
                        let right = a + s.bar_offset + s.bar_width / 2.;
                        let l = px(if axes[0].accepts(left) { left } else { x[0] });
                        let r = px(if axes[0].accepts(right) { right } else { x[1] });
                        let baseline = series_base;
                        let top = py(b).min(py(baseline));
                        let height = (py(b) - py(baseline)).abs();
                        let _ = write!(
                            svg,
                            "<rect x=\"{l:.2}\" y=\"{top:.2}\" width=\"{:.2}\" height=\"{height:.2}\" fill=\"{color}\"/>",
                            (r - l).max(0.)
                        );
                        if s.face_edge_color.is_some() && s.pattern != LinePattern::None {
                            let face = [
                                [
                                    axes[0].forward(if axes[0].accepts(left) {
                                        left
                                    } else {
                                        x[0]
                                    }),
                                    axes[1].forward(baseline),
                                ],
                                [
                                    axes[0].forward(if axes[0].accepts(right) {
                                        right
                                    } else {
                                        x[1]
                                    }),
                                    axes[1].forward(baseline),
                                ],
                                [
                                    axes[0].forward(if axes[0].accepts(right) {
                                        right
                                    } else {
                                        x[1]
                                    }),
                                    axes[1].forward(b),
                                ],
                                [
                                    axes[0].forward(if axes[0].accepts(left) {
                                        left
                                    } else {
                                        x[0]
                                    }),
                                    axes[1].forward(b),
                                ],
                            ];
                            face_outline(
                                &mut svg,
                                s,
                                &face,
                                [tx[0], tx[1], ty[0], ty[1]],
                                [px(x[0]), py(y[1]), px(x[1]), py(y[0])],
                                [true, axes[0].accepts(right), true, axes[0].accepts(left)],
                            );
                        }
                    }
                }
            }
        }
        let mut baselines = Vec::new();
        for s in self.series.iter().filter(|s| {
            s.visible && s.kind == PlotKind::Stem && self.baseline_for_series(&s.identity).is_none()
        }) {
            for (point_index, p) in s.points.iter().enumerate() {
                if svg.stopped(point_index) {
                    return String::new();
                }
                if axes[0].accepts(p[0]) && axes[1].accepts(p[1]) {
                    let baseline = s.stem_baseline(axes[1]);
                    if !baselines.contains(&baseline) {
                        baselines.push(baseline);
                    }
                    break;
                }
            }
        }
        let legacy_style = super::BaselineStyle::legacy_stem();
        let legacy_color = color_hex(legacy_style.display_rgb());
        let legacy_width = legacy_style.display_width();
        for baseline in baselines {
            let baseline_y = py(baseline);
            if !clipping::hazardous([left, baseline_y]) {
                let _ = write!(
                    svg,
                    "<path d=\"M{left} {:.2}H{right}\" fill=\"none\" stroke=\"{legacy_color}\" stroke-width=\"{legacy_width}\"/>",
                    baseline_y
                );
            }
        }
        // New Bar groups have one independent implicit stroke, with metadata-only
        // identity deduplication. Historical decoded groups have the flag off.
        for index in self.implicit_bar_baselines() {
            if svg.stopped(0) {
                return String::new();
            }
            let Ok(value) = self.render_baseline(index, axes[1]) else {
                return String::new();
            };
            let y = py(value);
            if !clipping::hazardous([left, y]) {
                let _ = write!(
                    svg,
                    "<path data-matrust-implicit-bar-baseline=\"true\" d=\"M{left} {y:.2}H{right}\" fill=\"none\" stroke=\"{legacy_color}\" stroke-width=\"{legacy_width}\"/>"
                );
            }
        }
        // Explicit records are independent of series visibility and legend membership.
        // Distinct records remain distinct even when their coordinates are equal.
        for baseline in &self.baselines {
            if svg.stopped(0) {
                return String::new();
            }
            let style = baseline.style;
            if !style.visible || style.pattern == LinePattern::None {
                continue;
            }
            let value = match baseline.value.resolve(axes[1]) {
                Ok(value) => value,
                Err(_) => return String::new(),
            };
            let y = py(value);
            if clipping::hazardous([left, y]) {
                continue;
            }
            let dash = match style.pattern {
                LinePattern::Solid | LinePattern::None => "",
                LinePattern::Dashed => "8 5",
                LinePattern::Dotted => "2 4",
                LinePattern::DashDot => "8 4 2 4",
            };
            // SVG percentages retain normalized RGB precision; no u8 model quantization.
            let rgb = style.color.map(|v| v * 100.);
            let width = style.width_points * 4. / 3.;
            let _ = write!(
                svg,
                "<path data-matrust-baseline=\"true\" d=\"M{left} {y:.2}H{right}\" fill=\"none\" stroke=\"rgb({}%,{}%,{}%)\" stroke-width=\"{width}\" stroke-dasharray=\"{dash}\"/>",
                rgb[0], rgb[1], rgb[2]
            );
        }
        svg.push_str("</g>");
        let location = if self.legend_location == LegendLocation::Best && self.legend {
            let available = [
                data.width() - 10_f64.min(data.width() * 0.1),
                data.height() - 2. * 10_f64.min(data.height() * 0.1),
            ];
            if let Some(extent) = legend::extent(self, available) {
                match best_legend_location(
                    self,
                    LegendBestView {
                        bounds: [tx[0], tx[1], ty[0], ty[1]],
                        transforms: axes,
                        frame: data,
                        extent,
                    },
                    cancel,
                ) {
                    Ok(result) => {
                        let available = data.inset([4.; 4]);
                        let extent = [
                            extent[0].min(available.width()),
                            extent[1].min(available.height()),
                        ];
                        placed_legend =
                            legend_layout::anchored(available, extent, result.location).ok();
                        result.location
                    }
                    Err(_) => return String::new(),
                }
            } else {
                LegendLocation::Northeast
            }
        } else {
            self.legend_location.fixed_anchor()
        };
        legend::draw(
            &mut svg,
            self,
            [left, top, right, bottom],
            clip_id,
            placed_legend,
            location,
            appearance,
        );
        svg.push_str("</svg>");
        svg.finish()
    }
}
fn scale(x: f64, min: f64, max: f64) -> f64 {
    let span = max - min;
    let delta = x - min;
    if span.is_finite() && span > 0. && delta.is_finite() {
        (delta / span).clamp(-1e6, 1e6)
    } else {
        ((x / 2. - min / 2.) / (max / 2. - min / 2.)).clamp(-1e6, 1e6)
    }
}
fn ticks(range: [f64; 2]) -> (Vec<f64>, f64) {
    let span = range[1] - range[0];
    let target = span / 5.;
    let unit = 10f64.powf(target.log10().floor());
    let ratio = target / unit;
    let step = unit
        * if ratio <= 1. {
            1.
        } else if ratio <= 2. {
            2.
        } else if ratio <= 5. {
            5.
        } else {
            10.
        };
    if !step.is_finite() || step <= 0. {
        return (
            (0..=5)
                .map(|i| {
                    let t = i as f64 / 5.;
                    range[0] * (1. - t) + range[1] * t
                })
                .collect(),
            target,
        );
    }
    let first = (range[0] / step).ceil() * step;
    let values: Vec<_> = (0..=10)
        .map(|i| first + step * i as f64)
        .filter(|v| v.is_finite() && *v >= range[0] && *v <= range[1])
        .collect();
    if values.is_empty() {
        (range.to_vec(), step)
    } else {
        (values, step)
    }
}
fn tick_text(value: f64, step: f64) -> String {
    if value == 0. {
        "0".into()
    } else if value.abs() >= 1e5 || value.abs() < 1e-3 {
        format!("{value:.2e}")
    } else {
        let decimals = (-step.abs().log10().floor()).clamp(0., 8.) as usize;
        format!("{value:.decimals$}")
    }
}
fn color_hex(c: [u8; 3]) -> String {
    format!("#{:02x}{:02x}{:02x}", c[0], c[1], c[2])
}
fn escape(s: &str) -> String {
    s.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
}

impl Figure {
    pub fn svg(&self) -> String {
        self.svg_with_theme(BaseTheme::Light)
    }
    /// Resolve automatic appearance against an explicit host, without changing the figure.
    pub fn svg_with_theme(&self, host: BaseTheme) -> String {
        self.svg_target_background(true, None, None, host)
    }
    #[cfg(all(test, feature = "plot-export"))]
    pub(crate) fn svg_without_background(&self, cancel: &AtomicBool) -> crate::Result<String> {
        self.svg_without_background_in_theme(BaseTheme::Light, cancel)
    }
    #[cfg(feature = "plot-export")]
    pub(crate) fn svg_without_background_in_theme(
        &self,
        host: BaseTheme,
        cancel: &AtomicBool,
    ) -> crate::Result<String> {
        let svg = self.svg_target_background(false, Some(cancel), None, host);
        if cancel.load(Ordering::Relaxed) {
            return Err("Cancelled".into());
        }
        if svg.is_empty() {
            return Err("SVG generation exceeded its32MiB allocation budget".into());
        }
        Ok(svg)
    }
    #[cfg(feature = "plot-export")]
    pub(crate) fn svg_opaque_with_cancel_in_theme(
        &self,
        host: BaseTheme,
        cancel: &AtomicBool,
    ) -> crate::Result<String> {
        let svg = self.svg_target_background(true, Some(cancel), None, host);
        crate::dsp::check_cancel(cancel)?;
        if svg.is_empty() {
            return Err("SVG generation exceeded its 32 MiB allocation budget".into());
        }
        Ok(svg)
    }
    #[cfg(all(test, feature = "plot-export"))]
    pub(crate) fn svg_axes_without_background(
        &self,
        position: super::Position,
        cancel: &AtomicBool,
    ) -> crate::Result<String> {
        self.svg_axes_without_background_in_theme(position, BaseTheme::Light, cancel)
    }
    #[cfg(feature = "plot-export")]
    pub(crate) fn svg_axes_without_background_in_theme(
        &self,
        position: super::Position,
        host: BaseTheme,
        cancel: &AtomicBool,
    ) -> crate::Result<String> {
        crate::dsp::check_cancel(cancel)?;
        if !self.all_axes().any(|(p, _)| p == position) {
            return Err("exportgraphics target axes does not exist".into());
        }
        let svg = self.svg_target_background(false, Some(cancel), Some(position), host);
        crate::dsp::check_cancel(cancel)?;
        if svg.is_empty() {
            return Err("SVG generation exceeded its32MiB allocation budget".into());
        }
        Ok(svg)
    }
    fn svg_target_background(
        &self,
        opaque: bool,
        cancel: Option<&AtomicBool>,
        target: Option<super::Position>,
        host: BaseTheme,
    ) -> String {
        let appearance = self.resolve_appearance(host);
        let surface = Surface {
            opaque,
            color: appearance.canvas,
            appearance,
        };
        if textbox::validate_textboxes(&self.textboxes).is_err() {
            return String::new();
        }
        if self.layout.is_none() {
            let mut inner = self.axes.svg_sized(960., 560., "axes", surface, cancel);
            if target.is_some() || self.textboxes.is_empty() || inner.is_empty() {
                return inner;
            }
            let Some(end) = inner.strip_suffix("</svg>").map(str::len) else {
                return String::new();
            };
            inner.truncate(end);
            let mut result = SvgBuffer::new(inner, cancel);
            draw_textboxes(&mut result, self);
            result.push_str("</svg>");
            return result.finish();
        }
        let mut panels: Vec<_> = self.all_axes().collect();
        panels.sort_by_key(|(p, _)| *p);
        let mut result = SvgBuffer::new(
            String::from(
                "<svg xmlns=\"http://www.w3.org/2000/svg\" width=\"960\" height=\"560\" viewBox=\"0 0 960 560\">",
            ),
            cancel,
        );
        if opaque {
            let _ = write!(
                result,
                "<rect width=\"960\" height=\"560\" fill=\"{}\"/>",
                surface.fill()
            );
        }
        let title = &self.layout.as_ref().unwrap().title;
        let header = if title.is_empty() { 0. } else { 36. };
        let canvas_height = 560. - header;
        // Size ordinary panels in figure units so a 2-by-2 grid retains the
        // same text size as a single axes. Only dense grids need a larger
        // internal viewport to leave bounded room for ticks and outside keys.
        // Custom positions already use figure coordinates and do not constrain
        // the grid's text scale.
        let scale =
            panels
                .iter()
                .filter(|(p, _)| p.custom.is_none())
                .fold(1_f64, |scale, (p, _)| {
                    let [l, t, r, b] = p.rectangle();
                    scale
                        .min(960. * (r - l) / 320.)
                        .min(canvas_height * (b - t) / 220.)
                });
        if target.is_none() && !title.is_empty() {
            let title_color = color_hex(appearance.shared_title);
            let _ = write!(
                result,
                "<text x=\"480\" y=\"25\" text-anchor=\"middle\" font-family=\"sans-serif\" font-size=\"20\" fill=\"{title_color}\">{}</text>",
                escape(title)
            );
        }
        for (i, (p, a)) in panels.into_iter().enumerate() {
            if target.is_some_and(|selected| p != selected) {
                continue;
            }
            let [left, top, right, bottom] = p.rectangle();
            if p.custom.is_some() {
                let inner = a.svg_positioned(
                    [960., 560.],
                    Some([
                        960. * left,
                        header + canvas_height * top,
                        960. * right,
                        header + canvas_height * bottom,
                    ]),
                    Some(LegendRect::new([0., header, 960., 560.])),
                    &format!("axes{i}"),
                    surface,
                    cancel,
                );
                if inner.is_empty() {
                    return String::new();
                }
                result.push_str(&inner);
                if result.failed() {
                    return String::new();
                }
                continue;
            }
            let width = 960. * (right - left) / scale;
            let height = canvas_height * (bottom - top) / scale;
            let inner = a.svg_sized(width, height, &format!("axes{i}"), surface, cancel);
            if inner.is_empty() {
                return String::new();
            }
            let Some(inner) = buffer::replace_first(
                inner,
                &format!("width=\"{width}\" height=\"{height}\""),
                &format!(
                    "x=\"{}\" y=\"{}\" width=\"{}\" height=\"{}\"",
                    960. * left,
                    header + canvas_height * top,
                    960. * (right - left),
                    canvas_height * (bottom - top)
                ),
                cancel,
            ) else {
                return String::new();
            };
            result.push_str(&inner);
            if result.failed() {
                return String::new();
            }
        }
        if target.is_none() {
            draw_textboxes(&mut result, self);
        }
        result.push_str("</svg>");
        result.finish()
    }
}

fn draw_marker(
    svg: &mut impl std::fmt::Write,
    series: &super::Series,
    x: f64,
    y: f64,
    color: &str,
    legend: bool,
    appearance: ResolvedAppearance,
) {
    use super::MarkerShape::*;
    let shape = series.effective_marker();
    let radius = if legend {
        if series.has_marker_style() {
            series.marker_radius().min(8.)
        } else {
            3.
        }
    } else {
        series.marker_radius()
    };
    let width = if legend {
        series.stroke_width(1.).min(3.)
    } else {
        series.stroke_width(1.)
    };
    let background = (legend && appearance.legend_background != [255; 3])
        .then(|| color_hex(appearance.legend_background));
    let fill = if series.marker_is_filled() {
        color
    } else if legend {
        background.as_deref().unwrap_or("white")
    } else {
        "none"
    };
    if matches!(shape, Circle | Point) {
        let _ = write!(
            svg,
            "<circle cx=\"{x:.2}\" cy=\"{y:.2}\" r=\"{radius}\" fill=\"{fill}\" stroke=\"{color}\" stroke-width=\"{width}\"/>"
        );
        return;
    }
    let radius = f64::from(radius);
    let q = std::f64::consts::FRAC_1_SQRT_2;
    let h = 0.8660254037844386;
    let (points, count) = match shape {
        Diamond => ([[0., 1.], [-1., 0.], [0., -1.], [1., 0.]], 4),
        Square => ([[-q, q], [-q, -q], [q, -q], [q, q]], 4),
        Up => ([[0., -1.], [h, 0.5], [-h, 0.5], [0., 0.]], 3),
        Down => ([[0., 1.], [-h, -0.5], [h, -0.5], [0., 0.]], 3),
        Left => ([[-1., 0.], [0.5, -h], [0.5, h], [0., 0.]], 3),
        Right => ([[1., 0.], [-0.5, h], [-0.5, -h], [0., 0.]], 3),
        _ => ([[0.; 2]; 4], 0),
    };
    if count > 0 {
        let _ = write!(
            svg,
            "<path fill=\"{fill}\" stroke=\"{color}\" stroke-width=\"{width}\" d=\""
        );
        for (i, p) in points[..count].iter().enumerate() {
            let _ = write!(
                svg,
                "{}{:.2},{:.2} ",
                if i == 0 { "M" } else { "L" },
                x + p[0] * radius,
                y + p[1] * radius
            );
        }
        let _ = svg.write_str("Z\"/>");
    } else {
        let segments = match shape {
            Plus => [[[-1., 0.], [1., 0.]], [[0., -1.], [0., 1.]], [[0.; 2]; 2]],
            Cross => [[[-q, -q], [q, q]], [[q, -q], [-q, q]], [[0.; 2]; 2]],
            _ => [
                [[0., -1.], [0., 1.]],
                [[-h, 0.5], [h, -0.5]],
                [[-h, -0.5], [h, 0.5]],
            ],
        };
        let _ = write!(
            svg,
            "<path fill=\"none\" stroke=\"{color}\" stroke-width=\"{width}\" d=\""
        );
        for [a, b] in &segments[..if shape == Asterisk { 3 } else { 2 }] {
            let _ = write!(
                svg,
                "M{:.2},{:.2} L{:.2},{:.2} ",
                x + a[0] * radius,
                y + a[1] * radius,
                x + b[0] * radius,
                y + b[1] * radius
            );
        }
        let _ = svg.write_str("\"/>");
    }
}

fn draw_textboxes(svg: &mut SvgBuffer<'_>, figure: &Figure) {
    for (index, textbox) in figure.textboxes.iter().enumerate() {
        if svg.stopped(0) {
            return;
        }
        if !textbox.visible || textbox.position[2] == 0. || textbox.position[3] == 0. {
            continue;
        }
        let [left, top, right, bottom] = textbox.rectangle([0., 0., 960., 560.]);
        let font = textbox.font_size * (4. / 3.);
        let (x, anchor) = match textbox.horizontal {
            TextAlign::Left => (left, "start"),
            TextAlign::Center => ((left + right) * 0.5, "middle"),
            TextAlign::Right => (right, "end"),
        };
        let _ = write!(
            svg,
            "<defs><clipPath id=\"textbox{index}\"><rect x=\"{left}\" y=\"{top}\" width=\"{}\" height=\"{}\"/></clipPath></defs><g clip-path=\"url(#textbox{index})\" font-family=\"sans-serif\" font-size=\"{font}\" font-weight=\"{}\" fill=\"{}\">",
            right - left,
            bottom - top,
            if textbox.bold { "bold" } else { "normal" },
            color_hex(textbox.color)
        );
        for (line, text) in textbox.display_lines(right - left).into_iter().enumerate() {
            let y = top + font * (1. + line as f64 * 1.2);
            if y - font > bottom {
                break;
            }
            let _ = write!(
                svg,
                "<text x=\"{x}\" y=\"{y}\" text-anchor=\"{anchor}\" xml:space=\"preserve\">{}</text>",
                escape(&text)
            );
        }
        svg.push_str("</g>");
    }
}

#[cfg(all(test, feature = "plot-export"))]
mod figure_color_tests {
    use super::*;
    #[test]
    fn transparent_export_does_not_emit_the_opaque_figure_or_axes_fill() {
        let mut figure = Figure::default();
        figure.set_canvas_color([18, 52, 86]);
        figure
            .add(vec![Series::new(vec![[1., 2.], [2., 3.]], PlotKind::Line)])
            .unwrap();
        let transparent = figure
            .svg_without_background(&AtomicBool::new(false))
            .unwrap();
        let parsed = resvg::usvg::roxmltree::Document::parse(&transparent).unwrap();
        assert!(
            !parsed.descendants().any(|n| n.has_tag_name("rect")
                && matches!(n.attribute("fill"), Some("white" | "#123456")))
        );
        assert!(figure.svg().contains("fill=\"#123456\""));
    }
}

#[cfg(all(test, feature = "plot-export"))]
mod selected_export_tests {
    use super::*;

    fn panel<'a>(svg: &'a str, clip: &str) -> &'a str {
        let clip_offset = svg.find(&format!("<clipPath id=\"{clip}\">")).unwrap();
        let begin = svg[..clip_offset].rfind("<svg ").unwrap();
        let end = clip_offset + svg[clip_offset..].find("</svg>").unwrap() + "</svg>".len();
        &svg[begin..end]
    }

    #[test]
    fn selected_tiled_subtree_preserves_exact_source_geometry_data_and_style() {
        let mut engine = crate::Engine::default();
        engine.execute("tiledlayout(1,2);nexttile;plot([1 2],[4 8],'Color',[1 0 0],'LineWidth',2);title('LEFT');nexttile;h=semilogy([1 2],[-2 -4]);title('RIGHT');set(gca,'XLim',[0 3],'YLim',[-10 -1],'XGrid','off','YGrid','on');legend(h,{'RIGHT KEY'},'Interpreter','none');sgtitle('SHARED ONLY');").unwrap();
        let figure = engine.figure.as_ref().unwrap();
        let cancel = AtomicBool::new(false);
        let whole = figure.svg_without_background(&cancel).unwrap();
        let mut axes: Vec<_> = figure.all_axes().collect();
        axes.sort_by_key(|(position, _)| *position);
        assert_eq!(axes.len(), 2);
        for (index, (position, _)) in axes.into_iter().enumerate() {
            let selected = figure
                .svg_axes_without_background(position, &cancel)
                .unwrap();
            let clip = format!("axes{index}");
            assert_eq!(panel(&selected, &clip), panel(&whole, &clip));
            assert_eq!(selected.matches("<svg ").count(), 2);
            assert!(!selected.contains("SHARED ONLY"));
            assert!(!selected.contains(&format!("<clipPath id=\"axes{}\">", 1 - index)));
        }
    }
}

// Stroke source edges separately from the clipped fill: viewport cuts are not outlines.
fn face_outline(
    svg: &mut SvgBuffer<'_>,
    series: &Series,
    face: &[[f64; 2]],
    bounds: [f64; 4],
    frame: [f64; 4],
    edges: [bool; 4],
) {
    let Some(rgb) = series.face_edge_color else {
        return;
    };
    if series.pattern == LinePattern::None
        || face.len() < 3
        || face.iter().flatten().any(|v| !v.is_finite())
    {
        return;
    }
    let color = color_hex(rgb);
    let dash = match series.pattern {
        LinePattern::Dashed => "8 5",
        LinePattern::Dotted => "2 4",
        LinePattern::DashDot => "8 4 2 4",
        _ => "",
    };
    let _ = write!(
        svg,
        "<path fill=\"none\" stroke=\"{color}\" stroke-width=\"{}\" stroke-dasharray=\"{dash}\" d=\"",
        series.face_edge_width()
    );
    let mut last = None;
    let mut first = None;
    let mut continuous = true;
    for i in 0..face.len() {
        if !edges[i] {
            last = None;
            continuous = false;
            continue;
        }
        if let Some([a, b]) = clip_face_edge(face[i], face[(i + 1) % face.len()], bounds) {
            let a = patch_screen_point(a, bounds, frame);
            let b = patch_screen_point(b, bounds, frame);
            if last != Some(a) {
                let _ = write!(svg, "M{:.2},{:.2} ", a[0], a[1]);
                if first.is_some() {
                    continuous = false;
                }
            }
            first.get_or_insert(a);
            let _ = write!(svg, "L{:.2},{:.2} ", b[0], b[1]);
            last = Some(b);
        } else {
            last = None;
            continuous = false;
        }
    }
    if continuous && last.is_some() && last == first {
        svg.push_str("Z");
    }
    svg.push_str("\"/>");
}
