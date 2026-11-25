//! Design View canvas (9a widget previews + grid placement/drag) and Code View
//! (read-only generated `.m` with editable callback/startup bodies).
use super::{AppDesigner, DesignerView, Selection};
use crate::{theme::ThemedScrollArea, tokens};
use eframe::egui::{self, Color32, RichText, Stroke};
use matrust::engine::app_designer_model::{
    AppComponent, AppComponentKind, AppValue, GridSpanSpec, codegen,
};

pub(super) fn show(designer: &mut AppDesigner, ui: &mut egui::Ui) {
    match designer.view {
        DesignerView::Design => show_design_canvas(designer, ui),
        DesignerView::Code => show_code_view(designer, ui),
    }
}

fn show_design_canvas(designer: &mut AppDesigner, ui: &mut egui::Ui) {
    let p = tokens::palette(ui.ctx());
    let read_only = designer.read_only.is_some();

    let avail = ui.available_rect_before_wrap();
    let fig_w = (avail.width() - tokens::S).clamp(0.0, 900.0);
    let fig_h = (avail.height() - tokens::S).clamp(0.0, 700.0);
    let fig_rect = egui::Rect::from_min_size(
        egui::pos2(
            avail.left() + (avail.width() - fig_w).max(0.0) * 0.5,
            avail.top() + tokens::XS,
        ),
        egui::vec2(fig_w, fig_h),
    );
    // Reserve the full figure rect so stacked layouts (narrow mode) place the inspector below it.
    ui.allocate_rect(fig_rect, egui::Sense::hover());

    // Figure window frame
    let fig_selected = designer.selection == Selection::Figure;
    ui.painter().rect(
        fig_rect,
        tokens::CONTROL_RADIUS,
        p.surface,
        Stroke::new(
            if fig_selected { 2.0 } else { 1.0 },
            if fig_selected {
                p.accent
            } else {
                p.pane_border
            },
        ),
        egui::StrokeKind::Inside,
    );

    // Title bar
    let title_h = 28.0;
    let title_rect = egui::Rect::from_min_size(fig_rect.min, egui::vec2(fig_rect.width(), title_h));
    ui.painter().rect_filled(title_rect, 0.0, p.canvas);
    ui.painter().hline(
        title_rect.x_range(),
        title_rect.bottom(),
        Stroke::new(1.0, p.hairline),
    );
    let title_resp = ui.interact(
        title_rect,
        egui::Id::new("app_designer_figure_title"),
        egui::Sense::click(),
    );
    if title_resp.clicked() {
        designer.selection = Selection::Figure;
        designer.sync_drafts(true);
    }
    let title_painter = ui.painter_at(title_rect.intersect(ui.clip_rect()));
    title_painter.text(
        egui::pos2(title_rect.left() + tokens::S, title_rect.center().y),
        egui::Align2::LEFT_CENTER,
        &designer.model.figure.title,
        egui::FontId::proportional(tokens::BODY),
        p.text,
    );
    if let Some(kind) = designer.placing {
        title_painter.text(
            egui::pos2(title_rect.right() - tokens::S, title_rect.center().y),
            egui::Align2::RIGHT_CENTER,
            format!(
                "Placing: {} (click an empty cell, Esc to cancel)",
                kind.display_name()
            ),
            egui::FontId::proportional(tokens::CAPTION),
            p.accent,
        );
    }

    let body_rect = egui::Rect::from_min_max(
        egui::pos2(fig_rect.left() + tokens::S, title_rect.bottom() + tokens::S),
        egui::pos2(fig_rect.right() - tokens::S, fig_rect.bottom() - tokens::S),
    );

    let Some(grid) = designer.model.grid.clone() else {
        show_position_canvas(designer, ui, body_rect);
        return;
    };

    let rows = grid.rows().max(1);
    let cols = grid.cols().max(1);
    let gap = tokens::S;
    let cell_w =
        ((body_rect.width() - gap * (cols.saturating_sub(1)) as f32) / cols as f32).max(24.0);
    let cell_h =
        ((body_rect.height() - gap * (rows.saturating_sub(1)) as f32) / rows as f32).max(24.0);

    let cell_rect = |r: usize, c: usize| -> egui::Rect {
        let x = body_rect.left() + (c - 1) as f32 * (cell_w + gap);
        let y = body_rect.top() + (r - 1) as f32 * (cell_h + gap);
        egui::Rect::from_min_size(egui::pos2(x, y), egui::vec2(cell_w, cell_h))
    };

    let pointer_pos = ui.input(|i| i.pointer.interact_pos().or_else(|| i.pointer.hover_pos()));
    let pointer_released = ui.input(|i| i.pointer.any_released());

    // 1. Draw grid cells & handle click-to-place and drag-drop targets
    let mut pending_place: Option<(usize, usize)> = None;
    let mut pending_drop: Option<(usize, usize)> = None;
    let mut pending_select: Option<String> = None;
    let mut cancel_placing = false;

    for r in 1..=rows {
        for c in 1..=cols {
            let rect = cell_rect(r, c);
            let hovered = pointer_pos.is_some_and(|pos| rect.contains(pos));
            let is_target = !read_only
                && (designer.placing.is_some() || designer.dragging.is_some())
                && hovered;
            let fill = if is_target {
                p.accent_soft
            } else {
                p.surface_sunken
            };
            let stroke = if is_target {
                Stroke::new(1.5, p.accent)
            } else {
                Stroke::new(1.0, p.hairline)
            };
            ui.painter().rect(
                rect,
                tokens::CONTROL_RADIUS,
                fill,
                stroke,
                egui::StrokeKind::Inside,
            );

            let occupant = designer.model.components.iter().find(|comp| {
                designer.model.is_in_root_grid(comp)
                    && comp.grid_row.zip(comp.grid_column).is_some_and(|(gr, gc)| {
                        gr.overlaps(GridSpanSpec::Single(r)) && gc.overlaps(GridSpanSpec::Single(c))
                    })
            });

            if occupant.is_none() {
                ui.painter().text(
                    rect.center(),
                    egui::Align2::CENTER_CENTER,
                    format!("({r}, {c})"),
                    egui::FontId::proportional(tokens::CAPTION),
                    p.text_muted,
                );
            }

            let resp = ui.interact(
                rect,
                egui::Id::new(("app_designer_cell", r, c)),
                egui::Sense::click(),
            );
            if resp.clicked() && !read_only && designer.placing.is_some() {
                if let Some(occ) = occupant {
                    pending_select = Some(occ.name.clone());
                    cancel_placing = true;
                } else {
                    pending_place = Some((r, c));
                }
            }
            if pointer_released && hovered && designer.dragging.is_some() {
                pending_drop = Some((r, c));
            }
        }
    }

    // 2. Draw components in their grid cells (with 9a widget look)
    let comps: Vec<AppComponent> = designer.model.components.clone();
    let mut started_drag: Option<String> = None;

    for comp in &comps {
        if !designer.model.is_in_root_grid(comp) {
            continue;
        }
        let (Some(gr), Some(gc)) = (comp.grid_row, comp.grid_column) else {
            continue;
        };
        let r0 = gr.start().clamp(1, rows);
        let r1 = gr.end().clamp(r0, rows);
        let c0 = gc.start().clamp(1, cols);
        let c1 = gc.end().clamp(c0, cols);
        let top_left = cell_rect(r0, c0);
        let bot_right = cell_rect(r1, c1);
        let rect = egui::Rect::from_min_max(top_left.min, bot_right.max);
        let inner = rect.shrink(tokens::XS);

        // Draw widget preview matching 9a ui_windows look
        ui.scope_builder(egui::UiBuilder::new().max_rect(inner), |ui| {
            ui.set_clip_rect(inner.intersect(ui.clip_rect()));
            draw_widget_preview(ui, inner, comp);
        });

        let selected = designer.selection == Selection::Component(comp.name.clone());
        if selected {
            ui.painter().rect_stroke(
                rect,
                tokens::CONTROL_RADIUS,
                Stroke::new(2.0, p.accent),
                egui::StrokeKind::Inside,
            );
        }

        let resp = ui.interact(
            rect,
            egui::Id::new(("app_designer_comp", &comp.name)),
            if read_only {
                egui::Sense::click()
            } else {
                egui::Sense::click_and_drag()
            },
        );
        if resp.clicked() {
            pending_select = Some(comp.name.clone());
            if designer.placing.is_some() {
                cancel_placing = true;
            }
        }
        if !read_only
            && (resp.drag_started() || (resp.dragged() && designer.dragging.is_none()))
            && designer.placing.is_none()
        {
            started_drag = Some(comp.name.clone());
        }
    }

    if let Some(name) = started_drag {
        designer.dragging = Some(name.clone());
        designer.selection = Selection::Component(name);
        designer.sync_drafts(true);
    }
    if cancel_placing {
        designer.placing = None;
        designer.clear_status();
    }
    if let Some(name) = pending_select {
        designer.selection = Selection::Component(name);
        designer.sync_drafts(true);
    } else if let Some((r, c)) = pending_place
        && let Some(kind) = designer.placing
    {
        designer.place_component_in_cell(kind, r, c);
    }
    if let Some((r, c)) = pending_drop
        && let Some(drag_name) = designer.dragging.take()
    {
        let same_cell = designer.model.component(&drag_name).is_some_and(|comp| {
            comp.grid_row == Some(GridSpanSpec::Single(r))
                && comp.grid_column == Some(GridSpanSpec::Single(c))
        });
        if !same_cell {
            match designer.model.move_component_grid(
                &drag_name,
                GridSpanSpec::Single(r),
                GridSpanSpec::Single(c),
            ) {
                Ok(()) => {
                    designer.mark_dirty();
                    designer.set_notice(format!("Moved '{drag_name}' to ({r}, {c})"));
                    designer.sync_drafts(true);
                }
                Err(err) => {
                    designer.set_error(err);
                }
            }
        }
    } else if pointer_released {
        designer.dragging = None;
    }
}

fn show_position_canvas(designer: &mut AppDesigner, ui: &mut egui::Ui, body_rect: egui::Rect) {
    let p = tokens::palette(ui.ctx());
    let comps = designer.model.components.clone();
    let mut pending_select = None;
    for comp in &comps {
        let [left, bottom, w, h] = comp.position;
        let x = body_rect.left() + left as f32;
        let y = body_rect.bottom() - (bottom + h) as f32;
        let rect = egui::Rect::from_min_size(egui::pos2(x, y), egui::vec2(w as f32, h as f32))
            .intersect(body_rect);
        if rect.width() <= 0.0 || rect.height() <= 0.0 {
            continue;
        }
        ui.scope_builder(egui::UiBuilder::new().max_rect(rect), |ui| {
            ui.set_clip_rect(rect.intersect(ui.clip_rect()));
            draw_widget_preview(ui, rect, comp);
        });
        if designer.selection == Selection::Component(comp.name.clone()) {
            ui.painter().rect_stroke(
                rect,
                tokens::CONTROL_RADIUS,
                Stroke::new(2.0, p.accent),
                egui::StrokeKind::Inside,
            );
        }
        if ui
            .interact(
                rect,
                egui::Id::new(("app_designer_pos_comp", &comp.name)),
                egui::Sense::click(),
            )
            .clicked()
        {
            pending_select = Some(comp.name.clone());
        }
    }
    if let Some(name) = pending_select {
        designer.selection = Selection::Component(name);
        designer.sync_drafts(true);
    }
}

/// Renders a non-interactive preview of `comp` styled identically to `src/desktop/ui_windows.rs`.
fn draw_widget_preview(ui: &mut egui::Ui, rect: egui::Rect, comp: &AppComponent) {
    let p = tokens::palette(ui.ctx());
    match comp.kind {
        AppComponentKind::Button => {
            let text = comp.props.text.as_deref().unwrap_or("Button");
            let btn_rect = egui::Rect::from_center_size(
                rect.center(),
                egui::vec2(rect.width().min(160.0), rect.height().min(tokens::CONTROL)),
            );
            ui.painter().rect(
                btn_rect,
                tokens::CONTROL_RADIUS,
                p.surface,
                Stroke::new(1.0, p.border),
                egui::StrokeKind::Inside,
            );
            ui.painter().text(
                btn_rect.center(),
                egui::Align2::CENTER_CENTER,
                text,
                egui::FontId::proportional(tokens::BODY),
                p.text,
            );
        }
        AppComponentKind::StateButton => {
            let text = comp.props.text.as_deref().unwrap_or("Button");
            let on = matches!(comp.props.value, Some(AppValue::Bool(true)));
            let btn_rect = egui::Rect::from_center_size(
                rect.center(),
                egui::vec2(rect.width().min(160.0), rect.height().min(tokens::CONTROL)),
            );
            ui.painter().rect(
                btn_rect,
                tokens::CONTROL_RADIUS,
                if on { p.accent_soft } else { p.surface },
                Stroke::new(1.0, p.border),
                egui::StrokeKind::Inside,
            );
            ui.painter().text(
                btn_rect.center(),
                egui::Align2::CENTER_CENTER,
                text,
                egui::FontId::proportional(tokens::BODY),
                if on { p.text } else { p.text_muted },
            );
        }
        AppComponentKind::Label => {
            let text = comp.props.text.as_deref().unwrap_or("Label");
            ui.painter().text(
                egui::pos2(rect.left() + tokens::XS, rect.center().y),
                egui::Align2::LEFT_CENTER,
                text,
                egui::FontId::proportional(tokens::BODY),
                p.text,
            );
        }
        AppComponentKind::EditField | AppComponentKind::NumericEditField => {
            let display = match comp.props.value.as_ref() {
                Some(AppValue::Number(x)) => format!("{x}"),
                Some(AppValue::Text(s)) => s.clone(),
                _ => {
                    if comp.kind == AppComponentKind::NumericEditField {
                        "0".into()
                    } else {
                        String::new()
                    }
                }
            };
            let box_rect = egui::Rect::from_center_size(
                rect.center(),
                egui::vec2(rect.width(), rect.height().min(tokens::CONTROL)),
            );
            ui.painter().rect(
                box_rect,
                tokens::CONTROL_RADIUS,
                p.surface,
                Stroke::new(1.0, p.border),
                egui::StrokeKind::Inside,
            );
            ui.painter().text(
                egui::pos2(box_rect.left() + tokens::S, box_rect.center().y),
                egui::Align2::LEFT_CENTER,
                display,
                egui::FontId::proportional(tokens::BODY),
                p.text,
            );
        }
        AppComponentKind::TextArea => {
            ui.painter().rect(
                rect,
                tokens::CONTROL_RADIUS,
                p.surface,
                Stroke::new(1.0, p.border),
                egui::StrokeKind::Inside,
            );
            let text = match comp.props.value.as_ref() {
                Some(AppValue::Lines(lines)) if !lines.is_empty() => lines.join("\n"),
                _ => comp.name.clone(),
            };
            ui.painter().text(
                egui::pos2(rect.left() + tokens::S, rect.top() + tokens::XS),
                egui::Align2::LEFT_TOP,
                text,
                egui::FontId::proportional(tokens::BODY),
                p.text_muted,
            );
        }
        AppComponentKind::DropDown => {
            let display = match comp.props.value.as_ref() {
                Some(AppValue::Text(s)) => s.as_str(),
                _ => comp
                    .props
                    .items
                    .as_ref()
                    .and_then(|v| v.first())
                    .map(String::as_str)
                    .unwrap_or("Option 1"),
            };
            let box_rect = egui::Rect::from_center_size(
                rect.center(),
                egui::vec2(rect.width(), rect.height().min(tokens::CONTROL)),
            );
            ui.painter().rect(
                box_rect,
                tokens::CONTROL_RADIUS,
                p.surface,
                Stroke::new(1.0, p.border),
                egui::StrokeKind::Inside,
            );
            ui.painter().text(
                egui::pos2(box_rect.left() + tokens::S, box_rect.center().y),
                egui::Align2::LEFT_CENTER,
                display,
                egui::FontId::proportional(tokens::BODY),
                p.text,
            );
            ui.painter().text(
                egui::pos2(box_rect.right() - tokens::S, box_rect.center().y),
                egui::Align2::RIGHT_CENTER,
                "v",
                egui::FontId::proportional(tokens::CAPTION),
                p.text_muted,
            );
        }
        AppComponentKind::ListBox => {
            ui.painter().rect(
                rect,
                tokens::CONTROL_RADIUS,
                p.surface,
                Stroke::new(1.0, p.border),
                egui::StrokeKind::Inside,
            );
            let items = comp.props.items.as_deref().unwrap_or(&[]);
            for (i, item) in items.iter().take(4).enumerate() {
                let y = rect.top() + tokens::XS + i as f32 * 18.0;
                if y + 14.0 > rect.bottom() {
                    break;
                }
                ui.painter().text(
                    egui::pos2(rect.left() + tokens::S, y),
                    egui::Align2::LEFT_TOP,
                    item,
                    egui::FontId::proportional(tokens::BODY),
                    p.text,
                );
            }
        }
        AppComponentKind::CheckBox => {
            let text = comp.props.text.as_deref().unwrap_or("Check Box");
            let checked = matches!(comp.props.value, Some(AppValue::Bool(true)));
            let sq = egui::Rect::from_center_size(
                egui::pos2(rect.left() + 12.0, rect.center().y),
                egui::vec2(14.0, 14.0),
            );
            ui.painter().rect(
                sq,
                2.0,
                if checked { p.accent_fill } else { p.surface },
                Stroke::new(1.0, p.border),
                egui::StrokeKind::Inside,
            );
            ui.painter().text(
                egui::pos2(sq.right() + tokens::S, rect.center().y),
                egui::Align2::LEFT_CENTER,
                text,
                egui::FontId::proportional(tokens::BODY),
                p.text,
            );
        }
        AppComponentKind::RadioButton => {
            let text = comp.props.text.as_deref().unwrap_or("Radio Button");
            let on = matches!(comp.props.value, Some(AppValue::Bool(true)));
            let center = egui::pos2(rect.left() + 12.0, rect.center().y);
            ui.painter()
                .circle(center, 7.0, p.surface, Stroke::new(1.0, p.border));
            if on {
                ui.painter().circle_filled(center, 3.5, p.accent_fill);
            }
            ui.painter().text(
                egui::pos2(center.x + 12.0, rect.center().y),
                egui::Align2::LEFT_CENTER,
                text,
                egui::FontId::proportional(tokens::BODY),
                p.text,
            );
        }
        AppComponentKind::Slider => {
            let y = rect.center().y;
            let left = rect.left() + tokens::S;
            let right = (rect.right() - tokens::S).max(left + 12.0);
            ui.painter()
                .hline(left..=right, y, Stroke::new(3.0, p.border));
            let lim = comp.props.limits.unwrap_or([0.0, 100.0]);
            let val = match comp.props.value {
                Some(AppValue::Number(v)) => v,
                _ => lim[0],
            };
            let frac = if lim[1] > lim[0] {
                ((val - lim[0]) / (lim[1] - lim[0])).clamp(0.0, 1.0) as f32
            } else {
                0.0
            };
            let tx = left + frac * (right - left);
            ui.painter().circle(
                egui::pos2(tx, y),
                6.0,
                p.accent_fill,
                Stroke::new(1.0, p.surface),
            );
        }
        AppComponentKind::Spinner => {
            let box_rect = egui::Rect::from_center_size(
                rect.center(),
                egui::vec2(rect.width(), rect.height().min(tokens::CONTROL)),
            );
            ui.painter().rect(
                box_rect,
                tokens::CONTROL_RADIUS,
                p.surface,
                Stroke::new(1.0, p.border),
                egui::StrokeKind::Inside,
            );
            let val = match comp.props.value {
                Some(AppValue::Number(v)) => format!("{v}"),
                _ => "0".into(),
            };
            ui.painter().text(
                box_rect.center(),
                egui::Align2::CENTER_CENTER,
                format!("-   {val}   +"),
                egui::FontId::proportional(tokens::BODY),
                p.text,
            );
        }
        AppComponentKind::Switch => {
            let items = comp.props.items.as_deref();
            let off_lbl = items
                .and_then(|v| v.first())
                .map(String::as_str)
                .unwrap_or("Off");
            let on_lbl = items
                .and_then(|v| v.get(1))
                .map(String::as_str)
                .unwrap_or("On");
            let is_on = matches!(comp.props.value.as_ref(), Some(AppValue::Text(s)) if s == on_lbl);
            let sw = egui::Rect::from_center_size(rect.center(), egui::vec2(36.0, 20.0));
            ui.painter().rect(
                sw,
                tokens::CONTROL_RADIUS,
                if is_on {
                    p.accent_fill
                } else {
                    p.surface_sunken
                },
                Stroke::new(1.0, p.border),
                egui::StrokeKind::Inside,
            );
            let thumb_x = if is_on {
                sw.right() - 16.0
            } else {
                sw.left() + 2.0
            };
            ui.painter().rect_filled(
                egui::Rect::from_min_size(
                    egui::pos2(thumb_x, sw.top() + 2.0),
                    egui::vec2(14.0, 16.0),
                ),
                2.0,
                p.surface,
            );
            ui.painter().text(
                egui::pos2(sw.left() - tokens::XS, sw.center().y),
                egui::Align2::RIGHT_CENTER,
                off_lbl,
                egui::FontId::proportional(tokens::CAPTION),
                p.text_muted,
            );
            ui.painter().text(
                egui::pos2(sw.right() + tokens::XS, sw.center().y),
                egui::Align2::LEFT_CENTER,
                on_lbl,
                egui::FontId::proportional(tokens::CAPTION),
                p.text_muted,
            );
        }
        AppComponentKind::Table => {
            ui.painter().rect(
                rect,
                tokens::CONTROL_RADIUS,
                p.surface,
                Stroke::new(1.0, p.border),
                egui::StrokeKind::Inside,
            );
            let cols = comp.props.column_name.as_deref().unwrap_or(&[]);
            let hdr_y = rect.top() + 20.0;
            ui.painter()
                .hline(rect.x_range(), hdr_y, Stroke::new(1.0, p.hairline));
            let n = cols.len().max(1);
            let cw = rect.width() / n as f32;
            for (i, col) in cols.iter().enumerate() {
                ui.painter().text(
                    egui::pos2(rect.left() + i as f32 * cw + tokens::XS, rect.top() + 10.0),
                    egui::Align2::LEFT_CENTER,
                    col,
                    egui::FontId::proportional(tokens::CAPTION),
                    p.text,
                );
            }
        }
        AppComponentKind::Axes => {
            ui.painter().rect(
                rect,
                tokens::CONTROL_RADIUS,
                Color32::WHITE,
                Stroke::new(1.0, p.border),
                egui::StrokeKind::Inside,
            );
            let plot_rect = rect.shrink2(egui::vec2(20.0, 14.0));
            if plot_rect.width() > 8.0 && plot_rect.height() > 8.0 {
                ui.painter().rect_stroke(
                    plot_rect,
                    0.0,
                    Stroke::new(1.0, Color32::from_gray(90)),
                    egui::StrokeKind::Inside,
                );
            }
            ui.painter().text(
                rect.center(),
                egui::Align2::CENTER_CENTER,
                &comp.name,
                egui::FontId::proportional(tokens::CAPTION),
                Color32::from_gray(110),
            );
        }
        AppComponentKind::Panel
        | AppComponentKind::ButtonGroup
        | AppComponentKind::TabGroup
        | AppComponentKind::Tab => {
            ui.painter().rect(
                rect,
                tokens::CONTROL_RADIUS,
                p.surface_sunken,
                Stroke::new(1.0, p.border),
                egui::StrokeKind::Inside,
            );
            let title = comp.props.title.as_deref().unwrap_or(&comp.name);
            ui.painter().text(
                egui::pos2(rect.left() + tokens::S, rect.top() + tokens::XS),
                egui::Align2::LEFT_TOP,
                title,
                egui::FontId::proportional(tokens::BODY),
                p.text,
            );
        }
    }
}

/// Code View: read-only generated `.m` with editable callback/startup bodies.
fn show_code_view(designer: &mut AppDesigner, ui: &mut egui::Ui) {
    let p = tokens::palette(ui.ctx());

    // Read-only file mode (hand-written classdef or .mlapp)
    if let Some(ref ro) = designer.read_only {
        let mut text = ro.code.clone();
        egui::ScrollArea::both()
            .id_salt("app_designer_ro_code_scroll")
            .auto_shrink([false, false])
            .show_themed(ui, |ui| {
                ui.add(
                    egui::TextEdit::multiline(&mut text)
                        .font(egui::FontId::monospace(tokens::CODE))
                        .code_editor()
                        .interactive(false)
                        .desired_width(ui.available_width().max(480.0)),
                );
            });
        return;
    }

    // Editable model mode
    let generated = designer.model.generate_m();
    egui::ScrollArea::vertical()
        .id_salt("app_designer_code_scroll")
        .auto_shrink([false, false])
        .show_themed(ui, |ui| {
            match generated {
                Ok(code) => {
                    let segments = split_code_segments(&code);
                    for (idx, seg) in segments.into_iter().enumerate() {
                        match seg {
                            CodeSegment::ReadOnly(chunk) => {
                                let mut ro = chunk;
                                ui.add(
                                    egui::TextEdit::multiline(&mut ro)
                                        .id(egui::Id::new(("app_designer_code_ro", idx)))
                                        .font(egui::FontId::monospace(tokens::CODE))
                                        .interactive(false)
                                        .frame(egui::Frame::NONE)
                                        .desired_width(ui.available_width()),
                                );
                            }
                            CodeSegment::EditableBody(method_name) => {
                                show_editable_body(designer, ui, &method_name);
                            }
                        }
                    }
                }
                Err(err) => {
                    ui.label(
                        RichText::new(format!(
                            "Fix validation problems to preview full generated code: {err}"
                        ))
                        .color(p.error),
                    );
                    ui.add_space(tokens::S);
                    // Still allow editing startup/callback bodies even while there are problems.
                    if designer.model.startup_fcn.is_some() {
                        show_editable_body(designer, ui, "startupFcn");
                    }
                    let names: Vec<String> = designer
                        .model
                        .callbacks
                        .iter()
                        .map(|c| c.method_name.clone())
                        .collect();
                    for method_name in names {
                        show_editable_body(designer, ui, &method_name);
                    }
                }
            }
        });
}

fn show_editable_body(designer: &mut AppDesigner, ui: &mut egui::Ui, method_name: &str) {
    let p = tokens::palette(ui.ctx());
    let mut body = if method_name == "startupFcn" {
        designer.model.startup_fcn.clone().unwrap_or_default()
    } else {
        designer
            .model
            .callbacks
            .iter()
            .find(|c| c.method_name == method_name)
            .map(|c| c.body.clone())
            .unwrap_or_default()
    };

    let edit_id = egui::Id::new(("app_designer_body", method_name));
    let focused = designer.focus_callback.as_deref() == Some(method_name);

    egui::Frame::new()
        .fill(p.surface_sunken)
        .stroke(Stroke::new(
            if focused { 1.5 } else { 1.0 },
            if focused { p.accent } else { p.border },
        ))
        .corner_radius(tokens::CONTROL_RADIUS)
        .inner_margin(egui::Margin::same(tokens::XS as i8))
        .show(ui, |ui| {
            ui.label(
                RichText::new(format!("Editable body: {method_name}"))
                    .size(tokens::CAPTION)
                    .color(p.text_muted),
            );
            let resp = ui.add(
                egui::TextEdit::multiline(&mut body)
                    .id(edit_id)
                    .font(egui::FontId::monospace(tokens::CODE))
                    .desired_rows(4)
                    .desired_width(ui.available_width()),
            );
            if focused {
                resp.request_focus();
                resp.scroll_to_me(Some(egui::Align::Center));
                designer.focus_callback = None;
            }
            if resp.changed() {
                // A marker-looking line would split regions on reload: refuse
                // it and keep the last accepted body.
                let applied = if method_name == "startupFcn" {
                    codegen::check_body_markers(&body, method_name).map(|()| {
                        designer.model.startup_fcn = Some(body);
                    })
                } else {
                    designer.model.set_callback_body(method_name, body)
                };
                match applied {
                    Ok(()) => {
                        designer.mark_dirty();
                        designer.clear_status();
                    }
                    Err(err) => designer.set_error(err),
                }
            }
        });
}

enum CodeSegment {
    ReadOnly(String),
    EditableBody(String),
}

fn split_code_segments(code: &str) -> Vec<CodeSegment> {
    let mut segments = Vec::new();
    let mut current_ro = String::new();
    let mut in_body: Option<String> = None;

    for line in code.split_inclusive('\n') {
        let trimmed = line.trim();
        if let Some(ref active) = in_body {
            if let Some(rest) = trimmed.strip_prefix(codegen::BODY_END_PREFIX)
                && rest.strip_suffix(']') == Some(active.as_str())
            {
                in_body = None;
                current_ro.push_str(line);
            }
            continue;
        }
        if let Some(rest) = trimmed.strip_prefix(codegen::BODY_START_PREFIX)
            && let Some(name) = rest.strip_suffix(']')
        {
            current_ro.push_str(line);
            if !current_ro.is_empty() {
                segments.push(CodeSegment::ReadOnly(std::mem::take(&mut current_ro)));
            }
            segments.push(CodeSegment::EditableBody(name.to_string()));
            in_body = Some(name.to_string());
            continue;
        }
        current_ro.push_str(line);
    }
    if !current_ro.is_empty() {
        segments.push(CodeSegment::ReadOnly(current_ro));
    }
    segments
}
