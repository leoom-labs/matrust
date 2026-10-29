#[path = "text_search.rs"]
pub mod text_search;
use matrust::{Array, Engine, Execution, Matrix, Result, files, plot::Figure, simulation::Model};
#[path = "figure_documents.rs"]
mod figure_documents;
#[path = "figure_history.rs"]
mod figure_history;
pub use figure_documents::{FigureConflict, FigureStatus};
use files::figure_version::DiskVersion;
use std::{
    collections::BTreeMap,
    path::PathBuf,
    sync::{
        Arc,
        atomic::AtomicBool,
        mpsc::{self, Receiver, SyncSender},
    },
    time::{Duration, Instant},
};
pub enum Job {
    SearchFolder(Box<text_search::Request>),
    OpenSearchResult(Box<SearchOpen>),
    RootedVariable(Box<super::variable_edit_job::Request>),
    ReplaceCell(Box<super::cell_replace_job::Request>),
    PrepareDetached(Box<super::detached_figures::Prepare>),
    SelectExistingFigure(super::figure_target::Select),
    CloseExistingFigure(super::figure_target::Key),
    ApplyFilter(Box<super::designer_apply::Request>),
    FirBlock(super::fir_designer::Design),
    IirBlock(super::iir_designer::Design),
    SaveIirDesign(Box<super::iir_designer::Save>),
    SaveFirDesign(Box<super::fir_designer::Save>),
    EditWorkspace(Box<super::workspace_edit::Request>),
    ScriptIo(Box<super::script_io::Request>),
    ModelWrite(Box<super::model_write::Request>),
    ReadModel(super::model_read::Request),
    PrepareOpen(Box<super::open_files::Request>),
    PrepareExportBatch(Box<super::export_batch::Prepare>),
    ExportBatch(Box<super::export_batch::Run>),
    PickSample(Box<super::sample_inspector::Request>),
    RestoreSession(
        PathBuf,
        Option<matrust::session::figures::FigureManifest>,
        Option<matrust::session::workspace::Manifest>,
    ),
    #[cfg(test)]
    Script(String),
    ScriptInFolder(String, PathBuf),
    DocumentInFolder(String, PathBuf, PathBuf),
    SelectionInFolder(Arc<str>, std::ops::Range<usize>, PathBuf),
    ScriptViewContext(Box<Job>, Vec<(u32, HistoryView)>),
    Simulate(Model),
    InspectWorkspace(PathBuf),
    LoadSelectedWorkspace(Box<files::MatCatalog>, std::collections::BTreeSet<String>),
    SaveWorkspace(PathBuf),
    SaveWorkspaceV73(PathBuf),
    ImportCsv(Box<super::csv_import::Request>),
    ExportVariable(PathBuf, String),
    #[cfg(test)]
    SetCell(String, usize, usize, matrust::Scalar),
    OpenFigure(PathBuf),
    OpenFigureOrSelect(PathBuf),
    OpenImage(PathBuf),
    OpenEps(PathBuf, PathBuf),
    OpenPagedImage(PathBuf, u32),
    ImagePage(super::image_pages::Page),
    ExportPreview(super::image_export::Request),
    OriginalCopy(Box<super::original_copy::Request>),
    SingleExport(Box<super::single_export::Request>),
    CopyPreview(Arc<eframe::egui::ColorImage>),
    CopyFigure(Box<super::figure_copy::Request>),
    ReloadFigure(u32, PathBuf),
    SaveFigureConfirmed(u32, PathBuf, Option<[f64; 4]>, DiskVersion),
    EditFigure(Box<FigureEdit>),
    ViewFigure(Box<FigureEdit>),
    UndoFigure(u32, Option<HistoryView>),
    RedoFigure(u32, Option<HistoryView>),
    NewLayout(matrust::plot::TileGrid),
    PlotData(Box<super::plot_request::Request>),
    SaveFigure(PathBuf, Option<[f64; 4]>),
    SaveFigureById(u32, PathBuf, Option<[f64; 4]>),
    #[cfg(test)]
    ExportFigure(PathBuf, Option<[f64; 4]>),
    ExportSnapshot(Box<ExportRequest>),
}
pub struct ExportRequest {
    pub vector: Option<matrust::plot::Canvas>,
    pub path: PathBuf,
    pub figure: Figure,
    pub width: u32,
    pub height: u32,
}
impl Job {
    pub fn absolute_figure_paths(self) -> Result<Self> {
        self.in_folder(std::path::Path::new("."))
    }
    pub fn in_folder(self, folder: &std::path::Path) -> Result<Self> {
        let absolute = |p: PathBuf| {
            std::path::absolute(if p.is_absolute() { p } else { folder.join(p) })
                .map_err(|e| e.to_string())
        };
        Ok(match self {
            Self::SaveFirDesign(mut request) => {
                request.path = absolute(std::mem::take(&mut request.path))?;
                Self::SaveFirDesign(request)
            }
            Self::SaveIirDesign(mut request) => {
                request.path = absolute(std::mem::take(&mut request.path))?;
                Self::SaveIirDesign(request)
            }
            Self::ScriptIo(mut request) => {
                if let super::script_io::Kind::Save { path, .. }
                | super::script_io::Kind::SaveEdited { path, .. } = &mut request.kind
                {
                    *path = absolute(std::mem::take(path))?;
                }
                Self::ScriptIo(request)
            }
            Self::ModelWrite(mut request) => {
                if let super::model_write::Request::Save(save) = request.as_mut() {
                    save.path = absolute(std::mem::take(&mut save.path))?;
                }
                Self::ModelWrite(request)
            }
            Self::ReadModel(mut request) => {
                request.path = absolute(request.path)?;
                Self::ReadModel(request)
            }
            Self::PrepareOpen(mut request) => {
                request.path = absolute(request.path)?;
                Self::PrepareOpen(request)
            }
            Self::DocumentInFolder(source, path, directory) => {
                Self::DocumentInFolder(source, absolute(path)?, directory)
            }
            Self::SaveWorkspace(p) => Self::SaveWorkspace(absolute(p)?),
            Self::SaveWorkspaceV73(p) => Self::SaveWorkspaceV73(absolute(p)?),
            Self::ImportCsv(mut request) => {
                request.path = absolute(request.path)?;
                Self::ImportCsv(request)
            }
            Self::ExportVariable(p, name) => Self::ExportVariable(absolute(p)?, name),
            Self::PrepareExportBatch(mut request) => {
                request.folder = absolute(request.folder)?;
                Self::PrepareExportBatch(request)
            }
            Self::OpenFigure(p) => Self::OpenFigure(absolute(p)?),
            Self::OpenFigureOrSelect(p) => Self::OpenFigureOrSelect(absolute(p)?),
            Self::OpenImage(p) => Self::OpenImage(absolute(p)?),
            Self::OpenEps(p, renderer) => Self::OpenEps(absolute(p)?, absolute(renderer)?),
            Self::OpenPagedImage(p, index) => Self::OpenPagedImage(absolute(p)?, index),
            Self::OriginalCopy(mut request) => {
                if let super::original_copy::Action::Prepare(prepare) = &mut request.action {
                    prepare.source = absolute(std::mem::take(&mut prepare.source))?;
                    prepare.destination = absolute(std::mem::take(&mut prepare.destination))?;
                }
                Self::OriginalCopy(request)
            }
            Self::ExportPreview(mut request) => {
                request.path = absolute(request.path)?;
                request.source = absolute(request.source)?;
                Self::ExportPreview(request)
            }
            Self::ReloadFigure(id, p) => Self::ReloadFigure(id, absolute(p)?),
            Self::SaveFigureConfirmed(id, p, v, expected) => {
                Self::SaveFigureConfirmed(id, absolute(p)?, v, expected)
            }
            Self::SaveFigure(p, v) => Self::SaveFigure(absolute(p)?, v),
            Self::SaveFigureById(id, p, v) => Self::SaveFigureById(id, absolute(p)?, v),
            #[cfg(test)]
            Self::ExportFigure(p, v) => Self::ExportFigure(absolute(p)?, v),
            Self::ExportSnapshot(mut request) => {
                request.path = absolute(request.path)?;
                Self::ExportSnapshot(request)
            }
            Self::InspectWorkspace(p) => Self::InspectWorkspace(absolute(p)?),
            job => job,
        })
    }
}
pub type HistoryView = ([f64; 4], [bool; 2]);
pub fn with_history_view(figure: &Figure, view: Option<HistoryView>) -> Result<Figure> {
    let mut snapshot = figure.clone();
    if let Some((v, auto)) = view {
        if !auto[0] {
            snapshot.xlim = Some([v[0], v[1]]);
        }
        if !auto[1] {
            snapshot.ylim = Some([v[2], v[3]]);
        }
    }
    snapshot.validate()?;
    Ok(snapshot)
}
pub struct FigureEdit {
    pub view_before: Option<HistoryView>,
    pub id: u32,
    pub original: Figure,
    pub updated: Figure,
}
impl FigureEdit {
    /// Visibility and position edits honor the user's current manual pan/zoom without
    /// freezing axes that are still automatic. Explicit property edits win.
    pub fn preserve_manual_view(&mut self, view: Option<[f64; 4]>, auto: [bool; 2]) {
        let Some(view) = view else { return };
        self.view_before = Some((view, auto));
        let changed = self.original.active_position() != self.updated.active_position()
            || self.updated.series.iter().any(|s| {
                self.original
                    .series
                    .iter()
                    .any(|old| old.points.same_storage(&s.points) && old.visible != s.visible)
            });
        if !changed {
            return;
        }
        if !auto[0]
            && self.updated.xlim == self.original.xlim
            && self.updated.xscale == self.original.xscale
        {
            self.updated.xlim = Some([view[0], view[1]]);
        }
        if !auto[1]
            && self.updated.ylim == self.original.ylim
            && self.updated.yscale == self.original.yscale
        {
            self.updated.ylim = Some([view[2], view[3]]);
        }
    }
}
pub struct RecoveredFigures {
    pub windows: BTreeMap<u32, super::figure_recovery::Placement>,
    pub placement_notes: Vec<String>,
    pub views: BTreeMap<u32, matrust::session::figures::FigureView>,
    pub warnings: Vec<String>,
}
pub struct SearchOpen {
    pub generation: u64,
    pub query: text_search::Query,
    pub hit: text_search::Hit,
    pub interaction: u64,
    pub open: super::open_files::Request,
}
pub struct SearchOpened {
    pub generation: u64,
    pub query: text_search::Query,
    pub hit: text_search::Hit,
    pub interaction: u64,
    pub existing_path: Option<PathBuf>,
    pub result: Result<super::open_files::Prepared>,
}
pub struct Finished {
    pub current_folder: Option<PathBuf>,
    /// The engine's `format` setting after the job, for workspace previews.
    pub display: matrust::display_format::DisplayFormat,
    pub folder_search: Option<text_search::Finished>,
    pub search_open: Option<SearchOpened>,
    pub variable_revision: super::variable_edit_job::Token,
    pub variable_edit: Option<super::variable_edit_job::Finished>,
    pub cell_replacement: Option<super::cell_replace_job::Finished>,
    pub detached_prepared: Option<super::detached_figures::Prepared>,
    pub selected_existing: Option<super::figure_target::Acknowledgement>,
    pub fir_block: Option<Result<matrust::simulation::Block>>,
    pub design_save: Option<Result<PathBuf>>,
    pub workspace_change: Option<super::workspace_edit::Change>,
    pub script_io: Option<super::script_io::Finished>,
    pub model_write: Option<super::model_write::Finished>,
    pub model_read: Option<super::model_read::Finished>,
    pub prepared_open: Option<Result<super::open_files::Prepared>>,
    pub export_plan: Option<super::export_batch::Plan>,
    pub original_copy: Option<super::original_copy::Finished>,
    pub single_export: Option<super::single_export::Finished>,
    pub export_outcome: Option<super::export_batch::Outcome>,
    pub sample_hit: Option<matrust::plot::SampleHit>,
    pub image_preview: Option<super::image_load::Preview>,
    pub page_preview: Option<super::image_pages::Page>,
    pub figures: BTreeMap<u32, Figure>,
    pub recovered: Option<RecoveredFigures>,
    pub result: Result<Execution>,
    pub vars: BTreeMap<String, Array>,
    pub figure: Option<Figure>,
    pub figure_ids: Vec<u32>,
    pub current_figure: u32,
    pub figure_documents: BTreeMap<u32, FigureStatus>,
    pub figure_history: BTreeMap<u32, (usize, usize)>,
    pub saved_figure: Option<u32>,
    pub reloaded_figure: Option<u32>,
    pub figure_conflict: Option<FigureConflict>,
    pub workspace_catalog: Option<files::MatCatalog>,
    pub elapsed: Duration,
    pub diagnostic: Option<matrust::syntax::Diagnostic>,
    pub file_diagnostic: Option<matrust::engine::FileDiagnostic>,
    pub partial_output: Execution,
}
/// Immutable host context bound to a job at submission, never read on the worker.
pub struct Request {
    pub job: Job,
    pub theme: matrust::plot::BaseTheme,
    /// Command window width in characters, for `Columns 1 through N` wrapping.
    pub console_width: usize,
}
impl From<Job> for Request {
    fn from(job: Job) -> Self {
        Self {
            job,
            theme: matrust::plot::BaseTheme::Light,
            console_width: matrust::display_format::DEFAULT_WIDTH,
        }
    }
}

pub struct Worker {
    pub opening: bool,
    pub send: SyncSender<Request>,
    pub receive: Receiver<Finished>,
    pub cancel: Arc<AtomicBool>,
    thread: Option<std::thread::JoinHandle<()>>,
    request_context: eframe::egui::Context,
    /// Width of the command window in characters, refreshed every frame.
    pub console_width: usize,
}
impl Worker {
    pub fn new(context: eframe::egui::Context) -> Self {
        let request_context = context.clone();
        let (send, requests) = mpsc::sync_channel::<Request>(1);
        let (responses, receive) = mpsc::sync_channel(1);
        let mut engine = Engine::default();
        let cancel = engine.cancellation();
        let thread = std::thread::spawn(move || {
            let mut documents = figure_documents::Documents::default();
            let mut variable_revision = super::variable_edit_job::Token::default();
            while let Ok(Request {
                job,
                theme,
                console_width,
            }) = requests.recv()
            {
                engine.set_graphics_theme_context(theme);
                engine.set_console_width(console_width);
                engine.file_diagnostic = None;
                let folder_job = match &job {
                    Job::ScriptViewContext(inner, _) => inner.as_ref(),
                    _ => &job,
                };
                let submitted_folder = match folder_job {
                    Job::ScriptInFolder(_, folder)
                    | Job::DocumentInFolder(_, _, folder)
                    | Job::SelectionInFolder(_, _, folder) => Some(folder.clone()),
                    _ => None,
                };
                // The folder the engine holds once the job's own folder was set;
                // a failed set reports no change.
                let start_folder = submitted_folder.and_then(|folder| {
                    engine.set_current_folder(&folder).ok()?;
                    engine.current_folder().map(std::path::Path::to_owned)
                });
                let start = Instant::now();
                let mut workspace_catalog = None;
                let mut recovered = None;
                let mut image_preview = None;
                let mut page_preview = None;
                let mut sample_hit = None;
                let mut export_plan = None;
                let mut original_copy = None;
                let mut single_export = None;
                let mut export_outcome = None;
                let mut prepared_open = None;
                let mut folder_search = None;
                let mut search_open = None;
                let mut model_read = None;
                let mut fir_block = None;
                let mut design_save = None;
                let mut model_write = None;
                let mut script_io = None;
                let mut workspace_change = None;
                let mut variable_edit = None;
                let mut cell_replacement = None;
                let selection = match &job {
                    Job::SelectExistingFigure(request) => {
                        Some((request.key.clone(), request.ticket))
                    }
                    _ => None,
                };
                let mut detached_prepared = None;
                let (result, saved_figure) = if let Job::RootedVariable(request) = job {
                    engine.diagnostic = None;
                    engine.failed_output = Execution::default();
                    let finished = super::variable_edit_job::perform(
                        &mut engine,
                        &variable_revision,
                        *request,
                    );
                    let result = finished
                        .result
                        .as_ref()
                        .map(|_| Execution::default())
                        .map_err(Clone::clone);
                    variable_edit = Some(finished);
                    (result, None)
                } else if let Job::ReplaceCell(request) = job {
                    engine.diagnostic = None;
                    engine.failed_output = Execution::default();
                    let finished =
                        super::cell_replace_job::perform(&mut engine, &variable_revision, *request);
                    let result = finished
                        .result
                        .as_ref()
                        .map(|_| Execution::default())
                        .map_err(Clone::clone);
                    cell_replacement = Some(finished);
                    (result, None)
                } else if let Job::PrepareDetached(request) = job {
                    engine.diagnostic = None;
                    engine.failed_output = Execution::default();
                    let geometry = request.key.check(&engine).and_then(|()| {
                        let live = engine
                            .figures()
                            .find(|(id, _)| *id == request.key.number)
                            .unwrap()
                            .1;
                        if !live.same_snapshot(&request.original) {
                            return Err("Figure changed before display preparation".into());
                        }
                        super::figure_geometry::Geometry::prepare(
                            request.key.number,
                            live,
                            &engine.cancellation(),
                        )
                    });
                    let result = geometry
                        .as_ref()
                        .map(|_| Execution::default())
                        .map_err(Clone::clone);
                    detached_prepared = Some(super::detached_figures::Prepared {
                        request: *request,
                        geometry,
                    });
                    (result, None)
                } else if let Job::SearchFolder(request) = job {
                    folder_search = Some(text_search::perform(*request, &engine.cancellation()));
                    (Ok(Execution::default()), None)
                } else if let Job::OpenSearchResult(request) = job {
                    let SearchOpen {
                        generation,
                        query,
                        hit,
                        interaction,
                        open,
                    } = *request;
                    let scripts = open.scripts.clone();
                    let result = text_search::verify(&hit, &engine.cancellation())
                        .and_then(|()| super::open_files::prepare(open, &engine.cancellation()));
                    let existing_path = match &result {
                        Ok(super::open_files::Prepared::ExistingScript(id, _)) => scripts
                            .into_iter()
                            .find(|(candidate, _)| candidate == id)
                            .map(|(_, path)| path),
                        _ => None,
                    };
                    search_open = Some(SearchOpened {
                        generation,
                        query,
                        hit,
                        interaction,
                        existing_path,
                        result,
                    });
                    (Ok(Execution::default()), None)
                } else if let Job::PrepareOpen(request) = job {
                    engine.diagnostic = None;
                    engine.failed_output = Execution::default();
                    let prepared = super::open_files::prepare(*request, &engine.cancellation());
                    let result = prepared
                        .as_ref()
                        .map(|_| Execution::default())
                        .map_err(Clone::clone);
                    prepared_open = Some(prepared);
                    (result, None)
                } else if let Job::EditWorkspace(request) = job {
                    engine.diagnostic = None;
                    engine.failed_output = Execution::default();
                    let result =
                        super::workspace_edit::perform(&mut engine, *request).map(|change| {
                            let output = vec![change.message()];
                            workspace_change = Some(change);
                            Execution {
                                output,
                                ..Default::default()
                            }
                        });
                    (result, None)
                } else if let Job::ScriptIo(request) = job {
                    engine.diagnostic = None;
                    engine.failed_output = Execution::default();
                    script_io = Some(super::script_io::perform(*request, &engine.cancellation()));
                    (Ok(Execution::default()), None)
                } else if let Job::ModelWrite(request) = job {
                    engine.diagnostic = None;
                    engine.failed_output = Execution::default();
                    model_write = Some(super::model_write::perform(
                        *request,
                        &engine.cancellation(),
                    ));
                    (Ok(Execution::default()), None)
                } else if let Job::SaveFirDesign(request) = job {
                    engine.diagnostic = None;
                    engine.failed_output = Execution::default();
                    design_save = Some(super::fir_designer::save(*request, &engine.cancellation()));
                    (Ok(Execution::default()), None)
                } else if let Job::SaveIirDesign(request) = job {
                    engine.diagnostic = None;
                    engine.failed_output = Execution::default();
                    design_save = Some(super::iir_designer::save(*request, &engine.cancellation()));
                    (Ok(Execution::default()), None)
                } else if let Job::IirBlock(request) = job {
                    engine.diagnostic = None;
                    engine.failed_output = Execution::default();
                    fir_block = Some(request.block(&engine.cancellation()));
                    (Ok(Execution::default()), None)
                } else if let Job::FirBlock(request) = job {
                    engine.diagnostic = None;
                    engine.failed_output = Execution::default();
                    fir_block = Some(request.block(&engine.cancellation()));
                    (Ok(Execution::default()), None)
                } else if let Job::ReadModel(request) = job {
                    engine.diagnostic = None;
                    engine.failed_output = Execution::default();
                    model_read = Some(super::model_read::read(request, &engine.cancellation()));
                    (Ok(Execution::default()), None)
                } else if matches!(
                    &job,
                    Job::OpenImage(_)
                        | Job::OpenEps(..)
                        | Job::OpenPagedImage(..)
                        | Job::ImagePage(_)
                ) {
                    engine.diagnostic = None;
                    engine.failed_output = Execution::default();
                    let side = context.input(|i| i.max_texture_side);
                    let cancel = engine.cancellation();
                    let loaded = match job {
                        Job::OpenPagedImage(path, index) => {
                            super::image_pages::open(&path, index, side, &cancel)
                                .map(|(image, page)| (image, Some(page)))
                        }
                        Job::ImagePage(page) => {
                            super::image_pages::render(page.document, page.index, side, &cancel)
                                .map(|(image, page)| (image, Some(page)))
                        }
                        Job::OpenEps(path, renderer) => {
                            super::eps_load::load(&path, &renderer, side, &cancel)
                                .map(|image| (image, None))
                        }
                        Job::OpenImage(path) => {
                            let load = if path
                                .extension()
                                .and_then(|e| e.to_str())
                                .is_some_and(|e| e.eq_ignore_ascii_case("svg"))
                            {
                                super::svg_load::load
                            } else {
                                super::image_load::load
                            };
                            load(&path, side, &cancel).map(|image| (image, None))
                        }
                        _ => unreachable!(),
                    };
                    let result = loaded.map(|(preview, page)| {
                        let message =
                            format!("Opened image {}", crate::path_label::display(&preview.path));
                        image_preview = Some(preview);
                        page_preview = page;
                        Execution {
                            output: vec![message],
                            clear_console: false,
                        }
                    });
                    (result, None)
                } else if matches!(&job, Job::CopyPreview(_) | Job::CopyFigure(_)) {
                    engine.diagnostic = None;
                    engine.failed_output = Execution::default();
                    let (prepared, message) = match job {
                        Job::CopyPreview(pixels) => (
                            super::image_export::clipboard(&pixels, &engine.cancellation()),
                            "Sent preview image to clipboard",
                        ),
                        Job::CopyFigure(request) => (
                            super::figure_copy::prepare_in_theme(
                                &request,
                                theme,
                                &engine.cancellation(),
                            ),
                            "Sent figure image to clipboard",
                        ),
                        _ => unreachable!(),
                    };
                    let result = prepared.map(|image| {
                        context.copy_image(image);
                        Execution {
                            output: vec![message.into()],
                            clear_console: false,
                        }
                    });
                    (result, None)
                } else if let Job::SingleExport(request) = job {
                    engine.diagnostic = None;
                    engine.failed_output = Execution::default();
                    single_export = Some(super::single_export::perform(
                        *request,
                        theme,
                        &engine.cancellation(),
                    ));
                    (Ok(Execution::default()), None)
                } else if let Job::OriginalCopy(request) = job {
                    engine.diagnostic = None;
                    engine.failed_output = Execution::default();
                    original_copy = Some(super::original_copy::perform(
                        *request,
                        &engine.cancellation(),
                    ));
                    (Ok(Execution::default()), None)
                } else if let Job::PrepareExportBatch(request) = job {
                    engine.diagnostic = None;
                    engine.failed_output = Execution::default();
                    let result = super::export_batch::prepare(*request, &engine.cancellation())
                        .map(|mut plan| {
                            plan.theme = theme;
                            export_plan = Some(plan);
                            Execution::default()
                        });
                    (result, None)
                } else if let Job::ExportBatch(request) = job {
                    engine.diagnostic = None;
                    engine.failed_output = Execution::default();
                    let outcome =
                        super::export_batch::run(*request, &engine.cancellation(), || {
                            context.request_repaint_of(eframe::egui::ViewportId::ROOT)
                        });
                    let result = outcome.error.clone().map_or_else(
                        || {
                            Ok(Execution {
                                output: vec![format!("Exported {} figures", outcome.completed)],
                                clear_console: false,
                            })
                        },
                        Err,
                    );
                    export_outcome = Some(outcome);
                    (result, None)
                } else if let Job::PickSample(request) = job {
                    engine.diagnostic = None;
                    engine.failed_output = Execution::default();
                    let result = request
                        .axes
                        .nearest_sample(request.query, &engine.cancellation())
                        .map(|hit| {
                            sample_hit = hit;
                            Execution::default()
                        });
                    (result, None)
                } else if let Job::InspectWorkspace(path) = job {
                    engine.diagnostic = None;
                    engine.failed_output = Execution::default();
                    let result =
                        files::inspect_workspace(&path, &engine.cancellation()).map(|catalog| {
                            let count = catalog.variables().len();
                            workspace_catalog = Some(catalog);
                            Execution {
                                output: vec![format!(
                                    "Inspected {count} variables in {}",
                                    crate::path_label::display(&path)
                                )],
                                clear_console: false,
                            }
                        });
                    (result, None)
                } else if let Job::RestoreSession(path, manifest, workspace) = job {
                    let report = manifest.map_or_else(
                        matrust::session::figures::FigureRestoration::empty,
                        |manifest| {
                            matrust::session::figures::restore_figures(
                                &path,
                                &manifest,
                                &engine.cancellation(),
                            )
                        },
                    );
                    let views = report
                        .figures
                        .figures
                        .iter()
                        .filter_map(|d| d.view.clone().map(|v| (d.id, v)))
                        .collect();
                    let count = report.figures.figures.len();
                    let mut windows: BTreeMap<_, _> = report
                        .figures
                        .figures
                        .iter()
                        .filter_map(|doc| {
                            doc.window.clone().map(|window| {
                                (
                                    doc.id,
                                    super::figure_recovery::Placement {
                                        identity: doc.figure.identity.clone(),
                                        window,
                                    },
                                )
                            })
                        })
                        .collect();
                    let mut bindings = report.workspace_bindings().clone();
                    let mut warnings = report.warnings;
                    let placement_notes = report.placement_notes;
                    let result =
                        documents
                            .restore(&mut engine, report.figures)
                            .map(|_| Execution {
                                output: vec![format!(
                                    "Recovered {count} open figures from the last checkpoint."
                                )],
                                clear_console: false,
                            });
                    if let Err(error) = &result {
                        bindings.detach_all();
                        windows.clear();
                        warnings.push(error.clone());
                    }
                    // A decoded reference alone is insufficient: only identities actually
                    // admitted by Documents::restore may leave this worker.
                    windows.retain(|id, p| {
                        engine.figures().any(|(live_id, f)| {
                            live_id == *id && p.identity.same_object(&f.identity)
                        })
                    });
                    let mut result = result;
                    if let Some(manifest) = workspace {
                        match matrust::session::workspace::restore_with_figures(
                            &path,
                            &manifest,
                            &bindings,
                            &engine.cancellation(),
                        )
                        .and_then(|restored| {
                            warnings.extend(restored.warnings);
                            let values = restored.values;
                            let count = values.len();
                            engine.merge_workspace(values)?;
                            Ok(count)
                        }) {
                            Ok(count) => {
                                if let Ok(execution) = &mut result {
                                    execution.output.push(format!("Recovered {count} workspace variables from the last checkpoint."));
                                    execution.output.extend(manifest.skipped_note());
                                }
                            }
                            Err(error) => warnings.push(format!("Workspace: {error}")),
                        }
                    }
                    recovered = Some(RecoveredFigures {
                        views,
                        warnings,
                        windows,
                        placement_notes,
                    });
                    (result, None)
                } else {
                    documents.perform(&mut engine, job)
                };
                let selected_existing =
                    selection.map(|(key, ticket)| super::figure_target::Acknowledgement {
                        key,
                        ticket,
                        accepted: result.is_ok(),
                    });
                // Every completed job invalidates earlier requests, including remove/rebind
                // to exactly the same immutable root within one script.
                variable_revision = super::variable_edit_job::Token::default();
                let current_folder = start_folder.and_then(|started| {
                    let current = engine.current_folder()?;
                    (current != started).then(|| current.to_owned())
                });
                let reply = Finished {
                    current_folder,
                    display: engine.display,
                    folder_search,
                    search_open,
                    variable_revision: variable_revision.clone(),
                    variable_edit,
                    cell_replacement,
                    detached_prepared,
                    selected_existing,
                    fir_block,
                    design_save,
                    workspace_change,
                    script_io,
                    model_write,
                    model_read,
                    prepared_open,
                    export_plan,
                    original_copy,
                    single_export,
                    export_outcome,
                    sample_hit,
                    image_preview,
                    page_preview,
                    figures: engine.figures().map(|(id, f)| (id, f.clone())).collect(),
                    recovered,
                    diagnostic: engine.diagnostic.take(),
                    file_diagnostic: engine.file_diagnostic.take(),
                    partial_output: std::mem::take(&mut engine.failed_output),
                    result,
                    vars: engine.workspace(),
                    figure: engine.figure.clone(),
                    figure_ids: engine.figure_ids(),
                    current_figure: engine.current_figure,
                    figure_documents: documents.statuses(),
                    figure_history: documents.history_counts(),
                    saved_figure,
                    reloaded_figure: documents.take_reloaded(),
                    figure_conflict: documents.take_conflict(),
                    workspace_catalog,
                    elapsed: start.elapsed(),
                };
                if responses.send(reply).is_err() {
                    break;
                }
                context.request_repaint_of(eframe::egui::ViewportId::ROOT);
            }
        });
        Self {
            opening: false,
            send,
            receive,
            cancel,
            thread: Some(thread),
            request_context,
            console_width: matrust::display_format::DEFAULT_WIDTH,
        }
    }
    pub fn theme(&self) -> matrust::plot::BaseTheme {
        super::figure_view::host_theme(&self.request_context)
    }
    pub fn request(&self, job: Job) -> Request {
        Request {
            job,
            theme: self.theme(),
            console_width: self.console_width,
        }
    }
    /// Release native results and let the worker finish before graphics/runtime
    /// teardown. Disconnect both channels so an unread response cannot block join.
    pub fn shutdown(&mut self) {
        if self.thread.is_none() {
            return;
        }
        self.cancel
            .store(true, std::sync::atomic::Ordering::Relaxed);
        let (send, _) = mpsc::sync_channel(0);
        let (_, receive) = mpsc::sync_channel(0);
        drop(std::mem::replace(&mut self.send, send));
        drop(std::mem::replace(&mut self.receive, receive));
        if let Some(thread) = self.thread.take()
            && (!self.opening || thread.is_finished())
        {
            let _ = thread.join();
        }
        // A preflight OS read may not be interruptible. Its channels are now
        // disconnected; the owned worker exits when that call returns, without
        // retaining a native preview or blocking window teardown.
    }
}
impl Drop for Worker {
    fn drop(&mut self) {
        self.shutdown();
    }
}

#[cfg(test)]
mod opening_shutdown_tests {
    use super::*;
    #[test]
    fn file_preflight_shutdown_disconnects_without_joining_a_blocked_read() {
        let (release, gate) = mpsc::channel();
        let (finished, done) = mpsc::channel();
        let thread = std::thread::spawn(move || {
            gate.recv_timeout(Duration::from_secs(2)).unwrap();
            finished.send(()).unwrap();
        });
        let (send, _requests) = mpsc::sync_channel(1);
        let (_responses, receive) = mpsc::sync_channel(1);
        let cancel = Arc::new(AtomicBool::new(false));
        let mut worker = Worker {
            opening: true,
            send,
            receive,
            cancel: cancel.clone(),
            thread: Some(thread),
            request_context: eframe::egui::Context::default(),
            console_width: matrust::display_format::DEFAULT_WIDTH,
        };
        worker.shutdown();
        assert!(cancel.load(std::sync::atomic::Ordering::Relaxed));
        assert!(worker.thread.is_none());
        // This send only succeeds if shutdown returned before the blocked read
        // was released. Then explicitly await termination so the test leaks none.
        release.send(()).unwrap();
        done.recv_timeout(Duration::from_secs(2)).unwrap();
    }
}
fn merge(engine: &mut Engine, new: BTreeMap<String, Matrix>) -> Result<()> {
    engine.merge_workspace(new.into_iter().map(|(name, m)| (name, m.into())).collect())
}
fn figure_snapshot(engine: &Engine, view: Option<[f64; 4]>) -> Result<Figure> {
    let mut figure = engine.figure.clone().ok_or("No figure to save or export")?;
    if let Some([a, b, c, d]) = view {
        matrust::plot::validate_limits([a, b])?;
        matrust::plot::validate_limits([c, d])?;
        figure.xlim = Some([a, b]);
        figure.ylim = Some([c, d]);
    }
    Ok(figure)
}
fn perform(engine: &mut Engine, job: Job) -> Result<Execution> {
    engine.diagnostic = None;
    engine.failed_output = Execution::default();
    let message = match job {
        Job::ApplyFilter(request) => return super::designer_apply::perform(engine, *request),
        Job::PrepareDetached(_) => {
            return Err("Display preparation requires the worker response channel".into());
        }
        Job::SelectExistingFigure(request) => {
            request.key.check(engine)?;
            engine.select_figure(request.key.number)?;
            return Ok(Execution::default());
        }
        Job::CloseExistingFigure(key) => {
            engine.close_existing_figure(key.number, &key.identity)?;
            return Ok(Execution::default());
        }
        Job::OpenFigureOrSelect(..)
        | Job::ScriptViewContext(..)
        | Job::ReloadFigure(..)
        | Job::SaveFigureConfirmed(..)
        | Job::UndoFigure(..)
        | Job::RedoFigure(..) => {
            return Err("Protected figure operation requires document tracking".into());
        }
        Job::RestoreSession(..) => {
            return Err("Figure recovery requires the worker response channel".into());
        }
        Job::InspectWorkspace(_) | Job::PickSample(_) => {
            return Err("Workspace inspection requires the worker response channel".into());
        }
        Job::RootedVariable(_)
        | Job::ReplaceCell(_)
        | Job::EditWorkspace(_)
        | Job::ScriptIo(_)
        | Job::ModelWrite(_)
        | Job::ReadModel(_)
        | Job::FirBlock(_)
        | Job::IirBlock(_)
        | Job::SaveIirDesign(_)
        | Job::SaveFirDesign(_)
        | Job::SearchFolder(_)
        | Job::OpenSearchResult(_)
        | Job::PrepareOpen(_)
        | Job::OriginalCopy(_)
        | Job::SingleExport(_)
        | Job::PrepareExportBatch(_)
        | Job::ExportBatch(_) => {
            return Err("Batch export requires the worker response channel".into());
        }
        Job::OpenImage(_)
        | Job::OpenEps(..)
        | Job::OpenPagedImage(..)
        | Job::ImagePage(_)
        | Job::CopyPreview(_)
        | Job::CopyFigure(_) => {
            return Err("Image preview requires the worker response channel".into());
        }
        Job::LoadSelectedWorkspace(catalog, names) => {
            let retained = engine
                .workspace()
                .iter()
                .filter(|(name, _)| !names.contains(*name))
                .map(|(_, m)| m.bytes() as u64)
                .sum::<u64>();
            let incoming = catalog
                .variables()
                .filter(|v| names.contains(&v.name))
                .try_fold(0u64, |n, v| {
                    n.checked_add(v.storage_bytes().unwrap_or(u64::MAX))
                })
                .ok_or("Selected workspace size overflow")?;
            if retained.saturating_add(incoming) > 256 * 1024 * 1024 {
                return Err("Selection plus existing variables exceeds 256 MiB; clear unused variables or select fewer".into());
            }
            let variables = files::load_selected_arrays(&catalog, &names, &engine.cancellation())?;
            let count = variables.len();
            engine.merge_workspace(variables)?;
            format!(
                "Loaded {count} selected variables from {}",
                crate::path_label::display(catalog.path())
            )
        }
        #[cfg(test)]
        Job::Script(source) => return engine.execute(&source),
        Job::ScriptInFolder(source, folder) => {
            engine.set_current_folder(&folder)?;
            return engine.execute(&source);
        }
        Job::DocumentInFolder(source, path, folder) => {
            engine.set_current_folder(&folder)?;
            return engine.execute_document(&source, &path);
        }
        Job::SelectionInFolder(source, range, folder) => {
            engine.set_current_folder(&folder)?;
            return engine.execute_selection(&source, range);
        }
        #[cfg(test)]
        Job::SetCell(name, row, col, value) => {
            let matrix = engine
                .array(&name)
                .ok_or("Variable no longer exists")?
                .with_scalar(row, col, value, &engine.cancellation())?;
            engine.merge_workspace(BTreeMap::from([(name.clone(), matrix)]))?;
            format!("{name}({},{}) = {value}", row + 1, col + 1)
        }
        Job::Simulate(model) => {
            let simulation = model.run(&engine.cancellation())?;
            let mut vars = BTreeMap::from([("sim_t".into(), simulation.time.clone())]);
            let mut description = Vec::new();
            for (i, trace) in simulation.traces.iter().enumerate() {
                let name = format!("sim_{}", i + 1);
                description.push(format!("{name}: {}", trace.name));
                vars.insert(name, trace.values.clone());
            }
            let mut figure = Figure::new(&simulation.time, &simulation.traces[0].values)?;
            figure.title = format!("Simulation · {}", simulation.traces[0].name);
            figure.xlabel = "Time (seconds)".into();
            figure.ylabel = "Amplitude".into();
            merge(engine, vars)?;
            engine.figure = Some(figure);
            format!(
                "Simulated {} samples at Ts={} s. Scope outputs:\n{}",
                model.steps,
                model.sample_time,
                description.join("\n")
            )
        }
        Job::SaveWorkspace(path) => {
            files::save_array_workspace(&path, &engine.workspace(), &engine.cancellation())?;
            format!("Saved workspace {}", crate::path_label::display(&path))
        }
        Job::SaveWorkspaceV73(path) => {
            files::save_array_workspace_v73(&path, &engine.workspace(), &engine.cancellation())?;
            format!("Saved v7.3 workspace {}", crate::path_label::display(&path))
        }
        Job::ImportCsv(request) => super::csv_import::perform(engine, *request)?,
        Job::ExportVariable(path, name) => {
            files::write_array_csv(
                &path,
                &engine.array(&name).ok_or("Select a workspace variable")?,
                &engine.cancellation(),
            )?;
            format!("Saved {}", crate::path_label::display(&path))
        }
        Job::OpenFigure(path) => {
            let ids = engine.figure_ids();
            if ids.len() >= 16 {
                return Err("At most 16 figures can be open; close unused figures".into());
            }
            let figure = files::load_figure(&path)?;
            let id = (1..=17).find(|id| !ids.contains(id)).unwrap();
            engine.select_figure(id)?;
            engine.figure = Some(figure);
            let details = if path
                .extension()
                .is_some_and(|s| s.eq_ignore_ascii_case("fig"))
            {
                " (MATLAB 2-D data/styles imported; fonts and unspecified marker sizes use MatRust defaults; normalized axes positions are retained; callbacks are not executed)"
            } else {
                ""
            };
            format!(
                "Opened figure {}{details}",
                crate::path_label::display(&path)
            )
        }
        Job::NewLayout(grid) => {
            let figure = Figure::tiled_layout(grid)?;
            let ids = engine.figure_ids();
            let id = (1..=17).find(|n| !ids.contains(n)).unwrap();
            engine.select_figure(id)?;
            engine.figure = Some(figure);
            format!("Created tiled layout in figure {id}")
        }
        Job::PlotData(request) => {
            let id = request.create(engine)?;
            format!("Created workspace plot in figure {id}")
        }
        Job::EditFigure(edit) | Job::ViewFigure(edit) => {
            if engine.current_figure != edit.id
                || !engine
                    .figure
                    .as_ref()
                    .is_some_and(|f| f.same_snapshot(&edit.original))
            {
                return Err("Figure changed while its properties were open. Cancel and reopen Properties to edit the current figure.".into());
            }
            edit.updated.validate()?;
            if edit.view_before.is_some() {
                with_history_view(&edit.original, edit.view_before)?;
            }
            engine.figure = Some(edit.updated);
            format!("Updated properties of figure {}", edit.id)
        }
        Job::SaveFigure(path, view) => {
            let figure = figure_snapshot(engine, view)?;
            if path
                .extension()
                .is_some_and(|s| s.eq_ignore_ascii_case("fig"))
            {
                files::save_matlab_figure_with_theme(
                    &path,
                    &figure,
                    engine.graphics_theme_context(),
                    &engine.cancellation(),
                )?;
            } else {
                files::save_figure(&path, &figure)?;
            }
            format!(
                "Saved editable figure {}",
                crate::path_label::display(&path)
            )
        }
        Job::SaveFigureById(id, path, view) => {
            let mut figure = engine
                .figures()
                .find(|(n, _)| *n == id)
                .map(|(_, f)| f.clone())
                .ok_or("Figure no longer exists")?;
            if let Some([a, b, c, d]) = view {
                figure.xlim = Some([a, b]);
                figure.ylim = Some([c, d]);
            }
            figure.validate()?;
            if path
                .extension()
                .is_some_and(|s| s.eq_ignore_ascii_case("fig"))
            {
                files::save_matlab_figure_with_theme(
                    &path,
                    &figure,
                    engine.graphics_theme_context(),
                    &engine.cancellation(),
                )?;
            } else {
                files::save_figure(&path, &figure)?;
            }
            format!(
                "Saved editable figure {}",
                crate::path_label::display(&path)
            )
        }
        #[cfg(test)]
        Job::ExportFigure(path, view) => {
            let figure = figure_snapshot(engine, view)?;
            if path
                .extension()
                .and_then(|s| s.to_str())
                .is_some_and(|s| s.eq_ignore_ascii_case("csv"))
            {
                files::write_figure_csv(&path, &figure)?;
            } else {
                matrust::export::export_figure_in_theme_with_cancel(
                    &figure,
                    &path,
                    [1440, 840],
                    engine.graphics_theme_context(),
                    &engine.cancellation(),
                )?;
            }
            format!("Exported {}", crate::path_label::display(&path))
        }
        Job::ExportSnapshot(request) => {
            if engine
                .cancellation()
                .load(std::sync::atomic::Ordering::Relaxed)
            {
                return Err("Cancelled".into());
            }
            if request
                .path
                .extension()
                .is_some_and(|s| s.eq_ignore_ascii_case("csv"))
            {
                files::write_figure_csv_with_cancel(
                    &request.path,
                    &request.figure,
                    &engine.cancellation(),
                )?;
            } else if request
                .path
                .extension()
                .is_some_and(|s| s.eq_ignore_ascii_case("m"))
            {
                files::write_matlab_plot_reconstruction_with_theme(
                    &request.path,
                    &request.figure,
                    engine.graphics_theme_context(),
                    &engine.cancellation(),
                )?;
            } else if let Some(canvas) = request.vector {
                matrust::export::export_vector_in_theme_with_cancel(
                    &request.figure,
                    &request.path,
                    canvas,
                    engine.graphics_theme_context(),
                    &engine.cancellation(),
                )?;
            } else {
                matrust::export::export_figure_in_theme_with_cancel(
                    &request.figure,
                    &request.path,
                    [request.width, request.height],
                    engine.graphics_theme_context(),
                    &engine.cancellation(),
                )?;
            }
            format!("Exported {}", crate::path_label::display(&request.path))
        }
        Job::ExportPreview(request) => {
            super::image_export::save(&request, &engine.cancellation())?;
            format!(
                "Saved preview to {}",
                crate::path_label::display(&request.path)
            )
        }
    };
    Ok(Execution {
        output: vec![message],
        clear_console: false,
    })
}

#[cfg(test)]
mod figure_tests {
    use super::*;
    #[test]
    fn csv_import_passes_cancellation_before_parsing_and_preserves_workspace() {
        use std::sync::atomic::Ordering;
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("data.csv");
        std::fs::write(&path, "not a numeric CSV").unwrap();
        let mut engine = Engine::default();
        let original = Matrix::scalar(42.);
        engine.variables.insert("data".into(), original.clone());
        engine.cancellation().store(true, Ordering::Relaxed);
        let error = perform(
            &mut engine,
            Job::ImportCsv(Box::new(super::super::csv_import::Request {
                path,
                name: "data".into(),
                expected: Some(original.clone().into()),
            })),
        )
        .unwrap_err();
        assert!(error.contains("cancelled"));
        assert_eq!(engine.variables.len(), 1);
        assert_eq!(
            engine.variables["data"].data().as_ptr(),
            original.data().as_ptr()
        );
    }
    #[test]
    fn selected_mat_import_is_transactional_and_preflights_existing_workspace() {
        use std::collections::BTreeSet;
        let mut engine = Engine::default();
        engine
            .variables
            .insert("compact".into(), Matrix::scalar(99.));
        let original = engine.variables["compact"].clone();
        let catalog = files::inspect_workspace(
            std::path::Path::new("tests/fixtures/mat/mat-import-v7.mat"),
            &engine.cancellation(),
        )
        .unwrap();
        assert!(
            perform(
                &mut engine,
                Job::LoadSelectedWorkspace(
                    Box::new(catalog.clone()),
                    // A missing name fails the whole selection (too_precise loads as int64 now).
                    BTreeSet::from(["compact".into(), "no_such_variable".into()])
                )
            )
            .is_err()
        );
        assert_eq!(engine.variables["compact"].data(), original.data());
        perform(
            &mut engine,
            Job::LoadSelectedWorkspace(
                Box::new(catalog.clone()),
                BTreeSet::from(["compact".into()]),
            ),
        )
        .unwrap();
        assert_eq!(engine.variables["compact"].data(), &[7.]);
        engine.variables.clear();
        let large = Matrix::filled(16 * 1024 * 1024, 1, 0.).unwrap();
        engine.variables.insert("d".into(), large.clone());
        engine.variables.insert("keep".into(), large);
        assert!(
            perform(
                &mut engine,
                Job::LoadSelectedWorkspace(
                    Box::new(catalog.clone()),
                    BTreeSet::from(["compact".into()])
                )
            )
            .unwrap_err()
            .contains("existing variables")
        );
        assert_eq!(engine.variables.len(), 2);
        perform(
            &mut engine,
            Job::LoadSelectedWorkspace(Box::new(catalog), BTreeSet::from(["d".into()])),
        )
        .unwrap();
        assert_eq!(engine.variables["d"].len(), 6);
        assert_eq!(engine.variables["keep"].len(), 16 * 1024 * 1024);
    }
    #[test]
    fn visibility_edits_preserve_manual_zoom_and_saved_hidden_data() {
        let mut engine = Engine::default();
        engine
            .execute("plot([1 2 3],[1 2 3], [1 2 3],[100 200 300]);")
            .unwrap();
        let original = engine.figure.clone().unwrap();
        let mut updated = original.clone();
        updated.series[1].visible = false;
        let mut edit = FigureEdit {
            view_before: None,
            id: 1,
            original: original.clone(),
            updated: updated.clone(),
        };
        edit.preserve_manual_view(Some([1.25, 2.5, 0., 50.]), [false, true]);
        assert_eq!(edit.updated.xlim, Some([1.25, 2.5]));
        assert!(edit.updated.ylim.is_none());
        perform(&mut engine, Job::EditFigure(Box::new(edit))).unwrap();
        assert_eq!(
            engine.figure.as_ref().unwrap().bounds(),
            ([1.25, 2.5], [1., 3.])
        );
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("visibility.mfig");
        perform(&mut engine, Job::SaveFigure(path.clone(), None)).unwrap();
        let f = files::load_figure(&path).unwrap();
        assert!(!f.series[1].visible);
        assert_eq!(f.series[1].points, original.series[1].points);
        let mut edit = FigureEdit {
            view_before: None,
            id: 1,
            original,
            updated,
        };
        edit.updated.xlim = Some([1., 10.]);
        edit.preserve_manual_view(Some([1.25, 2.5, 0., 50.]), [false, true]);
        assert_eq!(edit.updated.xlim, Some([1., 10.]));
    }
    #[test]
    fn position_edit_preserves_manual_zoom_rejects_stale_edits_and_saves() {
        let mut engine = Engine::default();
        engine
            .execute("plot(1:100);title('Keep samples');")
            .unwrap();
        let original = engine.figure.clone().unwrap();
        let mut updated = original.clone();
        updated.reposition_axes([0.15, 0.2, 0.7, 0.6]).unwrap();
        let mut edit = FigureEdit {
            view_before: None,
            id: 1,
            original: original.clone(),
            updated: updated.clone(),
        };
        edit.preserve_manual_view(Some([20., 40., 10., 60.]), [false, true]);
        assert_eq!(edit.updated.xlim, Some([20., 40.]));
        assert!(edit.updated.ylim.is_none());
        perform(&mut engine, Job::EditFigure(Box::new(edit))).unwrap();
        let current = engine.figure.clone().unwrap();
        assert!((original.series[0].points).same_storage(&current.series[0].points));
        assert!(
            perform(
                &mut engine,
                Job::EditFigure(Box::new(FigureEdit {
                    view_before: None,
                    id: 1,
                    original,
                    updated
                }))
            )
            .is_err()
        );
        assert!(current.same_snapshot(engine.figure.as_ref().unwrap()));
        let dir = tempfile::tempdir().unwrap();
        for extension in ["fig", "mfig"] {
            let path = dir.path().join(format!("position.{extension}"));
            perform(&mut engine, Job::SaveFigure(path.clone(), None)).unwrap();
            let loaded = files::load_figure(&path).unwrap();
            assert_eq!(loaded.svg(), current.svg());
            assert_eq!(loaded.xlim, Some([20., 40.]));
        }
        let mut explicit = current.clone();
        explicit.reposition_axes([0.1, 0.1, 0.8, 0.8]).unwrap();
        explicit.xlim = Some([10., 50.]);
        let mut edit = FigureEdit {
            view_before: None,
            id: 1,
            original: current,
            updated: explicit,
        };
        edit.preserve_manual_view(Some([20., 40., 10., 60.]), [false, false]);
        assert_eq!(edit.updated.xlim, Some([10., 50.]));
        assert_eq!(edit.updated.ylim, Some([10., 60.]));
    }
    #[test]
    fn property_edits_preserve_samples_and_roundtrip_metadata() {
        use matrust::plot::{AxisScale, LinePattern};
        let mut engine = Engine::default();
        engine
            .execute("plot([1 10 100],[1 2 3],[1 10 100],[3 2 1]);")
            .unwrap();
        let original = engine.figure.clone().unwrap();
        let mut updated = original.clone();
        updated.title = "Gain λ <reference>".into();
        updated.xlabel = "Frequency".into();
        updated.ylabel = "Gain".into();
        updated.xscale = AxisScale::Log;
        updated.xlim = Some([1., 100.]);
        updated.ylim = Some([0., 4.]);
        updated.legend = true;
        updated.legend_location = matrust::plot::LegendLocation::Southwest;
        updated.xgrid = false;
        updated.ygrid = false;
        updated.hold = true;
        updated.series.swap(0, 1);
        updated.series[0].label = "Measured".into();
        updated.series[0].color = [19, 127, 211];
        updated.series[0].explicit_color = true;
        updated.series[0].pattern = LinePattern::DashDot;
        updated.series[0].markers = true;
        updated.series[0].marker_filled = true;
        perform(
            &mut engine,
            Job::EditFigure(Box::new(FigureEdit {
                view_before: None,
                id: 1,
                original: original.clone(),
                updated,
            })),
        )
        .unwrap();
        let result = engine.figure.as_ref().unwrap();
        assert!((result.series[0].points).same_storage(&original.series[1].points));
        assert!((result.series[1].points).same_storage(&original.series[0].points));
        assert_eq!(original.title, "");
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("edited.mfig");
        perform(&mut engine, Job::SaveFigure(path.clone(), None)).unwrap();
        let loaded = files::load_figure(&path).unwrap();
        assert_eq!(loaded.title, "Gain λ <reference>");
        assert_eq!(loaded.xlabel, "Frequency");
        assert_eq!(loaded.ylabel, "Gain");
        assert_eq!(loaded.xscale, AxisScale::Log);
        assert_eq!(loaded.xlim, Some([1., 100.]));
        assert_eq!(loaded.ylim, Some([0., 4.]));
        assert!(loaded.legend && loaded.hold && !loaded.xgrid && !loaded.ygrid);
        assert_eq!(
            loaded.legend_location,
            matrust::plot::LegendLocation::Southwest
        );
        assert_eq!(loaded.series[0].label, "Measured");
        assert_eq!(loaded.series[0].color, [19, 127, 211]);
        assert_eq!(loaded.series[0].pattern, LinePattern::DashDot);
        assert!(loaded.series[0].markers && loaded.series[0].marker_filled);
        assert_eq!(loaded.series[0].points, original.series[1].points);
        let svg = loaded.svg();
        assert!(svg.contains("&lt;reference&gt;"));
        assert!(svg.contains("Measured"));
        let png = dir.path().join("edited.png");
        perform(&mut engine, Job::ExportFigure(png.clone(), None)).unwrap();
        let image = image::open(png).unwrap().to_rgb8();
        assert!(image.pixels().filter(|p| p.0 == [19, 127, 211]).count() > 25);
        let original = engine.figure.clone().unwrap();
        let mut updated = original.clone();
        updated.series.remove(0);
        perform(
            &mut engine,
            Job::EditFigure(Box::new(FigureEdit {
                view_before: None,
                id: 1,
                original,
                updated,
            })),
        )
        .unwrap();
        assert_eq!(engine.figure.as_ref().unwrap().series.len(), 1);
        engine.execute("figure(2);plot(1,2);figure(1);").unwrap();
        assert_eq!(engine.figure.as_ref().unwrap().title, "Gain λ <reference>");
    }
    #[test]
    fn stale_and_invalid_property_edits_leave_current_figure_untouched() {
        let mut engine = Engine::default();
        engine.execute("plot([1 2],[3 4]);").unwrap();
        let original = engine.figure.clone().unwrap();
        for (id, limits) in [
            (2, Some([1., 2.])),
            (1, Some([2., 1.])),
            (1, Some([0., f64::INFINITY])),
        ] {
            let mut updated = original.clone();
            updated.xlim = limits;
            assert!(
                perform(
                    &mut engine,
                    Job::EditFigure(Box::new(FigureEdit {
                        view_before: None,
                        id,
                        original: original.clone(),
                        updated
                    }))
                )
                .is_err()
            );
            assert!(engine.figure.as_ref().unwrap().same_snapshot(&original));
        }
        engine.execute("title('Newer title');").unwrap();
        let newer = engine.figure.clone().unwrap();
        assert!(
            perform(
                &mut engine,
                Job::EditFigure(Box::new(FigureEdit {
                    view_before: None,
                    id: 1,
                    original: original.clone(),
                    updated: original.clone()
                }))
            )
            .is_err()
        );
        assert!(engine.figure.as_ref().unwrap().same_snapshot(&newer));
        engine.execute("plot([1 2],[3 4]);").unwrap();
        assert!(!engine.figure.as_ref().unwrap().same_snapshot(&original));
    }
    #[test]
    fn matlab_figure_import_is_transactional_and_save_rejects_unsupported_legend() {
        let mut engine = Engine::default();
        perform(
            &mut engine,
            Job::OpenFigure(PathBuf::from("tests/fixtures/fig/matlab-lines.fig")),
        )
        .unwrap();
        let points = engine.figure.as_ref().unwrap().series[0].points.clone();
        assert!(
            perform(
                &mut engine,
                Job::OpenFigure(PathBuf::from("tests/fixtures/fig/matlab-surface.fig"))
            )
            .is_err()
        );
        assert!((points).same_storage(&engine.figure.as_ref().unwrap().series[0].points));
        let dir = tempfile::tempdir().unwrap();
        let supported = engine.figure.clone().unwrap();
        assert!(supported.legend && supported.legend_location.is_default());
        assert!(!supported.legend_state.as_ref().unwrap().auto_update);
        assert!(files::matlab_figure_export_restriction(&supported).is_none());
        let supported_path = dir.path().join("supported.fig");
        perform(&mut engine, Job::SaveFigure(supported_path.clone(), None)).unwrap();
        let mut reopened = Engine::default();
        perform(&mut reopened, Job::OpenFigure(supported_path)).unwrap();
        let restored = reopened.figure.as_ref().unwrap();
        assert!(restored.legend && restored.legend_location.is_default());
        assert!(!restored.legend_state.as_ref().unwrap().auto_update);
        assert_eq!(restored.series.len(), supported.series.len());
        assert_eq!(
            restored
                .legend_members()
                .map(|(i, _)| i)
                .collect::<Vec<_>>(),
            supported
                .legend_members()
                .map(|(i, _)| i)
                .collect::<Vec<_>>()
        );
        for (before, after) in supported.series.iter().zip(&restored.series) {
            assert_eq!(
                (before.kind, &before.label, before.visible),
                (after.kind, &after.label, after.visible)
            );
            assert_eq!(before.points.len(), after.points.len());
            for (before, after) in before.points.iter().zip(after.points.iter()) {
                assert_eq!(before.map(f64::to_bits), after.map(f64::to_bits));
            }
        }
        engine.execute("legend('Location','southwest');").unwrap();
        let unsupported = engine.figure.clone().unwrap();
        assert_eq!(
            unsupported.legend_location,
            matrust::plot::LegendLocation::Southwest
        );
        assert!(
            files::matlab_figure_export_restriction(&unsupported)
                .unwrap()
                .contains("visible northeast")
        );
        let path = dir.path().join("test.fig");
        std::fs::write(&path, b"existing").unwrap();
        assert!(perform(&mut engine, Job::SaveFigure(path.clone(), None)).is_err());
        assert_eq!(std::fs::read(path).unwrap(), b"existing");
        assert!(engine.figure.as_ref().unwrap().same_snapshot(&unsupported));
        assert!(points.same_storage(&engine.figure.as_ref().unwrap().series[0].points));
    }
    #[test]
    fn logarithmic_zoom_saves_data_limits_and_rejects_invalid_export() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("log.mfig");
        let mut engine = Engine::default();
        engine.execute("loglog([1 10 100],[1 10 100]);").unwrap();
        perform(
            &mut engine,
            Job::SaveFigure(path.clone(), Some([2., 20., 3., 30.])),
        )
        .unwrap();
        let f = files::load_figure(&path).unwrap();
        assert_eq!(f.xlim, Some([2., 20.]));
        assert_eq!(f.ylim, Some([3., 30.]));
        assert_eq!(f.xscale, matrust::plot::AxisScale::Log);
        let bytes = std::fs::read(&path).unwrap();
        assert!(
            perform(
                &mut engine,
                Job::SaveFigure(path.clone(), Some([-1., 20., 3., 30.]))
            )
            .is_err()
        );
        assert_eq!(std::fs::read(path).unwrap(), bytes);
    }
    #[test]
    fn saved_view_preserves_zoom_and_reopens_all_series() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("view.mfig");
        let mut engine = Engine::default();
        engine
            .execute("plot([0 1 2],[0 2 0],[0 1 2],[1 0 1]);legend('A','B');")
            .unwrap();
        perform(
            &mut engine,
            Job::SaveFigure(path.clone(), Some([0.5, 1.5, 0., 1.])),
        )
        .unwrap();
        assert!(engine.figure.as_ref().unwrap().xlim.is_none());
        engine.execute("clf;").unwrap();
        perform(&mut engine, Job::OpenFigure(path)).unwrap();
        let f = engine.figure.unwrap();
        assert_eq!(f.xlim, Some([0.5, 1.5]));
        assert_eq!(f.series.len(), 2);
        assert!(f.legend);
    }
}

#[cfg(test)]
mod typed_scalar_job_tests {
    use super::*;
    use matrust::{Complex64, Scalar};

    #[test]
    fn typed_cell_worker_rejects_complex_without_mutation_then_preserves_real_aliases() {
        let mut engine = Engine::default();
        engine.execute("A=[1 2];B=A;L=logical([0 1]);").unwrap();
        let before = engine.workspace();
        for imaginary in [0., -0., 3.] {
            let error = perform(
                &mut engine,
                Job::SetCell(
                    "A".into(),
                    0,
                    0,
                    Scalar::Complex(Complex64::new(9., imaginary)),
                ),
            )
            .unwrap_err();
            assert!(error.contains("requires a real or logical"));
            for (name, value) in &before {
                assert!(value.same_snapshot(&engine.array(name).unwrap()));
            }
        }
        perform(
            &mut engine,
            Job::SetCell("A".into(), 0, 0, Scalar::Real(9.)),
        )
        .unwrap();
        assert_eq!(engine.variables["A"].data(), &[9., 2.]);
        assert_eq!(engine.variables["B"].data(), &[1., 2.]);
        assert!(before["B"].same_snapshot(&engine.array("B").unwrap()));
        perform(
            &mut engine,
            Job::SetCell("L".into(), 0, 0, Scalar::Logical(true)),
        )
        .unwrap();
        assert_eq!(engine.logical_variables["L"].data(), &[1, 1]);
        let after = engine.array("A").unwrap();
        engine
            .cancellation()
            .store(true, std::sync::atomic::Ordering::Relaxed);
        assert!(
            perform(
                &mut engine,
                Job::SetCell("A".into(), 0, 0, Scalar::Real(10.))
            )
            .is_err()
        );
        assert!(after.same_snapshot(&engine.array("A").unwrap()));
    }
}

#[cfg(test)]
mod complex_value_job_tests {
    use super::*;
    use matrust::{Complex64, Scalar};

    #[test]
    fn complex_cell_and_workspace_jobs_preserve_type_aliases_and_failure_state() {
        use super::super::workspace_edit::{self, Action, Request};
        let mut engine = Engine::default();
        engine
            .execute("Z=complex([1 2],[-0 3]);Alias=Z;R=5;")
            .unwrap();
        let original = engine.array("Z").unwrap();
        perform(
            &mut engine,
            Job::SetCell("Z".into(), 0, 1, Scalar::Complex(Complex64::new(9., -8.))),
        )
        .unwrap();
        assert_eq!(
            engine.complex_variables["Z"].get(0, 1).unwrap(),
            Complex64::new(9., -8.)
        );
        assert!(engine.array("Alias").unwrap().same_snapshot(&original));
        let edited = engine.array("Z").unwrap();
        assert_eq!(
            edited.scalar(0, 0).unwrap().into_complex().im.to_bits(),
            (-0.0_f64).to_bits()
        );
        workspace_edit::perform(
            &mut engine,
            Request {
                targets: std::collections::BTreeMap::from([("Z".into(), edited.clone())]),
                action: Action::Rename("Renamed".into()),
            },
        )
        .unwrap();
        assert!(engine.array("Z").is_none());
        assert!(edited.same_snapshot(&engine.array("Renamed").unwrap()));
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("preserve.mat");
        std::fs::write(&path, b"existing destination").unwrap();
        perform(&mut engine, Job::SaveWorkspaceV73(path.clone())).unwrap();
        let v73 = files::inspect_workspace(&path, &engine.cancellation()).unwrap();
        let v73_loaded = files::load_selected_arrays(
            &v73,
            &["Renamed".to_owned()].into_iter().collect(),
            &engine.cancellation(),
        )
        .unwrap();
        assert_eq!(
            v73_loaded["Renamed"].as_complex().unwrap().data()[1],
            Complex64::new(9., -8.)
        );
        assert!(edited.same_snapshot(&engine.array("Renamed").unwrap()));
        perform(&mut engine, Job::SaveWorkspace(path.clone())).unwrap();
        let catalog = files::inspect_workspace(&path, &engine.cancellation()).unwrap();
        let loaded = files::load_selected_arrays(
            &catalog,
            &["Renamed".to_owned()].into_iter().collect(),
            &engine.cancellation(),
        )
        .unwrap();
        let restored = loaded["Renamed"].as_complex().unwrap();
        assert_eq!(restored.data()[0].im.to_bits(), (-0.0_f64).to_bits());
        assert_eq!(restored.data()[1], Complex64::new(9., -8.));
        assert!(edited.same_snapshot(&engine.array("Renamed").unwrap()));
        let cancel = engine.cancellation();
        cancel.store(true, std::sync::atomic::Ordering::Relaxed);
        assert!(
            perform(
                &mut engine,
                Job::SetCell("Renamed".into(), 0, 0, Scalar::Real(77.))
            )
            .unwrap_err()
            .contains("cancelled")
        );
        assert!(edited.same_snapshot(&engine.array("Renamed").unwrap()));
        cancel.store(false, std::sync::atomic::Ordering::Relaxed);
        perform(
            &mut engine,
            Job::SetCell("Renamed".into(), 0, 0, Scalar::Logical(true)),
        )
        .unwrap();
        assert_eq!(
            engine.complex_variables["Renamed"].get(0, 0).unwrap(),
            Complex64::new(1., 0.)
        );
        let removed = engine.array("Renamed").unwrap();
        workspace_edit::perform(
            &mut engine,
            Request {
                targets: std::collections::BTreeMap::from([("Renamed".into(), removed)]),
                action: Action::Remove { all: false },
            },
        )
        .unwrap();
        assert!(engine.array("Renamed").is_none());
        assert!(original.same_snapshot(&engine.array("Alias").unwrap()));
        assert_eq!(engine.variables["R"].data(), &[5.]);
    }
}

#[cfg(test)]
#[path = "worker_repaint_tests.rs"]
mod worker_repaint_tests;

#[cfg(test)]
#[path = "bar_outline_worker_tests.rs"]
mod bar_outline_worker_tests;
