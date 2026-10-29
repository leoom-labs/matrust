//! Bounded focus traversal over real widgets painted by the current native host.
use super::*;
use crate::theme::ThemedScrollArea;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Area {
    Files,
    Workspace,
    Editor(u64),
    Figure(u32),
    Command,
    Designer,
    Model,
}
impl Area {
    fn rank(self) -> u8 {
        match self {
            Self::Files => 0,
            Self::Workspace => 1,
            Self::Editor(_) => 2,
            Self::Figure(_) => 3,
            Self::Command => 4,
            // Tool tabs follow the document tabs; only the selected one is painted.
            Self::Designer => 5,
            Self::Model => 6,
        }
    }
    fn tool(self) -> Option<toolkits::Tool> {
        match self {
            Self::Designer => Some(toolkits::Tool::Designer),
            Self::Model => Some(toolkits::Tool::Model),
            _ => None,
        }
    }
    fn of_tool(tool: toolkits::Tool) -> Self {
        match tool {
            toolkits::Tool::Designer => Self::Designer,
            toolkits::Tool::Model => Self::Model,
        }
    }
}
#[derive(Clone, Copy)]
struct Entry {
    area: Area,
    id: egui::Id,
    region: egui::Rect,
    layer: egui::LayerId,
    /// An open tool tab that is not painted; traversal selects it first.
    hidden: bool,
}
const MAX_FOCUS_AREAS: usize = 8;
#[derive(Clone, Copy, Default)]
struct Entries {
    slots: [Option<Entry>; MAX_FOCUS_AREAS],
    len: usize,
}
impl Entries {
    fn len(&self) -> usize {
        self.len
    }
    fn is_empty(&self) -> bool {
        self.len == 0
    }
    fn iter(&self) -> impl Iterator<Item = &Entry> {
        self.slots[..self.len].iter().flatten()
    }
    fn push(&mut self, entry: Entry) {
        // Registration checks the capacity and uniqueness before admission.
        assert!(self.len < MAX_FOCUS_AREAS);
        self.slots[self.len] = Some(entry);
        self.len += 1;
    }
    fn sort_by_area(&mut self) {
        // Stable insertion sort: at most eight entries, no temporary allocation.
        // Equal-ranked Editor buffers retain their actual paint order.
        for index in 1..self.len {
            let mut cursor = index;
            while cursor > 0 && self[cursor - 1].area.rank() > self[cursor].area.rank() {
                self.slots.swap(cursor - 1, cursor);
                cursor -= 1;
            }
        }
    }
}
impl std::ops::Index<usize> for Entries {
    type Output = Entry;
    fn index(&self, index: usize) -> &Entry {
        self.slots[..self.len][index]
            .as_ref()
            .expect("occupied focus registry prefix")
    }
}
#[derive(Clone, Copy, Default)]
struct Painted {
    entries: Entries,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
struct Host {
    viewport: egui::ViewportId,
    editor: bool,
}
fn registry(host: Host) -> egui::Id {
    egui::Id::new(("desktop_focus_areas", host))
}
fn active_host(ctx: &egui::Context) -> Host {
    Host {
        viewport: ctx.viewport_id(),
        editor: ctx.data(|data| {
            data.get_temp::<bool>(egui::Id::new("focus_area_editor_host"))
                .unwrap_or(false)
        }),
    }
}
pub(super) fn register(ui: &egui::Ui, response: &egui::Response, area: Area) {
    let region = ui.clip_rect().intersect(ui.ctx().content_rect());
    let visible = response.interact_rect.intersect(region);
    if !response.enabled()
        || region.width() < 1.
        || region.height() < 1.
        || area != Area::Command && (visible.width() < 1. || visible.height() < 1.)
    {
        return;
    }
    // Only seven real destinations exist. No document text, samples or callbacks
    // are retained, and no synthetic focus-only widget is added to Tab order.
    let key = registry(active_host(ui.ctx()));
    ui.ctx().data_mut(|data| {
        let painted = data.get_temp_mut_or_default::<Painted>(key);
        if painted.entries.len() < MAX_FOCUS_AREAS
            && !painted.entries.iter().any(|old| old.area == area)
        {
            painted.entries.push(Entry {
                area,
                id: response.id,
                region,
                layer: response.layer_id,
                hidden: false,
            });
        }
    });
}
#[derive(Clone)]
struct Ticket {
    host: Host,
    frame: u64,
    interaction: u64,
    document: u64,
    origin: Option<egui::Id>,
    target: Entry,
    entries: Entries,
    folder: PathBuf,
    figure: Option<matrust::ObjectIdentity>,
    current_figure: u32,
}
#[derive(Clone, Copy)]
struct HelpTicket {
    host: Host,
    frame: u64,
    document: u64,
    interaction: u64,
    origin: Option<egui::Id>,
}
#[derive(Default)]
pub(super) struct State {
    pending: Option<Ticket>,
    /// Tool tab selected by traversal; its first area takes focus once painted.
    deferred: Option<Area>,
    finished: Option<(Host, u64)>,
    help: Option<Host>,
    help_request: Option<HelpTicket>,
    help_close: Option<(Host, u64)>,
}
impl State {
    pub(super) fn is_open(&self) -> bool {
        self.help.is_some()
    }
    pub(super) fn retire_editor_host(&mut self) {
        if self
            .pending
            .as_ref()
            .is_some_and(|ticket| ticket.host.editor)
            && let Some(ticket) = self.pending.take()
        {
            self.finished = Some((ticket.host, ticket.frame));
        }
        if self.help.is_some_and(|host| host.editor) {
            self.help = None;
        }
        if self.help_request.is_some_and(|ticket| ticket.host.editor) {
            self.help_request = None;
        }
        if self.help_close.is_some_and(|(host, _)| host.editor) {
            self.help_close = None;
        }
    }

    pub(super) fn cancel(&mut self) {
        if let Some(ticket) = self.pending.take() {
            self.finished = Some((ticket.host, ticket.frame));
        }
        self.help_request = None;
        self.help_close = None;
        self.help = None;
    }
}

impl Workspace {
    fn focus_area_host(&self, ctx: &egui::Context) -> Host {
        Host {
            viewport: ctx.viewport_id(),
            editor: self.editor_window.in_callback,
        }
    }
    pub(super) fn embedded_editor_owns_focus(&self, ctx: &egui::Context) -> bool {
        let host = self.focus_area_host(ctx);
        if !host.editor
            && self.editor_window.detached
            && self.editor_focus_viewport() == egui::ViewportId::ROOT
        {
            let child_key = registry(Host {
                editor: true,
                ..host
            });
            let child = ctx
                .data(|data| data.get_temp::<Painted>(child_key))
                .unwrap_or_default();
            let focused = ctx.memory(|memory| memory.focused());
            if child.entries.iter().any(|entry| focused == Some(entry.id))
                || focused
                    .and_then(|id| ctx.read_response(id))
                    .is_some_and(|response| {
                        child.entries.iter().any(|entry| {
                            entry.layer == response.layer_id
                                && entry.region.contains(response.rect.center())
                        })
                    })
            {
                return true;
            }
        }
        false
    }

    fn focus_areas_allowed(&self, ctx: &egui::Context) -> bool {
        (ctx.viewport_id() == egui::ViewportId::ROOT || self.editor_viewport_allowed(ctx))
            && !self.busy
            && !self.file_action_pending()
            && !self.recovering_session
            && !self.pending_quit
            && !self.allow_close
            && !egui::Popup::is_any_open(ctx)
            && !egui::DragAndDrop::has_any_payload(ctx)
            && ctx.memory(|memory| memory.top_modal_layer().is_none())
            && ctx.input(|input| {
                input.viewport().focused != Some(false) && !input.viewport().close_requested()
            })
    }

    // Painted areas, then every open tool tab in opening order. Tools that are
    // not painted are stand-ins that traversal selects before focusing.
    fn focus_ring(&self, painted: Entries) -> Entries {
        let mut ring = Entries::default();
        for entry in painted.iter().filter(|entry| entry.area.tool().is_none()) {
            ring.push(*entry);
        }
        for tool in self.toolkits.tabs() {
            let area = Area::of_tool(tool);
            let entry = painted
                .iter()
                .find(|entry| entry.area == area)
                .copied()
                .unwrap_or(Entry {
                    area,
                    id: egui::Id::NULL,
                    region: egui::Rect::NOTHING,
                    layer: egui::LayerId::background(),
                    hidden: true,
                });
            if ring.len() < MAX_FOCUS_AREAS {
                ring.push(entry);
            }
        }
        ring
    }

    pub(super) fn begin_focus_areas(&mut self, ctx: &egui::Context) {
        let host = self.focus_area_host(ctx);
        let key = registry(host);
        ctx.data_mut(|data| {
            *data.get_temp_mut_or_default::<bool>(egui::Id::new("focus_area_editor_host")) =
                host.editor;
        });
        // Observe a newer owner before old pointer responses can surrender it.
        if self.focus_areas.pending.as_ref().is_some_and(|ticket| {
            ticket.host == host
                && (ticket.frame != ctx.cumulative_frame_nr()
                    || ctx.memory(|memory| memory.focused()) != ticket.origin
                    || !self.focus_areas_allowed(ctx))
        }) {
            self.focus_areas.cancel();
        }
        if self
            .focus_areas
            .help_request
            .as_ref()
            .is_some_and(|ticket| {
                ticket.host == host
                    && (ticket.frame != ctx.cumulative_frame_nr()
                        || ticket.origin != ctx.memory(|memory| memory.focused())
                        || ticket.document != self.documents[self.active].id
                        || ticket.interaction != self.document_save.interaction_generation())
            })
        {
            self.focus_areas.help_request = None;
        }
        let mut previous = ctx
            .data_mut(|data| std::mem::take(data.get_temp_mut_or_default::<Painted>(key)).entries);
        if !self.focus_areas_allowed(ctx) {
            return;
        }
        // Embedded viewports share ROOT input: let the Editor callback consume
        // its own chord before any root-area traversal is queued.
        if self.embedded_editor_owns_focus(ctx) {
            return;
        }
        let stamp = (host, ctx.cumulative_frame_nr());
        if self.focus_areas.finished == Some(stamp) || self.focus_areas.pending.is_some() {
            return;
        }
        let chord = ctx.input_mut(|input| {
            let event = input.events.iter().find_map(|event| match event {
                egui::Event::Key {
                    key,
                    pressed: true,
                    repeat: false,
                    modifiers,
                    ..
                } if modifiers.ctrl
                    && !modifiers.alt
                    && !modifiers.mac_cmd
                    && (*key == egui::Key::F6 || *key == egui::Key::Slash && !modifiers.shift) =>
                {
                    Some((*key, *modifiers))
                }
                _ => None,
            });
            event.filter(|(key, modifiers)| input.consume_key(*modifiers, *key))
        });
        let Some((key, modifiers)) = chord else {
            return;
        };
        ctx.memory_mut(|memory| memory.move_focus(egui::FocusDirection::None));
        if key == egui::Key::Slash {
            self.queue_navigation_help(ctx);
            self.focus_areas.finished = Some(stamp);
            return;
        }
        previous.sort_by_area();
        let previous = self.focus_ring(previous);
        if previous.is_empty() {
            return;
        }
        let origin = ctx.memory(|memory| memory.focused());
        let current = previous
            .iter()
            .position(|entry| origin == Some(entry.id))
            .or_else(|| {
                origin
                    .and_then(|id| ctx.read_response(id))
                    .and_then(|response| {
                        previous.iter().position(|entry| {
                            entry.layer == response.layer_id
                                && entry.region.contains(response.rect.center())
                        })
                    })
            });
        let index = match (current, modifiers.shift) {
            (Some(index), false) => (index + 1) % previous.len(),
            (Some(index), true) => (index + previous.len() - 1) % previous.len(),
            (None, false) => 0,
            (None, true) => previous.len() - 1,
        };
        self.focus_areas.pending = Some(Ticket {
            host: stamp.0,
            frame: stamp.1,
            interaction: self.document_save.interaction_generation(),
            document: self.documents[self.active].id,
            origin,
            target: previous[index],
            entries: previous,
            folder: self.folder.clone(),
            current_figure: self.current_figure,
            figure: self.figure.as_ref().map(|figure| figure.identity.clone()),
        });
    }

    pub(super) fn finish_focus_areas(&mut self, ctx: &egui::Context) {
        if ctx.will_discard() {
            return;
        }
        let host = self.focus_area_host(ctx);
        let key = registry(host);
        if let Some(area) = self.focus_areas.deferred.take() {
            let painted = ctx
                .data(|data| data.get_temp::<Painted>(key))
                .unwrap_or_default();
            if let Some(entry) = painted.entries.iter().find(|entry| entry.area == area) {
                ctx.memory_mut(|memory| memory.request_focus(entry.id));
            }
        }
        let Some(ticket) = self.focus_areas.pending.as_ref() else {
            return;
        };
        if ticket.host != host {
            return;
        }
        let ticket = self.focus_areas.pending.take().unwrap();
        self.focus_areas.finished = Some((ticket.host, ticket.frame));
        let mut painted = ctx
            .data(|data| data.get_temp::<Painted>(key))
            .unwrap_or_default();
        painted.entries.sort_by_area();
        let same_figure = match (&ticket.figure, &self.figure) {
            (Some(before), Some(after)) => before.same_object(&after.identity),
            (None, None) => true,
            _ => false,
        };
        if ticket.frame != ctx.cumulative_frame_nr()
            || !self.focus_areas_allowed(ctx)
            || ticket.document != self.documents[self.active].id
            || ticket.interaction != self.document_save.interaction_generation()
            || ticket.folder != self.folder
            || ticket.current_figure != self.current_figure
            || !same_figure
            || ctx.memory(|memory| memory.focused()) != ticket.origin
            || !ticket
                .entries
                .iter()
                .filter(|entry| !entry.hidden)
                .map(|entry| (entry.area, entry.id))
                .eq(painted.entries.iter().map(|entry| (entry.area, entry.id)))
        {
            return;
        }
        if let Some(tool) = ticket.target.area.tool().filter(|_| ticket.target.hidden) {
            if !self.toolkits.is_open(tool) {
                return;
            }
            // Same path as clicking the tab; the tool paints next frame.
            self.select_tool_tab(tool);
            self.focus_areas.deferred = Some(ticket.target.area);
            ctx.request_repaint();
            return;
        }
        if let Area::Editor(document) = ticket.target.area {
            let Some(index) = self.documents.iter().position(|value| value.id == document) else {
                return;
            };
            self.activate_script_index(index);
        }
        ctx.memory_mut(|memory| {
            memory.move_focus(egui::FocusDirection::None);
            memory.request_focus(ticket.target.id);
        });
        ctx.request_repaint();
    }

    pub(super) fn queue_navigation_help(&mut self, ctx: &egui::Context) {
        let stamp = (self.focus_area_host(ctx), ctx.cumulative_frame_nr());
        if self.focus_areas.finished == Some(stamp) || self.focus_areas.help_request.is_some() {
            return;
        }
        self.focus_areas.finished = Some(stamp);
        self.focus_areas.help_request = Some(HelpTicket {
            host: self.focus_area_host(ctx),
            frame: ctx.cumulative_frame_nr(),
            document: self.documents[self.active].id,
            interaction: self.document_save.interaction_generation(),
            origin: ctx.memory(|memory| memory.focused()),
        });
    }

    pub(super) fn navigation_help_menu(&mut self, ui: &mut egui::Ui, close_menu: bool) {
        if ui
            .button("Navigation shortcuts")
            .on_hover_text("Ctrl+/")
            .clicked()
        {
            self.queue_navigation_help(ui.ctx());
            if close_menu {
                ui.close();
            }
        }
    }

    pub(super) fn navigation_help_ui(&mut self, ctx: &egui::Context) {
        let host = self.focus_area_host(ctx);
        // Existing modal controls must observe release/Escape on discarded
        // passes; only opening and committing Close wait for final admission.
        if !ctx.will_discard()
            && self
                .focus_areas
                .help_request
                .is_some_and(|ticket| ticket.host == host)
        {
            let ticket = self.focus_areas.help_request.take().unwrap();
            if ticket.frame == ctx.cumulative_frame_nr()
                && self.focus_areas_allowed(ctx)
                && ticket.document == self.documents[self.active].id
                && ticket.interaction == self.document_save.interaction_generation()
                && ticket.origin == ctx.memory(|memory| memory.focused())
            {
                self.focus_areas.pending = None;
                self.focus_areas.help = Some(host);
            }
        }
        if self.focus_areas.help != Some(host) {
            return;
        }
        let maximum = (ctx.content_rect().size() - egui::vec2(48., 64.)).max(egui::vec2(80., 80.));
        let response = egui::Modal::new(egui::Id::new("navigation_shortcuts")).show(ctx, |ui| {
            ui.set_max_width(maximum.x.min(560.));
            ui.label(crate::tokens::title(ui.ctx(), "Navigation shortcuts"));
            egui::ScrollArea::vertical().max_height((maximum.y - 80.).max(32.))
                .min_scrolled_height(0.).show_themed(ui, |ui| {
                for (keys, meaning) in [
                    ("Ctrl+F6 / Ctrl+Shift+F6", "Next / previous visible focus area, wrapping within this window."),
                    ("Ctrl+0 / Ctrl+Shift+0", "Focus Command Window / active Editor. Ctrl+0 returns from the separate Editor window."),
                    ("Ctrl+Tab / Ctrl+Shift+Tab", "Next / previous script tab."),
                    ("Ctrl+L / Alt+Up", "Current Folder address / parent folder (desktop)."),
                    ("Files: Up/Down, Home/End, Enter", "Select a file and open it without executing a script."),
                    ("Figure tabs: Left/Right, Home/End", "Select an open figure while its tab has focus."),
                    ("Ctrl+F / Ctrl+H", "Find / replace in the active script."),
                    ("Ctrl+Enter / F9", "Run the active script / selected code when execution is available."),
                    ("Ctrl+/ / Escape", "Open this help / close it. Closing help does not restore an older focus owner."),
                ] { ui.strong(keys); ui.label(meaning); ui.add_space(6.); }
                ui.label("Traversal visits enabled Files, Workspace filter, visible Editor buffers, the Figure selector, Command input and an open model canvas. Hidden areas are skipped. The separate Editor cycles only its visible buffers. Pending dialogs, operations and drag gestures block traversal. Other toolkit windows retain their ordinary Tab navigation.");
            });
            ui.button("Close").clicked()
        });
        if response.inner || response.should_close() {
            self.focus_areas.help_close = Some((host, ctx.cumulative_frame_nr()));
        }
        if !ctx.will_discard()
            && self.focus_areas.help_close == Some((host, ctx.cumulative_frame_nr()))
        {
            self.focus_areas.help_close = None;
            self.focus_areas.help = None;
            ctx.request_repaint();
        }
    }
}

#[cfg(test)]
pub(super) fn observations(ctx: &egui::Context, editor: bool) -> Vec<(Area, egui::Id)> {
    let key = registry(Host {
        viewport: ctx.viewport_id(),
        editor,
    });
    let mut painted = ctx
        .data(|data| data.get_temp::<Painted>(key))
        .unwrap_or_default();
    painted.entries.sort_by_area();
    painted
        .entries
        .iter()
        .map(|entry| (entry.area, entry.id))
        .collect()
}
