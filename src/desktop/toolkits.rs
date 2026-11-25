use crate::theme::ThemedScrollArea;
#[path = "model_routing_ui.rs"]
mod routing_ui;
use super::tokens;
use eframe::egui::{self, RichText};
use matrust::simulation::{Block, Model, Node};
use model_canvas::parameters::{Edit, Properties, Settings};
use std::path::PathBuf;
#[path = "coefficients.rs"]
mod coefficients;
#[path = "designer_preview.rs"]
mod designer_preview;
#[path = "model_canvas.rs"]
mod model_canvas;
#[path = "model_files.rs"]
pub(super) mod model_files;
#[path = "model_list.rs"]
mod model_list;

pub enum Action {
    Script(String),
    /// App Export: publish only after the complete script succeeds.
    ExportScript(String),
    ExportVolumeSettings(Box<super::apps::volume_viewer::LabelSettingsExport>),
    /// A Filter Designer Apply script, published only if it completes.
    Apply(String),
    /// Add a designed filter block (block, summary) to the model.
    FilterBlock(Block, String),
    ChooseDesignSave(super::filter_spec::Spec),
    Simulate(Model),
    ModelResolved,
    ModelCancelled,
    StopModel,
    NewModel,
    ModelFile(model_files::ToolbarKind),
    /// Generate script: the app's saved script (header + script), written
    /// through the Filter Designer's save path once a file is chosen.
    SaveAppScript(super::apps::AppKind, String),
    CloseAppDesigner,
    SaveVolumeScript(
        String,
        Option<matrust::Array>,
        Option<(String, matrust::Array)>,
    ),
}
/// Tone of an app's status-bar text.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AppStatus {
    Plain,
    Success,
    Error,
}
impl AppStatus {
    pub fn color(self, ctx: &egui::Context) -> egui::Color32 {
        let p = tokens::palette(ctx);
        match self {
            Self::Plain => p.text_muted,
            Self::Success => p.success,
            Self::Error => p.error,
        }
    }
}
/// Tools shown as closable tabs in the main work area: the two built-in
/// tools and the framework apps (`apps.rs`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Tool {
    Designer,
    Model,
    App(super::apps::AppKind),
}
impl Tool {
    pub fn label(self) -> &'static str {
        match self {
            Self::Designer => "Filter Designer",
            Self::Model => "Block Simulation",
            Self::App(kind) => kind.label(),
        }
    }
}
/// One toolkit pass: the same-frame edit guard and the action chosen so far.
pub struct Pass {
    node_edit_interactive: bool,
    action: Option<Action>,
}
pub struct Toolkits {
    pub designer_open: bool,
    pub model_open: bool,
    // Opening order; a reopened tool moves to the end like an editor tab.
    tab_order: Vec<Tool>,
    /// The framework apps: open state, inputs and private runs.
    pub(super) apps: super::apps::Apps,
    selected_tool: Option<Tool>,
    tab_region: Option<(Tool, egui::Rect, egui::LayerId)>,
    spec: super::filter_spec::Spec,
    saving_design: bool,
    /// Workspace signal the Apply buttons filter.
    input: String,
    view: designer_preview::View,
    preview: designer_preview::Cache,
    coefficient_editor: Option<coefficients::Editor>,
    pub model: Model,
    model_identity: u64,
    selected: usize,
    canvas: model_canvas::Canvas,
    validation: model_canvas::validation::Cache,
    block_list: model_list::List,
    model_blocks_tab: bool,
    model_inspector: bool,
    add_kind: usize,
    last_node_edit_frame: Option<u64>,
    // An app command clicked in the Apps action row, handled with this frame's pass.
    row_action: Option<Action>,
    file: matrust::simulation::document::ModelFile,
    model_conflict: Option<model_files::Conflict>,
    pending_model: Option<PathBuf>,
    new_model_pending: bool,
    model_read: Option<super::model_read::Request>,
    reading_model: Option<super::model_read::Purpose>,
    model_resolved: bool,
    model_write: Option<super::model_write::Request>,
    writing_model: bool,
    #[cfg(test)]
    pub(super) model_path_choice: Option<Option<PathBuf>>,
    pub message: String,
}
/// Flat Inspector tabs share the workbench's 32 px / 2 px underline rhythm.
fn model_inspector_tab(ui: &mut egui::Ui, label: &str, selected: bool) -> egui::Response {
    let p = tokens::palette(ui.ctx());
    let text = egui::RichText::new(label).color(if selected { p.text } else { p.text_muted });
    let response = ui.add_sized(
        [
            if label == "Parameters" { 100. } else { 72. },
            tokens::PANEL_HEADER,
        ],
        egui::Button::new(text).frame(false),
    );
    response.widget_info(|| {
        egui::WidgetInfo::selected(egui::WidgetType::Button, ui.is_enabled(), selected, label)
    });
    if selected {
        let rect = response.rect;
        ui.painter().hline(
            rect.x_range(),
            rect.bottom() - 1.,
            egui::Stroke::new(2., p.accent),
        );
    }
    crate::panels::focus_ring(ui, &response);
    response
}

impl Default for Toolkits {
    fn default() -> Self {
        Self {
            designer_open: false,
            model_open: false,
            tab_order: vec![Tool::Designer, Tool::Model],
            apps: Default::default(),
            selected_tool: None,
            tab_region: None,
            spec: Default::default(),
            saving_design: false,
            input: "x".into(),
            view: designer_preview::View::Magnitude,
            preview: Default::default(),
            coefficient_editor: None,
            model: Model::default(),
            model_identity: 0,
            selected: 3,
            canvas: Default::default(),
            validation: Default::default(),
            block_list: Default::default(),
            model_blocks_tab: false,
            model_inspector: false,
            add_kind: 0,
            last_node_edit_frame: None,
            row_action: None,
            file: Default::default(),
            model_conflict: None,
            pending_model: None,
            new_model_pending: false,
            model_read: None,
            reading_model: None,
            model_resolved: false,
            model_write: None,
            writing_model: false,
            #[cfg(test)]
            model_path_choice: None,
            message: String::new(),
        }
    }
}
impl Toolkits {
    pub fn is_open(&self, tool: Tool) -> bool {
        match tool {
            Tool::Designer => self.designer_open,
            Tool::Model => self.model_open,
            Tool::App(kind) => self.apps.is_open(kind),
        }
    }
    /// Adds the tool's tab if missing and selects it.
    pub fn open_tool(&mut self, tool: Tool) {
        if let Tool::App(kind) = tool
            && !kind.is_built()
        {
            return;
        }
        if !self.is_open(tool) {
            self.tab_order.retain(|t| *t != tool);
            self.tab_order.push(tool);
        }
        match tool {
            Tool::Designer => self.designer_open = true,
            Tool::Model => self.model_open = true,
            Tool::App(kind) => self.apps.open(kind),
        }
        self.selected_tool = Some(tool);
    }
    /// Open tabs for the session layout; `None` when no tool is open.
    pub fn tab_layout(&self) -> Option<matrust::session::ToolTabs> {
        let convert = |tool| match tool {
            Tool::Designer => matrust::session::ToolTab::Designer,
            Tool::Model => matrust::session::ToolTab::Model,
            Tool::App(kind) => kind.tab(),
        };
        let open: matrust::session::ToolTabList = self.tabs().map(convert).collect();
        (!open.is_empty()).then(|| matrust::session::ToolTabs {
            open,
            selected: self.selected_tool().map(convert),
        })
    }
    /// Reopens the saved tabs in their order; the shown tool is returned.
    /// A saved app that is not built here is skipped.
    pub fn restore_tab_layout(&mut self, tabs: Option<matrust::session::ToolTabs>) -> Option<Tool> {
        let convert = |tool| match tool {
            matrust::session::ToolTab::Designer => Some(Tool::Designer),
            matrust::session::ToolTab::Model => Some(Tool::Model),
            app => super::apps::AppKind::from_tab(app)
                .filter(|kind| kind.is_built())
                .map(Tool::App),
        };
        let tabs = tabs?;
        for tool in tabs.open.iter().filter_map(convert) {
            self.open_tool(tool);
        }
        self.deselect_tool();
        let selected = tabs.selected.and_then(convert);
        if let Some(tool) = selected {
            self.select_tool(tool);
        }
        selected
    }
    /// Open tool tabs in opening order.
    pub fn tabs(&self) -> impl Iterator<Item = Tool> + '_ {
        self.tab_order
            .iter()
            .copied()
            .filter(|tool| self.is_open(*tool))
    }
    /// Tab label; an unsaved model keeps the former title's `*` marker.
    pub fn tab_label(&self, tool: Tool) -> String {
        if (tool == Tool::Model && self.model_dirty())
            || (tool == Tool::App(super::apps::AppKind::AppDesigner)
                && self.apps.app_designer.is_dirty())
        {
            format!("{} *", tool.label())
        } else {
            tool.label().into()
        }
    }
    pub fn selected_tool(&self) -> Option<Tool> {
        self.selected_tool.filter(|tool| self.is_open(*tool))
    }
    pub fn select_tool(&mut self, tool: Tool) {
        if self.is_open(tool) {
            self.selected_tool = Some(tool);
        }
    }
    /// The documents (Command, Editor, Figure) own the main area again.
    pub fn deselect_tool(&mut self) {
        self.selected_tool = None;
    }
    /// Same as closing the former tool window: the model view hides and keeps
    /// unsaved work; no prompt.
    pub fn close_tool(&mut self, tool: Tool) {
        match tool {
            Tool::Designer => {
                self.designer_open = false;
                self.tool_closed(Tool::Designer);
            }
            Tool::Model => self.hide_model_view(),
            Tool::App(kind) => {
                self.apps.close(kind);
                self.tool_closed(tool);
            }
        }
    }
    // A closed selected tab hands the area to the other open tool, else the documents.
    fn tool_closed(&mut self, tool: Tool) {
        if self.selected_tool == Some(tool) {
            let next = self.tabs().next();
            self.selected_tool = next;
        }
    }
    pub fn open_designer_demo(&mut self, iir: bool) {
        use super::filter_spec::{Method, Response};
        self.open_tool(Tool::Designer);
        if iir {
            self.spec.set_response(Response::Lowpass);
            self.spec.set_method(Method::Butterworth);
            self.spec.minimum = false;
            self.spec.order = 6;
        }
    }
    pub(super) fn open_iir_capture(&mut self, case: super::screenshot_options::IirDesignerCase) {
        use super::filter_spec::{Method, Units};
        self.open_designer_demo(true);
        let (response, order) = case.settings();
        self.spec.set_units(Units::Hz);
        self.spec.fs = 1000.;
        self.spec.set_response(response);
        self.spec.set_method(Method::Butterworth);
        self.spec.minimum = false;
        self.spec.order = order;
        self.spec.fc = [100., 200.];
    }
    pub fn open_coefficients(&mut self, index: usize) {
        if let Some(editor) = self
            .model
            .nodes
            .get(index)
            .and_then(|node| coefficients::Editor::new(index, &node.block))
        {
            self.coefficient_editor = Some(editor);
            self.selected = index;
            self.open_tool(Tool::Model);
            self.canvas.reveal(index);
        }
    }
    pub fn take_diagram_change(&mut self) -> bool {
        self.canvas.take_changed()
    }
    /// Add a designed filter to the block model and open the model.
    pub(super) fn add_filter_block(&mut self, block: Block, summary: &str) -> bool {
        if self.saving_design {
            self.message = "A filter operation is already pending".into();
            return false;
        }
        if let Err(error) = self.model.validate_storage() {
            self.message = error;
            return false;
        }
        if self.model.nodes.len() >= 512 {
            self.message = "Model block limit reached".into();
            return false;
        }
        let count = self.model.nodes.len();
        self.add_block(block);
        if self.model.nodes.len() == count {
            return false;
        }
        self.open_tool(Tool::Model);
        self.message = format!("Added {summary}. Connect its input and a Scope in the model.");
        true
    }
    pub(super) fn selected_save_design(&self) -> &super::filter_spec::Spec {
        &self.spec
    }
    #[cfg(test)]
    pub(super) fn spec_mut(&mut self) -> &mut super::filter_spec::Spec {
        &mut self.spec
    }
    /// The background preview has finished (Add to block model needs it).
    #[cfg(test)]
    pub(super) fn preview_done(&self) -> bool {
        self.preview.result().is_some()
    }
    pub(super) fn design_save_available(&self) -> bool {
        self.designer_open
            && !self.saving_design
            && !self.model_io_pending()
            && self.spec.problems().is_empty()
    }
    pub(super) fn begin_design_save(&mut self, summary: String) -> matrust::Result<()> {
        if self.saving_design {
            return Err("A filter operation is already pending".into());
        }
        self.saving_design = true;
        self.message = format!("Saving {summary} script...");
        Ok(())
    }
    pub(super) fn finish_design_save(&mut self, result: matrust::Result<PathBuf>) {
        self.saving_design = false;
        self.message = match result {
            Ok(path) => format!("Saved {}", crate::path_label::display(&path)),
            Err(error) => error,
        };
    }
    /// The design script plus a response plot: what the capture and demo
    /// startups run in the console.
    pub(super) fn preview_script(&self) -> matrust::Result<String> {
        let script = self.spec.script()?;
        let [first, second] = self.spec.outputs();
        let call = if first == "sos" {
            // g scales the first section; one section is passed as b, a.
            "s = sos; s(1, 1:3) = g*s(1, 1:3);\n\
             if size(s, 1) == 1, freqz(s(1:3), s(4:6), 1024), else, freqz(s, 1024), end"
                .to_owned()
        } else {
            format!("freqz({first}, {second}, 1024)")
        };
        Ok(format!("{script}\n{call}; title('{}');", self.spec.title()))
    }
    pub(super) fn model_document_region(&self) -> Option<(egui::Rect, egui::LayerId)> {
        self.tab_region
            .filter(|(tool, ..)| *tool == Tool::Model)
            .map(|(_, rect, layer)| (rect, layer))
    }
    #[cfg(test)]
    pub(super) fn designer_region(&self) -> Option<egui::Rect> {
        self.tab_region
            .filter(|(tool, ..)| *tool == Tool::Designer)
            .map(|(_, rect, _)| rect)
    }
    pub(super) fn hide_model_view(&mut self) {
        self.model_open = false;
        self.tab_region = self.tab_region.filter(|(tool, ..)| *tool != Tool::Model);
        self.canvas.finish_edit(None);
        self.canvas.cancel_connection();
        self.tool_closed(Tool::Model);
    }

    /// Starts one pass. A repeated pass in the same frame does not edit nodes again.
    pub(super) fn begin_pass(&mut self, ctx: &egui::Context) -> Pass {
        let frame = ctx.cumulative_frame_nr();
        let node_edit_interactive = self.last_node_edit_frame != Some(frame);
        self.last_node_edit_frame = Some(frame);
        self.tab_region = None;
        Pass {
            node_edit_interactive,
            action: None,
        }
    }

    /// Paints `tool` into the main work area; its content scrolls when the area is small.
    pub(super) fn tab_ui(&mut self, ui: &mut egui::Ui, tool: Tool, busy: bool, pass: &mut Pass) {
        let region = ui
            .available_rect_before_wrap()
            .intersect(ui.ctx().content_rect());
        self.tab_region = Some((tool, region, ui.layer_id()));
        let interactive = pass.node_edit_interactive;
        match tool {
            Tool::Designer => self.designer_ui(ui, busy, interactive, &mut pass.action),
            // The app canvas and inspector have a 16 px work-area inset.
            Tool::Model => {
                egui::Frame::new()
                    .inner_margin(egui::Margin::same(tokens::L as i8))
                    .show(ui, |ui| {
                        self.model_ui(ui, busy, interactive, &mut pass.action)
                    });
            }
            Tool::App(kind) => self.apps.ui(ui, kind, busy, &mut pass.action),
        }
    }

    /// Apps action row: the open app's commands, then its Stop or last message.
    /// Called before the frame's pass; the click is handled by `finish_pass`.
    pub(super) fn app_commands(&mut self, ui: &mut egui::Ui, tool: Tool, busy: bool) {
        let interactive = self.last_node_edit_frame != Some(ui.ctx().cumulative_frame_nr());
        let mut action = None;
        self.app_commands_with(ui, tool, busy, interactive, &mut action);
        if action.is_some() {
            self.row_action = action;
        }
    }

    /// The open app's status for the status bar: design summary or model check.
    pub(super) fn app_status(&mut self, tool: Tool, busy: bool) -> Option<(String, AppStatus)> {
        match tool {
            Tool::Designer => Some(match self.spec.problems().into_iter().next() {
                None => (self.spec.title(), AppStatus::Plain),
                Some((_, message)) => (message, AppStatus::Error),
            }),
            Tool::Model => {
                // Empty documents are valid storage, but not runnable graphs. Keep
                // invalid layout/storage and run settings visible as errors.
                let empty_ready = self.model.nodes.is_empty()
                    && self.model.validate_storage().is_ok()
                    && self.model.sample_time > 0.
                    && (self.model.sample_time * self.model.steps as f64).is_finite()
                    && (1..=1_000_000).contains(&self.model.steps);
                Some(if empty_ready {
                    (
                        "Add a block to start building this model.".into(),
                        AppStatus::Plain,
                    )
                } else {
                    match self.validation.status(&self.model, !busy) {
                        Ok(_) => (
                            "Model valid \u{b7} ready to simulate".into(),
                            AppStatus::Success,
                        ),
                        Err(e) => (e.to_owned(), AppStatus::Error),
                    }
                })
            }
            Tool::App(kind) => self.apps.status(kind),
        }
    }

    fn app_commands_with(
        &mut self,
        ui: &mut egui::Ui,
        tool: Tool,
        busy: bool,
        interactive: bool,
        action: &mut Option<Action>,
    ) {
        ui.spacing_mut().interact_size.y = tokens::CONTROL;
        match tool {
            Tool::Designer => self.designer_commands(ui, busy, interactive, action),
            Tool::Model => self.model_commands(ui, busy, interactive, action),
            Tool::App(kind) => self.apps.commands(ui, kind),
        }
    }

    /// Block Simulation action-row commands; Run model is the one primary.
    fn model_commands(
        &mut self,
        ui: &mut egui::Ui,
        busy: bool,
        node_edit_interactive: bool,
        action: &mut Option<Action>,
    ) {
        let p = tokens::palette(ui.ctx());
        let control = egui::vec2(0., tokens::CONTROL);
        ui.add_enabled_ui(!self.model_io_pending(), |ui| {
            ui.horizontal(|ui| {
                ui.spacing_mut().item_spacing.x = tokens::S;
                // Disabled: text 3 without fill or border (plan section 9).
                let can_run = !busy && !self.model.nodes.is_empty() && ui.is_enabled();
                let (fill, ink) = if can_run {
                    (p.accent_fill, p.on_accent)
                } else {
                    (egui::Color32::TRANSPARENT, p.text_disabled)
                };
                let run = egui::Button::new((
                    crate::icons::text(ui, crate::icons::Icon::Play, 16.).color(ink),
                    egui::RichText::new("Run model").color(ink),
                ))
                .fill(fill)
                .stroke(egui::Stroke::NONE)
                .min_size(control);
                if ui.add_enabled(can_run, run).clicked() {
                    *action = Some(Action::Simulate(self.model.clone()));
                }
                if ui
                    .add_enabled(
                        !busy && !self.modal_open(),
                        egui::Button::new("New model").min_size(control),
                    )
                    .clicked()
                    && node_edit_interactive
                {
                    *action = Some(Action::NewModel);
                }
                if ui
                    .add(egui::Button::new("Open\u{2026}").min_size(control))
                    .clicked()
                {
                    *action = Some(Action::ModelFile(model_files::ToolbarKind::Open));
                }
                if ui
                    .add(egui::Button::new("Save model").min_size(control))
                    .clicked()
                {
                    *action = Some(Action::ModelFile(model_files::ToolbarKind::Save));
                }
                if ui
                    .add(egui::Button::new("Save as\u{2026}").min_size(control))
                    .clicked()
                {
                    *action = Some(Action::ModelFile(model_files::ToolbarKind::SaveAs));
                }
            });
        });
    }

    /// Standalone host: the selected (else first open) tool fills the viewport,
    /// under a 36 px row standing in for the desktop's Apps action row.
    #[cfg(test)]
    pub fn ui(&mut self, ctx: &egui::Context, busy: bool) -> Option<Action> {
        let mut pass = self.begin_pass(ctx);
        if let Some(tool) = self.selected_tool().or_else(|| self.tabs().next()) {
            let mut host = egui::Ui::new(
                ctx.clone(),
                egui::Id::new("toolkit_host"),
                egui::UiBuilder::new()
                    .layer_id(egui::LayerId::background())
                    .max_rect(ctx.content_rect()),
            );
            egui::CentralPanel::default().show(&mut host, |ui| {
                let region = ui
                    .available_rect_before_wrap()
                    .intersect(ui.ctx().content_rect());
                let layer = ui.layer_id();
                // A 24 px line standing in for the desktop status bar.
                if let Some((text, status)) = self.app_status(tool, busy) {
                    egui::Panel::bottom("toolkit_host_status")
                        .exact_size(24.)
                        .show(ui, |ui| {
                            ui.label(egui::RichText::new(text).color(status.color(ui.ctx())));
                        });
                }
                {
                    let interactive = pass.node_edit_interactive;
                    ui.allocate_ui_with_layout(
                        egui::vec2(ui.available_width(), 36.),
                        egui::Layout::left_to_right(egui::Align::Center),
                        |ui| self.app_commands_with(ui, tool, busy, interactive, &mut pass.action),
                    );
                }
                self.tab_ui(ui, tool, busy, &mut pass);
                self.tab_region = Some((tool, region, layer));
            });
        }
        self.finish_pass(ctx, pass)
    }

    fn model_ui(
        &mut self,
        ui: &mut egui::Ui,
        _busy: bool,
        node_edit_interactive: bool,
        action: &mut Option<Action>,
    ) {
        let ctx = &ui.ctx().clone();
        ui.set_clip_rect(ui.clip_rect().intersect(ui.max_rect()));
        ui.painter()
            .rect_filled(ui.max_rect(), 0., tokens::palette(ctx).surface_sunken);
        // Reserve a useful canvas before docking a 320 px inspector. The
        // desktop Navigator leaves less than this budget at 1024 px.
        let compact_inspector = ui.available_width() < 900.;
        if compact_inspector
            && self.model_inspector
            && ui.is_enabled()
            && ctx.memory(|memory| memory.top_modal_layer().is_none())
            && !egui::Popup::is_any_open(ctx)
            && !self.modal_open()
            && ctx.input_mut(|i| i.consume_key(egui::Modifiers::NONE, egui::Key::Escape))
        {
            self.model_inspector = false;
        }
        egui::ScrollArea::both().id_salt("block_simulation").auto_shrink([false, false]).show_themed(ui,|ui|{
            // Compact windows use dense rows, so the diagram keeps its height.
            if tokens::compact(ctx) { tokens::dense(ui); } else { ui.spacing_mut().interact_size.y = tokens::CONTROL; }
            if self.model_io_pending() {
                ui.horizontal_wrapped(|ui| {
                    ui.label(&self.message);
                    if ui.button("Stop model operation").clicked() { *action = Some(Action::StopModel); }
                });
                crate::tokens::helper(ui, "A write already in progress may finish; Stop cancels subsequent actions.");
                ui.disable();
            }
            let mut edit_active = None;
            let mut setting_changed = None;
            let settings_before = Settings::capture(&self.model);
            ui.horizontal_wrapped(|ui|{
                ui.label("Sample time (s)").on_hover_text("One fixed sample rate; scalar signals; Unit Delay and Forward Euler break feedback loops. Results: sim_t, sim_1, sim_2, ...");track_edit(&ui.add(egui::DragValue::new(&mut self.model.sample_time).speed(0.0001).max_decimals(6)), &mut edit_active, &mut setting_changed);
                ui.label("Samples");track_edit(&ui.add(egui::DragValue::new(&mut self.model.steps).range(1..=1_000_000)), &mut edit_active, &mut setting_changed);
                // Canvas toolbar: run settings, then Add block; app commands
                // (Run model, New, Open, Save) sit in the Apps action row.
                ui.separator();
                let names=["Constant","Sine","Step","Gain","Sum","Unit Delay","Discrete Filter","SOS Filter","Scope","Abs","Bias","Saturation","Product","Unary Minus","Switch","Discrete-Time Integrator"];
                egui::ComboBox::from_id_salt("add_block").selected_text(names[self.add_kind]).show_ui(ui,|ui|{for (i,name) in names.iter().enumerate(){ui.selectable_value(&mut self.add_kind,i,*name);}});
                if ui.add_enabled(self.model.nodes.len()<512,egui::Button::new("Add block")).clicked() && node_edit_interactive {let block=match self.add_kind{0=>Block::Constant{value:1.},1=>Block::Sine{amplitude:1.,frequency_hz:10.,phase:0.},2=>Block::Step{time:0.1,before:0.,after:1.},3=>Block::Gain{gain:1.},4=>Block::Sum,5=>Block::UnitDelay{initial:0.},6=>Block::Filter{b:vec![0.5,0.5],a:vec![1.]},7=>Block::SosFilter{sections:vec![[1.,0.,0.,1.,0.,0.]]},9=>Block::Abs,10=>Block::Bias{bias:1.},11=>Block::Saturation{lower:-0.5,upper:0.5},12=>Block::Product{operations:vec![matrust::simulation::ProductInput::Multiply;2]},13=>Block::UnaryMinus,14=>Block::Switch{criteria:matrust::simulation::SwitchCriteria::default(),threshold:0.},15=>Block::DiscreteIntegrator{initial:0.,gain:1.,method:matrust::simulation::IntegratorMethod::ForwardEuler},_=>Block::Scope};self.add_block(block);}
                if compact_inspector {
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        if tokens::toggle(ui, self.model_inspector, "Inspector").clicked() {
                            self.model_inspector = !self.model_inspector;
                        }
                    });
                }
            });
            if let Some(edit) = Edit::settings(&self.model, settings_before) { self.canvas.record_edit(edit, setting_changed); }
            if let Some(path)=&self.file.path{crate::tokens::helper(ui, crate::path_label::display(path).to_string());}
            let keyboard_allowed = !self.modal_open();
            let row = ui.text_style_height(&egui::TextStyle::Button).max(ui.spacing().interact_size.y) + ui.spacing().item_spacing.y;
            // Separate graph and inspector budgets: parameters never consume
            // diagram height. Tiny hosts retain the outer Window scroll fallback.
            // The model check sits in the status bar; a message adds one row.
            let footer = row * if self.message.is_empty() { 0. } else { 1. } + 8.;
            let body_height = (ui.clip_rect().bottom() - ui.cursor().top() - footer).max(240.);
            let body_width = ui.available_width().max(560.);
            let (body, _) = ui.allocate_exact_size(egui::vec2(body_width, body_height), egui::Sense::hover());
            // The Inspector divider sits 300 px from the window edge, as in
            // Filter Designer (plan: Inspector 300); tiny hosts keep 240.
            let split = (ctx.content_rect().right() - tokens::RIGHT_COLUMN + 12.).min(body.right() - 240.);
            let graph_rect = if compact_inspector { body } else {
                egui::Rect::from_min_max(body.min, egui::pos2(split - 24., body.bottom()))
            };
            let inspector_rect = egui::Rect::from_min_max(egui::pos2(split, body.top()), body.max);
            let mut graph = ui.new_child(egui::UiBuilder::new().id_salt("model_graph_pane").max_rect(graph_rect));
            graph.set_clip_rect(ui.clip_rect().intersect(graph_rect));
            let prior_topology_count = self.model.nodes.len();
            self.canvas.show_pane(&mut graph, &mut self.model, &mut self.selected, &mut self.message, keyboard_allowed);
            if self.model.nodes.len() != prior_topology_count { self.coefficient_editor = None; }
            if !compact_inspector {
                // Graph and Inspector are two panes: the pane_border between them.
                ui.painter().vline(split - 12., body.y_range(), egui::Stroke::new(1., tokens::palette(ctx).pane_border));
            }
            let prior_selected = self.selected;
            let prior_count = self.model.nodes.len();
            let prior_inputs = self.model.nodes.get(self.selected).map(|node| node.inputs.clone()).unwrap_or_default();
            let mut property_change = None;
            let mut delete_requested = false;
            let mut show_inspector = |inspector: &mut egui::Ui| {
            inspector.horizontal(|ui| {
                if model_inspector_tab(ui, "Parameters", !self.model_blocks_tab).clicked() {
                    self.model_blocks_tab = false;
                }
                if model_inspector_tab(ui, "Blocks", self.model_blocks_tab).clicked() {
                    self.model_blocks_tab = true;
                }
            });
            // Keep the list tab open after selection so range/keyboard navigation
            // stays available. Parameters is an explicit, persistent user choice.
            if self.model_blocks_tab {
                self.block_list.show(inspector, &self.model, &mut self.canvas, &mut self.selected);
            } else {
                self.block_list.sync(&self.model);
                let labels = &self.block_list.entries;
                let inspector_budget = inspector.available_height().max(1.);
                egui::ScrollArea::vertical().id_salt(("block_parameters",self.selected)).max_height(inspector_budget).content_margin(egui::Margin::same(1)).show_themed(inspector,|ui|{
                if self.canvas.selection_len()>1 {crate::tokens::helper(ui, format!("{} blocks selected; parameters affect the active block.",self.canvas.selection_len()));}
                if let Some(node)=self.model.nodes.get_mut(self.selected){
                    let before = Properties::capture(node);
                    let mut property_changed = None;
                    ui.label(RichText::new(node.block.label()).strong());track_edit(&ui.add(tokens::input(egui::TextEdit::singleline(&mut node.name).char_limit(128))), &mut edit_active, &mut property_changed);
                    for (port,input) in node.inputs.iter_mut().enumerate(){
                        let input_label = format!("Input {}: {}",port+1,labels.get(*input).map(|entry|entry.label.as_str()).unwrap_or("not connected"));
                        let width = ui.available_width().max(1.);
                        ui.allocate_ui_with_layout(egui::vec2(width, ui.spacing().interact_size.y), egui::Layout::left_to_right(egui::Align::Center), |ui| {
                            egui::ComboBox::from_id_salt(("input",self.selected,port)).selected_text(&input_label).width(width).truncate().show_ui(ui,|ui| {
                                ui.set_width(width);
                                ui.selectable_value(input,usize::MAX,"Not connected");
                                for (i,entry) in labels.iter().enumerate() {
                                    if ui.add_sized([width, ui.spacing().interact_size.y], egui::Button::selectable(*input == i, &entry.label).wrap_mode(egui::TextWrapMode::Truncate)).on_hover_text(&entry.label).clicked() {
                                        *input = i;
                                        ui.close();
                                    }
                                }
                            }).response.on_hover_text(&input_label);
                        });
                    }
                    if let Some(response) = routing_ui::parameters(ui, node, node_edit_interactive) { track_edit(&response, &mut edit_active, &mut property_changed); }
                    match &mut node.block {
                        Block::Constant{value}=>{track_edit(&number(ui,"Value",value), &mut edit_active, &mut property_changed);},Block::Sine{amplitude,frequency_hz,phase}=>{track_edit(&number(ui,"Amplitude",amplitude), &mut edit_active, &mut property_changed);track_edit(&number(ui,"Frequency (Hz)",frequency_hz), &mut edit_active, &mut property_changed);track_edit(&number(ui,"Phase (rad)",phase), &mut edit_active, &mut property_changed);},Block::Step{time,before,after}=>{track_edit(&number(ui,"Step time (s)",time), &mut edit_active, &mut property_changed);track_edit(&number(ui,"Before",before), &mut edit_active, &mut property_changed);track_edit(&number(ui,"After",after), &mut edit_active, &mut property_changed);},Block::Gain{gain}=>{track_edit(&number(ui,"Gain",gain), &mut edit_active, &mut property_changed);},Block::Bias{bias}=>{track_edit(&number(ui,"Bias",bias), &mut edit_active, &mut property_changed);},Block::Saturation{lower,upper}=>{track_edit(&number(ui,"Lower limit",lower), &mut edit_active, &mut property_changed);track_edit(&number(ui,"Upper limit",upper), &mut edit_active, &mut property_changed);},Block::UnitDelay{initial}=>{track_edit(&number(ui,"Initial output",initial), &mut edit_active, &mut property_changed);},Block::DiscreteIntegrator{initial,gain,..}=>{ui.label("Forward Euler; model sample time");track_edit(&number(ui,"Initial output",initial), &mut edit_active, &mut property_changed);track_edit(&number(ui,"Integrator gain",gain), &mut edit_active, &mut property_changed);},Block::Filter{b,a}=>{ui.label(format!("{} numerator / {} denominator coefficients",b.len(),a.len()));},Block::SosFilter{sections}=>{ui.label(format!("{} second-order sections",sections.len()));},_=>{}
                    }
                    if matches!(node.block,Block::Filter{..}|Block::SosFilter{..}) && ui.button("Edit coefficients…").clicked(){self.coefficient_editor=coefficients::Editor::new(self.selected,&node.block);}
                    if !before.matches(node) { property_change = Some((self.selected, before, property_changed)); }
                    if ui.button(if self.canvas.selection_len()>1 {"Delete selected blocks"}else{"Delete selected block"}).clicked() && node_edit_interactive { delete_requested = true; }
                }

                else { ui.label("No blocks yet"); }
                });
            }
            };
            if compact_inspector {
                if self.model_inspector {
                    let p = tokens::palette(ctx);
                    let mut close = false;
                    egui::Area::new(egui::Id::new("model_inspector_sheet"))
                        .enabled(ui.is_enabled())
                        .order(egui::Order::Foreground)
                        .fixed_pos(egui::pos2(body.right() - 320., body.top()))
                        .show(ctx, |ui| {
                            egui::Frame::new().fill(p.surface_sunken)
                                .stroke(egui::Stroke::new(1., p.border))
                                .inner_margin(16).show(ui, |ui| {
                                    ui.style_mut().override_text_style = None;
                                    ui.spacing_mut().interact_size.y = tokens::CONTROL;
                                    ui.set_width(288.);
                                    ui.set_min_height((body.height() - 32.).max(1.));
                                    ui.set_max_height((body.height() - 32.).max(1.));
                                    ui.horizontal(|ui| {
                                        ui.label(egui::RichText::new("INSPECTOR").font(crate::panels::caption_semibold(ui)));
                                        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                                            close = ui.button("Close").clicked();
                                        });
                                    });
                                    show_inspector(ui);
                                });
                        });
                    if close { self.model_inspector = false; }
                }
            } else {
                let mut inspector = ui.new_child(egui::UiBuilder::new().id_salt("model_inspector_pane").max_rect(inspector_rect));
                inspector.set_clip_rect(ui.clip_rect().intersect(inspector_rect));
                // Every pane carries a text title, as the compact sheet does.
                let title = egui::RichText::new("INSPECTOR").font(crate::panels::caption_semibold(&inspector));
                inspector.label(title);
                show_inspector(&mut inspector);
            }
            if let Some((index, before, group)) = property_change
                && let Some(edit) = Edit::node(&self.model, index, before) { self.canvas.record_edit(edit, group); }
            if self.selected == prior_selected && self.model.nodes.len() == prior_count
                && let Some(node) = self.model.nodes.get(self.selected)
                && !matches!(node.block, Block::Product { .. }) {
                    for (port, (before, after)) in prior_inputs.iter().zip(&node.inputs).enumerate() {
                        self.canvas.record(self.selected, port, *before, *after);
                    }
            }
            if delete_requested { self.delete_block(); self.canvas.focus(ctx); edit_active = None; }
            self.canvas.finish_edit(if self.model_io_pending() { None } else { edit_active });
            // The model check sits in the status bar (`app_status`).
            if !self.message.is_empty(){ui.label(&self.message);}
        });
    }

    /// Overlays shared by every host: coefficient editor and model prompts.
    pub(super) fn finish_pass(&mut self, ctx: &egui::Context, pass: Pass) -> Option<Action> {
        // Tool actions win over the resolved notice, as before docking.
        let resolved = std::mem::take(&mut self.model_resolved);
        let row = self.row_action.take();
        let mut action = pass
            .action
            .or(row)
            .or_else(|| resolved.then_some(Action::ModelResolved));
        if self.model_document_region().is_none() {
            // A hidden model (closed or another tab shown) keeps no drag or edit in flight.
            self.canvas.finish_edit(None);
            self.canvas.cancel_connection();
        }
        if let Some(editor) = &mut self.coefficient_editor
            && editor.ui(ctx, &mut self.model, &mut self.canvas)
        {
            self.coefficient_editor = None;
        }
        if (self.pending_model.is_some() || self.new_model_pending)
            && self.model_conflict.is_none()
            && !self.model_io_pending()
        {
            let creating = self.new_model_pending;
            let mut new_action = 0;
            let prompt = egui::Modal::new(egui::Id::new("replace_model")).show(ctx, |ui| {
                ui.label(crate::tokens::title(ui.ctx(), "Unsaved block model"));
                ui.set_width((ctx.content_rect().width() - 48.).clamp(200., 480.));
                ui.label(if creating {
                    "Save this model before creating an empty untitled model?"
                } else {
                    "Save this model before opening the selected model?"
                });
                ui.horizontal_wrapped(|ui| {
                    if ui.button("Save model").clicked() {
                        if creating {
                            new_action = 1;
                        } else if self.save_model() {
                            self.complete_model_open();
                        }
                    }
                    if ui.button("Discard changes").clicked() {
                        if creating {
                            new_action = 2;
                        } else {
                            self.complete_model_open();
                        }
                    }
                    if ui.button("Cancel").clicked() {
                        if creating {
                            new_action = 3;
                        } else {
                            self.pending_model = None;
                        }
                    }
                });
                ui.label(&self.message);
            });
            if creating
                && prompt.is_top_modal
                && !prompt.any_popup_open
                && ctx.input_mut(|i| i.consume_key(egui::Modifiers::NONE, egui::Key::Escape))
            {
                new_action = 3;
            }
            match new_action {
                1 if self.save_model() => self.complete_model_open(),
                2 => self.complete_model_open(),
                3 => self.cancel_new_model(),
                _ => {}
            }
        }
        if !self.model_io_pending()
            && let Some(event) = self.model_conflict_ui(ctx)
        {
            action = Some(event);
        }
        action
    }
    fn delete_block(&mut self) {
        match self
            .canvas
            .delete_selection(&mut self.model, &mut self.selected)
        {
            Ok(()) => {
                self.coefficient_editor = None;
                self.message = "Selected blocks deleted; Undo diagram restores them".into();
            }
            Err(error) => self.message = error,
        }
    }
    fn add_block(&mut self, block: Block) {
        if let Err(error) = self.model.validate_layout() {
            self.message = error;
            return;
        }
        if self.model.nodes.len() >= 512 {
            self.message = "Model block limit reached".into();
            return;
        }
        let inputs = match block {
            Block::Constant { .. } | Block::Sine { .. } | Block::Step { .. } => vec![],
            Block::Sum => vec![usize::MAX; 2],
            Block::Product { ref operations } => vec![usize::MAX; operations.len()],
            Block::Switch { .. } => vec![usize::MAX; 3],
            _ => vec![usize::MAX],
        };
        let mut suffix = self.model.nodes.len() + 1;
        let name = loop {
            let name = format!("{} {suffix}", block.label());
            if !self.model.nodes.iter().any(|n| n.name == name) {
                break name;
            }
            suffix += 1;
        };
        match self.canvas.insert(
            &mut self.model,
            Node {
                name,
                block,
                inputs,
            },
        ) {
            Ok(index) => {
                self.selected = index;
                self.coefficient_editor = None;
                self.message = "Block added".into();
            }
            Err(error) => self.message = error,
        }
    }
}
fn track_edit(
    response: &egui::Response,
    active: &mut Option<egui::Id>,
    changed: &mut Option<egui::Id>,
) {
    if response.changed() {
        *changed = Some(response.id);
    }
    if response.has_focus() || response.dragged() || response.is_pointer_button_down_on() {
        *active = Some(response.id);
    }
}
fn number(ui: &mut egui::Ui, name: &str, value: &mut f64) -> egui::Response {
    ui.horizontal(|ui| {
        ui.label(name);
        ui.add(egui::DragValue::new(value).speed(0.01))
    })
    .inner
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn opening_another_model_preserves_unsaved_work_until_confirmed() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("other.mrsim");
        Model::default().save(&path).unwrap();
        let mut tools = Toolkits::default();
        tools.model.steps = 123;
        tools.request_open(path.clone());
        let request = tools.take_model_read().unwrap();
        tools.finish_model_read(super::super::model_read::read(
            request,
            &std::sync::atomic::AtomicBool::new(false),
        ));
        assert_eq!(tools.model.steps, 123);
        assert!(tools.pending_model.is_some());
        tools.pending_model = None;
        assert_eq!(tools.model.steps, 123);
        let (model_path, baseline) = tools.model_metadata();
        let mut restored = Toolkits::default();
        restored.restore_model(tools.model.clone(), model_path, baseline);
        assert!(restored.model_dirty());
        restored.discard_model();
        assert_eq!(restored.model.steps, 1000);
        restored.request_open(path);
        let request = restored.take_model_read().unwrap();
        restored.finish_model_read(super::super::model_read::read(
            request,
            &std::sync::atomic::AtomicBool::new(false),
        ));
        assert!(!restored.model_dirty());
    }
}

#[cfg(test)]
#[path = "toolkit_window_tests.rs"]
mod window_tests;

#[path = "designer_ui.rs"]
mod designer_ui;
#[cfg(test)]
#[path = "designer_ui_tests.rs"]
mod designer_ui_tests;

#[cfg(test)]
mod iir_capture_state_tests {
    use super::*;
    use crate::screenshot_options::IirDesignerCase;
    #[test]
    fn iir_capture_sets_bounded_design_without_model_or_file_actions() {
        let mut tools = Toolkits {
            input: "retained_input".into(),
            ..Default::default()
        };
        tools.file.path = Some(PathBuf::from("private-untouched.mrsim"));
        let original = tools.model.clone();
        let path = tools.file.path.clone();
        let dirty = tools.model_dirty();
        for case in [
            IirDesignerCase::Bandpass1,
            IirDesignerCase::Bandpass20,
            IirDesignerCase::Bandstop1,
            IirDesignerCase::Bandstop20,
        ] {
            tools.open_iir_capture(case);
            let (response, order) = case.settings();
            assert!(tools.designer_open && tools.spec.method.iir());
            assert_eq!((tools.spec.response, tools.spec.order), (response, order));
            assert_eq!((tools.spec.fs, tools.spec.fc), (1000., [100., 200.]));
            assert!(
                tools.spec.problems().is_empty(),
                "{:?}",
                tools.spec.problems()
            );
            assert!(
                tools
                    .preview_script()
                    .unwrap()
                    .contains(&format!("butter({order}, "))
            );
            assert!(tools.model.same_storage(&original));
            assert_eq!(tools.file.path, path);
            assert_eq!(tools.model_dirty(), dirty);
            assert_eq!(tools.input, "retained_input");
            assert!(!tools.saving_design);
            assert!(tools.model_write.is_none() && tools.model_read.is_none());
        }
    }
}
