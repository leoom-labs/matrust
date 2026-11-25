//! Filter Designer work area: three labelled areas, SPECS | ANALYSIS | OUTPUT.
//! Every field row has a fixed height with a reserved message line, so a
//! validation message never moves the controls below it.
use super::designer_preview::{Preview, View};
use super::*;
use crate::filter_spec::{
    Field, KAISER, MaskBand, Method, RESPONSES, Spec, Structure, Units, WINDOWS,
};
use crate::panels::PaneBorder;
use crate::response_plot::Series;

const SPECS_WIDTH: f32 = 320.;
const OUTPUT_WIDTH: f32 = 248.;
const LABEL_WIDTH: f32 = 112.;
/// Message line under each field (its text or empty space).
const MESSAGE_LINE: f32 = 16.;
const MASK_SIZE: egui::Vec2 = egui::vec2(272., 96.);

impl Toolkits {
    pub(super) fn designer_ui(
        &mut self,
        ui: &mut egui::Ui,
        busy: bool,
        interactive: bool,
        action: &mut Option<Action>,
    ) {
        let ctx = ui.ctx().clone();
        let p = crate::tokens::palette(&ctx);
        self.preview.refresh(&ctx, &self.spec);
        let compact = ctx.content_rect().width() < 1100.;
        let frame = egui::Frame::new().fill(p.surface_sunken).inner_margin(16);
        egui::Panel::left("designer_specs")
            .exact_size(SPECS_WIDTH + 32.)
            .resizable(false)
            .frame(frame)
            .show_pane(ui, |ui| {
                egui::ScrollArea::vertical()
                    .id_salt("designer_specs_scroll")
                    .auto_shrink([false, false])
                    .show_themed(ui, |ui| self.specs_area(ui));
            });
        if compact {
            // Narrow: OUTPUT follows ANALYSIS in one scrolling column.
            egui::CentralPanel::default().frame(frame).show(ui, |ui| {
                egui::ScrollArea::vertical()
                    .id_salt("designer_compact")
                    .auto_shrink([false, false])
                    .show_themed(ui, |ui| {
                        self.analysis_area(ui, 240.);
                        ui.add_space(crate::tokens::L);
                        self.output_area(ui, busy, interactive, action);
                    });
            });
            return;
        }
        egui::Panel::right("designer_output")
            .exact_size(OUTPUT_WIDTH + 32.)
            .resizable(false)
            .frame(frame)
            .show_pane(ui, |ui| {
                egui::ScrollArea::vertical()
                    .id_salt("designer_output_scroll")
                    .auto_shrink([false, false])
                    .show_themed(ui, |ui| self.output_area(ui, busy, interactive, action));
            });
        egui::CentralPanel::default().frame(frame).show(ui, |ui| {
            let height = (ui.available_height() - 88.).max(240.);
            self.analysis_area(ui, height);
        });
    }

    fn specs_area(&mut self, ui: &mut egui::Ui) {
        caption(ui, "SPECS");
        ui.spacing_mut().item_spacing.y = 0.;
        let problems = self.spec.problems();
        let message = |field: Field| {
            problems
                .iter()
                .find(|(f, _)| *f == field)
                .map(|(_, m)| m.as_str())
        };
        let designed_order = self.preview.preview().map(|p| p.order);
        let spec = &mut self.spec;
        row(ui, "Response", None, |ui| {
            let mut response = spec.response;
            egui::ComboBox::from_id_salt("designer_response")
                .width(ui.available_width())
                .selected_text(response.label())
                .show_ui(ui, |ui| {
                    for r in RESPONSES {
                        ui.selectable_value(&mut response, r, r.label());
                    }
                });
            spec.set_response(response);
        });
        let (has_iir, has_fir) = spec.families();
        if has_iir && has_fir {
            row(ui, "Filter type", None, |ui| {
                if crate::tokens::tab(ui, spec.method.iir(), "IIR").clicked() {
                    spec.set_family(true);
                }
                if crate::tokens::tab(ui, !spec.method.iir(), "FIR").clicked() {
                    spec.set_family(false);
                }
            });
        }
        row(ui, "Method", None, |ui| {
            let mut method = spec.method;
            egui::ComboBox::from_id_salt("designer_method")
                .width(ui.available_width())
                .selected_text(method.label())
                .show_ui(ui, |ui| {
                    let family = spec.method.iir();
                    for m in spec.response.methods().iter().filter(|m| m.iir() == family) {
                        ui.selectable_value(
                            &mut method,
                            *m,
                            format!("{} ({})", m.label(), m.function()),
                        );
                    }
                });
            spec.set_method(method);
        });
        if spec.method != Method::NotchIir {
            row(ui, "Order", message(Field::Order), |ui| {
                if spec.minimum_available() {
                    if crate::tokens::tab(ui, !spec.minimum, "Specify").clicked() {
                        spec.minimum = false;
                    }
                    if crate::tokens::tab(ui, spec.minimum, "Minimum").clicked() {
                        spec.minimum = true;
                    }
                }
                if !spec.uses_minimum() {
                    ui.add(
                        egui::DragValue::new(&mut spec.order)
                            .range(1..=4096)
                            .speed(0.2),
                    );
                } else if let Some(order) = designed_order {
                    let p = crate::tokens::palette(ui.ctx());
                    ui.label(egui::RichText::new(format!("= {order}")).color(p.text_muted));
                }
            });
        }
        row(ui, "Units", None, |ui| {
            if crate::tokens::tab(ui, spec.units == Units::Hz, "Hz").clicked() {
                spec.set_units(Units::Hz);
            }
            if crate::tokens::tab(ui, spec.units == Units::Normalized, "Normalized").clicked() {
                spec.set_units(Units::Normalized);
            }
        });
        let unit = if spec.units == Units::Hz {
            "Hz"
        } else {
            "×π"
        };
        for field in spec.fields() {
            let text = message(field);
            match field {
                Field::Order | Field::Structure => {}
                Field::Fs => row(ui, "Fs (Hz)", text, |ui| number(ui, &mut spec.fs)),
                Field::Fc => edges(
                    ui,
                    spec.response.two_edges(),
                    "Cutoff",
                    unit,
                    &mut spec.fc,
                    text,
                ),
                Field::Fpass => edges(
                    ui,
                    spec.response.two_edges(),
                    "Fpass",
                    unit,
                    &mut spec.fpass,
                    text,
                ),
                Field::Fstop => edges(
                    ui,
                    spec.response.two_edges(),
                    "Fstop",
                    unit,
                    &mut spec.fstop,
                    text,
                ),
                Field::Apass => row(ui, "Apass (dB)", text, |ui| number(ui, &mut spec.apass)),
                Field::Astop => row(ui, "Astop (dB)", text, |ui| number(ui, &mut spec.astop)),
                Field::Wpass => row(ui, "Wpass", text, |ui| number(ui, &mut spec.wpass)),
                Field::Wstop => row(ui, "Wstop", text, |ui| number(ui, &mut spec.wstop)),
                Field::Window => row(ui, "Window", text, |ui| {
                    egui::ComboBox::from_id_salt("designer_window")
                        .width(ui.available_width())
                        .selected_text(WINDOWS[spec.window].0)
                        .show_ui(ui, |ui| {
                            for (i, (label, _)) in WINDOWS.iter().enumerate() {
                                ui.selectable_value(&mut spec.window, i, *label);
                            }
                        });
                }),
                Field::Beta if spec.window == KAISER => {
                    row(ui, "Kaiser β", text, |ui| number(ui, &mut spec.beta))
                }
                Field::Beta => {}
                Field::F0 => row(ui, &format!("Notch at ({unit})"), text, |ui| {
                    number(ui, &mut spec.f0)
                }),
                Field::Bandwidth => row(ui, &format!("Bandwidth ({unit})"), text, |ui| {
                    number(ui, &mut spec.bandwidth)
                }),
                Field::Freqs => row(ui, &format!("Frequencies ({unit})"), text, |ui| {
                    vector(ui, &mut spec.freqs)
                }),
                Field::Amps => row(ui, "Amplitudes", text, |ui| vector(ui, &mut spec.amps)),
                Field::Weights => row(ui, "Weights", text, |ui| vector(ui, &mut spec.weights)),
            }
        }
        if spec.method.iir() {
            row(ui, "Structure", None, |ui| {
                if crate::tokens::tab(ui, spec.structure == Structure::Sos, "Sections").clicked() {
                    spec.structure = Structure::Sos;
                }
                if crate::tokens::tab(ui, spec.structure == Structure::Direct, "Direct form")
                    .clicked()
                {
                    spec.structure = Structure::Direct;
                }
            });
        }
        ui.add_space(crate::tokens::S);
        mask_picture(ui, spec);
    }

    fn analysis_area(&mut self, ui: &mut egui::Ui, height: f32) {
        caption(ui, "ANALYSIS");
        ui.horizontal_wrapped(|ui| {
            ui.spacing_mut().item_spacing = egui::vec2(crate::tokens::XS, crate::tokens::XS);
            for view in View::ALL {
                let tab = crate::tokens::tab(ui, self.view == view, view.label());
                if view == View::Magnitude {
                    crate::focus_areas::register(ui, &tab, crate::focus_areas::Area::Designer);
                }
                if tab.clicked() {
                    self.view = view;
                }
            }
        });
        ui.add_space(crate::tokens::S);
        let p = crate::tokens::palette(ui.ctx());
        let size = egui::vec2(ui.available_width(), height);
        match self.preview.result() {
            Some(Ok(preview)) => {
                ui.label(egui::RichText::new(&preview.title).color(p.text_muted));
                ui.allocate_ui(size, |ui| {
                    ui.set_min_size(size);
                    analysis_view(ui, self.view, preview, height - 24.);
                });
            }
            Some(Err(error)) => {
                ui.allocate_ui(size, |ui| {
                    ui.set_min_size(size);
                    ui.colored_label(p.error, format!("No filter: {error}"));
                });
            }
            None => {
                ui.allocate_ui(size, |ui| {
                    ui.set_min_size(size);
                    ui.label("Designing…");
                });
            }
        }
    }

    fn output_area(
        &mut self,
        ui: &mut egui::Ui,
        busy: bool,
        interactive: bool,
        action: &mut Option<Action>,
    ) {
        caption(ui, "OUTPUT");
        let p = crate::tokens::palette(ui.ctx());
        let script = self.spec.script();
        let ready = script.is_ok() && !busy;
        let wide = egui::vec2(ui.available_width(), crate::tokens::CONTROL);
        let [first, second] = self.spec.outputs();
        ui.label(egui::RichText::new("Workspace").strong());
        if ui
            .add_enabled(
                ready,
                egui::Button::new(format!("Export {first}, {second}")).min_size(wide),
            )
            .clicked()
            && let Ok(script) = &script
        {
            *action = Some(Action::Script(script.clone()));
        }
        let digital = self.spec.designfilt();
        if ui
            .add_enabled(
                ready && digital.is_some(),
                egui::Button::new("Export digitalFilter d").min_size(wide),
            )
            .on_disabled_hover_text(
                "designfilt covers Window (Hamming), Butterworth and Chebyshev II at a set order",
            )
            .clicked()
            && let Some(digital) = digital
        {
            *action = Some(Action::Script(digital));
        }
        ui.add_space(crate::tokens::M);
        ui.label(egui::RichText::new("Apply to a signal").strong());
        ui.horizontal(|ui| {
            ui.label("Signal");
            ui.add(
                crate::tokens::input(egui::TextEdit::singleline(&mut self.input))
                    .desired_width(ui.available_width()),
            );
        });
        let valid_input = matrust::files::valid_name(&self.input);
        ui.horizontal(|ui| {
            let half = egui::vec2((wide.x - crate::tokens::S) / 2., wide.y);
            for (label, zero_phase) in [("filter", false), ("filtfilt", true)] {
                if ui
                    .add_enabled(
                        ready && valid_input,
                        egui::Button::new(label).min_size(half),
                    )
                    .clicked()
                    && let Ok(script) = &script
                {
                    *action = Some(Action::Apply(self.spec.apply_script(
                        script,
                        &self.input,
                        zero_phase,
                    )));
                }
            }
        });
        ui.label(
            egui::RichText::new(if valid_input {
                "Result in y, plotted"
            } else {
                "Signal must be a workspace variable name"
            })
            .size(crate::tokens::CAPTION)
            .color(if valid_input { p.text_muted } else { p.error }),
        );
        ui.add_space(crate::tokens::M);
        ui.label(egui::RichText::new("Script").strong());
        if ui
            .add_enabled(
                script.is_ok(),
                egui::Button::new("Copy script").min_size(wide),
            )
            .clicked()
            && let Ok(script) = &script
        {
            ui.ctx().copy_text(script.clone());
            self.message = "Script copied".into();
        }
        if ui
            .add_enabled(
                ready && !self.saving_design,
                egui::Button::new("Save script…").min_size(wide),
            )
            .clicked()
        {
            *action = Some(Action::ChooseDesignSave(self.spec.clone()));
        }
        ui.add_space(crate::tokens::M);
        ui.label(egui::RichText::new("Block model").strong());
        let block = self
            .preview
            .preview()
            .map(|p| crate::designer_output::block(&p.coefficients));
        if ui
            .add_enabled(
                !busy && matches!(block, Some(Ok(_))),
                egui::Button::new("Add to block model").min_size(wide),
            )
            .clicked()
            && interactive
            && let Some(Ok(block)) = block
        {
            *action = Some(Action::FilterBlock(block, self.spec.title()));
        }
        if self.saving_design
            && ui
                .add(egui::Button::new("Stop saving").min_size(wide))
                .clicked()
        {
            *action = Some(Action::StopModel);
        }
        if !self.message.is_empty() {
            ui.add_space(crate::tokens::S);
            ui.label(egui::RichText::new(&self.message).color(p.text_muted));
        }
    }

    /// The Apps action row shows only the designer's status message: every
    /// command lives in the OUTPUT area.
    pub(super) fn designer_commands(
        &mut self,
        ui: &mut egui::Ui,
        _busy: bool,
        _node_edit_interactive: bool,
        _action: &mut Option<Action>,
    ) {
        // The design title is in the status bar and above the plot.
        let p = crate::tokens::palette(ui.ctx());
        ui.add(egui::Label::new(egui::RichText::new(&self.message).color(p.text_muted)).truncate());
    }
}

fn caption(ui: &mut egui::Ui, text: &str) {
    let p = crate::tokens::palette(ui.ctx());
    ui.label(
        egui::RichText::new(text)
            .font(crate::panels::caption_semibold(ui))
            .color(p.text_muted),
    );
    ui.add_space(crate::tokens::S);
}

/// One fixed-height field row: label, control, then its message line.
fn row(ui: &mut egui::Ui, label: &str, message: Option<&str>, add: impl FnOnce(&mut egui::Ui)) {
    let p = crate::tokens::palette(ui.ctx());
    let width = ui.available_width().min(SPECS_WIDTH);
    ui.allocate_ui_with_layout(
        egui::vec2(width, crate::tokens::CONTROL),
        egui::Layout::left_to_right(egui::Align::Center),
        |ui| {
            ui.set_min_size(egui::vec2(width, crate::tokens::CONTROL));
            ui.spacing_mut().item_spacing.x = crate::tokens::XS;
            ui.allocate_ui_with_layout(
                egui::vec2(LABEL_WIDTH, crate::tokens::CONTROL),
                egui::Layout::left_to_right(egui::Align::Center),
                |ui| {
                    ui.set_min_width(LABEL_WIDTH);
                    ui.add(egui::Label::new(label).truncate());
                },
            );
            add(ui);
        },
    );
    let (rect, _) = ui.allocate_exact_size(egui::vec2(width, MESSAGE_LINE), egui::Sense::hover());
    if let Some(message) = message {
        let galley = ui.painter().layout(
            message.to_owned(),
            egui::FontId::proportional(crate::tokens::CAPTION),
            p.error,
            width - LABEL_WIDTH,
        );
        // A long message is clipped to its line rather than pushing rows down.
        ui.painter().with_clip_rect(rect).galley(
            rect.min + egui::vec2(LABEL_WIDTH, 0.),
            galley,
            p.error,
        );
        ui.interact(
            rect,
            ui.id().with(("field_message", label)),
            egui::Sense::hover(),
        )
        .on_hover_text(message);
    }
}

fn number(ui: &mut egui::Ui, value: &mut f64) {
    ui.add_sized(
        egui::vec2(ui.available_width(), crate::tokens::CONTROL - 4.),
        egui::DragValue::new(value).speed(0.1).max_decimals(6),
    );
}

fn vector(ui: &mut egui::Ui, text: &mut String) {
    ui.add(
        crate::tokens::input(egui::TextEdit::singleline(text))
            .hint_text("e.g. 0 0.3 0.5 1")
            .desired_width(ui.available_width()),
    );
}

fn edges(
    ui: &mut egui::Ui,
    two: bool,
    name: &str,
    unit: &str,
    values: &mut [f64; 2],
    message: Option<&str>,
) {
    row(ui, &format!("{name} ({unit})"), message, |ui| {
        let width = if two {
            (ui.available_width() - crate::tokens::XS) / 2.
        } else {
            ui.available_width()
        };
        for value in values.iter_mut().take(if two { 2 } else { 1 }) {
            ui.add_sized(
                egui::vec2(width, crate::tokens::CONTROL - 4.),
                egui::DragValue::new(value).speed(0.1).max_decimals(6),
            );
        }
    });
}

/// Small static picture of the spec: passbands as a band under 0 dB,
/// stopbands as blocks below −Astop, with the edge names.
fn mask_picture(ui: &mut egui::Ui, spec: &Spec) {
    let p = crate::tokens::palette(ui.ctx());
    let mask = spec.mask();
    let (rect, _) = ui.allocate_exact_size(MASK_SIZE, egui::Sense::hover());
    let painter = ui.painter_at(rect);
    painter.rect_stroke(
        rect,
        0.,
        egui::Stroke::new(1., p.hairline),
        egui::StrokeKind::Inside,
    );
    if mask.is_empty() {
        painter.text(
            rect.center(),
            egui::Align2::CENTER_CENTER,
            "No spec mask for this response",
            egui::FontId::proportional(crate::tokens::CAPTION),
            p.text_muted,
        );
        return;
    }
    let inner = rect.shrink2(egui::vec2(8., 12.));
    let x = |f: f64| inner.left() + inner.width() * f as f32;
    let top = inner.top() + 8.;
    let bottom = inner.bottom() - 12.;
    let font = egui::FontId::proportional(crate::tokens::CAPTION);
    for MaskBand { f0, f1, pass, .. } in &mask {
        let (x0, x1) = (x(*f0), x(*f1));
        if *pass {
            let band = egui::Rect::from_x_y_ranges(x0..=x1, top..=top + 12.);
            painter.rect_filled(band, 0., p.accent_soft);
            painter.line_segment(
                [band.left_top(), band.right_top()],
                egui::Stroke::new(1., p.text),
            );
        } else {
            let block = egui::Rect::from_x_y_ranges(x0..=x1, (bottom - 32.)..=bottom);
            painter.rect_filled(block, 0., p.accent_soft);
            painter.line_segment(
                [block.left_top(), block.right_top()],
                egui::Stroke::new(1., p.text),
            );
        }
    }
    // Edge labels under the axis.
    painter.line_segment(
        [
            egui::pos2(inner.left(), bottom),
            egui::pos2(inner.right(), bottom),
        ],
        egui::Stroke::new(1., p.text_muted),
    );
    // One edge name per band: at its inner edge, or centred when both of its
    // edges are inside, so a close pair of edges never prints on top of itself.
    let label = |pos: egui::Pos2, align: egui::Align2, text: &str| {
        painter.text(pos, align, text, font.clone(), p.text_muted);
    };
    for band in &mask {
        let name = if band.pass { "Fpass" } else { "Fstop" };
        let y = bottom + 2.;
        match (band.f0 > 0., band.f1 < 1.) {
            (true, true) => label(
                egui::pos2(x((band.f0 + band.f1) / 2.), y),
                egui::Align2::CENTER_TOP,
                name,
            ),
            (true, false) => label(egui::pos2(x(band.f0), y), egui::Align2::LEFT_TOP, name),
            (false, true) => label(egui::pos2(x(band.f1), y), egui::Align2::RIGHT_TOP, name),
            (false, false) => {}
        }
    }
    let fields = spec.fields();
    // Ripple and attenuation names sit inside their first band.
    if fields.contains(&Field::Apass)
        && let Some(band) = mask.iter().find(|b| b.pass)
    {
        label(
            egui::pos2(x(band.f0) + 2., top + 14.),
            egui::Align2::LEFT_TOP,
            "Apass",
        );
    }
    if fields.contains(&Field::Astop)
        && let Some(band) = mask.iter().find(|b| !b.pass)
    {
        label(
            egui::pos2(x(band.f0) + 2., bottom - 46.),
            egui::Align2::LEFT_TOP,
            "Astop",
        );
    }
}

fn trace_color() -> egui::Color32 {
    crate::response_plot::series_color(0)
}

fn plot(id: &str, height: f32, x: &str, y: &str) -> egui_plot::Plot<'static> {
    crate::response_plot::plot(("designer_plot", id.to_owned()), height, x, y)
}

fn curve(
    ui: &mut egui::Ui,
    id: &str,
    height: f32,
    x_label: &str,
    y_label: &str,
    x: &[f64],
    y: &[f64],
) {
    plot(id, height, x_label, y_label).show(ui, |plot_ui| {
        crate::response_plot::curves(
            plot_ui,
            &[Series {
                name: y_label,
                color: trace_color(),
                x,
                y,
            }],
        );
    });
}

fn magnitude(ui: &mut egui::Ui, preview: &Preview, height: f32) {
    let p = crate::tokens::palette(ui.ctx());
    let deepest = preview
        .mask
        .iter()
        .filter_map(|b| (!b.pass).then_some(b.db).flatten())
        .fold(80., f64::max);
    let floor = -deepest - 40.;
    let y: Vec<f64> = preview.magnitude_db.iter().map(|v| v.max(floor)).collect();
    let scale = preview.freq_scale;
    let mask_color = p.warning;
    plot("magnitude", height, preview.freq_label, "Magnitude (dB)").show(ui, |plot_ui| {
        for band in &preview.mask {
            let (x0, x1) = (band.f0 * scale, band.f1 * scale);
            let lines: Vec<f64> = match (band.pass, band.db) {
                (true, Some(apass)) => vec![0., -apass],
                (true, None) => vec![0.],
                (false, Some(astop)) => vec![-astop],
                (false, None) => vec![],
            };
            for level in lines {
                plot_ui.line(
                    egui_plot::Line::new("Spec mask", vec![[x0, level], [x1, level]])
                        .color(mask_color)
                        .style(egui_plot::LineStyle::dashed_dense())
                        .width(1.),
                );
            }
        }
        crate::response_plot::curves(
            plot_ui,
            &[Series {
                name: "Magnitude",
                color: trace_color(),
                x: &preview.freq,
                y: &y,
            }],
        );
    });
}

fn stems(ui: &mut egui::Ui, id: &str, height: f32, values: &[f64]) {
    let samples: Vec<f64> = (0..values.len()).map(|n| n as f64).collect();
    plot(id, height, "Samples", "Amplitude").show(ui, |plot_ui| {
        crate::response_plot::stems(
            plot_ui,
            &[Series {
                name: id,
                color: trace_color(),
                x: &samples,
                y: values,
            }],
        );
    });
}

fn pole_zero(ui: &mut egui::Ui, preview: &Preview, height: f32) {
    let p = crate::tokens::palette(ui.ctx());
    plot("pole_zero", height, "Real part", "Imaginary part")
        .data_aspect(1.)
        .show(ui, |plot_ui| {
            crate::response_plot::pole_zero(
                plot_ui,
                p.text_muted,
                &[crate::response_plot::Roots {
                    zeros_name: "Zeros",
                    poles_name: "Poles",
                    color: trace_color(),
                    zeros: &preview.zeros,
                    poles: &preview.poles,
                    marks: &[],
                }],
            );
        });
}

pub(super) fn coefficient_text(preview: &Preview) -> String {
    use crate::filter_spec::{Coefficients, format_vector};
    match &preview.coefficients {
        Coefficients::Tf { b, a } => {
            format!("b = [{}]\na = [{}]", format_vector(b), format_vector(a))
        }
        Coefficients::Sos { sos, g } => {
            let rows: Vec<String> = sos
                .iter()
                .map(|r| format!("  {}", format_vector(r)))
                .collect();
            format!(
                "sos = [\n{}\n]\ng = {}",
                rows.join("\n"),
                crate::filter_spec::fmt(*g)
            )
        }
    }
}

pub(super) fn info_rows(preview: &Preview) -> Vec<(&'static str, String)> {
    use crate::filter_spec::Coefficients;
    let mut rows = vec![
        ("Design", preview.title.clone()),
        ("Order", preview.order.to_string()),
        (
            "Stable",
            if preview.stable { "Yes" } else { "No" }.to_owned(),
        ),
        ("Structure", preview.structure.to_owned()),
    ];
    match &preview.coefficients {
        Coefficients::Sos { sos, .. } => rows.push(("Sections", sos.len().to_string())),
        Coefficients::Tf { b, a } => {
            rows.push(("Sections", "—".into()));
            let fir = a.iter().skip(1).all(|v| *v == 0.);
            rows.push(("Type", if fir { "FIR" } else { "IIR" }.into()));
            rows.push((
                "Coefficients",
                format!("{} numerator, {} denominator", b.len(), a.len()),
            ));
        }
    }
    rows
}

fn analysis_view(ui: &mut egui::Ui, view: View, preview: &Preview, height: f32) {
    let x = preview.freq_label;
    match view {
        View::Magnitude => magnitude(ui, preview, height),
        View::Phase => curve(
            ui,
            "phase",
            height,
            x,
            "Phase (degrees)",
            &preview.freq,
            &preview.phase,
        ),
        View::MagnitudePhase => {
            let half = (height - crate::tokens::S) / 2.;
            magnitude(ui, preview, half);
            ui.add_space(crate::tokens::S);
            curve(
                ui,
                "phase_pair",
                half,
                x,
                "Phase (degrees)",
                &preview.freq,
                &preview.phase,
            );
        }
        View::GroupDelay => curve(
            ui,
            "group_delay",
            height,
            x,
            "Group delay (samples)",
            &preview.freq,
            &preview.group_delay,
        ),
        View::PhaseDelay => curve(
            ui,
            "phase_delay",
            height,
            x,
            "Phase delay (samples)",
            &preview.freq,
            &preview.phase_delay,
        ),
        View::Impulse => stems(ui, "Impulse", height, &preview.impulse),
        View::Step => stems(ui, "Step", height, &preview.step),
        View::PoleZero => pole_zero(ui, preview, height),
        View::Coefficients => {
            let mut text = coefficient_text(preview);
            egui::ScrollArea::both()
                .id_salt("designer_coefficients")
                .max_height(height)
                .auto_shrink([false, false])
                .show_themed(ui, |ui| {
                    ui.add(
                        egui::TextEdit::multiline(&mut text)
                            .code_editor()
                            .interactive(false)
                            .desired_width(f32::INFINITY),
                    );
                });
        }
        View::Info => {
            egui::Grid::new("designer_info")
                .num_columns(2)
                .spacing(egui::vec2(crate::tokens::L, crate::tokens::S))
                .show(ui, |ui| {
                    for (name, value) in info_rows(preview) {
                        ui.label(name);
                        ui.label(value);
                        ui.end_row();
                    }
                });
        }
    }
}
