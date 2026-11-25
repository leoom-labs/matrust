//! Native scenario controls and channel-at-a-time themed trajectory views.
use super::*;

impl MpcDesigner {
    pub(super) fn scenario_ui(&mut self, ui: &mut egui::Ui) {
        ui.label("Scenario");
        ui.horizontal_wrapped(|ui| {
            if ui
                .add_enabled(
                    !self.running && !self.controllers.is_empty(),
                    egui::Button::new("Run scenario"),
                )
                .clicked()
            {
                self.command = Some(Command::Run);
            }
            if ui
                .add_enabled(self.running, egui::Button::new("Stop"))
                .clicked()
            {
                self.stop_requested = true;
            }
        });
        if self.running {
            if let Some(run) = self.trial.as_ref().and_then(|t| t.run.as_ref()) {
                ui.label(format!(
                    "Pending: {} · {} samples",
                    run.controllers
                        .iter()
                        .map(|c| c.spec.name.as_str())
                        .collect::<Vec<_>>()
                        .join(" / "),
                    run.scenario.samples
                ));
            }
            ui.label("Calculation running; last completed results remain visible.");
        }
        ui.add_enabled_ui(!self.running, |ui| {
            for (label, value, hint) in [
                ("Stored samples (1–10000)", &mut self.scenario.samples, "30"),
                ("Reference · physical output columns", &mut self.scenario.reference, "Empty = nominal outputs; e.g. 1 or [0; 1]"),
                ("Measured disturbances · physical MD columns", &mut self.scenario.disturbance, "Empty = nominal MD inputs"),
                ("Plant initial state", &mut self.scenario.initial, "Empty = nominal plant state"),
            ] {
                ui.label(label);
                ui.add(crate::tokens::input(egui::TextEdit::singleline(value).hint_text(hint).char_limit(32768)).desired_width(ui.available_width()));
            }
            if let Some(c) = self.controllers.get(self.selected)
                && let (Ok(n), Ok(ts)) = (self.scenario.count(), script::scalar(&c.spec.ts, "Sample time"))
            {
                crate::tokens::helper(ui, format!("Duration: {} time units · {} transitions", script::num((n-1) as f64*ts), n-1));
            }
            crate::tokens::helper(ui, "Rows are time samples; short signals hold their last row. Each run starts with fresh controller state. Reference includes all physical output columns; UO is not measured feedback.");
            ui.checkbox(&mut self.scenario.reference_preview, "Reference lookahead");
            ui.checkbox(&mut self.scenario.disturbance_preview, "MD lookahead");
            let label = self.compare.and_then(|i|self.controllers.get(i)).map_or("None".to_owned(), |c|c.spec.name.clone());
            egui::ComboBox::from_id_salt("mpc_compare").width(ui.available_width()).selected_text(format!("Compare: {label}")).show_ui(ui, |ui| {
                ui.selectable_value(&mut self.compare, None, "None");
                for (i,c) in self.controllers.iter().enumerate() { ui.selectable_value(&mut self.compare, Some(i), format!("Compare with {}", c.spec.name)); }
            });
            crate::tokens::helper(ui, "Run uses accepted controllers. Apply controller to accept tuning edits first.");
        });
        ui.separator();
    }
    pub(super) fn scenario_results_ui(&mut self, ui: &mut egui::Ui) {
        ui.horizontal_wrapped(|ui| {
            for (i, label) in [
                "Outputs",
                "Manipulated variables",
                "Data",
                "Controller summary",
            ]
            .iter()
            .enumerate()
            {
                ui.selectable_value(&mut self.view, i, *label);
            }
        });
        if self.view == 3 {
            if let Some(c) = self.controllers.get(self.selected) {
                egui::ScrollArea::both()
                    .id_salt("mpc_summary")
                    .show_themed(ui, |ui| {
                        ui.label(&c.spec.name);
                        ui.label(format!(
                            "Plant: {} states, {} inputs, {} outputs",
                            c.plant_size[0], c.plant_size[1], c.plant_size[2]
                        ));
                        ui.label(&c.summary);
                        for (kind, rows) in [("MV", &c.spec.mv), ("OV", &c.spec.ov)] {
                            for row in rows {
                                ui.label(format!(
                                    "{kind} plant channel {} · {} [{}] · Min {} · Max {}",
                                    row.channel + 1,
                                    row.name,
                                    row.units,
                                    row.bounds[0],
                                    row.bounds[1]
                                ));
                            }
                        }
                    });
            } else {
                ui.label("Choose a workspace plant or controller, then Create controller.");
            }
            return;
        }
        if self.running {
            let pending = self
                .trial
                .as_ref()
                .and_then(|t| t.run.as_ref())
                .map(|r| {
                    format!(
                        "{} · {} samples",
                        r.controllers
                            .iter()
                            .map(|c| c.spec.name.as_str())
                            .collect::<Vec<_>>()
                            .join(" / "),
                        r.scenario.samples
                    )
                })
                .unwrap_or_else(|| "controller preparation".into());
            ui.label(format!(
                "Pending: {pending}. Last completed results remain visible."
            ));
        }
        if let Some(error) = &self.error {
            super::super::expression_message(ui, Err(error.clone()));
        }
        let Some(result) = &self.result else {
            ui.label(if self.controllers.is_empty() {
                "Choose a workspace plant or controller, then Create controller."
            } else {
                "Set the scenario in Inputs, then Run scenario."
            });
            return;
        };
        let identities = result
            .run
            .controllers
            .iter()
            .map(|c| c.spec.name.as_str())
            .collect::<Vec<_>>()
            .join(" / ");
        ui.label(format!(
            "Completed run: {identities} · {} samples",
            result.run.scenario.samples
        ));
        if self.scenario != result.run.scenario
            || self
                .compare
                .and_then(|i| self.controllers.get(i))
                .map(|c| &c.spec.name)
                != result.run.controllers.get(1).map(|c| &c.spec.name)
            || self
                .controllers
                .get(self.selected)
                .is_none_or(|c| c.spec.name != result.run.controllers[0].spec.name)
        {
            crate::tokens::helper(
                ui,
                "Inputs changed. Curves and scenario export still use the completed run snapshot.",
            );
        }
        if self
            .editor
            .as_ref()
            .zip(self.controllers.get(self.selected))
            .is_some_and(|(s, c)| *s != c.spec)
        {
            crate::tokens::helper(
                ui,
                "Controller draft differs from the accepted controller. Displayed curves retain their run snapshot.",
            );
        }
        if result.run.controllers.iter().any(|old| {
            self.controllers
                .iter()
                .find(|c| c.spec.name == old.spec.name)
                .is_none_or(|c| c.spec != old.spec)
        }) {
            crate::tokens::helper(
                ui,
                "Accepted controllers changed. Displayed curves and scenario export retain the completed run snapshot.",
            );
        }
        let spec = &result.run.controllers[0].spec;
        let mv = self.view == 1;
        if self.view == 2 {
            ui.checkbox(&mut self.data_mv, "MV data");
        }
        let mv = mv || (self.view == 2 && self.data_mv);
        let count = if mv {
            spec.mv.len()
        } else {
            spec.outputs.len()
        };
        let selected = if mv {
            &mut self.mv_channel
        } else {
            &mut self.output_channel
        };
        *selected = (*selected).min(count.saturating_sub(1));
        egui::ComboBox::from_id_salt("mpc_result_channel")
            .width(ui.available_width().min(400.))
            .selected_text(channel_label(spec, mv, *selected))
            .show_ui(ui, |ui| {
                for i in 0..count {
                    ui.selectable_value(selected, i, channel_label(spec, mv, i));
                }
            });
        let channel = *selected;
        if self.view == 2 {
            egui::ScrollArea::both()
                .id_salt("mpc_data")
                .show_rows_themed(ui, 24., result.trajectories[0].t.rows() + 1, |ui, rows| {
                    ui.spacing_mut().item_spacing.y = 0.;
                    egui::Grid::new("mpc_data_grid")
                        .min_row_height(24.)
                        .spacing(egui::vec2(crate::tokens::S, 0.))
                        .striped(true)
                        .show(ui, |ui| {
                            for index in rows {
                                if index == 0 {
                                    ui.label("t");
                                    for c in &result.run.controllers {
                                        ui.add(egui::Label::new(&c.spec.name).extend());
                                    }
                                    if !mv {
                                        ui.label("Reference");
                                    }
                                } else {
                                    let row = index - 1;
                                    ui.label(script::num(result.trajectories[0].t.get(row, 0)));
                                    for data in &result.trajectories {
                                        ui.label(script::num(if mv {
                                            data.u.get(row, channel)
                                        } else {
                                            data.y.get(row, channel)
                                        }));
                                    }
                                    if !mv {
                                        ui.label(script::num(
                                            result
                                                .reference
                                                .get(row.min(result.reference.rows() - 1), channel),
                                        ));
                                    }
                                }
                                ui.end_row();
                            }
                        });
                });
        } else {
            plot(ui, result, mv, channel);
        }
    }
}
fn channel_label(s: &Spec, mv: bool, channel: usize) -> String {
    let row = if mv {
        s.mv.get(channel)
    } else {
        s.ov.iter().find(|r| r.channel == channel)
    };
    let physical = row.map_or(channel, |r| r.channel);
    format!(
        "{} {} · {} [{}]{}",
        if mv { "MV" } else { "Output" },
        physical + 1,
        row.map_or("", |r| r.name.as_str()),
        row.map_or("", |r| r.units.as_str()),
        if !mv && s.outputs.get(channel) == Some(&1) {
            " · unmeasured"
        } else {
            ""
        }
    )
}
fn plot(ui: &mut egui::Ui, result: &ScenarioResult, mv: bool, channel: usize) {
    let palette = crate::tokens::palette(ui.ctx());
    let first = &result.trajectories[0];
    let height = ui.available_height().clamp(180., 640.);
    let label_width = ui.available_width();
    egui_plot::Plot::new(("mpc_trajectory", mv, channel))
        .height(height)
        .x_axis_label("Time (sim)")
        .legend(egui_plot::Legend::default())
        .show(ui, |plot| {
            for (i, (controller, data)) in result
                .run
                .controllers
                .iter()
                .zip(&result.trajectories)
                .enumerate()
            {
                let dashed = if result.run.controllers.len() == 2
                    && result.run.controllers[0].spec.name != result.run.controllers[1].spec.name
                {
                    controller.spec.name > result.run.controllers[1 - i].spec.name
                } else {
                    i == 1
                };
                let name = format!(
                    "{} · {}",
                    controller.spec.name,
                    if dashed { "dashed" } else { "solid" }
                );
                let values = if mv { &data.u } else { &data.y };
                let points = (0..data.t.rows())
                    .map(|r| [data.t.get(r, 0), values.get(r, channel)])
                    .collect::<Vec<_>>();
                for segment in points
                    .split(|p| !p[1].is_finite())
                    .filter(|s| !s.is_empty())
                {
                    let mut line =
                        egui_plot::Line::new(name.clone(), segment.to_vec()).color(palette.accent);
                    if dashed {
                        line = line.style(egui_plot::LineStyle::dashed_dense());
                    }
                    plot.line(line);
                }
                if result.trajectories.len() <= 2
                    && (label_width >= 480. && controller.spec.name.len() < 24)
                {
                    plot.text(
                        egui_plot::Text::new(
                            format!("label{i}"),
                            egui_plot::PlotPoint::new(
                                data.t.get(data.t.rows() - 1, 0),
                                values.get(values.rows() - 1, channel),
                            ),
                            name,
                        )
                        .name("")
                        .allow_hover(false)
                        .anchor(if dashed {
                            egui::Align2::RIGHT_TOP
                        } else {
                            egui::Align2::RIGHT_BOTTOM
                        })
                        .color(palette.text),
                    );
                }
                let row = if mv {
                    controller.spec.mv.get(channel)
                } else {
                    controller.spec.ov.iter().find(|r| r.channel == channel)
                };
                if let Some(row) = row {
                    for (bound, label) in [(&row.bounds[0], "minimum"), (&row.bounds[1], "maximum")]
                    {
                        if let Ok(value) = script::scalar(bound, "Bound")
                            && value.is_finite()
                        {
                            plot.hline(
                                egui_plot::HLine::new(
                                    format!("{} {label}", controller.spec.name),
                                    value,
                                )
                                .color(palette.text_muted)
                                .style(egui_plot::LineStyle::dotted_dense()),
                            );
                        }
                    }
                }
            }
            if !mv {
                if label_width >= 480. {
                    plot.text(
                        egui_plot::Text::new(
                            "reference_label",
                            egui_plot::PlotPoint::new(
                                first.t.get(0, 0),
                                result.reference.get(0, channel),
                            ),
                            "Reference",
                        )
                        .name("")
                        .allow_hover(false)
                        .anchor(egui::Align2::LEFT_BOTTOM)
                        .color(palette.text),
                    );
                }
                let points = (0..first.t.rows())
                    .map(|r| {
                        [
                            first.t.get(r, 0),
                            result
                                .reference
                                .get(r.min(result.reference.rows() - 1), channel),
                        ]
                    })
                    .collect::<Vec<_>>();
                plot.line(
                    egui_plot::Line::new("Reference (physical output)", points)
                        .color(palette.text_muted)
                        .style(egui_plot::LineStyle::dashed_loose()),
                );
            }
        });
    crate::tokens::helper(
        ui,
        "Hover for numeric coordinates; drag to pan, scroll to zoom. Bounds show finite scalar controller limits; horizon-varying bounds are listed in Controller summary.",
    );
}
