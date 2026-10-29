//! Explicit axes decoration only. It contributes no bounds, legend, hover or IDs.
use super::*;
use matrust::plot::ResolvedAxesColors;

pub(super) fn marks(
    axis: AxisTransform,
    direction: usize,
    input: egui_plot::GridInput,
) -> Vec<egui_plot::GridMark> {
    if axis.scale == AxisScale::Log {
        axis.ticks([input.bounds.0, input.bounds.1])
            .into_iter()
            .map(|value| egui_plot::GridMark {
                value,
                step_size: ((input.bounds.1 - input.bounds.0) / 5.).max(input.base_step_size),
            })
            .collect()
    } else if direction == 0 {
        // Exact existing x-axis 1/2/5 spacer; shared by ticks and decoration.
        egui_plot::uniform_grid_spacer(|input| {
            let target = input.base_step_size * 10.;
            if !target.is_finite() || target <= 0. {
                return [f64::INFINITY; 3];
            }
            let power = 10_f64.powf(target.log10().floor());
            let step = if power == 0. {
                target
            } else if target <= 2. * power {
                2. * power
            } else if target <= 5. * power {
                5. * power
            } else {
                10. * power
            };
            let smallest = f64::from_bits(1);
            [(step / 10.).max(smallest), (step / 2.).max(smallest), step]
        })(input)
    } else {
        // Exact egui_plot 0.37 default y spacer.
        egui_plot::log_grid_spacer(10)(input)
    }
}

pub(super) struct Decoration {
    base: PlotItemBase,
    axes: [AxisTransform; 2],
    grid: [bool; 2],
    colors: ResolvedAxesColors,
    edges: Option<[egui::Stroke; 2]>,
}
impl Decoration {
    pub(super) fn new(
        figure: &Axes,
        axes: [AxisTransform; 2],
        colors: ResolvedAxesColors,
        appearance: ResolvedAppearance,
        default_edge: egui::Stroke,
    ) -> Option<Self> {
        let intent = figure.colors.unwrap_or_default();
        let explicit_grid = intent.grid.is_some() || !appearance.legacy;
        let edges = (intent.x.is_some() || intent.y.is_some()).then(|| {
            [
                if intent.x.is_some() {
                    egui::Stroke::new(default_edge.width, rgb(colors.x))
                } else {
                    default_edge
                },
                if intent.y.is_some() {
                    egui::Stroke::new(default_edge.width, rgb(colors.y))
                } else {
                    default_edge
                },
            ]
        });
        if edges.is_none() && !(explicit_grid && (figure.xgrid || figure.ygrid)) {
            return None;
        }
        Some(Self {
            base: PlotItemBase::new(String::new()),
            axes,
            grid: [figure.xgrid && explicit_grid, figure.ygrid && explicit_grid],
            colors,
            edges,
        })
    }
}
impl PlotItem for Decoration {
    fn shapes(&self, _ui: &egui::Ui, transform: &PlotTransform, shapes: &mut Vec<egui::Shape>) {
        let frame = *transform.frame();
        let bounds = transform.bounds();
        for (direction, visible) in self.grid.into_iter().enumerate() {
            if !visible {
                continue;
            }
            let input = egui_plot::GridInput {
                bounds: (bounds.min()[direction], bounds.max()[direction]),
                base_step_size: transform.dvalue_dpos()[direction].abs() * 8.,
            };
            for mark in marks(self.axes[direction], direction, input) {
                let spacing = (transform.dpos_dvalue()[direction] * mark.step_size).abs() as f32;
                if spacing <= 8. || !spacing.is_finite() {
                    continue;
                }
                // Preserve native density fading, multiplied by the saved alpha.
                let strength = ((spacing - 8.) / (300. - 8.)).clamp(0., 1.).sqrt();
                let color = [self.colors.grid_x, self.colors.grid_y][direction];
                let color = Color32::from_rgba_unmultiplied(
                    color[0],
                    color[1],
                    color[2],
                    (self.colors.grid_alpha * 255.).round() as u8,
                )
                .gamma_multiply(strength);
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
            shapes.push(egui::Shape::line_segment(
                [frame.left_top(), frame.right_top()],
                x,
            ));
            shapes.push(egui::Shape::line_segment(
                [frame.left_bottom(), frame.right_bottom()],
                x,
            ));
            shapes.push(egui::Shape::line_segment(
                [frame.left_top(), frame.left_bottom()],
                y,
            ));
            shapes.push(egui::Shape::line_segment(
                [frame.right_top(), frame.right_bottom()],
                y,
            ));
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
