//! Current Folder literal search; transient results never become session data.
use super::*;
use crate::theme::ThemedScrollArea;
use worker::text_search;

pub struct State {
    pub visible: bool,
    pub query: String,
    pub match_case: bool,
    pub subfolders: bool,
    generation: u64,
    pending: bool,
    requested: Option<text_search::Query>,
    results: Option<text_search::Results>,
    notice: String,
    action: Option<(u64, u64, Action)>,
    deferred: Option<worker::SearchOpened>,
    selected: Option<Selection>,
    reveal_selection: bool,
    key_frame: Option<u64>,
}
impl Default for State {
    fn default() -> Self {
        Self {
            visible: false,
            query: String::new(),
            match_case: true,
            subfolders: false,
            generation: 0,
            pending: false,
            requested: None,
            results: None,
            notice: String::new(),
            action: None,
            deferred: None,
            selected: None,
            reveal_selection: false,
            key_frame: None,
        }
    }
}
#[cfg(test)]
impl State {
    pub(super) fn hits(&self) -> &[text_search::Hit] {
        self.results.as_ref().map_or(&[], |r| r.hits.as_slice())
    }
    pub(super) fn notice(&self) -> &str {
        &self.notice
    }
    pub(super) fn selected_index(&self) -> Option<usize> {
        self.valid_selection().map(|s| s.index)
    }
}
#[derive(Clone, PartialEq, Eq)]
struct Selection {
    generation: u64,
    index: usize,
    path: std::sync::Arc<PathBuf>,
    bytes: std::ops::Range<usize>,
    raw_digest: blake3::Hash,
}
impl Selection {
    fn matches(&self, generation: u64, index: usize, hit: &text_search::Hit) -> bool {
        self.generation == generation
            && self.index == index
            && self.path == hit.path
            && self.bytes == hit.bytes
            && self.raw_digest == hit.raw_digest
    }
}
impl State {
    fn valid_selection(&self) -> Option<&Selection> {
        let selection = self.selected.as_ref()?;
        let hit = self.results.as_ref()?.hits.get(selection.index)?;
        selection
            .matches(self.generation, selection.index, hit)
            .then_some(selection)
    }
    fn select(&mut self, index: usize) {
        self.selected = self
            .results
            .as_ref()
            .and_then(|r| r.hits.get(index))
            .map(|hit| Selection {
                generation: self.generation,
                index,
                path: hit.path.clone(),
                bytes: hit.bytes.clone(),
                raw_digest: hit.raw_digest,
            });
        self.reveal_selection = self.selected.is_some();
    }
}
pub(super) fn list_id() -> egui::Id {
    egui::Id::new("folder_text_result_selection")
}
#[cfg(test)]
#[derive(Clone, Debug)]
pub(super) struct ResultGeometry {
    pub viewport: egui::Rect,
    pub parent_clip: egui::Rect,
    pub list: egui::Rect,
    pub bottom: f32,
    pub row: f32,
    pub stride: f32,
    pub selected: Option<(usize, egui::Rect, egui::Rect)>,
    pub scroll_id: egui::Id,
    pub scroll_offset: egui::Vec2,
    pub content_size: egui::Vec2,
    pub inner_rect: egui::Rect,
}
#[cfg(test)]
pub(super) fn result_geometry(ctx: &egui::Context) -> Option<String> {
    let geometry: ResultGeometry =
        ctx.data(|data| data.get_temp(egui::Id::new("folder_search_geometry_diagnostic")))?;
    Some(format!(
        "viewport={:?}, parent_clip={:?}, list={:?}, bottom={}, row={}, stride={}, selected={:?}, scroll_id={:?}, scroll_offset={:?}, content_size={:?}, inner_rect={:?}",
        geometry.viewport,
        geometry.parent_clip,
        geometry.list,
        geometry.bottom,
        geometry.row,
        geometry.stride,
        geometry.selected,
        geometry.scroll_id,
        geometry.scroll_offset,
        geometry.content_size,
        geometry.inner_rect
    ))
}

enum Action {
    Show(bool),
    Search,
    Stop,
    Open(usize, Option<Selection>),
}
impl Workspace {
    fn search_query(&self) -> text_search::Query {
        text_search::Query {
            folder: self.folder.clone(),
            text: self.folder_search.query.clone(),
            match_case: self.folder_search.match_case,
            subfolders: self.folder_search.subfolders,
        }
    }
    pub(super) fn cancel_folder_search(&mut self) {
        if self.folder_search.pending {
            self.worker.cancel.store(true, Ordering::Relaxed);
        }
        if self.folder_search.deferred.is_some() {
            self.document_save.open_completed();
        }
        self.folder_search.generation = self.folder_search.generation.wrapping_add(1);
        self.folder_search.pending = false;
        self.folder_search.requested = None;
        self.folder_search.results = None;
        self.folder_search.action = None;
        self.folder_search.deferred = None;
        self.folder_search.selected = None;
        self.folder_search.reveal_selection = false;
        self.folder_search.key_frame = None;
        self.folder_search.notice =
            "Search invalidated; choose Search to read current files".into();
    }
    fn search_context_allowed(&self, ctx: &egui::Context) -> bool {
        ctx.viewport_id() == egui::ViewportId::ROOT
            && self.navigation.left == Some(navigation::Left::Files)
            && !self.recovering_session
            && !self.allow_close
            && !self.pending_quit
            && !self.folder_browser.navigating()
            && self.open_queue.is_empty()
            && !egui::Popup::is_any_open(ctx)
            && ctx.memory(|m| m.top_modal_layer().is_none())
            && !ctx.input(|i| i.viewport().close_requested())
    }
    fn search_allowed(&self, ctx: &egui::Context) -> bool {
        self.search_context_allowed(ctx) && !self.file_action_pending()
    }
    /// "Search text" in the Files card header.
    pub(super) fn folder_search_link(&mut self, ui: &mut egui::Ui) {
        if panels::header_icon_button(
            ui,
            panels::Glyph::Search,
            "Search text",
            "Find literal text in .mr, .m and .txt files in Current Folder",
        )
        .clicked()
        {
            self.queue_search_action(ui.ctx(), Action::Show(true));
        }
    }
    fn queue_search_action(&mut self, ctx: &egui::Context, action: Action) {
        self.folder_search.action = Some((
            ctx.cumulative_frame_nr(),
            self.folder_search.generation,
            action,
        ));
    }
    pub(super) fn folder_search_ui(&mut self, ui: &mut egui::Ui, bottom: f32) -> bool {
        if !self.folder_search.visible {
            // The Files card header offers "Search text".
            return false;
        }
        shell::title(ui, "Search Current Folder");
        if ui.small_button("Back to files").clicked() {
            self.queue_search_action(ui.ctx(), Action::Show(false));
        }
        let mut changed = ui
            .add(
                egui::TextEdit::singleline(&mut self.folder_search.query)
                    .id(egui::Id::new("folder_text_query"))
                    .hint_text("Literal text in .m / .txt")
                    .char_limit(256)
                    .desired_width(ui.available_width()),
            )
            .changed();
        ui.horizontal_wrapped(|ui| {
            changed |= ui
                .checkbox(&mut self.folder_search.match_case, "Match case")
                .changed();
            changed |= ui
                .checkbox(&mut self.folder_search.subfolders, "Include subfolders")
                .changed();
        });
        if changed {
            self.cancel_folder_search();
        }
        ui.horizontal(|ui| {
            if ui
                .add_enabled(
                    !self.busy && !self.folder_search.query.is_empty(),
                    egui::Button::new("Search"),
                )
                .clicked()
            {
                self.queue_search_action(ui.ctx(), Action::Search);
            }
            if self.folder_search.pending && ui.button("Stop search").clicked() {
                self.queue_search_action(ui.ctx(), Action::Stop);
            }
        });
        // One line keeps the results list visible; the full notice is the
        // label's hover text when truncated.
        ui.add(egui::Label::new(&self.folder_search.notice).truncate());
        let height =
            (bottom - ui.cursor().top()).clamp(1., ui.ctx().content_rect().height().max(1.));
        let row = (ui.text_style_height(&egui::TextStyle::Button)
            + 2. * ui.spacing().button_padding.y)
            .max(ui.spacing().interact_size.y)
            .max(23.);
        let stride = row + ui.spacing().item_spacing.y;
        let rect =
            egui::Rect::from_min_size(ui.cursor().min, egui::vec2(ui.available_width(), height));
        ui.interact(rect,list_id(),egui::Sense::focusable_noninteractive())
            .on_hover_text("Tab to results; arrows, Home/End and Page Up/Down select; Enter opens the selected match");
        ui.memory_mut(|m| {
            m.set_focus_lock_filter(
                list_id(),
                egui::EventFilter {
                    vertical_arrows: true,
                    ..Default::default()
                },
            )
        });
        self.search_result_keys(ui.ctx(), (height / stride).floor().max(1.) as usize);
        let mut area = egui::ScrollArea::vertical()
            .id_salt("folder_text_results")
            .min_scrolled_height(0.)
            .max_height(height)
            .auto_shrink([false, false]);
        if self.folder_search.reveal_selection {
            if let Some(selection) = self.folder_search.valid_selection() {
                area = area.vertical_scroll_offset(selection.index as f32 * stride);
            }
            self.folder_search.reveal_selection = false;
        }
        let mut chosen = None;
        #[cfg(test)]
        let mut selected_geometry = None;
        if let Some(results) = &self.folder_search.results {
            let _shown = area.show_rows_themed(ui, row, results.hits.len(), |ui, rows| {
                for index in rows {
                    let hit = &results.hits[index];
                    let relative = hit.path.strip_prefix(&*hit.root).unwrap_or(&hit.path);
                    let label = format!(
                        "{}:{}  {}",
                        crate::path_label::display(relative),
                        hit.line,
                        hit.context
                    );
                    let selected = self
                        .folder_search
                        .valid_selection()
                        .is_some_and(|s| s.index == index);
                    let response = ui.add_enabled(
                        !self.busy,
                        egui::Button::selectable(selected, egui::RichText::new(label))
                            .truncate()
                            .min_size(egui::vec2(0., row))
                            .sense(egui::Sense::CLICK),
                    );
                    #[cfg(test)]
                    if selected {
                        selected_geometry = Some((index, response.rect, ui.clip_rect()));
                    }
                    if response.clicked() {
                        chosen = Some(index);
                    }
                }
            });
            #[cfg(test)]
            {
                let geometry = ResultGeometry {
                    viewport: ui.ctx().content_rect(),
                    parent_clip: ui.clip_rect(),
                    list: rect,
                    bottom,
                    row,
                    stride,
                    selected: selected_geometry,
                    scroll_id: _shown.id,
                    scroll_offset: _shown.state.offset,
                    content_size: _shown.content_size,
                    inner_rect: _shown.inner_rect,
                };
                ui.ctx().data_mut(|data| {
                    data.insert_temp(egui::Id::new("folder_search_geometry_diagnostic"), geometry)
                });
            }
        }
        if let Some(index) = chosen {
            self.folder_search.select(index);
            self.queue_search_action(ui.ctx(), Action::Open(index, None));
        }
        true
    }
    fn search_result_keys(&mut self, ctx: &egui::Context, page: usize) {
        let frame = ctx.cumulative_frame_nr();
        if self.folder_search.key_frame == Some(frame)
            || !self.folder_search.visible
            || self.busy
            || !self.search_allowed(ctx)
            || self.folder_search.requested.as_ref() != Some(&self.search_query())
            || !ctx.memory(|m| m.has_focus(list_id()))
            || !ctx.input(|i| i.modifiers.is_none())
        {
            return;
        }
        self.folder_search.key_frame = Some(frame);
        let key = ctx.input_mut(|input| {
            [
                egui::Key::ArrowUp,
                egui::Key::ArrowDown,
                egui::Key::Home,
                egui::Key::End,
                egui::Key::PageUp,
                egui::Key::PageDown,
                egui::Key::Enter,
            ]
            .into_iter()
            .find(|key| input.consume_key(egui::Modifiers::NONE, *key))
        });
        let Some(key) = key else {
            return;
        };
        if matches!(key, egui::Key::ArrowUp | egui::Key::ArrowDown) {
            ctx.memory_mut(|m| m.move_focus(egui::FocusDirection::None));
        }
        let Some(last) = self
            .folder_search
            .results
            .as_ref()
            .and_then(|r| r.hits.len().checked_sub(1))
        else {
            return;
        };
        let selection = self.folder_search.valid_selection().cloned();
        if key == egui::Key::Enter {
            if let Some(selection) = selection {
                self.queue_search_action(ctx, Action::Open(selection.index, Some(selection)));
            }
            return;
        }
        let index = selection.map(|s| s.index);
        let next = match key {
            egui::Key::Home => 0,
            egui::Key::End => last,
            egui::Key::ArrowUp => index.map_or(last, |i| i.saturating_sub(1)),
            egui::Key::ArrowDown => index.map_or(0, |i| (i + 1).min(last)),
            egui::Key::PageUp => index.map_or(0, |i| i.saturating_sub(page)),
            egui::Key::PageDown => index.map_or(0, |i| i.saturating_add(page).min(last)),
            _ => return,
        };
        self.folder_search.select(next);
    }
    pub(super) fn finish_folder_search(&mut self, ctx: &egui::Context) {
        if ctx.will_discard() {
            return;
        }
        let action = self.folder_search.action.take();
        // A retained Stop owns this generation even if its reply arrived between passes.
        if action.as_ref().is_some_and(|(frame, generation, action)| {
            *frame == ctx.cumulative_frame_nr()
                && *generation == self.folder_search.generation
                && matches!(action, Action::Stop)
        }) && self.search_context_allowed(ctx)
        {
            if self.folder_search.visible
                && (self.folder_search.pending || self.folder_search.deferred.is_some())
            {
                self.cancel_folder_search();
            }
            return;
        }
        self.finish_search_open(ctx);
        let Some((frame, generation, action)) = action else {
            return;
        };
        if frame != ctx.cumulative_frame_nr()
            || generation != self.folder_search.generation
            || !self.search_allowed(ctx)
        {
            return;
        }
        match action {
            Action::Show(show) => {
                self.cancel_folder_search();
                self.folder_search.visible = show;
            }
            Action::Stop => {}
            Action::Search if !self.busy => {
                self.cancel_folder_search();
                let query = self.search_query();
                let generation = self.folder_search.generation;
                self.submit(Job::SearchFolder(Box::new(text_search::Request {
                    generation,
                    query: query.clone(),
                })));
                if self.busy {
                    self.folder_search.pending = true;
                    self.folder_search.requested = Some(query);
                    self.folder_search.notice = "Searching .mr, .m and .txt files…".into();
                }
            }
            Action::Open(index, selection) if !self.busy => {
                if let Some(selection) = selection
                    && (!self.folder_search.visible
                        || !ctx.memory(|m| m.has_focus(list_id()))
                        || self.folder_search.valid_selection() != Some(&selection))
                {
                    return;
                }
                if self.folder_search.requested.as_ref() != Some(&self.search_query()) {
                    return;
                }
                let Some(hit) = self
                    .folder_search
                    .results
                    .as_ref()
                    .and_then(|r| r.hits.get(index))
                    .cloned()
                else {
                    return;
                };
                let scripts = self
                    .documents
                    .iter()
                    .filter_map(|d| d.path.clone().map(|p| (d.id, p)))
                    .collect();
                let request = worker::SearchOpen {
                    generation,
                    query: self.search_query(),
                    hit: hit.clone(),
                    interaction: self.document_save.interaction_generation(),
                    open: open_files::Request {
                        path: hit.path.as_ref().clone(),
                        scripts,
                        tabs: self.documents.len(),
                    },
                };
                self.submit(Job::OpenSearchResult(Box::new(request)));
                if self.busy {
                    self.worker.opening = true;
                    self.folder_search.pending = true;
                    self.folder_search.notice = "Verifying result before opening…".into();
                }
            }
            _ => {}
        }
    }
    pub(super) fn receive_folder_search(&mut self, reply: text_search::Finished) {
        if reply.generation != self.folder_search.generation
            || reply.query != self.search_query()
            || self.worker.cancel.load(Ordering::Relaxed)
        {
            return;
        }
        self.folder_search.pending = false;
        match reply.result {
            Ok(results) => {
                let c = &results.counts;
                self.folder_search.notice = format!(
                    "{} matches · {} files · {} bytes. Skipped: {} unsupported, {} binary/encoding, {} oversized, {} links, {} unreadable, {} too deep.{}",
                    results.hits.len(),
                    c.files,
                    c.read_bytes,
                    c.unsupported,
                    c.binary,
                    c.oversized,
                    c.linked,
                    c.unreadable,
                    c.depth,
                    results.truncated.map_or(String::new(), |limit| format!(
                        " Partial results: {limit}; narrow the folder/query."
                    ))
                );
                self.folder_search.results = Some(results);
            }
            Err(error) => self.folder_search.notice = error,
        }
    }
    pub(super) fn receive_search_open(&mut self, reply: worker::SearchOpened) {
        self.worker.opening = false;
        if reply.generation == self.folder_search.generation
            && reply.query == self.search_query()
            && !self.worker.cancel.load(Ordering::Relaxed)
        {
            self.folder_search.pending = false;
            self.folder_search.deferred = Some(reply);
        } else {
            self.document_save.open_completed();
        }
    }
    fn finish_search_open(&mut self, ctx: &egui::Context) {
        let Some(reply) = self.folder_search.deferred.take() else {
            return;
        };
        if !self.search_allowed(ctx)
            || !self.folder_search.visible
            || reply.generation != self.folder_search.generation
            || reply.query != self.search_query()
            || reply.interaction != self.document_save.interaction_generation()
            || self.worker.cancel.load(Ordering::Relaxed)
        {
            self.document_save.open_completed();
            self.folder_search.notice =
                "Selection changed while opening; choose the result again".into();
            return;
        }
        let prepared = match reply.result {
            Ok(p) => p,
            Err(error) => {
                self.document_save.open_completed();
                self.folder_search.notice = error;
                return;
            }
        };
        let document = match &prepared {
            open_files::Prepared::ExistingScript(id, _) => self
                .documents
                .iter()
                .find(|d| d.id == *id && d.path.is_some() && d.path == reply.existing_path),
            open_files::Prepared::Script(document) => Some(&**document),
            _ => None,
        };
        let Some(document) = document else {
            self.document_save.open_completed();
            self.folder_search.notice = "Search target is no longer an openable script".into();
            return;
        };
        if document.text.len() != reply.hit.text_len
            || document.text.len() > text_search::MAX_FILE
            || blake3::hash(document.text.as_bytes()) != reply.hit.text_digest
            || document.text.get(reply.hit.bytes.clone()).is_none()
        {
            self.document_save.open_completed();
            self.folder_search.notice="Editor text differs from this disk result; edits retained. Search again after saving, or open the file from Files.".into();
            return;
        }
        let id = document.id;
        let range = matrust::editor::byte_chars(&document.text, reply.hit.bytes.clone());
        self.complete_open(Ok(prepared));
        if self.documents[self.active].id != id {
            self.folder_search.notice =
                "Open retained a different selection; choose the result again".into();
            return;
        }
        self.editor.jump = Some((id, range));
        self.reveal_editor();
        self.queue_pane_focus(ctx, navigation::FocusPane::Editor);
        self.folder_search.notice = format!(
            "Opened {} at line {}",
            crate::path_label::display(&reply.hit.path),
            reply.hit.line
        );
    }
}
