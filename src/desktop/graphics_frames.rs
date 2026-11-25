//! `drawnow` while a script runs: the worker leaves its newest figure
//! snapshot in a one-slot mailbox and the UI shows it on its next frame, so
//! animation loops (`h.YData = ...; drawnow`) update visibly. The run's final
//! reply replaces this preview with the complete state as before.
use super::Workspace;
use egui_plot::PlotPoint;
use matrust::plot::{AxisTransform, Figure};

/// Display transforms and decimated points of an ordinary (non-subplot)
/// figure; a yyaxis figure draws its right side in left-axis coordinates.
#[cfg(test)]
pub(super) fn display_points(figure: &Figure) -> ([AxisTransform; 2], Vec<Vec<PlotPoint>>) {
    let (axes, points, _) = display_points_indexed(figure);
    (axes, points)
}
pub(super) type IndexedDisplayFrame = (
    [AxisTransform; 2],
    Vec<Vec<PlotPoint>>,
    Vec<Option<Vec<usize>>>,
);
/// Same as [`display_points`], also returning original point indices for
/// decimated Scatter series.
pub(super) fn display_points_indexed(figure: &Figure) -> IndexedDisplayFrame {
    let display = figure.yy_display_figure();
    let f = display.as_ref().unwrap_or(figure);
    let projected = matrust::plot::view3d::display_figure(f);
    let f = projected.as_ref();
    let axes = f.axis_transforms();
    let budget = 8192 / f.series.len().max(1);
    let never = std::sync::atomic::AtomicBool::new(false);
    let mut points = Vec::with_capacity(f.series.len());
    let mut indices = Vec::with_capacity(f.series.len());
    for s in &f.series {
        let (pts, idx) = s
            .display_points_axes_indexed_cancellable(budget, axes, &never)
            .expect("Infallible display checkpoint");
        points.push(pts.into_iter().map(|[x, y]| PlotPoint::new(x, y)).collect());
        indices.push(idx);
    }
    (axes, points, indices)
}

/// Whether the docked figure's prepared samples ([`display_points`]) still
/// fit `next`: same sample storage and nothing that moves a drawn sample. A
/// 3-D axes is projected, so its view, z limits, z data, grid and box
/// toggles (projected grid and box series) count too; z data compares by pointer first, then
/// by content.
pub(super) fn same_display_points(shown: &Figure, next: &Figure) -> bool {
    let same_z = |a: &std::sync::Arc<[f64]>, b: &std::sync::Arc<[f64]>| {
        std::sync::Arc::ptr_eq(a, b)
            || (a.len() == b.len()
                && a.iter()
                    .zip(b.iter())
                    .all(|(x, y)| x.to_bits() == y.to_bits()))
    };
    shown.active_position() == next.active_position()
        && shown.layout.is_some() == next.layout.is_some()
        && shown.series.len() == next.series.len()
        && shown.xscale == next.xscale
        && shown.yscale == next.yscale
        && shown.xlim == next.xlim
        && shown.ylim == next.ylim
        && shown.view == next.view
        && shown.zlim == next.zlim
        && shown.zlabel == next.zlabel
        && (!shown.is_3d()
            || ([shown.xgrid, shown.ygrid, shown.zgrid] == [next.xgrid, next.ygrid, next.zgrid]
                && shown.minor_grid == next.minor_grid
                && shown.box_outline == next.box_outline))
        && shown.series.iter().zip(&next.series).all(|(a, b)| {
            a.points.same_storage(&b.points)
                && same_z(&a.z, &b.z)
                && a.kind == b.kind
                && a.visible == b.visible
                && a.bar_width == b.bar_width
                && a.bar_offset == b.bar_offset
        })
}

/// The figure shown before a run's first `drawnow` frame. Frames replace the
/// shown figure, so the final reply decides "new or changed figure" against
/// this one, as a run without frames does against the shown figure.
#[derive(Default)]
pub(super) struct FrameReveal {
    before: Option<(u32, Option<Figure>)>,
    revealed: bool,
}
impl FrameReveal {
    /// Ends the run: the current figure number and figure from before its
    /// first frame, or `None` when no frame arrived.
    pub(super) fn finish(&mut self) -> Option<(u32, Option<Figure>)> {
        self.revealed = false;
        self.before.take()
    }
}

impl Workspace {
    pub(super) fn receive_graphics_frame(&mut self) {
        let Some(frame) = self.worker.take_frame() else {
            return;
        };
        // Frames only preview a running script; a finished run's reply wins.
        if !self.busy {
            return;
        }
        self.ui_windows.set_snapshot(frame.ui);
        self.all_figures = frame.figures.into_iter().collect();
        let shown_current = std::mem::replace(&mut self.current_figure, frame.current);
        if let Some(figure) = &frame.figure
            && figure.layout.is_none()
            && !self.detached.contains(frame.current)
        {
            let (axes, points, indices) = display_points_indexed(figure);
            self.plot_axes = axes;
            self.plot_points = points;
            self.plot_point_indices = indices;
        } else {
            self.plot_points = Vec::new();
            self.plot_point_indices = Vec::new();
        }
        self.view_bounds = None;
        self.reset_plot = true;
        let shown = std::mem::replace(&mut self.figure, frame.figure);
        let (before_current, before) = self
            .frame_reveal
            .before
            .get_or_insert((shown_current, shown));
        // A frame with a new or changed figure reveals it once per run, the
        // same way the reply of a plain plot does.
        let changed = self.figure.as_ref().is_some_and(|figure| {
            *before_current != self.current_figure
                || before.as_ref().is_none_or(|old| !old.same_snapshot(figure))
        });
        if changed
            && !self.frame_reveal.revealed
            && !self
                .worker
                .cancel
                .load(std::sync::atomic::Ordering::Relaxed)
        {
            self.frame_reveal.revealed = true;
            self.detached
                .reveal_tile(self.current_figure, &self.all_figures);
            if self.layout.center.is_some() {
                self.show_plot_pane();
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::super::{figure_view, worker};
    use eframe::egui;
    use std::time::{Duration, Instant};

    fn texts<'a>(shape: &'a egui::Shape, out: &mut Vec<&'a str>) {
        match shape {
            egui::Shape::Text(t) => out.push(t.galley.text()),
            egui::Shape::Vec(v) => v.iter().for_each(|s| texts(s, out)),
            _ => {}
        }
    }

    /// A running script's drawnow frames reach the UI side one by one, and a
    /// frame's figure paints its text, right axis and colorbar natively.
    #[test]
    fn drawnow_frames_arrive_while_the_script_runs_and_paint_overlays() {
        let context = egui::Context::default();
        let mut worker = worker::Worker::new(context.clone());
        worker
            .send
            .send(
                worker::Job::Script(
                    "x=1:20; h=plot(x,x); text(5,5,'MARK'); colorbar; yyaxis right; plot(x,100*x); \
                     for k=1:5; h.YData = x+k; drawnow; pause(0.05); end"
                        .into(),
                )
                .into(),
            )
            .unwrap();
        let start = Instant::now();
        let mut seen = Vec::new();
        let reply = loop {
            if let Some(frame) = worker.take_frame() {
                let figure = frame.figure.unwrap();
                seen.push(figure.axes.series[0].points.get(0).unwrap()[1]);
            }
            if let Ok(reply) = worker.receive.try_recv() {
                break reply;
            }
            assert!(
                start.elapsed() < Duration::from_secs(60),
                "script did not finish"
            );
            std::thread::sleep(Duration::from_millis(5));
        };
        assert!(reply.result.is_ok(), "{:?}", reply.result.err());
        // Frames are previews: several distinct iterations were seen while running.
        seen.dedup();
        assert!(seen.len() >= 3, "frames seen: {seen:?}");
        assert!(seen.windows(2).all(|w| w[0] < w[1]), "{seen:?}");
        let figure = reply.figure.unwrap();
        let (axes, points) = super::display_points(&figure);
        let ctx = egui::Context::default();
        let mut output = None;
        for pass in 0..3 {
            output = Some(crate::headless::run(
                &ctx,
                egui::RawInput {
                    screen_rect: Some(egui::Rect::from_min_size(
                        egui::Pos2::ZERO,
                        egui::vec2(900., 600.),
                    )),
                    ..Default::default()
                },
                |root| {
                    egui::CentralPanel::default().show(root, |ui| {
                        figure_view::show_interactive(ui, &figure, &points, axes, pass == 0, None);
                    });
                },
            ));
        }
        let mut found = Vec::new();
        let output = output.unwrap();
        for s in &output.shapes {
            texts(&s.shape, &mut found);
        }
        assert!(found.contains(&"MARK"), "{found:?}");
        // Right axis ticks of 100..2000 and colorbar ticks of [0 1].
        assert!(found.contains(&"2000"), "{found:?}");
        assert!(found.contains(&"0.6"), "{found:?}");
        worker.shutdown();
    }

    /// The docked figure re-projects its samples after a rotation, a ZData,
    /// zlim or zlabel set and a 3-D grid toggle; an unchanged reply keeps
    /// them, and a 2-D grid toggle still does not rebuild them.
    #[test]
    fn same_display_points_sees_3d_view_zlim_zlabel_and_z_data() {
        use super::same_display_points;
        let mut e = matrust::Engine::default();
        e.execute("h = plot3([0 1 2], [0 1 0], [0 1 2]);").unwrap();
        let mut shown = e.figure.clone().unwrap();
        assert!(same_display_points(&shown, &shown.clone()));
        for change in [
            "view(45, 20);",
            "h.ZData = [2 1 0];",
            "zlim([-1 5]);",
            "zlabel('height');",
            "grid on;",
            "view(2);",
        ] {
            e.execute(change).unwrap();
            let next = e.figure.clone().unwrap();
            assert!(!same_display_points(&shown, &next), "{change}");
            shown = next;
        }
        // Equal z contents in a new allocation are the same samples.
        let mut copy = shown.clone();
        copy.axes.series[0].z = copy.axes.series[0].z.iter().copied().collect();
        assert!(!std::sync::Arc::ptr_eq(
            &copy.axes.series[0].z,
            &shown.axes.series[0].z
        ));
        assert!(same_display_points(&shown, &copy));
        // 2-D: grid lines are not samples.
        let mut e = matrust::Engine::default();
        e.execute("plot([0 1], [1 0]);").unwrap();
        let shown = e.figure.clone().unwrap();
        e.execute("grid on;").unwrap();
        assert!(same_display_points(&shown, e.figure.as_ref().unwrap()));
    }

    /// Rotating a docked 3-D figure (here by `view`) replaces the prepared
    /// samples with the new projection instead of keeping the old one.
    #[test]
    fn docked_3d_figure_reprojects_after_a_view_change() {
        use super::super::Workspace;
        let mut app = Workspace::with_context(egui::Context::default(), true);
        app.run("h = plot3([0 1 2], [0 1 0], [0 1 2]);".into());
        finish(&mut app);
        let before = app.plot_points.clone();
        assert!(!before.is_empty());
        for script in ["view(45, 20);", "h.ZData = [2 1 0];"] {
            app.run(script.into());
            finish(&mut app);
            let (_, expected) = super::display_points(app.figure.as_ref().unwrap());
            assert_eq!(bits(&app.plot_points), bits(&expected), "{script}");
        }
        assert_ne!(bits(&app.plot_points), bits(&before));
    }

    /// Sample bits: projected box lines break at NaN, which `==` never matches.
    fn bits(points: &[Vec<egui_plot::PlotPoint>]) -> Vec<Vec<[u64; 2]>> {
        points
            .iter()
            .map(|line| {
                line.iter()
                    .map(|p| [p.x.to_bits(), p.y.to_bits()])
                    .collect()
            })
            .collect()
    }

    fn finish(app: &mut super::super::Workspace) {
        let deadline = Instant::now() + Duration::from_secs(30);
        while app.busy && Instant::now() < deadline {
            app.receive();
            std::thread::sleep(Duration::from_millis(1));
        }
        assert!(!app.busy, "{}", app.status);
    }

    /// A script that ends in drawnow reveals the figure pane like a plain
    /// plot. The frame used to replace the shown figure, so the final reply
    /// saw the same snapshot and revealed nothing: with a tool tab selected in
    /// the centre layout the figure stayed hidden.
    #[test]
    fn a_script_ending_in_drawnow_reveals_the_figure_pane() {
        use super::super::{Workspace, toolkits::Tool};
        for script in [
            // Control: a plain plot reveals through the reply.
            "plot(1:3);",
            "plot(1:3); drawnow",
            "h = plot(1:3); for k = 1:5; h.YData = [k k k]; drawnow; end",
        ] {
            let mut app = Workspace::with_context(egui::Context::default(), true);
            app.toolkits.open_tool(Tool::Designer);
            assert!(app.layout.center.is_some() && !app.figure_column());
            app.run(script.into());
            finish(&mut app);
            assert!(app.figure.is_some(), "{script}");
            assert_eq!(app.toolkits.selected_tool(), None, "{script}");
            assert_eq!(
                app.layout.mode,
                matrust::session::PaneMode::Figure,
                "{script}"
            );
        }
        // A frame reveals the figure while the script still runs.
        let mut app = Workspace::with_context(egui::Context::default(), true);
        app.toolkits.open_tool(Tool::Designer);
        app.run("plot(1:3); drawnow; pause(1)".into());
        let deadline = Instant::now() + Duration::from_secs(30);
        let mut revealed_while_running = false;
        while app.busy && Instant::now() < deadline {
            app.receive();
            revealed_while_running |= app.busy && app.toolkits.selected_tool().is_none();
            std::thread::sleep(Duration::from_millis(1));
        }
        assert!(!app.busy, "{}", app.status);
        assert!(revealed_while_running);
        assert_eq!(app.toolkits.selected_tool(), None);
    }

    /// Hovering a yyaxis right series shows that series' own value in the
    /// crosshair label, not the left-axis number it is drawn at.
    #[test]
    fn hover_on_a_right_axis_series_shows_its_own_value() {
        let mut engine = matrust::Engine::default();
        engine
            .execute(
                "yyaxis left; plot([1 2 3], [0 5 10]); ylim([-5 15]); \
                 yyaxis right; plot([1 2 3], [1020 1000 1010]); ylim([990 1030]);",
            )
            .unwrap();
        let figure = engine.figure.clone().unwrap();
        let (axes, points) = super::display_points(&figure);
        let ctx = egui::Context::default();
        let screen = egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(900., 600.));
        let frame = |events: Vec<egui::Event>, reset: bool| {
            let output = crate::headless::run(
                &ctx,
                egui::RawInput {
                    screen_rect: Some(screen),
                    events,
                    ..Default::default()
                },
                |root| {
                    egui::CentralPanel::default().show(root, |ui| {
                        figure_view::show_interactive(ui, &figure, &points, axes, reset, None);
                    });
                },
            );
            let mut found = Vec::new();
            for s in &output.shapes {
                texts(&s.shape, &mut found);
            }
            found.into_iter().map(str::to_owned).collect::<Vec<_>>()
        };
        for pass in 0..3 {
            frame(Vec::new(), pass == 0);
        }
        let plot = egui_plot::PlotMemory::load(&ctx, egui::Id::new("workspace_figure")).unwrap();
        // The right series' first point (1, 1020) is drawn at left-axis y 10.
        let at = plot
            .transform()
            .position_from_point(&egui_plot::PlotPoint::new(1., 10.));
        let mut events = vec![egui::Event::PointerMoved(at)];
        let label = (0..30)
            .find_map(|_| {
                frame(std::mem::take(&mut events), false)
                    .into_iter()
                    .find(|t| t.contains("\ny = "))
            })
            .expect("the crosshair label must show over the series point");
        assert!(label.contains("\ny = 1020"), "{label}");
    }
}
