use super::*;
use crate::theme::ThemedScrollArea;
use matrust::files::MatCatalog;
use std::collections::BTreeSet;

fn memory_size(bytes: u64) -> String {
    if bytes < 1024 {
        format!("{bytes} B")
    } else if bytes < 1048576 {
        format!("{:.2} KiB", bytes as f64 / 1024.)
    } else {
        format!("{:.2} MiB", bytes as f64 / 1048576.)
    }
}

pub struct Import {
    catalog: MatCatalog,
    pub(super) selected: BTreeSet<String>,
    pub loading: bool,
    pub error: Option<String>,
    query: String,
    applied_query: String,
    search: Vec<String>,
    visible: Vec<usize>,
    #[cfg(test)]
    rows_drawn: usize,
}
impl Import {
    pub fn new(catalog: MatCatalog) -> Self {
        let visible = (0..catalog.variables().len()).collect();
        let search = catalog
            .variables()
            .map(|v| {
                format!(
                    "{} {}{}",
                    v.name,
                    if v.complex { "complex " } else { "" },
                    v.class
                )
                .to_lowercase()
            })
            .collect();
        Self {
            visible,
            search,
            catalog,
            selected: BTreeSet::new(),
            loading: false,
            error: None,
            query: String::new(),
            applied_query: String::new(),
            #[cfg(test)]
            rows_drawn: 0,
        }
    }
    fn update_filter(&mut self) -> bool {
        if self.query == self.applied_query {
            return false;
        }
        self.applied_query.clone_from(&self.query);
        let query = self.query.trim().to_lowercase();
        self.visible.clear();
        self.visible.extend(
            self.search
                .iter()
                .enumerate()
                .filter_map(|(i, text)| (query.is_empty() || text.contains(&query)).then_some(i)),
        );
        true
    }
    fn select_matching(&mut self) {
        for &index in &self.visible {
            if let Some(variable) = self
                .catalog
                .variable(index)
                .filter(|v| v.unavailable.is_none())
            {
                self.selected.insert(variable.name.clone());
            }
        }
    }
}
impl Workspace {
    pub(super) fn workspace_import_ui(&mut self, ctx: &egui::Context) {
        let Some(import) = &mut self.workspace_import else {
            return;
        };
        let mut close = false;
        let mut job = None;
        let shown = egui::Modal::new(egui::Id::new("workspace_import")).show(ctx,|ui| {
            ui.set_width((ctx.content_rect().width() - 48.).clamp(300.,700.));
            ui.label(crate::tokens::title(ui.ctx(), "Import MAT workspace"));
            ui.add_enabled_ui(!self.busy, |ui| {
                ui.horizontal(|ui| {
                    ui.add(egui::TextEdit::singleline(&mut import.query).hint_text("Find variables by name or type").char_limit(128).desired_width((ui.available_width()-85.).max(60.)));
                    if ui.add_enabled(!import.query.is_empty(),egui::Button::new("Clear filter")).clicked() { import.query.clear(); }
                });
                ui.horizontal_wrapped(|ui| {
                    if ui.button(if import.query.trim().is_empty() { "Select supported" } else { "Select matches" }).on_hover_text("Add supported matching variables. Hidden selections stay selected.").clicked() { import.update_filter(); import.select_matching(); }
                    if ui.button("Clear selection").clicked() { import.selected.clear(); }
                });
            });
            let reset_scroll = import.update_filter();
            #[cfg(test)] { import.rows_drawn = 0; }
            egui::ScrollArea::vertical().id_salt("mat_import_body").min_scrolled_height(0.)
                .max_height((ctx.content_rect().height()-300.).clamp(24.,450.)).auto_shrink([false,true]).show_themed(ui,|ui| {
            ui.add(egui::Label::new(import.catalog.path().file_name().unwrap_or_default().to_string_lossy()).truncate()).on_hover_text(crate::path_label::display(import.catalog.path()).to_string());
            ui.label(format!("{} of {} variables; {} on disk",import.visible.len(),import.catalog.variables().len(),memory_size(import.catalog.file_bytes())));
            ui.label("Choose variables to load. Matching workspace names will be replaced.");
            if import.visible.is_empty() {ui.label("No matching variables");}
            ui.add_enabled_ui(!self.busy,|ui| {
                egui::ScrollArea::horizontal().id_salt("mat_import_columns").auto_shrink([false,true]).show_themed(ui,|ui| {
                // Reserve the full supported type name without taking space from
                // names/dimensions or shrinking the footer controls. Narrow panes
                // retain the existing horizontal table scroll.
                let type_width = ui.fonts_mut(|fonts| fonts.layout_no_wrap(
                    "complex double".into(), egui::TextStyle::Body.resolve(ui.style()),
                    ui.visuals().text_color(),
                )).size().x.ceil().max(94.) + 6.;
                ui.set_min_width(660. + type_width - 70.);
                ui.horizontal(|ui| {
                    for (width,label) in [(24.,""),(140.,"Name"),(110.,"Dimensions"),(type_width,"Type"),(85.,"Memory"),(180.,"Status")] {
                        ui.add_sized([width,20.],egui::Label::new(egui::RichText::new(label).strong()));
                    }
                });
                let mut rows = egui::ScrollArea::vertical().id_salt("mat_import_rows").max_height((ctx.content_rect().height()*0.3).clamp(60.,280.));
                if reset_scroll { rows = rows.vertical_scroll_offset(0.); }
                rows.show_rows_themed(ui,20.,import.visible.len(),|ui,range| {
                    for index in range {
                        let index=import.visible[index];
                        let variable=import.catalog.variable(index).expect("filtered catalog index");
                        #[cfg(test)] { import.rows_drawn += 1; }
                        ui.push_id(index,|ui|ui.horizontal(|ui| {
                            let mut selected=import.selected.contains(&variable.name);
                            if ui.add_enabled(variable.unavailable.is_none(),egui::Checkbox::without_text(&mut selected)).changed() {
                                if selected {import.selected.insert(variable.name.clone());} else {import.selected.remove(&variable.name);}
                            }
                            ui.add_sized([140.,20.],egui::Label::new(&variable.name).truncate()).on_hover_text(&variable.name);
                            let dimensions=variable.dimensions.iter().map(usize::to_string).collect::<Vec<_>>().join(" × ");
                            ui.add_sized([110.,20.],egui::Label::new(&dimensions).truncate()).on_hover_text(&dimensions);
                            let class = if variable.complex { format!("complex {}", variable.class) } else { variable.class.clone() };
                            ui.add_sized([type_width,20.],egui::Label::new(&class).truncate()).on_hover_text(&class);
                            ui.add_sized([85.,20.],egui::Label::new(variable.storage_bytes().map_or_else(||"Too large".into(),memory_size)));
                            let status=variable.unavailable.as_deref().unwrap_or(if self.variables.contains_key(&variable.name) {"Replaces workspace value"} else if variable.compressed {"Compressed"} else {"Ready"});
                            ui.add_sized([180.,20.],egui::Label::new(status).truncate()).on_hover_text(status);
                        }));
                    }
                });
            });
                });
                ui.small("Memory estimates include supported values and nested cell contents. Selected data are checked when loaded.");
            });
            let selected_bytes=import.catalog.variables().filter(|v|import.selected.contains(&v.name)).map(|v|v.storage_bytes().unwrap_or(0)).sum::<u64>();
            let retained=self.variables.iter().filter(|(name,_)|!import.selected.contains(*name)).map(|(_,m)|m.bytes() as u64).sum::<u64>();
            ui.separator();
            ui.label(format!("Selected: {} variables, {}",import.selected.len(),memory_size(selected_bytes)));
            ui.label(format!("Workspace after import: {} / 256 MiB",memory_size(selected_bytes+retained)));
            if !import.query.trim().is_empty() {
                let shown_selected=import.visible.iter().filter(|&&i| import.catalog.variable(i).is_some_and(|v| import.selected.contains(&v.name))).count();
                let hidden=import.selected.len().saturating_sub(shown_selected);
                if hidden>0 {ui.small(format!("{hidden} selected variables hidden by filter"));}
            }
            if let Some(error)=&import.error {
                egui::ScrollArea::vertical().id_salt("mat_import_error").min_scrolled_height(0.).max_height(48.).auto_shrink([false,true]).show_themed(ui,|ui| {ui.colored_label(tokens::palette(ui.ctx()).error,error);});
            }
            ui.horizontal_wrapped(|ui| {
                if !self.open_queue.is_empty() && ui.button("Cancel queued files").clicked() {self.open_queue.clear();}
                if ui.add_enabled(!self.busy && !import.selected.is_empty() && selected_bytes+retained<=256*1024*1024,egui::Button::new("Load selected")).clicked() {
                    job=Some(Job::LoadSelectedWorkspace(Box::new(import.catalog.clone()),import.selected.clone()));import.loading=true;import.error=None;
                }
                if ui.add_enabled(!self.busy,egui::Button::new("Refresh file")).clicked() {
                    job=Some(Job::InspectWorkspace(import.catalog.path().to_path_buf()));import.loading=true;import.error=None;
                }
                if self.busy {
                    ui.spinner();
                    if ui.button("Stop").clicked() {self.open_queue.clear();self.worker.cancel.store(true,Ordering::Relaxed);}
                } else if ui.button("Cancel").clicked() {close=true;}
            });
        });
        if shown.is_top_modal
            && !shown.any_popup_open
            && ctx.input_mut(|i| i.consume_key(egui::Modifiers::NONE, egui::Key::Escape))
        {
            if self.busy {
                self.open_queue.clear();
                self.worker.cancel.store(true, Ordering::Relaxed);
            } else {
                close = true;
            }
        }
        if close {
            self.workspace_import = None;
        }
        if let Some(job) = job {
            self.submit(job);
        }
    }
}

#[cfg(test)]
#[path = "workspace_import_search_tests.rs"]
mod search_tests;
