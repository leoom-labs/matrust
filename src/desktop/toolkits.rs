use crate::theme::ThemedScrollArea;
#[path = "model_routing_ui.rs"]
mod routing_ui;
use super::tokens;
use eframe::egui::{self, RichText};
use matrust::{
    dsp::{Analysis, Band},
    simulation::{Block, Model, Node},
};
use model_canvas::parameters::{Edit, Properties, Settings};
use std::path::PathBuf;
#[path = "coefficients.rs"]
mod coefficients;
#[path = "model_canvas.rs"]
mod model_canvas;
#[path = "model_files.rs"]
pub(super) mod model_files;
#[path = "model_list.rs"]
mod model_list;

pub enum Action {
    Script(String),
    Apply(super::designer_apply::Design, String),
    FirBlock(super::fir_designer::Design),
    IirBlock(super::iir_designer::Design),
    ChooseDesignSave(super::designer_apply::Design),
    // Existing tests retain the accepted-request reservation contract.
    #[cfg(test)]
    SaveFir(super::fir_designer::Save),
    Simulate(Model),
    ModelResolved,
    ModelCancelled,
    StopModel,
    NewModel,
    ModelFile(model_files::ToolbarKind),
}
/// Tools shown as closable tabs in the main work area.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Tool {
    Designer,
    Model,
}
impl Tool {
    pub fn label(self) -> &'static str {
        match self {
            Self::Designer => "Filter Designer",
            Self::Model => "Block Simulation",
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
    tab_order: [Tool; 2],
    selected_tool: Option<Tool>,
    tab_region: Option<(Tool, egui::Rect, egui::LayerId)>,
    iir: bool,
    iir_family: super::iir_designer::Family,
    ripple_db: f64,
    fir_window: matrust::dsp::WindowKind,
    fir_scale: bool,
    pending_design_model: Option<(Model, Option<PathBuf>, String)>,
    saving_design: bool,
    band: Band,
    order: usize,
    sample_rate: f64,
    cutoff: f64,
    upper: f64,
    input: String,
    analysis: Analysis,
    analysis_points: usize,
    coefficient_editor: Option<coefficients::Editor>,
    pub model: Model,
    model_identity: u64,
    selected: usize,
    canvas: model_canvas::Canvas,
    validation: model_canvas::validation::Cache,
    block_list: model_list::List,
    model_blocks_tab: bool,
    add_kind: usize,
    last_node_edit_frame: Option<u64>,
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
impl Default for Toolkits {
    fn default() -> Self {
        Self {
            designer_open: false,
            model_open: false,
            tab_order: [Tool::Designer, Tool::Model],
            selected_tool: None,
            tab_region: None,
            iir: false,
            iir_family: super::iir_designer::Family::Butterworth,
            ripple_db: 1.,
            fir_window: matrust::dsp::WindowKind::Hamming,
            fir_scale: true,
            pending_design_model: None,
            saving_design: false,
            band: Band::Lowpass,
            order: 64,
            sample_rate: 1000.,
            cutoff: 50.,
            upper: 150.,
            input: "x".into(),
            analysis: Analysis::Magnitude,
            analysis_points: 1024,
            coefficient_editor: None,
            model: Model::default(),
            model_identity: 0,
            selected: 3,
            canvas: Default::default(),
            validation: Default::default(),
            block_list: Default::default(),
            model_blocks_tab: false,
            add_kind: 0,
            last_node_edit_frame: None,
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
        }
    }
    /// Adds the tool's tab if missing and selects it.
    pub fn open_tool(&mut self, tool: Tool) {
        if !self.is_open(tool) && self.tab_order[0] == tool {
            self.tab_order.swap(0, 1);
        }
        match tool {
            Tool::Designer => self.designer_open = true,
            Tool::Model => self.model_open = true,
        }
        self.selected_tool = Some(tool);
    }
    /// Open tabs for the session layout; `None` when no tool is open.
    pub fn tab_layout(&self) -> Option<matrust::session::ToolTabs> {
        let convert = |tool| match tool {
            Tool::Designer => matrust::session::ToolTab::Designer,
            Tool::Model => matrust::session::ToolTab::Model,
        };
        let mut open = [None; 2];
        for (slot, tool) in open.iter_mut().zip(self.tabs()) {
            *slot = Some(convert(tool));
        }
        open[0].map(|_| matrust::session::ToolTabs {
            open,
            selected: self.selected_tool().map(convert),
        })
    }
    /// Reopens the saved tabs in their order; the shown tool is returned.
    pub fn restore_tab_layout(&mut self, tabs: Option<matrust::session::ToolTabs>) -> Option<Tool> {
        let convert = |tool| match tool {
            matrust::session::ToolTab::Designer => Tool::Designer,
            matrust::session::ToolTab::Model => Tool::Model,
        };
        let tabs = tabs?;
        for tool in tabs.open.into_iter().flatten() {
            self.open_tool(convert(tool));
        }
        self.deselect_tool();
        let selected = tabs.selected.map(convert);
        if let Some(tool) = selected {
            self.select_tool(tool);
        }
        selected
    }
    /// Open tool tabs in opening order.
    pub fn tabs(&self) -> impl Iterator<Item = Tool> + '_ {
        self.tab_order
            .into_iter()
            .filter(|tool| self.is_open(*tool))
    }
    /// Tab label; an unsaved model keeps the former title's `*` marker.
    pub fn tab_label(&self, tool: Tool) -> String {
        if tool == Tool::Model && self.model_dirty() {
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
        self.open_tool(Tool::Designer);
        self.iir = iir;
        if iir {
            self.iir_family = super::iir_designer::Family::Butterworth;
            self.order = 6;
            self.band = Band::Lowpass;
        }
    }
    pub(super) fn open_iir_capture(&mut self, case: super::screenshot_options::IirDesignerCase) {
        self.open_designer_demo(true);
        (self.band, self.order) = case.settings();
        self.sample_rate = 1000.;
        self.cutoff = 100.;
        self.upper = 200.;
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
    fn iir_design(&self) -> super::iir_designer::Design {
        super::iir_designer::Design {
            family: self.iir_family,
            ripple_db: self.ripple_db,
            order: self.order,
            band: self.band,
            sample_rate: self.sample_rate,
            cutoff: self.cutoff,
            upper: self.upper,
        }
    }
    fn fir_design(&self) -> super::fir_designer::Design {
        super::fir_designer::Design {
            order: self.order,
            band: self.band,
            sample_rate: self.sample_rate,
            cutoff: self.cutoff,
            upper: self.upper,
            window: self.fir_window,
            scale: self.fir_scale,
        }
    }
    #[cfg(test)]
    fn request_fir_block(&mut self) -> matrust::Result<super::fir_designer::Design> {
        let design = self.fir_design();
        self.begin_fir_block(design)?;
        Ok(design)
    }
    pub(super) fn begin_fir_block(
        &mut self,
        design: super::fir_designer::Design,
    ) -> matrust::Result<()> {
        self.begin_filter_block(design.summary()?)
    }
    pub(super) fn begin_iir_block(
        &mut self,
        design: super::iir_designer::Design,
    ) -> matrust::Result<()> {
        self.begin_filter_block(design.summary()?)
    }
    fn begin_filter_block(&mut self, summary: String) -> matrust::Result<()> {
        if self.pending_design_model.is_some() || self.saving_design {
            return Err("A filter operation is already pending".into());
        }
        self.model.validate_storage()?;
        if self.model.nodes.len() >= 512 {
            return Err("Model block limit reached".into());
        }
        // One bounded model snapshot guards publication; no workspace matrix is copied.
        self.pending_design_model =
            Some((self.model.clone(), self.file.path.clone(), summary.clone()));
        self.message = format!("Designing {summary} for the block model...");
        Ok(())
    }
    pub(super) fn finish_filter_block(&mut self, result: matrust::Result<Block>) -> bool {
        let Some((original, path, summary)) = self.pending_design_model.take() else {
            return false;
        };
        let block = match result {
            Ok(block) => block,
            Err(error) => {
                self.message = error;
                return false;
            }
        };
        if self.file.path != path || !self.model.same_storage(&original) {
            self.message =
                "Model changed while designing; no block was added. Try Add to block model again."
                    .into();
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
    pub(super) fn selected_save_design(&self) -> super::designer_apply::Design {
        if self.iir {
            super::designer_apply::Design::Iir(self.iir_design())
        } else {
            super::designer_apply::Design::Fir(self.fir_design())
        }
    }
    #[cfg(test)]
    pub(super) fn set_save_design_for_test(&mut self, design: super::designer_apply::Design) {
        match design {
            super::designer_apply::Design::Fir(d) => {
                self.iir = false;
                self.order = d.order;
                self.band = d.band;
                self.sample_rate = d.sample_rate;
                self.cutoff = d.cutoff;
                self.upper = d.upper;
                self.fir_window = d.window;
                self.fir_scale = d.scale;
            }
            super::designer_apply::Design::Iir(d) => {
                self.iir = true;
                self.iir_family = d.family;
                self.ripple_db = d.ripple_db;
                self.order = d.order;
                self.band = d.band;
                self.sample_rate = d.sample_rate;
                self.cutoff = d.cutoff;
                self.upper = d.upper;
            }
        }
    }
    pub(super) fn design_save_available(&self) -> bool {
        self.designer_open
            && self.pending_design_model.is_none()
            && !self.saving_design
            && !self.model_io_pending()
            && match self.selected_save_design() {
                super::designer_apply::Design::Fir(design) => design.summary().is_ok(),
                super::designer_apply::Design::Iir(design) => design.summary().is_ok(),
            }
    }
    pub(super) fn begin_iir_save(
        &mut self,
        design: super::iir_designer::Design,
    ) -> matrust::Result<()> {
        self.begin_design_save(design.summary()?)
    }
    pub(super) fn begin_fir_save(
        &mut self,
        design: super::fir_designer::Design,
    ) -> matrust::Result<()> {
        self.begin_design_save(design.summary()?)
    }
    fn begin_design_save(&mut self, summary: String) -> matrust::Result<()> {
        if self.pending_design_model.is_some() || self.saving_design {
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
    fn design_script(&self) -> matrust::Result<String> {
        if self.iir {
            self.iir_design().script()
        } else {
            self.fir_design().script()
        }
    }

    #[cfg(test)]
    fn exported_script(&self) -> matrust::Result<String> {
        if self.iir {
            return Err("IIR portable export requires the background save request".into());
        }
        self.fir_design().exported_script()
    }

    pub(super) fn preview_script(&self) -> matrust::Result<String> {
        let design = self.design_script()?;
        if !(1..=65536).contains(&self.analysis_points) {
            return Err("Analysis point count must be 1..65536".into());
        }
        let n = self.analysis_points;
        let kind = self.analysis;
        let coefficients = if self.iir { "sos" } else { "b,1" };
        let axis = if kind.time_domain() {
            format!("(0:{})/{}", n - 1, self.sample_rate)
        } else {
            format!("(0:{})*({}/(2*{}))", n - 1, self.sample_rate, n)
        };
        let values = if kind == Analysis::Magnitude {
            "20*log(max(response,1e-15))/log(10)"
        } else {
            "response"
        };
        Ok(format!(
            "{design}\nresponse = {}({coefficients},{n});\nresponse_axis = {axis};\nplot(response_axis,{values}); title('{}'); xlabel('{}'); ylabel('{}');",
            kind.function(self.iir),
            if self.iir {
                kind.label().to_owned()
            } else {
                format!("{} - {}", kind.label(), self.fir_design().summary()?)
            },
            if kind.time_domain() {
                "Time (s)"
            } else {
                "Frequency (Hz)"
            },
            if kind.time_domain() {
                "Amplitude"
            } else {
                kind.label()
            }
        ))
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
            Tool::Model => self.model_ui(ui, busy, interactive, &mut pass.action),
        }
    }

    /// Standalone host: the selected (else first open) tool fills the viewport.
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
                self.tab_ui(ui, tool, busy, &mut pass);
            });
        }
        self.finish_pass(ctx, pass)
    }

    fn model_ui(
        &mut self,
        ui: &mut egui::Ui,
        busy: bool,
        node_edit_interactive: bool,
        action: &mut Option<Action>,
    ) {
        let ctx = &ui.ctx().clone();
        egui::ScrollArea::both().id_salt("block_simulation").auto_shrink([false, false]).show_themed(ui,|ui|{
            // Compact windows use dense rows, so the diagram keeps its height.
            if tokens::compact(ctx) { tokens::dense(ui); }
            if self.model_io_pending() {
                ui.horizontal_wrapped(|ui| {
                    ui.label(&self.message);
                    if ui.button("Stop model operation").clicked() { *action = Some(Action::StopModel); }
                });
                ui.small("A write already in progress may finish; Stop cancels subsequent actions.");
                ui.disable();
            }
            let mut edit_active = None;
            let mut setting_changed = None;
            let settings_before = Settings::capture(&self.model);
            ui.horizontal_wrapped(|ui|{
                ui.label("Sample time (s)").on_hover_text("One fixed sample rate; scalar signals; Unit Delay and Forward Euler break feedback loops. Results: sim_t, sim_1, sim_2, ...");track_edit(&ui.add(egui::DragValue::new(&mut self.model.sample_time).speed(0.0001).max_decimals(6)), &mut edit_active, &mut setting_changed);
                ui.label("Samples");track_edit(&ui.add(egui::DragValue::new(&mut self.model.steps).range(1..=1_000_000)), &mut edit_active, &mut setting_changed);
                if ui.add_enabled(!busy && !self.model.nodes.is_empty(),egui::Button::new("▶ Run model")).clicked(){*action=Some(Action::Simulate(self.model.clone()));}
                if ui.add_enabled(!busy && !self.modal_open(), egui::Button::new("New model")).clicked() && node_edit_interactive { *action = Some(Action::NewModel); }
                if ui.button("Open…").clicked(){*action=Some(Action::ModelFile(model_files::ToolbarKind::Open));}
                if ui.button("Save model").clicked(){*action=Some(Action::ModelFile(model_files::ToolbarKind::Save));}
                if ui.button("Save as\u{2026}").clicked(){*action=Some(Action::ModelFile(model_files::ToolbarKind::SaveAs));}
            });
            if let Some(edit) = Edit::settings(&self.model, settings_before) { self.canvas.record_edit(edit, setting_changed); }
            if let Some(path)=&self.file.path{ui.small(crate::path_label::display(path).to_string());}
            ui.separator();
            ui.horizontal(|ui|{
                let names=["Constant","Sine","Step","Gain","Sum","Unit Delay","Discrete Filter","SOS Filter","Scope","Abs","Bias","Saturation","Product","Unary Minus","Switch","Discrete-Time Integrator"];
                egui::ComboBox::from_id_salt("add_block").selected_text(names[self.add_kind]).show_ui(ui,|ui|{for (i,name) in names.iter().enumerate(){ui.selectable_value(&mut self.add_kind,i,*name);}});
                if ui.add_enabled(self.model.nodes.len()<512,egui::Button::new("Add block")).clicked() && node_edit_interactive {let block=match self.add_kind{0=>Block::Constant{value:1.},1=>Block::Sine{amplitude:1.,frequency_hz:10.,phase:0.},2=>Block::Step{time:0.1,before:0.,after:1.},3=>Block::Gain{gain:1.},4=>Block::Sum,5=>Block::UnitDelay{initial:0.},6=>Block::Filter{b:vec![0.5,0.5],a:vec![1.]},7=>Block::SosFilter{sections:vec![[1.,0.,0.,1.,0.,0.]]},9=>Block::Abs,10=>Block::Bias{bias:1.},11=>Block::Saturation{lower:-0.5,upper:0.5},12=>Block::Product{operations:vec![matrust::simulation::ProductInput::Multiply;2]},13=>Block::UnaryMinus,14=>Block::Switch{criteria:matrust::simulation::SwitchCriteria::default(),threshold:0.},15=>Block::DiscreteIntegrator{initial:0.,gain:1.,method:matrust::simulation::IntegratorMethod::ForwardEuler},_=>Block::Scope};self.add_block(block);}
            });
            let keyboard_allowed = !self.modal_open();
            let row = ui.text_style_height(&egui::TextStyle::Button).max(ui.spacing().interact_size.y) + ui.spacing().item_spacing.y;
            // Separate graph and inspector budgets: parameters never consume
            // diagram height. Tiny hosts retain the outer Window scroll fallback.
            // The status line always follows the body; a message adds a second row.
            let footer = row * if self.message.is_empty() { 1. } else { 2. } + 8.;
            let body_height = (ui.clip_rect().bottom() - ui.cursor().top() - footer).max(240.);
            let body_width = ui.available_width().max(560.);
            let (body, _) = ui.allocate_exact_size(egui::vec2(body_width, body_height), egui::Sense::hover());
            let inspector_width = (body.width() * 0.35).clamp(240., 320.);
            let split = body.right() - inspector_width;
            let graph_rect = egui::Rect::from_min_max(body.min, egui::pos2(split - 8., body.bottom()));
            let inspector_rect = egui::Rect::from_min_max(egui::pos2(split, body.top()), body.max);
            let mut graph = ui.new_child(egui::UiBuilder::new().id_salt("model_graph_pane").max_rect(graph_rect));
            graph.set_clip_rect(ui.clip_rect().intersect(graph_rect));
            let prior_topology_count = self.model.nodes.len();
            self.canvas.show_pane(&mut graph, &mut self.model, &mut self.selected, &mut self.message, keyboard_allowed);
            if self.model.nodes.len() != prior_topology_count { self.coefficient_editor = None; }
            ui.painter().vline(split - 4., body.y_range(), ui.visuals().widgets.noninteractive.bg_stroke);
            let prior_selected = self.selected;
            let prior_count = self.model.nodes.len();
            let prior_inputs = self.model.nodes.get(self.selected).map(|node| node.inputs.clone()).unwrap_or_default();
            let mut property_change = None;
            let mut delete_requested = false;
            let mut inspector = ui.new_child(egui::UiBuilder::new().id_salt("model_inspector_pane").max_rect(inspector_rect));
            inspector.set_clip_rect(ui.clip_rect().intersect(inspector_rect));
            inspector.horizontal(|ui| {
                ui.selectable_value(&mut self.model_blocks_tab, false, "Parameters");
                ui.selectable_value(&mut self.model_blocks_tab, true, "Blocks");
            });
            // Keep the list tab open after selection so range/keyboard navigation
            // stays available. Parameters is an explicit, persistent user choice.
            if self.model_blocks_tab {
                self.block_list.show(&mut inspector, &self.model, &mut self.canvas, &mut self.selected);
            } else {
                self.block_list.sync(&self.model);
                let labels = &self.block_list.entries;
                let inspector_budget = inspector.available_height().max(1.);
                egui::ScrollArea::vertical().id_salt(("block_parameters",self.selected)).max_height(inspector_budget).content_margin(egui::Margin::same(1)).show_themed(&mut inspector,|ui|{
                if self.canvas.selection_len()>1 {ui.small(format!("{} blocks selected; parameters affect the active block.",self.canvas.selection_len()));}
                if let Some(node)=self.model.nodes.get_mut(self.selected){
                    let before = Properties::capture(node);
                    let mut property_changed = None;
                    ui.label(RichText::new(node.block.label()).strong());track_edit(&ui.add(egui::TextEdit::singleline(&mut node.name).char_limit(128)), &mut edit_active, &mut property_changed);
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
            // Empty documents are valid storage, but not runnable graphs. Keep
            // invalid layout/storage and run settings visible as errors.
            let empty_ready = self.model.nodes.is_empty()
                && self.model.validate_storage().is_ok()
                && self.model.sample_time > 0.
                && (self.model.sample_time * self.model.steps as f64).is_finite()
                && (1..=1_000_000).contains(&self.model.steps);
            if empty_ready {
                ui.label("Add a block to start building this model.");
            } else {
            match self.validation.status(&self.model, !busy){Ok(_)=>{ui.colored_label(super::tokens::palette(ui.ctx()).success,"Model valid · ready to simulate");},Err(e)=>{ui.colored_label(super::tokens::palette(ui.ctx()).error,e);}}
            }
            if !self.message.is_empty(){ui.label(&self.message);}
        });
    }

    /// Overlays shared by every host: coefficient editor and model prompts.
    pub(super) fn finish_pass(&mut self, ctx: &egui::Context, pass: Pass) -> Option<Action> {
        // Tool actions win over the resolved notice, as before docking.
        let resolved = std::mem::take(&mut self.model_resolved);
        let mut action = pass
            .action
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
    fn all_designer_previews_execute_with_correct_units_and_sample_count() {
        for iir in [false, true] {
            for analysis in Analysis::ALL {
                let designer = Toolkits {
                    iir,
                    analysis,
                    order: 6,
                    analysis_points: 128,
                    ..Default::default()
                };
                let mut engine = matrust::Engine::default();
                engine.execute(&designer.preview_script().unwrap()).unwrap();
                let figure = engine.figure.unwrap();
                assert_eq!(figure.series[0].points.len(), 128);
                assert_eq!(
                    figure.xlabel,
                    if analysis.time_domain() {
                        "Time (s)"
                    } else {
                        "Frequency (Hz)"
                    }
                );
                let end = if analysis.time_domain() {
                    127. / 1000.
                } else {
                    127. * 1000. / 256.
                };
                assert!((figure.series[0].points.get(127).unwrap()[0] - end).abs() < 1e-12);
                assert!(figure.series[0].points.iter().any(|p| p[1].is_finite()));
            }
        }
    }
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
    #[test]
    fn exported_sos_script_is_portable_and_reproduces_exact_coefficients() {
        let designer = Toolkits {
            iir: true,
            order: 6,
            ..Default::default()
        };
        let script = designer
            .iir_design()
            .portable_script(&std::sync::atomic::AtomicBool::new(false))
            .unwrap();
        assert!(!script.contains("butter_sos("));
        let mut engine = matrust::Engine::default();
        engine.execute(&script).unwrap();
        let expected = matrust::dsp::butter_sos(6, 0.1, Band::Lowpass).unwrap();
        assert_eq!(engine.variables["sos"].data(), expected.data());
    }
}

#[cfg(test)]
#[path = "toolkit_window_tests.rs"]
mod window_tests;

#[cfg(test)]
#[path = "fir_designer_tests.rs"]
mod fir_designer_tests;

#[cfg(test)]
#[path = "iir_bands_tests.rs"]
mod iir_bands_tests;
#[cfg(test)]
#[path = "iir_designer_tests.rs"]
mod iir_designer_tests;

#[cfg(test)]
#[path = "designer_layout_tests.rs"]
mod designer_layout_tests;
#[path = "designer_ui.rs"]
mod designer_ui;

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
            let (band, order) = case.settings();
            assert!(tools.designer_open && tools.iir);
            assert_eq!((tools.band, tools.order), (band, order));
            assert_eq!(
                (tools.sample_rate, tools.cutoff, tools.upper),
                (1000., 100., 200.)
            );
            tools.iir_design().validate().unwrap();
            assert!(tools.preview_script().unwrap().starts_with(&format!(
                "sos = butter_sos({order}, [0.2 0.4], '{}');",
                band.name()
            )));
            assert!(tools.model.same_storage(&original));
            assert_eq!(tools.file.path, path);
            assert_eq!(tools.model_dirty(), dirty);
            assert_eq!(tools.input, "retained_input");
            assert!(!tools.saving_design && tools.pending_design_model.is_none());
            assert!(tools.model_write.is_none() && tools.model_read.is_none());
        }
    }
}

#[cfg(test)]
#[path = "chebyshev_designer_tests.rs"]
mod chebyshev_designer_tests;
