use super::*;
use crate::theme::ThemedScrollArea;
use matrust::{Array, plot::PlotKind};
use plot_request::Request;

pub struct Dialog {
    pub y: String,
    pub x: Option<String>,
    pub kind: PlotKind,
    pub log_x: bool,
    pub log_y: bool,
    pub creating: bool,
    pub error: Option<String>,
    y_filter: String,
    x_filter: String,
}
impl Dialog {
    pub fn new(y: String) -> Self {
        Self {
            y,
            x: None,
            kind: PlotKind::Line,
            log_x: false,
            log_y: false,
            creating: false,
            error: None,
            y_filter: String::new(),
            x_filter: String::new(),
        }
    }
    fn request(&self, variables: &BTreeMap<String, Array>) -> matrust::Result<Request> {
        let get = |n: &String| {
            variables
                .get(n)
                .cloned()
                .map(|v| (n.clone(), v))
                .ok_or_else(|| format!("Variable {n} is no longer available"))
        };
        let request = Request {
            y: get(&self.y)?,
            x: self.x.as_ref().map(get).transpose()?,
            kind: self.kind,
            log_x: self.log_x,
            log_y: self.log_y,
        };
        request.validate()?;
        Ok(request)
    }
}

fn selector(
    ui: &mut egui::Ui,
    id: &str,
    current: &mut Option<String>,
    variables: &BTreeMap<String, Array>,
    optional: bool,
    filter: &mut String,
    automatic_label: &'static str,
) -> bool {
    let before = current.clone();
    egui::ComboBox::from_id_salt(id)
        .close_behavior(egui::PopupCloseBehavior::CloseOnClickOutside)
        .selected_text(current.as_deref().unwrap_or(automatic_label))
        .width((ui.ctx().content_rect().width() - 160.).clamp(150., 320.))
        .truncate()
        .show_ui(ui, |ui| {
            ui.style_mut().wrap_mode = Some(egui::TextWrapMode::Truncate);
            ui.horizontal(|ui| {
                ui.add(
                    egui::TextEdit::singleline(filter)
                        .hint_text("Find name or type")
                        .char_limit(128)
                        .desired_width((ui.available_width() - 48.).max(80.)),
                );
                if ui.button("Clear").clicked() {
                    filter.clear();
                }
            });
            if optional
                && ui
                    .selectable_value(current, None, automatic_label)
                    .clicked()
            {
                ui.close();
            }
            // Only create row widgets for visible variables, even in a large workspace.
            let query = filter.trim();
            let rows: Vec<_> = variables
                .iter()
                .filter(|(name, data)| {
                    matches_query(name, query) || matches_query(plot_type_label(data), query)
                })
                .collect();
            ui.weak(format!("{} of {} variables", rows.len(), variables.len()));
            if rows.is_empty() {
                ui.label("No matching variables");
            }
            let height = ui.spacing().interact_size.y.max(
                ui.text_style_height(&egui::TextStyle::Body) + 2. * ui.spacing().button_padding.y,
            );
            egui::ScrollArea::vertical()
                .max_height(180.)
                .show_rows_themed(ui, height, rows.len(), |ui, range| {
                    for &(name, data) in &rows[range] {
                        let row = ui
                            .selectable_value(
                                current,
                                Some(name.clone()),
                                format!(
                                    "{name}  ({} x {}, {})",
                                    data.rows(),
                                    data.cols(),
                                    plot_type_label(data)
                                ),
                            )
                            .on_hover_text(format!(
                                "{name}: {} x {} {}",
                                data.rows(),
                                data.cols(),
                                plot_type_label(data)
                            ));
                        if row.clicked() {
                            ui.close();
                        }
                    }
                });
        });
    before != *current
}
fn plot_type_label(data: &Array) -> &'static str {
    if data.is_complex() {
        "complex double"
    } else {
        data.class()
    }
}
fn matches_query(text: &str, query: &str) -> bool {
    query.is_empty()
        || text
            .as_bytes()
            .windows(query.len())
            .any(|part| part.eq_ignore_ascii_case(query.as_bytes()))
}

fn kind_label(kind: PlotKind) -> &'static str {
    match kind {
        PlotKind::Line => "Line",
        PlotKind::Scatter => "Scatter",
        PlotKind::Bar => "Bar",
        PlotKind::Stairs => "Stairs",
        PlotKind::Stem => "Stem",
        PlotKind::Patch => "Patch",
    }
}
impl Workspace {
    pub(super) fn begin_plot_data(&mut self) {
        if self.busy || self.variables.is_empty() {
            return;
        }
        let y = self
            .selected
            .as_ref()
            .filter(|n| self.variables.contains_key(*n))
            .cloned()
            .or_else(|| self.variables.keys().next().cloned())
            .unwrap();
        self.plot_data = Some(Dialog::new(y));
    }
    pub(super) fn plot_data_ui(&mut self, ctx: &egui::Context) {
        let Some(dialog) = &mut self.plot_data else {
            return;
        };
        let mut create = false;
        let mut script = false;
        let mut close = false;
        let viewport = ctx.content_rect();
        let shown = egui::Modal::new(egui::Id::new("plot_workspace_data")).show(ctx, |ui| {
            ui.set_width((viewport.width() - 48.).clamp(240., 430.));
            ui.label(crate::tokens::title(ui.ctx(), "Plot workspace data"));
            ui.label("Create a new figure from workspace variables.");
            egui::ScrollArea::vertical()
                .id_salt("plot_data_options")
                .min_scrolled_height(0.)
                .max_height((viewport.height() - 220.).clamp(50., 450.))
                .show_themed(ui, |ui| {
                    ui.add_enabled_ui(!dialog.creating, |ui| {
                        egui::Grid::new("plot_data_fields")
                            .spacing([15., 12.])
                            .show(ui, |ui| {
                                ui.label("Y data");
                                let mut selected = Some(dialog.y.clone());
                                if selector(
                                    ui,
                                    "plot_y",
                                    &mut selected,
                                    &self.variables,
                                    false,
                                    &mut dialog.y_filter,
                                    "Sample index",
                                ) {
                                    dialog.y = selected.unwrap();
                                    dialog.error = None;
                                }
                                ui.end_row();
                                ui.label("X data");
                                if selector(
                                    ui,
                                    "plot_x",
                                    &mut dialog.x,
                                    &self.variables,
                                    true,
                                    &mut dialog.x_filter,
                                    if dialog.kind == PlotKind::Line
                                        && self
                                            .variables
                                            .get(&dialog.y)
                                            .is_some_and(Array::is_complex)
                                    {
                                        "Real part of Y"
                                    } else {
                                        "Sample index"
                                    },
                                ) {
                                    dialog.error = None;
                                }
                                ui.end_row();
                                ui.label("Plot type");
                                egui::ComboBox::from_id_salt("plot_kind")
                                    .selected_text(kind_label(dialog.kind))
                                    .show_ui(ui, |ui| {
                                        for kind in [
                                            PlotKind::Line,
                                            PlotKind::Scatter,
                                            PlotKind::Bar,
                                            PlotKind::Stairs,
                                            PlotKind::Stem,
                                        ] {
                                            if ui
                                                .selectable_value(
                                                    &mut dialog.kind,
                                                    kind,
                                                    kind_label(kind),
                                                )
                                                .changed()
                                            {
                                                dialog.error = None;
                                            }
                                        }
                                    });
                                ui.end_row();
                            });
                        ui.horizontal_wrapped(|ui| {
                            if ui.checkbox(&mut dialog.log_x, "Logarithmic X").changed()
                                | ui.checkbox(&mut dialog.log_y, "Logarithmic Y").changed()
                            {
                                dialog.error = None;
                            }
                        });
                    });
                    if dialog.kind == PlotKind::Line {
                        let complex_y =
                            self.variables.get(&dialog.y).is_some_and(Array::is_complex);
                        let complex_x = dialog
                            .x
                            .as_ref()
                            .and_then(|x| self.variables.get(x))
                            .is_some_and(Array::is_complex);
                        if dialog.x.is_none() && complex_y {
                            ui.weak("Complex Y: imaginary versus real components.");
                        } else if dialog.x.is_some() && (complex_x || complex_y) {
                            ui.weak("Explicit X/Y: real components; imaginary parts are ignored.");
                        }
                    }
                    ui.weak(
                        "Matrices create multiple series. X and Y must have compatible dimensions.",
                    );
                    ui.weak("Up to 64 series and 1,000,000 points; bar plots up to 10,000 bars.");
                });
            let validation = dialog.request(&self.variables).err();
            if let Some(error) = dialog.error.as_ref().or(validation.as_ref()) {
                egui::ScrollArea::vertical()
                    .id_salt("plot_data_error")
                    .min_scrolled_height(0.)
                    .max_height(48.)
                    .show_themed(ui, |ui| {
                        ui.colored_label(tokens::palette(ui.ctx()).error, error);
                    });
            }
            ui.separator();
            ui.horizontal_wrapped(|ui| {
                if dialog.creating {
                    ui.spinner();
                    ui.label("Creating plot...");
                    if ui.button("Stop").clicked() {
                        self.worker.cancel.store(true, Ordering::Relaxed);
                    }
                } else {
                    create = ui
                        .add_enabled(validation.is_none(), egui::Button::new("Create plot"))
                        .clicked();
                    script = ui
                        .add_enabled(validation.is_none(), egui::Button::new("Open plot script"))
                        .clicked();
                    close = ui.button("Cancel").clicked();
                }
            });
        });
        if shown.is_top_modal
            && !shown.any_popup_open
            && ctx.input_mut(|i| i.consume_key(egui::Modifiers::NONE, egui::Key::Escape))
        {
            if dialog.creating {
                self.worker.cancel.store(true, Ordering::Relaxed);
            } else {
                close = true;
            }
        }
        if close {
            self.plot_data = None;
        } else if create || script {
            let request = dialog.request(&self.variables);
            match request {
                Ok(request) if script => match request.script() {
                    Ok(source) => {
                        if self.documents.len() >= matrust::session::MAX_DOCUMENTS {
                            dialog.error =
                                Some("Close a script tab before opening the plot script.".into());
                        } else {
                            self.plot_data = None;
                            self.new_document(&source);
                        }
                    }
                    Err(error) => dialog.error = Some(error),
                },
                Ok(request) => {
                    dialog.creating = true;
                    dialog.error = None;
                    self.submit(Job::PlotData(Box::new(request)));
                    if !self.busy
                        && let Some(dialog) = &mut self.plot_data
                    {
                        dialog.creating = false;
                        dialog.error = Some(self.status.clone());
                    }
                }
                Err(error) => dialog.error = Some(error),
            }
        }
    }
}

#[cfg(test)]
#[path = "plot_data_layout_tests.rs"]
mod layout_tests;
