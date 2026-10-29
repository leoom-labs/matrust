//! Genuine deferred figure viewports. A callback owns UI state, never Workspace.
use super::*;
use super::{
    figure_geometry::{self, Geometry},
    figure_target::{Acknowledgement, Key, Select},
};
use matrust::plot::{Figure, Position, SampleQuery};
use std::sync::{
    Mutex,
    atomic::{AtomicBool, AtomicU64},
};

#[path = "detached_capture.rs"]
pub(crate) mod capture;
#[path = "docked_scenes.rs"]
mod docked_scenes;
#[path = "detached_placement.rs"]
mod placement;
#[path = "figure_tiles.rs"]
mod tiles;
pub(super) use tiles::Authority as TileAuthority;

pub(super) const MAX_WINDOWS: usize = 16;
#[derive(Clone, Copy)]
pub(super) enum Action {
    Activate,
    Save,
    SaveAs,
    Export,
    Properties,
    Undo,
    Redo,
    Close,
    Dock,
    Reset,
    Grid,
    Legend,
    Hold,
    ScaleX,
    ScaleY,
    ClearSample,
}
pub(super) enum Command {
    Action(Action),
    Edit(Box<worker::FigureEdit>, bool),
    Pick(Position, SampleQuery),
}
struct Envelope {
    key: Key,
    revision: u64,
    sequence: u64,
    command: Command,
}
#[derive(Clone, Copy, PartialEq)]
struct ViewRecord {
    revision: u64,
    bounds: [f64; 4],
    auto: [bool; 2],
}
#[derive(Default)]
struct Mailbox {
    command: Option<Envelope>,
    view: Option<ViewRecord>,
    focus: Option<u64>,
    native: Option<placement::Message>,
    stop: bool,
    unavailable: bool,
}
struct Presentation {
    figure: Figure,
    revision: u64,
    caption: String,
    dirty: bool,
    history: (usize, usize),
    busy: bool,
    modal: bool,
    marker: Option<(Position, [f64; 2])>,
    sample: Option<String>,
    view: Option<worker::HistoryView>,
}
struct Child {
    key: Key,
    viewport: egui::ViewportId,
    presentation: Arc<Presentation>,
    geometry: Option<Geometry>,
    geometry_source: Option<Figure>,
    mailbox: Mailbox,
    last_view: Option<ViewRecord>,
    // Close is a one-shot viewport event; egui consumes it on a discarded pass.
    // Keep only an admitted intent for this frame and immutable revision.
    close_intent: Option<(u64, u64)>,
    reset: bool,
    last_frame: Option<u64>,
    focused: bool,
    minimized: bool,
    touched: u64,
    prepare_requested: bool,
    retired: bool,
    notice: String,
    capture: Option<capture::Request>,
    capture_size: Option<egui::Vec2>,
    activate: bool,
    minimum_size: egui::Vec2,
    placement_pending: bool,
    last_native: Option<placement::Observation>,
}
struct Pending {
    envelope: Envelope,
    ticket: u64,
    acknowledged: bool,
}
#[derive(Default)]
pub(super) struct Registry {
    windows: BTreeMap<u32, Arc<Mutex<Child>>>,
    sequence: Arc<AtomicU64>,
    generation: u64,
    pending: Option<Pending>,
    focus: Option<(Key, u64)>,
    owner: Option<Key>,
    last_frame: Option<u64>,
    placement: placement::Manager,
    capture_mode: bool,
    docked: docked_scenes::Coordinator,
    tiles: tiles::Tiles,
    preparation_in_flight: bool,
}
pub struct Prepare {
    // None preserves the detached owner; explicit IDs belong to a docked scene.
    pub surface: Option<egui::Id>,
    pub key: Key,
    pub revision: u64,
    pub original: Figure,
}
pub struct Prepared {
    pub request: Prepare,
    pub geometry: matrust::Result<Geometry>,
}
fn next(sequence: &AtomicU64) -> u64 {
    sequence.fetch_add(1, Ordering::Relaxed).wrapping_add(1)
}
fn callback(
    ui: &mut egui::Ui,
    class: egui::ViewportClass,
    child: &Arc<Mutex<Child>>,
    sequence: &AtomicU64,
    cancel: &AtomicBool,
) {
    let ctx = ui.ctx().clone();
    let Ok(mut state) = child.lock() else {
        ctx.request_repaint_of(egui::ViewportId::ROOT);
        return;
    };
    if state.retired {
        drop(state);
        // Registry retirement already sent Close once. A stale backend callback
        // must yield to the root, not restart a child close/repaint loop.
        ctx.request_repaint_of(egui::ViewportId::ROOT);
        return;
    }
    if class == egui::ViewportClass::EmbeddedWindow {
        state.mailbox.unavailable = true;
        ui.label(
            "Native figure windows are unavailable on this backend; the figure remains docked.",
        );
        drop(state);
        ctx.request_repaint_of(egui::ViewportId::ROOT);
        return;
    }
    let frame = ctx.cumulative_frame_nr();
    let focused = ctx.input(|i| i.viewport().focused.unwrap_or(false));
    let was_minimized = state.minimized;
    state.minimized = ctx.input(|i| i.viewport().minimized.unwrap_or(false));
    if was_minimized && !state.minimized && state.geometry.is_none() {
        state.prepare_requested = true;
    }
    if focused && !state.focused && (state.activate || !state.placement_pending) {
        let seq = next(sequence);
        state.mailbox.focus = Some(seq);
        state.touched = seq;
        state.prepare_requested = state.geometry.is_none();
    }
    state.focused = focused;
    let close_requested = ctx.input(|i| i.viewport().close_requested());
    let presentation = state.presentation.clone();
    let enabled = !presentation.busy && !presentation.modal && state.mailbox.command.is_none();
    if state.close_intent != Some((frame, presentation.revision)) || !enabled {
        state.close_intent = None;
    }
    if close_requested && enabled {
        state.close_intent = Some((frame, presentation.revision));
    }
    let closed = close_requested || state.close_intent.is_some();
    let mut command = state.close_intent.map(|_| Command::Action(Action::Close));
    let mut stop = false;
    ui.horizontal_wrapped(|ui| {
        ui.add(egui::Label::new(egui::RichText::new(&presentation.caption).strong()).truncate())
            .on_hover_text(&presentation.caption);
        ui.add_enabled_ui(enabled, |ui| {
            for (label, action) in [
                ("Save", Action::Save),
                ("Save as", Action::SaveAs),
                ("Export", Action::Export),
                ("Properties", Action::Properties),
            ] {
                if ui.button(label).clicked() {
                    command = Some(Command::Action(action));
                }
            }
            if ui
                .add_enabled(presentation.history.0 > 0, egui::Button::new("Undo"))
                .clicked()
            {
                command = Some(Command::Action(Action::Undo));
            }
            if ui
                .add_enabled(presentation.history.1 > 0, egui::Button::new("Redo"))
                .clicked()
            {
                command = Some(Command::Action(Action::Redo));
            }
            if ui
                .button("Dock figure")
                .on_hover_text("Move this view into the desktop; keep its document and edits")
                .clicked()
            {
                command = Some(Command::Action(Action::Dock));
            }
            if ui
                .button("Close figure")
                .on_hover_text("Close this document, with Save / Discard / Cancel when modified")
                .clicked()
            {
                command = Some(Command::Action(Action::Close));
            }
        });
        if ui
            .add_enabled(presentation.busy, egui::Button::new("Stop"))
            .clicked()
        {
            stop = true;
        }
    });
    ui.horizontal_wrapped(|ui| {
        ui.add_enabled_ui(enabled, |ui| {
            if ui.button("Reset view").clicked() {
                command = Some(Command::Action(Action::Reset));
            }
            ui.menu_button("View options", |ui| {
                for (label, action) in [
                    ("Toggle grid", Action::Grid),
                    ("Toggle legend", Action::Legend),
                    ("Toggle hold", Action::Hold),
                    ("Toggle X scale", Action::ScaleX),
                    ("Toggle Y scale", Action::ScaleY),
                ] {
                    if ui.button(label).clicked() {
                        command = Some(Command::Action(action));
                        ui.close();
                    }
                }
            });
        });
        if presentation.busy {
            // Worker completion refreshes the root and its child presentation.
            // Keep status/Stop without an idle child animation repaint loop.
            ui.label("Working...");
        }
        if presentation.modal {
            ui.label("Finish or cancel the decision in the desktop.");
        }
    });
    if let Some(sample) = &presentation.sample {
        ui.horizontal_wrapped(|ui| {
            ui.monospace(sample);
            if ui.button("Copy sample").clicked() {
                ctx.copy_text(sample.clone());
            }
            if ui
                .add_enabled(enabled, egui::Button::new("Clear sample"))
                .clicked()
            {
                command = Some(Command::Action(Action::ClearSample));
            }
        });
    }
    let mut view = None;
    let mut stale_geometry = false;
    let reset = state.reset;
    let key = state.key.clone();
    let Child {
        geometry,
        geometry_source,
        ..
    } = &mut *state;
    stale_geometry |= geometry.is_some() && geometry_source.is_none();
    if let (Some(geometry), Some(source)) = (geometry.as_mut(), geometry_source.as_ref()) {
        ui.add_enabled_ui(enabled, |ui| {
            use figure_geometry::scene;
            match scene::show(
                ui,
                scene::Frame {
                    key: &key,
                    revision: presentation.revision,
                    figure: &presentation.figure,
                    prepared_source: source,
                    marker: presentation.marker,
                    restored_view: presentation.view,
                },
                geometry,
                reset,
                scene::Identity::LegacyDetached,
            ) {
                Ok(outcome) if outcome.matches(&key, presentation.revision) => {
                    if let Some(interaction) = outcome.interaction {
                        command = Some(match interaction {
                            scene::Interaction::Edit(edit, undoable) => {
                                Command::Edit(edit, undoable)
                            }
                            scene::Interaction::Pick(position, query) => {
                                Command::Pick(position, query)
                            }
                        });
                    }
                    view = outcome.view.map(|view| ViewRecord {
                        revision: outcome.revision,
                        bounds: view.bounds,
                        auto: view.auto,
                    });
                }
                Ok(_) => {
                    stale_geometry = true;
                }
                Err(error) => {
                    ui.label(error);
                    stale_geometry = true;
                }
            }
        });
        state.reset = false;
    } else {
        ui.label(if state.prepare_requested {
            "Preparing figure display..."
        } else {
            "Activate this window to refresh its display."
        });
        if ui
            .add_enabled(
                !presentation.busy && !presentation.modal,
                egui::Button::new("Refresh display"),
            )
            .clicked()
        {
            state.prepare_requested = true;
        }
    }
    if stale_geometry {
        state.geometry = None;
        state.geometry_source = None;
        state.prepare_requested = true;
    }
    if !state.notice.is_empty() {
        ui.add(egui::Label::new(&state.notice).wrap());
    }
    if focused && enabled {
        let shortcut = ctx.input_mut(|i| {
            if i.consume_key(egui::Modifiers::CTRL | egui::Modifiers::SHIFT, egui::Key::S) {
                Some(Action::SaveAs)
            } else if i.consume_key(egui::Modifiers::CTRL, egui::Key::S) {
                Some(Action::Save)
            } else if i.consume_key(egui::Modifiers::CTRL, egui::Key::W) {
                Some(Action::Close)
            } else if i.consume_key(egui::Modifiers::CTRL, egui::Key::Z) {
                Some(Action::Undo)
            } else if i.consume_key(egui::Modifiers::CTRL, egui::Key::Y) {
                Some(Action::Redo)
            } else {
                None
            }
        });
        if let Some(action) = shortcut {
            command = Some(Command::Action(action));
        }
    }
    // X always wins over another action in its same frame and is cancelled at
    // the viewport until the root has completed document closure.
    if closed {
        command = Some(Command::Action(Action::Close));
    }
    let mut wake = false;
    if !ctx.will_discard() && state.last_frame != Some(frame) {
        state.last_frame = Some(frame);
        if stop {
            cancel.store(true, Ordering::Relaxed);
            state.mailbox.stop = true;
            wake = true;
        }
        if let Some(command) = command {
            if enabled {
                state.mailbox.command = Some(Envelope {
                    key: state.key.clone(),
                    revision: presentation.revision,
                    sequence: next(sequence),
                    command,
                });
                wake = true;
            } else if closed {
                state.notice =
                    "Finish the current operation or decision before closing this figure.".into();
            }
        }
        if enabled
            && let Some(view) = view
            && state.last_view != Some(view)
        {
            state.last_view = Some(view);
            state.mailbox.view = Some(view);
            wake = true;
        }
    }
    // Accepted/rejected decisions cannot become queued work in a later frame.
    if !ctx.will_discard() {
        state.close_intent = None;
    }
    if !ctx.will_discard()
        && let Some(observation) = placement::observe(&ctx)
        && (state.placement_pending || state.last_native != Some(observation))
    {
        state.last_native = Some(observation);
        state.mailbox.native = Some(placement::Message {
            viewport: state.viewport,
            sequence: next(sequence),
            observation,
        });
        wake = true;
    }
    let capture_commands = capture::child_frame(&mut state, &ctx, class);
    wake |= state.mailbox.focus.is_some() || state.prepare_requested;
    drop(state);
    if closed {
        ctx.send_viewport_cmd(egui::ViewportCommand::CancelClose);
    }
    capture_commands.dispatch(&ctx);
    if wake {
        ctx.request_repaint_of(egui::ViewportId::ROOT);
    }
}

// At most one ordinary ID plus two bounded sets of subplot metadata IDs.
// Framework cleanup must run after releasing the child mutex.
fn view_ids(child: &Child) -> Vec<egui::Id> {
    let mut ids = vec![egui::Id::new(("detached_figure", child.viewport))];
    for (position, _) in child.presentation.figure.all_axes() {
        ids.push(egui::Id::new((
            "detached_subplot",
            child.viewport,
            child.key.number,
            position,
        )));
    }
    if let Some(source) = &child.geometry_source {
        for (position, _) in source.all_axes() {
            ids.push(egui::Id::new((
                "detached_subplot",
                child.viewport,
                child.key.number,
                position,
            )));
        }
    }
    ids
}
fn live(app: &Workspace, key: &Key) -> bool {
    app.all_figures
        .get(&key.number)
        .is_some_and(|f| f.identity.same_object(&key.identity))
}
fn revision(app: &Workspace, id: u32) -> Option<u64> {
    app.figure_documents.get(&id).map(|d| d.revision)
}
fn presentation(app: &Workspace, id: u32) -> Option<Presentation> {
    let figure = app.all_figures.get(&id)?;
    let caption = app
        .figure_documents
        .get(&id)
        .and_then(|d| d.path.as_ref())
        .map(|p| {
            p.file_name()
                .unwrap_or_default()
                .to_string_lossy()
                .into_owned()
        })
        .unwrap_or_else(|| {
            if figure.name.is_empty() {
                format!("Figure {id}")
            } else {
                format!("Figure {id} - {}", figure.name)
            }
        });
    let dirty = app.figure_dirty(id);
    Some(Presentation {
        figure: figure.clone(),
        revision: revision(app, id)?,
        caption: format!("{caption}{}", if dirty { " *" } else { "" }),
        dirty,
        history: app.figure_history.get(&id).copied().unwrap_or_default(),
        busy: app.busy,
        modal: app.file_action_pending(),
        marker: app.sample_inspector.marker(id, figure),
        sample: app.sample_inspector.description(id, figure),
        view: app.figure_views.get(&id).map(|v| (v.bounds, v.auto)),
    })
}
fn root_cache_bytes(app: &Workspace) -> usize {
    app.plot_points
        .iter()
        .map(|v| v.capacity() * std::mem::size_of::<PlotPoint>())
        .sum::<usize>()
        + app.subplot_canvas.retained_point_bytes()
}
impl Registry {
    /// Retained display-point capacity across root, docked and detached scenes.
    /// Temporary conversion buffers/legend tokens are not a whole-process heap quota.
    fn retained_bytes(&self) -> usize {
        self.docked.retained_bytes()
            + self
                .windows
                .values()
                .map(|c| c.lock().unwrap_or_else(|poisoned| poisoned.into_inner()))
                .filter_map(|c| c.geometry.as_ref().map(Geometry::retained_bytes))
                .sum::<usize>()
    }
    fn reserve(&mut self, root: usize, incoming: usize) -> bool {
        if root
            .checked_add(incoming)
            .is_none_or(|n| n > figure_geometry::CACHE_BYTES)
        {
            return false;
        }
        let fits = |retained: usize| {
            root.checked_add(retained)
                .and_then(|n| n.checked_add(incoming))
                .is_some_and(|n| n <= figure_geometry::CACHE_BYTES)
        };
        if fits(self.retained_bytes()) {
            return true;
        }
        let mut caches: Vec<_> = self
            .windows
            .iter()
            .filter_map(|(&id, child)| {
                let child = child
                    .lock()
                    .unwrap_or_else(|poisoned| poisoned.into_inner());
                child
                    .geometry
                    .as_ref()
                    .map(|_| (!child.minimized, child.focused, child.touched, id))
            })
            .collect();
        caches.sort();
        for (_, _, _, id) in caches {
            if let Some(child) = self.windows.get(&id) {
                let mut child = child
                    .lock()
                    .unwrap_or_else(|poisoned| poisoned.into_inner());
                child.geometry = None;
                child.geometry_source = None;
                child.prepare_requested = false;
                child.notice =
                    "Display cache limit reached; focus this figure to prepare it again.".into();
            }
            if fits(self.retained_bytes()) {
                return true;
            }
        }
        while self.docked.evict_oldest() {
            if fits(self.retained_bytes()) {
                return true;
            }
        }
        fits(self.retained_bytes())
    }
    pub(super) fn reserve_root_cache(&mut self, root: usize, incoming: usize) -> bool {
        let pending = if self.preparation_in_flight {
            figure_geometry::ONE_CACHE_BYTES
        } else {
            0
        };
        incoming
            .checked_add(pending)
            .is_some_and(|incoming| self.reserve(root, incoming))
    }
    pub fn authorizes_close(&self, id: u32, figures: &BTreeMap<u32, Figure>) -> bool {
        self.owner
            .as_ref()
            .filter(|key| key.number == id)
            .is_none_or(|key| {
                figures
                    .get(&id)
                    .is_some_and(|figure| figure.identity.same_object(&key.identity))
            })
    }
    pub fn contains(&self, id: u32) -> bool {
        self.windows.contains_key(&id)
    }
    /// Called after final-pass tile reconciliation and preparation admission.
    pub fn capture_ready(&self) -> bool {
        !self.preparation_in_flight && self.docked.capture_ready()
    }
    pub fn has_transition(&self) -> bool {
        self.pending.is_some()
    }
    pub fn cancel_pending(&mut self) {
        self.docked.cancel();
        self.tiles.cancel_pending();
        self.pending = None;
        self.focus = None;
        for child in self.windows.values() {
            if let Ok(mut child) = child.lock() {
                child.mailbox = Mailbox::default();
                child.prepare_requested = false;
            }
        }
    }
    fn notice(&self, id: u32, text: &str) {
        if let Some(child) = self.windows.get(&id)
            && let Ok(mut child) = child.lock()
        {
            child.notice = text.into();
        }
    }
    fn retire(&mut self, id: u32, ctx: &egui::Context) -> Option<Geometry> {
        let child = self.windows.remove(&id)?;
        let mut child = child.lock().ok()?;
        let ids = view_ids(&child);
        let viewport = child.viewport;
        child.retired = true;
        child.mailbox = Mailbox::default();
        child.geometry_source = None;
        let geometry = child.geometry.take();
        drop(child);
        for id in ids {
            figure_view::forget_prepared(ctx, id);
        }
        ctx.send_viewport_cmd_to(viewport, egui::ViewportCommand::Close);
        geometry
    }
    fn register(&self, ctx: &egui::Context, cancel: &Arc<AtomicBool>) {
        for child in self.windows.values() {
            let Ok(state) = child.lock() else {
                continue;
            };
            let viewport = state.viewport;
            let title = state.presentation.caption.clone();
            let size = state.capture_size.unwrap_or(egui::vec2(900., 650.));
            let minimum = state.minimum_size;
            let activate = state.activate;
            drop(state);
            let child = child.clone();
            let sequence = self.sequence.clone();
            let cancel = cancel.clone();
            ctx.show_viewport_deferred(
                viewport,
                egui::ViewportBuilder::default()
                    .with_title(title)
                    .with_inner_size(size)
                    .with_min_inner_size(minimum)
                    .with_active(activate),
                move |ui, class| callback(ui, class, &child, &sequence, &cancel),
            );
        }
    }
    pub fn complete(
        &mut self,
        app: &Workspace,
        selected: Option<Acknowledgement>,
        prepared: Option<Prepared>,
    ) {
        if let Some(ack) = selected
            && let Some(pending) = &mut self.pending
            && pending.ticket == ack.ticket
            && pending.envelope.key.same(&ack.key)
        {
            pending.acknowledged = ack.accepted
                && live(app, &ack.key)
                && app.current_figure == ack.key.number
                && revision(app, ack.key.number) == Some(pending.envelope.revision);
            if !pending.acknowledged {
                self.pending = None;
            }
        }
        if let Some(mut prepared) = prepared {
            self.preparation_in_flight = false;
            let request = &prepared.request;
            if !live(app, &request.key)
                || revision(app, request.key.number) != Some(request.revision)
                || !app.all_figures[&request.key.number].same_snapshot(&request.original)
                || (request.surface.is_some() && !self.docked.accepts(request))
                || (request.surface.is_none()
                    && !self.windows.get(&request.key.number).is_some_and(|child| {
                        child.lock().is_ok_and(|child| child.key.same(&request.key))
                    }))
            {
                return;
            }
            if let Ok(geometry) = &prepared.geometry {
                let admitted = geometry.retained_bytes() <= figure_geometry::ONE_CACHE_BYTES
                    && self.reserve(root_cache_bytes(app), geometry.retained_bytes());
                if !admitted {
                    prepared.geometry = Err("Display cache limit reached".into());
                }
            }
            if prepared.request.surface.is_some() {
                self.docked.complete(prepared);
                return;
            }
            let request = prepared.request;
            if live(app, &request.key)
                && revision(app, request.key.number) == Some(request.revision)
                && app.all_figures[&request.key.number].same_snapshot(&request.original)
                && let Some(child) = self.windows.get(&request.key.number)
                && let Ok(mut child) = child.lock()
                && child.key.same(&request.key)
            {
                match prepared.geometry {
                    Ok(geometry) => {
                        child.geometry = Some(geometry);
                        child.geometry_source = Some(request.original);
                        child.reset = true;
                        child.notice.clear();
                    }
                    Err(error) => {
                        child.notice = error;
                        child.prepare_requested = false;
                        if self.pending.as_ref().is_some_and(|p| {
                            p.envelope.key.same(&request.key)
                                && matches!(p.envelope.command, Command::Action(Action::Dock))
                        }) {
                            self.pending = None;
                        }
                    }
                }
            }
        }
    }
    fn sync(&mut self, app: &mut Workspace, ctx: &egui::Context) {
        self.docked.sync(app, ctx, &self.windows);
        let removed: Vec<_> = self
            .windows
            .iter()
            .filter_map(|(&id, child)| {
                child
                    .lock()
                    .ok()
                    .filter(|c| live(app, &c.key))
                    .is_none()
                    .then_some(id)
            })
            .collect();
        for id in removed {
            self.retire(id, ctx);
        }
        if self.pending.as_ref().is_some_and(|p| {
            !live(app, &p.envelope.key)
                || revision(app, p.envelope.key.number) != Some(p.envelope.revision)
        }) {
            self.pending = None;
        }
        if self.focus.as_ref().is_some_and(|(key, _)| !live(app, key)) {
            self.focus = None;
        }
        if let Some(owner) = &self.owner
            && !live(app, owner)
        {
            if app.figure_close.as_ref().is_some_and(
                |flow| matches!(flow.action,figure_files::Closing::Figure(id) if id==owner.number),
            ) {
                app.figure_close = None;
            }
            self.owner = None;
        }
        let modal = app.file_action_pending();
        if self.owner.is_some()
            && !modal
            && !app.busy
            && let Some(owner) = self.owner.take()
            && let Some(child) = self.windows.get(&owner.number)
            && let Ok(child) = child.lock()
        {
            let viewport = child.viewport;
            drop(child);
            ctx.send_viewport_cmd_to(viewport, egui::ViewportCommand::Focus);
        }
        for (&id, child) in &self.windows {
            let Some(mut new) = presentation(app, id) else {
                continue;
            };
            // The registry is temporarily moved out of Workspace during advance.
            new.modal |= self.pending.is_some();
            let Ok(mut child) = child.lock() else {
                continue;
            };
            let mut forget = Vec::new();
            let old = &child.presentation;
            let changed = !old.figure.same_snapshot(&new.figure)
                || old.revision != new.revision
                || old.caption != new.caption
                || old.dirty != new.dirty
                || old.history != new.history
                || old.busy != new.busy
                || old.modal != new.modal
                || old.marker != new.marker
                || old.sample != new.sample
                || old.view != new.view;
            if child
                .geometry_source
                .as_ref()
                .is_some_and(|f| !f.same_snapshot(&new.figure))
            {
                if let Some(source) = &child.geometry_source {
                    for (position, _) in source.all_axes() {
                        if !new.figure.all_axes().any(|(p, _)| p == position) {
                            forget.push(egui::Id::new((
                                "detached_subplot",
                                child.viewport,
                                id,
                                position,
                            )));
                        }
                    }
                }
                child.geometry = None;
                child.geometry_source = None;
                child.prepare_requested = !child.minimized;
            }
            if changed {
                child.presentation = Arc::new(new);
            }
            let viewport = child.viewport;
            drop(child);
            for id in forget {
                figure_view::forget_prepared(ctx, id);
            }
            if changed {
                ctx.request_repaint_of(viewport);
            }
        }
    }
    fn view(&self, app: &mut Workspace, key: &Key, view: ViewRecord) {
        if !live(app, key)
            || revision(app, key.number) != Some(view.revision)
            || app.busy
            || app.file_action_pending()
        {
            return;
        }
        if matrust::plot::validate_limits([view.bounds[0], view.bounds[1]]).is_err()
            || matrust::plot::validate_limits([view.bounds[2], view.bounds[3]]).is_err()
        {
            return;
        }
        let old = app.figure_views.get(&key.number);
        let changed = old.is_some_and(|old| {
            (0..2).any(|a| {
                old.auto[a] != view.auto[a]
                    || ((!old.auto[a] || !view.auto[a])
                        && old.bounds[a * 2..a * 2 + 2] != view.bounds[a * 2..a * 2 + 2])
            })
        });
        let dirty = changed || old.is_some_and(|old| old.dirty);
        app.recovery_dirty |= changed;
        app.figure_views.insert(
            key.number,
            figure_files::View {
                bounds: view.bounds,
                auto: view.auto,
                dirty,
                revision: view.revision,
            },
        );
        if app.current_figure == key.number {
            app.view_bounds = Some(view.bounds);
            app.view_auto = view.auto;
        }
    }
    fn execute(&mut self, app: &mut Workspace, ctx: &egui::Context, envelope: Envelope) {
        let id = envelope.key.number;
        if !live(app, &envelope.key)
            || revision(app, id) != Some(envelope.revision)
            || app.current_figure != id
            || app.busy
            || app.file_action_pending()
        {
            return;
        }
        let view = app.figure_views.get(&id).map(|v| (v.bounds, v.auto));
        app.view_bounds = view.map(|v| v.0);
        app.view_auto = view.map_or([true; 2], |v| v.1);
        app.document_save.activate(document_save::Pane::Figure);
        let mut root_decision = false;
        match envelope.command {
            Command::Edit(edit, undoable) => app.submit(if undoable {
                Job::EditFigure(edit)
            } else {
                Job::ViewFigure(edit)
            }),
            Command::Pick(position, query) => app.inspect_plot_sample(position, query),
            Command::Action(action) => match action {
                Action::Activate => {}
                Action::Save | Action::SaveAs => {
                    ctx.send_viewport_cmd_to(egui::ViewportId::ROOT, egui::ViewportCommand::Focus);
                    app.save_figure_document(id, matches!(action, Action::SaveAs));
                    root_decision = true;
                }
                Action::Export => {
                    app.begin_figure_export();
                    root_decision = true;
                }
                Action::Properties => {
                    if let Some(figure) = &app.figure {
                        app.figure_editor =
                            Some(figure_editor::Editor::new(id, figure, app.view_bounds));
                    }
                    root_decision = true;
                }
                Action::Undo => app.submit(Job::UndoFigure(id, view)),
                Action::Redo => app.submit(Job::RedoFigure(id, view)),
                Action::Close => {
                    app.request_figure_close(id);
                    root_decision = app.figure_close.is_some();
                }
                Action::Dock => {
                    if !self.capture_mode {
                        self.placement.docked(app, &envelope.key);
                    }
                    if let Some(geometry) = self.retire(id, ctx) {
                        app.plot_points = Vec::new();
                        app.subplot_canvas.release_geometry();
                        // Transfer existing vectors, never clone or prepare during docking.
                        let _ = self.reserve(0, geometry.retained_bytes());
                        match geometry {
                            Geometry::Ordinary(p) => {
                                app.plot_points = p.points;
                                app.plot_axes = p.transforms;
                            }
                            Geometry::Subplots(canvas) => app.subplot_canvas = canvas,
                        }
                    }
                    app.reset_plot = true;
                    app.restore_plot_view = true;
                    ctx.send_viewport_cmd_to(egui::ViewportId::ROOT, egui::ViewportCommand::Focus);
                }
                Action::ClearSample => app.sample_inspector.clear(id),
                Action::Reset
                | Action::Grid
                | Action::Legend
                | Action::Hold
                | Action::ScaleX
                | Action::ScaleY => {
                    if let Some(original) = app.figure.clone() {
                        let mut updated = original.clone();
                        match action {
                            Action::Reset => {
                                updated.xlim = None;
                                updated.ylim = None;
                            }
                            Action::Grid => {
                                let enabled = !(updated.xgrid && updated.ygrid);
                                updated.xgrid = enabled;
                                updated.ygrid = enabled;
                            }
                            Action::Legend => updated.legend = !updated.legend,
                            Action::Hold => updated.hold = !updated.hold,
                            Action::ScaleX => updated.xscale = toggle_scale(updated.xscale),
                            Action::ScaleY => updated.yscale = toggle_scale(updated.yscale),
                            _ => unreachable!(),
                        }
                        // Style actions keep the displayed manual view across revision
                        // invalidation and scene re-preparation. History alone retains
                        // only the pre-edit view; the new model must retain it too.
                        // ViewRecord bounds are data coordinates, including log axes.
                        // Reset and scale actions deliberately retain their own policy.
                        if matches!(action, Action::Grid | Action::Legend | Action::Hold)
                            && let Some((bounds, auto)) = view
                        {
                            if !auto[0] {
                                updated.xlim = Some([bounds[0], bounds[1]]);
                            }
                            if !auto[1] {
                                updated.ylim = Some([bounds[2], bounds[3]]);
                            }
                        }
                        let edit = Box::new(worker::FigureEdit {
                            id,
                            original,
                            updated,
                            view_before: view,
                        });
                        app.submit(if matches!(action, Action::Reset) {
                            Job::ViewFigure(edit)
                        } else {
                            Job::EditFigure(edit)
                        });
                        if matches!(action, Action::Reset) {
                            app.figure_views.remove(&id);
                        }
                    }
                }
            },
        }
        if root_decision {
            self.owner = Some(envelope.key);
            ctx.send_viewport_cmd_to(egui::ViewportId::ROOT, egui::ViewportCommand::Focus);
        }
    }
    fn prepare(&mut self, app: &mut Workspace) {
        if app.busy || app.file_action_pending() {
            return;
        }
        if self.pending.is_none()
            && let Some(surface) = self.docked.candidate()
        {
            let admitted = self.reserve(root_cache_bytes(app), figure_geometry::ONE_CACHE_BYTES);
            if let Some(request) = self.docked.prepare(surface, admitted) {
                app.submit(Job::PrepareDetached(Box::new(request)));
                self.preparation_in_flight = app.busy;
                if !app.busy {
                    self.docked.submission_failed(surface, &app.status);
                }
                return;
            }
        }
        let preferred = self
            .pending
            .as_ref()
            .filter(|p| matches!(p.envelope.command, Command::Action(Action::Dock)))
            .map(|p| p.envelope.key.number);
        let candidate = self
            .windows
            .iter()
            .filter_map(|(&id, child)| {
                let child = child.lock().ok()?;
                ((child.prepare_requested && !child.minimized || preferred == Some(id))
                    && child.geometry.is_none())
                .then_some((preferred == Some(id), child.touched, id))
            })
            .max();
        let Some((_, _, id)) = candidate else {
            return;
        };
        if !self.reserve(root_cache_bytes(app), figure_geometry::ONE_CACHE_BYTES) {
            return;
        }
        let Some(child) = self.windows.get(&id) else {
            return;
        };
        let Ok(mut child) = child.lock() else {
            return;
        };
        if !live(app, &child.key) {
            return;
        }
        let request = Prepare {
            surface: None,
            key: child.key.clone(),
            revision: revision(app, id).unwrap(),
            original: app.all_figures[&id].clone(),
        };
        child.prepare_requested = false;
        drop(child);
        app.submit(Job::PrepareDetached(Box::new(request)));
        self.preparation_in_flight = app.busy;
    }
    fn advance(&mut self, app: &mut Workspace, ctx: &egui::Context) {
        self.sync(app, ctx);
        let frame = ctx.cumulative_frame_nr();
        if ctx.will_discard() || self.last_frame == Some(frame) {
            return;
        }
        self.last_frame = Some(frame);
        for child in self.windows.values() {
            if let Ok(mut child) = child.lock()
                && let Some(message) = child.mailbox.native.take()
            {
                self.placement.accept(&child.key, message);
            }
        }
        if app.allow_close {
            self.flush_placement(app);
            let ids: Vec<_> = self.windows.keys().copied().collect();
            for id in ids {
                self.retire(id, ctx);
            }
            self.cancel_pending();
            return;
        }
        self.advance_tiles(app, ctx);
        self.placement_frame(app, ctx);
        let mut messages = Vec::with_capacity(self.windows.len());
        for child in self.windows.values() {
            if let Ok(mut child) = child.lock() {
                messages.push((child.key.clone(), std::mem::take(&mut child.mailbox)));
            }
        }
        if messages.iter().any(|(_, m)| m.stop) {
            self.cancel_pending();
            app.stop_work();
            self.sync(app, ctx);
            return;
        }
        let mut commands = Vec::with_capacity(MAX_WINDOWS + 4);
        for (key, view, command) in self.docked.drain(frame) {
            if let Some(view) = view {
                self.view(app, &key, view);
            }
            if let Some(command) = command {
                commands.push(command);
            }
        }
        for (key, message) in messages {
            if let Some(view) = message.view {
                self.view(app, &key, view);
            }
            if let Some(sequence) = message.focus
                && self.focus.as_ref().is_none_or(|(_, old)| sequence > *old)
            {
                self.focus = Some((key.clone(), sequence));
            }
            if let Some(command) = message.command {
                commands.push(command);
            }
            if message.unavailable {
                commands.push(Envelope {
                    key: key.clone(),
                    revision: revision(app, key.number).unwrap_or(0),
                    sequence: next(&self.sequence),
                    command: Command::Action(Action::Dock),
                });
                app.status =
                    "Native figure windows unavailable; returning the figure to the desktop."
                        .into();
            }
        }
        commands.sort_by_key(|c| c.sequence);
        // Latest intent wins, but only an idle, modal-free publication is admitted.
        if let Some(command) = commands.pop()
            && !app.busy
            && !app.file_action_pending()
            && live(app, &command.key)
            && revision(app, command.key.number) == Some(command.revision)
        {
            self.focus = None;
            self.pending = Some(Pending {
                ticket: command.sequence,
                acknowledged: app.current_figure == command.key.number,
                envelope: command,
            });
        }
        if let Some((key, sequence)) = &self.focus
            && self
                .pending
                .as_ref()
                .is_some_and(|p| *sequence > p.envelope.sequence && !key.same(&p.envelope.key))
        {
            self.pending = None;
        }
        if self.pending.is_none()
            && !app.busy
            && !app.file_action_pending()
            && let Some((key, sequence)) = self.focus.take()
            && live(app, &key)
        {
            self.pending = Some(Pending {
                ticket: sequence,
                acknowledged: app.current_figure == key.number,
                envelope: Envelope {
                    revision: revision(app, key.number).unwrap(),
                    key,
                    sequence,
                    command: Command::Action(Action::Activate),
                },
            });
        }
        if !app.busy
            && !app.file_action_pending()
            && let Some(pending) = self.pending.take()
        {
            if !live(app, &pending.envelope.key)
                || revision(app, pending.envelope.key.number) != Some(pending.envelope.revision)
            {
                self.notice(
                    pending.envelope.key.number,
                    "The figure changed; choose the action again.",
                );
            } else if !pending.acknowledged {
                app.submit(Job::SelectExistingFigure(Select {
                    key: pending.envelope.key.clone(),
                    ticket: pending.ticket,
                }));
                if app.busy {
                    self.pending = Some(pending);
                }
            } else if matches!(pending.envelope.command, Command::Action(Action::Dock))
                && self
                    .windows
                    .get(&pending.envelope.key.number)
                    .is_some_and(|c| c.lock().is_ok_and(|c| c.geometry.is_none()))
            {
                self.pending = Some(pending);
            } else {
                self.execute(app, ctx, pending.envelope);
            }
        }
        self.prepare(app);
        self.sync(app, ctx);
    }
}
fn toggle_scale(scale: matrust::plot::AxisScale) -> matrust::plot::AxisScale {
    use matrust::plot::AxisScale;
    match scale {
        AxisScale::Linear => AxisScale::Log,
        AxisScale::Log => AxisScale::Linear,
    }
}
impl Workspace {
    pub(super) fn detached_frame(&mut self, ctx: &egui::Context) {
        let mut registry = std::mem::take(&mut self.detached);
        registry.advance(self, ctx);
        registry.register(ctx, &self.worker.cancel);
        self.detached = registry;
    }
    pub(super) fn undock_figure(&mut self, id: u32, ctx: &egui::Context) {
        if self.busy || self.file_action_pending() {
            return;
        }
        if self.detached.contains(id) {
            self.focus_detached(id, ctx);
            return;
        }
        let mut registry = std::mem::take(&mut self.detached);
        registry.create_window(self, id, ctx, true);
        self.detached = registry;
    }
    pub(super) fn undock_capture_figure(&mut self, id: u32, ctx: &egui::Context) {
        if self.busy || self.file_action_pending() {
            return;
        }
        let mut registry = std::mem::take(&mut self.detached);
        registry.capture_mode = true;
        registry.create_window(self, id, ctx, false);
        self.detached = registry;
    }
    pub(super) fn focus_detached(&mut self, id: u32, ctx: &egui::Context) {
        if self.busy || self.file_action_pending() {
            return;
        }
        if let Some(child) = self.detached.windows.get(&id)
            && let Ok(mut child) = child.lock()
        {
            if child.retired || !live(self, &child.key) {
                return;
            }
            let sequence = next(&self.detached.sequence);
            child.mailbox.focus = Some(sequence);
            child.prepare_requested = child.geometry.is_none();
            let viewport = child.viewport;
            drop(child);
            // Windows winit ignores Focus while minimized. This is an explicit
            // user request; restore visibility before focusing without issuing
            // a maximize/unmaximize command or changing observed child state.
            ctx.send_viewport_cmd_to(viewport, egui::ViewportCommand::Minimized(false));
            ctx.send_viewport_cmd_to(viewport, egui::ViewportCommand::Focus);
            ctx.request_repaint();
        }
    }
    pub(super) fn dock_figure(&mut self, id: u32, ctx: &egui::Context) {
        if self.busy || self.file_action_pending() {
            return;
        }
        if let Some(child) = self.detached.windows.get(&id)
            && let Ok(mut child) = child.lock()
        {
            child.mailbox.command = Some(Envelope {
                key: child.key.clone(),
                revision: child.presentation.revision,
                sequence: next(&self.detached.sequence),
                command: Command::Action(Action::Dock),
            });
            drop(child);
            ctx.request_repaint();
        }
    }
}

#[cfg(test)]
#[path = "detached_figure_tests.rs"]
mod tests;

impl Registry {
    fn create_window(&mut self, app: &mut Workspace, id: u32, ctx: &egui::Context, activate: bool) {
        if self.windows.contains_key(&id) {
            return;
        }
        if self.windows.len() >= MAX_WINDOWS {
            return;
        }
        let Some(presentation) = presentation(app, id) else {
            return;
        };
        let Some(generation) = self.generation.checked_add(1) else {
            app.status =
                "Window generation limit reached; restart before opening another detached figure."
                    .into();
            return;
        };
        self.generation = generation;
        let key = Key {
            number: id,
            identity: presentation.figure.identity.clone(),
        };
        let viewport = egui::ViewportId::from_hash_of(("detached_figure", id, generation));
        let placement_key = key.clone();
        let transferred =
            self.docked
                .detach(&key, presentation.revision, &presentation.figure, ctx);
        let (geometry, geometry_source) = transferred.map_or((None, None), |(geometry, source)| {
            (Some(geometry), Some(source))
        });
        let prepare_requested = geometry.is_none();
        let child = Child {
            key,
            viewport,
            presentation: Arc::new(presentation),
            geometry,
            geometry_source,
            mailbox: Mailbox::default(),
            last_view: None,
            close_intent: None,
            reset: true,
            last_frame: None,
            focused: false,
            minimized: false,
            touched: next(&self.sequence),
            prepare_requested,
            retired: false,
            notice: String::new(),
            capture: None,
            capture_size: None,
            activate,
            minimum_size: egui::vec2(440., 320.),
            placement_pending: !self.capture_mode,
            last_native: None,
        };
        self.windows.insert(id, Arc::new(Mutex::new(child)));
        if !self.capture_mode {
            let saved = app
                .figure_windows
                .get(&id)
                .filter(|p| p.identity.same_object(&placement_key.identity))
                .map(|p| p.window.clone());
            if let Ok(window) = matrust::session::figure_window::FigureWindow::new(
                true,
                saved.as_ref().and_then(|v| v.normal()).cloned(),
                saved.as_ref().is_some_and(|v| v.maximized()),
            ) {
                placement::publish(app, &placement_key, window.clone());
                self.placement
                    .created(placement_key, viewport, Some(window), activate);
            }
        }
        ctx.request_repaint();
    }
}
