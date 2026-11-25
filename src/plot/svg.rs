use super::*;
use std::fmt::Write;
#[path = "svg_buffer.rs"]
mod buffer;
#[path = "svg_clip.rs"]
mod clipping;
#[path = "svg_extras.rs"]
mod extras;
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
        self.export_legend_geometry_with_margins(width, height, position, layout_canvas, None)
    }
    pub(super) fn export_legend_geometry_with_margins(
        &self,
        width: f64,
        height: f64,
        position: Option<[f64; 4]>,
        layout_canvas: Option<LegendRect>,
        tile_margins: Option<[f64; 4]>,
    ) -> Result<LegendGeometry> {
        let headings = SvgHeadings::new(
            &self.title,
            self.extras.label_interpreters[0],
            &self.extras.subtitle,
            self.extras.title_style,
        );
        self.export_legend_geometry_for_headings(
            width,
            height,
            position,
            layout_canvas,
            tile_margins,
            headings.extra_height(),
        )
    }
    fn extra_axis_label_margins(&self) -> (f64, f64) {
        if self.extras.axis_off {
            return (0., 0.);
        }
        let label_margin = |text: &str, style: super::LabelStyle| {
            if text.is_empty() {
                return 0.;
            }
            let size = style.export_font_size_pixels(16., EXPORT_PX_PER_POINT);
            let glyph = (f64::from(size) - 16.).max(0.);
            let line_step = (f64::from(size) * 1.2).max(20.);
            let extra_lines = text.matches('\n').count().min(20) as f64;
            glyph + (line_step - 20.).max(0.) * extra_lines
        };
        (
            label_margin(&self.ylabel, self.extras.label_styles[1]),
            label_margin(&self.xlabel, self.extras.label_styles[0]),
        )
    }
    fn view3d_label_margins(&self, tile_margins: Option<[f64; 4]>) -> Option<[f64; 4]> {
        if self.extras.axis_off {
            return tile_margins;
        }
        let default_px = self.extras.font_size.unwrap_or(10.) * 1.1 * 4. / 3.;
        let extra_for = |label: &str, style: super::LabelStyle| {
            if label.is_empty() {
                return 0.;
            }
            let Some(pt) = style.font_size else {
                return 0.;
            };
            (pt * f64::from(EXPORT_PX_PER_POINT) - default_px).max(0.)
        };
        let extra_left = extra_for(&self.zlabel, self.extras.label_styles[2]);
        let extra_bottom = extra_for(&self.xlabel, self.extras.label_styles[0])
            .max(extra_for(&self.ylabel, self.extras.label_styles[1]));
        if extra_left == 0. && extra_bottom == 0. {
            return tile_margins;
        }
        let [ml, mt, mr, mb] = tile_margins.unwrap_or([70., 60., 40., 70.]);
        Some([ml + extra_left, mt, mr, mb + extra_bottom])
    }
    fn export_legend_geometry_for_headings(
        &self,
        width: f64,
        height: f64,
        position: Option<[f64; 4]>,
        layout_canvas: Option<LegendRect>,
        tile_margins: Option<[f64; 4]>,
        extra_title: f64,
    ) -> Result<LegendGeometry> {
        let (extra_left, extra_bottom) = self.extra_axis_label_margins();
        let default_margins = match tile_margins {
            Some([ml, mt, mr, mb]) => [ml + extra_left, mt + extra_title, mr, mb + extra_bottom],
            None => [70. + extra_left, 60. + extra_title, 40., 70. + extra_bottom],
        };
        let initial = LegendRect::new(
            position.unwrap_or([
                default_margins[0],
                default_margins[1]
                    .min((height - default_margins[3] - 16.).max(default_margins[1] - extra_title)),
                (width - default_margins[2]).max(default_margins[0] + 1.),
                (height - default_margins[3]).max(default_margins[1] + 1.),
            ]),
        );
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
            default_margins,
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
        self.svg_positioned([width, height], None, None, None, clip_id, surface, cancel)
    }
    #[allow(clippy::too_many_arguments)]
    fn svg_positioned(
        &self,
        size: [f64; 2],
        position: Option<[f64; 4]>,
        layout_canvas: Option<LegendRect>,
        tile_margins: Option<[f64; 4]>,
        clip_id: &str,
        surface: Surface,
        cancel: Option<&AtomicBool>,
    ) -> String {
        if let Some(display) = self.yy_display() {
            return display.svg_positioned(
                size,
                position,
                layout_canvas,
                tile_margins,
                clip_id,
                surface,
                cancel,
            );
        }
        if let std::borrow::Cow::Owned(display) =
            super::view3d::display_view_in(self, surface.appearance)
        {
            let tile_margins = self.view3d_label_margins(tile_margins);
            return display.svg_positioned(
                size,
                position,
                layout_canvas,
                tile_margins,
                clip_id,
                surface,
                cancel,
            );
        }
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
        // Minor grid: dotted, thinner, MATLAB's MinorGridAlpha where the major
        // grid is translucent; the opaque legacy grey otherwise.
        let minor_style = format!(
            " stroke-width=\"0.5\" stroke-dasharray=\"1 2\"{}",
            if intent.grid.is_some() || !appearance.legacy {
                format!(" stroke-opacity=\"{}\"", super::MINOR_GRID_ALPHA)
            } else {
                String::new()
            }
        );
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
        let headings = SvgHeadings::new(
            &self.title,
            self.extras.label_interpreters[0],
            &self.extras.subtitle,
            self.extras.title_style,
        );
        let geometry = match self.export_legend_geometry_for_headings(
            width,
            height,
            position,
            layout_canvas,
            tile_margins,
            headings.extra_height(),
        ) {
            Ok(g) => g,
            Err(_) => return String::new(),
        };
        let data = geometry.data;
        let mut placed_legend = geometry.legend;
        let [left, top, right, bottom] = data.values();
        // Colorbar and right y-axis space comes out of the data rectangle.
        let [reserve_right, reserve_bottom] = extras::reserve(self);
        let right = (right - reserve_right).max(left + 1.);
        let bottom = (bottom - reserve_bottom).max(top + 1.);
        let [left, top, right, bottom] = if self.extras.square_plot_box {
            let side = (right - left).min(bottom - top);
            let x = left + (right - left - side) / 2.;
            let y = top + (bottom - top - side) / 2.;
            [x, y, x + side, y + side]
        } else {
            [left, top, right, bottom]
        };
        let relocated = position.is_some()
            || self.legend_location.is_outside()
            || self.placement.is_some()
            || self.extras.square_plot_box;
        let extra_title_glyph = if self.title.is_empty() {
            0.
        } else {
            (f64::from(headings.title_size) - 20.).max(0.)
        };
        let title_y = if relocated {
            (top - 18. - headings.baseline_offset(&self.title)).max(18. + extra_title_glyph)
        } else {
            32. + extra_title_glyph
        };
        let xlabel_style = self.extras.label_styles[0];
        let xlabel_size = xlabel_style.export_font_size_pixels(16., EXPORT_PX_PER_POINT);
        let extra_xlabel_glyph = if self.extras.axis_off || self.xlabel.is_empty() {
            0.
        } else {
            (f64::from(xlabel_size) - 16.).max(0.)
        };
        let xlabel_step = (f64::from(xlabel_size) * 1.2).max(20.);
        let xlabel_extra = self.xlabel.matches('\n').count().min(20) as f64;
        let xlabel_descender = extra_xlabel_glyph * 0.2;
        let xlabel_y = if relocated {
            (bottom + 44. + extra_xlabel_glyph)
                .min(height - 14. - xlabel_descender - xlabel_step * xlabel_extra)
                .max(18. + extra_xlabel_glyph)
        } else {
            (height - 14. - xlabel_descender - xlabel_step * xlabel_extra)
                .max(18. + extra_xlabel_glyph)
        };
        let ylabel_style = self.extras.label_styles[1];
        let ylabel_size = ylabel_style.export_font_size_pixels(16., EXPORT_PX_PER_POINT);
        let extra_ylabel_glyph = if self.extras.axis_off || self.ylabel.is_empty() {
            0.
        } else {
            (f64::from(ylabel_size) - 16.).max(0.)
        };
        let ylabel_step = (f64::from(ylabel_size) * 1.2).max(20.);
        let ylabel_extra = self.ylabel.matches('\n').count().min(20) as f64;
        let ylabel_x = if relocated {
            (left - 52. - ylabel_step * ylabel_extra).max(12. + extra_ylabel_glyph)
        } else {
            18. + extra_ylabel_glyph
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
        if surface.opaque && surface.color != colors.background && !self.extras.axis_off {
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
        let (x, y) = super::image::equal_aspect(self, x, y, [right - left, bottom - top]);
        let tx = x.map(|v| axes[0].forward(v));
        let ty = y.map(|v| axes[1].forward(v));
        let flip = self.extras.y_reversed;
        let px = |v| left + (right - left) * scale(axes[0].forward(v), tx[0], tx[1]);
        let py = |v| {
            let t = scale(axes[1].forward(v), ty[0], ty[1]);
            if flip {
                top + (bottom - top) * t
            } else {
                bottom - (bottom - top) * t
            }
        };
        let axis_off = self.extras.axis_off;
        let mut svg = SvgBuffer::new(
            format!(
                "<svg xmlns=\"http://www.w3.org/2000/svg\" width=\"{width}\" height=\"{height}\" viewBox=\"0 0 {width} {height}\">{background}<defs><clipPath id=\"{clip_id}\"><rect x=\"{left}\" y=\"{top}\" width=\"{}\" height=\"{}\"/></clipPath></defs><g font-family=\"Arial\" font-size=\"16\" fill=\"{text_color}\">",
                right - left,
                bottom - top
            ),
            cancel,
        );
        let title_color = if intent.title.is_some() {
            color_hex(colors.title)
        } else {
            text_color.clone()
        };
        let plain_title = !self.title.contains('\n')
            && matches!(&headings.title, SvgMarkup::Inline { nodes, bold, italic } if inline_markup(nodes, left, *bold, *italic) == escape(&self.title));
        let title_center = (left + right) / 2.;
        let title_x = if plain_title { left } else { title_center };
        let title_anchor = if plain_title {
            ""
        } else {
            r#" text-anchor="middle""#
        };
        let title_size = headings.title_size;
        let title_attrs = style_attrs(self.extras.title_style);
        headings.title.write(
            &mut svg,
            format_args!(r#" x="{title_x}" y="{title_y}"{title_anchor}"#),
            [title_x, title_y - f64::from(title_size) * 0.8],
            TextAlign::Center,
            format_args!(r#" font-size="{title_size}"{title_attrs}{title_fill}"#),
            &title_color,
        );
        let subtitle_top = if self.title.is_empty() {
            title_y - 16.
        } else {
            title_y - f64::from(title_size) * 0.8
        };
        headings.write_subtitle(
            &mut svg,
            title_center,
            subtitle_top,
            &self.extras.subtitle,
            &title_color,
        );

        let xlabel_size_attr = xlabel_style
            .font_size
            .map(|_| format!(r#" font-size="{xlabel_size}""#))
            .unwrap_or_default();
        let xlabel_attrs = style_attrs(xlabel_style);
        let xlabel = SvgMarkup::new_with_style(
            if axis_off { "" } else { &self.xlabel },
            self.extras.label_interpreters[1],
            xlabel_size,
            TextAlign::Center,
            xlabel_style.bold.unwrap_or(false),
            xlabel_style.italic.unwrap_or(false),
        );
        let xlabel_color = color_hex(colors.xlabel);
        xlabel.write(
            &mut svg,
            format_args!(r#" x="{label_x}" y="{xlabel_y}" text-anchor="middle""#),
            [
                label_x,
                xlabel_y + 4. - xlabel.height(f64::from(xlabel_size)),
            ],
            TextAlign::Center,
            format_args!("{xlabel_size_attr}{xlabel_attrs}{xlabel_fill}"),
            &xlabel_color,
        );
        let ylabel_size_attr = ylabel_style
            .font_size
            .map(|_| format!(r#" font-size="{ylabel_size}""#))
            .unwrap_or_default();
        let ylabel_attrs = style_attrs(ylabel_style);
        let ylabel = SvgMarkup::new_with_style(
            if axis_off { "" } else { &self.ylabel },
            self.extras.label_interpreters[2],
            ylabel_size,
            TextAlign::Center,
            ylabel_style.bold.unwrap_or(false),
            ylabel_style.italic.unwrap_or(false),
        );
        let ylabel_color = color_hex(colors.ylabel);
        let ylabel_x = ylabel.rotated_x(ylabel_x);
        ylabel.write(
            &mut svg,
            format_args!(""),
            [0., -ylabel.height(f64::from(ylabel_size)) / 2.],
            TextAlign::Center,
            format_args!(r#" transform="translate({ylabel_x},{label_y}) rotate(-90)" text-anchor="middle"{ylabel_size_attr}{ylabel_attrs}{ylabel_fill}"#),
            &ylabel_color,
        );
        let x_rule_color = color_hex(colors.x);
        let y_rule_color = color_hex(colors.y);
        let tick_axes = if self.ticks_hidden || self.frame_hidden || axis_off {
            0
        } else {
            2
        };
        let tick_dir = self.extras.tick_dir.unwrap_or_default();
        let tick_len_px =
            (right - left).max(bottom - top) * self.extras.tick_length.map_or(0.01, |l| l[0]);
        let has_tick_style = self.extras.tick_dir.is_some()
            || self.extras.tick_length.is_some()
            || self.extras.line_width.is_some();
        let mut tick_paths = [String::new(), String::new()];
        // Layer 'top': grid lines and tick marks are emitted after the axes
        // children instead of under them (bounded by MAX_TICKS per axis).
        let layer_top = self.extras.layer.is_top();
        let mut top_grid = String::new();
        for (axis_idx, (range, horizontal, axis)) in [(x, true, axes[0]), (y, false, axes[1])]
            .into_iter()
            .enumerate()
            .take(tick_axes)
        {
            let ruler = &self.extras.rulers[axis_idx];
            let leaves = self.extras.leaf_ticks.as_ref().filter(|_| horizontal);
            let (ticks, step) = if let Some(manual) = &ruler.ticks {
                let step = if manual.len() >= 2 {
                    super::manual_tick_step(manual)
                } else {
                    1.
                };
                (manual.clone(), step)
            } else if let Some(leaves) = leaves {
                ((1..=leaves.len()).map(|i| i as f64).collect(), 1.)
            } else if !ruler.kind.is_numeric() {
                self.effective_ticks(axis_idx)
            } else if axis.scale == AxisScale::Log {
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
            if self.minor_grid[usize::from(!horizontal)] && leaves.is_none() {
                let minor: Vec<f64> = if axis.scale == AxisScale::Log {
                    super::minor_ticks(axis, range.map(|v| axis.forward(v)), 0.)
                        .into_iter()
                        .map(|v| axis.inverse(v))
                        .collect()
                } else {
                    super::minor_ticks(axis, range, step)
                };
                for value in minor {
                    let line = if horizontal {
                        let p = px(value);
                        format!(
                            "<path d=\"M{p} {top}V{bottom}\" stroke=\"{grid_x}\"{minor_style}/>"
                        )
                    } else {
                        let p = py(value);
                        format!(
                            "<path d=\"M{left} {p}H{right}\" stroke=\"{grid_y}\"{minor_style}/>"
                        )
                    };
                    if layer_top {
                        top_grid.push_str(&line);
                    } else {
                        svg.push_str(&line);
                    }
                }
            }
            let draw_ticks =
                (has_tick_style || ruler.ticks.is_some()) && tick_dir != super::TickDir::None;
            let span = (range[1] - range[0]).abs();
            for (idx, value) in ticks.into_iter().enumerate() {
                if ruler.ticks.is_some()
                    && (value < range[0] - span * 1e-9 || value > range[1] + span * 1e-9)
                {
                    continue;
                }
                let p = if horizontal { px(value) } else { py(value) };
                let label = if let Some(manual_labels) = &ruler.tick_labels {
                    manual_labels.get(idx).cloned().unwrap_or_default()
                } else if let Some(leaves) = leaves {
                    if value >= 1. && value.fract() == 0. {
                        leaves.get(value as usize - 1).cloned().unwrap_or_default()
                    } else {
                        String::new()
                    }
                } else if ruler.ticks.is_some()
                    || ruler.exponent != 0
                    || ruler.tick_label_format.is_some()
                    || !ruler.kind.is_numeric()
                {
                    self.format_tick_label(axis_idx, value, step, axis)
                } else if axis.scale == AxisScale::Log {
                    axis.label(axis.forward(value))
                } else {
                    tick_text(value, step)
                };
                let rot = if ruler.tick_label_rotation != 0. {
                    let (rx, ry) = if horizontal {
                        (p, bottom + 24.)
                    } else {
                        (left - 10., p + 4.)
                    };
                    format!(
                        " transform=\"rotate({} {rx} {ry})\"",
                        -ruler.tick_label_rotation
                    )
                } else {
                    String::new()
                };
                let (label_x, label_y, align) = if horizontal {
                    (p, bottom + 24., TextAlign::Center)
                } else {
                    (left - 10., p + 4., TextAlign::Right)
                };
                let label = SvgMarkup::new(&label, self.extras.tick_label_interpreter, 12., align);
                if horizontal {
                    if self.xgrid {
                        let _ = write!(
                            if layer_top {
                                &mut top_grid as &mut dyn Write
                            } else {
                                &mut svg
                            },
                            "<path d=\"M{p} {top}V{bottom}\" stroke=\"{grid_x}\"{grid_opacity}/>"
                        );
                    }
                    if draw_ticks {
                        match tick_dir {
                            super::TickDir::In => {
                                let _ = write!(tick_paths[0], "M{p} {bottom}v-{}", tick_len_px);
                            }
                            super::TickDir::Out => {
                                let _ = write!(tick_paths[0], "M{p} {bottom}v{}", tick_len_px);
                            }
                            super::TickDir::Both => {
                                let _ = write!(
                                    tick_paths[0],
                                    "M{p} {}v{}",
                                    bottom - tick_len_px,
                                    2. * tick_len_px
                                );
                            }
                            super::TickDir::None => {}
                        }
                    }
                    label.write(
                        &mut svg,
                        format_args!(r#" x="{p}" y="{label_y}" text-anchor="middle""#),
                        [p, label_y - 9.6],
                        align,
                        format_args!(r#" font-size="12"{x_fill}{rot}"#),
                        &x_rule_color,
                    );
                } else {
                    if self.ygrid {
                        let _ = write!(
                            if layer_top {
                                &mut top_grid as &mut dyn Write
                            } else {
                                &mut svg
                            },
                            "<path d=\"M{left} {p}H{right}\" stroke=\"{grid_y}\"{grid_opacity}/>"
                        );
                    }
                    if draw_ticks {
                        match tick_dir {
                            super::TickDir::In => {
                                let _ = write!(tick_paths[1], "M{left} {p}h{}", tick_len_px);
                            }
                            super::TickDir::Out => {
                                let _ = write!(tick_paths[1], "M{left} {p}h-{}", tick_len_px);
                            }
                            super::TickDir::Both => {
                                let _ = write!(
                                    tick_paths[1],
                                    "M{} {p}h{}",
                                    left - tick_len_px,
                                    2. * tick_len_px
                                );
                            }
                            super::TickDir::None => {}
                        }
                    }
                    label.write(
                        &mut svg,
                        format_args!(r#" x="{label_x}" y="{label_y}" text-anchor="end""#),
                        [label_x, label_y - 9.6],
                        align,
                        format_args!(r#" font-size="12"{y_fill}{rot}"#),
                        &y_rule_color,
                    );
                }
            }
            if horizontal
                && ruler.tick_label_format.is_none()
                && ruler.tick_labels.is_none()
                && matches!(ruler.kind, super::RulerKind::Datetime { .. })
                && let Some(date_label) = super::time_axes::secondary_date_label(range)
            {
                let y = bottom + 38.;
                SvgMarkup::new(
                    &date_label,
                    self.extras.tick_label_interpreter,
                    12.,
                    TextAlign::Left,
                )
                .write(
                    &mut svg,
                    format_args!(r#" x="{left}" y="{y}" text-anchor="start""#),
                    [left, y - 9.6],
                    TextAlign::Left,
                    format_args!(r#" font-size="12"{x_fill}"#),
                    &x_rule_color,
                );
            }
            if ruler.exponent != 0 {
                if horizontal {
                    let _ = write!(
                        svg,
                        "<text x=\"{right}\" y=\"{}\" text-anchor=\"end\" font-size=\"12\"{x_fill}>×10^{}</text>",
                        bottom + 38.,
                        ruler.exponent
                    );
                } else {
                    let _ = write!(
                        svg,
                        "<text x=\"{left}\" y=\"{}\" text-anchor=\"start\" font-size=\"12\"{y_fill}>×10^{}</text>",
                        (top - 8.).max(12.),
                        ruler.exponent
                    );
                }
            }
        }
        let axis_sw = self
            .extras
            .line_width
            .map(|w| format!(" stroke-width=\"{w}\""))
            .unwrap_or_default();
        // Layer 'top' moves only the tick marks; the axis lines stay put.
        let mut top_ticks = String::new();
        let tick_paths = if layer_top {
            let [x_ticks, y_ticks] = tick_paths;
            let tick_el = |path: &str, color: &str| {
                if path.is_empty() {
                    String::new()
                } else {
                    format!("<path d=\"{path}\" fill=\"none\" stroke=\"{color}\"{axis_sw}/>")
                }
            };
            if intent.x.is_some() || intent.y.is_some() {
                top_ticks = tick_el(&x_ticks, &color_hex(colors.x))
                    + &tick_el(&y_ticks, &color_hex(colors.y));
            } else {
                top_ticks = tick_el(&format!("{x_ticks}{y_ticks}"), &axis_color);
            }
            [String::new(), String::new()]
        } else {
            tick_paths
        };
        if self.frame_hidden || axis_off {
            let _ = write!(svg, "</g><g clip-path=\"url(#{clip_id})\">");
        } else if intent.x.is_some() || intent.y.is_some() {
            let x_color = color_hex(colors.x);
            let y_color = color_hex(colors.y);
            // Box off keeps only the bottom x and left y axis lines.
            let (x_top, y_right) = if self.box_outline {
                (
                    format!("M{left} {top}H{right}"),
                    format!("M{right} {top}V{bottom}"),
                )
            } else {
                (String::new(), String::new())
            };
            let _ = write!(
                svg,
                "</g><path d=\"{x_top}M{left} {bottom}H{right}{}\" fill=\"none\" stroke=\"{x_color}\"{axis_sw}/><path d=\"M{left} {top}V{bottom}{y_right}{}\" fill=\"none\" stroke=\"{y_color}\"{axis_sw}/><g clip-path=\"url(#{clip_id})\">",
                tick_paths[0], tick_paths[1]
            );
        } else if !self.box_outline {
            let _ = write!(
                svg,
                "</g><path d=\"M{left} {top}V{bottom}H{right}{}{}\" fill=\"none\" stroke=\"{axis_color}\"{axis_sw}/><g clip-path=\"url(#{clip_id})\">",
                tick_paths[0], tick_paths[1]
            );
        } else {
            let tick_all = format!("{}{}", tick_paths[0], tick_paths[1]);
            let tick_el = if tick_all.is_empty() {
                String::new()
            } else {
                format!("<path d=\"{tick_all}\" fill=\"none\" stroke=\"{axis_color}\"{axis_sw}/>")
            };
            let _ = write!(
                svg,
                "</g><rect x=\"{left}\" y=\"{top}\" width=\"{}\" height=\"{}\" fill=\"none\" stroke=\"{axis_color}\"{axis_sw}/>{tick_el}<g clip-path=\"url(#{clip_id})\">",
                right - left,
                bottom - top
            );
        }
        super::image::draw_svg(&mut svg, self, &px, &py);
        let mut volumes = self.volume_paint_order().into_iter().peekable();
        let mut draw_volumes = |svg: &mut SvgBuffer<'_>, slot| -> crate::Result<()> {
            while let Some((_, i)) = volumes.next_if(|(after, _)| *after == slot) {
                if svg.failed() {
                    return Err("Cancelled".into());
                }
                super::volume::draw_svg(svg, self, &self.extras.volumes[i], &px, &py, cancel)?;
            }
            Ok(())
        };
        if draw_volumes(&mut svg, 0).is_err() {
            return String::new();
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
                let scatter_style = s.kind == PlotKind::Scatter && s.has_scatter_style();
                let map = scatter_style.then(|| self.extras.effective_colormap());
                let clim = self.extras.color_limits();
                let axes_bg = self.resolve_colors(appearance).background;
                for (point_index, [a, b]) in s.points.iter().enumerate() {
                    if svg.stopped(point_index) {
                        return String::new();
                    }
                    if axes[0].accepts(a) && axes[1].accepts(b) {
                        let point = [px(a), py(b)];
                        if !clipping::hazardous(point)
                            || clipping::marker_visible(s, point, [left, top, right, bottom])
                        {
                            draw_marker_styled(
                                &mut svg,
                                s,
                                point[0],
                                point[1],
                                &color,
                                false,
                                appearance,
                                map.as_deref().map(|m| PointStyle {
                                    index: point_index,
                                    map: m.as_slice(),
                                    clim,
                                    axes_bg,
                                }),
                            );
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
                let flat_color = (s.face_color_mode == super::FaceColorMode::Flat)
                    .then_some(s.patch_cdata)
                    .flatten()
                    .map(|c| {
                        color_hex(super::colormap::scaled_rgb(
                            &self.extras.effective_colormap(),
                            self.extras.color_limits(),
                            c,
                        ))
                    });
                let base_color = flat_color.as_deref().unwrap_or(&color);
                let bounds = [tx[0], tx[1], ty[0], ty[1]];
                // FaceColor 'interp': per-vertex CData, banded by colormap row.
                let interp = (s.face_color_mode == super::FaceColorMode::Flat && width == 3)
                    .then_some(s.patch_vertex_cdata.as_ref())
                    .flatten()
                    .map(|v| {
                        (
                            v,
                            self.extras.effective_colormap(),
                            self.extras.color_limits(),
                            super::interp_band_limit(s.points.len() / 3),
                        )
                    });
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
                    let style = s.face_style(face_index);
                    let opacity = style.alpha.unwrap_or(1.);
                    let interp_values = interp.as_ref().and_then(|(v, ..)| v.triangle(face_index));
                    if let (Some((_, map, clim, limit)), Some(values)) = (&interp, interp_values)
                        && !s.face_color_none
                        && opacity > 0.
                    {
                        let corners = [transformed[0], transformed[1], transformed[2]];
                        super::interp_triangle_bands(
                            corners,
                            values,
                            bounds,
                            map,
                            *clim,
                            *limit,
                            |band, rgb| {
                                let fill = color_hex(rgb);
                                let _ = if style.alpha.is_some() {
                                    write!(
                                        svg,
                                        "<polygon fill=\"{fill}\" fill-opacity=\"{opacity}\" stroke=\"none\" points=\""
                                    )
                                } else {
                                    write!(
                                        svg,
                                        "<polygon fill=\"{fill}\" stroke=\"{fill}\" stroke-width=\"0.6\" stroke-linejoin=\"round\" points=\""
                                    )
                                };
                                for &p in band {
                                    let [x, y] =
                                        patch_screen_point(p, bounds, [left, top, right, bottom]);
                                    let _ = write!(svg, "{x:.2},{y:.2} ");
                                }
                                svg.push_str("\"/>");
                            },
                        );
                    } else if !s.face_color_none && polygon.points().len() >= 3 && opacity > 0. {
                        let own;
                        let color = match s.face_fill_color(face_index) {
                            Some(rgb) => {
                                own = color_hex(rgb);
                                &own
                            }
                            None => base_color,
                        };
                        // Triangles tile larger shapes (area, pie, fill): a hairline
                        // in the face colour hides anti-aliasing seams between them.
                        if style.alpha.is_some() {
                            let _ = write!(
                                svg,
                                "<polygon fill=\"{color}\" fill-opacity=\"{opacity}\" stroke=\"none\" points=\""
                            );
                        } else if width == 3 || s.face_colors.is_some() {
                            let _ = write!(
                                svg,
                                "<polygon fill=\"{color}\" stroke=\"{color}\" stroke-width=\"0.6\" stroke-linejoin=\"round\" points=\""
                            );
                        } else {
                            let _ =
                                write!(svg, "<polygon fill=\"{color}\" stroke=\"none\" points=\"");
                        }
                        for &p in polygon.points() {
                            let [x, y] = patch_screen_point(p, bounds, [left, top, right, bottom]);
                            let _ = write!(svg, "{x:.2},{y:.2} ");
                        }
                        svg.push_str("\"/>");
                    }
                    face_outline(
                        &mut svg,
                        style,
                        s.face_outline_color(face_index),
                        s.edge_alpha,
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
                    let (baseline, b) = if s.bar_layout == super::BarLayout::Stacked {
                        match self.bar_y_span(series_index, point_index, axes[1]) {
                            Ok(Some([base_y, top_y])) => (base_y, top_y),
                            Ok(None) => continue,
                            Err(_) => return String::new(),
                        }
                    } else {
                        (series_base, b)
                    };
                    if axes[0].accepts(a) && axes[1].accepts(b) && axes[1].accepts(baseline) {
                        let left = a + s.bar_offset - s.bar_width / 2.;
                        let right = a + s.bar_offset + s.bar_width / 2.;
                        let l = px(if axes[0].accepts(left) { left } else { x[0] });
                        let r = px(if axes[0].accepts(right) { right } else { x[1] });
                        let top = py(b).min(py(baseline));
                        let height = (py(b) - py(baseline)).abs();
                        let alpha = s.face_alpha.unwrap_or(1.);
                        if s.face_color_mode != super::FaceColorMode::None && alpha > 0. {
                            let fill = if s.face_color_mode == super::FaceColorMode::Flat {
                                s.face_colors
                                    .as_ref()
                                    .and_then(|c| c.get(point_index))
                                    .map(|&rgb| color_hex(rgb))
                                    .unwrap_or_else(|| color.clone())
                            } else {
                                color.clone()
                            };
                            if alpha < 1. {
                                let _ = write!(
                                    svg,
                                    "<rect x=\"{l:.2}\" y=\"{top:.2}\" width=\"{:.2}\" height=\"{height:.2}\" fill=\"{fill}\" fill-opacity=\"{alpha:.4}\"/>",
                                    (r - l).max(0.)
                                );
                            } else {
                                let _ = write!(
                                    svg,
                                    "<rect x=\"{l:.2}\" y=\"{top:.2}\" width=\"{:.2}\" height=\"{height:.2}\" fill=\"{fill}\"/>",
                                    (r - l).max(0.)
                                );
                            }
                        }
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
                                FaceStyle::of(s),
                                s.face_edge_color,
                                None,
                                &face,
                                [tx[0], tx[1], ty[0], ty[1]],
                                [px(x[0]), py(y[1]), px(x[1]), py(y[0])],
                                [true, axes[0].accepts(right), true, axes[0].accepts(left)],
                            );
                        }
                    }
                }
            }
            if draw_volumes(&mut svg, series_index + 1).is_err() {
                return String::new();
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
        let emit_top = layer_top && !(top_grid.is_empty() && top_ticks.is_empty());
        extras::draw(
            &mut svg,
            self,
            [left, top, right, bottom],
            &px,
            &py,
            appearance,
            emit_top,
        );
        if emit_top {
            let _ = write!(
                svg,
                "<g data-matrust-layer=\"top\">{top_grid}{top_ticks}</g>"
            );
            // The right axis stays above the top group; rectangles stay under it.
            extras::draw_right_axis(&mut svg, self, [top, right, bottom], &py);
        }
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
pub(super) fn ticks(range: [f64; 2]) -> (Vec<f64>, f64) {
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
pub(super) fn tick_text(value: f64, step: f64) -> String {
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
pub(super) struct SvgRun {
    text: String,
    size: f32,
    ascent: f32,
    inset: f32,
    style: &'static str,
}

const BOX_FONT_FAMILY: &str = "Arial, Liberation Sans, Helvetica, DejaVu Sans, sans-serif";

#[cfg(feature = "plot-export")]
fn svg_run_metrics(
    text: &str,
    size: f32,
    style: &str,
    options: &resvg::usvg::Options<'_>,
) -> Option<[f32; 4]> {
    // Shape the same text and style that will be written to the figure. Include
    // ink overhangs as well as advances when laying out the neighboring boxes.
    let svg = format!(
        r#"<svg xmlns="http://www.w3.org/2000/svg" width="1" height="1"><text font-family="{BOX_FONT_FAMILY}" font-size="{size}" xml:space="preserve"{style}>{text}</text></svg>"#
    );
    let tree = resvg::usvg::Tree::from_str(&svg, options).ok()?;
    let resvg::usvg::Node::Text(node) = tree.root().children().first()? else {
        return None;
    };
    let ink = node.stroke_bounding_box();
    let left = ink.left().min(0.);
    Some([
        node.bounding_box().right().max(ink.right()) - left,
        -left,
        (size * 0.8).max(-ink.top()),
        (size * 0.2).max(ink.bottom()),
    ])
}

fn fallback_run_width(text: &str, size: f32) -> f32 {
    size * text
        .chars()
        .map(|ch| match ch {
            ' ' | 'i' | 'l' | 'I' | '.' | ',' | ':' | ';' | '!' | '|' => 0.32,
            '@' => 1.02,
            'm' | 'w' | 'M' | 'W' | '∑' | '∏' => 0.9,
            ch if ch.is_ascii_uppercase() => 0.7,
            _ => 0.62,
        })
        .sum::<f32>()
}

pub(super) enum SvgMarkup {
    Inline {
        nodes: Vec<text_markup::Node>,
        bold: bool,
        italic: bool,
    },
    Boxes(markup_layout::Layout<SvgRun>),
}

impl SvgMarkup {
    pub(super) fn new(
        source: &str,
        interpreter: text_markup::Interpreter,
        size: f32,
        align: TextAlign,
    ) -> Self {
        Self::new_with_style(source, interpreter, size, align, false, false)
    }

    pub(super) fn new_with_style(
        source: &str,
        interpreter: text_markup::Interpreter,
        size: f32,
        align: TextAlign,
        bold: bool,
        italic: bool,
    ) -> Self {
        let nodes = text_markup::parse_nodes(source, interpreter);
        if !markup_layout::has_boxes(&nodes) {
            return Self::Inline {
                nodes,
                bold,
                italic,
            };
        }
        #[cfg(feature = "plot-export")]
        let options = crate::export::svg_render_options();
        Self::Boxes(markup_layout::layout(
            &nodes,
            size,
            size * 1.2,
            align,
            |text, size, run_bold, run_italic| {
                let style = match (bold || run_bold, italic || run_italic) {
                    (true, true) => r#" font-weight="bold" font-style="italic""#,
                    (true, false) => r#" font-weight="bold""#,
                    (false, true) => r#" font-style="italic""#,
                    (false, false) => "",
                };
                let escaped = escape(text);
                let fallback = || [fallback_run_width(text, size), 0., size * 0.8, size * 0.2];
                #[cfg(feature = "plot-export")]
                let [width, inset, ascent, descent] =
                    svg_run_metrics(&escaped, size, style, &options).unwrap_or_else(fallback);
                #[cfg(not(feature = "plot-export"))]
                let [width, inset, ascent, descent] = fallback();
                markup_layout::MeasuredRun {
                    width,
                    ascent,
                    descent,
                    glyph: SvgRun {
                        text: escaped,
                        size,
                        ascent,
                        inset,
                        style,
                    },
                }
            },
        ))
    }

    fn height(&self, nominal: f64) -> f64 {
        match self {
            Self::Inline { .. } => nominal,
            Self::Boxes(layout) => f64::from(layout.size[1]),
        }
    }

    fn rotated_x(&self, x: f64) -> f64 {
        match self {
            Self::Inline { .. } => x,
            Self::Boxes(layout) => x.max(f64::from(layout.size[1]) / 2. + 2.),
        }
    }

    /// Inline text retains SVG's native anchoring. Boxed text and rules share
    /// absolute coordinates and a transform, so rotations apply to both.
    fn write(
        &self,
        svg: &mut impl Write,
        inline_position: std::fmt::Arguments<'_>,
        boxed_origin: [f64; 2],
        align: TextAlign,
        attributes: std::fmt::Arguments<'_>,
        rule_color: &str,
    ) {
        let Self::Boxes(layout) = self else {
            if let Self::Inline {
                nodes,
                bold,
                italic,
            } = self
            {
                let text = inline_markup(nodes, boxed_origin[0], *bold, *italic);
                let _ = write!(svg, "<text{inline_position}{attributes}>{text}</text>");
            }
            return;
        };
        let [x, y] = boxed_origin;
        let x = x - f64::from(layout.size[0])
            * match align {
                TextAlign::Left => 0.,
                TextAlign::Center => 0.5,
                TextAlign::Right => 1.,
            };
        let _ = write!(svg, "<g{attributes}>");
        for run in &layout.glyphs {
            let tx = x + f64::from(run.origin[0] + run.glyph.inset);
            let ty = y + f64::from(run.origin[1] + run.glyph.ascent);
            let _ = write!(
                svg,
                r#"<text x="{tx:.3}" y="{ty:.3}" font-family="{BOX_FONT_FAMILY}" font-size="{:.3}" text-anchor="start" xml:space="preserve"{}>{}</text>"#,
                run.glyph.size, run.glyph.style, run.glyph.text,
            );
        }
        for rule in &layout.rules {
            let [x1, y1] = rule.start;
            let [x2, y2] = rule.end;
            let _ = write!(
                svg,
                "<line x1=\"{:.3}\" y1=\"{:.3}\" x2=\"{:.3}\" y2=\"{:.3}\" stroke-width=\"{:.3}\" stroke=\"{rule_color}\"/>",
                x + f64::from(x1),
                y + f64::from(y1),
                x + f64::from(x2),
                y + f64::from(y2),
                rule.thickness,
            );
        }
        let _ = svg.write_str("</g>");
    }
}

/// Export pixels per point for explicit FontSize and subtitle.
pub(super) const EXPORT_PX_PER_POINT: f32 = 1.7;
/// Export subtitle pixels per point: the 10 pt default stays at the 17 px it always had.
const SUBTITLE_PX_PER_POINT: f32 = EXPORT_PX_PER_POINT;

pub(super) fn style_attrs(style: super::LabelStyle) -> String {
    let mut out = String::new();
    if let Some(bold) = style.bold {
        out.push_str(if bold {
            r#" font-weight="bold""#
        } else {
            r#" font-weight="normal""#
        });
    }
    if let Some(italic) = style.italic {
        out.push_str(if italic {
            r#" font-style="italic""#
        } else {
            r#" font-style="normal""#
        });
    }
    out
}

struct SvgHeadings {
    title: SvgMarkup,
    title_height: f64,
    title_size: f32,
    subtitle: Option<SvgMarkup>,
    subtitle_height: f64,
    subtitle_size: f32,
}

impl SvgHeadings {
    fn new(
        title: &str,
        interpreter: text_markup::Interpreter,
        subtitle: &Subtitle,
        title_style: super::LabelStyle,
    ) -> Self {
        let title_size = title_style.export_font_size_pixels(20., EXPORT_PX_PER_POINT);
        let heading = SvgMarkup::new_with_style(
            title,
            interpreter,
            title_size,
            TextAlign::Center,
            title_style.bold.unwrap_or(false),
            title_style.italic.unwrap_or(false),
        );
        let title_height = if title.is_empty() {
            0.
        } else {
            heading.height(
                f64::from(title_size)
                    + f64::from(title_size) * 1.2 * title.matches('\n').count().min(20) as f64,
            )
        };
        let subtitle_size = subtitle.font_size_pixels(SUBTITLE_PX_PER_POINT);
        let subheading = (!subtitle.string.is_empty()).then(|| {
            SvgMarkup::new_with_style(
                &subtitle.string,
                subtitle.interpreter,
                subtitle_size,
                TextAlign::Center,
                subtitle.bold,
                subtitle.italic,
            )
        });
        let subtitle_height = subheading.as_ref().map_or(0., |markup| {
            markup.height(
                f64::from(subtitle_size)
                    * (1. + 1.2 * subtitle.string.matches('\n').count().min(20) as f64),
            )
        });
        Self {
            title: heading,
            title_height,
            title_size,
            subtitle: subheading,
            subtitle_height,
            subtitle_size,
        }
    }

    fn extra_height(&self) -> f64 {
        (self.title_height - 20.).max(0.) + self.subtitle_height + self.gap()
    }

    fn baseline_offset(&self, title: &str) -> f64 {
        match self.title {
            // The legacy margin caps the line count but the first baseline uses every line.
            SvgMarkup::Inline { .. } => {
                f64::from(self.title_size) * 1.2 * title.matches('\n').count() as f64
                    + self.subtitle_height
                    + self.gap()
            }
            SvgMarkup::Boxes(_) => self.extra_height(),
        }
    }

    fn gap(&self) -> f64 {
        if self.title_height > 0. && self.subtitle.is_some() {
            4.
        } else {
            0.
        }
    }

    fn write_subtitle(
        &self,
        svg: &mut SvgBuffer<'_>,
        x: f64,
        top: f64,
        subtitle: &Subtitle,
        default_color: &str,
    ) {
        let Some(markup) = &self.subtitle else {
            return;
        };
        let color = subtitle.color.map_or_else(
            || default_color.to_owned(),
            |rgb| color_hex(rgb.map(|v| (v * 255.).round() as u8)),
        );
        let top = top + self.title_height + self.gap();
        let size = self.subtitle_size;
        let baseline = top + f64::from(size * 0.8);
        let weight = if subtitle.bold { "bold" } else { "normal" };
        let style_attr = if subtitle.italic {
            r#" font-style="italic""#
        } else {
            ""
        };
        markup.write(
            svg,
            format_args!(r#" x="{x}" y="{baseline}" text-anchor="middle""#),
            [x, top],
            TextAlign::Center,
            format_args!(
                r#" font-family="Arial" font-size="{size}" font-weight="{weight}"{style_attr} fill="{color}""#
            ),
            &color,
        );
    }
}

fn inline_markup(nodes: &[text_markup::Node], x: f64, bold: bool, italic: bool) -> String {
    let mut result = String::new();
    let mut pending_lines = 0;
    for node in nodes {
        let text_markup::Node::Run(run) = node else {
            continue;
        };
        for (i, line) in run.text.split('\n').enumerate() {
            pending_lines += usize::from(i > 0);
            if line.is_empty() {
                continue;
            }
            let new_line = pending_lines > 0;
            if new_line {
                let _ = write!(
                    result,
                    "<tspan x=\"{x}\" dy=\"{}em\">",
                    pending_lines as f64 * 1.2
                );
                pending_lines = 0;
            }
            if run.scale == 1. && run.shift == 0. && !run.bold && !run.italic {
                result.push_str(&escape(line));
            } else {
                let _ = write!(
                    result,
                    "<tspan font-size=\"{}%\" baseline-shift=\"{}%\" font-weight=\"{}\" font-style=\"{}\">{}</tspan>",
                    run.scale * 100.,
                    run.shift * 100.,
                    if bold || run.bold { "bold" } else { "normal" },
                    if italic || run.italic {
                        "italic"
                    } else {
                        "normal"
                    },
                    escape(line)
                );
            }
            if new_line {
                result.push_str("</tspan>");
            }
        }
    }
    result
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
        if textbox::validate_textboxes(&self.textboxes).is_err()
            || super::annotation::validate_annotations(&self.annotations).is_err()
        {
            return String::new();
        }
        let [fig_w, fig_h] = self.export_svg_size();
        if self.layout.is_none() {
            let mut inner = self.axes.svg_sized(fig_w, fig_h, "axes", surface, cancel);
            if target.is_some()
                || (self.textboxes.is_empty() && self.annotations.is_empty())
                || inner.is_empty()
            {
                return inner;
            }
            let Some(end) = inner.strip_suffix("</svg>").map(str::len) else {
                return String::new();
            };
            inner.truncate(end);
            let mut result = SvgBuffer::new(inner, cancel);
            draw_textboxes(&mut result, self);
            draw_annotations(&mut result, self);
            result.push_str("</svg>");
            return result.finish();
        }
        let mut panels: Vec<_> = self.all_axes().collect();
        panels.sort_by_key(|(p, _)| *p);
        let mut result = SvgBuffer::new(
            format!(
                "<svg xmlns=\"http://www.w3.org/2000/svg\" width=\"{fig_w}\" height=\"{fig_h}\" viewBox=\"0 0 {fig_w} {fig_h}\">"
            ),
            cancel,
        );
        if opaque {
            let _ = write!(
                result,
                "<rect width=\"{fig_w}\" height=\"{fig_h}\" fill=\"{}\"/>",
                surface.fill()
            );
        }
        let layout = self.layout.as_ref().unwrap();
        let title = &layout.title;
        let xlabel = &layout.xlabel;
        let ylabel = &layout.ylabel;
        let headings = SvgHeadings::new(
            title,
            layout.text_interpreters[0],
            &layout.subtitle,
            layout.title_style,
        );
        let header = if title.is_empty() && headings.subtitle.is_none() {
            0.
        } else {
            36. + headings.extra_height()
        };
        let xlabel_extra = xlabel.matches('\n').count().min(20) as f64;
        let ylabel_extra = ylabel.matches('\n').count().min(20) as f64;
        let xlabel_style = layout.label_styles[0];
        let xlabel_size = xlabel_style.export_font_size_pixels(16., EXPORT_PX_PER_POINT);
        let extra_xlabel_glyph = if xlabel.is_empty() {
            0.
        } else {
            (f64::from(xlabel_size) - 16.).max(0.)
        };
        let xlabel_step = (f64::from(xlabel_size) * 1.2).max(20.);
        let ylabel_style = layout.label_styles[1];
        let ylabel_size = ylabel_style.export_font_size_pixels(16., EXPORT_PX_PER_POINT);
        let extra_ylabel_glyph = if ylabel.is_empty() {
            0.
        } else {
            (f64::from(ylabel_size) - 16.).max(0.)
        };
        let ylabel_step = (f64::from(ylabel_size) * 1.2).max(20.);
        let footer = if xlabel.is_empty() {
            0.
        } else {
            28. + extra_xlabel_glyph + xlabel_step * xlabel_extra
        };
        let left_margin = if ylabel.is_empty() {
            0.
        } else {
            28. + extra_ylabel_glyph + ylabel_step * ylabel_extra
        };
        let canvas_width = fig_w - left_margin;
        let canvas_height = fig_h - header - footer;
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
                        .min(canvas_width * (r - l) / 320.)
                        .min(canvas_height * (b - t) / 220.)
                });
        if target.is_none() {
            let title_color = color_hex(appearance.shared_title);
            let cx = left_margin + canvas_width / 2.;
            let extra_title_glyph = if title.is_empty() {
                0.
            } else {
                (f64::from(headings.title_size) - 20.).max(0.)
            };
            let title_y = 25. + extra_title_glyph;
            let title_top = if title.is_empty() {
                9.
            } else {
                title_y - f64::from(headings.title_size) * 0.8
            };
            if !title.is_empty() {
                let title_size = headings.title_size;
                let title_attrs = style_attrs(layout.title_style);
                headings.title.write(
                    &mut result,
                    format_args!(r#" x="{cx}" y="{title_y}" text-anchor="middle""#),
                    [cx, title_top],
                    TextAlign::Center,
                    format_args!(
                        r#" font-family="sans-serif" font-size="{title_size}"{title_attrs} fill="{title_color}""#
                    ),
                    &title_color,
                );
            }
            headings.write_subtitle(&mut result, cx, title_top, &layout.subtitle, &title_color);
            if !xlabel.is_empty() {
                let xlabel_attrs = style_attrs(xlabel_style);
                let y = (fig_h - 10. - extra_xlabel_glyph * 0.2 - xlabel_step * xlabel_extra)
                    .max(18. + extra_xlabel_glyph);
                let label = SvgMarkup::new_with_style(
                    xlabel,
                    layout.text_interpreters[1],
                    xlabel_size,
                    TextAlign::Center,
                    xlabel_style.bold.unwrap_or(false),
                    xlabel_style.italic.unwrap_or(false),
                );
                label.write(
                    &mut result,
                    format_args!(r#" x="{cx}" y="{y}" text-anchor="middle""#),
                    [cx, y + 4. - label.height(f64::from(xlabel_size))],
                    TextAlign::Center,
                    format_args!(
                        r#" font-family="sans-serif" font-size="{xlabel_size}"{xlabel_attrs} fill="{title_color}""#
                    ),
                    &title_color,
                );
            }
            if !ylabel.is_empty() {
                let ylabel_attrs = style_attrs(ylabel_style);
                let cy = header + canvas_height / 2.;
                let label = SvgMarkup::new_with_style(
                    ylabel,
                    layout.text_interpreters[2],
                    ylabel_size,
                    TextAlign::Center,
                    ylabel_style.bold.unwrap_or(false),
                    ylabel_style.italic.unwrap_or(false),
                );
                let x = label.rotated_x(18. + extra_ylabel_glyph);
                label.write(
                    &mut result,
                    format_args!(r#" x="{x}" y="{cy}" text-anchor="middle""#),
                    [x, cy - label.height(f64::from(ylabel_size)) / 2.],
                    TextAlign::Center,
                    format_args!(r#" transform="rotate(-90 {x} {cy})" font-family="sans-serif" font-size="{ylabel_size}"{ylabel_attrs} fill="{title_color}""#),
                    &title_color,
                );
            }
        }
        for (i, (p, a)) in panels.into_iter().enumerate() {
            if target.is_some_and(|selected| p != selected) {
                continue;
            }
            let [left, top, right, bottom] = p.rectangle();
            if p.custom.is_some() {
                let inner = a.svg_positioned(
                    [fig_w, fig_h],
                    Some([
                        left_margin + canvas_width * left,
                        header + canvas_height * top,
                        left_margin + canvas_width * right,
                        header + canvas_height * bottom,
                    ]),
                    Some(LegendRect::new([
                        left_margin,
                        header,
                        fig_w,
                        fig_h - footer,
                    ])),
                    None,
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
            let width = canvas_width * (right - left) / scale;
            let height = canvas_height * (bottom - top) / scale;
            let tile_margins = layout.tiled_svg_margins(p);
            let inner = a.svg_positioned(
                [width, height],
                None,
                None,
                tile_margins,
                &format!("axes{i}"),
                surface,
                cancel,
            );
            if inner.is_empty() {
                return String::new();
            }
            let Some(inner) = buffer::replace_first(
                inner,
                &format!("width=\"{width}\" height=\"{height}\""),
                &format!(
                    "x=\"{}\" y=\"{}\" width=\"{}\" height=\"{}\"",
                    left_margin + canvas_width * left,
                    header + canvas_height * top,
                    canvas_width * (right - left),
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
            draw_annotations(&mut result, self);
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
    draw_marker_styled(svg, series, x, y, color, legend, appearance, None);
}

struct PointStyle<'a> {
    index: usize,
    map: &'a [[f64; 3]],
    clim: [f64; 2],
    axes_bg: [u8; 3],
}

#[allow(clippy::too_many_arguments)]
fn draw_marker_styled(
    svg: &mut impl std::fmt::Write,
    series: &super::Series,
    x: f64,
    y: f64,
    color: &str,
    legend: bool,
    appearance: ResolvedAppearance,
    point: Option<PointStyle<'_>>,
) {
    use super::{MarkerColor, MarkerShape::*};
    let shape = series.effective_marker();
    let radius = if legend {
        if series.has_marker_style() {
            series.marker_radius().min(8.)
        } else {
            3.
        }
    } else if let Some(ref p) = point {
        series.marker_radius_at(p.index)
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
    let cdata_owned = point.as_ref().and_then(|p| {
        series.scatter_cdata.as_ref().map(|_| {
            color_hex(series.scatter_color_at(
                p.index,
                appearance.series_color(series),
                p.map,
                p.clim,
            ))
        })
    });
    let cdata_hex = cdata_owned.as_deref().unwrap_or(color);
    let fill_owned;
    let fill = if legend {
        if series.marker_is_filled() {
            color
        } else {
            background.as_deref().unwrap_or("white")
        }
    } else if series.has_scatter_style() {
        match series.marker_face_color {
            MarkerColor::None => "none",
            MarkerColor::Flat => cdata_hex,
            MarkerColor::Auto => {
                let bg = point
                    .as_ref()
                    .map_or(appearance.legend_background, |p| p.axes_bg);
                fill_owned = color_hex(bg);
                &fill_owned
            }
            MarkerColor::Rgb(rgb) => {
                fill_owned = color_hex(rgb);
                &fill_owned
            }
            MarkerColor::Default => {
                if series.marker_is_filled() {
                    cdata_hex
                } else {
                    "none"
                }
            }
        }
    } else if series.marker_is_filled() {
        color
    } else {
        "none"
    };
    let stroke_owned;
    let stroke = if !legend && series.has_scatter_style() {
        match series.marker_edge_color {
            MarkerColor::None => "none",
            MarkerColor::Default | MarkerColor::Flat | MarkerColor::Auto => cdata_hex,
            MarkerColor::Rgb(rgb) => {
                stroke_owned = color_hex(rgb);
                &stroke_owned
            }
        }
    } else {
        color
    };
    let fill_alpha = (!legend)
        .then_some(series.marker_face_alpha)
        .flatten()
        .filter(|&a| a < 1. && fill != "none");
    let stroke_alpha = (!legend)
        .then_some(series.marker_edge_alpha)
        .flatten()
        .filter(|&a| a < 1. && stroke != "none");
    let mut alpha_attrs = String::new();
    if let Some(a) = fill_alpha {
        let _ = write!(alpha_attrs, " fill-opacity=\"{a}\"");
    }
    if let Some(a) = stroke_alpha {
        let _ = write!(alpha_attrs, " stroke-opacity=\"{a}\"");
    }
    if matches!(shape, Circle | Point) {
        let _ = write!(
            svg,
            "<circle cx=\"{x:.2}\" cy=\"{y:.2}\" r=\"{radius}\" fill=\"{fill}\" stroke=\"{stroke}\" stroke-width=\"{width}\"{alpha_attrs}/>"
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
            "<path fill=\"{fill}\" stroke=\"{stroke}\" stroke-width=\"{width}\"{alpha_attrs} d=\""
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
        let stroke_only_alpha =
            stroke_alpha.map_or(String::new(), |a| format!(" stroke-opacity=\"{a}\""));
        let _ = write!(
            svg,
            "<path fill=\"none\" stroke=\"{stroke}\" stroke-width=\"{width}\"{stroke_only_alpha} d=\""
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
    let [fig_w, fig_h] = figure.export_svg_size();
    for (index, textbox) in figure.textboxes.iter().enumerate() {
        if svg.stopped(0) {
            return;
        }
        if !textbox.visible || textbox.position[2] == 0. || textbox.position[3] == 0. {
            continue;
        }
        let [left, top, right, bottom] = textbox.rectangle([0., 0., fig_w, fig_h]);
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

fn draw_annotations(svg: &mut SvgBuffer<'_>, figure: &Figure) {
    use super::annotation::{Annotation, AnnotationKind};
    let [fig_w, fig_h] = figure.export_svg_size();
    let canvas = [0., 0., fig_w, fig_h];
    for ann in &figure.annotations {
        if svg.stopped(0) {
            return;
        }
        if !ann.visible {
            continue;
        }
        let stroke_color = color_hex(ann.color);
        let stroke_width = ann.line_width * (4.0 / 3.0);
        let dash = match ann.pattern {
            LinePattern::Solid | LinePattern::None => "",
            LinePattern::Dashed => "8 5",
            LinePattern::Dotted => "2 4",
            LinePattern::DashDot => "8 4 2 4",
        };
        let dash_attr = if dash.is_empty() {
            String::new()
        } else {
            format!(" stroke-dasharray=\"{dash}\"")
        };
        if ann.kind.is_box() {
            let [left, top, right, bottom] = ann.rectangle(canvas);
            let w = (right - left).max(0.0);
            let h = (bottom - top).max(0.0);
            if w == 0.0 && h == 0.0 {
                continue;
            }
            let fill = match ann.face_color {
                Some(fc) if ann.face_alpha > 0.0 => {
                    if (ann.face_alpha - 1.0).abs() < 1e-6 {
                        format!("fill=\"{}\"", color_hex(fc))
                    } else {
                        format!(
                            "fill=\"{}\" fill-opacity=\"{:.4}\"",
                            color_hex(fc),
                            ann.face_alpha
                        )
                    }
                }
                _ => "fill=\"none\"".to_string(),
            };
            let stroke = if !ann.edge_none && ann.pattern != LinePattern::None {
                format!("stroke=\"{stroke_color}\" stroke-width=\"{stroke_width:.3}\"{dash_attr}")
            } else {
                "stroke=\"none\"".to_string()
            };
            match ann.kind {
                AnnotationKind::Rectangle => {
                    let _ = write!(
                        svg,
                        "<rect x=\"{left:.2}\" y=\"{top:.2}\" width=\"{w:.2}\" height=\"{h:.2}\" {fill} {stroke}/>"
                    );
                }
                AnnotationKind::Ellipse => {
                    let cx = (left + right) * 0.5;
                    let cy = (top + bottom) * 0.5;
                    let rx = w * 0.5;
                    let ry = h * 0.5;
                    let _ = write!(
                        svg,
                        "<ellipse cx=\"{cx:.2}\" cy=\"{cy:.2}\" rx=\"{rx:.2}\" ry=\"{ry:.2}\" {fill} {stroke}/>"
                    );
                }
                _ => {}
            }
            continue;
        }
        let (p0, p1) = ann.pixel_endpoints(canvas);
        let head_end = matches!(
            ann.kind,
            AnnotationKind::Arrow | AnnotationKind::DoubleArrow | AnnotationKind::TextArrow
        )
        .then(|| {
            Annotation::arrow_head_geometry(p1, p0, ann.head_style, ann.head_length, ann.head_width)
        })
        .flatten();
        let head_start = (ann.kind == AnnotationKind::DoubleArrow)
            .then(|| {
                Annotation::arrow_head_geometry(
                    p0,
                    p1,
                    ann.head_style,
                    ann.head_length,
                    ann.head_width,
                )
            })
            .flatten();
        let shaft_start = head_start.as_ref().map_or(p0, |(_, attach)| *attach);
        let shaft_end = head_end.as_ref().map_or(p1, |(_, attach)| *attach);
        if ann.pattern != LinePattern::None {
            let _ = write!(
                svg,
                "<line x1=\"{:.2}\" y1=\"{:.2}\" x2=\"{:.2}\" y2=\"{:.2}\" stroke=\"{stroke_color}\" stroke-width=\"{stroke_width:.3}\"{dash_attr}/>",
                shaft_start[0], shaft_start[1], shaft_end[0], shaft_end[1]
            );
        }
        for (poly, _) in [head_start, head_end].into_iter().flatten() {
            svg.push_str("<polygon points=\"");
            for (i, pt) in poly.iter().enumerate() {
                if i > 0 {
                    svg.push_str(" ");
                }
                let _ = write!(svg, "{:.2},{:.2}", pt[0], pt[1]);
            }
            let _ = write!(svg, "\" fill=\"{stroke_color}\"/>");
        }
        if ann.kind == AnnotationKind::TextArrow && !ann.text.is_empty() {
            let font = ann.font_size * (4.0 / 3.0);
            let dx = p1[0] - p0[0];
            let (tx, anchor) = if dx >= 0.0 {
                (p0[0] - 4.0, "end")
            } else {
                (p0[0] + 4.0, "start")
            };
            let ty = p0[1] + font * 0.35;
            let _ = write!(
                svg,
                "<text x=\"{tx:.2}\" y=\"{ty:.2}\" font-family=\"sans-serif\" font-size=\"{font:.2}\" fill=\"{stroke_color}\" text-anchor=\"{anchor}\" xml:space=\"preserve\">{}</text>",
                escape(&ann.text)
            );
        }
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
#[allow(clippy::too_many_arguments)]
fn face_outline(
    svg: &mut SvgBuffer<'_>,
    style: FaceStyle,
    edge: Option<[u8; 3]>,
    edge_alpha: Option<f64>,
    face: &[[f64; 2]],
    bounds: [f64; 4],
    frame: [f64; 4],
    edges: [bool; 4],
) {
    let Some(rgb) = edge else {
        return;
    };
    let alpha = edge_alpha.unwrap_or(1.);
    if alpha <= 0.
        || style.pattern == LinePattern::None
        || face.len() < 3
        || face.iter().flatten().any(|v| !v.is_finite())
    {
        return;
    }
    let color = color_hex(rgb);
    let dash = match style.pattern {
        LinePattern::Dashed => "8 5",
        LinePattern::Dotted => "2 4",
        LinePattern::DashDot => "8 4 2 4",
        _ => "",
    };
    let opacity_attr = if edge_alpha.is_some() && alpha < 1. {
        format!(" stroke-opacity=\"{alpha}\"")
    } else {
        String::new()
    };
    let _ = write!(
        svg,
        "<path fill=\"none\" stroke=\"{color}\"{opacity_attr} stroke-width=\"{}\" stroke-dasharray=\"{dash}\" d=\"",
        style.edge_width
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
