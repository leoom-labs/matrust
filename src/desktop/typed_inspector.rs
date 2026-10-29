//! Shared typed navigation, with numeric leaves delegated to the rooted editor.
use super::*;
use crate::theme::ThemedScrollArea;
use matrust::{Array, GraphicsKind};

const MAX_DEPTH: usize = 16;
const ROW_HEIGHT: f32 = 24.;

#[derive(Default)]
pub(super) struct State {
    root: Option<(String, Array)>,
    pub(super) path: Vec<usize>,
    pub(super) replacement: cell_replace_ui::State,
    replacement_open: Option<(u64, Vec<usize>)>,
    first: usize,
    changed: bool,
    action_frame: Option<u64>,
}
impl State {
    pub(super) fn reset_navigation(&mut self) {
        self.path.clear();
        self.first = 1;
        // Retain prior root so sync still distinguishes a genuine changed value.
    }
    pub(super) fn accept_edit(&mut self, before: &Array, after: &Array) {
        if let Some((_, root)) = &mut self.root
            && root.same_snapshot(before)
        {
            *root = after.clone();
        }
    }
    fn sync(&mut self, name: &str, root: &Array) {
        let same = self
            .root
            .as_ref()
            .is_some_and(|(n, old)| n == name && old.same_snapshot(root));
        if !same {
            self.changed = self.root.as_ref().is_some_and(|(n, _)| n == name);
            self.root = Some((name.into(), root.clone()));
            self.path.clear();
            self.first = 1;
        }
    }
    fn leaf<'a>(&self, root: &'a Array) -> Option<&'a Array> {
        let mut leaf = root;
        for &index in self.path.iter().take(MAX_DEPTH) {
            let Array::Cell(cell) = leaf else { return None };
            leaf = cell.get_linear(index).ok()?.as_ref();
        }
        Some(leaf)
    }
    fn show(
        &mut self,
        ui: &mut egui::Ui,
        name: &str,
        root: &Array,
        figures: &BTreeMap<u32, Figure>,
        enabled: bool,
    ) -> Option<(Array, Vec<usize>)> {
        self.sync(name, root);
        ui.label(
            "Open numeric or character cell contents to edit. Replace changes one cell from an existing workspace value. Graphics properties are read-only.",
        );
        if self.changed {
            ui.small("Variable changed; returned to the root value.");
        }
        let mut truncate = None;
        ui.add_enabled_ui(enabled, |ui| {
            ui.horizontal_wrapped(|ui| {
                if ui.button("Root").clicked() {
                    truncate = Some(0);
                }
                let mut value = root;
                for (depth, &index) in self.path.iter().enumerate() {
                    let Array::Cell(cell) = value else { break };
                    ui.label("/");
                    if ui
                        .button(format!(
                            "Cell {{{},{}}}",
                            index % cell.rows() + 1,
                            index / cell.rows() + 1
                        ))
                        .clicked()
                    {
                        truncate = Some(depth + 1);
                    }
                    let Ok(child) = cell.get_linear(index) else {
                        break;
                    };
                    value = child.as_ref();
                }
            });
        });
        let frame = ui.ctx().cumulative_frame_nr();
        if enabled
            && !ui.ctx().will_discard()
            && self.action_frame != Some(frame)
            && let Some(depth) = truncate.take()
        {
            self.path.truncate(depth);
            self.first = 1;
            self.action_frame = Some(frame);
        }
        let Some(value) = self.leaf(root) else {
            self.path.clear();
            self.first = 1;
            return None;
        };
        if value.is_numeric_array() || matches!(value, Array::Character(_) | Array::Typed(_)) {
            return Some((value.clone(), self.path.clone()));
        }
        ui.label(format!(
            "{} rows x {} columns | {}",
            value.rows(),
            value.cols(),
            workspace_type_label(value)
        ));
        if value.is_empty() {
            ui.label("Empty value; no elements.");
        }
        let count = value.len();
        ui.add_enabled_ui(enabled && count != 0, |ui| {
            ui.horizontal(|ui| {
                ui.label("First item (column-major)");
                ui.add(egui::DragValue::new(&mut self.first).range(1..=count.max(1)));
            });
        });
        self.first = self.first.clamp(1, count.max(1));
        if matches!(value, Array::Graphics(_)) {
            ui.small("State refers to the current workspace figure snapshot. Handles are not numeric IDs.");
        }
        let mut descend = None;
        let mut replace = None;
        let start = self.first - 1;
        let height = ui.available_height().clamp(48., 320.);
        egui::ScrollArea::both()
            .id_salt(("typed_items", name, &self.path, self.first))
            .max_height(height)
            .show_rows_themed(ui, ROW_HEIGHT, count.saturating_sub(start), |ui, rows| {
                for offset in rows.take(32) {
                    let index = start + offset;
                    ui.push_id(index, |ui| {
                        ui.horizontal(|ui| {
                            ui.set_min_height(ROW_HEIGHT);
                            match value {
                                Array::Cell(cell) => {
                                    let Ok(child) = cell.get_linear(index) else {
                                        return;
                                    };
                                    let row = index % cell.rows() + 1;
                                    let col = index / cell.rows() + 1;
                                    if ui
                                        .add_enabled(
                                            enabled && self.path.len() < MAX_DEPTH,
                                            egui::Button::new(format!("Open {{{row},{col}}}")),
                                        )
                                        .clicked()
                                    {
                                        descend = Some(index);
                                    }
                                    if ui
                                        .add_enabled(
                                            enabled && self.path.len() < MAX_DEPTH,
                                            egui::Button::new(format!("Replace {{{row},{col}}}")),
                                        )
                                        .clicked()
                                    {
                                        replace = Some(index);
                                    }
                                    ui.label(format!(
                                        "{} x {} | {}",
                                        child.rows(),
                                        child.cols(),
                                        workspace_type_label(child)
                                    ));
                                }
                                Array::Graphics(handles) => {
                                    let Ok(handle) = handles.get_linear(index) else {
                                        return;
                                    };
                                    let live = figures
                                        .iter()
                                        .take(16)
                                        .find(|(_, figure)| handle.is_live_in(figure))
                                        .map(|(&id, _)| id);
                                    let state = live.map_or_else(
                                        || "Deleted / unavailable".into(),
                                        |id| format!("Live in Figure {id}"),
                                    );
                                    ui.monospace(format!(
                                        "Handle {} | {} | {state}",
                                        index + 1,
                                        graphics_kind(handle.kind())
                                    ));
                                }
                                _ => {
                                    let row = index % value.rows();
                                    let col = index / value.rows();
                                    let Ok(scalar) = value.scalar(row, col) else {
                                        return;
                                    };
                                    ui.label(format!("({},{})", row + 1, col + 1));
                                    let _ = scalar_cell_button(ui, scalar, false);
                                    if ui
                                        .add_enabled(enabled, egui::Button::new("Copy value"))
                                        .clicked()
                                    {
                                        ui.ctx().copy_text(scalar.to_string());
                                    }
                                }
                            }
                        });
                    });
                }
            });
        let frame = ui.ctx().cumulative_frame_nr();
        if let Some(index) = replace
            && self.replacement_open.is_none()
            && self.action_frame != Some(frame)
        {
            let mut path = Vec::with_capacity(self.path.len() + 1);
            path.extend_from_slice(&self.path);
            path.push(index);
            self.replacement_open = Some((frame, path));
        }
        if enabled && !ui.ctx().will_discard() && self.action_frame != Some(frame) {
            if let Some(depth) = truncate {
                self.path.truncate(depth);
                self.first = 1;
                self.action_frame = Some(frame);
            } else if let Some(index) = descend {
                self.path.push(index);
                self.first = 1;
                self.action_frame = Some(frame);
            }
        }
        None
    }
}
fn graphics_kind(kind: GraphicsKind) -> &'static str {
    match kind {
        GraphicsKind::Figure => "Figure",
        GraphicsKind::Axes => "Axes",
        GraphicsKind::Series => "Series",
        GraphicsKind::TextBox => "TextBox",
        GraphicsKind::Legend => "Legend",
        GraphicsKind::Baseline => "Baseline",
    }
}

impl Workspace {
    pub(super) fn variable_editor(&mut self, ctx: &egui::Context) {
        let Some(name) = self.editing_variable.clone() else {
            self.numeric_editor.invalidate(false);
            self.typed_inspector = Default::default();
            return;
        };
        let Some(matrix) = self.variables.get(&name).cloned() else {
            self.editing_variable = None;
            self.numeric_editor.invalidate(false);
            self.typed_inspector = Default::default();
            return;
        };
        let mut open = true;
        egui::Window::new(format!("Variable Editor — {name}"))
            .id(egui::Id::new("variable_editor"))
            // Keep the initial inspector clear of the desktop menu row.
            .default_pos(ctx.content_rect().center() - egui::vec2(450., 250.))
            .open(&mut open)
            .default_size([900., 500.])
            .show(ctx, |ui| {
                ui.label(format!(
                    "{} rows × {} columns · {}",
                    matrix.rows(),
                    matrix.cols(),
                    workspace_type_label(&matrix)
                ));
                let enabled = !self.busy && !self.file_action_pending() && !self.allow_close;
                self.cell_replacement_ui(ui, enabled);
                let enabled = !self.busy && !self.file_action_pending() && !self.allow_close;
                if matrix.is_numeric_array()
                    || matches!(matrix, Array::Character(_) | Array::Typed(_))
                {
                    self.typed_inspector.sync(&name, &matrix);
                    self.numeric_variable_ui(ui, &name, &matrix, &matrix, vec![], enabled);
                } else if let Some((leaf, path)) =
                    self.typed_inspector
                        .show(ui, &name, &matrix, &self.all_figures, enabled)
                {
                    self.numeric_variable_ui(ui, &name, &matrix, &leaf, path, enabled);
                } else {
                    self.numeric_editor.invalidate(false);
                }
                if !ctx.will_discard()
                    && let Some((frame, path)) = self.typed_inspector.replacement_open.take()
                    && enabled
                    && frame == ctx.cumulative_frame_nr()
                {
                    use matrust::variable_edit::cell_replace::{Binding, CellTarget};
                    if let Ok(target) = Binding::new(name.clone(), matrix.clone())
                        .and_then(|binding| CellTarget::new(binding, path))
                    {
                        self.typed_inspector
                            .replacement
                            .open(target, self.numeric_editor.revision());
                        self.typed_inspector.action_frame = Some(frame);
                        ctx.request_repaint();
                    }
                }
            });
        if !open {
            self.editing_variable = None;
            self.numeric_editor.invalidate(false);
            self.typed_inspector = Default::default();
        }
    }
}
