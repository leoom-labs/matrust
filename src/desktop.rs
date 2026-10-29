#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]
#[path = "desktop/cell_replace_job.rs"]
mod cell_replace_job;
#[cfg(test)]
#[path = "desktop/cell_replace_tests.rs"]
mod cell_replace_tests;
#[path = "desktop/cell_replace_ui.rs"]
mod cell_replace_ui;
#[path = "desktop/chrome.rs"]
mod chrome;
#[path = "desktop/command_window.rs"]
mod command_window;
#[cfg(test)]
#[path = "desktop/container_workspace_tests.rs"]
mod container_workspace_tests;
#[path = "desktop/csv_import.rs"]
mod csv_import;
#[path = "desktop/designer_apply.rs"]
mod designer_apply;
#[path = "desktop/designer_save.rs"]
mod designer_save;
#[path = "desktop/detached_figures.rs"]
mod detached_figures;
#[path = "desktop/document_save.rs"]
mod document_save;
#[path = "desktop/editor.rs"]
mod editor;
#[path = "desktop/editor_columns.rs"]
mod editor_columns;
#[path = "desktop/editor_window.rs"]
mod editor_window;
#[path = "desktop/eps_capture.rs"]
mod eps_capture;
#[path = "desktop/eps_load.rs"]
mod eps_load;
#[path = "desktop/eps_preview.rs"]
mod eps_preview;
#[path = "desktop/export_batch.rs"]
mod export_batch;
#[path = "desktop/figure_copy.rs"]
mod figure_copy;
#[path = "desktop/figure_editor.rs"]
mod figure_editor;
#[path = "desktop/figure_export.rs"]
mod figure_export;
#[path = "desktop/figure_files.rs"]
mod figure_files;
#[path = "desktop/figure_geometry.rs"]
mod figure_geometry;
#[path = "desktop/figure_recovery.rs"]
mod figure_recovery;
#[path = "desktop/figure_selector.rs"]
mod figure_selector;
#[path = "desktop/figure_target.rs"]
mod figure_target;
#[path = "desktop/figure_view.rs"]
mod figure_view;
#[path = "desktop/fir_designer.rs"]
mod fir_designer;
#[path = "desktop/focus_areas.rs"]
mod focus_areas;
#[path = "desktop/folder_browser.rs"]
mod folder_browser;
#[path = "desktop/folder_navigation.rs"]
mod folder_navigation;
#[path = "desktop/folder_scan.rs"]
mod folder_scan;
#[path = "desktop/folder_search_ui.rs"]
mod folder_search_ui;
#[path = "desktop/global_search.rs"]
mod global_search;
#[cfg(test)]
#[path = "desktop/headless.rs"]
mod headless;
#[path = "desktop/iir_designer.rs"]
mod iir_designer;
#[path = "desktop/image_export.rs"]
mod image_export;
#[path = "desktop/image_load.rs"]
mod image_load;
#[path = "desktop/image_pages.rs"]
mod image_pages;
#[path = "desktop/image_view.rs"]
mod image_view;
#[path = "desktop/input.rs"]
mod input;
#[path = "desktop/layout.rs"]
mod layout;
#[path = "desktop/model_new.rs"]
mod model_new;
#[path = "desktop/model_read.rs"]
mod model_read;
#[path = "desktop/model_write.rs"]
mod model_write;
#[path = "desktop/named_layout.rs"]
mod named_layout;
#[path = "desktop/navigation.rs"]
mod navigation;
#[path = "desktop/numeric_editor.rs"]
mod numeric_editor;
#[path = "desktop/open_files.rs"]
mod open_files;
#[path = "desktop/open_queue.rs"]
mod open_queue;
#[path = "desktop/original_copy.rs"]
mod original_copy;
#[path = "desktop/original_copy_ui.rs"]
mod original_copy_ui;
#[path = "desktop/panels.rs"]
mod panels;
#[path = "desktop/path_label.rs"]
mod path_label;
#[path = "desktop/pdf_load.rs"]
mod pdf_load;
#[path = "desktop/pdf_protocol.rs"]
mod pdf_protocol;
#[path = "desktop/plot_data.rs"]
mod plot_data;
#[path = "desktop/plot_request.rs"]
mod plot_request;
#[path = "desktop/preview_recovery.rs"]
mod preview_recovery;
#[path = "desktop/recovery.rs"]
mod recovery;
#[path = "desktop/sample_inspector.rs"]
mod sample_inspector;
#[path = "desktop/screenshot_options.rs"]
mod screenshot_options;
#[path = "desktop/script_actions.rs"]
mod script_actions;
#[path = "desktop/script_files.rs"]
mod script_files;
#[path = "desktop/script_focus.rs"]
mod script_focus;
#[path = "desktop/script_io.rs"]
mod script_io;
#[path = "desktop/script_reload.rs"]
mod script_reload;
#[path = "desktop/script_run.rs"]
mod script_run;
#[path = "desktop/shell.rs"]
mod shell;
#[path = "desktop/single_export.rs"]
mod single_export;
#[path = "desktop/single_export_ui.rs"]
mod single_export_ui;
#[path = "desktop/subplot_view.rs"]
mod subplot_view;
#[path = "desktop/svg_load.rs"]
mod svg_load;
#[path = "desktop/theme.rs"]
mod theme;
#[path = "desktop/tiff_load.rs"]
mod tiff_load;
#[path = "desktop/tokens.rs"]
mod tokens;
#[cfg(test)]
#[path = "desktop/tokens_tests.rs"]
mod tokens_tests;
#[path = "desktop/toolkits.rs"]
mod toolkits;
#[path = "desktop/toolstrip.rs"]
mod toolstrip;
#[path = "desktop/typed_inspector.rs"]
mod typed_inspector;
#[cfg(test)]
#[path = "desktop/typed_inspector_tests.rs"]
mod typed_inspector_tests;
#[path = "desktop/variable_edit_job.rs"]
mod variable_edit_job;
#[cfg(test)]
#[path = "desktop/variable_edit_ui_tests.rs"]
mod variable_edit_ui_tests;
#[path = "desktop/worker.rs"]
mod worker;
#[path = "desktop/workspace_dock.rs"]
mod workspace_dock;
#[path = "desktop/workspace_edit.rs"]
mod workspace_edit;
#[path = "desktop/workspace_import.rs"]
mod workspace_import;
#[path = "desktop/workspace_tools.rs"]
mod workspace_tools;
use eframe::egui::{self, Color32, RichText};
use egui_plot::PlotPoint;
use matrust::session::{self, FolderEntry, History, Session};
use matrust::{document::Document, plot::Figure};
use std::{
    collections::BTreeMap,
    path::PathBuf,
    sync::{Arc, atomic::Ordering},
    time::{Duration, Instant},
};
use worker::{Job, Worker};

use tokens::FIGURE_ACCENT as ACCENT;
const DEMO: &str = include_str!("../examples/signals.m");
/// Smallest Command Window card: header, a few output lines and the 34 px input.
const CONSOLE_MIN: f32 = 164.;
/// Output lines a docked Command Window keeps visible.
const CONSOLE_MIN_LINES: f32 = 6.;

/// Docked Command Window height that shows `CONSOLE_MIN_LINES` output lines
/// above the input: card frame, header, the history's M/S gaps and the field.
fn console_six_lines(ctx: &egui::Context) -> f32 {
    let lines = CONSOLE_MIN_LINES * tokens::CONSOLE * tokens::CONSOLE_LINE;
    let frame = tokens::card(ctx).inner_margin(1).total_margin().sum().y;
    let body = tokens::M + lines + tokens::S + tokens::INPUT_HEIGHT + tokens::M;
    (panels::header_height(ctx) + body + frame + tokens::XS).ceil()
}
type ScriptFilter = (&'static str, &'static [&'static str]);
/// Script Save dialog filters and suggested name. New scripts default to
/// native .mr; an existing .m keeps its filter first, so the dialog's default
/// extension never renames the user's file.
fn script_save_choices(current: Option<&std::path::Path>) -> ([ScriptFilter; 2], String) {
    const NATIVE: ScriptFilter = ("MatRust script (*.mr)", &["mr"]);
    const MATLAB: ScriptFilter = ("MATLAB script (*.m)", &["m"]);
    let filters = if current
        .and_then(std::path::Path::extension)
        .is_some_and(|e| e.eq_ignore_ascii_case("m"))
    {
        [MATLAB, NATIVE]
    } else {
        [NATIVE, MATLAB]
    };
    let name = current.and_then(std::path::Path::file_name).map_or_else(
        || "untitled.mr".into(),
        |n| n.to_string_lossy().into_owned(),
    );
    (filters, name)
}
struct RunOrigin {
    document: u64,
    title: String,
    source: Arc<str>,
}
struct EditorIssue {
    origin: RunOrigin,
    diagnostic: matrust::syntax::Diagnostic,
    external: Option<PathBuf>,
}
struct Workspace {
    all_figures: BTreeMap<u32, Figure>,
    recovery_manifest: Option<session::figures::FigureManifest>,
    workspace_manifest: Option<session::workspace::Manifest>,
    recovering_session: bool,
    recovery_blocked: bool,
    quit_discarded_figures: std::collections::BTreeSet<u32>,
    layout: session::DesktopLayout,
    layout_presets: session::LayoutPresets,
    named_layout: named_layout::State,
    compact_figure: bool,
    navigation: navigation::State,
    focus_areas: focus_areas::State,
    document_save: document_save::State,
    documents: Vec<Document>,
    active: usize,
    editor: editor::Editor,
    editor_columns: editor_columns::State,
    editor_window: editor_window::State,
    running_origin: Option<RunOrigin>,
    editor_issue: Option<EditorIssue>,
    pending_close: Option<usize>,
    script_conflict: Option<script_files::Conflict>,
    script_reload: Option<script_reload::Confirmation>,
    script_run: script_run::State,
    global_search: global_search::State,
    /// The engine's `format` setting as of the last finished job.
    display_format: matrust::display_format::DisplayFormat,
    /// `disp` text of the selected variable, kept while value and format hold.
    selected_display: Option<workspace_dock::SelectedDisplay>,
    new_model_requested: bool,
    script_io: script_io::State,
    script_save_queue: Option<std::collections::VecDeque<u64>>,
    script_tab_reveal: Option<u64>,
    ui_dispatch_frame: Option<u64>,
    sample_inspector: sample_inspector::Inspector,
    pending_quit: bool,
    allow_close: bool,
    command: String,
    history: History,
    recent: Vec<PathBuf>,
    open_queue: std::collections::VecDeque<PathBuf>,
    folder: PathBuf,
    folder_text: String,
    folder_navigation: folder_navigation::Navigation,
    entries: Vec<FolderEntry>,
    folder_browser: folder_browser::Browser,
    folder_browse: folder_browser::Browse,
    folder_search: folder_search_ui::State,
    selection: Option<(usize, std::ops::Range<usize>)>,
    suggestions: Vec<String>,
    completion_range: Option<std::ops::Range<usize>>,
    recovery: Option<recovery::Recovery>,
    recovery_dirty: bool,
    recovery_due: Instant,
    console: Vec<String>,
    variables: BTreeMap<String, matrust::Array>,
    figure: Option<Figure>,
    figure_editor: Option<figure_editor::Editor>,
    figure_documents: BTreeMap<u32, worker::FigureStatus>,
    figure_history: BTreeMap<u32, (usize, usize)>,
    figure_history_demo_stage: u8,
    figure_views: BTreeMap<u32, figure_files::View>,
    figure_windows: BTreeMap<u32, figure_recovery::Placement>,
    figure_close: Option<figure_files::CloseFlow>,
    figure_conflict: Option<worker::FigureConflict>,
    properties_demo_pending: bool,
    figure_files_demo_stage: u8,
    figure_conflict_demo: bool,
    figure_reload_demo: bool,
    figure_reopen_demo: bool,
    workspace_import: Option<workspace_import::Import>,
    csv_import: Option<csv_import::Import>,
    root_open: open_files::PickerState,
    root_figure_save: figure_files::RootSave,
    designer_save: designer_save::State,
    csv_import_picker: csv_import::PickerState,
    csv_naming: csv_import::NamingState,
    figure_export: Option<figure_export::Dialog>,
    plot_data: Option<plot_data::Dialog>,
    plot_data_demo_pending: bool,
    pending_image: Option<image_load::Preview>,
    pending_page: Option<image_pages::Page>,
    image_view: Option<image_view::Viewer>,
    preview_restore: Option<preview_recovery::Restore>,
    image_loading: bool,
    eps_preview: eps_preview::Settings,
    eps_capture: Option<eps_capture::Coordinator>,
    image_action: bool,
    original_copy: original_copy_ui::State,
    single_export: single_export_ui::State,
    image_ignore_result: bool,
    export_settings: figure_export::Settings,
    export_demo_pending: bool,
    large_import_demo: bool,
    figure_ids: Vec<u32>,
    current_figure: u32,
    selected: Option<String>,
    workspace_tools: workspace_tools::Tools,
    workspace_mat_save: toolstrip::WorkspaceMatSave,
    editing_variable: Option<String>,
    typed_inspector: typed_inspector::State,
    numeric_editor: numeric_editor::State,
    cell_error: Option<String>,
    cell_job: Option<String>,
    edit_row: usize,
    edit_col: usize,
    edit_value: String,
    first_col: usize,
    subplot_canvas: subplot_view::Canvas,
    detached: detached_figures::Registry,
    detached_capture: Option<detached_figures::capture::Coordinator>,
    layout_grid: [usize; 2],
    plot_points: Vec<Vec<PlotPoint>>,
    plot_axes: [matrust::plot::AxisTransform; 2],
    reset_plot: bool,
    restore_plot_view: bool,
    view_bounds: Option<[f64; 4]>,
    view_auto: [bool; 2],
    worker: Worker,
    toolkits: toolkits::Toolkits,
    model_toolbar: toolkits::model_files::ToolbarState,
    busy: bool,
    status: String,
    chrome: chrome::State,
    // Explicit capture appearance overrides recovery without being persisted.
    screenshot_theme_override: Option<egui::ThemePreference>,
    screenshot_path: Option<std::path::PathBuf>,
    screenshot_requested: bool,
    screenshot_ready_frames: u8,
    screenshot_ready_target: u8,
    screenshot_model_new_pending: bool,
    screenshot_figure_selector_popup: bool,
}
impl Workspace {
    fn new(
        cc: &eframe::CreationContext<'_>,
        iir_capture: Option<screenshot_options::IirDesignerCase>,
    ) -> Self {
        if iir_capture.is_some() {
            Self::with_capture_context(cc.egui_ctx.clone(), false, iir_capture)
        } else {
            Self::with_context(cc.egui_ctx.clone(), false)
        }
    }
    fn with_context(ctx: egui::Context, forced_isolated: bool) -> Self {
        Self::with_capture_context(ctx, forced_isolated, None)
    }
    fn with_capture_context(
        ctx: egui::Context,
        forced_isolated: bool,
        iir_capture: Option<screenshot_options::IirDesignerCase>,
    ) -> Self {
        theme::configure(&ctx);
        if std::env::args().any(|a| a == "--legend-keyboard-demo") {
            ctx.data_mut(|d| d.insert_temp(egui::Id::new("legend_keyboard_capture"), 0_u8));
        }
        let worker = Worker::new(ctx.clone());
        // Keep the established hand-grouped field list; rustfmt never laid it out.
        #[rustfmt::skip]
        let mut workspace = Self {
            folder_browser: folder_browser::Browser::new(ctx.clone()),
            folder_browse: Default::default(),
            folder_search: Default::default(),
            all_figures: Default::default(), recovery_manifest: None, workspace_manifest: None, recovering_session: false,
            recovery_blocked: false, quit_discarded_figures: Default::default(),
            layout_presets: Default::default(), named_layout: Default::default(), layout: session::DesktopLayout::fresh_desktop(), compact_figure: false, navigation: Default::default(), focus_areas: Default::default(), document_save: Default::default(), script_conflict: None, script_reload: None, script_run: Default::default(), global_search: Default::default(), display_format: Default::default(), selected_display: None, new_model_requested: false, script_io: Default::default(), script_save_queue: None, script_tab_reveal: None, ui_dispatch_frame: None, sample_inspector: Default::default(), figure_conflict: None,
            large_import_demo:std::env::args().any(|a|a=="--large-mat-import-demo"),
            workspace_import:None,csv_import:None,csv_import_picker:Default::default(),csv_naming:Default::default(),root_open:Default::default(),root_figure_save:Default::default(), designer_save:Default::default(), pending_image:None, pending_page:None, image_view:None, preview_restore:None, image_loading:false, eps_preview:eps_preview::Settings::configured(forced_isolated), eps_capture:None, image_action:false, original_copy:Default::default(), single_export:Default::default(), image_ignore_result:false,
            figure_export:None, plot_data:None, plot_data_demo_pending:std::env::args().any(|a|a=="--plot-data-demo"), export_settings:Default::default(),
            export_demo_pending:std::env::args().any(|a| matches!(a.as_str(), "--export-demo" | "--batch-export-demo" | "--matlab-export-demo")),
            figure_reopen_demo: std::env::args().any(|a|a=="--figure-reopen-demo"),
            figure_reload_demo: std::env::args().any(|a|a=="--figure-reload-demo"),
            figure_conflict_demo: std::env::args().any(|a|a=="--figure-conflict-demo"),
            figure_files_demo_stage: if std::env::args().any(|a|a=="--figure-files-demo" || a=="--close-all-figures-demo") {1} else {0},
            figure_documents:Default::default(),figure_history:Default::default(),figure_history_demo_stage:if std::env::args().any(|a|matches!(a.as_str(),"--figure-history-demo"|"--figure-history-undone-demo")){1}else{0},figure_views:Default::default(),figure_windows:Default::default(),figure_close:None,restore_plot_view:false,
            documents:vec![Document::new()],active:0,editor:Default::default(),editor_columns:Default::default(),editor_window:Default::default(),running_origin:None,editor_issue:None,pending_close:None,pending_quit:false,allow_close:false, command: String::new(),
            history:History::default(),recent:vec![],open_queue:Default::default(),folder:std::env::current_dir().unwrap_or_else(|_|PathBuf::from(".")),folder_text:String::new(),folder_navigation:Default::default(),entries:vec![],selection:None,suggestions:vec![],completion_range:None,recovery:None,recovery_dirty:false,recovery_due:Instant::now()+Duration::from_secs(2),
            console: chrome::banner(),
            variables: BTreeMap::new(), figure: None, figure_editor:None, properties_demo_pending:std::env::args().any(|a|matches!(a.as_str(),"--properties-demo"|"--series-properties-demo"|"--layout-properties-demo"|"--line-width-properties-demo")), selected: None, workspace_tools: workspace_tools::Tools::new(), workspace_mat_save:Default::default(), cell_error:None,cell_job:None,editing_variable:None,typed_inspector:Default::default(),numeric_editor:Default::default(),edit_row:1,edit_col:1,edit_value:String::new(),first_col:1, subplot_canvas:Default::default(),detached:Default::default(),detached_capture:None, layout_grid:[2,2], plot_points: Vec::new(), worker, toolkits:Default::default(), model_toolbar:Default::default(),
            busy: false, status: "Ready".into(), chrome: Default::default(),
            screenshot_theme_override: None,
            screenshot_path: std::env::args().skip_while(|a|a!="--screenshot").nth(1).map(Into::into),
            screenshot_requested: false,screenshot_ready_frames:0,screenshot_ready_target:3,screenshot_model_new_pending:false,screenshot_figure_selector_popup:false,reset_plot:true,figure_ids:Vec::new(),current_figure:1,view_bounds:None,view_auto:[true;2],plot_axes:Default::default(),
        };
        let initial_folder = workspace.folder.clone();
        let mut restore_path = None;
        let isolated = forced_isolated
            || std::env::args().any(|a| a == "--isolated")
            || (std::env::var_os("MATRUST_SESSION_PATH").is_none()
                && std::env::args().any(|a| {
                    matches!(
                        a.as_str(),
                        "--demo"
                            | "--compact-figure-demo"
                            | "--editor-demo"
                            | "--streaming-demo"
                            | "--indexed-demo"
                            | "--logical-demo"
                            | "--control-flow-demo"
                            | "--functions-demo"
                            | "--function-error-demo"
                            | "--selection-functions-demo"
                            | "--logical-inspector-demo"
                            | "--plots-demo"
                            | "--discrete-plots-demo"
                            | "--plot-properties-demo"
                            | "--marker-styles-demo"
                            | "--legend-keyboard-demo"
                            | "--subplots-demo"
                            | "--tiled-demo"
                            | "--toolstrip-demo"
                            | "--plot-data-demo"
                            | "--preview-image"
                            | "--toolstrip-plots-demo"
                            | "--toolstrip-apps-demo"
                            | "--fig-layout-demo"
                            | "--figure-history-demo"
                            | "--figure-history-undone-demo"
                            | "--layout-properties-demo"
                            | "--line-width-properties-demo"
                            | "--bars-demo"
                            | "--log-demo"
                            | "--fig-demo"
                            | "--visibility-demo"
                            | "--figure-files-demo"
                            | "--close-all-figures-demo"
                            | "--figure-conflict-demo"
                            | "--figure-reload-demo"
                            | "--figure-reopen-demo"
                            | "--mat-import-demo"
                            | "--large-mat-import-demo"
                            | "--properties-demo"
                            | "--export-demo"
                            | "--batch-export-demo"
                            | "--series-properties-demo"
                            | "--coefficients-demo"
                            | "--sos-coefficients-demo"
                            | "--designer-demo"
                            | "--iir-designer-demo"
                            | "--simulation-demo"
                            | "--inspector-demo"
                            | "--screenshot"
                            | "--script-conflict-demo"
                            | "--model-conflict-demo"
                            | "--isolated"
                    )
                }));
        if !isolated {
            let path = std::env::var_os("MATRUST_SESSION_PATH")
                .map(PathBuf::from)
                .unwrap_or_else(|| {
                    std::env::var_os("LOCALAPPDATA")
                        .map(PathBuf::from)
                        .unwrap_or_else(|| workspace.folder.clone())
                        .join("MatRust/session.json")
                });
            let path = std::path::absolute(&path).unwrap_or(path);
            match recovery::Recovery::open(path.clone()) {
                Ok(store) => {
                    let restored = if path.exists() {
                        Session::load(&path).map(Some)
                    } else {
                        Ok(None)
                    };
                    match restored {
                        Ok(Some(saved)) => {
                            workspace.restore_session(saved);
                            restore_path = Some(path.clone());
                            workspace.recovery = Some(store);
                        },
                        Ok(None)=>workspace.recovery=Some(store),
                        Err(e)=>workspace.console.push(format!("Recovery: {e}. Existing recovery file preserved; autosave is disabled in this window.")),
                    }
                }
                Err(e) => workspace.console.push(format!("Recovery: {e}")),
            }
        }
        for doc in &mut workspace.documents {
            if let Some(path) = &doc.path
                && let Ok(absolute) = std::fs::canonicalize(path)
            {
                doc.path = Some(absolute);
            }
        }
        if forced_isolated {
            workspace.folder_text = workspace.folder.display().to_string();
            workspace.refresh_folder();
        } else {
            let restored_folder = std::mem::replace(&mut workspace.folder, initial_folder);
            workspace.change_folder(restored_folder);
        }
        if let Some(path) = restore_path {
            workspace.start_session_recovery(path);
        }
        if std::env::args().any(|arg| arg == "--script-conflict-demo") {
            workspace.preview_script_conflict();
        }
        if std::env::args().any(|arg| arg == "--model-conflict-demo") {
            workspace.toolkits.preview_model_conflict();
        }
        if std::env::args().any(|arg| {
            matches!(
                arg.as_str(),
                "--editor-demo"
                    | "--streaming-demo"
                    | "--plots-demo"
                    | "--bars-demo"
                    | "--log-demo"
                    | "--properties-demo"
                    | "--series-properties-demo"
                    | "--export-demo"
                    | "--batch-export-demo"
                    | "--indexed-demo"
                    | "--logical-demo"
                    | "--logical-inspector-demo"
                    | "--control-flow-demo"
                    | "--functions-demo"
            )
        }) {
            workspace.reveal_editor();
        }
        if std::env::args().any(|arg| arg == "--demo") {
            workspace.run(DEMO.into());
        }
        if std::env::args().any(|arg| arg == "--editor-demo") {
            workspace.documents = vec![Document::new()];
            workspace.documents[0].text="% Digital filtering — editor demonstration\n% Design a lowpass and compare filtered samples.\nfs = 1000;\nt = (0:999)' / fs;\nx = sin(2*pi*30*t) + 0.3*sin(2*pi*220*t);\nb = fir1(48, 100/(fs/2), 'low');\ny = filter(b, 1, x);\nplot(t, x, 'b:', t, y, 'r-');\ntitle('FIR lowpass: original and filtered signal');\nxlabel('Time (seconds)'); ylabel('Amplitude');\nlegend('Input', 'Filtered');\nxlim([0 0.15]);\n\n% A failing line keeps earlier output and the figure.\npeak = max(abs(y))\nbad = filter(b, 0, x);\n".into();
            workspace.documents[0].changed();
            workspace.run_document(None);
        }
        if std::env::args().any(|arg| arg == "--streaming-demo") {
            workspace.documents = vec![
                Document::open(std::path::Path::new("examples/streaming_filter.m")).unwrap_or_else(
                    |_| {
                        let mut doc = Document::new();
                        doc.text = include_str!("../examples/streaming_filter.m").into();
                        doc.changed();
                        doc
                    },
                ),
            ];
            workspace.active = 0;
            workspace.run_document(None);
        }
        if std::env::args().any(|a| a == "--plots-demo" || a == "--bars-demo") {
            if let Ok(document) = Document::open(std::path::Path::new("examples/multiple_plots.m"))
            {
                workspace.documents = vec![document];
                workspace.active = 0;
            }
            let mut script = include_str!("../examples/multiple_plots.m").to_owned();
            if std::env::args().any(|a| a == "--bars-demo") {
                script.push_str("\nfigure(2);\n");
            }
            workspace.run(script);
        }
        let sos_coefficients_demo = std::env::args().any(|a| a == "--sos-coefficients-demo");
        if sos_coefficients_demo || std::env::args().any(|a| a == "--coefficients-demo") {
            workspace.toolkits.model.nodes[3].block = if sos_coefficients_demo {
                matrust::simulation::Block::SosFilter {
                    sections: vec![[1., 0., 0., 1., -0.5, 0.]; 64],
                }
            } else {
                matrust::simulation::Block::Filter {
                    b: vec![0.000244140625; 4097],
                    a: vec![1.],
                }
            };
            workspace.toolkits.open_coefficients(3);
        }
        if iir_capture.is_some()
            || std::env::args().any(|a| a == "--designer-demo" || a == "--iir-designer-demo")
        {
            if let Some(case) = iir_capture {
                workspace.toolkits.open_iir_capture(case);
                eprintln!(
                    "IIR Designer capture: case={} sample_rate=1000 cutoff=100 upper=200",
                    case.name()
                );
            } else {
                workspace
                    .toolkits
                    .open_designer_demo(std::env::args().any(|a| a == "--iir-designer-demo"));
            }
            if let Ok(script) = workspace.toolkits.preview_script() {
                workspace.run(script);
            }
        }
        if std::env::args().any(|a| a == "--log-demo") {
            workspace.documents =
                vec![Document::open(std::path::Path::new("examples/log_plots.m")).unwrap()];
            workspace.active = 0;
            workspace.run(include_str!("../examples/log_plots.m").into());
        }
        if std::env::args().any(|a| a == "--fig-demo") {
            workspace.submit(Job::OpenFigure(PathBuf::from(
                "tests/fixtures/fig/matlab-lines.fig",
            )));
        }
        if std::env::args().any(|a| a == "--visibility-demo") {
            workspace.submit(Job::OpenFigure(PathBuf::from(
                "tests/fixtures/fig/matlab-hidden.fig",
            )));
        }
        if workspace.figure_files_demo_stage != 0 {
            workspace.submit(Job::OpenFigure(PathBuf::from(
                "tests/fixtures/ui/hidden.mfig",
            )));
        }
        if std::env::args().any(|a| a == "--mat-import-demo") {
            workspace.submit(Job::InspectWorkspace(PathBuf::from(
                "tests/fixtures/mat/mat-import-v7.mat",
            )));
        }
        if workspace.large_import_demo {
            workspace.submit(Job::InspectWorkspace(PathBuf::from(
                "target/evidence/performance/data/selective-v6.mat",
            )));
        }
        if !std::env::args().any(|a| a == "--open-files")
            && std::env::args().any(|a| {
                matches!(
                    a.as_str(),
                    "--properties-demo" | "--series-properties-demo" | "--export-demo"
                )
            })
        {
            let source = "t=linspace(0,2,1000);x=sin(2*pi*3*t);y=0.7*sin(2*pi*3*t+0.4);plot(t,x,'b-',t,y,'r--');title('Compare two signals');xlabel('Time (seconds)');ylabel('Amplitude');legend('Reference','Measured');grid on;";
            workspace.documents[0].text = source.into();
            workspace.run(source.into());
        }
        if std::env::args().any(|a| a == "--simulation-demo") {
            workspace.toolkits.open_tool(toolkits::Tool::Model);
            workspace.submit(Job::Simulate(workspace.toolkits.model.clone()));
        }
        if !std::env::args().any(|a| a == "--open-files")
            && std::env::args().any(|a| a == "--batch-export-demo")
        {
            let source = "t=linspace(0,2,1000);figure(1);plot(t,sin(2*pi*3*t));title('Reference signal');xlabel('Time');figure(2);plot(t,exp(-t).*cos(2*pi*3*t));title('Damped response');xlabel('Time');";
            workspace.documents[0].text = source.into();
            workspace.run(source.into());
        }
        if std::env::args().any(|a| a == "--inspector-demo") {
            workspace.run(
                "A=reshape(1:60,10,6);B=A;plot(A(:,1),A(:,2));title('Editable workspace data');"
                    .into(),
            );
        }
        if std::env::args().any(|a| a == "--indexed-demo") {
            let document = std::fs::canonicalize("examples/indexed_assignment.m")
                .map_err(|error| error.to_string())
                .and_then(|path| Document::open(&path))
                .unwrap_or_else(|_| {
                    let mut document = Document::new();
                    document.text = include_str!("../examples/indexed_assignment.m").into();
                    document.changed();
                    document
                });
            let source = document.text.clone();
            workspace.documents[0] = document;
            workspace.run(source);
        }
        if std::env::args()
            .any(|a| matches!(a.as_str(), "--logical-demo" | "--logical-inspector-demo"))
        {
            let document = std::fs::canonicalize("examples/logical_selection.m")
                .map_err(|e| e.to_string())
                .and_then(|p| Document::open(&p))
                .unwrap_or_else(|_| {
                    let mut d = Document::new();
                    d.text = include_str!("../examples/logical_selection.m").into();
                    d.changed();
                    d
                });
            let source = document.text.clone();
            workspace.documents[0] = document;
            workspace.run(source);
        }
        if std::env::args()
            .any(|a| matches!(a.as_str(), "--control-flow-demo" | "--functions-demo"))
        {
            let functions = std::env::args().any(|a| a == "--functions-demo");
            let path = if functions {
                "examples/user_functions.m"
            } else {
                "examples/control_flow_filtering.m"
            };
            let example = if functions {
                include_str!("../examples/user_functions.m")
            } else {
                include_str!("../examples/control_flow_filtering.m")
            };
            let document = std::fs::canonicalize(path)
                .map_err(|e| e.to_string())
                .and_then(|p| Document::open(&p))
                .unwrap_or_else(|_| {
                    let mut d = Document::new();
                    d.text = example.into();
                    d.changed();
                    d
                });
            workspace.documents[0] = document;
            workspace.run_document(None);
        }
        if std::env::args().any(|a| a == "--function-error-demo") {
            match std::fs::canonicalize("examples/function_errors") {
                Ok(folder) => {
                    workspace.folder = folder;
                    workspace.folder_text = workspace.folder.display().to_string();
                    workspace.new_document(include_str!("../examples/function_errors/driver.m"));
                    workspace.run_document(None);
                }
                Err(error) => workspace.status = error.to_string(),
            }
        }
        if std::env::args().any(|a| a == "--selection-functions-demo") {
            let source = include_str!("../examples/selection_functions.m");
            workspace.new_document(source);
            let start = source.find("t = linspace").unwrap();
            let end = source.find("% End selection").unwrap();
            let range = matrust::editor::byte_chars(source, start..end);
            workspace.editor.jump = Some((workspace.documents[workspace.active].id, range.clone()));
            workspace.run_document(Some(range));
        }
        if workspace.figure_history_demo_stage != 0 {
            workspace.new_document(include_str!("../examples/fig_layouts.m"));
            workspace.run_document(None);
        }
        if !std::env::args().any(|a| a == "--open-files")
            && std::env::args().any(|a| a == "--layout-properties-demo")
        {
            workspace.new_document(include_str!("../examples/tiled_layout.m"));
            workspace.run_document(None);
        }
        if std::env::args().any(|a| a == "--fig-layout-demo") {
            workspace.new_document(include_str!("../examples/fig_layouts.m"));
            workspace.submit(Job::OpenFigure(PathBuf::from(
                "tests/fixtures/fig/roundtrip.fig",
            )));
        }
        if std::env::args().any(|a| a == "--tiled-demo") {
            workspace.new_document(include_str!("../examples/tiled_layout.m"));
            workspace.run_document(None);
        }
        if let Some(path) = std::env::args()
            .skip_while(|a| a != "--preview-image")
            .nth(1)
        {
            let path = PathBuf::from(path);
            let page = std::env::args()
                .skip_while(|a| a != "--preview-pdf-page" && a != "--preview-page")
                .nth(1)
                .and_then(|p| p.parse::<u32>().ok())
                .filter(|&p| p > 0);
            if path.extension().and_then(|e| e.to_str()).is_some_and(|e| {
                ["pdf", "tif", "tiff"]
                    .iter()
                    .any(|format| e.eq_ignore_ascii_case(format))
            }) && let Some(page) = page
            {
                workspace.submit(Job::OpenPagedImage(path, page - 1));
            } else {
                workspace.open_path(path);
            }
        }
        if workspace.plot_data_demo_pending {
            workspace.new_document(include_str!("../examples/workspace_plotting.m"));
            workspace.run_document(None);
            workspace.layout.toolstrip = matrust::session::ToolstripTab::Plots;
        }
        if let Some(flag) = std::env::args().find(|a| {
            matches!(
                a.as_str(),
                "--toolstrip-demo" | "--toolstrip-plots-demo" | "--toolstrip-apps-demo"
            )
        }) {
            workspace.layout.toolstrip = match flag.as_str() {
                "--toolstrip-plots-demo" => matrust::session::ToolstripTab::Plots,
                "--toolstrip-apps-demo" => matrust::session::ToolstripTab::Apps,
                _ => matrust::session::ToolstripTab::Home,
            };
            workspace.new_document(include_str!("../examples/fig_layouts.m"));
            workspace.run_document(None);
        }
        if std::env::args().any(|a| a == "--subplots-demo") {
            workspace.new_document(include_str!("../examples/subplots.m"));
            workspace.run_document(None);
        }
        if std::env::args().any(|a| a == "--discrete-plots-demo") {
            workspace.new_document(include_str!("../examples/discrete_plots.m"));
            workspace.run_document(None);
        }
        if std::env::args().any(|a| a == "--plot-properties-demo") {
            workspace.new_document(include_str!("../examples/plot_properties.m"));
            workspace.run_document(None);
        }
        if std::env::args().any(|a| a == "--marker-styles-demo") {
            workspace.new_document(include_str!("../examples/marker_styles.m"));
            workspace.run_document(None);
        }
        workspace.queue_open_paths(
            std::env::args_os()
                .skip_while(|arg| arg != "--open-files")
                .skip(1)
                .map(PathBuf::from),
        );
        if std::env::args().any(|arg| arg == "--compact-figure-demo") {
            workspace.compact_figure = true;
        }
        ctx.set_zoom_factor(f32::from(workspace.layout.ui_scale_percent()) / 100.);
        workspace
    }
    fn run(&mut self, source: String) {
        if !self.busy && !source.trim().is_empty() {
            self.submit(Job::ScriptInFolder(source, self.folder.clone()));
            if self.busy {
                self.editor_issue = None;
            }
        }
    }
    fn run_document(&mut self, selected: Option<std::ops::Range<usize>>) {
        if self.busy {
            return;
        }
        let doc = &self.documents[self.active];
        let (source, selection) = if let Some(range) = selected {
            match session::selected_text(&doc.text, range.clone()) {
                Ok(source) => (
                    source,
                    Some(
                        matrust::editor::char_byte(&doc.text, range.start)
                            ..matrust::editor::char_byte(&doc.text, range.end),
                    ),
                ),
                Err(error) => {
                    self.status = error;
                    return;
                }
            }
        } else {
            (doc.text.clone(), None)
        };
        if source.trim().is_empty() {
            return;
        }
        let origin = RunOrigin {
            document: doc.id,
            title: doc.title(),
            source: Arc::from(doc.text.as_str()),
        };
        let job = if let Some(range) = selection {
            Job::SelectionInFolder(origin.source.clone(), range, self.folder.clone())
        } else if let Some(path) = &doc.path {
            Job::DocumentInFolder(source, path.clone(), self.folder.clone())
        } else {
            Job::ScriptInFolder(source, self.folder.clone())
        };
        self.submit(job);
        if self.busy {
            self.running_origin = Some(origin);
            self.editor_issue = None;
        }
    }
    fn jump_to_error(&mut self) {
        if let Some(issue) = &mut self.editor_issue
            && let Some(path) = &issue.external
        {
            let index = self.documents.iter().position(|doc| {
                doc.path
                    .as_ref()
                    .is_some_and(|p| matrust::document::same_path(p, path))
            });
            let index = match index {
                Some(index) => index,
                None => {
                    if self.documents.len() >= session::MAX_DOCUMENTS {
                        self.status = "Close a script before opening more than 32 tabs".into();
                        return;
                    }
                    match Document::from_loaded_script(
                        path.clone(),
                        issue.origin.source.to_string(),
                    ) {
                        Ok(document) => self.documents.push(document),
                        Err(error) => {
                            self.status = error;
                            return;
                        }
                    }
                    session::remember(&mut self.recent, path.clone());
                    self.documents.len() - 1
                }
            };
            issue.origin.document = self.documents[index].id;
        }
        let Some(issue) = &self.editor_issue else {
            return;
        };
        let Some(index) = self
            .documents
            .iter()
            .position(|doc| doc.id == issue.origin.document)
        else {
            self.status = "The script that produced this error has been closed".into();
            return;
        };
        let doc = &self.documents[index];
        if doc.text.as_str() != issue.origin.source.as_ref() {
            self.status =
                "This script has changed since the error; run it again to update its location"
                    .into();
            return;
        }
        let start = issue.diagnostic.byte;
        let end = doc.text[start..]
            .find(['\r', '\n'])
            .map_or(doc.text.len(), |i| start + i);
        let jump = (doc.id, matrust::editor::byte_chars(&doc.text, start..end));
        let external = issue.external.is_some();
        self.activate_script_index(index);
        self.editor.jump = Some(jump);
        if external {
            self.status =
                "Showing the function source used by the failed run; save checks for disk changes"
                    .into();
        }
        self.reveal_editor();
    }
    fn new_document(&mut self, source: &str) {
        if self.documents.len() >= session::MAX_DOCUMENTS {
            self.status = "Close a script before opening more than 32 tabs".into();
            return;
        }
        let replace_placeholder = self.has_startup_placeholder();
        self.reveal_editor();
        let mut document = Document::new();
        document.text = source.into();
        document.changed();
        if replace_placeholder {
            self.forget_script_editor(self.documents[0].id);
            self.documents[0] = document;
            self.activate_script_index(0);
        } else {
            self.documents.push(document);
            self.activate_script_index(self.documents.len() - 1);
        }
        self.selection = None;
        self.recovery_dirty = true;
    }
    fn save_document(&mut self, index: usize, save_as: bool) -> bool {
        if self.script_conflict.is_some() || self.script_io.pending() {
            return false;
        }
        let path = if save_as || self.documents[index].path.is_none() {
            self.pick_script_save_path(index)
        } else {
            self.documents[index].path.clone()
        };
        let Some(path) = path else {
            self.status = "Save cancelled; script remains open".into();
            return false;
        };
        self.save_script_path(index, &path, None)
    }
    fn pick_script_save_path(&mut self, index: usize) -> Option<PathBuf> {
        #[cfg(test)]
        if let Some(picker) = self.script_io.save_picker {
            self.script_io.save_picker_calls += 1;
            return picker(self, index);
        }
        let current = self.documents[index].path.as_deref();
        let (filters, file_name) = script_save_choices(current);
        let mut dialog = rfd::FileDialog::new().set_directory(
            current
                .and_then(|path| path.parent())
                .unwrap_or(&self.folder),
        );
        for (name, extensions) in filters {
            dialog = dialog.add_filter(name, extensions);
        }
        dialog.set_file_name(file_name).save_file()
    }
    fn remove_document(&mut self, index: usize) {
        self.sync_editor_columns();
        let closed = self.documents[index].id;
        let visible =
            closed == self.documents[self.active].id || self.editor_columns.other == Some(closed);
        let survivor = if closed == self.documents[self.active].id {
            self.editor_columns.other
        } else {
            Some(self.documents[self.active].id)
        };
        self.forget_script_editor(closed);
        self.documents.remove(index);
        if self.documents.is_empty() {
            self.documents.push(Document::new());
            if let Some(center) = &mut self.layout.center {
                center.editor_revealed = false;
                center.command = true;
            }
        }
        if index < self.active {
            self.active -= 1;
        }
        self.active = self.active.min(self.documents.len() - 1);
        if visible {
            if let Some(index) =
                survivor.and_then(|id| self.documents.iter().position(|doc| doc.id == id))
            {
                self.active = index;
            }
            self.sync_editor_columns();
            self.collapse_editor_columns();
        }
        self.sync_editor_columns();
        self.pending_close = None;
        self.selection = None;
        self.recovery_dirty = true;
    }
    fn open_document(&mut self) {
        if let Some(paths) = self.pick_open_paths(open_files::PickerKind::Supported) {
            self.queue_open_paths(paths);
        }
    }
    fn open_path(&mut self, path: PathBuf) {
        if self.busy || self.file_action_pending() || !self.open_queue.is_empty() {
            self.queue_open_paths([path]);
        } else {
            self.try_open_path(path);
        }
    }
    fn restore_session(&mut self, saved: Session) {
        self.export_settings = saved.export;
        if self.eps_preview.renderer.is_none() {
            self.eps_preview.renderer = saved.eps_renderer;
        }
        self.workspace_manifest = saved.workspace_manifest;
        self.preview_restore = saved
            .preview
            .filter(matrust::session::PreviewState::valid)
            .map(preview_recovery::Restore::new);
        self.recovery_manifest = saved
            .figure_manifest
            .filter(|m| m.error.is_some() || !m.entries.is_empty());
        self.layout_presets = saved.layout_presets;
        self.layout = saved.layout;
        self.navigation = navigation::State::from_panels(self.layout.panels);
        self.documents = saved.documents;
        self.active = saved.active;
        self.restore_editor_columns();
        // One recovery reconciliation, not a per-frame empty-buffer heuristic.
        // Explicit empty editor tabs retain their serialized revealed flag.
        if let Some(center) = &mut self.layout.center {
            center.editor_revealed |= self.documents.iter().any(|d| d.dirty || d.path.is_some());
            if !center.command
                && self.layout.mode != layout::PaneMode::Figure
                && !center.editor_revealed
            {
                center.command = true;
            }
        }
        self.recent = saved.recent;
        self.history = History::from_entries(saved.history);
        self.toolkits
            .restore_model(saved.model, saved.model_path, saved.model_baseline);
        if let Some(tool) = self.toolkits.restore_tab_layout(self.layout.tools) {
            self.select_tool_tab(tool);
        }
        self.folder = saved.folder;
        self.console.push("Restored scripts, command history, block model and layout from the last session. Unsaved scripts retain their * markers.".into());
    }
    fn snapshot(&self) -> Session {
        let (model_path, model_baseline) = self.toolkits.model_metadata();
        let mut layout = self.layout;
        // Capture actual current intent even when Close snapshots before the
        // frame's preference bookkeeping. Transient focus tickets stay private.
        layout.panels = self.navigation.panels();
        layout.editor_columns = self.editor_columns_snapshot();
        layout.tools = self.toolkits.tab_layout();
        Session {
            layout_presets: self.layout_presets.clone(),
            export: self.export_settings,
            workspace: Some(self.variables.clone()),
            workspace_manifest: self.workspace_manifest.clone(),
            preview: self.preview_checkpoint(),
            eps_renderer: self.eps_preview.renderer.clone(),
            figure_manifest: self.recovery_manifest.clone(),
            figures: Some(self.figure_checkpoint()),
            layout,
            version: 1,
            documents: self.documents.clone(),
            active: self.active,
            folder: self.folder.clone(),
            recent: self.recent.clone(),
            history: self.history.entries.clone(),
            model: self.toolkits.model.clone(),
            model_path,
            model_baseline,
        }
    }
    fn run_selection(&mut self) {
        if let Some((_, range)) = self.selection.as_ref().filter(|(i, _)| *i == self.active) {
            self.run_document(Some(range.clone()));
        } else {
            self.status = "Select code in the editor first".into();
        }
    }
    fn inspect_variable(&mut self, name: String) {
        self.numeric_editor.invalidate(false);
        self.typed_inspector = Default::default();
        if self.variables.contains_key(&name) {
            self.selected = Some(name.clone());
            self.workspace_tools.names = std::collections::BTreeSet::from([name.clone()]);
            self.edit_row = 1;
            self.edit_col = 1;
            self.first_col = 1;
            // The numeric grid formats its initial value within the per-frame budget.
            self.edit_value.clear();
            self.cell_error = None;
            self.editing_variable = Some(name);
        }
    }
    fn submit(&mut self, job: Job) {
        if self.busy {
            return;
        }
        // Bind relative file paths to this workspace before asynchronous dispatch.
        // The host process directory stays unchanged.
        let job = match job.in_folder(&self.folder) {
            Ok(job) => job,
            Err(e) => {
                self.status = e;
                return;
            }
        };
        if !matches!(&job, Job::PrepareDetached(_) | Job::SelectExistingFigure(_)) {
            self.detached.cancel_pending();
        }
        let image_action = matches!(&job, Job::ExportPreview(_) | Job::CopyPreview(_));
        let job = self.bind_single_export(job);
        self.worker.cancel.store(false, Ordering::Relaxed);
        let cell_job = match &job {
            #[cfg(test)]
            Job::SetCell(name, ..) => Some(name.clone()),
            _ => None,
        };
        let opening_job = matches!(&job, Job::PrepareOpen(_) | Job::OpenSearchResult(_));
        let image_job = matches!(
            &job,
            Job::OpenImage(_) | Job::OpenEps(..) | Job::OpenPagedImage(..) | Job::ImagePage(_)
        );
        let job = if matches!(
            &job,
            Job::ScriptInFolder(..) | Job::DocumentInFolder(..) | Job::SelectionInFolder(..)
        ) && !self.figure_views.is_empty()
        {
            Job::ScriptViewContext(
                Box::new(job),
                self.figure_views
                    .iter()
                    .map(|(&id, view)| (id, (view.bounds, view.auto)))
                    .collect(),
            )
        } else {
            job
        };
        match self.worker.send.try_send(self.worker.request(job)) {
            Ok(()) => {
                if opening_job {
                    self.document_save.open_started();
                }
                if image_job {
                    self.document_save.preview_started();
                }
                if image_job && self.preview_restore.take().is_some() {
                    self.recovery_dirty = true;
                }
                self.image_loading = image_job;
                self.image_action = image_action;
                if image_action && let Some(viewer) = &mut self.image_view {
                    viewer.error = None;
                    viewer.notice = None;
                }
                self.image_ignore_result = false;
                if cell_job.is_some() {
                    self.cell_error = None;
                }
                self.cell_job = cell_job;
                self.busy = true;
                self.running_origin = None;
                self.status = chrome::RUNNING.into();
            }
            Err(e) => {
                self.status = format!("Worker unavailable: {e}");
                if self.single_export.pending() {
                    self.cancel_single_export();
                }
                if cell_job.is_some() {
                    self.cell_error = Some(self.status.clone());
                }
                if let Some(editor) = &mut self.figure_editor {
                    editor.saving = false;
                    editor.error = Some(self.status.clone());
                }
            }
        }
    }
    fn receive(&mut self) {
        self.advance_script_io();
        self.advance_model_read();
        self.receive_folder();
        if let Some(store) = &self.recovery {
            while let Ok(error) = store.errors.try_recv() {
                self.console.push(format!("Recovery: {error}"));
            }
        }
        if self.console.len() > 200 {
            self.console.drain(..self.console.len() - 200);
        }
        if let Ok(reply) = self.worker.receive.try_recv() {
            let detached_prepared = reply.detached_prepared;
            let selected_existing = reply.selected_existing;
            self.busy = false;
            self.display_format = reply.display;
            if let Some(folder) = reply.current_folder {
                self.commit_folder(folder.clone(), None);
                self.resolve_folder(folder, None);
            }
            self.receive_variable_edit(reply.variable_revision, reply.variable_edit);
            self.receive_cell_replacement(reply.cell_replacement);
            if let Some(result) = reply.folder_search {
                self.receive_folder_search(result);
                return;
            }
            if let Some(result) = reply.search_open {
                self.receive_search_open(result);
                return;
            }
            if let Some(finished) = reply.single_export {
                self.receive_single_export(finished);
                return;
            }
            if let Some(finished) = reply.original_copy {
                self.receive_original_copy(finished);
                return;
            }
            if let Some(script) = reply.script_io {
                self.receive_script_io(script);
                return;
            }
            if let Some(model) = reply.model_write {
                self.receive_model_write(model);
                return;
            }
            if let Some(result) = reply.design_save {
                self.complete_design_save(result);
                return;
            }
            if let Some(mut result) = reply.fir_block {
                if self.worker.cancel.load(Ordering::Relaxed) {
                    result = Err("Filter design cancelled; model unchanged".into());
                }
                self.recovery_dirty |= self.toolkits.finish_filter_block(result);
                self.status = self.toolkits.message.clone();
                return;
            }
            if let Some(model) = reply.model_read {
                self.receive_model_read(model);
                return;
            }
            if let Some(prepared) = reply.prepared_open {
                self.complete_open(prepared);
                return;
            }
            if self.image_action {
                if let Some(viewer) = &mut self.image_view {
                    match &reply.result {
                        Ok(execution) => viewer.notice = execution.output.last().cloned(),
                        Err(error) => viewer.error = Some(error.clone()),
                    }
                }
                self.image_action = false;
            }
            if self.image_loading {
                if let Some(path) = self.eps_preview.loading.take()
                    && !self.image_ignore_result
                    && !self.worker.cancel.load(Ordering::Relaxed)
                    && let Err(error) = &reply.result
                {
                    self.eps_preview.pending = Some(path);
                    self.eps_preview.error = Some(error.clone());
                }
                if let Err(error) = &reply.result
                    && let Some(restore) = &mut self.preview_restore
                {
                    restore.error = Some(error.clone());
                }
                if let Err(error) = &reply.result
                    && let Some(viewer) = &mut self.image_view
                {
                    viewer.error = Some(error.clone());
                }
                if !self.image_ignore_result
                    && let Some(image) = reply.image_preview
                {
                    self.pending_image = Some(image);
                    self.pending_page = reply.page_preview;
                }
                self.image_loading = false;
                self.image_ignore_result = false;
            }
            if self
                .cell_job
                .take()
                .is_some_and(|name| self.editing_variable.as_ref() == Some(&name))
            {
                self.cell_error = reply.result.as_ref().err().cloned();
            }
            if let Some(catalog) = reply.workspace_catalog {
                let mut import = workspace_import::Import::new(catalog);
                if self.large_import_demo {
                    import.selected.insert("small".into());
                }
                self.workspace_import = Some(import);
            } else if let Some(import) = &mut self.workspace_import
                && import.loading
            {
                import.loading = false;
                match &reply.result {
                    Ok(_) => self.workspace_import = None,
                    Err(e) => import.error = Some(e.clone()),
                }
            }
            self.finish_csv_import(&reply.result);
            let restored = reply.recovered.is_some();
            let activate_opened_figure = self.document_save.figure_open_completed();
            let reveal_changed_figure = !restored
                && reply.result.is_ok()
                && !self.worker.cancel.load(Ordering::Relaxed)
                && activate_opened_figure != Some(false)
                && reply.figure.as_ref().is_some_and(|figure| {
                    activate_opened_figure == Some(true)
                        || self.current_figure != reply.current_figure
                        || self
                            .figure
                            .as_ref()
                            .is_none_or(|old| !old.same_snapshot(figure))
                });
            self.recovery_dirty |= self.figure_documents != reply.figure_documents
                || self.current_figure != reply.current_figure;
            self.all_figures = reply.figures;
            self.figure_windows.retain(|id, placement| {
                self.all_figures
                    .get(id)
                    .is_some_and(|f| placement.identity.same_object(&f.identity))
            });
            self.figure_documents = reply.figure_documents;
            self.figure_history = reply.figure_history;
            self.figure_views.retain(|id, view| {
                self.figure_documents
                    .get(id)
                    .is_some_and(|d| d.revision == view.revision)
            });
            if let Some(id) = reply.reloaded_figure {
                self.figure_views.remove(&id);
                if id == reply.current_figure {
                    self.view_bounds = None;
                    self.reset_plot = true;
                }
            }
            if let Some(conflict) = reply.figure_conflict {
                self.figure_conflict = Some(conflict);
            } else if self.figure_conflict.as_ref().is_some_and(|c| {
                reply.saved_figure == Some(c.id) || reply.reloaded_figure == Some(c.id)
            }) {
                self.figure_conflict = None;
                if let Some(flow) = &mut self.figure_close {
                    flow.saving = true;
                }
            }
            if let Some(recovered) = reply.recovered {
                self.recovered_figures(recovered);
            }
            if let Some(id) = reply.saved_figure
                && let Some(view) = self.figure_views.get_mut(&id)
            {
                view.dirty = false;
            }
            if reply.result.is_err()
                && let Some(flow) = &mut self.figure_close
            {
                flow.saving = false;
                if matches!(flow.action, figure_files::Closing::SaveOnly)
                    && self.figure_conflict.is_none()
                {
                    self.figure_close = None;
                }
            }
            if let Some(editor) = &mut self.figure_editor
                && editor.saving
            {
                match &reply.result {
                    Ok(_) => {
                        self.reset_plot |= editor.reset_view;
                        self.figure_editor = None;
                    }
                    Err(e) => {
                        editor.saving = false;
                        editor.error = Some(e.clone());
                    }
                }
            }
            self.reset_plot |= self.current_figure != reply.current_figure;
            self.restore_plot_view = restored || self.current_figure != reply.current_figure;
            self.figure_ids = reply.figure_ids;
            self.current_figure = reply.current_figure;
            self.recovery_dirty |= self.variables.len() != reply.vars.len()
                || self.variables.iter().any(|(name, value)| {
                    reply.vars.get(name).is_none_or(|v| !value.same_snapshot(v))
                });
            self.variables = reply.vars;
            self.reconcile_workspace_change(reply.workspace_change, reply.result.as_ref().err());
            if std::env::args().any(|a| a == "--logical-inspector-demo")
                && self.editing_variable.is_none()
            {
                self.inspect_variable("selected".into());
            }
            if std::env::args().any(|a| a == "--inspector-demo") && self.editing_variable.is_none()
            {
                self.inspect_variable("A".into());
            }
            if reply.figure.as_ref().is_some_and(|f| f.layout.is_some()) {
                self.figure_views.remove(&self.current_figure);
                self.view_bounds = None;
            }
            let same_points = match (&self.figure, &reply.figure) {
                (Some(a), Some(b)) => {
                    a.active_position() == b.active_position()
                        && a.layout.is_some() == b.layout.is_some()
                        && a.series.len() == b.series.len()
                        && a.xscale == b.xscale
                        && a.yscale == b.yscale
                        && a.xlim == b.xlim
                        && a.ylim == b.ylim
                        && a.series.iter().zip(&b.series).all(|(a, b)| {
                            a.points.same_storage(&b.points)
                                && a.kind == b.kind
                                && a.visible == b.visible
                                && a.bar_width == b.bar_width
                                && a.bar_offset == b.bar_offset
                        })
                }
                (None, None) => true,
                _ => false,
            };
            if !same_points {
                self.view_bounds = None;
                // Release obsolete root display buffers BEFORE replacement allocation.
                // PlotMemory and committed views retain interaction state separately.
                self.plot_points = Vec::new();
                if let Some(figure) = reply.figure.as_ref().filter(|f| f.layout.is_some()) {
                    self.subplot_canvas
                        .retain_source(self.current_figure, figure);
                } else {
                    self.subplot_canvas.release_geometry();
                }
                // The one-cache reservation includes retained unchanged subplot axes.
                let _ = self
                    .detached
                    .reserve_root_cache(0, figure_geometry::ONE_CACHE_BYTES);
                self.plot_points = reply
                    .figure
                    .as_ref()
                    .filter(|f| f.layout.is_none() && !self.detached.contains(self.current_figure))
                    .map(|f| {
                        self.plot_axes = f.axis_transforms();
                        f.series
                            .iter()
                            .map(|s| {
                                s.display_points_axes(8192 / f.series.len().max(1), self.plot_axes)
                                    .into_iter()
                                    .map(|[x, y]| PlotPoint::new(x, y))
                                    .collect()
                            })
                            .collect()
                    })
                    .unwrap_or_default();
            }
            self.reset_plot |= !same_points
                || match (&self.figure, &reply.figure) {
                    (Some(a), Some(b)) => a.xlim != b.xlim || a.ylim != b.ylim,
                    _ => false,
                };
            if self.reset_plot {
                self.view_bounds = None;
            }
            self.figure = reply.figure;
            if reveal_changed_figure {
                self.detached
                    .reveal_tile(self.current_figure, &self.all_figures);
            }
            if reveal_changed_figure && self.layout.center.is_some() {
                self.show_plot_pane();
            }
            if !restored
                && self.figure.is_none()
                && self.layout.mode == layout::PaneMode::Figure
                && let Some(center) = &mut self.layout.center
            {
                center.command = true;
            }
            self.complete_sample_pick(reply.sample_hit, reply.result.as_ref().err().cloned());
            self.status = format!(
                "Completed in {:.2} ms",
                reply.elapsed.as_secs_f64() * 1000.0
            );
            if let Some(plan) = reply.export_plan
                && let Some(dialog) = &mut self.figure_export
            {
                dialog.review(plan);
            } else if let Some(outcome) = reply.export_outcome
                && let Some(dialog) = &mut self.figure_export
            {
                if outcome.error.is_none() {
                    self.figure_export = None;
                } else {
                    dialog.complete_batch(outcome);
                }
            } else if let Some(dialog) = &mut self.figure_export
                && dialog.saving
            {
                match &reply.result {
                    Ok(_) => self.figure_export = None,
                    Err(error) => {
                        dialog.saving = false;
                        dialog.error = Some(error.clone());
                    }
                }
            }
            if let Some(dialog) = &mut self.plot_data
                && dialog.creating
            {
                match &reply.result {
                    Ok(_) => {
                        self.plot_data = None;
                        if self.layout.mode == layout::PaneMode::Editor {
                            self.layout.mode = layout::PaneMode::Split;
                        }
                    }
                    Err(error) => {
                        dialog.creating = false;
                        dialog.error = Some(error.clone());
                    }
                }
            }
            match reply.result {
                Ok(result) => {
                    if result.clear_console {
                        self.console.clear();
                    }
                    self.console.extend(result.output);
                }
                Err(error) => {
                    self.status = if self.figure_conflict.is_some() {
                        error.clone()
                    } else {
                        "Calculation stopped".into()
                    };
                    if reply.partial_output.clear_console {
                        self.console.clear();
                    }
                    self.console.extend(reply.partial_output.output);
                    if let Some(file) = reply.file_diagnostic {
                        self.console.push(format!("Error: {error}"));
                        self.editor_issue = Some(EditorIssue {
                            origin: RunOrigin {
                                document: 0,
                                title: file
                                    .path
                                    .file_name()
                                    .unwrap_or_default()
                                    .to_string_lossy()
                                    .into_owned(),
                                source: file.source,
                            },
                            diagnostic: file.diagnostic,
                            external: Some(file.path),
                        });
                    } else if let (Some(origin), Some(diagnostic)) =
                        (self.running_origin.take(), reply.diagnostic)
                    {
                        self.console
                            .push(format!("Error in {}: {diagnostic}", origin.title));
                        self.editor_issue = Some(EditorIssue {
                            origin,
                            diagnostic,
                            external: None,
                        });
                    } else {
                        self.console.push(format!("Error: {error}"));
                    }
                }
            }
            self.running_origin = None;
            if std::env::args().any(|a| a == "--function-error-demo") {
                self.jump_to_error();
            }
            if self.console.len() > 200 {
                self.console.drain(..self.console.len() - 200);
            }
            self.refresh_folder();
            let mut detached = std::mem::take(&mut self.detached);
            detached.complete(self, selected_existing, detached_prepared);
            self.detached = detached;
            if self.detached.contains(self.current_figure) {
                self.view_bounds = self
                    .figure_views
                    .get(&self.current_figure)
                    .map(|v| v.bounds);
                self.view_auto = self
                    .figure_views
                    .get(&self.current_figure)
                    .map_or([true; 2], |v| v.auto);
                self.reset_plot = false;
                self.restore_plot_view = false;
            }
        }
    }
}

impl Workspace {
    fn ui(&mut self, root: &mut egui::Ui) {
        let ctx = &root.ctx().clone();
        self.observe_workspace_mat_save(ctx);
        self.observe_root_open(ctx);
        self.observe_root_figure_save(ctx);
        self.observe_designer_save(ctx);
        self.observe_model_toolbar(ctx);
        self.observe_folder_browse(ctx);
        self.observe_csv_import_picker(ctx);
        self.observe_csv_naming_action(ctx);
        self.observe_ordinary_script_conflict(ctx);
        self.observe_editor_window_root(ctx);
        self.begin_focus_areas(ctx);
        self.observe_workspace_csv(ctx);
        self.observe_global_search(ctx);
        self.observe_named_layout(ctx);
        self.apply_desktop_theme(ctx);
        root.set_style(ctx.global_style());
        // Panel cards sit on the canvas; the gutters between them show it.
        root.painter()
            .rect_filled(root.max_rect(), 0., tokens::palette(ctx).canvas);
        // Every card keeps a 4 px margin; the window edge adds the other half
        // of the 8 px gutter.
        let mut inset = root.new_child(
            egui::UiBuilder::new()
                .max_rect(root.max_rect().shrink2(egui::vec2(tokens::GUTTER / 2., 0.)))
                .layout(*root.layout()),
        );
        let root = &mut inset;
        self.document_save
            .set_tiled(self.layout.figure_tiling.is_some());
        self.observe_editor_columns(ctx);
        self.document_save
            .begin_pass(ctx, editor::id(self.documents[self.active].id));
        self.observe_script_focus(ctx);
        let previous_layout = self.layout;
        self.prepare_layout(ctx);
        // Layout can retry a pass within one frame. Dispatch side effects once,
        // so file drops and save queues are not replayed during that retry.
        let frame = ctx.cumulative_frame_nr();
        if self.ui_dispatch_frame != Some(frame) {
            self.ui_dispatch_frame = Some(frame);
            self.desktop_scale_shortcuts(ctx);
            self.receive();
            self.workspace_tools_demo();
            self.advance_script_saves(ctx);
            let dropped = ctx.input(|i| {
                i.raw
                    .dropped_files
                    .iter()
                    .map(|f| f.path().to_path_buf())
                    .take(open_queue::MAX_QUEUED_FILES + 1)
                    .collect::<Vec<_>>()
            });
            self.queue_open_paths(dropped);
            if ctx.input_mut(|i| {
                i.consume_key(egui::Modifiers::CTRL | egui::Modifiers::SHIFT, egui::Key::X)
            }) {
                self.cancel_queued_files();
            }
            self.advance_open_queue(ctx);
            self.sample_inspection_demo();
        }
        self.sync_editor_columns();
        self.document_save
            .set_script_document(self.script_action_document(ctx));
        let pending = self.file_action_pending();
        if !self.allow_close
            && self.script_save_queue.is_some()
            && ctx.input(|i| i.viewport().close_requested())
        {
            ctx.send_viewport_cmd(egui::ViewportCommand::CancelClose);
            self.status = "Finish or cancel Save all scripts before closing".into();
        }
        if !self.allow_close
            && self.plot_data.is_some()
            && ctx.input(|i| i.viewport().close_requested())
        {
            ctx.send_viewport_cmd(egui::ViewportCommand::CancelClose);
            if let Some(dialog) = &mut self.plot_data {
                dialog.error = Some("Finish or cancel plot creation before closing.".into());
            }
        }
        if ctx.input(|i| i.viewport().close_requested())
            && !self.allow_close
            && let Some(editor) = &mut self.figure_editor
        {
            ctx.send_viewport_cmd(egui::ViewportCommand::CancelClose);
            editor.error = Some("Apply or cancel figure properties before closing.".into());
        }
        if ctx.input(|i| i.viewport().close_requested())
            && !self.allow_close
            && self.figure_editor.is_none()
            && self.figure_export.is_none()
            && self.plot_data.is_none()
            && self.script_save_queue.is_none()
        {
            self.request_quit(ctx);
        }
        if ctx.input(|i| i.viewport().close_requested())
            && !self.allow_close
            && let Some(dialog) = &mut self.figure_export
        {
            ctx.send_viewport_cmd(egui::ViewportCommand::CancelClose);
            dialog.error = Some("Finish or cancel the export before closing.".into());
        }
        let shortcut =
            !pending && ctx.input_mut(|i| i.consume_key(egui::Modifiers::CTRL, egui::Key::Enter));
        if shortcut && !self.busy {
            self.request_script_run(ctx, false);
        }
        if !pending {
            if ctx.input(|i| {
                i.modifiers.ctrl && (i.key_pressed(egui::Key::F) || i.key_pressed(egui::Key::H))
            }) {
                self.reveal_editor();
                // Find/Replace owns query focus; do not replace it with a
                // generic foreground document focus after the child paints.
                self.editor_window.clear_focus();
                if self.editor_window.detached {
                    ctx.send_viewport_cmd_to(
                        editor_window::viewport(),
                        egui::ViewportCommand::Focus,
                    );
                    ctx.request_repaint_of(editor_window::viewport());
                }
            }
            self.editor.shortcuts(ctx);
        }
        if !pending && ctx.input_mut(|i| i.consume_key(egui::Modifiers::NONE, egui::Key::F9)) {
            self.request_script_run(ctx, true);
        }
        if !pending
            && ctx.input_mut(|i| {
                i.consume_key(egui::Modifiers::CTRL | egui::Modifiers::ALT, egui::Key::S)
            })
        {
            self.begin_save_all_scripts();
        } else if !pending
            && ctx.input_mut(|i| {
                i.consume_key(egui::Modifiers::CTRL | egui::Modifiers::SHIFT, egui::Key::S)
            })
        {
            self.document_save
                .queue(ctx.cumulative_frame_nr(), true, self.generic_save_target());
        } else if !pending && ctx.input_mut(|i| i.consume_key(egui::Modifiers::CTRL, egui::Key::S))
        {
            self.document_save
                .queue(ctx.cumulative_frame_nr(), false, self.generic_save_target());
        }
        if !pending
            && ctx.input_mut(|i| {
                i.consume_key(
                    egui::Modifiers::CTRL | egui::Modifiers::SHIFT,
                    egui::Key::Tab,
                )
            })
        {
            self.cycle_script(ctx, true);
        } else if !pending
            && ctx.input_mut(|i| i.consume_key(egui::Modifiers::CTRL, egui::Key::Tab))
        {
            self.cycle_script(ctx, false);
        }
        if !pending && ctx.input_mut(|i| i.consume_key(egui::Modifiers::CTRL, egui::Key::W)) {
            self.document_save
                .queue_close(ctx.cumulative_frame_nr(), self.generic_save_target());
        }
        self.root_open_shortcut(ctx);
        if !pending && ctx.input_mut(|i| i.consume_key(egui::Modifiers::CTRL, egui::Key::N)) {
            self.new_document("");
        }
        self.pane_focus_shortcuts(ctx);
        self.folder_shortcuts(ctx, pending);
        self.desktop_toolbar(root, pending);
        let status_frame = egui::Frame::new()
            .fill(tokens::palette(root.ctx()).canvas)
            .inner_margin(egui::Margin::symmetric(tokens::M as i8, 0));
        let compact = tokens::compact(root.ctx());
        egui::Panel::bottom("status")
            .show_separator_line(false)
            .frame(status_frame)
            .exact_size(if compact {
                tokens::COMPACT_ROW
            } else {
                tokens::STATUS_BAR
            })
            .show(root, |ui| self.status_bar(ui, compact));
        self.desktop_side_panels(root);
        self.center_tabs(root);
        // Freeze placement for this pass: actions inside a pane cannot cause a
        // second command TextEdit to render under the same ID. A selected tool
        // tab owns the main area; Command then docks below it like the Editor.
        let tool = self.toolkits.selected_tool();
        let central_command = tool.is_none() && self.central_command();
        let figure_column = self.figure_column();
        let editor_above = self.editor_above_command();
        if figure_column {
            let width = root.available_rect_before_wrap().width();
            egui::Panel::right("figure_column")
                .show_separator_line(false)
                .frame(tokens::card(ctx).inner_margin(1))
                .default_size(self.layout.figure_width.unwrap_or(tokens::RIGHT_COLUMN))
                .size_range(280.0..=(width * 0.5).max(280.))
                .resizable(true)
                .show(root, |ui| self.figure_pane(ui));
            if let Some(size) = layout::resized_panel(ctx, "figure_column") {
                self.layout.figure_width = Some(size.x);
            }
        }
        if self.navigation.command && (!central_command || editor_above) {
            // Tool tabs and legacy compact panes retain useful canvas height
            // beside the visible toolstrip: keep their Command Window minimal.
            let minimal = (tool.is_some() || self.layout.center.is_none())
                && ctx.content_rect().height() < 760.0;
            // Beside an editor and figure the console keeps six output lines;
            // the editor above yields the height.
            let console_min = if minimal {
                CONSOLE_MIN
            } else {
                console_six_lines(ctx)
            };
            egui::Panel::bottom("console")
                .show_separator_line(false)
                .frame(tokens::card(ctx).inner_margin(1))
                .default_size(self.layout.console_height.unwrap_or(
                    if ctx.content_rect().height() < 750.0 {
                        console_min.max(CONSOLE_MIN + 20.)
                    } else {
                        230.0
                    },
                ))
                .min_size(console_min)
                .max_size(if minimal {
                    CONSOLE_MIN
                } else {
                    (root.available_rect_before_wrap().height()
                        * if ctx.content_rect().height() < 750.0 {
                            0.35
                        } else {
                            0.40
                        })
                    .max(console_min)
                })
                .resizable(true)
                .show(root, |ui| {
                    self.command_window(ui);
                });
        }
        let mut tool_pass = self.toolkits.begin_pass(ctx);
        egui::CentralPanel::default()
            .frame(tokens::card(ctx).inner_margin(1))
            .show(root, |ui| {
                if let Some(tool) = tool {
                    self.document_save.set_compact(false);
                    self.toolkits.tab_ui(ui, tool, self.busy, &mut tool_pass);
                    return;
                }
                if editor_above {
                    self.document_save.set_compact(false);
                    self.editor_pane(ui);
                    return;
                }
                if central_command {
                    self.document_save.set_compact(false);
                    self.command_window(ui);
                    return;
                }
                self.document_save.set_compact(
                    self.layout.mode == layout::PaneMode::Split
                        && ui.available_width() < 580.
                        && (self.figure.is_some() || self.image_view.is_some()),
                );
                match self.layout.mode {
                    layout::PaneMode::Editor => self.editor_pane(ui),
                    layout::PaneMode::Figure => self.figure_pane(ui),
                    layout::PaneMode::Split
                        if self.figure.is_none() && self.image_view.is_none() =>
                    {
                        self.editor_pane(ui)
                    }
                    layout::PaneMode::Split if ui.available_width() < 580. => {
                        self.compact_panes(ui)
                    }
                    layout::PaneMode::Split => {
                        let width = ui.available_width();
                        self.prepare_split(ctx, width);
                        egui::Panel::left("editor_split")
                            .default_size(width * self.layout.editor_fraction.unwrap_or(0.5))
                            .size_range(280.0..=(width - 290.0).max(280.0))
                            .resizable(true)
                            .show(ui, |ui| self.editor_pane(ui));
                        if let Some(size) = layout::resized_panel(ctx, "editor_split") {
                            self.layout.editor_fraction = Some((size.x / width).clamp(0.15, 0.85));
                        }
                        egui::CentralPanel::default().show(ui, |ui| self.figure_pane(ui));
                    }
                }
            });
        if let Some(action) = self.toolkits.finish_pass(ctx, tool_pass) {
            match action {
                toolkits::Action::Script(s) => self.run(s),
                toolkits::Action::Apply(design, input) => {
                    let view = self
                        .figure_views
                        .get(&self.current_figure)
                        .map(|v| (self.current_figure, (v.bounds, v.auto)));
                    self.submit(Job::ApplyFilter(Box::new(designer_apply::Request {
                        design,
                        input,
                        folder: self.folder.clone(),
                        view,
                    })));
                }
                toolkits::Action::FirBlock(request) => self.submit_fir_block(request),
                toolkits::Action::IirBlock(request) => self.submit_iir_block(request),
                toolkits::Action::ChooseDesignSave(design) => {
                    self.queue_designer_save(ctx, design);
                }
                #[cfg(test)]
                toolkits::Action::SaveFir(request) => self.submit_fir_save(request),
                toolkits::Action::Simulate(m) => self.submit(Job::Simulate(m)),
                toolkits::Action::ModelResolved => {
                    self.recovery_dirty = true;
                    if self.pending_quit {
                        self.save_pending_scripts(ctx);
                    }
                }
                toolkits::Action::StopModel => self.stop_work(),
                toolkits::Action::NewModel => self.request_new_model(),
                toolkits::Action::ModelFile(kind) => self.queue_model_toolbar(ctx, kind),
                toolkits::Action::ModelCancelled => {
                    self.pending_quit = false;
                    self.pending_close = None;
                    self.quit_discarded_figures.clear();
                }
            }
        }
        if self.toolkits.model_open
            && let Some((rect, layer)) = self.toolkits.model_document_region()
        {
            self.document_save
                .register(document_save::Pane::Model, rect, layer);
        } else if !self.toolkits.model_open {
            self.document_save.model_closed();
        }
        self.advance_model_read();
        self.script_io_ui(ctx);
        self.advance_script_io();
        self.workspace_action_ui(ctx);
        self.variable_editor(ctx);
        self.recovery_dirty |= self.toolkits.take_diagram_change();
        if self.toolkits.model_open
            && ctx.input(|i| {
                i.pointer.any_released()
                    || i.events.iter().any(|e| {
                        matches!(
                            e,
                            egui::Event::Text(_)
                                | egui::Event::Paste(_)
                                | egui::Event::Key { pressed: true, .. }
                        )
                    })
            })
        {
            self.recovery_dirty = true;
        }
        if !self.busy
            && self.figure_history_demo_stage == 1
            && let Some(original) = self.figure.clone()
        {
            let mut updated = original.clone();
            updated.title = "Edited damped signal".into();
            updated.series[0].color = [0, 114, 189];
            updated.reposition_axes([0.15, 0.1, 0.7, 0.3]).unwrap();
            self.figure_history_demo_stage = 2;
            self.submit(Job::EditFigure(Box::new(worker::FigureEdit {
                view_before: None,
                id: self.current_figure,
                original,
                updated,
            })));
        } else if !self.busy
            && self.figure_history_demo_stage == 2
            && std::env::args().any(|a| a == "--figure-history-undone-demo")
        {
            self.figure_history_demo_stage = 3;
            self.submit(Job::UndoFigure(
                self.current_figure,
                self.view_bounds.map(|v| (v, self.view_auto)),
            ));
        }
        if self.properties_demo_pending
            && !self.busy
            && !self.recovering_session
            && self.open_queue.is_empty()
            && !self.file_action_pending()
            && (self.view_bounds.is_some()
                || self.figure.as_ref().is_some_and(|f| f.layout.is_some()))
            && self.figure_editor.is_none()
            && let Some(f) = &self.figure
        {
            let mut editor = figure_editor::Editor::new(self.current_figure, f, self.view_bounds);
            if std::env::args().any(|a| {
                matches!(
                    a.as_str(),
                    "--series-properties-demo" | "--line-width-properties-demo"
                )
            }) {
                editor.show_series();
            }
            if std::env::args().any(|a| a == "--layout-properties-demo") {
                editor.show_layout();
            }
            self.figure_editor = Some(editor);
            self.properties_demo_pending = false;
        }
        if self.figure_reopen_demo
            && !self.busy
            && !self.recovering_session
            && self.figure.is_some()
        {
            self.figure_reopen_demo = false;
            if let Some(path) = self
                .figure_documents
                .get(&self.current_figure)
                .and_then(|d| d.path.clone())
            {
                self.submit(Job::OpenFigureOrSelect(path));
            }
        }
        if self.figure_reload_demo
            && !self.busy
            && !self.recovering_session
            && self.figure.is_some()
        {
            self.figure_reload_demo = false;
            self.request_figure_reload();
        }
        if self.figure_conflict_demo
            && !self.busy
            && !self.recovering_session
            && self.figure.is_some()
        {
            self.figure_conflict_demo = false;
            self.save_figure_document(self.current_figure, false);
        }
        if self.figure_files_demo_stage != 0 && !self.busy && self.figure.is_some() {
            if self.figure_files_demo_stage == 1 {
                self.run("title('Edited saved figure'); figure; t=linspace(0,2,1000);plot(t,sin(2*pi*3*t));title('Unsaved measurement');xlabel('Time (seconds)');ylabel('Amplitude');".into());
                self.figure_files_demo_stage = 2;
            } else {
                self.figure_close = Some(figure_files::CloseFlow {
                    action: if std::env::args().any(|a| a == "--close-all-figures-demo") {
                        figure_files::Closing::All
                    } else {
                        figure_files::Closing::Application
                    },
                    saving: false,
                });
                self.figure_files_demo_stage = 0;
            }
            self.screenshot_ready_frames = 0;
        }
        if self.export_demo_pending
            && !self.busy
            && !self.recovering_session
            && self.open_queue.is_empty()
            && !self.file_action_pending()
            && (self.view_bounds.is_some()
                || self.figure.as_ref().is_some_and(|f| f.layout.is_some()))
            && self.figure.is_some()
        {
            self.export_settings.width = 1920;
            self.export_settings.height = 1080;
            if std::env::args().any(|a| a == "--matlab-export-demo") {
                self.export_settings.format = figure_export::Format::MatlabScript;
            }
            self.begin_figure_export();
            self.export_demo_pending = false;
            if std::env::args().any(|a| a == "--batch-export-demo") {
                self.figure_export.as_mut().unwrap().select_all();
                if let Some(folder) = std::env::var_os("MATRUST_BATCH_EXPORT_DEMO_FOLDER") {
                    self.prepare_export_folder(folder.into());
                }
            }
        }
        if !self.single_export.reviewing() {
            self.figure_export_ui(ctx);
        }
        self.single_export_ui(ctx);
        self.eps_preview_ui(ctx);
        self.preview_recovery_ui(ctx, pending);
        self.named_layout_ui(ctx);
        self.image_view_ui(ctx);
        self.original_copy_ui(ctx);
        if self.plot_data_demo_pending && !self.busy && self.variables.contains_key("y") {
            self.selected = Some("y".into());
            self.begin_plot_data();
            self.plot_data.as_mut().unwrap().x = Some("t".into());
            self.plot_data_demo_pending = false;
        }
        self.plot_data_ui(ctx);
        if let Some(editor) = &mut self.figure_editor {
            match editor.show(ctx) {
                figure_editor::Action::None => {}
                figure_editor::Action::Cancel => self.figure_editor = None,
                figure_editor::Action::Apply(mut edit) => {
                    edit.view_before = self.view_bounds.map(|v| (v, self.view_auto));
                    if !editor.reset_view {
                        edit.preserve_manual_view(self.view_bounds, self.view_auto);
                    }
                    self.submit(Job::EditFigure(edit));
                }
            }
        }
        self.workspace_import_ui(ctx);
        self.csv_import_ui(ctx);
        self.figure_close_ui(ctx);
        self.figure_conflict_ui(ctx);
        if !self.script_io.pending()
            && self.script_conflict.is_none()
            && self.figure_conflict.is_none()
            && !self.toolkits.model_file_modal_open()
            && (self.pending_quit || self.pending_close.is_some())
        {
            egui::Modal::new(egui::Id::new("unsaved_scripts")).show(root, |ui| {
                ui.label(crate::tokens::title(ui.ctx(), "Unsaved work"));
                ui.label("Save changes before closing?");
                if self.pending_quit && self.toolkits.model_dirty() {
                    ui.label("The block model has unsaved changes.");
                }
                ui.horizontal(|ui| {
                    let mut finish = false;
                    if ui.button("Save").clicked() {
                        self.save_pending_scripts(ctx);
                    }
                    if ui.button("Discard changes").clicked() {
                        if self.pending_quit {
                            for doc in &mut self.documents {
                                doc.discard();
                            }
                            self.toolkits.discard_model();
                        }
                        finish = true;
                    }
                    if ui.button("Cancel").clicked() {
                        self.pending_close = None;
                        self.pending_quit = false;
                        self.quit_discarded_figures.clear();
                    }
                    if finish {
                        if self.pending_quit {
                            self.allow_close = true;
                            ctx.send_viewport_cmd(egui::ViewportCommand::Close);
                        } else if let Some(i) = self.pending_close {
                            self.remove_document(i);
                        }
                    }
                });
                ui.label(&self.status);
            });
        }
        self.script_reload_ui(ctx);
        self.script_conflict_ui(ctx);
        self.script_saves_ui(ctx);
        self.global_search_ui(ctx);
        self.finish_global_search(ctx);
        self.finish_document_action(ctx);
        self.finish_script_reload(ctx);
        // Capture-only intent waits for restore/readiness and the final layout
        // pass, then uses the same queued action as File / New model.
        let capture_new = self.screenshot_model_new_pending
            && self.screenshot_path.is_some()
            && !ctx.will_discard()
            && self.can_new_model()
            && !self.folder_browser.loading;
        if capture_new {
            self.screenshot_model_new_pending = false;
            self.screenshot_ready_frames = 0;
            self.request_new_model();
        }
        self.finish_new_model(ctx);
        self.finish_script_run(ctx);
        self.detached_frame(ctx);
        if !ctx.will_discard()
            && self.detached_capture.is_none()
            && let Some(path) = &self.screenshot_path
        {
            let image = ctx.input(|i| {
                i.events.iter().find_map(|event| match event {
                    egui::Event::Screenshot { image, .. } => Some(image.clone()),
                    _ => None,
                })
            });
            if let Some(image) = image {
                if let Err(e) = save_screenshot(path, &image) {
                    eprintln!("Screenshot: {e}");
                }
                self.allow_close = true;
                ctx.send_viewport_cmd(egui::ViewportCommand::Close);
            } else if !self.busy
                && self.detached.capture_ready()
                && self.eps_capture.as_ref().is_none_or(|c| c.ready())
                && (self.script_save_queue.is_none() || self.script_conflict.is_some())
                && (self.open_queue.is_empty() || pending)
                && !self.folder_browser.loading
                && !self.screenshot_model_new_pending
                && !self.screenshot_requested
                && ctx.data(|d| {
                    d.get_temp::<u8>(egui::Id::new("legend_keyboard_capture"))
                        .is_none_or(|stage| stage >= 8)
                })
            {
                // Plot tick layout uses the preceding frame's bounds. Let it
                // settle after an asynchronous import before capturing evidence.
                self.screenshot_ready_frames += 1;
                if self.screenshot_ready_frames >= self.screenshot_ready_target {
                    eprintln!(
                        "Capture request: ready_frames={} frame={} selected_theme={:?} animation_time={} window_fill={:?}",
                        self.screenshot_ready_frames,
                        ctx.cumulative_frame_nr(),
                        ctx.theme(),
                        ctx.global_style().animation_time,
                        ctx.global_style().visuals.window_fill,
                    );
                    self.screenshot_requested = true;
                    ctx.send_viewport_cmd(egui::ViewportCommand::Screenshot(Default::default()));
                }
                ctx.request_repaint();
            }
        }
        self.finish_editor_columns(ctx);
        self.finish_folder_action(ctx);
        self.finish_folder_search(ctx);
        self.finish_pane_focus(ctx);
        self.finish_script_focus(ctx);
        self.navigation_help_ui(ctx);
        self.about_ui(ctx);
        self.update_window_title(ctx);
        self.finish_focus_areas(ctx);
        self.finish_preview_save(ctx);
        self.finish_workspace_csv(ctx);
        if let Some(mut capture) = self.detached_capture.take() {
            capture.frame(self, ctx);
            self.detached_capture = Some(capture);
        }
        if capture_new {
            eprintln!(
                "New model capture: nodes={} dirty={} modal={} pending_io={}",
                self.toolkits.model.nodes.len(),
                self.toolkits.model_dirty(),
                self.toolkits.model_file_modal_open(),
                self.toolkits.model_io_pending()
            );
        }
        if let Some(size) = layout::resized_panel(ctx, "workspace") {
            self.layout.sidebar_width = Some(size.x);
        }
        if self.navigation.workspace
            && !self.workspace_on_left()
            && let Some(size) = layout::resized_panel(ctx, "workspace_variables")
        {
            self.layout.workspace_width = Some(size.x);
        }
        if self.navigation.command
            && (!central_command || self.editor_above_command())
            && let Some(size) = layout::resized_panel(ctx, "console")
        {
            self.layout.console_height = Some(size.y);
        }
        self.finish_named_layout(ctx);
        self.finish_editor_window_action(ctx);
        self.editor_window_frame(ctx);
        self.eps_capture_frame(ctx);
        self.finish_workspace_mat_save(ctx);
        self.finish_root_open(ctx);
        self.finish_root_figure_save(ctx);
        self.finish_designer_save(ctx);
        self.finish_model_toolbar(ctx);
        self.finish_folder_browse(ctx);
        self.finish_csv_import_picker(ctx);
        self.finish_csv_naming_action(ctx);
        // Existing accepted rail/history/focus actions own navigation. Mirror
        // only its bounded visibility preference into normal recovery tracking.
        self.layout.panels = self.navigation.panels();
        self.layout.editor_columns = self.editor_columns_snapshot();
        self.layout.tools = self.toolkits.tab_layout();
        self.recovery_dirty |= self.layout != previous_layout;
        ctx.set_zoom_factor(f32::from(self.layout.ui_scale_percent()) / 100.);
        if self.recovery.is_some()
            && self.recovery_dirty
            && !self.recovering_session
            && !self.recovery_blocked
        {
            if Instant::now() >= self.recovery_due {
                if self
                    .recovery
                    .as_ref()
                    .is_some_and(|store| store.queue(self.snapshot()))
                {
                    self.recovery_dirty = false;
                }
                self.recovery_due = Instant::now() + Duration::from_secs(2);
            }
            ctx.request_repaint_after(self.recovery_due.saturating_duration_since(Instant::now()));
        }
    }
}
impl eframe::App for Workspace {
    fn raw_input_hook(&mut self, _ctx: &egui::Context, input: &mut egui::RawInput) {
        input::sanitize(input);
        self.observe_native_script_focus(input);
    }
    fn clear_color(&self, visuals: &egui::Visuals) -> [f32; 4] {
        visuals.panel_fill.to_normalized_gamma_f32()
    }
    fn ui(&mut self, root: &mut egui::Ui, _frame: &mut eframe::Frame) {
        self.ui(root);
    }
    fn on_exit(&mut self, _gl: Option<&eframe::glow::Context>) {
        self.flush_detached_placement();
        // Resource cleanup is not the user's Close preview action. Preserve the
        // small reference before releasing native helpers and image buffers.
        let preview = self.preview_checkpoint();
        // Release preview helpers and queued results, then join the worker
        // before the UI graphics/runtime is destroyed.
        self.close_image_preview();
        self.worker.shutdown();
        if let Some(store) = &self.recovery
            && !self.recovering_session
            && !self.recovery_blocked
        {
            let mut snapshot = self.exit_snapshot();
            snapshot.preview = preview;
            if let Err(e) = store.flush(snapshot) {
                eprintln!("Recovery: {e}");
            }
        }
    }
}
impl Drop for Workspace {
    fn drop(&mut self) {
        self.close_image_preview();
        self.worker.shutdown();
    }
}
fn workspace_type_label(value: &matrust::Array) -> &'static str {
    if value.is_complex() {
        "complex double"
    } else {
        value.class()
    }
}

fn scalar_cell_button(ui: &mut egui::Ui, value: matrust::Scalar, selected: bool) -> egui::Response {
    ui.add_sized(
        [90., 22.],
        egui::Button::new(RichText::new(format_scalar_cell(value)).monospace())
            .truncate()
            .selected(selected),
    )
    .on_hover_ui(|ui| {
        // Full Scalar Display is formatted only for an actual tooltip. It can
        // wrap within the tooltip without affecting the virtualized grid.
        ui.add(egui::Label::new(RichText::new(value.to_string()).monospace()).wrap());
    })
}

#[cfg(test)]
fn scalar_edit_literal(value: matrust::Scalar) -> String {
    let text = value.to_string();
    if text.len() <= 256 {
        return text;
    }
    // Display of binary64 MAX or a tiny subnormal may use hundreds of decimal
    // digits. LowerExp's default precision retains round-trip precision while
    // fitting the editor's bounded literal syntax. Copy/hover still use Display.
    match value {
        matrust::Scalar::Real(x) => format!("{x:e}"),
        matrust::Scalar::Logical(_) | matrust::Scalar::Integer(_) => text,
        matrust::Scalar::Complex(x) => format!(
            "{:e}{}{:e}i",
            x.re,
            if x.im.is_sign_negative() { "-" } else { "+" },
            x.im.abs(),
        ),
    }
}

fn format_scalar_cell(value: matrust::Scalar) -> String {
    match value {
        matrust::Scalar::Real(x) => format_cell(x),
        matrust::Scalar::Integer(x) => x.to_string(),
        matrust::Scalar::Logical(x) => format_cell(f64::from(x)),
        matrust::Scalar::Complex(x) => format!(
            "{}{}{}i",
            format_cell(x.re),
            if x.im.is_sign_negative() { "-" } else { "+" },
            format_cell(x.im.abs())
        ),
    }
}
fn format_cell(value: f64) -> String {
    if value == 0. {
        "0".into()
    } else if value.abs() >= 0.0001 && value.abs() < 1_000_000. {
        format!("{value:.6}")
            .trim_end_matches('0')
            .trim_end_matches('.')
            .to_owned()
    } else {
        format!("{value:.6e}")
    }
}
fn save_screenshot(
    path: &std::path::Path,
    image: &egui::ColorImage,
) -> Result<(), Box<dyn std::error::Error>> {
    let file = std::fs::File::create(path)?;
    let mut encoder = png::Encoder::new(
        std::io::BufWriter::new(file),
        image.width() as u32,
        image.height() as u32,
    );
    encoder.set_color(png::ColorType::Rgba);
    encoder.set_depth(png::BitDepth::Eight);
    let mut writer = encoder.write_header()?;
    let rgba: Vec<u8> = image.pixels.iter().flat_map(|p| p.to_array()).collect();
    writer.write_image_data(&rgba)?;
    Ok(())
}

fn main() -> eframe::Result {
    if std::env::args()
        .nth(1)
        .is_some_and(|a| a == "--version" || a == "-V")
    {
        #[cfg(windows)]
        {
            #[link(name = "kernel32")]
            unsafe extern "system" {
                fn AttachConsole(id: u32) -> i32;
            }
            // GUI-subsystem build: show the output in the parent console; failure is fine (piped).
            unsafe { AttachConsole(u32::MAX) };
        }
        // A closed pipe (PowerShell does not wait for GUI programs) must not panic.
        let _ = std::io::Write::write_fmt(
            &mut std::io::stdout(),
            format_args!("MatRust {}\n", env!("CARGO_PKG_VERSION")),
        );
        return Ok(());
    }
    let capture =
        screenshot_options::Options::parse(std::env::args().skip(1)).map_err(|message| {
            eframe::Error::AppCreation(
                std::io::Error::new(std::io::ErrorKind::InvalidInput, message).into(),
            )
        })?;
    let detached_capture = detached_figures::capture::Config::parse(
        std::env::args().skip(1).collect(),
        &capture,
        std::env::var_os("MATRUST_SESSION_PATH").map(std::path::PathBuf::from),
    )
    .map_err(|message| {
        eframe::Error::AppCreation(
            std::io::Error::new(std::io::ErrorKind::InvalidInput, message).into(),
        )
    })?;
    let eps_capture = eps_capture::Config::parse(std::env::args().skip(1).collect(), &capture)
        .map_err(|message| {
            eframe::Error::AppCreation(
                std::io::Error::new(std::io::ErrorKind::InvalidInput, message).into(),
            )
        })?;
    let threads = std::thread::available_parallelism()
        .map_or(1, usize::from)
        .min(8);
    if let Err(e) = matrust::initialize_worker_pool(threads) {
        eprintln!("Worker pool: {e}");
    }
    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_inner_size(if std::env::args().any(|a| a == "--compact-demo") {
                [960., 650.]
            } else {
                [1440., 900.]
            })
            .with_min_inner_size([960.0, 650.0])
            .with_title("MatRust")
            .with_icon(std::sync::Arc::new(chrome::window_icon())),
        ..Default::default()
    };
    eframe::run_native(
        "MatRust",
        options,
        Box::new(move |cc| {
            if let Some(theme) = capture.theme {
                cc.egui_ctx.set_theme(theme);
            }
            let mut workspace = Workspace::new(cc, capture.iir_designer_case);
            workspace.detached_capture = detached_capture
                .map(|config| config.start())
                .transpose()
                .map_err(|message| {
                    std::io::Error::new(std::io::ErrorKind::InvalidInput, message)
                })?;
            workspace.eps_capture = eps_capture
                .map(|config| config.start())
                .transpose()
                .map_err(|message| {
                    std::io::Error::new(std::io::ErrorKind::InvalidInput, message)
                })?;
            workspace.screenshot_theme_override = capture.theme;
            workspace.apply_desktop_theme(&cc.egui_ctx);
            workspace.screenshot_path = capture.path;
            workspace.screenshot_ready_target = capture.ready_frames;
            workspace.screenshot_model_new_pending = capture.model_new_demo;
            workspace.screenshot_figure_selector_popup = capture.figure_selector_demo;
            if capture.script_reload_demo {
                // Capture-only: show the real confirmation for the restored
                // private script, but never confirm, read, save or run it.
                workspace.reveal_editor();
                workspace.request_script_reload();
                eprintln!(
                    "Reload confirmation capture: open={} dirty={} pending_read={}",
                    workspace.script_reload.is_some(),
                    workspace.documents[workspace.active].dirty,
                    workspace.script_io.pending(),
                );
            }
            Ok(Box::new(workspace))
        }),
    )
}

#[cfg(test)]
mod ui_tests {
    use super::*;

    // Existing geometry/focus fixtures explicitly retain their old sample and
    // split arrangement. Fresh-start coverage below uses the real constructor.
    fn legacy_sample_layout() -> session::DesktopLayout {
        session::DesktopLayout {
            center: None,
            ..session::DesktopLayout::fresh_desktop()
        }
    }
    fn legacy_sample_desktop(ctx: egui::Context, isolated: bool) -> Workspace {
        let mut app = Workspace::with_context(ctx, isolated);
        app.layout = legacy_sample_layout();
        app.documents = vec![
            Document::open(std::path::Path::new("examples/signals.m"))
                .expect("Explicit historical UI sample fixture"),
        ];
        app.active = 0;
        app
    }

    #[test]
    fn isolated_iir_capture_startup_submits_selected_preview_without_model_changes() {
        use screenshot_options::IirDesignerCase;
        for case in [
            IirDesignerCase::Bandpass1,
            IirDesignerCase::Bandpass20,
            IirDesignerCase::Bandstop1,
            IirDesignerCase::Bandstop20,
        ] {
            let ctx = egui::Context::default();
            let mut app = Workspace::with_capture_context(ctx, true, Some(case));
            assert!(app.recovery.is_none());
            assert!(app.toolkits.designer_open);
            assert!(app.busy, "Selected preview uses the existing worker");
            let original_model = app.toolkits.model.clone();
            let documents: Vec<_> = app
                .documents
                .iter()
                .map(|d| (d.text.clone(), d.path.clone(), d.dirty))
                .collect();
            finish(&mut app);
            assert!(
                app.figure.is_some(),
                "Selected preview should produce a response plot: {}",
                app.status
            );
            let (band, order) = case.settings();
            let sos = app.variables["sos"].as_numeric().unwrap();
            assert_eq!((sos.rows(), sos.cols()), (order, 6));
            assert!(app.toolkits.preview_script().unwrap().contains(&format!(
                "butter_sos({order}, [0.2 0.4], '{}')",
                band.name()
            )));
            assert!(app.toolkits.model.same_storage(&original_model));
            assert_eq!(
                app.documents
                    .iter()
                    .map(|d| (d.text.clone(), d.path.clone(), d.dirty))
                    .collect::<Vec<_>>(),
                documents
            );
            assert!(app.recovery.is_none() && app.pending_close.is_none() && !app.pending_quit);
        }
    }

    include!("desktop/script_file_commands_tests.rs");
    include!("desktop/script_figure_files_tests.rs");
    include!("desktop/workspace_v73_save_tests.rs");
    include!("desktop/scale_tests.rs");
    include!("desktop/theme_animation_tests.rs");
    include!("desktop/theme_tests.rs");
    pub(crate) fn dropped_file(path: PathBuf) -> egui::DroppedFileHandle {
        #[derive(Debug)]
        struct TestFile(PathBuf);
        impl egui::DroppedFile for TestFile {
            fn path(&self) -> &std::path::Path {
                &self.0
            }
            fn bytes(&self) -> Result<Vec<u8>, String> {
                panic!("Desktop drops must queue paths for the existing bounded worker reader")
            }
        }
        Arc::new(TestFile(path))
    }

    include!("desktop/figure_recovery_tests.rs");
    include!("desktop/script_files_tests.rs");
    include!("desktop/script_io_tests.rs");
    include!("desktop/script_reload_tests.rs");
    include!("desktop/script_focus_tests.rs");
    include!("desktop/model_files_tests.rs");
    include!("desktop/model_read_tests.rs");
    include!("desktop/slx_import_tests.rs");
    include!("desktop/model_write_tests.rs");
    include!("desktop/figure_conflict_tests.rs");
    include!("desktop/figure_copy_tests.rs");
    include!("desktop/preview_recovery_tests.rs");
    include!("desktop/preview_navigation_tests.rs");
    include!("desktop/workspace_recovery_tests.rs");
    include!("desktop/workspace_tools_tests.rs");
    include!("desktop/complex_cell_layout_tests.rs");
    include!("desktop/vector_export_tests.rs");
    include!("desktop/open_queue_tests.rs");
    include!("desktop/script_actions_tests.rs");
    include!("desktop/script_run_tests.rs");
    include!("desktop/document_save_tests.rs");
    include!("desktop/document_close_tests.rs");
    include!("desktop/model_document_tests.rs");
    include!("desktop/tool_tabs_tests.rs");
    include!("desktop/model_new_tests.rs");
    include!("desktop/sample_inspector_tests.rs");
    include!("desktop/figure_save_all_tests.rs");
    include!("desktop/figure_selector_tests.rs");
    include!("desktop/path_label_ui_tests.rs");
    include!("desktop/folder_navigation_tests.rs");
    include!("desktop/shell_tests.rs");
    include!("desktop/chrome_density_tests.rs");
    include!("desktop/navigation_tests.rs");
    include!("desktop/editor_columns_tests.rs");
    include!("desktop/focus_areas_tests.rs");
    include!("desktop/workspace_mat_save_tests.rs");
    include!("desktop/editor_window_tests.rs");
    include!("desktop/named_layout_tests.rs");
    include!("desktop/folder_search_ui_tests.rs");
    include!("desktop/command_first_tests.rs");
    include!("desktop/global_search_tests.rs");
    include!("desktop/workspace_dock_tests.rs");
    include!("desktop/workspace_height_tests.rs");
    include!("desktop/figure_export_batch_tests.rs");
    include!("desktop/folder_browser_tests.rs");
    include!("desktop/folder_resolution_tests.rs");
    include!("desktop/open_files_tests.rs");
    include!("desktop/native_extension_tests.rs");
    include!("desktop/csv_import_tests.rs");
    include!("desktop/product_chrome_tests.rs");
    include!("desktop/redesign_tests.rs");
    #[test]
    fn layout_drag_preferences_survive_worker_save_restart_and_compact_clamping() {
        fn panel(ctx: &egui::Context, name: &str) -> egui::Rect {
            egui::containers::panel::PanelState::load(ctx, egui::Id::new(name))
                .unwrap()
                .outer_rect
        }
        fn drag(
            app: &mut Workspace,
            ctx: &egui::Context,
            size: egui::Vec2,
            from: egui::Pos2,
            to: egui::Pos2,
        ) {
            frame_at(app, ctx, size, vec![egui::Event::PointerMoved(from)]);
            pointer_frame(app, ctx, size, from, true);
            frame_at(app, ctx, size, vec![egui::Event::PointerMoved(to)]);
            pointer_frame(app, ctx, size, to, false);
            frame_at(app, ctx, size, vec![]);
        }
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("session.json");
        let size = egui::vec2(1440., 900.);
        let ctx = egui::Context::default();
        let mut app = legacy_sample_desktop(ctx.clone(), true);
        app.run("plot([0 1],[1 2]);".into());
        finish(&mut app);
        app.recovery = Some(recovery::Recovery::open(path.clone()).unwrap());
        app.recovery_dirty = false;
        app.recovery_due = Instant::now() + Duration::from_secs(3600);
        for _ in 0..3 {
            frame_at(&mut app, &ctx, size, vec![]);
        }
        assert_eq!(app.layout, legacy_sample_layout());
        let r = panel(&ctx, "workspace");
        let from = egui::pos2(r.right(), r.center().y);
        drag(&mut app, &ctx, size, from, from + egui::vec2(50., 0.));
        assert!(app.layout.sidebar_width.unwrap() > r.width() + 30.);
        let r = panel(&ctx, "console");
        let from = egui::pos2(r.center().x, r.top());
        drag(&mut app, &ctx, size, from, from - egui::vec2(0., 30.));
        assert!(app.layout.console_height.unwrap() > r.height() + 20.);
        let r = panel(&ctx, "editor_split");
        let from = egui::pos2(r.right(), r.center().y);
        drag(&mut app, &ctx, size, from, from + egui::vec2(55., 0.));
        assert!(app.layout.editor_fraction.unwrap() > 0.52);
        assert!(app.recovery_dirty);
        app.recovery_due = Instant::now();
        frame_at(&mut app, &ctx, size, vec![]);
        assert!(
            !app.recovery_dirty,
            "layout drag did not enter autosave queue"
        );
        let saved = app.layout;
        let expected = [
            panel(&ctx, "workspace"),
            panel(&ctx, "console"),
            panel(&ctx, "editor_split"),
        ];
        app.recovery
            .as_ref()
            .unwrap()
            .flush(app.snapshot())
            .unwrap();
        let new_ctx = egui::Context::default();
        let mut reopened = legacy_sample_desktop(new_ctx.clone(), true);
        reopened.restore_session(Session::load(&path).unwrap());
        reopened.start_session_recovery(path.clone());
        finish(&mut reopened);
        assert!(reopened.figure.is_some() && !reopened.recovering_session);
        for _ in 0..3 {
            frame_at(&mut reopened, &new_ctx, size, vec![]);
        }
        assert_eq!(reopened.layout, saved);
        for (name, expected) in ["workspace", "console", "editor_split"]
            .into_iter()
            .zip(expected)
        {
            let actual = panel(&new_ctx, name);
            assert!(
                (actual.width() - expected.width()).abs() < 2.
                    && (actual.height() - expected.height()).abs() < 2.,
                "{name}: {actual:?} != {expected:?}"
            );
        }
        // A smaller display clamps visible panels without rewriting saved preferences.
        let small_ctx = egui::Context::default();
        let mut small = legacy_sample_desktop(small_ctx.clone(), true);
        small.restore_session(Session::load(&path).unwrap());
        let compact = egui::vec2(960., 650.);
        for _ in 0..3 {
            frame_at(&mut small, &small_ctx, compact, vec![]);
        }
        let maximum = (compact.x * 0.25).max(workspace_tools::minimum_dock_width(&small_ctx));
        assert!(panel(&small_ctx, "workspace").width() <= maximum);
        // Clamped to 35 % of the compact window, not the saved wide height.
        assert!(panel(&small_ctx, "console").height() < 215.);
        assert_eq!(small.layout, saved);
        assert!(!small.recovery_dirty);
        // Completion choices must also fit at the console's minimum height,
        // keeping the visible divider aligned with its interactive handle.
        let r = panel(&small_ctx, "console");
        let from = egui::pos2(r.center().x, r.top());
        drag(
            &mut small,
            &small_ctx,
            compact,
            from,
            from + egui::vec2(0., 100.),
        );
        small.suggestions = vec!["abs".into(), "acos".into(), "asin".into(), "atan".into()];
        for _ in 0..3 {
            frame_at(&mut small, &small_ctx, compact, vec![]);
        }
        let handle = small_ctx
            .read_response(egui::Id::new("console").with("__resize"))
            .unwrap()
            .rect;
        let visible = panel(&small_ctx, "console");
        assert!(
            (visible.top() - handle.center().y).abs() < 1.,
            "console divider {visible:?}, handle {handle:?}"
        );
        small.suggestions.clear();
        click_label_at(&mut small, &small_ctx, compact, "Menu");
        click_label_at(&mut small, &small_ctx, compact, "Layout");
        click_label_at(&mut small, &small_ctx, compact, "Editor only");
        assert_eq!(small.layout.mode, layout::PaneMode::Editor);
        assert!(small.recovery_dirty);
        small.snapshot().save(&path).unwrap();
        let final_ctx = egui::Context::default();
        let mut final_app = legacy_sample_desktop(final_ctx.clone(), true);
        final_app.restore_session(Session::load(&path).unwrap());
        assert_eq!(final_app.layout.mode, layout::PaneMode::Editor);
        for _ in 0..3 {
            frame_at(&mut final_app, &final_ctx, size, vec![]);
        }
        assert!(
            egui::containers::panel::PanelState::load(&final_ctx, egui::Id::new("editor_split"))
                .is_none()
        );
        let original_text = final_app.documents[0].text.clone();
        click_label_at(&mut final_app, &final_ctx, size, "Menu");
        click_label_at(&mut final_app, &final_ctx, size, "Layout");
        click_label_at(&mut final_app, &final_ctx, size, "Reset layout");
        assert_eq!(
            final_app.layout,
            session::DesktopLayout {
                center: Some(session::CenterLayout {
                    command: true,
                    editor_revealed: true
                }),
                ..session::DesktopLayout::fresh_desktop()
            }
        );
        for _ in 0..3 {
            frame_at(&mut final_app, &final_ctx, size, vec![]);
        }
        assert!((panel(&final_ctx, "workspace").width() - tokens::LEFT_COLUMN).abs() < 1.);
        assert_eq!(final_app.documents[0].text, original_text);
        assert!(final_app.recovery_dirty);
    }
    #[test]
    fn mat_import_dialog_previews_selects_preserves_errors_and_cancels() {
        let ctx = egui::Context::default();
        let mut app = legacy_sample_desktop(ctx.clone(), true);
        app.run("compact=99;untouched=3;".into());
        finish(&mut app);
        app.open_path(PathBuf::from("tests/fixtures/mat/mat-import-v7.mat"));
        finish(&mut app);
        assert!(app.workspace_import.is_some());
        assert_eq!(
            app.variables["compact"].as_numeric().unwrap().data(),
            &[99.]
        );
        click_label(&mut app, &ctx, "Select supported");
        assert!(
            app.workspace_import
                .as_ref()
                .unwrap()
                .selected
                .contains("too_precise")
        );
        assert!(
            app.workspace_import
                .as_ref()
                .unwrap()
                .selected
                .contains("complex_data")
        );
        assert!(
            !app.workspace_import
                .as_ref()
                .unwrap()
                .selected
                .contains("sparse_data")
        );
        click_label(&mut app, &ctx, "Load selected");
        finish(&mut app);
        // int64 data beyond 2^53 now loads exactly as int64 instead of failing on precision.
        assert!(app.workspace_import.is_none());
        assert_eq!(app.variables["too_precise"].class(), "int64");
        assert_eq!(app.variables["compact"].as_numeric().unwrap().data(), &[7.]);
        app.open_path(PathBuf::from("tests/fixtures/mat/mat-import-v7.mat"));
        finish(&mut app);
        click_label(&mut app, &ctx, "Clear selection");
        app.workspace_import
            .as_mut()
            .unwrap()
            .selected
            .extend(["compact".into(), "complex_data".into()]);
        click_label(&mut app, &ctx, "Load selected");
        finish(&mut app);
        assert!(app.workspace_import.is_none());
        assert_eq!(app.variables["compact"].as_numeric().unwrap().data(), &[7.]);
        assert_eq!(
            app.variables["untouched"].as_numeric().unwrap().data(),
            &[3.]
        );
        let loaded = app.variables.len();
        let complex = app.variables["complex_data"].clone();
        assert_eq!(
            complex.as_complex().unwrap().data(),
            &[
                matrust::Complex64::new(1., 3.),
                matrust::Complex64::new(2., 4.)
            ]
        );
        app.open_path(PathBuf::from("tests/fixtures/mat/mat-import-v7.mat"));
        finish(&mut app);
        click_label(&mut app, &ctx, "Cancel");
        assert!(app.workspace_import.is_none());
        assert_eq!(app.variables.len(), loaded);
        assert!(complex.same_snapshot(&app.variables["complex_data"]));
    }
    #[test]
    fn figure_undo_redo_buttons_preserve_data_and_survive_axes_selection() {
        let ctx = egui::Context::default();
        let mut app = legacy_sample_desktop(ctx.clone(), true);
        app.run(include_str!("../examples/fig_layouts.m").into());
        finish(&mut app);
        frame(&mut app, &ctx, vec![]);
        let original = app.figure.clone().unwrap();
        let mut updated = original.clone();
        updated.title = "Changed title".into();
        updated.reposition_axes([0.15, 0.1, 0.7, 0.3]).unwrap();
        app.submit(Job::EditFigure(Box::new(worker::FigureEdit {
            view_before: None,
            id: app.current_figure,
            original: original.clone(),
            updated: updated.clone(),
        })));
        finish(&mut app);
        assert_eq!(app.figure_history[&1], (1, 0));
        click_label(&mut app, &ctx, "Actions");
        click_label(&mut app, &ctx, "Undo edit");
        finish(&mut app);
        assert!(original.same_snapshot(app.figure.as_ref().unwrap()));
        assert_eq!(app.figure_history[&1], (0, 1));
        click_label(&mut app, &ctx, "Axes 1");
        finish(&mut app);
        assert_eq!(app.figure_history[&1], (0, 1));
        click_label(&mut app, &ctx, "Actions");
        click_label(&mut app, &ctx, "Redo edit");
        finish(&mut app);
        assert!(updated.same_snapshot(app.figure.as_ref().unwrap()));
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("redo.mfig");
        app.submit(Job::SaveFigure(path.clone(), None));
        finish(&mut app);
        assert_eq!(
            matrust::files::load_figure(&path).unwrap().svg(),
            updated.svg()
        );
        click_label(&mut app, &ctx, "Actions");
        click_label(&mut app, &ctx, "Undo edit");
        finish(&mut app);
        assert!(app.figure_dirty(1));
        app.run("title('Script edit');".into());
        finish(&mut app);
        assert_eq!(app.figure_history[&1], (0, 0));
        click_label(&mut app, &ctx, "Actions");
        click_label(&mut app, &ctx, "Redo edit");
        assert!(!app.busy);
        assert_eq!(app.figure.as_ref().unwrap().title, "Script edit");
    }
    #[test]
    fn figure_properties_block_shortcuts_and_keep_cached_samples_for_style_edits() {
        let ctx = egui::Context::default();
        let mut app = legacy_sample_desktop(ctx.clone(), true);
        app.run("plot(1:10000,sin(1:10000));".into());
        finish(&mut app);
        app.view_bounds = Some([10., 20., -1., 1.]);
        let original = app.figure.clone().unwrap();
        app.figure_editor = Some(figure_editor::Editor::new(1, &original, app.view_bounds));
        app.documents[0].text = "unexpected_variable=1;".into();
        frame(&mut app, &ctx, vec![]);
        frame(
            &mut app,
            &ctx,
            vec![egui::Event::Key {
                key: egui::Key::Enter,
                physical_key: Some(egui::Key::Enter),
                pressed: true,
                repeat: false,
                modifiers: egui::Modifiers::CTRL,
            }],
        );
        assert!(!app.busy);
        assert!(!app.variables.contains_key("unexpected_variable"));
        press(&mut app, &ctx, egui::Key::Escape);
        assert!(app.figure_editor.is_none());
        assert!(app.figure.as_ref().unwrap().same_snapshot(&original));
        let cached = app.plot_points[0].as_ptr();
        let view = app.view_bounds;
        let mut updated = original.clone();
        updated.title = "Edited title".into();
        updated.series[0].color = [100, 20, 60];
        app.figure_editor = Some(figure_editor::Editor::new(1, &original, view));
        app.figure_editor.as_mut().unwrap().saving = true;
        app.submit(Job::EditFigure(Box::new(worker::FigureEdit {
            view_before: None,
            id: 1,
            original,
            updated,
        })));
        finish(&mut app);
        assert!(app.figure_editor.is_none());
        assert_eq!(app.figure.as_ref().unwrap().title, "Edited title");
        assert_eq!(app.plot_points[0].as_ptr(), cached);
        assert_eq!(app.view_bounds, view);
    }
    fn frame(
        app: &mut Workspace,
        ctx: &egui::Context,
        events: Vec<egui::Event>,
    ) -> egui::FullOutput {
        frame_at(app, ctx, egui::vec2(1440., 900.), events)
    }
    fn frame_at(
        app: &mut Workspace,
        ctx: &egui::Context,
        size: egui::Vec2,
        events: Vec<egui::Event>,
    ) -> egui::FullOutput {
        crate::headless::run(
            ctx,
            egui::RawInput {
                screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, size)),
                events: std::iter::once(egui::Event::ModifiersChanged(
                    events
                        .iter()
                        .find_map(|e| {
                            if let egui::Event::Key { modifiers, .. } = e {
                                Some(*modifiers)
                            } else {
                                None
                            }
                        })
                        .unwrap_or_default(),
                ))
                .chain(events)
                .collect(),
                ..Default::default()
            },
            |root| app.ui(root),
        )
    }
    fn text_position(shape: &egui::epaint::Shape, label: &str) -> Option<egui::Pos2> {
        match shape {
            egui::epaint::Shape::Text(t) if t.galley.text() == label => {
                Some(t.pos + t.galley.size() * 0.5)
            }
            egui::epaint::Shape::Vec(v) => v.iter().rev().find_map(|s| text_position(s, label)),
            _ => None,
        }
    }
    fn pointer_frame(
        app: &mut Workspace,
        ctx: &egui::Context,
        size: egui::Vec2,
        pos: egui::Pos2,
        pressed: bool,
    ) {
        frame_at(
            app,
            ctx,
            size,
            vec![
                egui::Event::PointerMoved(pos),
                egui::Event::PointerButton {
                    pos,
                    button: egui::PointerButton::Primary,
                    pressed,
                    modifiers: egui::Modifiers::NONE,
                },
            ],
        );
    }
    fn click_label(app: &mut Workspace, ctx: &egui::Context, label: &str) {
        click_label_at(app, ctx, egui::vec2(1440., 900.), label);
    }
    fn click_label_at(app: &mut Workspace, ctx: &egui::Context, size: egui::Vec2, label: &str) {
        let mut position = None;
        for _ in 0..3 {
            position = frame_at(app, ctx, size, vec![])
                .shapes
                .iter()
                .rev()
                .find_map(|s| text_position(&s.shape, label));
        }
        let pos = position.unwrap_or_else(|| panic!("Missing {label}"));
        for pressed in [true, false] {
            pointer_frame(app, ctx, size, pos, pressed);
        }
    }
    #[test]
    fn toolstrip_tabs_route_actions_without_replacing_work() {
        use matrust::session::ToolstripTab;
        let ctx = egui::Context::default();
        let mut app = legacy_sample_desktop(ctx.clone(), true);
        app.run("x=1:10;plot(x,x);title('Keep this figure');".into());
        finish(&mut app);
        let original = app.figure.clone().unwrap();
        let documents = app.documents.len();
        click_label(&mut app, &ctx, "New script");
        assert_eq!(app.documents.len(), documents + 1);
        app.documents[app.active].text = "answer=42;".into();
        click_label(&mut app, &ctx, "Run");
        finish(&mut app);
        assert_eq!(
            app.variables["answer"]
                .scalar(0, 0)
                .unwrap()
                .into_real()
                .unwrap(),
            42.
        );
        click_label(&mut app, &ctx, "Plots");
        assert_eq!(app.layout.toolstrip, ToolstripTab::Plots);
        click_label(&mut app, &ctx, "Figure properties");
        assert!(app.figure_editor.is_some());
        click_label(&mut app, &ctx, "Cancel");
        click_label(&mut app, &ctx, "Export figure");
        assert!(app.figure_export.is_some());
        click_label(&mut app, &ctx, "Cancel");
        app.layout.mode = layout::PaneMode::Editor;
        click_label(&mut app, &ctx, "2 by 2 layout");
        finish(&mut app);
        assert_eq!(app.layout.mode, layout::PaneMode::Split);
        assert_eq!(app.figure_ids, vec![1, 2]);
        app.run("figure(1);".into());
        finish(&mut app);
        assert!(original.same_snapshot(app.figure.as_ref().unwrap()));
        click_label(&mut app, &ctx, "Apps");
        click_label(&mut app, &ctx, "Filter Designer");
        assert!(app.toolkits.designer_open);
        app.toolkits.designer_open = false;
        click_label(&mut app, &ctx, "Block Simulation");
        assert!(app.toolkits.model_open);
        app.toolkits.model_open = false;
        let count = app.documents.len();
        click_label(&mut app, &ctx, "Filtering example");
        assert_eq!(app.documents.len(), count + 1);
        assert!(!app.busy);
        assert_eq!(
            app.documents[app.active].text,
            include_str!("../examples/streaming_filter.m")
        );
    }

    #[test]
    fn workspace_plot_dialog_retries_shapes_and_preserves_prior_figure() {
        let ctx = egui::Context::default();
        let mut app = legacy_sample_desktop(ctx.clone(), true);
        app.run("t=1:4;y=[1 2;3 4;5 6;7 8];bad=1:3;plot(t,t);title('Keep');".into());
        finish(&mut app);
        let original = app.figure.clone().unwrap();
        let input = app.variables["y"].as_numeric().unwrap().data().as_ptr();
        app.selected = Some("y".into());
        click_label(&mut app, &ctx, "Plots");
        click_label(&mut app, &ctx, "Plot");
        assert_eq!(app.plot_data.as_ref().unwrap().y, "y");
        click_label(&mut app, &ctx, "Sample index");
        click_label(&mut app, &ctx, "bad  (1 x 3, double)");
        click_label(&mut app, &ctx, "Create plot");
        finish(&mut app);
        assert!(
            app.plot_data
                .as_ref()
                .unwrap()
                .error
                .as_ref()
                .unwrap()
                .contains("length")
        );
        assert_eq!(app.figure_ids, vec![1]);
        assert!(original.same_snapshot(app.figure.as_ref().unwrap()));
        click_label(&mut app, &ctx, "bad");
        click_label(&mut app, &ctx, "t  (1 x 4, double)");
        click_label(&mut app, &ctx, "Line");
        click_label(&mut app, &ctx, "Stem");
        click_label(&mut app, &ctx, "Logarithmic Y");
        click_label(&mut app, &ctx, "Create plot");
        finish(&mut app);
        assert!(app.plot_data.is_none());
        assert_eq!(app.figure_ids, vec![1, 2]);
        let figure = app.figure.as_ref().unwrap();
        assert_eq!(figure.series.len(), 2);
        assert_eq!(figure.series[0].kind, matrust::plot::PlotKind::Stem);
        assert_eq!(figure.yscale, matrust::plot::AxisScale::Log);
        assert_eq!(
            figure.series[1]
                .points
                .iter()
                .collect::<Vec<_>>()
                .as_slice(),
            &[[1., 2.], [2., 4.], [3., 6.], [4., 8.]]
        );
        assert!(original.same_snapshot(&app.all_figures[&1]));
        assert_eq!(
            input,
            app.variables["y"].as_numeric().unwrap().data().as_ptr()
        );
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("workspace.mfig");
        app.submit(Job::SaveFigure(path.clone(), None));
        finish(&mut app);
        assert_eq!(
            format!("{:?}", matrust::files::load_figure(&path).unwrap()),
            format!("{:?}", app.figure.as_ref().unwrap())
        );
    }

    #[test]
    fn workspace_plot_dialog_script_cancel_and_modal_protection() {
        let ctx = egui::Context::default();
        let mut app = legacy_sample_desktop(ctx.clone(), true);
        app.run("y=[3 1 4];".into());
        finish(&mut app);
        app.begin_plot_data();
        let count = app.documents.len();
        click_label(&mut app, &ctx, "New script");
        assert_eq!(app.documents.len(), count);
        assert!(app.figure_ids.is_empty());
        click_label(&mut app, &ctx, "Cancel");
        assert!(app.plot_data.is_none());
        app.begin_plot_data();
        click_label(&mut app, &ctx, "Line");
        click_label(&mut app, &ctx, "Scatter");
        click_label(&mut app, &ctx, "Open plot script");
        assert!(app.plot_data.is_none());
        assert_eq!(app.documents.len(), count + 1);
        assert!(app.documents[app.active].text.contains("scatter(y)"));
        assert!(!app.busy);
        assert!(app.figure_ids.is_empty());
        app.run_document(None);
        finish(&mut app);
        assert_eq!(
            app.figure.as_ref().unwrap().series[0].kind,
            matrust::plot::PlotKind::Scatter
        );
    }

    #[test]
    fn workspace_plot_dialog_protects_close_but_honors_approved_exit() {
        let ctx = egui::Context::default();
        let mut app = legacy_sample_desktop(ctx.clone(), true);
        app.run("y=[1 2];".into());
        finish(&mut app);
        app.begin_plot_data();
        for approved in [false, true] {
            app.allow_close = approved;
            let mut input = egui::RawInput {
                screen_rect: Some(egui::Rect::from_min_size(
                    egui::Pos2::ZERO,
                    egui::vec2(960., 650.),
                )),
                ..Default::default()
            };
            input
                .viewports
                .get_mut(&egui::ViewportId::ROOT)
                .unwrap()
                .events
                .push(egui::ViewportEvent::Close);
            let output = crate::headless::run(&ctx, input, |root| app.ui(root));
            let cancelled = output.viewport_output[&egui::ViewportId::ROOT]
                .commands
                .iter()
                .any(|c| matches!(c, egui::ViewportCommand::CancelClose));
            assert_eq!(cancelled, !approved);
            assert!(app.plot_data.is_some());
            assert!(!app.pending_quit);
        }
    }

    #[test]
    #[cfg(windows)]
    fn page_preview_navigates_snapshot_reloads_and_preserves_workspace() {
        for (fixture, extension) in [
            ("pdf-preview/pages.pdf", "PDF"),
            ("tiff-preview/pages.tiff", "TIFF"),
        ] {
            let ctx = egui::Context::default();
            let mut app = legacy_sample_desktop(ctx.clone(), true);
            app.run("keep=[1 3 2];plot(keep);".into());
            finish(&mut app);
            let figure = app.figure.clone().unwrap();
            let pointer = app.variables["keep"].as_numeric().unwrap().data().as_ptr();
            let dir = tempfile::tempdir().unwrap();
            let path = dir.path().join(format!("pages.{extension}"));
            let source = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
                .join("tests/fixtures/preview")
                .join(fixture);
            std::fs::copy(&source, &path).unwrap();
            let input = egui::RawInput {
                screen_rect: Some(egui::Rect::from_min_size(
                    egui::Pos2::ZERO,
                    egui::vec2(960., 650.),
                )),
                dropped_files: vec![crate::ui_tests::dropped_file(path.clone())],
                ..Default::default()
            };
            let _ = crate::headless::run(&ctx, input, |root| app.ui(root));
            finish(&mut app);
            frame_at(&mut app, &ctx, egui::vec2(960., 650.), vec![]);
            let snapshot = app
                .image_view
                .as_ref()
                .unwrap()
                .page
                .as_ref()
                .unwrap()
                .document
                .clone();
            assert_eq!(snapshot.page_count, 3);
            click_label(&mut app, &ctx, "Next page");
            finish(&mut app);
            frame(&mut app, &ctx, vec![]);
            let page = app.image_view.as_ref().unwrap().page.as_ref().unwrap();
            assert_eq!(page.index, 1);
            assert!(Arc::ptr_eq(&snapshot, &page.document));
            std::fs::write(&path, b"broken").unwrap();
            app.image_view.as_mut().unwrap().page_number = 3;
            click_label(&mut app, &ctx, "Go to page");
            finish(&mut app);
            frame(&mut app, &ctx, vec![]);
            assert_eq!(
                app.image_view
                    .as_ref()
                    .unwrap()
                    .page
                    .as_ref()
                    .unwrap()
                    .index,
                2
            );
            click_label(&mut app, &ctx, "Reload image");
            finish(&mut app);
            assert!(app.image_view.as_ref().unwrap().error.is_some());
            assert_eq!(
                app.image_view
                    .as_ref()
                    .unwrap()
                    .page
                    .as_ref()
                    .unwrap()
                    .index,
                2
            );
            std::fs::copy(&source, &path).unwrap();
            click_label(&mut app, &ctx, "Reload image");
            finish(&mut app);
            frame(&mut app, &ctx, vec![]);
            assert!(app.image_view.as_ref().unwrap().error.is_none());
            assert!(!Arc::ptr_eq(
                &snapshot,
                &app.image_view
                    .as_ref()
                    .unwrap()
                    .page
                    .as_ref()
                    .unwrap()
                    .document
            ));
            click_label(&mut app, &ctx, "Previous page");
            finish(&mut app);
            frame(&mut app, &ctx, vec![]);
            assert_eq!(
                app.image_view
                    .as_ref()
                    .unwrap()
                    .page
                    .as_ref()
                    .unwrap()
                    .index,
                1
            );
            assert!(figure.same_snapshot(app.figure.as_ref().unwrap()));
            assert_eq!(
                app.variables["keep"].as_numeric().unwrap().data().as_ptr(),
                pointer
            );
            let current = Arc::downgrade(
                &app.image_view
                    .as_ref()
                    .unwrap()
                    .page
                    .as_ref()
                    .unwrap()
                    .document,
            );
            app.open_path(path);
            finish(&mut app);
            app.close_image_preview();
            finish(&mut app);
            frame(&mut app, &ctx, vec![]);
            assert!(
                app.image_view.is_none()
                    && app.pending_image.is_none()
                    && app.pending_page.is_none()
            );
            assert!(current.upgrade().is_none());
            let png = dir.path().join("replacement.png");
            image::RgbImage::new(32, 16).save(&png).unwrap();
            app.open_path(png);
            finish(&mut app);
            frame(&mut app, &ctx, vec![]);
            assert!(app.image_view.as_ref().unwrap().page.is_none());
        }
    }

    #[test]
    fn tiff_page_failure_preserves_current_view_and_can_navigate_back() {
        let ctx = egui::Context::default();
        let mut app = legacy_sample_desktop(ctx.clone(), true);
        app.open_path(
            std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
                .join("tests/fixtures/preview/tiff-preview/invalid-later.tiff"),
        );
        finish(&mut app);
        frame(&mut app, &ctx, vec![]);
        let snapshot = app
            .image_view
            .as_ref()
            .unwrap()
            .page
            .as_ref()
            .unwrap()
            .document
            .clone();
        let size = app.image_view.as_ref().unwrap().original_size;
        click_label(&mut app, &ctx, "Next page");
        finish(&mut app);
        frame(&mut app, &ctx, vec![]);
        let view = app.image_view.as_ref().unwrap();
        assert!(
            view.error
                .as_ref()
                .is_some_and(|e| e.contains("limit") || e.contains("8192"))
        );
        assert_eq!(view.original_size, size);
        assert_eq!(view.page.as_ref().unwrap().index, 0);
        assert!(Arc::ptr_eq(
            &snapshot,
            &view.page.as_ref().unwrap().document
        ));
        app.image_view.as_mut().unwrap().page_number = 1;
        click_label(&mut app, &ctx, "Go to page");
        finish(&mut app);
        frame(&mut app, &ctx, vec![]);
        assert!(app.image_view.as_ref().unwrap().error.is_none());
    }

    #[test]
    fn preview_save_and_copy_use_selected_page_and_preserve_workspace_and_source() {
        let ctx = egui::Context::default();
        let mut app = legacy_sample_desktop(ctx.clone(), true);
        app.run("keep=[1 3 2];plot(keep);".into());
        finish(&mut app);
        let figure = app.figure.clone().unwrap();
        let matrix = app.variables["keep"].as_numeric().unwrap().data().as_ptr();
        let dir = tempfile::tempdir().unwrap();
        let source = dir.path().join("pages.tiff");
        std::fs::copy(
            std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
                .join("tests/fixtures/preview/tiff-preview/pages.tiff"),
            &source,
        )
        .unwrap();
        let original = std::fs::read(&source).unwrap();
        app.submit(Job::OpenPagedImage(source.clone(), 1));
        finish(&mut app);
        frame(&mut app, &ctx, vec![]);
        let pixels = app.image_view.as_ref().unwrap().pixels.clone();
        assert_eq!(pixels.size, [160, 240]);
        let output = dir.path().join("saved.png");
        app.export_preview_to(output.clone());
        finish(&mut app);
        assert!(
            app.image_view
                .as_ref()
                .unwrap()
                .notice
                .as_ref()
                .unwrap()
                .contains("Saved preview")
        );
        let saved =
            image_load::load(&output, 4096, &std::sync::atomic::AtomicBool::new(false)).unwrap();
        assert_eq!(saved.pixels.pixels, pixels.pixels);
        assert!(Arc::ptr_eq(
            &pixels,
            &app.image_view.as_ref().unwrap().pixels
        ));
        // Capture every native egui output frame so even a fast worker's command
        // is observed on the release frame rather than lost by the click helper.
        let rendered = frame(&mut app, &ctx, vec![]);
        let pos = rendered
            .shapes
            .iter()
            .find_map(|s| text_position(&s.shape, "Copy image"))
            .unwrap();
        let mut outputs = Vec::new();
        for pressed in [true, false] {
            outputs.push(frame(
                &mut app,
                &ctx,
                vec![
                    egui::Event::PointerMoved(pos),
                    egui::Event::PointerButton {
                        pos,
                        button: egui::PointerButton::Primary,
                        pressed,
                        modifiers: egui::Modifiers::NONE,
                    },
                ],
            ));
        }
        finish(&mut app);
        outputs.push(frame(&mut app, &ctx, vec![]));
        let copied = outputs
            .iter()
            .flat_map(|o| &o.platform_output.commands)
            .find_map(|command| {
                if let egui::OutputCommand::CopyImage(image) = command {
                    Some(image)
                } else {
                    None
                }
            })
            .unwrap();
        assert_eq!(copied.size, pixels.size);
        assert_eq!(copied.pixels[0], Color32::from_rgb(255, 127, 127));
        assert!(copied.pixels.iter().all(|p| p.a() == 255));
        app.image_view.as_mut().unwrap().export_format = image_export::Format::Tiff;
        app.export_preview_to(source.clone());
        finish(&mut app);
        assert!(
            app.image_view
                .as_ref()
                .unwrap()
                .error
                .as_ref()
                .unwrap()
                .contains("different file")
        );
        assert_eq!(std::fs::read(&source).unwrap(), original);
        assert_eq!(
            app.variables["keep"].as_numeric().unwrap().data().as_ptr(),
            matrix
        );
        assert!(figure.same_snapshot(app.figure.as_ref().unwrap()));
        let final_path = dir.path().join("continued.tiff");
        app.export_preview_to(final_path.clone());
        app.close_image_preview();
        finish(&mut app);
        frame(&mut app, &ctx, vec![]);
        assert!(app.image_view.is_none() && app.pending_image.is_none());
        assert!(final_path.exists());
        assert_eq!(
            image_load::load(
                &final_path,
                4096,
                &std::sync::atomic::AtomicBool::new(false)
            )
            .unwrap()
            .pixels
            .pixels,
            pixels.pixels
        );
    }

    #[cfg(windows)]
    #[test]
    fn page_preview_exit_releases_snapshot_and_disconnects_worker() {
        let ctx = egui::Context::default();
        let mut app = legacy_sample_desktop(ctx.clone(), true);
        app.open_path(
            std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
                .join("tests/fixtures/preview/pdf-preview/pages.pdf"),
        );
        finish(&mut app);
        frame(&mut app, &ctx, vec![]);
        let document = Arc::downgrade(
            &app.image_view
                .as_ref()
                .unwrap()
                .page
                .as_ref()
                .unwrap()
                .document,
        );
        eframe::App::on_exit(&mut app, None);
        assert!(document.upgrade().is_none());
        assert!(
            app.worker
                .send
                .try_send(Job::Script("a=1;".into()).into())
                .is_err()
        );
        assert!(app.image_view.is_none() && app.pending_page.is_none());
        // Drop repeats cleanup safely after the framework's exit hook.
        drop(app);
    }

    #[test]
    fn svg_preview_open_reload_and_dropped_files_preserve_existing_documents() {
        let ctx = egui::Context::default();
        let mut app = legacy_sample_desktop(ctx.clone(), true);
        app.run("keep=[1 3 2];plot(keep);".into());
        finish(&mut app);
        frame(&mut app, &ctx, vec![]);
        let figure = app.figure.clone().unwrap();
        let pointer = app.variables["keep"].as_numeric().unwrap().data().as_ptr();
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("plot.SVG");
        std::fs::write(&path, figure.svg()).unwrap();
        let input = egui::RawInput {
            screen_rect: Some(egui::Rect::from_min_size(
                egui::Pos2::ZERO,
                egui::vec2(960., 650.),
            )),
            dropped_files: vec![crate::ui_tests::dropped_file(path.clone())],
            ..Default::default()
        };
        let _ = crate::headless::run(&ctx, input, |root| app.ui(root));
        finish(&mut app);
        frame_at(&mut app, &ctx, egui::vec2(960., 650.), vec![]);
        assert_eq!(app.image_view.as_ref().unwrap().original_size, [960, 560]);
        click_label(&mut app, &ctx, "1:1 preview");
        click_label(&mut app, &ctx, "Zoom in");
        std::fs::write(&path, "<svg/>").unwrap();
        click_label(&mut app, &ctx, "Reload image");
        finish(&mut app);
        assert!(app.image_view.as_ref().unwrap().error.is_some());
        assert_eq!(app.image_view.as_ref().unwrap().zoom, 1.25);
        std::fs::write(&path, figure.svg()).unwrap();
        click_label(&mut app, &ctx, "Reload image");
        finish(&mut app);
        frame(&mut app, &ctx, vec![]);
        assert!(app.image_view.as_ref().unwrap().error.is_none());
        assert_eq!(app.image_view.as_ref().unwrap().zoom, 1.25);
        assert!(figure.same_snapshot(app.figure.as_ref().unwrap()));
        assert_eq!(
            app.variables["keep"].as_numeric().unwrap().data().as_ptr(),
            pointer
        );
        app.open_path(path);
        finish(&mut app);
        app.close_image_preview();
        finish(&mut app);
        frame(&mut app, &ctx, vec![]);
        assert!(app.image_view.is_none() && app.pending_image.is_none());
    }

    #[test]
    fn raster_preview_open_reload_zoom_and_errors_preserve_workspace_and_figures() {
        let ctx = egui::Context::default();
        let mut app = legacy_sample_desktop(ctx.clone(), true);
        app.run("keep=42;plot([1 2 3]);title('Original');".into());
        finish(&mut app);
        frame(&mut app, &ctx, vec![]);
        let original = app.figure.clone().unwrap();
        let variable = app.variables["keep"].as_numeric().unwrap().data().as_ptr();
        let geometry = app.plot_points[0].as_ptr();
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("plot.PNG");
        image::RgbImage::from_pixel(128, 64, image::Rgb([10, 100, 220]))
            .save(&path)
            .unwrap();
        let tabs = app.documents.len();
        app.open_path(path.clone());
        finish(&mut app);
        frame(&mut app, &ctx, vec![]);
        assert_eq!(app.image_view.as_ref().unwrap().original_size, [128, 64]);
        assert!(app.image_view.as_ref().unwrap().fit);
        click_label(&mut app, &ctx, "1:1 preview");
        click_label(&mut app, &ctx, "Zoom in");
        assert_eq!(app.image_view.as_ref().unwrap().zoom, 1.25);
        image::RgbImage::from_pixel(256, 128, image::Rgb([220, 100, 10]))
            .save(&path)
            .unwrap();
        click_label(&mut app, &ctx, "Reload image");
        finish(&mut app);
        frame(&mut app, &ctx, vec![]);
        assert_eq!(app.image_view.as_ref().unwrap().original_size, [256, 128]);
        assert_eq!(app.image_view.as_ref().unwrap().zoom, 1.25);
        std::fs::write(&path, b"broken image").unwrap();
        click_label(&mut app, &ctx, "Reload image");
        finish(&mut app);
        assert_eq!(app.image_view.as_ref().unwrap().original_size, [256, 128]);
        assert!(app.image_view.as_ref().unwrap().error.is_some());
        assert!(original.same_snapshot(app.figure.as_ref().unwrap()));
        assert_eq!(
            app.variables["keep"].as_numeric().unwrap().data().as_ptr(),
            variable
        );
        assert_eq!(app.plot_points[0].as_ptr(), geometry);
        assert_eq!(app.documents.len(), tabs);
        click_label(&mut app, &ctx, "Close preview");
        assert!(app.image_view.is_none());
    }

    #[test]
    fn raster_preview_dropped_files_and_cancelled_results_do_not_reopen_closed_preview() {
        let ctx = egui::Context::default();
        let mut app = legacy_sample_desktop(ctx.clone(), true);
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("preview.bmp");
        image::RgbImage::new(64, 32).save(&path).unwrap();
        let input = egui::RawInput {
            screen_rect: Some(egui::Rect::from_min_size(
                egui::Pos2::ZERO,
                egui::vec2(960., 650.),
            )),
            dropped_files: vec![crate::ui_tests::dropped_file(path.clone())],
            ..Default::default()
        };
        let _ = crate::headless::run(&ctx, input, |root| app.ui(root));
        finish(&mut app);
        frame_at(&mut app, &ctx, egui::vec2(960., 650.), vec![]);
        assert_eq!(app.image_view.as_ref().unwrap().original_size, [64, 32]);
        assert!(app.figure_ids.is_empty());
        app.submit(Job::OpenImage(path));
        app.close_image_preview();
        finish(&mut app);
        frame_at(&mut app, &ctx, egui::vec2(960., 650.), vec![]);
        assert!(app.image_view.is_none() && app.pending_image.is_none());
        assert!(!app.image_loading);
    }

    #[test]
    fn toolstrip_compact_menus_and_collapsed_preference_survive_resize() {
        use matrust::session::ToolstripTab;
        let ctx = egui::Context::default();
        let mut app = legacy_sample_desktop(ctx.clone(), true);
        click_label(&mut app, &ctx, "Menu");
        click_label(&mut app, &ctx, "Layout");
        click_label(&mut app, &ctx, "Hide tools");
        assert!(app.layout.toolstrip_collapsed);
        click_label(&mut app, &ctx, "Apps");
        click_label(&mut app, &ctx, "Plotting example");
        assert_eq!(app.layout.toolstrip, ToolstripTab::Apps);
        assert_eq!(
            app.documents[app.active].text,
            include_str!("../examples/tiled_layout.m")
        );
        let small = egui::vec2(960., 650.);
        click_label_at(&mut app, &ctx, small, "Home");
        let count = app.documents.len();
        click_label_at(&mut app, &ctx, small, "New script");
        assert_eq!(app.documents.len(), count + 1);
        assert!(app.layout.toolstrip_collapsed);
        click_label_at(&mut app, &ctx, small, "Menu");
        click_label_at(&mut app, &ctx, small, "Layout");
        click_label_at(&mut app, &ctx, small, "Show tools");
        assert!(!app.layout.toolstrip_collapsed);
        click_label(&mut app, &ctx, "Plots");
        for _ in 0..3 {
            frame_at(&mut app, &ctx, small, vec![]);
        }
        assert!(!app.layout.toolstrip_collapsed);
        assert_eq!(app.layout.toolstrip, ToolstripTab::Plots);
        click_label_at(&mut app, &ctx, small, "Home");
        click_label_at(&mut app, &ctx, small, "More");
        click_label_at(&mut app, &ctx, small, "Layout");
        click_label_at(&mut app, &ctx, small, "Reset layout");
        assert_eq!(app.layout.toolstrip, ToolstripTab::Home);
        assert!(!app.layout.toolstrip_collapsed);
    }

    #[test]
    fn toolstrip_figure_history_actions_and_pending_dialog_protection() {
        let ctx = egui::Context::default();
        let mut app = legacy_sample_desktop(ctx.clone(), true);
        app.run("plot(1:4);".into());
        finish(&mut app);
        let original = app.figure.clone().unwrap();
        let mut updated = original.clone();
        updated.title = "Edited through history".into();
        app.submit(Job::EditFigure(Box::new(worker::FigureEdit {
            id: app.current_figure,
            original: original.clone(),
            updated: updated.clone(),
            view_before: None,
        })));
        finish(&mut app);
        click_label(&mut app, &ctx, "Plots");
        click_label(&mut app, &ctx, "Undo figure edit");
        finish(&mut app);
        assert!(original.same_snapshot(app.figure.as_ref().unwrap()));
        click_label(&mut app, &ctx, "Redo figure edit");
        finish(&mut app);
        assert!(updated.same_snapshot(app.figure.as_ref().unwrap()));
        // Nothing left to redo: the action is hidden, not greyed.
        assert!(
            frame(&mut app, &ctx, vec![])
                .shapes
                .iter()
                .all(|s| text_position(&s.shape, "Redo figure edit").is_none())
        );
        assert!(!app.busy);
        click_label(&mut app, &ctx, "Figure properties");
        click_label(&mut app, &ctx, "New figure");
        assert!(!app.busy);
        assert_eq!(app.figure_ids, vec![1]);
        assert!(app.figure_editor.is_some());
        click_label(&mut app, &ctx, "Cancel");
    }

    #[test]
    fn compact_layout_resizes_expands_and_preserves_zoom_and_script() {
        let ctx = egui::Context::default();
        let mut app = legacy_sample_desktop(ctx.clone(), true);
        // This regression deliberately exercises the two-sidebar compact tab route.
        app.layout.workspace_dock = None;
        assert!(!app.workspace_on_left());
        app.run("plot(1:100,sin(1:100));title('Compact layout');xlabel('Time');ylabel('Signal');legend('Data');".into());
        finish(&mut app);
        let script = app.documents[app.active].text.clone();
        let plot_id = egui::Id::new("workspace_figure");
        let size = egui::vec2(1440., 900.);
        let compact = egui::vec2(960., 650.);
        // Preserve real splitter interaction in a viewport where both panes fit.
        for _ in 0..3 {
            frame_at(&mut app, &ctx, size, vec![]);
        }
        for _ in 0..3 {
            frame_at(&mut app, &ctx, size, vec![]);
        }
        let memory = egui_plot::PlotMemory::load(&ctx, plot_id).unwrap();
        let plot = *memory.transform().frame();
        assert!(plot.height() >= 200., "compact plot: {plot:?}");
        assert!(plot.right() <= size.x && plot.bottom() < size.y);
        let panel = egui::containers::panel::PanelState::load(&ctx, egui::Id::new("editor_split"))
            .unwrap()
            .outer_rect;
        assert!(panel.height() >= 300. && panel.width() >= 280., "{panel:?}");
        // Actual pointer drag adjusts the splitter while both panes remain usable.
        let pos = egui::pos2(panel.right(), panel.center().y);
        frame_at(&mut app, &ctx, size, vec![egui::Event::PointerMoved(pos)]);
        pointer_frame(&mut app, &ctx, size, pos, true);
        let moved = pos - egui::vec2(45., 0.);
        frame_at(&mut app, &ctx, size, vec![egui::Event::PointerMoved(moved)]);
        pointer_frame(&mut app, &ctx, size, moved, false);
        frame_at(&mut app, &ctx, size, vec![]);
        let resized =
            egui::containers::panel::PanelState::load(&ctx, egui::Id::new("editor_split"))
                .unwrap()
                .outer_rect;
        assert!(
            resized.width() < panel.width() - 20.,
            "splitter did not move: {panel:?} -> {resized:?}"
        );
        let mut memory = egui_plot::PlotMemory::load(&ctx, plot_id).unwrap();
        let bounds = egui_plot::PlotBounds::from_min_max([20., -0.5], [40., 0.5]);
        memory.set_bounds(bounds);
        memory.auto_bounds = [false, false].into();
        memory.store(&ctx, plot_id);
        frame_at(&mut app, &ctx, size, vec![]);
        // The figure card's Expand is in its overflow menu (painted last).
        click_label_at(&mut app, &ctx, size, "Actions");
        click_label_at(&mut app, &ctx, size, "Expand");
        assert_eq!(app.layout.mode, layout::PaneMode::Figure);
        frame_at(&mut app, &ctx, size, vec![]);
        let memory = egui_plot::PlotMemory::load(&ctx, plot_id).unwrap();
        assert!(memory.transform().frame().width() > plot.width() + 200.);
        assert_eq!(memory.bounds(), &bounds);
        click_label_at(&mut app, &ctx, size, "Actions");
        click_label_at(&mut app, &ctx, size, "Restore split");
        assert_eq!(app.layout.mode, layout::PaneMode::Split);
        frame_at(&mut app, &ctx, size, vec![]);
        assert_eq!(
            egui_plot::PlotMemory::load(&ctx, plot_id).unwrap().bounds(),
            &bounds
        );
        click_label_at(&mut app, &ctx, size, "Menu");
        click_label_at(&mut app, &ctx, size, "Layout");
        click_label_at(&mut app, &ctx, size, "Editor only");
        assert_eq!(app.layout.mode, layout::PaneMode::Editor);
        click_label_at(&mut app, &ctx, size, "Editor actions");
        click_label_at(&mut app, &ctx, size, "Restore split");
        assert_eq!(app.layout.mode, layout::PaneMode::Split);
        assert_eq!(app.documents[app.active].text, script);
        let split_preference = app.layout.editor_fraction;
        for _ in 0..3 {
            frame_at(&mut app, &ctx, compact, vec![]);
        }
        assert!(
            egui::containers::panel::PanelState::load(&ctx, egui::Id::new("editor_split"))
                .is_none()
        );
        click_label_at(&mut app, &ctx, compact, "Plot figure");
        assert!(app.compact_figure);
        for _ in 0..3 {
            frame_at(&mut app, &ctx, compact, vec![]);
        }
        let memory = egui_plot::PlotMemory::load(&ctx, plot_id).unwrap();
        assert_eq!(memory.bounds(), &bounds);
        assert!(
            memory.transform().frame().height() >= 200.,
            "Compact useful data height: frame={:?}, viewport={:?}, requested={compact:?}, zoom={}, console={:?}",
            memory.transform().frame(),
            ctx.content_rect(),
            ctx.zoom_factor(),
            egui::containers::panel::PanelState::load(&ctx, egui::Id::new("console"))
                .map(|panel| panel.outer_rect)
        );
        assert!(
            egui::Rect::from_min_size(egui::Pos2::ZERO, compact)
                .contains_rect(*memory.transform().frame())
        );
        click_label_at(&mut app, &ctx, compact, "Actions");
        click_label_at(&mut app, &ctx, compact, "Expand");
        assert_eq!(app.layout.mode, layout::PaneMode::Figure);
        click_label_at(&mut app, &ctx, compact, "Actions");
        click_label_at(&mut app, &ctx, compact, "Restore split");
        click_label_at(&mut app, &ctx, compact, "Script editor");
        assert!(!app.compact_figure);
        assert_eq!(app.layout.editor_fraction, split_preference);
        assert_eq!(app.documents[app.active].text, script);

        app.layout.mode = layout::PaneMode::Figure;
        app.open_path(PathBuf::from("examples/signals.m"));
        finish(&mut app);
        assert_eq!(app.layout.mode, layout::PaneMode::Split);
        app.layout.mode = layout::PaneMode::Figure;
        frame_at(
            &mut app,
            &ctx,
            size,
            vec![egui::Event::Key {
                key: egui::Key::F,
                physical_key: Some(egui::Key::F),
                pressed: true,
                repeat: false,
                modifiers: egui::Modifiers::CTRL,
            }],
        );
        assert_eq!(app.layout.mode, layout::PaneMode::Split);
        app.layout.mode = layout::PaneMode::Figure;
        app.new_document("new_buffer = 1;");
        assert_eq!(app.layout.mode, layout::PaneMode::Split);
    }
    #[test]
    fn compact_menus_reach_tools_figure_properties_and_view_controls() {
        for right in [false, true] {
            let ctx = egui::Context::default();
            let mut app = legacy_sample_desktop(ctx.clone(), true);
            if right {
                app.layout.workspace_dock = None;
            }
            assert_eq!(app.workspace_on_left(), !right);
            let size = egui::vec2(960., 650.);
            app.run("plot([1 2 3]);".into());
            finish(&mut app);
            click_label_at(&mut app, &ctx, size, "Menu");
            click_label_at(&mut app, &ctx, size, "Tools");
            click_label_at(&mut app, &ctx, size, "Filter Designer");
            assert!(app.toolkits.designer_open);
            app.toolkits.designer_open = false;
            click_label_at(&mut app, &ctx, size, "Menu");
            click_label_at(&mut app, &ctx, size, "Tools");
            click_label_at(&mut app, &ctx, size, "Block Simulation");
            assert!(app.toolkits.model_open);
            app.toolkits.model_open = false;
            if right {
                click_label_at(&mut app, &ctx, size, "Plot figure");
                assert!(app.compact_figure);
            } else {
                // Fresh Below Files keeps the editor and figure simultaneously visible.
                let output = frame_at(&mut app, &ctx, size, vec![]);
                assert!(
                    egui::containers::panel::PanelState::load(&ctx, egui::Id::new("editor_split"))
                        .is_some()
                );
                assert!(
                    !output
                        .shapes
                        .iter()
                        .any(|s| text_position(&s.shape, "Plot figure").is_some())
                );
                assert!(
                    egui_plot::PlotMemory::load(&ctx, egui::Id::new("workspace_figure")).is_some()
                );
            }
            click_label_at(&mut app, &ctx, size, "Menu");
            click_label_at(&mut app, &ctx, size, "File");
            click_label_at(&mut app, &ctx, size, "Figures");
            click_label_at(&mut app, &ctx, size, "Properties\u{2026}");
            assert!(app.figure_editor.is_some());
            app.figure_editor = None;
            click_label_at(&mut app, &ctx, size, "Actions");
            click_label_at(&mut app, &ctx, size, "View options");
            let label = if app.figure.as_ref().unwrap().xgrid {
                "Grid: on"
            } else {
                "Grid: off"
            };
            let before = [
                app.figure.as_ref().unwrap().xgrid,
                app.figure.as_ref().unwrap().ygrid,
            ];
            click_label_at(&mut app, &ctx, size, label);
            finish(&mut app);
            assert_ne!(
                [
                    app.figure.as_ref().unwrap().xgrid,
                    app.figure.as_ref().unwrap().ygrid
                ],
                before
            );
        }
    }
    #[test]
    fn figure_close_cancel_save_and_failed_save_use_actual_dialog() {
        let ctx = egui::Context::default();
        let mut app = legacy_sample_desktop(ctx.clone(), true);
        let dir = tempfile::tempdir().unwrap();
        let sub = dir.path().join("existing");
        std::fs::create_dir(&sub).unwrap();
        let path = sub.join("figure.mfig");
        app.run("plot([1 2 3]);".into());
        finish(&mut app);
        app.submit(Job::SaveFigure(path.clone(), None));
        finish(&mut app);
        app.run("title('Edited');".into());
        finish(&mut app);
        app.request_figure_close(1);
        click_label(&mut app, &ctx, "Cancel");
        assert!(app.figure_close.is_none());
        assert!(app.figure.is_some());
        assert!(app.figure_dirty(1));
        std::fs::remove_file(&path).unwrap();
        std::fs::remove_dir(&sub).unwrap();
        app.request_figure_close(1);
        click_label(&mut app, &ctx, "Save");
        finish(&mut app);
        assert!(!app.figure_close.as_ref().unwrap().saving);
        assert!(app.figure_dirty(1));
        assert!(app.figure.is_some());
        assert!(app.figure_conflict.is_some());
        assert!(!path.exists());
        click_label(&mut app, &ctx, "Overwrite figure");
        finish(&mut app);
        assert!(app.figure_conflict.is_some() && app.figure_dirty(1));
        assert!(!path.exists(), "Failed save must not publish a file");
        std::fs::create_dir(&sub).unwrap();
        click_label(&mut app, &ctx, "Overwrite figure");
        finish(&mut app);
        frame(&mut app, &ctx, vec![]);
        finish(&mut app);
        assert!(app.figure.is_none());
        assert!(app.figure_close.is_none());
        assert_eq!(matrust::files::load_figure(&path).unwrap().title, "Edited");
    }
    #[test]
    fn application_quit_saves_all_figures_before_prompting_for_scripts() {
        let ctx = egui::Context::default();
        let mut app = legacy_sample_desktop(ctx.clone(), true);
        let dir = tempfile::tempdir().unwrap();
        for id in 1..=2 {
            app.run(format!("figure({id});plot([1 2 3]);"));
            finish(&mut app);
            app.submit(Job::SaveFigure(dir.path().join(format!("{id}.mfig")), None));
            finish(&mut app);
        }
        app.run("figure(1);title('First');figure(2);title('Second');".into());
        finish(&mut app);
        app.documents[0].dirty = true;
        let mut input = egui::RawInput::default();
        input
            .viewports
            .entry(egui::ViewportId::ROOT)
            .or_default()
            .events
            .push(egui::ViewportEvent::Close);
        let out = crate::headless::run(&ctx, input, |root| app.ui(root));
        assert!(
            out.viewport_output[&egui::ViewportId::ROOT]
                .commands
                .iter()
                .any(|c| matches!(c, egui::ViewportCommand::CancelClose))
        );
        assert!(app.figure_close.is_some());
        assert!(!app.allow_close);
        click_label(&mut app, &ctx, "Save all");
        finish(&mut app);
        assert_eq!(app.current_figure, 2);
        frame(&mut app, &ctx, vec![]);
        finish(&mut app);
        frame(&mut app, &ctx, vec![]);
        assert!(app.figure_close.is_none());
        assert!(app.pending_quit);
        assert!(!app.allow_close);
        assert!(!app.figure_dirty(1));
        assert!(!app.figure_dirty(2));
        assert_eq!(
            matrust::files::load_figure(&dir.path().join("1.mfig"))
                .unwrap()
                .title,
            "First"
        );
        assert_eq!(
            matrust::files::load_figure(&dir.path().join("2.mfig"))
                .unwrap()
                .title,
            "Second"
        );
        click_label(&mut app, &ctx, "Cancel");
        assert!(!app.pending_quit);
        assert!(app.documents[0].dirty);
        app.run("title('Discard this edit');".into());
        finish(&mut app);
        app.request_figure_close(2);
        click_label(&mut app, &ctx, "Discard changes");
        finish(&mut app);
        assert_eq!(app.figure_ids, vec![1]);
    }
    #[test]
    fn saved_figure_views_stay_clean_and_manual_zoom_survives_switching() {
        let ctx = egui::Context::default();
        let mut app = legacy_sample_desktop(ctx.clone(), true);
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("view.mfig");
        app.run("plot(1:100,sin(1:100));".into());
        finish(&mut app);
        for _ in 0..3 {
            frame(&mut app, &ctx, vec![]);
        }
        app.submit(Job::SaveFigure(path.clone(), app.view_bounds));
        finish(&mut app);
        for _ in 0..3 {
            frame(&mut app, &ctx, vec![]);
        }
        assert!(!app.figure_dirty(1));
        let view = [20., 40., -0.5, 0.5];
        app.record_figure_view(view, [false, false]);
        assert!(app.figure_dirty(1));
        app.run("figure(2);plot([3 4 5]);".into());
        finish(&mut app);
        frame(&mut app, &ctx, vec![]);
        app.run("figure(1);".into());
        finish(&mut app);
        for _ in 0..3 {
            frame(&mut app, &ctx, vec![]);
        }
        for (a, b) in app.view_bounds.unwrap().into_iter().zip(view) {
            assert!((a - b).abs() < 1e-10, "{a} != {b}");
        }
        assert!(app.figure_dirty(1));
        assert!(app.save_figure_document(1, false));
        finish(&mut app);
        for _ in 0..3 {
            frame(&mut app, &ctx, vec![]);
        }
        assert!(!app.figure_dirty(1));
        let saved = matrust::files::load_figure(&path).unwrap();
        assert_eq!(saved.xlim, Some([20., 40.]));
        // An automatic Y-axis update must not dirty an unchanged manual X view.
        app.figure_views.get_mut(&1).unwrap().auto = [false, true];
        app.record_figure_view([20., 40., -2., 2.], [false, true]);
        assert!(!app.figure_dirty(1));
        app.request_figure_close(1);
        assert!(app.figure_close.is_none());
        finish(&mut app);
        assert_eq!(app.figure_ids, vec![2]);
    }
    fn press(app: &mut Workspace, ctx: &egui::Context, key: egui::Key) {
        frame(
            app,
            ctx,
            vec![egui::Event::Key {
                key,
                physical_key: Some(key),
                pressed: true,
                repeat: false,
                modifiers: egui::Modifiers::NONE,
            }],
        );
        frame(
            app,
            ctx,
            vec![egui::Event::Key {
                key,
                physical_key: Some(key),
                pressed: false,
                repeat: false,
                modifiers: egui::Modifiers::NONE,
            }],
        );
    }
    fn focus_command(app: &Workspace, ctx: &egui::Context) {
        let id = egui::Id::new("command_input");
        ctx.memory_mut(|m| m.request_focus(id));
        let mut state = egui::text_edit::TextEditState::load(ctx, id).unwrap_or_default();
        state
            .cursor
            .set_char_range(Some(egui::text::CCursorRange::one(
                egui::text::CCursor::new(app.command.chars().count()),
            )));
        state.store(ctx, id);
    }
    fn finish(app: &mut Workspace) {
        let deadline = Instant::now() + Duration::from_secs(3);
        while app.busy && Instant::now() < deadline {
            app.receive();
            std::thread::sleep(Duration::from_millis(1));
        }
        assert!(!app.busy, "Worker did not finish: {}", app.status);
    }
    #[test]
    fn command_cd_updates_files_folder_and_next_pwd() {
        let temp = tempfile::Builder::new()
            .prefix(".envfiles-desktop-")
            .tempdir_in(env!("CARGO_MANIFEST_DIR"))
            .unwrap();
        let destination = temp.path().join("next");
        std::fs::create_dir(&destination).unwrap();
        std::fs::write(destination.join("visible.txt"), "test").unwrap();
        let destination = destination.canonicalize().unwrap();
        let ctx = egui::Context::default();
        let mut app = legacy_sample_desktop(ctx.clone(), true);
        app.folder = temp.path().canonicalize().unwrap();
        frame(&mut app, &ctx, vec![]);
        app.command = format!(
            "cd('{}')",
            folder_navigation::address_text(&destination).replace('\'', "''")
        );
        focus_command(&app, &ctx);
        press(&mut app, &ctx, egui::Key::Enter);
        finish(&mut app);
        assert_eq!(app.folder, destination);
        assert_eq!(
            app.folder_text,
            folder_navigation::address_text(&destination)
        );
        let deadline = Instant::now() + Duration::from_secs(3);
        while app.folder_browser.loading && Instant::now() < deadline {
            app.receive_folder();
            std::thread::sleep(Duration::from_millis(1));
        }
        assert!(!app.folder_browser.loading);
        assert!(
            app.entries
                .iter()
                .any(|entry| entry.path == destination.join("visible.txt"))
        );
        app.command = "pwd".into();
        focus_command(&app, &ctx);
        press(&mut app, &ctx, egui::Key::Enter);
        finish(&mut app);
        assert!(
            app.console
                .iter()
                .any(|line| line.contains(&folder_navigation::address_text(&destination)))
        );
    }

    #[test]
    fn logical_workspace_editor_and_mat_exchange_preserve_types_and_aliases() {
        let ctx = egui::Context::default();
        let mut app = legacy_sample_desktop(ctx.clone(), true);
        app.run("mask=logical([0 1 0]);original=mask;numeric=7;".into());
        finish(&mut app);
        assert!(app.variables["mask"].is_logical());
        assert_eq!(app.variables["mask"].bytes(), 3);
        app.inspect_variable("mask".into());
        frame(&mut app, &ctx, vec![]);
        app.edit_value = "true".into();
        click_label(&mut app, &ctx, "Apply cell");
        finish(&mut app);
        assert_eq!(
            app.variables["mask"]
                .scalar(0, 0)
                .unwrap()
                .into_real()
                .unwrap(),
            1.
        );
        assert_eq!(
            app.variables["original"]
                .scalar(0, 0)
                .unwrap()
                .into_real()
                .unwrap(),
            0.
        );
        // Adopt the completed rooted edit before entering the next literal.
        frame(&mut app, &ctx, vec![]);
        app.edit_value = "NaN".into();
        click_label(&mut app, &ctx, "Apply cell");
        finish(&mut app);
        assert!(app.cell_error.as_ref().is_some_and(|e| e.contains("NaN")));
        assert_eq!(
            app.variables["mask"]
                .scalar(0, 0)
                .unwrap()
                .into_real()
                .unwrap(),
            1.
        );
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("logical.mat");
        app.submit(Job::SaveWorkspace(path.clone()));
        finish(&mut app);
        app.run("clear;".into());
        finish(&mut app);
        app.editing_variable = None;
        app.open_path(path);
        finish(&mut app);
        click_label(&mut app, &ctx, "Select supported");
        click_label(&mut app, &ctx, "Load selected");
        finish(&mut app);
        assert!(app.variables["mask"].is_logical());
        assert!(!app.variables["numeric"].is_logical());
        assert_eq!(app.variables["mask"].bytes(), 3);
        let csv = dir.path().join("mask.csv");
        app.submit(Job::ExportVariable(csv.clone(), "mask".into()));
        finish(&mut app);
        assert_eq!(std::fs::read_to_string(csv).unwrap(), "1,1,0\n");
        app.run("mask=9;".into());
        finish(&mut app);
        assert!(!app.variables["mask"].is_logical());
        assert!(matrust::session::FUNCTIONS.contains(&"logical"));
    }
    #[test]
    fn command_keyboard_completion_history_and_execution() {
        let ctx = egui::Context::default();
        let mut app = legacy_sample_desktop(ctx.clone(), true);
        frame(&mut app, &ctx, vec![]);
        app.command = "linsp".into();
        focus_command(&app, &ctx);
        press(&mut app, &ctx, egui::Key::ArrowUp);
        assert_eq!(app.command, "linsp");
        assert!(ctx.memory(|m| m.has_focus(egui::Id::new("command_input"))));
        press(&mut app, &ctx, egui::Key::Tab);
        assert_eq!(app.command, "linspace");
        app.command = "s".into();
        focus_command(&app, &ctx);
        press(&mut app, &ctx, egui::Key::Tab);
        assert!(app.suggestions.len() > 1);
        app.history.push("a=1".into());
        app.history.push("b=2".into());
        app.command = "unfinished(".into();
        focus_command(&app, &ctx);
        press(&mut app, &ctx, egui::Key::ArrowUp);
        assert_eq!(app.command, "b=2");
        assert!(
            app.suggestions.is_empty(),
            "History recall must invalidate completion ranges"
        );
        assert!(
            ctx.memory(|m| m.has_focus(egui::Id::new("command_input"))),
            "History recall lost command focus: {:?}",
            ctx.memory(|m| m.focused())
        );
        press(&mut app, &ctx, egui::Key::ArrowUp);
        assert_eq!(app.command, "a=1");
        press(&mut app, &ctx, egui::Key::ArrowDown);
        press(&mut app, &ctx, egui::Key::ArrowDown);
        assert_eq!(app.command, "unfinished(");
        app.command = "A=[1 2;3 4];".into();
        focus_command(&app, &ctx);
        press(&mut app, &ctx, egui::Key::Enter);
        finish(&mut app);
        assert_eq!(
            app.variables["A"].as_numeric().unwrap().data(),
            &[1., 3., 2., 4.]
        );
        assert_eq!(app.history.entries.last().unwrap(), "A=[1 2;3 4];");
    }
    #[test]
    fn f9_runs_only_selection_and_cell_edits_preserve_aliases() {
        let ctx = egui::Context::default();
        let mut app = legacy_sample_desktop(ctx.clone(), true);
        frame(&mut app, &ctx, vec![]);
        app.documents[0].text = "% αβ\nselected=7;\nskipped=9;".into();
        app.documents[0].changed();
        app.selection = Some((0, 5..16));
        press(&mut app, &ctx, egui::Key::F9);
        finish(&mut app);
        assert_eq!(
            app.variables["selected"].as_numeric().unwrap().data(),
            &[7.]
        );
        assert!(!app.variables.contains_key("skipped"));
        app.run("A=[1 2;3 4];B=A;".into());
        finish(&mut app);
        app.inspect_variable("A".into());
        frame(&mut app, &ctx, vec![]);
        app.submit(Job::SetCell("A".into(), 1, 0, 9.0.into()));
        finish(&mut app);
        assert_eq!(
            app.variables["A"].as_numeric().unwrap().data(),
            &[1., 9., 2., 4.]
        );
        assert_eq!(
            app.variables["B"].as_numeric().unwrap().data(),
            &[1., 3., 2., 4.]
        );
    }
    #[test]
    fn opening_files_keeps_dirty_tabs_and_closing_prior_tab_preserves_selection() {
        let ctx = egui::Context::default();
        let mut app = legacy_sample_desktop(ctx, true);
        app.new_document("unsaved=1;");
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("other.m");
        std::fs::write(&path, "other=2;").unwrap();
        app.open_path(path.clone());
        finish(&mut app);
        assert_eq!(app.documents.len(), 3);
        assert_eq!(app.documents[1].text, "unsaved=1;");
        assert!(app.documents[1].dirty);
        app.open_path(path);
        finish(&mut app);
        assert_eq!(app.documents.len(), 3);
        app.remove_document(0);
        assert_eq!(app.active, 1);
        assert_eq!(app.documents[app.active].text, "other=2;");
    }
    #[test]
    fn closing_with_an_unsaved_model_is_cancelled_until_a_decision() {
        let ctx = egui::Context::default();
        let mut app = legacy_sample_desktop(ctx.clone(), true);
        frame(&mut app, &ctx, vec![]);
        app.toolkits.model.steps = 123;
        let mut input = egui::RawInput::default();
        input
            .viewports
            .entry(egui::ViewportId::ROOT)
            .or_default()
            .events
            .push(egui::ViewportEvent::Close);
        let output = crate::headless::run(&ctx, input, |root| app.ui(root));
        assert!(app.pending_quit);
        assert!(
            output.viewport_output[&egui::ViewportId::ROOT]
                .commands
                .iter()
                .any(|cmd| matches!(cmd, egui::ViewportCommand::CancelClose))
        );
        app.toolkits.discard_model();
        assert!(!app.toolkits.model_dirty());
    }
    #[test]
    fn f9_uses_document_local_functions_and_navigates_to_unselected_helper_errors() {
        let ctx = egui::Context::default();
        let mut app = legacy_sample_desktop(ctx.clone(), true);
        frame(&mut app, &ctx, vec![]);
        let source = "% αβ\nskipped=missing;\nR=twice(4);\nfunction y=twice(x)\ny=x*2;\nend";
        app.documents[0].text = source.into();
        app.documents[0].changed();
        let start = source.find("R=twice").unwrap();
        let range = matrust::editor::byte_chars(source, start..start + 11);
        app.selection = Some((0, range.clone()));
        press(&mut app, &ctx, egui::Key::F9);
        finish(&mut app);
        assert_eq!(
            app.variables["R"]
                .scalar(0, 0)
                .unwrap()
                .into_real()
                .unwrap(),
            8.
        );
        assert!(!app.variables.contains_key("skipped"));
        app.documents[0].text = source.replace("y=x*2", "y=missing");
        app.documents[0].changed();
        app.selection = Some((0, range));
        press(&mut app, &ctx, egui::Key::F9);
        finish(&mut app);
        assert_eq!(app.editor_issue.as_ref().unwrap().diagnostic.line, 5);
        app.jump_to_error();
        frame(&mut app, &ctx, vec![]);
        assert_eq!(
            session::selected_text(
                &app.documents[0].text,
                app.selection.as_ref().unwrap().1.clone()
            )
            .unwrap(),
            "y=missing;"
        );
        assert_eq!(
            app.variables["R"]
                .scalar(0, 0)
                .unwrap()
                .into_real()
                .unwrap(),
            8.
        );
        app.documents[0].text.push_str("\n% edited after failure");
        app.jump_to_error();
        assert!(app.status.contains("has changed"));
    }

    #[test]
    fn error_navigation_tracks_document_identity_and_selection_offset() {
        let ctx = egui::Context::default();
        let mut app = legacy_sample_desktop(ctx.clone(), true);
        app.new_document("% αβ\nskip=1; good=7; bad=missing(1);\n");
        let id = app.documents[1].id;
        let start = app.documents[1].text.find("good=").unwrap();
        let range =
            matrust::editor::byte_chars(&app.documents[1].text, start..app.documents[1].text.len());
        app.run_document(Some(range));
        app.remove_document(0);
        app.new_document("other=2;");
        finish(&mut app);
        let issue = app.editor_issue.as_ref().unwrap();
        assert_eq!(issue.origin.document, id);
        assert_eq!((issue.diagnostic.line, issue.diagnostic.column), (2, 17));
        assert_eq!(app.variables["good"].as_numeric().unwrap().data(), &[7.]);
        assert!(!app.variables.contains_key("skip"));
        app.jump_to_error();
        assert_eq!(app.active, 0);
        frame(&mut app, &ctx, vec![]);
        let selected = app.selection.as_ref().unwrap().1.clone();
        assert_eq!(
            session::selected_text(&app.documents[0].text, selected).unwrap(),
            "bad=missing(1);"
        );
        app.documents[0].text.push_str("% changed");
        app.active = 1;
        app.jump_to_error();
        assert_eq!(app.active, 1);
        assert!(app.status.contains("has changed"));
        app.remove_document(0);
        app.jump_to_error();
        assert!(app.status.contains("has been closed"));
    }

    #[test]
    fn control_flow_filtering_and_nested_errors_run_through_the_desktop_worker() {
        let ctx = egui::Context::default();
        let mut app = legacy_sample_desktop(ctx.clone(), true);
        app.new_document(include_str!("../examples/control_flow_filtering.m"));
        app.run_document(None);
        finish(&mut app);
        assert!(app.editor_issue.is_none());
        assert_eq!(
            app.variables["filters_run"]
                .scalar(0, 0)
                .unwrap()
                .into_real()
                .unwrap(),
            3.
        );
        assert_eq!(
            app.variables["chunks_run"]
                .scalar(0, 0)
                .unwrap()
                .into_real()
                .unwrap(),
            24.
        );
        assert_eq!(app.figure.as_ref().unwrap().series.len(), 4);
        app.new_document("% α\nfor k=1:3\n  if k==2\n    missing;\n  end\nend\n");
        app.run_document(None);
        finish(&mut app);
        assert_eq!(
            app.variables["k"]
                .scalar(0, 0)
                .unwrap()
                .into_real()
                .unwrap(),
            2.
        );
        assert_eq!(app.editor_issue.as_ref().unwrap().diagnostic.line, 4);
        app.jump_to_error();
        frame(&mut app, &ctx, vec![]);
        let selection = app.selection.as_ref().unwrap().1.clone();
        assert_eq!(
            session::selected_text(&app.documents[app.active].text, selection)
                .unwrap()
                .trim(),
            "missing;"
        );
    }

    #[test]
    fn user_functions_run_on_worker_and_local_errors_navigate_to_function_body() {
        let ctx = egui::Context::default();
        let mut app = legacy_sample_desktop(ctx.clone(), true);
        app.new_document(include_str!("../examples/user_functions.m"));
        app.run_document(None);
        finish(&mut app);
        assert!(app.editor_issue.is_none(), "{}", app.status);
        assert!(
            app.variables["error"]
                .scalar(0, 0)
                .unwrap()
                .into_real()
                .unwrap()
                < 1e-13
        );
        assert_eq!(app.figure.as_ref().unwrap().series.len(), 2);
        assert!(!app.variables.contains_key("first"));
        app.new_document("% α\nR=broken(1);\nfunction y=broken(x)\n  missing;\nend");
        app.run_document(None);
        finish(&mut app);
        assert_eq!(app.editor_issue.as_ref().unwrap().diagnostic.line, 4);
        app.jump_to_error();
        frame(&mut app, &ctx, vec![]);
        let selected = app.selection.as_ref().unwrap().1.clone();
        assert_eq!(
            session::selected_text(&app.documents[app.active].text, selected)
                .unwrap()
                .trim(),
            "missing;"
        );
    }

    #[test]
    fn external_error_navigation_opens_exact_source_and_preserves_disk_conflicts() {
        let folder = tempfile::tempdir().unwrap();
        let path = folder.path().join("broken.m");
        let source = "function y=broken(x)\n% αβ\n  y=missing;\nend";
        std::fs::write(&path, source).unwrap();
        let ctx = egui::Context::default();
        let mut app = legacy_sample_desktop(ctx.clone(), true);
        app.folder = folder.path().canonicalize().unwrap();
        app.new_document("R=broken(1);");
        app.run_document(None);
        finish(&mut app);
        assert_eq!(app.editor_issue.as_ref().unwrap().diagnostic.line, 3);
        let changed = "function y=broken(x);y=x;end";
        std::fs::write(&path, changed).unwrap();
        app.jump_to_error();
        assert_eq!(app.documents[app.active].text, source);
        assert!(!app.documents[app.active].dirty);
        frame(&mut app, &ctx, vec![]);
        assert_eq!(
            session::selected_text(
                &app.documents[app.active].text,
                app.selection.as_ref().unwrap().1.clone()
            )
            .unwrap(),
            "y=missing;"
        );
        assert!(matches!(
            app.documents[app.active].try_save(&path, None).unwrap(),
            matrust::document::SaveOutcome::Conflict(_)
        ));
        assert_eq!(std::fs::read_to_string(&path).unwrap(), changed);
        let count = app.documents.len();
        app.jump_to_error();
        assert_eq!(app.documents.len(), count);
        let helper = app.active;
        app.documents[helper].text.push_str("\n% unsaved");
        app.documents[helper].changed();
        app.active = 0;
        app.jump_to_error();
        assert_eq!(app.active, 0);
        assert!(app.status.contains("has changed"));
        assert!(app.documents[helper].dirty);
    }

    #[test]
    fn command_errors_navigate_to_helpers_and_respect_tab_limit() {
        let folder = tempfile::tempdir().unwrap();
        let path = folder.path().join("broken.m");
        std::fs::write(&path, "function y=broken(x)\ny=(;\nend").unwrap();
        let mut app = legacy_sample_desktop(egui::Context::default(), true);
        app.folder = folder.path().canonicalize().unwrap();
        app.run("R=broken(1);".into());
        finish(&mut app);
        assert_eq!(app.editor_issue.as_ref().unwrap().diagnostic.line, 2);
        while app.documents.len() < session::MAX_DOCUMENTS {
            app.new_document("% keep");
        }
        app.jump_to_error();
        assert!(app.status.contains("32 tabs"));
        app.remove_document(app.documents.len() - 1);
        app.jump_to_error();
        assert!(
            app.documents[app.active]
                .path
                .as_ref()
                .unwrap()
                .ends_with("broken.m")
        );
        let helper = app.active;
        app.remove_document(helper);
        std::fs::remove_file(path).unwrap();
        app.jump_to_error();
        assert!(app.documents[app.active].text.contains("y=(;"));
        app.run("R=1;".into());
        finish(&mut app);
        assert!(app.editor_issue.is_none());
    }

    #[test]
    fn external_error_navigation_reuses_matching_open_helper() {
        let folder = tempfile::tempdir().unwrap();
        let path = folder.path().join("broken.m");
        std::fs::write(&path, "function y=broken(x)\ny=missing;\nend").unwrap();
        let mut app = legacy_sample_desktop(egui::Context::default(), true);
        app.folder = folder.path().canonicalize().unwrap();
        app.documents.push(Document::open(&path).unwrap());
        let id = app.documents[1].id;
        app.run("R=broken(1);".into());
        finish(&mut app);
        app.jump_to_error();
        assert_eq!(app.documents.len(), 2);
        assert_eq!(app.documents[app.active].id, id);
    }

    #[test]
    fn export_dialog_retries_failures_keeps_dirty_figure_and_remembers_size() {
        let ctx = egui::Context::default();
        let mut app = legacy_sample_desktop(ctx.clone(), true);
        app.run("plot([0 1 2],[0 1 0]);".into());
        finish(&mut app);
        app.begin_figure_export();
        frame(&mut app, &ctx, vec![]);
        click_label(&mut app, &ctx, "Wide");
        assert_eq!(app.figure_export.as_ref().unwrap().settings.width, 1920);
        click_label(&mut app, &ctx, "Cancel");
        assert!(app.figure_export.is_none());
        app.begin_figure_export();
        assert_eq!(app.figure_export.as_ref().unwrap().settings.height, 1080);
        app.documents[0].text = "should_not_run=9;".into();
        app.selection = Some((0, 0..app.documents[0].text.chars().count()));
        press(&mut app, &ctx, egui::Key::F9);
        assert!(!app.busy);
        assert!(!app.variables.contains_key("should_not_run"));
        press(&mut app, &ctx, egui::Key::Escape);
        assert!(app.figure_export.is_none());
        app.begin_figure_export();
        let dir = tempfile::tempdir().unwrap();
        app.export_to_path(dir.path().join("wrong.jpg"));
        assert!(!app.busy);
        assert!(
            app.figure_export
                .as_ref()
                .unwrap()
                .error
                .as_ref()
                .unwrap()
                .contains(".png")
        );
        app.figure_export.as_mut().unwrap().settings.width = 8192;
        app.figure_export.as_mut().unwrap().settings.height = 8192;
        app.export_to_path(dir.path().join("oversized.png"));
        assert!(!app.busy);
        assert!(
            app.figure_export
                .as_ref()
                .unwrap()
                .error
                .as_ref()
                .unwrap()
                .contains("32 million")
        );
        app.figure_export.as_mut().unwrap().settings.width = 400;
        app.figure_export.as_mut().unwrap().settings.height = 300;
        app.export_to_path(dir.path().join("missing/failed.png"));
        finish(&mut app);
        assert!(app.figure_export.as_ref().unwrap().error.is_some());
        assert!(!app.figure_export.as_ref().unwrap().saving);
        let path = dir.path().join("success.png");
        app.export_to_path(path.clone());
        finish(&mut app);
        assert!(app.figure_export.is_none());
        assert_eq!(
            image::open(path).unwrap().to_rgb8().dimensions(),
            (400, 300)
        );
        assert!(app.figure_dirty(1));
        assert!(app.figure_documents[&1].path.is_none());
    }

    #[test]
    fn export_snapshot_keeps_original_figure_and_zoom_when_engine_changes() {
        let mut app = legacy_sample_desktop(egui::Context::default(), true);
        app.run("plot([0 1 2],[0 1 0]);title('Original');".into());
        finish(&mut app);
        app.view_bounds = Some([0.5, 1.5, 0.25, 0.75]);
        app.begin_figure_export();
        let dir = tempfile::tempdir().unwrap();
        app.figure_export.as_mut().unwrap().settings.format = figure_export::Format::Svg;
        let path = dir.path().join("original.svg");
        let job = app
            .figure_export
            .as_ref()
            .unwrap()
            .request(path.clone())
            .unwrap();
        let Job::ExportSnapshot(request) = &job else {
            panic!("Wrong export job")
        };
        assert!(
            (request.figure.series[0].points)
                .same_storage(&app.figure.as_ref().unwrap().series[0].points)
        );
        assert_eq!(request.figure.xlim, Some([0.5, 1.5]));
        assert_eq!(request.figure.ylim, Some([0.25, 0.75]));
        let expected = request.figure.svg();
        // Simulate a newer worker figure after the export snapshot was taken.
        app.run("figure(2);plot([0 1],[9 8]);title('Other');".into());
        finish(&mut app);
        app.submit(job);
        finish(&mut app);
        assert_eq!(std::fs::read_to_string(path).unwrap(), expected);
        assert_eq!(app.current_figure, 2);
    }

    #[test]
    fn discrete_plot_geometry_refreshes_and_worker_files_preserve_types() {
        let ctx = egui::Context::default();
        let mut app = legacy_sample_desktop(ctx.clone(), true);
        app.run(include_str!("../examples/discrete_plots.m").into());
        finish(&mut app);
        assert_eq!(app.plot_points[0].len(), 81);
        assert_eq!(app.plot_points[1].len(), 123);
        frame(&mut app, &ctx, vec![]);
        let original = app.figure.as_ref().unwrap().clone();
        let mut updated = original.clone();
        updated.series[0].kind = matrust::plot::PlotKind::Line;
        app.submit(Job::EditFigure(Box::new(worker::FigureEdit {
            view_before: None,
            id: app.current_figure,
            original: original.clone(),
            updated,
        })));
        finish(&mut app);
        assert_eq!(app.plot_points[0].len(), 41);
        assert!(
            (original.series[0].points)
                .same_storage(&app.figure.as_ref().unwrap().series[0].points)
        );
        app.run(include_str!("../examples/discrete_plots.m").replace("n = 0:40;", "clf;n = 0:40;"));
        finish(&mut app);
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("discrete.mfig");
        app.submit(Job::SaveFigure(path.clone(), None));
        finish(&mut app);
        assert!(!app.figure_dirty(app.current_figure));
        app.run("close all;".into());
        finish(&mut app);
        app.submit(Job::OpenFigure(path));
        finish(&mut app);
        assert_eq!(
            app.figure.as_ref().unwrap().series[0].kind,
            matrust::plot::PlotKind::Stairs
        );
        assert_eq!(
            app.figure.as_ref().unwrap().series[1].kind,
            matrust::plot::PlotKind::Stem
        );
        let png = dir.path().join("discrete.png");
        app.begin_figure_export();
        app.export_to_path(png.clone());
        finish(&mut app);
        assert!(app.figure_export.is_none());
        let image = image::open(png).unwrap().to_rgb8();
        assert!(
            image
                .pixels()
                .filter(|p| p.0[0] > p.0[2].saturating_add(80))
                .count()
                > 100
        );
        assert!(
            image
                .pixels()
                .filter(|p| p.0[2] > p.0[0].saturating_add(80))
                .count()
                > 100
        );
    }

    #[test]
    fn switching_from_subplots_restores_an_ordinary_figures_manual_zoom() {
        let ctx = egui::Context::default();
        let mut app = legacy_sample_desktop(ctx.clone(), true);
        app.run("figure(1);plot(1:100,1:100);".into());
        finish(&mut app);
        frame(&mut app, &ctx, vec![]);
        let zoom = [10., 20., 30., 40.];
        app.record_figure_view(zoom, [false, false]);
        app.view_bounds = Some(zoom);
        app.run("figure(2);subplot(2,1,1);plot([1 2]);subplot(2,1,2);stem([3 1]);".into());
        finish(&mut app);
        assert!(
            app.plot_points.is_empty(),
            "Subplot canvas owns the geometry cache"
        );
        frame(&mut app, &ctx, vec![]);
        assert!(!app.busy);
        app.run("figure(1)".into());
        finish(&mut app);
        assert_eq!(app.figure_views.get(&1).unwrap().bounds, zoom);
        frame(&mut app, &ctx, vec![]);
        assert!(!app.plot_points.is_empty());
        for (actual, expected) in app.view_bounds.unwrap().into_iter().zip(zoom) {
            assert!((actual - expected).abs() < 1e-9);
        }
    }

    #[test]
    fn subplot_worker_save_reopen_and_export_include_inactive_axes() {
        let ctx = egui::Context::default();
        let mut app = legacy_sample_desktop(ctx.clone(), true);
        app.run(include_str!("../examples/subplots.m").into());
        finish(&mut app);
        frame(&mut app, &ctx, vec![]);
        assert!(!app.busy);
        let original = app.figure.clone().unwrap();
        let mut updated = original.clone();
        updated.layout.as_mut().unwrap().others[0].axes.xlim = Some([1., 5.]);
        app.submit(Job::EditFigure(Box::new(worker::FigureEdit {
            view_before: None,
            id: app.current_figure,
            original,
            updated,
        })));
        finish(&mut app);
        let expected = app.figure.as_ref().unwrap().svg();
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("subplot.mfig");
        app.submit(Job::SaveFigure(path.clone(), None));
        finish(&mut app);
        assert!(!app.figure_dirty(app.current_figure));
        app.run("close all".into());
        finish(&mut app);
        app.submit(Job::OpenFigure(path));
        finish(&mut app);
        assert_eq!(app.figure.as_ref().unwrap().all_axes().count(), 3);
        assert_eq!(app.figure.as_ref().unwrap().svg(), expected);
        frame(&mut app, &ctx, vec![]);
        assert!(!app.busy);
        app.begin_figure_export();
        let png = dir.path().join("subplot.png");
        app.export_to_path(png.clone());
        finish(&mut app);
        let image = image::open(png).unwrap().to_rgb8();
        let (w, h) = image.dimensions();
        let mut counts = [0; 4];
        for (x, y, p) in image.enumerate_pixels() {
            let blue = p[2] > 180 && p[0] < 80 && p[1] < 80;
            let red = p[0] > 180 && p[1] < 80 && p[2] < 80;
            if y < h / 2 {
                if x < w / 2 && blue {
                    counts[0] += 1;
                }
                if x >= w / 2 && red {
                    counts[1] += 1;
                }
            } else {
                if blue {
                    counts[2] += 1;
                }
                if red {
                    counts[3] += 1;
                }
            }
        }
        assert!(counts.iter().all(|&n| n > 30), "{counts:?}");
        assert!(!app.figure_dirty(app.current_figure));
    }

    #[test]
    fn tiled_menu_preserves_prior_figure_and_roundtrips_empty_layout_recovery() {
        use matrust::session::SessionStore;
        let ctx = egui::Context::default();
        let mut app = legacy_sample_desktop(ctx.clone(), true);
        app.run("plot([5 7]);title('Preserve me');".into());
        finish(&mut app);
        let original = app.figure.clone().unwrap();
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("prior.mfig");
        app.submit(Job::SaveFigure(path.clone(), None));
        finish(&mut app);
        click_label(&mut app, &ctx, "Menu");
        click_label(&mut app, &ctx, "File");
        click_label(&mut app, &ctx, "Figures");
        click_label(&mut app, &ctx, "New layout");
        click_label(&mut app, &ctx, "2 by 2");
        finish(&mut app);
        assert_eq!(app.figure_ids.len(), 2);
        assert!(app.figure.as_ref().unwrap().empty_layout());
        assert_eq!(
            app.figure
                .as_ref()
                .unwrap()
                .layout
                .as_ref()
                .unwrap()
                .tiled
                .unwrap()
                .rows,
            2
        );
        assert!(!app.figure_dirty(1));
        assert_eq!(app.figure_documents[&1].path, Some(path));
        let recovery = dir.path().join("session.json");
        let mut store = SessionStore::new(&recovery);
        store.save(app.snapshot()).unwrap();
        let first = Session::load(&recovery).unwrap().figure_manifest.unwrap();
        app.run("sgtitle('Empty with title');".into());
        finish(&mut app);
        store.save(app.snapshot()).unwrap();
        let saved = Session::load(&recovery).unwrap();
        assert_ne!(
            first.entries.last().unwrap().file,
            saved
                .figure_manifest
                .as_ref()
                .unwrap()
                .entries
                .last()
                .unwrap()
                .file
        );
        let restored_ctx = egui::Context::default();
        let mut restored = legacy_sample_desktop(restored_ctx.clone(), true);
        restored.restore_session(saved);
        restored.start_session_recovery(recovery);
        finish(&mut restored);
        assert!(restored.figure.as_ref().unwrap().empty_layout());
        assert_eq!(
            restored
                .figure
                .as_ref()
                .unwrap()
                .layout
                .as_ref()
                .unwrap()
                .title,
            "Empty with title"
        );
        click_label(&mut restored, &restored_ctx, "Menu");
        click_label(&mut restored, &restored_ctx, "File");
        click_label(&mut restored, &restored_ctx, "Figures");
        click_label(&mut restored, &restored_ctx, "Next tile");
        finish(&mut restored);
        assert_eq!(restored.figure.as_ref().unwrap().all_axes().count(), 1);
        restored.run("plot([3 1]);nexttile;stem([2 4]);".into());
        finish(&mut restored);
        click_label(&mut restored, &restored_ctx, "Tile 1");
        finish(&mut restored);
        assert_eq!(
            restored.figure.as_ref().unwrap().series[0]
                .points
                .get(0)
                .unwrap()[1],
            3.
        );
        let tiled_path = dir.path().join("tiles.mfig");
        restored.submit(Job::SaveFigure(tiled_path.clone(), None));
        finish(&mut restored);
        let expected = restored.figure.as_ref().unwrap().svg();
        restored.submit(Job::OpenFigure(tiled_path));
        finish(&mut restored);
        assert_eq!(restored.figure.as_ref().unwrap().svg(), expected);
        assert!(
            restored
                .figure
                .as_ref()
                .unwrap()
                .layout
                .as_ref()
                .unwrap()
                .tiled
                .is_some()
        );
        app.run("figure(1);".into());
        finish(&mut app);
        assert!(app.figure.as_ref().unwrap().same_snapshot(&original));
        app.run("for k=1:16;figure(k);end;".into());
        finish(&mut app);
        let full = app.figure.clone().unwrap();
        let id = app.current_figure;
        app.submit(Job::NewLayout(
            matrust::plot::TileGrid::new(matrust::plot::Arrangement::Fixed, 2, 2).unwrap(),
        ));
        finish(&mut app);
        assert_eq!(app.figure_ids.len(), 16);
        assert_eq!(app.current_figure, id);
        assert!(app.figure.as_ref().unwrap().same_snapshot(&full));
    }

    #[test]
    fn positioned_fig_worker_open_save_recovery_and_export_preserve_all_axes() {
        let ctx = egui::Context::default();
        let mut app = legacy_sample_desktop(ctx.clone(), true);
        app.run("keep=42;plot([1 2]);".into());
        finish(&mut app);
        let prior = app.figure.clone().unwrap();
        app.submit(Job::OpenFigure(PathBuf::from(
            "tests/fixtures/fig/matlab-subplots.fig",
        )));
        finish(&mut app);
        assert_eq!(app.figure_ids.len(), 2);
        assert_eq!(app.figure.as_ref().unwrap().all_axes().count(), 2);
        assert_eq!(
            app.variables["keep"]
                .scalar(0, 0)
                .unwrap()
                .into_real()
                .unwrap(),
            42.
        );
        app.run("figure(1)".into());
        finish(&mut app);
        assert!(app.figure.as_ref().unwrap().same_snapshot(&prior));
        app.run(include_str!("../examples/fig_layouts.m").into());
        finish(&mut app);
        frame(&mut app, &ctx, vec![]);
        let dir = tempfile::tempdir().unwrap();
        let fig = dir.path().join("layout.fig");
        app.submit(Job::SaveFigure(fig.clone(), None));
        finish(&mut app);
        assert!(!app.figure_dirty(app.current_figure));
        let expected = app.figure.as_ref().unwrap().svg();
        app.submit(Job::OpenFigure(fig));
        finish(&mut app);
        assert_eq!(app.figure.as_ref().unwrap().svg(), expected);
        click_label(&mut app, &ctx, "Axes 1");
        finish(&mut app);
        assert_eq!(app.figure.as_ref().unwrap().title, "Input");
        let recovery = dir.path().join("session.json");
        let mut store = matrust::session::SessionStore::new(&recovery);
        store.save(app.snapshot()).unwrap();
        let mut restored = legacy_sample_desktop(egui::Context::default(), true);
        restored.restore_session(Session::load(&recovery).unwrap());
        restored.start_session_recovery(recovery);
        finish(&mut restored);
        assert_eq!(restored.figure.as_ref().unwrap().all_axes().count(), 3);
        assert_eq!(restored.figure.as_ref().unwrap().title, "Input");
        restored.begin_figure_export();
        let png = dir.path().join("layout.png");
        restored.export_to_path(png.clone());
        finish(&mut restored);
        let image = image::open(png).unwrap().to_rgb8();
        let mut blue = 0;
        let mut red = 0;
        for p in image.pixels() {
            if p[2] > 180 && p[0] < 80 && p[1] < 80 {
                blue += 1;
            }
            if p[0] > 180 && p[1] < 80 && p[2] < 80 {
                red += 1;
            }
        }
        assert!(blue > 100 && red > 100);
    }

    #[test]
    fn worker_resolves_function_files_in_selected_folder_and_sees_saved_edits() {
        let folder = tempfile::tempdir().unwrap();
        let path = folder.path().join("scale.m");
        std::fs::write(&path, "function y=scale(x);y=x*3;end").unwrap();
        let ctx = egui::Context::default();
        let mut app = legacy_sample_desktop(ctx, true);
        app.folder = folder.path().canonicalize().unwrap();
        app.new_document("R=scale(7);");
        app.run_document(None);
        finish(&mut app);
        assert_eq!(
            app.variables["R"]
                .scalar(0, 0)
                .unwrap()
                .into_real()
                .unwrap(),
            21.
        );
        std::fs::write(&path, "function y=scale(x);y=x*4;end").unwrap();
        app.run_document(None);
        finish(&mut app);
        assert_eq!(
            app.variables["R"]
                .scalar(0, 0)
                .unwrap()
                .into_real()
                .unwrap(),
            28.
        );
        assert!(!app.variables.contains_key("x"));
    }
}

#[cfg(test)]
#[path = "desktop/model_clipboard_recovery_tests.rs"]
mod model_clipboard_recovery_tests;

#[cfg(test)]
#[path = "desktop/figure_appearance_tests.rs"]
mod figure_appearance_tests;

#[cfg(test)]
#[path = "desktop/automatic_series_palette_tests.rs"]
mod automatic_series_palette_tests;
