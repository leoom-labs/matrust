use super::{Model, egui, model_canvas::Canvas};
use crate::theme::ThemedScrollArea;

#[derive(Default)]
pub(super) struct Entry {
    name: String,
    kind: &'static str,
    pub label: String,
    search: String,
}

#[derive(Default)]
pub(super) struct List {
    pub entries: Vec<Entry>,
    query: String,
    applied: String,
    visible: Vec<usize>,
    selected: Option<usize>,
    offset: f32,
    height: f32,
    reveal: bool,
    #[cfg(test)]
    rows_drawn: usize,
}

impl List {
    pub fn sync(&mut self, model: &Model) {
        let count = model.nodes.len().min(512);
        let mut changed = count != self.entries.len();
        self.entries.resize_with(count, Entry::default);
        for (index, (entry, node)) in self.entries.iter_mut().zip(&model.nodes).enumerate() {
            let kind = node.block.label();
            if entry.name != node.name || entry.kind != kind {
                entry.name.clone_from(&node.name);
                entry.kind = kind;
                entry.label = format!("{}. {}", index + 1, node.name);
                entry.search = format!("{} {}", node.name, kind).to_lowercase();
                changed = true;
            }
        }
        if changed || self.query != self.applied {
            self.applied.clone_from(&self.query);
            let query = self.query.trim().to_lowercase();
            self.visible.clear();
            self.visible
                .extend(self.entries.iter().enumerate().filter_map(|(i, entry)| {
                    (query.is_empty() || entry.search.contains(&query)).then_some(i)
                }));
            self.reveal = true;
        }
    }

    pub fn show(
        &mut self,
        ui: &mut egui::Ui,
        model: &Model,
        canvas: &mut Canvas,
        selected: &mut usize,
    ) {
        ui.horizontal(|ui| {
            ui.add(
                egui::TextEdit::singleline(&mut self.query)
                    .id_salt("block_search")
                    .hint_text("Find blocks by name or type")
                    .char_limit(128)
                    .desired_width((ui.available_width() - 52.).max(40.)),
            );
            if ui
                .add_enabled(!self.query.is_empty(), egui::Button::new("Clear"))
                .clicked()
            {
                self.query.clear();
            }
        });
        self.sync(model);
        ui.small(format!(
            "{} of {} blocks",
            self.visible.len(),
            model.nodes.len()
        )).on_hover_text("Search filters this list only. Shift selects a range in model order, including hidden blocks.");
        if self.entries.get(*selected).is_some() && !self.visible.contains(selected) {
            ui.small("Active block hidden by search; parameters still apply.");
        }
        if self.visible.is_empty() {
            ui.label(if model.nodes.is_empty() {
                "No blocks yet"
            } else {
                "No matching blocks"
            });
        }
        let row_height = ui
            .text_style_height(&egui::TextStyle::Button)
            .max(ui.spacing().interact_size.y);
        let stride = row_height + ui.spacing().item_spacing.y;
        let mut scroll = egui::ScrollArea::vertical()
            .id_salt("block_list")
            .max_height(170.)
            .animated(false);
        if self.reveal || self.selected != Some(*selected) {
            let offset = if let Some(index) = self.visible.iter().position(|i| i == selected) {
                let top = index as f32 * stride;
                let bottom = top + row_height;
                let height = if self.height > 0. { self.height } else { 170. }.max(row_height);
                if top < self.offset {
                    top
                } else if bottom > self.offset + height {
                    bottom - height
                } else {
                    self.offset
                }
            } else {
                0.
            };
            scroll = scroll.vertical_scroll_offset(offset);
        }
        self.reveal = false;
        self.selected = Some(*selected);
        #[cfg(test)]
        {
            self.rows_drawn = 0;
        }
        let output = scroll.show_rows_themed(ui, row_height, self.visible.len(), |ui, range| {
            for row in range {
                let index = self.visible[row];
                let entry = &self.entries[index];
                #[cfg(test)]
                {
                    self.rows_drawn += 1;
                }
                let response = ui
                    .push_id(index, |ui| {
                        ui.add_sized(
                            [ui.available_width(), row_height],
                            egui::Button::selectable(canvas.is_selected(index), &entry.label)
                                .wrap_mode(egui::TextWrapMode::Truncate),
                        )
                    })
                    .inner
                    .on_hover_text(format!("{} ({})", entry.name, entry.kind));
                if response.clicked() {
                    canvas.select(
                        index,
                        ui.input(|i| i.modifiers),
                        model.nodes.len(),
                        selected,
                    );
                    canvas.reveal(index);
                    canvas.focus(ui.ctx());
                }
            }
        });
        self.offset = output.state.offset.y;
        self.height = output.inner_rect.height();
    }
}

#[cfg(test)]
#[path = "model_list_tests.rs"]
mod tests;
