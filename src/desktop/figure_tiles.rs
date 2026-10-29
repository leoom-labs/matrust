//! Visible independent figures. Slots contain identities, never engine selections.
use super::*;
use crate::session::{FigureTileSize, FigureTiling};
use crate::theme::ThemedScrollArea;

#[derive(Clone)]
pub(crate) struct Authority {
    pub key: Key,
    pub revision: u64,
}
impl Authority {
    fn same(&self, other: &Self) -> bool {
        self.key.same(&other.key) && self.revision == other.revision
    }
}
enum Choice {
    Mode(Option<FigureTiling>),
    Size(Option<FigureTileSize>),
    Assign(usize, Key),
    Undock(Authority),
    Focus(Key),
}
#[derive(Default)]
pub(super) struct Tiles {
    slots: [Option<Key>; 4],
    active: Option<Key>,
    incoming: Option<Key>,
    pending: Option<(u64, Choice)>,
}
impl Tiles {
    pub(super) fn cancel_pending(&mut self) {
        self.pending = None;
    }
}
fn count(mode: Option<FigureTiling>) -> usize {
    match mode {
        None => 0,
        Some(FigureTiling::Two) => 2,
        Some(FigureTiling::Four) => 4,
    }
}
impl Registry {
    pub(crate) fn hide_tiles(&mut self) {
        self.docked.show_only(&[]);
    }
    pub(crate) fn reveal_tile(&mut self, number: u32, figures: &BTreeMap<u32, Figure>) {
        self.tiles.incoming = figures.get(&number).map(|figure| Key {
            number,
            identity: figure.identity.clone(),
        });
    }
    fn tile_authority(&self, app: &Workspace, key: &Key) -> Option<Authority> {
        if !live(app, key)
            || self.contains(key.number)
            || !self.tiles.slots[..count(app.layout.figure_tiling)]
                .iter()
                .flatten()
                .any(|k| k.same(key))
        {
            return None;
        }
        Some(Authority {
            key: key.clone(),
            revision: revision(app, key.number)?,
        })
    }
    pub(crate) fn valid_tile(&self, app: &Workspace, authority: &Authority) -> bool {
        self.tile_authority(app, &authority.key)
            .is_some_and(|actual| actual.same(authority))
    }
    pub(super) fn tile_action(
        &mut self,
        app: &Workspace,
        ctx: &egui::Context,
        authority: &Authority,
        action: Action,
    ) -> bool {
        self.valid_tile(app, authority)
            && self.pending.is_none()
            && self
                .docked
                .action(app, ctx, &authority.key, action, &self.sequence)
    }
    fn reconcile_tiles(&mut self, app: &mut Workspace, ctx: &egui::Context) {
        for slot in &mut self.tiles.slots {
            if slot.as_ref().is_some_and(|key| !live(app, key)) {
                *slot = None;
            }
        }
        let visible = count(app.layout.figure_tiling);
        if visible == 0 {
            self.hide_tiles();
            return;
        }
        if let Some(key) = self.tiles.incoming.take().filter(|key| live(app, key)) {
            if let Some(index) = self
                .tiles
                .slots
                .iter()
                .position(|slot| slot.as_ref().is_some_and(|old| old.same(&key)))
            {
                if index >= visible {
                    self.tiles.slots.swap(index, visible - 1);
                }
            } else {
                let index = self.tiles.slots[..visible]
                    .iter()
                    .position(Option::is_none)
                    .unwrap_or(visible - 1);
                self.tiles.slots[index] = Some(key.clone());
            }
            self.tiles.active = Some(key);
        }
        // Populate unassigned slots only from existing figures. Recovery never
        // creates a Figure and never restores stale serialized object identities.
        for id in &app.figure_ids {
            if self
                .tiles
                .slots
                .iter()
                .flatten()
                .any(|key| key.number == *id)
            {
                continue;
            }
            let Some(index) = self.tiles.slots.iter().position(Option::is_none) else {
                break;
            };
            if let Some(figure) = app.all_figures.get(id) {
                self.tiles.slots[index] = Some(Key {
                    number: *id,
                    identity: figure.identity.clone(),
                });
            }
        }
        let keys: Vec<_> = self
            .tiles
            .slots
            .iter()
            .flatten()
            .filter(|key| !self.contains(key.number))
            .cloned()
            .collect();
        if let Err(error) = self.docked.configure(app, ctx, &keys, &self.windows) {
            // The model can change only between passes. Invalid metadata does
            // not replace already prepared owners or select another figure.
            app.status = error;
            return;
        }
        let visible_keys: Vec<_> = self.tiles.slots[..visible]
            .iter()
            .flatten()
            .cloned()
            .collect();
        self.docked.show_only(&visible_keys);
        if self
            .tiles
            .active
            .as_ref()
            .is_none_or(|key| !visible_keys.iter().any(|k| k.same(key)))
        {
            self.tiles.active = visible_keys.first().cloned();
        }
    }
    pub(super) fn advance_tiles(&mut self, app: &mut Workspace, ctx: &egui::Context) {
        let frame = ctx.cumulative_frame_nr();
        if let Some((queued, choice)) = self.tiles.pending.take()
            && queued == frame
            && !app.busy
            && !app.file_action_pending()
            && self.pending.is_none()
        {
            match choice {
                Choice::Mode(None) => {
                    if let Some(key) = &self.tiles.active {
                        let _ = self
                            .docked
                            .action(app, ctx, key, Action::Activate, &self.sequence);
                    }
                    app.layout.figure_tiling = None;
                    app.recovery_dirty = true;
                    self.hide_tiles();
                }
                Choice::Mode(mode) => {
                    app.layout.figure_tiling = mode;
                    app.recovery_dirty = true;
                    self.reconcile_tiles(app, ctx);
                }
                Choice::Size(size) if app.layout.figure_tiling.is_some() => {
                    app.layout.figure_tile_size = size;
                    app.recovery_dirty = true;
                    ctx.request_repaint();
                }
                Choice::Assign(slot, key)
                    if slot < count(app.layout.figure_tiling) && live(app, &key) =>
                {
                    if let Some(old) = self
                        .tiles
                        .slots
                        .iter()
                        .position(|s| s.as_ref().is_some_and(|k| k.same(&key)))
                    {
                        self.tiles.slots.swap(old, slot);
                    } else {
                        self.tiles.slots[slot] = Some(key.clone());
                    }
                    self.tiles.active = Some(key.clone());
                    self.reconcile_tiles(app, ctx);
                    if !self.contains(key.number) {
                        self.docked.request(&key);
                        let _ =
                            self.docked
                                .action(app, ctx, &key, Action::Activate, &self.sequence);
                    }
                }
                Choice::Undock(authority) if self.valid_tile(app, &authority) => {
                    self.create_window(app, authority.key.number, ctx, true);
                }
                Choice::Focus(key) if live(app, &key) => {
                    if let Some(child) = self.windows.get(&key.number)
                        && let Ok(mut child) = child.lock()
                        && child.key.same(&key)
                        && !child.retired
                    {
                        child.mailbox.focus = Some(next(&self.sequence));
                        child.prepare_requested = child.geometry.is_none();
                        let viewport = child.viewport;
                        drop(child);
                        ctx.send_viewport_cmd_to(viewport, egui::ViewportCommand::Minimized(false));
                        ctx.send_viewport_cmd_to(viewport, egui::ViewportCommand::Focus);
                    }
                }
                _ => {}
            }
        }
    }
    fn paint_tiles(&mut self, app: &mut Workspace, ui: &mut egui::Ui) {
        self.reconcile_tiles(app, ui.ctx());
        let n = count(app.layout.figure_tiling);
        if n == 0 {
            return;
        }
        let size = ui.available_size().max(egui::Vec2::splat(1.));
        let readable = app.layout.figure_tile_size == Some(FigureTileSize::Readable);
        let enabled = !app.busy && !app.file_action_pending() && self.pending.is_none();
        if !readable {
            // The finite parent pane owns the entire grid. Never enlarge it to
            // the readable minimum or retain a previous scrollbar offset.
            let _area = self.paint_tile_grid(app, ui, n, size, enabled);
            #[cfg(test)]
            ui.ctx().data_mut(|data| {
                data.insert_temp(
                    egui::Id::new("figure_tile_scroll_observation"),
                    (_area, size, egui::Vec2::ZERO),
                )
            });
            return;
        }
        let rows = n / 2;
        let extent = egui::vec2(size.x.max(720.), size.y.max(rows as f32 * 280.));
        let _tiles_scroll = egui::ScrollArea::both()
            .id_salt("independent_figure_tiles")
            .scroll_source(egui::scroll_area::ScrollSource::SCROLL_BAR)
            .auto_shrink([false, false])
            .show_themed(ui, |ui| self.paint_tile_grid(app, ui, n, extent, enabled));
        // Observations expose actual geometry, never control scroll state.
        #[cfg(test)]
        ui.ctx().data_mut(|data| {
            data.insert_temp(
                egui::Id::new("figure_tile_scroll_observation"),
                (
                    _tiles_scroll.inner_rect,
                    _tiles_scroll.content_size,
                    _tiles_scroll.state.offset,
                ),
            )
        });
    }
    fn paint_tile_grid(
        &mut self,
        app: &mut Workspace,
        ui: &mut egui::Ui,
        n: usize,
        extent: egui::Vec2,
        enabled: bool,
    ) -> egui::Rect {
        let rows = n / 2;
        let (area, _) = ui.allocate_exact_size(extent, egui::Sense::hover());
        let gap = 8_f32.min(extent.x).min(extent.y);
        let cell = egui::vec2(
            (extent.x - gap) / 2.,
            (extent.y - gap * (rows - 1) as f32) / rows as f32,
        );
        for slot in 0..n {
            let rect = egui::Rect::from_min_size(
                area.min
                    + egui::vec2(
                        (slot % 2) as f32 * (cell.x + gap),
                        (slot / 2) as f32 * (cell.y + gap),
                    ),
                cell,
            );
            ui.scope_builder(
                egui::UiBuilder::new()
                    .id_salt(("tile_slot", slot))
                    .max_rect(rect),
                |ui| {
                    ui.set_clip_rect(rect.intersect(ui.clip_rect()));
                    let selected = self.tiles.slots[slot].as_ref().is_some_and(|key| {
                        self.tiles
                            .active
                            .as_ref()
                            .is_some_and(|active| active.same(key))
                    });
                    let frame = egui::Frame::group(ui.style());
                    let frame = if selected {
                        frame.stroke(ui.visuals().selection.stroke)
                    } else {
                        frame
                    };
                    frame.show(ui, |ui| {
                        ui.set_min_size((cell - egui::vec2(14., 14.)).max(egui::Vec2::ZERO));
                        self.paint_tile(app, ui, slot, enabled);
                    });
                },
            );
        }
        area
    }
    fn paint_tile(&mut self, app: &mut Workspace, ui: &mut egui::Ui, slot: usize, enabled: bool) {
        let key = self.tiles.slots[slot].clone();
        let authority = key.as_ref().and_then(|key| self.tile_authority(app, key));
        if let Some(authority) = &authority {
            app.document_save.register_tile(
                slot,
                authority.clone(),
                ui.available_rect_before_wrap().intersect(ui.clip_rect()),
                ui.layer_id(),
            );
            if self
                .tiles
                .active
                .as_ref()
                .is_some_and(|key| key.same(&authority.key))
            {
                app.document_save.default_tile(authority.clone());
            }
            let press = ui.ctx().input(|input| {
                input.events.iter().rev().find_map(|event| match event {
                    egui::Event::PointerButton {
                        pos, pressed: true, ..
                    } => Some(*pos),
                    _ => None,
                })
            });
            let pressed = press.is_some_and(|pos| {
                ui.available_rect_before_wrap()
                    .intersect(ui.clip_rect())
                    .contains(pos)
                    && ui.ctx().layer_id_at(pos) == Some(ui.layer_id())
            });
            if enabled && pressed && !egui::Popup::is_any_open(ui.ctx()) {
                self.tiles.active = Some(authority.key.clone());
                app.document_save.activate_tile(authority.clone());
            }
        }
        let caption = key
            .as_ref()
            .and_then(|key| presentation(app, key.number))
            .map_or_else(|| "Empty tile".into(), |p| p.caption);
        let mut command = None;
        ui.horizontal_wrapped(|ui| {
            egui::ComboBox::from_id_salt(("figure_tile_choice", slot))
                .selected_text(&caption)
                .width(150.)
                .show_ui(ui, |ui| {
                    for (&number, figure) in &app.all_figures {
                        if ui
                            .add_enabled(
                                enabled,
                                egui::Button::selectable(
                                    key.as_ref().is_some_and(|k| k.number == number),
                                    format!("Figure {number}"),
                                ),
                            )
                            .clicked()
                        {
                            self.tiles.pending = Some((
                                ui.ctx().cumulative_frame_nr(),
                                Choice::Assign(
                                    slot,
                                    Key {
                                        number,
                                        identity: figure.identity.clone(),
                                    },
                                ),
                            ));
                        }
                    }
                });
            if let Some(authority) = &authority {
                if ui.add_enabled(enabled, egui::Button::new("Save")).clicked() {
                    command = Some(Action::Save);
                }
                ui.add_enabled_ui(enabled, |ui| {
                    ui.menu_button("Actions", |ui| {
                        for (label, action) in [
                            ("Save as...", Action::SaveAs),
                            ("Export...", Action::Export),
                            ("Properties...", Action::Properties),
                            ("Undo edit", Action::Undo),
                            ("Redo edit", Action::Redo),
                            ("Reset view", Action::Reset),
                            ("Grid", Action::Grid),
                            ("Legend", Action::Legend),
                            ("Close figure", Action::Close),
                        ] {
                            if ui.button(label).clicked() {
                                command = Some(action);
                                ui.close();
                            }
                        }
                        if ui.button("Detach figure").clicked() {
                            self.tiles.pending = Some((
                                ui.ctx().cumulative_frame_nr(),
                                Choice::Undock(authority.clone()),
                            ));
                            ui.close();
                        }
                    })
                });
            }
        });
        let Some(key) = key else {
            ui.weak("Open a figure or run a plotting command.");
            return;
        };
        if self.contains(key.number) {
            ui.weak("This figure is in its own window.");
            if ui
                .add_enabled(enabled, egui::Button::new("Focus figure window"))
                .clicked()
            {
                self.tiles.pending = Some((ui.ctx().cumulative_frame_nr(), Choice::Focus(key)));
            }
            return;
        }
        // Axes.title is artwork inside the prepared figure canvas. The selector
        // above retains document/window identity; do not duplicate its axes title.
        if !self.docked.has_geometry(&key)
            && ui
                .add_enabled(enabled, egui::Button::new("Prepare display"))
                .on_hover_text("Prepare an evicted or cancelled scene using the existing worker")
                .clicked()
        {
            self.docked.request(&key);
        }
        let plot_rect = ui.available_rect_before_wrap().intersect(ui.clip_rect());
        if let Err(error) = ui
            .add_enabled_ui(enabled, |ui| {
                if app.layout.figure_tile_size == Some(FigureTileSize::Readable) {
                    self.docked.paint(ui, app, &key, &self.sequence)
                } else {
                    self.docked.paint_fitted(ui, app, &key, &self.sequence)
                }
            })
            .inner
        {
            ui.weak(error);
        }
        if let Some(authority) = authority {
            let press = ui.ctx().input(|input| {
                input.events.iter().rev().find_map(|event| match event {
                    egui::Event::PointerButton {
                        pos,
                        pressed: false,
                        ..
                    } => Some(*pos),
                    _ => None,
                })
            });
            if enabled
                && command.is_none()
                && !self.docked.has_command(&key)
                && !egui::Popup::is_any_open(ui.ctx())
                && press.is_some_and(|pos| {
                    plot_rect.contains(pos) && ui.ctx().layer_id_at(pos) == Some(ui.layer_id())
                })
            {
                let _ = self.tile_action(app, ui.ctx(), &authority, Action::Activate);
            }
            if let Some(action) = command {
                self.tiles.active = Some(key.clone());
                app.document_save.activate_tile(authority.clone());
                let _ = self.tile_action(app, ui.ctx(), &authority, action);
            }
        }
    }
}

impl Workspace {
    /// Single / 2-up / 4-up and, while tiled, the sizing choices. `reverse`
    /// adds them right to left, for a right-aligned header.
    pub(crate) fn figure_tile_controls(&mut self, ui: &mut egui::Ui, reverse: bool) {
        enum Control {
            Mode(&'static str, Option<FigureTiling>),
            Size(&'static str, Option<FigureTileSize>),
            Separator,
        }
        let enabled = !self.busy && !self.file_action_pending();
        let mut controls = vec![
            Control::Mode("Single", None),
            Control::Mode("2-up", Some(FigureTiling::Two)),
            Control::Mode("4-up", Some(FigureTiling::Four)),
        ];
        if self.layout.figure_tiling.is_some() {
            controls.extend([
                Control::Separator,
                Control::Size("Fit all figures", None),
                Control::Size("Readable size", Some(FigureTileSize::Readable)),
            ]);
        }
        if reverse {
            controls.reverse();
        }
        for control in controls {
            match control {
                Control::Mode(label, mode) => {
                    if ui
                        .add_enabled(
                            enabled,
                            egui::Button::selectable(self.layout.figure_tiling == mode, label),
                        )
                        .clicked()
                    {
                        self.detached.tiles.pending =
                            Some((ui.ctx().cumulative_frame_nr(), Choice::Mode(mode)));
                    }
                }
                Control::Size(label, size) => {
                    if ui
                        .add_enabled(
                            enabled,
                            egui::Button::selectable(self.layout.figure_tile_size == size, label),
                        )
                        .clicked()
                    {
                        self.detached.tiles.pending =
                            Some((ui.ctx().cumulative_frame_nr(), Choice::Size(size)));
                    }
                }
                Control::Separator => {
                    ui.separator();
                }
            }
        }
    }
    pub(crate) fn figure_tiles(&mut self, ui: &mut egui::Ui) {
        let mut registry = std::mem::take(&mut self.detached);
        registry.paint_tiles(self, ui);
        self.detached = registry;
    }
    pub(crate) fn dispatch_tile(
        &mut self,
        ctx: &egui::Context,
        authority: &Authority,
        action: Action,
    ) -> bool {
        let mut registry = std::mem::take(&mut self.detached);
        let accepted = registry.tile_action(self, ctx, authority, action);
        self.detached = registry;
        accepted
    }
}

#[cfg(test)]
#[path = "figure_tile_tests.rs"]
mod tests;
