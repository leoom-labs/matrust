//! Right pane of App Designer: component/figure property inspector and callbacks list.
use super::{AppDesigner, DesignerView, Selection, format_span_input, parse_span_input};
use crate::tokens;
use eframe::egui::{self, RichText};
use matrust::engine::app_designer_model::{
    AppPropertyValue, AppValue, AppValueKind, is_valid_identifier,
};

pub(super) fn show(designer: &mut AppDesigner, ui: &mut egui::Ui) {
    let p = tokens::palette(ui.ctx());
    let read_only = designer.read_only.is_some();

    ui.label(
        RichText::new("INSPECTOR")
            .font(crate::panels::caption_semibold(ui))
            .color(p.text_muted),
    );
    ui.add_space(tokens::XS);

    ui.add_enabled_ui(!read_only, |ui| match designer.selection.clone() {
        Selection::Figure => show_figure_inspector(designer, ui),
        Selection::Component(name) => show_component_inspector(designer, ui, &name),
    });

    ui.add_space(tokens::S);
    ui.separator();
    ui.add_space(tokens::XS);

    show_callbacks_section(designer, ui);
}

fn show_figure_inspector(designer: &mut AppDesigner, ui: &mut egui::Ui) {
    let field_w = ui.available_width().max(60.0);

    ui.label("Class Name");
    let resp = ui.add_sized(
        [field_w, tokens::CONTROL],
        tokens::input(egui::TextEdit::singleline(&mut designer.drafts.class_name))
            .id(egui::Id::new("app_designer_class_name")),
    );
    let commit =
        resp.lost_focus() || (resp.changed() && ui.input(|i| i.key_pressed(egui::Key::Enter)));
    if commit && designer.drafts.class_name != designer.model.class_name {
        let candidate = designer.drafts.class_name.trim().to_string();
        if is_valid_identifier(&candidate) {
            designer.model.class_name = candidate;
            designer.mark_dirty();
            designer.clear_status();
        } else {
            designer.drafts.class_name = designer.model.class_name.clone();
            designer.set_error(format!("'{candidate}' is not a valid MATLAB identifier."));
        }
    }

    ui.add_space(tokens::XS);
    ui.label("Figure Title");
    let mut title = designer.model.figure.title.clone();
    if ui
        .add_sized(
            [field_w, tokens::CONTROL],
            tokens::input(egui::TextEdit::singleline(&mut title))
                .id(egui::Id::new("app_designer_fig_title")),
        )
        .changed()
    {
        designer.model.figure.title = title;
        designer.mark_dirty();
        designer.clear_status();
    }

    ui.add_space(tokens::XS);
    ui.horizontal_wrapped(|ui| {
        if designer.model.startup_fcn.is_none() {
            if ui.button("+ Add startupFcn").clicked() {
                designer.model.startup_fcn = Some(String::new());
                designer.mark_dirty();
                designer.clear_status();
                designer.view = DesignerView::Code;
                designer.focus_callback = Some("startupFcn".into());
            }
        } else {
            if ui.button("Edit startupFcn").clicked() {
                designer.view = DesignerView::Code;
                designer.focus_callback = Some("startupFcn".into());
            }
            if ui.button("Remove startupFcn").clicked() {
                designer.model.startup_fcn = None;
                designer.mark_dirty();
                designer.clear_status();
            }
        }
    });
}

fn show_component_inspector(designer: &mut AppDesigner, ui: &mut egui::Ui, name: &str) {
    let p = tokens::palette(ui.ctx());
    let Some(comp) = designer.model.component(name).cloned() else {
        designer.selection = Selection::Figure;
        return;
    };
    let full_w = ui.available_width().max(60.0);

    ui.add(
        egui::Label::new(
            RichText::new(format!(
                "{}\n{}",
                comp.kind.display_name(),
                comp.kind.matlab_class_name()
            ))
            .size(tokens::CAPTION)
            .color(p.text_muted),
        )
        .wrap(),
    );
    ui.add_space(tokens::XS);

    // Name (validated rename)
    ui.label("Name");
    let mut commit_name = false;
    ui.allocate_ui_with_layout(
        egui::vec2(full_w, tokens::CONTROL),
        egui::Layout::right_to_left(egui::Align::Center),
        |ui| {
            ui.spacing_mut().item_spacing.x = tokens::XS;
            if ui
                .add(egui::Button::new("Rename").min_size(egui::vec2(64.0, tokens::CONTROL)))
                .clicked()
            {
                commit_name = true;
            }
            let field_w = ui.available_width().max(48.0);
            let resp = ui.add_sized(
                [field_w, tokens::CONTROL],
                tokens::input(egui::TextEdit::singleline(&mut designer.drafts.name))
                    .desired_width((field_w - tokens::S * 2.0).max(24.0))
                    .id(egui::Id::new("app_designer_comp_name")),
            );
            if resp.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter)) {
                commit_name = true;
            }
        },
    );
    if commit_name {
        let candidate = designer.drafts.name.trim().to_string();
        match designer.model.rename_component(name, &candidate) {
            Ok(()) => {
                designer.selection = Selection::Component(candidate.clone());
                designer.drafts.name = candidate.clone();
                designer.mark_dirty();
                designer.set_notice(format!("Renamed component to {candidate}."));
                designer.sync_drafts(true);
                return;
            }
            Err(err) => {
                // Revert the refused invalid/duplicate name in the Name field.
                designer.drafts.name = name.to_string();
                designer.set_error(err);
            }
        }
    }

    // Grid Row / Column
    if designer.model.is_in_root_grid(&comp) {
        ui.add_space(tokens::XS);
        ui.horizontal(|ui| {
            ui.spacing_mut().item_spacing.x = tokens::XS;
            let cell_input_w = ((full_w - 110.0) * 0.5).clamp(36.0, 56.0);
            ui.label("Row");
            let r_resp = ui.add_sized(
                [cell_input_w, tokens::CONTROL],
                tokens::input(egui::TextEdit::singleline(&mut designer.drafts.row))
                    .id(egui::Id::new("app_designer_grid_row")),
            );
            ui.label("Col");
            let c_resp = ui.add_sized(
                [cell_input_w, tokens::CONTROL],
                tokens::input(egui::TextEdit::singleline(&mut designer.drafts.col))
                    .id(egui::Id::new("app_designer_grid_col")),
            );
            let apply = ui
                .add_sized([38.0, tokens::CONTROL], egui::Button::new("Set"))
                .clicked()
                || ((r_resp.lost_focus() || c_resp.lost_focus())
                    && ui.input(|i| i.key_pressed(egui::Key::Enter)));
            if apply {
                match parse_span_input(&designer.drafts.row)
                    .and_then(|r| parse_span_input(&designer.drafts.col).map(|c| (r, c)))
                {
                    Ok((r, c)) => match designer.model.move_component_grid(name, r, c) {
                        Ok(()) => {
                            designer.mark_dirty();
                            designer.clear_status();
                            designer.sync_drafts(true);
                        }
                        Err(err) => {
                            designer.drafts.row =
                                comp.grid_row.map(format_span_input).unwrap_or_default();
                            designer.drafts.col =
                                comp.grid_column.map(format_span_input).unwrap_or_default();
                            designer.set_error(err);
                        }
                    },
                    Err(err) => {
                        designer.drafts.row =
                            comp.grid_row.map(format_span_input).unwrap_or_default();
                        designer.drafts.col =
                            comp.grid_column.map(format_span_input).unwrap_or_default();
                        designer.set_error(err);
                    }
                }
            }
        });
    }

    // Common properties
    if comp.kind.supports_text() {
        ui.add_space(tokens::XS);
        ui.label("Text");
        let resp = ui.add_sized(
            [full_w, tokens::CONTROL],
            tokens::input(egui::TextEdit::singleline(&mut designer.drafts.text))
                .id(egui::Id::new("app_designer_prop_text")),
        );
        if resp.changed() {
            match designer.model.set_property(
                name,
                "Text",
                AppPropertyValue::Text(designer.drafts.text.clone()),
            ) {
                Ok(()) => {
                    designer.mark_dirty();
                    designer.clear_status();
                }
                Err(err) => designer.set_error(err),
            }
        }
    }

    if comp.kind.supports_title() {
        ui.add_space(tokens::XS);
        ui.label("Title");
        let resp = ui.add_sized(
            [full_w, tokens::CONTROL],
            tokens::input(egui::TextEdit::singleline(&mut designer.drafts.title))
                .id(egui::Id::new("app_designer_prop_title")),
        );
        if resp.changed() {
            match designer.model.set_property(
                name,
                "Title",
                AppPropertyValue::Text(designer.drafts.title.clone()),
            ) {
                Ok(()) => {
                    designer.mark_dirty();
                    designer.clear_status();
                }
                Err(err) => designer.set_error(err),
            }
        }
    }

    if comp.kind.supports_items() {
        ui.add_space(tokens::XS);
        ui.label("Items (comma-separated)");
        let resp = ui.add_sized(
            [full_w, tokens::CONTROL],
            tokens::input(egui::TextEdit::singleline(&mut designer.drafts.items))
                .id(egui::Id::new("app_designer_prop_items")),
        );
        if resp.lost_focus() || resp.changed() {
            let items: Vec<String> = designer
                .drafts
                .items
                .split(',')
                .map(|s| s.trim().to_string())
                .filter(|s| !s.is_empty())
                .collect();
            if !items.is_empty() {
                match designer
                    .model
                    .set_property(name, "Items", AppPropertyValue::Lines(items))
                {
                    Ok(()) => {
                        designer.mark_dirty();
                        designer.clear_status();
                    }
                    Err(err) => designer.set_error(err),
                }
            }
        }
    }

    if comp.kind.supports_limits() {
        ui.add_space(tokens::XS);
        ui.label("Limits [min, max]");
        ui.horizontal(|ui| {
            ui.spacing_mut().item_spacing.x = tokens::XS;
            let half = ((full_w - tokens::XS) * 0.5).max(48.0);
            let r1 = ui.add_sized(
                [half, tokens::CONTROL],
                tokens::input(egui::TextEdit::singleline(&mut designer.drafts.limits_min))
                    .id(egui::Id::new("app_designer_prop_lim_min")),
            );
            let r2 = ui.add_sized(
                [half, tokens::CONTROL],
                tokens::input(egui::TextEdit::singleline(&mut designer.drafts.limits_max))
                    .id(egui::Id::new("app_designer_prop_lim_max")),
            );
            if r1.changed() || r2.changed() {
                if let (Ok(lo), Ok(hi)) = (
                    designer.drafts.limits_min.trim().parse::<f64>(),
                    designer.drafts.limits_max.trim().parse::<f64>(),
                ) {
                    match designer.model.set_property(
                        name,
                        "Limits",
                        AppPropertyValue::Limits([lo, hi]),
                    ) {
                        Ok(()) => {
                            designer.mark_dirty();
                            designer.clear_status();
                        }
                        Err(err) => designer.set_error(err),
                    }
                } else {
                    designer.set_error("Limits must be numeric [min, max].");
                }
            }
        });
    }

    if comp.kind.supports_column_name() {
        ui.add_space(tokens::XS);
        ui.label("ColumnName (comma-separated)");
        let resp = ui.add_sized(
            [full_w, tokens::CONTROL],
            tokens::input(egui::TextEdit::singleline(&mut designer.drafts.column_name))
                .id(egui::Id::new("app_designer_prop_cols")),
        );
        if resp.changed() {
            let cols: Vec<String> = designer
                .drafts
                .column_name
                .split(',')
                .map(|s| s.trim().to_string())
                .filter(|s| !s.is_empty())
                .collect();
            match designer
                .model
                .set_property(name, "ColumnName", AppPropertyValue::Lines(cols))
            {
                Ok(()) => {
                    designer.mark_dirty();
                    designer.clear_status();
                }
                Err(err) => designer.set_error(err),
            }
        }
    }

    if let Some(vk) = comp.kind.value_kind() {
        ui.add_space(tokens::XS);
        ui.label("Value");
        match vk {
            AppValueKind::Number => {
                let resp = ui.add_sized(
                    [full_w, tokens::CONTROL],
                    tokens::input(egui::TextEdit::singleline(&mut designer.drafts.value_num))
                        .id(egui::Id::new("app_designer_prop_val_num")),
                );
                if resp.changed() {
                    match designer.drafts.value_num.trim().parse::<f64>() {
                        Ok(v) => {
                            match designer.model.set_property(
                                name,
                                "Value",
                                AppPropertyValue::Number(v),
                            ) {
                                Ok(()) => {
                                    designer.mark_dirty();
                                    designer.clear_status();
                                }
                                Err(err) => designer.set_error(err),
                            }
                        }
                        Err(_) => {
                            designer.set_error("Value must be a finite number.");
                        }
                    }
                }
            }
            AppValueKind::Text => {
                let resp = ui.add_sized(
                    [full_w, tokens::CONTROL],
                    tokens::input(egui::TextEdit::singleline(&mut designer.drafts.value_text))
                        .id(egui::Id::new("app_designer_prop_val_text")),
                );
                if resp.changed() {
                    match designer.model.set_property(
                        name,
                        "Value",
                        AppPropertyValue::Text(designer.drafts.value_text.clone()),
                    ) {
                        Ok(()) => {
                            designer.mark_dirty();
                            designer.clear_status();
                        }
                        Err(err) => designer.set_error(err),
                    }
                }
            }
            AppValueKind::Bool => {
                let mut b = matches!(comp.props.value, Some(AppValue::Bool(true)));
                if ui.checkbox(&mut b, "Checked / On").changed()
                    && designer
                        .model
                        .set_property(name, "Value", AppPropertyValue::Bool(b))
                        .is_ok()
                {
                    designer.mark_dirty();
                    designer.clear_status();
                }
            }
            AppValueKind::Lines => {
                let resp = ui.add(
                    egui::TextEdit::multiline(&mut designer.drafts.value_lines)
                        .id(egui::Id::new("app_designer_prop_val_lines"))
                        .desired_rows(3)
                        .desired_width(full_w),
                );
                if resp.changed() {
                    let lines: Vec<String> = designer
                        .drafts
                        .value_lines
                        .split('\n')
                        .map(|l| l.strip_suffix('\r').unwrap_or(l).to_string())
                        .collect();
                    match designer
                        .model
                        .set_property(name, "Value", AppPropertyValue::Lines(lines))
                    {
                        Ok(()) => {
                            designer.mark_dirty();
                            designer.clear_status();
                        }
                        Err(err) => designer.set_error(err),
                    }
                }
            }
        }
    }
}

fn show_callbacks_section(designer: &mut AppDesigner, ui: &mut egui::Ui) {
    let p = tokens::palette(ui.ctx());
    let read_only = designer.read_only.is_some();

    ui.label(
        RichText::new("CALLBACKS")
            .font(crate::panels::caption_semibold(ui))
            .color(p.text_muted),
    );
    ui.add_space(tokens::XS);

    // Add-callback buttons for currently selected component (or UIFigure)
    let (target_name, supported_events): (String, &'static [&'static str]) =
        match &designer.selection {
            Selection::Figure => (designer.model.figure.name.clone(), &["CloseRequestFcn"]),
            Selection::Component(name) => {
                let events = designer
                    .model
                    .component(name)
                    .map(|c| c.kind.supported_events())
                    .unwrap_or(&[]);
                (name.clone(), events)
            }
        };

    ui.add_enabled_ui(!read_only, |ui| {
        for &ev in supported_events {
            let exists = designer
                .model
                .callbacks
                .iter()
                .any(|cb| cb.component == target_name && cb.event == ev);
            if !exists && ui.button(format!("+ {ev}")).clicked() {
                designer.add_callback_for(&target_name, ev);
            }
        }
    });

    ui.add_space(tokens::XS);

    let callbacks = designer.model.callbacks.clone();
    if callbacks.is_empty() {
        tokens::helper(ui, "No callbacks yet.");
        return;
    }

    for cb in callbacks {
        let missing_target = cb.component != designer.model.figure.name
            && designer.model.component(&cb.component).is_none();
        egui::Frame::new()
            .fill(p.surface)
            .stroke(egui::Stroke::new(
                1.0,
                if missing_target { p.error } else { p.hairline },
            ))
            .corner_radius(tokens::CONTROL_RADIUS)
            .inner_margin(egui::Margin::same(tokens::XS as i8))
            .show(ui, |ui| {
                ui.add(
                    egui::Label::new(
                        RichText::new(&cb.method_name)
                            .strong()
                            .color(if missing_target { p.error } else { p.text }),
                    )
                    .wrap(),
                );
                ui.add(
                    egui::Label::new(
                        RichText::new(format!("{}.{}", cb.component, cb.event))
                            .size(tokens::CAPTION)
                            .color(if missing_target {
                                p.error
                            } else {
                                p.text_muted
                            }),
                    )
                    .wrap(),
                );
                if missing_target {
                    ui.add(
                        egui::Label::new(
                            RichText::new("Target component was deleted")
                                .size(tokens::CAPTION)
                                .color(p.error),
                        )
                        .wrap(),
                    );
                }

                // Inline rename input if active for this callback
                if designer.drafts.cb_rename_target.as_deref() == Some(cb.method_name.as_str()) {
                    ui.horizontal(|ui| {
                        ui.spacing_mut().item_spacing.x = tokens::XS;
                        let field_w = (ui.available_width() - 96.0).max(60.0);
                        let resp = ui.add_sized(
                            [field_w, tokens::CONTROL],
                            tokens::input(egui::TextEdit::singleline(
                                &mut designer.drafts.cb_rename_draft,
                            ))
                            .id(egui::Id::new(("app_designer_cb_rename", &cb.method_name))),
                        );
                        let ok = ui.button("OK").clicked()
                            || (resp.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter)));
                        if ok {
                            let new_name = designer.drafts.cb_rename_draft.trim().to_string();
                            match designer.model.rename_callback(&cb.method_name, &new_name) {
                                Ok(()) => {
                                    designer.drafts.cb_rename_target = None;
                                    designer.mark_dirty();
                                    designer.clear_status();
                                }
                                Err(err) => {
                                    designer.set_error(err);
                                }
                            }
                        }
                        if ui.button("Cancel").clicked() {
                            designer.drafts.cb_rename_target = None;
                            designer.clear_status();
                        }
                    });
                }

                ui.add_enabled_ui(!read_only, |ui| {
                    ui.horizontal_wrapped(|ui| {
                        ui.spacing_mut().item_spacing.x = tokens::XS;
                        if ui.button("Code").clicked() {
                            designer.view = DesignerView::Code;
                            designer.focus_callback = Some(cb.method_name.clone());
                        }
                        if ui.button("Rename").clicked() {
                            designer.drafts.cb_rename_target = Some(cb.method_name.clone());
                            designer.drafts.cb_rename_draft = cb.method_name.clone();
                            designer.clear_status();
                        }
                        if ui.button("Remove").clicked()
                            && designer.model.remove_callback(&cb.method_name).is_ok()
                        {
                            designer.mark_dirty();
                            designer.clear_status();
                        }
                    });
                });
            });
        ui.add_space(tokens::XS);
    }
}
