//! Figures mode navigation and a contextual, bounded Inspector.
use super::*;

pub(super) fn panels(app: &mut Workspace, root: &mut egui::Ui) {
    let ctx = root.ctx().clone();
    let compact = tokens::compact(&ctx);
    egui::Panel::left("figure_navigator")
        .default_size(
            app.layout
                .sidebar_width
                .unwrap_or(if compact { 200. } else { 240. }),
        )
        .size_range(170.0..=300.)
        .resizable(true)
        .frame(tokens::side_pane(&ctx))
        .show_pane(root, |ui| navigator(app, ui));
    if let Some(size) = layout::resized_panel(&ctx, "figure_navigator") {
        app.layout.sidebar_width = Some(size.x);
    }
    if !compact {
        egui::Panel::right("figure_inspector")
            .default_size(app.layout.workspace_width.unwrap_or(300.))
            .size_range(280.0..=360.)
            .resizable(true)
            .frame(tokens::side_pane(&ctx))
            .show_pane(root, |ui| inspector(app, ui, true));
        if let Some(size) = layout::resized_panel(&ctx, "figure_inspector") {
            app.layout.workspace_width = Some(size.x);
        }
    } else {
        let key = egui::Id::new("figure_inspector_open");
        let mut open = ctx.data(|d| d.get_temp::<bool>(key).unwrap_or(false));
        if open
            && !app.file_action_pending()
            && ctx.input_mut(|i| i.consume_key(egui::Modifiers::NONE, egui::Key::Escape))
        {
            open = false;
        }
        if open {
            let area = root.available_rect_before_wrap();
            egui::Area::new(egui::Id::new("figure_inspector_sheet"))
                .order(egui::Order::Foreground)
                .fixed_pos(egui::pos2(area.right() - 320., area.top()))
                .movable(false)
                .show(&ctx, |ui| {
                    if !root.is_enabled() {
                        ui.disable();
                    }
                    egui::Frame::new()
                        .fill(tokens::palette(&ctx).surface_sunken)
                        .stroke(egui::Stroke::new(1., tokens::palette(&ctx).border))
                        .inner_margin(16)
                        .show(ui, |ui| {
                            ui.set_width(286.);
                            ui.set_max_height((area.height() - 68.).max(80.));
                            ui.horizontal(|ui| {
                                ui.label(
                                    egui::RichText::new("INSPECTOR")
                                        .font(panels::caption_semibold(ui)),
                                );
                                ui.with_layout(
                                    egui::Layout::right_to_left(egui::Align::Center),
                                    |ui| {
                                        if ui.button("Close").clicked() {
                                            open = false;
                                        }
                                    },
                                );
                            });
                            inspector(app, ui, false);
                        });
                });
        }
        ctx.data_mut(|d| d.insert_temp(key, open));
    }
}

fn navigator(app: &mut Workspace, ui: &mut egui::Ui) {
    modes::section_caption(ui, "FIGURES");
    let enabled = !app.busy && !app.file_action_pending() && !app.allow_close;
    let mut picked = None;
    let mut axes_pick = None;
    egui::ScrollArea::vertical()
        .id_salt("figure_tree")
        .auto_shrink([false, false])
        .show_themed(ui, |ui| {
            ui.spacing_mut().item_spacing.y = 0.;
            if app.figure_ids.is_empty() {
                egui::Frame::new().inner_margin(12).show(ui, |ui| {
                    ui.label("No figures yet");
                    ui.weak("Create a figure or run a plotting command.");
                });
            }
            for &id in &app.figure_ids {
                let selected = id == app.current_figure;
                let (name, detail) = app.figure_choice_label(id);
                let label = format!("Figure {name}");
                let (row, _) = modes::nav_row(ui, &label, selected, false, enabled);
                if row.on_hover_text(detail).clicked() {
                    picked = Some(id);
                }
                if selected && let Some(figure) = &app.figure {
                    let mut axes: Vec<_> = figure.all_axes().collect();
                    axes.sort_by_key(|(position, _)| *position);
                    for (index, (position, axes)) in axes.into_iter().enumerate() {
                        let title: String = axes
                            .title
                            .chars()
                            .take(64)
                            .map(|c| if c.is_control() { ' ' } else { c })
                            .collect();
                        let label = if title.is_empty() {
                            format!("    Axes {}", index + 1)
                        } else {
                            format!("    Axes {} · {title}", index + 1)
                        };
                        let (row, _) = modes::nav_row(
                            ui,
                            &label,
                            position == figure.active_position(),
                            false,
                            enabled && !app.detached.contains(id),
                        );
                        if row.on_hover_text(&axes.title).clicked() {
                            axes_pick = Some(position);
                        }
                    }
                }
            }
        });
    if let Some(id) = picked {
        app.document_save.activate(document_save::Pane::Figure);
        if app.detached.contains(id) {
            app.focus_detached(id, ui.ctx());
        } else if id != app.current_figure {
            app.run(format!("figure({id})"));
        }
    } else if let Some(position) = axes_pick
        && let Some(original) = &app.figure
        && position != original.active_position()
    {
        // Swap existing metadata exactly as the prepared canvas does. The
        // worker validates the candidate; immutable sample storage stays shared.
        let mut updated = original.clone();
        if let Some(layout) = &mut updated.layout
            && let Some(index) = layout.others.iter().position(|p| p.position == position)
        {
            std::mem::swap(&mut updated.axes, &mut layout.others[index].axes);
            std::mem::swap(&mut layout.active, &mut layout.others[index].position);
            layout.others.sort_by_key(|p| p.position);
            app.submit(Job::ViewFigure(Box::new(worker::FigureEdit {
                id: app.current_figure,
                original: original.clone(),
                updated,
                view_before: None,
            })));
        }
    }
}

fn inspector(app: &mut Workspace, ui: &mut egui::Ui, caption: bool) {
    ui.spacing_mut().interact_size.y = tokens::CONTROL;
    ui.spacing_mut().button_padding = egui::vec2(12., 4.);
    if caption {
        modes::section_caption(ui, "INSPECTOR");
    }
    egui::ScrollArea::vertical()
        .id_salt("figure_properties_summary")
        .auto_shrink([false, false])
        .show_themed(ui, |ui| {
            egui::Frame::new().inner_margin(12).show(ui, |ui| {
                let Some(figure) = &app.figure else {
                    ui.label("Select a figure to inspect its axes.");
                    return;
                };
                ui.label(
                    egui::RichText::new(format!("Figure {}", app.current_figure))
                        .font(panels::semibold_font(ui, tokens::HEADING)),
                );
                ui.add_space(8.);
                ui.label(egui::RichText::new("ACTIVE AXES").font(panels::caption_semibold(ui)));
                ui.add_space(8.);
                let mut positions: Vec<_> = figure.all_axes().map(|(p, _)| p).collect();
                positions.sort();
                let index = positions
                    .iter()
                    .position(|p| *p == figure.active_position())
                    .unwrap_or(0)
                    + 1;
                ui.label(if positions.is_empty() {
                    "No axes yet".into()
                } else {
                    format!("Axes {index} of {}", positions.len())
                });
                if !figure.title.is_empty() {
                    ui.add(egui::Label::new(format!("Title: {}", figure.title)).truncate())
                        .on_hover_text(&figure.title);
                }
                ui.add_space(8.);
                egui::Grid::new("figure_axes_summary")
                    .num_columns(2)
                    .min_row_height(16.)
                    .spacing(egui::vec2(12., 8.))
                    .show(ui, |ui| {
                        for (label, value) in [
                            ("X scale", format!("{:?}", figure.xscale)),
                            ("Y scale", format!("{:?}", figure.yscale)),
                            ("X limits", limits(figure.xlim)),
                            ("Y limits", limits(figure.ylim)),
                            ("Series", figure.series.len().to_string()),
                        ] {
                            ui.weak(label);
                            ui.add(
                                egui::Label::new(egui::RichText::new(value).monospace()).truncate(),
                            );
                            ui.end_row();
                        }
                    });
                ui.add_space(8.);
                let action = toolstrip::Action::Properties;
                if ui
                    .add_enabled(
                        action.enabled(app),
                        egui::Button::new("Edit axes properties…"),
                    )
                    .clicked()
                {
                    action.invoke(app, ui.ctx());
                }
                ui.add_space(8.);
                ui.separator();
                ui.add_space(8.);
                ui.label(egui::RichText::new("FIGURE FILE").font(panels::caption_semibold(ui)));
                ui.add_space(8.);
                let path = app
                    .figure_documents
                    .get(&app.current_figure)
                    .and_then(|d| d.path.as_ref());
                let name = path
                    .map(|p| crate::path_label::display(p).to_string())
                    .unwrap_or_else(|| "Not saved to a file".into());
                ui.add(egui::Label::new(&name).truncate())
                    .on_hover_text(&name);
                ui.label(if app.figure_dirty(app.current_figure) {
                    "Unsaved changes"
                } else {
                    "No unsaved changes"
                });
                ui.add_space(8.);
                ui.horizontal_wrapped(|ui| {
                    for action in [
                        toolstrip::Action::SaveFigure,
                        toolstrip::Action::ExportFigure,
                    ] {
                        if ui
                            .add_enabled(action.enabled(app), egui::Button::new(action.label()))
                            .clicked()
                        {
                            action.invoke(app, ui.ctx());
                        }
                    }
                });
            });
        });
}

fn limits(range: Option<[f64; 2]>) -> String {
    range.map_or_else(|| "Auto".into(), |[a, b]| format!("{a:.4} … {b:.4}"))
}
