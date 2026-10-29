//! Designer fields scroll independently; accepted actions remain in a fixed footer.
use super::*;
use crate::theme::ThemedScrollArea;

impl Toolkits {
    pub(super) fn designer_ui(
        &mut self,
        ui: &mut egui::Ui,
        busy: bool,
        node_edit_interactive: bool,
        action: &mut Option<Action>,
    ) {
        // Below this size the whole tool scrolls instead of squeezing the footer.
        let size = ui.available_size().max(egui::vec2(320., 280.));
        egui::ScrollArea::both()
            .id_salt("designer_tab")
            .auto_shrink([false, false])
            .show_themed(ui, |ui| {
                let bounds = egui::Rect::from_min_size(ui.cursor().min, size);
                ui.scope_builder(egui::UiBuilder::new().max_rect(bounds), |ui| {
                let row = ui.spacing().interact_size.y.max(
                    ui.text_style_height(&egui::TextStyle::Button) + 2. * ui.spacing().button_padding.y,
                );
                // Reserve the result summary and all three action rows independently
                // of help/script scrolling and busy state. Three text lines cover the
                // bounded summaries at the minimum supported window width.
                let summary_height = 3. * ui.text_style_height(&egui::TextStyle::Body);
                let footer_height = summary_height + 3. * row
                    + 5. * ui.spacing().item_spacing.y + 12.;
                let split = (bounds.bottom() - footer_height).max(bounds.top());
                let fields = egui::Rect::from_min_max(bounds.min, egui::pos2(bounds.right(), split));
                let footer = egui::Rect::from_min_max(egui::pos2(bounds.left(), split), bounds.max);
                // Evaluate settings first, so same-frame edits affect captured requests.
                ui.scope_builder(egui::UiBuilder::new().max_rect(fields), |ui| {
                    ui.set_clip_rect(ui.clip_rect().intersect(fields));
                    egui::ScrollArea::both()
                        .id_salt("designer_fields")
                        .max_height(fields.height())
                        .auto_shrink([false, false])
                        .show_themed(ui, |ui| {
                            self.designer_fields(ui);
                            match self.design_script() {
                                Ok(script) => {
                                    ui.separator();
                                    ui.monospace(script);
                                }
                                Err(error) => { ui.colored_label(crate::tokens::palette(ui.ctx()).error, error); }
                            }
                            if self.saving_design { ui.small("A save already in progress may finish before Stop takes effect."); }
                            if !self.message.is_empty() { ui.separator(); ui.label(&self.message); }
                        });
                });
                ui.scope_builder(egui::UiBuilder::new().max_rect(footer), |ui| {
                    ui.set_clip_rect(ui.clip_rect().intersect(footer));
                    ui.separator();
                    ui.allocate_ui_with_layout(
                        egui::vec2(ui.available_width(), summary_height),
                        egui::Layout::top_down(egui::Align::Min),
                        |ui| {
                            ui.set_min_height(summary_height);
                            let summary = if self.iir {
                                self.iir_design().summary()
                            } else {
                                self.fir_design().summary()
                            };
                            match summary {
                                Ok(summary) => { ui.add(egui::Label::new(summary).wrap()); }
                                Err(_) => { ui.colored_label(crate::tokens::palette(ui.ctx()).error, "Invalid filter settings"); }
                            }
                        },
                    );
                    let design = self.design_script();
                    ui.add_enabled_ui(!busy && design.is_ok(), |ui| {
                        ui.horizontal(|ui| {
                            if ui.button("Design + response").clicked() {
                                match self.preview_script() { Ok(script) => *action=Some(Action::Script(script)), Err(error) => self.message=error }
                            }
                            if ui.button("Apply to y").clicked() {
                                if matrust::files::valid_name(&self.input) {
                                    let design = if self.iir {
                                        super::super::designer_apply::Design::Iir(self.iir_design())
                                    } else {
                                        super::super::designer_apply::Design::Fir(self.fir_design())
                                    };
                                    *action=Some(Action::Apply(design, self.input.clone()));
                                } else { self.message="Input must be a workspace variable name".into(); }
                            }
                        });
                        ui.horizontal(|ui| {
                            if ui.button("Save design script…").clicked() {
                                *action = Some(Action::ChooseDesignSave(self.selected_save_design()));
                            }
                            if ui.button("Add to block model").clicked() && node_edit_interactive {
                                *action=Some(if self.iir {Action::IirBlock(self.iir_design())} else {Action::FirBlock(self.fir_design())});
                            }
                        });
                    });
                    if (self.pending_design_model.is_some() || self.saving_design)
                        && ui.button("Stop filter design").clicked()
                    {
                        *action = Some(Action::StopModel);
                    }
                });
                ui.expand_to_include_rect(bounds);
                });
            });
    }

    fn designer_fields(&mut self, ui: &mut egui::Ui) {
        ui.label("Design in Hz. Coefficients are shared with scripts and simulation blocks.");
        ui.separator();
        ui.horizontal_wrapped(|ui| {
            let first = ui.selectable_value(&mut self.iir, false, "FIR · windowed");
            super::super::focus_areas::register(
                ui,
                &first,
                super::super::focus_areas::Area::Designer,
            );
            if ui
                .selectable_value(&mut self.iir, true, "IIR · SOS")
                .clicked()
            {
                self.order = self.order.min(6);
            }
        });
        egui::Grid::new("filter_parameters")
            .num_columns(2)
            .spacing([20., 10.])
            .show(ui, |ui| {
                if self.iir {
                    ui.label("Design family");
                    egui::ComboBox::from_id_salt("iir_family")
                        .selected_text(self.iir_family.label())
                        .show_ui(ui, |ui| {
                            for family in [
                                super::super::iir_designer::Family::Butterworth,
                                super::super::iir_designer::Family::Chebyshev1,
                            ] {
                                ui.selectable_value(&mut self.iir_family, family, family.label());
                            }
                        });
                    ui.end_row();
                    if self.iir_family == super::super::iir_designer::Family::Chebyshev1 {
                        ui.label("Passband ripple (dB)");
                        let response = ui.add(egui::DragValue::new(&mut self.ripple_db).speed(0.1));
                        #[cfg(test)]
                        ui.ctx().data_mut(|data| {
                            data.insert_temp(
                                egui::Id::new("designer_ripple_response"),
                                response.rect,
                            )
                        });
                        let _ = response;
                        ui.end_row();
                    }
                }
                ui.label("Response");
                egui::ComboBox::from_id_salt("filter_band")
                    .selected_text(self.band.name())
                    .show_ui(ui, |ui| {
                        for kind in [
                            Band::Lowpass,
                            Band::Highpass,
                            Band::Bandpass,
                            Band::Bandstop,
                        ] {
                            ui.selectable_value(&mut self.band, kind, kind.name());
                        }
                    });
                ui.end_row();
                if !self.iir {
                    ui.label("Window");
                    egui::ComboBox::from_id_salt("fir_window")
                        .selected_text(self.fir_design().window_info().0)
                        .show_ui(ui, |ui| {
                            for (kind, label, _) in super::super::fir_designer::WINDOWS {
                                ui.selectable_value(&mut self.fir_window, kind, label);
                            }
                        });
                    ui.end_row();
                    ui.label("Normalization");
                    ui.checkbox(&mut self.fir_scale, "Scale passband gain");
                    ui.end_row();
                }
                ui.label("Order");
                ui.add(
                    egui::DragValue::new(&mut self.order)
                        .range(1..=if self.iir { 20 } else { 4096 }),
                );
                ui.end_row();
                ui.label("Sample rate (Hz)");
                ui.add(egui::DragValue::new(&mut self.sample_rate).speed(10.));
                ui.end_row();
                ui.label("Cutoff / lower edge (Hz)");
                ui.add(egui::DragValue::new(&mut self.cutoff).speed(1.));
                ui.end_row();
                if matches!(self.band, Band::Bandpass | Band::Bandstop) {
                    ui.label("Upper edge (Hz)");
                    ui.add(egui::DragValue::new(&mut self.upper).speed(1.));
                    ui.end_row();
                }
                ui.label("Workspace input variable");
                ui.text_edit_singleline(&mut self.input);
                ui.end_row();
                ui.label("Analysis");
                egui::ComboBox::from_id_salt("filter_analysis")
                    .selected_text(self.analysis.label())
                    .show_ui(ui, |ui| {
                        for kind in Analysis::ALL {
                            ui.selectable_value(&mut self.analysis, kind, kind.label());
                        }
                    });
                ui.end_row();
                ui.label("Analysis points / samples");
                ui.add(egui::DragValue::new(&mut self.analysis_points).range(1..=65536));
                ui.end_row();
            });
        if self.iir {
            ui.small("IIR prototype orders 1–20. Bandpass/stop result order is twice the prototype order; at most 20 SOS sections.");
            if self.iir_family == super::super::iir_designer::Family::Chebyshev1 {
                ui.small("Chebyshev I: positive finite passband ripple in dB; cutoffs are passband edges. Unrepresentable extreme settings report a worker error.");
            }
            ui.small("Apply to y uses causal SOS filtering.");
            if self.order == 1
                || (self.order == 2 && matches!(self.band, Band::Lowpass | Band::Highpass))
            {
                ui.small("One SOS row: use separate numerator/denominator vectors for zero-phase filtfilt in scripts.");
            }
        } else {
            ui.small("Symmetric windows; orders 1–4096. Odd highpass/stop orders increase by one.");
            ui.small("Custom workspace windows: use fir1 in a script. This selector offers nine built-in windows.");
        }
        if self.analysis == Analysis::GroupDelay {
            ui.small("Delay is undefined at response zeros. Gaps mark singular values; deep stopbands can lose precision.");
        }
    }
}
