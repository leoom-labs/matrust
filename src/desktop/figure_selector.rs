use super::*;
use unicode_segmentation::UnicodeSegmentation;

// Bound label layout work, preserving whole graphemes; complete metadata remains
// available in the tooltip and the existing path caption below the toolbar.
fn short_name(text: &str) -> String {
    let mut result = String::new();
    for (index, cluster) in text.trim().graphemes(true).enumerate() {
        if index >= 64 || result.len() + cluster.len() > 256 {
            result.push('…');
            break;
        }
        for ch in cluster.chars() {
            result.push(if ch.is_control() { ' ' } else { ch });
        }
    }
    result
}

impl Workspace {
    pub(super) fn figure_choice_label(&self, id: u32) -> (String, String) {
        let path = self
            .figure_documents
            .get(&id)
            .and_then(|d| d.path.as_deref());
        let title = self.all_figures.get(&id).map_or("", |f| f.title.trim());
        let figure_name = self.all_figures.get(&id).map_or("", |f| f.name.trim());
        let name = path
            .and_then(|p| p.file_name())
            .map(|n| n.to_string_lossy());
        let name =
            name.as_deref()
                .filter(|n| !n.trim().is_empty())
                .unwrap_or(if title.is_empty() {
                    "Untitled figure"
                } else {
                    title
                });
        let dirty = self.figure_dirty(id);
        let label = format!(
            "{id}{} · {}",
            if dirty { " *" } else { "" },
            short_name(if figure_name.is_empty() {
                name
            } else {
                figure_name
            })
        );
        let mut details = format!(
            "Figure {id}{}",
            if dirty { " — unsaved changes" } else { "" }
        );
        if let Some(path) = path {
            details.push('\n');
            details.push_str(&crate::path_label::display(path).to_string());
        } else {
            details.push_str("\nNot saved to a file");
        }
        if !figure_name.is_empty() {
            details.push_str("\nName: ");
            details.push_str(figure_name);
        }
        if !title.is_empty() {
            details.push_str("\nTitle: ");
            details.push_str(title);
        }
        (label, details)
    }

    pub(super) fn named_figure_selector(&self, ui: &mut egui::Ui) -> Option<u32> {
        let mut picked = self.current_figure;
        let (_, details) = if self.figure_ids.contains(&picked) {
            self.figure_choice_label(picked)
        } else {
            ("No figure".into(), "Open or create a figure".into())
        };
        // ComboBox::width is a MINIMUM in egui. A bounded child UI plus truncate
        // limits the actual collapsed width; wrapping keeps adjacent actions usable.
        let width = ui.available_width().clamp(60., 104.);
        let popup_width = (ui.ctx().content_rect().width() - 32.).clamp(80., 420.);
        let popup_height = (ui.ctx().content_rect().height() * 0.65).clamp(64., 320.);
        ui.add_enabled_ui(
            !self.busy && !self.file_action_pending() && !self.figure_ids.is_empty(),
            |ui| {
                ui.allocate_ui_with_layout(
                    egui::vec2(width, ui.spacing().interact_size.y),
                    egui::Layout::left_to_right(egui::Align::Center),
                    |ui| {
                        // Explicit screenshot-only popup capture; no pointer/selection
                        // or file operation is synthesized. Opens only after recovery
                        // and queued opens have finished.
                        if self.screenshot_figure_selector_popup
                            && self.screenshot_path.is_some()
                            && !self.busy
                            && !self.figure_ids.is_empty()
                            && !self.recovering_session
                            && self.open_queue.is_empty()
                            && !self.file_action_pending()
                        {
                            // egui 0.36.2 stores IdSalt::new in ComboBox, then hashes
                            // that stored salt in show_ui. A raw string here names
                            // a different, unpainted popup that expires each frame.
                            let id = ui.make_persistent_id(egui::IdSalt::new("figure_number"));
                            if !egui::ComboBox::is_open(ui.ctx(), id) {
                                egui::Popup::open_id(ui.ctx(), id.with("popup"));
                                eprintln!(
                                    "Figure selector capture: popup opened; current={} figures={}",
                                    self.current_figure,
                                    self.figure_ids.len()
                                );
                            }
                        }
                        egui::ComboBox::from_id_salt("figure_number")
                            .selected_text("All figures")
                            .width(width)
                            .height(popup_height)
                            .truncate()
                            .show_ui(ui, |ui| {
                                ui.set_width(popup_width);
                                for &id in self.figure_ids.iter().take(16) {
                                    let (label, details) = self.figure_choice_label(id);
                                    if ui
                                        .add(
                                            egui::Button::selectable(picked == id, label)
                                                .truncate(),
                                        )
                                        .on_hover_text(details)
                                        .clicked()
                                    {
                                        picked = id;
                                    }
                                }
                            })
                            .response
                            .on_hover_text(details);
                    },
                );
            },
        );
        (picked != self.current_figure).then_some(picked)
    }
}

#[cfg(test)]
mod name_tests {
    use super::*;
    #[test]
    fn explicit_figure_name_is_distinct_from_file_and_axes_title() {
        let ctx = egui::Context::default();
        let mut app = Workspace::with_context(ctx, true);
        let figure = matrust::plot::Figure {
            name: "Named response".into(),
            axes: matrust::plot::Axes {
                title: "Axes title".into(),
                ..Default::default()
            },
            ..Default::default()
        };
        app.all_figures.insert(7, figure);
        let (label, detail) = app.figure_choice_label(7);
        assert!(label.ends_with("Named response"));
        assert!(!label.contains("Axes title"));
        assert!(detail.contains("Name: Named response") && detail.contains("Title: Axes title"));
        app.all_figures.get_mut(&7).unwrap().name = "x".repeat(4096);
        let (label, detail) = app.figure_choice_label(7);
        assert!(label.len() < 300);
        assert!(detail.contains(&"x".repeat(4096)));
    }
}
