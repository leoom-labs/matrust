use super::{tokens, worker::FigureEdit};
use crate::theme::ThemedScrollArea;
use eframe::egui;
use matrust::{
    Result,
    plot::{AxisScale, Figure, LinePattern, PlotKind, Position},
};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Tab {
    Axes,
    Series,
    Layout,
    Appearance,
}
struct LayoutPreview {
    position: [String; 4],
    outer_position: [String; 4],
    constraint: Option<matrust::plot::PositionConstraint>,
    metadata: matrust::plot::Axes,
    title: String,
    panels: Vec<(Position, [f64; 4])>,
    allocations: Vec<[f64; 4]>,
    legends: Vec<[f64; 4]>,
    active: Position,
    error: Option<String>,
}

pub struct Editor {
    id: u32,
    original: Figure,
    draft: Figure,
    limits: [[String; 2]; 2],
    auto: [bool; 2],
    view: Option<[f64; 4]>,
    tab: Tab,
    position: [String; 4],
    initial_position: [String; 4],
    outer_position: [String; 4],
    initial_outer_position: [String; 4],
    constraint: Option<matrust::plot::PositionConstraint>,
    initial_constraint: Option<matrust::plot::PositionConstraint>,
    layout_preview: Option<LayoutPreview>,
    selected: usize,
    pub saving: bool,
    pub error: Option<String>,
    pub reset_view: bool,
}

pub enum Action {
    None,
    Cancel,
    Apply(Box<FigureEdit>),
}
impl Editor {
    pub fn new(id: u32, figure: &Figure, view: Option<[f64; 4]>) -> Self {
        // Opening Properties must remain O(series), even with huge plots. The
        // displayed view already has bounds; do not rescan samples on the UI.
        let limits = [figure.xlim, figure.ylim]
            .into_iter()
            .enumerate()
            .map(|(i, limit)| {
                limit
                    .or_else(|| view.map(|v| [v[2 * i], v[2 * i + 1]]))
                    .unwrap_or([0., 1.])
                    .map(|v| v.to_string())
            })
            .collect::<Vec<_>>()
            .try_into()
            .unwrap();
        let position = figure
            .normalized_axes_positions()
            .first()
            .map_or([0., 0., 1., 1.], |(_, p)| *p)
            .map(|v| v.to_string());
        let outer_position = figure
            .initial_axes_placement()
            .map_or([0., 0., 1., 1.], |p| p.requested_outer)
            .map(|v| v.to_string());
        let constraint = figure.placement.map(|p| p.constraint);
        Self {
            id,
            original: figure.clone(),
            draft: figure.clone(),
            limits,
            auto: [figure.xlim.is_none(), figure.ylim.is_none()],
            view,
            tab: Tab::Axes,
            initial_position: position.clone(),
            position,
            initial_outer_position: outer_position.clone(),
            outer_position,
            constraint,
            initial_constraint: constraint,
            layout_preview: None,
            selected: 0,
            saving: false,
            error: None,
            reset_view: false,
        }
    }
    pub fn show_series(&mut self) {
        self.tab = Tab::Series;
    }
    pub fn show_layout(&mut self) {
        self.tab = Tab::Layout;
    }
    fn position_values(&self) -> Result<[f64; 4]> {
        let mut values = [0.; 4];
        for (i, value) in values.iter_mut().enumerate() {
            *value = self.position[i]
                .trim()
                .parse()
                .map_err(|_| "Position values must be numbers".to_owned())?;
        }
        Position::from_normalized(values)?;
        Ok(values)
    }
    fn candidate(&self) -> Result<Figure> {
        let mut f = self.draft.clone();
        for (i, target) in [&mut f.axes.xlim, &mut f.axes.ylim].into_iter().enumerate() {
            *target = if self.auto[i] {
                None
            } else {
                let parse = |s: &str| {
                    s.trim().parse::<f64>().map_err(|_| {
                        format!("{} limits must be numbers", if i == 0 { "X" } else { "Y" })
                    })
                };
                Some([parse(&self.limits[i][0])?, parse(&self.limits[i][1])?])
            };
        }
        if f.empty_layout() && !f.axes.same_snapshot(&self.original.axes) {
            f.activate_tile();
        }
        let boundary_changed = self.constraint != self.initial_constraint
            || self.outer_position != self.initial_outer_position;
        if boundary_changed {
            if let Some(constraint) = self.constraint {
                let mut placement = f.initial_axes_placement()?;
                for (target, text) in placement
                    .requested_outer
                    .iter_mut()
                    .zip(&self.outer_position)
                {
                    *target = text
                        .trim()
                        .parse::<f64>()
                        .map_err(|_| "OuterPosition values must be numbers".to_owned())?;
                }
                placement.requested_inner = self.position_values()?;
                placement.constraint = constraint;
                f.set_axes_placement(placement)?;
            } else {
                // Reset the explicit constraint. A custom outside legend then
                // derives a fresh, deterministic outer allocation below;
                // an inside legend returns to the legacy requested data box.
                f.placement = None;
                if self.position != self.initial_position {
                    f.reposition_axes(self.position_values()?)?;
                }
            }
        } else if self.position != self.initial_position {
            f.reposition_axes(self.position_values()?)?;
        }
        if f.legend_location != self.original.legend_location
            && let Some(placement) = &mut f.placement
        {
            placement.imported_solved = false;
        }
        f.prepare_legend_layout()?;
        f.validate()?;
        Ok(f)
    }
    pub fn show(&mut self, ctx: &egui::Context) -> Action {
        let mut action = Action::None;
        let shown = egui::Modal::new(egui::Id::new("figure_properties")).show(ctx, |ui| {
            ui.set_width((ctx.content_rect().width() - 48.).clamp(240., 520.));
            ui.label(crate::tokens::title(
                ui.ctx(),
                format!("Figure {} properties", self.id),
            ));
            ui.label("Edit the figure, then apply your changes.");
            ui.separator();
            ui.add_enabled_ui(!self.saving, |ui| {
                ui.horizontal_wrapped(|ui| {
                    ui.selectable_value(&mut self.tab, Tab::Axes, "Axes and labels");
                    ui.selectable_value(&mut self.tab, Tab::Series, "Series");
                    ui.selectable_value(&mut self.tab, Tab::Layout, "Layout");
                    ui.selectable_value(&mut self.tab, Tab::Appearance, "Appearance");
                });
                egui::ScrollArea::vertical()
                    .id_salt(("figure_property_body", self.tab as u8))
                    .max_height(
                        (ctx.content_rect().height() - 250.)
                            .clamp(40., if self.tab == Tab::Layout { 560. } else { 480. }),
                    )
                    .auto_shrink([false, true])
                    .show_viewport_themed(ui, |ui, viewport| match self.tab {
                        Tab::Axes => self.axes_ui(ui),
                        Tab::Series => self.series_ui(ui),
                        Tab::Layout => self.layout_ui(ui, viewport.height()),
                        Tab::Appearance => self.appearance_ui(ui),
                    });
                ui.separator();
                if let Some(error) = &self.error {
                    egui::ScrollArea::vertical()
                        .id_salt("figure_property_error")
                        .max_height(70.)
                        .auto_shrink([false, true])
                        .show_themed(ui, |ui| {
                            ui.colored_label(tokens::palette(ui.ctx()).error, error);
                        });
                }
                ui.horizontal_wrapped(|ui| {
                    if ui.button("Apply changes").clicked() {
                        match self.candidate() {
                            Ok(updated) => {
                                self.saving = true;
                                self.error = None;
                                action = Action::Apply(Box::new(FigureEdit {
                                    view_before: None,
                                    id: self.id,
                                    original: self.original.clone(),
                                    updated,
                                }));
                            }
                            Err(e) => self.error = Some(e),
                        }
                    }
                    if ui.button("Cancel").clicked() {
                        action = Action::Cancel;
                    }
                    if ui.button("Restore original").clicked() {
                        let tab = self.tab;
                        *self = Self::new(self.id, &self.original, self.view);
                        self.tab = tab;
                    }
                });
            });
            if self.saving {
                ui.spinner();
                ui.label("Applying changes…");
            }
            ui.small("Use Save in the Figure toolbar to keep an editable copy.");
        });
        if !self.saving
            && shown.is_top_modal
            && !shown.any_popup_open
            && ctx.input_mut(|i| i.consume_key(egui::Modifiers::NONE, egui::Key::Escape))
        {
            action = Action::Cancel;
        }
        action
    }
    fn layout_ui(&mut self, ui: &mut egui::Ui, viewport_height: f32) {
        // Measure consumed content relative to its unscrolled origin. Using
        // clip.bottom - cursor.top would resize the preview as the user scrolls.
        let content_top = ui.cursor().top();
        let field_width = (ui.available_width() - 90.).clamp(70., 390.);
        if self.original.empty_layout() {
            ui.label("Choose Next tile and create an axes before editing its position.");
            return;
        }
        ui.label("Move or resize the selected axes. Other axes keep their data.");
        ui.small("Fractions of the plot canvas: Left and Bottom start at 0; Width and Height end within 1. Shared titles sit above this canvas.");
        ui.add_space(8.);
        egui::Grid::new("axes_position_fields")
            .num_columns(2)
            .spacing([12., 8.])
            .show(ui, |ui| {
                for (i, label) in ["Left", "Bottom", "Width", "Height"]
                    .into_iter()
                    .enumerate()
                {
                    ui.label(label);
                    ui.add(
                        egui::TextEdit::singleline(&mut self.position[i])
                            .id(egui::Id::new(("axes_position", i)))
                            .char_limit(64)
                            .desired_width(field_width),
                    );
                    ui.end_row();
                }
            });
        if ui.button("Reset position").clicked() {
            self.position = self.initial_position.clone();
            self.outer_position = self.initial_outer_position.clone();
            self.constraint = self.initial_constraint;
            self.error = None;
        }
        ui.label("Held layout boundary");
        egui::ComboBox::from_id_salt("axes_position_constraint")
            .selected_text(self.constraint.map_or("Automatic allocation", |v| v.name()))
            .show_ui(ui, |ui| {
                ui.selectable_value(&mut self.constraint, None, "Reset to automatic allocation");
                ui.selectable_value(
                    &mut self.constraint,
                    Some(matrust::plot::PositionConstraint::InnerPosition),
                    "Hold inner plot box",
                );
                ui.selectable_value(
                    &mut self.constraint,
                    Some(matrust::plot::PositionConstraint::OuterPosition),
                    "Hold outer boundary",
                );
            });
        ui.small("Automatic preserves the requested data box for inside legends and derives an outer allocation for custom outside legends. Explicit held boundaries are saved with the figure.");
        egui::CollapsingHeader::new("Requested OuterPosition").show(ui, |ui| {
            ui.add_enabled_ui(self.constraint.is_some(), |ui| {
                egui::Grid::new("axes_outer_position_fields")
                    .num_columns(2)
                    .show(ui, |ui| {
                        for (i, label) in
                            ["Outer left", "Outer bottom", "Outer width", "Outer height"]
                                .into_iter()
                                .enumerate()
                        {
                            ui.label(label);
                            ui.add(
                                egui::TextEdit::singleline(&mut self.outer_position[i])
                                    .id(egui::Id::new(("axes_outer_position", i)))
                                    .char_limit(64)
                                    .desired_width(field_width),
                            );
                            ui.end_row();
                        }
                    });
            });
        });
        if self
            .original
            .layout
            .as_ref()
            .is_some_and(|l| l.tiled.is_some())
        {
            ui.small("Changing a position converts this tiled layout to custom axes. All plots and the shared title are retained.");
        }
        let preview = self.layout_preview();
        if let Some(error) = &preview.error {
            ui.colored_label(tokens::palette(ui.ctx()).error, error);
        }
        ui.add_space(8.);
        let caption = egui::WidgetText::from(
            "Selected data box is blue; outer allocations are gray outlines and outside legends are gold. Overlapping allocations cannot be applied."
        ).into_galley(ui, Some(egui::TextWrapMode::Wrap), ui.available_width(), egui::TextStyle::Small);
        let width = ui.available_width().min(500.);
        let remaining = viewport_height
            - (ui.cursor().top() - content_top)
            - caption.size().y
            - ui.spacing().item_spacing.y;
        // Keep a useful diagram when expanded fields/errors exceed the body;
        // the existing body scrollbar then exposes it and the caption.
        let height = (width * 0.38).min(remaining.max(64.));
        let (rect, _) = ui.allocate_exact_size(egui::vec2(width, height), egui::Sense::hover());
        let pal = tokens::palette(ui.ctx());
        ui.painter().rect_filled(rect, 3., pal.surface_sunken);
        let preview_rect = |p: [f64; 4]| {
            egui::Rect::from_min_size(
                rect.min
                    + egui::vec2(
                        p[0] as f32 * rect.width(),
                        (1. - p[1] - p[3]) as f32 * rect.height(),
                    ),
                egui::vec2(p[2] as f32 * rect.width(), p[3] as f32 * rect.height()),
            )
        };
        for &outer in &preview.allocations {
            ui.painter().rect_stroke(
                preview_rect(outer),
                0.,
                egui::Stroke::new(1., pal.text_disabled),
                egui::StrokeKind::Inside,
            );
        }
        for &legend in &preview.legends {
            ui.painter()
                .rect_filled(preview_rect(legend), 0., pal.warning.gamma_multiply(0.3));
        }
        for (i, (p, [left, bottom, width, height])) in preview.panels.iter().copied().enumerate() {
            let panel = egui::Rect::from_min_size(
                rect.min
                    + egui::vec2(
                        left as f32 * rect.width(),
                        (1. - bottom - height) as f32 * rect.height(),
                    ),
                egui::vec2(width as f32 * rect.width(), height as f32 * rect.height()),
            );
            let active = p == preview.active;
            let color = if active { pal.accent } else { pal.text_muted };
            ui.painter().rect_filled(
                panel,
                0.,
                if active { pal.accent_soft } else { pal.surface },
            );
            ui.painter().rect_stroke(
                panel,
                0.,
                egui::Stroke::new(if active { 2_f32 } else { 1_f32 }, color),
                egui::StrokeKind::Inside,
            );
            if panel.width() >= 45. && panel.height() >= 18. {
                ui.painter().text(
                    panel.center(),
                    egui::Align2::CENTER_CENTER,
                    format!("Axes {}", i + 1),
                    egui::FontId::proportional(tokens::CAPTION),
                    color,
                );
            }
        }
        ui.add(egui::Label::new(caption));
    }
    fn layout_preview(&mut self) -> &LayoutPreview {
        let title = self.draft.layout.as_ref().map_or("", |l| l.title.as_str());
        if self.layout_preview.as_ref().is_none_or(|p| {
            p.position != self.position
                || p.outer_position != self.outer_position
                || p.constraint != self.constraint
                || !p.metadata.same_metadata(&self.draft.axes)
                || p.title != title
        }) {
            // Rebuild only after geometry input changes. The retained cache
            // contains at most 16 rectangles and bounded style/label metadata.
            // No sample payload is retained or inspected by the preview key.
            let (figure, mut error) = match self.candidate() {
                Ok(f) => (f, None),
                Err(e) => (self.draft.clone(), Some(e)),
            };
            let unit = matrust::plot::LegendRect::new([0., 0., 1., 1.]);
            let mut allocations = Vec::new();
            let mut legends = Vec::new();
            let mut panels = match figure.resolved_axes_geometry() {
                Ok(geometry) => geometry
                    .into_iter()
                    .map(|(p, g)| {
                        if figure
                            .all_axes()
                            .find(|(q, _)| *q == p)
                            .is_some_and(|(_, a)| {
                                a.placement.is_some() || a.legend_location.is_outside()
                            })
                        {
                            allocations.push(g.outer.normalized(unit));
                        }
                        if let Some(legend) = g.legend {
                            legends.push(legend.normalized(unit));
                        }
                        (p, g.data.normalized(unit))
                    })
                    .collect(),
                Err(e) => {
                    error = Some(e);
                    figure.normalized_axes_positions()
                }
            };
            panels.sort_by_key(|(p, _)| *p);
            let mut metadata = self.draft.axes.clone();
            let empty = matrust::plot::SeriesPoints::from(Vec::new());
            for s in &mut metadata.series {
                s.points = empty.clone();
            }
            self.layout_preview = Some(LayoutPreview {
                position: self.position.clone(),
                outer_position: self.outer_position.clone(),
                constraint: self.constraint,
                metadata,
                title: title.to_owned(),
                panels,
                allocations,
                legends,
                active: figure.active_position(),
                error,
            });
        }
        self.layout_preview.as_ref().unwrap()
    }
    fn appearance_ui(&mut self, ui: &mut egui::Ui) {
        use matrust::plot::{ColorMode, FigureTheme};
        let host = super::figure_view::host_theme(ui.ctx());
        let policy = self.draft.appearance_policy();
        ui.label("Figure appearance");
        let mut theme = policy.theme;
        ui.horizontal_wrapped(|ui| {
            ui.label("Theme");
            ui.selectable_value(&mut theme, FigureTheme::Auto, "Desktop");
            ui.selectable_value(&mut theme, FigureTheme::Light, "Light");
            ui.selectable_value(&mut theme, FigureTheme::Dark, "Dark");
        });
        if theme != policy.theme {
            self.draft.set_figure_theme(theme);
        }
        ui.label("Desktop follows the active desktop theme. Light and Dark stay fixed.");
        let mut automatic = policy.canvas_mode == ColorMode::Auto;
        if ui
            .checkbox(&mut automatic, "Automatic canvas background")
            .changed()
        {
            self.draft.set_canvas_mode(
                if automatic {
                    ColorMode::Auto
                } else {
                    ColorMode::Manual
                },
                host,
            );
        }
        if !automatic {
            let mut color = self.draft.resolve_appearance(host).canvas;
            ui.horizontal(|ui| {
                ui.label("Canvas color");
                if ui.color_edit_button_srgb(&mut color).changed() {
                    self.draft.set_canvas_color(color);
                }
            });
        }
        ui.weak("Automatic series follow the figure theme. Explicit colors stay fixed. Apply records this choice; Cancel keeps the original.");
    }
    fn axes_ui(&mut self, ui: &mut egui::Ui) {
        let field_width = (ui.available_width() - 90.).clamp(70., 390.);
        if let Some(layout) = &mut self.draft.layout {
            ui.label("Shared figure title");
            ui.add(
                egui::TextEdit::singleline(&mut layout.title)
                    .char_limit(4096)
                    .desired_width(f32::INFINITY),
            );
            ui.add_space(8.);
        }
        egui::Grid::new("figure_labels")
            .num_columns(2)
            .spacing([16., 8.])
            .show(ui, |ui| {
                for (name, text) in [
                    ("Title", &mut self.draft.axes.title),
                    ("X label", &mut self.draft.axes.xlabel),
                    ("Y label", &mut self.draft.axes.ylabel),
                ] {
                    ui.label(name);
                    ui.add(
                        egui::TextEdit::singleline(text)
                            .id(egui::Id::new(("figure_property", name)))
                            .char_limit(4096)
                            .desired_width(field_width),
                    );
                    ui.end_row();
                }
            });
        ui.add_space(10.);
        for (i, scale) in [&mut self.draft.axes.xscale, &mut self.draft.axes.yscale]
            .into_iter()
            .enumerate()
        {
            ui.group(|ui| {
                ui.horizontal_wrapped(|ui| {
                    ui.strong(if i == 0 { "X axis" } else { "Y axis" });
                    self.reset_view |= ui
                        .selectable_value(scale, AxisScale::Linear, "Linear")
                        .changed();
                    self.reset_view |= ui
                        .selectable_value(scale, AxisScale::Log, "Logarithmic")
                        .changed();
                    self.reset_view |= ui.checkbox(&mut self.auto[i], "Automatic limits").changed();
                });
                ui.add_enabled_ui(!self.auto[i], |ui| {
                    ui.horizontal_wrapped(|ui| {
                        ui.label("Minimum");
                        ui.add(
                            egui::TextEdit::singleline(&mut self.limits[i][0])
                                .desired_width(130.)
                                .char_limit(64),
                        );
                        ui.label("Maximum");
                        ui.add(
                            egui::TextEdit::singleline(&mut self.limits[i][1])
                                .desired_width(130.)
                                .char_limit(64),
                        );
                    });
                });
            });
        }
        if ui
            .add_enabled(
                self.view.is_some(),
                egui::Button::new("Use current zoom limits"),
            )
            .clicked()
            && let Some(v) = self.view
        {
            self.auto = [false; 2];
            self.limits = [
                [v[0].to_string(), v[1].to_string()],
                [v[2].to_string(), v[3].to_string()],
            ];
        }
        if ui.button("Reset to automatic limits").clicked() {
            self.auto = [true; 2];
            self.reset_view = true;
        }
        ui.horizontal_wrapped(|ui| {
            ui.checkbox(&mut self.draft.xgrid, "X grid");
            ui.checkbox(&mut self.draft.ygrid, "Y grid");
            ui.checkbox(&mut self.draft.legend, "Legend");
            ui.checkbox(&mut self.draft.hold, "Hold new plots");
        });
        ui.horizontal_wrapped(|ui| {
            ui.label("Legend position");
            egui::ComboBox::from_id_salt("property_legend_location")
                .selected_text(self.draft.legend_location.label())
                .show_ui(ui, |ui| {
                    for location in matrust::plot::LegendLocation::ALL {
                        ui.selectable_value(
                            &mut self.draft.legend_location,
                            location,
                            location.label(),
                        );
                    }
                });
        });
    }
    fn series_ui(&mut self, ui: &mut egui::Ui) {
        if self.draft.series.is_empty() {
            ui.label("This figure has no series.");
            return;
        }
        self.selected = self.selected.min(self.draft.series.len() - 1);
        egui::ComboBox::from_id_salt("property_series")
            .width(ui.available_width().min(470.))
            .truncate()
            .selected_text(format!(
                "{}. {}",
                self.selected + 1,
                self.draft.series[self.selected]
                    .label
                    .chars()
                    .take(48)
                    .collect::<String>()
            ))
            .show_ui(ui, |ui| {
                for (i, s) in self.draft.series.iter().enumerate() {
                    ui.selectable_value(
                        &mut self.selected,
                        i,
                        format!(
                            "{}. {} ({} points)",
                            i + 1,
                            s.label.chars().take(48).collect::<String>(),
                            s.points.len()
                        ),
                    );
                }
            });
        let appearance = self
            .draft
            .resolve_appearance(super::figure_view::host_theme(ui.ctx()));
        let s = &mut self.draft.series[self.selected];
        ui.checkbox(&mut s.visible, "Visible in plot");
        ui.horizontal_wrapped(|ui| {
            ui.label("Legend label");
            ui.add(
                egui::TextEdit::singleline(&mut s.label)
                    .desired_width(ui.available_width().min(370.))
                    .char_limit(4096),
            );
        });
        ui.horizontal_wrapped(|ui| {
            ui.label("Color");
            let mut color = appearance.series_color(s);
            if ui.color_edit_button_srgb(&mut color).changed() {
                s.color = color;
                s.explicit_color = true;
            }
            if s.kind == PlotKind::Patch {
                ui.label("Patch (face geometry)");
            } else {
                egui::ComboBox::from_id_salt("property_kind")
                    .selected_text(format!("{:?}", s.kind))
                    .show_ui(ui, |ui| {
                        for kind in [
                            PlotKind::Line,
                            PlotKind::Scatter,
                            PlotKind::Bar,
                            PlotKind::Stairs,
                            PlotKind::Stem,
                        ] {
                            if ui
                                .selectable_value(&mut s.kind, kind, format!("{kind:?}"))
                                .changed()
                            {
                                if kind == PlotKind::Stem {
                                    s.markers = true;
                                }
                                if !matches!(kind, PlotKind::Bar | PlotKind::Patch) {
                                    s.face_edge_color = None;
                                }
                            }
                        }
                    });
            }
        });
        if matches!(s.kind, PlotKind::Bar | PlotKind::Patch) {
            ui.horizontal_wrapped(|ui| {
                let mut outlined = s.face_edge_color.is_some();
                if ui.checkbox(&mut outlined, "Outline").changed() {
                    s.face_edge_color = outlined.then_some([0; 3]);
                    if outlined && s.pattern == LinePattern::None {
                        s.pattern = LinePattern::Solid;
                    }
                }
                if let Some(edge) = &mut s.face_edge_color {
                    ui.color_edit_button_srgb(edge);
                }
            });
        }
        if matches!(
            s.kind,
            PlotKind::Line | PlotKind::Stairs | PlotKind::Stem | PlotKind::Bar | PlotKind::Patch
        ) {
            egui::ComboBox::from_id_salt("property_pattern")
                .selected_text(format!("{:?}", s.pattern))
                .show_ui(ui, |ui| {
                    for (pattern, name) in [
                        (LinePattern::Solid, "Solid"),
                        (LinePattern::Dashed, "Dashed"),
                        (LinePattern::Dotted, "Dotted"),
                        (LinePattern::DashDot, "Dash-dot"),
                        (LinePattern::None, "None"),
                    ] {
                        ui.selectable_value(&mut s.pattern, pattern, name);
                    }
                });
            if !matches!(s.kind, PlotKind::Bar | PlotKind::Patch) {
                ui.checkbox(&mut s.markers, "Show markers");
            }
        }
        {
            ui.horizontal_wrapped(|ui| {
                let mut custom = s.line_width.is_some();
                if ui.checkbox(&mut custom, "Custom line width").changed() {
                    s.line_width = custom.then_some(1.5);
                }
                if let Some(width) = &mut s.line_width {
                    ui.add(
                        egui::DragValue::new(width)
                            .speed(0.1)
                            .range(0.1..=50.)
                            .suffix(" pt"),
                    );
                }
            });
        }
        if s.kind == PlotKind::Scatter || s.markers {
            ui.horizontal_wrapped(|ui| {
                ui.label("Marker");
                egui::ComboBox::from_id_salt("marker_shape")
                    .selected_text(
                        s.marker_shape
                            .map_or("Legacy default", |shape| shape.label()),
                    )
                    .show_ui(ui, |ui| {
                        ui.selectable_value(&mut s.marker_shape, None, "Legacy default");
                        for shape in matrust::plot::MarkerShape::ALL {
                            ui.selectable_value(&mut s.marker_shape, Some(shape), shape.label());
                        }
                    });
                let mut custom = s.marker_size.is_some();
                if ui.checkbox(&mut custom, "Custom marker size").changed() {
                    s.marker_size = custom.then_some(s.marker_size_points());
                }
                if let Some(size) = &mut s.marker_size {
                    ui.add(
                        egui::DragValue::new(size)
                            .range(0.1..=100.)
                            .speed(0.1)
                            .suffix(" pt"),
                    );
                }
            });
            if s.marker_shape.is_none()
                || !matches!(
                    s.effective_marker(),
                    matrust::plot::MarkerShape::Point
                        | matrust::plot::MarkerShape::Cross
                        | matrust::plot::MarkerShape::Plus
                        | matrust::plot::MarkerShape::Asterisk
                )
            {
                ui.checkbox(&mut s.marker_filled, "Filled point markers");
            }
        }
        if s.kind == PlotKind::Bar {
            ui.horizontal_wrapped(|ui| {
                ui.add(
                    egui::DragValue::new(&mut s.bar_width)
                        .speed(0.01)
                        .prefix("Width "),
                );
                ui.add(
                    egui::DragValue::new(&mut s.bar_offset)
                        .speed(0.01)
                        .prefix("Offset "),
                );
            });
            ui.small("Bar figures are limited to 10,000 total points.");
        }
        ui.add_space(12.);
        ui.horizontal_wrapped(|ui| {
            if ui
                .add_enabled(self.selected > 0, egui::Button::new("Move earlier"))
                .clicked()
            {
                self.draft.series.swap(self.selected, self.selected - 1);
                self.selected -= 1;
            }
            if ui
                .add_enabled(
                    self.selected + 1 < self.draft.series.len(),
                    egui::Button::new("Move later"),
                )
                .clicked()
            {
                self.draft.series.swap(self.selected, self.selected + 1);
                self.selected += 1;
            }
            if ui.button("Remove series").clicked() {
                self.draft
                    .remove_series(self.selected)
                    .expect("Displayed series index is valid");
                self.selected = self.selected.saturating_sub(1);
            }
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn outside_preview_tracks_label_edits_and_constraint_reset_without_sample_retention() {
        let mut engine = matrust::Engine::default();
        engine.execute("subplot('Position',[0.15 0.2 0.55 0.6]);plot([1 2]);legend('Short','Location','eastoutside');").unwrap();
        let source = engine.figure.unwrap();
        let samples = source.series[0].points.clone();
        let mut editor = Editor::new(1, &source, None);
        let owners = samples.strong_count();
        assert_eq!(editor.layout_preview().metadata.series[0].label, "Short");
        assert_eq!(samples.strong_count(), owners);
        editor.draft.series[0].label = "A much longer label with the same entry count".into();
        let label = editor.draft.series[0].label.clone();
        assert_eq!(editor.layout_preview().metadata.series[0].label, label);
        assert_eq!(samples.strong_count(), owners);
        editor.constraint = None;
        editor.outer_position = ["invalid".into(), "0".into(), "0".into(), "0".into()];
        let auto = editor.candidate().unwrap();
        assert_eq!(
            auto.placement.unwrap().requested_outer,
            source.placement.unwrap().requested_outer
        );
        assert!(!auto.placement.unwrap().imported_solved);
        editor.draft.legend_location = matrust::plot::LegendLocation::Northwest;
        let legacy = editor.candidate().unwrap();
        assert!(legacy.placement.is_none());
        assert!((samples).same_storage(&legacy.series[0].points));
        // Restoring the original editor state also restores the original held
        // boundary; it does not use a previously solved preview as its request.
        let restored = Editor::new(1, &source, None).candidate().unwrap();
        assert!(restored.same_snapshot(&source));
    }
    fn frame(
        editor: &mut Editor,
        ctx: &egui::Context,
        events: Vec<egui::Event>,
    ) -> (egui::FullOutput, Action) {
        let mut action = Action::None;
        let output = crate::headless::run(
            ctx,
            egui::RawInput {
                screen_rect: Some(egui::Rect::from_min_size(
                    egui::Pos2::ZERO,
                    egui::vec2(960., 650.),
                )),
                events,
                ..Default::default()
            },
            |root| {
                let ctx = &root.ctx().clone();
                action = editor.show(ctx);
            },
        );
        (output, action)
    }
    fn button(shape: &egui::Shape, label: &str) -> Option<egui::Pos2> {
        match shape {
            egui::Shape::Text(text) if text.galley.text() == label => {
                Some(text.pos + text.galley.size() * 0.5)
            }
            egui::Shape::Vec(shapes) => shapes.iter().find_map(|s| button(s, label)),
            _ => None,
        }
    }
    fn click(editor: &mut Editor, ctx: &egui::Context, label: &str) -> Action {
        let mut position = None;
        for _ in 0..3 {
            let (output, _) = frame(editor, ctx, vec![]);
            position = output.shapes.iter().find_map(|s| button(&s.shape, label));
        }
        let position = position.unwrap();
        for pressed in [true, false] {
            let (_, action) = frame(
                editor,
                ctx,
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
            if !matches!(action, Action::None) {
                return action;
            }
        }
        Action::None
    }
    #[test]
    fn legend_location_property_picker_applies_without_copying_samples() {
        let mut engine = matrust::Engine::default();
        engine
            .execute("plot([1 2],[3 4]);legend('Signal');")
            .unwrap();
        let source = engine.figure.unwrap();
        let ctx = egui::Context::default();
        let mut editor = Editor::new(1, &source, None);
        click(&mut editor, &ctx, "Top right");
        click(&mut editor, &ctx, "Bottom left");
        assert_eq!(
            editor.draft.legend_location,
            matrust::plot::LegendLocation::Southwest
        );
        let Action::Apply(edit) = click(&mut editor, &ctx, "Apply changes") else {
            panic!("missing apply")
        };
        assert_eq!(
            edit.updated.legend_location,
            matrust::plot::LegendLocation::Southwest
        );
        assert!(!edit.original.same_snapshot(&edit.updated));
        assert!((edit.original.series[0].points).same_storage(&edit.updated.series[0].points));
    }
    #[test]
    fn layout_widgets_validate_apply_reset_and_preserve_other_axes() {
        let mut engine = matrust::Engine::default();
        engine
            .execute(include_str!("../../examples/fig_layouts.m"))
            .unwrap();
        let source = engine.figure.unwrap();
        let ctx = egui::Context::default();
        let mut editor = Editor::new(1, &source, None);
        click(&mut editor, &ctx, "Layout");
        frame(&mut editor, &ctx, vec![]);
        let width_id = egui::Id::new(("axes_position", 2_usize));
        ctx.memory_mut(|m| m.request_focus(width_id));
        frame(
            &mut editor,
            &ctx,
            vec![
                egui::Event::Key {
                    key: egui::Key::A,
                    physical_key: None,
                    pressed: true,
                    repeat: false,
                    modifiers: egui::Modifiers {
                        ctrl: true,
                        command: true,
                        ..Default::default()
                    },
                },
                egui::Event::Text("0.7".into()),
            ],
        );
        assert_eq!(editor.position[2], "0.7");
        editor.position = ["0", "0", "1", "1"].map(str::to_owned);
        assert!(matches!(
            click(&mut editor, &ctx, "Apply changes"),
            Action::None
        ));
        assert!(editor.error.as_ref().unwrap().contains("overlap"));
        assert!(!editor.saving);
        click(&mut editor, &ctx, "Reset position");
        assert_eq!(editor.position, editor.initial_position);
        assert!(editor.candidate().unwrap().same_snapshot(&source));
        editor.position = ["0.15", "0.1", "0.7", "0.3"].map(str::to_owned);
        let Action::Apply(edit) = click(&mut editor, &ctx, "Apply changes") else {
            panic!("Apply not dispatched")
        };
        assert_eq!(
            edit.updated.active_position().normalized_plot_box(),
            Some([0.15, 0.1, 0.7, 0.3])
        );
        assert_eq!(edit.updated.point_count(), 600);
        for (_, a) in edit.updated.all_axes() {
            let (_, old) = source
                .all_axes()
                .find(|(_, old)| old.title == a.title)
                .unwrap();
            assert!(a.same_snapshot(old));
        }
        editor.saving = false;
        click(&mut editor, &ctx, "Restore original");
        assert_eq!(editor.tab, Tab::Layout);
        assert!(editor.candidate().unwrap().same_snapshot(&source));
        assert!(matches!(click(&mut editor, &ctx, "Cancel"), Action::Cancel));
    }
    #[test]
    fn properties_widgets_apply_validate_restore_and_cancel_without_copying_samples() {
        let mut engine = matrust::Engine::default();
        engine.execute("plot([1 10 100],[2 3 4]);").unwrap();
        let source = engine.figure.unwrap();
        let ctx = egui::Context::default();
        let mut editor = Editor::new(1, &source, Some([1., 100., 0., 5.]));
        frame(&mut editor, &ctx, vec![]);
        ctx.memory_mut(|m| m.request_focus(egui::Id::new(("figure_property", "Title"))));
        frame(
            &mut editor,
            &ctx,
            vec![egui::Event::Text("Edited λ".into())],
        );
        assert_eq!(editor.draft.title, "Edited λ");
        editor.auto[0] = false;
        editor.limits[0] = ["100".into(), "1".into()];
        assert!(matches!(
            click(&mut editor, &ctx, "Apply changes"),
            Action::None
        ));
        assert!(editor.error.as_ref().unwrap().contains("increasing"));
        assert!(!editor.saving);
        editor.limits[0] = ["1".into(), "100".into()];
        let Action::Apply(edit) = click(&mut editor, &ctx, "Apply changes") else {
            panic!("Apply not dispatched")
        };
        assert!(editor.saving);
        assert_eq!(edit.updated.title, "Edited λ");
        assert_eq!(edit.updated.xlim, Some([1., 100.]));
        assert!((source.series[0].points).same_storage(&edit.updated.series[0].points));
        assert_eq!(source.title, "");
        editor.saving = false;
        click(&mut editor, &ctx, "Restore original");
        assert_eq!(editor.draft.title, "");
        assert!(editor.error.is_none());
        editor.show_series();
        click(&mut editor, &ctx, "Remove series");
        assert!(editor.draft.series.is_empty());
        assert!(matches!(click(&mut editor, &ctx, "Cancel"), Action::Cancel));
        assert_eq!(source.series.len(), 1);
    }
    #[test]
    fn bar_outline_checkbox_submits_metadata_only_edit_and_preserves_fill() {
        let mut engine = matrust::Engine::default();
        engine.execute("bar([1 2],[3 4]);").unwrap();
        let source = engine.figure.unwrap();
        let ctx = egui::Context::default();
        let mut editor = Editor::new(1, &source, None);
        editor.show_series();
        click(&mut editor, &ctx, "Outline");
        assert_eq!(editor.draft.series[0].face_edge_color, Some([0; 3]));
        assert_eq!(editor.draft.series[0].color, source.series[0].color);
        let Action::Apply(edit) = click(&mut editor, &ctx, "Apply changes") else {
            panic!("Outline edit not submitted");
        };
        assert!((edit.updated.series[0].points).same_storage(&source.series[0].points));
        assert_eq!(edit.updated.series[0].face_edge_color, Some([0; 3]));
        assert!(source.series[0].face_edge_color.is_none());
    }
    #[test]
    fn properties_reject_bad_limits_labels_and_oversize_bar_conversion() {
        let mut source = Figure::default();
        source
            .add(vec![matrust::plot::Series::new(
                vec![[1., 2.]; 10001],
                PlotKind::Line,
            )])
            .unwrap();
        let mut editor = Editor::new(1, &source, None);
        editor.auto[0] = false;
        for limits in [["bad", "2"], ["1", "inf"], ["NaN", "2"]] {
            editor.limits[0] = limits.map(str::to_owned);
            assert!(editor.candidate().is_err());
        }
        editor.limits[0] = ["-1".into(), "2".into()];
        editor.draft.xscale = AxisScale::Log;
        assert!(editor.candidate().is_err());
        editor.auto[0] = true;
        editor.draft.series[0].kind = PlotKind::Bar;
        assert!(editor.candidate().is_err());
        editor.draft.series[0].kind = PlotKind::Line;
        editor.draft.title = "λ".repeat(3000);
        assert!(editor.candidate().is_err());
        assert_eq!(source.title, "");
        assert_eq!(source.series[0].kind, PlotKind::Line);
    }
    #[test]
    fn appearance_controls_apply_or_cancel_at_compact_size_without_copying_samples() {
        use matrust::plot::{BaseTheme, ColorMode, FigureTheme};
        for theme in [egui::Theme::Light, egui::Theme::Dark] {
            let ctx = egui::Context::default();
            crate::theme::configure(&ctx);
            ctx.set_theme(theme);
            let mut engine = matrust::Engine::default();
            engine.execute("plot([0 1 2],[0 1 0]);").unwrap();
            let source = engine.figure.unwrap();
            let original = source.clone();
            let mut editor = Editor::new(1, &source, None);
            assert!(matches!(
                click(&mut editor, &ctx, "Appearance"),
                Action::None
            ));
            let (output, _) = frame(&mut editor, &ctx, vec![]);
            fn visible(shape: &egui::Shape, clip: egui::Rect, label: &str) -> bool {
                match shape {
                    egui::Shape::Text(t) if t.galley.text() == label => {
                        !t.galley.elided && clip.contains_rect(t.visual_bounding_rect())
                    }
                    egui::Shape::Vec(v) => v.iter().any(|s| visible(s, clip, label)),
                    _ => false,
                }
            }
            for label in [
                "Desktop",
                "Light",
                "Dark",
                "Automatic canvas background",
                "Apply changes",
                "Cancel",
            ] {
                assert!(
                    output
                        .shapes
                        .iter()
                        .any(|s| visible(&s.shape, s.clip_rect, label)),
                    "{label} must be fully visible at 960x650"
                );
            }
            assert_eq!(
                ctx.global_style().visuals.dark_mode,
                theme == egui::Theme::Dark
            );
            click(&mut editor, &ctx, "Dark");
            let dark_canvas = editor.draft.resolve_appearance(BaseTheme::Dark).canvas;
            click(&mut editor, &ctx, "Automatic canvas background");
            assert_eq!(
                editor.draft.appearance_policy().canvas_mode,
                ColorMode::Manual
            );
            assert_eq!(editor.draft.color, dark_canvas);
            click(&mut editor, &ctx, "Light");
            let Action::Apply(edit) = click(&mut editor, &ctx, "Apply changes") else {
                panic!("Actual Apply must return a typed edit")
            };
            assert_eq!(edit.updated.appearance_policy().theme, FigureTheme::Light);
            assert_eq!(
                edit.updated.color, dark_canvas,
                "Manual canvas survives a theme change"
            );
            assert!(edit.original.same_snapshot(&source));
            assert!(
                edit.updated.series[0]
                    .points
                    .same_storage(&source.series[0].points)
            );
            let mut cancelled = Editor::new(1, &source, None);
            click(&mut cancelled, &ctx, "Appearance");
            click(&mut cancelled, &ctx, "Dark");
            assert!(matches!(
                click(&mut cancelled, &ctx, "Cancel"),
                Action::Cancel
            ));
            assert!(source.same_snapshot(&original));
        }
    }
    #[test]
    fn automatic_series_swatch_is_resolved_without_materializing_rgb_and_apply_keeps_intent() {
        fn has_color(shape: &egui::Shape, wanted: egui::Color32) -> bool {
            match shape {
                egui::Shape::Rect(r) => r.fill == wanted,
                egui::Shape::Mesh(m) => m.vertices.iter().any(|v| v.color == wanted),
                egui::Shape::Vec(v) => v.iter().any(|s| has_color(s, wanted)),
                _ => false,
            }
        }
        let mut engine = matrust::Engine::default();
        engine.execute("plot([0 1],[1 2]);").unwrap();
        let source = engine.figure.unwrap();
        for (theme, rgb) in [
            (egui::Theme::Light, [17, 113, 190]),
            (egui::Theme::Dark, [86, 180, 233]),
        ] {
            let ctx = egui::Context::default();
            crate::theme::configure(&ctx);
            ctx.set_theme(theme);
            let mut editor = Editor::new(1, &source, None);
            click(&mut editor, &ctx, "Series");
            for _ in 0..3 {
                frame(&mut editor, &ctx, vec![]);
            }
            let (output, _) = frame(&mut editor, &ctx, vec![]);
            let wanted = egui::Color32::from_rgb(rgb[0], rgb[1], rgb[2]);
            assert!(
                output.shapes.iter().any(|s| has_color(&s.shape, wanted)),
                "actual resolved swatch {theme:?}"
            );
            assert!(
                editor.draft.same_snapshot(&source),
                "painting must not capture display RGB into the draft"
            );
            click(&mut editor, &ctx, "Appearance");
            click(&mut editor, &ctx, "Dark");
            let Action::Apply(edit) = click(&mut editor, &ctx, "Apply changes") else {
                panic!("typed Apply")
            };
            assert_eq!(
                edit.updated.series[0].auto_palette,
                source.series[0].auto_palette
            );
            assert_eq!(edit.updated.series[0].color, source.series[0].color);
            assert!(!edit.updated.series[0].explicit_color);
            assert!(
                edit.updated.series[0]
                    .points
                    .same_storage(&source.series[0].points)
            );
        }
    }
}

#[cfg(test)]
#[path = "figure_property_layout_tests.rs"]
mod layout_tests;
