use super::*;

pub struct ModelControls {
    pub kind: ModelKind,
    pub p: String,
    pub d: String,
    pub q: String,
    pub sar: String,
    pub sma: String,
    pub period: String,
    pub constant: bool,
    pub student: bool,
    pub vp: String,
    pub vq: String,
    pub lags: String,
    pub horizon: String,
    pub sort_bic: bool,
}
impl Default for ModelControls {
    fn default() -> Self {
        Self {
            kind: ModelKind::Arima,
            p: "1".into(),
            d: "0".into(),
            q: "0".into(),
            sar: String::new(),
            sma: String::new(),
            period: "0".into(),
            constant: true,
            student: false,
            vp: "1".into(),
            vq: "1".into(),
            lags: "10".into(),
            horizon: "12".into(),
            sort_bic: false,
        }
    }
}
impl ModelControls {
    pub fn spec(&self, series: usize, count: usize) -> Result<ModelSpec, String> {
        let mean = self.kind.mean();
        let spec = ModelSpec {
            kind: self.kind,
            series,
            p: if mean {
                integer(&self.p, 0, 16, "AR order p")?
            } else {
                0
            },
            d: if mean {
                integer(&self.d, 0, 2, "Difference order D")?
            } else {
                0
            },
            q: if mean {
                integer(&self.q, 0, 16, "MA order q")?
            } else {
                0
            },
            sar: if mean { parse_lags(&self.sar)? } else { vec![] },
            sma: if mean { parse_lags(&self.sma)? } else { vec![] },
            period: if mean {
                integer(&self.period, 0, 256, "Seasonal period")?
            } else {
                0
            },
            constant: self.constant,
            student: self.student,
            vp: if self.kind == ModelKind::Arima {
                1
            } else {
                integer(&self.vp, 0, 16, "Variance order P")?
            },
            vq: if self.kind == ModelKind::Arima {
                1
            } else {
                integer(&self.vq, 1, 16, "Variance order Q")?
            },
            diagnostic_lags: integer(&self.lags, 1, 256, "Residual lags")?,
            ..ModelSpec::default()
        };
        spec.validate(count)?;
        Ok(spec)
    }
}
fn text(ui: &mut egui::Ui, label: &str, value: &mut String) {
    field(ui, label, |ui| {
        ui.add(
            crate::tokens::input(egui::TextEdit::singleline(value).char_limit(128))
                .desired_width(ui.available_width()),
        );
    });
}
impl EconometricModeler {
    pub fn model_order(&self, bic: bool) -> Vec<usize> {
        let mut indices = (0..self.plan.models.len()).collect::<Vec<_>>();
        if let Some(v) = &self.current {
            indices.sort_by(|&a, &b| {
                let score = |i: usize| {
                    if bic {
                        v.models[i].bic
                    } else {
                        v.models[i].aic
                    }
                };
                score(a).total_cmp(&score(b)).then(a.cmp(&b))
            });
        }
        indices
    }
    pub fn rename_model(&mut self, name: &str) -> Result<(), String> {
        checked_name(name)?;
        if self
            .plan
            .models
            .iter()
            .enumerate()
            .any(|(i, m)| i != self.selected_model && m.name == name)
        {
            return Err("Model names must be unique".into());
        }
        self.plan
            .models
            .get_mut(self.selected_model)
            .ok_or("Select a fitted model")?
            .name = name.into();
        self.rename = name.into();
        Ok(())
    }
    pub fn delete_model(&mut self) -> Result<(), String> {
        if self.selected_model >= self.plan.models.len() {
            return Err("Select a fitted model".into());
        }
        self.plan.models.remove(self.selected_model);
        if let Some(v) = &mut self.current {
            v.models.remove(self.selected_model);
        }
        self.selected_model = self
            .selected_model
            .min(self.plan.models.len().saturating_sub(1));
        if let Some(m) = self.plan.models.get(self.selected_model) {
            self.rename = m.name.clone();
        }
        Ok(())
    }
    pub(super) fn output_names(&self) -> Vec<String> {
        let mut n = vec![self.name.clone()];
        if !self.plan.models.is_empty() {
            n.extend(self.model_names.clone());
        }
        n
    }
    pub(super) fn output_script(&self) -> Result<String, String> {
        let names = self.output_names();
        let roots = self.plan.roots();
        for (i, n) in names.iter().enumerate() {
            checked_name(n)?;
            if names[..i].contains(n) || roots.contains(n) {
                return Err("Output names must be unique and differ from source variables".into());
            }
        }
        let mut plan = self.plan.clone();
        plan.models.retain(|m| m.selected);
        if !self.plan.models.is_empty() && plan.models.is_empty() {
            return Err("Select at least one model for output".into());
        }
        let code = build_script(&plan, &self.name)?;
        if plan.models.is_empty() {
            return Ok(code);
        }
        let (body, functions) = super::super::split_local_functions(&code);
        let [models, residuals, forecasts] = &self.model_names;
        let mut out = format!("{body}{models}={{}}; {residuals}={{}}; {forecasts}={{}};\n");
        for i in 1..=plan.models.len() {
            let r = format!("{}.Models{{{i}}}", self.name);
            out += &format!(
                "{models}{{{i}}}={r}.Model;\n{residuals}{{{i}}}={r}.Residuals;\n{forecasts}{{{i}}}=struct('Forecast',{r}.Forecast,'YMSE',{r}.YMSE,'Lower',{r}.Lower,'Upper',{r}.Upper);\n"
            );
        }
        out += functions;
        Ok(out)
    }
    pub(super) fn model_inputs(&mut self, ui: &mut egui::Ui) {
        let c = &mut self.controls;
        field(ui, "Model family", |ui| {
            egui::ComboBox::from_id_salt("econ_model_kind")
                .selected_text(c.kind.label())
                .show_ui(ui, |ui| {
                    for kind in ModelKind::ALL {
                        ui.selectable_value(&mut c.kind, kind, kind.label());
                    }
                });
        });
        if c.kind.mean() {
            text(ui, "AR order p", &mut c.p);
            text(ui, "Difference order D", &mut c.d);
            text(ui, "MA order q", &mut c.q);
            text(ui, "SAR lags (e.g. [12 24])", &mut c.sar);
            text(ui, "SMA lags", &mut c.sma);
            text(ui, "Seasonal period (0 = none)", &mut c.period);
            ui.checkbox(&mut c.constant, "Estimate constant");
        }
        if c.kind != ModelKind::Arima {
            text(ui, "Variance order P", &mut c.vp);
            text(ui, "Variance order Q", &mut c.vq);
        }
        ui.checkbox(&mut c.student, "Student t distribution");
        text(ui, "Residual diagnostic lags", &mut c.lags);
        if matches!(
            c.kind,
            ModelKind::ArimaGarch | ModelKind::ArimaEgarch | ModelKind::ArimaGjr
        ) {
            crate::tokens::helper(
                ui,
                "Combined conditional mean/variance estimation requires engine support; errors retain your fitted models.",
            );
        }
        if !self.plan.series.is_empty() {
            super::super::expression_message(
                ui,
                c.spec(self.selected, self.plan.series.len()).map(|_| ()),
            );
        }
        if ui
            .add_enabled(!self.plan.series.is_empty(), egui::Button::new("Estimate"))
            .clicked()
        {
            self.command = Some(Ok(Command::Estimate));
        }
        ui.add_space(crate::tokens::S);
        text(ui, "Forecast horizon", &mut c.horizon);
        super::super::expression_message(
            ui,
            integer(&c.horizon, 1, 1000, "Forecast horizon").map(|_| ()),
        );
        if ui
            .add_enabled(!self.plan.models.is_empty(), egui::Button::new("Forecast"))
            .clicked()
        {
            self.command = Some(Ok(Command::Forecast));
        }
    }
    pub(super) fn models_ui(&mut self, ui: &mut egui::Ui) {
        egui::ScrollArea::vertical()
            .id_salt("econ_models_view")
            .show_themed(ui, |ui| {
                ui.add_enabled_ui(!self.running, |ui| {
                    ui.horizontal_wrapped(|ui| {
                        ui.label("Sort by");
                        ui.selectable_value(&mut self.controls.sort_bic, false, "AIC");
                        ui.selectable_value(&mut self.controls.sort_bic, true, "BIC");
                    });
                    egui::ScrollArea::horizontal()
                        .id_salt("econ_models_table_scroll")
                        .show_themed(ui, |ui| {
                            egui::Grid::new("econ_models_table")
                                .spacing(egui::vec2(crate::tokens::S, crate::tokens::XS))
                                .striped(true)
                                .show(ui, |ui| {
                                    for label in
                                        ["Output", "Model", "Type / series", "logL", "AIC", "BIC"]
                                    {
                                        ui.label(label);
                                    }
                                    ui.end_row();
                                    for i in self.model_order(self.controls.sort_bic) {
                                        let m = &mut self.plan.models[i];
                                        ui.checkbox(&mut m.selected, "");
                                        if ui
                                            .selectable_value(&mut self.selected_model, i, &m.name)
                                            .changed()
                                        {
                                            self.rename = m.name.clone();
                                        }
                                        ui.label(format!("{} / {}", m.kind.label(), m.series + 1));
                                        if let Some(v) =
                                            self.current.as_ref().and_then(|v| v.models.get(i))
                                        {
                                            for x in [v.log_likelihood, v.aic, v.bic] {
                                                ui.label(short(x));
                                            }
                                        }
                                        ui.end_row();
                                    }
                                });
                        });
                    if !self.plan.models.is_empty() {
                        text(ui, "Model name", &mut self.rename);
                        super::super::expression_message(ui, checked_name(&self.rename));
                        ui.horizontal_wrapped(|ui| {
                            if ui.button("Rename model").clicked() {
                                let name = self.rename.clone();
                                self.edit_result = Some(self.rename_model(&name));
                            }
                            if ui.button("Delete model").clicked() {
                                self.edit_result = Some(self.delete_model());
                            }
                        });
                    }
                });
                let Some((spec, v)) = self.plan.models.get(self.selected_model).zip(
                    self.current
                        .as_ref()
                        .and_then(|v| v.models.get(self.selected_model)),
                ) else {
                    crate::tokens::helper(
                        ui,
                        "Choose a model family and Estimate to keep a fitted candidate.",
                    );
                    return;
                };
                ui.add_space(crate::tokens::S);
                ui.label(format!("{} · {}", spec.name, spec.kind.label()));
                egui::ScrollArea::horizontal()
                    .id_salt("econ_parameters_scroll")
                    .show_themed(ui, |ui| {
                        egui::Grid::new("econ_parameters")
                            .striped(true)
                            .show(ui, |ui| {
                                for s in ["Parameter", "Value", "SE", "t", "p"] {
                                    ui.label(s);
                                }
                                ui.end_row();
                                for p in &v.parameters {
                                    ui.label(&p.name);
                                    for x in p.values {
                                        ui.label(short(x));
                                    }
                                    ui.end_row();
                                }
                            });
                    });
                ui.label(if spec.kind.mean() {
                    "Residuals"
                } else {
                    "Standardized residuals"
                });
                line_plot(
                    ui,
                    "econ_residuals",
                    "Observation",
                    &indexed(&v.residuals, 0),
                    &[],
                    160.,
                );
                ui.label("Residual ACF");
                line_plot(
                    ui,
                    "econ_res_acf",
                    "Lag",
                    &paired(&v.acf_lags, &v.acf),
                    &v.acf_bounds,
                    160.,
                );
                ui.label(format!(
                    "Ljung-Box: h={} · p={} · statistic={} · critical={}",
                    short(v.ljung_box[0]),
                    short(v.ljung_box[1]),
                    short(v.ljung_box[2]),
                    short(v.ljung_box[3])
                ));
                let p = crate::tokens::palette(ui.ctx());
                ui.label("Residual density and fitted normal");
                egui_plot::Plot::new("econ_hist")
                    .height(160.)
                    .show(ui, |plot| {
                        let width = if v.histogram_x.len() > 1 {
                            v.histogram_x[1] - v.histogram_x[0]
                        } else {
                            1.
                        };
                        plot.bar_chart(
                            egui_plot::BarChart::new(
                                "Density",
                                v.histogram_x
                                    .iter()
                                    .zip(&v.histogram_y)
                                    .map(|(&x, &y)| egui_plot::Bar::new(x, y).width(width))
                                    .collect(),
                            )
                            .color(p.accent),
                        );
                        plot.line(
                            egui_plot::Line::new("Normal fit", paired(&v.histogram_x, &v.normal))
                                .color(p.text_muted),
                        );
                    });
                ui.label("Normal quantiles / residual quantiles");
                egui_plot::Plot::new("econ_qq")
                    .height(160.)
                    .show(ui, |plot| {
                        plot.points(
                            egui_plot::Points::new("Residual quantiles", paired(&v.qq_x, &v.qq_y))
                                .color(p.accent),
                        );
                        plot.line(
                            egui_plot::Line::new("Normal fit", paired(&v.qq_x, &v.qq_fit))
                                .color(p.text_muted),
                        );
                    });
                if !v.forecast.is_empty() {
                    if let Some(mse) = v.mse.last() {
                        ui.label(format!("Final forecast MSE: {}", short(*mse)));
                    }
                    ui.label(if spec.kind.mean() {
                        "Forecast and 95% pointwise bands"
                    } else {
                        "Forecast conditional variance"
                    });
                    let series = &self.current.as_ref().unwrap().series[spec.series];
                    egui_plot::Plot::new("econ_forecast")
                        .height(220.)
                        .x_axis_label("Observation")
                        .show(ui, |plot| {
                            if spec.kind.mean() {
                                plot.line(
                                    egui_plot::Line::new("Data", series.points.clone())
                                        .color(p.text_muted),
                                );
                            } else {
                                plot.line(
                                    egui_plot::Line::new(
                                        "Inferred variance",
                                        indexed(&v.variance, 0),
                                    )
                                    .color(p.text_muted),
                                );
                            }
                            let offset = series.values.len();
                            plot.line(
                                egui_plot::Line::new("Forecast", indexed(&v.forecast, offset))
                                    .color(p.accent),
                            );
                            for (label, values) in [("Lower", &v.lower), ("Upper", &v.upper)] {
                                if !values.is_empty() {
                                    plot.line(
                                        egui_plot::Line::new(label, indexed(values, offset))
                                            .color(p.text_muted)
                                            .style(egui_plot::LineStyle::dashed_dense()),
                                    );
                                }
                            }
                        });
                }
            });
    }
}
fn paired(x: &[f64], y: &[f64]) -> Vec<[f64; 2]> {
    let stride = x.len().div_ceil(2000).max(1);
    x.iter()
        .zip(y)
        .step_by(stride)
        .map(|(&x, &y)| [x, y])
        .collect()
}
fn indexed(y: &[f64], offset: usize) -> Vec<[f64; 2]> {
    let stride = y.len().div_ceil(2000).max(1);
    y.iter()
        .enumerate()
        .step_by(stride)
        .map(|(i, &y)| [(i + offset + 1) as f64, y])
        .collect()
}
