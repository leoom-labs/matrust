//! Interactive 2-D CSG drawing canvas for PDE Modeler (8c2).
//! Supports Rectangle, Circle, Ellipse, Polygon and Select tools, snap-to-grid,
//! wheel zoom, middle/right pan, shape outlines/labels, and decomposed edge
//! picking.
use crate::tokens;
use eframe::egui;
use matrust::{
    Matrix,
    engine::pde_app_model::{PdeAppModel, Shape},
};
#[cfg(test)]
use std::collections::BTreeMap;
use std::f64::consts::{FRAC_1_SQRT_2, FRAC_PI_2, TAU};

/// Active drawing / interaction tool on the PDE canvas.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) enum DrawTool {
    #[default]
    Rectangle,
    Circle,
    Ellipse,
    Polygon,
    Select,
}

impl DrawTool {
    pub const ALL: [Self; 5] = [
        Self::Rectangle,
        Self::Circle,
        Self::Ellipse,
        Self::Polygon,
        Self::Select,
    ];

    pub fn label(self) -> &'static str {
        match self {
            Self::Rectangle => "Rectangle",
            Self::Circle => "Circle",
            Self::Ellipse => "Ellipse",
            Self::Polygon => "Polygon",
            Self::Select => "Select",
        }
    }
}

/// One decomposed boundary segment decoded from `decsg` output `dl`.
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct DecomposedEdge {
    pub id: usize,
    pub points: Vec<[f64; 2]>,
    pub label_pos: [f64; 2],
    /// Region 0 (outside) on one side (`dl` rows 6-7): an outer or hole
    /// boundary that takes a boundary condition. Interior (subdomain) edges
    /// are drawn but never take a condition or a click.
    pub boundary: bool,
}

/// Isotropic 2-D data <-> screen coordinate transform for the drawing canvas.
#[derive(Clone, Debug)]
pub(crate) struct CanvasView {
    pub center: [f64; 2],
    /// Half-span of the limiting canvas dimension in data units.
    pub half_extent: f64,
    /// Last allocated canvas rectangle in screen points.
    pub rect: egui::Rect,
}

impl Default for CanvasView {
    fn default() -> Self {
        Self {
            center: [0.0, 0.0],
            half_extent: 1.5,
            rect: egui::Rect::from_min_size(egui::pos2(0.0, 0.0), egui::vec2(600.0, 600.0)),
        }
    }
}

impl CanvasView {
    /// Pixels per data unit (isotropic).
    pub fn scale(&self) -> f64 {
        let min_dim = (self.rect.width().min(self.rect.height()) as f64).max(1.0);
        min_dim / (2.0 * self.half_extent.max(1e-9))
    }

    pub fn data_to_screen_f64(&self, p: [f64; 2]) -> [f64; 2] {
        let cx = self.rect.center().x as f64;
        let cy = self.rect.center().y as f64;
        let s = self.scale();
        [
            cx + (p[0] - self.center[0]) * s,
            cy - (p[1] - self.center[1]) * s,
        ]
    }

    pub fn screen_to_data_f64(&self, p: [f64; 2]) -> [f64; 2] {
        let cx = self.rect.center().x as f64;
        let cy = self.rect.center().y as f64;
        let s = self.scale();
        [
            self.center[0] + (p[0] - cx) / s,
            self.center[1] - (p[1] - cy) / s,
        ]
    }

    pub fn data_to_screen(&self, p: [f64; 2]) -> egui::Pos2 {
        let q = self.data_to_screen_f64(p);
        egui::pos2(q[0] as f32, q[1] as f32)
    }

    pub fn screen_to_data(&self, pos: egui::Pos2) -> [f64; 2] {
        self.screen_to_data_f64([pos.x as f64, pos.y as f64])
    }

    /// Visible data-space bounds `[x_min, x_max, y_min, y_max]`.
    pub fn data_bounds(&self) -> [f64; 4] {
        let tl = self.screen_to_data_f64([self.rect.left() as f64, self.rect.top() as f64]);
        let br = self.screen_to_data_f64([self.rect.right() as f64, self.rect.bottom() as f64]);
        [
            tl[0].min(br[0]),
            tl[0].max(br[0]),
            tl[1].min(br[1]),
            tl[1].max(br[1]),
        ]
    }

    /// Auto-fit the view to the bounding box of `shapes` with a 25% margin,
    /// or reset to `[-1.5, 1.5]^2` when empty.
    pub fn fit_shapes(&mut self, shapes: &[Shape]) {
        let Some([min_x, max_x, min_y, max_y]) = shapes_bbox(shapes) else {
            self.center = [0.0, 0.0];
            self.half_extent = 1.5;
            return;
        };
        self.center = [(min_x + max_x) * 0.5, (min_y + max_y) * 0.5];
        let span = (max_x - min_x).max(max_y - min_y);
        self.half_extent = (span * 0.5 * 1.25).max(0.5);
    }

    /// Refit the view if any shape extends outside the current visible bounds.
    pub fn refit_if_outside(&mut self, shapes: &[Shape]) {
        let Some([min_x, max_x, min_y, max_y]) = shapes_bbox(shapes) else {
            return;
        };
        let [vx0, vx1, vy0, vy1] = self.data_bounds();
        if min_x < vx0 || max_x > vx1 || min_y < vy0 || max_y > vy1 {
            self.fit_shapes(shapes);
        }
    }
}

/// Interactive state of the drawing canvas.
pub(crate) struct CanvasState {
    pub tool: DrawTool,
    pub snap: bool,
    pub snap_step: f64,
    pub view: CanvasView,
    pub selected_shape: Option<usize>,
    pub selected_edge: Option<usize>,
    pub info_message: Option<String>,
    pub decomposed_edges: Vec<DecomposedEdge>,
    drag_start_data: Option<[f64; 2]>,
    drag_current_data: Option<[f64; 2]>,
    pub poly_draft: Vec<[f64; 2]>,
    move_orig_shape: Option<(usize, Shape)>,
    move_start_data: Option<[f64; 2]>,
    pan_start_center: Option<[f64; 2]>,
    pan_start_screen: Option<egui::Pos2>,
    #[cfg(test)]
    pub last_shape_labels: Vec<(String, egui::Rect)>,
    #[cfg(test)]
    pub last_drawn_edge_labels: Vec<usize>,
}

impl Default for CanvasState {
    fn default() -> Self {
        Self {
            tool: DrawTool::Rectangle,
            snap: true,
            snap_step: 0.1,
            view: CanvasView::default(),
            selected_shape: None,
            selected_edge: None,
            info_message: None,
            decomposed_edges: Vec::new(),
            drag_start_data: None,
            drag_current_data: None,
            poly_draft: Vec::new(),
            move_orig_shape: None,
            move_start_data: None,
            pan_start_center: None,
            pan_start_screen: None,
            #[cfg(test)]
            last_shape_labels: Vec::new(),
            #[cfg(test)]
            last_drawn_edge_labels: Vec::new(),
        }
    }
}

/// Events reported back to `PdeModeler` after one frame of canvas interaction.
#[derive(Default)]
pub(crate) struct CanvasOutcome {
    pub geometry_changed: bool,
    pub edge_clicked: Option<usize>,
}

impl CanvasState {
    pub fn cancel_drafts(&mut self) {
        self.drag_start_data = None;
        self.drag_current_data = None;
        self.poly_draft.clear();
        self.move_orig_shape = None;
        self.move_start_data = None;
    }

    fn maybe_snap(&self, p: [f64; 2]) -> [f64; 2] {
        if self.snap {
            [
                snap_value(p[0], self.snap_step),
                snap_value(p[1], self.snap_step),
            ]
        } else {
            p
        }
    }

    pub fn show(
        &mut self,
        ui: &mut egui::Ui,
        model: &mut PdeAppModel,
        #[cfg(test)] controls: &mut BTreeMap<String, egui::Rect>,
    ) -> CanvasOutcome {
        let mut outcome = CanvasOutcome::default();
        let p = tokens::palette(ui.ctx());

        // Toolbar row: tool picker + snap toggle + fit button.
        ui.horizontal_wrapped(|ui| {
            ui.spacing_mut().item_spacing.x = tokens::XS;
            for tool in DrawTool::ALL {
                let resp = tokens::toggle(ui, self.tool == tool, tool.label());
                #[cfg(test)]
                controls.insert(format!("Tool:{}", tool.label()), resp.rect);
                if resp.clicked() && self.tool != tool {
                    self.tool = tool;
                    self.cancel_drafts();
                }
            }
            ui.add_space(tokens::S);
            let snap_resp = tokens::toggle(ui, self.snap, "Snap");
            #[cfg(test)]
            controls.insert("Canvas:Snap".into(), snap_resp.rect);
            if snap_resp.clicked() {
                self.snap = !self.snap;
            }
            let fit_resp =
                ui.add(egui::Button::new("Fit").min_size(egui::vec2(0.0, tokens::CONTROL)));
            #[cfg(test)]
            controls.insert("Canvas:Fit".into(), fit_resp.rect);
            if fit_resp.clicked() {
                self.view.fit_shapes(&model.shapes);
            }
        });

        ui.add_space(tokens::S);

        // Reserve space for the 1-line info / coordinate footer.
        let footer_h = tokens::ROW;
        let avail = ui.available_size();
        let canvas_size = egui::vec2(
            avail.x.max(120.0),
            (avail.y - footer_h - tokens::XS).max(160.0),
        );
        let (rect, response) = ui.allocate_exact_size(canvas_size, egui::Sense::click_and_drag());
        self.view.rect = rect;
        #[cfg(test)]
        controls.insert("Canvas".into(), rect);

        // Keyboard shortcuts (Esc cancels active drafts; Delete removes selected shape).
        let wants_kb = ui.ctx().egui_wants_keyboard_input();
        if ui.input(|i| i.key_pressed(egui::Key::Escape)) {
            if !self.poly_draft.is_empty() {
                self.poly_draft.clear();
                self.info_message = Some("Polygon drawing cancelled.".into());
            } else if self.drag_start_data.is_some() || self.move_orig_shape.is_some() {
                if let Some((idx, orig)) = self.move_orig_shape.take()
                    && idx < model.shapes.len()
                {
                    model.shapes[idx] = orig;
                }
                self.drag_start_data = None;
                self.drag_current_data = None;
                self.move_start_data = None;
                self.info_message = Some("Action cancelled.".into());
            }
        }
        if !wants_kb
            && ui.input(|i| i.key_pressed(egui::Key::Delete))
            && let Some(idx) = self.selected_shape
            && idx < model.shapes.len()
        {
            model.shapes.remove(idx);
            self.selected_shape = None;
            self.move_orig_shape = None;
            self.info_message = None;
            outcome.geometry_changed = true;
        }

        // Wheel zoom around pointer position.
        if response.hovered() {
            let scroll_y = ui.input(|i| i.smooth_scroll_delta.y);
            if scroll_y.abs() > 0.0 {
                let factor = (-scroll_y as f64 * 0.002).exp();
                let anchor_screen = ui
                    .input(|i| i.pointer.hover_pos())
                    .unwrap_or_else(|| rect.center());
                let anchor_data = self.view.screen_to_data(anchor_screen);
                let new_extent = (self.view.half_extent * factor).clamp(1e-3, 1e6);
                self.view.half_extent = new_extent;
                let after = self.view.screen_to_data(anchor_screen);
                self.view.center[0] += anchor_data[0] - after[0];
                self.view.center[1] += anchor_data[1] - after[1];
            }
        }

        // Pan with middle or right mouse drag.
        if response.drag_started_by(egui::PointerButton::Middle)
            || response.drag_started_by(egui::PointerButton::Secondary)
        {
            self.pan_start_center = Some(self.view.center);
            self.pan_start_screen =
                ui.input(|i| i.pointer.press_origin().or(i.pointer.interact_pos()));
        }
        if (response.dragged_by(egui::PointerButton::Middle)
            || response.dragged_by(egui::PointerButton::Secondary))
            && let (Some(orig_center), Some(start_scr), Some(cur_scr)) = (
                self.pan_start_center,
                self.pan_start_screen,
                ui.input(|i| i.pointer.interact_pos()),
            )
        {
            let s = self.view.scale();
            let dx = (cur_scr.x - start_scr.x) as f64 / s;
            let dy = (cur_scr.y - start_scr.y) as f64 / s;
            self.view.center = [orig_center[0] - dx, orig_center[1] + dy];
        }
        if response.drag_stopped_by(egui::PointerButton::Middle)
            || response.drag_stopped_by(egui::PointerButton::Secondary)
        {
            self.pan_start_center = None;
            self.pan_start_screen = None;
        }

        // Primary pointer tool interactions.
        self.handle_primary_pointer(ui, &response, model, &mut outcome);

        // Paint canvas.
        let hover_pos = response.hover_pos();
        let painter = ui.painter_at(rect);
        painter.rect_filled(rect, 0.0, p.surface);
        self.paint_grid(&painter, p);
        self.paint_shapes(&painter, model, p);
        self.paint_draft(&painter, ui, p);
        self.paint_decomposed_edges(&painter, hover_pos, p);
        painter.rect_stroke(
            rect,
            0.0,
            egui::Stroke::new(1.0, p.border),
            egui::StrokeKind::Inside,
        );

        ui.add_space(tokens::XS);

        // Footer info line: error/refusal message, or cursor coordinates & tool hint.
        ui.horizontal(|ui| {
            if let Some(msg) = &self.info_message {
                ui.label(
                    egui::RichText::new(msg)
                        .size(tokens::SMALL_CODE)
                        .color(p.text_muted),
                );
            } else if let Some(pos) = hover_pos {
                let d = self.maybe_snap(self.view.screen_to_data(pos));
                ui.label(
                    egui::RichText::new(format!("x = {:.3}, y = {:.3}", d[0], d[1]))
                        .size(tokens::SMALL_CODE)
                        .color(p.text_muted),
                );
            } else {
                let hint = match self.tool {
                    DrawTool::Rectangle => "Drag corner to corner to draw a rectangle.",
                    DrawTool::Circle => "Drag centre to radius to draw a circle.",
                    DrawTool::Ellipse => "Drag corner to corner to draw an ellipse.",
                    DrawTool::Polygon => {
                        "Click vertices; click first vertex or double-click to close; Esc cancels."
                    }
                    DrawTool::Select => {
                        "Click shape or edge to select; drag shape to move; Delete removes."
                    }
                };
                ui.label(
                    egui::RichText::new(hint)
                        .size(tokens::SMALL_CODE)
                        .color(p.text_muted),
                );
            }
        });

        outcome
    }

    fn handle_primary_pointer(
        &mut self,
        ui: &egui::Ui,
        response: &egui::Response,
        model: &mut PdeAppModel,
        outcome: &mut CanvasOutcome,
    ) {
        let press_origin = ui.input(|i| i.pointer.press_origin());
        let interact_pos = ui.input(|i| i.pointer.interact_pos());

        // Clicking a decomposed edge selects its BC row regardless of active tool
        // (unless actively placing polygon vertices).
        if response.clicked_by(egui::PointerButton::Primary)
            && self.poly_draft.is_empty()
            && !self.decomposed_edges.is_empty()
            && let Some(pos) = interact_pos
            && let Some(eid) = hit_test_edge(&self.decomposed_edges, pos, &self.view, 10.0)
        {
            self.drag_start_data = None;
            self.drag_current_data = None;
            self.selected_edge = Some(eid);
            self.info_message = None;
            outcome.edge_clicked = Some(eid);
            return;
        }

        match self.tool {
            DrawTool::Rectangle | DrawTool::Circle | DrawTool::Ellipse => {
                if response.drag_started_by(egui::PointerButton::Primary)
                    && let Some(pos) = press_origin.or(interact_pos)
                {
                    let d = self.maybe_snap(self.view.screen_to_data(pos));
                    self.drag_start_data = Some(d);
                    self.drag_current_data = Some(d);
                    self.info_message = None;
                }
                if response.dragged_by(egui::PointerButton::Primary)
                    && let Some(pos) = interact_pos
                {
                    self.drag_current_data = Some(self.maybe_snap(self.view.screen_to_data(pos)));
                }
                if response.drag_stopped_by(egui::PointerButton::Primary) {
                    let end = interact_pos
                        .map(|p| self.maybe_snap(self.view.screen_to_data(p)))
                        .or_else(|| self.drag_current_data.take());
                    self.drag_current_data = None;
                    if let (Some(a), Some(b)) = (self.drag_start_data.take(), end) {
                        self.finish_drag_shape(a, b, model, outcome);
                    }
                } else if response.clicked_by(egui::PointerButton::Primary) {
                    // A click without movement is a zero-size drag.
                    self.drag_start_data = None;
                    self.drag_current_data = None;
                    let kind = self.tool.label().to_lowercase();
                    self.info_message =
                        Some(format!("Refused degenerate {kind}: drag a nonzero size."));
                }
            }
            DrawTool::Polygon => {
                if response.double_clicked_by(egui::PointerButton::Primary) {
                    self.try_close_polygon(model, outcome);
                } else if response.clicked_by(egui::PointerButton::Primary)
                    && let Some(pos) = interact_pos
                {
                    let pt = self.maybe_snap(self.view.screen_to_data(pos));
                    if let Some(&first) = self.poly_draft.first() {
                        let first_scr = self.view.data_to_screen(first);
                        let near_first = first_scr.distance(pos) <= 10.0
                            || ((pt[0] - first[0]).abs() <= 1e-9
                                && (pt[1] - first[1]).abs() <= 1e-9);
                        if near_first {
                            self.try_close_polygon(model, outcome);
                            return;
                        }
                    }
                    if self.poly_draft.last().is_none_or(|last| {
                        (last[0] - pt[0]).abs() > 1e-9 || (last[1] - pt[1]).abs() > 1e-9
                    }) {
                        self.poly_draft.push(pt);
                        self.info_message = None;
                    }
                }
            }
            DrawTool::Select => {
                if response.drag_started_by(egui::PointerButton::Primary)
                    && let Some(pos) = press_origin.or(interact_pos)
                {
                    let raw_data = self.view.screen_to_data(pos);
                    let tol = 6.0 / self.view.scale();
                    let hit = self
                        .selected_shape
                        .filter(|&idx| {
                            model
                                .shapes
                                .get(idx)
                                .is_some_and(|s| hit_test_shape(s, raw_data, tol))
                        })
                        .or_else(|| pick_shape(&model.shapes, raw_data, tol));
                    if let Some(idx) = hit {
                        self.selected_shape = Some(idx);
                        self.move_orig_shape = Some((idx, model.shapes[idx].clone()));
                        self.move_start_data = Some(self.maybe_snap(raw_data));
                        self.info_message = None;
                    }
                }
                if response.dragged_by(egui::PointerButton::Primary)
                    && let (Some((idx, orig)), Some(start), Some(pos)) =
                        (&self.move_orig_shape, self.move_start_data, interact_pos)
                {
                    let cur = self.maybe_snap(self.view.screen_to_data(pos));
                    let dx = cur[0] - start[0];
                    let dy = cur[1] - start[1];
                    if *idx < model.shapes.len() {
                        model.shapes[*idx] = translate_shape(orig, dx, dy);
                    }
                }
                if response.drag_stopped_by(egui::PointerButton::Primary) {
                    if let Some((idx, orig)) = self.move_orig_shape.take() {
                        if let (Some(start), Some(pos)) = (self.move_start_data, interact_pos)
                            && idx < model.shapes.len()
                        {
                            let cur = self.maybe_snap(self.view.screen_to_data(pos));
                            let dx = cur[0] - start[0];
                            let dy = cur[1] - start[1];
                            model.shapes[idx] = translate_shape(&orig, dx, dy);
                        }
                        if idx < model.shapes.len() && model.shapes[idx] != orig {
                            outcome.geometry_changed = true;
                        }
                    }
                    self.move_start_data = None;
                }
                if response.clicked_by(egui::PointerButton::Primary)
                    && let Some(pos) = interact_pos
                {
                    // Prefer decomposed edge picking when clicking directly on an edge.
                    if let Some(eid) = hit_test_edge(&self.decomposed_edges, pos, &self.view, 10.0)
                    {
                        self.selected_edge = Some(eid);
                        outcome.edge_clicked = Some(eid);
                    } else {
                        let raw_data = self.view.screen_to_data(pos);
                        let tol = 6.0 / self.view.scale();
                        self.selected_shape = pick_shape(&model.shapes, raw_data, tol);
                    }
                }
            }
        }
    }

    fn finish_drag_shape(
        &mut self,
        a: [f64; 2],
        b: [f64; 2],
        model: &mut PdeAppModel,
        outcome: &mut CanvasOutcome,
    ) {
        match self.tool {
            DrawTool::Rectangle => {
                let x0 = a[0].min(b[0]);
                let y0 = a[1].min(b[1]);
                let w = (b[0] - a[0]).abs();
                let h = (b[1] - a[1]).abs();
                if w <= 1e-9 || h <= 1e-9 {
                    self.info_message = Some(
                        "Refused degenerate rectangle: width and height must be positive.".into(),
                    );
                    return;
                }
                model.add_rectangle(x0, y0, w, h);
                self.selected_shape = Some(model.shapes.len() - 1);
                self.info_message = None;
                outcome.geometry_changed = true;
            }
            DrawTool::Circle => {
                let r = (b[0] - a[0]).hypot(b[1] - a[1]);
                if r <= 1e-9 {
                    self.info_message =
                        Some("Refused degenerate circle: radius must be positive.".into());
                    return;
                }
                model.add_circle(a[0], a[1], r);
                self.selected_shape = Some(model.shapes.len() - 1);
                self.info_message = None;
                outcome.geometry_changed = true;
            }
            DrawTool::Ellipse => {
                let cx = 0.5 * (a[0] + b[0]);
                let cy = 0.5 * (a[1] + b[1]);
                let rx = 0.5 * (b[0] - a[0]).abs();
                let ry = 0.5 * (b[1] - a[1]).abs();
                if rx <= 1e-9 || ry <= 1e-9 {
                    self.info_message =
                        Some("Refused degenerate ellipse: semi-axes must be positive.".into());
                    return;
                }
                model.add_ellipse(cx, cy, rx, ry, 0.0);
                self.selected_shape = Some(model.shapes.len() - 1);
                self.info_message = None;
                outcome.geometry_changed = true;
            }
            DrawTool::Polygon | DrawTool::Select => {}
        }
    }

    fn try_close_polygon(&mut self, model: &mut PdeAppModel, outcome: &mut CanvasOutcome) {
        let pts = std::mem::take(&mut self.poly_draft);
        match validate_polygon(&pts) {
            Ok(()) => {
                model.add_polygon(pts);
                self.selected_shape = Some(model.shapes.len() - 1);
                self.info_message = None;
                outcome.geometry_changed = true;
            }
            Err(msg) => {
                self.info_message = Some(msg);
            }
        }
    }

    fn paint_grid(&self, painter: &egui::Painter, p: &tokens::Palette) {
        let [x_min, x_max, y_min, y_max] = self.view.data_bounds();
        let step = nice_step((x_max - x_min).max(y_max - y_min) / 6.0);
        let grid_stroke = egui::Stroke::new(1.0, p.hairline);
        let axis_stroke = egui::Stroke::new(1.0, p.border);
        let font = egui::FontId::proportional(tokens::CAPTION);

        let mut x = (x_min / step).floor() * step;
        let mut guard = 0;
        while x <= x_max + step * 0.5 && guard < 200 {
            guard += 1;
            let sx = self.view.data_to_screen([x, 0.0]).x;
            let stroke = if x.abs() < step * 1e-4 {
                axis_stroke
            } else {
                grid_stroke
            };
            painter.vline(sx, self.view.rect.y_range(), stroke);
            let label = fmt_tick(x);
            painter.text(
                egui::pos2(sx + 2.0, self.view.rect.bottom() - 2.0),
                egui::Align2::LEFT_BOTTOM,
                label,
                font.clone(),
                p.text_muted,
            );
            x += step;
        }

        let mut y = (y_min / step).floor() * step;
        guard = 0;
        while y <= y_max + step * 0.5 && guard < 200 {
            guard += 1;
            let sy = self.view.data_to_screen([0.0, y]).y;
            let stroke = if y.abs() < step * 1e-4 {
                axis_stroke
            } else {
                grid_stroke
            };
            painter.hline(self.view.rect.x_range(), sy, stroke);
            let label = fmt_tick(y);
            painter.text(
                egui::pos2(self.view.rect.left() + 4.0, sy - 2.0),
                egui::Align2::LEFT_BOTTOM,
                label,
                font.clone(),
                p.text_muted,
            );
            y += step;
        }
    }

    fn paint_shapes(&mut self, painter: &egui::Painter, model: &PdeAppModel, p: &tokens::Palette) {
        let fill_selected =
            egui::Color32::from_rgba_unmultiplied(p.accent.r(), p.accent.g(), p.accent.b(), 36);
        let font = egui::FontId::proportional(tokens::BODY);
        let origin = self.view.data_to_screen([0.0, 0.0]);
        let mut placed_labels: Vec<egui::Rect> = Vec::new();
        #[cfg(test)]
        self.last_shape_labels.clear();

        for (i, shape) in model.shapes.iter().enumerate() {
            let selected = self.selected_shape == Some(i);
            let stroke = egui::Stroke::new(if selected { 2.2 } else { 1.5 }, p.accent);
            let fill = if selected {
                fill_selected
            } else {
                egui::Color32::TRANSPARENT
            };
            let pts = sample_shape_outline(shape);
            if pts.len() >= 3 {
                let scr: Vec<egui::Pos2> =
                    pts.iter().map(|&pt| self.view.data_to_screen(pt)).collect();
                if selected {
                    match shape {
                        // convex_polygon fills the hull and spills into an L-notch.
                        Shape::Polygon { pts, .. } => {
                            paint_polygon_fill(painter, &self.view, pts, fill);
                        }
                        Shape::Rectangle { .. } | Shape::Circle { .. } | Shape::Ellipse { .. } => {
                            painter.add(egui::Shape::convex_polygon(
                                scr.clone(),
                                fill,
                                egui::Stroke::NONE,
                            ));
                        }
                    }
                }
                painter.add(egui::Shape::closed_line(scr, stroke));
            }
            let anchor = shape_label_anchor(shape);
            let tl = self.view.data_to_screen(anchor);
            let galley = painter.layout_no_wrap(shape.name().to_owned(), font.clone(), p.text);
            let size = galley.size();
            let mut label_rect =
                egui::Rect::from_min_size(egui::pos2(tl.x + tokens::XS, tl.y + tokens::XS), size);
            let step_y = size.y.max(tokens::BODY);
            for _ in 0..32 {
                let hits_origin = label_rect.expand(2.0).contains(origin);
                let hits_placed = placed_labels.iter().any(|r| r.intersects(label_rect));
                if !hits_origin && !hits_placed {
                    break;
                }
                let dy = if hits_origin {
                    (origin.y + tokens::XS - label_rect.top()).max(step_y)
                } else {
                    step_y
                };
                label_rect = label_rect.translate(egui::vec2(0.0, dy));
            }
            painter.galley(label_rect.min, galley, p.text);
            placed_labels.push(label_rect);
            #[cfg(test)]
            self.last_shape_labels
                .push((shape.name().to_owned(), label_rect));
        }
    }

    fn paint_draft(&self, painter: &egui::Painter, ui: &egui::Ui, p: &tokens::Palette) {
        let stroke = egui::Stroke::new(1.5, p.accent);
        if let (Some(a), Some(b)) = (self.drag_start_data, self.drag_current_data) {
            match self.tool {
                DrawTool::Rectangle => {
                    let p0 = self.view.data_to_screen([a[0].min(b[0]), a[1].max(b[1])]);
                    let p1 = self.view.data_to_screen([a[0].max(b[0]), a[1].min(b[1])]);
                    painter.rect_stroke(
                        egui::Rect::from_two_pos(p0, p1),
                        0.0,
                        stroke,
                        egui::StrokeKind::Middle,
                    );
                }
                DrawTool::Circle => {
                    let r = (b[0] - a[0]).hypot(b[1] - a[1]);
                    let center = self.view.data_to_screen(a);
                    let r_px = (r * self.view.scale()) as f32;
                    painter.circle_stroke(center, r_px, stroke);
                }
                DrawTool::Ellipse => {
                    let cx = 0.5 * (a[0] + b[0]);
                    let cy = 0.5 * (a[1] + b[1]);
                    let rx = 0.5 * (b[0] - a[0]).abs();
                    let ry = 0.5 * (b[1] - a[1]).abs();
                    let tmp = Shape::Ellipse {
                        name: String::new(),
                        cx,
                        cy,
                        a: rx,
                        b: ry,
                        angle: 0.0,
                    };
                    let scr: Vec<egui::Pos2> = sample_shape_outline(&tmp)
                        .into_iter()
                        .map(|pt| self.view.data_to_screen(pt))
                        .collect();
                    if scr.len() >= 3 {
                        painter.add(egui::Shape::closed_line(scr, stroke));
                    }
                }
                DrawTool::Polygon | DrawTool::Select => {}
            }
        }

        if !self.poly_draft.is_empty() {
            let mut scr: Vec<egui::Pos2> = self
                .poly_draft
                .iter()
                .map(|&pt| self.view.data_to_screen(pt))
                .collect();
            for &pt in &scr {
                painter.circle_filled(pt, 3.5, p.accent);
            }
            if let Some(hover) = ui.input(|i| i.pointer.hover_pos())
                && self.view.rect.contains(hover)
            {
                let d = self.maybe_snap(self.view.screen_to_data(hover));
                scr.push(self.view.data_to_screen(d));
            }
            if scr.len() >= 2 {
                painter.add(egui::Shape::line(scr, stroke));
            }
        }
    }

    fn paint_decomposed_edges(
        &mut self,
        painter: &egui::Painter,
        hover_pos: Option<egui::Pos2>,
        p: &tokens::Palette,
    ) {
        #[cfg(test)]
        self.last_drawn_edge_labels.clear();
        if self.decomposed_edges.is_empty() {
            return;
        }
        let font = egui::FontId::proportional(tokens::CAPTION);
        let paint_badge = |edge: &DecomposedEdge, pos: egui::Pos2| {
            let selected = self.selected_edge == Some(edge.id);
            let label = format!("E{}", edge.id);
            let bg = if selected { p.accent_soft } else { p.surface };
            let fg = if selected { p.accent_text } else { p.text };
            let badge = egui::Rect::from_center_size(pos, egui::vec2(tokens::XL, tokens::L));
            painter.rect_filled(badge, tokens::CONTROL_RADIUS as f32, bg);
            painter.rect_stroke(
                badge,
                tokens::CONTROL_RADIUS as f32,
                egui::Stroke::new(1.0, if selected { p.accent } else { p.border }),
                egui::StrokeKind::Inside,
            );
            painter.text(pos, egui::Align2::CENTER_CENTER, label, font.clone(), fg);
        };

        let mut drawn_centers: Vec<egui::Pos2> = Vec::new();
        let mut skipped_indices: Vec<usize> = Vec::new();
        for (idx, edge) in self.decomposed_edges.iter().enumerate() {
            let selected = self.selected_edge == Some(edge.id);
            let stroke = if selected {
                egui::Stroke::new(3.0, p.accent)
            } else if !edge.boundary {
                egui::Stroke::new(1.0, p.hairline)
            } else {
                egui::Stroke::new(1.5, p.text_muted)
            };
            let scr: Vec<egui::Pos2> = edge
                .points
                .iter()
                .map(|&pt| self.view.data_to_screen(pt))
                .collect();
            if scr.len() >= 2 {
                painter.add(egui::Shape::line(scr, stroke));
            }
            if !edge.boundary {
                continue;
            }
            let pos = self.view.data_to_screen(edge.label_pos);
            if drawn_centers.iter().any(|&c| c.distance(pos) < 12.0) {
                skipped_indices.push(idx);
            } else {
                paint_badge(edge, pos);
                drawn_centers.push(pos);
                #[cfg(test)]
                self.last_drawn_edge_labels.push(edge.id);
            }
        }

        // Show a skipped edge label when the pointer hovers near that edge.
        if let Some(hover) = hover_pos
            && !skipped_indices.is_empty()
        {
            let mut best: Option<(usize, f32)> = None;
            for idx in skipped_indices {
                let edge = &self.decomposed_edges[idx];
                let d = edge_screen_distance(edge, hover, &self.view);
                if d <= 12.0 && best.is_none_or(|(_, bd)| d < bd) {
                    best = Some((idx, d));
                }
            }
            if let Some((idx, _)) = best {
                let edge = &self.decomposed_edges[idx];
                let pos = self.view.data_to_screen(edge.label_pos);
                paint_badge(edge, pos);
                #[cfg(test)]
                self.last_drawn_edge_labels.push(edge.id);
            }
        }
    }
}

// ---------------------------------------------------------------------------
// Pure geometry helpers (snap, validation, hit-testing, dl decoding)

pub(crate) fn snap_value(v: f64, step: f64) -> f64 {
    if !step.is_finite() || step <= 0.0 {
        return v;
    }
    let inv = (1.0 / step).round();
    let snapped = if (inv * step - 1.0).abs() < 1e-12 && inv >= 1.0 {
        (v * inv).round() / inv
    } else {
        (v / step).round() * step
    };
    if snapped == 0.0 { 0.0 } else { snapped }
}

/// Ear-clip a simple polygon into triangle vertices (3 per triangle).
/// Concave outlines such as an L stay inside the polygon. Degenerate input
/// returns nothing; a non-simple remainder falls back to a fan.
pub(crate) fn triangulate_polygon(poly: &[[f64; 2]]) -> Vec<[f64; 2]> {
    let mut pts: Vec<[f64; 2]> = Vec::with_capacity(poly.len());
    for &p in poly {
        if pts.last() != Some(&p) {
            pts.push(p);
        }
    }
    if pts.len() > 1 && pts.first() == pts.last() {
        pts.pop();
    }
    if pts.len() < 3 {
        return Vec::new();
    }
    let area: f64 = (0..pts.len())
        .map(|i| {
            let (a, b) = (pts[i], pts[(i + 1) % pts.len()]);
            a[0] * b[1] - b[0] * a[1]
        })
        .sum();
    let sign = if area >= 0.0 { 1.0 } else { -1.0 };
    let cross = |a: [f64; 2], b: [f64; 2], c: [f64; 2]| {
        (b[0] - a[0]) * (c[1] - a[1]) - (b[1] - a[1]) * (c[0] - a[0])
    };
    let mut index: Vec<usize> = (0..pts.len()).collect();
    let mut out = Vec::with_capacity((pts.len() - 2) * 3);
    let mut guard = 0;
    while index.len() > 3 && guard < 4 * pts.len() * pts.len() {
        guard += 1;
        let n = index.len();
        let mut clipped = false;
        for k in 0..n {
            let (ia, ib, ic) = (index[(k + n - 1) % n], index[k], index[(k + 1) % n]);
            let (a, b, c) = (pts[ia], pts[ib], pts[ic]);
            if sign * cross(a, b, c) <= 0.0 {
                continue;
            }
            let inside = index.iter().any(|&j| {
                if j == ia || j == ib || j == ic {
                    return false;
                }
                let p = pts[j];
                sign * cross(a, b, p) >= 0.0
                    && sign * cross(b, c, p) >= 0.0
                    && sign * cross(c, a, p) >= 0.0
            });
            if inside {
                continue;
            }
            out.extend([a, b, c]);
            index.remove(k);
            clipped = true;
            break;
        }
        if !clipped {
            for k in 1..index.len() - 1 {
                out.extend([pts[index[0]], pts[index[k]], pts[index[k + 1]]]);
            }
            return out;
        }
    }
    if index.len() == 3 {
        out.extend([pts[index[0]], pts[index[1]], pts[index[2]]]);
    }
    out
}

fn paint_polygon_fill(
    painter: &egui::Painter,
    view: &CanvasView,
    pts: &[[f64; 2]],
    fill: egui::Color32,
) {
    let tris = triangulate_polygon(pts);
    if tris.len() < 3 {
        return;
    }
    let mut mesh = egui::Mesh::default();
    for tri in tris.chunks(3) {
        if tri.len() < 3 {
            continue;
        }
        let base = mesh.vertices.len() as u32;
        for p in tri {
            mesh.colored_vertex(view.data_to_screen(*p), fill);
        }
        mesh.add_triangle(base, base + 1, base + 2);
    }
    if !mesh.is_empty() {
        painter.add(egui::Shape::mesh(mesh));
    }
}

pub(crate) fn validate_polygon(pts: &[[f64; 2]]) -> Result<(), String> {
    let n = pts.len();
    if n < 3 {
        return Err("Refused degenerate polygon: at least 3 vertices required.".into());
    }
    let mut area2 = 0.0;
    for i in 0..n {
        let a = pts[i];
        let b = pts[(i + 1) % n];
        if (b[0] - a[0]).hypot(b[1] - a[1]) <= 1e-9 {
            return Err("Refused degenerate polygon: consecutive vertices coincide.".into());
        }
        area2 += a[0] * b[1] - b[0] * a[1];
    }
    for i in 0..n {
        let a1 = pts[i];
        let a2 = pts[(i + 1) % n];
        for j in (i + 1)..n {
            // Skip adjacent edges (they share a vertex).
            if j == i + 1 || (i == 0 && j == n - 1) {
                continue;
            }
            let b1 = pts[j];
            let b2 = pts[(j + 1) % n];
            if segments_intersect(a1, a2, b1, b2) {
                return Err("Refused degenerate polygon: edges must not self-intersect.".into());
            }
        }
    }
    if area2.abs() * 0.5 <= 1e-12 {
        return Err("Refused degenerate polygon: area must be positive.".into());
    }
    Ok(())
}

fn orient(a: [f64; 2], b: [f64; 2], c: [f64; 2]) -> f64 {
    (b[0] - a[0]) * (c[1] - a[1]) - (b[1] - a[1]) * (c[0] - a[0])
}

fn on_segment(a: [f64; 2], b: [f64; 2], p: [f64; 2]) -> bool {
    p[0] >= a[0].min(b[0]) - 1e-12
        && p[0] <= a[0].max(b[0]) + 1e-12
        && p[1] >= a[1].min(b[1]) - 1e-12
        && p[1] <= a[1].max(b[1]) + 1e-12
}

fn segments_intersect(a1: [f64; 2], a2: [f64; 2], b1: [f64; 2], b2: [f64; 2]) -> bool {
    let o1 = orient(a1, a2, b1);
    let o2 = orient(a1, a2, b2);
    let o3 = orient(b1, b2, a1);
    let o4 = orient(b1, b2, a2);
    let eps = 1e-12;
    if ((o1 > eps && o2 < -eps) || (o1 < -eps && o2 > eps))
        && ((o3 > eps && o4 < -eps) || (o3 < -eps && o4 > eps))
    {
        return true;
    }
    if o1.abs() <= eps && on_segment(a1, a2, b1) {
        return true;
    }
    if o2.abs() <= eps && on_segment(a1, a2, b2) {
        return true;
    }
    if o3.abs() <= eps && on_segment(b1, b2, a1) {
        return true;
    }
    if o4.abs() <= eps && on_segment(b1, b2, a2) {
        return true;
    }
    false
}

fn sample_shape_outline(shape: &Shape) -> Vec<[f64; 2]> {
    match shape {
        Shape::Rectangle { x0, y0, w, h, .. } => {
            vec![[*x0, *y0], [x0 + w, *y0], [x0 + w, y0 + h], [*x0, y0 + h]]
        }
        Shape::Circle { cx, cy, r, .. } => {
            let n = 48;
            (0..n)
                .map(|k| {
                    let t = TAU * (k as f64) / (n as f64);
                    [cx + r * t.cos(), cy + r * t.sin()]
                })
                .collect()
        }
        Shape::Ellipse {
            cx,
            cy,
            a,
            b,
            angle,
            ..
        } => {
            let n = 48;
            let (sin_p, cos_p) = angle.sin_cos();
            (0..n)
                .map(|k| {
                    let t = TAU * (k as f64) / (n as f64);
                    let lx = a * t.cos();
                    let ly = b * t.sin();
                    [cx + cos_p * lx - sin_p * ly, cy + sin_p * lx + cos_p * ly]
                })
                .collect()
        }
        Shape::Polygon { pts, .. } => pts.clone(),
    }
}

fn shape_label_anchor(shape: &Shape) -> [f64; 2] {
    match shape {
        Shape::Rectangle { x0, y0, w, h, .. } => [x0.min(x0 + w), y0.max(y0 + h)],
        Shape::Circle { cx, cy, r, .. } => {
            let d = r.abs() * FRAC_1_SQRT_2;
            [cx - d, cy + d]
        }
        Shape::Ellipse {
            cx,
            cy,
            a,
            b,
            angle,
            ..
        } => {
            let (s, c) = angle.sin_cos();
            let lx = -a.abs() * FRAC_1_SQRT_2;
            let ly = b.abs() * FRAC_1_SQRT_2;
            [cx + c * lx - s * ly, cy + s * lx + c * ly]
        }
        Shape::Polygon { pts, .. } => {
            if pts.is_empty() {
                return [0.0, 0.0];
            }
            let min_x = pts.iter().map(|p| p[0]).fold(f64::INFINITY, f64::min);
            let max_y = pts.iter().map(|p| p[1]).fold(f64::NEG_INFINITY, f64::max);
            [min_x, max_y]
        }
    }
}

fn shapes_bbox(shapes: &[Shape]) -> Option<[f64; 4]> {
    if shapes.is_empty() {
        return None;
    }
    let mut min_x = f64::INFINITY;
    let mut max_x = f64::NEG_INFINITY;
    let mut min_y = f64::INFINITY;
    let mut max_y = f64::NEG_INFINITY;
    let mut include = |x: f64, y: f64| {
        if x.is_finite() && y.is_finite() {
            min_x = min_x.min(x);
            max_x = max_x.max(x);
            min_y = min_y.min(y);
            max_y = max_y.max(y);
        }
    };
    for shape in shapes {
        match shape {
            Shape::Rectangle { x0, y0, w, h, .. } => {
                include(*x0, *y0);
                include(x0 + w, y0 + h);
            }
            Shape::Circle { cx, cy, r, .. } => {
                include(cx - r, cy - r);
                include(cx + r, cy + r);
            }
            Shape::Ellipse {
                cx,
                cy,
                a,
                b,
                angle,
                ..
            } => {
                let (s, c) = angle.sin_cos();
                let dx = ((a * c).powi(2) + (b * s).powi(2)).sqrt();
                let dy = ((a * s).powi(2) + (b * c).powi(2)).sqrt();
                include(cx - dx, cy - dy);
                include(cx + dx, cy + dy);
            }
            Shape::Polygon { pts, .. } => {
                for &[x, y] in pts {
                    include(x, y);
                }
            }
        }
    }
    if min_x.is_finite() && max_x.is_finite() && min_y.is_finite() && max_y.is_finite() {
        Some([min_x, max_x, min_y, max_y])
    } else {
        None
    }
}

pub(crate) fn translate_shape(shape: &Shape, dx: f64, dy: f64) -> Shape {
    match shape {
        Shape::Rectangle { name, x0, y0, w, h } => Shape::Rectangle {
            name: name.clone(),
            x0: x0 + dx,
            y0: y0 + dy,
            w: *w,
            h: *h,
        },
        Shape::Circle { name, cx, cy, r } => Shape::Circle {
            name: name.clone(),
            cx: cx + dx,
            cy: cy + dy,
            r: *r,
        },
        Shape::Ellipse {
            name,
            cx,
            cy,
            a,
            b,
            angle,
        } => Shape::Ellipse {
            name: name.clone(),
            cx: cx + dx,
            cy: cy + dy,
            a: *a,
            b: *b,
            angle: *angle,
        },
        Shape::Polygon { name, pts } => Shape::Polygon {
            name: name.clone(),
            pts: pts.iter().map(|p| [p[0] + dx, p[1] + dy]).collect(),
        },
    }
}

fn pick_shape(shapes: &[Shape], p: [f64; 2], tol: f64) -> Option<usize> {
    for (i, shape) in shapes.iter().enumerate().rev() {
        if hit_test_shape(shape, p, tol) {
            return Some(i);
        }
    }
    None
}

fn hit_test_shape(shape: &Shape, p: [f64; 2], tol: f64) -> bool {
    match shape {
        Shape::Rectangle { x0, y0, w, h, .. } => {
            p[0] >= x0 - tol && p[0] <= x0 + w + tol && p[1] >= y0 - tol && p[1] <= y0 + h + tol
        }
        Shape::Circle { cx, cy, r, .. } => (p[0] - cx).hypot(p[1] - cy) <= r + tol,
        Shape::Ellipse {
            cx,
            cy,
            a,
            b,
            angle,
            ..
        } => {
            let (s, c) = angle.sin_cos();
            let dx = p[0] - cx;
            let dy = p[1] - cy;
            let lx = c * dx + s * dy;
            let ly = -s * dx + c * dy;
            let ea = (a + tol).max(1e-9);
            let eb = (b + tol).max(1e-9);
            (lx / ea).powi(2) + (ly / eb).powi(2) <= 1.0
        }
        Shape::Polygon { pts, .. } => {
            if pts.len() < 3 {
                return false;
            }
            for i in 0..pts.len() {
                if dist_to_segment_f64(p, pts[i], pts[(i + 1) % pts.len()]) <= tol {
                    return true;
                }
            }
            point_in_polygon(p, pts)
        }
    }
}

fn point_in_polygon(p: [f64; 2], pts: &[[f64; 2]]) -> bool {
    let mut inside = false;
    let n = pts.len();
    let mut j = n - 1;
    for i in 0..n {
        let yi = pts[i][1];
        let yj = pts[j][1];
        if ((yi > p[1]) != (yj > p[1]))
            && (p[0] < (pts[j][0] - pts[i][0]) * (p[1] - yi) / (yj - yi) + pts[i][0])
        {
            inside = !inside;
        }
        j = i;
    }
    inside
}

fn dist_to_segment_f64(p: [f64; 2], a: [f64; 2], b: [f64; 2]) -> f64 {
    let abx = b[0] - a[0];
    let aby = b[1] - a[1];
    let len2 = abx * abx + aby * aby;
    if len2 <= 1e-18 {
        return (p[0] - a[0]).hypot(p[1] - a[1]);
    }
    let t = (((p[0] - a[0]) * abx + (p[1] - a[1]) * aby) / len2).clamp(0.0, 1.0);
    let px = a[0] + t * abx;
    let py = a[1] + t * aby;
    (p[0] - px).hypot(p[1] - py)
}

fn edge_screen_distance(edge: &DecomposedEdge, pos: egui::Pos2, view: &CanvasView) -> f32 {
    let label_scr = view.data_to_screen(edge.label_pos);
    let mut d_min = label_scr.distance(pos);
    for w in edge.points.windows(2) {
        let a = view.data_to_screen(w[0]);
        let b = view.data_to_screen(w[1]);
        let d = dist_to_segment_f64(
            [pos.x as f64, pos.y as f64],
            [a.x as f64, a.y as f64],
            [b.x as f64, b.y as f64],
        ) as f32;
        d_min = d_min.min(d);
    }
    d_min
}

fn hit_test_edge(
    edges: &[DecomposedEdge],
    pos: egui::Pos2,
    view: &CanvasView,
    tol_px: f32,
) -> Option<usize> {
    let mut best: Option<(usize, f32)> = None;
    for edge in edges.iter().filter(|e| e.boundary) {
        let d_min = edge_screen_distance(edge, pos, view);
        if d_min <= tol_px && best.is_none_or(|(_, bd)| d_min < bd) {
            best = Some((edge.id, d_min));
        }
    }
    best.map(|(id, _)| id)
}

/// Decode a `decsg` decomposed geometry matrix `dl` into drawable edges `E1..En`.
pub(crate) fn decode_dl_edges(dl: &Matrix) -> Vec<DecomposedEdge> {
    let rows = dl.rows();
    let cols = dl.cols();
    if rows < 7 || cols == 0 {
        return Vec::new();
    }
    let mut out = Vec::with_capacity(cols);
    for j in 0..cols {
        let at = |r: usize| dl.get(r, j);
        let kind = at(0);
        let x0 = at(1);
        let x1 = at(2);
        let y0 = at(3);
        let y1 = at(4);
        let (points, label_pos) = if kind == 1.0 && rows >= 10 {
            let cx = at(7);
            let cy = at(8);
            let r = at(9);
            let t0 = (y0 - cy).atan2(x0 - cx);
            let t1 = (y1 - cy).atan2(x1 - cx);
            let mut dt = t1 - t0;
            if dt <= 0.0 {
                dt += TAU;
            }
            let steps = ((dt / FRAC_PI_2) * 16.0).ceil().clamp(4.0, 64.0) as usize;
            let pts: Vec<[f64; 2]> = (0..=steps)
                .map(|k| {
                    let t = t0 + dt * (k as f64) / (steps as f64);
                    [cx + r * t.cos(), cy + r * t.sin()]
                })
                .collect();
            let tm = t0 + 0.5 * dt;
            (pts, [cx + r * tm.cos(), cy + r * tm.sin()])
        } else if kind == 4.0 && rows >= 12 {
            let cx = at(7);
            let cy = at(8);
            let a = at(9).max(1e-12);
            let b = at(10).max(1e-12);
            let phi = at(11);
            let (s, c) = phi.sin_cos();
            let to_angle = |x: f64, y: f64| {
                let dx = x - cx;
                let dy = y - cy;
                let lx = (c * dx + s * dy) / a;
                let ly = (-s * dx + c * dy) / b;
                ly.atan2(lx)
            };
            let t0 = to_angle(x0, y0);
            let t1 = to_angle(x1, y1);
            let mut dt = t1 - t0;
            if dt <= 0.0 {
                dt += TAU;
            }
            let steps = ((dt / FRAC_PI_2) * 16.0).ceil().clamp(4.0, 64.0) as usize;
            let eval = |t: f64| {
                let lx = a * t.cos();
                let ly = b * t.sin();
                [cx + c * lx - s * ly, cy + s * lx + c * ly]
            };
            let pts: Vec<[f64; 2]> = (0..=steps)
                .map(|k| eval(t0 + dt * (k as f64) / (steps as f64)))
                .collect();
            (pts, eval(t0 + 0.5 * dt))
        } else {
            (vec![[x0, y0], [x1, y1]], [0.5 * (x0 + x1), 0.5 * (y0 + y1)])
        };
        out.push(DecomposedEdge {
            id: j + 1,
            points,
            label_pos,
            boundary: at(5) == 0.0 || at(6) == 0.0,
        });
    }
    out
}

fn nice_step(raw: f64) -> f64 {
    if !raw.is_finite() || raw <= 0.0 {
        return 0.5;
    }
    let exp = raw.log10().floor();
    let base = 10f64.powf(exp);
    let f = raw / base;
    let nice = if f <= 1.0 {
        1.0
    } else if f <= 2.0 {
        2.0
    } else if f <= 5.0 {
        5.0
    } else {
        10.0
    };
    nice * base
}

fn fmt_tick(v: f64) -> String {
    if v.abs() < 1e-12 {
        "0".into()
    } else if v.abs() >= 1e4 || v.abs() < 1e-3 {
        format!("{v:.1e}")
    } else {
        let s = format!("{v:.3}");
        s.trim_end_matches('0').trim_end_matches('.').to_owned()
    }
}
