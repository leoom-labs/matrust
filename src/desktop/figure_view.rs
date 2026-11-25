use super::graphics_overlay::{self, MarkupLayout};
use crate::theme::ThemedScrollArea;
#[path = "figure_axes_style.rs"]
mod axes_style;
#[path = "figure_legend.rs"]
mod legend;
use eframe::egui::emath::GuiRounding;
use eframe::egui::{self, Color32};
use egui_plot::{
    Bar, BarChart, Line, Plot, PlotBounds, PlotGeometry, PlotItem, PlotItemBase, PlotPoint,
    PlotPoints, PlotTransform, Points,
};
use matrust::plot::{
    Axes, AxisScale, AxisTransform, BaseTheme, Figure, LinePattern, PlotKind, ResolvedAppearance,
    SampleQuery,
};
use std::borrow::Cow;

#[cfg(test)]
pub(super) const CUSTOM_TICK_LAYOUT_COUNT: &str = "custom_tick_layout_count";

fn linear_tick(mark: egui_plot::GridMark) -> String {
    let value = mark.value;
    let step = mark.step_size.abs();
    if !value.is_finite() || !step.is_finite() || step == 0. {
        return String::new();
    }
    if value == 0. {
        return "0".into();
    }
    let order = step.log10().floor();
    // Half-major marks may be 0.25 or 2.5, not just powers of ten. Keep an
    // extra decimal and trim it, instead of rounding these ticks to integers.
    if value.abs() >= 1e5 || value.abs() < 1e-3 {
        let digits = (value.abs().log10().floor() - order + 1.).clamp(0., 16.) as usize;
        let text = format!("{value:.digits$e}");
        let (mantissa, exponent) = text.split_once('e').unwrap();
        let mantissa = if mantissa.contains('.') {
            mantissa.trim_end_matches('0').trim_end_matches('.')
        } else {
            mantissa
        };
        format!("{mantissa}e{exponent}")
    } else {
        let digits = (1. - order).clamp(0., 17.) as usize;
        let text = format!("{value:.digits$}");
        if text.contains('.') {
            text.trim_end_matches('0').trim_end_matches('.').into()
        } else {
            text
        }
    }
}

pub struct View {
    pub bounds: [f64; 4],
    pub auto: [bool; 2],
    pub visibility: Option<Vec<bool>>,
    pub pick: Option<SampleQuery>,
    /// A 3-D axes: its shown limits are projected screen coordinates, never
    /// x/y limits to record, and a left drag rotates the view instead.
    pub projected: bool,
    /// Left-drag movement in points over a 3-D axes this frame (dx, dy);
    /// callers apply it with `matrust::plot::view3d::drag_view`.
    pub rotate: Option<[f64; 2]>,
    /// Updated `sliceViewer` slice index from slider, wheel, or arrow keys.
    pub slice_number: Option<usize>,
}
#[derive(Default)]
pub struct PanelContext {
    pub marker: Option<[f64; 2]>,
    pub canvas: Option<egui::Rect>,
    pub appearance: ResolvedAppearance,
}
/// Worker-prepared full-model bounds and retained display geometry.
#[derive(Clone, Copy)]
pub struct PreparedView<'a> {
    pub points: &'a [Vec<PlotPoint>],
    pub point_indices: Option<&'a [Option<Vec<usize>>]>,
    pub axes: [AxisTransform; 2],
    pub bounds: ([f64; 2], [f64; 2]),
    pub legend: Option<&'a matrust::plot::LegendBestPrepared>,
}
struct PreparedLayout<'a> {
    bounds: ([f64; 2], [f64; 2]),
    legend: Option<&'a matrust::plot::LegendBestPrepared>,
    point_indices: Option<&'a [Option<Vec<usize>>]>,
    plot_id: Option<egui::Id>,
    fit: bool,
}
struct SceneContext<'a> {
    panel: PanelContext,
    ordinary_title: bool,
    bounds: Option<([f64; 2], [f64; 2])>,
    legend: Option<&'a matrust::plot::LegendBestPrepared>,
    point_indices: Option<&'a [Option<Vec<usize>>]>,
}
#[cfg(test)]
pub fn show_interactive(
    ui: &mut egui::Ui,
    figure: &Figure,
    points: &[Vec<PlotPoint>],
    axes: [AxisTransform; 2],
    reset: bool,
    marker: Option<[f64; 2]>,
) -> View {
    show_interactive_impl(ui, figure, points, None, axes, reset, marker, None)
}
pub fn show_interactive_indexed(
    ui: &mut egui::Ui,
    figure: &Figure,
    points: &[Vec<PlotPoint>],
    point_indices: Option<&[Option<Vec<usize>>]>,
    axes: [AxisTransform; 2],
    reset: bool,
    marker: Option<[f64; 2]>,
) -> View {
    show_interactive_impl(ui, figure, points, point_indices, axes, reset, marker, None)
}
pub fn show_interactive_prepared(
    ui: &mut egui::Ui,
    figure: &Figure,
    prepared: PreparedView<'_>,
    reset: bool,
    marker: Option<[f64; 2]>,
) -> View {
    show_interactive_impl(
        ui,
        figure,
        prepared.points,
        prepared.point_indices,
        prepared.axes,
        reset,
        marker,
        Some(PreparedLayout {
            bounds: prepared.bounds,
            legend: prepared.legend,
            point_indices: prepared.point_indices,
            plot_id: None,
            fit: false,
        }),
    )
}
/// Render one prepared figure under a caller-owned, stable view identity.
/// The caller must include the live figure identity/generation, not its tile slot.
/// This scopes ancillary widgets as well as PlotMemory, without changing selection.
// The future tiled surface is a separate layer; legacy callers retain their IDs.
#[cfg_attr(not(test), allow(dead_code))]
pub fn show_interactive_prepared_with_id(
    ui: &mut egui::Ui,
    plot_id: egui::Id,
    figure: &Figure,
    prepared: PreparedView<'_>,
    reset: bool,
    marker: Option<[f64; 2]>,
) -> View {
    show_prepared_with_id(ui, plot_id, figure, prepared, reset, marker, false)
}
/// Fit a prepared ordinary scene into the caller's finite canvas, retaining the
/// same view identity and manual bounds as the readable-size entrypoint.
#[cfg_attr(not(test), allow(dead_code))]
pub fn show_interactive_prepared_fitted_with_id(
    ui: &mut egui::Ui,
    plot_id: egui::Id,
    figure: &Figure,
    prepared: PreparedView<'_>,
    reset: bool,
    marker: Option<[f64; 2]>,
) -> View {
    show_prepared_with_id(ui, plot_id, figure, prepared, reset, marker, true)
}
fn show_prepared_with_id(
    ui: &mut egui::Ui,
    plot_id: egui::Id,
    figure: &Figure,
    prepared: PreparedView<'_>,
    reset: bool,
    marker: Option<[f64; 2]>,
    fit: bool,
) -> View {
    let mut scope = egui::UiBuilder::new().id(plot_id.with("prepared_scene"));
    if fit {
        scope = scope.max_rect(ui.available_rect_before_wrap().intersect(ui.clip_rect()));
    }
    ui.scope_builder(scope, |ui| {
        show_interactive_impl(
            ui,
            figure,
            prepared.points,
            prepared.point_indices,
            prepared.axes,
            reset,
            marker,
            Some(PreparedLayout {
                bounds: prepared.bounds,
                legend: prepared.legend,
                point_indices: prepared.point_indices,
                plot_id: Some(plot_id),
                fit,
            }),
        )
    })
    .inner
}
#[allow(clippy::too_many_arguments)]
fn show_interactive_impl(
    ui: &mut egui::Ui,
    figure: &Figure,
    points: &[Vec<PlotPoint>],
    point_indices: Option<&[Option<Vec<usize>>]>,
    axes: [AxisTransform; 2],
    reset: bool,
    marker: Option<[f64; 2]>,
    prepared: Option<PreparedLayout<'_>>,
) -> View {
    let fit = prepared.as_ref().is_some_and(|p| p.fit);
    let explicit_id = prepared.as_ref().and_then(|p| p.plot_id);
    let (bounds, legend, point_indices) = prepared.map_or((None, None, point_indices), |p| {
        (Some(p.bounds), p.legend, p.point_indices.or(point_indices))
    });
    let plot_id = explicit_id.unwrap_or_else(|| {
        if bounds.is_some() {
            egui::Id::new(("detached_figure", ui.ctx().viewport_id()))
        } else {
            egui::Id::new("workspace_figure")
        }
    });
    let appearance = figure.resolve_appearance(host_theme(ui.ctx()));
    let render = |ui: &mut egui::Ui, canvas: egui::Rect| {
        paint_figure_background(ui, figure, canvas);
        let sv = figure.slice_viewers.first().filter(|v| v.visible);
        let (axes_canvas, slider_rect) = if sv.is_some() && canvas.height() > 60.0 {
            let split = canvas.bottom() - 26.0;
            (
                egui::Rect::from_min_max(canvas.min, egui::pos2(canvas.right(), split)),
                Some(egui::Rect::from_min_max(
                    egui::pos2(canvas.left(), split),
                    canvas.max,
                )),
            )
        } else {
            (canvas, None)
        };
        let mut view = if let Some(p) = figure.placement {
            let request = if p.imported_solved
                || p.constraint == matrust::plot::PositionConstraint::InnerPosition
            {
                p.requested_inner
            } else {
                p.requested_outer
            };
            let panel = egui::Rect::from_min_size(
                axes_canvas.min
                    + egui::vec2(
                        request[0] as f32 * axes_canvas.width(),
                        (1. - request[1] - request[3]) as f32 * axes_canvas.height(),
                    ),
                egui::vec2(
                    request[2] as f32 * axes_canvas.width(),
                    request[3] as f32 * axes_canvas.height(),
                ),
            );
            ui.scope_builder(egui::UiBuilder::new().max_rect(panel), |ui| {
                show_axes_impl(
                    ui,
                    &figure.axes,
                    points,
                    axes,
                    reset,
                    plot_id,
                    SceneContext {
                        ordinary_title: true,
                        panel: PanelContext {
                            marker,
                            canvas: Some(axes_canvas),
                            appearance,
                        },
                        bounds,
                        legend,
                        point_indices,
                    },
                )
            })
            .inner
        } else {
            ui.scope_builder(egui::UiBuilder::new().max_rect(axes_canvas), |ui| {
                show_axes_impl(
                    ui,
                    &figure.axes,
                    points,
                    axes,
                    reset,
                    plot_id,
                    SceneContext {
                        ordinary_title: true,
                        panel: PanelContext {
                            marker,
                            canvas: None,
                            appearance,
                        },
                        bounds,
                        legend,
                        point_indices,
                    },
                )
            })
            .inner
        };
        if let (Some(sv), Some(s_rect)) = (sv, slider_rect) {
            view.slice_number = slice_slider_and_input(
                ui,
                s_rect,
                axes_canvas,
                sv.slice_number,
                sv.max_slice(),
                plot_id.with("slice_slider"),
            );
        }
        // Annotations belong to the complete figure canvas, including ordinary
        // one-axes documents. Paint after axes without changing stored positions.
        paint_textboxes(ui, figure, canvas);
        paint_annotations(ui, figure, canvas);
        view
    };
    let request = figure.placement.map(|p| {
        if p.imported_solved || p.constraint == matrust::plot::PositionConstraint::InnerPosition {
            p.requested_inner
        } else {
            p.requested_outer
        }
    });
    if !fit && figure.legend && figure.legend_location.is_outside() {
        // Scrolling children have an unbounded available extent. Allocate the
        // complete scene first, then give geometry and painting that finite box.
        // Ordinary docked panes need no artificial 420 x 280 scroll minimum.
        let minimum = request.map_or(egui::vec2(200., 160.), |p| {
            egui::vec2(200. / p[2] as f32, 160. / p[3] as f32)
        });
        let available = ui.available_size();
        let size = egui::vec2(available.x.max(minimum.x), available.y.max(minimum.y));
        return egui::ScrollArea::both()
            .id_salt("outside_legend_scene")
            .auto_shrink([false, false])
            .show_themed(ui, |ui| {
                let (_, scene) = ui.allocate_space(size);
                let mut child = ui.new_child(egui::UiBuilder::new().max_rect(scene));
                child.set_clip_rect(scene.intersect(ui.clip_rect()));
                render(&mut child, scene)
            })
            .inner;
    }
    let canvas = ui.available_rect_before_wrap();
    render(ui, canvas)
}
pub(super) fn paint_textboxes(ui: &egui::Ui, figure: &Figure, area: egui::Rect) {
    let canvas = [
        f64::from(area.left()),
        f64::from(area.top()),
        f64::from(area.right()),
        f64::from(area.bottom()),
    ];
    for textbox in &figure.textboxes {
        if !textbox.visible || textbox.position[2] == 0. || textbox.position[3] == 0. {
            continue;
        }
        let [left, top, right, bottom] = textbox.rectangle(canvas);
        let clip = egui::Rect::from_min_max(
            egui::pos2(left as f32, top as f32),
            egui::pos2(right as f32, bottom as f32),
        )
        .intersect(ui.clip_rect());
        if !clip.is_positive() {
            continue;
        }
        let font = textbox.font_size as f32 * (4. / 3.);
        let (x, align) = match textbox.horizontal {
            matrust::plot::TextAlign::Left => (left, egui::Align2::LEFT_TOP),
            matrust::plot::TextAlign::Center => ((left + right) * 0.5, egui::Align2::CENTER_TOP),
            matrust::plot::TextAlign::Right => (right, egui::Align2::RIGHT_TOP),
        };
        let painter = ui.painter().with_clip_rect(clip);
        let color = egui::Color32::from_rgb(textbox.color[0], textbox.color[1], textbox.color[2]);
        for (line, text) in textbox.display_lines(right - left).into_iter().enumerate() {
            let y = top + f64::from(font) * line as f64 * 1.2;
            if y >= bottom {
                break;
            }
            let position = egui::pos2(x as f32, y as f32);
            // Egui's installed proportional family has no separate bold face;
            // small synthetic emboldening is a visible, documented approximation.
            if textbox.bold {
                painter.text(
                    position + egui::vec2(0.35, 0.),
                    align,
                    &text,
                    super::theme::figure_font(ui.ctx(), font),
                    color,
                );
            }
            painter.text(
                position,
                align,
                &text,
                super::theme::figure_font(ui.ctx(), font),
                color,
            );
        }
    }
}

pub(super) fn paint_annotations(ui: &egui::Ui, figure: &Figure, area: egui::Rect) {
    use matrust::plot::{Annotation, AnnotationKind};
    let canvas = [
        f64::from(area.left()),
        f64::from(area.top()),
        f64::from(area.right()),
        f64::from(area.bottom()),
    ];
    let painter = ui.painter().with_clip_rect(ui.clip_rect());
    let mut dash_budget = 16_384.0_f64;
    for ann in &figure.annotations {
        if !ann.visible {
            continue;
        }
        let stroke_color = egui::Color32::from_rgb(ann.color[0], ann.color[1], ann.color[2]);
        let stroke_width = (ann.line_width * (4.0 / 3.0)) as f32;
        let stroke = egui::Stroke::new(stroke_width, stroke_color);
        if ann.kind.is_box() {
            let [left, top, right, bottom] = ann.rectangle(canvas);
            let rect = egui::Rect::from_min_max(
                egui::pos2(left as f32, top as f32),
                egui::pos2(right as f32, bottom as f32),
            );
            let fill = match ann.face_color {
                Some(fc) if ann.face_alpha > 0.0 => {
                    let alpha = (ann.face_alpha * 255.0).round().clamp(0.0, 255.0) as u8;
                    egui::Color32::from_rgba_unmultiplied(fc[0], fc[1], fc[2], alpha)
                }
                _ => egui::Color32::TRANSPARENT,
            };
            let draw_edge = !ann.edge_none && ann.pattern != LinePattern::None;
            match ann.kind {
                AnnotationKind::Rectangle => {
                    if fill != egui::Color32::TRANSPARENT {
                        painter.rect_filled(rect, 0.0, fill);
                    }
                    if draw_edge {
                        if ann.pattern == LinePattern::Solid {
                            painter.rect_stroke(rect, 0.0, stroke, egui::StrokeKind::Middle);
                        } else {
                            let mut path = vec![
                                rect.left_top(),
                                rect.right_top(),
                                rect.right_bottom(),
                                rect.left_bottom(),
                                rect.left_top(),
                            ];
                            let mut shapes = Vec::new();
                            face_edge_path(
                                &mut path,
                                stroke,
                                ann.pattern,
                                &mut dash_budget,
                                &mut shapes,
                            );
                            for s in shapes {
                                painter.add(s);
                            }
                        }
                    }
                }
                AnnotationKind::Ellipse => {
                    let center = rect.center();
                    let radius = rect.size() * 0.5;
                    if fill != egui::Color32::TRANSPARENT {
                        painter.add(egui::epaint::EllipseShape::filled(center, radius, fill));
                    }
                    if draw_edge {
                        if ann.pattern == LinePattern::Solid {
                            painter.add(egui::epaint::EllipseShape::stroke(center, radius, stroke));
                        } else {
                            let steps = 48;
                            let mut path: Vec<_> = (0..=steps)
                                .map(|i| {
                                    let theta = (i as f32 / steps as f32) * std::f32::consts::TAU;
                                    egui::pos2(
                                        center.x + radius.x * theta.cos(),
                                        center.y + radius.y * theta.sin(),
                                    )
                                })
                                .collect();
                            let mut shapes = Vec::new();
                            face_edge_path(
                                &mut path,
                                stroke,
                                ann.pattern,
                                &mut dash_budget,
                                &mut shapes,
                            );
                            for s in shapes {
                                painter.add(s);
                            }
                        }
                    }
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
            let mut path = vec![
                egui::pos2(shaft_start[0] as f32, shaft_start[1] as f32),
                egui::pos2(shaft_end[0] as f32, shaft_end[1] as f32),
            ];
            let mut shapes = Vec::new();
            face_edge_path(
                &mut path,
                stroke,
                ann.pattern,
                &mut dash_budget,
                &mut shapes,
            );
            for s in shapes {
                painter.add(s);
            }
        }
        for (poly, _) in [head_start, head_end].into_iter().flatten() {
            let pts: Vec<_> = poly
                .into_iter()
                .map(|p| egui::pos2(p[0] as f32, p[1] as f32))
                .collect();
            if pts.len() == 3 {
                painter.add(egui::Shape::convex_polygon(
                    pts,
                    stroke_color,
                    egui::Stroke::NONE,
                ));
            } else if pts.len() == 4 {
                // Split notched vback2 polygon into two convex triangles.
                painter.add(egui::Shape::convex_polygon(
                    vec![pts[0], pts[1], pts[2]],
                    stroke_color,
                    egui::Stroke::NONE,
                ));
                painter.add(egui::Shape::convex_polygon(
                    vec![pts[0], pts[2], pts[3]],
                    stroke_color,
                    egui::Stroke::NONE,
                ));
            }
        }
        if ann.kind == AnnotationKind::TextArrow && !ann.text.is_empty() {
            let font = ann.font_size as f32 * (4.0 / 3.0);
            let dx = p1[0] - p0[0];
            let (tx, align) = if dx >= 0.0 {
                (p0[0] - 4.0, egui::Align2::RIGHT_CENTER)
            } else {
                (p0[0] + 4.0, egui::Align2::LEFT_CENTER)
            };
            painter.text(
                egui::pos2(tx as f32, p0[1] as f32),
                align,
                &ann.text,
                super::theme::figure_font(ui.ctx(), font),
                stroke_color,
            );
        }
    }
}

#[cfg(test)]
pub fn show_axes(
    ui: &mut egui::Ui,
    figure: &Axes,
    points: &[Vec<PlotPoint>],
    axes: [AxisTransform; 2],
    reset: bool,
    plot_id: egui::Id,
    context: PanelContext,
) -> View {
    show_axes_indexed(ui, figure, points, None, axes, reset, plot_id, context)
}
#[allow(clippy::too_many_arguments)]
pub fn show_axes_indexed(
    ui: &mut egui::Ui,
    figure: &Axes,
    points: &[Vec<PlotPoint>],
    point_indices: Option<&[Option<Vec<usize>>]>,
    axes: [AxisTransform; 2],
    reset: bool,
    plot_id: egui::Id,
    context: PanelContext,
) -> View {
    show_axes_impl(
        ui,
        figure,
        points,
        axes,
        reset,
        plot_id,
        SceneContext {
            ordinary_title: false,
            panel: context,
            bounds: None,
            legend: None,
            point_indices,
        },
    )
}
pub fn show_axes_prepared(
    ui: &mut egui::Ui,
    figure: &Axes,
    prepared: PreparedView<'_>,
    reset: bool,
    plot_id: egui::Id,
    context: PanelContext,
) -> View {
    show_axes_impl(
        ui,
        figure,
        prepared.points,
        prepared.axes,
        reset,
        plot_id,
        SceneContext {
            ordinary_title: false,
            panel: context,
            bounds: Some(prepared.bounds),
            legend: prepared.legend,
            point_indices: prepared.point_indices,
        },
    )
}
/// A 3-D axes paints its projected display copy. The prepared points were
/// projected by the worker; this copy then only supplies labels, limits and
/// the box/grid series list, so samples are not projected again per frame.
/// Without a prepared legend source the legend's `best` score reads this
/// copy's own series, so it is then projected in full.
fn painted_axes<'a>(figure: &'a Axes, context: &SceneContext<'_>) -> Cow<'a, Axes> {
    let appearance = context.panel.appearance;
    let scores_here = figure.legend
        && figure.legend_location == matrust::plot::LegendLocation::Best
        && context.legend.is_none()
        && context.bounds.is_none();
    if scores_here {
        matrust::plot::view3d::display_view_in(figure, appearance)
    } else {
        matrust::plot::view3d::display_frame_in(figure, appearance)
    }
}
fn show_axes_impl(
    ui: &mut egui::Ui,
    figure: &Axes,
    points: &[Vec<PlotPoint>],
    axes: [AxisTransform; 2],
    reset: bool,
    plot_id: egui::Id,
    context: SceneContext<'_>,
) -> View {
    let model = figure;
    let display = painted_axes(figure, &context);
    let figure = display.as_ref();
    let mut view = ui
        .scope(|ui| {
            apply_appearance(ui, context.panel.appearance);
            show_document_axes(ui, figure, points, axes, reset, plot_id, context)
        })
        .inner;
    if view.projected {
        // 3-D: the projected screen box is not a data x/y range. Report the
        // model's own limit modes (and manual limits) so stored views, script
        // reruns and dialogs never turn the screen box into XLim/YLim.
        view.auto = [model.xlim.is_none(), model.ylim.is_none()];
        for (axis, limit) in [model.xlim, model.ylim].into_iter().enumerate() {
            if let Some([lo, hi]) = limit {
                view.bounds[2 * axis] = lo;
                view.bounds[2 * axis + 1] = hi;
            }
        }
    }
    view
}
fn axes_title_galley(
    ui: &egui::Ui,
    nodes: &[matrust::plot::text_markup::Node],
    font: &egui::FontId,
    width: f32,
    rows: usize,
    color: Color32,
    emphasis: graphics_overlay::TextEmphasis,
) -> std::sync::Arc<egui::Galley> {
    let mut job = graphics_overlay::markup_job_from_nodes_styled(nodes, font, color, emphasis);
    job.halign = egui::Align::Center;
    job.wrap.max_width = (width - 16.).max(1.);
    job.wrap.max_rows = rows;
    job.wrap.break_anywhere = true;
    ui.painter().layout_job(job)
}

/// Only the outer short-pane scrollbar may grow ordinary automatic artwork.
/// Fixed axes geometry and tile Fit rendering keep their existing finite boxes.
pub(super) fn reserve_readable_title_canvas(ui: &mut egui::Ui, figure: &Figure) {
    if figure.placement.is_some()
        || (figure.title.is_empty() && figure.extras.subtitle.string.is_empty())
    {
        return;
    }
    let style = figure.extras.title_style;
    let emphasis = graphics_overlay::TextEmphasis::from(style);
    let mut font = super::theme::figure_body(ui.ctx());
    font.size = style.font_size_pixels(font.size);
    let line_height = ui.fonts_mut(|fonts| fonts.row_height(&font));
    let interpreter = figure.extras.label_interpreters[0];
    let nodes = matrust::plot::text_markup::parse_nodes(&figure.title, interpreter);
    let color = ui.visuals().text_color();
    let subtitle_height = graphics_overlay::layout_subtitle(
        ui.painter(),
        &figure.extras.subtitle,
        font.clone(),
        color,
    )
    .map_or(0., |(layout, _)| {
        layout.size.y + if figure.title.is_empty() { 0. } else { 4. }
    });
    let title_height = if figure.title.is_empty() {
        0.
    } else if let Some(layout) = graphics_overlay::layout_structured_markup_styled(
        ui.painter(),
        &nodes,
        &font,
        color,
        emphasis,
    ) {
        layout.size.y
    } else {
        let row_limit = if figure.title.contains('\n') { 256 } else { 3 };
        let galley = axes_title_galley(
            ui,
            &nodes,
            &font,
            ui.available_width(),
            row_limit,
            color,
            emphasis,
        );
        line_height * galley.rows.len().min(row_limit) as f32
    };
    // Use the same bounded rows and gutter as painting. One point covers
    // roundoff when the subsequent division determines the admitted row count.
    let minimum = (80. + 8. + line_height * 0.5 + title_height + subtitle_height).ceil() + 1.;
    ui.set_min_height(minimum);
}

// An axes title is document artwork, not a window or toolbar caption. This
// bounded band uses the existing finite canvas and does not allocate plot IDs or
// alter the document, prepared samples, transform state or history.
fn paint_axes_title(
    ui: &mut egui::Ui,
    axes: &Axes,
    panel: egui::Rect,
    appearance: ResolvedAppearance,
    plot_id: egui::Id,
    fixed_canvas: Option<egui::Rect>,
) -> Option<(egui::Rect, f32)> {
    let title = &axes.title;
    let subtitle = &axes.extras.subtitle;
    if title.is_empty() && subtitle.string.is_empty() {
        return Some((panel, 0.));
    }
    if !panel.is_finite() || !panel.is_positive() {
        return None;
    }
    let style = axes.extras.title_style;
    let emphasis = graphics_overlay::TextEmphasis::from(style);
    let mut font = super::theme::figure_body(ui.ctx());
    font.size = style.font_size_pixels(font.size);
    let bold = emphasis.bold;
    let line_height = ui.fonts_mut(|fonts| fonts.row_height(&font));
    // Keep a half-line for a valid y tick centered on the top data edge.
    let padding = 8. + line_height * 0.5;
    let available_height =
        fixed_canvas.map_or(panel.height() - 80., |canvas| panel.top() - canvas.top());
    let color = rgb(appearance.text);
    let subheading = graphics_overlay::layout_subtitle(ui.painter(), subtitle, font.clone(), color);
    let gap = if !title.is_empty() && subheading.is_some() {
        4.
    } else {
        0.
    };
    let subtitle_height = subheading.as_ref().map_or(0., |(layout, _)| layout.size.y);
    let title_room = available_height - padding - subtitle_height - gap;
    if title_room < if title.is_empty() { 0. } else { line_height } || panel.width() <= 16. {
        return None;
    }
    let nodes = matrust::plot::text_markup::parse_nodes(title, axes.extras.label_interpreters[0]);
    let structured_layout = graphics_overlay::layout_structured_markup_styled(
        ui.painter(),
        &nodes,
        &font,
        color,
        emphasis,
    )
    .filter(|layout| layout.size.y <= title_room);
    let (title_height, plain_galley) = if title.is_empty() {
        (0., None)
    } else if let Some(layout) = &structured_layout {
        (layout.size.y, None)
    } else {
        let row_limit = if title.contains('\n') { 256 } else { 3 };
        let rows = ((title_room / line_height).floor() as usize).clamp(1, row_limit);
        let galley = axes_title_galley(ui, &nodes, &font, panel.width(), rows, color, emphasis);
        (galley.size().y, Some(galley))
    };
    let content_height = title_height + gap + subtitle_height;
    let height = content_height + padding;
    let top = if fixed_canvas.is_some() {
        panel.top() - height
    } else {
        panel.top()
    };
    let band = egui::Rect::from_min_size(
        egui::pos2(panel.left(), top),
        egui::vec2(panel.width(), height),
    );
    if fixed_canvas.is_some_and(|canvas| !canvas.contains_rect(band)) {
        return None;
    }
    ui.painter().rect_filled(band, 0., rgb(appearance.canvas));
    let clipped = ui.painter().with_clip_rect(band.intersect(ui.clip_rect()));
    if let Some(layout) = structured_layout {
        let top_left = egui::pos2(panel.center().x - layout.size.x * 0.5, band.top() + 4.);
        layout.with_bold(bold).paint(&clipped, top_left, color, 0.);
    } else if let Some(galley) = plain_galley {
        let position = egui::pos2(panel.center().x, band.top() + 4.);
        if bold {
            graphics_overlay::paint_bold_copy(&clipped, position, &galley, color);
        }
        clipped.galley(position, galley, color);
    }
    if let Some((layout, color)) = subheading {
        let top_left = egui::pos2(
            panel.center().x - layout.size.x / 2.,
            band.top() + 4. + title_height + gap,
        );
        layout.paint(&clipped, top_left, color, 0.);
    }
    ui.interact(band, plot_id.with("axes_title"), egui::Sense::hover())
        .on_hover_text(title);
    if fixed_canvas.is_some() {
        // Fixed InnerPosition/imported solved panels stay exactly where requested.
        // The outside-legend solver must reserve the title as a top decoration.
        Some((panel, height))
    } else {
        Some((
            egui::Rect::from_min_max(egui::pos2(panel.left(), band.bottom()), panel.max),
            0.,
        ))
    }
}

struct AxisArtwork {
    label: Option<MarkupLayout>,
    date: Option<MarkupLayout>,
    rotated_label_x: f32,
}

impl AxisArtwork {
    fn label_space(&self) -> f32 {
        self.label.as_ref().map_or(0., |label| label.size.y * 1.25)
    }

    fn reserve(&self) -> f32 {
        self.label_space() + self.date.as_ref().map_or(0., |date| date.size.y + 4.)
    }

    fn paint(
        &self,
        painter: &egui::Painter,
        frame: egui::Rect,
        plot: egui::Rect,
        horizontal: bool,
        [label_color, date_color]: [Color32; 2],
    ) {
        if horizontal {
            let label_top = plot.bottom() - self.label_space();
            if let Some(label) = &self.label {
                label.paint(
                    painter,
                    egui::pos2(frame.center().x - label.size.x / 2., label_top),
                    label_color,
                    0.,
                );
            }
            if let Some(date) = &self.date {
                date.paint(
                    painter,
                    egui::pos2(frame.left(), label_top - date.size.y - 4.),
                    date_color,
                    0.,
                );
            }
        } else {
            let angle = -std::f32::consts::FRAC_PI_2;
            if let Some(label) = &self.label {
                label.paint(
                    painter,
                    egui::pos2(self.rotated_label_x, frame.center().y + label.size.x / 2.),
                    label_color,
                    angle,
                );
            }
            if let Some(date) = &self.date {
                date.paint(
                    painter,
                    egui::pos2(plot.left() + self.label_space(), frame.bottom()),
                    date_color,
                    angle,
                );
            }
        }
    }
}

#[derive(Clone, Copy, PartialEq)]
struct TickGutters {
    // Both arrays are in x-axis, y-axis order (bottom, left).
    artwork: [f32; 2],
    // Space allocated by egui_plot, including the artwork thickness hint.
    allocated: Option<[f32; 2]>,
}

fn tick_label_shape(
    galley: std::sync::Arc<egui::Galley>,
    horizontal: bool,
    rotation: f64,
) -> egui::epaint::TextShape {
    // The position is relative to the tick's anchor on the data frame.
    let pos = if horizontal {
        egui::pos2(-galley.size().x / 2., if rotation == 0. { 0. } else { 4. })
    } else {
        egui::pos2(-4. - galley.size().x, -galley.size().y / 2.)
    };
    egui::epaint::TextShape::new(pos, galley, Color32::PLACEHOLDER)
        .with_angle(-(rotation as f32).to_radians())
}

fn show_document_axes(
    ui: &mut egui::Ui,
    figure: &Axes,
    points: &[Vec<PlotPoint>],
    axes: [AxisTransform; 2],
    reset: bool,
    plot_id: egui::Id,
    context: SceneContext<'_>,
) -> View {
    let SceneContext {
        panel:
            PanelContext {
                marker,
                canvas: layout_canvas,
                appearance,
            },
        ordinary_title,
        bounds: prepared_bounds,
        legend: prepared_legend,
        point_indices,
    } = context;
    let colors = figure.resolve_colors(appearance);
    let intent = figure.colors.unwrap_or_default();
    let panel = ui.available_rect_before_wrap();
    let fixed = figure.placement.is_some_and(|p| {
        p.imported_solved || p.constraint == matrust::plot::PositionConstraint::InnerPosition
    });
    let (panel, title_margin) = if ordinary_title {
        let fixed_canvas = fixed.then_some(layout_canvas.unwrap_or(panel));
        let Some(layout) = paint_axes_title(
            ui,
            figure,
            panel,
            ResolvedAppearance {
                text: colors.title,
                ..appearance
            },
            plot_id,
            fixed_canvas,
        ) else {
            return unavailable_layout_prepared(
                ui,
                figure,
                plot_id,
                "Axes title needs more room; enlarge the pane or choose Readable size.",
                axes,
                prepared_bounds,
            );
        };
        layout
    } else {
        (panel, 0.)
    };
    let to_model_rect = |r: egui::Rect| {
        matrust::plot::LegendRect::new([
            r.left() as f64,
            r.top() as f64,
            r.right() as f64,
            r.bottom() as f64,
        ])
    };
    let to_ui_rect = |r: matrust::plot::LegendRect| {
        egui::Rect::from_min_max(
            egui::pos2(r.left as f32, r.top as f32),
            egui::pos2(r.right as f32, r.bottom as f32),
        )
    };
    let mut outside = None;
    let mut plot_rect = panel;
    if figure.legend_location.is_outside() && figure.legend {
        let side = figure.legend_location.outside_side().unwrap();
        let budget = if side == 0 || side == 2 {
            egui::vec2(
                (panel.width() * 0.35).clamp(48., 240.).min(panel.width()),
                panel.height(),
            )
        } else {
            egui::vec2(
                panel.width(),
                (panel.height() * 0.35).clamp(32., 180.).min(panel.height()),
            )
        };
        let extent = legend::extent(
            ui,
            figure,
            egui::Rect::from_min_size(panel.min, budget),
            true,
        );
        if extent.is_none() && figure.legend_entries().next().is_some() {
            return unavailable_layout_prepared(
                ui,
                figure,
                plot_id,
                "Legend layout needs more room; enlarge the pane or choose Readable size.",
                axes,
                prepared_bounds,
            );
        }
        if let Some(extent) = extent {
            let outer = if fixed {
                layout_canvas.unwrap_or(panel)
            } else {
                panel
            };
            match matrust::plot::legend_geometry(
                to_model_rect(outer),
                to_model_rect(panel),
                [0., f64::from(title_margin), 0., 0.],
                Some([extent.x as f64, extent.y as f64]),
                figure.legend_location,
                fixed,
            ) {
                Ok(g) => {
                    plot_rect = to_ui_rect(g.data);
                    outside = g.legend.map(to_ui_rect);
                }
                Err(error) => {
                    // An outside request must never silently become an inside
                    // legend. Preserve the current view state while exposing
                    // the bounded-layout error; the editor can enlarge it.
                    return unavailable_layout_prepared(
                        ui,
                        figure,
                        plot_id,
                        &error,
                        axes,
                        prepared_bounds,
                    );
                }
            }
        }
    }
    let show_axes = !figure.ticks_hidden && !figure.frame_hidden && !figure.extras.axis_off;
    let memory = egui_plot::PlotMemory::load(ui.ctx(), plot_id);
    let previous_transform = memory.as_ref().map(egui_plot::PlotMemory::transform);
    // Prepared geometry can precede the first view. Initialize explicit limits
    // once for a new plot identity, while preserving later manual pan and zoom.
    let reset = reset || memory.is_none();
    let date_axes = [0, 1].map(|axis| {
        let ruler = &figure.extras.rulers[axis];
        show_axes
            && ruler.tick_label_format.is_none()
            && ruler.tick_labels.is_none()
            && matches!(ruler.kind, matrust::plot::RulerKind::Datetime { .. })
    });
    let manual_view = memory.as_ref().filter(|_| !reset);
    let manual_dates = std::array::from_fn::<_, 2, _>(|axis| {
        date_axes[axis] && manual_view.is_some_and(|view| !view.auto_bounds[axis])
    });
    // Use the export's model range for an automatic view and the current
    // ruler limits for a panned or zoomed view. Reuse worker-prepared bounds.
    let date_ranges = date_axes.contains(&true).then(|| {
        let (x, y) = prepared_bounds.unwrap_or_else(|| figure.bounds());
        std::array::from_fn::<_, 2, _>(|axis| {
            if let Some(view) = manual_view.filter(|view| !view.auto_bounds[axis]) {
                let bounds = view.bounds();
                [
                    axes[axis].inverse(bounds.min()[axis]),
                    axes[axis].inverse(bounds.max()[axis]),
                ]
            } else {
                [figure.xlim, figure.ylim][axis]
                    .filter(|limits| limits.iter().all(|value| value.is_finite()))
                    .unwrap_or([x, y][axis])
            }
        })
    });
    let label_colors = [rgb(colors.xlabel), rgb(colors.ylabel)];
    let tick_colors = [rgb(colors.x), rgb(colors.y)];
    let font = super::theme::figure_body(ui.ctx());
    let tick_font = egui::TextStyle::Body.resolve(ui.style());
    let tick_painter = ui.painter().clone();
    let ruler_galley = |axis: usize, label: &str| {
        tick_painter.layout_job(graphics_overlay::markup_job(
            label,
            figure.extras.tick_label_interpreter,
            tick_font.clone(),
            tick_colors[axis],
        ))
    };
    #[cfg(test)]
    let tick_layout_count = std::cell::Cell::new(0_usize);
    let tick_galley = |axis: usize, label: &str| {
        #[cfg(test)]
        tick_layout_count.set(tick_layout_count.get() + 1);
        ruler_galley(axis, label)
    };
    // Keep room for the plot and its existing overlays before adding axis artwork.
    plot_rect.min.y += graphics_overlay::top_reserve(figure).min(plot_rect.height().max(0.) * 0.4);
    plot_rect.max.y -=
        graphics_overlay::bottom_reserve(figure).min(plot_rect.height().max(0.) * 0.4);
    let right_reserve = graphics_overlay::right_reserve(figure);
    let right_gutter = |width: f32| right_reserve.min(width.max(0.) * 0.4);
    let artwork_budget = [
        plot_rect.height() - 64.,
        plot_rect.width() - right_gutter(plot_rect.width()) - 96.,
    ];
    // egui_plot places a rotated y label a quarter line outside its widget.
    let y_label_gutter = if figure.ylabel.is_empty() {
        0.
    } else {
        (ui.text_style_height(&egui::TextStyle::Body) * 0.25).min(plot_rect.width().max(0.) * 0.25)
    };
    let [(x_label, x_artwork), (y_label, mut y_artwork)] = std::array::from_fn(|axis| {
        let source = [&figure.xlabel, &figure.ylabel][axis];
        let style = figure.extras.label_styles[axis];
        let emphasis = graphics_overlay::TextEmphasis::from(style);
        let mut label_font = font.clone();
        label_font.size = style.font_size_pixels(font.size);
        let nodes = matrust::plot::text_markup::parse_nodes(
            source,
            figure.extras.label_interpreters[axis + 1],
        );
        let date = date_ranges
            .filter(|_| date_axes[axis])
            .and_then(|ranges| matrust::plot::time_axes::secondary_date_label(ranges[axis]));
        let mut label = show_axes
            .then(|| {
                graphics_overlay::layout_structured_markup_styled(
                    ui.painter(),
                    &nodes,
                    &label_font,
                    label_colors[axis],
                    emphasis,
                )
            })
            .flatten();
        let structured = label.is_some();
        let flat_label = || {
            graphics_overlay::markup_job_from_nodes_styled(
                &nodes,
                &label_font,
                label_colors[axis],
                emphasis,
            )
        };
        let mut job = egui::text::LayoutJob::default();
        if label.is_none() {
            job = flat_label();
            if show_axes && (date.is_some() || emphasis.bold) && !job.text.is_empty() {
                label = Some(
                    MarkupLayout::from_galley(ui.painter().layout_job(std::mem::take(&mut job)))
                        .with_bold(emphasis.bold),
                );
            }
        }
        let date = date.map(|date| MarkupLayout::from_galley(ruler_galley(axis, &date)));
        let rotated_label_x = label.as_ref().map_or(0., |label| {
            let quarter_height = label.size.y * 0.25;
            if structured {
                plot_rect.left() + quarter_height
            } else {
                // Native axis widgets round their allocated rect before placing the label.
                (plot_rect.left() + y_label_gutter).round_ui() - quarter_height
            }
        });
        let mut artwork = AxisArtwork {
            label,
            date,
            rotated_label_x,
        };
        if artwork.reserve() > artwork_budget[axis] {
            artwork.date = None;
            // Plain labels only need an external band when sharing it with a date.
            if artwork.label.is_some()
                && (!structured || artwork.label_space() > artwork_budget[axis])
            {
                artwork.label = None;
                job = flat_label();
            }
        }
        (job, artwork)
    });
    if y_artwork.label.is_none() {
        plot_rect.min.x += y_label_gutter;
    }
    plot_rect.max.x -= right_gutter(plot_rect.width());
    let mut artwork_rect = plot_rect;
    let artwork_reserve = [x_artwork.reserve(), y_artwork.reserve()];
    plot_rect.max.y -= artwork_reserve[0];
    plot_rect.min.x += artwork_reserve[1];
    // Leave room at the right edge for the last tick label.
    plot_rect.max = plot_rect.min
        + egui::vec2(
            (plot_rect.width() - 32.).max(1.),
            plot_rect.height().max(1.),
        );
    let hidden: Vec<_> = figure
        .series
        .iter()
        .enumerate()
        .filter(|(_, s)| !s.visible)
        .map(|(i, _)| egui::Id::new(("series", i)))
        .collect();
    if let Some(mut memory) = memory {
        memory.hidden_items = hidden.iter().copied().collect();
        if !figure.legend {
            memory.hovered_legend_item = None;
        }
        memory.store(ui.ctx(), plot_id);
    }
    let square = figure.extras.square_plot_box;
    let mut plot = Plot::new("figure")
        .id(plot_id)
        .x_axis_label(x_label.clone())
        .y_axis_label(y_label.clone())
        .show_grid([figure.xgrid, figure.ygrid])
        .show_axes([show_axes; 2])
        .invert_y(figure.extras.y_reversed);
    if appearance.theme == BaseTheme::Dark {
        plot = plot.grid_color(rgb(appearance.grid));
    }
    if intent.x.is_some() {
        plot = plot.custom_x_axes(vec![
            egui_plot::AxisHints::new_x()
                .label(x_label.clone())
                .tick_label_color(rgb(colors.x)),
        ]);
    }
    if intent.y.is_some() {
        plot = plot.custom_y_axes(vec![
            egui_plot::AxisHints::new_y()
                .label(y_label.clone())
                .tick_label_color(rgb(colors.y)),
        ]);
    }
    if intent.xlabel.is_some() || intent.x.is_some() {
        plot = plot.x_axis_label(x_label.clone());
    }
    if intent.ylabel.is_some() || intent.y.is_some() {
        plot = plot.y_axis_label(y_label.clone());
    }
    if intent.grid.is_some() || !appearance.legacy || figure.extras.layer.is_top() {
        // The library exposes one grid color; this explicit decoration supplies
        // both directional colors under the series using the same final transform.
        // Layer 'top' repaints egui's grid after the children instead.
        plot = plot.show_grid(false);
    }
    if figure.extras.equal_aspect {
        plot = plot.data_aspect(1.);
    }
    if figure.extras.axis_off {
        plot = plot
            .show_grid(false)
            .show_background(false)
            .x_axis_label("")
            .y_axis_label("");
    }
    if figure.frame_hidden {
        // 3-D: the projected box replaces the 2-D frame; a left drag rotates.
        // Projected units are isotropic: keep them square (as print does) so a
        // tall or wide tile letterboxes the box instead of stretching it.
        plot = plot
            .data_aspect(1.)
            .show_background(false)
            .show_grid(false)
            .allow_scroll(false)
            .allow_zoom(false)
            .allow_drag(false)
            .allow_boxed_zoom(false)
            .allow_double_click_reset(false)
            .show_x(false)
            .show_y(false);
    }
    for baseline in &figure.baselines {
        if baseline.style.visible
            && baseline.style.pattern != LinePattern::None
            && let Ok(value) = baseline.value.resolve(axes[1])
        {
            plot = plot.include_y(axes[1].forward(value));
        }
    }
    plot = plot
        .x_grid_spacer(move |input| axes_style::marks(axes[0], input))
        .y_grid_spacer(move |input| axes_style::marks(axes[1], input));
    const MIN_ARTWORK_TICK_GUTTER: f32 = 16.;
    // egui_plot allocates axis space before resolving bounds and interactions.
    // Feed the measured thickness from the last frame back into that allocation.
    let gutter_id = plot_id.with("artwork_tick_gutters");
    let default_gutters = [MIN_ARTWORK_TICK_GUTTER; 2];
    let stored_gutters = ui
        .data(|data| data.get_temp::<TickGutters>(gutter_id))
        .unwrap_or(TickGutters {
            artwork: default_gutters,
            allocated: None,
        });
    let previous_gutters = if reset {
        default_gutters
    } else {
        stored_gutters.artwork
    };
    let mut fit_gutters = if show_axes {
        stored_gutters.allocated.unwrap_or([14.; 2])
    } else {
        [0.; 2]
    };
    let mut painted_ticks: [Option<Vec<(f64, String)>>; 2] = [None, None];
    let mut custom_grid: [Option<(Vec<f64>, f64)>; 2] = [None, None];
    for axis in 0..2 {
        let ruler = &figure.extras.rulers[axis];
        let custom = ruler.ticks.is_some()
            || ruler.tick_labels.is_some()
            || ruler.exponent != 0
            || ruler.tick_label_format.is_some()
            || ruler.tick_label_rotation != 0.
            || !ruler.kind.is_numeric()
            || (axis == 0 && figure.extras.leaf_ticks.is_some());
        if custom {
            let (ticks, step, labels) = if let Some(ranges) = date_ranges
                && date_axes[axis]
                && let matrust::plot::RulerKind::Datetime { format, .. } = &ruler.kind
            {
                // Match the model's calendar tick and format selection for the shown range.
                let (ticks, step, auto_format) =
                    matrust::plot::time_axes::datetime_ticks(ranges[axis]);
                let (ticks, step) = if ruler.ticks.is_some() {
                    figure.effective_ticks(axis)
                } else {
                    (ticks, step)
                };
                let format = if auto_format == matrust::datetime_array::DEFAULT_DATE_FORMAT
                    && !format.is_empty()
                {
                    format
                } else {
                    auto_format
                };
                let labels = ticks
                    .iter()
                    .map(|&tick| matrust::plot::time_axes::format_datetime_tick(tick, format))
                    .collect();
                (ticks, step, labels)
            } else {
                let (ticks, step) = figure.effective_ticks(axis);
                (ticks, step, figure.effective_tick_labels(axis))
            };
            let values: Vec<f64> = ticks.into_iter().map(|v| axes[axis].forward(v)).collect();
            let marks = values.clone();
            custom_grid[axis] = Some((values.clone(), step));
            let labels_data = values.into_iter().zip(labels).collect::<Vec<_>>();
            let hints = if axis == 0 {
                egui_plot::AxisHints::new_x()
            } else {
                egui_plot::AxisHints::new_y()
            };
            let hints = if artwork_reserve[axis] > 0. {
                // The custom tick shapes supply the gutter beside artwork.
                let room = if axis == 0 {
                    plot_rect.height()
                } else {
                    plot_rect.width()
                };
                let thickness = previous_gutters[axis].min((room - 16.).max(14.));
                if show_axes {
                    fit_gutters[axis] = fit_gutters[axis].max(thickness);
                }
                hints
                    .min_thickness(thickness)
                    .formatter(|_, _| String::new())
            } else {
                // Keep native gutter measurement when there is no artwork band.
                let labels_data = labels_data.clone();
                let interpreter = figure.extras.tick_label_interpreter;
                hints
                    .tick_label_color(Color32::TRANSPARENT)
                    .formatter(move |mark, _| {
                        labels_data
                            .iter()
                            .find(|(v, _)| *v == mark.value)
                            .map(|(_, label)| {
                                matrust::plot::text_markup::parse(label, interpreter)
                                    .into_iter()
                                    .map(|run| run.text)
                                    .collect()
                            })
                            .unwrap_or_default()
                    })
            };
            painted_ticks[axis] = Some(labels_data);
            let spacer =
                move |input: egui_plot::GridInput| axes_style::custom_marks(&marks, step, input);
            if axis == 0 {
                plot = plot
                    .custom_x_axes(vec![hints.label(x_label.clone())])
                    .x_grid_spacer(spacer);
            } else {
                plot = plot
                    .custom_y_axes(vec![hints.label(y_label.clone())])
                    .y_grid_spacer(spacer);
            }
        } else if axis == 0 {
            if axes[0].scale == AxisScale::Log {
                plot = plot.x_axis_formatter(move |mark, _| axes[0].label(mark.value));
            } else {
                plot = plot.x_axis_formatter(|mark, _| linear_tick(mark));
            }
        } else if axes[1].scale == AxisScale::Log {
            plot = plot.y_axis_formatter(move |mark, _| axes[1].label(mark.value));
        } else {
            plot = plot.y_axis_formatter(|mark, _| linear_tick(mark));
        }
    }
    let decoration = axes_style::Decoration::new(
        figure,
        axes,
        colors,
        appearance,
        ui.visuals().widgets.noninteractive.bg_stroke,
        custom_grid,
    )
    .filter(|_| !figure.frame_hidden && !figure.extras.axis_off);
    let (decoration, top_grid) = match decoration {
        Some(decoration) if figure.extras.layer.is_top() => decoration.split_layer(),
        decoration => (decoration, None),
    };
    let tick_marks = axes_style::TickMarks::new(
        figure,
        axes,
        colors,
        ui.visuals().widgets.noninteractive.bg_stroke,
        &painted_ticks,
    );
    // Layer 'bottom' paints the inside part of the ticks under the children;
    // the outside part, and every part for 'top', is painted after the plot.
    let late_tick_part = if figure.extras.layer.is_top() {
        axes_style::TickPart::All
    } else {
        axes_style::TickPart::Outside
    };
    if square {
        // Centre the square within the space available to the data frame.
        // The native ruler gutters remain outside that square.
        let data_size = plot_rect.size() - egui::vec2(fit_gutters[1], fit_gutters[0]);
        let side = data_size.min_elem();
        if side >= 1. {
            let inset = (data_size - egui::Vec2::splat(side)) * 0.5;
            plot_rect = plot_rect.shrink2(inset);
            artwork_rect.min.x += inset.x;
            artwork_rect.max.y -= inset.y;
            y_artwork.rotated_label_x += inset.x;
        }
    }
    // Set both dimensions explicitly to keep the widget within the layout allocation.
    plot = plot.width(plot_rect.width()).height(plot_rect.height());
    plot = plot.label_formatter(move |position| {
        let (name, point, near) = match position {
            egui_plot::HoverPosition::NearDataPoint {
                plot_name,
                position,
                ..
            } => (*plot_name, *position, true),
            egui_plot::HoverPosition::Elsewhere { position } => ("", *position, false),
        };
        let y = axes[1].inverse(point.y);
        // A yyaxis right series reports its own value, not the left-axis one.
        let y = if near {
            super::graphics_overlay::hover_y(figure, points, name, point, y)
        } else {
            y
        };
        Some(format!(
            "{name}\nx = {}\ny = {}",
            hover_number(axes[0].inverse(point.x)),
            hover_number(y)
        ))
    });
    let over_legend = figure.legend
        && ui
            .ctx()
            .data(|d| {
                d.get_temp::<Option<egui::Rect>>(plot_id.with("legend_rect"))
                    .flatten()
            })
            .is_some_and(|rect| {
                ui.input(|i| i.pointer.hover_pos().is_some_and(|p| rect.contains(p)))
            });
    if over_legend {
        plot = plot
            .allow_scroll(false)
            .allow_zoom(false)
            .allow_drag(false)
            .allow_boxed_zoom(false)
            .show_x(false)
            .show_y(false);
    }
    if reset {
        plot = plot.reset();
    }
    let response = ui
        .scope_builder(egui::UiBuilder::new().max_rect(plot_rect), |host| {
            if intent.background.is_some() {
                host.visuals_mut().extreme_bg_color = rgb(colors.background);
            }
            // Explicit ruler colors, box off, and explicit axes LineWidth draw their own axis lines.
            if intent.x.is_some()
                || intent.y.is_some()
                || !figure.box_outline
                || figure.extras.line_width.is_some()
            {
                host.visuals_mut().widgets.noninteractive.bg_stroke = egui::Stroke::NONE;
            }
            plot.show(host, |ui| {
                if let Some(decoration) = decoration {
                    ui.add(decoration);
                }
                if late_tick_part != axes_style::TickPart::All
                    && let Some(ticks) = tick_marks.as_ref()
                {
                    ui.add(ticks.clone());
                }
                if reset {
                    let (x, y) = prepared_bounds.unwrap_or_else(|| figure.bounds());
                    let (effective_x, effective_y) = figure.bounds();
                    let x = if figure.xlim.is_some() {
                        effective_x
                    } else {
                        x
                    };
                    let y = if figure.ylim.is_some() {
                        effective_y
                    } else {
                        y
                    };
                    if figure.xlim.is_some() {
                        ui.set_plot_bounds_x(axes[0].forward(x[0])..=axes[0].forward(x[1]));
                    }
                    if figure.ylim.is_some() {
                        ui.set_plot_bounds_y(axes[1].forward(y[0])..=axes[1].forward(y[1]));
                    }
                    if !figure.extras.images.is_empty() {
                        // Images fill the axes exactly, as in the exported PNG.
                        ui.set_plot_bounds_x(axes[0].forward(x[0])..=axes[0].forward(x[1]));
                        ui.set_plot_bounds_y(axes[1].forward(y[0])..=axes[1].forward(y[1]));
                    }
                }
                let mut volumes = super::figure_volumes::add_background(ui, figure, plot_id)
                    .into_iter()
                    .peekable();
                super::figure_images::add(ui, figure, plot_id);
                for (i, (s, points)) in figure.series.iter().zip(points).enumerate() {
                    let color = rgb(appearance.series_color(s));
                    let id = egui::Id::new(("series", i));
                    if !s.visible {
                        continue;
                    }
                    let series_base = if matches!(s.kind, PlotKind::Bar | PlotKind::Stem) {
                        figure
                            .render_baseline(i, axes[1])
                            .expect("Validated prepared baseline")
                    } else {
                        0.
                    };
                    match s.kind {
                        PlotKind::Line | PlotKind::Stairs | PlotKind::Stem => {
                            let line = Line::new(&s.label, PlotPoints::from(points.as_slice()))
                                .id(id)
                                .color(color)
                                .width(s.stroke_width(1.7));
                            ui.add(StyledLine {
                                line,
                                width: s.stroke_width(1.7),
                                pattern: s.pattern,
                                dash_budget: 200000. / figure.series.len().max(1) as f64,
                            });
                            if s.markers {
                                let markers = match s.kind {
                                    PlotKind::Stairs => PlotPoints::Owned(
                                        points.iter().step_by(2).copied().collect(),
                                    ),
                                    PlotKind::Stem => PlotPoints::Owned(
                                        points.iter().skip(1).step_by(3).copied().collect(),
                                    ),
                                    _ => PlotPoints::from(points.as_slice()),
                                };
                                ui.add(StyledPoints {
                                    width: (s.line_width.is_some() || s.has_marker_style())
                                        .then(|| s.stroke_width(1.)),
                                    points: Points::new(&s.label, markers)
                                        .id(id)
                                        .color(color)
                                        .filled(s.marker_is_filled())
                                        .shape(native_marker(s.effective_marker()))
                                        .radius(s.marker_radius()),
                                });
                            }
                        }
                        PlotKind::Scatter if s.has_scatter_style() => {
                            let map = figure.extras.effective_colormap();
                            let clim = figure.extras.color_limits();
                            let axes_bg = rgb(figure.resolve_colors(appearance).background);
                            let base_rgb = appearance.series_color(s);
                            let face_alpha = s.marker_face_alpha.unwrap_or(1.);
                            let edge_alpha = s.marker_edge_alpha.unwrap_or(1.);
                            let decimated_indices = point_indices
                                .and_then(|all| all.get(i))
                                .and_then(|v| v.as_deref());
                            let mut items = Vec::with_capacity(points.len());
                            for (idx, p) in points.iter().enumerate() {
                                if !p.x.is_finite() || !p.y.is_finite() {
                                    continue;
                                }
                                let orig_idx = decimated_indices
                                    .and_then(|indices| indices.get(idx).copied())
                                    .unwrap_or(idx);
                                let cdata_rgb = s.scatter_color_at(orig_idx, base_rgb, &map, clim);
                                let fill = match s.marker_face_color {
                                    matrust::plot::MarkerColor::None => Color32::TRANSPARENT,
                                    matrust::plot::MarkerColor::Flat => {
                                        with_alpha(cdata_rgb, face_alpha)
                                    }
                                    matrust::plot::MarkerColor::Auto => with_alpha(
                                        [axes_bg.r(), axes_bg.g(), axes_bg.b()],
                                        face_alpha,
                                    ),
                                    matrust::plot::MarkerColor::Rgb(c) => with_alpha(c, face_alpha),
                                    matrust::plot::MarkerColor::Default => {
                                        if s.marker_is_filled() {
                                            with_alpha(cdata_rgb, face_alpha)
                                        } else {
                                            Color32::TRANSPARENT
                                        }
                                    }
                                };
                                let stroke_color = match s.marker_edge_color {
                                    matrust::plot::MarkerColor::None => Color32::TRANSPARENT,
                                    matrust::plot::MarkerColor::Default
                                    | matrust::plot::MarkerColor::Flat
                                    | matrust::plot::MarkerColor::Auto => {
                                        with_alpha(cdata_rgb, edge_alpha)
                                    }
                                    matrust::plot::MarkerColor::Rgb(c) => with_alpha(c, edge_alpha),
                                };
                                items.push(ScatterPoint {
                                    point: *p,
                                    radius: s.marker_radius_at(orig_idx),
                                    fill,
                                    stroke_color,
                                });
                            }
                            ui.add(ScatterPoints {
                                base: Points::new(&s.label, PlotPoints::from(points.as_slice()))
                                    .id(id)
                                    .color(color),
                                shape: s.effective_marker(),
                                stroke_width: s.stroke_width(1.),
                                items,
                            });
                        }
                        PlotKind::Scatter => ui.add(StyledPoints {
                            width: (s.line_width.is_some() || s.has_marker_style())
                                .then(|| s.stroke_width(1.)),
                            points: Points::new(&s.label, PlotPoints::from(points.as_slice()))
                                .id(id)
                                .color(color)
                                .filled(s.marker_is_filled())
                                .shape(native_marker(s.effective_marker()))
                                .radius(s.marker_radius()),
                        }),
                        PlotKind::Patch => {
                            let patch_color = if s.face_color_mode
                                == matrust::plot::FaceColorMode::Flat
                                && let Some(c) = s.patch_cdata
                            {
                                rgb(matrust::plot::colormap::scaled_rgb(
                                    &figure.extras.effective_colormap(),
                                    figure.color_limits(),
                                    c,
                                ))
                            } else {
                                color
                            };
                            ui.add(StyledPatch {
                                line: Line::new(&s.label, PlotPoints::from(points.as_slice()))
                                    .id(id)
                                    .color(patch_color),
                                width: s.patch.map_or(0, |p| p.vertices_per_face()),
                                edge: s
                                    .face_edge_color
                                    .map(|c| with_alpha(c, s.edge_alpha.unwrap_or(1.))),
                                stroke_width: s.face_edge_width(),
                                pattern: s.pattern,
                                dash_budget: 200000. / figure.series.len().max(1) as f64,
                                bar: None,
                                fill: !s.face_color_none,
                                surface: SurfaceStyle::of(s),
                                interp: InterpFill::of(s, figure, points.len()),
                            });
                        }
                        PlotKind::Bar => {
                            let base_rgb = appearance.series_color(s);
                            let alpha = s.face_alpha.unwrap_or(1.);
                            let stacked_edge = (s.bar_layout == matrust::plot::BarLayout::Stacked)
                                .then_some(s.face_edge_color)
                                .flatten()
                                .filter(|_| s.pattern != LinePattern::None)
                                .map(|edge| {
                                    egui::Stroke::new(
                                        s.face_edge_width(),
                                        Color32::from_rgb(edge[0], edge[1], edge[2]),
                                    )
                                })
                                .unwrap_or(egui::Stroke::NONE);
                            ui.bar_chart(
                                BarChart::new(
                                    &s.label,
                                    points
                                        .iter()
                                        .enumerate()
                                        .filter(|(_, p)| {
                                            axes[0].accepts(p.x) && axes[1].accepts(p.y)
                                        })
                                        .filter_map(|(pt_idx, p)| {
                                            let (base_val, top_val) = if s.bar_layout
                                                == matrust::plot::BarLayout::Stacked
                                            {
                                                let [b, t] = figure
                                                    .bar_y_span(i, pt_idx, axes[1])
                                                    .ok()
                                                    .flatten()?;
                                                (b, t)
                                            } else {
                                                (series_base, p.y)
                                            };
                                            let l = axes[0]
                                                .forward(p.x + s.bar_offset - s.bar_width / 2.);
                                            let r = axes[0]
                                                .forward(p.x + s.bar_offset + s.bar_width / 2.);
                                            let l = if l.is_finite() {
                                                l
                                            } else {
                                                ui.plot_bounds().min()[0]
                                            };
                                            let r = if r.is_finite() {
                                                r
                                            } else {
                                                ui.plot_bounds().max()[0]
                                            };
                                            let baseline = axes[1].forward(base_val);
                                            let top = axes[1].forward(top_val);
                                            let fill = match s.face_color_mode {
                                                matrust::plot::FaceColorMode::None => {
                                                    Color32::TRANSPARENT
                                                }
                                                matrust::plot::FaceColorMode::Flat => {
                                                    let rgb = s
                                                        .face_colors
                                                        .as_ref()
                                                        .and_then(|c| c.get(pt_idx).copied())
                                                        .unwrap_or(base_rgb);
                                                    with_alpha(rgb, alpha)
                                                }
                                                matrust::plot::FaceColorMode::Color => {
                                                    with_alpha(base_rgb, alpha)
                                                }
                                            };
                                            Some(
                                                Bar::new((l + r) * 0.5, top - baseline)
                                                    .base_offset(baseline)
                                                    .width((r - l).max(0.))
                                                    .fill(fill)
                                                    .stroke(stacked_edge),
                                            )
                                        })
                                        .collect(),
                                )
                                .id(id)
                                .color(color),
                            );
                            if s.bar_layout != matrust::plot::BarLayout::Stacked
                                && let Some(edge) = s.face_edge_color
                            {
                                ui.add(StyledPatch {
                                    line: Line::new("", PlotPoints::from(points.as_slice()))
                                        .color(color),
                                    width: 1,
                                    edge: Some(Color32::from_rgb(edge[0], edge[1], edge[2])),
                                    stroke_width: s.face_edge_width(),
                                    pattern: s.pattern,
                                    dash_budget: 200000. / figure.series.len().max(1) as f64,
                                    bar: Some((axes, s.bar_width, s.bar_offset, series_base)),
                                    fill: false,
                                    surface: None,
                                    interp: None,
                                });
                            }
                        }
                    }
                    while let Some((_, image)) = volumes.next_if(|(after, _)| *after == i + 1) {
                        ui.add(image);
                    }
                }
                let mut baselines = Vec::new();
                for (s, p) in figure.series.iter().zip(points) {
                    if s.visible
                        && s.kind == PlotKind::Stem
                        && figure.baseline_for_series(&s.identity).is_none()
                        && p.iter().any(|p| p.x.is_finite() && p.y.is_finite())
                    {
                        let baseline = axes[1].forward(s.stem_baseline(axes[1]));
                        if baseline.is_finite() && !baselines.contains(&baseline) {
                            baselines.push(baseline);
                        }
                    }
                }
                let legacy_style = matrust::plot::BaselineStyle::legacy_stem();
                let rgb = legacy_style.display_rgb();
                for baseline in baselines {
                    ui.hline(
                        egui_plot::HLine::new("", baseline)
                            .color(Color32::from_rgb(rgb[0], rgb[1], rgb[2]))
                            .width(legacy_style.display_width() as f32)
                            .allow_hover(false),
                    );
                }
            })
        })
        .inner;
    if square || outside.is_some() || artwork_reserve != [0.; 2] {
        ui.advance_cursor_after_rect(panel);
    }
    let bounds = response.transform.bounds();
    let min = bounds.min();
    let max = bounds.max();
    let frame = *response.transform.frame();
    if artwork_reserve != [0.; 2] {
        let painter = ui.painter().with_clip_rect(panel.intersect(ui.clip_rect()));
        x_artwork.paint(
            &painter,
            frame,
            artwork_rect,
            true,
            [label_colors[0], tick_colors[0]],
        );
        y_artwork.paint(
            &painter,
            frame,
            artwork_rect,
            false,
            [label_colors[1], tick_colors[1]],
        );
    }
    if response.response.contains_pointer() {
        let pass = ui.ctx().cumulative_pass_nr();
        let tooltip = egui::Tooltip::tooltip_id(response.response.id, 0);
        let coordinates = egui::containers::AreaState::load(ui.ctx(), tooltip).map(|s| s.rect());
        let hover_id = hovered_plot_id(ui.ctx());
        ui.ctx()
            .data_mut(|d| d.insert_temp(hover_id, (pass, HoveredPlot { frame, coordinates })));
    }
    paint_baselines(ui, figure, axes[1], &response.transform);
    // Layer 'top' defers the yyaxis right axis until after the late tick marks.
    // SVG paints that axis after the top group; an x tick at xlim max shares
    // frame.right() and would otherwise cover the bottom of the axis line.
    let defer_right_axis = top_grid.is_some();
    graphics_overlay::paint(
        ui,
        figure,
        axes,
        &response.transform,
        appearance,
        (artwork_reserve[0] > 0.).then_some(artwork_rect.bottom()),
        defer_right_axis,
    );
    // Layer 'top': the grid sits above every axes child, including baselines and overlays.
    if !figure.frame_hidden
        && !figure.extras.axis_off
        && let Some(top_grid) = &top_grid
    {
        let grid_painter = ui.painter().with_clip_rect(frame.intersect(ui.clip_rect()));
        let mut shapes = Vec::new();
        top_grid.shapes(ui, &response.transform, &mut shapes);
        grid_painter.extend(shapes);
    }
    let mut pick = if ui.is_enabled()
        && response.response.clicked_by(egui::PointerButton::Primary)
        && ui.input(|i| i.modifiers.ctrl)
    {
        response
            .response
            .interact_pointer_pos()
            .filter(|p| frame.contains(*p))
            .map(|p| {
                let point = response.transform.value_from_position(p);
                SampleQuery {
                    transforms: axes,
                    bounds: [min[0], max[0], min[1], max[1]],
                    pointer: [point.x, point.y],
                    pixels: [frame.width() as f64, frame.height() as f64],
                }
            })
    } else {
        None
    };
    if let Some([x, y]) = marker {
        let point = PlotPoint::new(axes[0].forward(x), axes[1].forward(y));
        if point.x.is_finite() && point.y.is_finite() {
            let position = response.transform.position_from_point(&point);
            if frame.contains(position) {
                let painter = ui.painter().with_clip_rect(frame.intersect(ui.clip_rect()));
                painter.circle_filled(position, 5., Color32::WHITE);
                painter.circle_stroke(position, 5., egui::Stroke::new(2_f32, Color32::BLACK));
            }
        }
    }
    let mut measured_gutters = TickGutters {
        artwork: default_gutters,
        ..stored_gutters
    };
    if square && show_axes {
        // egui_plot measures its own labels and applies their thickness on the
        // next draw. Record the allocation that actually reached the data frame.
        let allocated = [
            (plot_rect.bottom() - frame.bottom()).max(0.),
            (frame.left() - plot_rect.left()).max(0.),
        ];
        let changed = stored_gutters.allocated.is_none_or(|previous| {
            (0..2).any(|axis| (previous[axis] - allocated[axis]).abs() > 0.1)
        });
        if changed {
            measured_gutters.allocated = Some(allocated);
        }
        if changed
            || reset
            || previous_transform
                .is_some_and(|view| *view.frame() != frame || view.bounds() != bounds)
        {
            ui.ctx().request_repaint();
        }
    }
    if !figure.frame_hidden && !figure.extras.axis_off {
        if let Some(ticks) = &tick_marks {
            let tick_painter = ui.painter().with_clip_rect(panel.intersect(ui.clip_rect()));
            let mut segments = Vec::new();
            ticks.segments(&response.transform, late_tick_part, &mut segments);
            for (seg, stroke) in segments {
                tick_painter.line_segment(seg, stroke);
            }
        }
        if defer_right_axis {
            graphics_overlay::paint_right_axis(ui, figure, axes, &response.transform);
        }
        for axis in 0..2 {
            let exponent = figure.extras.rulers[axis].exponent;
            if exponent != 0 {
                let pos = if axis == 0 {
                    egui::pos2(frame.right(), frame.bottom() + 4.)
                } else {
                    egui::pos2(frame.left(), frame.top() - 16.)
                };
                ui.painter().text(
                    pos,
                    if axis == 0 {
                        egui::Align2::RIGHT_TOP
                    } else {
                        egui::Align2::LEFT_TOP
                    },
                    format!("×10^{exponent}"),
                    egui::FontId::default(),
                    if axis == 0 {
                        rgb(colors.x)
                    } else {
                        rgb(colors.y)
                    },
                );
            }
            if figure.ticks_hidden {
                continue;
            }
            let rotation = figure.extras.rulers[axis].tick_label_rotation;
            let Some(ticks_and_labels) = &painted_ticks[axis] else {
                continue;
            };
            let mut tick_clip = panel.intersect(ui.clip_rect());
            let measure_gutter = artwork_reserve[axis] > 0.;
            if measure_gutter {
                if axis == 0 {
                    tick_clip.max.y = tick_clip.max.y.min(plot_rect.bottom());
                } else {
                    tick_clip.min.x = tick_clip.min.x.max(plot_rect.left());
                }
            }
            let painter = ui.painter().with_clip_rect(tick_clip);
            for (value, label) in ticks_and_labels {
                if label.is_empty()
                    || !value.is_finite()
                    || *value < min[axis]
                    || *value > max[axis]
                {
                    continue;
                }
                let point = if axis == 0 {
                    PlotPoint::new(*value, min[1])
                } else {
                    PlotPoint::new(min[0], *value)
                };
                let pos = response.transform.position_from_point(&point);
                let galley = tick_galley(axis, label);
                if axis == 0 && figure.extras.leaf_ticks.is_some() {
                    let spacing = response.transform.dpos_dvalue()[0].abs() as f32;
                    if spacing < galley.size().x || spacing <= 20. {
                        continue;
                    }
                }
                let edge = if axis == 0 {
                    egui::pos2(pos.x, frame.bottom())
                } else {
                    egui::pos2(frame.left(), pos.y)
                };
                let mut shape = tick_label_shape(galley, axis == 0, rotation);
                if measure_gutter {
                    let ink = shape.visual_bounding_rect();
                    let extent = if axis == 0 { ink.bottom() } else { -ink.left() };
                    measured_gutters.artwork[axis] =
                        measured_gutters.artwork[axis].max(extent + 2.);
                }
                shape.pos += edge.to_vec2();
                painter.add(shape);
            }
        }
    }
    if measured_gutters != stored_gutters {
        ui.data_mut(|data| data.insert_temp(gutter_id, measured_gutters));
    }
    if measured_gutters.artwork != previous_gutters {
        ui.ctx().request_repaint();
    }
    let mut location = figure.legend_location.fixed_anchor();
    if figure.legend_location == matrust::plot::LegendLocation::Best
        && let Some(extent) = legend::extent(
            ui,
            figure,
            frame.shrink(4.).intersect(ui.clip_rect()),
            false,
        )
    {
        let view = matrust::plot::LegendBestView {
            bounds: [min[0], max[0], min[1], max[1]],
            transforms: axes,
            frame: to_model_rect(frame),
            extent: [extent.x as f64, extent.y as f64],
        };
        let freeze = over_legend && ui.input(|i| i.pointer.any_down());
        if let Ok(result) = ui.ctx().data_mut(|d| {
            let cache = d.get_temp_mut_or_default::<matrust::plot::LegendBestCache>(
                plot_id.with("legend_best"),
            );
            if let Some(prepared) = prepared_legend {
                prepared.resolve(cache, view, freeze)
            } else if prepared_bounds.is_some() {
                Err("Prepared legend source is unavailable".into())
            } else {
                cache.resolve(figure, view, freeze)
            }
        }) {
            location = result.location;
        }
    }
    // Preserve the existing legend widget/scroll IDs: no extra Ui scope.
    let legend_appearance = figure.legend_appearance(appearance);
    let old_text = ui.visuals().override_text_color;
    let old_background = ui.visuals().extreme_bg_color;
    let old_border = ui.visuals().window_stroke.color;
    if appearance.theme == BaseTheme::Dark || figure.colors.is_some_and(|c| c.legend.is_some()) {
        ui.visuals_mut().extreme_bg_color = rgb(legend_appearance.legend_background);
        ui.visuals_mut().window_stroke.color = rgb(legend_appearance.legend_border);
        ui.visuals_mut().override_text_color = Some(rgb(legend_appearance.text));
    }
    let legend = legend::show_placed(
        ui,
        figure,
        frame,
        plot_id,
        outside,
        location,
        legend_appearance,
    );
    ui.visuals_mut().override_text_color = old_text;
    ui.visuals_mut().extreme_bg_color = old_background;
    ui.visuals_mut().window_stroke.color = old_border;
    ui.ctx()
        .data_mut(|d| d.insert_temp(plot_id.with("legend_rect"), legend.rect));
    if legend.rect.is_some_and(|rect| {
        ui.input(|i| i.pointer.interact_pos().is_some_and(|p| rect.contains(p)))
    }) {
        pick = None;
    }
    let mut memory = egui_plot::PlotMemory::load(ui.ctx(), plot_id);
    if let Some(memory) = &mut memory {
        if let Some(ranges) = date_ranges
            && (0..2).any(|axis| {
                let manual = !memory.auto_bounds[axis];
                date_axes[axis]
                    && (manual != manual_dates[axis]
                        || (manual
                            && ranges[axis]
                                != [axes[axis].inverse(min[axis]), axes[axis].inverse(max[axis])]))
            })
        {
            ui.ctx().request_repaint();
        }
        memory.hovered_legend_item = legend.hovered;
        memory.clone().store(ui.ctx(), plot_id);
    }
    let visibility = legend.visibility;
    #[cfg(test)]
    ui.data_mut(|data| {
        *data.get_temp_mut_or_default::<usize>(plot_id.with(CUSTOM_TICK_LAYOUT_COUNT)) +=
            tick_layout_count.get();
    });
    let rotate = (figure.frame_hidden
        && ui.is_enabled()
        && response.response.dragged_by(egui::PointerButton::Primary))
    .then(|| response.response.drag_delta())
    .filter(|d| *d != egui::Vec2::ZERO)
    .map(|d| [f64::from(d.x), f64::from(d.y)]);
    View {
        bounds: model_bounds(figure, axes, min, max),
        auto: if figure.frame_hidden {
            [true; 2]
        } else {
            memory.map_or([true; 2], |m| [m.auto_bounds.x, m.auto_bounds.y])
        },
        visibility,
        pick: pick.filter(|_| !figure.frame_hidden),
        projected: figure.frame_hidden,
        rotate,
        slice_number: None,
    }
}
// A fixed log view is stored in data coordinates. Its forward/inverse pair
// need not round-trip bit-for-bit. Preserve the exact model limit while the
// displayed bound is unchanged; actual pan/zoom bounds still decode normally.
fn model_bounds(figure: &Axes, axes: [AxisTransform; 2], min: [f64; 2], max: [f64; 2]) -> [f64; 4] {
    let mut result = [0.; 4];
    for (axis, saved) in [figure.xlim, figure.ylim].into_iter().enumerate() {
        let bounds = saved
            .filter(|saved| {
                axes[axis].forward(saved[0]) == min[axis]
                    && axes[axis].forward(saved[1]) == max[axis]
            })
            .unwrap_or_else(|| [axes[axis].inverse(min[axis]), axes[axis].inverse(max[axis])]);
        result[2 * axis..2 * axis + 2].copy_from_slice(&bounds);
    }
    result
}
fn unavailable_layout_prepared(
    ui: &mut egui::Ui,
    figure: &Axes,
    plot_id: egui::Id,
    error: &str,
    axes: [AxisTransform; 2],
    prepared_bounds: Option<([f64; 2], [f64; 2])>,
) -> View {
    ui.add(
        egui::Label::new(
            egui::RichText::new(error)
                .color(super::theme::error_color(ui, Color32::RED))
                .background_color(ui.ctx().global_style().visuals.panel_fill),
        )
        .truncate(),
    )
    .on_hover_text(error);
    ui.ctx()
        .data_mut(|d| d.remove::<Option<egui::Rect>>(plot_id.with("legend_rect")));
    let memory = egui_plot::PlotMemory::load(ui.ctx(), plot_id);
    let bounds = memory.as_ref().map_or_else(
        || {
            let (x, y) = prepared_bounds.unwrap_or_else(|| figure.bounds());
            [x[0], x[1], y[0], y[1]]
        },
        |memory| {
            let min = memory.bounds().min();
            let max = memory.bounds().max();
            model_bounds(figure, axes, min, max)
        },
    );
    View {
        bounds,
        auto: if figure.frame_hidden {
            [true; 2]
        } else {
            memory.map_or([figure.xlim.is_none(), figure.ylim.is_none()], |m| {
                [m.auto_bounds.x, m.auto_bounds.y]
            })
        },
        visibility: None,
        pick: None,
        projected: figure.frame_hidden,
        rotate: None,
        slice_number: None,
    }
}

pub(super) fn slice_slider_and_input(
    ui: &mut egui::Ui,
    slider_rect: egui::Rect,
    plot_rect: egui::Rect,
    slice_number: usize,
    max_slice: usize,
    id: egui::Id,
) -> Option<usize> {
    if max_slice <= 1 {
        return None;
    }
    let mut val = slice_number.clamp(1, max_slice);
    let before = val;
    ui.scope_builder(egui::UiBuilder::new().max_rect(slider_rect), |ui| {
        ui.push_id(id, |ui| {
            ui.horizontal(|ui| {
                ui.add_space(6.0);
                let slider = egui::Slider::new(&mut val, 1..=max_slice)
                    .step_by(1.0)
                    .integer()
                    .show_value(true);
                ui.add_sized(
                    egui::vec2((slider_rect.width() - 16.0).max(40.0), 20.0),
                    slider,
                );
            });
        });
    });
    let hovered = ui.input(|i| {
        i.pointer
            .hover_pos()
            .is_some_and(|p| plot_rect.contains(p) || slider_rect.contains(p))
    });
    if hovered {
        let delta = ui.input(|i| {
            let wheel = if i.smooth_scroll_delta.y > 0.5 {
                1isize
            } else if i.smooth_scroll_delta.y < -0.5 {
                -1isize
            } else {
                0isize
            };
            let key = if i.key_pressed(egui::Key::ArrowRight) || i.key_pressed(egui::Key::ArrowUp) {
                1isize
            } else if i.key_pressed(egui::Key::ArrowLeft) || i.key_pressed(egui::Key::ArrowDown) {
                -1isize
            } else {
                0isize
            };
            if wheel != 0 { wheel } else { key }
        });
        if delta != 0 {
            val = (val as isize + delta).clamp(1, max_slice as isize) as usize;
        }
    }
    (val != before).then_some(val)
}

#[cfg(test)]
fn show(
    ui: &mut egui::Ui,
    figure: &Figure,
    points: &[Vec<PlotPoint>],
    axes: [AxisTransform; 2],
    reset: bool,
) -> [f64; 4] {
    show_interactive(ui, figure, points, axes, reset, None).bounds
}
fn native_marker(shape: matrust::plot::MarkerShape) -> egui_plot::MarkerShape {
    use egui_plot::MarkerShape as N;
    use matrust::plot::MarkerShape as M;
    match shape {
        M::Circle | M::Point => N::Circle,
        M::Plus => N::Plus,
        M::Cross => N::Cross,
        M::Asterisk => N::Asterisk,
        M::Square => N::Square,
        M::Diamond => N::Diamond,
        M::Up => N::Up,
        M::Down => N::Down,
        M::Left => N::Left,
        M::Right => N::Right,
    }
}
struct StyledPoints<'a> {
    points: Points<'a>,
    width: Option<f32>,
}
impl PlotItem for StyledPoints<'_> {
    fn shapes(&self, ui: &egui::Ui, transform: &PlotTransform, shapes: &mut Vec<egui::Shape>) {
        let start = shapes.len();
        self.points.shapes(ui, transform, shapes);
        if let Some(width) = self.width {
            for shape in &mut shapes[start..] {
                let stroke = egui::Stroke::new(width, PlotItem::color(&self.points));
                match shape {
                    egui::Shape::Circle(circle) => circle.stroke = stroke,
                    egui::Shape::Path(path) => path.stroke = stroke.into(),
                    egui::Shape::LineSegment { stroke: target, .. } => *target = stroke,
                    _ => {}
                }
            }
        }
    }
    fn initialize(&mut self, range: std::ops::RangeInclusive<f64>) {
        self.points.initialize(range);
    }
    fn color(&self) -> Color32 {
        PlotItem::color(&self.points)
    }
    fn geometry(&self) -> PlotGeometry<'_> {
        self.points.geometry()
    }
    fn bounds(&self) -> PlotBounds {
        self.points.bounds()
    }
    fn base(&self) -> &PlotItemBase {
        self.points.base()
    }
    fn base_mut(&mut self) -> &mut PlotItemBase {
        self.points.base_mut()
    }
}
fn with_alpha(rgb: [u8; 3], alpha: f64) -> Color32 {
    let a = (alpha.clamp(0., 1.) * 255.).round() as u8;
    Color32::from_rgba_unmultiplied(rgb[0], rgb[1], rgb[2], a)
}
struct ScatterPoint {
    point: PlotPoint,
    radius: f32,
    fill: Color32,
    stroke_color: Color32,
}
struct ScatterPoints<'a> {
    base: Points<'a>,
    shape: matrust::plot::MarkerShape,
    stroke_width: f32,
    items: Vec<ScatterPoint>,
}
impl PlotItem for ScatterPoints<'_> {
    fn shapes(&self, _ui: &egui::Ui, transform: &PlotTransform, shapes: &mut Vec<egui::Shape>) {
        use matrust::plot::MarkerShape::*;
        let q = std::f32::consts::FRAC_1_SQRT_2;
        let h = 0.866_025_4_f32;
        for item in &self.items {
            let center = transform.position_from_point(&item.point);
            let r = item.radius;
            let stroke = if item.stroke_color == Color32::TRANSPARENT {
                egui::Stroke::NONE
            } else {
                egui::Stroke::new(self.stroke_width, item.stroke_color)
            };
            match self.shape {
                Circle | Point => {
                    shapes.push(egui::Shape::circle_filled(center, r, item.fill));
                    if stroke != egui::Stroke::NONE {
                        shapes.push(egui::Shape::circle_stroke(center, r, stroke));
                    }
                }
                Diamond | Square | Up | Down | Left | Right => {
                    let offsets: &[[f32; 2]] = match self.shape {
                        Diamond => &[[0., 1.], [-1., 0.], [0., -1.], [1., 0.]],
                        Square => &[[-q, q], [-q, -q], [q, -q], [q, q]],
                        Up => &[[0., -1.], [h, 0.5], [-h, 0.5]],
                        Down => &[[0., 1.], [-h, -0.5], [h, -0.5]],
                        Left => &[[-1., 0.], [0.5, -h], [0.5, h]],
                        Right => &[[1., 0.], [-0.5, h], [-0.5, -h]],
                        _ => unreachable!(),
                    };
                    let pts: Vec<egui::Pos2> = offsets
                        .iter()
                        .map(|o| egui::pos2(center.x + o[0] * r, center.y + o[1] * r))
                        .collect();
                    shapes.push(egui::Shape::convex_polygon(pts, item.fill, stroke));
                }
                Plus | Cross | Asterisk => {
                    let segs: &[[[f32; 2]; 2]] = match self.shape {
                        Plus => &[[[-1., 0.], [1., 0.]], [[0., -1.], [0., 1.]]],
                        Cross => &[[[-q, -q], [q, q]], [[q, -q], [-q, q]]],
                        _ => &[
                            [[0., -1.], [0., 1.]],
                            [[-h, 0.5], [h, -0.5]],
                            [[-h, -0.5], [h, 0.5]],
                        ],
                    };
                    for [a, b] in segs {
                        shapes.push(egui::Shape::line_segment(
                            [
                                egui::pos2(center.x + a[0] * r, center.y + a[1] * r),
                                egui::pos2(center.x + b[0] * r, center.y + b[1] * r),
                            ],
                            stroke,
                        ));
                    }
                }
            }
        }
    }
    fn initialize(&mut self, range: std::ops::RangeInclusive<f64>) {
        self.base.initialize(range);
    }
    fn color(&self) -> Color32 {
        PlotItem::color(&self.base)
    }
    fn geometry(&self) -> PlotGeometry<'_> {
        self.base.geometry()
    }
    fn bounds(&self) -> PlotBounds {
        self.base.bounds()
    }
    fn base(&self) -> &PlotItemBase {
        self.base.base()
    }
    fn base_mut(&mut self) -> &mut PlotItemBase {
        self.base.base_mut()
    }
}
struct StyledLine<'a> {
    line: Line<'a>,
    width: f32,
    pattern: LinePattern,
    dash_budget: f64,
}
impl PlotItem for StyledLine<'_> {
    fn shapes(&self, _ui: &egui::Ui, transform: &PlotTransform, shapes: &mut Vec<egui::Shape>) {
        if self.pattern == LinePattern::None {
            return;
        }
        let PlotGeometry::Points(points) = self.line.geometry() else {
            return;
        };
        let stroke = egui::Stroke::new(
            if self.line.highlighted() {
                self.width / 1.7 * 2.6
            } else {
                self.width
            },
            PlotItem::color(&self.line),
        );
        let mut path = Vec::new();
        let mut budget = self.dash_budget;
        let mut flush = |path: &mut Vec<egui::Pos2>, shapes: &mut Vec<egui::Shape>| {
            if path.len() > 1 {
                let length: f64 = path.windows(2).map(|p| p[0].distance(p[1]) as f64).sum();
                if self.pattern == LinePattern::Solid || length > budget {
                    shapes.push(egui::Shape::line(std::mem::take(path), stroke));
                } else {
                    budget -= length;
                    let (dash, gap): (&[f32], &[f32]) = match self.pattern {
                        LinePattern::Dashed => (&[8.], &[5.]),
                        LinePattern::Dotted => (&[2.], &[4.]),
                        LinePattern::DashDot => (&[8., 2.], &[4., 4.]),
                        _ => unreachable!(),
                    };
                    shapes.extend(egui::Shape::dashed_line_with_offset(
                        path, stroke, dash, gap, 0.,
                    ));
                    path.clear();
                }
            } else {
                path.clear();
            }
        };
        for pair in points.windows(2) {
            let a = transform.position_from_point(&pair[0]);
            let b = transform.position_from_point(&pair[1]);
            if let Some([a, b]) = clip(a, b, *transform.frame()) {
                if path.last().is_some_and(|p| *p != a) {
                    flush(&mut path, shapes);
                }
                if path.is_empty() {
                    path.push(a);
                }
                path.push(b);
            } else {
                flush(&mut path, shapes);
            }
        }
        flush(&mut path, shapes);
    }
    fn initialize(&mut self, range: std::ops::RangeInclusive<f64>) {
        self.line.initialize(range);
    }
    fn color(&self) -> Color32 {
        PlotItem::color(&self.line)
    }
    fn geometry(&self) -> PlotGeometry<'_> {
        // Retain bounds and stored samples but do not hover an invisible stroke.
        if self.pattern == LinePattern::None {
            PlotGeometry::None
        } else {
            self.line.geometry()
        }
    }
    fn bounds(&self) -> PlotBounds {
        self.line.bounds()
    }
    fn base(&self) -> &PlotItemBase {
        self.line.base()
    }
    fn base_mut(&mut self) -> &mut PlotItemBase {
        self.line.base_mut()
    }
}
// Liang-Barsky clipping bounds dashed geometry work to the visible viewport.
fn clip(a: egui::Pos2, b: egui::Pos2, r: egui::Rect) -> Option<[egui::Pos2; 2]> {
    if !a.is_finite() || !b.is_finite() {
        return None;
    }
    let d = b - a;
    if !d.is_finite() {
        return None;
    }
    let (mut lo, mut hi) = (0.0f64, 1.0f64);
    for (p, q) in [
        (-(d.x as f64), (a.x - r.min.x) as f64),
        (d.x as f64, (r.max.x - a.x) as f64),
        (-(d.y as f64), (a.y - r.min.y) as f64),
        (d.y as f64, (r.max.y - a.y) as f64),
    ] {
        if p == 0. {
            if q < 0. {
                return None;
            }
        } else {
            let t = q / p;
            if p < 0. {
                lo = lo.max(t);
            } else {
                hi = hi.min(t);
            }
        }
    }
    if lo > hi {
        None
    } else {
        Some([a + d * lo as f32, a + d * hi as f32])
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    /// One headless frame of a 200x200 surf (39,601 depth-sorted quads with
    /// per-face colours and edges) is prepared and painted; the times are
    /// printed for the report (`--nocapture`).
    #[test]
    fn surf_200_by_200_headless_frame_finishes() {
        let mut e = matrust::Engine::default();
        e.execute("[X,Y] = meshgrid(linspace(-3,3,200)); surf(X, Y, sin(X).*cos(Y));")
            .unwrap();
        let figure = e.figure.clone().unwrap();
        assert_eq!(figure.series[0].patch_face_count(), 199 * 199);
        let start = std::time::Instant::now();
        let (axes, points) = crate::graphics_frames::display_points(&figure);
        let prepared = start.elapsed();
        let ctx = egui::Context::default();
        let start = std::time::Instant::now();
        let output = crate::headless::run(
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
                    show_interactive(ui, &figure, &points, axes, true, None);
                });
            },
        );
        let frame = start.elapsed();
        eprintln!("surf 200x200: prepare {prepared:?}, frame {frame:?}");
        assert!(output.shapes.len() > 1000, "{}", output.shapes.len());
    }
    /// A sliver surface face (thin quad, as projected in a small subplot
    /// tile) tessellates inside its own pixel box: no miter spikes from
    /// feathering or a closed-path stroke (docs/steps/surf-subplot-bugs.md).
    #[test]
    fn sliver_surface_face_tessellates_inside_its_box() {
        let frame = egui::Rect::from_min_size(egui::pos2(0., 0.), egui::vec2(100., 100.));
        let transform = PlotTransform::new(
            frame,
            PlotBounds::from_min_max([-1., -1.], [1., 1.]),
            [false; 2],
        );
        let corners = vec![[-0.9, 0.], [0.9, 0.001], [0.9, 0.002], [-0.9, 0.0005]];
        let patch = StyledPatch {
            line: Line::new("face", corners).color(Color32::BLUE),
            width: 4,
            edge: None,
            stroke_width: 0.5,
            pattern: LinePattern::Solid,
            dash_budget: 200000.,
            bar: None,
            fill: true,
            surface: Some(SurfaceStyle {
                fill: Some(std::sync::Arc::from([[0, 128, 0]])),
                edges: None,
                alpha: None,
                faces: None,
            }),
            interp: None,
        };
        let ctx = egui::Context::default();
        let mut shapes = Vec::new();
        crate::headless::run(&ctx, Default::default(), |root| {
            egui::CentralPanel::default()
                .show(root, |ui| patch.shapes(ui, &transform, &mut shapes));
        });
        assert!(!shapes.is_empty());
        let mut tessellator = egui::epaint::Tessellator::new(
            1.,
            egui::epaint::TessellationOptions::default(),
            [1, 1],
            Vec::new(),
        );
        let mut mesh = egui::Mesh::default();
        for shape in shapes {
            tessellator.tessellate_shape(shape, &mut mesh);
        }
        // The face spans x 5..95 and y ~49.9..50 in pixels.
        let allowed = egui::Rect::from_min_max(egui::pos2(3., 47.), egui::pos2(97., 53.));
        let outside: Vec<_> = mesh
            .vertices
            .iter()
            .map(|v| v.pos)
            .filter(|p| !allowed.contains(*p))
            .collect();
        assert!(outside.is_empty(), "spike vertices {outside:?}");
    }
    #[test]
    fn square_plot_box_fits_the_actual_desktop_data_frame() {
        let points = vec![vec![PlotPoint::new(0., 0.), PlotPoint::new(10000., 1.)]];
        let id = egui::Id::new("square_plot_box");
        for labels in [
            "xlabel('Length');ylabel('Height');",
            r"xlabel('$\frac{a}{b}$','Interpreter','latex');
              ylabel('$\sqrt{x}$','Interpreter','latex');
              xticks([0 5000 10000]);xticklabels({'Zero','Halfway','End'});xtickangle(45);
              yticks([0 .5 1]);yticklabels({'Lower limit','Middle','Upper limit'});",
        ] {
            let mut engine = matrust::Engine::default();
            engine
                .execute(&format!("plot([0 10000],[0 1]);{labels}axis square;"))
                .unwrap();
            let ctx = egui::Context::default();
            let frame = |figure: &Figure, size| {
                for _ in 0..4 {
                    crate::headless::run(
                        &ctx,
                        egui::RawInput {
                            screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, size)),
                            ..Default::default()
                        },
                        |root| {
                            egui::CentralPanel::default().show(root, |ui| {
                                show_axes(
                                    ui,
                                    &figure.axes,
                                    &points,
                                    figure.axis_transforms(),
                                    false,
                                    id,
                                    PanelContext::default(),
                                );
                            });
                        },
                    );
                }
                *egui_plot::PlotMemory::load(&ctx, id)
                    .unwrap()
                    .transform()
                    .frame()
            };
            let wide = egui::vec2(800., 350.);
            for size in [wide, egui::vec2(350., 650.)] {
                let square = frame(engine.figure.as_ref().unwrap(), size);
                assert!(square.width() > 100., "{square:?}");
                assert!((square.width() - square.height()).abs() < 0.5, "{square:?}");
                engine.execute("axis normal;").unwrap();
                let normal = frame(engine.figure.as_ref().unwrap(), size);
                // Native tick density can change the perpendicular gutter.
                // Compare centres along the dimension with room to centre.
                let (centre_delta, edge_delta) = if size == wide {
                    assert!(normal.width() > normal.height() * 2.);
                    (
                        square.center().x - normal.center().x,
                        square.top() - normal.top(),
                    )
                } else {
                    (
                        square.center().y - normal.center().y,
                        square.right() - normal.right(),
                    )
                };
                assert!(
                    centre_delta.abs() < 0.5 && edge_delta.abs() < 0.5,
                    "square {square:?} does not fit the available data area {normal:?}"
                );
                engine.execute("axis square;").unwrap();
            }
        }
    }

    #[test]
    fn outside_empty_legend_membership_does_not_hide_excluded_labeled_series() {
        for call in [
            "legend(a,[],{},'Interpreter','none','Location','southoutside');",
            "legend(a,h(1),{''},'Interpreter','none','Location','southoutside');",
        ] {
            let mut e = matrust::Engine::default();
            e.execute("a=gca;h=plot(a,[0 1],[0 1],[0 1],[1 0]);")
                .unwrap();
            e.execute(call).unwrap();
            let f = e.figure.unwrap();
            let snapshot = f.clone();
            assert!(f.legend_entries().next().is_none());
            assert!(!f.series[1].label.is_empty());
            let points = vec![
                vec![PlotPoint::new(0., 0.), PlotPoint::new(1., 1.)],
                vec![PlotPoint::new(0., 1.), PlotPoint::new(1., 0.)],
            ];
            let ctx = egui::Context::default();
            let id = egui::Id::new("empty_explicit_outside");
            for _ in 0..3 {
                let output = crate::headless::run(
                    &ctx,
                    egui::RawInput {
                        screen_rect: Some(egui::Rect::from_min_size(
                            egui::Pos2::ZERO,
                            egui::vec2(320., 240.),
                        )),
                        ..Default::default()
                    },
                    |root| {
                        egui::CentralPanel::default().show(root, |ui| {
                            show_axes(
                                ui,
                                &f.axes,
                                &points,
                                f.axis_transforms(),
                                false,
                                id,
                                PanelContext::default(),
                            );
                        });
                    },
                );
                assert!(!output.shapes.iter().any(|s|matches!(&s.shape,egui::Shape::Text(t) if t.galley.text().starts_with("Legend layout"))));
            }
            assert!(
                egui_plot::PlotMemory::load(&ctx, id).is_some(),
                "The actual data plot must be laid out"
            );
            assert!(
                ctx.data(|d| d.get_temp::<Option<egui::Rect>>(id.with("legend_rect")))
                    .flatten()
                    .is_none()
            );
            assert!(f.same_snapshot(&snapshot));
        }
    }
    #[test]
    fn undersized_outside_layout_preserves_existing_manual_transformed_view() {
        for script in [
            "plot([1 10 100],[1 10 100]);legend('Signal','Location','southoutside');",
            "loglog([1 10 100],[1 10 100]);legend('Signal','Location','southoutside');",
        ] {
            let mut engine = matrust::Engine::default();
            engine.execute(script).unwrap();
            let figure = engine.figure.unwrap();
            let axes = figure.axis_transforms();
            let points = vec![
                figure.series[0]
                    .display_points_axes(64, axes)
                    .into_iter()
                    .map(|p| PlotPoint::new(p[0], p[1]))
                    .collect(),
            ];
            let ctx = egui::Context::default();
            let id = egui::Id::new("preserved_tiny_outside");
            let frame = |size: egui::Vec2| {
                let mut result = None;
                let _ = crate::headless::run(
                    &ctx,
                    egui::RawInput {
                        screen_rect: Some(egui::Rect::from_min_size(
                            egui::Pos2::ZERO,
                            egui::vec2(700., 500.),
                        )),
                        ..Default::default()
                    },
                    |root| {
                        egui::CentralPanel::default().show(root, |ui| {
                            let rect = egui::Rect::from_min_size(ui.cursor().min, size);
                            ui.scope_builder(egui::UiBuilder::new().max_rect(rect), |ui| {
                                result = Some(show_axes(
                                    ui,
                                    &figure.axes,
                                    &points,
                                    axes,
                                    false,
                                    id,
                                    PanelContext::default(),
                                ));
                            });
                        });
                    },
                );
                result.unwrap()
            };
            for _ in 0..3 {
                frame(egui::vec2(600., 400.));
            }
            let mut memory = egui_plot::PlotMemory::load(&ctx, id).unwrap();
            memory.set_bounds(PlotBounds::from_min_max(
                [axes[0].forward(2.), axes[1].forward(3.)],
                [axes[0].forward(20.), axes[1].forward(30.)],
            ));
            memory.auto_bounds = [false, false].into();
            let expected = [
                axes[0].inverse(memory.bounds().min()[0]),
                axes[0].inverse(memory.bounds().max()[0]),
                axes[1].inverse(memory.bounds().min()[1]),
                axes[1].inverse(memory.bounds().max()[1]),
            ];
            memory.clone().store(&ctx, id);
            assert!(
                ctx.data(|d| d.get_temp::<Option<egui::Rect>>(id.with("legend_rect")))
                    .flatten()
                    .is_some()
            );
            let view = frame(egui::vec2(220., 24.));
            assert_eq!(view.bounds, expected);
            assert_eq!(view.auto, [false, false]);
            assert!(
                ctx.data(|d| d.get_temp::<Option<egui::Rect>>(id.with("legend_rect")))
                    .flatten()
                    .is_none()
            );
            assert_eq!(
                egui_plot::PlotMemory::load(&ctx, id).unwrap().bounds(),
                memory.bounds()
            );
        }
    }

    #[test]
    fn undersized_outside_panel_reports_layout_without_inside_fallback() {
        let mut engine = matrust::Engine::default();
        engine
            .execute("plot([0 1],[0 1]);legend('Signal','Location','southoutside');")
            .unwrap();
        let figure = engine.figure.unwrap();
        let original = figure.clone();
        let points = vec![vec![PlotPoint::new(0., 0.), PlotPoint::new(1., 1.)]];
        let ctx = egui::Context::default();
        let id = egui::Id::new("tiny_outside");
        let output = crate::headless::run(
            &ctx,
            egui::RawInput {
                screen_rect: Some(egui::Rect::from_min_size(
                    egui::Pos2::ZERO,
                    egui::vec2(300., 80.),
                )),
                ..Default::default()
            },
            |root| {
                egui::CentralPanel::default().show(root, |ui| {
                    let rect = egui::Rect::from_min_size(ui.cursor().min, egui::vec2(220., 24.));
                    ui.scope_builder(egui::UiBuilder::new().max_rect(rect), |ui| {
                        let view = show_axes(
                            ui,
                            &figure.axes,
                            &points,
                            figure.axis_transforms(),
                            false,
                            id,
                            PanelContext::default(),
                        );
                        assert!(view.visibility.is_none() && view.pick.is_none());
                    });
                });
            },
        );
        assert!(egui_plot::PlotMemory::load(&ctx, id).is_none());
        assert!(
            ctx.data(|d| d.get_temp::<Option<egui::Rect>>(id.with("legend_rect")))
                .flatten()
                .is_none()
        );
        assert!(output.shapes.iter().any(|s| matches!(&s.shape, egui::Shape::Text(t) if t.galley.text().starts_with("Legend layout"))));
        assert!(figure.same_snapshot(&original));
    }

    #[test]
    fn docked_outside_scene_contains_legend_plot_and_numeric_ticks() {
        for size in [
            egui::vec2(710., 230.),
            egui::vec2(710., 280.),
            egui::vec2(1200., 560.),
        ] {
            for location in ["southoutside", "eastoutside", "northoutside", "westoutside"] {
                let mut engine = matrust::Engine::default();
                engine.execute(&format!("plot([0 1 2],[0 1 0]);axis([0 2 0 1]);legend('Signal','Location','{location}');")).unwrap();
                let figure = engine.figure.unwrap();
                let original = figure.clone();
                let points = vec![vec![
                    PlotPoint::new(0., 0.),
                    PlotPoint::new(1., 1.),
                    PlotPoint::new(2., 0.),
                ]];
                let ctx = egui::Context::default();
                let mut viewport = egui::Rect::NOTHING;
                let mut output = None;
                for pass in 0..4 {
                    output = Some(crate::headless::run(
                        &ctx,
                        egui::RawInput {
                            screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, size)),
                            ..Default::default()
                        },
                        |root| {
                            egui::CentralPanel::default().show(root, |ui| {
                                viewport = ui.available_rect_before_wrap();
                                let view = show_interactive(
                                    ui,
                                    &figure,
                                    &points,
                                    figure.axis_transforms(),
                                    pass == 0,
                                    None,
                                );
                                assert!(view.visibility.is_none());
                            });
                        },
                    ));
                }
                let id = egui::Id::new("workspace_figure");
                let legend = ctx
                    .data(|d| d.get_temp::<Option<egui::Rect>>(id.with("legend_rect")))
                    .flatten()
                    .expect("Outside legend must actually paint");
                assert!(
                    viewport.contains_rect(legend),
                    "{size:?} {location}: {legend:?} vs {viewport:?}"
                );
                let memory = egui_plot::PlotMemory::load(&ctx, id).unwrap();
                assert!(viewport.contains_rect(*memory.transform().frame()));
                let mut found = false;
                let mut ticks = 0;
                fn inspect(
                    shape: &egui::Shape,
                    clip: egui::Rect,
                    viewport: egui::Rect,
                    found: &mut bool,
                    ticks: &mut usize,
                ) {
                    match shape {
                        egui::Shape::Text(t) => {
                            let text = t.galley.text();
                            if text == "Signal" || text.parse::<f64>().is_ok() {
                                let rect = egui::Rect::from_min_size(t.pos, t.galley.size());
                                assert!(
                                    viewport.contains_rect(rect),
                                    "Clipped {text}: {rect:?} / {viewport:?}"
                                );
                                assert!(clip.contains_rect(rect), "Clipped by painter: {text}");
                                *found |= text == "Signal";
                                *ticks += usize::from(text != "Signal");
                            }
                        }
                        egui::Shape::Vec(v) => {
                            for item in v {
                                inspect(item, clip, viewport, found, ticks);
                            }
                        }
                        _ => {}
                    }
                }
                for shape in output.unwrap().shapes {
                    inspect(
                        &shape.shape,
                        shape.clip_rect,
                        viewport,
                        &mut found,
                        &mut ticks,
                    );
                }
                assert!(
                    found && ticks >= 2,
                    "Legend and numeric ticks must be visible"
                );
                assert!(figure.same_snapshot(&original));
            }
        }
    }

    #[test]
    fn native_outside_legends_keep_visible_labels_clicks_and_view_bounds() {
        for size in [egui::vec2(640., 480.), egui::vec2(960., 650.)] {
            for location in matrust::plot::LegendLocation::ALL
                .into_iter()
                .filter(|v| v.is_outside())
            {
                let mut engine = matrust::Engine::default();
                engine.execute(&format!("plot([0 1],[0 1]);legend('Outside signal','Location','{}');xlim([0 1]);ylim([0 1]);", location.name())).unwrap();
                let f = engine.figure.unwrap();
                let samples = f.series[0].points.clone();
                let ctx = egui::Context::default();
                let points = vec![vec![PlotPoint::new(0., 0.), PlotPoint::new(1., 1.)]];
                let frame = |events: Vec<egui::Event>| {
                    let mut result = None;
                    let output = crate::headless::run(
                        &ctx,
                        egui::RawInput {
                            screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, size)),
                            events,
                            ..Default::default()
                        },
                        |root| {
                            egui::CentralPanel::default().show(root, |ui| {
                                result = Some(show_interactive(
                                    ui,
                                    &f,
                                    &points,
                                    [AxisTransform::default(); 2],
                                    true,
                                    None,
                                ));
                            });
                        },
                    );
                    (output, result.unwrap())
                };
                for _ in 0..3 {
                    frame(vec![]);
                }
                let (output, before) = frame(vec![]);
                let legend = ctx
                    .data(|d| {
                        d.get_temp::<Option<egui::Rect>>(
                            egui::Id::new("workspace_figure").with("legend_rect"),
                        )
                    })
                    .flatten()
                    .unwrap();
                assert!(egui::Rect::from_min_size(egui::Pos2::ZERO, size).contains_rect(legend));
                let label = output
                    .shapes
                    .iter()
                    .find_map(|s| {
                        if let egui::Shape::Text(t) = &s.shape
                            && t.galley.text() == "Outside signal"
                        {
                            let bounds = egui::Rect::from_min_size(t.pos, t.galley.size());
                            assert!(
                                s.clip_rect.contains_rect(bounds),
                                "clipped {location:?} label"
                            );
                            assert!(legend.contains_rect(bounds));
                            Some(bounds.center())
                        } else {
                            None
                        }
                    })
                    .expect("Outside legend label must be fully painted");
                for pressed in [true, false] {
                    let (_, response) = frame(vec![
                        egui::Event::PointerMoved(label),
                        egui::Event::PointerButton {
                            pos: label,
                            button: egui::PointerButton::Primary,
                            pressed,
                            modifiers: egui::Modifiers::NONE,
                        },
                    ]);
                    assert!(response.pick.is_none());
                    assert_eq!(response.bounds, before.bounds);
                    if !pressed {
                        assert_eq!(response.visibility, Some(vec![false]));
                    }
                }
                assert!((samples).same_storage(&f.series[0].points));
                assert_eq!(f.legend_location, location);
            }
        }
    }
    #[test]
    fn native_legend_toggles_persistent_visibility_without_cross_figure_leaks() {
        let mut e = matrust::Engine::default();
        e.execute(
            "plot([1 2 3],[1 2 3],'bo-', [1 2 3],[100 200 300],'r-');legend('Shown','Hidden');",
        )
        .unwrap();
        let mut f = e.figure.unwrap();
        let ctx = egui::Context::default();
        let frame = |f: &Figure, reset, events| {
            let axes = f.axis_transforms();
            let points: Vec<_> = f
                .series
                .iter()
                .map(|s| {
                    s.display_points_axes(1024, axes)
                        .into_iter()
                        .map(|p| PlotPoint::new(p[0], p[1]))
                        .collect()
                })
                .collect();
            let mut view = None;
            let output = crate::headless::run(
                &ctx,
                egui::RawInput {
                    screen_rect: Some(egui::Rect::from_min_size(
                        egui::Pos2::ZERO,
                        egui::vec2(900., 600.),
                    )),
                    events,
                    ..Default::default()
                },
                |root| {
                    egui::CentralPanel::default().show(root, |ui| {
                        view = Some(show_interactive(ui, f, &points, axes, reset, None));
                    });
                },
            );
            (output, view.unwrap())
        };
        let mut position = None;
        for index in 0..3 {
            let (output, view) = frame(&f, index == 0, vec![]);
            assert!(view.visibility.is_none());
            position = output.shapes.iter().find_map(|s| {
                if let egui::Shape::Text(t) = &s.shape
                    && t.galley.text() == "Hidden"
                {
                    Some(t.pos + t.galley.size() * 0.5)
                } else {
                    None
                }
            });
        }
        let position = position.expect("Legend label not rendered");
        for pressed in [true, false] {
            let (_, view) = frame(
                &f,
                false,
                vec![
                    egui::Event::PointerMoved(position),
                    egui::Event::PointerButton {
                        pos: position,
                        button: egui::PointerButton::Primary,
                        pressed,
                        modifiers: egui::Modifiers::NONE,
                    },
                ],
            );
            if !pressed {
                assert_eq!(view.visibility, Some(vec![true, false]));
                assert_eq!(view.auto, [true; 2]);
            }
        }
        f.series[1].visible = false;
        let (_, view) = frame(&f, true, vec![]);
        assert!(view.visibility.is_none());
        assert!(view.bounds[3] < 10.);
        f.legend = false;
        let (_, view) = frame(&f, false, vec![]);
        assert!(view.visibility.is_none());
        assert!(view.bounds[3] < 10.);
        f.legend = true;
        let (_, view) = frame(&f, true, vec![]);
        assert!(view.visibility.is_none());
        // Another figure with identical series indexes must not inherit hiding.
        let mut other = f.clone();
        other.series[1].visible = true;
        let (_, view) = frame(&other, true, vec![]);
        assert!(view.visibility.is_none());
        assert!(view.bounds[3] >= 300.);
        let (_, view) = frame(&f, true, vec![]);
        assert!(view.visibility.is_none());
        assert!(view.bounds[3] < 10.);
        f.series[0].visible = false;
        let (_, view) = frame(&f, true, vec![]);
        assert!(view.bounds.iter().all(|v| v.is_finite()));
        assert!(view.visibility.is_none());
        f.series[1].label.clear();
        let (_, view) = frame(&f, true, vec![]);
        assert!(view.visibility.is_none());
    }
    #[test]
    fn native_legend_pointer_gestures_do_not_pick_samples_or_zoom_underlying_axes() {
        let ctx = egui::Context::default();
        let mut e = matrust::Engine::default();
        e.execute("plot([0 1],[1 2]);legend('Series 00');").unwrap();
        let mut f = e.figure.unwrap();
        for i in 1..64 {
            let mut s = f.series[0].clone();
            s.label = format!("Series {i:02}");
            f.series.push(s);
        }
        let points = vec![vec![PlotPoint::new(0., 1.), PlotPoint::new(1., 2.)]; 64];
        let frame = |reset, events, modifiers| {
            let mut view = None;
            let output = crate::headless::run(
                &ctx,
                egui::RawInput {
                    screen_rect: Some(egui::Rect::from_min_size(
                        egui::Pos2::ZERO,
                        egui::vec2(700., 500.),
                    )),
                    events: std::iter::once(egui::Event::ModifiersChanged(modifiers))
                        .chain(events)
                        .collect(),
                    ..Default::default()
                },
                |root| {
                    egui::CentralPanel::default().show(root, |ui| {
                        view = Some(show_interactive(
                            ui,
                            &f,
                            &points,
                            f.axis_transforms(),
                            reset,
                            None,
                        ));
                    });
                },
            );
            (output, view.unwrap())
        };
        let mut pos = None;
        let mut bounds = [0.; 4];
        for i in 0..3 {
            let (out, view) = frame(i == 0, vec![], egui::Modifiers::NONE);
            bounds = view.bounds;
            pos = out.shapes.iter().find_map(|s| match &s.shape {
                egui::Shape::Text(t) if t.galley.text() == "Series 00" => {
                    Some(t.pos + t.galley.size() * 0.5)
                }
                _ => None,
            });
        }
        let pos = pos.unwrap();
        let ctrl = egui::Modifiers {
            ctrl: true,
            ..Default::default()
        };
        for pressed in [true, false] {
            let (_, view) = frame(
                false,
                vec![
                    egui::Event::PointerMoved(pos),
                    egui::Event::PointerButton {
                        pos,
                        button: egui::PointerButton::Primary,
                        pressed,
                        modifiers: ctrl,
                    },
                ],
                ctrl,
            );
            assert!(view.pick.is_none());
            assert_eq!(view.bounds, bounds);
            if !pressed {
                assert!(!view.visibility.unwrap()[0]);
            }
        }
        let (_, view) = frame(
            false,
            vec![
                egui::Event::PointerMoved(pos),
                egui::Event::MouseWheel {
                    phase: egui::TouchPhase::Move,
                    unit: egui::MouseWheelUnit::Point,
                    delta: egui::vec2(0., -500.),
                    modifiers: egui::Modifiers::NONE,
                },
            ],
            egui::Modifiers::NONE,
        );
        assert_eq!(view.bounds, bounds);
        for _ in 0..10 {
            assert_eq!(frame(false, vec![], egui::Modifiers::NONE).1.bounds, bounds);
        }
    }
    #[test]
    fn automatic_log_views_remain_automatic_for_visibility_changes() {
        let mut e = matrust::Engine::default();
        e.execute("loglog([1 10 100],[1 10 100]);").unwrap();
        let f = e.figure.unwrap();
        let axes = f.axis_transforms();
        let points = vec![
            f.series[0]
                .display_points_axes(1024, axes)
                .into_iter()
                .map(|p| PlotPoint::new(p[0], p[1]))
                .collect(),
        ];
        let ctx = egui::Context::default();
        let _ = crate::headless::run(&ctx, Default::default(), |root| {
            egui::CentralPanel::default().show(root, |ui| {
                let view = show_interactive(ui, &f, &points, axes, true, None);
                assert_eq!(view.auto, [true; 2]);
            });
        });
    }
    #[test]
    fn fractional_linear_ticks_keep_distinct_labels() {
        for (value, step, expected) in [
            (0.25, 0.25, "0.25"),
            (0.5, 0.5, "0.5"),
            (1.5, 0.5, "1.5"),
            (2.5, 2.5, "2.5"),
            (100., 100., "100"),
            (1e6, 1e5, "1e6"),
            (0.00025, 0.00025, "2.5e-4"),
            (-0., 0.5, "0"),
        ] {
            assert_eq!(
                linear_tick(egui_plot::GridMark {
                    value,
                    step_size: step
                }),
                expected
            );
        }
        let mut engine = matrust::Engine::default();
        engine
            .execute("plot([0 2],[0 1]);axis([0 2 0 1]);")
            .unwrap();
        let f = engine.figure.unwrap();
        let points = vec![vec![PlotPoint::new(0., 0.), PlotPoint::new(2., 1.)]];
        let ctx = egui::Context::default();
        let mut text = Vec::new();
        for frame in 0..4 {
            let output = crate::headless::run(
                &ctx,
                egui::RawInput {
                    screen_rect: Some(egui::Rect::from_min_size(
                        egui::Pos2::ZERO,
                        egui::vec2(600., 400.),
                    )),
                    ..Default::default()
                },
                |root| {
                    egui::CentralPanel::default().show(root, |ui| {
                        show(ui, &f, &points, f.axis_transforms(), frame == 0);
                    });
                },
            );
            text = output
                .shapes
                .into_iter()
                .filter_map(|s| {
                    if let egui::Shape::Text(t) = s.shape {
                        Some(t.galley.text().to_owned())
                    } else {
                        None
                    }
                })
                .collect();
        }
        assert!(
            text.iter().any(|s| s == "1.5"),
            "Horizontal fractional tick missing: {text:?}"
        );
    }
    #[test]
    fn linear_axes_keep_numeric_tick_labels_inside_manual_ranges() {
        let mut engine = matrust::Engine::default();
        engine
            .execute("plot([1 10 100],[1 2 4]);axis([1 100 0 8]);xlabel('Time');")
            .unwrap();
        let f = engine.figure.unwrap();
        let points = vec![
            f.series[0]
                .points
                .iter()
                .map(|p| PlotPoint::new(p[0], p[1]))
                .collect(),
        ];
        for width in [400., 600., 900.] {
            let ctx = egui::Context::default();
            let mut labels = Vec::new();
            for frame in 0..4 {
                let output = crate::headless::run(
                    &ctx,
                    egui::RawInput {
                        screen_rect: Some(egui::Rect::from_min_size(
                            egui::Pos2::ZERO,
                            egui::vec2(width, 500.),
                        )),
                        ..Default::default()
                    },
                    |root| {
                        egui::CentralPanel::default().show(root, |ui| {
                            show(ui, &f, &points, f.axis_transforms(), frame == 0);
                        });
                    },
                );
                labels.clear();
                fn text(shape: &egui::Shape, labels: &mut Vec<String>) {
                    match shape {
                        egui::Shape::Text(t) => labels.push(t.galley.text().to_owned()),
                        egui::Shape::Vec(shapes) => {
                            for s in shapes {
                                text(s, labels)
                            }
                        }
                        _ => {}
                    }
                }
                for shape in output.shapes {
                    text(&shape.shape, &mut labels);
                }
            }
            assert!(
                labels
                    .iter()
                    .filter_map(|s| s.parse::<f64>().ok())
                    .any(|n| n > 8. && n < 100.),
                "Missing horizontal labels at width {width}: {labels:?}"
            );
        }
    }
    #[test]
    fn native_log_axes_render_and_return_data_space_zoom_bounds() {
        for script in [
            "loglog([1 10 100],[1 10 100]);axis([1 100 1 100]);",
            "loglog([-100 -10 -1],[-100 -10 -1]);axis([-100 -1 -100 -1]);",
            "bar([1 10],[0.1 100]);xscale('log');yscale('log');axis([0.1 100 0.1 100]);",
            "semilogx([0 1 10 NaN 100],[1 2 3 4 5]);axis([1 100 0 6]);",
        ] {
            let mut engine = matrust::Engine::default();
            engine.execute(script).unwrap();
            let f = engine.figure.unwrap();
            let axes = f.axis_transforms();
            let points: Vec<_> = f
                .series
                .iter()
                .map(|s| {
                    s.display_points_axes(1024, axes)
                        .into_iter()
                        .map(|[x, y]| PlotPoint::new(x, y))
                        .collect()
                })
                .collect();
            let ctx = egui::Context::default();
            let mut bounds = [0.; 4];
            for frame in 0..3 {
                let output = crate::headless::run(
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
                            bounds = show(ui, &f, &points, axes, frame == 0);
                        });
                    },
                );
                let meshes = ctx.tessellate(output.shapes, output.pixels_per_point);
                assert!(!meshes.is_empty());
                for mesh in meshes {
                    if let egui::epaint::Primitive::Mesh(mesh) = mesh.primitive {
                        assert!(mesh.vertices.iter().all(|v| v.pos.is_finite()));
                    }
                }
            }
            let (x, y) = f.bounds();
            for (a, b) in bounds.into_iter().zip([x[0], x[1], y[0], y[1]]) {
                assert!((a - b).abs() <= 1e-12 * b.abs().max(1.));
            }
        }
    }
    #[test]
    fn ticks_axes_desktop_manual_labels_rotation_and_finite_reset() {
        let mut engine = matrust::Engine::default();
        engine.execute("plot(1:10); xticks([2 8]); xticklabels({'alpha','beta'}); xtickangle(45); ylim([0 inf]);").unwrap();
        let f = engine.figure.unwrap();
        let points: Vec<_> = f
            .series
            .iter()
            .map(|s| {
                s.display_points(1024)
                    .into_iter()
                    .map(|[x, y]| PlotPoint::new(x, y))
                    .collect()
            })
            .collect();
        let ctx = egui::Context::default();
        let mut seen = Vec::new();
        for frame in 0..3 {
            let mut bounds = [0.; 4];
            let output = crate::headless::run(
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
                        bounds = show(ui, &f, &points, f.axis_transforms(), frame == 0);
                    });
                },
            );
            assert!(bounds.iter().all(|v| v.is_finite()));
            for shape in &output.shapes {
                // egui_plot paints an invisible, unrotated copy to reserve the gutter.
                if let egui::Shape::Text(t) = &shape.shape
                    && ["alpha", "beta"].contains(&t.galley.text())
                    && t.fallback_color != Color32::TRANSPARENT
                {
                    assert!((t.angle + std::f32::consts::FRAC_PI_4).abs() < 1e-5);
                    seen.push(t.galley.text().to_owned());
                }
            }
            let meshes = ctx.tessellate(output.shapes, output.pixels_per_point);
            for mesh in meshes {
                if let egui::epaint::Primitive::Mesh(mesh) = mesh.primitive {
                    assert!(mesh.vertices.iter().all(|v| v.pos.is_finite()));
                }
            }
        }
        assert!(seen.contains(&"alpha".to_owned()) && seen.contains(&"beta".to_owned()));
    }
    #[test]
    fn native_plot_styles_gaps_and_manual_limits_tessellate() {
        let mut engine = matrust::Engine::default();
        engine.execute("plot([0 1 2 3],[0 1 NaN 1],'r-.o');hold on;bar([0.5 1.5],[0.4 -0.2]);scatter([1 2],[0.3 0.5]);legend('Line','Bars','Points');axis([0 3 -1 2]);").unwrap();
        let f = engine.figure.unwrap();
        let points: Vec<_> = f
            .series
            .iter()
            .map(|s| {
                s.display_points(1024)
                    .into_iter()
                    .map(|[x, y]| PlotPoint::new(x, y))
                    .collect()
            })
            .collect();
        let ctx = egui::Context::default();
        let mut bounds = [0.; 4];
        for frame in 0..3 {
            let output = crate::headless::run(
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
                        bounds = show(ui, &f, &points, f.axis_transforms(), frame == 0);
                    });
                },
            );
            let meshes = ctx.tessellate(output.shapes, output.pixels_per_point);
            assert!(!meshes.is_empty());
            for mesh in meshes {
                if let egui::epaint::Primitive::Mesh(mesh) = mesh.primitive {
                    assert!(mesh.vertices.iter().all(|v| v.pos.is_finite()));
                }
            }
        }
        assert_eq!(bounds, [0., 3., -1., 2.]);
        let r = egui::Rect::from_min_max(egui::pos2(0., 0.), egui::pos2(10., 10.));
        assert_eq!(
            clip(egui::pos2(-100., 5.), egui::pos2(100., 5.), r),
            Some([egui::pos2(0., 5.), egui::pos2(10., 5.)])
        );
    }
}

#[cfg(test)]
mod line_width_tests {
    use super::*;
    #[test]
    fn line_width_native_lines_and_marker_edges_use_point_conversion() {
        let mut e = matrust::Engine::default();
        e.execute("plot([0 1 2],[0 1 0],'ro-');").unwrap();
        let mut figure = e.figure.unwrap();
        figure.series[0].line_width = Some(3.);
        let data: Vec<_> = figure
            .series
            .iter()
            .map(|s| {
                s.display_points(100)
                    .into_iter()
                    .map(|[x, y]| PlotPoint::new(x, y))
                    .collect()
            })
            .collect();
        let ctx = egui::Context::default();
        let mut widths = Vec::new();
        fn collect(shape: &egui::Shape, out: &mut Vec<f32>) {
            match shape {
                egui::Shape::Path(p)
                    if p.stroke.color == egui::epaint::ColorMode::Solid(Color32::RED) =>
                {
                    out.push(p.stroke.width)
                }
                egui::Shape::Circle(c) if c.stroke.color == Color32::RED => {
                    out.push(c.stroke.width)
                }
                egui::Shape::Vec(shapes) => {
                    for s in shapes {
                        collect(s, out);
                    }
                }
                _ => {}
            }
        }
        for _ in 0..3 {
            let output = crate::headless::run(
                &ctx,
                egui::RawInput {
                    screen_rect: Some(egui::Rect::from_min_size(
                        egui::Pos2::ZERO,
                        egui::vec2(960., 560.),
                    )),
                    ..Default::default()
                },
                |root| {
                    egui::CentralPanel::default().show(root, |ui| {
                        show(ui, &figure, &data, [AxisTransform::default(); 2], true);
                    });
                },
            );
            widths.clear();
            for shape in output.shapes {
                collect(&shape.shape, &mut widths);
            }
        }
        assert!(widths.len() >= 4, "{widths:?}");
        assert!(
            widths.iter().all(|width| (*width - 4.).abs() < 1e-6),
            "{widths:?}"
        );
    }
}

#[cfg(test)]
mod marker_style_tests {
    use super::*;
    #[test]
    fn marker_styles_native_shapes_have_expected_radius_and_stroke_width() {
        use matrust::plot::MarkerShape as M;
        for marker in M::ALL {
            let mut engine = matrust::Engine::default();
            engine.execute(&format!("scatter([1],[1],'Marker','{}','SizeData',144,'LineWidth',1.5,'MarkerEdgeColor','r');axis([0 2 0 2]);",marker.symbol())).unwrap();
            let figure = engine.figure.unwrap();
            let ctx = egui::Context::default();
            let data = vec![vec![PlotPoint::new(1., 1.)]];
            let mut count = 0;
            fn inspect(shape: &egui::Shape, marker: M, count: &mut usize) {
                match shape {
                    egui::Shape::Circle(c) if c.stroke.color == Color32::RED => {
                        assert_eq!(c.stroke.width, 2.);
                        assert!(
                            (c.radius - if marker == M::Point { 8. / 3. } else { 8. }).abs() < 1e-6
                        );
                        *count += 1;
                    }
                    egui::Shape::Path(p)
                        if p.stroke.color == egui::epaint::ColorMode::Solid(Color32::RED) =>
                    {
                        assert_eq!(p.stroke.width, 2.);
                        assert!(p.closed && (3..=4).contains(&p.points.len()));
                        assert!(p.points.iter().all(|p| p.is_finite()));
                        let bounds = egui::Rect::from_points(&p.points);
                        assert!(
                            bounds.width() <= 16.01
                                && bounds.height() <= 16.01
                                && bounds.width() > 5.
                        );
                        *count += 1;
                    }
                    egui::Shape::LineSegment { points, stroke } if stroke.color == Color32::RED => {
                        assert_eq!(stroke.width, 2.);
                        assert!(((points[1] - points[0]).length() - 16.).abs() < 0.01);
                        *count += 1;
                    }
                    egui::Shape::Vec(shapes) => {
                        for s in shapes {
                            inspect(s, marker, count)
                        }
                    }
                    _ => {}
                }
            }
            for _ in 0..3 {
                let output = crate::headless::run(
                    &ctx,
                    egui::RawInput {
                        screen_rect: Some(egui::Rect::from_min_size(
                            egui::Pos2::ZERO,
                            egui::vec2(960., 560.),
                        )),
                        ..Default::default()
                    },
                    |root| {
                        egui::CentralPanel::default().show(root, |ui| {
                            show(ui, &figure, &data, [AxisTransform::default(); 2], true);
                        });
                    },
                );
                count = 0;
                for shape in output.shapes {
                    inspect(&shape.shape, marker, &mut count);
                }
            }
            assert_eq!(
                count,
                match marker {
                    M::Plus | M::Cross => 2,
                    M::Asterisk => 3,
                    _ => 1,
                },
                "{marker:?}"
            );
        }
    }
}

#[cfg(test)]
mod line_style_none_tests {
    use super::*;
    #[test]
    fn line_style_none_native_stroke_has_bounds_but_no_hover_geometry_or_shapes() {
        let line = StyledLine {
            line: Line::new("Hidden stroke", vec![[1., 2.], [3., 4.]]).color(Color32::RED),
            width: 2.,
            pattern: LinePattern::None,
            dash_budget: 0.,
        };
        assert!(matches!(line.geometry(), PlotGeometry::None));
        assert_eq!(line.bounds().min(), [1., 2.]);
        assert_eq!(line.bounds().max(), [3., 4.]);
        let transform = PlotTransform::new(
            egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(200., 200.)),
            PlotBounds::from_min_max([0., 0.], [5., 5.]),
            [false; 2],
        );
        let ctx = egui::Context::default();
        let mut shapes = Vec::new();
        crate::headless::run(&ctx, Default::default(), |root| {
            egui::CentralPanel::default().show(root, |ui| line.shapes(ui, &transform, &mut shapes));
        });
        assert!(
            shapes.is_empty(),
            "Even a zero dash budget cannot turn None into Solid"
        );
    }
}

// Same bounded native dash policy as StyledLine. Clipped fill boundaries are never stroked.
fn face_edge_path(
    path: &mut Vec<egui::Pos2>,
    stroke: egui::Stroke,
    pattern: LinePattern,
    budget: &mut f64,
    shapes: &mut Vec<egui::Shape>,
) {
    if path.len() < 2 {
        path.clear();
        return;
    }
    let length: f64 = path
        .windows(2)
        .map(|p| f64::from(p[0].distance(p[1])))
        .sum();
    if pattern == LinePattern::Solid || length > *budget {
        shapes.push(egui::Shape::line(std::mem::take(path), stroke));
    } else {
        *budget -= length;
        let (dash, gap): (&[f32], &[f32]) = match pattern {
            LinePattern::Dashed => (&[8.], &[5.]),
            LinePattern::Dotted => (&[2.], &[4.]),
            LinePattern::DashDot => (&[8., 2.], &[4., 4.]),
            _ => {
                path.clear();
                return;
            }
        };
        shapes.extend(egui::Shape::dashed_line_with_offset(
            path, stroke, dash, gap, 0.,
        ));
        path.clear();
    }
}

struct StyledPatch<'a> {
    line: Line<'a>,
    width: usize,
    edge: Option<Color32>,
    stroke_width: f32,
    pattern: LinePattern,
    dash_budget: f64,
    bar: Option<([AxisTransform; 2], f64, f64, f64)>,
    fill: bool,
    /// Per-face colours and fill alpha of a surface (surf, mesh); None for
    /// one-colour patches.
    surface: Option<SurfaceStyle>,
    /// Per-vertex scalar CData of a FaceColor 'interp' fill; None otherwise.
    interp: Option<InterpFill>,
}
/// Colormap, limits and band budget an interpolated fill is painted with.
struct InterpFill {
    cdata: matrust::plot::VertexCData,
    map: std::sync::Arc<Vec<[f64; 3]>>,
    clim: [f64; 2],
    limit: usize,
}
impl InterpFill {
    fn of(
        series: &matrust::plot::Series,
        axes: &matrust::plot::Axes,
        drawn_points: usize,
    ) -> Option<Self> {
        let cdata = series.patch_vertex_cdata.as_ref()?;
        (series.face_color_mode == matrust::plot::FaceColorMode::Flat
            && series.patch.map(|p| p.vertices_per_face()) == Some(3)
            && cdata.corners.len() == drawn_points)
            .then(|| Self {
                cdata: cdata.clone(),
                map: axes.extras.effective_colormap(),
                clim: axes.color_limits(),
                limit: matrust::plot::interp_band_limit(drawn_points / 3),
            })
    }
}
/// Per-face fill/edge colours and the fill alpha of a surface patch series;
/// `faces` gives each face of a merged several-surface draw list its own
/// surface's alpha and outline style.
struct SurfaceStyle {
    fill: Option<std::sync::Arc<[[u8; 3]]>>,
    edges: Option<std::sync::Arc<[[u8; 3]]>>,
    alpha: Option<f64>,
    faces: Option<std::sync::Arc<matrust::plot::FaceStyles>>,
}
impl SurfaceStyle {
    fn of(series: &matrust::plot::Series) -> Option<Self> {
        (series.face_colors.is_some()
            || series.face_edge_colors.is_some()
            || series.face_alpha.is_some()
            || series.face_styles.is_some())
        .then(|| Self {
            fill: series.face_colors.clone(),
            edges: series.face_edge_colors.clone(),
            alpha: series.face_alpha,
            faces: series.face_styles.clone(),
        })
    }
}
impl PlotItem for StyledPatch<'_> {
    fn shapes(&self, _ui: &egui::Ui, transform: &PlotTransform, shapes: &mut Vec<egui::Shape>) {
        let PlotGeometry::Points(points) = self.line.geometry() else {
            return;
        };
        if !matches!(self.width, 3 | 4) && !(self.width == 1 && self.bar.is_some()) {
            return;
        }
        let min = transform.bounds().min();
        let max = transform.bounds().max();
        let bounds = [min[0], max[0], min[1], max[1]];
        let frame = transform.frame();
        let rect = [
            f64::from(frame.left()),
            f64::from(frame.top()),
            f64::from(frame.right()),
            f64::from(frame.bottom()),
        ];
        let mut budget = self.dash_budget;
        let surface = self.surface.as_ref();
        let alpha = surface.and_then(|s| s.alpha).unwrap_or(1.);
        for (face_index, face) in points.chunks_exact(self.width).enumerate() {
            let mut vertices = [[0.; 2]; 4];
            let mut edges = [true; 4];
            let width = if let Some((axes, width, offset, baseline)) = self.bar {
                let p = face[0];
                if !axes[0].accepts(p.x) || !axes[1].accepts(p.y) {
                    continue;
                }
                let l = axes[0].forward(p.x + offset - width / 2.);
                let r = axes[0].forward(p.x + offset + width / 2.);
                edges[3] = l.is_finite();
                edges[1] = r.is_finite();
                let l = if l.is_finite() { l } else { min[0] };
                let r = if r.is_finite() { r } else { max[0] };
                let base = axes[1].forward(baseline);
                let y = axes[1].forward(p.y);
                vertices = [[l, base], [r, base], [r, y], [l, y]];
                4
            } else {
                for (out, p) in vertices.iter_mut().zip(face) {
                    *out = [p.x, p.y];
                }
                self.width
            };
            let own = surface
                .and_then(|s| s.faces.as_ref())
                .and_then(|t| t.get(face_index));
            let alpha = own.map_or(alpha, |f| f.alpha.unwrap_or(1.));
            if let Some(style) = surface {
                // A projected surface quad may be non-convex: fill it as two
                // triangles, each clipped to the plot.
                let rgb = style.fill.as_ref().and_then(|f| f.get(face_index).copied());
                let base = rgb.map_or_else(
                    || PlotItem::color(&self.line),
                    |c| Color32::from_rgb(c[0], c[1], c[2]),
                );
                let color = if alpha < 1. {
                    Color32::from_rgba_unmultiplied(
                        base.r(),
                        base.g(),
                        base.b(),
                        (alpha * 255.).round() as u8,
                    )
                } else {
                    base
                };
                if self.fill && alpha > 0. {
                    let triangles: &[[usize; 3]] = if width == 4 {
                        &[[0, 1, 2], [0, 2, 3]]
                    } else {
                        &[[0, 1, 2]]
                    };
                    // A plain triangle mesh, not a stroked/feathered polygon:
                    // epaint does not cut sharp corners of closed paths, so a
                    // sliver face (common in small tiles) grew long spikes.
                    // Both halves share one mesh, so the quad has no seam.
                    let mut mesh = egui::Mesh::default();
                    for t in triangles {
                        let corners = t.map(|i| vertices[i]);
                        let clipped = matrust::plot::clip_patch_face(&corners, bounds);
                        let points = clipped.points();
                        if points.len() >= 3 {
                            let first = mesh.vertices.len() as u32;
                            for &p in points {
                                let [x, y] = matrust::plot::patch_screen_point(p, bounds, rect);
                                mesh.colored_vertex(egui::pos2(x as f32, y as f32), color);
                            }
                            for i in 1..points.len() as u32 - 1 {
                                mesh.add_triangle(first, first + i, first + i + 1);
                            }
                        }
                    }
                    if !mesh.is_empty() {
                        shapes.push(egui::Shape::mesh(mesh));
                    }
                }
            }
            let clipped = matrust::plot::clip_patch_face(&vertices[..width], bounds);
            let interp = self
                .interp
                .as_ref()
                .filter(|_| width == 3)
                .and_then(|f| f.cdata.triangle(face_index).map(|v| (f, v)));
            if let Some((fill, values)) = interp
                && self.fill
                && surface.is_none()
            {
                // One mesh per triangle: each colormap band is a convex fan.
                let mut mesh = egui::Mesh::default();
                matrust::plot::interp_triangle_bands(
                    [vertices[0], vertices[1], vertices[2]],
                    values,
                    bounds,
                    &fill.map,
                    fill.clim,
                    fill.limit,
                    |band, c| {
                        let color = Color32::from_rgb(c[0], c[1], c[2]);
                        let first = mesh.vertices.len() as u32;
                        for &p in band {
                            let [x, y] = matrust::plot::patch_screen_point(p, bounds, rect);
                            mesh.colored_vertex(egui::pos2(x as f32, y as f32), color);
                        }
                        for i in 1..band.len() as u32 - 1 {
                            mesh.add_triangle(first, first + i, first + i + 1);
                        }
                    },
                );
                if !mesh.is_empty() {
                    shapes.push(egui::Shape::mesh(mesh));
                }
            } else if self.fill && surface.is_none() && clipped.points().len() >= 3 {
                let pixels: Vec<_> = clipped
                    .points()
                    .iter()
                    .map(|&p| {
                        let [x, y] = matrust::plot::patch_screen_point(p, bounds, rect);
                        egui::pos2(x as f32, y as f32)
                    })
                    .collect();
                shapes.push(egui::Shape::convex_polygon(
                    pixels,
                    PlotItem::color(&self.line),
                    egui::Stroke::NONE,
                ));
            }
            let edge = match surface.and_then(|s| s.edges.as_ref()) {
                _ if own.is_some_and(|f| !f.outline) => None,
                Some(list) => list
                    .get(face_index)
                    .map(|c| Color32::from_rgb(c[0], c[1], c[2])),
                None => self.edge,
            };
            let (stroke_width, pattern) = own.map_or((self.stroke_width, self.pattern), |f| {
                (f.edge_width, f.pattern)
            });
            if let Some(edge) = edge
                && pattern != LinePattern::None
            {
                let stroke = egui::Stroke::new(stroke_width, edge);
                let mut path = Vec::with_capacity(8);
                for i in 0..width {
                    if !edges[i] {
                        face_edge_path(&mut path, stroke, pattern, &mut budget, shapes);
                        continue;
                    }
                    let segment = matrust::plot::clip_face_edge(
                        vertices[i],
                        vertices[(i + 1) % width],
                        bounds,
                    );
                    if let Some([a, b]) = segment {
                        let pixel = |p| {
                            let [x, y] = matrust::plot::patch_screen_point(p, bounds, rect);
                            egui::pos2(x as f32, y as f32)
                        };
                        let a = pixel(a);
                        let b = pixel(b);
                        if path.last().is_some_and(|p| *p != a) {
                            face_edge_path(&mut path, stroke, pattern, &mut budget, shapes);
                        }
                        if path.is_empty() {
                            path.push(a);
                        }
                        path.push(b);
                    } else {
                        face_edge_path(&mut path, stroke, pattern, &mut budget, shapes);
                    }
                }
                face_edge_path(&mut path, stroke, pattern, &mut budget, shapes);
            }
        }
    }
    fn initialize(&mut self, range: std::ops::RangeInclusive<f64>) {
        self.line.initialize(range);
    }
    fn color(&self) -> Color32 {
        PlotItem::color(&self.line)
    }
    fn geometry(&self) -> PlotGeometry<'_> {
        PlotGeometry::None
    }
    fn bounds(&self) -> PlotBounds {
        // The existing BarChart supplies transformed bar bounds. This borrowed
        // stroke overlay must not add its raw coordinates to log auto-bounds.
        if self.bar.is_some() {
            PlotBounds::NOTHING
        } else {
            self.line.bounds()
        }
    }
    fn base(&self) -> &PlotItemBase {
        self.line.base()
    }
    fn base_mut(&mut self) -> &mut PlotItemBase {
        self.line.base_mut()
    }
}

#[cfg(test)]
mod patch_render_tests {
    use super::*;
    #[test]
    fn native_patch_is_a_clipped_closed_fill_in_wide_and_compact_views() {
        for size in [egui::vec2(640., 480.), egui::vec2(1440., 900.)] {
            let frame = egui::Rect::from_min_size(egui::pos2(15., 25.), size);
            let transform = PlotTransform::new(
                frame,
                PlotBounds::from_min_max([-1., -1.], [1., 1.]),
                [false; 2],
            );
            let patch = StyledPatch {
                line: Line::new(
                    "Independent face",
                    vec![
                        [-f64::MAX, -f64::MAX],
                        [f64::MAX, -f64::MAX],
                        [f64::MAX, f64::MAX],
                        [-f64::MAX, f64::MAX],
                    ],
                )
                .color(Color32::RED),
                width: 4,
                edge: None,
                stroke_width: 2. / 3.,
                pattern: LinePattern::Solid,
                dash_budget: 200000.,
                bar: None,
                fill: true,
                surface: None,
                interp: None,
            };
            let ctx = egui::Context::default();
            let mut shapes = Vec::new();
            crate::headless::run(&ctx, Default::default(), |root| {
                egui::CentralPanel::default()
                    .show(root, |ui| patch.shapes(ui, &transform, &mut shapes));
            });
            assert_eq!(shapes.len(), 1);
            let egui::Shape::Path(path) = &shapes[0] else {
                panic!("Expected actual polygon path");
            };
            assert!(path.closed);
            assert_eq!(path.fill, Color32::RED);
            assert_eq!(path.points.len(), 4);
            assert!(
                path.points
                    .iter()
                    .all(|p| p.is_finite() && frame.contains(*p))
            );
            assert_eq!(
                path.points
                    .iter()
                    .map(|p| p.x)
                    .fold(f32::INFINITY, f32::min),
                frame.left()
            );
            assert_eq!(
                path.points
                    .iter()
                    .map(|p| p.x)
                    .fold(f32::NEG_INFINITY, f32::max),
                frame.right()
            );
        }
    }
}

fn hovered_plot_id(ctx: &egui::Context) -> egui::Id {
    egui::Id::new(("hovered_plot_frame", ctx.viewport_id()))
}

/// The axes frame under the pointer in this pass, if any plot drew one.
/// A frame recorded in an earlier pass is stale: the pointer has left it.
#[derive(Clone, Copy)]
pub(super) struct HoveredPlot {
    pub frame: egui::Rect,
    pub coordinates: Option<egui::Rect>,
}

pub(super) fn hovered_plot(ctx: &egui::Context) -> Option<HoveredPlot> {
    let pass = ctx.cumulative_pass_nr();
    let id = hovered_plot_id(ctx);
    ctx.data(|d| d.get_temp::<(u64, HoveredPlot)>(id))
        .and_then(|(seen, plot)| (seen == pass).then_some(plot))
}

pub(super) fn host_theme(ctx: &egui::Context) -> BaseTheme {
    if ctx.global_style().visuals.dark_mode {
        BaseTheme::Dark
    } else {
        BaseTheme::Light
    }
}
fn rgb(color: [u8; 3]) -> Color32 {
    Color32::from_rgb(color[0], color[1], color[2])
}
pub(super) fn apply_appearance(ui: &mut egui::Ui, appearance: ResolvedAppearance) {
    let theme = match appearance.theme {
        BaseTheme::Light => egui::Theme::Light,
        BaseTheme::Dark => egui::Theme::Dark,
    };
    super::theme::apply_figure_style(ui, theme);
    if appearance.theme == BaseTheme::Dark {
        let visuals = ui.visuals_mut();
        visuals.extreme_bg_color = rgb(appearance.axes);
        visuals.override_text_color = Some(rgb(appearance.text));
        visuals.weak_text_color = Some(rgb(appearance.disabled));
        visuals.widgets.noninteractive.bg_stroke.color = rgb(appearance.axis);
        visuals.window_fill = rgb(appearance.legend_background);
        visuals.window_stroke.color = rgb(appearance.legend_border);
    }
}
pub(super) fn paint_figure_background(ui: &egui::Ui, figure: &Figure, canvas: egui::Rect) {
    let appearance = figure.resolve_appearance(host_theme(ui.ctx()));
    if !appearance.legacy || appearance.canvas != [255; 3] || ui.visuals().dark_mode {
        ui.painter().rect_filled(canvas, 0., rgb(appearance.canvas));
    }
}

#[cfg(test)]
mod figure_background_tests {
    use super::*;
    fn rects(shape: &egui::Shape, out: &mut Vec<(egui::Rect, Color32)>) {
        match shape {
            egui::Shape::Rect(r) => out.push((r.rect, r.fill)),
            egui::Shape::Vec(shapes) => {
                for shape in shapes {
                    rects(shape, out);
                }
            }
            _ => {}
        }
    }
    #[test]
    fn native_nonwhite_canvas_keeps_a_white_data_background() {
        for (size, theme) in [egui::vec2(640., 480.), egui::vec2(1440., 900.)]
            .into_iter()
            .flat_map(|s| [egui::Theme::Light, egui::Theme::Dark].map(|t| (s, t)))
        {
            let ctx = egui::Context::default();
            ctx.set_theme(theme);
            // Desktop palettes really differ; document colors remain stable.
            let _workspace = super::super::Workspace::with_context(ctx.clone(), true);
            assert_eq!(
                ctx.global_style().visuals.dark_mode,
                theme == egui::Theme::Dark
            );
            let mut figure = Figure {
                color: [18, 52, 86],
                appearance: None, // Historical fixed-Light native fixture.
                ..Default::default()
            };
            figure
                .add(vec![matrust::plot::Series::new(
                    vec![[1., 2.], [2., 3.]],
                    PlotKind::Line,
                )])
                .unwrap();
            let transforms = figure.axis_transforms();
            let points = vec![
                figure.series[0]
                    .display_points_axes(8192, transforms)
                    .into_iter()
                    .map(|[x, y]| PlotPoint::new(x, y))
                    .collect(),
            ];
            let mut rendered = Vec::new();
            for _ in 0..3 {
                let output = crate::headless::run(
                    &ctx,
                    egui::RawInput {
                        screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, size)),
                        ..Default::default()
                    },
                    |root| {
                        egui::CentralPanel::default().show(root, |ui| {
                            show_interactive(ui, &figure, &points, transforms, true, None);
                        });
                    },
                );
                rendered.clear();
                for shape in &output.shapes {
                    rects(&shape.shape, &mut rendered);
                }
            }
            let canvas = rendered
                .iter()
                .find(|(_, c)| *c == Color32::from_rgb(18, 52, 86))
                .unwrap()
                .0;
            assert!(canvas.width() > 300. && canvas.height() > 200.);
            assert!(rendered.iter().any(|(r, c)| *c == Color32::WHITE
                && r.width() > 100.
                && r.height() > 100.
                && canvas.contains_rect(*r)
                && r.width() < canvas.width()));
        }
    }

    #[test]
    fn dark_desktop_default_paper_covers_axes_text_in_all_native_layouts() {
        fn inspect(
            shape: &egui::Shape,
            clip: egui::Rect,
            paper: egui::Rect,
            labels: &mut [usize; 3],
            ticks: &mut usize,
        ) {
            match shape {
                egui::Shape::Text(t) => {
                    let text = t.galley.text();
                    let label = ["Time (s)", "Amplitude", "Panel title"]
                        .iter()
                        .position(|s| *s == text);
                    let tick = text.parse::<f64>().is_ok();
                    if label.is_some() || tick {
                        // This includes the actual rotated y-label glyph bounds.
                        let bounds = t.visual_bounding_rect();
                        assert!(
                            paper.contains_rect(bounds),
                            "{text}: {bounds:?} outside white paper {paper:?}"
                        );
                        assert!(
                            clip.contains_rect(bounds),
                            "{text}: glyphs clipped by {clip:?}"
                        );
                        for section in &t.galley.job.sections {
                            let color = t.override_text_color.unwrap_or_else(|| {
                                if section.format.color == Color32::PLACEHOLDER {
                                    t.fallback_color
                                } else {
                                    section.format.color
                                }
                            });
                            assert!(
                                color.r() < 128 && color.g() < 128 && color.b() < 128,
                                "{text}: light-theme document text expected, got {color:?}"
                            );
                        }
                        if let Some(index) = label {
                            labels[index] += 1;
                        }
                        *ticks += usize::from(tick);
                    }
                }
                egui::Shape::Vec(shapes) => {
                    for shape in shapes {
                        inspect(shape, clip, paper, labels, ticks);
                    }
                }
                _ => {}
            }
        }
        let series =
            "plot([0 1 2],[0 1 0]);xlabel('Time (s)');ylabel('Amplitude');title('Panel title');";
        for (kind, script, axes_count) in [
            ("ordinary", series.to_owned(), 1),
            (
                "grid",
                format!("subplot(1,2,1);{series}subplot(1,2,2);{series}"),
                2,
            ),
            (
                "custom",
                format!(
                    "subplot('Position',[0.08 0.18 0.36 0.65]);{series}subplot('Position',[0.56 0.18 0.36 0.65]);{series}"
                ),
                2,
            ),
        ] {
            let mut engine = matrust::Engine::default();
            engine.execute(&script).unwrap();
            let mut figure = engine.figure.unwrap();
            figure.appearance = None; // This regression covers legacy white-paper documents.
            let original = figure.clone();
            assert_eq!(figure.color, [255; 3]);
            for size in [egui::vec2(640., 480.), egui::vec2(1440., 900.)] {
                let ctx = egui::Context::default();
                ctx.set_theme(egui::Theme::Dark);
                let _workspace = super::super::Workspace::with_context(ctx.clone(), true);
                assert!(ctx.global_style().visuals.dark_mode);
                let dark_style = ctx.global_style().visuals.clone();
                let mut subplot = super::super::subplot_view::Canvas::default();
                let transforms = figure.axis_transforms();
                let points = figure
                    .series
                    .iter()
                    .map(|s| {
                        s.display_points_axes(8192, transforms)
                            .into_iter()
                            .map(|[x, y]| PlotPoint::new(x, y))
                            .collect()
                    })
                    .collect::<Vec<Vec<PlotPoint>>>();
                let mut final_output = None;
                for _ in 0..4 {
                    final_output = Some(crate::headless::run(
                        &ctx,
                        egui::RawInput {
                            screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, size)),
                            ..Default::default()
                        },
                        |root| {
                            egui::CentralPanel::default().show(root, |ui| {
                                assert!(ui.visuals().dark_mode);
                                if figure.layout.is_some() {
                                    assert!(subplot.show(ui, 1, &figure, false, None).is_none());
                                } else {
                                    let view = show_interactive(
                                        ui, &figure, &points, transforms, false, None,
                                    );
                                    assert!(view.visibility.is_none());
                                }
                                assert!(
                                    ui.visuals().dark_mode,
                                    "Document scope leaked into desktop"
                                );
                            });
                        },
                    ));
                }
                let output = final_output.unwrap();
                let mut rectangles = Vec::new();
                for shape in &output.shapes {
                    rects(&shape.shape, &mut rectangles);
                }
                let paper = rectangles
                    .iter()
                    .filter(|(_, color)| *color == Color32::WHITE)
                    .max_by(|a, b| a.0.area().total_cmp(&b.0.area()))
                    .unwrap_or_else(|| panic!("No white paper for {kind} at {size:?}"))
                    .0;
                assert!(paper.width() > 500. && paper.height() > 350.);
                let mut labels = [0; 3];
                let mut ticks = 0;
                for shape in &output.shapes {
                    inspect(
                        &shape.shape,
                        shape.clip_rect,
                        paper,
                        &mut labels,
                        &mut ticks,
                    );
                }
                assert_eq!(labels[0], axes_count, "Missing x-labels in {kind}");
                assert_eq!(labels[1], axes_count, "Missing y-labels in {kind}");
                if kind != "ordinary" {
                    assert_eq!(labels[2], axes_count, "Missing subplot titles in {kind}");
                }
                assert!(ticks >= axes_count * 2, "No actual ticks in {kind}");
                assert_eq!(ctx.global_style().visuals, dark_style);
                assert!(
                    figure.same_snapshot(&original),
                    "Rendering changed document or sample storage"
                );
            }
        }
    }
}
#[cfg(test)]
mod ordinary_annotation_roundtrip_tests {
    use super::*;
    fn text_shapes<'a>(
        shape: &'a egui::Shape,
        clip: egui::Rect,
        found: &mut Vec<(egui::Rect, &'a egui::epaint::TextShape)>,
    ) {
        match shape {
            egui::Shape::Text(t) if t.galley.text() == "Literal <&> textbox" => {
                found.push((clip, t))
            }
            egui::Shape::Vec(v) => {
                for s in v {
                    text_shapes(s, clip, found);
                }
            }
            _ => {}
        }
    }
    #[test]
    fn ordinary_original_and_executed_annotations_use_the_complete_document_canvas() {
        let mut source = matrust::Engine::default();
        source.execute("f=figure('Color',[.8 .9 1]);a=gca;h=plot(a,[1 2],[3 4]);legend(a,h,{'line'},'Interpreter','none','Location','eastoutside');annotation(f,'textbox',[.04 .1 .92 .3],'String','Literal <&> textbox','Interpreter','none','EdgeColor','none');").unwrap();
        let original = source.figure.unwrap();
        let script = matrust::files::matlab_plot_script(
            &original,
            &std::sync::atomic::AtomicBool::new(false),
        )
        .unwrap();
        let mut replay = matrust::Engine::default();
        replay.execute(&script).unwrap();
        let replay = replay.figure.unwrap();
        for figure in [&original, &replay] {
            assert!(figure.layout.is_none());
            let before = figure.clone();
            let axes = figure.axis_transforms();
            let points = figure
                .series
                .iter()
                .map(|s| {
                    s.display_points_axes(8192, axes)
                        .into_iter()
                        .map(|[x, y]| PlotPoint::new(x, y))
                        .collect()
                })
                .collect::<Vec<Vec<PlotPoint>>>();
            for size in [egui::vec2(640., 480.), egui::vec2(1440., 900.)] {
                for theme in [egui::Theme::Light, egui::Theme::Dark] {
                    let ctx = egui::Context::default();
                    ctx.set_theme(theme);
                    let mut area = egui::Rect::NOTHING;
                    let mut output = None;
                    for pass in 0..3 {
                        output = Some(crate::headless::run(
                            &ctx,
                            egui::RawInput {
                                screen_rect: Some(egui::Rect::from_min_size(
                                    egui::Pos2::ZERO,
                                    size,
                                )),
                                ..Default::default()
                            },
                            |root| {
                                egui::CentralPanel::default().show(root, |ui| {
                                    area = ui.available_rect_before_wrap();
                                    let view = show_interactive(
                                        ui,
                                        figure,
                                        &points,
                                        axes,
                                        pass == 0,
                                        None,
                                    );
                                    assert!(view.visibility.is_none());
                                });
                            },
                        ));
                    }
                    let output = output.unwrap();
                    let mut found = vec![];
                    for s in &output.shapes {
                        text_shapes(&s.shape, s.clip_rect, &mut found);
                    }
                    assert_eq!(
                        found.len(),
                        1,
                        "ordinary textbox must be painted exactly once"
                    );
                    let (clip, text) = found[0];
                    let rect = egui::Rect::from_min_size(text.pos, text.galley.size());
                    assert!(
                        clip.contains_rect(rect),
                        "Readably sized textbox must be fully visible: {rect:?} {clip:?}"
                    );
                    let expected = figure.textboxes[0].rectangle([
                        area.left() as f64,
                        area.top() as f64,
                        area.right() as f64,
                        area.bottom() as f64,
                    ]);
                    for (actual, expected) in [clip.left(), clip.top(), clip.right(), clip.bottom()]
                        .into_iter()
                        .zip(expected)
                    {
                        assert!(
                            (f64::from(actual) - expected).abs() < 1.,
                            "TextBox clipped against axes instead of figure canvas"
                        );
                    }
                    assert!(figure.same_snapshot(&before));
                }
            }
        }
    }
    #[test]
    fn unchanged_log_display_preserves_limit_bits_but_changed_bounds_still_decode() {
        for y in [[0.3, 13.], [-13., -0.3]] {
            let figure = Axes {
                yscale: AxisScale::Log,
                xlim: Some([-0., 3.]),
                ylim: Some(y),
                ..Default::default()
            };
            let axes = figure.axis_transforms();
            let min = [axes[0].forward(-0.), axes[1].forward(y[0])];
            let max = [axes[0].forward(3.), axes[1].forward(y[1])];
            assert_eq!(
                model_bounds(&figure, axes, min, max).map(f64::to_bits),
                [-0., 3., y[0], y[1]].map(f64::to_bits)
            );
            let moved_min = [min[0] + 0.25, min[1] + 0.125];
            let moved_max = [max[0] + 0.25, max[1] + 0.125];
            assert_eq!(
                model_bounds(&figure, axes, moved_min, moved_max).map(f64::to_bits),
                [
                    axes[0].inverse(moved_min[0]),
                    axes[0].inverse(moved_max[0]),
                    axes[1].inverse(moved_min[1]),
                    axes[1].inverse(moved_max[1])
                ]
                .map(f64::to_bits)
            );
        }
    }
}

/// Drop only this plot's owned caches; other root/child views remain intact.
pub(super) fn forget_prepared(ctx: &egui::Context, id: egui::Id) {
    ctx.data_mut(|d| {
        d.remove::<egui_plot::PlotMemory>(id);
        d.remove::<TickGutters>(id.with("artwork_tick_gutters"));
        d.remove::<matrust::plot::LegendBestCache>(id.with("legend_best"));
        d.remove::<Option<egui::Rect>>(id.with("legend_rect"));
    });
    legend::forget(ctx, id);
    super::figure_volumes::forget(ctx, id);
}

#[cfg(test)]
mod status_theme_tests {
    use super::*;

    #[test]
    fn failed_layout_error_uses_chrome_theme_before_any_plot_canvas_is_painted() {
        fn fills(shape: &egui::Shape, color: Color32) -> bool {
            match shape {
                egui::Shape::Rect(rect) => {
                    rect.fill == color && rect.rect.width() > 100. && rect.rect.height() > 30.
                }
                egui::Shape::Vec(shapes) => shapes.iter().any(|s| fills(s, color)),
                _ => false,
            }
        }
        for (theme, backdrop) in [
            (egui::Theme::Light, None),
            (egui::Theme::Dark, None),
            (egui::Theme::Light, Some([255; 3])),
            (egui::Theme::Dark, Some([255; 3])),
            (egui::Theme::Light, Some([240, 245, 230])),
            (egui::Theme::Dark, Some([240, 245, 230])),
        ] {
            let ctx = egui::Context::default();
            crate::theme::configure(&ctx);
            ctx.set_theme(theme);
            let mut engine = matrust::Engine::default();
            engine
                .execute("plot([0 1],[0 1]);legend('Signal','Location','southoutside');")
                .unwrap();
            let mut figure = engine.figure.unwrap();
            figure.appearance = None; // Preserve this legacy insufficient-space policy fixture.
            if let Some(color) = backdrop {
                figure.color = color;
            }
            let original = figure.clone();
            let points = vec![vec![PlotPoint::new(0., 0.), PlotPoint::new(1., 1.)]];
            let id = egui::Id::new("status_theme_failed_plot");
            let output = crate::headless::run(
                &ctx,
                egui::RawInput {
                    screen_rect: Some(egui::Rect::from_min_size(
                        egui::Pos2::ZERO,
                        egui::vec2(900., 100.),
                    )),
                    ..Default::default()
                },
                |root| {
                    egui::CentralPanel::default().show(root, |ui| {
                        // Same background painter and order used by show_interactive_impl.
                        if backdrop.is_some() {
                            paint_figure_background(ui, &figure, ui.available_rect_before_wrap());
                        }
                        let rect =
                            egui::Rect::from_min_size(ui.cursor().min, egui::vec2(850., 24.));
                        ui.scope_builder(egui::UiBuilder::new().max_rect(rect), |ui| {
                            let shown = show_axes(
                                ui,
                                &figure.axes,
                                &points,
                                figure.axis_transforms(),
                                false,
                                id,
                                PanelContext::default(),
                            );
                            assert!(shown.visibility.is_none() && shown.pick.is_none());
                        });
                    });
                },
            );
            assert!(egui_plot::PlotMemory::load(&ctx, id).is_none());
            let background = ctx.global_style().visuals.panel_fill;
            assert!(output.shapes.iter().any(|s| fills(&s.shape, background)));
            if theme == egui::Theme::Dark && backdrop.is_none() {
                assert!(
                    !output
                        .shapes
                        .iter()
                        .any(|s| fills(&s.shape, Color32::WHITE)),
                    "Failure must precede the white data canvas"
                );
            }
            let color = if theme == egui::Theme::Dark {
                Color32::LIGHT_RED
            } else {
                Color32::RED
            };
            crate::theme::assert_painted_status(
                &output,
                "Legend layout needs more room; enlarge the pane or choose Readable size.",
                color,
            );
            fn status_backing(shape: &egui::Shape, background: Color32) -> bool {
                match shape {
                    egui::Shape::Text(t) if t.galley.text().starts_with("Legend layout") => t
                        .galley
                        .job
                        .sections
                        .iter()
                        .all(|s| s.format.background == background),
                    egui::Shape::Vec(shapes) => {
                        shapes.iter().any(|s| status_backing(s, background))
                    }
                    _ => false,
                }
            }
            assert!(
                output
                    .shapes
                    .iter()
                    .any(|s| status_backing(&s.shape, background))
            );
            if let Some(rgb) = backdrop {
                assert!(
                    output
                        .shapes
                        .iter()
                        .any(|s| fills(&s.shape, Color32::from_rgb(rgb[0], rgb[1], rgb[2])))
                );
            }
            assert!(figure.same_snapshot(&original));
        }
    }
}

#[cfg(test)]
mod face_outline_render_tests {
    use super::*;
    #[test]
    fn native_face_fill_and_edge_are_independent_and_viewport_cuts_have_no_outline() {
        for size in [egui::vec2(640., 480.), egui::vec2(1440., 900.)] {
            let frame = egui::Rect::from_min_size(egui::pos2(10., 20.), size);
            let transform = PlotTransform::new(
                frame,
                PlotBounds::from_min_max([-1., -1.], [1., 1.]),
                [false; 2],
            );
            for extreme in [false, true] {
                let m = if extreme { f64::MAX } else { 0.5 };
                let patch = StyledPatch {
                    line: Line::new("face", vec![[-m, -m], [m, -m], [m, m], [-m, m]])
                        .color(Color32::BLUE),
                    width: 4,
                    edge: Some(Color32::RED),
                    stroke_width: 2.,
                    pattern: LinePattern::Solid,
                    dash_budget: 200000.,
                    bar: None,
                    fill: true,
                    surface: None,
                    interp: None,
                };
                let ctx = egui::Context::default();
                let mut shapes = Vec::new();
                crate::headless::run(&ctx, Default::default(), |root| {
                    egui::CentralPanel::default()
                        .show(root, |ui| patch.shapes(ui, &transform, &mut shapes));
                });
                let fills = shapes
                    .iter()
                    .filter(|s| matches!(s,egui::Shape::Path(p) if p.fill==Color32::BLUE))
                    .count();
                let edges: Vec<_> = shapes
                    .iter()
                    .filter_map(|s| match s {
                        egui::Shape::Path(p)
                            if p.stroke.color == egui::epaint::ColorMode::Solid(Color32::RED) =>
                        {
                            Some(p)
                        }
                        _ => None,
                    })
                    .collect();
                assert_eq!(fills, 1);
                assert_eq!(edges.len(), usize::from(!extreme));
                for path in edges {
                    assert_eq!(path.stroke.width, 2.);
                    assert_eq!(path.points.first(), path.points.last());
                    assert_eq!(path.points.len(), 5);
                    assert!(path.points.iter().all(|p| frame.contains(*p)));
                }
            }
        }
    }
}

#[cfg(test)]
mod bar_log_outline_tests {
    use super::*;
    #[test]
    fn log_domain_bar_cut_does_not_acquire_an_artificial_vertical_outline() {
        for negative in [false, true] {
            let axes = [
                AxisTransform {
                    scale: AxisScale::Log,
                    negative,
                },
                AxisTransform::default(),
            ];
            let bounds = if negative { [0., 2.] } else { [-2., 0.] };
            let frame = egui::Rect::from_min_size(egui::pos2(10., 20.), egui::vec2(600., 400.));
            let transform = PlotTransform::new(
                frame,
                PlotBounds::from_min_max([bounds[0], -1.], [bounds[1], 2.]),
                [false; 2],
            );
            let patch = StyledPatch {
                line: Line::new("bar", vec![[if negative { -0.1 } else { 0.1 }, 1.]]),
                width: 1,
                edge: Some(Color32::RED),
                stroke_width: 2.,
                pattern: LinePattern::Solid,
                dash_budget: 200000.,
                bar: Some((axes, 0.8, 0., 0.)),
                fill: false,
                surface: None,
                interp: None,
            };
            assert_eq!(patch.bounds(), PlotBounds::NOTHING);
            let ctx = egui::Context::default();
            let mut shapes = Vec::new();
            crate::headless::run(&ctx, Default::default(), |root| {
                egui::CentralPanel::default()
                    .show(root, |ui| patch.shapes(ui, &transform, &mut shapes));
            });
            let cut = if negative {
                frame.right()
            } else {
                frame.left()
            };
            let mut segments = 0;
            for shape in shapes {
                if let egui::Shape::Path(p) = shape {
                    for pair in p.points.windows(2) {
                        segments += 1;
                        assert!(!(pair[0].x == cut && pair[1].x == cut));
                    }
                }
            }
            assert_eq!(segments, 3);
        }
    }
}

fn paint_baselines(
    ui: &egui::Ui,
    axes: &Axes,
    transform_y: AxisTransform,
    transform: &PlotTransform,
) {
    let frame = *transform.frame();
    let painter = ui.painter().with_clip_rect(frame.intersect(ui.clip_rect()));
    let implicit_style = matrust::plot::BaselineStyle::legacy_stem();
    let rgb = implicit_style.display_rgb();
    for index in axes.implicit_bar_baselines() {
        let Ok(value) = axes.render_baseline(index, transform_y) else {
            continue;
        };
        let y = transform
            .position_from_point(&PlotPoint::new(0., transform_y.forward(value)))
            .y;
        if !y.is_finite() || y < frame.top() || y > frame.bottom() {
            continue;
        }
        painter.line_segment(
            [egui::pos2(frame.left(), y), egui::pos2(frame.right(), y)],
            egui::Stroke::new(
                implicit_style.display_width() as f32,
                Color32::from_rgb(rgb[0], rgb[1], rgb[2]),
            ),
        );
    }
    for baseline in &axes.baselines {
        let style = baseline.style;
        if !style.visible || style.pattern == LinePattern::None {
            continue;
        }
        let Ok(value) = baseline.value.resolve(transform_y) else {
            continue;
        };
        let y = transform
            .position_from_point(&PlotPoint::new(0., transform_y.forward(value)))
            .y;
        if !y.is_finite() || y < frame.top() || y > frame.bottom() {
            continue;
        }
        let rgb = style.color.map(|v| (v * 255.).round() as u8);
        let stroke = egui::Stroke::new(
            (style.width_points * 4. / 3.) as f32,
            Color32::from_rgb(rgb[0], rgb[1], rgb[2]),
        );
        let line = [egui::pos2(frame.left(), y), egui::pos2(frame.right(), y)];
        if style.pattern == LinePattern::Solid
            || line[0].distance(line[1]) as f64 > 200000. / axes.baselines.len().max(1) as f64
        {
            // Same bounded dash-work fallback as the series renderer.
            painter.line_segment(line, stroke);
        } else {
            let (dash, gap): (&[f32], &[f32]) = match style.pattern {
                LinePattern::Dashed => (&[8.], &[5.]),
                LinePattern::Dotted => (&[2.], &[4.]),
                LinePattern::DashDot => (&[8., 2.], &[4., 4.]),
                _ => unreachable!(),
            };
            painter.extend(egui::Shape::dashed_line_with_offset(
                &line, stroke, dash, gap, 0.,
            ));
        }
    }
}

#[cfg(test)]
mod explicit_baseline_render_tests {
    use super::*;
    use matrust::plot::{Baseline, BaselineStyle, BaselineValue, Series};
    use std::sync::atomic::AtomicBool;
    fn scene() -> Figure {
        let mut f = Figure::default();
        f.series
            .push(Series::new(vec![[1., 2.], [2., 3.]], PlotKind::Stem));
        f.xlim = Some([0., 3.]);
        f.ylim = Some([-4., 4.]);
        let b = Baseline::new(
            BaselineValue::Value(-2.),
            BaselineStyle {
                color: [0.15, 0.25, 0.35],
                pattern: LinePattern::DashDot,
                width_points: 2.25,
                visible: true,
            },
            vec![f.series[0].identity.clone()],
        )
        .unwrap();
        f.replace_baselines(vec![b], &AtomicBool::new(false))
            .unwrap();
        f
    }
    fn has_color(shape: &egui::Shape, color: Color32) -> bool {
        match shape {
            egui::Shape::LineSegment { stroke, .. } => stroke.color == color,
            egui::Shape::Path(path) => path.stroke.color == egui::epaint::ColorMode::Solid(color),
            egui::Shape::Vec(v) => v.iter().any(|s| has_color(s, color)),
            _ => false,
        }
    }
    #[test]
    fn prepared_native_baseline_paints_owned_style_without_series_or_legend_items() {
        for theme in [egui::Theme::Light, egui::Theme::Dark] {
            let ctx = egui::Context::default();
            ctx.set_theme(theme);
            let mut f = scene();
            let shared = f.series[0].points.clone();
            let original = f.clone();
            let color = Color32::from_rgb(38, 64, 89);
            for visible in [true, false] {
                f.baselines[0].style.visible = visible;
                let crate::figure_geometry::Geometry::Ordinary(prepared) =
                    crate::figure_geometry::Geometry::prepare(1, &f, &AtomicBool::new(false))
                        .unwrap()
                else {
                    panic!("ordinary");
                };
                assert_eq!(prepared.points[0][0].y, -2.);
                let mut output = Default::default();
                for pass in 0..3 {
                    output = crate::headless::run(
                        &ctx,
                        egui::RawInput {
                            screen_rect: Some(egui::Rect::from_min_size(
                                egui::Pos2::ZERO,
                                egui::vec2(960., 640.),
                            )),
                            ..Default::default()
                        },
                        |root| {
                            egui::CentralPanel::default().show(root, |ui| {
                                let view = show_interactive_prepared(
                                    ui,
                                    &f,
                                    prepared.view(),
                                    pass == 0,
                                    None,
                                );
                                assert!(view.pick.is_none());
                                assert!(view.visibility.is_none());
                            });
                        },
                    );
                }
                assert_eq!(
                    output.shapes.iter().any(|s| has_color(&s.shape, color)),
                    visible
                );
                assert_eq!(f.series.len(), 1);
                assert_eq!(f.legend_entries().count(), 0);
                assert!(shared.same_storage(&f.series[0].points));
            }
            f.baselines[0].style.visible = true;
            assert!(f.same_snapshot(&original));
        }
    }
    #[test]
    fn compact_patch_and_explicit_baseline_share_real_prepared_native_canvas() {
        for theme in [egui::Theme::Light, egui::Theme::Dark] {
            let mut engine = matrust::Engine::default();
            engine.execute("x=[1;2;3];y=[2;-1;4];a=x-.25;b=x+.25;c=y*0;patch('XData',[a b b a].','YData',[c c y y].','FaceColor',[.2 .4 .6],'EdgeColor',[.6 .2 .1]);hold on;s=stem([1 2],[3 4]);bl=get(s,'BaseLine');set(bl,'Color',[.15 .25 .35],'LineWidth',2.25,'BaseValue',-2);").unwrap();
            let figure = engine.figure.unwrap();
            let before = figure.clone();
            assert!(figure.series[0].points.patch().is_some());
            let crate::figure_geometry::Geometry::Ordinary(prepared) =
                crate::figure_geometry::Geometry::prepare(1, &figure, &AtomicBool::new(false))
                    .unwrap()
            else {
                panic!("ordinary");
            };
            assert_eq!(prepared.points[0].len(), figure.series[0].points.len());
            assert_eq!(prepared.points[1][0].y, -2.);
            let ctx = egui::Context::default();
            ctx.set_theme(theme);
            let mut output = Default::default();
            for pass in 0..3 {
                output = crate::headless::run(
                    &ctx,
                    egui::RawInput {
                        screen_rect: Some(egui::Rect::from_min_size(
                            egui::Pos2::ZERO,
                            egui::vec2(960., 640.),
                        )),
                        ..Default::default()
                    },
                    |root| {
                        egui::CentralPanel::default().show(root, |ui| {
                            let view = show_interactive_prepared(
                                ui,
                                &figure,
                                prepared.view(),
                                pass == 0,
                                None,
                            );
                            assert!(view.pick.is_none());
                            assert!(view.visibility.is_none());
                        });
                    },
                );
            }
            assert!(
                output
                    .shapes
                    .iter()
                    .any(|s| has_color(&s.shape, Color32::from_rgb(38, 64, 89)))
            );
            assert!(figure.same_snapshot(&before));
            for (a, b) in figure.series.iter().zip(&before.series) {
                assert!(a.points.same_storage(&b.points));
            }
            assert!(
                crate::figure_geometry::Geometry::prepare(1, &figure, &AtomicBool::new(true))
                    .is_err()
            );
        }
    }
    #[test]
    fn corpus_scalar_fill_desktop_mesh_bound_interior_follows_palette_until_fixed_rgb() {
        fn has_fill(shape: &egui::Shape, color: Color32) -> bool {
            match shape {
                egui::Shape::Path(path) => path.closed && path.fill == color,
                egui::Shape::Vec(v) => v.iter().any(|s| has_fill(s, color)),
                _ => false,
            }
        }
        fn render_shapes(figure: &matrust::plot::Figure) -> Vec<egui::epaint::ClippedShape> {
            let crate::figure_geometry::Geometry::Ordinary(prepared) =
                crate::figure_geometry::Geometry::prepare(1, figure, &AtomicBool::new(false))
                    .unwrap()
            else {
                panic!("ordinary");
            };
            let ctx = egui::Context::default();
            let mut output = Default::default();
            for pass in 0..2 {
                output = crate::headless::run(
                    &ctx,
                    egui::RawInput {
                        screen_rect: Some(egui::Rect::from_min_size(
                            egui::Pos2::ZERO,
                            egui::vec2(640., 480.),
                        )),
                        ..Default::default()
                    },
                    |root| {
                        egui::CentralPanel::default().show(root, |ui| {
                            show_interactive_prepared(ui, figure, prepared.view(), pass == 0, None);
                        });
                    },
                );
            }
            output.shapes
        }

        let mut engine = matrust::Engine::default();
        engine
            .execute(
                "h=fill([0;2;2;0],[0;0;1;1],1); \
                 colormap([1 0 0; 0 0 1]); clim([0 2]);",
            )
            .unwrap();
        let shapes_blue = render_shapes(engine.figure.as_ref().unwrap());
        assert!(
            shapes_blue
                .iter()
                .any(|s| has_fill(&s.shape, Color32::from_rgb(0, 0, 255))),
            "expected blue mapped interior in headless desktop shapes"
        );

        engine.execute("colormap([0 1 0; 1 1 0]);").unwrap();
        let shapes_yellow = render_shapes(engine.figure.as_ref().unwrap());
        assert!(
            shapes_yellow
                .iter()
                .any(|s| has_fill(&s.shape, Color32::from_rgb(255, 255, 0))),
            "expected yellow mapped interior after colormap change"
        );
        assert!(
            !shapes_yellow
                .iter()
                .any(|s| has_fill(&s.shape, Color32::from_rgb(0, 0, 255)))
        );

        engine
            .execute(
                "set(h,'EdgeColor',[.7 .7 .7],'FaceColor',[.7 .7 .7]); \
                 colormap([1 0 1; 0 1 1]); cd=get(h,'CData');",
            )
            .unwrap();
        let cd_text = engine.workspace().get("cd").unwrap().to_string();
        assert_eq!(cd_text.trim(), "1");
        let shapes_fixed = render_shapes(engine.figure.as_ref().unwrap());
        assert!(
            shapes_fixed
                .iter()
                .any(|s| has_fill(&s.shape, Color32::from_rgb(179, 179, 179))),
            "expected fixed #b3b3b3 interior after setting RGB FaceColor"
        );
        assert!(
            !shapes_fixed
                .iter()
                .any(|s| has_fill(&s.shape, Color32::from_rgb(0, 255, 255)))
        );
    }
    #[test]
    fn corpus_vertex_fill_desktop_interpolates_within_one_triangle_and_follows_palette() {
        // Same fixture and expectations as tests/plots_2d_more.rs
        // corpus_vertex_fill_export_*: 256-row ramps, CLim [0 1], C = x/2 in
        // the first rendered triangle, so sample rows are 38 and 166.
        fn render_shapes(figure: &matrust::plot::Figure) -> Vec<egui::Shape> {
            fn collect(shape: &egui::Shape, out: &mut Vec<egui::Shape>) {
                match shape {
                    egui::Shape::Vec(v) => v.iter().for_each(|s| collect(s, out)),
                    other => out.push(other.clone()),
                }
            }
            let crate::figure_geometry::Geometry::Ordinary(prepared) =
                crate::figure_geometry::Geometry::prepare(1, figure, &AtomicBool::new(false))
                    .unwrap()
            else {
                panic!("ordinary");
            };
            let ctx = egui::Context::default();
            let mut output = Default::default();
            for pass in 0..2 {
                output = crate::headless::run(
                    &ctx,
                    egui::RawInput {
                        screen_rect: Some(egui::Rect::from_min_size(
                            egui::Pos2::ZERO,
                            egui::vec2(800., 600.),
                        )),
                        ..Default::default()
                    },
                    |root| {
                        egui::CentralPanel::default().show(root, |ui| {
                            show_interactive_prepared(ui, figure, prepared.view(), pass == 0, None);
                        });
                    },
                );
            }
            let mut shapes = Vec::new();
            for clipped in &output.shapes {
                collect(&clipped.shape, &mut shapes);
            }
            shapes
        }
        fn render_meshes(figure: &matrust::plot::Figure) -> Vec<egui::Mesh> {
            render_shapes(figure)
                .into_iter()
                .filter_map(|s| match s {
                    egui::Shape::Mesh(m) => Some((*m).clone()),
                    _ => None,
                })
                .collect()
        }
        fn has_fill(figure: &matrust::plot::Figure, color: Color32) -> bool {
            render_shapes(figure).iter().any(|s| match s {
                egui::Shape::Path(path) => path.closed && path.fill == color,
                _ => false,
            })
        }
        /// Colour of the mesh triangle under each data point; the box of
        /// the [0 2]x[0 1] patch is the extent of its band vertices.
        fn sample(meshes: &[egui::Mesh], at: &[[f64; 2]]) -> Vec<[u8; 3]> {
            let mut b = [
                f32::INFINITY,
                f32::NEG_INFINITY,
                f32::INFINITY,
                f32::NEG_INFINITY,
            ];
            for v in meshes.iter().flat_map(|m| &m.vertices) {
                b = [
                    b[0].min(v.pos.x),
                    b[1].max(v.pos.x),
                    b[2].min(v.pos.y),
                    b[3].max(v.pos.y),
                ];
            }
            assert!(b[1] - b[0] > 100. && b[3] - b[2] > 50., "patch box {b:?}");
            at.iter()
                .map(|&[x, y]| {
                    let p = egui::pos2(
                        b[0] + (x as f32 / 2.) * (b[1] - b[0]),
                        b[3] - y as f32 * (b[3] - b[2]),
                    );
                    let mut hit = None;
                    for m in meshes {
                        for t in m.indices.as_chunks::<3>().0 {
                            let [a, bb, c] = [0, 1, 2].map(|k| m.vertices[t[k] as usize]);
                            let cross = |u: egui::Pos2, w: egui::Pos2| {
                                (w.x - u.x) * (p.y - u.y) - (w.y - u.y) * (p.x - u.x)
                            };
                            let d = [
                                cross(a.pos, bb.pos),
                                cross(bb.pos, c.pos),
                                cross(c.pos, a.pos),
                            ];
                            if d.iter().all(|&v| v >= 0.) || d.iter().all(|&v| v <= 0.) {
                                hit = Some([a.color.r(), a.color.g(), a.color.b()]);
                            }
                        }
                    }
                    hit.expect("sample is covered by the interpolated fill")
                })
                .collect()
        }
        fn near(actual: &[[u8; 3]], expected: &[[u8; 3]]) -> bool {
            actual
                .iter()
                .zip(expected)
                .all(|(a, e)| a.iter().zip(e).all(|(x, y)| x.abs_diff(*y) <= 1))
        }
        let samples = [[0.3, 0.2], [1.3, 0.2]];
        let rows: [u8; 2] = [38, 166];
        let mut engine = matrust::Engine::default();
        engine
            .execute(
                "h=fill([0;2;2;0],[0;0;1;1],[0;1;1;0],'EdgeColor','none'); \
                 colormap([linspace(1,0,256)' zeros(256,1) linspace(0,1,256)']); clim([0 1]);",
            )
            .unwrap();
        let figure = engine.figure.as_ref().unwrap();
        let tri: Vec<[f64; 2]> = figure.series[0].points.iter().take(3).collect();
        assert_eq!(tri, vec![[0., 1.], [0., 0.], [2., 0.]]);
        let meshes = render_meshes(figure);
        let red_blue = sample(&meshes, &samples);
        eprintln!("desktop red->blue samples {red_blue:?}");
        assert!(
            near(&red_blue, &rows.map(|k| [255 - k, 0, k])),
            "{red_blue:?}"
        );
        assert_ne!(red_blue[0], red_blue[1]);

        engine
            .execute("colormap([linspace(0,1,256)' ones(256,1) zeros(256,1)]);")
            .unwrap();
        let green_yellow = sample(&render_meshes(engine.figure.as_ref().unwrap()), &samples);
        eprintln!("desktop green->yellow samples {green_yellow:?}");
        assert!(
            near(&green_yellow, &rows.map(|k| [k, 255, 0])),
            "{green_yellow:?}"
        );

        // A fixed row RGB fill keeps its colour after the palette changes.
        engine
            .execute("figure; g=fill([0;2;2;0],[0;0;1;1],[1 0 .5],'EdgeColor','none'); colormap([linspace(1,0,256)' zeros(256,1) linspace(0,1,256)']);")
            .unwrap();
        let pink = Color32::from_rgb(255, 0, 128);
        assert!(has_fill(engine.figure.as_ref().unwrap(), pink));
        assert!(render_meshes(engine.figure.as_ref().unwrap()).is_empty());
        engine
            .execute("colormap([linspace(0,1,256)' ones(256,1) zeros(256,1)]);")
            .unwrap();
        assert!(has_fill(engine.figure.as_ref().unwrap(), pink));
        assert!(render_meshes(engine.figure.as_ref().unwrap()).is_empty());
    }
    #[test]
    fn prepared_subplots_invalidate_on_baseline_edit_without_replacing_sample_storage() {
        let mut f = scene();
        let left_axes = f.axes.clone();
        f.select_subplot(
            matrust::plot::Position::from_grid(1, 2, &[1]).unwrap(),
            false,
        )
        .unwrap();
        f.axes = left_axes;
        f.select_subplot(
            matrust::plot::Position::from_grid(1, 2, &[2]).unwrap(),
            false,
        )
        .unwrap();
        let cancel = AtomicBool::new(false);
        let canvas = crate::subplot_view::Canvas::prepare(3, &f, &cancel).unwrap();
        assert!(canvas.matches_prepared(3, &f));
        let old = f.clone();
        for axes in f.all_axes_mut() {
            for baseline in &mut axes.baselines {
                baseline.value = BaselineValue::Value(-3.);
            }
        }
        assert!(!canvas.matches_prepared(3, &f));
        let rebuilt = crate::subplot_view::Canvas::prepare(3, &f, &cancel).unwrap();
        assert!(rebuilt.matches_prepared(3, &f));
        for ((_, a), (_, b)) in old.all_axes().zip(f.all_axes()) {
            for (s, t) in a.series.iter().zip(&b.series) {
                assert!(s.points.same_storage(&t.points));
            }
        }
    }
}

#[cfg(test)]
mod legacy_baseline_default_policy_tests {
    use super::*;
    use matrust::plot::Series;
    use std::sync::atomic::AtomicBool;
    fn count_baselines(shape: &egui::Shape) -> usize {
        let color = Color32::from_rgb(119, 119, 119);
        let horizontal = |points: &[egui::Pos2]| {
            points.len() >= 2
                && points.iter().all(|p| (p.y - points[0].y).abs() < 0.01)
                && (points.last().unwrap().x - points[0].x).abs() > 100.
        };
        match shape {
            egui::Shape::LineSegment { points, stroke } => {
                usize::from(stroke.color == color && stroke.width == 1. && horizontal(points))
            }
            egui::Shape::Path(path) => usize::from(
                path.stroke.color == egui::epaint::ColorMode::Solid(color)
                    && path.stroke.width == 1.
                    && horizontal(&path.points),
            ),
            egui::Shape::Vec(shapes) => shapes.iter().map(count_baselines).sum(),
            _ => 0,
        }
    }
    #[test]
    fn actual_prepared_legacy_stem_uses_svg_default_in_both_desktop_themes() {
        for theme in [egui::Theme::Light, egui::Theme::Dark] {
            for visible in [true, false] {
                let ctx = egui::Context::default();
                ctx.set_theme(theme);
                let mut f = Figure::default();
                let mut series = Series::new(vec![[1., 2.], [2., 4.]], PlotKind::Stem);
                series.color = [17, 129, 241];
                series.explicit_color = true;
                series.visible = visible;
                f.add(vec![series]).unwrap();
                f.legend = false;
                f.xgrid = false;
                f.ygrid = false;
                f.xlim = Some([0., 3.]);
                f.ylim = Some([-1., 5.]);
                let before = f.clone();
                let samples = f.series[0].points.clone();
                let crate::figure_geometry::Geometry::Ordinary(prepared) =
                    crate::figure_geometry::Geometry::prepare(1, &f, &AtomicBool::new(false))
                        .unwrap()
                else {
                    panic!("ordinary figure");
                };
                let mut output = Default::default();
                for pass in 0..3 {
                    output = crate::headless::run(
                        &ctx,
                        egui::RawInput {
                            screen_rect: Some(egui::Rect::from_min_size(
                                egui::Pos2::ZERO,
                                egui::vec2(960., 640.),
                            )),
                            ..Default::default()
                        },
                        |root| {
                            egui::CentralPanel::default().show(root, |ui| {
                                let view = show_interactive_prepared(
                                    ui,
                                    &f,
                                    prepared.view(),
                                    pass == 0,
                                    None,
                                );
                                assert!(view.pick.is_none());
                                assert!(view.visibility.is_none());
                            });
                        },
                    );
                }
                assert_eq!(
                    output
                        .shapes
                        .iter()
                        .map(|s| count_baselines(&s.shape))
                        .sum::<usize>(),
                    usize::from(visible)
                );
                assert_eq!(f.svg().contains("stroke=\"#777777\""), visible);
                assert!(f.same_snapshot(&before));
                assert!(f.baselines.is_empty());
                assert!(samples.same_storage(&f.series[0].points));
            }
        }
    }
}

#[cfg(test)]
mod implicit_bar_default_policy_tests {
    use super::*;
    use std::sync::atomic::AtomicBool;
    fn strokes(shape: &egui::Shape) -> usize {
        match shape {
            egui::Shape::LineSegment { points, stroke } => usize::from(
                stroke.color == Color32::from_rgb(119, 119, 119)
                    && stroke.width == 1.
                    && (points[0].y - points[1].y).abs() < 0.01
                    && (points[0].x - points[1].x).abs() > 100.,
            ),
            egui::Shape::Vec(items) => items.iter().map(strokes).sum(),
            _ => 0,
        }
    }
    #[test]
    fn new_bar_implicit_group_paints_once_and_off_remains_off_in_both_themes() {
        for theme in [egui::Theme::Light, egui::Theme::Dark] {
            for enabled in [true, false] {
                let ctx = egui::Context::default();
                ctx.set_theme(theme);
                let mut engine = matrust::Engine::default();
                engine
                    .execute("b=bar([1 2],[2 3;3 4]);hold on;c=bar([1 2],[4 5]);")
                    .unwrap();
                if !enabled {
                    engine.execute("set(b(1),'ShowBaseLine','off');").unwrap();
                }
                let mut figure = engine.figure.unwrap();
                figure.xlim = Some([0., 3.]);
                figure.ylim = Some([-1., 6.]);
                figure.legend = false;
                figure.xgrid = false;
                figure.ygrid = false;
                let before = figure.clone();
                let token = figure.observe_series_baseline(0).unwrap().identity.clone();
                assert_eq!(
                    figure.observe_baseline(&token).unwrap().style.visible,
                    enabled
                );
                let crate::figure_geometry::Geometry::Ordinary(prepared) =
                    crate::figure_geometry::Geometry::prepare(1, &figure, &AtomicBool::new(false))
                        .unwrap()
                else {
                    panic!("ordinary figure");
                };
                let mut output = Default::default();
                for pass in 0..3 {
                    output = crate::headless::run(
                        &ctx,
                        egui::RawInput {
                            screen_rect: Some(egui::Rect::from_min_size(
                                egui::Pos2::ZERO,
                                egui::vec2(960., 640.),
                            )),
                            ..Default::default()
                        },
                        |root| {
                            egui::CentralPanel::default().show(root, |ui| {
                                let view = show_interactive_prepared(
                                    ui,
                                    &figure,
                                    prepared.view(),
                                    pass == 0,
                                    None,
                                );
                                assert!(view.pick.is_none());
                                assert!(view.visibility.is_none());
                            });
                        },
                    );
                }
                assert_eq!(
                    output
                        .shapes
                        .iter()
                        .map(|s| strokes(&s.shape))
                        .sum::<usize>(),
                    usize::from(enabled)
                );
                assert_eq!(
                    figure
                        .svg()
                        .matches("data-matrust-implicit-bar-baseline")
                        .count(),
                    usize::from(enabled)
                );
                assert!(figure.baselines.is_empty());
                assert!(figure.same_snapshot(&before));
                for (a, b) in figure.series.iter().zip(&before.series) {
                    assert!(a.points.same_storage(&b.points));
                }
            }
        }
    }
}

#[cfg(test)]
mod explicit_baseline_shape_contract_tests {
    use super::*;
    use matrust::plot::{Baseline, BaselineStyle, BaselineValue, Series};
    use std::sync::atomic::AtomicBool;

    const COLOR: Color32 = Color32::from_rgb(38, 64, 89);
    fn segments(
        shape: &egui::Shape,
        clip: egui::Rect,
        out: &mut Vec<([egui::Pos2; 2], egui::Stroke, egui::Rect)>,
    ) {
        match shape {
            egui::Shape::LineSegment { points, stroke } if stroke.color == COLOR => {
                out.push((*points, *stroke, clip))
            }
            egui::Shape::Vec(shapes) => {
                for shape in shapes {
                    segments(shape, clip, out);
                }
            }
            egui::Shape::Path(path)
                if path.stroke.color == egui::epaint::ColorMode::Solid(COLOR) =>
            {
                panic!("Baseline must emit bounded line segments")
            }
            _ => {}
        }
    }
    fn scene(negative: bool, pattern: LinePattern, visible: bool, outside: bool) -> Figure {
        let sign = if negative { -1. } else { 1. };
        let mut f = Figure::default();
        f.add(vec![Series::new(
            vec![[1., sign * 3.], [2., sign * 4.]],
            PlotKind::Stem,
        )])
        .unwrap();
        f.xlim = Some([0., 3.]);
        f.yscale = AxisScale::Log;
        f.ylim = Some(if negative { [-8., -1.] } else { [1., 8.] });
        f.xgrid = false;
        f.ygrid = false;
        f.legend = false;
        let baseline = Baseline::new(
            BaselineValue::Value(sign * if outside { 16. } else { 2. }),
            BaselineStyle {
                color: [0.15, 0.25, 0.35],
                pattern,
                width_points: 2.25,
                visible,
            },
            vec![f.series[0].identity.clone()],
        )
        .unwrap();
        f.replace_baselines(vec![baseline], &AtomicBool::new(false))
            .unwrap();
        f
    }
    fn paint(
        figure: &Figure,
        theme: egui::Theme,
    ) -> Vec<([egui::Pos2; 2], egui::Stroke, egui::Rect)> {
        let crate::figure_geometry::Geometry::Ordinary(prepared) =
            crate::figure_geometry::Geometry::prepare(1, figure, &AtomicBool::new(false)).unwrap()
        else {
            panic!("ordinary fixture");
        };
        let ctx = egui::Context::default();
        ctx.set_theme(theme);
        let mut output = Default::default();
        let mut parent_clip = egui::Rect::NOTHING;
        for pass in 0..3 {
            output = crate::headless::run(
                &ctx,
                egui::RawInput {
                    screen_rect: Some(egui::Rect::from_min_size(
                        egui::Pos2::ZERO,
                        egui::vec2(960., 650.),
                    )),
                    ..Default::default()
                },
                |root| {
                    egui::CentralPanel::default().show(root, |ui| {
                        // A real narrower ancestor clip exercises the painter's clip
                        // intersection while retaining the unchanged full plot frame.
                        parent_clip = ui.clip_rect();
                        parent_clip.min.x += 80.;
                        parent_clip.max.x -= 80.;
                        ui.set_clip_rect(parent_clip);
                        let view =
                            show_interactive_prepared(ui, figure, prepared.view(), pass == 0, None);
                        assert!(view.pick.is_none());
                        assert!(view.visibility.is_none());
                        assert!(view.bounds.iter().all(|v| v.is_finite()));
                    });
                },
            );
        }
        let mut result = Vec::new();
        for shape in output.shapes {
            segments(&shape.shape, shape.clip_rect, &mut result);
        }
        for (points, stroke, clip) in &result {
            assert_eq!(
                stroke.width, 3.,
                "2.25 points must become exactly 3 logical pixels"
            );
            assert!(parent_clip.contains_rect(*clip));
            assert!(points.iter().all(|p| p.x.is_finite() && p.y.is_finite()));
            assert_eq!(points[0].y, points[1].y);
            assert!(points[0].x < points[1].x);
            assert!((clip.top()..=clip.bottom()).contains(&points[0].y));
        }
        result
    }
    #[test]
    fn prepared_baseline_width_dash_clip_and_both_log_domains_are_real_shapes() {
        for theme in [egui::Theme::Light, egui::Theme::Dark] {
            for negative in [false, true] {
                for pattern in [
                    LinePattern::Solid,
                    LinePattern::Dashed,
                    LinePattern::Dotted,
                    LinePattern::DashDot,
                ] {
                    let figure = scene(negative, pattern, true, false);
                    let before = figure.clone();
                    let lines = paint(&figure, theme);
                    let first = lines.first().expect("visible in-domain baseline");
                    assert!(first.0[0].x <= first.2.left());
                    assert!(lines.last().unwrap().0[1].x >= first.2.right());
                    // log2 lies one third through log8. Negative-domain
                    // transforms reverse that placement, not the source bits.
                    let fraction = if negative { 1. / 3. } else { 2. / 3. };
                    let expected_y = first.2.top() + first.2.height() * fraction;
                    let y_rounding = 8. * f32::EPSILON * first.2.bottom().abs().max(1.);
                    assert!((first.0[0].y - expected_y).abs() <= y_rounding);
                    if pattern == LinePattern::Solid {
                        assert_eq!(lines.len(), 1);
                    } else {
                        assert!(lines.len() > 8);
                        // epaint interpolates f32 screen positions with a
                        // multiply/divide/add; allow only a rounding bound
                        // scaled to screen coordinates, not a visual epsilon.
                        for (i, pair) in lines.windows(2).take(8).enumerate() {
                            let expected_dash = match pattern {
                                LinePattern::Dotted => 2.,
                                LinePattern::DashDot if !i.is_multiple_of(2) => 2.,
                                _ => 8.,
                            };
                            let expected_gap = if pattern == LinePattern::Dashed {
                                5.
                            } else {
                                4.
                            };
                            let magnitude = pair
                                .iter()
                                .flat_map(|s| s.0)
                                .map(|p| p.x.abs())
                                .fold(1_f32, f32::max);
                            let rounding = 8. * f32::EPSILON * magnitude;
                            assert!(
                                ((pair[0].0[1].x - pair[0].0[0].x) - expected_dash).abs()
                                    <= rounding
                            );
                            assert!(
                                ((pair[1].0[0].x - pair[0].0[1].x) - expected_gap).abs()
                                    <= rounding
                            );
                        }
                    }
                    assert!(figure.same_snapshot(&before));
                    assert!(
                        figure.series[0]
                            .points
                            .same_storage(&before.series[0].points)
                    );
                }
                for figure in [
                    scene(negative, LinePattern::Solid, false, false),
                    scene(negative, LinePattern::None, true, false),
                    scene(negative, LinePattern::Solid, true, true),
                ] {
                    assert!(paint(&figure, theme).is_empty());
                }
                let mut invalid = scene(negative, LinePattern::Solid, true, false);
                invalid.baselines[0].value = BaselineValue::Value(if negative { 2. } else { -2. });
                let before = invalid.clone();
                assert!(
                    crate::figure_geometry::Geometry::prepare(1, &invalid, &AtomicBool::new(false))
                        .is_err()
                );
                assert!(invalid.same_snapshot(&before));
            }
        }
    }
}

#[cfg(test)]
#[path = "figure_view_prepared_identity_tests.rs"]
mod prepared_identity_tests;

#[cfg(test)]
#[path = "figure_view_title_tests.rs"]
pub(super) mod title_artwork_tests;

#[cfg(test)]
#[path = "figure_view_volume_tests.rs"]
mod volume_tests;

#[cfg(test)]
#[path = "figure_axes_color_tests.rs"]
mod axes_color_tests;

/// Hover coordinates like C's `%g` with five significant digits: `5.524`,
/// `77.03`, `1.2346e+05`, `1e-05`.
pub(super) fn hover_number(value: f64) -> String {
    if value.is_nan() {
        return "NaN".into();
    }
    if value.is_infinite() {
        return if value > 0. { "Inf" } else { "-Inf" }.into();
    }
    if value == 0. {
        return "0".into();
    }
    fn trim(text: &str) -> &str {
        if text.contains('.') {
            text.trim_end_matches('0').trim_end_matches('.')
        } else {
            text
        }
    }
    // The exponent after rounding to five digits picks the notation.
    let scientific = format!("{value:.4e}");
    let (mantissa, exponent) = scientific.split_once('e').unwrap_or((&scientific, "0"));
    let exponent: i32 = exponent.parse().unwrap_or(0);
    if (-4..5).contains(&exponent) {
        let decimals = (4 - exponent) as usize;
        trim(&format!("{value:.decimals$}")).to_owned()
    } else {
        format!(
            "{}e{}{:02}",
            trim(mantissa),
            if exponent < 0 { '-' } else { '+' },
            exponent.unsigned_abs()
        )
    }
}

#[cfg(test)]
mod hover_number_tests {
    #[test]
    fn hover_coordinates_use_five_significant_digits() {
        for (value, text) in [
            (5.524025, "5.524"),
            (77.03048, "77.03"),
            (123456.7, "1.2346e+05"),
            (99999.0, "99999"),
            (99999.5, "1e+05"),
            (0.00012345, "0.00012345"),
            (0.000012345, "1.2345e-05"),
            (-2.5, "-2.5"),
            (0., "0"),
            (16., "16"),
            (1e300, "1e+300"),
            (f64::NAN, "NaN"),
            (f64::NEG_INFINITY, "-Inf"),
        ] {
            assert_eq!(super::hover_number(value), text, "{value}");
        }
    }
}

#[cfg(test)]
mod view3d_legend_tests {
    use super::*;

    fn samples(axes: &Axes) -> Vec<[u64; 2]> {
        axes.series[0]
            .points
            .iter()
            .map(|[x, y]| [x.to_bits(), y.to_bits()])
            .collect()
    }

    fn context(bounds: Option<([f64; 2], [f64; 2])>) -> SceneContext<'static> {
        SceneContext {
            panel: PanelContext::default(),
            ordinary_title: false,
            bounds,
            legend: None,
            point_indices: None,
        }
    }

    /// A docked 3-D axes has no prepared legend source: its `best` score must
    /// read the projected points, not the raw (x, y) samples of the model.
    #[test]
    fn best_legend_of_a_docked_3d_axes_scores_the_projected_points() {
        let mut engine = matrust::Engine::default();
        engine
            .execute("plot3([0 1 2],[0 1 0],[0 1 2]); legend('trace','Location','best');")
            .unwrap();
        let figure = engine.figure.unwrap();
        let axes = &figure.axes;
        assert!(axes.is_3d());
        assert_eq!(axes.legend_location, matrust::plot::LegendLocation::Best);
        let raw = samples(axes);
        let projected = matrust::plot::view3d::display_view_in(axes, Default::default());
        let expected = samples(&projected);
        assert_ne!(raw, expected);
        // No prepared legend and no prepared bounds: the copy is projected.
        assert_eq!(samples(&painted_axes(axes, &context(None))), expected);
        // Prepared geometry supplies the points (and the legend source): the
        // copy keeps the raw samples and is not projected again per frame.
        let kept = painted_axes(axes, &context(Some(([0., 1.], [0., 1.]))));
        assert_eq!(samples(&kept), raw);
        assert_eq!(kept.series.len(), projected.series.len());
    }
}

#[cfg(test)]
mod slice_viewer_desktop_tests {
    use super::*;

    #[test]
    fn slice_viewer_slider_step_changes_slice_number_and_drawn_texture() {
        let mut engine = matrust::Engine::default();
        engine
            .execute("V = reshape(1:27, 3, 3, 3); s = sliceViewer(V);")
            .unwrap();
        let mut figure = engine.figure.clone().unwrap();
        assert_eq!(figure.slice_viewers[0].slice_number, 2);

        let ctx = egui::Context::default();
        let tex_id = egui::Id::new("workspace_figure").with(("matrust_image", 0usize));

        let transforms = figure.axes.axis_transforms();
        let raw0 = egui::RawInput {
            screen_rect: Some(egui::Rect::from_min_size(
                egui::Pos2::ZERO,
                egui::vec2(400.0, 300.0),
            )),
            ..Default::default()
        };
        let _ = ctx.run_ui(raw0, |ui| {
            show_interactive(ui, &figure, &[], transforms, true, None);
        });
        let (key_before, _) = ctx
            .data(|d| d.get_temp::<(u64, egui::TextureHandle)>(tex_id))
            .expect("sliceViewer image texture should be cached");

        // Hover over the plot area and press Right arrow to step SliceNumber from 2 to 3.
        let raw_step = egui::RawInput {
            screen_rect: Some(egui::Rect::from_min_size(
                egui::Pos2::ZERO,
                egui::vec2(400.0, 300.0),
            )),
            events: vec![
                egui::Event::PointerMoved(egui::pos2(200.0, 150.0)),
                egui::Event::Key {
                    key: egui::Key::ArrowRight,
                    physical_key: None,
                    pressed: true,
                    repeat: false,
                    modifiers: egui::Modifiers::NONE,
                },
            ],
            ..Default::default()
        };
        let mut stepped = None;
        let _ = ctx.run_ui(raw_step, |ui| {
            let view = show_interactive(ui, &figure, &[], transforms, false, None);
            stepped = view.slice_number;
        });
        let new_slice = stepped.expect("arrow step should update slice_number");
        assert_eq!(new_slice, 3);

        figure.slice_viewers[0].slice_number = new_slice;
        let sv = figure.slice_viewers[0].clone();
        sv.sync_axes(&mut figure.axes);

        let raw_after = egui::RawInput {
            screen_rect: Some(egui::Rect::from_min_size(
                egui::Pos2::ZERO,
                egui::vec2(400.0, 300.0),
            )),
            ..Default::default()
        };
        let _ = ctx.run_ui(raw_after, |ui| {
            show_interactive(ui, &figure, &[], transforms, false, None);
        });
        let (key_after, _) = ctx
            .data(|d| d.get_temp::<(u64, egui::TextureHandle)>(tex_id))
            .expect("updated sliceViewer image texture should be cached");
        assert_ne!(
            key_before, key_after,
            "drawn texture must update after slice step"
        );
    }

    fn finish_workspace(app: &mut crate::Workspace) {
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
        while app.busy && std::time::Instant::now() < deadline {
            app.receive();
            std::thread::sleep(std::time::Duration::from_millis(1));
        }
        assert!(!app.busy, "{}", app.status);
    }

    /// Whole color uploads from `FullOutput.textures_delta`, kept across settle.
    /// A later free must not erase the pixels a paint mesh already bound.
    struct SliceUpload {
        size: [usize; 2],
        rgba: Vec<[u8; 4]>,
    }

    type SliceUploads = std::collections::HashMap<egui::TextureId, SliceUpload>;

    fn root_frame(
        app: &mut crate::Workspace,
        ctx: &egui::Context,
        events: Vec<egui::Event>,
        kept: &mut SliceUploads,
    ) -> egui::FullOutput {
        paint_frame(
            ctx,
            egui::RawInput {
                screen_rect: Some(egui::Rect::from_min_size(
                    egui::Pos2::ZERO,
                    egui::vec2(1440., 900.),
                )),
                events,
                ..Default::default()
            },
            kept,
            |ui| app.ui(ui),
        )
    }

    fn paint_frame(
        ctx: &egui::Context,
        mut input: egui::RawInput,
        kept: &mut SliceUploads,
        draw: impl FnMut(&mut egui::Ui),
    ) -> egui::FullOutput {
        crate::input::sanitize(&mut input);
        let mut output = ctx.run_ui(input, draw);
        // Settle used to drop this output. Retain the uploads first; headless
        // tests have no GPU painter, so the delta is the only pixel source.
        for (id, updates) in std::mem::take(&mut output.textures_delta.set) {
            for update in updates {
                if !update.is_whole() {
                    continue;
                }
                let egui::ImageData::Color(image) = update.image;
                kept.insert(
                    id,
                    SliceUpload {
                        size: image.size,
                        rgba: image.pixels.iter().map(|pixel| pixel.to_array()).collect(),
                    },
                );
            }
        }
        output.textures_delta.clear();
        output
    }

    fn detached_viewport(output: &egui::FullOutput) -> egui::ViewportId {
        output
            .viewport_output
            .keys()
            .copied()
            .find(|&id| id != egui::ViewportId::ROOT)
            .expect("detached figure viewport should exist")
    }

    fn child_frame(
        app: &mut crate::Workspace,
        ctx: &egui::Context,
        focused: bool,
        events: Vec<egui::Event>,
        kept: &mut SliceUploads,
    ) -> (egui::ViewportId, egui::FullOutput) {
        let output = root_frame(app, ctx, vec![], kept);
        let viewport = detached_viewport(&output);
        let callback = output.viewport_output[&viewport]
            .viewport_ui_cb
            .as_ref()
            .unwrap()
            .clone();
        let mut input = egui::RawInput {
            viewport_id: viewport,
            screen_rect: Some(egui::Rect::from_min_size(
                egui::Pos2::ZERO,
                egui::vec2(900., 650.),
            )),
            events,
            ..Default::default()
        };
        let info = input.viewports.entry(viewport).or_default();
        info.parent = Some(egui::ViewportId::ROOT);
        info.focused = Some(focused);
        (
            viewport,
            paint_frame(ctx, input, kept, move |ui| callback(ui)),
        )
    }

    fn settle_workspace(app: &mut crate::Workspace, ctx: &egui::Context, kept: &mut SliceUploads) {
        for _ in 0..8 {
            finish_workspace(app);
            root_frame(app, ctx, vec![], kept);
        }
        finish_workspace(app);
    }

    fn settle_detached(
        app: &mut crate::Workspace,
        ctx: &egui::Context,
        kept: &mut SliceUploads,
    ) -> egui::ViewportId {
        let mut viewport = egui::ViewportId::ROOT;
        for _ in 0..6 {
            settle_workspace(app, ctx, kept);
            viewport = child_frame(app, ctx, true, vec![], kept).0;
        }
        settle_workspace(app, ctx, kept);
        viewport
    }

    /// Texture bound by the detached viewport's tessellated paint mesh, matched
    /// to a 2×2 upload retained from `textures_delta` on this or an earlier frame.
    fn bound_slice_rgba(
        ctx: &egui::Context,
        output: &egui::FullOutput,
        kept: &SliceUploads,
    ) -> (egui::TextureId, Vec<[u8; 4]>) {
        let primitives = ctx.tessellate(output.shapes.clone(), output.pixels_per_point);
        let mut matched = Vec::new();
        let mut mesh_ids = Vec::new();
        for primitive in primitives {
            let egui::epaint::Primitive::Mesh(mesh) = &primitive.primitive else {
                continue;
            };
            if mesh.indices.is_empty()
                || mesh.texture_id == egui::TextureId::default()
                || mesh_ids.contains(&mesh.texture_id)
            {
                continue;
            }
            mesh_ids.push(mesh.texture_id);
            if let Some(image) = kept.get(&mesh.texture_id)
                && image.size == [2, 2]
            {
                matched.push((mesh.texture_id, image.rgba.clone()));
            }
        }
        assert_eq!(
            matched.len(),
            1,
            "detached viewport paint mesh should bind one kept 2x2 upload; meshes={mesh_ids:?} kept={:?}",
            kept.iter()
                .map(|(id, image)| (*id, image.size))
                .collect::<Vec<_>>()
        );
        matched.pop().unwrap()
    }

    fn arrow_right_at(pos: Option<egui::Pos2>) -> Vec<egui::Event> {
        let mut events = Vec::new();
        if let Some(p) = pos {
            events.push(egui::Event::PointerMoved(p));
        } else {
            events.push(egui::Event::PointerGone);
        }
        events.push(egui::Event::Key {
            key: egui::Key::ArrowRight,
            physical_key: None,
            pressed: true,
            repeat: false,
            modifiers: egui::Modifiers::NONE,
        });
        events
    }

    #[test]
    fn detached_and_docked_slice_viewer_step_routes_through_command_queue_only_to_hovered_target() {
        let ctx = egui::Context::default();
        ctx.set_embed_viewports(false);
        let dir = tempfile::tempdir().unwrap();
        let mut app = crate::Workspace::with_context(ctx.clone(), true);
        app.folder = dir.path().into();
        // Eight opaque rows. DisplayRange [5 13] maps integer v to row v-5
        // (floor(8 * (v - 5) / 8)), so the four texels of slice 2 and slice 3
        // are eight distinct colors. Expected RGBA is that table, not screen_raster.
        app.run(
            "cm = [0 0 0; 1 0 0; 0 1 0; 0 0 1; 1 1 0; 1 0 1; 0 1 1; 1 1 1]; \
             V = reshape(1:12, 2, 2, 3); figure(1); \
             s = sliceViewer(V, 'Colormap', cm, 'DisplayRange', [5 13]); \
             V2 = reshape(101:112, 2, 2, 3); figure(2); s2 = sliceViewer(V2); \
             figure(3); plot([1 2 3]); figure(2);"
                .into(),
        );
        finish_workspace(&mut app);
        assert_eq!(app.all_figures[&1].slice_viewers[0].slice_number, 2);
        assert_eq!(app.all_figures[&2].slice_viewers[0].slice_number, 2);
        let fig3_before = app.all_figures[&3].clone();
        let mut kept = SliceUploads::new();

        // Column-major V(:,:,2) = [5 7; 6 8], row 1 at the top of the texture.
        // Map rows 0..3 are black, red, green, blue, so the texels are
        // black, green / red, blue.
        let slice2 = [
            [0, 0, 0, 255],
            [0, 255, 0, 255],
            [255, 0, 0, 255],
            [0, 0, 255, 255],
        ];
        // V(:,:,3) = [9 11; 10 12] -> yellow, cyan / magenta, white.
        let slice3 = [
            [255, 255, 0, 255],
            [0, 255, 255, 255],
            [255, 0, 255, 255],
            [255, 255, 255, 255],
        ];
        assert_ne!(
            slice2, slice3,
            "slice-2 RGBA must fail a slice-3 pixel comparison"
        );
        for pixel in slice3 {
            assert_eq!(slice3.iter().filter(|other| **other == pixel).count(), 1);
            assert!(!slice2.contains(&pixel));
        }

        // Undock figure 1 while figure 2 remains the active docked figure.
        app.undock_figure(1, &ctx);
        let viewport1 = settle_detached(&mut app, &ctx, &mut kept);
        let plot_id1 = egui::Id::new(("detached_figure", viewport1));
        let center1 = egui_plot::PlotMemory::load(&ctx, plot_id1)
            .expect("detached plot memory should be cached")
            .transform()
            .frame()
            .center();

        // Input without an active hovered/focused slice viewer must not mutate any figure.
        child_frame(&mut app, &ctx, false, arrow_right_at(None), &mut kept);
        settle_detached(&mut app, &ctx, &mut kept);
        assert_eq!(app.all_figures[&1].slice_viewers[0].slice_number, 2);
        assert_eq!(app.all_figures[&2].slice_viewers[0].slice_number, 2);
        assert!(app.all_figures[&3].same_snapshot(&fig3_before));
        let (_, before_paint) = child_frame(&mut app, &ctx, true, vec![], &mut kept);
        let (tex_before, pixels_before) = bound_slice_rgba(&ctx, &before_paint, &kept);
        assert_eq!(
            pixels_before, slice2,
            "detached paint mesh before the key should show slice 2"
        );

        // Feed ArrowRight over detached figure 1; command queue applies index 3.
        child_frame(
            &mut app,
            &ctx,
            true,
            arrow_right_at(Some(center1)),
            &mut kept,
        );
        settle_detached(&mut app, &ctx, &mut kept);
        let (_, after_paint) = child_frame(&mut app, &ctx, true, vec![], &mut kept);

        assert_eq!(app.all_figures[&1].slice_viewers[0].slice_number, 3);
        assert_eq!(app.all_figures[&2].slice_viewers[0].slice_number, 2);
        assert!(app.all_figures[&3].same_snapshot(&fig3_before));

        app.run("r1 = s.SliceNumber; r2 = s2.SliceNumber;".into());
        finish_workspace(&mut app);
        assert_eq!(app.variables["r1"].as_numeric().unwrap().get(0, 0), 3.0);
        assert_eq!(app.variables["r2"].as_numeric().unwrap().get(0, 0), 2.0);

        let (tex_after, pixels_after) = bound_slice_rgba(&ctx, &after_paint, &kept);
        assert_ne!(tex_before, tex_after);
        assert_eq!(
            pixels_after, slice3,
            "stale slice-2 pixels {slice2:?} must not pass; got {pixels_after:?}"
        );
        assert_ne!(pixels_after, slice2);

        // Verify the same control remains effective when docked on figure 2.
        app.show_plot_pane();
        app.layout.mode = crate::layout::PaneMode::Figure;
        app.run("figure(2);".into());
        finish_workspace(&mut app);
        settle_workspace(&mut app, &ctx, &mut kept);

        let docked_plot_id = egui::Id::new("workspace_figure");
        let docked_center = egui_plot::PlotMemory::load(&ctx, docked_plot_id)
            .expect("docked plot memory should be cached")
            .transform()
            .frame()
            .center();
        root_frame(&mut app, &ctx, arrow_right_at(None), &mut kept);
        settle_workspace(&mut app, &ctx, &mut kept);
        assert_eq!(app.all_figures[&2].slice_viewers[0].slice_number, 2);

        root_frame(
            &mut app,
            &ctx,
            arrow_right_at(Some(docked_center)),
            &mut kept,
        );
        settle_workspace(&mut app, &ctx, &mut kept);
        assert_eq!(app.all_figures[&2].slice_viewers[0].slice_number, 3);
        app.run("r2_docked = s2.SliceNumber;".into());
        finish_workspace(&mut app);
        assert_eq!(
            app.variables["r2_docked"].as_numeric().unwrap().get(0, 0),
            3.0
        );
    }

    #[test]
    fn closed_detached_slice_viewer_pane_cannot_mutate_later_replacement() {
        let ctx = egui::Context::default();
        ctx.set_embed_viewports(false);
        let dir = tempfile::tempdir().unwrap();
        let mut app = crate::Workspace::with_context(ctx.clone(), true);
        app.folder = dir.path().into();
        app.run("V = reshape(1:12, 2, 2, 3); figure(1); s = sliceViewer(V);".into());
        finish_workspace(&mut app);
        app.undock_figure(1, &ctx);
        let mut kept = SliceUploads::new();
        let viewport1 = settle_detached(&mut app, &ctx, &mut kept);
        let plot_id1 = egui::Id::new(("detached_figure", viewport1));
        let center1 = egui_plot::PlotMemory::load(&ctx, plot_id1)
            .expect("detached plot memory should be cached")
            .transform()
            .frame()
            .center();

        // Queue a slice-step command in the detached child window, then close and replace figure 1
        // before the root frame drains the queued command.
        child_frame(
            &mut app,
            &ctx,
            true,
            arrow_right_at(Some(center1)),
            &mut kept,
        );
        app.run(
            "close(1); figure(1); V_new = reshape(1:12, 2, 2, 3); s_new = sliceViewer(V_new);"
                .into(),
        );
        finish_workspace(&mut app);
        settle_workspace(&mut app, &ctx, &mut kept);

        assert_eq!(app.all_figures[&1].slice_viewers[0].slice_number, 2);
        app.run("r_new = s_new.SliceNumber;".into());
        finish_workspace(&mut app);
        assert_eq!(app.variables["r_new"].as_numeric().unwrap().get(0, 0), 2.0);
    }
}

#[cfg(test)]
mod scatter_decimation_tests {
    use super::*;

    #[test]
    fn decimated_scatter_renders_original_point_colors_and_sizes() {
        let mut engine = matrust::Engine::default();
        engine
            .execute(
                "n = 10000;
                 x = 1:n;
                 y = sin(x / 100);
                 c = [repmat([1 0 0], n/2, 1); repmat([0 0 1], n/2, 1)];
                 s = [repmat(16, 1, n/2), repmat(64, 1, n/2)];
                 scatter(x, y, s, c, 'filled');",
            )
            .unwrap();
        let figure = engine.figure.unwrap();
        let axes = figure.axis_transforms();
        let never = std::sync::atomic::AtomicBool::new(false);
        let (raw_points, indices) = figure.series[0]
            .display_points_axes_indexed_cancellable(500, axes, &never)
            .unwrap();
        let points = vec![
            raw_points
                .into_iter()
                .map(|[x, y]| egui_plot::PlotPoint::new(x, y))
                .collect::<Vec<_>>(),
        ];
        let point_indices = vec![indices];
        assert!(points[0].len() < figure.series[0].points.len());

        let ctx = egui::Context::default();
        let output = ctx.run_ui(
            egui::RawInput {
                screen_rect: Some(egui::Rect::from_min_size(
                    egui::Pos2::ZERO,
                    egui::vec2(800., 600.),
                )),
                ..Default::default()
            },
            |ui| {
                show_interactive_indexed(
                    ui,
                    &figure,
                    &points,
                    Some(&point_indices),
                    axes,
                    true,
                    None,
                );
            },
        );
        let mut red_small = 0usize;
        let mut blue_large = 0usize;
        let small_r = (16.0_f64.sqrt() * 2. / 3.) as f32;
        let large_r = (64.0_f64.sqrt() * 2. / 3.) as f32;
        for clipped in &output.shapes {
            if let egui::Shape::Circle(c) = &clipped.shape {
                if c.fill == Color32::from_rgb(255, 0, 0) && (c.radius - small_r).abs() < 0.1 {
                    red_small += 1;
                } else if c.fill == Color32::from_rgb(0, 0, 255) && (c.radius - large_r).abs() < 0.1
                {
                    blue_large += 1;
                }
            }
        }
        assert!(
            red_small > 100,
            "expected red small circles, got {red_small}"
        );
        assert!(
            blue_large > 100,
            "expected blue large circles, got {blue_large}"
        );
    }

    #[test]
    fn axes_tick_dir_out_tick_length_and_line_width_change_frame_and_ticks() {
        let mut plain_engine = matrust::Engine::default();
        plain_engine.execute("plot(1:3);").unwrap();
        let plain = plain_engine.figure.unwrap();

        let mut styled_engine = matrust::Engine::default();
        styled_engine
            .execute("plot(1:3); set(gca,'TickDir','out','TickLength',[.03 .03],'LineWidth',3);")
            .unwrap();
        let styled = styled_engine.figure.unwrap();

        let render = |figure: &Figure| {
            let (axes, points) = crate::graphics_frames::display_points(figure);
            let ctx = egui::Context::default();
            crate::theme::configure(&ctx);
            let mut out = None;
            for pass in 0..2 {
                out = Some(crate::headless::run(
                    &ctx,
                    egui::RawInput {
                        screen_rect: Some(egui::Rect::from_min_size(
                            egui::Pos2::ZERO,
                            egui::vec2(640., 420.),
                        )),
                        ..Default::default()
                    },
                    |root| {
                        egui::CentralPanel::default().show(root, |ui| {
                            let _ = show_interactive(ui, figure, &points, axes, pass == 0, None);
                        });
                    },
                ));
            }
            let mut segments = Vec::new();
            fn walk(shape: &egui::Shape, out: &mut Vec<([egui::Pos2; 2], egui::Stroke)>) {
                match shape {
                    egui::Shape::LineSegment { points, stroke } => {
                        out.push((*points, *stroke));
                    }
                    egui::Shape::Vec(children) => {
                        for c in children {
                            walk(c, out);
                        }
                    }
                    _ => {}
                }
            }
            for clipped in &out.unwrap().shapes {
                walk(&clipped.shape, &mut segments);
            }
            segments
        };

        let plain_segments = render(&plain);
        let styled_segments = render(&styled);

        let expected_stroke = 4.0_f32; // LineWidth 3 pt * 4/3 = 4 px
        assert!(
            !plain_segments
                .iter()
                .any(|(_, s)| (s.width - expected_stroke).abs() < 0.05),
            "default axes should not paint 4px frame/tick strokes"
        );

        let thick_segments: Vec<_> = styled_segments
            .iter()
            .filter(|(_, s)| (s.width - expected_stroke).abs() < 0.05)
            .collect();
        // 4 frame edges + outward x and y ticks.
        assert!(
            thick_segments.len() >= 8,
            "expected thick frame edges and ticks, got {}",
            thick_segments.len()
        );
        let frame_left = thick_segments
            .iter()
            .filter(|([a, b], _)| (a.x - b.x).abs() < 0.05 && (a.y - b.y).abs() > 100.)
            .map(|([a, _], _)| a.x)
            .fold(f32::INFINITY, f32::min);
        let frame_right = thick_segments
            .iter()
            .filter(|([a, b], _)| (a.x - b.x).abs() < 0.05 && (a.y - b.y).abs() > 100.)
            .map(|([a, _], _)| a.x)
            .fold(f32::NEG_INFINITY, f32::max);
        let frame_top = thick_segments
            .iter()
            .filter(|([a, b], _)| (a.y - b.y).abs() < 0.05 && (a.x - b.x).abs() > 100.)
            .map(|([a, _], _)| a.y)
            .fold(f32::INFINITY, f32::min);
        let frame_bottom = thick_segments
            .iter()
            .filter(|([a, b], _)| (a.y - b.y).abs() < 0.05 && (a.x - b.x).abs() > 100.)
            .map(|([a, _], _)| a.y)
            .fold(f32::NEG_INFINITY, f32::max);
        let frame_w = frame_right - frame_left + expected_stroke;
        let frame_h = frame_bottom - frame_top + expected_stroke;
        let expected_tick_len = frame_w.max(frame_h) * 0.03;
        let outward_x = thick_segments.iter().filter(|([a, b], _)| {
            (a.x - b.x).abs() < 0.05
                && (a.y - frame_bottom).abs() < 0.5
                && b.y > frame_bottom
                && ((b.y - a.y).abs() - expected_tick_len).abs() < 0.5
        });
        let outward_y = thick_segments.iter().filter(|([a, b], _)| {
            (a.y - b.y).abs() < 0.05
                && (a.x - frame_left).abs() < 0.5
                && b.x < frame_left
                && ((b.x - a.x).abs() - expected_tick_len).abs() < 0.5
        });
        assert!(outward_x.count() >= 2, "expected outward x ticks");
        assert!(outward_y.count() >= 2, "expected outward y ticks");
    }
}
