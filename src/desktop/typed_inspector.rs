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
    /// `disp` text of the last text-only value (tables, structs, ...), per snapshot.
    text: Option<(Array, String)>,
}
impl State {
    pub(super) fn reset_navigation(&mut self) {
        self.path.clear();
        self.first = 1;
        // Retain prior root so sync still distinguishes a genuine changed value.
    }
    pub(super) fn open_cell(&mut self, index: usize, frame: u64) {
        if self.path.len() < MAX_DEPTH && self.action_frame != Some(frame) {
            self.path.push(index);
            self.first = 1;
            self.action_frame = Some(frame);
        }
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
        if matches!(root, Array::Cell(_) | Array::Graphics(_)) {
            ui.label(
                "Edit scalar cell contents in the grid. Double-click other contents to open them. Replace uses a workspace value.",
            );
        }
        if self.changed {
            ui.small("Variable changed; returned to the root value.");
        }
        let mut truncate = None;
        ui.add_enabled_ui(enabled, |ui| {
            // Breadcrumb: frameless path segments in text 2.
            ui.horizontal_wrapped(|ui| {
                ui.visuals_mut().widgets.inactive.fg_stroke.color =
                    tokens::palette(ui.ctx()).text_muted;
                if ui.add(egui::Button::new("Root").frame(false)).clicked() {
                    truncate = Some(0);
                }
                let mut value = root;
                for (depth, &index) in self.path.iter().enumerate() {
                    let Array::Cell(cell) = value else { break };
                    ui.label("/");
                    if ui
                        .add(
                            egui::Button::new(format!(
                                "Cell {{{},{}}}",
                                index % cell.rows() + 1,
                                index / cell.rows() + 1
                            ))
                            .frame(false),
                        )
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
        if value.is_numeric_array()
            || matches!(value, Array::Character(_) | Array::Typed(_) | Array::Str(_))
            || matches!(value, Array::Table(t) if !t.is_timetable())
        {
            return Some((value.clone(), self.path.clone()));
        }
        if text_only(value) {
            self.text_view(ui, value);
            return None;
        }
        if let Array::Cell(cell) = value {
            self.cell_navigation(ui, name, cell, enabled);
            let value = self.leaf(root)?;
            return Some((value.clone(), self.path.clone()));
        }
        ui.label(format!(
            "{} rows x {} columns | {}",
            value.rows(),
            value.cols(),
            workspace_type_label(value)
        ));
        if matches!(value, Array::Str(_)) {
            ui.label("String values are read-only.");
        }
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
                                Array::Str(s) => {
                                    ui.label(format!(
                                        "({},{})",
                                        index % s.rows() + 1,
                                        index / s.rows() + 1
                                    ));
                                    ui.monospace(matrust::StringArray::preview_item(
                                        &s.data()[index],
                                    ));
                                }
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
    /// Compact, virtualized access to the established Open/Replace workflow.
    /// Three items per row leave space for the spreadsheet in a normal window.
    fn cell_navigation(
        &mut self,
        ui: &mut egui::Ui,
        name: &str,
        cell: &matrust::CellArray,
        enabled: bool,
    ) {
        let count = cell.len();
        ui.add_enabled_ui(enabled && count != 0, |ui| {
            ui.horizontal(|ui| {
                ui.label("First item (column-major)");
                ui.add(egui::DragValue::new(&mut self.first).range(1..=count.max(1)));
            });
        });
        self.first = self.first.clamp(1, count.max(1));
        let start = self.first - 1;
        let mut open = None;
        let mut replace = None;
        egui::ScrollArea::vertical()
            .id_salt(("cell_navigation", name, &self.path, self.first))
            .max_height(60.)
            .show_rows_themed(
                ui,
                ROW_HEIGHT,
                count.saturating_sub(start).div_ceil(3),
                |ui, rows| {
                    for row in rows.take(8) {
                        ui.horizontal(|ui| {
                            for offset in 0..3 {
                                let index = start + row * 3 + offset;
                                if index >= count {
                                    break;
                                }
                                let (r, c) = (index % cell.rows() + 1, index / cell.rows() + 1);
                                ui.push_id(index, |ui| {
                                    ui.add_enabled_ui(
                                        enabled && self.path.len() < MAX_DEPTH,
                                        |ui| {
                                            if ui.button(format!("Open {{{r},{c}}}")).clicked() {
                                                open = Some(index);
                                            }
                                            if ui.button(format!("Replace {{{r},{c}}}")).clicked() {
                                                replace = Some(index);
                                            }
                                        },
                                    );
                                });
                            }
                        });
                    }
                },
            );
        let frame = ui.ctx().cumulative_frame_nr();
        if enabled && self.action_frame != Some(frame) {
            if let Some(index) = replace
                && self.replacement_open.is_none()
            {
                let mut path = self.path.clone();
                path.push(index);
                self.replacement_open = Some((frame, path));
            }
            if !ui.ctx().will_discard()
                && let Some(index) = open
            {
                self.open_cell(index, frame);
            }
        }
    }
}
/// Kinds with no per-element editor: shown as their `disp` text, read-only.
fn text_only(value: &Array) -> bool {
    matches!(
        value,
        Array::Struct(_)
            | Array::Categorical(_)
            | Array::Datetime(_)
            | Array::Duration(_)
            | Array::Sparse(_)
            | Array::Handle(_)
    ) || matches!(value, Array::Table(t) if t.is_timetable())
}
/// Above this the preview would stall the UI thread; `head` shows part of it.
const TEXT_PREVIEW_BYTES: usize = 8 << 20;
impl State {
    fn text_view(&mut self, ui: &mut egui::Ui, value: &Array) {
        if !self
            .text
            .as_ref()
            .is_some_and(|(v, _)| v.same_snapshot(value))
        {
            let text = if value.bytes() > TEXT_PREVIEW_BYTES {
                "Too large to preview here. Show part of it in the Command Window, e.g. head(T)."
                    .into()
            } else {
                let fmt = matrust::display_format::DisplayFormat {
                    width: 10_000,
                    ..Default::default()
                };
                match matrust::display_format::disp(value, &fmt) {
                    Ok(Some(text)) => text,
                    Ok(None) => "Empty value; no elements.".into(),
                    Err(e) => format!("Cannot display this value: {e}"),
                }
            };
            self.text = Some((value.clone(), text));
        }
        let muted = tokens::palette(ui.ctx()).text_muted;
        ui.label(
            egui::RichText::new(
                "Read-only view. Change values with commands in the Command Window.",
            )
            .color(muted),
        );
        let text = self.text.as_ref().map_or("", |(_, t)| t.as_str());
        if matches!(value, Array::Table(_))
            && let Some(grid) = text_grid(text)
        {
            grid_view(ui, &grid);
            return;
        }
        egui::ScrollArea::both()
            .id_salt("typed_text")
            .auto_shrink([false, false])
            .show_themed(ui, |ui| {
                ui.add(egui::Label::new(egui::RichText::new(text).monospace()).extend());
            });
    }
}

/// A table's `disp` text as a caption header and cells. The rule line
/// (underscores) under the header gives each column's extent; text left of the first column is
/// the row-name column.
pub(super) struct TextGrid {
    pub(super) header: Vec<String>,
    pub(super) rows: Vec<Vec<String>>,
    /// Columns whose every non-empty cell is a number (right-aligned).
    pub(super) numeric: Vec<bool>,
    /// Widest cell per column, in characters.
    chars: Vec<usize>,
}

pub(super) fn text_grid(text: &str) -> Option<TextGrid> {
    let lines: Vec<Vec<char>> = text.lines().map(|l| l.chars().collect()).collect();
    let rule = |c: char| c == '_' || c == '-';
    let dash = lines
        .iter()
        .position(|l| l.iter().any(|&c| rule(c)) && l.iter().all(|&c| rule(c) || c == ' '))?;
    if dash == 0 {
        return None;
    }
    let mut runs = Vec::new();
    let line = &lines[dash];
    let mut i = 0;
    while i < line.len() {
        if rule(line[i]) {
            let start = i;
            while i < line.len() && rule(line[i]) {
                i += 1;
            }
            runs.push((start, i));
        } else {
            i += 1;
        }
    }
    if runs.is_empty() || runs.len() > 512 {
        return None;
    }
    let body: Vec<&Vec<char>> = lines[dash + 1..]
        .iter()
        .filter(|l| l.iter().any(|c| !c.is_whitespace()))
        .collect();
    let first = runs[0].0;
    let names = body
        .iter()
        .any(|l| l.iter().take(first).any(|c| !c.is_whitespace()));
    let mut bounds = vec![if names { first } else { 0 }];
    for pair in runs.windows(2) {
        bounds.push((pair[0].1 + pair[1].0).div_ceil(2));
    }
    bounds.push(usize::MAX);
    let cut = |line: &[char]| -> Vec<String> {
        let mut cells = Vec::with_capacity(bounds.len());
        if names {
            let end = first.min(line.len());
            cells.push(line[..end].iter().collect::<String>().trim().to_owned());
        }
        for pair in bounds.windows(2) {
            let (a, b) = (pair[0].min(line.len()), pair[1].min(line.len()));
            cells.push(line[a..b].iter().collect::<String>().trim().to_owned());
        }
        cells
    };
    let header = cut(&lines[dash - 1]);
    let rows: Vec<Vec<String>> = body.iter().map(|l| cut(l)).collect();
    let columns = header.len();
    let numeric = (0..columns)
        .map(|k| {
            rows.iter().any(|r| !r[k].is_empty())
                && rows
                    .iter()
                    .all(|r| r[k].is_empty() || r[k].parse::<f64>().is_ok())
        })
        .collect();
    let chars = (0..columns)
        .map(|k| rows.iter().map(|r| r[k].chars().count()).max().unwrap_or(0))
        .collect();
    Some(TextGrid {
        header,
        rows,
        numeric,
        chars,
    })
}

/// Plan section 9 table: 24 px rows, caption header with one divider under
/// it, values in mono (numbers right-aligned), no zebra.
fn grid_view(ui: &mut egui::Ui, grid: &TextGrid) {
    let p = tokens::palette(ui.ctx());
    let mono = egui::FontId::monospace(tokens::CODE);
    let caption = crate::panels::caption_semibold(ui);
    let digit = ui.fonts_mut(|f| f.glyph_width(&mono, '0'));
    let widths: Vec<f32> = (0..grid.header.len())
        .map(|k| {
            let head = ui.fonts_mut(|f| {
                f.layout_no_wrap(grid.header[k].to_uppercase(), caption.clone(), p.text)
                    .size()
                    .x
            });
            (grid.chars[k] as f32 * digit).max(head).max(32.) + 2. * tokens::M
        })
        .collect();
    let total: f32 = widths.iter().sum();
    ui.add_space(tokens::XS);
    ui.spacing_mut().item_spacing.y = 0.;
    egui::ScrollArea::both()
        .id_salt("typed_grid")
        .auto_shrink([false, false])
        .show_rows_themed(ui, ROW_HEIGHT, grid.rows.len() + 1, |ui, range| {
            let width = total.max(ui.available_width());
            for index in range {
                let (rect, _) =
                    ui.allocate_exact_size(egui::vec2(width, ROW_HEIGHT), egui::Sense::hover());
                let (cells, font, color) = match index.checked_sub(1) {
                    None => (&grid.header, caption.clone(), p.text_muted),
                    Some(row) => (&grid.rows[row], mono.clone(), p.text),
                };
                let mut x = rect.left();
                for (k, cell) in cells.iter().enumerate() {
                    let w = widths[k];
                    let text = if index == 0 {
                        cell.to_uppercase()
                    } else {
                        cell.clone()
                    };
                    let (pos, align) = if grid.numeric[k] {
                        (
                            egui::pos2(x + w - tokens::M, rect.center().y),
                            egui::Align2::RIGHT_CENTER,
                        )
                    } else {
                        (
                            egui::pos2(x + tokens::M, rect.center().y),
                            egui::Align2::LEFT_CENTER,
                        )
                    };
                    ui.painter().text(pos, align, text, font.clone(), color);
                    x += w;
                }
                if index == 0 {
                    ui.painter().hline(
                        rect.x_range(),
                        rect.bottom() - 0.5,
                        egui::Stroke::new(1., p.hairline),
                    );
                }
            }
        });
}
fn graphics_kind(kind: GraphicsKind) -> &'static str {
    match kind {
        GraphicsKind::Placeholder => "GraphicsPlaceholder",
        GraphicsKind::Root => "Root",
        GraphicsKind::Figure => "Figure",
        GraphicsKind::Axes => "Axes",
        GraphicsKind::Series => "Series",
        GraphicsKind::TextBox => "TextBox",
        GraphicsKind::Annotation => "Annotation",
        GraphicsKind::Legend => "Legend",
        GraphicsKind::Baseline => "Baseline",
        GraphicsKind::Text => "Text",
        GraphicsKind::Colorbar => "Colorbar",
        GraphicsKind::Image => "Image",
        GraphicsKind::Volume => "Volume",
        GraphicsKind::SliceViewer => "SliceViewer",
        GraphicsKind::OrthosliceViewer => "OrthosliceViewer",
        GraphicsKind::TiledLayout => "TiledChartLayout",
        GraphicsKind::NumericRuler => "NumericRuler",
        GraphicsKind::ConstantLine => "ConstantLine",
        GraphicsKind::Rectangle => "Rectangle",
        GraphicsKind::UiComponent => "UIComponent",
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
        // Plan section 9: no collapse control, 16 px padding, and never larger
        // than the window (1024x640 keeps the bars and status line visible).
        let screen = ctx.content_rect();
        let size = egui::vec2(
            900f32.min(screen.width() - 96.).max(320.),
            440f32.min(screen.height() - 160.).max(200.),
        );
        let title = format!("Variable Editor — {name}");
        egui::Window::new(title.as_str())
            .id(egui::Id::new("variable_editor"))
            // Keep the initial inspector clear of the desktop menu row.
            .default_pos(screen.center() - size / 2.)
            .collapsible(false)
            .title_bar(false)
            .frame(dialog_windows::dialog_frame(ctx))
            .default_size(size)
            .show(ctx, |ui| {
                dialog_windows::window_chrome(ui, &title, Some(&mut open), |ui| {
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
                })
            });
        if !open {
            self.editing_variable = None;
            self.numeric_editor.invalidate(false);
            self.typed_inspector = Default::default();
        }
    }
}
