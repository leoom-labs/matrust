use crate::theme::ThemedScrollArea;
#[path = "figure_axes_style.rs"]
mod axes_style;
#[path = "figure_legend.rs"]
mod legend;
use eframe::egui::{self, Color32};
use egui_plot::{
    Bar, BarChart, Line, Plot, PlotBounds, PlotGeometry, PlotItem, PlotItemBase, PlotPoint,
    PlotPoints, PlotTransform, Points,
};
use matrust::plot::{
    Axes, AxisScale, AxisTransform, BaseTheme, Figure, LinePattern, PlotKind, ResolvedAppearance,
    SampleQuery,
};

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
    pub axes: [AxisTransform; 2],
    pub bounds: ([f64; 2], [f64; 2]),
    pub legend: Option<&'a matrust::plot::LegendBestPrepared>,
}
struct PreparedLayout<'a> {
    bounds: ([f64; 2], [f64; 2]),
    legend: Option<&'a matrust::plot::LegendBestPrepared>,
    plot_id: Option<egui::Id>,
    fit: bool,
}
struct SceneContext<'a> {
    panel: PanelContext,
    ordinary_title: bool,
    bounds: Option<([f64; 2], [f64; 2])>,
    legend: Option<&'a matrust::plot::LegendBestPrepared>,
}
pub fn show_interactive(
    ui: &mut egui::Ui,
    figure: &Figure,
    points: &[Vec<PlotPoint>],
    axes: [AxisTransform; 2],
    reset: bool,
    marker: Option<[f64; 2]>,
) -> View {
    show_interactive_impl(ui, figure, points, axes, reset, marker, None)
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
        prepared.axes,
        reset,
        marker,
        Some(PreparedLayout {
            bounds: prepared.bounds,
            legend: prepared.legend,
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
            prepared.axes,
            reset,
            marker,
            Some(PreparedLayout {
                bounds: prepared.bounds,
                legend: prepared.legend,
                plot_id: Some(plot_id),
                fit,
            }),
        )
    })
    .inner
}
fn show_interactive_impl(
    ui: &mut egui::Ui,
    figure: &Figure,
    points: &[Vec<PlotPoint>],
    axes: [AxisTransform; 2],
    reset: bool,
    marker: Option<[f64; 2]>,
    prepared: Option<PreparedLayout<'_>>,
) -> View {
    let fit = prepared.as_ref().is_some_and(|p| p.fit);
    let explicit_id = prepared.as_ref().and_then(|p| p.plot_id);
    let (bounds, legend) = prepared.map_or((None, None), |p| (Some(p.bounds), p.legend));
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
        let view = if let Some(p) = figure.placement {
            let request = if p.imported_solved
                || p.constraint == matrust::plot::PositionConstraint::InnerPosition
            {
                p.requested_inner
            } else {
                p.requested_outer
            };
            let panel = egui::Rect::from_min_size(
                canvas.min
                    + egui::vec2(
                        request[0] as f32 * canvas.width(),
                        (1. - request[1] - request[3]) as f32 * canvas.height(),
                    ),
                egui::vec2(
                    request[2] as f32 * canvas.width(),
                    request[3] as f32 * canvas.height(),
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
                            canvas: Some(canvas),
                            appearance,
                        },
                        bounds,
                        legend,
                    },
                )
            })
            .inner
        } else {
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
                },
            )
        };
        // Annotations belong to the complete figure canvas, including ordinary
        // one-axes documents. Paint after axes without changing stored positions.
        paint_textboxes(ui, figure, canvas);
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

pub fn show_axes(
    ui: &mut egui::Ui,
    figure: &Axes,
    points: &[Vec<PlotPoint>],
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
        },
    )
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
    ui.scope(|ui| {
        apply_appearance(ui, context.panel.appearance);
        show_document_axes(ui, figure, points, axes, reset, plot_id, context)
    })
    .inner
}
fn axes_title_galley(
    ui: &egui::Ui,
    title: &str,
    width: f32,
    rows: usize,
    color: Color32,
) -> std::sync::Arc<egui::Galley> {
    let mut job = egui::text::LayoutJob::simple(
        title.to_owned(),
        super::theme::figure_body(ui.ctx()),
        color,
        (width - 16.).max(1.),
    );
    job.wrap.max_rows = rows;
    job.wrap.break_anywhere = true;
    ui.painter().layout_job(job)
}

/// Only the outer short-pane scrollbar may grow ordinary automatic artwork.
/// Fixed axes geometry and tile Fit rendering keep their existing finite boxes.
pub(super) fn reserve_readable_title_canvas(ui: &mut egui::Ui, figure: &Figure) {
    if figure.placement.is_some() || figure.title.is_empty() {
        return;
    }
    let font = super::theme::figure_body(ui.ctx());
    let line_height = ui.fonts_mut(|fonts| fonts.row_height(&font));
    let galley = axes_title_galley(
        ui,
        &figure.title,
        ui.available_width(),
        3,
        ui.visuals().text_color(),
    );
    // Use the same bounded rows and gutter as painting. One point covers
    // roundoff when the subsequent division determines the admitted row count.
    let minimum =
        (80. + 8. + line_height * 0.5 + line_height * galley.rows.len().min(3) as f32).ceil() + 1.;
    ui.set_min_height(minimum);
}

// An axes title is document artwork, not a window or toolbar caption. This
// bounded band uses the existing finite canvas and does not allocate plot IDs or
// alter the document, prepared samples, transform state or history.
fn paint_axes_title(
    ui: &mut egui::Ui,
    title: &str,
    panel: egui::Rect,
    appearance: ResolvedAppearance,
    plot_id: egui::Id,
    fixed_canvas: Option<egui::Rect>,
) -> Option<(egui::Rect, f32)> {
    if title.is_empty() {
        return Some((panel, 0.));
    }
    if !panel.is_finite() || !panel.is_positive() {
        return None;
    }
    let font = super::theme::figure_body(ui.ctx());
    let line_height = ui.fonts_mut(|fonts| fonts.row_height(&font));
    // Keep a half-line for a valid y tick centered on the top data edge.
    let padding = 8. + line_height * 0.5;
    let available_height =
        fixed_canvas.map_or(panel.height() - 80., |canvas| panel.top() - canvas.top());
    if available_height < line_height + padding || panel.width() <= 16. {
        return None;
    }
    let rows = (((available_height - padding) / line_height).floor() as usize).clamp(1, 3);
    let galley = axes_title_galley(ui, title, panel.width(), rows, rgb(appearance.text));
    let height = galley.size().y + padding;
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
    let position = egui::pos2(panel.center().x - galley.size().x * 0.5, band.top() + 4.);
    ui.painter().rect_filled(band, 0., rgb(appearance.canvas));
    ui.painter()
        .with_clip_rect(band.intersect(ui.clip_rect()))
        .galley(position, galley, rgb(appearance.text));
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
            &figure.title,
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
    if !figure.ylabel.is_empty() {
        // egui_plot positions a rotated y label a quarter line beyond its
        // widget edge. Reserve that gutter inside the document canvas so the
        // actual glyphs remain on paper and inside subplot clips.
        plot_rect.min.x += (ui.text_style_height(&egui::TextStyle::Body) * 0.25)
            .min(plot_rect.width().max(0.) * 0.25);
    }
    let hidden: Vec<_> = figure
        .series
        .iter()
        .enumerate()
        .filter(|(_, s)| !s.visible)
        .map(|(i, _)| egui::Id::new(("series", i)))
        .collect();
    let memory = egui_plot::PlotMemory::load(ui.ctx(), plot_id);
    // Prepared geometry can precede the first view. Initialize explicit limits
    // once for a new plot identity, while preserving later manual pan and zoom.
    let reset = reset || memory.is_none();
    if let Some(mut memory) = memory {
        memory.hidden_items = hidden.iter().copied().collect();
        if !figure.legend {
            memory.hovered_legend_item = None;
        }
        memory.store(ui.ctx(), plot_id);
    }
    let mut plot = Plot::new("figure")
        .id(plot_id)
        .x_axis_label(&figure.xlabel)
        .y_axis_label(&figure.ylabel)
        .show_grid([figure.xgrid, figure.ygrid])
        // Leave room for the final tick label at the edge of a docked panel.
        .width((plot_rect.width() - 32.).max(1.))
        // A fitting overview must not enlarge the widget behind the resolver.
        // The readable-size view provides larger axes.
        .height(plot_rect.height().max(1.));
    if appearance.theme == BaseTheme::Dark {
        plot = plot.grid_color(rgb(appearance.grid));
    }
    if intent.x.is_some() {
        plot = plot.custom_x_axes(vec![
            egui_plot::AxisHints::new_x()
                .label(&figure.xlabel)
                .tick_label_color(rgb(colors.x)),
        ]);
    }
    if intent.y.is_some() {
        plot = plot.custom_y_axes(vec![
            egui_plot::AxisHints::new_y()
                .label(&figure.ylabel)
                .tick_label_color(rgb(colors.y)),
        ]);
    }
    if intent.xlabel.is_some() || intent.x.is_some() {
        plot = plot.x_axis_label(egui::RichText::new(&figure.xlabel).color(rgb(colors.xlabel)));
    }
    if intent.ylabel.is_some() || intent.y.is_some() {
        plot = plot.y_axis_label(egui::RichText::new(&figure.ylabel).color(rgb(colors.ylabel)));
    }
    if intent.grid.is_some() || !appearance.legacy {
        // The library exposes one grid color; this explicit decoration supplies
        // both directional colors under the series using the same final transform.
        plot = plot.show_grid(false);
    }
    let decoration = axes_style::Decoration::new(
        figure,
        axes,
        colors,
        appearance,
        ui.visuals().widgets.noninteractive.bg_stroke,
    );
    for baseline in &figure.baselines {
        if baseline.style.visible
            && baseline.style.pattern != LinePattern::None
            && let Ok(value) = baseline.value.resolve(axes[1])
        {
            plot = plot.include_y(axes[1].forward(value));
        }
    }
    plot = plot
        .x_grid_spacer(move |input| axes_style::marks(axes[0], 0, input))
        .y_grid_spacer(move |input| axes_style::marks(axes[1], 1, input));
    if axes[0].scale == AxisScale::Log {
        plot = plot.x_axis_formatter(move |mark, _| axes[0].label(mark.value));
    } else {
        plot = plot.x_axis_formatter(|mark, _| linear_tick(mark));
    }
    if axes[1].scale == AxisScale::Log {
        plot = plot.y_axis_formatter(move |mark, _| axes[1].label(mark.value));
    }
    plot = plot.label_formatter(move |position| {
        let (name, point) = match position {
            egui_plot::HoverPosition::NearDataPoint {
                plot_name,
                position,
                ..
            } => (*plot_name, *position),
            egui_plot::HoverPosition::Elsewhere { position } => ("", *position),
        };
        Some(format!(
            "{name}\nx = {}\ny = {}",
            hover_number(axes[0].inverse(point.x)),
            hover_number(axes[1].inverse(point.y))
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
            if intent.x.is_some() || intent.y.is_some() {
                host.visuals_mut().widgets.noninteractive.bg_stroke = egui::Stroke::NONE;
            }
            plot.show(host, |ui| {
                if let Some(decoration) = decoration {
                    ui.add(decoration);
                }
                if reset {
                    let (x, y) = prepared_bounds.unwrap_or_else(|| figure.bounds());
                    let x = figure.xlim.unwrap_or(x);
                    let y = figure.ylim.unwrap_or(y);
                    if figure.xlim.is_some() {
                        ui.set_plot_bounds_x(axes[0].forward(x[0])..=axes[0].forward(x[1]));
                    }
                    if figure.ylim.is_some() {
                        ui.set_plot_bounds_y(axes[1].forward(y[0])..=axes[1].forward(y[1]));
                    }
                }
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
                        PlotKind::Patch => ui.add(StyledPatch {
                            line: Line::new(&s.label, PlotPoints::from(points.as_slice()))
                                .id(id)
                                .color(color),
                            width: s.patch.map_or(0, |p| p.vertices_per_face()),
                            edge: s
                                .face_edge_color
                                .map(|c| Color32::from_rgb(c[0], c[1], c[2])),
                            stroke_width: s.face_edge_width(),
                            pattern: s.pattern,
                            dash_budget: 200000. / figure.series.len().max(1) as f64,
                            bar: None,
                            fill: true,
                        }),
                        PlotKind::Bar => {
                            ui.bar_chart(
                                BarChart::new(
                                    &s.label,
                                    points
                                        .iter()
                                        .filter(|p| axes[0].accepts(p.x) && axes[1].accepts(p.y))
                                        .map(|p| {
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
                                            let baseline = axes[1].forward(series_base);
                                            Bar::new((l + r) * 0.5, axes[1].forward(p.y) - baseline)
                                                .base_offset(baseline)
                                                .width((r - l).max(0.))
                                                .fill(color)
                                                .stroke(egui::Stroke::NONE)
                                        })
                                        .collect(),
                                )
                                .id(id)
                                .color(color),
                            );
                            if let Some(edge) = s.face_edge_color {
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
                                });
                            }
                        }
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
    if outside.is_some() {
        ui.advance_cursor_after_rect(panel);
    }
    let bounds = response.transform.bounds();
    let min = bounds.min();
    let max = bounds.max();
    let frame = *response.transform.frame();
    if response.response.contains_pointer() {
        let pass = ui.ctx().cumulative_pass_nr();
        let tooltip = egui::Tooltip::tooltip_id(response.response.id, 0);
        let coordinates = egui::containers::AreaState::load(ui.ctx(), tooltip).map(|s| s.rect());
        let hover_id = hovered_plot_id(ui.ctx());
        ui.ctx()
            .data_mut(|d| d.insert_temp(hover_id, (pass, HoveredPlot { frame, coordinates })));
    }
    paint_baselines(ui, figure, axes[1], &response.transform);
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
        memory.hovered_legend_item = legend.hovered;
        memory.clone().store(ui.ctx(), plot_id);
    }
    let visibility = legend.visibility;
    View {
        bounds: model_bounds(figure, axes, min, max),
        auto: memory.map_or([true; 2], |m| [m.auto_bounds.x, m.auto_bounds.y]),
        visibility,
        pick,
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
        auto: memory.map_or([figure.xlim.is_none(), figure.ylim.is_none()], |m| {
            [m.auto_bounds.x, m.auto_bounds.y]
        }),
        visibility: None,
        pick: None,
    }
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
        for face in points.chunks_exact(self.width) {
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
            let clipped = matrust::plot::clip_patch_face(&vertices[..width], bounds);
            if self.fill && clipped.points().len() >= 3 {
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
            if let Some(edge) = self.edge
                && self.pattern != LinePattern::None
            {
                let stroke = egui::Stroke::new(self.stroke_width, edge);
                let mut path = Vec::with_capacity(8);
                for i in 0..width {
                    if !edges[i] {
                        face_edge_path(&mut path, stroke, self.pattern, &mut budget, shapes);
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
                            face_edge_path(&mut path, stroke, self.pattern, &mut budget, shapes);
                        }
                        if path.is_empty() {
                            path.push(a);
                        }
                        path.push(b);
                    } else {
                        face_edge_path(&mut path, stroke, self.pattern, &mut budget, shapes);
                    }
                }
                face_edge_path(&mut path, stroke, self.pattern, &mut budget, shapes);
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
        d.remove::<matrust::plot::LegendBestCache>(id.with("legend_best"));
        d.remove::<Option<egui::Rect>>(id.with("legend_rect"));
    });
    legend::forget(ctx, id);
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
mod title_artwork_tests;

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
