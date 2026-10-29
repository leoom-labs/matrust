use super::*;
use matrust::plot::WeakSeriesPoints;
use matrust::plot::{Axes, PlotKind, Position, SampleHit, SampleQuery};

#[derive(Clone)]
pub struct Request {
    pub figure: u32,
    pub position: Position,
    pub axes: Axes,
    pub query: SampleQuery,
}
pub struct Selection {
    position: Position,
    hit: SampleHit,
    points: WeakSeriesPoints,
}
#[derive(Default)]
pub struct Inspector {
    selected: BTreeMap<u32, Selection>,
    pub pending: Option<Request>,
    notice: Option<(u32, String)>,
    demo_complete: bool,
}
impl Selection {
    fn series<'a>(&self, figure: &'a Figure) -> Option<&'a matrust::plot::Series> {
        let axes = figure.all_axes().find(|(p, _)| *p == self.position)?.1;
        let series = axes.series.get(self.hit.series)?;
        (series.visible && self.points.matches(&series.points)).then_some(series)
    }
}
impl Inspector {
    pub fn clear(&mut self, id: u32) {
        self.selected.remove(&id);
        self.notice = None;
    }
    pub fn description(&self, id: u32, figure: &Figure) -> Option<String> {
        let selected = self.selected.get(&id)?;
        selected.series(figure)?;
        let hit = selected.hit;
        Some(format!(
            "Series\tSample\tX\tY\n{}\t{}\t{}\t{}\n",
            hit.series + 1,
            hit.index + 1,
            hit.point[0],
            hit.point[1]
        ))
    }

    pub fn marker(&self, id: u32, figure: &Figure) -> Option<(Position, [f64; 2])> {
        let selected = self.selected.get(&id)?;
        let series = selected.series(figure)?;
        let mut point = selected.hit.point;
        if series.kind == PlotKind::Bar {
            point[0] += series.bar_offset;
        }
        Some((selected.position, point))
    }
}
impl Workspace {
    pub(super) fn sample_inspection_demo(&mut self) {
        if self.busy
            || self.sample_inspector.demo_complete
            || !std::env::args().any(|a| a == "--sample-inspector-demo")
        {
            return;
        }
        let Some(figure) = &self.figure else {
            return;
        };
        let Some(series) = figure.series.first() else {
            return;
        };
        let Some(point) = series.points.get(series.points.len() / 3) else {
            return;
        };
        let transforms = figure.axis_transforms();
        let (x, y) = figure.bounds();
        let query = SampleQuery {
            transforms,
            bounds: [
                transforms[0].forward(x[0]),
                transforms[0].forward(x[1]),
                transforms[1].forward(y[0]),
                transforms[1].forward(y[1]),
            ],
            pointer: [
                transforms[0].forward(point[0]),
                transforms[1].forward(point[1]),
            ],
            pixels: [800., 600.],
        };
        self.sample_inspector.demo_complete = true;
        self.inspect_plot_sample(figure.active_position(), query);
    }
    pub(super) fn inspect_plot_sample(&mut self, position: Position, query: SampleQuery) {
        if self.busy {
            return;
        }
        let Some(axes) = self.figure.as_ref().and_then(|f| {
            f.all_axes()
                .find(|(p, _)| *p == position)
                .map(|(_, a)| a.clone())
        }) else {
            return;
        };
        let request = Request {
            figure: self.current_figure,
            position,
            axes,
            query,
        };
        self.sample_inspector.notice = None;
        self.sample_inspector.pending = Some(request.clone());
        self.submit(Job::PickSample(Box::new(request)));
        if !self.busy {
            self.sample_inspector.pending = None;
        }
    }
    pub(super) fn complete_sample_pick(&mut self, hit: Option<SampleHit>, error: Option<String>) {
        self.sample_inspector.selected.retain(|id, selected| {
            self.all_figures
                .get(id)
                .is_some_and(|f| selected.series(f).is_some())
        });
        let Some(request) = self.sample_inspector.pending.take() else {
            return;
        };
        if let Some(error) = error {
            self.sample_inspector.notice = Some((request.figure, error));
        } else if let Some(hit) = hit {
            let selection = Selection {
                position: request.position,
                hit,
                points: request.axes.series[hit.series].points.downgrade(),
            };
            if self
                .all_figures
                .get(&request.figure)
                .is_some_and(|f| selection.series(f).is_some())
            {
                self.sample_inspector
                    .selected
                    .insert(request.figure, selection);
            }
        } else {
            self.sample_inspector.selected.remove(&request.figure);
            self.sample_inspector.notice = Some((
                request.figure,
                "No visible finite samples in this view".into(),
            ));
        }
    }
    pub(super) fn sample_inspector_ui(&mut self, ui: &mut egui::Ui) {
        let Some(figure) = &self.figure else {
            return;
        };
        let selected = self
            .sample_inspector
            .selected
            .get(&self.current_figure)
            .and_then(|s| s.series(figure).map(|series| (s, series)));
        let mut clear = false;
        if let Some((selection, series)) = selected {
            let hit = selection.hit;
            let mut positions: Vec<_> = figure.all_axes().map(|(p, _)| p).collect();
            positions.sort();
            let axes = positions
                .iter()
                .position(|p| *p == selection.position)
                .unwrap()
                + 1;
            let mut open = true;
            let screen = ui.ctx().content_rect();
            egui::Window::new("Sample inspection")
                .id(egui::Id::new("sample_inspection"))
                .default_width(340.)
                .default_pos(egui::pos2(
                    screen.left() + 16.,
                    (screen.bottom() - 180.).max(70.),
                ))
                .resizable(false)
                .open(&mut open)
                .show(ui.ctx(), |ui| {
                    ui.label(format!(
                        "Axes {axes} · Series {} · Sample {}",
                        hit.series + 1,
                        hit.index + 1
                    ))
                    .on_hover_text(&series.label);
                    let value = |v: f64| {
                        if v != 0. && (v.abs() >= 1e6 || v.abs() < 1e-4) {
                            format!("{v:e}")
                        } else {
                            v.to_string()
                        }
                    };
                    ui.monospace(format!("X: {}", value(hit.point[0])));
                    ui.monospace(format!("Y: {}", value(hit.point[1])));
                    ui.horizontal(|ui| {
                        if ui.small_button("Copy sample").clicked() {
                            ui.ctx().copy_text(format!(
                                "Series\tSample\tX\tY\n{}\t{}\t{}\t{}\n",
                                hit.series + 1,
                                hit.index + 1,
                                hit.point[0],
                                hit.point[1]
                            ));
                        }
                        clear = ui.small_button("Clear sample").clicked();
                    });
                });
            clear |= !open;
        }
        if let Some((id, notice)) = &self.sample_inspector.notice
            && *id == self.current_figure
        {
            ui.weak(notice);
        }
        if clear {
            self.sample_inspector.selected.remove(&self.current_figure);
            self.sample_inspector.notice = None;
        }
    }
}
