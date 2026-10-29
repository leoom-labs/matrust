use super::*;
use crate::theme::ThemedScrollArea;

pub use matrust::session::{ExportFormat as Format, ExportSettings as Settings};
pub struct Dialog {
    id: u32,
    figure: Figure,
    pub settings: Settings,
    pub saving: bool,
    pub error: Option<String>,
    figures: Vec<(u32, Figure)>,
    all: bool,
    plan: Option<export_batch::Plan>,
    completed: usize,
    replace: bool,
    progress: Arc<std::sync::atomic::AtomicUsize>,
    action: Option<ActionIntent>,
    action_finished: Option<(egui::ViewportId, u64)>,
    plan_revision: u64,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Action {
    None,
    ChoosePath,
    ChooseFolder,
    ExportAll,
    Copy,
    Cancel,
    Stop,
}
#[derive(Clone, Copy)]
struct ActionIntent {
    viewport: egui::ViewportId,
    frame: u64,
    action: Action,
    settings: Settings,
    all: bool,
    saving: bool,
    completed: usize,
    replace: bool,
    plan_revision: u64,
}
impl Dialog {
    pub fn select_all(&mut self) {
        self.all = true;
    }
    pub fn new(id: u32, figure: &Figure, view: Option<[f64; 4]>, settings: Settings) -> Self {
        let mut figure = figure.clone();
        if let Some([a, b, c, d]) = view {
            figure.xlim = Some([a, b]);
            figure.ylim = Some([c, d]);
        }
        Self {
            id,
            figure,
            settings,
            saving: false,
            error: None,
            figures: vec![],
            all: false,
            plan: None,
            completed: 0,
            replace: false,
            progress: Arc::new(std::sync::atomic::AtomicUsize::new(0)),
            action: None,
            action_finished: None,
            plan_revision: 0,
        }
    }
    pub fn review(&mut self, plan: export_batch::Plan) {
        self.plan_revision = self.plan_revision.wrapping_add(1);
        self.plan = Some(plan);
        self.completed = 0;
        self.replace = false;
        self.saving = false;
        self.error = None;
    }
    pub fn complete_batch(&mut self, outcome: export_batch::Outcome) {
        self.plan_revision = self.plan_revision.wrapping_add(1);
        self.completed = outcome.completed;
        self.saving = false;
        self.error = outcome.error;
    }
    fn validate(&self) -> matrust::Result<()> {
        if self.settings.format.raster() {
            matrust::export::validate_size(self.settings.width, self.settings.height)?;
        } else if self.settings.format.vector() {
            self.settings.vector.validate()?;
        }
        Ok(())
    }
    pub fn request(&self, path: PathBuf) -> matrust::Result<Job> {
        self.validate()?;
        if !path
            .extension()
            .and_then(|e| e.to_str())
            .is_some_and(|ext| {
                self.settings
                    .format
                    .extensions()
                    .iter()
                    .any(|expected| ext.eq_ignore_ascii_case(expected))
            })
        {
            return Err(format!(
                "Use .{} for the selected {} format",
                self.settings.format.extensions()[0],
                self.settings.format.label()
            ));
        }
        if self.settings.format == Format::MatlabScript {
            matrust::files::validate_matlab_plot_script_path(&path)?;
        }
        let (width, height) = if self.settings.format.raster() {
            (self.settings.width, self.settings.height)
        } else {
            (960, 560)
        };
        Ok(Job::ExportSnapshot(Box::new(worker::ExportRequest {
            vector: self
                .settings
                .format
                .vector()
                .then_some(self.settings.vector),
            path,
            figure: self.figure.clone(),
            width,
            height,
        })))
    }
    pub fn copy_request(&self) -> matrust::Result<Job> {
        matrust::export::validate_size(self.settings.width, self.settings.height)?;
        Ok(Job::CopyFigure(Box::new(figure_copy::Request {
            figure: self.figure.clone(),
            width: self.settings.width,
            height: self.settings.height,
        })))
    }
    pub(super) fn cancel_action(&mut self) {
        if let Some(intent) = self.action.take() {
            self.action_finished = Some((intent.viewport, intent.frame));
        }
    }
    fn action_allowed(&self, action: Action) -> bool {
        match action {
            Action::None => false,
            Action::Cancel => !self.saving,
            Action::Stop => self.saving,
            Action::ChoosePath => !self.saving && !self.all && self.validate().is_ok(),
            Action::ChooseFolder => !self.saving && self.all && self.validate().is_ok(),
            Action::Copy => {
                !self.saving
                    && !self.all
                    && self.settings.format.raster()
                    && self.validate().is_ok()
            }
            Action::ExportAll => {
                !self.saving
                    && self.all
                    && self.validate().is_ok()
                    && self.plan.as_ref().is_some_and(|plan| {
                        self.replace
                            || !plan
                                .items
                                .iter()
                                .skip(self.completed)
                                .any(export_batch::Item::replaces)
                    })
            }
        }
    }
    fn finish_action(&mut self, ctx: &egui::Context, observed: Action, owns_modal: bool) -> Action {
        let stamp = (ctx.viewport_id(), ctx.cumulative_frame_nr());
        if self
            .action
            .is_some_and(|intent| (intent.viewport, intent.frame) != stamp)
        {
            self.cancel_action();
        }
        if observed != Action::None
            && owns_modal
            && self.action_allowed(observed)
            && self.action.is_none()
            && self.action_finished != Some(stamp)
        {
            self.action = Some(ActionIntent {
                viewport: stamp.0,
                frame: stamp.1,
                action: observed,
                settings: self.settings,
                all: self.all,
                saving: self.saving,
                completed: self.completed,
                replace: self.replace,
                plan_revision: self.plan_revision,
            });
        }
        if ctx.will_discard() {
            return Action::None;
        }
        let Some(intent) = self.action.take() else {
            return Action::None;
        };
        self.action_finished = Some((intent.viewport, intent.frame));
        if (intent.viewport, intent.frame) != stamp
            || !owns_modal
            || intent.settings != self.settings
            || intent.all != self.all
            || intent.saving != self.saving
            || intent.completed != self.completed
            || intent.replace != self.replace
            || intent.plan_revision != self.plan_revision
            || !self.action_allowed(intent.action)
        {
            return Action::None;
        }
        intent.action
    }
    fn show(&mut self, ctx: &egui::Context) -> Action {
        let mut action = Action::None;
        let shown = egui::Modal::new(egui::Id::new("figure_export")).show(ctx, |ui| {
            ui.set_width((ctx.content_rect().width() - 48.).clamp(240., 420.));
            ui.label(crate::tokens::title(ui.ctx(), if self.all {
                "Export all open figures".into()
            } else {
                format!("Export figure {}", self.id)
            }));
            egui::ScrollArea::vertical()
                .id_salt("export_options")
                .max_height((ctx.content_rect().height() - 200.).clamp(60., 600.))
                .auto_shrink([false, true])
                .show_themed(ui, |ui| {
            ui.label("Export the current view. Your editable figure stays open.");
            ui.separator();
            if self.plan.is_none() && !self.figures.is_empty() {
                ui.add_enabled(
                    !self.saving,
                    egui::Checkbox::new(
                        &mut self.all,
                        format!("All open figures ({})", self.figures.len()),
                    ),
                );
            }
            ui.add_enabled_ui(!self.saving && self.plan.is_none(), |ui| {
                egui::ComboBox::from_label("Format")
                    .selected_text(self.settings.format.label())
                    .show_ui(ui, |ui| {
                        for format in [
                            Format::Png,
                            Format::Jpeg,
                            Format::Svg,
                            Format::Pdf,
                            Format::Eps,
                            Format::Tiff,
                            Format::Bmp,
                            Format::Csv,
                            Format::MatlabScript,
                        ] {
                            if ui
                                .selectable_value(&mut self.settings.format, format, format.label())
                                .changed()
                            {
                                self.error = None;
                            }
                        }
                    });
                if self.settings.format.raster() {
                    ui.horizontal_wrapped(|ui| {
                        for (label, w, h) in [
                            ("Standard", 1440, 840),
                            ("Large", 2880, 1680),
                            ("Wide", 1920, 1080),
                            ("Square", 1600, 1600),
                        ] {
                            if ui.small_button(label).clicked() {
                                self.settings.width = w;
                                self.settings.height = h;
                                self.error = None;
                            }
                        }
                    });
                    egui::Grid::new("export_dimensions").show(ui, |ui| {
                        ui.label("Width (pixels)");
                        ui.add(
                            egui::DragValue::new(&mut self.settings.width)
                                .range(64..=8192)
                                .speed(10.),
                        );
                        ui.end_row();
                        ui.label("Height (pixels)");
                        ui.add(
                            egui::DragValue::new(&mut self.settings.height)
                                .range(64..=8192)
                                .speed(10.),
                        );
                        ui.end_row();
                    });
                    ui.label(
                        "The plot fits without stretching. Other proportions use the figure background for margins.",
                    );
                    ui.weak("64–8192 pixels per side; up to 32 million pixels.");
                } else if self.settings.format == Format::MatlabScript {
                    ui.label("Export original data and plotting commands for MATLAB.");
                    ui.weak("Large plots include a companion data folder. Keep it beside the script when moving or sharing both.");
                    ui.weak("Open the script and Run to reconstruct the figure. Some dense bar/patch combinations exceed MatRust's replay limits. MATLAB rendering may differ; exports never run automatically.");
                } else if self.settings.format == Format::Csv {
                    ui.label("Export all original samples, including hidden series.");
                } else {
                    ui.horizontal_wrapped(|ui| {
                        let presets = if matches!(self.settings.format, Format::Pdf | Format::Eps) {
                            [
                                ("Plot canvas", 960., 560.),
                                ("A4 portrait", 595.2756, 841.8898),
                                ("A4 landscape", 841.8898, 595.2756),
                                ("Letter", 612., 792.),
                            ]
                        } else {
                            [
                                ("Plot canvas", 960., 560.),
                                ("Large", 2880., 1680.),
                                ("Wide", 1920., 1080.),
                                ("Square", 1600., 1600.),
                            ]
                        };
                        for (label, width, height) in presets {
                            if ui.small_button(label).clicked() {
                                self.settings.vector = matrust::plot::Canvas { width, height };
                                self.error = None;
                            }
                        }
                    });
                    let unit = if matches!(self.settings.format, Format::Pdf | Format::Eps) {
                        "points"
                    } else {
                        "SVG pixels"
                    };
                    egui::Grid::new("vector_dimensions").show(ui, |ui| {
                        ui.label(format!("Width ({unit})"));
                        ui.add(
                            egui::DragValue::new(&mut self.settings.vector.width)
                                .range(64.0..=8192.)
                                .speed(1.)
                                .max_decimals(2),
                        );
                        ui.end_row();
                        ui.label(format!("Height ({unit})"));
                        ui.add(
                            egui::DragValue::new(&mut self.settings.vector.height)
                                .range(64.0..=8192.)
                                .speed(1.)
                                .max_decimals(2),
                        );
                        ui.end_row();
                    });
                    if matches!(self.settings.format, Format::Pdf | Format::Eps) {
                        ui.label(format!(
                            "{:.2} × {:.2} inches; 72 points per inch",
                            self.settings.vector.width / 72.,
                            self.settings.vector.height / 72.
                        ));
                    }
                    ui.label("Vector artwork stays sharp when resized.");
                    ui.weak("Centered fit preserves proportions; extra space uses the figure color.");
                    if self.settings.format == Format::Eps {
                        ui.weak("Opaque color EPS with outlined text. Preview with your configured Ghostscript.");
                    }
                }
            });
            if let Some(plan) = &self.plan {
                ui.separator();
                ui.add(egui::Label::new(crate::path_label::display(&plan.folder).to_string()).truncate())
                    .on_hover_text(crate::path_label::display(&plan.folder).to_string());
                egui::ScrollArea::vertical()
                    .id_salt("export_destinations")
                    .max_height(110.)
                    .show_themed(ui, |ui| {
                        for (index, item) in plan.items.iter().enumerate() {
                            let state = if index < self.completed {
                                "Exported"
                            } else if item.replaces() {
                                "Replace"
                            } else {
                                "New"
                            };
                            ui.label(format!(
                                "{} — {state}",
                                item.path.file_name().unwrap_or_default().to_string_lossy()
                            ));
                        }
                    });
                if plan
                    .items
                    .iter()
                    .skip(self.completed)
                    .any(export_batch::Item::replaces)
                {
                    ui.add_enabled(
                        !self.saving,
                        egui::Checkbox::new(&mut self.replace, "Replace existing files"),
                    );
                }
                ui.weak("Completed exports remain if you stop or a later file fails.");
            }
                });
            let validation = self.validate().err();
            if let Some(error) = self.error.as_ref().or(validation.as_ref()) {
                egui::ScrollArea::vertical().id_salt("export_error").max_height(70.)
                    .auto_shrink([false, true]).show_themed(ui, |ui| {
                        ui.colored_label(tokens::palette(ui.ctx()).error, error);
                    });
            }
            ui.separator();
            ui.horizontal_wrapped(|ui| {
                if self.saving {
                    ui.spinner();
                    ui.label(if self.all {
                        if let Some(plan) = &self.plan {
                            format!(
                                "Exported {} / {}",
                                self.progress.load(Ordering::Relaxed),
                                plan.items.len()
                            )
                        } else {
                            "Checking destinations…".into()
                        }
                    } else {
                        "Exporting…".into()
                    });
                    if ui.button("Stop").on_hover_text("Stop export (Esc)").clicked() {
                        action = Action::Stop;
                    }
                } else {
                    if self.all {
                        if let Some(plan) = &self.plan {
                            let allowed = self.replace
                                || !plan
                                    .items
                                    .iter()
                                    .skip(self.completed)
                                    .any(export_batch::Item::replaces);
                            if ui
                                .add_enabled(
                                    allowed && validation.is_none(),
                                    egui::Button::new(if self.completed == 0 {
                                        "Export all"
                                    } else {
                                        "Retry remaining"
                                    }),
                                )
                                .clicked()
                            {
                                action = Action::ExportAll;
                            }
                        }
                        if ui
                            .add_enabled(
                                validation.is_none(),
                                egui::Button::new(if self.plan.is_some() {
                                    "Change folder…"
                                } else {
                                    "Choose folder…"
                                }),
                            )
                            .clicked()
                        {
                            action = Action::ChooseFolder;
                        }
                    } else if ui
                        .add_enabled(validation.is_none(), egui::Button::new("Choose file…"))
                        .clicked()
                    {
                        action = Action::ChoosePath;
                    }
                    if ui.button("Cancel").clicked() {
                        action = Action::Cancel;
                    }
                    if !self.all
                        && self.settings.format.raster()
                        && ui
                            .add_enabled(validation.is_none(), egui::Button::new("Copy image"))
                            .on_hover_text(
                                "Copy the current view at the selected pixel dimensions, on the resolved figure background",
                            )
                            .clicked()
                    {
                        action = Action::Copy;
                    }
                }
            });
        });
        if shown.is_top_modal
            && !shown.any_popup_open
            && ctx.input_mut(|i| i.consume_key(egui::Modifiers::NONE, egui::Key::Escape))
        {
            action = if self.saving {
                Action::Stop
            } else {
                Action::Cancel
            };
        }
        self.finish_action(ctx, action, shown.is_top_modal && !shown.any_popup_open)
    }
}
impl Workspace {
    pub(super) fn copy_figure(&mut self) {
        if self.busy {
            return;
        }
        if let Some(figure) = &self.figure {
            let dialog = Dialog::new(
                self.current_figure,
                figure,
                self.view_bounds,
                self.export_settings,
            );
            match dialog.copy_request() {
                Ok(job) => self.submit(job),
                Err(error) => self.status = error,
            }
        }
    }
    pub(super) fn copy_export_image(&mut self) {
        let Some(dialog) = &mut self.figure_export else {
            return;
        };
        if dialog.saving || self.busy {
            return;
        }
        match dialog.copy_request() {
            Ok(job) => {
                self.recovery_dirty |= self.export_settings != dialog.settings;
                self.export_settings = dialog.settings;
                self.export_settings.normalize();
                dialog.error = None;
                dialog.saving = true;
                self.submit(job);
                if !self.busy
                    && let Some(dialog) = &mut self.figure_export
                {
                    dialog.saving = false;
                    dialog.error = Some(self.status.clone());
                }
            }
            Err(error) => dialog.error = Some(error),
        }
    }
    pub(super) fn begin_figure_export(&mut self) {
        if self.busy {
            return;
        }
        if let Some(figure) = &self.figure {
            let mut dialog = Dialog::new(
                self.current_figure,
                figure,
                self.view_bounds,
                self.export_settings,
            );
            dialog.figures = self
                .all_figures
                .iter()
                .map(|(&id, f)| {
                    let mut snapshot = f.clone();
                    if id == self.current_figure {
                        snapshot = dialog.figure.clone();
                    } else if let Some(view) = self.figure_views.get(&id) {
                        if !view.auto[0] {
                            snapshot.xlim = Some([view.bounds[0], view.bounds[1]]);
                        }
                        if !view.auto[1] {
                            snapshot.ylim = Some([view.bounds[2], view.bounds[3]]);
                        }
                    }
                    (id, snapshot)
                })
                .collect();
            self.figure_export = Some(dialog);
        }
    }
    pub(super) fn prepare_export_folder(&mut self, folder: PathBuf) {
        let Some(dialog) = &mut self.figure_export else {
            return;
        };
        if self.busy || dialog.saving || !dialog.all {
            return;
        }
        if let Err(error) = dialog.validate() {
            dialog.error = Some(error);
            return;
        }
        dialog.plan = None;
        dialog.completed = 0;
        dialog.replace = false;
        let job = Job::PrepareExportBatch(Box::new(export_batch::Prepare {
            folder,
            figures: dialog.figures.clone(),
            settings: dialog.settings,
        }));
        self.submit_export_batch(job);
    }
    pub(super) fn run_export_batch(&mut self) {
        let Some(dialog) = &mut self.figure_export else {
            return;
        };
        if self.busy || dialog.saving || !dialog.all {
            return;
        }
        let Some(plan) = dialog.plan.clone() else {
            return;
        };
        dialog.progress.store(dialog.completed, Ordering::Relaxed);
        let job = Job::ExportBatch(Box::new(export_batch::Run {
            plan,
            completed: dialog.completed,
            replace: dialog.replace,
            progress: dialog.progress.clone(),
        }));
        self.submit_export_batch(job);
    }
    fn submit_export_batch(&mut self, job: Job) {
        let dialog = self.figure_export.as_mut().unwrap();
        dialog.saving = true;
        dialog.error = None;
        self.submit(job);
        if !self.busy
            && let Some(dialog) = &mut self.figure_export
        {
            dialog.saving = false;
            dialog.error = Some(self.status.clone());
        }
    }
    pub(super) fn export_to_path(&mut self, path: PathBuf) {
        let Some(dialog) = &mut self.figure_export else {
            return;
        };
        if dialog.saving || self.busy {
            return;
        }
        match dialog.request(path) {
            Ok(job) => {
                self.recovery_dirty |= self.export_settings != dialog.settings;
                self.export_settings = dialog.settings;
                self.export_settings.normalize();
                dialog.error = None;
                dialog.saving = true;
                self.submit(job);
                if !self.busy
                    && let Some(dialog) = &mut self.figure_export
                {
                    dialog.saving = false;
                    dialog.error = Some(self.status.clone());
                }
            }
            Err(error) => dialog.error = Some(error),
        }
    }
    fn figure_export_action(&mut self, ctx: &egui::Context) -> Action {
        let Some(dialog) = &mut self.figure_export else {
            return Action::None;
        };
        let blocked = ctx.viewport_id() != egui::ViewportId::ROOT
            || self.pending_quit
            || self.allow_close
            || ctx.input(|i| i.viewport().close_requested())
            || (self.busy && !dialog.saving);
        let action = dialog.show(ctx);
        let mut settings = dialog.settings;
        settings.normalize();
        self.recovery_dirty |= self.export_settings != settings;
        self.export_settings = settings;
        if blocked {
            dialog.cancel_action();
            Action::None
        } else {
            action
        }
    }
    pub(super) fn figure_export_ui(&mut self, ctx: &egui::Context) {
        let action = self.figure_export_action(ctx);
        let Some(dialog) = &mut self.figure_export else {
            return;
        };
        match action {
            Action::None => {}
            Action::ChooseFolder => {
                if let Some(path) = rfd::FileDialog::new()
                    .set_directory(&self.folder)
                    .pick_folder()
                {
                    self.prepare_export_folder(path);
                }
            }
            Action::ExportAll => self.run_export_batch(),
            Action::Copy => self.copy_export_image(),
            Action::Cancel => {
                self.figure_export = None;
            }
            Action::Stop => self.worker.cancel.store(true, Ordering::Relaxed),
            Action::ChoosePath => {
                if let Some(path) = rfd::FileDialog::new()
                    .set_directory(&self.folder)
                    .add_filter(
                        dialog.settings.format.label(),
                        dialog.settings.format.extensions(),
                    )
                    .set_file_name(dialog.settings.format.file_name(dialog.id))
                    .save_file()
                {
                    self.export_to_path(path);
                }
            }
        }
    }
}

#[cfg(test)]
#[path = "figure_export_layout_tests.rs"]
mod layout_tests;
