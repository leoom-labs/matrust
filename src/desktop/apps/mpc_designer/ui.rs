use super::*;
fn edit(ui: &mut egui::Ui, label: &str, value: &mut String) {
    super::super::field(ui, label, |ui| {
        ui.add(
            crate::tokens::input(egui::TextEdit::singleline(value).char_limit(32768))
                .desired_width(ui.available_width()),
        );
        super::super::expression_message(ui, script::literal(value).map(|_| ()));
    });
}
impl MpcDesigner {
    pub(super) fn editor_ui(&mut self, ui: &mut egui::Ui, ws: &Snapshot) {
        self.scenario_ui(ui);
        ui.add_enabled_ui(!self.running,|ui| {
            super::super::field(ui,"Workspace plant or mpc controller",|ui| {
                ui.add(crate::tokens::input(egui::TextEdit::singleline(&mut self.source_text).char_limit(63)).desired_width(ui.available_width()));
                egui::ComboBox::from_id_salt("mpc_source").selected_text("Choose a variable").show_ui(ui,|ui| {
                    for (name,class) in ws.lti_models() {ui.selectable_value(&mut self.source_text,name.to_owned(),format!("{name} ({class})"));}
                    for source in &self.catalogue {ui.selectable_value(&mut self.source_text,source.name.clone(),format!("{} (mpc)",source.name));}
                });
                if !self.source_text.is_empty() {super::super::expression_message(ui,self.source(ws).map(|_|()));}
            });
            if ui.button("Create controller").clicked() {self.command=Some(Command::Create);}
            ui.add_space(crate::tokens::S);
            ui.label("Controllers");
            let mut selected=self.selected;
            for (i,c) in self.controllers.iter().enumerate() {ui.selectable_value(&mut selected,i,&c.spec.name);}
            if selected!=self.selected {self.select(selected);self.edit_result=Some(Ok(()));}
            if !self.controllers.is_empty() {
                ui.horizontal(|ui| {
                    if ui.button("Duplicate").clicked() {
                        let mut c=self.controllers[self.selected].clone();
                        c.spec.name=super::super::unused_name("controller",|n|ws.contains(n)||self.controllers.iter().any(|c|c.spec.name==n));
                        if self.controllers.len()<16 {self.controllers.push(c);self.select(self.controllers.len()-1);self.edit_result=Some(Ok(()));} else {self.edit_result=Some(Err("Keep at most 16 controllers".into()));}
                    }
                    if ui.button("Delete").clicked() {self.compare=self.compare.and_then(|i|if i==self.selected {None} else {Some(i-usize::from(i>self.selected))});self.controllers.remove(self.selected);self.select(self.selected.min(self.controllers.len().saturating_sub(1)));self.edit_result=Some(Ok(()));}
                });
                super::super::name_field(ui,"Controller / export name",&mut self.rename,ws);
                if ui.button("Rename").clicked() {self.edit_result=Some(self.rename_selected());}
            }
            let Some(s)=&mut self.editor else {return};
            // Apply sits above the longer tables so it stays reachable on compact displays.
            if ui.button("Apply controller").clicked() {self.command=Some(Command::Apply);}
            super::super::expression_message(ui,script::validate(s));
            edit(ui,"Sample time",&mut s.ts);
            edit(ui,"Prediction horizon",&mut s.prediction);
            edit(ui,"Control horizon / blocking vector",&mut s.control);
            ui.collapsing("Signal roles",|ui| {
                if s.imported {crate::tokens::helper(ui,"Changing roles rebuilds default disturbance and estimator models; MV/OV targets and scaling follow retained channels.");}
                let mut changed=false;
                for (input,roles) in [(true,&mut s.inputs),(false,&mut s.outputs)] {
                    for (i,role) in roles.iter_mut().enumerate() {
                        ui.horizontal(|ui| {
                            ui.label(format!("{} {}",if input {"Input"} else {"Output"},i+1));
                            let labels: &[&str]=if input {&["MV","MD","UD"]} else {&["MO","UO"]};
                            egui::ComboBox::from_id_salt(("mpc_role",input,i)).selected_text(labels[*role]).show_ui(ui,|ui| {
                                for (j,label) in labels.iter().enumerate() {changed|=ui.selectable_value(role,j,*label).changed();}
                            });
                        });
                    }
                }
                if changed {s.sync_roles();}
            });
            egui::CollapsingHeader::new("Weights").default_open(true).show(ui,|ui| {
                crate::tokens::helper(ui,"Rows follow the constraint tables below. A scalar applies to every signal.");
                for (label,value) in ["OV weights","MV weights","MVRate weights"].into_iter().zip(&mut s.weights) {edit(ui,label,value);}
                edit(ui,"ECR weight",&mut s.ecr);
            });
            egui::CollapsingHeader::new("Constraints").default_open(true).show(ui,|ui| {
                for (kind,rows) in [("MV",&mut s.mv),("OV",&mut s.ov)] {
                    for (i,row) in rows.iter_mut().enumerate() {
                        ui.push_id((kind,i),|ui| {
                            ui.label(format!("{kind} {}: {} {} (plant channel {})",i+1,row.name,row.units,row.channel+1));
                            egui::Grid::new("bounds").num_columns(2).spacing(egui::vec2(8.,4.)).show(ui,|ui| {
                                for (j,label) in BOUNDS.iter().enumerate() {
                                    if kind=="OV" && (j==2||j==3) {continue}
                                    ui.label(*label);
                                    ui.add(crate::tokens::input(egui::TextEdit::singleline(&mut row.bounds[j]).char_limit(32768)).desired_width(128.));
                                    ui.end_row();
                                }
                                for (j,label) in [(4,"Soft minimum"),(5,"Soft maximum")] {
                                    let mut soft=script::literal(&row.bounds[j]).is_ok_and(|v|v.iter().flatten().any(|x|*x>0.));
                                    ui.label(label);
                                    if ui.checkbox(&mut soft,"").changed() {row.bounds[j]=if soft {"1"} else {"0"}.into();}
                                    ui.end_row();
                                }
                            });
                        });
                        ui.add_space(crate::tokens::S);
                    }
                }
            });
        });
    }
}

/// Reflow this app's three areas vertically on narrow hosts. Actions remain
/// above the scrollable editor; the ordinary desktop layout is unchanged.
pub(super) fn show_compact(
    host: &mut Host<MpcDesigner>,
    ui: &mut egui::Ui,
    ws: &Snapshot,
    busy: bool,
    action: &mut Option<Action>,
) {
    host.refresh(ui.ctx(), ws);
    host.app.running = host.running();
    let mut clicked = None;
    ui.horizontal_wrapped(|ui| {
        if ui
            .add_enabled(
                !host.running() && !host.app.controllers.is_empty(),
                egui::Button::new("Run scenario"),
            )
            .clicked()
        {
            host.app.command = Some(Command::Run);
        }
        if ui
            .add_enabled(host.running(), egui::Button::new("Stop"))
            .clicked()
        {
            host.app.stop_requested = true;
        }
        clicked = super::super::output_buttons(ui, host.ready(ws, busy));
    });
    if let Some(error) = &host.app.error {
        super::super::expression_message(ui, Err(error.clone()));
    }
    egui::ScrollArea::vertical()
        .id_salt("mpc_compact")
        .auto_shrink([false, false])
        .show_themed(ui, |ui| {
            egui::CollapsingHeader::new("Inputs")
                .default_open(host.app.controllers.is_empty())
                .show(ui, |ui| host.app.editor_ui(ui, ws));
            ui.separator();
            ui.label("Scenario results");
            ui.allocate_ui(egui::vec2(ui.available_width(), 400.), |ui| {
                host.app.scenario_results_ui(ui)
            });
            ui.separator();
            ui.label("Output");
            host.app.names_ui(ui, ws);
        });
    match clicked {
        Some(super::super::OutputClick::Export) => *action = host.export(ws),
        Some(super::super::OutputClick::Script) => *action = host.generate(ws, super::super::now()),
        None => {}
    }
}
