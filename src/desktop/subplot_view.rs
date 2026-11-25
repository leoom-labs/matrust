use crate::theme::ThemedScrollArea;
#[cfg(test)]
thread_local! {static UI_EDIT_VALIDATIONS: std::cell::Cell<usize> = const {std::cell::Cell::new(0)};}
#[cfg(test)]
pub(super) fn take_ui_edit_validations() -> usize {
    UI_EDIT_VALIDATIONS.with(|count| count.replace(0))
}
use super::figure_view;
use super::graphics_overlay;
use eframe::egui;
use egui_plot::PlotPoint;
use matrust::plot::{Axes, AxisTransform, Figure, Position, SampleQuery};
use std::collections::BTreeMap;

struct CachedAxes {
    source: Axes,
    points: Vec<Vec<PlotPoint>>,
    point_indices: Vec<Option<Vec<usize>>>,
    transforms: [AxisTransform; 2],
    bounds: Option<[f64; 4]>,
    budget: usize,
    prepared_bounds: Option<([f64; 2], [f64; 2])>,
    legend: Option<matrust::plot::LegendBestPrepared>,
}
#[derive(Default)]
pub struct Canvas {
    figure: u32,
    fit_overview: Option<bool>,
    cache: BTreeMap<Position, CachedAxes>,
    pub undoable_change: bool,
    pub pick: Option<(Position, SampleQuery)>,
}
enum RenderPath {
    Ordinary,
    Prepared,
    Explicit(egui::Id),
    ExplicitFitted(egui::Id),
}

fn paint_layout_text(
    ui: &egui::Ui,
    layout: &matrust::plot::Layout,
    mut area: egui::Rect,
    color: egui::Color32,
) -> egui::Rect {
    let font = super::theme::figure_body(ui.ctx());
    let painter = ui.painter();
    let label = |source: &str, index: usize, style: matrust::plot::LabelStyle| {
        (!source.is_empty()).then(|| {
            let mut label_font = font.clone();
            label_font.size = style.font_size_pixels(font.size);
            graphics_overlay::layout_markup_styled(
                painter,
                source,
                layout.text_interpreters[index],
                label_font,
                color,
                graphics_overlay::TextEmphasis::from(style),
            )
        })
    };
    let xlabel = label(&layout.xlabel, 1, layout.label_styles[0]);
    let ylabel = label(&layout.ylabel, 2, layout.label_styles[1]);
    let label_space = |label: &graphics_overlay::MarkupLayout| {
        if label.rules.is_empty() {
            24.
        } else {
            24_f32.max(label.size.y + 4.)
        }
    };
    let bottom = xlabel.as_ref().map_or(0., label_space);
    let left = ylabel.as_ref().map_or(0., label_space);
    area.max.y -= bottom;
    area.min.x += left;
    if !layout.title.is_empty() {
        let mut font = font.clone();
        font.size = layout.title_style.font_size_pixels(font.size);
        let emphasis = graphics_overlay::TextEmphasis::from(layout.title_style);
        let bold = emphasis.bold;
        let nodes =
            matrust::plot::text_markup::parse_nodes(&layout.title, layout.text_interpreters[0]);
        let pos = egui::pos2(area.center().x, area.top());
        let height = if let Some(title) = graphics_overlay::layout_structured_markup_styled(
            painter, &nodes, &font, color, emphasis,
        ) {
            let height = title.size.y;
            let top_left = pos - egui::vec2(title.size.x / 2., 0.);
            title
                .with_bold(bold)
                .paint(&painter.with_clip_rect(area), top_left, color, 0.);
            height
        } else {
            let mut job =
                graphics_overlay::markup_job_from_nodes_styled(&nodes, &font, color, emphasis);
            job.halign = egui::Align::Center;
            job.wrap.max_rows = 21;
            let galley = painter.layout_job(job);
            let height = galley.size().y;
            if bold {
                graphics_overlay::paint_bold_copy(painter, pos, &galley, color);
            }
            painter.galley(pos, galley, color);
            height
        };
        area.min.y += height + ui.spacing().item_spacing.y;
    }
    if let Some((subtitle, color)) =
        graphics_overlay::layout_subtitle(painter, &layout.subtitle, font, color)
    {
        let pos = egui::pos2(area.center().x - subtitle.size.x / 2., area.top());
        subtitle.paint(&painter.with_clip_rect(area), pos, color, 0.);
        area.min.y += subtitle.size.y + ui.spacing().item_spacing.y;
    }
    if let Some(label) = xlabel {
        let pos = egui::pos2(area.center().x, area.bottom() + bottom / 2.) - label.size / 2.;
        label.paint(painter, pos, color, 0.);
    }
    if let Some(label) = ylabel {
        let center = egui::pos2(area.left() - left / 2., area.center().y);
        let pos = center + egui::vec2(-label.size.y / 2., label.size.x / 2.);
        label.paint(painter, pos, color, -std::f32::consts::FRAC_PI_2);
    }
    area
}

fn paint_tile_title(ui: &mut egui::Ui, axes: &Axes, appearance: matrust::plot::ResolvedAppearance) {
    use matrust::plot::text_markup::{self, Node};
    let title = &axes.title;
    let nodes = text_markup::parse_nodes(title, axes.extras.label_interpreters[0]);
    let explicit_color = axes.colors.and_then(|colors| colors.title).map(|_| {
        let [r, g, b] = axes.resolve_colors(appearance).title;
        egui::Color32::from_rgb(r, g, b)
    });
    let style = axes.extras.title_style;
    let emphasis = graphics_overlay::TextEmphasis::from(style);
    let mut font = egui::TextStyle::Body.resolve(ui.style());
    font.size = style.font_size_pixels(font.size);
    let color = explicit_color.unwrap_or_else(|| ui.visuals().text_color());
    if let Some(layout) = graphics_overlay::layout_structured_markup_styled(
        ui.painter(),
        &nodes,
        &font,
        color,
        emphasis,
    ) {
        let size = egui::vec2(
            layout.size.x.min(ui.available_width().max(0.)),
            layout.size.y,
        );
        let (rect, response) = ui.allocate_exact_size(size, egui::Sense::hover());
        layout.paint(&ui.painter().with_clip_rect(rect), rect.min, color, 0.);
        response.on_hover_text(title);
    } else {
        let literal = style.is_default()
            && (title.is_empty()
                || matches!(nodes.as_slice(), [Node::Run(run)]
                if run.text == *title && run.scale == 1. && run.shift == 0. && !run.bold && !run.italic));
        let text: egui::WidgetText = if literal {
            let mut text = egui::RichText::new(title);
            if let Some(color) = explicit_color {
                text = text.color(color);
            }
            text.into()
        } else {
            graphics_overlay::markup_job_from_nodes_styled(
                &nodes,
                &font,
                explicit_color.unwrap_or(egui::Color32::PLACEHOLDER),
                emphasis,
            )
            .into()
        };
        let label = egui::Label::new(text).truncate();
        if emphasis.bold {
            let (pos, galley, response) = label.layout_in_ui(ui);
            if ui.is_rect_visible(response.rect) {
                graphics_overlay::paint_bold_copy(ui.painter(), pos, &galley, color);
                ui.painter().galley(pos, galley, color);
            }
            response.on_hover_text(title);
        } else {
            ui.add(label).on_hover_text(title);
        }
    }
}

impl Canvas {
    /// Release display allocations without changing the user's overview choice.
    pub(super) fn release_geometry(&mut self) {
        self.cache.clear();
        self.pick = None;
    }
    /// Retain unaffected axes while releasing obsolete buffers before rebuilding.
    pub(super) fn retain_source(&mut self, id: u32, figure: &Figure) {
        let budget = 8192 / figure.series_count().max(1);
        let same_figure = self.figure == id;
        self.cache.retain(|position, cached| {
            same_figure
                && cached.budget == budget
                && figure.all_axes().any(|(current, source)| {
                    current == *position && cached.source.same_snapshot(source)
                })
        });
        self.pick = None;
    }
    pub(super) fn matches_source(&self, id: u32, figure: &Figure) -> bool {
        let budget = 8192 / figure.series_count().max(1);
        self.figure == id
            && self.cache.len() == figure.all_axes().count()
            && figure.all_axes().all(|(position, source)| {
                self.cache.get(&position).is_some_and(|cached| {
                    cached.budget == budget && cached.source.same_snapshot(source)
                })
            })
    }
    pub fn matches_prepared(&self, id: u32, figure: &Figure) -> bool {
        let budget = 8192 / figure.series_count().max(1);
        self.figure == id
            && self.cache.len() == figure.all_axes().count()
            && figure.all_axes().all(|(position, source)| {
                self.cache.get(&position).is_some_and(|cached| {
                    cached.prepared_bounds.is_some()
                        && cached.budget == budget
                        && cached.source.same_snapshot(source)
                        && cached
                            .legend
                            .as_ref()
                            .is_none_or(|token| token.matches(source))
                })
            })
    }
    /// A detached callback must never enter the ordinary synchronous miss path.
    pub fn show_prepared(
        &mut self,
        ui: &mut egui::Ui,
        id: u32,
        figure: &Figure,
        reset: bool,
        marker: Option<(Position, [f64; 2])>,
    ) -> matrust::Result<Option<Figure>> {
        if !self.matches_prepared(id, figure) {
            return Err("Figure changed; preparing its display".into());
        }
        Ok(self.show_impl(ui, id, figure, reset, marker, RenderPath::Prepared))
    }
    /// Prepared ROOT views need explicit plot and ancillary widget identities.
    pub fn show_prepared_with_id(
        &mut self,
        ui: &mut egui::Ui,
        plot_id: egui::Id,
        id: u32,
        figure: &Figure,
        reset: bool,
        marker: Option<(Position, [f64; 2])>,
    ) -> matrust::Result<Option<Figure>> {
        if !self.matches_prepared(id, figure) {
            return Err("Figure changed; preparing its display".into());
        }
        Ok(ui
            .scope_builder(
                egui::UiBuilder::new().id(plot_id.with("prepared_scene")),
                |ui| self.show_impl(ui, id, figure, reset, marker, RenderPath::Explicit(plot_id)),
            )
            .inner)
    }
    /// Fit an independent tile without changing the retained per-figure sizing preference.
    pub fn show_prepared_fitted_with_id(
        &mut self,
        ui: &mut egui::Ui,
        plot_id: egui::Id,
        id: u32,
        figure: &Figure,
        reset: bool,
        marker: Option<(Position, [f64; 2])>,
    ) -> matrust::Result<Option<Figure>> {
        if !self.matches_prepared(id, figure) {
            return Err("Figure changed; preparing its display".into());
        }
        Ok(ui
            .scope_builder(
                egui::UiBuilder::new()
                    .id(plot_id.with("prepared_scene"))
                    .max_rect(ui.available_rect_before_wrap().intersect(ui.clip_rect())),
                |ui| {
                    self.show_impl(
                        ui,
                        id,
                        figure,
                        reset,
                        marker,
                        RenderPath::ExplicitFitted(plot_id),
                    )
                },
            )
            .inner)
    }
    /// Build the same cache off the UI thread; samples stay immutable/shared.
    pub fn prepare(
        id: u32,
        figure: &Figure,
        cancel: &std::sync::atomic::AtomicBool,
    ) -> matrust::Result<Self> {
        let mut result = Self {
            figure: id,
            ..Default::default()
        };
        let budget = 8192 / figure.series_count().max(1);
        for (position, source) in figure.all_axes() {
            // yyaxis: right series in left-axis coordinates; the cache keeps the source.
            let display = source.yy_display();
            let shown = display.as_ref().unwrap_or(source);
            // 3-D: projected samples plus box/grid series; 2-D: borrowed as is.
            let projected = matrust::plot::view3d::display_view(shown);
            let shown = projected.as_ref();
            let (transforms, bounds) = shown.display_axes_cancellable(cancel)?;
            let mut points = Vec::with_capacity(shown.series.len());
            let mut point_indices = Vec::with_capacity(shown.series.len());
            for index in 0..shown.series.len() {
                let (pts, indices) = shown
                    .display_series_points_indexed_cancellable(index, budget, transforms, cancel)?;
                points.push(super::figure_geometry::convert(pts, cancel)?);
                point_indices.push(indices);
            }
            result.cache.insert(
                position,
                CachedAxes {
                    source: source.clone(),
                    points,
                    point_indices,
                    transforms,
                    bounds: None,
                    budget,
                    prepared_bounds: Some(bounds),
                    legend: super::figure_geometry::prepare_legend(shown, cancel)?,
                },
            );
        }
        if cancel.load(std::sync::atomic::Ordering::Relaxed) {
            return Err("Figure preparation cancelled".into());
        }
        Ok(result)
    }
    pub fn retained_point_bytes(&self) -> usize {
        self.cache
            .values()
            .flat_map(|c| {
                c.points
                    .iter()
                    .map(|p| p.capacity() * std::mem::size_of::<PlotPoint>())
                    .chain(
                        c.point_indices
                            .iter()
                            .flatten()
                            .map(|v| v.capacity() * std::mem::size_of::<usize>()),
                    )
            })
            .sum()
    }
    #[cfg(test)]
    pub fn retained_points(&self) -> usize {
        self.cache
            .values()
            .flat_map(|c| &c.points)
            .map(Vec::len)
            .sum()
    }
    pub fn active_bounds(&self, figure: &Figure) -> Option<[f64; 4]> {
        self.cache
            .get(&figure.active_position())
            .and_then(|c| c.bounds)
    }
    pub fn show(
        &mut self,
        ui: &mut egui::Ui,
        id: u32,
        figure: &Figure,
        reset: bool,
        marker: Option<(Position, [f64; 2])>,
    ) -> Option<Figure> {
        self.show_impl(ui, id, figure, reset, marker, RenderPath::Ordinary)
    }
    fn show_impl(
        &mut self,
        ui: &mut egui::Ui,
        id: u32,
        figure: &Figure,
        reset: bool,
        marker: Option<(Position, [f64; 2])>,
        path: RenderPath,
    ) -> Option<Figure> {
        let (prepared, explicit_id, forced_fit) = match path {
            RenderPath::Ordinary => (false, None, false),
            RenderPath::Prepared => (true, None, false),
            RenderPath::Explicit(id) => (true, Some(id), false),
            RenderPath::ExplicitFitted(id) => (true, Some(id), true),
        };
        let appearance = figure.resolve_appearance(figure_view::host_theme(ui.ctx()));
        self.undoable_change = false;
        self.pick = None;
        if self.figure != id {
            self.cache.clear();
            self.figure = id;
            self.fit_overview = None;
        }
        let mut axes: Vec<_> = figure.all_axes().collect();
        axes.sort_by_key(|(p, _)| *p);
        self.cache.retain(|p, _| axes.iter().any(|(q, _)| p == q));
        if figure.empty_layout() {
            let canvas = ui.available_rect_before_wrap();
            figure_view::paint_figure_background(ui, figure, canvas);
            if let Some(layout) = &figure.layout {
                let [r, g, b] = appearance.shared_title;
                let area = paint_layout_text(ui, layout, canvas, egui::Color32::from_rgb(r, g, b));
                if area.top() > canvas.top() {
                    ui.add_space(area.top() - canvas.top());
                }
            }
            ui.weak("Empty layout. Choose Next tile, then plot your data.");
            figure_view::paint_textboxes(ui, figure, ui.available_rect_before_wrap());
            figure_view::paint_annotations(ui, figure, ui.available_rect_before_wrap());
            return None;
        }
        let budget = 8192 / figure.series_count().max(1);
        let mut updated = figure.clone();
        let mut changed = false;
        let mut selected = None;
        let mut synced_axes: Vec<(matrust::ObjectIdentity, bool, bool)> = Vec::new();
        let minimum = axes.iter().fold([0_f32; 2], |[w, h], (p, a)| {
            let [l, t, r, b] = a
                .placement
                .filter(|s| {
                    !s.imported_solved
                        && s.constraint == matrust::plot::PositionConstraint::OuterPosition
                })
                .map_or_else(
                    || p.rectangle(),
                    |s| {
                        let [l, b, w, h] = s.requested_outer;
                        [l, 1. - b - h, l + w, 1. - b]
                    },
                );
            // Custom positions describe the data box; grid positions include
            // the surrounding selector, ticks and labels.
            let [width, height] = if a.legend && a.legend_location.is_outside() {
                [520., 320.]
            } else if p.custom.is_some() {
                [200., 120.]
            } else {
                [280., 190.]
            };
            [
                w.max(width / (r - l) as f32),
                h.max(height / (b - t) as f32),
            ]
        });
        let outside = axes
            .iter()
            .any(|(_, a)| a.legend && a.legend_location.is_outside());
        // Default to the overview when every tile keeps at least 60 % of its
        // readable size; a scrolled canvas hides whole axes on first sight.
        let room = ui.available_size();
        let roomy = room.x >= minimum[0] * 0.6 && room.y >= minimum[1] * 0.6;
        let mut fit = forced_fit || self.fit_overview.unwrap_or(outside || roomy);
        if !forced_fit {
            ui.horizontal(|ui| {
                if crate::tokens::toggle(ui, fit, "Fit all axes")
                    .on_hover_text("Overview within this pane; use Readable size for larger plots.")
                    .clicked()
                {
                    fit = true;
                    self.fit_overview = Some(true);
                }
                if crate::tokens::toggle(ui, !fit, "Readable size")
                    .on_hover_text("Keep larger axes and scroll to reach the rest of the figure.")
                    .clicked()
                {
                    fit = false;
                    self.fit_overview = Some(false);
                }
            });
        }
        let available = ui.available_size().max(egui::vec2(1., 1.));
        let size = if fit {
            available
        } else {
            egui::vec2(available.x.max(minimum[0]), available.y.max(minimum[1]))
        };
        egui::ScrollArea::both()
            .auto_shrink([false, false])
            .id_salt(("subplot_canvas", fit))
            .show_themed(ui, |ui| {
                let (_, area) = ui.allocate_space(size);
                figure_view::paint_figure_background(ui, figure, area);
                let area = figure.layout.as_ref().map_or(area, |layout| {
                    let [r, g, b] = appearance.shared_title;
                    paint_layout_text(ui, layout, area, egui::Color32::from_rgb(r, g, b))
                });
                let size = area.size().max(egui::vec2(1., 1.));
                for (index, (position, source)) in axes.into_iter().enumerate() {
                    let rebuild = self
                        .cache
                        .get(&position)
                        .is_none_or(|c| c.budget != budget || !c.source.same_snapshot(source));
                    if rebuild {
                        let display = source.yy_display();
                        let shown = display.as_ref().unwrap_or(source);
                        let projected = matrust::plot::view3d::display_view(shown);
                        let shown = projected.as_ref();
                        let transforms = shown.axis_transforms();
                        let mut points = Vec::with_capacity(shown.series.len());
                        let mut point_indices = Vec::with_capacity(shown.series.len());
                        let never = std::sync::atomic::AtomicBool::new(false);
                        for index in 0..shown.series.len() {
                            let (pts, indices) = shown
                                .display_series_points_indexed_cancellable(
                                    index, budget, transforms, &never,
                                )
                                .expect("Validated axes display metadata");
                            points
                                .push(pts.into_iter().map(|[x, y]| PlotPoint::new(x, y)).collect());
                            point_indices.push(indices);
                        }
                        self.cache.insert(
                            position,
                            CachedAxes {
                                source: source.clone(),
                                points,
                                point_indices,
                                transforms,
                                bounds: None,
                                budget,
                                prepared_bounds: None,
                                legend: None,
                            },
                        );
                    }
                    let cached = self.cache.get_mut(&position).unwrap();
                    let outer_constraint = source.placement.is_some_and(|s| {
                        !s.imported_solved
                            && s.constraint == matrust::plot::PositionConstraint::OuterPosition
                    });
                    let [l, t, r, b] = source.placement.filter(|_| outer_constraint).map_or_else(
                        || position.rectangle(),
                        |s| {
                            let [l, b, w, h] = s.requested_outer;
                            [l, 1. - b - h, l + w, 1. - b]
                        },
                    );
                    let insets = figure
                        .layout
                        .as_ref()
                        .map_or([4.; 4], |l| l.tiled_desktop_insets(position));
                    let mut rect = egui::Rect::from_min_max(
                        area.min
                            + egui::vec2(
                                l as f32 * size.x + insets[0],
                                t as f32 * size.y + insets[1],
                            ),
                        area.min
                            + egui::vec2(
                                r as f32 * size.x - insets[2],
                                b as f32 * size.y - insets[3],
                            ),
                    );
                    if position.custom.is_some() && !outer_constraint {
                        // Position identifies the data box. Native plot widgets also
                        // need room for the selector, ticks and labels around it.
                        rect.min -= egui::vec2(48., 30.);
                        rect.max += egui::vec2(12., 42.);
                        rect = rect.intersect(area);
                    }
                    ui.scope_builder(egui::UiBuilder::new().max_rect(rect), |ui| {
                        // Resolve the enclosing document once; prepared geometry is unchanged.
                        figure_view::apply_appearance(ui, appearance);
                        let fixed_inner = source.placement.is_some_and(|p| {
                            p.imported_solved
                                || p.constraint == matrust::plot::PositionConstraint::InnerPosition
                        });
                        ui.set_clip_rect(
                            if fixed_inner { area } else { rect }.intersect(ui.clip_rect()),
                        );
                        ui.horizontal(|ui| {
                            // Chip: selection tint + text 1 when active; the
                            // figure blue stays for data series only.
                            let active = position == figure.active_position();
                            let p = super::tokens::palette(ui.ctx());
                            if ui
                                .add(
                                    egui::Button::selectable(
                                        active,
                                        egui::RichText::new(
                                            figure
                                                .layout
                                                .as_ref()
                                                .and_then(|l| l.tiled)
                                                .and_then(|g| g.cell(position).ok())
                                                .map_or_else(
                                                    || format!("Axes {}", index + 1),
                                                    |(tile, _)| format!("Tile {tile}"),
                                                ),
                                        )
                                        .color(if active {
                                            p.text
                                        } else {
                                            p.text_muted
                                        }),
                                    )
                                    .fill(if active {
                                        p.accent_soft
                                    } else {
                                        egui::Color32::TRANSPARENT
                                    })
                                    .stroke(egui::Stroke::NONE),
                                )
                                .clicked()
                            {
                                selected = Some(position);
                            }
                            if source.extras.subtitle.string.is_empty() {
                                paint_tile_title(ui, source, appearance);
                            } else {
                                ui.vertical(|ui| {
                                    if !source.title.is_empty() {
                                        paint_tile_title(ui, source, appearance);
                                    }
                                    let [r, g, b] = source.resolve_colors(appearance).title;
                                    let color = egui::Color32::from_rgb(r, g, b);
                                    let font = egui::TextStyle::Body.resolve(ui.style());
                                    if let Some((subtitle, color)) =
                                        graphics_overlay::layout_subtitle(
                                            ui.painter(),
                                            &source.extras.subtitle,
                                            font,
                                            color,
                                        )
                                    {
                                        // Glyph ink can extend beyond the measured text advance.
                                        let size = egui::vec2(
                                            (subtitle.size.x + 2.)
                                                .min(ui.available_width().max(0.)),
                                            subtitle.size.y,
                                        );
                                        let (rect, _) =
                                            ui.allocate_exact_size(size, egui::Sense::hover());
                                        subtitle.paint(
                                            &ui.painter().with_clip_rect(rect),
                                            rect.min + egui::vec2(1., 0.),
                                            color,
                                            0.,
                                        );
                                    }
                                });
                            }
                        });
                        let plot_id = if let Some(root) = explicit_id {
                            root.with(("axes", position))
                        } else if cached.prepared_bounds.is_some()
                            && ui.ctx().viewport_id() != egui::ViewportId::ROOT
                        {
                            egui::Id::new((
                                "detached_subplot",
                                ui.ctx().viewport_id(),
                                id,
                                position,
                            ))
                        } else {
                            egui::Id::new(("subplot", id, position))
                        };
                        let context = figure_view::PanelContext {
                            marker: marker
                                .filter(|(p, _)| *p == position)
                                .map(|(_, point)| point),
                            canvas: Some(area),
                            appearance,
                        };
                        let ortho_pane = figure.orthoslice_viewers.first().and_then(|ov| {
                            ov.axes_identities
                                .iter()
                                .position(|ax_id| ax_id.same_object(&source.identity))
                                .map(|pane_idx| {
                                    let [ny, nx, nz] = ov.dims;
                                    let (dim_idx, max_n) = match pane_idx {
                                        0 => (2, nz),
                                        1 => (0, nx),
                                        _ => (1, ny),
                                    };
                                    (ov.slice_numbers[dim_idx], max_n, dim_idx)
                                })
                        });
                        let avail = ui.available_rect_before_wrap();
                        let (axes_rect, slider_rect) =
                            if ortho_pane.is_some() && avail.height() > 50.0 {
                                let split = avail.bottom() - 24.0;
                                (
                                    egui::Rect::from_min_max(
                                        avail.min,
                                        egui::pos2(avail.right(), split),
                                    ),
                                    Some(egui::Rect::from_min_max(
                                        egui::pos2(avail.left(), split),
                                        avail.max,
                                    )),
                                )
                            } else {
                                (avail, None)
                            };
                        let view = ui
                            .scope_builder(egui::UiBuilder::new().max_rect(axes_rect), |ui| {
                                if let Some(bounds) = cached.prepared_bounds {
                                    figure_view::show_axes_prepared(
                                        ui,
                                        source,
                                        figure_view::PreparedView {
                                            points: &cached.points,
                                            point_indices: Some(&cached.point_indices),
                                            axes: cached.transforms,
                                            bounds,
                                            legend: cached.legend.as_ref(),
                                        },
                                        reset || rebuild,
                                        plot_id,
                                        context,
                                    )
                                } else {
                                    figure_view::show_axes_indexed(
                                        ui,
                                        source,
                                        &cached.points,
                                        Some(&cached.point_indices),
                                        cached.transforms,
                                        reset || rebuild,
                                        plot_id,
                                        context,
                                    )
                                }
                            })
                            .inner;
                        if let (Some((cur, max_n, dim_idx)), Some(s_rect)) =
                            (ortho_pane, slider_rect)
                            && let Some(new_val) = figure_view::slice_slider_and_input(
                                ui,
                                s_rect,
                                axes_rect,
                                cur,
                                max_n,
                                plot_id.with("ortho_slider"),
                            )
                            && let Some(ov) = updated.orthoslice_viewers.first_mut()
                        {
                            ov.slice_numbers[dim_idx] = new_val;
                            let ov_clone = ov.clone();
                            for ax in updated.all_axes_mut() {
                                for (idx, ax_id) in ov_clone.axes_identities.iter().enumerate() {
                                    if ax.identity.same_object(ax_id) {
                                        ov_clone.sync_pane(idx, ax);
                                        break;
                                    }
                                }
                            }
                            changed = true;
                        }
                        if let Some(query) = view.pick {
                            self.pick = Some((position, query));
                        }
                        cached.bounds = Some(view.bounds);
                        let target = if position == updated.active_position() {
                            &mut updated.axes
                        } else {
                            &mut updated
                                .layout
                                .as_mut()
                                .unwrap()
                                .others
                                .iter_mut()
                                .find(|p| p.position == position)
                                .unwrap()
                                .axes
                        };
                        if let Some(visible) = view.visibility {
                            for (s, v) in target.series.iter_mut().zip(visible) {
                                self.undoable_change |= s.visible != v;
                                changed |= s.visible != v;
                                s.visible = v;
                            }
                        }
                        if view.projected {
                            // 3-D: a left drag rotates; projected limits are
                            // never recorded as x/y limits.
                            if let Some(delta) = view.rotate {
                                target.view = matrust::plot::view3d::drag_view(target.view, delta);
                                changed = true;
                            }
                        } else if !ui.input(|i| i.pointer.any_down()) {
                            let mut xy_changed = [false; 2];
                            for (axis, limits) in
                                [&mut target.xlim, &mut target.ylim].into_iter().enumerate()
                            {
                                let next = if view.auto[axis] {
                                    None
                                } else {
                                    Some([view.bounds[2 * axis], view.bounds[2 * axis + 1]])
                                };
                                xy_changed[axis] = *limits != next;
                                changed |= xy_changed[axis];
                                *limits = next;
                            }
                            if xy_changed[0] || xy_changed[1] {
                                synced_axes.push((
                                    target.identity.clone(),
                                    xy_changed[0],
                                    xy_changed[1],
                                ));
                            }
                        }
                    });
                    if position == figure.active_position() {
                        ui.painter().rect_stroke(
                            rect,
                            0.,
                            egui::Stroke::new(1_f32, super::tokens::palette(ui.ctx()).accent),
                            egui::StrokeKind::Inside,
                        );
                    }
                }
                figure_view::paint_textboxes(ui, figure, area);
                figure_view::paint_annotations(ui, figure, area);
            });
        for (target_id, sync_x, sync_y) in synced_axes {
            updated.sync_linked_axes(&target_id, sync_x, sync_y);
        }
        if let Some(position) = selected
            && position != figure.active_position()
        {
            if prepared {
                // This position came from the unchanged prepared source. Swap only
                // existing metadata ownership; no creation or sample validation on
                // the native callback. The worker validates the private candidate.
                if let Some(layout) = &mut updated.layout
                    && let Some(index) = layout
                        .others
                        .iter()
                        .position(|panel| panel.position == position)
                {
                    std::mem::swap(&mut updated.axes, &mut layout.others[index].axes);
                    std::mem::swap(&mut layout.active, &mut layout.others[index].position);
                    layout.others.sort_by_key(|panel| panel.position);
                    changed = true;
                }
            } else {
                #[cfg(test)]
                UI_EDIT_VALIDATIONS.with(|count| count.set(count.get() + 1));
                if updated.select_subplot(position, false).is_ok() {
                    changed = true;
                }
            }
        }
        if !changed {
            return None;
        }
        if prepared {
            return Some(updated);
        }
        #[cfg(test)]
        UI_EDIT_VALIDATIONS.with(|count| count.set(count.get() + 1));
        updated.validate().is_ok().then_some(updated)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn frame(
        canvas: &mut Canvas,
        ctx: &egui::Context,
        figure: &Figure,
        events: Vec<egui::Event>,
    ) -> (egui::FullOutput, Option<Figure>) {
        let mut changed = None;
        let output = crate::headless::run(
            ctx,
            egui::RawInput {
                screen_rect: Some(egui::Rect::from_min_size(
                    egui::Pos2::ZERO,
                    egui::vec2(1000., 700.),
                )),
                events,
                ..Default::default()
            },
            |root| {
                egui::CentralPanel::default()
                    .show(root, |ui| changed = canvas.show(ui, 1, figure, false, None));
            },
        );
        (output, changed)
    }
    fn pointer(pos: egui::Pos2, pressed: bool) -> Vec<egui::Event> {
        vec![
            egui::Event::PointerMoved(pos),
            egui::Event::PointerButton {
                pos,
                button: egui::PointerButton::Primary,
                pressed,
                modifiers: egui::Modifiers::NONE,
            },
        ]
    }
    fn compact_frame(
        canvas: &mut Canvas,
        ctx: &egui::Context,
        figure: &Figure,
        size: egui::Vec2,
        events: Vec<egui::Event>,
    ) -> (egui::FullOutput, egui::Rect) {
        let mut viewport = egui::Rect::NOTHING;
        let output = crate::headless::run(
            ctx,
            egui::RawInput {
                screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, size)),
                events,
                ..Default::default()
            },
            |root| {
                egui::CentralPanel::default().show(root, |ui| {
                    viewport = ui.available_rect_before_wrap();
                    assert!(
                        canvas.show(ui, 1, figure, false, None).is_none(),
                        "View mode must not edit the figure"
                    );
                });
            },
        );
        (output, viewport)
    }
    fn painted_label(output: &egui::FullOutput, label: &str) -> Option<egui::Rect> {
        fn find(shape: &egui::Shape, clip: egui::Rect, label: &str) -> Option<egui::Rect> {
            match shape {
                egui::Shape::Text(t) if t.galley.text() == label => {
                    let rect = egui::Rect::from_min_size(t.pos, t.galley.size());
                    clip.contains_rect(rect).then_some(rect)
                }
                egui::Shape::Vec(v) => v.iter().find_map(|s| find(s, clip, label)),
                _ => None,
            }
        }
        output
            .shapes
            .iter()
            .find_map(|s| find(&s.shape, s.clip_rect, label))
    }
    #[test]
    fn outside_grid_overview_contains_all_legends_and_plot_frames() {
        let mut engine = matrust::Engine::default();
        engine.execute("subplot(2,2,1);plot([0 1],[0 1]);legend('North','Location','northoutside');subplot(2,2,2);plot([0 1],[1 0]);legend('East','Location','eastoutside');subplot(2,2,3);plot([0 1],[1 0]);legend('West','Location','westoutside');subplot(2,2,4);plot([0 1],[0 1]);legend('South','Location','southoutside');sgtitle('Outside legend grid');").unwrap();
        let figure = engine.figure.unwrap();
        let original = figure.clone();
        for size in [egui::vec2(710., 270.), egui::vec2(1200., 560.)] {
            let ctx = egui::Context::default();
            let mut canvas = Canvas::default();
            for _ in 0..3 {
                compact_frame(&mut canvas, &ctx, &figure, size, vec![]);
            }
            let (output, viewport) = compact_frame(&mut canvas, &ctx, &figure, size, vec![]);
            assert!(viewport.contains_rect(painted_label(&output, "Outside legend grid").unwrap()));
            for label in ["North", "East", "West", "South"] {
                assert!(
                    viewport
                        .contains_rect(painted_label(&output, label).unwrap_or_else(|| panic!(
                            "Missing fully visible {label} at {size:?}"
                        )))
                );
            }
            for (position, _) in figure.all_axes() {
                let id = egui::Id::new(("subplot", 1_u32, position));
                let memory = egui_plot::PlotMemory::load(&ctx, id).unwrap();
                let frame = *memory.transform().frame();
                assert!(viewport.contains_rect(frame));
                assert!(
                    frame.width() > 40. && frame.height() > 16.,
                    "Overview must retain a useful data box: {frame:?}"
                );
                let legend = ctx
                    .data(|d| d.get_temp::<Option<egui::Rect>>(id.with("legend_rect")))
                    .flatten()
                    .unwrap();
                assert!(viewport.contains_rect(legend));
                assert!(!frame.intersects(legend));
            }
            assert!(figure.same_snapshot(&original));
        }
    }
    #[test]
    fn custom_outside_overview_and_readable_toggle_preserve_document() {
        let mut engine = matrust::Engine::default();
        engine.execute("subplot('Position',[0.1 0.2 0.3 0.6]);plot([0 1],[0 1]);left=gca;subplot('Position',[0.6 0.2 0.3 0.6]);plot([0 1],[1 0]);right=gca;axes(left);legend('Left','Location','westoutside');axes(right);legend('Right','Location','eastoutside');").unwrap();
        let figure = engine.figure.unwrap();
        let original = figure.clone();
        let ctx = egui::Context::default();
        let mut canvas = Canvas::default();
        let size = egui::vec2(710., 270.);
        for _ in 0..3 {
            compact_frame(&mut canvas, &ctx, &figure, size, vec![]);
        }
        let (output, viewport) = compact_frame(&mut canvas, &ctx, &figure, size, vec![]);
        for label in ["Left", "Right"] {
            assert!(viewport.contains_rect(painted_label(&output, label).unwrap()));
        }
        let readable = painted_label(&output, "Readable size").unwrap().center();
        for pressed in [true, false] {
            compact_frame(&mut canvas, &ctx, &figure, size, pointer(readable, pressed));
        }
        assert_eq!(canvas.fit_overview, Some(false));
        let (output, _) = compact_frame(&mut canvas, &ctx, &figure, size, vec![]);
        let fit = painted_label(&output, "Fit all axes").unwrap().center();
        for pressed in [true, false] {
            compact_frame(&mut canvas, &ctx, &figure, size, pointer(fit, pressed));
        }
        assert_eq!(canvas.fit_overview, Some(true));
        for _ in 0..3 {
            compact_frame(&mut canvas, &ctx, &figure, size, vec![]);
        }
        let (output, _) = compact_frame(&mut canvas, &ctx, &figure, size, vec![]);
        assert!(
            painted_label(&output, "Left").is_some() && painted_label(&output, "Right").is_some()
        );
        assert!(figure.same_snapshot(&original));
        for cached in canvas.cache.values() {
            assert!(
                figure
                    .all_axes()
                    .any(|(_, a)| a.same_snapshot(&cached.source))
            );
        }
    }

    #[test]
    fn fixed_positive_and_negative_log_views_commit_real_pointer_pan() {
        for (values, limits) in [("[1 8]", "[.3 13]"), ("[-1 -8]", "[-13 -.3]")] {
            let mut e = matrust::Engine::default();
            e.execute(&format!(
                "subplot(1,1,1);semilogy([1 2],{values});ylim({limits});xlim([0 3]);"
            ))
            .unwrap();
            let mut figure = e.figure.unwrap();
            let original = figure.clone();
            let samples = figure.series[0].points.clone();
            let ctx = egui::Context::default();
            let mut canvas = Canvas::default();
            for _ in 0..3 {
                assert!(frame(&mut canvas, &ctx, &figure, vec![]).1.is_none());
            }
            frame(
                &mut canvas,
                &ctx,
                &figure,
                pointer(egui::pos2(200., 180.), true),
            );
            frame(
                &mut canvas,
                &ctx,
                &figure,
                vec![egui::Event::PointerMoved(egui::pos2(245., 205.))],
            );
            figure = frame(
                &mut canvas,
                &ctx,
                &figure,
                pointer(egui::pos2(245., 205.), false),
            )
            .1
            .expect("Real log pan must commit on release");
            assert_ne!(
                figure.ylim.unwrap().map(f64::to_bits),
                original.ylim.unwrap().map(f64::to_bits)
            );
            assert!((samples).same_storage(&figure.series[0].points));
            let observed = canvas.active_bounds(&figure).unwrap();
            assert_eq!(
                figure.ylim.unwrap().map(f64::to_bits),
                [observed[2], observed[3]].map(f64::to_bits)
            );
            for _ in 0..3 {
                assert!(frame(&mut canvas, &ctx, &figure, vec![]).1.is_none());
            }
        }
    }

    #[test]
    fn subplot_canvas_selection_and_pan_preserve_neighboring_axes_and_buffers() {
        let mut e = matrust::Engine::default();
        e.execute(include_str!("../../examples/subplots.m"))
            .unwrap();
        let mut figure = e.figure.unwrap();
        let ctx = egui::Context::default();
        let mut canvas = Canvas::default();
        let mut button = None;
        for _ in 0..3 {
            let (output, changed) = frame(&mut canvas, &ctx, &figure, vec![]);
            assert!(
                changed.is_none(),
                "An untouched subplot frame must not change document state"
            );
            button = output.shapes.iter().find_map(|s| {
                if let egui::Shape::Text(t) = &s.shape
                    && t.galley.text() == "Axes 1"
                {
                    Some(t.pos + t.galley.size() * 0.5)
                } else {
                    None
                }
            });
        }
        let position = button.unwrap();
        frame(&mut canvas, &ctx, &figure, pointer(position, true));
        figure = frame(&mut canvas, &ctx, &figure, pointer(position, false))
            .1
            .unwrap();
        assert_eq!(figure.title, "Sine");
        let samples = figure.series[0].points.clone();
        let original = figure.clone();
        frame(&mut canvas, &ctx, &figure, vec![]);
        frame(
            &mut canvas,
            &ctx,
            &figure,
            pointer(egui::pos2(200., 180.), true),
        );
        frame(
            &mut canvas,
            &ctx,
            &figure,
            vec![egui::Event::PointerMoved(egui::pos2(245., 205.))],
        );
        figure = frame(
            &mut canvas,
            &ctx,
            &figure,
            pointer(egui::pos2(245., 205.), false),
        )
        .1
        .expect("Pan must commit axes limits after release");
        assert!(figure.xlim.is_some() && figure.ylim.is_some());
        assert!((samples).same_storage(&figure.series[0].points));
        for panel in &original.layout.as_ref().unwrap().others {
            let updated = figure
                .layout
                .as_ref()
                .unwrap()
                .others
                .iter()
                .find(|p| p.position == panel.position)
                .unwrap();
            assert!(panel.axes.same_snapshot(&updated.axes));
        }
        for _ in 0..3 {
            assert!(frame(&mut canvas, &ctx, &figure, vec![]).1.is_none());
        }
        assert!(
            canvas
                .cache
                .values()
                .flat_map(|c| &c.points)
                .map(Vec::len)
                .sum::<usize>()
                <= 8192
        );
    }
}

#[cfg(test)]
mod annotation_render_tests {
    use super::*;
    #[test]
    fn native_annotation_is_literal_and_inside_its_clip_at_both_viewports() {
        for size in [egui::vec2(640., 480.), egui::vec2(1440., 900.)] {
            let mut figure = Figure::default();
            figure
                .add_textbox(
                    matrust::plot::TextBox::new([0., 0.9, 1., 0.1], "Literal <textbox>".into())
                        .unwrap(),
                )
                .unwrap();
            let ctx = egui::Context::default();
            let mut area = egui::Rect::NOTHING;
            let output = crate::headless::run(
                &ctx,
                egui::RawInput {
                    screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, size)),
                    ..Default::default()
                },
                |root| {
                    egui::CentralPanel::default().show(root, |ui| {
                        area = ui.available_rect_before_wrap();
                        figure_view::paint_textboxes(ui, &figure, area);
                        figure_view::paint_annotations(ui, &figure, area);
                    });
                },
            );
            let texts: Vec<_> = output
                .shapes
                .iter()
                .filter_map(|s| {
                    if let egui::Shape::Text(t) = &s.shape {
                        Some((s.clip_rect, t))
                    } else {
                        None
                    }
                })
                .collect();
            assert_eq!(texts.len(), 1);
            assert_eq!(texts[0].1.galley.job.text, "Literal <textbox>");
            assert!(
                texts[0].0.min.y >= area.min.y
                    && texts[0].0.max.y <= area.min.y + area.height() * 0.11
            );
            assert!(texts[0].0.contains(texts[0].1.pos));
        }
    }
}
#[cfg(test)]
mod log_idle_tests {
    use super::*;
    #[test]
    fn explicit_log_axes_idle_frames_do_not_publish_a_model_edit() {
        for limits in ["[.3 13]", "[-13 -.3]"] {
            let values = if limits.starts_with("[-") {
                "[-1 -8]"
            } else {
                "[1 8]"
            };
            let mut e = matrust::Engine::default();
            e.execute(&format!("subplot(1,2,1);semilogy([1 2],{values});ylim({limits});subplot(1,2,2);semilogy([1 2],{values});ylim({limits});")).unwrap();
            let figure = e.figure.unwrap();
            let before = figure.clone();
            for size in [egui::vec2(640., 480.), egui::vec2(1440., 900.)] {
                let ctx = egui::Context::default();
                let mut canvas = Canvas::default();
                for _ in 0..4 {
                    let _ = crate::headless::run(
                        &ctx,
                        egui::RawInput {
                            screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, size)),
                            ..Default::default()
                        },
                        |root| {
                            egui::CentralPanel::default().show(root, |ui| {
                                assert!(
                                    canvas.show(ui, 1, &figure, false, None).is_none(),
                                    "No pointer/keyboard input must not rewrite fixed log limits"
                                )
                            });
                        },
                    );
                }
                assert!(figure.same_snapshot(&before));
            }
        }
    }
}
