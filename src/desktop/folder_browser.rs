use super::*;
use crate::theme::ThemedScrollArea;

pub struct Browser {
    scanner: folder_scan::Scanner,
    generation: u64,
    pub loading: bool,
    pub filter: String,
    /// The Files header shows the filename field instead of the filter icon.
    pub filter_open: bool,
    filter_focus: bool,
    due: Option<Instant>,
    pub notice: String,
    navigation: Option<Option<usize>>,
    selection: Option<(usize, PathBuf)>,
    reveal_selection: bool,
    key_frame: Option<u64>,
    action: Option<Action>,
    focus: Option<SelectionFocus>,
}
/// Folder navigation buttons of the Files header.
#[derive(Clone, Copy, PartialEq)]
enum Nav {
    Back,
    Forward,
    Up,
    Refresh,
    Browse,
}
impl Browser {
    pub fn new(context: egui::Context) -> Self {
        Self {
            scanner: folder_scan::Scanner::new(context),
            generation: 0,
            loading: false,
            filter: String::new(),
            filter_open: false,
            filter_focus: false,
            due: None,
            notice: String::new(),
            navigation: None,
            selection: None,
            reveal_selection: false,
            key_frame: None,
            action: None,
            focus: None,
        }
    }
    pub(super) fn selected_path(&self) -> Option<&std::path::Path> {
        self.selection.as_ref().map(|(_, path)| path.as_path())
    }
    pub(super) fn cancel_action(&mut self) {
        self.action = None;
        self.focus = None;
    }
    fn clear_selection(&mut self) {
        self.selection = None;
        self.reveal_selection = false;
        self.action = None;
        self.focus = None;
    }
    pub fn navigating(&self) -> bool {
        self.navigation.is_some()
    }
    fn request(&mut self, path: PathBuf) {
        if self.navigation.is_some() {
            return;
        }
        self.action = None;
        self.focus = None;
        self.generation = self.scanner.request(path, self.filter.clone());
        self.loading = true;
        self.due = None;
        self.notice = "Reading folder…".into();
    }
    pub fn stop(&mut self) {
        self.action = None;
        self.focus = None;
        self.scanner.cancel();
        self.loading = false;
        self.due = None;
        self.notice = if self.navigation.take().is_some() {
            "Folder change cancelled"
        } else {
            "Folder scan stopped. Refresh to try again."
        }
        .into();
    }
}
// A single transient intent, bound to the exact listing and visible selection.
// No deferred file open survives a later frame, scan, pane switch or modal.
struct SelectionFocus {
    frame: u64,
    generation: u64,
    folder: PathBuf,
    selected: PathBuf,
}
struct Action {
    frame: u64,
    generation: u64,
    folder: PathBuf,
    selected: Option<PathBuf>,
    path: PathBuf,
    directory: bool,
}
pub(super) fn list_id() -> egui::Id {
    egui::Id::new("folder_file_selection")
}
impl Workspace {
    pub(super) fn resolve_folder(&mut self, path: PathBuf, index: Option<usize>) {
        let browser = &mut self.folder_browser;
        browser.clear_selection();
        browser.generation = browser.scanner.navigate(path);
        browser.navigation = Some(index);
        browser.loading = true;
        browser.due = None;
        browser.notice = "Opening folder…".into();
    }
    pub(super) fn refresh_folder(&mut self) {
        self.folder_browser.request(self.folder.clone());
    }
    pub(super) fn receive_folder(&mut self) {
        if let Some(reply) = self.folder_browser.scanner.take()
            && reply.generation == self.folder_browser.generation
        {
            self.folder_browser.loading = false;
            let navigation = self.folder_browser.navigation.take();
            match reply.result {
                Ok(listing) => {
                    if let (Some(index), Some(path)) = (navigation, reply.directory) {
                        self.commit_folder(path, index);
                    }
                    self.folder_browser.notice = if listing.truncated {
                        format!(
                            "{} shown · listing limit reached. Refine the filename filter.",
                            listing.entries.len()
                        )
                    } else if self.folder_browser.filter.is_empty() {
                        entry_count(listing.entries.len())
                    } else {
                        format!(
                            "{} matches · {} checked",
                            listing.entries.len(),
                            listing.visited
                        )
                    };
                    // Refresh preserves an existing selection by path, never by the old row index.
                    self.folder_browser.selection =
                        self.folder_browser.selected_path().and_then(|path| {
                            listing
                                .entries
                                .iter()
                                .position(|entry| entry.path == path)
                                .map(|index| (index, path.to_owned()))
                        });
                    self.folder_browser.action = None;
                    self.folder_browser.focus = None;
                    self.entries = listing.entries;
                }
                Err(error) => {
                    if navigation.is_some() {
                        self.status = format!("Could not open folder: {error}");
                    }
                    self.folder_browser.notice = format!("Could not read folder: {error}");
                }
            }
        }
    }
    /// Filename filter of the Files header: an icon until it is used, then
    /// the field. An emptied field folds back into the icon.
    fn folder_filter_control(&mut self, ui: &mut egui::Ui) {
        if !self.folder_browser.filter_open && self.folder_browser.filter.is_empty() {
            if panels::header_icon_button(
                ui,
                panels::Glyph::Filter,
                "Filter filenames",
                "Filter filenames",
            )
            .clicked()
            {
                self.folder_browser.filter_open = true;
                self.folder_browser.filter_focus = true;
            }
            return;
        }
        if panels::header_icon_button(
            ui,
            panels::Glyph::Filter,
            "Clear filename filter",
            "Clear filename filter",
        )
        .clicked()
        {
            self.folder_browser.filter.clear();
            self.folder_browser.filter_open = false;
            self.folder_browser.clear_selection();
            self.entries.clear();
            self.refresh_folder();
            return;
        }
        // The header packs its icons edge to edge; the field keeps a gap.
        ui.add_space(tokens::XS);
        let response = ui.add(
            egui::TextEdit::singleline(&mut self.folder_browser.filter)
                .id(egui::Id::new("folder_filter"))
                .hint_text(RichText::new("Filter…").color(tokens::palette(ui.ctx()).text_disabled))
                .char_limit(256)
                // Leave the title its room.
                .desired_width((ui.available_width() - 72.).clamp(40., 110.)),
        );
        ui.add_space(tokens::XS);
        if std::mem::take(&mut self.folder_browser.filter_focus) {
            response.request_focus();
        }
        if response.lost_focus() && self.folder_browser.filter.is_empty() {
            self.folder_browser.filter_open = false;
        }
        if response.changed() {
            self.folder_browser.scanner.cancel();
            self.folder_browser.loading = false;
            self.folder_browser.due = Some(Instant::now() + Duration::from_millis(250));
            self.folder_browser.notice = "Waiting for filename filter…".into();
            self.folder_browser.clear_selection();
            self.entries.clear();
        }
    }

    /// Folder navigation in the Files header, painted right to left: Back,
    /// Forward, Up, Refresh and Browse as icon buttons with tooltips. A narrow
    /// column keeps what fits after the title, Up, Back and Forward first;
    /// Refresh (F5) and Browse (the top-bar folder) are the first to go.
    fn folder_navigation_icons(&mut self, ui: &mut egui::Ui, title_room: f32) {
        let ctx = ui.ctx().clone();
        let idle = !self.busy && !self.file_action_pending();
        // n icons take n slots less the spacing after the last one.
        let spacing = ui.spacing().item_spacing.x;
        let slot = tokens::HEADER_ICON + spacing;
        let fit = ((ui.available_width() - title_room + spacing) / slot).floor();
        let priority = [Nav::Up, Nav::Back, Nav::Forward, Nav::Refresh, Nav::Browse];
        let shown = &priority[..(fit.max(0.) as usize).min(priority.len())];
        for nav in [Nav::Browse, Nav::Refresh, Nav::Up, Nav::Forward, Nav::Back] {
            if !shown.contains(&nav) {
                continue;
            }
            let (glyph, label, hint, enabled) = match nav {
                Nav::Back => (
                    panels::Glyph::Back,
                    "Back",
                    "Back · Alt+Left",
                    idle && self.folder_navigation.destination(false).is_some(),
                ),
                Nav::Forward => (
                    panels::Glyph::Forward,
                    "Forward",
                    "Forward · Alt+Right",
                    idle && self.folder_navigation.destination(true).is_some(),
                ),
                Nav::Up => (
                    panels::Glyph::Up,
                    "Up",
                    "Parent folder · Alt+Up",
                    idle && self.folder.parent().is_some(),
                ),
                Nav::Refresh => (
                    panels::Glyph::Refresh,
                    "Refresh",
                    "Refresh this folder · F5",
                    idle,
                ),
                Nav::Browse => (
                    panels::Glyph::Browse,
                    "Browse",
                    "Choose another folder…",
                    self.folder_browse_context(&ctx),
                ),
            };
            if !ui
                .add_enabled_ui(enabled, |ui| {
                    panels::header_icon_button(ui, glyph, label, hint)
                })
                .inner
                .clicked()
            {
                continue;
            }
            match nav {
                Nav::Back => self.step_folder(false),
                Nav::Forward => self.step_folder(true),
                Nav::Up => {
                    if let Some(parent) = self.folder.parent() {
                        self.change_folder(parent.to_owned());
                    }
                }
                Nav::Refresh => self.refresh_folder(),
                Nav::Browse => self.queue_folder_browse(&ctx),
            }
        }
    }

    /// Ctrl+L: a path field takes the place of the header tools; Enter goes
    /// there, Escape or a click elsewhere keeps the folder.
    fn folder_address_field(&mut self, ui: &mut egui::Ui, title_room: f32) {
        let idle = !self.busy && !self.file_action_pending();
        let edit = ui.add_enabled(
            idle,
            egui::TextEdit::singleline(&mut self.folder_text)
                .id(folder_navigation::address_id())
                .desired_width((ui.available_width() - title_room).max(40.)),
        );
        self.folder_address_focus(&edit);
        if edit.lost_focus() {
            if ui.input(|i| i.key_pressed(egui::Key::Enter)) {
                self.change_folder(PathBuf::from(&self.folder_text));
            } else {
                self.folder_text = folder_navigation::address_text(&self.folder);
            }
            self.folder_navigation.address_open = false;
        }
    }

    pub(super) fn folder_browser_ui(&mut self, ui: &mut egui::Ui, bottom: f32) {
        if self.folder_search_ui(ui, bottom) {
            return;
        }
        // Observe a deliberate competing owner before this pass recreates any
        // filter/list response that could surrender it on the old pointer release.
        if self.folder_browser.focus.is_some()
            && ui.memory(|memory| memory.focused().is_some_and(|id| id != list_id()))
        {
            self.folder_browser.focus = None;
        }
        // The plain entry count sits in the header; other notices keep a line.
        let plain = !self.folder_browser.loading
            && self.folder_browser.notice == entry_count(self.entries.len());
        let count = plain.then(|| self.folder_browser.notice.clone());
        panels::header(ui, "Files", count.as_deref(), |ui| {
            // The title stays whole: the tools take only the room it leaves.
            let title_room = panels::header_title_width(ui, "Files") + ui.spacing().item_spacing.x;
            ui.scope(|ui| {
                ui.spacing_mut().item_spacing.x = 0.;
                if self.folder_navigation.address_open {
                    self.folder_address_field(ui, title_room);
                    return;
                }
                self.folder_search_link(ui);
                ui.add_enabled_ui(self.folder_browser.navigation.is_none(), |ui| {
                    self.folder_filter_control(ui);
                });
                self.folder_navigation_icons(ui, title_room);
            });
        });
        if let Some(due) = self.folder_browser.due {
            if Instant::now() >= due {
                self.refresh_folder();
            } else {
                ui.ctx()
                    .request_repaint_after(due.saturating_duration_since(Instant::now()));
            }
        }
        if !plain {
            ui.horizontal_wrapped(|ui| {
                ui.add_space(tokens::M);
                if self.folder_browser.loading {
                    ui.spinner();
                    if ui
                        .small_button(if self.folder_browser.navigating() {
                            "Cancel folder change"
                        } else {
                            "Stop scan"
                        })
                        .clicked()
                    {
                        self.folder_browser.stop();
                    }
                }
                ui.weak(&self.folder_browser.notice);
            });
        }
        let height =
            (bottom - ui.cursor().top()).clamp(1., ui.ctx().content_rect().height().max(1.));
        let list_rect =
            egui::Rect::from_min_size(ui.cursor().min, egui::vec2(ui.available_width(), height));
        // One stable keyboard owner stays registered even when selected rows are virtualized away.
        let focus_area = ui.interact(
            list_rect,
            list_id(),
            egui::Sense::focusable_noninteractive(),
        );
        if self.folder_actions_allowed(ui.ctx()) {
            focus_areas::register(ui, &focus_area, focus_areas::Area::Files);
        }
        ui.memory_mut(|memory| {
            memory.set_focus_lock_filter(
                list_id(),
                egui::EventFilter {
                    vertical_arrows: true,
                    ..Default::default()
                },
            )
        });
        let row_height = if tokens::compact(ui.ctx()) {
            tokens::COMPACT_ROW
        } else {
            tokens::ROW
        }
        .max(ui.text_style_height(&egui::TextStyle::Button) + 4.);
        ui.spacing_mut().item_spacing.y = 0.;
        let stride = row_height;
        let enabled = self.folder_actions_allowed(ui.ctx());
        if enabled {
            self.folder_list_keys(ui.ctx(), (height / stride).floor().max(1.) as usize);
        }
        // A reserved scrollbar remains visible whenever rows are clipped.
        ui.spacing_mut().scroll.floating = false;
        let mut area = egui::ScrollArea::vertical()
            .id_salt(("folder_files", &self.folder, &self.folder_browser.filter))
            .max_height(height)
            .auto_shrink([false, false]);
        if self.folder_browser.reveal_selection {
            if let Some((index, _)) = &self.folder_browser.selection {
                area = area.vertical_scroll_offset(*index as f32 * stride);
            }
            self.folder_browser.reveal_selection = false;
        }
        area.show_rows_themed(ui, row_height, self.entries.len(), |ui, rows| {
            for index in rows {
                let entry = &self.entries[index];
                let selected = self.folder_browser.selected_path() == Some(entry.path.as_path());
                let label = format!("{}{}", entry.name, if entry.directory { "/" } else { "" });
                let kind = if entry.directory {
                    panels::FileKind::Folder
                } else if entry.name.to_ascii_lowercase().ends_with(".m") {
                    panels::FileKind::Script
                } else {
                    panels::FileKind::Data
                };
                let response = ui
                    .add_enabled_ui(enabled, |ui| {
                        panels::file_row(ui, &label, kind, selected, row_height)
                    })
                    .inner
                    .on_hover_text(crate::path_label::display(&entry.path).to_string());
                if response.clicked() {
                    self.folder_browser.selection = Some((index, entry.path.clone()));
                    self.folder_browser.focus = Some(SelectionFocus {
                        frame: ui.ctx().cumulative_frame_nr(),
                        generation: self.folder_browser.generation,
                        folder: self.folder.clone(),
                        selected: entry.path.clone(),
                    });
                    ui.memory_mut(|memory| memory.request_focus(list_id()));
                }
                if response.double_clicked() {
                    self.queue_folder_action(ui.ctx(), entry.path.clone(), entry.directory);
                }
            }
        });
    }
    fn folder_actions_allowed(&self, ctx: &egui::Context) -> bool {
        ctx.viewport_id() == egui::ViewportId::ROOT
            && self.navigation.left == Some(navigation::Left::Files)
            && !self.busy
            && !self.folder_search.visible
            && !self.file_action_pending()
            && !self.allow_close
            && !self.recovering_session
            && !self.folder_browser.loading
            && self.folder_browser.due.is_none()
            && self.open_queue.is_empty()
            && ctx.memory(|memory| memory.top_modal_layer().is_none())
            && !egui::Popup::is_any_open(ctx)
            && !ctx.input(|input| input.viewport().close_requested())
    }
    fn queue_folder_action(&mut self, ctx: &egui::Context, path: PathBuf, directory: bool) {
        self.folder_browser.action = Some(Action {
            frame: ctx.cumulative_frame_nr(),
            generation: self.folder_browser.generation,
            folder: self.folder.clone(),
            selected: self
                .folder_browser
                .selected_path()
                .map(std::path::Path::to_owned),
            path,
            directory,
        });
    }
    fn folder_list_keys(&mut self, ctx: &egui::Context, page: usize) {
        let frame = ctx.cumulative_frame_nr();
        if self.folder_browser.key_frame == Some(frame)
            || !ctx.input(|input| input.modifiers.is_none())
        {
            return;
        }
        let focus = ctx.memory(|memory| memory.focused());
        if focus != Some(list_id()) {
            // Down leaves filename editing deliberately; ordinary typing/arrows remain editor-owned.
            if focus == Some(egui::Id::new("folder_filter"))
                && ctx.input_mut(|input| {
                    input.consume_key(egui::Modifiers::NONE, egui::Key::ArrowDown)
                })
            {
                self.folder_browser.key_frame = Some(frame);
                ctx.memory_mut(|memory| {
                    memory.request_focus(list_id());
                    memory.move_focus(egui::FocusDirection::None);
                });
                if let Some(entry) = self.entries.first() {
                    self.folder_browser.selection = Some((0, entry.path.clone()));
                    self.folder_browser.reveal_selection = true;
                }
            }
            return;
        }
        self.folder_browser.key_frame = Some(frame);
        let key = ctx.input_mut(|input| {
            [
                egui::Key::ArrowUp,
                egui::Key::ArrowDown,
                egui::Key::Home,
                egui::Key::End,
                egui::Key::PageUp,
                egui::Key::PageDown,
                egui::Key::Enter,
                egui::Key::Backspace,
            ]
            .into_iter()
            .find(|key| input.consume_key(egui::Modifiers::NONE, *key))
        });
        let Some(key) = key else {
            return;
        };
        if matches!(key, egui::Key::ArrowUp | egui::Key::ArrowDown) {
            // egui schedules directional focus before widgets consume input. On the first
            // focused frame the filter above is not yet active, so retire that movement too.
            ctx.memory_mut(|memory| memory.move_focus(egui::FocusDirection::None));
        }
        if key == egui::Key::Backspace {
            if let Some(parent) = self.folder.parent() {
                self.queue_folder_action(ctx, parent.to_owned(), true);
            }
            return;
        }
        let Some(last) = self.entries.len().checked_sub(1) else {
            return;
        };
        let index = self
            .folder_browser
            .selection
            .as_ref()
            .map(|(index, _)| *index)
            .filter(|index| *index <= last);
        if key == egui::Key::Enter {
            if let Some(index) = index {
                let entry = &self.entries[index];
                self.queue_folder_action(ctx, entry.path.clone(), entry.directory);
            }
            return;
        }
        let next = match key {
            egui::Key::Home => 0,
            egui::Key::End => last,
            egui::Key::ArrowUp => index.map_or(last, |index| index.saturating_sub(1)),
            egui::Key::ArrowDown => index.map_or(0, |index| (index + 1).min(last)),
            egui::Key::PageUp => index.map_or(0, |index| index.saturating_sub(page)),
            egui::Key::PageDown => index.map_or(0, |index| index.saturating_add(page).min(last)),
            _ => return,
        };
        self.folder_browser.selection = Some((next, self.entries[next].path.clone()));
        self.folder_browser.reveal_selection = true;
    }
    fn finish_folder_focus(&mut self, ctx: &egui::Context) {
        let Some(intent) = self.folder_browser.focus.take() else {
            return;
        };
        if intent.frame != ctx.cumulative_frame_nr()
            || intent.generation != self.folder_browser.generation
            || intent.folder != self.folder
            || Some(intent.selected.as_path()) != self.folder_browser.selected_path()
            || !self.folder_actions_allowed(ctx)
            || ctx.memory(|memory| memory.focused().is_some_and(|id| id != list_id()))
        {
            return;
        }
        // Recreating the covered aggregate response in a discarded pass can
        // surrender its focus after the row release event has been consumed.
        // Restore only this same-frame row intent, never over another owner.
        ctx.memory_mut(|memory| {
            memory.request_focus(list_id());
            memory.move_focus(egui::FocusDirection::None);
        });
    }
    pub(super) fn finish_folder_action(&mut self, ctx: &egui::Context) {
        if ctx.will_discard() {
            return;
        }
        self.finish_folder_focus(ctx);
        let Some(action) = self.folder_browser.action.take() else {
            return;
        };
        if action.frame != ctx.cumulative_frame_nr()
            || action.generation != self.folder_browser.generation
            || action.folder != self.folder
            || action.selected.as_deref() != self.folder_browser.selected_path()
            || !ctx.memory(|memory| memory.has_focus(list_id()))
            || !self.folder_actions_allowed(ctx)
        {
            return;
        }
        if action.directory {
            self.change_folder(action.path);
        } else {
            self.open_path(action.path);
        }
    }
}

/// "1 entry", "5 entries".
pub(super) fn entry_count(count: usize) -> String {
    if count == 1 {
        "1 entry".into()
    } else {
        format!("{count} entries")
    }
}

// The toolbar picker is separate from the existing Files list action. Retain
// only bounded path/owner metadata, never the listing or workspace values.
#[cfg(test)]
type BrowsePicker = fn(&mut Workspace, &egui::Context) -> Option<PathBuf>;

#[derive(Default)]
pub(super) struct Browse {
    action: Option<BrowseAction>,
    finished: Option<(egui::ViewportId, u64)>,
    generation: u64,
    #[cfg(test)]
    picker: Option<BrowsePicker>,
    #[cfg(test)]
    calls: usize,
}

struct BrowseAction {
    stamp: (egui::ViewportId, u64),
    generation: u64,
    scan_generation: u64,
    folder: PathBuf,
    document: u64,
    interaction: u64,
    focus: Option<egui::Id>,
}

impl Workspace {
    pub(super) fn folder_browse_context(&self, ctx: &egui::Context) -> bool {
        // Browse may replace an in-progress navigation or scan, as before.
        // The captured scanner generation rejects a *later* competing request.
        ctx.viewport_id() == egui::ViewportId::ROOT
            && ctx.input(|i| i.focused && !i.viewport().close_requested())
            && ctx.memory(|m| m.top_modal_layer().is_none())
            && !egui::Popup::is_any_open(ctx)
            && !self.busy
            && !self.file_action_pending()
            && !self.allow_close
            && !self.recovering_session
            && !self.recovery_blocked
    }

    fn valid_folder_browse(&self, ctx: &egui::Context, action: &BrowseAction) -> bool {
        self.folder_browse_context(ctx)
            && action.stamp == (ctx.viewport_id(), ctx.cumulative_frame_nr())
            && action.generation == self.folder_browse.generation
            && action.scan_generation == self.folder_browser.generation
            && action.folder == self.folder
            && action.document == self.documents[self.active].id
            && action.interaction == self.document_save.interaction_generation()
            && action.focus == ctx.memory(|m| m.focused())
    }

    pub(super) fn queue_folder_browse(&mut self, ctx: &egui::Context) {
        let stamp = (ctx.viewport_id(), ctx.cumulative_frame_nr());
        if !self.folder_browse_context(ctx)
            || self.folder_browse.action.is_some()
            || self.folder_browse.finished == Some(stamp)
        {
            return;
        }
        self.folder_browse.action = Some(BrowseAction {
            stamp,
            generation: self.folder_browse.generation,
            scan_generation: self.folder_browser.generation,
            folder: self.folder.clone(),
            document: self.documents[self.active].id,
            interaction: self.document_save.interaction_generation(),
            focus: ctx.memory(|m| m.focused()),
        });
    }

    pub(super) fn cancel_folder_browse(&mut self) {
        self.folder_browse.generation = self.folder_browse.generation.wrapping_add(1);
        if let Some(action) = self.folder_browse.action.take() {
            self.folder_browse.finished = Some(action.stamp);
        }
    }

    pub(super) fn observe_folder_browse(&mut self, ctx: &egui::Context) {
        if self
            .folder_browse
            .action
            .as_ref()
            .is_some_and(|action| !self.valid_folder_browse(ctx, action))
        {
            self.cancel_folder_browse();
        }
    }

    pub(super) fn finish_folder_browse(&mut self, ctx: &egui::Context) {
        if ctx.will_discard() {
            return;
        }
        let Some(action) = self.folder_browse.action.take() else {
            return;
        };
        // Consume before the native call, including Cancel and invalid results.
        self.folder_browse.finished = Some(action.stamp);
        if !self.valid_folder_browse(ctx, &action) {
            return;
        }
        if let Some(path) = self.pick_folder_browse(ctx, &action.folder)
            && !ctx.will_discard()
            && self.valid_folder_browse(ctx, &action)
        {
            self.change_folder(path);
        }
    }

    fn pick_folder_browse(
        &mut self,
        _ctx: &egui::Context,
        _folder: &std::path::Path,
    ) -> Option<PathBuf> {
        #[cfg(test)]
        {
            self.folder_browse.calls += 1;
            assert!(self.folder_browse.calls <= 128);
            self.folder_browse.picker.expect("headless Browse picker")(self, _ctx)
        }
        #[cfg(not(test))]
        {
            rfd::FileDialog::new().set_directory(_folder).pick_folder()
        }
    }
}

#[cfg(test)]
mod browse_tests {
    use super::*;
    const WIDE: egui::Vec2 = egui::vec2(1440., 900.);
    const COMPACT: egui::Vec2 = egui::vec2(960., 650.);

    fn finish(app: &mut Workspace) {
        let until = Instant::now() + Duration::from_secs(10);
        while (app.busy || app.folder_browser.loading) && Instant::now() < until {
            app.receive();
            app.receive_folder();
            std::thread::sleep(Duration::from_millis(1));
        }
        assert!(!app.busy && !app.folder_browser.loading, "{}", app.status);
    }

    fn frame(
        app: &mut Workspace,
        ctx: &egui::Context,
        size: egui::Vec2,
        events: Vec<egui::Event>,
    ) -> egui::FullOutput {
        headless::run(
            ctx,
            egui::RawInput {
                screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, size)),
                events,
                ..Default::default()
            },
            |root| app.ui(root),
        )
    }

    fn setup(size: egui::Vec2) -> (egui::Context, Workspace, tempfile::TempDir) {
        let ctx = egui::Context::default();
        let dir = tempfile::tempdir().unwrap();
        for name in ["chosen", "other"] {
            std::fs::create_dir(dir.path().join(name)).unwrap();
            std::fs::write(dir.path().join(name).join("inside.m"), "never_run=41;").unwrap();
        }
        let mut app = Workspace::with_context(ctx.clone(), true);
        assert!(app.change_folder(dir.path().to_owned()));
        finish(&mut app);
        app.run("keep=[1 2 3]; alias=keep; plot(keep);".into());
        finish(&mut app);
        app.new_document("dirty_script_must_remain=7;");
        app.folder_browse.picker = Some(|_, _| None);
        for _ in 0..5 {
            frame(&mut app, &ctx, size, vec![]);
        }
        assert!(app.folder_browse_context(&ctx));
        (ctx, app, dir)
    }

    fn text_rect(shape: &egui::epaint::Shape) -> Option<egui::Rect> {
        match shape {
            egui::epaint::Shape::Text(text) if text.galley.text() == "Browse" => {
                Some(text.galley.rect.translate(text.pos.to_vec2()))
            }
            egui::epaint::Shape::Vec(shapes) => shapes.iter().find_map(text_rect),
            _ => None,
        }
    }

    fn button(app: &mut Workspace, ctx: &egui::Context, size: egui::Vec2) -> egui::Pos2 {
        let output = frame(app, ctx, size, vec![]);
        output
            .shapes
            .iter()
            .find_map(|shape| {
                text_rect(&shape.shape).filter(|rect| shape.clip_rect.contains_rect(*rect))
            })
            .expect("visible Current Folder Browse button")
            .center()
    }

    fn pointer(pos: egui::Pos2, pressed: bool) -> Vec<egui::Event> {
        vec![
            egui::Event::PointerMoved(pos),
            egui::Event::PointerButton {
                pos,
                button: egui::PointerButton::Primary,
                pressed,
                modifiers: egui::Modifiers::NONE,
            },
        ]
    }

    fn release_discarded(
        app: &mut Workspace,
        ctx: &egui::Context,
        size: egui::Vec2,
        pos: egui::Pos2,
        mut between: impl FnMut(&mut Workspace, &egui::Context),
    ) {
        let calls = app.folder_browse.calls;
        let scan = app.folder_browser.generation;
        let mut passes = 0;
        headless::run(
            ctx,
            egui::RawInput {
                screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, size)),
                events: pointer(pos, false),
                ..Default::default()
            },
            |root| {
                passes += 1;
                if passes == 1 {
                    root.ctx()
                        .request_discard("Browse must await final admission");
                }
                app.ui(root);
                if passes == 1 {
                    assert!(
                        app.folder_browse.action.is_some(),
                        "actual button release retained"
                    );
                    assert_eq!(app.folder_browse.calls, calls);
                    assert_eq!(
                        app.folder_browser.generation, scan,
                        "no early navigation submitted"
                    );
                    between(app, ctx);
                }
            },
        );
        assert!(passes >= 2);
    }

    #[test]
    fn folder_browse_cancel_is_final_once_and_fresh_gestures_work() {
        for size in [COMPACT, WIDE] {
            let (ctx, mut app, _dir) = setup(size);
            let folder = app.folder.clone();
            let scan = app.folder_browser.generation;
            for expected in 1..=2 {
                let pos = button(&mut app, &ctx, size);
                frame(&mut app, &ctx, size, pointer(pos, true));
                release_discarded(&mut app, &ctx, size, pos, |_, _| {});
                assert_eq!(app.folder_browse.calls, expected);
                assert_eq!(app.folder, folder);
                assert_eq!(app.folder_browser.generation, scan);
            }
            let pos = button(&mut app, &ctx, size);
            frame(&mut app, &ctx, size, pointer(pos, true));
            let mut passes = 0;
            headless::run(
                &ctx,
                egui::RawInput {
                    screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, size)),
                    events: pointer(pos, false),
                    ..Default::default()
                },
                |root| {
                    passes += 1;
                    app.ui(root);
                    if passes == 1 {
                        assert_eq!(app.folder_browse.calls, 3);
                        root.ctx().request_discard("Completed Cancel cannot replay");
                    }
                },
            );
            assert!(passes >= 2);
            assert_eq!(app.folder_browse.calls, 3);
            assert_eq!(app.folder, folder);
            assert_eq!(app.folder_browser.generation, scan);
        }
    }

    fn invalidate(app: &mut Workspace, ctx: &egui::Context, case: usize) {
        match case {
            0 => app.stop_work(),
            1 => app.busy = true,
            2 => app.pending_quit = true,
            3 => app.allow_close = true,
            4 => app.recovering_session = true,
            5 => app.recovery_blocked = true,
            6 => app.folder.push("different"),
            7 => app.new_document("different_owner=1;"),
            8 => {
                app.begin_workspace_action(false, true);
                assert!(app.workspace_tools.dialog.is_some());
            }
            9 => ctx.memory_mut(|m| m.request_focus(egui::Id::new("late_browse_owner"))),
            10 => app.refresh_folder(),
            11 => {
                assert!(app.change_folder(app.folder.join("other")));
            }
            12 => egui::Popup::open_id(ctx, egui::Id::new("late_browse_popup")),
            13 => app.document_save.command_selected(),
            14 => ctx.input_mut(|i| i.focused = false),
            15 => {
                ctx.request_discard("Picker result lost final-pass authority");
            }
            _ => unreachable!(),
        }
    }

    #[test]
    fn folder_browse_rejects_late_owner_scan_stop_and_picker_results() {
        for after in [false, true] {
            for case in 0..(if after { 16 } else { 15 }) {
                let (ctx, mut app, dir) = setup(WIDE);
                let chosen = std::fs::canonicalize(dir.path().join("chosen")).unwrap();
                let folder = app.folder.clone();
                let scan = app.folder_browser.generation;
                if after {
                    app.folder_browse.picker = Some(|app, ctx| {
                        let path = app.folder.join("chosen");
                        let case = app
                            .status
                            .strip_prefix("browse test ")
                            .unwrap()
                            .parse()
                            .unwrap();
                        invalidate(app, ctx, case);
                        Some(path)
                    });
                } else {
                    app.folder_browse.picker = Some(|_, _| panic!("stale Browse opened picker"));
                }
                let pos = button(&mut app, &ctx, WIDE);
                frame(&mut app, &ctx, WIDE, pointer(pos, true));
                let (send, receive) = std::sync::mpsc::sync_channel(8);
                let original = std::mem::replace(&mut app.worker.send, send);
                if after {
                    app.status = format!("browse test {case}");
                    frame(&mut app, &ctx, WIDE, pointer(pos, false));
                } else {
                    release_discarded(&mut app, &ctx, WIDE, pos, |app, ctx| {
                        invalidate(app, ctx, case);
                        // egui retries restore focused from RawInput. Observe
                        // this injected loss before that restoration, explicitly.
                        if case == 14 {
                            assert!(!ctx.input(|i| i.focused));
                            app.observe_folder_browse(ctx);
                            assert!(app.folder_browse.action.is_none());
                        }
                    });
                }
                assert_eq!(app.folder_browse.calls, usize::from(after), "case {case}");
                assert!(app.folder_browse.action.is_none());
                assert_eq!(
                    app.folder_browser.generation,
                    scan + u64::from(matches!(case, 10 | 11)),
                    "case {case}"
                );
                assert!(matches!(
                    receive.try_recv(),
                    Err(std::sync::mpsc::TryRecvError::Empty)
                ));
                app.worker.send = original;
                app.busy = false;
                finish(&mut app);
                assert_ne!(
                    app.folder, chosen,
                    "returned path not admitted: case {case}"
                );
                if !matches!(case, 6 | 11) {
                    assert_eq!(app.folder, folder);
                }
            }
        }
    }

    #[test]
    fn folder_browse_success_scans_selected_folder_and_preserves_open_work() {
        for size in [COMPACT, WIDE] {
            let (ctx, mut app, dir) = setup(size);
            let old_folder = app.folder.clone();
            let selected = std::fs::canonicalize(dir.path().join("chosen")).unwrap();
            let values = app.variables.clone();
            let figure = app.figure.clone().unwrap();
            let documents: Vec<_> = app
                .documents
                .iter()
                .map(|d| (d.id, d.text.clone()))
                .collect();
            let cwd = std::env::current_dir().unwrap();
            app.folder_browse.picker = Some(|app, _| Some(app.folder.join("chosen")));
            let pos = button(&mut app, &ctx, size);
            frame(&mut app, &ctx, size, pointer(pos, true));
            release_discarded(&mut app, &ctx, size, pos, |app, _| {
                assert_eq!(app.folder, old_folder);
            });
            assert_eq!(app.folder_browse.calls, 1);
            assert!(app.folder_browser.navigating());
            assert_eq!(
                app.folder, old_folder,
                "navigation publishes only after scanner result"
            );
            finish(&mut app);
            assert_eq!(app.folder, selected);
            assert!(
                app.entries
                    .iter()
                    .any(|e| e.name == "inside.m" && !e.directory)
            );
            assert!(!app.variables.contains_key("never_run"));
            for (name, value) in &values {
                assert!(value.same_snapshot(&app.variables[name]));
            }
            assert!(app.variables["keep"].same_snapshot(&app.variables["alias"]));
            assert!(app.figure.as_ref().unwrap().same_snapshot(&figure));
            assert_eq!(
                documents,
                app.documents
                    .iter()
                    .map(|d| (d.id, d.text.clone()))
                    .collect::<Vec<_>>()
            );
            assert_eq!(std::env::current_dir().unwrap(), cwd);
            app.step_folder(false);
            finish(&mut app);
            assert_eq!(app.folder, old_folder, "existing Back history retained");
        }
    }

    #[test]
    fn folder_browse_keeps_scan_navigation_replacement_available_and_stop_retires_intent() {
        let (ctx, mut app, _dir) = setup(WIDE);
        let folder = app.folder.clone();
        // Match old Browse eligibility: neither scan nor navigation alone is a modal.
        app.refresh_folder();
        assert!(app.folder_browse_context(&ctx));
        assert!(app.change_folder(folder.join("other")));
        assert!(app.folder_browser.navigating() && app.folder_browse_context(&ctx));
        app.stop_work();
        finish(&mut app);
        let pos = button(&mut app, &ctx, WIDE);
        frame(&mut app, &ctx, WIDE, pointer(pos, true));
        release_discarded(&mut app, &ctx, WIDE, pos, |app, _| app.stop_work());
        assert_eq!(app.folder_browse.calls, 0);
        assert_eq!(app.folder, folder);
        app.folder_browse.picker = Some(|app, _| Some(app.folder.join("chosen")));
        let pos = button(&mut app, &ctx, WIDE);
        frame(&mut app, &ctx, WIDE, pointer(pos, true));
        frame(&mut app, &ctx, WIDE, pointer(pos, false));
        assert_eq!(app.folder_browse.calls, 1);
        finish(&mut app);
        assert!(app.folder.ends_with("chosen"));
    }
}
