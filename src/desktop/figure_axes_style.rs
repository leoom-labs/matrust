//! Explicit axes decoration only. It contributes no bounds, legend, hover or IDs.
use super::*;
use matrust::plot::{ResolvedAxesColors, TickDir};

pub(super) fn marks(axis: AxisTransform, input: egui_plot::GridInput) -> Vec<egui_plot::GridMark> {
    if axis.scale == AxisScale::Log {
        axis.ticks([input.bounds.0, input.bounds.1])
            .into_iter()
            .map(|value| egui_plot::GridMark {
                value,
                step_size: ((input.bounds.1 - input.bounds.0) / 5.).max(input.base_step_size),
            })
            .collect()
    } else {
        // One 1/2/5 spacer for both axes (MATLAB-like 5-11 labels); shared by
        // ticks and decoration. egui's default y spacer labelled every 0.1.
        egui_plot::uniform_grid_spacer(|input| {
            let Some(step) = linear_step(input.base_step_size) else {
                return [f64::INFINITY; 3];
            };
            let smallest = f64::from_bits(1);
            [(step / 10.).max(smallest), (step / 2.).max(smallest), step]
        })(input)
    }
}

/// The 1/2/5 major step of a linear axis for egui's base step size.
fn linear_step(base_step_size: f64) -> Option<f64> {
    let target = base_step_size * 10.;
    if !target.is_finite() || target <= 0. {
        return None;
    }
    let power = 10_f64.powf(target.log10().floor());
    Some(if power == 0. {
        target
    } else if target <= 2. * power {
        2. * power
    } else if target <= 5. * power {
        5. * power
    } else {
        10. * power
    })
}

pub(super) fn custom_marks(
    values: &[f64],
    step: f64,
    input: egui_plot::GridInput,
) -> Vec<egui_plot::GridMark> {
    values
        .iter()
        .copied()
        .filter(|v| *v >= input.bounds.0 && *v <= input.bounds.1)
        .map(|value| egui_plot::GridMark {
            value,
            step_size: step,
        })
        .collect()
}

/// Smallest positive gap between tick values. This is the spacing the eye
/// sees; `manual_tick_step` is only the label's decimal precision.
fn real_tick_gap(values: &[f64]) -> f64 {
    let mut ordered: Vec<f64> = values.iter().copied().filter(|v| v.is_finite()).collect();
    ordered.sort_by(f64::total_cmp);
    let gap = ordered
        .windows(2)
        .map(|pair| pair[1] - pair[0])
        .filter(|gap| *gap > 0.)
        .fold(f64::INFINITY, f64::min);
    if gap.is_finite() { gap } else { 1. }
}

pub(super) struct Decoration {
    base: PlotItemBase,
    axes: [AxisTransform; 2],
    grid: [bool; 2],
    /// Custom ruler positions (transformed) and the egui spacer step.
    /// That step fades only the legacy Layer-top replacement. Explicit grids
    /// use the real tick gap and are not density-faded.
    custom_grid: [Option<(Vec<f64>, f64)>; 2],
    /// X and y minor grids (MATLAB XMinorGrid/YMinorGrid), dotted.
    minor: [bool; 2],
    minor_alpha: f32,
    colors: ResolvedAxesColors,
    /// Explicit axis lines: [x, y]. Also drawn for `box off`, when the
    /// caller removes the native frame stroke.
    edges: Option<[egui::Stroke; 2]>,
    box_outline: bool,
    /// Layer 'top' replacement for egui's internal grid: its base colour
    /// (`None` = the visuals' text colour, as egui_plot uses).
    native_grid: Option<Option<Color32>>,
}
impl Decoration {
    pub(super) fn new(
        figure: &Axes,
        axes: [AxisTransform; 2],
        colors: ResolvedAxesColors,
        appearance: ResolvedAppearance,
        default_edge: egui::Stroke,
        custom_grid: [Option<(Vec<f64>, f64)>; 2],
    ) -> Option<Self> {
        let intent = figure.colors.unwrap_or_default();
        let explicit_grid = intent.grid.is_some() || !appearance.legacy;
        // Layer 'top' cannot reorder egui's internal grid, so the caller turns
        // it off and this decoration paints the same lines after the children.
        let native_top = figure.extras.layer.is_top() && !explicit_grid;
        let stroke_width = figure
            .extras
            .line_width
            .map_or(default_edge.width, |w| (w as f32 * 4. / 3.).max(0.5));
        let edges = (intent.x.is_some()
            || intent.y.is_some()
            || !figure.box_outline
            || figure.extras.line_width.is_some())
        .then(|| {
            [
                egui::Stroke::new(
                    stroke_width,
                    if intent.x.is_some() {
                        rgb(colors.x)
                    } else {
                        default_edge.color
                    },
                ),
                egui::Stroke::new(
                    stroke_width,
                    if intent.y.is_some() {
                        rgb(colors.y)
                    } else {
                        default_edge.color
                    },
                ),
            ]
        });
        let minor = [figure.minor_grid[0], figure.minor_grid[1]];
        if edges.is_none()
            && !((explicit_grid || native_top) && (figure.xgrid || figure.ygrid))
            && !minor.contains(&true)
        {
            return None;
        }
        Some(Self {
            base: PlotItemBase::new(String::new()),
            axes,
            grid: [
                figure.xgrid && (explicit_grid || native_top),
                figure.ygrid && (explicit_grid || native_top),
            ],
            custom_grid,
            minor,
            // MATLAB's MinorGridAlpha where the major grid is translucent.
            minor_alpha: if explicit_grid {
                matrust::plot::MINOR_GRID_ALPHA as f32
            } else {
                colors.grid_alpha as f32
            },
            colors,
            edges,
            box_outline: figure.box_outline,
            native_grid: native_top.then(|| {
                (appearance.theme == matrust::plot::BaseTheme::Dark).then(|| rgb(appearance.grid))
            }),
        })
    }

    /// Layer 'top': splits the axis lines (still under the children) from the
    /// major and minor grid, which the caller adds after the children.
    pub(super) fn split_layer(self) -> (Option<Self>, Option<Self>) {
        let has_grid = self.grid.contains(&true) || self.minor.contains(&true);
        let lines = self.edges.map(|edges| Self {
            base: PlotItemBase::new(String::new()),
            axes: self.axes,
            grid: [false; 2],
            custom_grid: [None, None],
            minor: [false; 2],
            minor_alpha: self.minor_alpha,
            colors: self.colors,
            edges: Some(edges),
            box_outline: self.box_outline,
            native_grid: None,
        });
        let grid = has_grid.then_some(Self {
            edges: None,
            ..self
        });
        (lines, grid)
    }
}
impl PlotItem for Decoration {
    fn shapes(&self, ui: &egui::Ui, transform: &PlotTransform, shapes: &mut Vec<egui::Shape>) {
        let frame = *transform.frame();
        let bounds = transform.bounds();
        for (direction, visible) in self.minor.into_iter().enumerate() {
            if !visible {
                continue;
            }
            let range = [bounds.min()[direction], bounds.max()[direction]];
            let axis = self.axes[direction];
            let values = if axis.scale == AxisScale::Log {
                matrust::plot::minor_ticks(axis, range, 0.)
            } else {
                let base = transform.dvalue_dpos()[direction].abs() * 8.;
                let Some(step) = linear_step(base) else {
                    continue;
                };
                matrust::plot::minor_ticks(axis, range, step)
            };
            let color = [self.colors.grid_x, self.colors.grid_y][direction];
            let color = Color32::from_rgba_unmultiplied(
                color[0],
                color[1],
                color[2],
                (self.minor_alpha.clamp(0., 1.) * 255.).round() as u8,
            );
            let mut previous = f32::NEG_INFINITY;
            for value in values {
                let mut point = [0.; 2];
                point[direction] = value;
                point[1 - direction] = bounds.min()[1 - direction];
                let mut start = transform.position_from_point(&PlotPoint::new(point[0], point[1]));
                if !start.is_finite()
                    || start[direction] < frame.min[direction]
                    || start[direction] > frame.max[direction]
                    // Lines closer than 3 px read as a fill, not a grid.
                    || (start[direction] - previous).abs() < 3.
                {
                    continue;
                }
                previous = start[direction];
                let mut end = start;
                start[1 - direction] = frame.min[1 - direction];
                end[1 - direction] = frame.max[1 - direction];
                shapes.extend(egui::Shape::dashed_line(
                    &[start, end],
                    egui::Stroke::new(1., color),
                    1.,
                    2.,
                ));
            }
        }
        for (direction, visible) in self.grid.into_iter().enumerate() {
            if !visible {
                continue;
            }
            let input = egui_plot::GridInput {
                bounds: (bounds.min()[direction], bounds.max()[direction]),
                base_step_size: transform.dvalue_dpos()[direction].abs() * 8.,
            };
            // The spacer step is label precision. It must fade only the legacy
            // replacement of egui's own grid. An explicit grid is the ruler's
            // ticks at full GridAlpha; its fade key, if any, is the real gap.
            let explicit_ticks =
                self.custom_grid[direction].is_some() && self.native_grid.is_none();
            let grid_marks = if let Some((values, step)) = &self.custom_grid[direction] {
                let step = if explicit_ticks {
                    real_tick_gap(values)
                } else {
                    *step
                };
                custom_marks(values, step, input)
            } else {
                marks(self.axes[direction], input)
            };
            for mark in grid_marks {
                let spacing = (transform.dpos_dvalue()[direction] * mark.step_size).abs() as f32;
                if !explicit_ticks && (spacing <= 8. || !spacing.is_finite()) {
                    continue;
                }
                // Preserve native density fading, multiplied by the saved alpha.
                // Explicit ruler grids stay at full strength: the gap is the
                // tick spacing the user asked for, not a fade hint.
                let strength = if explicit_ticks {
                    1.
                } else {
                    ((spacing - 8.) / (300. - 8.)).clamp(0., 1.).sqrt()
                };
                let color = if let Some(native) = self.native_grid {
                    // egui_plot's own grid colour and fade.
                    native
                        .unwrap_or_else(|| ui.visuals().text_color())
                        .gamma_multiply(strength)
                } else {
                    let color = [self.colors.grid_x, self.colors.grid_y][direction];
                    Color32::from_rgba_unmultiplied(
                        color[0],
                        color[1],
                        color[2],
                        (self.colors.grid_alpha * 255.).round() as u8,
                    )
                    .gamma_multiply(strength)
                };
                let mut point = [0.; 2];
                point[direction] = mark.value;
                point[1 - direction] = bounds.min()[1 - direction];
                let mut start = transform.position_from_point(&PlotPoint::new(point[0], point[1]));
                if !start.is_finite()
                    || start[direction] < frame.min[direction]
                    || start[direction] > frame.max[direction]
                {
                    continue;
                }
                let mut end = start;
                start[1 - direction] = frame.min[1 - direction];
                end[1 - direction] = frame.max[1 - direction];
                shapes.push(egui::Shape::line_segment(
                    [start, end],
                    egui::Stroke::new(1., color),
                ));
            }
        }
        if let Some([x, y]) = self.edges {
            let frame = frame.shrink(x.width.max(y.width) * 0.5);
            // Box off keeps only the bottom x and left y axis lines.
            if self.box_outline {
                shapes.push(egui::Shape::line_segment(
                    [frame.left_top(), frame.right_top()],
                    x,
                ));
            }
            shapes.push(egui::Shape::line_segment(
                [frame.left_bottom(), frame.right_bottom()],
                x,
            ));
            shapes.push(egui::Shape::line_segment(
                [frame.left_top(), frame.left_bottom()],
                y,
            ));
            if self.box_outline {
                shapes.push(egui::Shape::line_segment(
                    [frame.right_top(), frame.right_bottom()],
                    y,
                ));
            }
        }
    }
    fn initialize(&mut self, _: std::ops::RangeInclusive<f64>) {}
    fn color(&self) -> Color32 {
        Color32::TRANSPARENT
    }
    fn allow_hover(&self) -> bool {
        false
    }
    fn geometry(&self) -> PlotGeometry<'_> {
        PlotGeometry::None
    }
    fn bounds(&self) -> PlotBounds {
        PlotBounds::NOTHING
    }
    fn base(&self) -> &PlotItemBase {
        &self.base
    }
    fn base_mut(&mut self) -> &mut PlotItemBase {
        &mut self.base
    }
}

/// Which part of each 2-D tick mark to paint.
#[derive(Clone, Copy, PartialEq, Eq)]
pub(super) enum TickPart {
    All,
    /// Inside the data frame: under the children for Layer 'bottom'.
    Inside,
    /// Outside the data frame: painted late, never clipped to the frame.
    Outside,
}

/// The 2-D tick marks of one axes, resolved against the final transform.
#[derive(Clone)]
pub(super) struct TickMarks {
    base: PlotItemBase,
    axes: [AxisTransform; 2],
    dir: TickDir,
    /// TickLength(1), a fraction of the longer frame side.
    length: f32,
    stroke_width: f32,
    /// Per axis; `None` = no ticks on that axis.
    strokes: [Option<egui::Stroke>; 2],
    /// Custom tick values (transformed), else the shared grid marks.
    values: [Option<Vec<f64>>; 2],
}
impl TickMarks {
    pub(super) fn new(
        figure: &Axes,
        axes: [AxisTransform; 2],
        colors: ResolvedAxesColors,
        default_edge: egui::Stroke,
        painted_ticks: &[Option<Vec<(f64, String)>>; 2],
    ) -> Option<Self> {
        let dir = figure.extras.tick_dir.unwrap_or_default();
        if figure.frame_hidden
            || figure.extras.axis_off
            || figure.ticks_hidden
            || dir == TickDir::None
        {
            return None;
        }
        let intent = figure.colors.unwrap_or_default();
        let has_tick_style = figure.extras.tick_dir.is_some()
            || figure.extras.tick_length.is_some()
            || figure.extras.line_width.is_some();
        let stroke_width = figure
            .extras
            .line_width
            .map_or(default_edge.width, |w| (w as f32 * 4. / 3.).max(0.5));
        let strokes = [0, 1].map(|axis| {
            (has_tick_style || figure.extras.rulers[axis].ticks.is_some()).then(|| {
                egui::Stroke::new(
                    stroke_width,
                    if (axis == 0 && intent.x.is_some()) || (axis == 1 && intent.y.is_some()) {
                        rgb(if axis == 0 { colors.x } else { colors.y })
                    } else {
                        default_edge.color
                    },
                )
            })
        });
        if strokes == [None, None] {
            return None;
        }
        Some(Self {
            base: PlotItemBase::new(String::new()),
            axes,
            dir,
            length: figure.extras.tick_length.map_or(0.01, |l| l[0]) as f32,
            stroke_width,
            strokes,
            values: [0, 1].map(|axis| {
                painted_ticks[axis]
                    .as_ref()
                    .map(|ticks| ticks.iter().map(|(v, _)| *v).collect())
            }),
        })
    }

    pub(super) fn segments(
        &self,
        transform: &PlotTransform,
        part: TickPart,
        out: &mut Vec<([egui::Pos2; 2], egui::Stroke)>,
    ) {
        let frame = *transform.frame();
        let bounds = transform.bounds();
        let (min, max) = (bounds.min(), bounds.max());
        let tick_len_px = frame.width().max(frame.height()) * self.length;
        let edge_frame = frame.shrink(self.stroke_width * 0.5);
        let (inner, outer) = match self.dir {
            TickDir::In => (true, false),
            TickDir::Out => (false, true),
            TickDir::Both => (true, true),
            TickDir::None => return,
        };
        // `All` keeps the old single segment. A split side starts on the frame
        // edge so outward ticks still read as (edge, outside end).
        let (draw_outer, draw_inner) = match part {
            TickPart::All => (outer, inner),
            TickPart::Inside => (false, inner),
            TickPart::Outside => (outer, false),
        };
        if !draw_outer && !draw_inner {
            return;
        }
        for axis in 0..2 {
            let Some(stroke) = self.strokes[axis] else {
                continue;
            };
            let tick_values: Vec<f64> = if let Some(values) = &self.values[axis] {
                values.clone()
            } else {
                let input = egui_plot::GridInput {
                    bounds: (min[axis], max[axis]),
                    base_step_size: transform.dvalue_dpos()[axis].abs() * 8.,
                };
                let marks = marks(self.axes[axis], input);
                let max_step = marks.iter().map(|m| m.step_size).fold(0.0_f64, f64::max);
                marks
                    .into_iter()
                    .filter(|m| m.step_size >= max_step * (1.0 - 1e-6))
                    .map(|m| m.value)
                    .collect()
            };
            for value in tick_values {
                if !value.is_finite() || value < min[axis] || value > max[axis] {
                    continue;
                }
                let point = if axis == 0 {
                    PlotPoint::new(value, min[1])
                } else {
                    PlotPoint::new(min[0], value)
                };
                let pos = transform.position_from_point(&point);
                if !pos.is_finite() {
                    continue;
                }
                // X ticks leave the bottom edge (outward is +y). Y ticks leave
                // the left edge (outward is −x). A full `both` tick runs from
                // the inward end to the outward end.
                let seg = if axis == 0 {
                    if pos.x < frame.left() - 0.5 || pos.x > frame.right() + 0.5 {
                        continue;
                    }
                    let y0 = edge_frame.bottom();
                    let [y_a, y_b] = if draw_inner && draw_outer {
                        [y0 - tick_len_px, y0 + tick_len_px]
                    } else if draw_outer {
                        [y0, y0 + tick_len_px]
                    } else {
                        [y0, y0 - tick_len_px]
                    };
                    [egui::pos2(pos.x, y_a), egui::pos2(pos.x, y_b)]
                } else {
                    if pos.y < frame.top() - 0.5 || pos.y > frame.bottom() + 0.5 {
                        continue;
                    }
                    let x0 = edge_frame.left();
                    let [x_a, x_b] = if draw_inner && draw_outer {
                        [x0 - tick_len_px, x0 + tick_len_px]
                    } else if draw_outer {
                        [x0, x0 - tick_len_px]
                    } else {
                        [x0, x0 + tick_len_px]
                    };
                    [egui::pos2(x_a, pos.y), egui::pos2(x_b, pos.y)]
                };
                out.push((seg, stroke));
            }
        }
    }
}
/// Layer 'bottom': the inside part of the ticks, painted before the children.
impl PlotItem for TickMarks {
    fn shapes(&self, _ui: &egui::Ui, transform: &PlotTransform, shapes: &mut Vec<egui::Shape>) {
        let mut segments = Vec::new();
        self.segments(transform, TickPart::Inside, &mut segments);
        shapes.extend(
            segments
                .into_iter()
                .map(|(seg, stroke)| egui::Shape::line_segment(seg, stroke)),
        );
    }
    fn initialize(&mut self, _: std::ops::RangeInclusive<f64>) {}
    fn color(&self) -> Color32 {
        Color32::TRANSPARENT
    }
    fn allow_hover(&self) -> bool {
        false
    }
    fn geometry(&self) -> PlotGeometry<'_> {
        PlotGeometry::None
    }
    fn bounds(&self) -> PlotBounds {
        PlotBounds::NOTHING
    }
    fn base(&self) -> &PlotItemBase {
        &self.base
    }
    fn base_mut(&mut self) -> &mut PlotItemBase {
        &mut self.base
    }
}
