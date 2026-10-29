use super::*;
use matrust::session::ToolstripTab;

#[derive(Clone, Copy, PartialEq, Eq)]
pub(super) enum Action {
    NewScript,
    OpenScript,
    SaveScript,
    SaveScriptAs,
    ReloadScript,
    RunScript,
    RunSelection,
    LoadWorkspace,
    SaveWorkspace,
    SaveWorkspaceV73,
    ImportCsv,
    InspectVariable,
    NewFigure,
    PlotData,
    OpenImage,
    OpenFigure,
    SaveFigure,
    ReloadFigure,
    ExportFigure,
    CopyFigure,
    Properties,
    Undo,
    Redo,
    Grid,
    Vertical,
    FilterDesigner,
    Simulation,
    FilteringExample,
    PlottingExample,
    Split,
    EditorOnly,
    FigureOnly,
    ResetLayout,
    CommandOnly,
    /// Opens the layout presets menu; not a command of its own.
    LayoutMenu,
    /// Opens the example scripts menu; not a command of its own.
    ExamplesMenu,
}
impl Action {
    pub(super) const ALL: [Self; 34] = [
        Self::NewScript,
        Self::OpenScript,
        Self::SaveScript,
        Self::SaveScriptAs,
        Self::ReloadScript,
        Self::RunScript,
        Self::RunSelection,
        Self::LoadWorkspace,
        Self::SaveWorkspace,
        Self::SaveWorkspaceV73,
        Self::ImportCsv,
        Self::InspectVariable,
        Self::NewFigure,
        Self::PlotData,
        Self::OpenImage,
        Self::OpenFigure,
        Self::SaveFigure,
        Self::ReloadFigure,
        Self::ExportFigure,
        Self::CopyFigure,
        Self::Properties,
        Self::Undo,
        Self::Redo,
        Self::Grid,
        Self::Vertical,
        Self::FilterDesigner,
        Self::Simulation,
        Self::FilteringExample,
        Self::PlottingExample,
        Self::Split,
        Self::EditorOnly,
        Self::FigureOnly,
        Self::ResetLayout,
        Self::CommandOnly,
    ];
    pub(super) fn label(self) -> &'static str {
        match self {
            Self::NewScript => "New script",
            Self::OpenScript => "Open script",
            Self::SaveScript => "Save script",
            Self::SaveScriptAs => "Save script as",
            Self::ReloadScript => "Reload script",
            Self::RunScript => "Run script",
            Self::RunSelection => "Run selection",
            Self::LoadWorkspace => "Load workspace",
            Self::SaveWorkspace => "Save workspace",
            Self::SaveWorkspaceV73 => "MATLAB v7.3 (HDF5)",
            Self::ImportCsv => "Import CSV",
            Self::InspectVariable => "Inspect variable",
            Self::NewFigure => "New figure",
            Self::PlotData => "Plot workspace data",
            Self::OpenImage => "Open plot image",
            Self::OpenFigure => "Open figure",
            Self::SaveFigure => "Save figure",
            Self::ReloadFigure => "Reload figure",
            Self::ExportFigure => "Export figure",
            Self::CopyFigure => "Copy figure",
            Self::Properties => "Figure properties",
            Self::Undo => "Undo figure edit",
            Self::Redo => "Redo figure edit",
            Self::Grid => "2 by 2 layout",
            Self::Vertical => "Vertical layout",
            Self::FilterDesigner => "Filter Designer",
            Self::Simulation => "Block Simulation",
            Self::FilteringExample => "Filtering example",
            Self::PlottingExample => "Plotting example",
            Self::Split => "Editor and figure",
            Self::EditorOnly => "Editor only",
            Self::FigureOnly => "Figure only",
            Self::ResetLayout => "Reset layout",
            Self::CommandOnly => "Command Window only",
            Self::LayoutMenu => "Layout",
            Self::ExamplesMenu => "Examples",
        }
    }
    /// Toolstrip tile caption: short, never wrapped or cut.
    fn strip_label(self) -> &'static str {
        match self {
            Self::OpenScript => "Open",
            Self::SaveScript => "Save",
            Self::RunScript => "Run",
            Self::RunSelection => "Run section",
            Self::PlotData => "Plot",
            Self::ImportCsv => "Import data",
            other => other.label(),
        }
    }
    fn icon(self) -> shell::Icon {
        use shell::Icon;
        match self {
            Self::NewScript
            | Self::FilteringExample
            | Self::PlottingExample
            | Self::ExamplesMenu => Icon::Document,
            Self::OpenScript | Self::OpenImage | Self::OpenFigure | Self::LoadWorkspace => {
                Icon::Folder
            }
            Self::SaveScript
            | Self::SaveScriptAs
            | Self::SaveWorkspace
            | Self::SaveWorkspaceV73
            | Self::SaveFigure
            | Self::ExportFigure => Icon::Save,
            Self::RunScript | Self::RunSelection => Icon::Run,
            Self::ImportCsv | Self::InspectVariable => Icon::Table,
            Self::NewFigure | Self::PlotData | Self::CopyFigure => Icon::Plot,
            Self::Undo | Self::Redo | Self::ReloadFigure | Self::ReloadScript => Icon::Undo,
            Self::Properties | Self::FilterDesigner | Self::Simulation => Icon::Tools,
            Self::Grid
            | Self::Vertical
            | Self::Split
            | Self::EditorOnly
            | Self::FigureOnly
            | Self::ResetLayout
            | Self::CommandOnly
            | Self::LayoutMenu => Icon::Layout,
        }
    }
    pub(super) fn enabled(self, w: &Workspace) -> bool {
        self.enabled_with_pending(w, w.file_action_pending())
    }
    pub(super) fn enabled_for_search(self, w: &Workspace) -> bool {
        self.enabled_with_pending(w, w.file_action_pending_without_global_search())
    }
    fn enabled_with_pending(self, w: &Workspace, pending: bool) -> bool {
        if w.busy {
            return false;
        }
        match self {
            Self::SaveScript | Self::SaveScriptAs | Self::ReloadScript => {
                w.layout.center.is_none_or(|c| c.editor_revealed)
                    && (!matches!(self, Self::ReloadScript)
                        || w.can_reload_script_with_pending(pending))
            }
            Self::SaveWorkspace | Self::SaveWorkspaceV73 => {
                !pending && !w.allow_close && !w.recovering_session && !w.recovery_blocked
            }
            Self::PlotData => !w.variables.is_empty(),
            Self::ReloadFigure => w.can_reload_figure_with_pending(pending),

            Self::RunScript => !w.has_startup_placeholder(),
            Self::RunSelection => {
                !w.has_startup_placeholder()
                    && w.selection
                        .as_ref()
                        .is_some_and(|(i, r)| *i == w.active && !r.is_empty())
            }
            Self::InspectVariable => w
                .selected
                .as_ref()
                .is_some_and(|n| w.variables.contains_key(n)),
            Self::SaveFigure => {
                !pending
                    && !w.allow_close
                    && !w.recovering_session
                    && !w.recovery_blocked
                    && w.figure.is_some()
            }
            Self::ExportFigure | Self::CopyFigure | Self::Properties => w.figure.is_some(),
            Self::Undo => w
                .figure_history
                .get(&w.current_figure)
                .is_some_and(|(n, _)| *n > 0),
            Self::Redo => w
                .figure_history
                .get(&w.current_figure)
                .is_some_and(|(_, n)| *n > 0),
            _ => true,
        }
    }
    pub(super) fn invoke(self, w: &mut Workspace, ctx: &egui::Context) {
        match self {
            Self::PlotData => w.begin_plot_data(),
            Self::OpenImage => w.queue_root_open(ctx, open_files::PickerKind::Image),
            Self::NewScript => w.new_document(""),
            Self::OpenScript => w.queue_root_open(ctx, open_files::PickerKind::Supported),
            Self::SaveScript => {
                w.save_document(w.active, false);
            }
            Self::SaveScriptAs => {
                w.save_document(w.active, true);
            }
            Self::ReloadScript => w.request_script_reload(),
            Self::RunScript => w.request_script_run(ctx, false),
            Self::RunSelection => w.request_script_run(ctx, true),
            Self::LoadWorkspace => {
                w.queue_root_open(ctx, open_files::PickerKind::WorkspaceCurrentFolder)
            }
            Self::SaveWorkspace | Self::SaveWorkspaceV73 => {
                w.queue_workspace_mat_save(ctx, self);
            }
            Self::ImportCsv => w.request_csv_import_files(ctx),
            Self::InspectVariable => {
                if let Some(name) = w.selected.clone() {
                    w.inspect_variable(name);
                }
            }
            Self::NewFigure => {
                w.show_plot_pane();
                w.run("figure".into());
            }
            Self::OpenFigure => w.queue_root_open(ctx, open_files::PickerKind::Figure),
            Self::SaveFigure => {
                w.queue_root_figure_save(ctx, false);
            }
            Self::ReloadFigure => w.request_figure_reload(),
            Self::ExportFigure => w.begin_figure_export(),
            Self::CopyFigure => w.copy_figure(),
            Self::Properties => {
                if let Some(f) = &w.figure {
                    w.figure_editor = Some(figure_editor::Editor::new(
                        w.current_figure,
                        f,
                        w.view_bounds.or_else(|| w.subplot_canvas.active_bounds(f)),
                    ));
                }
            }
            Self::Undo => w.submit(Job::UndoFigure(
                w.current_figure,
                w.view_bounds.map(|v| (v, w.view_auto)),
            )),
            Self::Redo => w.submit(Job::RedoFigure(
                w.current_figure,
                w.view_bounds.map(|v| (v, w.view_auto)),
            )),
            Self::Grid | Self::Vertical => {
                use matrust::plot::{Arrangement, TileGrid};
                let grid = match self {
                    Self::Grid => TileGrid::new(Arrangement::Fixed, 2, 2),
                    _ => TileGrid::new(Arrangement::Vertical, 1, 1),
                }
                .unwrap();
                w.show_plot_pane();
                w.submit(Job::NewLayout(grid));
            }
            Self::FilterDesigner => w.toolkits.open_tool(toolkits::Tool::Designer),
            Self::Simulation => w.show_model_document(),
            Self::FilteringExample => {
                w.new_document(include_str!("../../examples/streaming_filter.m"))
            }
            Self::PlottingExample => w.new_document(include_str!("../../examples/tiled_layout.m")),
            Self::CommandOnly => w.show_command_center(),
            Self::Split => w.select_document_layout(layout::PaneMode::Split),
            Self::EditorOnly => {
                w.select_document_layout(layout::PaneMode::Editor);
                w.document_save.activate(document_save::Pane::Script);
            }
            Self::FigureOnly => {
                w.select_document_layout(layout::PaneMode::Figure);
                w.document_save.activate(document_save::Pane::Figure);
            }
            Self::ResetLayout => w.reset_layout(ctx),
            // Their buttons open a menu; there is nothing to run.
            Self::LayoutMenu | Self::ExamplesMenu => {}
        }
    }
}
fn groups(tab: ToolstripTab) -> &'static [(&'static str, &'static [Action])] {
    use Action::*;
    match tab {
        ToolstripTab::Home => &[
            ("SCRIPTS", &[NewScript, OpenScript, SaveScript]),
            ("EXECUTION", &[RunSelection]),
            ("WORKSPACE", &[PlotData, ImportCsv, SaveWorkspace]),
            ("DESKTOP", &[LayoutMenu]),
        ],
        ToolstripTab::Plots => &[
            ("WORKSPACE DATA", &[PlotData]),
            (
                "FIGURES",
                &[NewFigure, OpenFigure, SaveFigure, ExportFigure],
            ),
            ("EDIT", &[Properties, Undo, Redo, ReloadFigure]),
            ("LAYOUTS", &[Grid, Vertical]),
            ("IMAGES", &[OpenImage, CopyFigure]),
        ],
        ToolstripTab::Apps => &[
            ("SIGNAL PROCESSING", &[FilterDesigner, FilteringExample]),
            ("SIMULATION", &[Simulation]),
            ("EXAMPLES", &[PlottingExample, ExamplesMenu]),
        ],
    }
}

impl Workspace {
    fn workspace_save_formats(&mut self, ui: &mut egui::Ui, ctx: &egui::Context) {
        for (label, action) in [
            ("MATLAB v7 (Level-5)", Action::SaveWorkspace),
            ("MATLAB v7.3 (HDF5)", Action::SaveWorkspaceV73),
        ] {
            let response = ui.add_enabled(action.enabled(self), egui::Button::new(label));
            if response.clicked() {
                ui.close();
                action.invoke(self, ctx);
            }
        }
    }

    pub(super) fn show_plot_pane(&mut self) {
        if self.layout.center.is_some() {
            if self.toolkits.selected_tool().is_none()
                && (self.central_command() || self.layout.mode == layout::PaneMode::Editor)
            {
                // The figure takes its own right column; the work stays put.
                self.document_save.activate(document_save::Pane::Figure);
                return;
            }
            self.select_document_layout(layout::PaneMode::Figure);
            return;
        }
        self.toolkits.deselect_tool();
        self.document_save.activate(document_save::Pane::Figure);
        self.compact_figure = true;
        if self.layout.mode == layout::PaneMode::Editor {
            self.layout.mode = layout::PaneMode::Split;
        }
    }
    fn toolstrip_action_button(&mut self, ui: &mut egui::Ui, ctx: &egui::Context, action: Action) {
        let run = matches!(action, Action::RunScript | Action::RunSelection);
        let enabled = self.action_enabled(action);
        let response = ui
            .add_enabled_ui(enabled, |ui| {
                panels::strip_action(ui, action.strip_label(), action.icon(), run)
            })
            .inner;
        if matches!(action, Action::SaveWorkspace) {
            egui::Popup::menu(&response).show(|ui| self.workspace_save_formats(ui, ctx));
        } else if matches!(action, Action::LayoutMenu) {
            egui::Popup::menu(&response).show(|ui| self.layout_presets_menu(ui));
        } else if matches!(action, Action::ExamplesMenu) {
            egui::Popup::menu(&response).show(|ui| self.examples_menu(ui));
        } else if response.clicked() {
            self.close_search_for_chrome(ctx);
            action.invoke(self, ctx);
        }
    }
    /// The open search palette alone does not disable the chrome: a click on an
    /// action closes the palette (click outside) and performs the action.
    fn action_enabled(&self, action: Action) -> bool {
        if self.global_search.is_open() {
            action.enabled_for_search(self)
        } else {
            action.enabled(self)
        }
    }
    fn close_search_for_chrome(&mut self, ctx: &egui::Context) {
        if self.global_search.is_open() {
            self.global_search.close(ctx);
        }
    }
    /// Menu form of a group's actions (collapsed tabs and the "More" overflow).
    fn toolstrip_menu_actions(
        &mut self,
        ui: &mut egui::Ui,
        ctx: &egui::Context,
        index: usize,
        actions: &[Action],
    ) {
        egui::Grid::new(("toolstrip_actions", index, true))
            .num_columns(2)
            .show(ui, |ui| {
                for (i, action) in actions.iter().enumerate() {
                    let button =
                        egui::Button::new(RichText::new(action.label()).size(tokens::BODY))
                            .min_size(egui::vec2(112., 24.));
                    if matches!(action, Action::SaveWorkspace) {
                        ui.add_enabled_ui(self.action_enabled(*action), |ui| {
                            egui::containers::menu::SubMenuButton::from_button(button)
                                .ui(ui, |ui| self.workspace_save_formats(ui, ctx));
                        });
                    } else if matches!(action, Action::LayoutMenu) {
                        egui::containers::menu::SubMenuButton::from_button(button)
                            .ui(ui, |ui| self.layout_presets_menu(ui));
                    } else if matches!(action, Action::ExamplesMenu) {
                        egui::containers::menu::SubMenuButton::from_button(button)
                            .ui(ui, |ui| self.examples_menu(ui));
                    } else if ui
                        .add_enabled(self.action_enabled(*action), button)
                        .clicked()
                    {
                        self.close_search_for_chrome(ctx);
                        action.invoke(self, ctx);
                        ui.close();
                    }
                    if !i.is_multiple_of(2) {
                        ui.end_row();
                    }
                }
            });
    }
    fn toolstrip_actions(
        &mut self,
        ui: &mut egui::Ui,
        ctx: &egui::Context,
        tab: ToolstripTab,
        popup: bool,
    ) {
        if popup {
            for (index, (name, actions)) in groups(tab).iter().enumerate() {
                if index > 0 {
                    ui.separator();
                }
                ui.strong(*name);
                self.toolstrip_menu_actions(ui, ctx, index, actions);
            }
            return;
        }
        // Actions that do nothing now are hidden, not greyed; Save stays.
        let shown: Vec<(usize, Vec<Action>)> = groups(tab)
            .iter()
            .enumerate()
            .map(|(index, (_, actions))| {
                let visible = actions
                    .iter()
                    .copied()
                    .filter(|a| *a == Action::SaveScript || a.enabled(self))
                    .collect::<Vec<_>>();
                (index, visible)
            })
            .filter(|(_, actions)| !actions.is_empty())
            .collect();
        let spacing = ui.spacing().item_spacing.x;
        let divider = 2. * tokens::S + 1. + spacing;
        let widths: Vec<f32> = shown
            .iter()
            .map(|(_, actions)| {
                actions
                    .iter()
                    .map(|a| panels::strip_action_width(ui, a.strip_label()) + spacing)
                    .sum()
            })
            .collect();
        let narrow = ui.ctx().content_rect().width() < tokens::OVERFLOW_WIDTH;
        let available = ui.available_width() - panels::strip_action_width(ui, "More") - divider;
        // Below the overflow width the last group always moves into "More";
        // any group that does not fit follows it.
        let mut split = shown.len();
        let mut used = 0.;
        for (k, width) in widths.iter().enumerate() {
            let next = used + width + if k > 0 { divider } else { 0. };
            if next > available || (narrow && k > 0 && k + 1 == shown.len()) {
                split = k;
                break;
            }
            used = next;
        }
        let row = egui::vec2(ui.available_width(), panels::STRIP_TILE);
        ui.allocate_ui_with_layout(
            row,
            egui::Layout::left_to_right(egui::Align::Center),
            |ui| {
                ui.set_height(panels::STRIP_TILE);
                for (k, (_, actions)) in shown[..split].iter().enumerate() {
                    if k > 0 {
                        panels::strip_divider(ui);
                    }
                    for action in actions {
                        self.toolstrip_action_button(ui, ctx, *action);
                    }
                }
                if split < shown.len() {
                    if split > 0 {
                        panels::strip_divider(ui);
                    }
                    let more = panels::strip_action(ui, "More", shell::Icon::Tools, false);
                    egui::Popup::menu(&more).show(|ui| {
                        for (k, (index, actions)) in shown[split..].iter().enumerate() {
                            if k > 0 {
                                ui.separator();
                            }
                            ui.strong(groups(tab)[*index].0);
                            self.toolstrip_menu_actions(ui, ctx, *index, actions);
                        }
                    });
                }
            },
        );
    }

    /// The preset whose panes are the ones shown now; `None` when a tool tab
    /// owns the main area or the panes match no preset.
    pub(super) fn current_layout_preset(&self) -> Option<Option<layout::PaneMode>> {
        if self.toolkits.selected_tool().is_some() {
            return None;
        }
        if !self.central_command() {
            // The panes the main area really shows, plus the figure column.
            let figureless = self.figure.is_none() && self.image_view.is_none();
            let (editor, figure) = match self.layout.mode {
                layout::PaneMode::Editor => (true, self.figure_column()),
                layout::PaneMode::Figure => (false, true),
                layout::PaneMode::Split if figureless => (true, false),
                layout::PaneMode::Split if self.document_save.is_compact() => {
                    (!self.compact_figure, self.compact_figure)
                }
                layout::PaneMode::Split => (true, true),
            };
            return match (editor, figure) {
                (true, true) => Some(Some(layout::PaneMode::Split)),
                (true, false) => Some(Some(layout::PaneMode::Editor)),
                (false, true) => Some(Some(layout::PaneMode::Figure)),
                (false, false) => None,
            };
        }
        // Around the Command Window an open editor and figure column add panes.
        match (self.editor_above_command(), self.figure_column()) {
            (false, false) => Some(None),
            (true, true) => Some(Some(layout::PaneMode::Split)),
            (true, false) => Some(Some(layout::PaneMode::Editor)),
            (false, true) => Some(Some(layout::PaneMode::Figure)),
        }
    }

    /// Layout presets: the Layout toolstrip action and Menu > Layout share it.
    pub(super) fn layout_presets_menu(&mut self, ui: &mut egui::Ui) {
        let current = self.current_layout_preset();
        if ui
            .selectable_label(current == Some(None), "Command Window only")
            .clicked()
        {
            self.show_command_center();
            ui.close();
        }
        for (mode, name) in [
            (layout::PaneMode::Split, "Editor and figure"),
            (layout::PaneMode::Editor, "Editor only"),
            (layout::PaneMode::Figure, "Figure only"),
        ] {
            if ui
                .selectable_label(current == Some(Some(mode)), name)
                .clicked()
            {
                self.select_document_layout(mode);
                ui.close();
            }
        }
        ui.separator();
        if ui.button("Reset layout").clicked() {
            self.reset_layout(ui.ctx());
            ui.close();
        }
    }

    /// Example scripts (the former Examples rail pane).
    pub(super) fn examples_menu(&mut self, ui: &mut egui::Ui) {
        for (label, source) in [
            ("Damped oscillation", DEMO),
            (
                "Linear system",
                include_str!("../../examples/linear_system.m"),
            ),
            (
                "Digital filtering",
                include_str!("../../examples/filter_design.m"),
            ),
            (
                "Streaming filter",
                include_str!("../../examples/streaming_filter.m"),
            ),
            (
                "Logical selection",
                include_str!("../../examples/logical_selection.m"),
            ),
            (
                "Loops and filtering",
                include_str!("../../examples/control_flow_filtering.m"),
            ),
            (
                "User functions",
                include_str!("../../examples/user_functions.m"),
            ),
            (
                "Multiple figures",
                include_str!("../../examples/multiple_plots.m"),
            ),
            ("Subplots", include_str!("../../examples/subplots.m")),
            (
                "Positioned axes / FIG",
                include_str!("../../examples/fig_layouts.m"),
            ),
            (
                "Tiled layouts",
                include_str!("../../examples/tiled_layout.m"),
            ),
            (
                "Discrete signals",
                include_str!("../../examples/discrete_plots.m"),
            ),
            (
                "Logarithmic plots",
                include_str!("../../examples/log_plots.m"),
            ),
        ] {
            if ui.button(label).clicked() {
                self.new_document(source);
                ui.close();
            }
        }
    }

    /// The File, Workspace, Tools and Layout menus behind one ghost "Menu".
    fn app_menu(&mut self, ui: &mut egui::Ui, ctx: &egui::Context, expanded: bool) {
        ui.menu_button("Menu", |ui| {
            ui.menu_button("File", |ui| {
                if ui.button("New script").clicked() {
                    self.new_document("");
                }
                if ui.button("Open…").clicked() {
                    self.queue_root_open(ctx, open_files::PickerKind::Supported);
                    ui.close();
                }
                if ui
                    .add_enabled(!self.busy, egui::Button::new("Load workspace…"))
                    .clicked()
                {
                    self.queue_root_open(ctx, open_files::PickerKind::WorkspaceDefaultDirectory);
                    ui.close();
                }
                if ui
                    .add_enabled(self.can_new_model(), egui::Button::new("New model"))
                    .clicked()
                {
                    self.request_new_model();
                    ui.close();
                }
                self.generic_save_menu(ui);
                if ui
                    .add_enabled(
                        self.can_reload_script(),
                        egui::Button::new("Reload script from disk..."),
                    )
                    .clicked()
                {
                    self.request_script_reload();
                    ui.close();
                }
                if ui
                    .add_enabled(
                        !self.busy && self.documents.iter().any(|d| d.dirty),
                        egui::Button::new("Save all scripts").shortcut_text("Ctrl+Alt+S"),
                    )
                    .clicked()
                {
                    self.begin_save_all_scripts();
                    ui.close();
                }
                ui.menu_button("Figures", |ui| self.figure_file_menu(ui));
                self.generic_close_menu(ui);
                ui.menu_button("Recent", |ui| {
                    let mut open = None;
                    if self.recent.is_empty() {
                        ui.label("No recent files");
                    }
                    for path in &self.recent {
                        if ui
                            .button(crate::path_label::display(path).to_string())
                            .clicked()
                        {
                            open = Some(path.clone());
                            ui.close();
                        }
                    }
                    if let Some(path) = open {
                        self.open_path(path);
                    }
                });
                if ui
                    .add_enabled(
                        !self.busy
                            && !self.has_startup_placeholder()
                            && self
                                .selection
                                .as_ref()
                                .is_some_and(|(i, r)| *i == self.active && !r.is_empty()),
                        egui::Button::new("Run selection · F9"),
                    )
                    .clicked()
                {
                    self.request_script_run(ctx, true);
                }
            });
            ui.menu_button("Workspace", |ui| {
                let available = !self.busy && !self.file_action_pending();
                let selected = self.workspace_selection_count();
                if ui
                    .add_enabled(
                        available && selected == 1,
                        egui::Button::new("Rename variable..."),
                    )
                    .clicked()
                {
                    self.begin_workspace_action(true, false);
                    ui.close();
                }
                if ui
                    .add_enabled(
                        available && selected > 0,
                        egui::Button::new("Delete selected variables..."),
                    )
                    .clicked()
                {
                    self.begin_workspace_action(false, false);
                    ui.close();
                }
                if ui
                    .add_enabled(
                        available && !self.variables.is_empty(),
                        egui::Button::new("Clear workspace..."),
                    )
                    .clicked()
                {
                    self.begin_workspace_action(false, true);
                    ui.close();
                }
                ui.separator();
                ui.add_enabled_ui(!self.busy, |ui| {
                    ui.menu_button("Save workspace", |ui| self.workspace_save_formats(ui, ctx));
                    if ui.button("Import CSV\u{2026}").clicked() {
                        self.request_csv_import_files(ui.ctx());
                        ui.close();
                    }
                });
            });
            ui.menu_button("Tools", |ui| {
                if ui.button("Filter Designer").clicked() {
                    self.toolkits.open_tool(toolkits::Tool::Designer);
                    ui.close();
                }
                if ui.button("Block Simulation").clicked() {
                    self.show_model_document();
                    ui.close();
                }
            });
            ui.menu_button("Layout", |ui| {
                // 24 px rows already separate the entries; keep this long
                // menu inside short and scaled windows.
                ui.spacing_mut().item_spacing.y = 0.;
                self.named_layout_menu(ui);
                ui.separator();
                self.pane_focus_menu(ui);
                ui.separator();
                self.desktop_scale_menu(ui);
                self.desktop_theme_menu(ui);
                self.workspace_position_menu(ui);
                ui.separator();
                self.layout_presets_menu(ui);
                ui.separator();
                if ui
                    .button(if expanded { "Hide tools" } else { "Show tools" })
                    .on_hover_text("Collapse or expand the toolstrip")
                    .clicked()
                {
                    self.layout.toolstrip_collapsed = !self.layout.toolstrip_collapsed;
                    ui.close();
                }
            });
        });
    }

    pub(super) fn desktop_toolbar(&mut self, root: &mut egui::Ui, pending: bool) {
        let ctx = &root.ctx().clone();
        let compact = tokens::compact(ctx);
        // A tool tab brings its own toolbars: below 760 px the toolstrip card
        // yields its height to the diagram, and the tabs become menus.
        let tool_room =
            self.toolkits.selected_tool().is_some() && ctx.content_rect().height() < 760.;
        let expanded = !self.layout.toolstrip_collapsed && !tokens::short(ctx) && !tool_room;
        // The top bar, menu row and folder row sit on the canvas; the
        // toolstrip is the first card.
        let frame = egui::Frame::new()
            .fill(tokens::palette(ctx).canvas)
            .inner_margin(egui::Margin::ZERO);
        egui::Panel::top("toolbar")
            .show_separator_line(false)
            .frame(frame)
            .show(root, |ui| {
                let palette = tokens::palette(ctx);
                if compact {
                    tokens::dense(ui);
                }
                ui.spacing_mut().item_spacing.y = 0.;
                // The search palette floats over the chrome without dimming or
                // disabling it: a click on a tab or action closes the palette.
                let pending = pending
                    && (!self.global_search.is_open()
                        || self.file_action_pending_without_global_search());
                // Chrome on the canvas uses ghost buttons: no fill or outline
                // until hovered.
                let widgets = &mut ui.visuals_mut().widgets;
                widgets.inactive.weak_bg_fill = egui::Color32::TRANSPARENT;
                widgets.inactive.bg_stroke = egui::Stroke::NONE;
                let bar = egui::vec2(
                    ui.available_width(),
                    if compact {
                        tokens::TOP_BAR - tokens::S
                    } else {
                        tokens::TOP_BAR
                    },
                );
                ui.allocate_ui_with_layout(
                    bar,
                    egui::Layout::left_to_right(egui::Align::Center),
                    |ui| {
                        ui.set_min_size(bar);
                        ui.spacing_mut().item_spacing.x = tokens::XS;
                        ui.add_space(tokens::S);
                        panels::brand(ui);
                        ui.add_space(tokens::S);
                        ui.add_enabled_ui(!pending, |ui| {
                            for (tab, label) in [
                                (ToolstripTab::Home, "Home"),
                                (ToolstripTab::Plots, "Plots"),
                                (ToolstripTab::Apps, "Apps"),
                            ] {
                                let selected = self.layout.toolstrip == tab;
                                if expanded {
                                    if panels::ghost_tab(ui, selected, label).clicked() {
                                        self.close_search_for_chrome(ctx);
                                        self.layout.toolstrip = tab;
                                    }
                                } else {
                                    // A collapsed toolstrip opens each tab as a menu.
                                    let text = RichText::new(label)
                                        .font(panels::semibold_font(ui, tokens::BODY))
                                        .color(if selected {
                                            palette.text
                                        } else {
                                            palette.text_muted
                                        });
                                    let menu = ui.menu_button(text, |ui| {
                                        ui.set_min_width(280.);
                                        self.toolstrip_actions(ui, ctx, tab, true);
                                    });
                                    if menu.response.clicked() {
                                        self.layout.toolstrip = tab;
                                    }
                                }
                            }
                        });
                        ui.add_space(tokens::S);
                        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                            ui.spacing_mut().item_spacing.x = tokens::S;
                            // Menu and Help keep the same type size as the tabs.
                            ui.style_mut().override_text_style = None;
                            let enabled = !self.busy && !self.has_startup_placeholder();
                            // The one filled primary action in the chrome.
                            if panels::primary_button(ui, "Run", "Ctrl Enter", enabled).clicked() {
                                self.request_script_run(ctx, false);
                            }
                            self.help_menu(ui);
                            self.app_menu(ui, ctx, expanded);
                            self.global_search_launcher(ui);
                            if (self.busy
                                || self.folder_browser.navigating()
                                || !self.open_queue.is_empty())
                                && ui.button("Stop").clicked()
                            {
                                self.stop_work();
                            }
                            if !self.open_queue.is_empty() {
                                ui.label(format!(
                                    "{} files waiting · Ctrl+Shift+X to clear",
                                    self.open_queue.len()
                                ));
                            }
                            if ctx.content_rect().width() >= tokens::SEARCH_ICON_WIDTH {
                                ui.with_layout(
                                    egui::Layout::left_to_right(egui::Align::Center),
                                    |ui| {
                                        let crumb = panels::crumb(ui, &self.folder);
                                        if crumb.clicked() && self.folder_browse_context(ctx) {
                                            self.queue_folder_browse(ctx);
                                        }
                                    },
                                );
                            }
                        });
                    },
                );
                if expanded {
                    ui.add_space(tokens::XS);
                    tokens::card(ctx)
                        .inner_margin(egui::Margin::symmetric(tokens::S as i8, 3))
                        .show(ui, |ui| {
                            ui.set_min_width(ui.available_width());
                            ui.add_enabled_ui(!pending, |ui| {
                                self.toolstrip_actions(ui, ctx, self.layout.toolstrip, false)
                            });
                        });
                }
                ui.add_space(tokens::XS);
            });
    }
}

// The production picker and save submission retain their original behavior.
fn workspace_save_path(folder: &std::path::Path) -> Option<PathBuf> {
    #[cfg(test)]
    if let Some(choice) = WORKSPACE_SAVE_CHOICE.with(|slot| {
        let mut slot = slot.borrow_mut();
        slot.as_mut().map(|(path, calls)| {
            *calls += 1;
            path.clone()
        })
    }) {
        return choice;
    }
    rfd::FileDialog::new()
        .set_directory(folder)
        .add_filter("MAT workspace", &["mat"])
        .set_file_name("workspace.mat")
        .save_file()
}

#[cfg(test)]
thread_local! {
    static WORKSPACE_SAVE_CHOICE: std::cell::RefCell<Option<(Option<PathBuf>, usize)>> = const { std::cell::RefCell::new(None) };
}
#[cfg(test)]
pub(super) struct WorkspaceSaveChoice;
#[cfg(test)]
impl WorkspaceSaveChoice {
    pub(super) fn calls(&self) -> usize {
        WORKSPACE_SAVE_CHOICE.with(|slot| slot.borrow().as_ref().unwrap().1)
    }
}
#[cfg(test)]
impl Drop for WorkspaceSaveChoice {
    fn drop(&mut self) {
        WORKSPACE_SAVE_CHOICE.with(|slot| *slot.borrow_mut() = None);
    }
}
#[cfg(test)]
pub(super) fn workspace_save_choice_for_test(path: Option<PathBuf>) -> WorkspaceSaveChoice {
    WORKSPACE_SAVE_CHOICE.with(|slot| {
        assert!(slot.borrow().is_none(), "Nested workspace picker override");
        *slot.borrow_mut() = Some((path, 0));
    });
    WorkspaceSaveChoice
}

#[cfg(test)]
mod complex_save_policy_tests {
    use super::*;
    #[test]
    fn whole_workspace_v73_accepts_complex_and_picker_cancel_preserves_workspace() {
        let ctx = egui::Context::default();
        let mut app = Workspace::with_context(ctx.clone(), true);
        app.variables.insert(
            "Z".into(),
            matrust::ComplexMatrix::from_column_vec(
                0,
                3,
                vec![],
                &std::sync::atomic::AtomicBool::new(false),
            )
            .unwrap()
            .into(),
        );
        assert!(Action::SaveWorkspaceV73.enabled(&app));
        assert!(Action::SaveWorkspace.enabled(&app));
        let before = app.variables["Z"].clone();
        let status = app.status.clone();
        let choice = workspace_save_choice_for_test(None);
        crate::headless::run(&ctx, egui::RawInput::default(), |root| {
            Action::SaveWorkspaceV73.invoke(&mut app, root.ctx());
            app.finish_workspace_mat_save(root.ctx());
        });
        assert_eq!(choice.calls(), 1);
        assert_eq!(app.status, status);
        assert!(!app.busy && before.same_snapshot(&app.variables["Z"]));
        app.busy = true;
        assert!(!Action::SaveWorkspace.enabled(&app) && !Action::SaveWorkspaceV73.enabled(&app));
    }
}

// One whole-workspace Save gesture; no workspace values or matrix samples copied.
#[derive(Default)]
pub(super) struct WorkspaceMatSave {
    action: Option<WorkspaceMatSaveAction>,
    finished: Option<(egui::ViewportId, u64)>,
    generation: u64,
    #[cfg(test)]
    pub(super) picker: Option<fn(&mut Workspace) -> Option<PathBuf>>,
    #[cfg(test)]
    pub(super) picker_calls: usize,
}
struct WorkspaceMatSaveAction {
    stamp: (egui::ViewportId, u64),
    generation: u64,
    format: Action,
    document: u64,
    interaction: u64,
    focus: Option<egui::Id>,
    folder: PathBuf,
}
impl Workspace {
    fn workspace_mat_save_context(&self, ctx: &egui::Context) -> bool {
        ctx.viewport_id() == egui::ViewportId::ROOT
            && ctx.input(|input| input.focused && !input.viewport().close_requested())
            && ctx.memory(|memory| memory.top_modal_layer().is_none())
            && !self.busy
            && !self.file_action_pending()
            && self.open_queue.is_empty()
            && !self.folder_browser.navigating()
            && !self.recovering_session
            && !self.recovery_blocked
            && !self.allow_close
    }
    fn valid_workspace_mat_save(
        &self,
        ctx: &egui::Context,
        action: &WorkspaceMatSaveAction,
    ) -> bool {
        self.workspace_mat_save_context(ctx)
            && !egui::Popup::is_any_open(ctx)
            && action.stamp == (ctx.viewport_id(), ctx.cumulative_frame_nr())
            && action.generation == self.workspace_mat_save.generation
            && action.document == self.documents[self.active].id
            && action.interaction == self.document_save.interaction_generation()
            && action.focus == ctx.memory(|memory| memory.focused())
            && action.folder == self.folder
    }
    fn queue_workspace_mat_save(&mut self, ctx: &egui::Context, format: Action) {
        let stamp = (ctx.viewport_id(), ctx.cumulative_frame_nr());
        // The clicked menu may still be unwinding here. Final admission requires
        // it closed; Global search already closes its own popup before invoking.
        if !matches!(format, Action::SaveWorkspace | Action::SaveWorkspaceV73)
            || !self.workspace_mat_save_context(ctx)
            || self.workspace_mat_save.action.is_some()
            || self.workspace_mat_save.finished == Some(stamp)
        {
            return;
        }
        self.workspace_mat_save.action = Some(WorkspaceMatSaveAction {
            stamp,
            generation: self.workspace_mat_save.generation,
            format,
            document: self.documents[self.active].id,
            interaction: self.document_save.interaction_generation(),
            focus: ctx.memory(|memory| memory.focused()),
            folder: self.folder.clone(),
        });
    }
    pub(super) fn cancel_workspace_mat_save(&mut self) {
        self.workspace_mat_save.generation = self.workspace_mat_save.generation.wrapping_add(1);
        if let Some(action) = self.workspace_mat_save.action.take() {
            self.workspace_mat_save.finished = Some(action.stamp);
        }
    }
    pub(super) fn observe_workspace_mat_save(&mut self, ctx: &egui::Context) {
        // Observe foreign ownership before widget painting can surrender its focus.
        if self
            .workspace_mat_save
            .action
            .as_ref()
            .is_some_and(|action| !self.valid_workspace_mat_save(ctx, action))
        {
            self.cancel_workspace_mat_save();
        }
    }
    fn pick_workspace_mat_save(&mut self) -> Option<PathBuf> {
        #[cfg(test)]
        if let Some(picker) = self.workspace_mat_save.picker {
            self.workspace_mat_save.picker_calls += 1;
            return picker(self);
        }
        workspace_save_path(&self.folder)
    }
    pub(super) fn finish_workspace_mat_save(&mut self, ctx: &egui::Context) {
        if ctx.will_discard() {
            return;
        }
        let Some(action) = self.workspace_mat_save.action.take() else {
            return;
        };
        // Consume before the picker, including Cancel and any later discarded pass.
        self.workspace_mat_save.finished = Some(action.stamp);
        if !self.valid_workspace_mat_save(ctx, &action) {
            return;
        }
        if let Some(path) = self.pick_workspace_mat_save()
            && self.valid_workspace_mat_save(ctx, &action)
        {
            // The sole Engine worker is idle; submit marks busy synchronously.
            // No intervening workspace-mutating job can pass these final guards.
            self.submit(if action.format == Action::SaveWorkspaceV73 {
                Job::SaveWorkspaceV73(path)
            } else {
                Job::SaveWorkspace(path)
            });
        }
    }
}
