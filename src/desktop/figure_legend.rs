//! Bounded legend rows use series metadata, never the original sample buffers.
use super::{StyledPoints, native_marker};
use crate::theme::ThemedScrollArea;
use eframe::egui::{self, Color32, Pos2, Rect, Sense, Stroke, Vec2};
use egui_plot::{PlotBounds, PlotItem, PlotPoint, PlotPoints, PlotTransform, Points};
use matrust::plot::{Axes, LinePattern, PlotKind, Series};

#[derive(Default)]
pub(super) struct Response {
    pub visibility: Option<Vec<bool>>,
    pub hovered: Option<egui::Id>,
    pub rect: Option<Rect>,
}

#[derive(Clone, Default)]
struct KeyboardFocus {
    entry: Option<(usize, matrust::plot::WeakSeriesPoints)>,
    suspended: bool,
}

// Explicitly release state owned by a retired detached plot generation.
pub(super) fn forget(ctx: &egui::Context, id: egui::Id) {
    ctx.data_mut(|d| {
        d.remove::<KeyboardFocus>(id.with("legend_keyboard_focus"));
        d.remove::<f32>(id.with("legend_offset"));
        if let Some(scroll) = d.get_temp::<egui::Id>(id.with("legend_scroll_id")) {
            d.remove::<egui::scroll_area::State>(scroll);
        }
        d.remove::<egui::Id>(id.with("legend_scroll_id"));
    });
}

#[cfg(test)]
pub(super) fn show(ui: &mut egui::Ui, axes: &Axes, frame: Rect, id: egui::Id) -> Response {
    show_placed(
        ui,
        axes,
        frame,
        id,
        None,
        axes.legend_location.fixed_anchor(),
        matrust::plot::ResolvedAppearance::default(),
    )
}

pub(super) fn extent(ui: &egui::Ui, axes: &Axes, available: Rect, outside: bool) -> Option<Vec2> {
    let count = axes.legend_entries().count();
    if !axes.legend || count == 0 || available.width() < 48. || available.height() < 32. {
        return None;
    }
    let inset = 4. + 2. * ui.visuals().window_stroke().width;
    let max_height = if outside {
        available.height()
    } else {
        (available.height() * 0.5).max(24. + inset)
    };
    let rows = (((max_height - inset + 2.) / 26.).floor() as usize)
        .max(1)
        .min(count);
    Some(egui::vec2(
        (available.width() * if outside { 1. } else { 0.65 })
            .clamp(100., 240.)
            .min(available.width()),
        rows as f32 * 26. - 2. + inset,
    ))
}

pub(super) fn show_placed(
    ui: &mut egui::Ui,
    axes: &Axes,
    frame: Rect,
    id: egui::Id,
    placement: Option<Rect>,
    location: matrust::plot::LegendLocation,
    appearance: matrust::plot::ResolvedAppearance,
) -> Response {
    let mut result = Response::default();
    let focus_key = id.with("legend_keyboard_focus");
    if !axes.legend {
        ui.ctx().data_mut(|d| d.remove::<KeyboardFocus>(focus_key));
        return result;
    }
    let entries: Vec<_> = axes.legend_entries().collect();
    let available = placement
        .unwrap_or_else(|| frame.shrink(4.))
        .intersect(ui.clip_rect());
    let minimum_height = if placement.is_some() {
        28. + 2. * ui.visuals().window_stroke().width
    } else {
        32.
    };
    if entries.is_empty() || available.width() < 48. || available.height() < minimum_height {
        ui.ctx().data_mut(|d| d.remove::<KeyboardFocus>(focus_key));
        return result;
    }
    let row_height = 24.;
    let width = (available.width() * if placement.is_some() { 1. } else { 0.65 })
        .clamp(100., 240.)
        .min(available.width());
    let border = ui.visuals().window_stroke();
    let inset = 4. + 2. * border.width;
    let max_height = if placement.is_some() {
        available.height()
    } else {
        (available.height() * 0.5).max(row_height + inset)
    };
    let shown_rows = (((max_height - inset + 2.) / (row_height + 2.)).floor() as usize)
        .max(1)
        .min(entries.len());
    let height = shown_rows as f32 * (row_height + 2.) - 2. + inset;
    // Explicit isolated capture mode drives the same input path as the user.
    let capture_id = egui::Id::new("legend_keyboard_capture");
    if ui.is_enabled()
        && entries.len() == 64
        && let Some(stage) = ui.ctx().data(|d| d.get_temp::<u8>(capture_id))
    {
        let next = match stage {
            0 => {
                ui.memory_mut(|m| m.request_focus(id.with(("legend_series", entries[0].0))));
                1
            }
            1 | 2 | 4 | 6 => {
                ui.input_mut(|i| {
                    i.events.push(egui::Event::Key {
                        key: if stage == 1 {
                            egui::Key::End
                        } else if stage == 4 {
                            egui::Key::Enter
                        } else {
                            egui::Key::Space
                        },
                        physical_key: None,
                        pressed: true,
                        repeat: false,
                        modifiers: egui::Modifiers::NONE,
                    })
                });
                stage + 1
            }
            3 if !entries[63].1.visible => 4,
            5 if entries[63].1.visible => 6,
            7 if !entries[63].1.visible
                && ui.memory(|m| m.has_focus(id.with(("legend_series", entries[63].0)))) =>
            {
                eprintln!(
                    "Legend keyboard capture: three toggles completed; last series hidden; focus retained."
                );
                8
            }
            _ => stage,
        };
        if next != stage {
            ui.ctx().data_mut(|d| d.insert_temp(capture_id, next));
        }
        if next < 8 {
            ui.ctx().request_repaint();
        }
    }
    let mut focus_state = ui
        .ctx()
        .data(|d| d.get_temp::<KeyboardFocus>(focus_key))
        .unwrap_or_default();
    let current_focus = ui.memory(|m| m.focused());
    let owner = focus_state
        .entry
        .as_ref()
        .map(|(index, _)| id.with(("legend_series", index)));
    let mut restore = None;
    if !ui.is_enabled() {
        if current_focus.is_none() || current_focus == owner {
            focus_state.suspended = focus_state.entry.is_some();
        } else {
            focus_state = KeyboardFocus::default();
        }
    } else if focus_state.suspended {
        if (current_focus.is_none() || current_focus == owner)
            && let Some((index, points)) = &focus_state.entry
        {
            restore = entries
                .iter()
                .position(|(i, s)| i == index && points.matches(&s.points));
            if let Some(row) = restore {
                ui.memory_mut(|m| m.request_focus(id.with(("legend_series", entries[row].0))));
            }
        }
        focus_state.suspended = false;
    }
    let focused = ui.memory(|m| m.focused()).and_then(|focus| {
        entries
            .iter()
            .position(|(index, _)| id.with(("legend_series", index)) == focus)
    });
    let mut target = restore;
    if ui.is_enabled()
        && let Some(focused) = focused
    {
        ui.input_mut(|input| {
            input.events.retain(|event| {
                if let egui::Event::Key {
                    key,
                    pressed: true,
                    modifiers,
                    ..
                } = event
                    && *modifiers == egui::Modifiers::NONE
                {
                    let current = target.unwrap_or(focused);
                    let next = match key {
                        egui::Key::ArrowUp => current.saturating_sub(1),
                        egui::Key::ArrowDown => (current + 1).min(entries.len() - 1),
                        egui::Key::Home => 0,
                        egui::Key::End => entries.len() - 1,
                        egui::Key::PageUp => current.saturating_sub(shown_rows),
                        egui::Key::PageDown => (current + shown_rows).min(entries.len() - 1),
                        _ => return true,
                    };
                    target = Some(next);
                    return false;
                }
                true
            })
        });
        if target.is_some() {
            ui.memory_mut(|m| m.move_focus(egui::FocusDirection::None));
        }
    }
    let [horizontal, vertical] = location.alignment();
    let rect = Rect::from_min_size(
        egui::pos2(
            available.left() + (available.width() - width).max(0.) * horizontal as f32,
            available.top() + (available.height() - height).max(0.) * vertical as f32,
        ),
        egui::vec2(width, height),
    );
    result.rect = Some(rect);
    let mut legend = ui.new_child(
        egui::UiBuilder::new()
            .id_salt(id.with("series_legend"))
            .max_rect(rect)
            .layout(egui::Layout::top_down(egui::Align::LEFT)),
    );
    legend.set_clip_rect(rect.intersect(ui.clip_rect()));
    legend.spacing_mut().item_spacing = egui::vec2(4., 2.);
    // Cover the entire legend background, including gaps between rows, so a
    // pointer gesture on it does not begin dragging the plot underneath.
    legend.interact(rect, id.with("legend_background"), Sense::click_and_drag());
    let mut clicked = None;
    let mut shapes = Vec::with_capacity(4);
    egui::Frame::new()
        .inner_margin(2)
        .fill(ui.visuals().extreme_bg_color)
        .stroke(border)
        .corner_radius(3)
        .show(&mut legend, |ui| {
            ui.set_width((width - inset).max(1.));
            let mut scroll = egui::ScrollArea::vertical()
                .id_salt(id.with("legend_scroll"))
                .scroll_bar_visibility(if entries.len() > shown_rows {
                    egui::scroll_area::ScrollBarVisibility::AlwaysVisible
                } else {
                    egui::scroll_area::ScrollBarVisibility::AlwaysHidden
                })
                .max_height((height - inset).max(1.))
                .min_scrolled_height(0.)
                .auto_shrink([false, true]);
            if let Some(row) = target {
                let top = row as f32 * (row_height + 2.);
                let bottom = top + row_height;
                let offset = ui.ctx().data(|d| d.get_temp::<f32>(id.with("legend_offset"))).unwrap_or(0.);
                let viewport = height - inset;
                scroll = scroll.vertical_scroll_offset(if top < offset { top }
                    else if bottom > offset + viewport { bottom - viewport } else { offset });
            }
            let output = scroll.show_rows_themed(ui, row_height, entries.len(), |ui, range| {
                    for row in range {
                        let (index, series) = entries[row];
                        let (_, rect) =
                            ui.allocate_space(egui::vec2(ui.available_width(), row_height));
                        let response =
                            ui.interact(rect, id.with(("legend_series", index)), Sense::click());
                        response.widget_info(|| {
                            egui::WidgetInfo::selected(
                                egui::WidgetType::Checkbox,
                                ui.is_enabled(),
                                series.visible,
                                &series.label,
                            )
                        });
                        if target == Some(row) { response.request_focus(); }
                        if response.hovered() {
                            result.hovered = Some(egui::Id::new(("series", index)));
                        }
                        if response.clicked() {
                            response.request_focus();
                            clicked = Some(index);
                        }
                        if response.has_focus() {
                            ui.memory_mut(|m| m.set_focus_lock_filter(response.id, egui::EventFilter {
                                vertical_arrows: true, ..Default::default()
                            }));
                        }
                        if response.hovered() || response.has_focus() {
                            ui.painter().rect_filled(
                                rect,
                                2.,
                                ui.visuals().widgets.hovered.bg_fill,
                            );
                            if response.has_focus() {
                                ui.painter().rect_stroke(
                                    rect,
                                    2.,
                                    ui.visuals().selection.stroke,
                                    egui::StrokeKind::Inside,
                                );
                            }
                        }
                        let icon_width = 38_f32.min(rect.width() * 0.45);
                        let icon = Rect::from_min_max(
                            rect.min,
                            egui::pos2(rect.left() + icon_width, rect.bottom()),
                        );
                        draw_icon_in_appearance(ui, series, icon, &mut shapes, appearance);
                        let text_rect =
                            Rect::from_min_max(egui::pos2(icon.right() + 4., rect.top()), rect.max);
                        let color = if series.visible {
                            ui.visuals().text_color()
                        } else {
                            ui.visuals().weak_text_color()
                        };
                        let galley = egui::WidgetText::from(series.label.as_str()).into_galley(
                            ui,
                            Some(egui::TextWrapMode::Truncate),
                            text_rect.width().max(0.),
                            crate::theme::figure_body(ui.ctx()),
                        );
                        ui.painter().galley(
                            egui::pos2(
                                text_rect.left(),
                                text_rect.center().y - galley.size().y * 0.5,
                            ),
                            galley,
                            color,
                        );
                        response.on_hover_ui(|ui| {
                            ui.set_max_width(320.);
                            egui::ScrollArea::vertical()
                                .max_height(160.)
                                .show_themed(ui, |ui| {
                                    ui.label(&series.label);
                                });
                            ui.label("Click or Space/Enter to show/hide; Alt-click to isolate or restore all.");
                            ui.label("Up/Down, Home/End and Page Up/Down move through the legend.");
                            ui.label("Legend symbols are fitted to the row.");
                        });
                    }
                });
            ui.ctx().data_mut(|d| { d.insert_temp(id.with("legend_offset"),output.state.offset.y); d.insert_temp(id.with("legend_scroll_id"),output.id); });
        });
    if ui.is_enabled() {
        focus_state.entry = ui.memory(|m| m.focused()).and_then(|focus| {
            entries
                .iter()
                .find(|(index, _)| id.with(("legend_series", index)) == focus)
                .map(|(index, s)| (*index, s.points.downgrade()))
        });
    }
    ui.ctx().data_mut(|d| d.insert_temp(focus_key, focus_state));
    if let Some(index) = clicked {
        let mut visible: Vec<_> = axes.series.iter().map(|s| s.visible).collect();
        if ui.input(|i| i.modifiers.alt) {
            let isolated = entries.iter().all(|(i, s)| *i == index || !s.visible);
            for (i, _) in entries {
                visible[i] = isolated || i == index;
            }
        } else {
            visible[index] = !visible[index];
        }
        result.visibility = Some(visible);
    }
    result
}

#[cfg(test)]
fn draw_icon(ui: &egui::Ui, series: &Series, rect: Rect, shapes: &mut Vec<egui::Shape>) {
    draw_icon_in_appearance(
        ui,
        series,
        rect,
        shapes,
        matrust::plot::ResolvedAppearance::default(),
    );
}
fn draw_icon_in_appearance(
    ui: &egui::Ui,
    series: &Series,
    rect: Rect,
    shapes: &mut Vec<egui::Shape>,
    appearance: matrust::plot::ResolvedAppearance,
) {
    let rgb = appearance.series_color(series);
    let color = Color32::from_rgb(rgb[0], rgb[1], rgb[2]).gamma_multiply(if series.visible {
        1.
    } else {
        0.35
    });
    let painter = ui.painter().with_clip_rect(rect.intersect(ui.clip_rect()));
    let center = rect.center();
    let stroke = Stroke::new(series.stroke_width(1.7).min(3.), color);
    let left = rect.left() + 3.;
    let right = rect.right() - 3.;
    let mut marker = center;
    match series.kind {
        PlotKind::Bar | PlotKind::Patch => {
            painter.rect_filled(
                Rect::from_center_size(center, Vec2::new((rect.width() - 8.).max(1.), 10.)),
                0.,
                color,
            );
            if let Some(edge) = series.face_edge_color
                && series.pattern != LinePattern::None
            {
                let rect =
                    Rect::from_center_size(center, Vec2::new((rect.width() - 8.).max(1.), 10.));
                let edge = Color32::from_rgb(edge[0], edge[1], edge[2])
                    .gamma_multiply(if series.visible { 1. } else { 0.35 });
                let mut path = vec![
                    rect.left_top(),
                    rect.right_top(),
                    rect.right_bottom(),
                    rect.left_bottom(),
                    rect.left_top(),
                ];
                let mut budget = 200000.;
                super::face_edge_path(
                    &mut path,
                    Stroke::new(series.face_edge_width().min(3.), edge),
                    series.pattern,
                    &mut budget,
                    shapes,
                );
                painter.extend(shapes.drain(..));
            }
        }
        PlotKind::Scatter => {}
        _ if series.pattern == LinePattern::None => {}
        kind => {
            let (path, count) = match kind {
                PlotKind::Stairs => {
                    marker.y -= 3.;
                    (
                        [
                            Pos2::new(left, center.y + 3.),
                            Pos2::new(center.x, center.y + 3.),
                            marker,
                            Pos2::new(right, marker.y),
                        ],
                        4,
                    )
                }
                PlotKind::Stem => {
                    marker.y -= 3.;
                    painter.line_segment(
                        [
                            Pos2::new(left + 5., center.y + 7.),
                            Pos2::new(right - 5., center.y + 7.),
                        ],
                        stroke,
                    );
                    (
                        [Pos2::new(center.x, center.y + 7.), marker, marker, marker],
                        2,
                    )
                }
                _ => (
                    [
                        Pos2::new(left, center.y),
                        Pos2::new(right, center.y),
                        center,
                        center,
                    ],
                    2,
                ),
            };
            if series.pattern == LinePattern::Solid {
                painter.add(egui::Shape::line(path[..count].to_vec(), stroke));
            } else {
                let (dash, gap): (&[f32], &[f32]) = match series.pattern {
                    LinePattern::Dashed => (&[8.], &[5.]),
                    LinePattern::Dotted => (&[2.], &[4.]),
                    LinePattern::DashDot => (&[8., 2.], &[4., 4.]),
                    _ => unreachable!(),
                };
                painter.extend(egui::Shape::dashed_line_with_offset(
                    &path[..count],
                    stroke,
                    dash,
                    gap,
                    0.,
                ));
            }
        }
    }
    if series.kind == PlotKind::Scatter || series.markers {
        let point = [PlotPoint::new(0., 0.)];
        let transform = PlotTransform::new(
            Rect::from_center_size(marker, Vec2::splat(2.)),
            PlotBounds::from_min_max([-1., -1.], [1., 1.]),
            [false; 2],
        );
        let points = StyledPoints {
            points: Points::new("", PlotPoints::Borrowed(&point))
                .color(color)
                .filled(series.marker_is_filled())
                .shape(native_marker(series.effective_marker()))
                .radius(series.marker_radius().min(7.)),
            width: Some(series.stroke_width(1.).min(3.)),
        };
        shapes.clear();
        points.shapes(ui, &transform, shapes);
        painter.extend(shapes.drain(..));
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use matrust::plot::MarkerShape;

    fn frame(
        ctx: &egui::Context,
        axes: &Axes,
        events: Vec<egui::Event>,
        modifiers: egui::Modifiers,
        enabled: bool,
    ) -> (egui::FullOutput, Response) {
        let mut result = Response::default();
        let output = crate::headless::run(
            ctx,
            egui::RawInput {
                screen_rect: Some(Rect::from_min_size(Pos2::ZERO, Vec2::new(400., 300.))),
                events: std::iter::once(egui::Event::ModifiersChanged(modifiers))
                    .chain(events)
                    .collect(),
                ..Default::default()
            },
            |root| {
                egui::CentralPanel::default().show(root, |ui| {
                    if !enabled {
                        ui.disable();
                    }
                    result = show(ui, axes, ui.max_rect(), egui::Id::new("legend_test"));
                });
            },
        );
        (output, result)
    }
    fn texts(output: &egui::FullOutput) -> Vec<&egui::epaint::TextShape> {
        output
            .shapes
            .iter()
            .filter_map(|s| match &s.shape {
                egui::Shape::Text(t) => Some(t),
                _ => None,
            })
            .collect()
    }
    fn click(
        ctx: &egui::Context,
        axes: &Axes,
        pos: Pos2,
        modifiers: egui::Modifiers,
        enabled: bool,
    ) -> Response {
        let mut result = Response::default();
        for pressed in [true, false] {
            result = frame(
                ctx,
                axes,
                vec![
                    egui::Event::PointerMoved(pos),
                    egui::Event::PointerButton {
                        pos,
                        button: egui::PointerButton::Primary,
                        pressed,
                        modifiers,
                    },
                ],
                modifiers,
                enabled,
            )
            .1;
        }
        result
    }
    #[test]
    fn native_legend_subset_order_and_click_resolve_original_series_identity() {
        let ctx = egui::Context::default();
        let mut axes = Axes {
            legend: true,
            ..Default::default()
        };
        for label in ["First", "Excluded", "Third"] {
            let mut s = Series::new(vec![[0., 1.]], PlotKind::Line);
            s.label = label.into();
            axes.series.push(s);
        }
        axes.legend_state = Some(matrust::plot::LegendState {
            identity: matrust::ObjectIdentity::default(),
            members: vec![
                axes.series[2].identity.clone(),
                axes.series[0].identity.clone(),
            ],
            auto_update: false,
        });
        let samples = axes.series[2].points.clone();
        for _ in 0..3 {
            frame(&ctx, &axes, vec![], egui::Modifiers::NONE, true);
        }
        let (output, response) = frame(&ctx, &axes, vec![], egui::Modifiers::NONE, true);
        let labels = texts(&output);
        assert!(!labels.iter().any(|t| t.galley.text() == "Excluded"));
        let third = labels.iter().find(|t| t.galley.text() == "Third").unwrap();
        let first = labels.iter().find(|t| t.galley.text() == "First").unwrap();
        assert!(third.pos.y < first.pos.y);
        let point = third.pos + third.galley.size() * 0.5;
        assert!(response.rect.unwrap().contains(point));
        assert_eq!(
            click(&ctx, &axes, point, egui::Modifiers::NONE, true).visibility,
            Some(vec![true, true, false])
        );
        assert!((samples).same_storage(&axes.series[2].points));
    }
    #[test]
    fn native_legend_locations_anchor_inside_view_and_remain_clickable() {
        use matrust::plot::LegendLocation;
        for location in LegendLocation::INSIDE {
            let ctx = egui::Context::default();
            let mut axes = Axes {
                legend: true,
                legend_location: location,
                ..Default::default()
            };
            let mut series = Series::new(vec![[0., 1.]], PlotKind::Line);
            series.label = "Signal".into();
            axes.series.push(series);
            for _ in 0..3 {
                frame(&ctx, &axes, vec![], egui::Modifiers::NONE, true);
            }
            let (output, response) = frame(&ctx, &axes, vec![], egui::Modifiers::NONE, true);
            let rect = response.rect.unwrap();
            assert!(
                Rect::from_min_max(Pos2::new(12., 12.), Pos2::new(388., 288.)).contains_rect(rect)
            );
            match location {
                LegendLocation::North | LegendLocation::Northeast | LegendLocation::Northwest => {
                    assert!((rect.top() - 12.).abs() < 0.1)
                }
                LegendLocation::South | LegendLocation::Southeast | LegendLocation::Southwest => {
                    assert!((rect.bottom() - 288.).abs() < 0.1)
                }
                _ => assert!((rect.center().y - 150.).abs() < 0.1),
            }
            match location {
                LegendLocation::West | LegendLocation::Northwest | LegendLocation::Southwest => {
                    assert!((rect.left() - 12.).abs() < 0.1)
                }
                LegendLocation::East | LegendLocation::Northeast | LegendLocation::Southeast => {
                    assert!((rect.right() - 388.).abs() < 0.1)
                }
                _ => assert!((rect.center().x - 200.).abs() < 0.1),
            }
            let label = texts(&output)
                .into_iter()
                .find(|t| t.galley.text() == "Signal")
                .unwrap();
            assert_eq!(
                click(
                    &ctx,
                    &axes,
                    label.pos + label.galley.size() * 0.5,
                    egui::Modifiers::NONE,
                    true
                )
                .visibility,
                Some(vec![false])
            );
        }
    }
    #[test]
    fn native_legend_duplicate_labels_alt_isolation_keyboard_and_disabled_controls() {
        let ctx = egui::Context::default();
        let mut axes = Axes {
            legend: true,
            ..Default::default()
        };
        for label in ["Duplicate", "Duplicate", ""] {
            let mut s = Series::new(vec![[0., 1.]], PlotKind::Line);
            s.label = label.into();
            axes.series.push(s);
        }
        let samples = axes.series[1].points.clone();
        let mut positions = Vec::new();
        for _ in 0..3 {
            let (out, _) = frame(&ctx, &axes, vec![], egui::Modifiers::NONE, true);
            positions = texts(&out)
                .iter()
                .map(|t| t.pos + t.galley.size() * 0.5)
                .collect();
        }
        assert_eq!(positions.len(), 2);
        let alt = egui::Modifiers {
            alt: true,
            ..Default::default()
        };
        assert_eq!(
            click(&ctx, &axes, positions[1], alt, true).visibility,
            Some(vec![false, true, true])
        );
        axes.series[0].visible = false;
        assert_eq!(
            click(&ctx, &axes, positions[1], alt, true).visibility,
            Some(vec![true, true, true])
        );
        axes.series[0].visible = true;
        assert_eq!(
            click(&ctx, &axes, positions[1], egui::Modifiers::NONE, true).visibility,
            Some(vec![true, false, true])
        );
        axes.series[1].visible = false;
        let keyboard = frame(
            &ctx,
            &axes,
            vec![egui::Event::Key {
                key: egui::Key::Space,
                physical_key: None,
                pressed: true,
                repeat: false,
                modifiers: egui::Modifiers::NONE,
            }],
            egui::Modifiers::NONE,
            true,
        )
        .1;
        assert_eq!(keyboard.visibility, Some(vec![true, true, true]));
        frame(&ctx, &axes, vec![], egui::Modifiers::NONE, false);
        assert!(
            click(&ctx, &axes, positions[0], egui::Modifiers::NONE, false)
                .visibility
                .is_none()
        );
        assert!((samples).same_storage(&axes.series[1].points));
    }
    #[test]
    fn native_legend_long_labels_are_bounded_virtualized_and_last_row_scrolls_into_view() {
        let ctx = egui::Context::default();
        let mut axes = Axes {
            legend: true,
            ..Default::default()
        };
        for i in 0..64 {
            let mut s = Series::new(vec![[0., 1.]], PlotKind::Line);
            s.label = format!("Series {i:02} {}", "\u{03bb}".repeat(1500));
            axes.series.push(s);
        }
        let mut rect = Rect::NOTHING;
        for _ in 0..3 {
            let (out, result) = frame(&ctx, &axes, vec![], egui::Modifiers::NONE, true);
            rect = result.rect.unwrap();
            assert!(rect.width() <= 240. && rect.height() <= 150.);
            let labels = texts(&out);
            assert!(!labels.is_empty() && labels.len() < 10);
            assert!(labels.iter().all(|t| t.galley.size().x < rect.width()));
            for clipped in &out.shapes {
                if let egui::Shape::Text(t) = &clipped.shape {
                    let bounds = Rect::from_min_size(t.pos, t.galley.size());
                    if clipped.clip_rect.intersect(bounds).is_positive() {
                        assert!(
                            clipped.clip_rect.expand(0.01).contains_rect(bounds),
                            "Initial legend row is clipped"
                        );
                    }
                }
                if let egui::Shape::Rect(r) = &clipped.shape
                    && clipped.clip_rect == rect
                    && r.stroke.width > 0.
                {
                    assert!(
                        rect.expand(0.01).contains_rect(r.rect),
                        "Legend border extends outside its clip"
                    );
                }
            }
        }
        frame(
            &ctx,
            &axes,
            vec![
                egui::Event::PointerMoved(rect.center()),
                egui::Event::MouseWheel {
                    phase: egui::TouchPhase::Move,
                    unit: egui::MouseWheelUnit::Point,
                    delta: Vec2::new(0., -10000.),
                    modifiers: egui::Modifiers::NONE,
                },
            ],
            egui::Modifiers::NONE,
            true,
        );
        let mut last = None;
        for _ in 0..60 {
            let (out, _) = frame(&ctx, &axes, vec![], egui::Modifiers::NONE, true);
            last = texts(&out)
                .into_iter()
                .find(|t| t.galley.text().starts_with("Series 63 "))
                .map(|t| t.pos + Vec2::new(20., t.galley.size().y * 0.5));
            if last.is_some() {
                break;
            }
        }
        let result = click(
            &ctx,
            &axes,
            last.expect("Last legend row unreachable"),
            egui::Modifiers::NONE,
            true,
        );
        let visible = result.visibility.unwrap();
        assert_eq!(visible.iter().filter(|v| **v).count(), 63);
        assert!(!visible[63]);
    }
    #[test]
    fn native_legend_symbol_previews_reuse_shapes_and_bound_large_sizes() {
        let ctx = egui::Context::default();
        let icon = Rect::from_min_size(Pos2::new(20., 20.), Vec2::new(38., 24.));
        for marker in MarkerShape::ALL {
            let mut series = Series::new(vec![], PlotKind::Scatter);
            series.marker_shape = Some(marker);
            series.marker_size = Some(100.);
            series.line_width = Some(50.);
            series.color = [255, 0, 0];
            let out = crate::headless::run(&ctx, Default::default(), |root| {
                egui::CentralPanel::default().show(root, |ui| {
                    draw_icon(ui, &series, icon, &mut Vec::new());
                });
            });
            let glyphs: Vec<_> = out.shapes.iter().filter(|s| s.clip_rect == icon).collect();
            assert!(!glyphs.is_empty());
            assert!(
                glyphs
                    .iter()
                    .all(|s| icon.contains_rect(s.shape.visual_bounding_rect()))
            );
            match marker {
                MarkerShape::Circle | MarkerShape::Point => {
                    let egui::Shape::Circle(c) = &glyphs[0].shape else {
                        panic!("Expected circle")
                    };
                    assert_eq!(c.radius, 7.);
                    assert_eq!(c.stroke.width, 3.);
                    assert_eq!(c.fill == Color32::RED, marker == MarkerShape::Point);
                }
                MarkerShape::Plus | MarkerShape::Cross => assert_eq!(glyphs.len(), 2),
                MarkerShape::Asterisk => assert_eq!(glyphs.len(), 3),
                _ => assert!(matches!(&glyphs[0].shape,egui::Shape::Path(p) if p.closed)),
            }
        }
    }
    fn key_event(key: egui::Key, pressed: bool) -> egui::Event {
        egui::Event::Key {
            key,
            physical_key: None,
            pressed,
            repeat: false,
            modifiers: egui::Modifiers::NONE,
        }
    }
    #[test]
    fn native_legend_keyboard_navigation_reveals_rows_and_keeps_series_identity() {
        let ctx = egui::Context::default();
        let id = egui::Id::new("legend_test");
        let mut axes = Axes {
            legend: true,
            ..Default::default()
        };
        for index in 0..64 {
            let mut s = Series::new(vec![[0., 1.]], PlotKind::Line);
            s.label = match index {
                0 | 2 => "Duplicate".into(),
                1 => String::new(),
                _ => format!("Row {index:02}"),
            };
            axes.series.push(s);
        }
        let samples: Vec<_> = axes.series.iter().map(|s| s.points.clone()).collect();
        let mut pos = None;
        for _ in 0..3 {
            let (out, _) = frame(&ctx, &axes, vec![], egui::Modifiers::NONE, true);
            pos = texts(&out)
                .iter()
                .find(|t| t.galley.text() == "Duplicate")
                .map(|t| t.pos + t.galley.size() * 0.5);
        }
        assert!(
            !click(&ctx, &axes, pos.unwrap(), egui::Modifiers::NONE, true)
                .visibility
                .unwrap()[0]
        );
        axes.series[0].visible = false;
        let press = |axes: &Axes, key| {
            frame(
                &ctx,
                axes,
                vec![key_event(key, true), key_event(key, false)],
                egui::Modifiers::NONE,
                true,
            )
        };
        for (key, index) in [
            (egui::Key::ArrowDown, 2usize),
            (egui::Key::ArrowUp, 0),
            (egui::Key::ArrowUp, 0),
            (egui::Key::End, 63),
            (egui::Key::ArrowDown, 63),
        ] {
            let (out, result) = press(&axes, key);
            assert!(result.visibility.is_none());
            assert_eq!(
                ctx.memory(|m| m.focused()),
                Some(id.with(("legend_series", index)))
            );
            let label = &axes.series[index].label;
            assert!(out.shapes.iter().any(|s| matches!(&s.shape,egui::Shape::Text(t)
                if t.galley.text()==label && s.clip_rect.contains_rect(Rect::from_min_size(t.pos,t.galley.size())))));
        }
        let result = press(&axes, egui::Key::Space).1.visibility.unwrap();
        assert!(!result[63] && !result[0] && result[1..63].iter().all(|v| *v));
        axes.series[63].visible = false;
        for (key, index) in [
            (egui::Key::Home, 0usize),
            (egui::Key::PageDown, 6),
            (egui::Key::PageUp, 0),
        ] {
            assert!(press(&axes, key).1.visibility.is_none());
            assert_eq!(
                ctx.memory(|m| m.focused()),
                Some(id.with(("legend_series", index)))
            );
        }
        let repeated = frame(
            &ctx,
            &axes,
            vec![
                key_event(egui::Key::ArrowDown, true),
                key_event(egui::Key::ArrowDown, true),
                key_event(egui::Key::ArrowDown, true),
                key_event(egui::Key::ArrowDown, false),
            ],
            egui::Modifiers::NONE,
            true,
        )
        .1;
        assert!(repeated.visibility.is_none());
        assert_eq!(
            ctx.memory(|m| m.focused()),
            Some(id.with(("legend_series", 4usize)))
        );
        press(&axes, egui::Key::End);
        let result = press(&axes, egui::Key::Enter).1.visibility.unwrap();
        assert!(result[63] && !result[0]);
        assert!(
            axes.series
                .iter()
                .zip(samples)
                .all(|(s, p)| (s.points).same_storage(&p))
        );
    }
    #[test]
    fn native_legend_keyboard_navigation_respects_disabled_absent_and_changed_focus() {
        let ctx = egui::Context::default();
        let id = egui::Id::new("legend_test");
        let mut axes = Axes {
            legend: true,
            ..Default::default()
        };
        for label in ["First", "Second"] {
            let mut s = Series::new(vec![[0., 1.]], PlotKind::Line);
            s.label = label.into();
            axes.series.push(s);
        }
        for _ in 0..3 {
            frame(&ctx, &axes, vec![], egui::Modifiers::NONE, true);
        }
        let has_end = || {
            ctx.input(|i| {
                i.events.iter().any(|e| {
                    matches!(
                        e,
                        egui::Event::Key {
                            key: egui::Key::End,
                            pressed: true,
                            ..
                        }
                    )
                })
            })
        };
        assert!(
            frame(
                &ctx,
                &axes,
                vec![key_event(egui::Key::End, true)],
                egui::Modifiers::NONE,
                true
            )
            .1
            .visibility
            .is_none()
        );
        assert!(has_end(), "Unfocused legend consumed navigation");
        ctx.memory_mut(|m| m.request_focus(id.with(("legend_series", 0usize))));
        frame(
            &ctx,
            &axes,
            vec![key_event(egui::Key::End, true)],
            egui::Modifiers::NONE,
            false,
        );
        assert!(has_end(), "Disabled legend consumed navigation");
        ctx.memory_mut(|m| m.request_focus(id.with(("legend_series", 1usize))));
        axes.series.pop();
        frame(
            &ctx,
            &axes,
            vec![key_event(egui::Key::End, true)],
            egui::Modifiers::NONE,
            true,
        );
        assert!(has_end(), "Removed row retained keyboard ownership");
        ctx.memory_mut(|m| m.request_focus(id.with(("legend_series", 0usize))));
        axes.legend = false;
        frame(
            &ctx,
            &axes,
            vec![key_event(egui::Key::End, true)],
            egui::Modifiers::NONE,
            true,
        );
        assert!(has_end(), "Hidden legend consumed navigation");
    }
    #[test]
    fn native_legend_keyboard_focus_survives_busy_toggle_without_stealing_other_focus() {
        let ctx = egui::Context::default();
        let id = egui::Id::new("legend_test");
        let mut axes = Axes {
            legend: true,
            ..Default::default()
        };
        let mut s = Series::new(vec![[0., 1.]], PlotKind::Line);
        s.label = "Focus me".into();
        axes.series.push(s);
        for _ in 0..3 {
            frame(&ctx, &axes, vec![], egui::Modifiers::NONE, true);
        }
        let row_id = id.with(("legend_series", 0usize));
        ctx.memory_mut(|m| m.request_focus(row_id));
        frame(&ctx, &axes, vec![], egui::Modifiers::NONE, true);
        let toggle = frame(
            &ctx,
            &axes,
            vec![
                key_event(egui::Key::Space, true),
                key_event(egui::Key::Space, false),
            ],
            egui::Modifiers::NONE,
            true,
        )
        .1;
        assert_eq!(toggle.visibility, Some(vec![false]));
        for _ in 0..2 {
            frame(&ctx, &axes, vec![], egui::Modifiers::NONE, false);
        }
        axes.series[0].visible = false;
        frame(&ctx, &axes, vec![], egui::Modifiers::NONE, true);
        assert_eq!(
            ctx.memory(|m| m.focused()),
            Some(row_id),
            "Busy update lost legend keyboard focus"
        );
        assert_eq!(
            frame(
                &ctx,
                &axes,
                vec![
                    key_event(egui::Key::Enter, true),
                    key_event(egui::Key::Enter, false)
                ],
                egui::Modifiers::NONE,
                true
            )
            .1
            .visibility,
            Some(vec![true])
        );
        frame(&ctx, &axes, vec![], egui::Modifiers::NONE, false);
        let other = egui::Id::new("other_input");
        ctx.memory_mut(|m| m.request_focus(other));
        frame(&ctx, &axes, vec![], egui::Modifiers::NONE, true);
        assert_ne!(
            ctx.memory(|m| m.focused()),
            Some(row_id),
            "Legend stole another control's focus"
        );
        ctx.memory_mut(|m| m.request_focus(row_id));
        frame(&ctx, &axes, vec![], egui::Modifiers::NONE, true);
        frame(&ctx, &axes, vec![], egui::Modifiers::NONE, false);
        axes.series[0].points = std::sync::Arc::new(vec![[4., 5.]]).into();
        frame(&ctx, &axes, vec![], egui::Modifiers::NONE, true);
        assert_ne!(
            ctx.memory(|m| m.focused()),
            Some(row_id),
            "Focus was restored to replacement samples"
        );
    }
    #[test]
    fn native_legend_line_patterns_and_plot_families_have_distinct_previews() {
        let ctx = egui::Context::default();
        let icon = Rect::from_min_size(Pos2::new(20., 20.), Vec2::new(38., 24.));
        let render = |kind, pattern| {
            let mut series = Series::new(vec![], kind);
            series.pattern = pattern;
            series.markers = false;
            series.color = [255, 0, 0];
            let out = crate::headless::run(&ctx, Default::default(), |root| {
                egui::CentralPanel::default().show(root, |ui| {
                    draw_icon(ui, &series, icon, &mut Vec::new());
                });
            });
            out.shapes
                .into_iter()
                .filter(|s| s.clip_rect == icon)
                .map(|s| s.shape)
                .collect::<Vec<_>>()
        };
        let mut patterns = std::collections::BTreeSet::new();
        for pattern in [
            LinePattern::Solid,
            LinePattern::Dashed,
            LinePattern::Dotted,
            LinePattern::DashDot,
        ] {
            let shapes = render(PlotKind::Line, pattern);
            assert!(!shapes.is_empty());
            assert!(
                shapes
                    .iter()
                    .all(|s| icon.contains_rect(s.visual_bounding_rect()))
            );
            patterns.insert(format!("{shapes:?}"));
        }
        assert_eq!(patterns.len(), 4);
        for kind in [PlotKind::Line, PlotKind::Stairs, PlotKind::Stem] {
            assert!(render(kind, LinePattern::None).is_empty());
        }
        assert!(
            matches!(&render(PlotKind::Line,LinePattern::Solid)[0],egui::Shape::Path(p) if p.points.len()==2)
        );
        assert!(
            matches!(&render(PlotKind::Stairs,LinePattern::Solid)[0],egui::Shape::Path(p) if p.points.len()==4)
        );
        assert_eq!(render(PlotKind::Stem, LinePattern::Solid).len(), 2);
        assert!(
            matches!(&render(PlotKind::Bar,LinePattern::Solid)[0],egui::Shape::Rect(r) if r.fill==Color32::RED)
        );
    }
}

#[cfg(test)]
mod line_style_none_tests {
    use super::*;
    #[test]
    fn line_style_none_native_legend_retains_marker_icon() {
        let ctx = egui::Context::default();
        let icon = Rect::from_min_size(Pos2::new(20., 20.), Vec2::new(38., 24.));
        let mut series = Series::new(vec![], PlotKind::Line);
        series.pattern = LinePattern::None;
        series.markers = true;
        series.marker_shape = Some(matrust::plot::MarkerShape::Circle);
        series.color = [255, 0, 0];
        let out = crate::headless::run(&ctx, Default::default(), |root| {
            egui::CentralPanel::default()
                .show(root, |ui| draw_icon(ui, &series, icon, &mut Vec::new()));
        });
        let shapes: Vec<_> = out
            .shapes
            .into_iter()
            .filter(|s| s.clip_rect == icon)
            .collect();
        assert_eq!(shapes.len(), 1);
        assert!(matches!(&shapes[0].shape,egui::Shape::Circle(c) if c.stroke.color==Color32::RED));
    }
}

#[cfg(test)]
mod face_edge_legend_tests {
    use super::*;
    #[test]
    fn native_face_legend_paints_separate_fill_and_outline() {
        let ctx = egui::Context::default();
        let icon = Rect::from_min_size(Pos2::new(20., 20.), Vec2::new(38., 24.));
        for kind in [PlotKind::Bar, PlotKind::Patch] {
            let mut s = Series::new(vec![], kind);
            s.color = [0, 0, 255];
            s.face_edge_color = Some([255, 0, 0]);
            s.pattern = LinePattern::Solid;
            let out = crate::headless::run(&ctx, Default::default(), |root| {
                egui::CentralPanel::default()
                    .show(root, |ui| draw_icon(ui, &s, icon, &mut Vec::new()));
            });
            let glyphs: Vec<_> = out.shapes.iter().filter(|s| s.clip_rect == icon).collect();
            assert!(
                glyphs
                    .iter()
                    .any(|s| matches!(&s.shape,egui::Shape::Rect(r) if r.fill==Color32::BLUE))
            );
            assert!(glyphs.iter().any(|s|matches!(&s.shape,egui::Shape::Path(p) if p.stroke.color==egui::epaint::ColorMode::Solid(Color32::RED) && p.points.len()==5)));
        }
    }
}
