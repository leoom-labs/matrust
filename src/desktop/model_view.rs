//! Transient diagram magnification; model geometry remains in logical units.
use super::{Pos2, Rect, Vec2};

// Orthogonal wires extend at most20 logical units beyond node bounds (ports
// use10/12, upper lanes12..18, lower lanes14..20). Preserve additional room
// for ports/strokes, including custom nodes placed at coordinate zero.
pub(super) const CONTENT_PAD: f32 = 24.;

#[derive(Clone, Copy)]
pub(super) enum Command {
    In,
    Out,
    Actual,
    All,
    Selection,
}
pub(super) struct View {
    pub zoom: f32,
    pub request: Option<Command>,
    pub focus: Option<Rect>,
    pub viewport: Rect,
    pub origin: Pos2,
    pub offset: Vec2,
    pub requested_offset: Option<Vec2>,
    pub pan: Option<(Pos2, Vec2)>,
}
impl Default for View {
    fn default() -> Self {
        Self {
            zoom: 1.,
            request: None,
            focus: None,
            viewport: Rect::NOTHING,
            origin: Pos2::ZERO,
            offset: Vec2::ZERO,
            requested_offset: None,
            pan: None,
        }
    }
}
impl View {
    pub fn prepare(&mut self, boxes: &[Rect], selected: &[usize], viewport: Vec2) {
        let Some(command) = self.request.take() else {
            return;
        };
        let selected_bounds = selected
            .iter()
            .filter_map(|index| boxes.get(*index))
            .fold(Rect::NOTHING, |bounds, rect| bounds.union(*rect));
        let bounds = if matches!(command, Command::All) {
            boxes
                .iter()
                .fold(Rect::NOTHING, |bounds, rect| bounds.union(*rect))
        } else {
            selected_bounds
        };
        let bounds = if bounds.is_finite() {
            bounds.expand(CONTENT_PAD)
        } else {
            bounds
        };
        self.zoom = match command {
            Command::In => self.zoom * 1.25,
            Command::Out => self.zoom / 1.25,
            Command::Actual => 1.,
            Command::All | Command::Selection if bounds.is_finite() => ((viewport.x - 24.).max(1.)
                / bounds.width().max(1.))
            .min((viewport.y - 24.).max(1.) / bounds.height().max(1.))
            .min(1.),
            _ => self.zoom,
        }
        .clamp(0.0001, 4.);
        self.focus = bounds.is_finite().then_some(bounds);
    }
    pub fn screen_rect(&self, rect: Rect, origin: Pos2) -> Rect {
        Rect::from_min_max(
            origin + rect.min.to_vec2() * self.zoom,
            origin + rect.max.to_vec2() * self.zoom,
        )
    }
    pub fn model_point(&self, point: Pos2, origin: Pos2) -> Pos2 {
        ((point - origin) / self.zoom).to_pos2()
    }
    pub fn model_rect(&self, rect: Rect, origin: Pos2) -> Rect {
        Rect::from_min_max(
            self.model_point(rect.min, origin),
            self.model_point(rect.max, origin),
        )
    }
}

impl super::Canvas {
    /// Install a fresh transient view only when a model is replaced/restored.
    /// The pending fit uses the first real canvas dimensions; later navigation
    /// and ordinary model edits never trigger another automatic fit.
    pub fn reset_for_model(&mut self, model: &super::Model, selected: usize) {
        self.reset();
        if model.positions.is_empty() {
            self.reveal(selected);
        } else {
            self.view.request = Some(Command::All);
        }
    }

    pub(super) fn navigate_pointer(&mut self, ui: &super::egui::Ui, allowed: bool) {
        use super::egui::{Event, MouseWheelUnit, PointerButton, Popup};
        if !allowed
            || !ui.is_enabled()
            || !ui.input(|input| input.focused)
            || Popup::is_any_open(ui.ctx())
        {
            self.view.pan = None;
            return;
        }
        let hovered = ui.rect_contains_pointer(self.view.viewport);
        let wheel = if hovered {
            ui.input_mut(|input| {
                let mut steps = 0_f32;
                input.events.retain(|event| {
                    if let Event::MouseWheel {
                        unit,
                        delta,
                        modifiers,
                        ..
                    } = event
                        && (modifiers.ctrl || modifiers.command)
                        && !modifiers.alt
                    {
                        let amount = match unit {
                            MouseWheelUnit::Point => delta.y / 100.,
                            MouseWheelUnit::Line | MouseWheelUnit::Page => delta.y,
                        };
                        if amount.is_finite() {
                            steps += amount;
                        }
                        false
                    } else {
                        true
                    }
                });
                if steps != 0. {
                    input.smooth_scroll_delta = Vec2::ZERO;
                }
                steps.clamp(-8., 8.)
            })
        } else {
            0.
        };
        if wheel != 0.
            && let Some(pointer) = ui.input(|input| input.pointer.hover_pos())
        {
            let model_point = self.view.model_point(pointer, self.view.origin);
            self.cancel_connection();
            self.reveal = None;
            self.reveal_port = None;
            self.view.focus = None;
            self.view.zoom = (self.view.zoom * 1.25_f32.powf(wheel)).clamp(0.0001, 4.);
            self.view.requested_offset = Some(
                ((model_point.to_vec2() + Vec2::splat(CONTENT_PAD)) * self.view.zoom
                    - (pointer - self.view.viewport.min))
                    .max(Vec2::ZERO),
            );
        }
        if hovered
            && ui.input(|input| input.pointer.button_pressed(PointerButton::Middle))
            && let Some(pointer) = ui.input(|input| input.pointer.interact_pos())
        {
            self.cancel_connection();
            self.reveal = None;
            self.reveal_port = None;
            self.view.focus = None;
            self.view.pan = Some((pointer, self.view.offset));
            self.focus(ui.ctx());
        }
        if let Some((origin, offset)) = self.view.pan {
            let down = ui.input(|input| input.pointer.button_down(PointerButton::Middle));
            if (down || ui.input(|input| input.pointer.button_released(PointerButton::Middle)))
                && let Some(pointer) = ui.input(|input| input.pointer.interact_pos())
            {
                self.view.requested_offset = Some((offset + (origin - pointer)).max(Vec2::ZERO));
                ui.ctx().set_cursor_icon(super::egui::CursorIcon::Grabbing);
            }
            if !down {
                self.view.pan = None;
            }
        }
    }
}
